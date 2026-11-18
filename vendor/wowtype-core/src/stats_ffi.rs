//! 打字统计的 C ABI（1.12 起）：输入法进程持有记录器，设置界面只读文件、请求清空。
//!
//! 约定见 core/include/wowtype_core.h：句柄由调用方保证生命周期（一个线程用），
//! 失败用 wowtype_last_error 取原因。记录器新建时是关着的，外壳按 `TypingStats` 打开。

use std::os::raw::{c_char, c_int, c_uint};
use std::time::Duration;

use crate::ffi::set_last_error;
use crate::stats::{self, Recorder};

pub struct WowtypeStats {
    recorder: Recorder,
}

unsafe fn cstr(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    unsafe { std::ffi::CStr::from_ptr(ptr) }
        .to_str()
        .ok()
        .map(str::to_owned)
}

/// 建一个记录器（关着）；`path` 是统计文件的绝对路径。不读、不建文件。失败返回 NULL。
#[no_mangle]
pub unsafe extern "C" fn wowtype_stats_new(path: *const c_char) -> *mut WowtypeStats {
    let Some(path) = (unsafe { cstr(path) }) else {
        set_last_error("stats path is null or not utf8");
        return std::ptr::null_mut();
    };
    if path.is_empty() {
        set_last_error("stats path is empty");
        return std::ptr::null_mut();
    }
    Box::into_raw(Box::new(WowtypeStats {
        recorder: Recorder::new(path),
    }))
}

/// 交出最后攒着的（开着时），等写完再释放。
#[no_mangle]
pub unsafe extern "C" fn wowtype_stats_free(stats: *mut WowtypeStats) {
    if stats.is_null() {
        return;
    }
    drop(unsafe { Box::from_raw(stats) });
}

/// 打开（非 0）或关上（0）统计。关上时丢掉内存里还没交出去的计数，以后什么都不记。
#[no_mangle]
pub unsafe extern "C" fn wowtype_stats_set_enabled(stats: *mut WowtypeStats, enabled: c_int) -> c_int {
    let Some(stats) = (unsafe { stats.as_mut() }) else {
        set_last_error("stats is null");
        return -1;
    };
    stats.recorder.set_enabled(enabled != 0, stats::now_ms());
    0
}

/// 1 开，0 关（或句柄为 NULL）。
#[no_mangle]
pub unsafe extern "C" fn wowtype_stats_enabled(stats: *const WowtypeStats) -> c_int {
    let Some(stats) = (unsafe { stats.as_ref() }) else {
        return 0;
    };
    c_int::from(stats.recorder.enabled())
}

/// 一次物理按键。`platform`：0 = X11/fcitx5（`code` 是 XKB 键码），1 = macOS（`NSEvent.keyCode`）。
/// `down` 非 0 表示按下。`flags` 带 WOWTYPE_STATS_SENSITIVE 时不算。返回 0 成功，-1 参数错误。
#[no_mangle]
pub unsafe extern "C" fn wowtype_stats_key(
    stats: *mut WowtypeStats,
    platform: c_int,
    code: c_uint,
    down: c_int,
    flags: c_uint,
    now_ms: i64,
) -> c_int {
    let Some(stats) = (unsafe { stats.as_mut() }) else {
        set_last_error("stats is null");
        return -1;
    };
    stats.recorder.key(platform, code, down != 0, flags, now_ms);
    0
}

/// 一次上屏（文字只数各类字符有几个，不留下）。`cand_index` 是候选位置（1 起），不是选候选时为 0。
#[no_mangle]
pub unsafe extern "C" fn wowtype_stats_commit(
    stats: *mut WowtypeStats,
    text: *const c_char,
    source: *const c_char,
    scheme: *const c_char,
    cand_index: c_int,
    flags: c_uint,
    now_ms: i64,
) -> c_int {
    let Some(stats) = (unsafe { stats.as_mut() }) else {
        set_last_error("stats is null");
        return -1;
    };
    if !stats.recorder.enabled() || flags & stats::SENSITIVE != 0 {
        return 0;
    }
    let Some(text) = (unsafe { cstr(text) }) else {
        set_last_error("stats text is null or not utf8");
        return -1;
    };
    let source = unsafe { cstr(source) }.unwrap_or_default();
    let scheme = unsafe { cstr(scheme) }.unwrap_or_default();
    stats
        .recorder
        .commit(&text, &source, &scheme, cand_index.max(0) as u32, flags, now_ms);
    0
}

/// 把攒着的计数交给写文件线程（不等写完；失焦时调）。返回 0。
#[no_mangle]
pub unsafe extern "C" fn wowtype_stats_flush(stats: *mut WowtypeStats, now_ms: i64) -> c_int {
    let Some(stats) = (unsafe { stats.as_mut() }) else {
        set_last_error("stats is null");
        return -1;
    };
    stats.recorder.flush(now_ms);
    0
}

/// 等交出去的都写完。`timeout_ms` < 0 一直等。返回 0 写完了，-1 有写失败的或等超时（原因见 wowtype_last_error）。
#[no_mangle]
pub unsafe extern "C" fn wowtype_stats_wait(stats: *mut WowtypeStats, timeout_ms: i64) -> c_int {
    let Some(stats) = (unsafe { stats.as_mut() }) else {
        set_last_error("stats is null");
        return -1;
    };
    let timeout = (timeout_ms >= 0).then(|| Duration::from_millis(timeout_ms as u64));
    match stats.recorder.wait(timeout) {
        Ok(()) => 0,
        Err(error) => {
            set_last_error(error);
            -1
        }
    }
}

/// 设置界面用：清空统计（`stats.clear` 里的数加一，删掉统计文件）。输入法开着时攒着的旧计数也会丢掉。
#[no_mangle]
pub unsafe extern "C" fn wowtype_stats_clear(path: *const c_char) -> c_int {
    let Some(path) = (unsafe { cstr(path) }) else {
        set_last_error("stats path is null or not utf8");
        return -1;
    };
    match stats::clear(std::path::Path::new(&path)) {
        Ok(()) => 0,
        Err(error) => {
            set_last_error(error);
            -1
        }
    }
}

/// 当前 Unix 毫秒，外壳不必自己算。
#[no_mangle]
pub extern "C" fn wowtype_stats_now_ms() -> i64 {
    stats::now_ms()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    #[test]
    fn ffi_round_trip() {
        let dir = std::env::temp_dir().join(format!("wowtype-stats-ffi-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("stats.json");
        let path = CString::new(file.to_str().unwrap()).unwrap();
        let stats = unsafe { wowtype_stats_new(path.as_ptr()) };
        assert!(!stats.is_null());
        assert_eq!(unsafe { wowtype_stats_enabled(stats) }, 0, "默认关");
        let text = CString::new("你好").unwrap();
        let source = CString::new("quanpin").unwrap();
        let now = wowtype_stats_now_ms();
        let commit = |flags| unsafe {
            wowtype_stats_commit(stats, text.as_ptr(), source.as_ptr(), std::ptr::null(), 1, flags, now)
        };
        assert_eq!(commit(0), 0);
        assert_eq!(unsafe { wowtype_stats_flush(stats, now) }, 0);
        assert_eq!(unsafe { wowtype_stats_wait(stats, -1) }, 0);
        assert!(!file.exists(), "关着时不建文件");

        assert_eq!(unsafe { wowtype_stats_set_enabled(stats, 1) }, 0);
        assert_eq!(unsafe { wowtype_stats_enabled(stats) }, 1);
        assert_eq!(commit(0), 0);
        assert_eq!(commit(WOWTYPE_STATS_SENSITIVE), 0);
        // 键码超过 63 的（这里 8 + 464 = Fn）不出错
        assert_eq!(unsafe { wowtype_stats_key(stats, 0, 472, 1, 0, now) }, 0);
        assert_eq!(unsafe { wowtype_stats_key(stats, 0, 100_000, 1, 0, now) }, 0);
        assert_eq!(unsafe { wowtype_stats_flush(stats, now) }, 0);
        assert_eq!(unsafe { wowtype_stats_wait(stats, 5_000) }, 0);
        let document = stats::Document::load(&file).unwrap();
        let day = &document.days[&stats::local_day(now)];
        assert_eq!(day.chars, 2, "密码框里的那次不算");
        assert_eq!(day.keys.get("Fn"), Some(&1));

        assert_eq!(unsafe { wowtype_stats_clear(path.as_ptr()) }, 0);
        assert!(!file.exists());
        // 清空以后才交出去的这批（攒的时候清空过）整批丢掉
        assert_eq!(commit(0), 0);
        unsafe { wowtype_stats_free(stats) };
        assert!(!file.exists(), "清空以后释放不会把旧的写回去");
        let _ = std::fs::remove_dir_all(&dir);
    }

    const WOWTYPE_STATS_SENSITIVE: c_uint = stats::SENSITIVE;
}
