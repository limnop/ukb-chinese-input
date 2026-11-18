//! 最近用过的 emoji（表情面板不搜索时排最前）：进程内一份，所有输入框共用，最多
//! [`MAX`] 个，最近的在前。外壳给一个文件路径（Linux 的 `~/.local/share/wowtype/emoji-recent.txt`、
//! macOS 数据目录下同名文件），一行一个，每次记下一个就整份写回去。

use std::path::PathBuf;
use std::sync::Mutex;

/// 最多记几个（表情面板正好三行）。
pub const MAX: usize = 18;

struct Recent {
    path: Option<PathBuf>,
    items: Vec<String>,
}

static RECENT: Mutex<Recent> = Mutex::new(Recent {
    path: None,
    items: Vec::new(),
});

/// 用这个文件存（读进已有的）。换了路径会重新读；空串表示只记在内存里，并清空。
pub fn set_path(path: &str) {
    let Ok(mut recent) = RECENT.lock() else {
        return;
    };
    if path.is_empty() {
        recent.path = None;
        recent.items.clear();
        return;
    }
    let path = PathBuf::from(path);
    if recent.path.as_ref() == Some(&path) {
        return;
    }
    recent.items = std::fs::read_to_string(&path)
        .map(|text| {
            text.lines()
                .filter(|l| !l.trim().is_empty())
                .take(MAX)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    recent.path = Some(path);
}

/// 记下用了一个 emoji：挪到最前，写回文件。
pub fn remember(emoji: &str) {
    if emoji.trim().is_empty() || emoji.contains('\n') {
        return;
    }
    let Ok(mut recent) = RECENT.lock() else {
        return;
    };
    recent.items.retain(|e| e != emoji);
    recent.items.insert(0, emoji.to_owned());
    recent.items.truncate(MAX);
    if let Some(path) = recent.path.clone() {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let mut out = recent.items.join("\n");
        out.push('\n');
        // 丢了也无所谓（最多 18 个），不等刷到盘上：这是在按键的线程上
        let _ = crate::fsutil::write_replace(&path, out.as_bytes());
    }
}

/// 最近用过的，最近的在前。
pub fn list() -> Vec<String> {
    RECENT.lock().map(|r| r.items.clone()).unwrap_or_default()
}

