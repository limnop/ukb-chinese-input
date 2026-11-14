//! 简繁转换（字符级）。
//!
//! 映射表是「简体<TAB>繁体」的 TSV，可由 `tools/opencc2tsv.py` 从 OpenCC 数据生成。
//! 字符级转换无法消解「发/髮」这类歧义，需要词组级规则时再在此扩展。

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use anyhow::{bail, Context, Result};

/// 字符映射表。
#[derive(Clone, Debug, Default)]
pub struct Converter {
    chars: HashMap<char, char>,
}

impl Converter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.chars.is_empty()
    }

    pub fn len(&self) -> usize {
        self.chars.len()
    }

    /// 从 TSV 文本加载：简体<TAB>繁体，每行一个字符。以 # 开头的是注释。
    pub fn from_tsv_str(data: &str) -> Result<Self> {
        let mut chars = HashMap::new();
        for (line_no, raw) in data.lines().enumerate() {
            let line = raw.trim_end();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut cols = line.split('\t');
            let (Some(from), Some(to)) = (cols.next(), cols.next()) else {
                bail!("第 {} 行应为两列：简体<TAB>繁体", line_no + 1);
            };
            let from: char = from
                .chars()
                .next()
                .with_context(|| format!("第 {} 行左侧为空", line_no + 1))?;
            let to: char = to
                .chars()
                .next()
                .with_context(|| format!("第 {} 行右侧为空", line_no + 1))?;
            chars.insert(from, to);
        }
        if chars.is_empty() {
            bail!("映射表为空");
        }
        Ok(Self { chars })
    }

    pub fn from_tsv_path<P: AsRef<Path>>(path: P) -> Result<Self> {
        let data = fs::read_to_string(path.as_ref())
            .with_context(|| format!("读取简繁表失败: {}", path.as_ref().display()))?;
        Self::from_tsv_str(&data)
    }

    /// 逐字转换，未收录的字符原样保留。
    pub fn convert(&self, text: &str) -> String {
        if self.chars.is_empty() {
            return text.to_string();
        }
        text.chars()
            .map(|c| self.chars.get(&c).copied().unwrap_or(c))
            .collect()
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
    }
}
