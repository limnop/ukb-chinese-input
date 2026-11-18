//! 数据文件（词库、英文词表、五笔码表、简繁表、语言模型）的版本标记。
//!
//! 每个 WowType 发布的数据文件开头带一个机器可读的标记：
//!
//! ```text
//! wowtype-data: <名字> <版本> [来源]
//! ```
//!
//! - 名字：数据文件在系统数据目录里的文件名（`lexicon.tsv`、`words.lm`……），不带空白；
//!   不是装进系统目录的数据（万象词库包）用自己的名字，免得被当成同名文件的另一版。
//! - 版本：点分隔的数字（`2026.11.14.5`，数据日期 + 修订号），逐段按数字比大小，缺的段算 0。
//! - 来源：剩下的整行（可以有空格），给人看，程序不比较。
//!
//! 放在哪：
//!
//! - 文本文件（TSV、英文词表）：头部一行注释 `# wowtype-data: ...`。读取这些文件的代码
//!   （0.4.0 起都一样）把 `#` 开头的行当注释跳过，所以旧插件读新数据也不出错。第一行本来是注释的
//!   （「# 由 tools/rime2tsv.py 生成」等），标记插在第二行，旧插件认旧副本的办法照样管用。
//! - `words.lm`（`WTLM` 版本 1）：词表的 1 号词就是标记本身（0 号是 `<s>`），没有搭配、回退分 0。
//!   格式版本不变，旧插件当成一个永远不会被查到的词，照常读；见 [`crate::wordlm::set_data_marker`]。
//! - 别的二进制文件（如 `nnlm.bin`）：把同样的一串放在文件前 [`SCAN_BYTES`] 字节里，
//!   以 `\n` 或 `\0` 结尾。
//!
//! 找标记只看文件前 [`SCAN_BYTES`] 字节。插件（`fcitx5-addon/src/data_files.h`）、
//! 设置界面（`settings/wowtype-config`）、生成工具（`tools/datamark.py`）各有一份同样规则的实现，
//! 改规则时三处一起改。规则和发布步骤见 `docs/DATA.md` 的「数据版本标记」一节。

use std::cmp::Ordering;
use std::io::Read;
use std::path::Path;

/// 标记的开头。
pub const KEY: &str = "wowtype-data:";
/// 只在文件的前这么多字节里找标记。
pub const SCAN_BYTES: usize = 4096;

/// 加标记以前发布的官方 `words.lm`（没有标记，按字节数认）：(字节数, 版本, 来源)。
/// 以后发布的都带标记，这张表不用再加。
pub const LEGACY_OFFICIAL_LM: &[(u64, &str, &str)] = &[(
    38_455_852,
    "2026.11.09",
    "lm-2026.11.09（sha256 42cb7c4a7cededa29b1a9ff105530085529fefff96535cc051341ebd51605dbe）",
)];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DataMarker {
    pub name: String,
    pub version: String,
    pub source: String,
}

impl DataMarker {
    /// 名字、版本不合规矩时返回 `None`。
    pub fn new(name: &str, version: &str, source: &str) -> Option<Self> {
        if !valid_name(name) || !valid_version(version) || source.contains(['\n', '\r', '\0']) {
            return None;
        }
        Some(Self {
            name: name.to_string(),
            version: version.to_string(),
            source: source.trim().to_string(),
        })
    }

    /// 解析 `wowtype-data: 名字 版本 [来源]`（一行，前面可以有别的字，如 `# `）。
    pub fn parse(text: &str) -> Option<Self> {
        let at = text.find(KEY)?;
        let rest = &text[at + KEY.len()..];
        let rest = rest
            .split(['\n', '\r', '\0'])
            .next()
            .unwrap_or_default();
        if !rest.starts_with(' ') {
            return None;
        }
        let rest = rest.trim_start_matches(' ');
        let (name, rest) = rest.split_once(' ').unwrap_or((rest, ""));
        let rest = rest.trim_start_matches(' ');
        let (version, source) = rest.split_once(' ').unwrap_or((rest, ""));
        Self::new(name, version, source)
    }

    /// `wowtype-data: 名字 版本 来源`（`words.lm` 的 1 号词就是这一串）。
    pub fn body(&self) -> String {
        let mut s = format!("{KEY} {} {}", self.name, self.version);
        if !self.source.is_empty() {
            s.push(' ');
            s.push_str(&self.source);
        }
        s
    }

    /// 文本文件里的那一行（不带换行）。
    pub fn comment_line(&self) -> String {
        format!("# {}", self.body())
    }
}

/// 名字：非空、没有空白和控制字符的 ASCII。
pub fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|b| (0x21..0x7f).contains(&b))
}

/// 版本：点分隔的数字，每段至少一位（`2026.11.14.5`）。
pub fn valid_version(version: &str) -> bool {
    !version.is_empty()
        && version
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
}

/// 逐段按数字比（`2026.11.9` < `2026.11.10`，缺的段算 0：`2026.11.14` = `2026.11.14.0`）。
/// 每段去掉前导 0 后先比位数再比字面，多长的数字都不会溢出。
pub fn compare_versions(a: &str, b: &str) -> Ordering {
    let mut x = a.split('.');
    let mut y = b.split('.');
    loop {
        match (x.next(), y.next()) {
            (None, None) => return Ordering::Equal,
            (p, q) => {
                let p = p.unwrap_or("0").trim_start_matches('0');
                let q = q.unwrap_or("0").trim_start_matches('0');
                let ord = p.len().cmp(&q.len()).then_with(|| p.cmp(q));
                if ord != Ordering::Equal {
                    return ord;
                }
            }
        }
    }
}

/// 从文件开头的字节里找标记：`WTLM` 文件看 1 号词，别的文件在前 [`SCAN_BYTES`] 字节里找
/// `wowtype-data:`，要在这个范围里以换行或 `\0` 结束。
pub fn scan(head: &[u8]) -> Option<DataMarker> {
    let head = &head[..head.len().min(SCAN_BYTES)];
    if head.starts_with(b"WTLM") {
        return wtlm_marker(head);
    }
    let key = KEY.as_bytes();
    let at = head.windows(key.len()).position(|w| w == key)?;
    let end = head[at..]
        .iter()
        .position(|&b| b == b'\n' || b == b'\r' || b == 0)?;
    DataMarker::parse(std::str::from_utf8(&head[at..at + end]).ok()?)
}

/// `WTLM` 版本 1：u32 版本、词数、二元组数之后是词表，0 号 `<s>`，1 号是标记。
fn wtlm_marker(head: &[u8]) -> Option<DataMarker> {
    let u32_at = |at: usize| -> Option<u32> {
        Some(u32::from_le_bytes(head.get(at..at + 4)?.try_into().ok()?))
    };
    if u32_at(4)? != 1 || u32_at(8)? < 2 {
        return None;
    }
    let mut at = 16;
    let len0 = *head.get(at)? as usize;
    at += 1 + len0;
    let len1 = *head.get(at)? as usize;
    let word = head.get(at + 1..at + 1 + len1)?;
    let word = std::str::from_utf8(word).ok()?;
    if !word.starts_with(KEY) {
        return None;
    }
    DataMarker::parse(word)
}

/// 读文件开头找标记；读不了、没有标记返回 `None`。
pub fn read_marker<P: AsRef<Path>>(path: P) -> Option<DataMarker> {
    let mut head = Vec::with_capacity(SCAN_BYTES);
    std::fs::File::open(path)
        .ok()?
        .take(SCAN_BYTES as u64)
        .read_to_end(&mut head)
        .ok()?;
    scan(&head)
}

/// 没有标记的 `words.lm`：字节数是加标记以前哪一版官方发布的，返回 (版本, 来源)。
pub fn legacy_lm_version(size: u64) -> Option<(&'static str, &'static str)> {
    LEGACY_OFFICIAL_LM
        .iter()
        .find(|e| e.0 == size)
        .map(|e| (e.1, e.2))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_format() {
        let m = DataMarker::parse("# wowtype-data: lexicon.tsv 2026.11.14.5 rime-ice abc (2026-11-14)\n")
            .unwrap();
        assert_eq!(m.name, "lexicon.tsv");
        assert_eq!(m.version, "2026.11.14.5");
        assert_eq!(m.source, "rime-ice abc (2026-11-14)");
        assert_eq!(
            m.comment_line(),
            "# wowtype-data: lexicon.tsv 2026.11.14.5 rime-ice abc (2026-11-14)"
        );
        assert_eq!(DataMarker::parse(&m.body()), Some(m));
        let bare = DataMarker::parse("wowtype-data: s2t.tsv 2026.11.14.0").unwrap();
        assert_eq!(bare.source, "");
        assert_eq!(bare.body(), "wowtype-data: s2t.tsv 2026.11.14.0");
        // 版本不是点分隔的数字、少了名字或版本：不算
        assert!(DataMarker::parse("# wowtype-data: lexicon.tsv 2026-11-14.5").is_none());
        assert!(DataMarker::parse("# wowtype-data: lexicon.tsv").is_none());
        assert!(DataMarker::parse("# wowtype-data:lexicon.tsv 1").is_none());
        assert!(DataMarker::parse("# wowtype-data: lexicon.tsv 1..2").is_none());
        assert!(DataMarker::new("a b", "1", "").is_none());
        assert!(DataMarker::new("a", "1", "x\ny").is_none());
    }

    #[test]
    fn version_order() {
        assert_eq!(compare_versions("2026.11.9", "2026.11.10"), Ordering::Less);
        assert_eq!(compare_versions("2026.11.14", "2026.11.14.0"), Ordering::Equal);
        assert_eq!(compare_versions("2026.11.14.1", "2026.11.14"), Ordering::Greater);
        assert_eq!(compare_versions("2026.011.14", "2026.11.14"), Ordering::Equal);
        assert_eq!(
            compare_versions("99999999999999999999999.1", "99999999999999999999998.9"),
            Ordering::Greater
        );
        assert_eq!(compare_versions("2026.11.09", "2026.11.9"), Ordering::Equal);
    }

    #[test]
    fn scan_text_and_binary() {
        let text = "# 由 tools/rime2tsv.py 生成\n# wowtype-data: lexicon.tsv 2026.11.14.5 rime-ice\nni\t你\t1\n";
        assert_eq!(scan(text.as_bytes()).unwrap().version, "2026.11.14.5");
        // 别的二进制格式：前 4 KB 里一串以 \0 结束的标记
        let mut bin = b"WTNN\x01\x00\x00\x00".to_vec();
        bin.extend_from_slice(b"wowtype-data: nnlm.bin 2026.11.20.0 nnlm-2026.11.20\0");
        bin.extend_from_slice(&[0xff; 100]);
        let m = scan(&bin).unwrap();
        assert_eq!((m.name.as_str(), m.source.as_str()), ("nnlm.bin", "nnlm-2026.11.20"));
        // 在 4 KB 以外、没有结尾的不算
        let mut far = vec![b'x'; SCAN_BYTES];
        far.extend_from_slice(b"\n# wowtype-data: lexicon.tsv 1\n");
        assert!(scan(&far).is_none());
        let mut cut = vec![b'x'; SCAN_BYTES - 30];
        cut.extend_from_slice(b"# wowtype-data: lexicon.tsv 2026.11.14.5 xxxxxxxx\n");
        assert!(scan(&cut).is_none());
        assert!(scan(b"ni\t\xe4\xbd\xa0\t1\n").is_none());
    }

    #[test]
    fn legacy_lm() {
        assert_eq!(legacy_lm_version(38_455_852).unwrap().0, "2026.11.09");
        assert!(legacy_lm_version(38_455_853).is_none());
    }

    /// 读这几种数据的代码把标记那行当注释跳过（0.4.0 起都是这样，旧插件读新数据不出错）。
    #[test]
    fn readers_skip_marker_line() {
        let lexicon = "# 由 tools/rime2tsv.py 生成，来源：rime_ice.dict.yaml\n\
                       # wowtype-data: lexicon.tsv 2026.11.14.5 rime-ice\n\
                       ni hao\t你好\t100\n";
        use crate::lexicon::{Lexicon, MemoryLexicon};
        let lex = MemoryLexicon::from_tsv_str(lexicon).unwrap();
        assert_eq!(lex.len(), 1);

        let wubi = "# wowtype-data: wubi86.tsv 2023.10.25.0 rime-wubi\na\t工\t99454797\naa\t式\t1\n";
        let table = crate::wubi::WubiTable::from_tsv_str(wubi).unwrap();
        assert_eq!(table.len(), 2);

        let s2t = "# 由 tools/opencc2tsv.py 生成\n# wowtype-data: s2t.tsv 2026.11.14.0 OpenCC 1.4.1\n万\t萬\n";
        let conv = crate::convert::Converter::from_tsv_str(s2t).unwrap();
        assert_eq!(conv.convert("万"), "萬");

        let english = "# Rime dictionary\n# wowtype-data: english.txt 2026.11.14.5 rime-ice\n\
                       ---\nname: en\n...\nhello\thello\n";
        let engine = crate::Engine::new(
            std::sync::Arc::new(MemoryLexicon::from_tsv_str(lexicon).unwrap()),
            std::sync::Arc::new(crate::lm::BigramLm::default()),
        );
        assert_eq!(engine.load_english_str(english), 1);
    }

    /// 仓库里的简繁表带着标记。
    #[test]
    fn bundled_s2t_is_marked() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/data/s2t.tsv");
        let m = read_marker(path).expect("core/data/s2t.tsv 没有 wowtype-data 标记");
        assert_eq!(m.name, "s2t.tsv");
    }
}
