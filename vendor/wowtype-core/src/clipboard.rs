//! 剪贴板历史：只在内存里，不写盘，进程退出就没了。
//!
//! 输入法自己拿不到系统剪贴板，由外壳（fcitx5 插件、以后的 macOS 外壳）在合适的时候
//! 把看到的剪贴板文字交给 [`remember`]，工具栏的「剪贴板」列表从 [`history`] 取。

use std::sync::Mutex;

/// 最多记多少条。
pub const CAPACITY: usize = 20;
/// 一条最多多少字节，再长的不记（大段代码、整篇文章选了再上屏也不方便）。
pub const MAX_BYTES: usize = 16 * 1024;

static RING: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// 记下一条剪贴板文字，最新的在最前；重复的挪到最前面。空白、太长的不记。
/// 返回是否记下了（已经在最前面也算）。
pub fn remember(text: &str) -> bool {
    if text.trim().is_empty() || text.len() > MAX_BYTES {
        return false;
    }
    let Ok(mut ring) = RING.lock() else {
        return false;
    };
    if let Some(at) = ring.iter().position(|t| t == text) {
        let item = ring.remove(at);
        ring.insert(0, item);
    } else {
        ring.insert(0, text.to_string());
        ring.truncate(CAPACITY);
    }
    true
}

/// 最新的在前。
pub fn history() -> Vec<String> {
    RING.lock().map(|r| r.clone()).unwrap_or_default()
}

/// 清空。
pub fn clear() {
    if let Ok(mut ring) = RING.lock() {
        ring.clear();
    }
}

#[cfg(test)]
pub(crate) static TEST_LOCK: Mutex<()> = Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newest_first_dedupe_and_cap() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear();
        assert!(remember("a"));
        assert!(remember("b"));
        assert!(remember("a"));
        assert_eq!(history(), vec!["a", "b"]);
        assert!(!remember("   \n"));
        assert!(!remember(&"x".repeat(MAX_BYTES + 1)));
        for i in 0..CAPACITY + 5 {
            remember(&i.to_string());
        }
        let h = history();
        assert_eq!(h.len(), CAPACITY);
        assert_eq!(h[0], (CAPACITY + 4).to_string());
        clear();
        assert!(history().is_empty());
    }
}
