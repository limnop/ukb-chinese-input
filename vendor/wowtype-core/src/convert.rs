//! 简繁转换（字符级 + 词组级）。
//!
//! 映射表是「简体<TAB>繁体」的 TSV，可由 `tools/opencc2tsv.py` 从 OpenCC 数据生成。
//! 左边是单字、右边也是单字的行是字符映射；左边多于一个字的行是词组映射
//! （例如 `头发<TAB>頭髮`、`软件<TAB>軟體`），右边可以和左边不等长。
//! 转换时从左到右做最长匹配：先找以当前位置开头的最长词组，找不到再逐字映射，
//! 这样「发/髮」「干/乾/幹」这类一对多的字可以靠词组消歧。

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use anyhow::{bail, Context, Result};

/// 简繁映射表。
#[derive(Clone, Debug, Default)]
pub struct Converter {
    chars: HashMap<char, char>,
    phrases: HashMap<String, String>,
    /// 最长词组的字数（0 表示没有词组）。
    max_phrase_chars: usize,
}

impl Converter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.chars.is_empty() && self.phrases.is_empty()
    }

    pub fn len(&self) -> usize {
        self.chars.len() + self.phrases.len()
    }

    /// 词组映射的条数。
    pub fn phrase_len(&self) -> usize {
        self.phrases.len()
    }

    /// 从 TSV 文本加载：简体<TAB>繁体。以 # 开头的是注释。
    ///
    /// 两边都是单字的行进字符表，其余进词组表。
    pub fn from_tsv_str(data: &str) -> Result<Self> {
        let mut conv = Self::default();
        for (line_no, raw) in data.lines().enumerate() {
            let line = raw.trim_end();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut cols = line.split('\t');
            let (Some(from), Some(to)) = (cols.next(), cols.next()) else {
                bail!("第 {} 行应为两列：简体<TAB>繁体", line_no + 1);
            };
            if from.is_empty() {
                bail!("第 {} 行左侧为空", line_no + 1);
            }
            if to.is_empty() {
                bail!("第 {} 行右侧为空", line_no + 1);
            }
            let from_chars = from.chars().count();
            let mut to_iter = to.chars();
            match (from_chars, to_iter.next(), to_iter.next()) {
                (1, Some(t), None) => {
                    let f = from.chars().next().expect("非空");
                    conv.chars.insert(f, t);
                }
                _ => {
                    conv.max_phrase_chars = conv.max_phrase_chars.max(from_chars);
                    conv.phrases.insert(from.to_string(), to.to_string());
                }
            }
        }
        if conv.is_empty() {
            bail!("映射表为空");
        }
        Ok(conv)
    }

    pub fn from_tsv_path<P: AsRef<Path>>(path: P) -> Result<Self> {
        let data = fs::read_to_string(path.as_ref())
            .with_context(|| format!("读取简繁表失败: {}", path.as_ref().display()))?;
        Self::from_tsv_str(&data)
    }

    /// 转换：词组最长匹配优先，其余逐字转换，未收录的字符原样保留。
    pub fn convert(&self, text: &str) -> String {
        if self.is_empty() {
            return text.to_string();
        }
        if self.phrases.is_empty() {
            return text
                .chars()
                .map(|c| self.chars.get(&c).copied().unwrap_or(c))
                .collect();
        }
        // 每个字符的字节起点，末尾补一个 text.len()，方便按字符切片
        let bounds: Vec<usize> = text
            .char_indices()
            .map(|(i, _)| i)
            .chain(std::iter::once(text.len()))
            .collect();
        let n = bounds.len() - 1;
        let mut out = String::with_capacity(text.len() + 8);
        let mut i = 0;
        'outer: while i < n {
            let longest = self.max_phrase_chars.min(n - i);
            for len in (2..=longest).rev() {
                if let Some(to) = self.phrases.get(&text[bounds[i]..bounds[i + len]]) {
                    out.push_str(to);
                    i += len;
                    continue 'outer;
                }
            }
            let c = text[bounds[i]..bounds[i + 1]].chars().next().expect("非空");
            match self.phrases.get(&text[bounds[i]..bounds[i + 1]]) {
                Some(to) => out.push_str(to),
                None => out.push(self.chars.get(&c).copied().unwrap_or(c)),
            }
            i += 1;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_characters() {
        let conv = Converter::from_tsv_str("# 注释\n国\t國\n学\t學\n").unwrap();
        assert_eq!(conv.len(), 2);
        assert_eq!(conv.convert("中国学生"), "中國學生");
        // 未收录字符原样保留
        assert_eq!(conv.convert("abc"), "abc");
    }

    #[test]
    fn empty_table_is_identity() {
        let conv = Converter::new();
        assert!(conv.is_empty());
        assert_eq!(conv.convert("中国"), "中国");
    }

    #[test]
    fn rejects_bad_lines() {
        assert!(Converter::from_tsv_str("只有一列\n").is_err());
        assert!(Converter::from_tsv_str("\n\n").is_err());
        assert!(Converter::from_tsv_str("\t繁\n").is_err());
        assert!(Converter::from_tsv_str("简\t\n").is_err());
    }

    const TABLE: &str = "发\t發\n干\t幹\n头\t頭\n净\t淨\n软\t軟\n件\t件\n只\t只\n猫\t貓\n\
                         头发\t頭髮\n干净\t乾淨\n软件\t軟體\n一只\t一隻\n计算机\t電腦\n计算\t計算\n";

    #[test]
    fn phrases_disambiguate_characters() {
        let conv = Converter::from_tsv_str(TABLE).unwrap();
        assert_eq!(conv.phrase_len(), 6);
        assert_eq!(conv.convert("头发"), "頭髮");
        assert_eq!(conv.convert("发现"), "發现");
        assert_eq!(conv.convert("干净"), "乾淨");
        assert_eq!(conv.convert("干活"), "幹活");
        assert_eq!(conv.convert("一只猫"), "一隻貓");
    }

    #[test]
    fn phrases_can_change_length_and_prefer_longest() {
        let conv = Converter::from_tsv_str(TABLE).unwrap();
        assert_eq!(conv.convert("软件"), "軟體");
        assert_eq!(conv.convert("计算机"), "電腦");
        assert_eq!(conv.convert("计算"), "計算");
        assert_eq!(conv.convert("我的计算机和软件"), "我的電腦和軟體");
    }

    #[test]
    fn mixed_text_keeps_other_characters() {
        let conv = Converter::from_tsv_str(TABLE).unwrap();
        assert_eq!(conv.convert("a头发b🙂干净"), "a頭髮b🙂乾淨");
        assert_eq!(conv.convert(""), "");
    }

    #[test]
    fn single_char_with_multi_char_target_is_a_phrase() {
        let conv = Converter::from_tsv_str("么\t麼\n㐷\t傌\n叁\t參參\n").unwrap();
        assert_eq!(conv.convert("叁么"), "參參麼");
    }

    #[test]
    fn bundled_table_disambiguates_phrases() {
        let conv = Converter::from_tsv_str(include_str!("../data/s2t.tsv")).unwrap();
        assert!(conv.phrase_len() > 0, "core/data/s2t.tsv 没有词组");
        for (s, t) in [
            ("头发", "頭髮"),
            ("干净", "乾淨"),
            ("干部", "幹部"),
            ("皇后", "皇后"),
            ("后来", "後來"),
            ("公里", "公里"),
            ("里面", "裏面"),
            ("一只", "一隻"),
            ("只是", "只是"),
            // 短词组不能吃掉后面词的字：断发 / 发展
            ("不断发展", "不斷發展"),
            ("被发现", "被發現"),
            // 逐字就能转对的词组也要在表里，整句里靠它挡住别的词组（了解 -> 瞭解、制面 -> 製麪）
            ("了解", "瞭解"),
            ("为了解决问题", "爲了解決問題"),
            ("想了解", "想了解"),
            ("控制面板", "控制面板"),
            ("发表", "發表"),
        ] {
            assert_eq!(conv.convert(s), t, "{s}");
        }
    }
}
