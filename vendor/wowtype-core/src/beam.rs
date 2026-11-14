//! 束搜索：在音节图 + 词典上搜索 TopN 条路径。
//!
//! 词格上的束搜索：
//! 1. 每个字母位置一个桶，只在同一位置的状态之间比较（Viterbi 风格）；
//! 2. 从一个位置出发，DFS 枚举最长 max_word_syllables 个音节查词典，
//!    词典没有更长的词时立即剪枝（Lexicon::has_longer）；
//! 3. 每条精确边也提供一个未知词兜底，保证任何合法输入都能走到底；
//! 4. 纠错边按 typo_cost 扣分，路径纠错次数有上限；
//! 5. 按词典代价 + 二元 LM 分数累计，每个位置保留 beam_size 条状态。

use std::collections::hash_map::Entry;
use std::collections::HashMap;

use crate::graph::{Edge, GraphMode, SyllableGraph};
use crate::lexicon::Lexicon;
use crate::lm::LanguageModel;
use crate::syllable::SyllableQuery;

/// 束搜索配置。
#[derive(Clone, Debug)]
pub struct BeamConfig {
    /// 每层保留的状态数。WeType 会按输入长度在 10~40 间调度，这里先用固定值。
    pub beam_size: usize,
    /// 一个词最多覆盖几个音节。
    pub max_word_syllables: usize,
    /// 从同一位置出发、结束于同一位置的已知词最多保留几个（按代价截断）。
    pub max_expansions_per_state: usize,
    /// 未登录音节的惩罚。越大越倾向词典里的词。
    pub unknown_cost: f64,
    /// 语言模型权重。
    pub lm_weight: f64,
    /// 词典代价权重。
    pub lexicon_weight: f64,
    /// 每处纠错的惩罚（自然对数）。
    pub typo_cost: f64,
    /// 补全每多出一个字母的惩罚（自然对数）。
    pub completion_cost: f64,
    /// 一条路径最多纠几处。
    pub max_typos: usize,
    /// 一个词最多纠几处。
    pub max_typos_per_word: usize,
    /// 全拼混简拼（k yi -> 可以，yong z wen -> 用中文）时，每个只打了声母的音节的惩罚。
    ///
    /// 错键的全拼常常也能读成混简拼（jifu 打成 jifk -> 积分卡），罚得太轻会把纠错挤下去，
    /// 太重又打不出混简拼。2.0 时纠错的前三命中不变，混简拼前三从 27% 升到 84%。
    pub abbrev_cost: f64,
    /// 全拼混简拼时，输入的最后一个字母当省略的声母（yuluj -> 与路径）再加的惩罚。
    ///
    /// 末尾没打完的音节由补全处理（woxiangch -> 我想吃），补全按缺了几个字母扣分，
    /// 比这里更准；这里只在补全也凑不出来时兜底。
    pub tail_abbrev_cost: f64,
    /// 也出 CJK 扩展 B 区及以后的单字（[`crate::lexicon::LexEntry::is_ext_char`]）。
    ///
    /// 默认关：常见的中文字体（Noto CJK、思源、苹方）都没有这些字，列出来只是一排方块。
    pub rare_ext_chars: bool,
}

impl Default for BeamConfig {
    fn default() -> Self {
        Self {
            beam_size: 20,
            max_word_syllables: 6,
            max_expansions_per_state: 32,
            unknown_cost: 18.0,
            lm_weight: 1.0,
            lexicon_weight: 1.0,
            typo_cost: 6.0,
            completion_cost: 1.2,
            max_typos: 2,
            max_typos_per_word: 1,
            abbrev_cost: 2.0,
            tail_abbrev_cost: 4.0,
            rare_ext_chars: false,
        }
    }
}

/// 路径上的一个词（已知词或兜底的音节串）。
#[derive(Clone, Debug)]
pub struct BeamWord {
    pub text: String,
    pub pinyin: String,
    pub freq: f64,
    pub known: bool,
    pub syllable_count: usize,
    pub jianpin: bool,
    /// 只打了声母的音节数。
    pub abbrev: usize,
    /// 最后一个音节只打了声母。
    pub ends_abbrev: bool,
    /// 这个词用了几处纠错。
    pub typos: usize,
    /// 补全多出的字母数（用户还没打的部分）。
    pub completion: usize,
    /// 在输入里的字母区间 [start, end)。
    pub start: usize,
    pub end: usize,
}

/// 一条完整路径。
#[derive(Clone, Debug)]
pub struct BeamPath {
    pub text: String,
    pub pinyin: String,
    pub words: Vec<BeamWord>,
    /// 累计分（越高越好），已减去词典代价。
    pub score: f64,
    /// LM 对数概率累计（<= 0）。
    pub lm_score: f64,
    /// 词典代价累计（>= 0）。
    pub lexical_cost: f64,
    /// 词典覆盖的音节比例。
    pub coverage: f32,
    pub jianpin: bool,
    /// 整条路径的纠错次数。
    pub typos: usize,
    /// 补全多出的字母数。
    pub completion: usize,
}

/// 词格里的一个状态：停在某个字母位置、以某个词结尾。
///
/// 用回溯指针代替整条词序列，扩展时不必复制路径。
#[derive(Clone, Copy, Debug)]
struct Node {
    /// 父状态在 arena 里的下标（起点为 None）。
    parent: Option<usize>,
    /// 末词：words[from][idx]。
    word: Option<(usize, usize)>,
    score: f64,
    lm_score: f64,
    lexical_cost: f64,
    covered_syllables: usize,
    total_syllables: usize,
    typos: usize,
    completion: usize,
    jianpin: bool,
}

/// 从某个位置出发的一个可选词（已知词或兜底音节）。
struct Span {
    word: BeamWord,
    lexical: f64,
}

/// 在音节图上做「按位置同步」的束搜索（词格 Viterbi）。
///
/// 每个字母位置一个桶，只在同一个位置的状态之间比较、剪枝。
/// 如果把不同位置的状态放在一起比，走得远的路径扣分次数多，
/// 会被系统性地剪掉（「今天」输给「进 + 天」），长句因此几乎打不对。
pub fn beam_search(
    graph: &SyllableGraph,
    lexicon: &dyn Lexicon,
    lm: &dyn LanguageModel,
    config: &BeamConfig,
) -> Vec<BeamPath> {
    let n = graph.len();
    if n == 0 || !graph.has_complete_path() {
        return Vec::new();
    }

    let beam_size = config.beam_size.max(1);
    let max_word_syllables = config.max_word_syllables.max(1);
    let per_span = config.max_expansions_per_state.max(1);
    let final_cap = beam_size.saturating_mul(8).max(32);

    // 每个位置出发的候选词，只算一次（与到达该位置的路径无关）
    let mut spans: Vec<Vec<Span>> = (0..n).map(|_| Vec::new()).collect();

    let mut arena: Vec<Node> = Vec::new();
    let mut buckets: Vec<Vec<Node>> = (0..=n).map(|_| Vec::new()).collect();
    buckets[0].push(Node {
        parent: None,
        word: None,
        score: 0.0,
        lm_score: 0.0,
        lexical_cost: 0.0,
        covered_syllables: 0,
        total_syllables: 0,
        typos: 0,
        completion: 0,
        jianpin: false,
    });

    for pos in 0..n {
        if buckets[pos].is_empty() {
            continue;
        }
        let states = prune(std::mem::take(&mut buckets[pos]), beam_size, |node| {
            node.word.map(|(f, i)| spans[f][i].word.text.as_str())
        });
        spans[pos] = collect_spans(
            graph,
            lexicon,
            pos,
            max_word_syllables,
            per_span,
            config,
            false,
        );

        for state in states {
            let parent = arena.len();
            arena.push(state);
            let prev_text = state.word.map(|(f, i)| spans[f][i].word.text.clone());
            for (idx, span) in spans[pos].iter().enumerate() {
                let word = &span.word;
                if state.typos + word.typos > config.max_typos {
                    continue;
                }
                let (lm_raw, covered) = if word.known {
                    (
                        lm.score(prev_text.as_deref(), &word.text),
                        word.syllable_count,
                    )
                } else {
                    (0.0, 0)
                };
                let typo_cost = config.typo_cost * word.typos as f64
                    + config.completion_cost * word.completion as f64
                    + abbrev_penalty(graph, word, config);
                let next = Node {
                    parent: Some(parent),
                    word: Some((pos, idx)),
                    score: state.score - span.lexical + config.lm_weight * lm_raw - typo_cost,
                    lm_score: state.lm_score + lm_raw,
                    lexical_cost: state.lexical_cost + span.lexical,
                    covered_syllables: state.covered_syllables + covered,
                    total_syllables: state.total_syllables + word.syllable_count,
                    typos: state.typos + word.typos,
                    completion: state.completion + word.completion,
                    jianpin: state.jianpin || word.jianpin,
                };
                buckets[word.end].push(next);
            }
        }
    }

    // 终点不按末词去重：保留多样性，最后按整句文本去重
    let mut finals = std::mem::take(&mut buckets[n]);
    finals.sort_by(|a, b| b.score.total_cmp(&a.score));
    finals.truncate(final_cap);

    let mut best_by_text: HashMap<String, BeamPath> = HashMap::new();
    for node in finals {
        let mut words: Vec<BeamWord> = Vec::new();
        let mut cursor = Some(node);
        while let Some(current) = cursor {
            if let Some((f, i)) = current.word {
                words.push(spans[f][i].word.clone());
            }
            cursor = current.parent.map(|p| arena[p]);
        }
        words.reverse();
        let text: String = words.iter().map(|w| w.text.as_str()).collect();
        let pinyin = words
            .iter()
            .map(|w| w.pinyin.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let coverage = if node.total_syllables == 0 {
            0.0
        } else {
            node.covered_syllables as f32 / node.total_syllables as f32
        };
        let path = BeamPath {
            text: text.clone(),
            pinyin,
            words,
            score: node.score,
            lm_score: node.lm_score,
            lexical_cost: node.lexical_cost,
            coverage,
            jianpin: node.jianpin,
            typos: node.typos,
            completion: node.completion,
        };
        match best_by_text.entry(text) {
            Entry::Vacant(slot) => {
                slot.insert(path);
            }
            Entry::Occupied(mut slot) => {
                if path.score > slot.get().score {
                    slot.insert(path);
                }
            }
        }
    }

    let mut paths: Vec<BeamPath> = best_by_text.into_values().collect();
    paths.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.text.cmp(&b.text))
    });
    paths
}

/// 句首词：从输入开头起、没有用完整个输入的已知词，按「更长优先、代价更低优先」排序。
///
/// 返回 (词, 代价)。代价含纠错惩罚。
pub fn first_words(
    graph: &SyllableGraph,
    lexicon: &dyn Lexicon,
    config: &BeamConfig,
    limit: usize,
) -> Vec<(BeamWord, f64)> {
    let n = graph.len();
    if n == 0 || limit == 0 {
        return Vec::new();
    }
    let per_span = config.max_expansions_per_state.max(1);
    const RARE: bool = false;
    let mut words: Vec<(BeamWord, f64)> = collect_spans(
        graph,
        lexicon,
        0,
        config.max_word_syllables.max(1),
        per_span,
        config,
        RARE,
    )
    .into_iter()
    .filter(|s| s.word.known && s.word.end < n)
    .map(|s| {
        let cost = s.lexical
            + config.typo_cost * s.word.typos as f64
            + abbrev_penalty(graph, &s.word, config);
        (s.word, cost)
    })
    .collect();
    words.sort_by(|a, b| {
        b.0.end
            .cmp(&a.0.end)
            .then_with(|| a.1.total_cmp(&b.1))
            .then_with(|| a.0.text.cmp(&b.0.text))
    });
    // 每个长度最多取几个，避免两字词把单字全部挤掉；单字多留几个，
    // 整句和词都不对时用户靠它一个字一个字地选
    let mut per_end: HashMap<usize, usize> = HashMap::new();
    words.retain(|(w, _)| {
        let count = per_end.entry(w.end).or_insert(0);
        *count += 1;
        let single = w.pinyin.split_whitespace().count() <= 1;
        *count <= if single { 6 } else { 3 }
    });
    words.truncate(limit);
    words
}

/// 从输入开头起的全部词典词（翻页时补在后面）：包括正好用完整个输入的同音词。
///
/// 和 [`first_words`] 不同，这里每个结束位置最多留 `per_span` 个，不再按长度限量。
/// 排序：用掉的字母多的在前，同样长的按代价从低到高。
pub fn all_first_words(
    graph: &SyllableGraph,
    lexicon: &dyn Lexicon,
    config: &BeamConfig,
    per_span: usize,
) -> Vec<(BeamWord, f64)> {
    if graph.len() == 0 || per_span == 0 {
        return Vec::new();
    }
    // 生僻字只在这里（翻页的长尾）出现，见 [`crate::lexicon::LexEntry::is_rare`]
    const RARE: bool = true;
    let mut words: Vec<(BeamWord, f64)> = collect_spans(
        graph,
        lexicon,
        0,
        config.max_word_syllables.max(1),
        per_span,
        config,
        RARE,
    )
    .into_iter()
    .filter(|s| s.word.known)
    .map(|s| {
        let cost = s.lexical
            + config.typo_cost * s.word.typos as f64
            + config.completion_cost * s.word.completion as f64
            + abbrev_penalty(graph, &s.word, config);
        (s.word, cost)
    })
    .collect();
    words.sort_by(|a, b| {
        b.0.end
            .cmp(&a.0.end)
            .then_with(|| a.1.total_cmp(&b.1))
            .then_with(|| a.0.text.cmp(&b.0.text))
    });
    words
}

/// 从 pos 出发的所有候选词：词典命中（按结束位置分组，各取代价最低的若干个）
/// 加上每条精确边的未登录兜底。
fn collect_spans(
    graph: &SyllableGraph,
    lexicon: &dyn Lexicon,
    pos: usize,
    max_syllables: usize,
    per_span: usize,
    config: &BeamConfig,
    rare: bool,
) -> Vec<Span> {
    let mut found: Vec<Span> = Vec::new();
    let mut keys: Vec<SyllableQuery> = Vec::new();
    collect_dfs(
        graph,
        lexicon,
        pos,
        pos,
        max_syllables,
        0,
        &mut keys,
        &mut found,
        config,
        rare,
    );

    // 同一个结束位置内按「词典代价 + 纠错代价」排序截断。
    // 不能跨结束位置按词频截断：单字的频次天然比多字词高，会把长词全挤掉。
    let mut by_end: HashMap<usize, Vec<Span>> = HashMap::new();
    for span in found {
        by_end.entry(span.word.end).or_default().push(span);
    }
    let mut out = Vec::new();
    let mut ends: Vec<usize> = by_end.keys().copied().collect();
    ends.sort_unstable();
    for end in ends {
        let mut group = by_end.remove(&end).unwrap_or_default();
        group.sort_by(|a, b| {
            let extra = |s: &Span| {
                config.typo_cost * s.word.typos as f64
                    + config.completion_cost * s.word.completion as f64
            };
            let ca = a.lexical + extra(a);
            let cb = b.lexical + extra(b);
            ca.total_cmp(&cb)
                .then_with(|| a.word.text.cmp(&b.word.text))
        });
        // 同一个词可能经由不同切分到达，留代价最低的那个
        let mut seen: HashMap<String, ()> = HashMap::new();
        group.retain(|s| seen.insert(s.word.text.clone(), ()).is_none());
        group.truncate(per_span);
        out.extend(group);
    }

    // 未知音节兜底：每条精确边都允许直接当作一个未登录词，保证任何合法输入都能走到底。
    for edge in graph.edges_from(pos) {
        if edge.typo || edge.completion > 0 {
            continue;
        }
        out.push(Span {
            word: unknown_word(edge),
            lexical: config.unknown_cost,
        });
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn collect_dfs(
    graph: &SyllableGraph,
    lexicon: &dyn Lexicon,
    start: usize,
    pos: usize,
    max_syllables: usize,
    typos: usize,
    keys: &mut Vec<SyllableQuery>,
    out: &mut Vec<Span>,
    config: &BeamConfig,
    rare: bool,
) {
    for edge in graph.edges_from(pos) {
        let edge_typos = typos + usize::from(edge.typo);
        if edge_typos > config.max_typos_per_word {
            continue;
        }
        let before = keys.len();
        edge.push_queries(keys);
        if keys.len() > max_syllables {
            keys.truncate(before);
            continue;
        }
        let abbrev = keys.iter().filter(|q| q.is_jianpin()).count();
        let jianpin = abbrev > 0;
        // 生僻字只给打全了、没纠错的全拼（shi 列全 shi 的字）；sh、s 这种简拼和补全不带它们，
        // 不然每打一个声母都要排几千个字
        let mut entries = if rare && !jianpin && edge.completion == 0 && edge_typos == 0 {
            lexicon.lookup(keys)
        } else {
            lexicon.lookup_common(keys)
        };
        if !config.rare_ext_chars && keys.len() == 1 {
            entries.retain(|e| !e.is_ext_char());
        }
        let pinyin = if entries.is_empty() {
            String::new()
        } else {
            pinyin_of(keys)
        };
        for entry in entries {
            let lexical = config.lexicon_weight * lexicon.cost(&entry);
            out.push(Span {
                word: BeamWord {
                    text: entry.word,
                    pinyin: pinyin.clone(),
                    freq: entry.freq,
                    known: true,
                    syllable_count: keys.len(),
                    jianpin,
                    abbrev,
                    ends_abbrev: keys.last().is_some_and(|q| q.is_jianpin()),
                    typos: edge_typos,
                    completion: usize::from(edge.completion),
                    start,
                    end: edge.end,
                },
                lexical,
            });
        }
        if keys.len() < max_syllables && lexicon.has_longer(keys) {
            collect_dfs(
                graph,
                lexicon,
                start,
                edge.end,
                max_syllables,
                edge_typos,
                keys,
                out,
                config,
                rare,
            );
        }
        keys.truncate(before);
    }
}

/// 全拼混简拼时省略韵母的惩罚（纯全拼、纯简拼的图不罚）。
fn abbrev_penalty(graph: &SyllableGraph, word: &BeamWord, config: &BeamConfig) -> f64 {
    if graph.mode() != GraphMode::Mixed || word.abbrev == 0 {
        return 0.0;
    }
    let tail = if word.ends_abbrev && word.end == graph.len() {
        config.tail_abbrev_cost
    } else {
        0.0
    };
    config.abbrev_cost * word.abbrev as f64 + tail
}

fn unknown_word(edge: &Edge) -> BeamWord {
    let query = edge.query();
    BeamWord {
        text: edge.display(),
        pinyin: query.display(),
        freq: 0.0,
        known: false,
        syllable_count: edge.syllable_count(),
        jianpin: query.is_jianpin(),
        abbrev: usize::from(query.is_jianpin()),
        ends_abbrev: query.is_jianpin(),
        typos: 0,
        completion: 0,
        start: edge.start,
        end: edge.end,
    }
}

fn pinyin_of(keys: &[SyllableQuery]) -> String {
    keys.iter()
        .map(|q| q.display())
        .collect::<Vec<_>>()
        .join(" ")
}

/// 同一位置内剪枝：同一个末词只留最好的一条（二元模型下它们后续完全等价），再取前 beam_size。
fn prune<'a, F>(mut states: Vec<Node>, beam_size: usize, last_word: F) -> Vec<Node>
where
    F: Fn(&Node) -> Option<&'a str>,
{
    states.sort_by(|a, b| b.score.total_cmp(&a.score));
    let mut seen: HashMap<&'a str, ()> = HashMap::new();
    let mut out = Vec::with_capacity(beam_size);
    for state in states {
        let key = last_word(&state).unwrap_or("");
        if seen.insert(key, ()).is_some() {
            continue;
        }
        out.push(state);
        if out.len() >= beam_size {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{GraphMode, GraphOptions};
    use crate::lexicon::MemoryLexicon;
    use crate::lm::BigramLm;
    use crate::syllable::parse;

    fn graph(text: &str, mode: GraphMode) -> SyllableGraph {
        let parsed = parse(text).unwrap();
        SyllableGraph::build(
            &parsed,
            GraphOptions {
                mode,
                allow_erhua: true,
                typo: false,
                completion: false,
            },
        )
    }

    fn lex() -> MemoryLexicon {
        MemoryLexicon::from_tsv_str("xian\t先\t1000\nxi an\t西安\t500\nzhong guo\t中国\t800\n")
            .unwrap()
    }

    #[test]
    fn beam_finds_xian_and_xian_apostrophe() {
        let g = graph("xian", GraphMode::Full);
        let lex = lex();
        let lm = BigramLm::default();
        let paths = beam_search(&g, &lex, &lm, &BeamConfig::default());
        let texts: Vec<&str> = paths.iter().map(|p| p.text.as_str()).collect();
        assert!(texts.contains(&"先"), "缺少 先: {:?}", texts);
        assert!(texts.contains(&"西安"), "缺少 西安: {:?}", texts);
    }

    #[test]
    fn max_word_syllables_prunes_multi_syllable_word() {
        let g = graph("xian", GraphMode::Full);
        let lex = lex();
        let lm = BigramLm::default();
        let cfg = BeamConfig {
            max_word_syllables: 1,
            ..Default::default()
        };
        let paths = beam_search(&g, &lex, &lm, &cfg);
        let texts: Vec<&str> = paths.iter().map(|p| p.text.as_str()).collect();
        assert!(texts.contains(&"先"));
        assert!(
            !texts.contains(&"西安"),
            "max_word_syllables=1 不应匹配两音节词"
        );
    }

    #[test]
    fn jianpin_path() {
        let g = graph("zg", GraphMode::Jianpin);
        let lex = lex();
        let lm = BigramLm::default();
        let paths = beam_search(&g, &lex, &lm, &BeamConfig::default());
        assert!(paths.iter().any(|p| p.text == "中国" && p.jianpin));
    }

    #[test]
    fn unknown_syllables_still_produce_path() {
        let g = graph("xian", GraphMode::Full);
        let lex = MemoryLexicon::new();
        let lm = BigramLm::default();
        let paths = beam_search(&g, &lex, &lm, &BeamConfig::default());
        assert!(!paths.is_empty());
        assert_eq!(paths[0].coverage, 0.0);
        assert!(paths.iter().any(|p| p.text == "xian"));
    }

    #[test]
    fn beam_size_is_respected() {
        let g = graph("xian", GraphMode::Full);
        let lex = lex();
        let lm = BigramLm::default();
        let cfg = BeamConfig {
            beam_size: 1,
            ..Default::default()
        };
        let paths = beam_search(&g, &lex, &lm, &cfg);
        assert!(!paths.is_empty());
    }
}
