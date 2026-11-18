//! 五笔 86 码表：`wubi86.tsv`，每行 `码<TAB>词<TAB>频次`（`scripts/make-wubi.sh` 从 rime-wubi 生成）。
//!
//! - 按码查（[`WubiTable::lookup`]）：先是码正好对上的词，按码表的频次排（不调频，码位固定）；
//!   再是以它开头的更长的码的词（逐码提示），同一个词只出一次，带着它最短的码。
//! - 反查（[`WubiTable::hint`]）：字、词 → 码，z 键临时拼音的候选后面显示，用来学码。
//!   码表里没有的词组按造词规则从字的全码算（[`WubiTable::phrase_code`]）。
//!
//! 九万多条，按码排好放在一个数组里：精确码和前缀都是一段连续的区间，二分找到；
//! 词放在一整块字符串里，反查用另一个按词排的下标数组二分。

use std::path::Path;

use anyhow::{bail, Result};

/// 五笔码最多几码。
pub const MAX_CODE: usize = 4;

/// 一个码：字母 a–y，不满 4 码的后面补 0，按字节比就是码的字典序（`a` < `aa` < `ab`）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
struct Code([u8; MAX_CODE]);

impl Code {
    /// 1–4 个 a–y 的字母；别的（含 z：五笔不用 z）是 None。
    fn parse(text: &str) -> Option<Self> {
        let bytes = text.as_bytes();
        if bytes.is_empty() || bytes.len() > MAX_CODE || !bytes.iter().all(|b| (b'a'..=b'y').contains(b)) {
            return None;
        }
        let mut code = [0; MAX_CODE];
        code[..bytes.len()].copy_from_slice(bytes);
        Some(Self(code))
    }

    fn len(&self) -> usize {
        self.0.iter().position(|&b| b == 0).unwrap_or(MAX_CODE)
    }

    fn as_str(&self) -> &str {
        // 只会装 a–y
        std::str::from_utf8(&self.0[..self.len()]).unwrap_or("")
    }

    fn starts_with(&self, prefix: &Code) -> bool {
        let n = prefix.len();
        self.0[..n] == prefix.0[..n]
    }
}

#[derive(Clone, Copy, Debug)]
struct Entry {
    code: Code,
    /// 频次（码表里最大约 42.5 亿，u32 放得下；更大的按 u32::MAX 算）。
    weight: u32,
    /// 词在 [`WubiTable::words`] 里的起点和字节数。
    start: u32,
    len: u32,
}

/// 查出来的一个词。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WubiHit<'a> {
    pub word: &'a str,
    /// 这个词的码：精确匹配时就是打的码，前缀联想时是以打的码开头的、这个词最短的码。
    pub code: &'a str,
}

/// 五笔码表。
#[derive(Debug, Default)]
pub struct WubiTable {
    /// 按码升序、同码按频次降序（同频次照文件里的顺序）。
    entries: Vec<Entry>,
    words: String,
    /// entries 的下标，按词排（同一个词的几个码挨在一起，码升序）。
    by_word: Vec<u32>,
}

impl WubiTable {
    /// 读 `码<TAB>词<TAB>频次`。空行、`#` 开头的行跳过；码不是 1–4 个 a–y、没有词、频次认不出的行也跳过。
    /// 一条都没有时报错（文件不对）。
    pub fn from_tsv_str(text: &str) -> Result<Self> {
        let mut table = Self::default();
        for line in text.lines() {
            let line = line.trim_end_matches('\r');
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut fields = line.split('\t');
            let (Some(code), Some(word), Some(weight)) = (fields.next(), fields.next(), fields.next()) else {
                continue;
            };
            let (Some(code), Ok(weight)) = (Code::parse(code), weight.trim().parse::<u64>()) else {
                continue;
            };
            if word.is_empty() {
                continue;
            }
            table.entries.push(Entry {
                code,
                weight: u32::try_from(weight).unwrap_or(u32::MAX),
                start: table.words.len() as u32,
                len: word.len() as u32,
            });
            table.words.push_str(word);
        }
        if table.entries.is_empty() {
            bail!("五笔码表是空的（不是 码<TAB>词<TAB>频次 的格式？）");
        }
        // 文件本来就是排好的，稳定排序在排好的数组上只走一遍
        table
            .entries
            .sort_by(|a, b| a.code.cmp(&b.code).then(b.weight.cmp(&a.weight)));
        let mut by_word: Vec<u32> = (0..table.entries.len() as u32).collect();
        by_word.sort_by(|&a, &b| table.word(a as usize).cmp(table.word(b as usize)).then(a.cmp(&b)));
        table.by_word = by_word;
        Ok(table)
    }

    pub fn load_tsv<P: AsRef<Path>>(path: P) -> Result<Self> {
        Self::from_tsv_str(&std::fs::read_to_string(path)?)
    }

    /// 条数。
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 大致占多少内存（字节，压测看）。
    pub fn heap_bytes(&self) -> usize {
        self.entries.capacity() * std::mem::size_of::<Entry>()
            + self.words.capacity()
            + self.by_word.capacity() * std::mem::size_of::<u32>()
    }

    fn word(&self, index: usize) -> &str {
        let e = &self.entries[index];
        &self.words[e.start as usize..(e.start + e.len) as usize]
    }

    /// 码以 prefix 开头的条目在 entries 里的区间。
    fn range(&self, prefix: &Code) -> std::ops::Range<usize> {
        let start = self.entries.partition_point(|e| e.code < *prefix);
        let len = self.entries[start..].partition_point(|e| e.code.starts_with(prefix));
        start..start + len
    }

    /// 打了 code 时的候选：码正好对上的全部（码表的顺序），再加至多 more 个更长的码的词
    /// （频次高的在前；精确候选里有的、重复的不要）。code 不是五笔码（空的、超过 4 码、有 a–y 以外的字母）时为空。
    pub fn lookup(&self, code: &str, more: usize) -> Vec<WubiHit<'_>> {
        let Some(prefix) = Code::parse(code) else {
            return Vec::new();
        };
        let range = self.range(&prefix);
        let exact = self.entries[range.clone()].partition_point(|e| e.code == prefix);
        let mut hits: Vec<WubiHit<'_>> = (range.start..range.start + exact)
            .map(|i| WubiHit {
                word: self.word(i),
                code: self.entries[i].code.as_str(),
            })
            .collect();
        if more == 0 {
            return hits;
        }
        let mut longer: Vec<usize> = (range.start + exact..range.end).collect();
        longer.sort_by(|&a, &b| {
            let (x, y) = (&self.entries[a], &self.entries[b]);
            y.weight.cmp(&x.weight).then(x.code.cmp(&y.code))
        });
        let mut added = 0;
        for i in longer {
            if added == more {
                break;
            }
            let word = self.word(i);
            if hits.iter().any(|h| h.word == word) {
                continue;
            }
            hits.push(WubiHit {
                word,
                code: self.shortest_code(word, &prefix).unwrap_or(self.entries[i].code.as_str()),
            });
            added += 1;
        }
        hits
    }

    /// word 的码在 entries 里的下标（码升序）。
    fn codes_of<'a>(&'a self, word: &'a str) -> impl Iterator<Item = usize> + 'a {
        let start = self.by_word.partition_point(|&i| self.word(i as usize) < word);
        self.by_word[start..]
            .iter()
            .map(|&i| i as usize)
            .take_while(move |&i| self.word(i) == word)
    }

    /// word 以 prefix 开头的码里最短的。
    fn shortest_code(&self, word: &str, prefix: &Code) -> Option<&str> {
        self.codes_of(word)
            .map(|i| &self.entries[i].code)
            .filter(|code| code.starts_with(prefix))
            .min_by_key(|code| code.len())
            .map(Code::as_str)
    }

    /// 一个字（词）在码表里的全码：最长的码，一样长的取频次高的。
    pub fn full_code(&self, word: &str) -> Option<&str> {
        self.codes_of(word)
            .map(|i| &self.entries[i])
            .max_by(|a, b| a.code.len().cmp(&b.code.len()).then(a.weight.cmp(&b.weight)))
            .map(|e| e.code.as_str())
    }

    /// 按造词规则算词的码（和 Rime 五笔的 encoder.rules 一样），用各字的全码：
    /// 二字 AaAbBaBb、三字 AaBaCaCb、四字及以上 AaBaCaZa。有字查不到码、码不够长时是 None。
    pub fn phrase_code(&self, word: &str) -> Option<String> {
        let chars: Vec<String> = word.chars().map(String::from).collect();
        let codes: Vec<&[u8]> = chars
            .iter()
            .map(|c| self.full_code(c).map(str::as_bytes))
            .collect::<Option<_>>()?;
        // (第几个字, 取它全码的第几码)
        let picks: [(usize, usize); 4] = match codes.len() {
            0 | 1 => return None,
            2 => [(0, 0), (0, 1), (1, 0), (1, 1)],
            3 => [(0, 0), (1, 0), (2, 0), (2, 1)],
            n => [(0, 0), (1, 0), (2, 0), (n - 1, 0)],
        };
        picks
            .iter()
            .map(|&(c, k)| codes[c].get(k).map(|&b| char::from(b)))
            .collect()
    }

    /// 编码提示（反查）：有简码时是「简码/全码」（工 → `a/aaaa`），只有一个码时就是它；
    /// 码表里没有的词组按造词规则算。查不到是 None。
    pub fn hint(&self, word: &str) -> Option<String> {
        let shortest = self
            .codes_of(word)
            .map(|i| &self.entries[i].code)
            .min_by_key(|code| code.len());
        match (shortest, self.full_code(word)) {
            (Some(short), Some(full)) if short.as_str() != full => Some(format!("{}/{full}", short.as_str())),
            (_, Some(full)) => Some(full.to_owned()),
            _ => self.phrase_code(word),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 节选自 rime-wubi 的 wubi86（LGPL-3.0），够测排序、前缀、反查和造词规则
    const TABLE: &str = "\
a\t工\t99454797
aa\t式\t71453053
aaa\t工\t551000000
aaaa\t工\t551000000
aaaa\t恭恭敬敬\t672000
g\t一\t2015124793
ggl\t一\t2830000000
ggll\t一\t2830000000
wq\t你\t597727619
wqi\t你\t1490000000
wqiy\t你\t1490000000
wqvb\t你好\t70500000
wqvb\t您好\t34700000
vb\t好\t300000000
vbg\t好\t400000000
vbg\t妇\t100000000
vbgy\t好\t400000000
k\t中\t563673763
khk\t中\t600000000
khkh\t中\t600000000
l\t国\t410863159
lgyi\t国\t500000000
khlg\t中国\t200000000
j\t是\t1632303407
";

    fn table() -> WubiTable {
        WubiTable::from_tsv_str(TABLE).unwrap()
    }

    fn words<'a>(hits: &[WubiHit<'a>]) -> Vec<(&'a str, &'a str)> {
        hits.iter().map(|h| (h.word, h.code)).collect()
    }

    #[test]
    fn exact_codes_keep_table_order() {
        let t = table();
        assert_eq!(words(&t.lookup("wqvb", 0)), [("你好", "wqvb"), ("您好", "wqvb")]);
        assert_eq!(words(&t.lookup("g", 0)), [("一", "g")]);
        assert!(t.lookup("zz", 10).is_empty(), "z 不是五笔码");
        assert!(t.lookup("aaaaa", 10).is_empty(), "最多四码");
        assert!(t.lookup("", 10).is_empty());
        assert!(t.lookup("xy", 10).is_empty(), "空码");
    }

    #[test]
    fn longer_codes_follow_with_their_shortest_code() {
        let t = table();
        // 你 在 wq 是精确码；后面按频次是你好、您好（你 不重复出现）
        assert_eq!(words(&t.lookup("wq", 10)), [("你", "wq"), ("你好", "wqvb"), ("您好", "wqvb")]);
        // 只打 w：你 带最短的码 wq（虽然 wqi、wqiy 的频次更高）
        assert_eq!(words(&t.lookup("w", 1)), [("你", "wq")]);
        // 同一个词在前缀下的几个码只出一次
        let a = t.lookup("a", 10);
        assert_eq!(words(&a), [("工", "a"), ("式", "aa"), ("恭恭敬敬", "aaaa")]);
    }

    #[test]
    fn reverse_lookup_and_phrase_rules() {
        let t = table();
        assert_eq!(t.full_code("工"), Some("aaaa"));
        assert_eq!(t.hint("工").as_deref(), Some("a/aaaa"));
        assert_eq!(t.hint("你好").as_deref(), Some("wqvb"));
        assert_eq!(t.hint("中国").as_deref(), Some("khlg"));
        assert_eq!(t.hint("没有").as_deref(), None);
        // 码表里没有的词组按规则算：二字 AaAbBaBb
        assert_eq!(t.phrase_code("国中").as_deref(), Some("lgkh"));
        // 三字 AaBaCaCb
        assert_eq!(t.phrase_code("中国好").as_deref(), Some("klvb"));
        // 四字及以上 AaBaCaZa
        assert_eq!(t.phrase_code("中国好是一").as_deref(), Some("klvg"));
        assert_eq!(t.phrase_code("中"), None, "单字不用造词");
        assert_eq!(t.phrase_code("中没"), None, "有字查不到");
    }

    #[test]
    fn bad_lines_are_skipped_and_empty_is_an_error() {
        let t = WubiTable::from_tsv_str("# 注释\n\nzz\t符\t1\nabcde\t长\t1\nab\t\t1\nab\t好\tx\nab\t对\t5\r\n").unwrap();
        assert_eq!(t.len(), 1);
        assert_eq!(words(&t.lookup("ab", 0)), [("对", "ab")]);
        assert!(WubiTable::from_tsv_str("# 只有注释\n").is_err());
    }
}
