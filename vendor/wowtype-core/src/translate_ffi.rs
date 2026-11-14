//! 互译的 C 接口，见 core/include/wowtype_core.h。
//!
//! 插件线程只调这些函数，都不阻塞：请求在后台线程里跑（见 [`crate::translate`]）。
//! poll 把当前结果拍一份快照存在句柄里，取文字的函数返回的指针在下一次 poll 前有效。

#![allow(clippy::missing_safety_doc)]

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_uint};
use std::path::PathBuf;

use crate::translate::{direction_tag, JobState, TranslateConfig, Translator};

pub const TRANSLATE_IDLE: c_int = 0;
pub const TRANSLATE_RUNNING: c_int = 1;
pub const TRANSLATE_DONE: c_int = 2;
pub const TRANSLATE_ERROR: c_int = 3;

static EMPTY: [u8; 1] = [0];

fn empty() -> *const c_char {
    EMPTY.as_ptr() as *const c_char
}

fn to_c(text: &str) -> CString {
    CString::new(text.replace('\0', " ")).unwrap_or_default()
}

unsafe fn arg(ptr: *const c_char) -> String {
    if ptr.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned()
}

pub struct WowtypeTranslator {
    translator: Translator,
    state: c_int,
    source: CString,
    text: CString,
    error: CString,
    tag: CString,
}

#[no_mangle]
pub extern "C" fn wowtype_translate_new() -> *mut WowtypeTranslator {
    Box::into_raw(Box::new(WowtypeTranslator {
        translator: Translator::new(),
        state: TRANSLATE_IDLE,
        source: CString::default(),
        text: CString::default(),
        error: CString::default(),
        tag: CString::default(),
    }))
}

#[no_mangle]
pub unsafe extern "C" fn wowtype_translate_free(handle: *mut WowtypeTranslator) {
    if !handle.is_null() {
        drop(unsafe { Box::from_raw(handle) });
    }
}

/// 改配置。字符串可为 NULL（当空串）；key_file 为 NULL/空串表示不带 Key。
#[no_mangle]
pub unsafe extern "C" fn wowtype_translate_configure(
    handle: *mut WowtypeTranslator,
    base_url: *const c_char,
    model: *const c_char,
    target: *const c_char,
    formal: c_int,
    key_file: *const c_char,
) {
    let Some(handle) = (unsafe { handle.as_mut() }) else {
        return;
    };
    let key_file = unsafe { arg(key_file) };
    let target = unsafe { arg(target) };
    // 本地模型的设置单独由 wowtype_translate_configure_local 改，这里保留
    let local = handle.translator.config().local;
    handle.translator.configure(TranslateConfig {
        base_url: unsafe { arg(base_url) },
        model: unsafe { arg(model) },
        target: if target.trim().is_empty() {
            "en".to_string()
        } else {
            target
        },
        formal: formal != 0,
        key_file: (!key_file.is_empty()).then(|| PathBuf::from(key_file)),
        local,
    });
}

/// 用本地模型翻译（ABI 1.1 起）：model 是 GGUF 文件；server 是 llama-server 的路径或名字
/// （NULL/空串表示在 PATH 里找 llama-server）；闲置 idle_secs 秒后停掉（0 表示不停）。
/// model 为 NULL/空串时改回用 wowtype_translate_configure 配的接口，并马上停掉本地服务。
/// llama-server 第一次翻译时才启动，只监听 127.0.0.1。
#[no_mangle]
pub unsafe extern "C" fn wowtype_translate_configure_local(
    handle: *mut WowtypeTranslator,
    model: *const c_char,
    server: *const c_char,
    idle_secs: c_uint,
) {
    let Some(handle) = (unsafe { handle.as_mut() }) else {
        return;
    };
    let model = unsafe { arg(model) };
    let mut config = handle.translator.config();
    config.local = (!model.trim().is_empty()).then(|| crate::llama::LocalConfig {
        model: crate::llama::expand_home(model.trim()),
        server: {
            let server = unsafe { arg(server) };
            let server = server.trim();
            if server.starts_with("~/") {
                crate::llama::expand_home(server)
                    .to_string_lossy()
                    .into_owned()
            } else {
                server.to_string()
            }
        },
        idle_secs: u64::from(idle_secs),
    });
    handle.translator.configure(config);
}

/// 本地模型的 llama-server 现在在跑时返回 1（ABI 1.1 起）。
#[no_mangle]
pub unsafe extern "C" fn wowtype_translate_local_running(
    handle: *const WowtypeTranslator,
) -> c_int {
    match unsafe { handle.as_ref() } {
        Some(handle) => c_int::from(handle.translator.local_running()),
        None => 0,
    }
}

/// 填了地址和模型时返回 1。
#[no_mangle]
pub unsafe extern "C" fn wowtype_translate_ready(handle: *const WowtypeTranslator) -> c_int {
    match unsafe { handle.as_ref() } {
        Some(handle) => c_int::from(handle.translator.config().is_ready()),
        None => 0,
    }
}

/// 开始翻译 text（取消别的请求）；正在译或已经译好同一句话时什么也不做。
#[no_mangle]
pub unsafe extern "C" fn wowtype_translate_start(
    handle: *mut WowtypeTranslator,
    text: *const c_char,
) {
    if let Some(handle) = unsafe { handle.as_mut() } {
        handle.translator.start(&unsafe { arg(text) });
    }
}

#[no_mangle]
pub unsafe extern "C" fn wowtype_translate_cancel(handle: *mut WowtypeTranslator) {
    if let Some(handle) = unsafe { handle.as_mut() } {
        handle.translator.cancel();
    }
}

/// 取一份当前结果的快照，返回 WOWTYPE_TRANSLATE_*。
#[no_mangle]
pub unsafe extern "C" fn wowtype_translate_poll(handle: *mut WowtypeTranslator) -> c_int {
    let Some(handle) = (unsafe { handle.as_mut() }) else {
        return TRANSLATE_IDLE;
    };
    let snap = handle.translator.poll();
    handle.state = match snap.state {
        JobState::Idle => TRANSLATE_IDLE,
        JobState::Running => TRANSLATE_RUNNING,
        JobState::Done => TRANSLATE_DONE,
        JobState::Error => TRANSLATE_ERROR,
    };
    handle.source = to_c(&snap.source);
    handle.text = to_c(&snap.text);
    handle.error = to_c(&snap.error);
    handle.state
}

/// 快照里到目前的译文（状态是 RUNNING 时可能还没写完；没有就是空串）。
#[no_mangle]
pub unsafe extern "C" fn wowtype_translate_text(handle: *const WowtypeTranslator) -> *const c_char {
    unsafe { handle.as_ref() }.map_or(empty(), |h| h.text.as_ptr())
}

/// 快照对应的原文。
#[no_mangle]
pub unsafe extern "C" fn wowtype_translate_source(
    handle: *const WowtypeTranslator,
) -> *const c_char {
    unsafe { handle.as_ref() }.map_or(empty(), |h| h.source.as_ptr())
}

/// 出错时给用户看的原因。
#[no_mangle]
pub unsafe extern "C" fn wowtype_translate_error(
    handle: *const WowtypeTranslator,
) -> *const c_char {
    unsafe { handle.as_ref() }.map_or(empty(), |h| h.error.as_ptr())
}

/// 面板上的方向标签（「中→英」「英→中」），按 text 判断；下一次调用前有效。
#[no_mangle]
pub unsafe extern "C" fn wowtype_translate_tag(
    handle: *mut WowtypeTranslator,
    text: *const c_char,
) -> *const c_char {
    let Some(handle) = (unsafe { handle.as_mut() }) else {
        return empty();
    };
    let tag = direction_tag(&handle.translator.config(), &unsafe { arg(text) });
    handle.tag = to_c(&tag);
    handle.tag.as_ptr()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ptr;

    #[test]
    fn ffi_without_config() {
        let handle = wowtype_translate_new();
        unsafe {
            assert_eq!(wowtype_translate_ready(handle), 0);
            let url = CString::new("http://127.0.0.1:1/v1").unwrap();
            let model = CString::new("m").unwrap();
            let ja = CString::new("ja").unwrap();
            wowtype_translate_configure(
                handle,
                url.as_ptr(),
                model.as_ptr(),
                ja.as_ptr(),
                0,
                ptr::null(),
            );
            assert_eq!(wowtype_translate_ready(handle), 1);
            let text = CString::new("你好").unwrap();
            let tag = CStr::from_ptr(wowtype_translate_tag(handle, text.as_ptr()));
            assert_eq!(tag.to_str().unwrap(), "中→日");
            assert_eq!(wowtype_translate_poll(handle), TRANSLATE_IDLE);
            assert_eq!(
                CStr::from_ptr(wowtype_translate_text(handle)).to_bytes(),
                b""
            );
            wowtype_translate_cancel(handle);
            wowtype_translate_free(handle);
            // 空句柄不崩
            assert_eq!(wowtype_translate_poll(ptr::null_mut()), TRANSLATE_IDLE);
            wowtype_translate_free(ptr::null_mut());
        }
    }
}
