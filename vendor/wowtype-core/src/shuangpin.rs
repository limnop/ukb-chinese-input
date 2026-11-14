//! 双拼：每个音节两个键（声母键 + 韵母键）。
//!
//! 支持七种常用方案：自然码、小鹤、微软、搜狗、智能 ABC、紫光、拼音加加。
//! 键位表是这些方案公开的键盘布局；写好后逐个音节和 rime 的双拼方案核对过。
//!
//! 双拼只负责把键翻成全拼（`vsgo` -> `zhong'guo`），后面整句、语言模型、句首词、
//! 学习这些都和全拼走同一条路。最后单出来的一个键当声母（`vsg` -> `zhong'g`）。
//!
//! 零声母音节的打法各方案不同：
//! - 自然码、小鹤：单韵母双写（`aa` 啊），双字母韵母照打（`ai` 爱），
//!   也可以「韵母首字母 + 韵母键」（`al` 爱）；
//! - 微软、搜狗、拼音加加：`o` + 韵母键（`oj` 安），也可以「韵母首字母 + 韵母键」；
//! - 智能 ABC、紫光：只用 `o` + 韵母键。

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::syllable::{split, SYLLABLES};

/// 变体上限，防止病态输入把候选数量炸开。
pub const MAX_VARIANTS: usize = 8;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Scheme {
    #[default]
    Ziranma,
    Xiaohe,
    Microsoft,
    Sogou,
    Abc,
    Ziguang,
    PinyinJiajia,
}

pub const SCHEMES: [Scheme; 7] = [
    Scheme::Ziranma,
    Scheme::Xiaohe,
    Scheme::Microsoft,
    Scheme::Sogou,
    Scheme::Abc,
    Scheme::Ziguang,
    Scheme::PinyinJiajia,
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Zero {
    /// 单韵母双写、双字母韵母照打，或韵母首字母 + 韵母键（自然码、小鹤）
    Natural,
    /// `o` + 韵母键，或韵母首字母 + 韵母键（微软、搜狗、拼音加加）
    OOrFirst,
    /// 只有 `o` + 韵母键（智能 ABC、紫光）
    OOnly,
}

struct Layout {
    zh: u8,
    ch: u8,
    sh: u8,
    /// 多字母韵母（以及改了键的单韵母）的键；一个韵母可以有几个键。
    finals: &'static [(&'static str, u8)],
    zero: Zero,
}

const ZIRANMA: Layout = Layout {
    zh: b'v',
    ch: b'i',
    sh: b'u',
    finals: &[
        ("iu", b'q'),
        ("ia", b'w'),
        ("ua", b'w'),
        ("uan", b'r'),
        ("van", b'r'),
        ("ue", b't'),
        ("ve", b't'),
        ("ing", b'y'),
        ("uai", b'y'),
        ("uo", b'o'),
        ("un", b'p'),
        ("vn", b'p'),
        ("ong", b's'),
        ("iong", b's'),
        ("iang", b'd'),
        ("uang", b'd'),
        ("en", b'f'),
        ("eng", b'g'),
        ("ang", b'h'),
        ("ian", b'm'),
        ("an", b'j'),
        ("iao", b'c'),
        ("ao", b'k'),
        ("ai", b'l'),
        ("ei", b'z'),
        ("ie", b'x'),
        ("ui", b'v'),
        ("ou", b'b'),
        ("in", b'n'),
    ],
    zero: Zero::Natural,
};

const XIAOHE: Layout = Layout {
    zh: b'v',
    ch: b'i',
    sh: b'u',
    finals: &[
        ("iu", b'q'),
        ("ei", b'w'),
        ("uan", b'r'),
        ("ue", b't'),
        ("ve", b't'),
        ("un", b'y'),
        ("uo", b'o'),
        ("ie", b'p'),
        ("ong", b's'),
        ("iong", b's'),
        ("ing", b'k'),
        ("uai", b'k'),
        ("ai", b'd'),
        ("en", b'f'),
        ("eng", b'g'),
        ("iang", b'l'),
        ("uang", b'l'),
        ("ang", b'h'),
        ("ian", b'm'),
        ("an", b'j'),
        ("ou", b'z'),
        ("ia", b'x'),
        ("ua", b'x'),
        ("iao", b'n'),
        ("ao", b'c'),
        ("ui", b'v'),
        ("in", b'b'),
    ],
    zero: Zero::Natural,
};

/// 微软和搜狗只差一处：微软的 üe 除了 t 还能打 v。
const MICROSOFT_FINALS: &[(&str, u8)] = &[
    ("iu", b'q'),
    ("ia", b'w'),
    ("ua", b'w'),
    ("er", b'r'),
    ("uan", b'r'),
    ("van", b'r'),
    ("ue", b't'),
    ("ve", b't'),
    ("ue", b'v'),
    ("ve", b'v'),
    ("v", b'y'),
    ("uai", b'y'),
    ("uo", b'o'),
    ("un", b'p'),
    ("vn", b'p'),
    ("ong", b's'),
    ("iong", b's'),
    ("iang", b'd'),
    ("uang", b'd'),
    ("en", b'f'),
    ("eng", b'g'),
    ("ang", b'h'),
    ("ian", b'm'),
    ("an", b'j'),
    ("iao", b'c'),
    ("ao", b'k'),
    ("ai", b'l'),
    ("ei", b'z'),
    ("ie", b'x'),
    ("ui", b'v'),
    ("ou", b'b'),
    ("in", b'n'),
    ("ing", b';'),
];

const MICROSOFT: Layout = Layout {
    zh: b'v',
    ch: b'i',
    sh: b'u',
    finals: MICROSOFT_FINALS,
    zero: Zero::OOrFirst,
};

const SOGOU: Layout = Layout {
    zh: b'v',
    ch: b'i',
    sh: b'u',
    finals: &[
        ("iu", b'q'),
        ("ia", b'w'),
        ("ua", b'w'),
        ("er", b'r'),
        ("uan", b'r'),
        ("van", b'r'),
        ("ue", b't'),
        ("ve", b't'),
        ("v", b'y'),
        ("uai", b'y'),
        ("uo", b'o'),
        ("un", b'p'),
        ("vn", b'p'),
        ("ong", b's'),
        ("iong", b's'),
        ("iang", b'd'),
        ("uang", b'd'),
        ("en", b'f'),
        ("eng", b'g'),
        ("ang", b'h'),
        ("ian", b'm'),
        ("an", b'j'),
        ("iao", b'c'),
        ("ao", b'k'),
        ("ai", b'l'),
        ("ei", b'z'),
        ("ie", b'x'),
        ("ui", b'v'),
        ("ou", b'b'),
        ("in", b'n'),
        ("ing", b';'),
    ],
    zero: Zero::OOrFirst,
};

const ABC: Layout = Layout {
    zh: b'a',
    ch: b'e',
    sh: b'v',
    finals: &[
        ("ei", b'q'),
        ("ian", b'w'),
        ("er", b'r'),
        ("iu", b'r'),
        ("iang", b't'),
        ("uang", b't'),
        ("ing", b'y'),
        ("uo", b'o'),
        ("uan", b'p'),
        ("ong", b's'),
        ("iong", b's'),
        ("ia", b'd'),
        ("ua", b'd'),
        ("en", b'f'),
        ("eng", b'g'),
        ("ang", b'h'),
        ("an", b'j'),
        ("iao", b'z'),
        ("ao", b'k'),
        ("in", b'c'),
        ("uai", b'c'),
        ("ai", b'l'),
        ("ie", b'x'),
        ("ou", b'b'),
        ("un", b'n'),
        ("ue", b'm'),
        ("ve", b'm'),
        ("ui", b'm'),
    ],
    zero: Zero::OOnly,
};

const ZIGUANG: Layout = Layout {
    zh: b'u',
    ch: b'a',
    sh: b'i',
    finals: &[
        ("en", b'w'),
        ("eng", b't'),
        ("in", b'y'),
        ("uai", b'y'),
        ("uo", b'o'),
        ("ai", b'p'),
        ("iang", b'g'),
        ("uang", b'g'),
        ("ang", b's'),
        ("ie", b'd'),
        ("ian", b'f'),
        ("ong", b'h'),
        ("iong", b'h'),
        ("er", b'j'),
        ("iu", b'j'),
        ("ei", b'k'),
        ("uan", b'l'),
        ("ing", b';'),
        ("ou", b'z'),
        ("ia", b'x'),
        ("ua", b'x'),
        ("iao", b'b'),
        ("ue", b'n'),
        ("ui", b'n'),
        ("ve", b'n'),
        ("un", b'm'),
        ("ao", b'q'),
        ("an", b'r'),
    ],
    zero: Zero::OOnly,
};

const PINYIN_JIAJIA: Layout = Layout {
    zh: b'v',
    ch: b'u',
    sh: b'i',
    finals: &[
        ("iu", b'n'),
        ("ia", b'b'),
        ("ua", b'b'),
        ("er", b'q'),
        ("ing", b'q'),
        ("uan", b'c'),
        ("van", b'c'),
        ("ue", b'x'),
        ("ve", b'x'),
        ("uai", b'x'),
        ("uo", b'o'),
        ("un", b'z'),
        ("vn", b'z'),
        ("ong", b'y'),
        ("iong", b'y'),
        ("iang", b'h'),
        ("uang", b'h'),
        ("en", b'r'),
        ("eng", b't'),
        ("ang", b'g'),
        ("ian", b'j'),
        ("an", b'f'),
        ("iao", b'k'),
        ("ao", b'd'),
        ("ai", b's'),
        ("ei", b'w'),
        ("ie", b'm'),
        ("ui", b'v'),
        ("ou", b'p'),
        ("in", b'l'),
    ],
    zero: Zero::OOrFirst,
};

impl Scheme {
    /// 配置文件里的名字。
    pub fn id(self) -> &'static str {
        match self {
            Scheme::Ziranma => "ziranma",
            Scheme::Xiaohe => "xiaohe",
            Scheme::Microsoft => "microsoft",
            Scheme::Sogou => "sogou",
            Scheme::Abc => "abc",
            Scheme::Ziguang => "ziguang",
            Scheme::PinyinJiajia => "pinyinjiajia",
        }
    }

    /// 界面上显示的名字。
    pub fn name(self) -> &'static str {
        match self {
            Scheme::Ziranma => "自然码",
            Scheme::Xiaohe => "小鹤双拼",
            Scheme::Microsoft => "微软双拼",
            Scheme::Sogou => "搜狗双拼",
            Scheme::Abc => "智能 ABC",
            Scheme::Ziguang => "紫光双拼",
            Scheme::PinyinJiajia => "拼音加加",
        }
    }

    /// 认 id、中文名和几个常见别名（flypy、mspy、jiajia），不分大小写。
    pub fn from_name(name: &str) -> Option<Self> {
        let key: String = name
            .trim()
            .chars()
            .filter(|c| !c.is_whitespace() && *c != '_' && *c != '-')
            .flat_map(char::to_lowercase)
            .collect();
        let alias = match key.as_str() {
            "flypy" | "xiaoheshuangpin" | "小鹤" => "xiaohe",
            "mspy" | "ms" | "微软" => "microsoft",
            "搜狗" => "sogou",
            "智能abc" => "abc",
            "紫光" => "ziguang",
            "jiajia" | "pyjj" => "pinyinjiajia",
            other => other,
        };
        SCHEMES
            .into_iter()
            .find(|s| s.id() == alias || s.name().replace(' ', "").to_lowercase() == alias)
    }

    fn layout(self) -> &'static Layout {
        match self {
            Scheme::Ziranma => &ZIRANMA,
            Scheme::Xiaohe => &XIAOHE,
            Scheme::Microsoft => &MICROSOFT,
            Scheme::Sogou => &SOGOU,
            Scheme::Abc => &ABC,
            Scheme::Ziguang => &ZIGUANG,
            Scheme::PinyinJiajia => &PINYIN_JIAJIA,
        }
    }

    /// 这个方案有没有用分号键（微软、搜狗、紫光的 ing）。
    pub fn uses_semicolon(self) -> bool {
        self.layout().finals.iter().any(|&(_, k)| k == b';')
    }

    /// 是不是这个方案的键（字母，加上用得到的分号）。
    pub fn is_key(self, b: u8) -> bool {
        b.is_ascii_lowercase() || (b == b';' && self.uses_semicolon())
    }
}

fn final_keys(layout: &Layout, final_: &str) -> Vec<u8> {
    let keys: Vec<u8> = layout
        .finals
        .iter()
        .filter(|(f, _)| *f == final_)
        .map(|&(_, k)| k)
        .collect();
    if !keys.is_empty() {
        return keys;
    }
    match final_.as_bytes() {
        [b @ (b'a' | b'o' | b'e' | b'i' | b'u' | b'v')] => vec![*b],
        _ => Vec::new(),
    }
}

/// j q x y 后面的 u 其实是 ü，表里同时有 ju 和 jv 两种写法；只编码 u 的写法。
fn is_v_spelling(syllable: &str) -> bool {
    let b = syllable.as_bytes();
    b.len() >= 2 && matches!(b[0], b'j' | b'q' | b'x' | b'y') && b[1] == b'v'
}

/// 一个全拼音节在这个方案下所有的两键打法。
pub fn encode(scheme: Scheme, syllable: &str) -> Vec<[u8; 2]> {
    let mut out = Vec::new();
    if is_v_spelling(syllable) {
        return out;
    }
    let Some((initial, final_)) = split(syllable) else {
        return out;
    };
    let layout = scheme.layout();
    let keys = final_keys(layout, final_);
    let f = final_.as_bytes();
    let mut push = |code: [u8; 2]| {
        if !out.contains(&code) {
            out.push(code);
        }
    };
    if initial.is_empty() {
        if !matches!(f.first(), Some(b'a' | b'o' | b'e')) {
            return out; // ng、hm 这类没有双拼打法
        }
        if layout.zero == Zero::Natural && f.len() <= 2 {
            push(if f.len() == 1 {
                [f[0], f[0]]
            } else {
                [f[0], f[1]]
            });
        }
        // 微软、搜狗、拼音加加的 ou 也能照打
        if layout.zero == Zero::OOrFirst && final_ == "ou" {
            push([b'o', b'u']);
        }
        for &k in &keys {
            if layout.zero != Zero::OOnly {
                push([f[0], k]);
            }
            if layout.zero != Zero::Natural {
                push([b'o', k]);
            }
        }
        return out;
    }
    let ini = match initial {
        "zh" => layout.zh,
        "ch" => layout.ch,
        "sh" => layout.sh,
        other => other.as_bytes()[0],
    };
    for &k in &keys {
        push([ini, k]);
    }
    // ju、jue、juan、jun 也能按 ü 的键打
    if matches!(initial, "j" | "q" | "x" | "y") && f.first() == Some(&b'u') {
        let v_final = format!("v{}", &final_[1..]);
        for k in final_keys(layout, &v_final) {
            push([ini, k]);
        }
    }
    out
}

fn table(scheme: Scheme) -> &'static HashMap<[u8; 2], Vec<&'static str>> {
    static TABLES: [OnceLock<HashMap<[u8; 2], Vec<&'static str>>>; 7] =
        [const { OnceLock::new() }; 7];
    let idx = SCHEMES.iter().position(|&s| s == scheme).unwrap_or(0);
    TABLES[idx].get_or_init(|| {
        let mut map: HashMap<[u8; 2], Vec<&'static str>> = HashMap::new();
        for syllable in SYLLABLES {
            for code in encode(scheme, syllable) {
                let list = map.entry(code).or_default();
                if !list.contains(syllable) {
                    list.push(*syllable);
                }
            }
        }
        map
    })
}

/// 两键对应的所有音节（个别方案有少量重码）。
pub fn decode_all(scheme: Scheme, a: u8, b: u8) -> &'static [&'static str] {
    table(scheme)
        .get(&[a.to_ascii_lowercase(), b.to_ascii_lowercase()])
        .map(|v| v.as_slice())
        .unwrap_or(&[])
}

/// 最后单出来的一个键当声母：自然码的 v -> zh，b -> b，a -> a。
pub fn trailing_initial(scheme: Scheme, key: u8) -> Option<&'static str> {
    let layout = scheme.layout();
    let key = key.to_ascii_lowercase();
    if key == layout.zh {
        return Some("zh");
    }
    if key == layout.ch {
        return Some("ch");
    }
    if key == layout.sh {
        return Some("sh");
    }
    const LETTERS: &[&str] = &[
        "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m", "n", "o", "p", "q", "r",
        "s", "t", "u", "v", "w", "x", "y", "z",
    ];
    match key {
        // 不是声母的元音键：a e o 可以开头（啊、饿、哦），i u v 不行
        b'i' | b'u' | b'v' => None,
        b'a'..=b'z' => Some(LETTERS[(key - b'a') as usize]),
        _ => None,
    }
}

/// 一种解码结果：全拼（音节之间用隔音符）和每个音节在原始输入里的结束位置。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decoded {
    /// `zhong'guo`、`zhong'g`
    pub pinyin: String,
    /// 每个音节：(在 pinyin 里的结束字节, 在原始输入里的结束字节)
    pub ends: Vec<(usize, usize)>,
}

impl Decoded {
    /// 全拼里用掉前 `pinyin_bytes` 字节，对应原始输入的前多少字节；
    /// 落在音节中间时往前取整，一个音节都不到返回 0。
    pub fn raw_bytes(&self, pinyin_bytes: usize, raw_len: usize) -> usize {
        if pinyin_bytes >= self.pinyin.len() {
            return raw_len;
        }
        self.ends
            .iter()
            .take_while(|(p, _)| *p <= pinyin_bytes)
            .last()
            .map(|&(_, r)| r)
            .unwrap_or(0)
    }
}

/// 把双拼输入翻成全拼。有重码时给出几种（最多 [`MAX_VARIANTS`]）；
/// 有一对键对不上任何音节时返回空。用户打的隔音符跳过。
pub fn decode(scheme: Scheme, input: &str) -> Vec<Decoded> {
    // (键, 这个键之后在原始输入里的字节位置，含紧跟着的隔音符)
    let bytes = input.as_bytes();
    let mut keys: Vec<(u8, usize)> = Vec::new();
    for (i, &b) in bytes.iter().enumerate() {
        let lower = b.to_ascii_lowercase();
        if scheme.is_key(lower) {
            keys.push((lower, i + 1));
        } else if b == b'\'' {
            if let Some(last) = keys.last_mut() {
                last.1 = i + 1;
            }
        } else {
            return Vec::new();
        }
    }
    if keys.is_empty() {
        return Vec::new();
    }
    // 第一个键不能是分号
    if keys[0].0 == b';' {
        return Vec::new();
    }
    let mut variants = vec![Decoded {
        pinyin: String::new(),
        ends: Vec::new(),
    }];
    let pairs = keys.len() / 2;
    for p in 0..pairs {
        let (a, _) = keys[2 * p];
        let (b, end) = keys[2 * p + 1];
        let options = decode_all(scheme, a, b);
        if options.is_empty() {
            return Vec::new();
        }
        let mut next = Vec::new();
        for prefix in &variants {
            for syllable in options {
                if next.len() >= MAX_VARIANTS {
                    break;
                }
                let mut d = prefix.clone();
                if !d.pinyin.is_empty() {
                    d.pinyin.push('\'');
                }
                d.pinyin.push_str(syllable);
                d.ends.push((d.pinyin.len(), end));
                next.push(d);
            }
        }
        variants = next;
    }
    if keys.len() % 2 == 1 {
        let (key, end) = keys[keys.len() - 1];
        if let Some(initial) = trailing_initial(scheme, key) {
            for d in &mut variants {
                if !d.pinyin.is_empty() {
                    d.pinyin.push('\'');
                }
                d.pinyin.push_str(initial);
                d.ends.push((d.pinyin.len(), end));
            }
        }
        // 当不了声母的键（i u v）先不管，等下一个键
    }
    variants.retain(|d| !d.ends.is_empty());
    variants
}

/// 编辑区显示：`vsgo` -> `zhong'guo`，打了一半 `vsg` -> `zhong'g`。
/// 翻不了的照原样显示。
pub fn display(scheme: Scheme, input: &str) -> String {
    match decode(scheme, input).into_iter().next() {
        Some(d) => {
            let used = d.ends.last().map(|&(_, r)| r).unwrap_or(0);
            let mut shown = d.pinyin;
            if used < input.len() {
                shown.push('\'');
                shown.push_str(&input[used..]);
            }
            shown
        }
        None => input.to_string(),
    }
}

/// 双拼键串翻成全拼音节（空格分隔），取第一种；翻不了返回空串。
pub fn to_full_pinyin(scheme: Scheme, input: &str) -> String {
    decode(scheme, input)
        .into_iter()
        .next()
        .map(|d| d.pinyin.replace('\'', " "))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codes(scheme: Scheme, syllable: &str) -> Vec<String> {
        encode(scheme, syllable)
            .into_iter()
            .map(|[a, b]| format!("{}{}", a as char, b as char))
            .collect()
    }

    #[test]
    fn ziranma_layout() {
        let s = Scheme::Ziranma;
        assert_eq!(codes(s, "zhong"), ["vs"]);
        assert_eq!(codes(s, "guo"), ["go"]);
        assert_eq!(codes(s, "hao"), ["hk"]);
        assert_eq!(codes(s, "xie"), ["xx"]);
        assert_eq!(codes(s, "xiao"), ["xc"]);
        assert_eq!(codes(s, "xin"), ["xn"]);
        assert_eq!(codes(s, "jia"), ["jw"]);
        assert_eq!(codes(s, "a"), ["aa"]);
        assert_eq!(codes(s, "ai"), ["ai", "al"]);
        assert_eq!(codes(s, "ang"), ["ah"]);
        assert_eq!(codes(s, "er"), ["er"]);
        assert!(codes(s, "jue").contains(&"jt".to_string()));
    }

    #[test]
    fn other_layouts() {
        assert_eq!(codes(Scheme::Xiaohe, "xie"), ["xp"]);
        assert_eq!(codes(Scheme::Xiaohe, "zhuang"), ["vl"]);
        assert_eq!(codes(Scheme::Xiaohe, "ou"), ["ou", "oz"]);
        assert_eq!(codes(Scheme::Microsoft, "ming"), ["m;"]);
        assert_eq!(codes(Scheme::Microsoft, "an"), ["aj", "oj"]);
        assert_eq!(codes(Scheme::Microsoft, "lv"), ["ly"]);
        assert!(codes(Scheme::Microsoft, "lve").contains(&"lv".to_string()));
        assert!(!codes(Scheme::Sogou, "lve").contains(&"lv".to_string()));
        assert_eq!(codes(Scheme::Abc, "zhang"), ["ah"]);
        assert_eq!(codes(Scheme::Abc, "ai"), ["ol"]);
        assert_eq!(codes(Scheme::Ziguang, "chang"), ["as"]);
        assert_eq!(codes(Scheme::Ziguang, "qing"), ["q;"]);
        assert_eq!(codes(Scheme::PinyinJiajia, "shi"), ["ii"]);
        assert_eq!(codes(Scheme::PinyinJiajia, "chu"), ["uu"]);
    }

    #[test]
    fn every_scheme_round_trips() {
        for scheme in SCHEMES {
            let mut total = 0;
            for syllable in SYLLABLES {
                for [a, b] in encode(scheme, syllable) {
                    total += 1;
                    assert!(
                        decode_all(scheme, a, b).contains(syllable),
                        "{scheme:?} {syllable}"
                    );
                }
            }
            assert!(total > 390, "{scheme:?}: 可编码音节太少 {total}");
        }
    }

    #[test]
    fn scheme_names() {
        assert_eq!(Scheme::from_name("flypy"), Some(Scheme::Xiaohe));
        assert_eq!(Scheme::from_name("Microsoft"), Some(Scheme::Microsoft));
        assert_eq!(Scheme::from_name("智能 ABC"), Some(Scheme::Abc));
        assert_eq!(Scheme::from_name("拼音加加"), Some(Scheme::PinyinJiajia));
        assert_eq!(Scheme::from_name("nope"), None);
        for s in SCHEMES {
            assert_eq!(Scheme::from_name(s.id()), Some(s));
        }
    }

    #[test]
    fn decodes_input() {
        let s = Scheme::Ziranma;
        let d = decode(s, "vsgo");
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].pinyin, "zhong'guo");
        assert_eq!(d[0].ends, vec![(5, 2), (9, 4)]);
        // 单出来的键当声母
        assert_eq!(decode(s, "vsg")[0].pinyin, "zhong'g");
        assert_eq!(decode(s, "vsv")[0].pinyin, "zhong'zh");
        // i u v 当不了声母，先不算
        assert_eq!(decode(s, "vsu")[0].pinyin, "zhong'sh");
        assert_eq!(decode(Scheme::Xiaohe, "nii")[0].pinyin, "ni'ch");
        assert!(decode(s, "bb").is_empty());
        assert_eq!(display(s, "nihk"), "ni'hao");
        assert_eq!(to_full_pinyin(s, "nihk"), "ni hao");
        // 分号只在用它的方案里算键
        assert_eq!(decode(Scheme::Microsoft, "m;")[0].pinyin, "ming");
        assert!(decode(Scheme::Ziranma, "m;").is_empty());
    }

    #[test]
    fn maps_consumed_bytes() {
        let d = &decode(Scheme::Ziranma, "vs'go")[0];
        assert_eq!(d.raw_bytes(5, 5), 3);
        assert_eq!(d.raw_bytes(6, 5), 3);
        assert_eq!(d.raw_bytes(9, 5), 5);
        assert_eq!(d.raw_bytes(2, 5), 0);
    }
}
