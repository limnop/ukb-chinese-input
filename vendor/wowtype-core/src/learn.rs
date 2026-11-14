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

use std::collections::HashMap;
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

/// 按半衰期衰减后的次数。
pub fn decayed(entry: HistoryEntry, at_ms: u64) -> f64 {
    let age = at_ms.saturating_sub(entry.last_used_ms) as f64;
    f64::from(entry.count) * 0.5f64.powf(age / HALF_LIFE_MS)
}

/// 用户学到的词搭配：前一个词 -> 后一个词 -> 次数与上次使用时间。
#[derive(Default, Debug, Clone)]
pub struct UserPairs {
    map: HashMap<String, HashMap<String, HistoryEntry>>,
}

impl UserPairs {
    pub fn learn(&mut self, prev: &str, cur: &str, at_ms: u64) {
        if prev.is_empty() || cur.is_empty() {
            return;
        }
        let entry = self
            .map
            .entry(prev.to_string())
            .or_default()
            .entry(cur.to_string())
            .or_insert(HistoryEntry {
                count: 0,
                last_used_ms: at_ms,
            });
        // 次数先按时间衰减再加 1，这样很久以前用过很多次的搭配不会一直顶着
        let kept = decayed(*entry, at_ms).round() as u32;
        entry.count = kept.saturating_add(1);
        entry.last_used_ms = at_ms;
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
}

impl LanguageModel for LearnedLm<'_> {
    fn score(&self, prev: Option<&str>, cur: &str) -> f64 {
        let base = self.base.score(prev, cur);
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

/// 存盘格式（TSV，`#` 开头是注释，各列用 tab 分隔）：
///
/// ```text
/// w <词> <次数> <上次使用，epoch 毫秒>
/// p <前一个词> <后一个词> <次数> <上次使用>
/// ```
pub fn save_history<P: AsRef<Path>>(
    path: P,
    words: &HashMap<String, HistoryEntry>,
    pairs: &UserPairs,
) -> Result<()> {
    let path = path.as_ref();
    let mut out = String::from("# wowtype 用户输入习惯 v1\n");
    let clean = |s: &str| !s.contains(['\t', '\n', '\r']);
    let mut words: Vec<_> = words.iter().filter(|(w, _)| clean(w)).collect();
    words.sort_by(|a, b| a.0.cmp(b.0));
    for (word, entry) in words {
        out.push_str(&format!(
            "w\t{word}\t{}\t{}\n",
            entry.count, entry.last_used_ms
        ));
    }
    let mut pairs: Vec<_> = pairs
        .iter()
        .filter(|(p, c, _)| clean(p) && clean(c))
        .collect();
    pairs.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
    for (prev, cur, entry) in pairs {
        out.push_str(&format!(
            "p\t{prev}\t{cur}\t{}\t{}\n",
            entry.count, entry.last_used_ms
        ));
    }
    // 先写临时文件再改名，写到一半断电也不会把旧文件弄坏
    let tmp = path.with_extension("tsv.tmp");
    std::fs::write(&tmp, out).with_context(|| format!("写入 {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("改名为 {}", path.display()))?;
    Ok(())
}

/// 读回 [`save_history`] 写的文件，返回读到的条数。坏行跳过。
pub fn load_history<P: AsRef<Path>>(
    path: P,
    words: &mut HashMap<String, HistoryEntry>,
    pairs: &mut UserPairs,
) -> Result<usize> {
    let path = path.as_ref();
    let text = std::fs::read_to_string(path).with_context(|| format!("读取 {}", path.display()))?;
    let mut loaded = 0usize;
    for line in text.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split('\t').collect();
        let entry = |count: &str, last: &str| -> Option<HistoryEntry> {
            Some(HistoryEntry {
                count: count.parse().ok()?,
                last_used_ms: last.parse().ok()?,
            })
        };
        match parts.as_slice() {
            ["w", word, count, last] if !word.is_empty() => {
                if let Some(e) = entry(count, last) {
                    words.insert((*word).to_string(), e);
                    loaded += 1;
                }
            }
            ["p", prev, cur, count, last] if !prev.is_empty() && !cur.is_empty() => {
                if let Some(e) = entry(count, last) {
                    pairs.insert((*prev).to_string(), (*cur).to_string(), e);
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

    #[test]
    fn heat_decays_with_half_life() {
        let e = HistoryEntry {
            count: 4,
            last_used_ms: 0,
        };
        assert!((decayed(e, 0) - 4.0).abs() < 1e-9);
        assert!((decayed(e, HALF_LIFE_MS as u64) - 2.0).abs() < 1e-6);
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
                count: 3,
                last_used_ms: 42,
            },
        );
        let mut pairs = UserPairs::default();
        pairs.learn("发", "给", 7);
        save_history(&path, &words, &pairs).unwrap();

        let mut w2 = HashMap::new();
        let mut p2 = UserPairs::default();
        assert_eq!(load_history(&path, &mut w2, &mut p2).unwrap(), 2);
        assert_eq!(w2["种地"].count, 3);
        assert_eq!(p2.get("发", "给").unwrap().count, 1);
        std::fs::remove_dir_all(&dir).ok();
    }
}
