//! 中英文之间自动补空格（`你好Linux` → `你好 Linux`、`第5章` → `第 5 章`）。
//!
//! 只看「光标前一个字」和「要上屏的第一个字」这一处边界：词里本来就有的字母
//! （`T恤`、`卡拉OK`）不拆开。默认关闭，由外壳决定什么时候问（密码框、网址、
//! 邮箱输入框，读不到光标前文字时都不问）。

/// 汉字和字母之间补空格。
pub const LETTERS: u32 = 1;
/// 汉字和数字之间补空格。
pub const DIGITS: u32 = 2;

/// 是不是汉字（基本区、扩展 A–G、兼容区）。假名、谚文、全角标点都不算。
pub fn is_han(c: char) -> bool {
    matches!(c as u32,
        0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0x20000..=0x3134F)
}

fn is_ascii_kind(c: char, flags: u32) -> bool {
    (flags & LETTERS != 0 && c.is_ascii_alphabetic()) || (flags & DIGITS != 0 && c.is_ascii_digit())
}

/// `prev` 是光标前一个字，`next` 是要上屏的第一个字：两者之间要不要补一个空格。
pub fn needs_space(prev: char, next: char, flags: u32) -> bool {
    (is_han(prev) && is_ascii_kind(next, flags)) || (is_ascii_kind(prev, flags) && is_han(next))
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOTH: u32 = LETTERS | DIGITS;

    #[test]
    fn han_and_ascii() {
        assert!(needs_space('好', 'L', BOTH));
        assert!(needs_space('x', '系', BOTH));
        assert!(needs_space('第', '5', BOTH));
        assert!(needs_space('5', '章', BOTH));
        assert!(needs_space('𠀀', 'a', BOTH), "扩展 B 也是汉字");
    }

    #[test]
    fn flags_are_separate() {
        assert!(needs_space('好', 'L', LETTERS));
        assert!(!needs_space('第', '5', LETTERS));
        assert!(needs_space('第', '5', DIGITS));
        assert!(!needs_space('好', 'L', DIGITS));
        assert!(!needs_space('好', 'L', 0));
    }

    #[test]
    fn leaves_everything_else() {
        assert!(!needs_space('你', '好', BOTH), "纯中文");
        assert!(!needs_space('a', 'b', BOTH), "纯英文");
        assert!(!needs_space(' ', 'a', BOTH), "已经有空格");
        assert!(!needs_space('好', ' ', BOTH));
        assert!(!needs_space('，', 'a', BOTH), "全角标点");
        assert!(!needs_space('好', '.', BOTH), "ASCII 标点");
        assert!(!needs_space('(', '你', BOTH));
        assert!(!needs_space('あ', 'a', BOTH), "假名不算汉字");
        assert!(!needs_space('好', 'ａ', BOTH), "全角字母不算");
    }
}
