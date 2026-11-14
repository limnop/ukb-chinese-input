//! 中文标点与常用符号。
//!
//! 两块能力：
//! 1. `fullwidth_punct`：把 ASCII 标点换成中文标点（输入法里最常用的行为）
//! 2. `search`：按拼音/中文名前缀检索符号，供符号面板使用

/// 一个符号条目：展示文本 + 可检索的名字（拼音优先，也接受中文名）。
pub struct Symbol {
    pub text: &'static str,
    pub names: &'static [&'static str],
}

const fn s(text: &'static str, names: &'static [&'static str]) -> Symbol {
    Symbol { text, names }
}

/// 内置符号表。名字用小写拼音，便于前缀检索。
pub const SYMBOLS: &[Symbol] = &[
    s("，", &["douhao", "逗号"]),
    s("。", &["juhao", "句号"]),
    s("、", &["dunhao", "顿号"]),
    s("；", &["fenhao", "分号"]),
    s("：", &["maohao", "冒号"]),
    s("？", &["wenhao", "问号"]),
    s("！", &["tanhao", "感叹号"]),
    s("“", &["yinhao", "引号", "zuoyinhao"]),
    s("”", &["yinhao", "引号", "youyinhao"]),
    s("‘", &["danyinhao"]),
    s("’", &["danyinhao"]),
    s("（", &["kuohao", "括号", "zuokuohao"]),
    s("）", &["kuohao", "括号", "youkuohao"]),
    s("【", &["fangkuohao", "方括号"]),
    s("】", &["fangkuohao", "方括号"]),
    s("《", &["shuminghao", "书名号"]),
    s("》", &["shuminghao", "书名号"]),
    s("——", &["pozhehao", "破折号"]),
    s("……", &["shengluehao", "省略号"]),
    s("·", &["jianhaohao", "间隔号"]),
    s("～", &["bolanghao", "波浪号"]),
    s("￥", &["renminbi", "人民币", "yuan"]),
    s("＄", &["meiyuan", "美元"]),
    s("％", &["baifenhao", "百分号"]),
    s("‰", &["qianfenhao"]),
    s("℃", &["sheshidu", "摄氏度"]),
    s("±", &["zhengfuhao", "正负号"]),
    s("×", &["chenghao", "乘号"]),
    s("÷", &["chuhao", "除号"]),
    s("≈", &["yuedengyu", "约等于"]),
    s("≠", &["budengyu", "不等于"]),
    s("≤", &["xiaoyudengyu", "小于等于"]),
    s("≥", &["dayudengyu", "大于等于"]),
    s("∞", &["wuqiong", "无穷"]),
    s("√", &["genhao", "根号"]),
    s("°", &["du", "度"]),
    s("→", &["youjiantou", "右箭头"]),
    s("←", &["zuojiantou", "左箭头"]),
    s("↑", &["shangjiantou", "上箭头"]),
    s("↓", &["xiajiantou", "下箭头"]),
    s("☆", &["xingxing", "星星"]),
    s("★", &["xingxing", "星星"]),
    s("○", &["yuanquan", "圆圈"]),
    s("●", &["yuanquan", "圆圈"]),
    s("△", &["sanjiao", "三角"]),
    s("▲", &["sanjiao", "三角"]),
    s("□", &["fangkuang", "方框"]),
    s("■", &["fangkuang", "方框"]),
    s("✓", &["gou", "对勾"]),
    s("✗", &["cha", "叉"]),
    s("①", &["yihao", "1"]),
    s("②", &["erhao", "2"]),
    s("③", &["sanhao", "3"]),
    s("④", &["sihao", "4"]),
    s("⑤", &["wuhao", "5"]),
    s("§", &["duanluohao"]),
    s("№", &["bianhao"]),
];

/// ASCII 标点 -> 中文标点。未收录的返回 None。
///
/// `@ # % & *` 不收：邮箱、话题、百分比、Markdown 里都要半角，主流输入法也原样上屏。
/// `{ }` 照 macOS 自带拼音给直角引号「」。
pub fn fullwidth_punct(ascii: char) -> Option<&'static str> {
    let text = match ascii {
        ',' => "，",
        '.' => "。",
        '?' => "？",
        '!' => "！",
        ';' => "；",
        ':' => "：",
        '(' => "（",
        ')' => "）",
        '[' => "【",
        ']' => "】",
        '<' => "《",
        '>' => "》",
        '\\' => "、",
        '~' => "～",
        '`' => "·",
        '^' => "……",
        '_' => "——",
        '"' => "“",
        '\'' => "‘",
        '{' => "「",
        '}' => "」",
        '$' => "￥",
        _ => return None,
    };
    Some(text)
}

/// 标点样式：全角（中文标点）或半角（ASCII 原样上屏）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PunctStyle {
    Fullwidth,
    Halfwidth,
}

/// 按样式和光标前一个字符决定标点。返回 `None` 表示原样上屏这个 ASCII 字符。
///
/// - 半角样式：一律原样上屏
/// - 全角样式：数字后面的符号一律原样（3.14、20:22、3,000、50%），不换成中文标点
pub fn punct(ascii: char, style: PunctStyle, prev: Option<char>) -> Option<&'static str> {
    if style == PunctStyle::Halfwidth {
        return None;
    }
    if prev.is_some_and(|c| c.is_ascii_digit()) {
        return None;
    }
    if ascii == '"' || ascii == '\'' {
        return Some(quote(ascii, prev));
    }
    fullwidth_punct(ascii)
}

thread_local! {
    /// 双引号、单引号各自上一次给的是不是左引号。
    static QUOTE_OPEN: std::cell::Cell<[bool; 2]> = const { std::cell::Cell::new([false; 2]) };
}

/// 引号左右交替：“……” ‘……’。光标前是空白或左括号时一定给左引号。
fn quote(ascii: char, prev: Option<char>) -> &'static str {
    let (slot, open, close) = if ascii == '"' {
        (0, "“", "”")
    } else {
        (1, "‘", "’")
    };
    QUOTE_OPEN.with(|state| {
        let mut opened = state.get();
        let want_open = prev.is_some_and(|c| c.is_whitespace() || "(（[【<《{「『".contains(c))
            || !opened[slot];
        opened[slot] = want_open;
        state.set(opened);
        if want_open {
            open
        } else {
            close
        }
    })
}

/// 按拼音或中文名前缀检索符号。
pub fn search(query: &str, limit: usize) -> Vec<&'static Symbol> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<&'static Symbol> = SYMBOLS
        .iter()
        .filter(|sym| sym.names.iter().any(|name| name.starts_with(&q)))
        .collect();
    // 名字完全相等的排前面
    out.sort_by_key(|sym| {
        if sym.names.iter().any(|name| *name == q) {
            0
        } else {
            1
        }
    });
    out.truncate(limit.max(1));
    out
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_common_punctuation() {
        assert_eq!(fullwidth_punct(','), Some("，"));
        assert_eq!(fullwidth_punct('?'), Some("？"));
        assert_eq!(fullwidth_punct('a'), None);
    }

    #[test]
    fn at_hash_percent_stay_ascii_and_braces_are_corner_brackets() {
        use PunctStyle::*;
        for c in ['@', '#', '%', '&', '*', '/', '+', '='] {
            assert_eq!(punct(c, Fullwidth, None), None, "{c}");
        }
        assert_eq!(punct('{', Fullwidth, None), Some("「"));
        assert_eq!(punct('}', Fullwidth, Some('好')), Some("」"));
        assert_eq!(punct('[', Fullwidth, None), Some("【"));
        assert_eq!(punct('{', Halfwidth, None), None);
    }

    #[test]
    fn punct_follows_style() {
        use PunctStyle::*;
        for c in [',', '.', '(', ')', ';', ':', '?', '!'] {
            assert_eq!(punct(c, Halfwidth, None), None, "{c}");
            assert_eq!(punct(c, Fullwidth, None), fullwidth_punct(c), "{c}");
        }
        assert_eq!(punct(',', Fullwidth, Some('好')), Some("，"));
    }

    #[test]
    fn punct_after_digit_stays_ascii() {
        use PunctStyle::*;
        // 3.14、20:22、3,000、50%：数字后面的符号都原样
        for c in ['.', ':', ',', ';', '(', ')', '$', '~', '"', '}'] {
            assert_eq!(punct(c, Fullwidth, Some('3')), None, "{c}");
        }
        assert_eq!(punct('.', Fullwidth, Some('好')), Some("。"));
        assert_eq!(punct('.', Fullwidth, Some('a')), Some("。"));
        assert_eq!(punct(':', Fullwidth, Some('好')), Some("："));
        // 全角数字不算
        assert_eq!(punct(':', Fullwidth, Some('３')), Some("："));
    }

    /// 主流输入法的写法：Shift+6 省略号、Shift+- 破折号、Shift+4 人民币符号、` 间隔号。
    /// 反馈见 https://x.com/so1v3/status/2120415675969896729
    #[test]
    fn maps_ellipsis_dash_yuan_and_middle_dot() {
        use PunctStyle::*;
        assert_eq!(punct('^', Fullwidth, None), Some("……"));
        assert_eq!(punct('_', Fullwidth, None), Some("——"));
        assert_eq!(punct('$', Fullwidth, None), Some("￥"));
        assert_eq!(punct('`', Fullwidth, None), Some("·"));
        assert_eq!(punct('^', Halfwidth, None), None);
    }

    #[test]
    fn quotes_alternate_left_and_right() {
        use PunctStyle::*;
        // 各测试跑在自己的线程里，引号状态是线程内的
        assert_eq!(punct('"', Fullwidth, None), Some("“"));
        assert_eq!(punct('"', Fullwidth, Some('好')), Some("”"));
        assert_eq!(punct('"', Fullwidth, Some('。')), Some("“"));
        // 单引号单独记
        assert_eq!(punct('\'', Fullwidth, None), Some("‘"));
        assert_eq!(punct('"', Fullwidth, Some('好')), Some("”"));
        assert_eq!(punct('\'', Fullwidth, None), Some("’"));
        // 光标前是空白或左括号：一定是左引号
        assert_eq!(punct('"', Fullwidth, None), Some("“"));
        assert_eq!(punct('"', Fullwidth, Some(' ')), Some("“"));
        assert_eq!(punct('"', Fullwidth, Some('（')), Some("“"));
        assert_eq!(punct('"', Halfwidth, None), None);
    }

    #[test]
    fn searches_by_pinyin_prefix() {
        let hits = search("juhao", 5);
        assert_eq!(hits[0].text, "。");
        let hits = search("kuo", 5);
        assert!(hits.iter().any(|s| s.text == "（"));
        assert!(hits.iter().any(|s| s.text == "）"));
    }

    #[test]
    fn searches_by_chinese_name() {
        let hits = search("逗号", 3);
        assert_eq!(hits[0].text, "，");
    }

    #[test]
    fn exact_name_ranks_first() {
        let hits = search("yinhao", 5);
        assert!(hits.len() >= 2);
        assert!(hits[0].names.contains(&"yinhao"));
    }

    #[test]
    fn empty_query_returns_nothing() {
        assert!(search("", 5).is_empty());
        assert!(search("   ", 5).is_empty());
    }
}
