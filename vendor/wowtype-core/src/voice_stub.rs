//! 不带 voice 特性编译时的语音接口：`wowtype_voice_new` 返回 NULL，其余什么也不做。
//! 这样头文件不用分两套，插件见到 NULL 就不接语音热键。

#![allow(clippy::missing_safety_doc)]

use std::os::raw::{c_char, c_int};

pub struct WowtypeVoice;

static EMPTY: [u8; 1] = [0];

#[no_mangle]
pub unsafe extern "C" fn wowtype_voice_new(_model_dir: *const c_char) -> *mut WowtypeVoice {
    std::ptr::null_mut()
}
#[no_mangle]
pub unsafe extern "C" fn wowtype_voice_free(_voice: *mut WowtypeVoice) {}
#[no_mangle]
pub unsafe extern "C" fn wowtype_voice_key_down(_voice: *mut WowtypeVoice) {}
#[no_mangle]
pub unsafe extern "C" fn wowtype_voice_key_up(_voice: *mut WowtypeVoice) {}
#[no_mangle]
pub unsafe extern "C" fn wowtype_voice_cancel(_voice: *mut WowtypeVoice) {}
#[no_mangle]
pub unsafe extern "C" fn wowtype_voice_poll(_voice: *mut WowtypeVoice) -> c_int {
    0
}
#[no_mangle]
pub unsafe extern "C" fn wowtype_voice_text(_voice: *const WowtypeVoice) -> *const c_char {
    EMPTY.as_ptr() as *const c_char
}
#[no_mangle]
pub unsafe extern "C" fn wowtype_voice_level(_voice: *const WowtypeVoice) -> c_int {
    0
}
#[no_mangle]
pub unsafe extern "C" fn wowtype_voice_model_state(_voice: *const WowtypeVoice) -> c_int {
    0
}
#[no_mangle]
pub unsafe extern "C" fn wowtype_voice_is_tap(_voice: *const WowtypeVoice) -> c_int {
    0
}
