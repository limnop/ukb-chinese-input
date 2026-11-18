//! 从用户自己的输入里学：选过的词有多「热」，哪两个词常挨着用。
//!
//! 照着微信输入法的思路（`SelectedHistory`、`history_hot_val`、`context_bigram_score`）：
//!
//! - 热度：每选一次加 1，按 30 天半衰期衰减（WeType 超参数里的 2592000 秒）。
//!   排序用 ln(1+热度)，选得越多越靠前，但涨得越来越慢；很久不用就慢慢退回去。
//! - 搭配：用户上屏时记下「上一个词 -> 这个词」（同一句里相邻的词，以及上一次上屏的
//!   最后一个词 -> 这次的第一个词）。束搜索给这些搭配加分，所以 `fagei` 选过一次
//!   「发给」以后，`wofageini` 出「我发给你」而不是「我发个你」。
//!
//! 都存在 `history.tsv`（和用户词库放一起），重启输入法不会丢。
//!
//! 存盘时淘汰一些（[`save_history`]）：只用过一次、又很久没再用的不存；条数超过上限时不存最冷的。
//! 不淘汰的话文件只会变长——用一两周就有九千多行，每次存盘都整份重写。

use std::collections::{HashMap, VecDeque};
use std::path::Path;

use anyhow::{Context, Result};

use crate::lm::LanguageModel;
use crate::HistoryEntry;

/// 半衰期：30 天（毫秒）。
pub const HALF_LIFE_MS: f64 = 30.0 * 24.0 * 3600.0 * 1000.0;

/// 一个学过的搭配给多少分（自然对数）：`PAIR_WEIGHT * ln(1+热度)`，最多 [`PAIR_BONUS_CAP`]。
///
/// 用过一次约 1.7，三次约 3.5——足够让常见的同音搭配（发给/发个、再/在）翻过来，
/// 又不至于让一次误选永远压住词库里差得很远的词。
pub const PAIR_WEIGHT: f64 = 2.5;
pub const PAIR_BONUS_CAP: f64 = 6.0;

/// 最多存这么多个词的热度（`w` 行）。超了就丢衰减以后热度最低的。
///
/// 正常用不到：热度衰减到 [`PRUNE_HEAT`] 以下的本来就不存（选过一次的约 150 天）。
/// 留这个上限是防着文件无止境地长（每次存盘整份重写，载入也整份读进内存）。
pub const MAX_WORDS: usize = 20_000;

/// 最多存这么多条搭配（`p` 行），同上。搭配比词多（一两周约六千条），上限给到两倍。
pub const MAX_PAIRS: usize = 40_000;

/// 选词热度衰减到这么低（相当于不到半次）就不再让候选提前（[`crate::Engine`] 的「上次选过的放第一」）：
/// 误选一次的词 30 天以后不再一直顶在第一个。
pub const MIN_PICK_HEAT: f64 = 0.5;

/// 存盘时热度低于这个的记录不再写（选过一次的约 150 天）：排序里已经几乎不起作用，留着只让文件越来越大。
pub const PRUNE_HEAT: f64 = 1.0 / 32.0;

/// 分几次选出来拼成的词第一次只记成「候选新词」，这么久以内再拼一次才收进用户词库（30 天，毫秒）。
pub const NEW_WORD_TTL_MS: u64 = 30 * 24 * 3600 * 1000;

/// 按半衰期衰减后的次数（热度）。
pub fn decayed(entry: HistoryEntry, at_ms: u64) -> f64 {
    let age = at_ms.saturating_sub(entry.last_used_ms) as f64;
    entry.count * 0.5f64.powf(age / HALF_LIFE_MS)
}

/// 先按时间衰减再加 1（很久以前用得多的不会一直顶着），不取整：取整会把半次凑成一次。
pub fn bumped(before: Option<HistoryEntry>, at_ms: u64) -> HistoryEntry {
    HistoryEntry {
        count: before.map_or(0.0, |e| decayed(e, at_ms)) + 1.0,
        last_used_ms: at_ms,
    }
}

/// 分几次选出来拼成、还没收进用户词库的词（「候选新词」）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewWord {
    pub pinyin: String,
    /// 第一次拼出来的时间（epoch 毫秒）。
    pub at_ms: u64,
}

/// 一次学习改了什么：改之前、改之后的值（None 是没有这一条）。撤销时只在现在的值还等于「之后」时
/// 才改回「之前」，后来又被别的学习改过的不动。
#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    /// 选词热度（history.tsv 的 w 行）。
    Word {
        word: String,
        before: Option<HistoryEntry>,
        after: Option<HistoryEntry>,
    },
    /// 词搭配（p 行）。
    Pair {
        prev: String,
        cur: String,
        before: Option<HistoryEntry>,
        after: Option<HistoryEntry>,
    },
    /// 最近一次选的词（接下一次的搭配用）。
    LastWord { before: String, after: String },
    /// 用户词库里的词：(拼音, 频次)。
    UserWord {
        word: String,
        before: Option<(String, f64)>,
        after: Option<(String, f64)>,
    },
    /// 候选新词（n 行）。
    NewWord {
        word: String,
        before: Option<NewWord>,
        after: Option<NewWord>,
    },
}

/// 学习日志最多留几条改动（一次上屏一般十来条，够撤销最近几次）。
pub const JOURNAL_MAX: usize = 1024;

/// 最近的学习改动，按顺序编号：撤销最近一次学习时按编号区间退回去，不用重算全部。
#[derive(Debug, Default)]
pub struct Journal {
    /// changes[0] 的编号。
    first: u64,
    /// 撤销过的是 None（编号不变）。
    changes: VecDeque<Option<Change>>,
}

impl Journal {
    /// 下一条改动的编号：学习前后各取一次，中间就是这次学习改的。
    pub fn mark(&self) -> u64 {
        self.first + self.changes.len() as u64
    }

    pub fn push(&mut self, change: Change) {
        self.changes.push_back(Some(change));
        while self.changes.len() > JOURNAL_MAX {
            self.changes.pop_front();
            self.first += 1;
        }
    }

    /// 取出编号在 [start, end) 里、还没撤销的改动，新的在前。start 那条已经挤出日志时返回 None
    /// （只退一半不如不退）。
    pub fn take(&mut self, start: u64, end: u64) -> Option<Vec<Change>> {
        if start < self.first {
            return None;
        }
        let end = end.min(self.mark());
        let mut out = Vec::new();
        for seq in (start..end).rev() {
            if let Some(change) = self.changes[(seq - self.first) as usize].take() {
                out.push(change);
            }
        }
        Some(out)
    }
}

/// 用户学到的词搭配：前一个词 -> 后一个词 -> 次数与上次使用时间。
#[derive(Default, Debug, Clone)]
pub struct UserPairs {
    map: HashMap<String, HashMap<String, HistoryEntry>>,
}

impl UserPairs {
    /// 记一次 prev -> cur，返回改动（撤销用）；有一个是空的什么也不记。
    pub fn learn(&mut self, prev: &str, cur: &str, at_ms: u64) -> Option<Change> {
        if prev.is_empty() || cur.is_empty() {
            return None;
        }
        // 次数先按时间衰减再加 1，这样很久以前用过很多次的搭配不会一直顶着
        let before = self.get(prev, cur);
        let after = bumped(before, at_ms);
        self.insert(prev.to_string(), cur.to_string(), after);
        Some(Change::Pair {
            prev: prev.to_string(),
            cur: cur.to_string(),
            before,
            after: Some(after),
        })
    }

    /// 改成 entry（None 是去掉这一条），撤销用。
    pub fn set(&mut self, prev: &str, cur: &str, entry: Option<HistoryEntry>) {
        match entry {
            Some(entry) => self.insert(prev.to_string(), cur.to_string(), entry),
            None => {
                if let Some(next) = self.map.get_mut(prev) {
                    next.remove(cur);
                    if next.is_empty() {
                        self.map.remove(prev);
                    }
                }
            }
        }
    }

    /// prev 后面学过的词和它们的热度（at_ms 时），热的在前。
    pub fn next_words(&self, prev: &str, at_ms: u64) -> Vec<(String, f64)> {
        let mut out: Vec<(String, f64)> = self
            .map
            .get(prev)
            .map(|next| {
                next.iter()
                    .map(|(cur, entry)| (cur.clone(), decayed(*entry, at_ms)))
                    .collect()
            })
            .unwrap_or_default();
        out.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        out
    }

    pub fn get(&self, prev: &str, cur: &str) -> Option<HistoryEntry> {
        self.map.get(prev).and_then(|next| next.get(cur)).copied()
    }

    /// 这个搭配的加分（没学过是 0）。
    pub fn bonus(&self, prev: &str, cur: &str, at_ms: u64) -> f64 {
        match self.get(prev, cur) {
            Some(entry) => (PAIR_WEIGHT * (1.0 + decayed(entry, at_ms)).ln()).min(PAIR_BONUS_CAP),
            None => 0.0,
        }
    }

    /// 去掉和 word 有关的搭配（它在前或在后的都去掉），返回去掉了几条。
    pub fn remove_word(&mut self, word: &str) -> usize {
        let mut removed = self.map.remove(word).map_or(0, |next| next.len());
        for next in self.map.values_mut() {
            removed += usize::from(next.remove(word).is_some());
        }
        self.map.retain(|_, next| !next.is_empty());
        removed
    }

    pub fn len(&self) -> usize {
        self.map.values().map(HashMap::len).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str, HistoryEntry)> {
        self.map.iter().flat_map(|(prev, next)| {
            next.iter()
                .map(move |(cur, entry)| (prev.as_str(), cur.as_str(), *entry))
        })
    }

    fn insert(&mut self, prev: String, cur: String, entry: HistoryEntry) {
        self.map.entry(prev).or_default().insert(cur, entry);
    }
}

/// 基础语言模型 + 用户学到的搭配。束搜索用它打分。
///
/// `start` 是光标前最后一个词（上一次上屏的），句首的第一个词用它查搭配。
pub struct LearnedLm<'a> {
    pub base: &'a dyn LanguageModel,
    pub pairs: &'a UserPairs,
    pub start: Option<&'a str>,
    pub at_ms: u64,
    /// 光标前已经有字（接着前文打）：第一个词不是句首，不加基础模型的句首分。
    pub mid_sentence: bool,
}

impl LanguageModel for LearnedLm<'_> {
    fn score(&self, prev: Option<&str>, cur: &str) -> f64 {
        let base = if prev.is_none() && self.mid_sentence {
            0.0
        } else {
            self.base.score(prev, cur)
        };
        match prev.or(self.start) {
            Some(prev) => base + self.pairs.bonus(prev, cur, self.at_ms),
            None => base,
        }
    }

    fn has_bigram(&self, prev: &str, cur: &str) -> bool {
        self.base.has_bigram(prev, cur) || self.pairs.get(prev, cur).is_some()
    }

    fn top_next(&self, prev: &str, limit: usize) -> Vec<(String, f64)> {
        self.base.top_next(prev, limit)
    }
}

/// 存盘时留下哪些条目：比 `cap` 多就只留衰减以后热度最高的 `cap` 条，没超就原样返回。
/// （热度太低的在调用方就去掉了，见 [`PRUNE_HEAT`]。）
fn kept_for_save<K: Ord>(
    mut items: Vec<(K, HistoryEntry)>,
    _now_ms: u64,
    cap: usize,
) -> Vec<(K, HistoryEntry)> {
    if items.len() > cap {
        // 衰减后的热度是 次数 × 0.5^(过了多久 / 半衰期)。比大小用它的对数：很久以前的条目
        // 直接算会全变成 0、分不出先后；而且「现在」是几点对谁热谁冷没有影响，可以不要
        let heat = |e: &HistoryEntry| {
            e.count.max(1e-9).ln() + e.last_used_ms as f64 * std::f64::consts::LN_2 / HALF_LIFE_MS
        };
        // 热的在前；一样热的留后用的，再一样按字面（每次存出来的都一样）
        items.select_nth_unstable_by(cap, |a, b| {
            heat(&b.1)
                .total_cmp(&heat(&a.1))
                .then(b.1.last_used_ms.cmp(&a.1.last_used_ms))
                .then_with(|| a.0.cmp(&b.0))
        });
        items.truncate(cap);
    }
    items
}

/// 没有一个非 ASCII 字符的词（回车上屏的原始字母串、英文词）：不存也不载入。
///
/// 选词记录只在给束搜索出来的中文候选打分时用，原始串和英文候选的位置从来不看它；以前回车上屏的原始字母串
/// 也记一次选词，终端里中文模式下打的密码就这样明文留在了 history.tsv 里。现在会话不再记它们，
/// 载入时也跳过旧文件里的这些行，下次存盘就从文件里清掉了。
fn plain_ascii(text: &str) -> bool {
    text.is_ascii()
}

/// 存盘格式（TSV，`#` 开头是注释，各列用 tab 分隔）：
///
/// ```text
/// w <词> <热度> <上次使用，epoch 毫秒>
/// p <前一个词> <后一个词> <热度> <上次使用>
/// n <候选新词> <拼音> <第一次拼出来的时间>
/// ```
///
/// 热度是上次使用时衰减过的次数，可以带小数（最多三位；旧版本只写整数，读得进来）。
/// now_ms 时热度已经低于 [`PRUNE_HEAT`] 的、过了 [`NEW_WORD_TTL_MS`] 的候选新词不写；
/// 超过 [`MAX_WORDS`] / [`MAX_PAIRS`] 的不写最冷的。内存里的不动（这次会话里排序不变）。
/// 没有一个非 ASCII 字符的词（回车上屏的原始字母串、英文词）不写也不载入，见 [`plain_ascii`]。
pub fn history_text(
    words: &HashMap<String, HistoryEntry>,
    pairs: &UserPairs,
    new_words: &HashMap<String, NewWord>,
    now_ms: u64,
) -> String {
    let mut out = String::from("# wowtype 用户输入习惯 v1\n");
    let one_column = |s: &str| !s.is_empty() && !s.contains(['\t', '\n', '\r']);
    let clean = |s: &str| one_column(s) && !plain_ascii(s);
    let live = |e: &HistoryEntry| decayed(*e, now_ms) >= PRUNE_HEAT;
    let words = words
        .iter()
        .filter(|(w, e)| clean(w) && live(e))
        .map(|(w, e)| (w, *e))
        .collect();
    let mut words = kept_for_save(words, now_ms, MAX_WORDS);
    words.sort_by(|a, b| a.0.cmp(b.0));
    for (word, entry) in words {
        out.push_str(&format!(
            "w\t{word}\t{}\t{}\n",
            heat_text(entry.count),
            entry.last_used_ms
        ));
    }
    let pairs = pairs
        .iter()
        .filter(|(p, c, e)| clean(p) && clean(c) && live(e))
        .map(|(p, c, e)| ((p, c), e))
        .collect();
    let mut pairs = kept_for_save(pairs, now_ms, MAX_PAIRS);
    pairs.sort_by(|a, b| a.0.cmp(&b.0));
    for ((prev, cur), entry) in pairs {
        out.push_str(&format!(
            "p\t{prev}\t{cur}\t{}\t{}\n",
            heat_text(entry.count),
            entry.last_used_ms
        ));
    }
    let mut new_words: Vec<_> = new_words
        .iter()
        .filter(|(w, n)| {
            clean(w) && one_column(&n.pinyin) && now_ms.saturating_sub(n.at_ms) <= NEW_WORD_TTL_MS
        })
        .collect();
    new_words.sort_by(|a, b| a.0.cmp(b.0));
    for (word, new) in new_words {
        out.push_str(&format!("n\t{word}\t{}\t{}\n", new.pinyin, new.at_ms));
    }
    out
}

/// 热度写成文字：整数照旧写整数，否则最多三位小数。
fn heat_text(count: f64) -> String {
    if (count - count.round()).abs() < 1e-6 {
        return format!("{}", count.round().max(0.0) as u64);
    }
    let text = format!("{count:.3}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// 把 [`history_text`] 整份写进 path。
pub fn save_history<P: AsRef<Path>>(
    path: P,
    words: &HashMap<String, HistoryEntry>,
    pairs: &UserPairs,
    new_words: &HashMap<String, NewWord>,
    now_ms: u64,
) -> Result<()> {
    let text = history_text(words, pairs, new_words, now_ms);
    // 先写临时文件再改名，写到一半断电也不会把旧文件弄坏
    crate::fsutil::write_atomic(path.as_ref(), text.as_bytes())
}

/// 读回 [`save_history`] 写的文件，返回读到的条数。坏行（列数不对、数字解析不了、
/// 不是合法 UTF-8）跳过，别的行照常，见 [`crate::fsutil::read_lines_lossy`]。
pub fn load_history<P: AsRef<Path>>(
    path: P,
    words: &mut HashMap<String, HistoryEntry>,
    pairs: &mut UserPairs,
    new_words: &mut HashMap<String, NewWord>,
) -> Result<usize> {
    let path = path.as_ref();
    let (lines, _bad) = crate::fsutil::read_lines_lossy(path)
        .with_context(|| format!("读取 {}", path.display()))?;
    let mut loaded = 0usize;
    for line in &lines {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split('\t').collect();
        let entry = |count: &str, last: &str| -> Option<HistoryEntry> {
            let count: f64 = count.parse().ok()?;
            (count.is_finite() && count >= 0.0).then_some(())?;
            Some(HistoryEntry {
                count,
                last_used_ms: last.parse().ok()?,
            })
        };
        match parts.as_slice() {
            ["w", word, count, last] if !word.is_empty() && !plain_ascii(word) => {
                if let Some(e) = entry(count, last) {
                    words.insert((*word).to_string(), e);
                    loaded += 1;
                }
            }
            ["p", prev, cur, count, last]
                if !prev.is_empty() && !cur.is_empty() && !plain_ascii(prev) && !plain_ascii(cur) =>
            {
                if let Some(e) = entry(count, last) {
                    pairs.insert((*prev).to_string(), (*cur).to_string(), e);
                    loaded += 1;
                }
            }
            ["n", word, pinyin, at] if !word.is_empty() && !pinyin.is_empty() => {
                if let Ok(at_ms) = at.parse() {
                    let new = NewWord {
                        pinyin: (*pinyin).to_string(),
                        at_ms,
                    };
                    new_words.insert((*word).to_string(), new);
                    loaded += 1;
                }
            }
            _ => {}
        }
    }
    Ok(loaded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lm::BigramLm;

    /// 旧文件里纯 ASCII 的词和搭配（回车上屏的原始字母串）载入时跳过，存盘后文件里也没有了。
    #[test]
    fn plain_ascii_entries_are_dropped() {
        let dir = std::env::temp_dir().join(format!("wowtype_ascii_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("history.tsv");
        std::fs::write(
            &path,
            "# wowtype 用户输入习惯 v1\nw\thunter2pass\t3\t1000\nw\t你好\t2\t1000\n\
             p\t你好\tsecret\t1\t1000\np\tsecret\t世界\t1\t1000\np\t你好\t世界\t1\t1000\n",
        )
        .unwrap();
        let mut words = HashMap::new();
        let mut pairs = UserPairs::default();
        let mut new_words = HashMap::new();
        assert_eq!(load_history(&path, &mut words, &mut pairs, &mut new_words).unwrap(), 2);
        assert!(words.contains_key("你好"));
        assert!(!words.contains_key("hunter2pass"));
        save_history(&path, &words, &pairs, &new_words, 1000).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("hunter2pass") && !text.contains("secret"), "{text}");
        assert!(text.contains("你好\t世界"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn heat_decays_with_half_life() {
        let e = HistoryEntry {
            count: 4.0,
            last_used_ms: 0,
        };
        assert!((decayed(e, 0) - 4.0).abs() < 1e-9);
        assert!((decayed(e, HALF_LIFE_MS as u64) - 2.0).abs() < 1e-6);
    }

    #[test]
    fn bumped_keeps_fractions() {
        // 30 天前选过一次：衰减到一半再加 1，是 1.5 次，不取整成 2
        let once = bumped(None, 0);
        assert_eq!(once.count, 1.0);
        let again = bumped(Some(once), HALF_LIFE_MS as u64);
        assert!((again.count - 1.5).abs() < 1e-9);
    }

    #[test]
    fn journal_takes_ranges_newest_first() {
        let mut journal = Journal::default();
        let start = journal.mark();
        for i in 0..3 {
            journal.push(Change::LastWord {
                before: i.to_string(),
                after: (i + 1).to_string(),
            });
        }
        let end = journal.mark();
        assert_eq!(end - start, 3);
        let taken = journal.take(start + 1, end).unwrap();
        assert_eq!(taken.len(), 2);
        assert!(matches!(&taken[0], Change::LastWord { after, .. } if after == "3"));
        // 撤销过的不会再取出来
        assert_eq!(journal.take(start, end).unwrap().len(), 1);
        // 挤出日志的取不了
        for _ in 0..JOURNAL_MAX {
            journal.push(Change::LastWord {
                before: String::new(),
                after: String::new(),
            });
        }
        assert!(journal.take(start, end).is_none());
    }

    #[test]
    fn learned_pair_adds_to_score_and_uses_start_word() {
        let mut pairs = UserPairs::default();
        pairs.learn("发", "给", 1000);
        let base = BigramLm::default();
        let lm = LearnedLm {
            base: &base,
            pairs: &pairs,
            start: Some("发"),
            at_ms: 1000,
            mid_sentence: false,
        };
        assert!(lm.score(Some("发"), "给") > lm.score(Some("发"), "个"));
        // 句首没有前一个词时用 start
        assert!(lm.score(None, "给") > lm.score(None, "个"));
    }

    #[test]
    fn history_round_trips() {
        let dir = std::env::temp_dir().join(format!("wowtype-learn-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("history.tsv");
        let mut words = HashMap::new();
        words.insert(
            "种地".to_string(),
            HistoryEntry {
                count: 3.0,
                last_used_ms: 42,
            },
        );
        words.insert(
            "衰减过".to_string(),
            HistoryEntry {
                count: 1.2599210498948732,
                last_used_ms: 42,
            },
        );
        let mut pairs = UserPairs::default();
        pairs.learn("发", "给", 7);
        let mut new_words = HashMap::new();
        new_words.insert(
            "霜频阁".to_string(),
            NewWord {
                pinyin: "shuang pin ge".into(),
                at_ms: 40,
            },
        );
        save_history(&path, &words, &pairs, &new_words, 42).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("w\t衰减过\t1.26\t42\n"), "带小数的热度：{text}");
        assert!(text.contains("w\t种地\t3\t42\n"), "整数照旧：{text}");

        let mut w2 = HashMap::new();
        let mut p2 = UserPairs::default();
        let mut n2 = HashMap::new();
        assert_eq!(load_history(&path, &mut w2, &mut p2, &mut n2).unwrap(), 4);
        assert_eq!(w2["种地"].count, 3.0);
        assert_eq!(w2["衰减过"].count, 1.26);
        assert_eq!(p2.get("发", "给").unwrap().count, 1.0);
        assert_eq!(n2["霜频阁"].pinyin, "shuang pin ge");

        // 存盘时丢掉衰减到几乎没有的记录、过期的候选新词
        let later = 42 + 6 * HALF_LIFE_MS as u64;
        let text = history_text(&w2, &p2, &n2, later);
        assert!(!text.contains("衰减过"), "{text}");
        assert!(!text.contains("霜频阁"), "{text}");
        assert!(text.contains("种地"), "3 次的还在（6 个半衰期后约 0.05）：{text}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn one_bad_byte_does_not_lose_the_history() {
        // 以前 read_to_string 整份失败，外壳照常存盘，文件被这次会话的内容盖掉
        let dir = std::env::temp_dir().join(format!("wowtype-learn-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("history.tsv");
        let mut body = "# wowtype 用户输入习惯 v1\nw\t种地\t3\t42\n"
            .as_bytes()
            .to_vec();
        body.extend_from_slice(b"w\t\xe7\xa7\xff\t9\t50\n");
        body.extend_from_slice("w\t不是数字\tx\t1\np\t发\t给\t2\t7\n".as_bytes());
        std::fs::write(&path, body).unwrap();

        let mut words = HashMap::new();
        let mut pairs = UserPairs::default();
        let mut new_words = HashMap::new();
        assert_eq!(load_history(&path, &mut words, &mut pairs, &mut new_words).unwrap(), 2);
        assert_eq!(words.len(), 1);
        assert_eq!(words["种地"].count, 3.0);
        assert_eq!(pairs.get("发", "给").unwrap().count, 2.0);
        std::fs::remove_dir_all(&dir).ok();
        // 文件不存在还是报错
        assert!(load_history(&path, &mut words, &mut pairs, &mut new_words).is_err());
    }

    const DAY_MS: u64 = 24 * 3600 * 1000;

    fn entry(count: u32, last_used_ms: u64) -> HistoryEntry {
        HistoryEntry {
            count: f64::from(count),
            last_used_ms,
        }
    }

    fn save_and_load(
        tag: &str,
        words: &HashMap<String, HistoryEntry>,
        pairs: &UserPairs,
        now_ms: u64,
    ) -> (HashMap<String, HistoryEntry>, UserPairs, String) {
        let dir = std::env::temp_dir().join(format!("wowtype-learn-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("history.tsv");
        save_history(&path, words, pairs, &HashMap::new(), now_ms).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let mut w2 = HashMap::new();
        let mut p2 = UserPairs::default();
        load_history(&path, &mut w2, &mut p2, &mut HashMap::new()).unwrap();
        std::fs::remove_dir_all(&dir).ok();
        (w2, p2, text)
    }

    /// 衰减以后热度低于 PRUNE_HEAT（1/32，五个半衰期）的不存：选过一次的 150 天，两次的 180 天。
    #[test]
    fn cold_entries_are_not_saved() {
        let now = 400 * DAY_MS;
        let mut words = HashMap::new();
        words.insert("刚用".to_string(), entry(1, now));
        words.insert("一次很久".to_string(), entry(1, now - 151 * DAY_MS));
        words.insert("一次不久".to_string(), entry(1, now - 140 * DAY_MS));
        words.insert("两次很久".to_string(), entry(2, now - 175 * DAY_MS));
        let mut pairs = UserPairs::default();
        pairs.insert("发".into(), "给".into(), entry(1, now - 151 * DAY_MS));
        pairs.insert("再".into(), "见".into(), entry(3, now - 190 * DAY_MS));
        let (w2, p2, _) = save_and_load("cold", &words, &pairs, now);
        let mut kept: Vec<_> = w2.keys().cloned().collect();
        kept.sort();
        assert_eq!(kept, ["一次不久", "两次很久", "刚用"]);
        assert!(p2.get("发", "给").is_none());
        assert!(p2.get("再", "见").is_some(), "3 × 0.5^6.3 ≈ 0.037，还在门槛上");
        // 内存里的没动
        assert_eq!(words.len(), 4);
        assert_eq!(pairs.len(), 2);
    }

    #[test]
    fn nothing_is_dropped_when_all_is_recent() {
        // 没碰上淘汰的条件：存出来的和以前一模一样
        let now = 400 * DAY_MS;
        let mut words = HashMap::new();
        words.insert("种地".to_string(), entry(3, now));
        words.insert("一次".to_string(), entry(1, now - 90 * DAY_MS));
        words.insert("带\t制表符".to_string(), entry(9, now));
        let mut pairs = UserPairs::default();
        pairs.insert("发".into(), "给".into(), entry(1, now - 30 * DAY_MS));
        pairs.insert("发".into(), "个".into(), entry(2, now - 100 * DAY_MS));
        let (_, _, text) = save_and_load("recent", &words, &pairs, now);
        let a = now - 90 * DAY_MS;
        let b = now - 100 * DAY_MS;
        let c = now - 30 * DAY_MS;
        assert_eq!(
            text,
            format!(
                "# wowtype 用户输入习惯 v1\nw\t一次\t1\t{a}\nw\t种地\t3\t{now}\n\
                 p\t发\t个\t2\t{b}\np\t发\t给\t1\t{c}\n"
            )
        );
    }

    #[test]
    fn over_the_cap_keeps_the_hottest() {
        let now = 400 * DAY_MS;
        let items = vec![
            ("常用", entry(8, now - 10 * DAY_MS)),
            // 很久以前用过很多次：8 × 0.5^(120/30) = 0.5，比不上最近的两次
            ("以前常用", entry(8, now - 120 * DAY_MS)),
            ("最近两次", entry(2, now - DAY_MS)),
            ("最近一次", entry(1, now)),
            ("一次不久", entry(1, now - 60 * DAY_MS)),
            // 只用过一次、100 天前：最冷，超上限时最先丢
            ("一次很久", entry(1, now - 100 * DAY_MS)),
        ];
        let names = |kept: Vec<(&'static str, HistoryEntry)>| {
            let mut names: Vec<_> = kept.into_iter().map(|(k, _)| k).collect();
            names.sort_unstable();
            names
        };
        // 超了一条：丢最冷的那一条
        let kept = names(kept_for_save(items.clone(), now, 5));
        assert_eq!(
            kept,
            ["一次不久", "以前常用", "常用", "最近一次", "最近两次"]
        );
        // 超了：留最热的三条（8×0.79=6.3、2×0.98=1.95、1），顺序和算热度的时刻无关
        for at in [now, now - 30 * DAY_MS, 0] {
            let mut input = items.clone();
            input.retain(|(k, _)| *k != "一次很久");
            assert_eq!(
                names(kept_for_save(input, at, 3)),
                ["常用", "最近一次", "最近两次"]
            );
        }
        // 一样热的留后用的
        let ties = vec![
            ("甲", entry(2, 5)),
            ("乙", entry(2, 5)),
            ("丙", entry(2, 6)),
        ];
        assert_eq!(names(kept_for_save(ties, 6, 2)), ["丙", "乙"]);
    }

    #[test]
    fn file_stops_growing_at_the_caps() {
        // 都用过两次（不按「只用过一次」丢），越靠后的越新
        let mut words = HashMap::new();
        for i in 0..MAX_WORDS + 10 {
            words.insert(format!("词{i}"), entry(2, 1000 + i as u64));
        }
        let mut pairs = UserPairs::default();
        for i in 0..MAX_PAIRS + 10 {
            pairs.insert(
                format!("前{}", i % 100),
                format!("后{i}"),
                entry(2, 1000 + i as u64),
            );
        }
        let now = 1000 + (MAX_PAIRS + 10) as u64;
        let (w2, p2, _) = save_and_load("caps", &words, &pairs, now);
        assert_eq!(w2.len(), MAX_WORDS);
        assert_eq!(p2.len(), MAX_PAIRS);
        // 丢的是最旧的那 10 个
        for i in 0..10 {
            assert!(!w2.contains_key(&format!("词{i}")));
            assert!(p2
                .get(&format!("前{}", i % 100), &format!("后{i}"))
                .is_none());
        }
        assert!(w2.contains_key("词10"));
        assert!(p2.get("前10", "后10").is_some());
    }
}
