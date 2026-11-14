//! Emoji：打字时推荐，以及表情面板里的搜索。
//!
//! 数据编进库里：`emoji/cldr.tsv`（Unicode CLDR 的中英文关键词，scripts/make-emoji.py 生成）
//! 和 `emoji/zh-extra.tsv`（手工整理的聊天常用词，优先）。
//!
//! - `recommend`：根据首选候选的文本找 emoji。整个词对上了，emoji 替换这个词；
//!   句子以某个词结尾（「今天好开心」），emoji 接在整句后面。只看两个字以上、
//!   够具体的词（「脸」「手」这种几十个 emoji 都有的关键词不推荐）。
//! - `search_words` / `search_english`：表情面板里按中文词、英文前缀找。

use std::collections::HashMap;
use std::sync::OnceLock;

const CLDR: &str = include_str!("emoji/cldr.tsv");
const EXTRA: &str = include_str!("emoji/zh-extra.tsv");

/// 一个关键词对应超过这么多 emoji 就算太泛，不拿来做打字推荐。
const SPECIFIC_MAX: usize = 3;

struct Entry {
    emoji: &'static str,
    zh: Vec<&'static str>,
    en: Vec<&'static str>,
}

struct Table {
    entries: Vec<Entry>,
    /// 中文关键词 -> emoji 下标，按「关键词在这个 emoji 里排得越前越贴切」排好。
    by_zh: HashMap<&'static str, Vec<usize>>,
    /// 手工整理的词 -> emoji。
    extra: HashMap<&'static str, Vec<&'static str>>,
}

fn table() -> &'static Table {
    static TABLE: OnceLock<Table> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut entries = Vec::new();
        let mut by_zh: HashMap<&'static str, Vec<(usize, usize)>> = HashMap::new();
        for line in CLDR.lines() {
            if line.starts_with('#') || line.is_empty() {
                continue;
            }
            let mut cols = line.split('\t');
            let (Some(emoji), Some(zh), Some(en)) = (cols.next(), cols.next(), cols.next()) else {
                continue;
            };
            let split = |s: &'static str| -> Vec<&'static str> {
                s.split('|').filter(|w| !w.is_empty()).collect()
            };
            let index = entries.len();
            let zh = split(zh);
            for (pos, word) in zh.iter().enumerate() {
                by_zh.entry(word).or_default().push((pos, index));
            }
            entries.push(Entry {
                emoji,
                zh,
                en: split(en),
            });
        }
        let by_zh = by_zh
            .into_iter()
            .map(|(word, mut hits)| {
                hits.sort();
                (word, hits.into_iter().map(|(_, index)| index).collect())
            })
            .collect();
        let mut extra = HashMap::new();
        for line in EXTRA.lines() {
            if line.starts_with('#') || line.is_empty() {
                continue;
            }
            if let Some((word, emojis)) = line.split_once('\t') {
                extra.insert(word.trim(), emojis.split_whitespace().collect());
            }
        }
        Table {
            entries,
            by_zh,
            extra,
        }
    })
}

/// 全部 emoji，按 Unicode 的顺序（笑脸在前）。
pub fn all() -> Vec<&'static str> {
    table().entries.iter().map(|e| e.emoji).collect()
}

/// 打字推荐的一个结果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggestion {
    pub emoji: &'static str,
    /// 候选列表里显示的文字。
    pub display: String,
    /// 选中后上屏的文字：整个词对上时只有 emoji，句尾对上时是整句 + emoji。
    pub commit: String,
}

/// 这个词拿来做打字推荐时对应的 emoji（够具体才算）。
fn specific(word: &str) -> Vec<&'static str> {
    if word.chars().count() < 2 {
        return Vec::new();
    }
    let table = table();
    if let Some(list) = table.extra.get(word) {
        return list.clone();
    }
    match table.by_zh.get(word) {
        Some(hits) if hits.len() <= SPECIFIC_MAX => {
            hits.iter().map(|&i| table.entries[i].emoji).collect()
        }
        _ => Vec::new(),
    }
}

/// 数字字和按键 emoji：打 ba 时「八」后面跟一个 8️⃣（手机输入法都这样）。
const KEYCAPS: [(&str, &std::ffi::CStr); 12] = [
    ("零", c"0️⃣"),
    ("〇", c"0️⃣"),
    ("一", c"1️⃣"),
    ("二", c"2️⃣"),
    ("三", c"3️⃣"),
    ("四", c"4️⃣"),
    ("五", c"5️⃣"),
    ("六", c"6️⃣"),
    ("七", c"7️⃣"),
    ("八", c"8️⃣"),
    ("九", c"9️⃣"),
    ("十", c"🔟"),
];

/// 候选正好是一个数字字（零到十）时对应的按键 emoji，别的给 None。
pub fn keycap_c(text: &str) -> Option<&'static std::ffi::CStr> {
    let text = text.trim();
    KEYCAPS
        .iter()
        .find(|(word, _)| *word == text)
        .map(|(_, emoji)| *emoji)
}

/// 同 [`keycap_c`]，给 Rust 用。
pub fn keycap(text: &str) -> Option<&'static str> {
    keycap_c(text).and_then(|emoji| emoji.to_str().ok())
}

/// 根据首选候选的文本推荐 emoji，最多 limit 个。
pub fn recommend(text: &str, limit: usize) -> Vec<Suggestion> {
    let text = text.trim();
    if text.is_empty() || limit == 0 || text.chars().count() > 40 {
        return Vec::new();
    }
    let exact = specific(text);
    if !exact.is_empty() {
        return exact
            .into_iter()
            .take(limit)
            .map(|emoji| Suggestion {
                emoji,
                display: emoji.to_string(),
                commit: emoji.to_string(),
            })
            .collect();
    }
    // 句尾的词，越长越具体：「今天好开心」先试「天好开心」……最后「开心」
    let starts: Vec<usize> = text.char_indices().map(|(i, _)| i).skip(1).collect();
    for start in starts {
        let tail = &text[start..];
        let hits = specific(tail);
        if !hits.is_empty() {
            return hits
                .into_iter()
                .take(limit)
                .map(|emoji| Suggestion {
                    emoji,
                    display: format!("{tail}{emoji}"),
                    commit: format!("{text}{emoji}"),
                })
                .collect();
        }
    }
    Vec::new()
}

/// 表情面板：按中文词找（词按候选顺序给），关键词完全相同的在前，包含这个词的在后。
pub fn search_words(words: &[&str], limit: usize) -> Vec<&'static str> {
    let mut out = search_words_exact(words, limit);
    for emoji in search_words_containing(words, limit) {
        if !out.contains(&emoji) {
            out.push(emoji);
        }
    }
    out.truncate(limit);
    out
}

/// 关键词和某个词完全相同的 emoji，按词的顺序。
pub fn search_words_exact(words: &[&str], limit: usize) -> Vec<&'static str> {
    let table = table();
    let mut out: Vec<&'static str> = Vec::new();
    for word in words {
        let word = word.trim();
        let extra = table.extra.get(word).into_iter().flatten().copied();
        let cldr = table
            .by_zh
            .get(word)
            .into_iter()
            .flatten()
            .map(|&i| table.entries[i].emoji);
        for emoji in extra.chain(cldr) {
            if !out.contains(&emoji) {
                out.push(emoji);
            }
        }
    }
    out.truncate(limit);
    out
}

/// 关键词里包含某个词的 emoji（「笑」-> 笑哭了、大笑……），按词的顺序。
pub fn search_words_containing(words: &[&str], limit: usize) -> Vec<&'static str> {
    let table = table();
    let mut out: Vec<&'static str> = Vec::new();
    for word in words {
        let word = word.trim();
        if word.is_empty() {
            continue;
        }
        for entry in &table.entries {
            if entry.zh.iter().any(|k| k.contains(word)) && !out.contains(&entry.emoji) {
                out.push(entry.emoji);
            }
        }
    }
    out.truncate(limit);
    out
}

/// 表情面板：按英文找。某个英文关键词里有单词完全等于 query 的排前面，以 query 开头的在后。
pub fn search_english(query: &str, limit: usize) -> Vec<&'static str> {
    let q = query.trim().to_ascii_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    let table = table();
    let mut exact = Vec::new();
    let mut prefix = Vec::new();
    for entry in &table.entries {
        let mut best: Option<u8> = None;
        for (pos, keyword) in entry.en.iter().enumerate() {
            for token in keyword.split(|c: char| !c.is_ascii_alphanumeric()) {
                let rank = if token == q {
                    if pos == 0 {
                        0
                    } else {
                        1
                    }
                } else if token.starts_with(&q) {
                    2
                } else {
                    continue;
                };
                best = Some(best.map_or(rank, |b: u8| b.min(rank)));
            }
        }
        match best {
            Some(rank) if rank <= 1 => exact.push((rank, entry.emoji)),
            Some(_) => prefix.push(entry.emoji),
            None => {}
        }
    }
    exact.sort_by_key(|(rank, _)| *rank);
    let mut out: Vec<&'static str> = exact.into_iter().map(|(_, e)| e).collect();
    out.extend(prefix);
    out.truncate(limit);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keycaps() {
        assert_eq!(keycap("八"), Some("8️⃣"));
        assert_eq!(keycap("零"), Some("0️⃣"));
        assert_eq!(keycap("〇"), Some("0️⃣"));
        assert_eq!(keycap("十"), Some("🔟"));
        assert_eq!(keycap("吧"), None);
        assert_eq!(keycap("八十"), None);
        assert_eq!(keycap(""), None);
        // 每个都在 emoji 表里
        let all = all();
        for (_, emoji) in KEYCAPS {
            assert!(all.contains(&emoji.to_str().unwrap()), "{emoji:?}");
        }
    }

    #[test]
    fn table_loads() {
        let all = all();
        assert!(all.len() > 1500);
        assert_eq!(all[0], "😀");
    }

    #[test]
    fn recommends_whole_word() {
        let hits = recommend("哈哈", 2);
        assert_eq!(hits[0].emoji, "😄");
        assert_eq!(hits[0].commit, "😄");
        assert_eq!(recommend("点赞", 1)[0].emoji, "👍");
        // CLDR 里的具体词
        assert_eq!(recommend("西瓜", 1)[0].emoji, "🍉");
    }

    #[test]
    fn recommends_sentence_tail() {
        let hits = recommend("今天好开心", 1);
        assert_eq!(hits[0].emoji, "😊");
        assert_eq!(hits[0].display, "开心😊");
        assert_eq!(hits[0].commit, "今天好开心😊");
    }

    #[test]
    fn skips_single_chars_and_vague_words() {
        assert!(recommend("好", 2).is_empty());
        assert!(recommend("是", 2).is_empty());
        // 「水果」对应一大堆，不推荐
        assert!(recommend("水果", 2).is_empty());
        assert!(recommend("我们去开会吧", 2).is_empty());
    }

    #[test]
    fn searches_chinese_words() {
        let hits = search_words(&["笑"], 50);
        assert!(hits.contains(&"😂"));
        assert!(hits.contains(&"😄"));
        let hits = search_words(&["狗"], 20);
        assert!(hits.contains(&"🐶"));
        assert!(search_words(&["中国"], 5).contains(&"🇨🇳"));
    }

    #[test]
    fn searches_english() {
        let hits = search_english("smile", 50);
        assert!(hits.contains(&"😄"));
        let hits = search_english("thumb", 10);
        assert!(hits.contains(&"👍"));
        assert_eq!(search_english("pizza", 3)[0], "🍕");
        assert!(search_english("", 3).is_empty());
    }
}
