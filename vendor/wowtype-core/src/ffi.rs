//! C ABI：供 fcitx5 的 C++ 插件通过 cdylib 调用。
//!
//! 设计成“不透明句柄 + 候选句柄”，避免 C++ 侧解析 JSON。字符串一律 UTF-8。

// 安全契约见 core/include/wowtype_core.h：空指针、句柄生命周期由调用方保证。
#![allow(clippy::missing_safety_doc)]

use std::cell::RefCell;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::panic;
use std::ptr;
use std::sync::Arc;

use crate::lexicon::{Lexicon, MemoryLexicon};
use crate::lm::{BigramLm, LanguageModel};
use crate::Engine;

thread_local! {
    static LAST_ERROR: RefCell<CString> =
        RefCell::new(CString::new("no error").expect("static error string"));
}

pub(crate) fn set_last_error(message: impl Into<String>) {
    let text = message.into().replace('\0', " ");
    let value = CString::new(text)
        .unwrap_or_else(|_| CString::new("unknown error").expect("static error string"));
    LAST_ERROR.with(|slot| *slot.borrow_mut() = value);
}

unsafe fn cstr_to_str<'a>(ptr: *const c_char) -> Option<&'a str> {
    if ptr.is_null() {
        return None;
    }
    unsafe { CStr::from_ptr(ptr) }.to_str().ok()
}

/// 引擎句柄。
pub struct WowtypeEngineHandle {
    pub(crate) engine: Engine,
}

/// 一次输入产生的候选集合（预测结果也用这个结构承载）。
pub struct WowtypeCandidates {
    texts: Vec<CString>,
    scores: Vec<f64>,
    /// 候选对应的拼音（空格分隔音节）；预测、符号结果为空串。
    pinyins: Vec<CString>,
    /// 候选用掉了输入的多少字节；整句候选等于输入长度。
    consumed: Vec<usize>,
    /// 选中后上屏的文字（emoji 推荐用）；空表示就是 text。
    commits: Vec<CString>,
}

impl WowtypeCandidates {
    /// 只有文本和分数的结果（预测、符号）。
    fn plain(texts: Vec<CString>, scores: Vec<f64>) -> Self {
        let n = texts.len();
        Self {
            texts,
            scores,
            pinyins: vec![CString::default(); n],
            consumed: vec![0; n],
            commits: Vec::new(),
        }
    }
}

/// 创建引擎。
///
/// lexicon_tsv_path 必填；bigram_tsv_path 可为 NULL/空串，表示不加载 LM。
/// 失败返回 NULL，错误信息用 wowtype_last_error 获取。
#[no_mangle]
pub unsafe extern "C" fn wowtype_engine_new(
    lexicon_tsv_path: *const c_char,
    bigram_tsv_path: *const c_char,
) -> *mut WowtypeEngineHandle {
    let lexicon_path = match unsafe { cstr_to_str(lexicon_tsv_path) } {
        Some(path) if !path.is_empty() => path,
        _ => {
            set_last_error("lexicon path is null or empty");
            return ptr::null_mut();
        }
    };

    let result = (|| -> anyhow::Result<Engine> {
        let lexicon = MemoryLexicon::from_tsv_path_cached(lexicon_path)?;
        let lexicon: Arc<dyn Lexicon> = Arc::new(lexicon);
        let lm: Arc<dyn LanguageModel> = match unsafe { cstr_to_str(bigram_tsv_path) } {
            Some(path) if !path.is_empty() => crate::lm::load_lm(path)?,
            _ => Arc::new(BigramLm::default()),
        };
        Ok(Engine::new(lexicon, lm))
    })();

    match result {
        Ok(engine) => Box::into_raw(Box::new(WowtypeEngineHandle { engine })),
        Err(error) => {
            set_last_error(format!("{error:#}"));
            ptr::null_mut()
        }
    }
}

/// 释放引擎。
#[no_mangle]
pub unsafe extern "C" fn wowtype_engine_free(engine: *mut WowtypeEngineHandle) {
    if !engine.is_null() {
        drop(unsafe { Box::from_raw(engine) });
    }
}

/// 设置 beam size；返回 0 成功，-1 失败。
#[no_mangle]
pub unsafe extern "C" fn wowtype_engine_set_beam_size(
    engine: *mut WowtypeEngineHandle,
    beam_size: u32,
) -> c_int {
    match unsafe { engine.as_mut() } {
        Some(handle) => {
            handle.engine.set_beam_size(beam_size as usize);
            0
        }
        None => {
            set_last_error("engine handle is null");
            -1
        }
    }
}

/// 输入拼音串，返回候选句柄；失败返回 NULL。
#[no_mangle]
pub unsafe extern "C" fn wowtype_engine_input(
    engine: *mut WowtypeEngineHandle,
    input: *const c_char,
) -> *mut WowtypeCandidates {
    let handle = match unsafe { engine.as_ref() } {
        Some(handle) => handle,
        None => {
            set_last_error("engine handle is null");
            return ptr::null_mut();
        }
    };
    let input = match unsafe { cstr_to_str(input) } {
        Some(input) => input,
        None => {
            set_last_error("input is null or not valid UTF-8");
            return ptr::null_mut();
        }
    };

    let outcome = panic::catch_unwind(panic::AssertUnwindSafe(|| handle.engine.try_input(input)));
    match outcome {
        Ok(Ok(candidates)) => {
            let mut texts = Vec::with_capacity(candidates.len());
            let mut scores = Vec::with_capacity(candidates.len());
            let mut pinyins = Vec::with_capacity(candidates.len());
            let mut consumed = Vec::with_capacity(candidates.len());
            for candidate in candidates {
                let text = candidate.text.replace('\0', " ");
                texts.push(CString::new(text).unwrap_or_default());
                scores.push(candidate.score);
                pinyins.push(CString::new(candidate.pinyin.replace('\0', " ")).unwrap_or_default());
                consumed.push(candidate.consumed.min(input.len()));
            }
            Box::into_raw(Box::new(WowtypeCandidates {
                texts,
                scores,
                pinyins,
                consumed,
                commits: Vec::new(),
            }))
        }
        Ok(Err(error)) => {
            set_last_error(format!("{error:#}"));
            ptr::null_mut()
        }
        Err(_) => {
            set_last_error("Rust panic while processing input");
            ptr::null_mut()
        }
    }
}

/// 基于上一词的预测（预测词库）。
///
/// 成功返回候选集合句柄（text 是预测词，score 是对数概率），
/// 失败返回 NULL；结果同样用 wowtype_candidates_free 释放。
#[no_mangle]
pub unsafe extern "C" fn wowtype_engine_predict_next(
    engine: *const WowtypeEngineHandle,
    prev_word: *const c_char,
    limit: usize,
) -> *mut WowtypeCandidates {
    let outcome = panic::catch_unwind(panic::AssertUnwindSafe(|| {
        if engine.is_null() {
            set_last_error("engine is null");
            return ptr::null_mut();
        }
        let Some(prev) = (unsafe { cstr_to_str(prev_word) }) else {
            set_last_error("prev_word is null or not utf8");
            return ptr::null_mut();
        };
        let items = unsafe { &*engine }.engine.predict_next(prev, limit);
        let mut texts = Vec::with_capacity(items.len());
        let mut scores = Vec::with_capacity(items.len());
        for (word, score) in items {
            match CString::new(word) {
                Ok(text) => {
                    texts.push(text);
                    scores.push(score);
                }
                Err(_) => continue,
            }
        }
        Box::into_raw(Box::new(WowtypeCandidates::plain(texts, scores)))
    }));
    match outcome {
        Ok(candidates) => candidates,
        Err(_) => {
            set_last_error("panic in wowtype_engine_predict_next");
            ptr::null_mut()
        }
    }
}

/// 记录一次选词，用于历史特征。返回 0 表示成功。
#[no_mangle]
pub unsafe extern "C" fn wowtype_engine_record_selection(
    engine: *mut WowtypeEngineHandle,
    word: *const c_char,
) -> c_int {
    if engine.is_null() {
        set_last_error("engine is null");
        return -1;
    }
    let Some(word) = (unsafe { cstr_to_str(word) }) else {
        set_last_error("word is null or not utf8");
        return -1;
    };
    unsafe { &*engine }.engine.record_selection(word);
    0
}

/// 设置光标前文本，用于上下文 bigram 特征。返回 0 表示成功。
#[no_mangle]
pub unsafe extern "C" fn wowtype_engine_set_context(
    engine: *mut WowtypeEngineHandle,
    text: *const c_char,
) -> c_int {
    if engine.is_null() {
        set_last_error("engine is null");
        return -1;
    }
    let Some(text) = (unsafe { cstr_to_str(text) }) else {
        set_last_error("context is null or not utf8");
        return -1;
    };
    unsafe { &*engine }.engine.set_context(text);
    0
}

/// 添加用户词。返回 0 成功，-1 失败（拼音非法等）。
#[no_mangle]
pub unsafe extern "C" fn wowtype_engine_add_user_word(
    engine: *mut WowtypeEngineHandle,
    pinyin: *const c_char,
    word: *const c_char,
) -> c_int {
    if engine.is_null() {
        set_last_error("engine is null");
        return -1;
    }
    let (Some(pinyin), Some(word)) = (unsafe { cstr_to_str(pinyin) }, unsafe { cstr_to_str(word) })
    else {
        set_last_error("pinyin or word is null/not utf8");
        return -1;
    };
    match unsafe { &*engine }
        .engine
        .add_user_word(pinyin, word, 5000.0)
    {
        Ok(()) => 0,
        Err(e) => {
            set_last_error(format!("{e:#}"));
            -1
        }
    }
}

/// 删除用户词。返回 1 删掉了，0 没找到，-1 出错。
#[no_mangle]
pub unsafe extern "C" fn wowtype_engine_remove_user_word(
    engine: *mut WowtypeEngineHandle,
    word: *const c_char,
) -> c_int {
    if engine.is_null() {
        set_last_error("engine is null");
        return -1;
    }
    let Some(word) = (unsafe { cstr_to_str(word) }) else {
        set_last_error("word is null or not utf8");
        return -1;
    };
    if unsafe { &*engine }.engine.remove_user_word(word) {
        1
    } else {
        0
    }
}

/// 保存用户词到 TSV。返回 0 成功，-1 失败。
#[no_mangle]
pub unsafe extern "C" fn wowtype_engine_save_user_dict(
    engine: *mut WowtypeEngineHandle,
    path: *const c_char,
) -> c_int {
    if engine.is_null() {
        set_last_error("engine is null");
        return -1;
    }
    let Some(path) = (unsafe { cstr_to_str(path) }) else {
        set_last_error("path is null or not utf8");
        return -1;
    };
    match unsafe { &*engine }.engine.save_user_dict(path) {
        Ok(()) => 0,
        Err(e) => {
            set_last_error(format!("{e:#}"));
            -1
        }
    }
}

/// 从 TSV 载入用户词，返回载入条数，-1 表示出错。
#[no_mangle]
pub unsafe extern "C" fn wowtype_engine_load_user_dict(
    engine: *mut WowtypeEngineHandle,
    path: *const c_char,
) -> c_int {
    if engine.is_null() {
        set_last_error("engine is null");
        return -1;
    }
    let Some(path) = (unsafe { cstr_to_str(path) }) else {
        set_last_error("path is null or not utf8");
        return -1;
    };
    match unsafe { &*engine }.engine.load_user_dict(path) {
        Ok(n) => n as c_int,
        Err(e) => {
            set_last_error(format!("{e:#}"));
            -1
        }
    }
}

/// 读自定义词文件（`拼音<TAB>词[<TAB>频次]`，设置界面写的 custom.tsv），整体替换上一次读到的；
/// 文件不存在就清空。返回收下的条数，-1 出错。
#[no_mangle]
pub unsafe extern "C" fn wowtype_engine_load_custom_words(
    engine: *mut WowtypeEngineHandle,
    path: *const c_char,
) -> c_int {
    if engine.is_null() {
        set_last_error("engine is null");
        return -1;
    }
    let Some(path) = (unsafe { cstr_to_str(path) }) else {
        set_last_error("path is null or not utf8");
        return -1;
    };
    match unsafe { &*engine }.engine.load_custom_words(path) {
        Ok(n) => n as c_int,
        Err(e) => {
            set_last_error(format!("{e:#}"));
            -1
        }
    }
}

/// 保存用户输入习惯（选过的词的热度、词搭配）到 TSV。返回 0 成功，-1 失败。
#[no_mangle]
pub unsafe extern "C" fn wowtype_engine_save_history(
    engine: *mut WowtypeEngineHandle,
    path: *const c_char,
) -> c_int {
    if engine.is_null() {
        set_last_error("engine is null");
        return -1;
    }
    let Some(path) = (unsafe { cstr_to_str(path) }) else {
        set_last_error("path is null or not utf8");
        return -1;
    };
    match unsafe { &*engine }.engine.save_history(path) {
        Ok(()) => 0,
        Err(e) => {
            set_last_error(format!("{e:#}"));
            -1
        }
    }
}

/// 读回用户输入习惯，返回条数，-1 表示出错（文件不存在也是 -1）。
#[no_mangle]
pub unsafe extern "C" fn wowtype_engine_load_history(
    engine: *mut WowtypeEngineHandle,
    path: *const c_char,
) -> c_int {
    if engine.is_null() {
        set_last_error("engine is null");
        return -1;
    }
    let Some(path) = (unsafe { cstr_to_str(path) }) else {
        set_last_error("path is null or not utf8");
        return -1;
    };
    match unsafe { &*engine }.engine.load_history(path) {
        Ok(n) => n as c_int,
        Err(e) => {
            set_last_error(format!("{e:#}"));
            -1
        }
    }
}

/// 每次输入最多返回多少个整句候选（句首词候选另算）。返回 0 成功，-1 失败。
#[no_mangle]
pub unsafe extern "C" fn wowtype_engine_set_max_candidates(
    engine: *mut WowtypeEngineHandle,
    max_candidates: u32,
) -> c_int {
    if engine.is_null() {
        set_last_error("engine is null");
        return -1;
    }
    unsafe { &mut *engine }
        .engine
        .set_max_candidates(max_candidates as usize);
    0
}

/// 开关按错键纠错（相邻键、两键按反、多按/漏按）。返回 0 成功，-1 失败。
#[no_mangle]
pub unsafe extern "C" fn wowtype_engine_set_typo_correction(
    engine: *mut WowtypeEngineHandle,
    on: c_int,
) -> c_int {
    if engine.is_null() {
        set_last_error("engine is null");
        return -1;
    }
    unsafe { &mut *engine }.engine.set_typo_correction(on != 0);
    0
}

/// 是否也出 CJK 扩展 B 区及以后的单字（ABI 1.9 起，默认不出；通用规范汉字表里的照常）。返回 0 成功，-1 失败。
#[no_mangle]
pub unsafe extern "C" fn wowtype_engine_set_rare_ext_chars(
    engine: *mut WowtypeEngineHandle,
    on: c_int,
) -> c_int {
    if engine.is_null() {
        set_last_error("engine is null");
        return -1;
    }
    unsafe { &mut *engine }.engine.set_rare_ext_chars(on != 0);
    0
}

/// 载入英文词表（每行一个词，或 rime 的 en.dict.yaml），返回累计词条数；失败返回 -1。
#[no_mangle]
pub unsafe extern "C" fn wowtype_engine_load_english(
    engine: *mut WowtypeEngineHandle,
    path: *const c_char,
) -> c_int {
    if engine.is_null() {
        set_last_error("engine is null");
        return -1;
    }
    let Some(path) = (unsafe { cstr_to_str(path) }) else {
        set_last_error("path is null or not utf8");
        return -1;
    };
    match unsafe { &*engine }.engine.load_english(path) {
        Ok(n) => n.min(c_int::MAX as usize) as c_int,
        Err(e) => {
            set_last_error(format!("{e:#}"));
            -1
        }
    }
}

/// 载入字级神经语言模型（微信输入法的 nlrgpt2_nnlm.bin）。返回字表大小，失败返回 -1。
#[no_mangle]
pub unsafe extern "C" fn wowtype_engine_load_nnlm(
    engine: *mut WowtypeEngineHandle,
    path: *const c_char,
) -> c_int {
    if engine.is_null() {
        set_last_error("engine is null");
        return -1;
    }
    let Some(path) = (unsafe { cstr_to_str(path) }) else {
        set_last_error("path is null or not utf8");
        return -1;
    };
    match unsafe { &*engine }.engine.load_nnlm(path) {
        Ok(n) => n.min(c_int::MAX as usize) as c_int,
        Err(e) => {
            set_last_error(format!("{e:#}"));
            -1
        }
    }
}

/// 开关模糊音（z/zh、an/ang 等）。返回 0 成功，-1 失败。
#[no_mangle]
pub unsafe extern "C" fn wowtype_engine_set_fuzzy(
    engine: *mut WowtypeEngineHandle,
    on: c_int,
) -> c_int {
    if engine.is_null() {
        set_last_error("engine is null");
        return -1;
    }
    unsafe { &mut *engine }.engine.set_fuzzy(on != 0);
    0
}

/// 设置打开的模糊音分组（位掩码：1 z/zh，2 c/ch，4 s/sh，8 n/l，16 f/h，32 r/l，
/// 64 an/ang，128 en/eng，256 in/ing，512 ian/iang，1024 uan/uang，2048 un/ong）。总开关仍是 wowtype_engine_set_fuzzy。返回 0 成功，-1 失败。
#[no_mangle]
pub unsafe extern "C" fn wowtype_engine_set_fuzzy_groups(
    engine: *mut WowtypeEngineHandle,
    groups: u32,
) -> c_int {
    if engine.is_null() {
        set_last_error("engine is null");
        return -1;
    }
    unsafe { &mut *engine }.engine.set_fuzzy_groups(groups);
    0
}

/// 开关双拼（方案用 wowtype_engine_set_shuangpin_scheme 选）。返回 0 成功，-1 失败。
#[no_mangle]
pub unsafe extern "C" fn wowtype_engine_set_shuangpin(
    engine: *mut WowtypeEngineHandle,
    on: c_int,
) -> c_int {
    if engine.is_null() {
        set_last_error("engine is null");
        return -1;
    }
    unsafe { &mut *engine }.engine.set_shuangpin(on != 0);
    0
}

/// 选双拼方案：ziranma xiaohe microsoft sogou abc ziguang pinyinjiajia（也认中文名）。
/// 返回 0 成功，-1 失败（不认识的方案保持原样）。
#[no_mangle]
pub unsafe extern "C" fn wowtype_engine_set_shuangpin_scheme(
    engine: *mut WowtypeEngineHandle,
    name: *const c_char,
) -> c_int {
    if engine.is_null() {
        set_last_error("engine is null");
        return -1;
    }
    let Some(scheme) = unsafe { cstr_to_str(name) }.and_then(crate::shuangpin::Scheme::from_name)
    else {
        set_last_error("unknown shuangpin scheme");
        return -1;
    };
    unsafe { &mut *engine }.engine.set_shuangpin_scheme(scheme);
    0
}

/// 这个双拼方案有没有用分号键（微软、搜狗、紫光的 ing）。不认识的方案返回 0。
#[no_mangle]
pub unsafe extern "C" fn wowtype_shuangpin_uses_semicolon(name: *const c_char) -> c_int {
    unsafe { cstr_to_str(name) }
        .and_then(crate::shuangpin::Scheme::from_name)
        .is_some_and(|s| s.uses_semicolon())
        .into()
}

/// 双拼时编辑区显示的全拼：vsgo -> zhong'guo。没开双拼返回 NULL。
/// 返回的串在同一线程下一次调用 wowtype_segment_input 或本函数前有效。
#[no_mangle]
pub unsafe extern "C" fn wowtype_engine_shuangpin_display(
    engine: *const WowtypeEngineHandle,
    input: *const c_char,
) -> *const c_char {
    let (Some(engine), Some(input)) = (unsafe { engine.as_ref() }, unsafe { cstr_to_str(input) })
    else {
        return ptr::null();
    };
    match engine
        .engine
        .shuangpin_display(input)
        .and_then(|s| CString::new(s).ok())
    {
        Some(text) => SEGMENTED.with(|slot| {
            *slot.borrow_mut() = text;
            slot.borrow().as_ptr()
        }),
        None => ptr::null(),
    }
}

/// 开关繁体输出。返回 0 成功，-1 失败。
#[no_mangle]
pub unsafe extern "C" fn wowtype_engine_set_traditional(
    engine: *mut WowtypeEngineHandle,
    on: c_int,
) -> c_int {
    if engine.is_null() {
        set_last_error("engine is null");
        return -1;
    }
    unsafe { &mut *engine }.engine.set_traditional(on != 0);
    0
}

/// 载入简繁映射表（简体<TAB>繁体），返回条目数；失败返回 -1。
#[no_mangle]
pub unsafe extern "C" fn wowtype_engine_load_conversion(
    engine: *mut WowtypeEngineHandle,
    path: *const c_char,
) -> c_int {
    if engine.is_null() {
        set_last_error("engine is null");
        return -1;
    }
    let Some(path) = (unsafe { cstr_to_str(path) }) else {
        set_last_error("path is null or not utf8");
        return -1;
    };
    match unsafe { &*engine }.engine.load_conversion(path) {
        Ok(n) => n as c_int,
        Err(e) => {
            set_last_error(format!("{e:#}"));
            -1
        }
    }
}

thread_local! {
    static PUNCT_BUF: RefCell<CString> =
        RefCell::new(CString::new("，").expect("static punct"));
}

/// ASCII 标点转中文标点。返回的指针在下一次调用前有效，未收录返回 NULL。
#[no_mangle]
pub extern "C" fn wowtype_fullwidth_punct(code: c_int) -> *const c_char {
    let Some(ch) = char::from_u32(code as u32) else {
        return ptr::null();
    };
    let Some(text) = crate::symbols::fullwidth_punct(ch) else {
        return ptr::null();
    };
    PUNCT_BUF.with(|slot| {
        *slot.borrow_mut() = CString::new(text).unwrap_or_default();
        slot.borrow().as_ptr()
    })
}

/// 按标点样式转换。style：0 全角、1 半角；prev 是光标前一个字符（不知道传 0）。
/// 返回 NULL 表示原样上屏这个 ASCII 字符。指针在下一次调用前有效。
#[no_mangle]
pub extern "C" fn wowtype_punct(code: c_int, style: c_int, prev: c_int) -> *const c_char {
    let Some(ch) = char::from_u32(code as u32) else {
        return ptr::null();
    };
    let style = if style == 1 {
        crate::symbols::PunctStyle::Halfwidth
    } else {
        crate::symbols::PunctStyle::Fullwidth
    };
    let prev = if prev > 0 {
        char::from_u32(prev as u32)
    } else {
        None
    };
    let Some(text) = crate::symbols::punct(ch, style, prev) else {
        return ptr::null();
    };
    PUNCT_BUF.with(|slot| {
        *slot.borrow_mut() = CString::new(text).unwrap_or_default();
        slot.borrow().as_ptr()
    })
}

/// 中英文之间自动补空格（ABI 1.2 起）：prev 是光标前一个字，next 是要上屏的第一个字，
/// flags：1 汉字和字母之间、2 汉字和数字之间。要补空格返回 1，否则 0。
#[no_mangle]
pub extern "C" fn wowtype_autospace(prev: c_int, next: c_int, flags: c_int) -> c_int {
    let (Some(prev), Some(next)) = (char::from_u32(prev as u32), char::from_u32(next as u32))
    else {
        return 0;
    };
    c_int::from(crate::autospace::needs_space(prev, next, flags as u32))
}

/// 全部内置符号（ABI 1.2 起），按内置顺序：工具栏的「符号」面板用。
#[no_mangle]
pub extern "C" fn wowtype_symbol_list() -> *mut WowtypeCandidates {
    let texts: Vec<CString> = crate::symbols::SYMBOLS
        .iter()
        .filter_map(|s| CString::new(s.text).ok())
        .collect();
    let scores = vec![0.0; texts.len()];
    Box::into_raw(Box::new(WowtypeCandidates::plain(texts, scores)))
}

/// 记下一条剪贴板文字（ABI 1.2 起；只在内存里，最多 20 条，最新的在前，重复的挪到最前）。
/// 空白、超过 16 KB 的不记。记下返回 1，否则 0。
#[no_mangle]
pub unsafe extern "C" fn wowtype_clipboard_remember(text: *const c_char) -> c_int {
    match unsafe { cstr_to_str(text) } {
        Some(text) => c_int::from(crate::clipboard::remember(text)),
        None => 0,
    }
}

/// 剪贴板历史（ABI 1.2 起），最新的在前。
#[no_mangle]
pub extern "C" fn wowtype_clipboard_history() -> *mut WowtypeCandidates {
    let texts: Vec<CString> = crate::clipboard::history()
        .into_iter()
        .filter_map(|t| CString::new(t).ok())
        .collect();
    let scores = vec![0.0; texts.len()];
    Box::into_raw(Box::new(WowtypeCandidates::plain(texts, scores)))
}

/// 清空剪贴板历史（ABI 1.2 起）。
#[no_mangle]
pub extern "C" fn wowtype_clipboard_clear() {
    crate::clipboard::clear();
}

/// 自定义词（常用语）按 custom.tsv 里的顺序（ABI 1.2 起）。text 是词（`\n` 等转义原样显示），
/// pinyin 是编码，wowtype_candidates_commit 是上屏的文字（转义换成了真的换行）。
#[no_mangle]
pub unsafe extern "C" fn wowtype_engine_custom_phrases(
    engine: *const WowtypeEngineHandle,
) -> *mut WowtypeCandidates {
    if engine.is_null() {
        set_last_error("engine is null");
        return ptr::null_mut();
    }
    let mut out = WowtypeCandidates::plain(Vec::new(), Vec::new());
    for (code, word) in unsafe { &*engine }.engine.lexicon().custom_phrases() {
        let (Ok(text), Ok(code), Ok(commit)) = (
            CString::new(word.as_str()),
            CString::new(code),
            CString::new(crate::unescape_custom(&word)),
        ) else {
            continue;
        };
        out.texts.push(text);
        out.scores.push(0.0);
        out.pinyins.push(code);
        out.consumed.push(0);
        out.commits.push(commit);
    }
    Box::into_raw(Box::new(out))
}

/// 符号检索。query 为拼音或中文名前缀，结果放在候选结构里（score 恒为 0）。
#[no_mangle]
pub unsafe extern "C" fn wowtype_symbol_search(
    query: *const c_char,
    limit: usize,
) -> *mut WowtypeCandidates {
    let Some(query) = (unsafe { cstr_to_str(query) }) else {
        set_last_error("query is null or not utf8");
        return ptr::null_mut();
    };
    let mut texts = Vec::new();
    let mut scores = Vec::new();
    for symbol in crate::symbols::search(query, limit) {
        match CString::new(symbol.text) {
            Ok(text) => {
                texts.push(text);
                scores.push(0.0);
            }
            Err(_) => continue,
        }
    }
    Box::into_raw(Box::new(WowtypeCandidates::plain(texts, scores)))
}

/// 候选正好是数字字（零到十）时对应的按键 emoji（「八」-> 8️⃣），别的返回 NULL。
/// 返回的是静态字符串，不用释放。
#[no_mangle]
pub unsafe extern "C" fn wowtype_emoji_keycap(text: *const c_char) -> *const c_char {
    match unsafe { cstr_to_str(text) }.and_then(crate::emoji::keycap_c) {
        Some(emoji) => emoji.as_ptr(),
        None => ptr::null(),
    }
}

/// 打字时的 emoji 推荐：text 是首选候选的文字。结果的 text 是显示文字，
/// 上屏文字用 wowtype_candidates_commit 取（整个词对上时只有 emoji，句尾对上时是整句 + emoji）。
#[no_mangle]
pub unsafe extern "C" fn wowtype_emoji_recommend(
    text: *const c_char,
    limit: usize,
) -> *mut WowtypeCandidates {
    let Some(text) = (unsafe { cstr_to_str(text) }) else {
        set_last_error("text is null or not utf8");
        return ptr::null_mut();
    };
    let suggestions = crate::emoji::recommend(text, limit);
    let mut texts = Vec::new();
    let mut commits = Vec::new();
    for suggestion in suggestions {
        if let (Ok(display), Ok(commit)) = (
            CString::new(suggestion.display),
            CString::new(suggestion.commit),
        ) {
            texts.push(display);
            commits.push(commit);
        }
    }
    let scores = vec![0.0; texts.len()];
    let mut out = WowtypeCandidates::plain(texts, scores);
    out.commits = commits;
    Box::into_raw(Box::new(out))
}

/// 日期、时间候选：key 是触发词（riqi / shijian / xingqi / date / time，可带隔音符），
/// 时间字段由调用方按本地时区给出（weekday 0 = 星期日）。不是触发词时返回空列表。
#[no_mangle]
pub unsafe extern "C" fn wowtype_datetime_candidates(
    key: *const c_char,
    year: c_int,
    month: c_int,
    day: c_int,
    hour: c_int,
    minute: c_int,
    second: c_int,
    weekday: c_int,
) -> *mut WowtypeCandidates {
    let Some(key) = (unsafe { cstr_to_str(key) }) else {
        set_last_error("key is null or not utf8");
        return ptr::null_mut();
    };
    let now = crate::datetime::LocalTime {
        year,
        month: month.max(0) as u32,
        day: day.max(0) as u32,
        hour: hour.max(0) as u32,
        minute: minute.max(0) as u32,
        second: second.max(0) as u32,
        weekday: weekday.max(0) as u32,
    };
    plain_candidates(crate::datetime::candidates(key, &now))
}

/// V 模式计算器：expr 是 v 后面的算式。算得出来时给「结果」「算式=结果」，
/// 没写完时只给算式本身。
#[no_mangle]
pub unsafe extern "C" fn wowtype_calc_candidates(expr: *const c_char) -> *mut WowtypeCandidates {
    let Some(expr) = (unsafe { cstr_to_str(expr) }) else {
        set_last_error("expr is null or not utf8");
        return ptr::null_mut();
    };
    plain_candidates(crate::calc::candidates(expr))
}

fn plain_candidates(list: Vec<String>) -> *mut WowtypeCandidates {
    let texts: Vec<CString> = list
        .into_iter()
        .filter_map(|t| CString::new(t).ok())
        .collect();
    let scores = vec![0.0; texts.len()];
    Box::into_raw(Box::new(WowtypeCandidates::plain(texts, scores)))
}

/// 表情面板的搜索：query 为空给全部 emoji；否则按拼音（经引擎转成中文词）、英文和
/// 符号名一起找。engine 可为 NULL（只按英文和符号名找）。
#[no_mangle]
pub unsafe extern "C" fn wowtype_engine_emoji_search(
    engine: *const WowtypeEngineHandle,
    query: *const c_char,
    limit: usize,
) -> *mut WowtypeCandidates {
    let Some(query) = (unsafe { cstr_to_str(query) }) else {
        set_last_error("query is null or not utf8");
        return ptr::null_mut();
    };
    let query = query.trim();
    let results: Vec<String> = if query.is_empty() {
        crate::emoji::all()
            .into_iter()
            .map(str::to_string)
            .collect()
    } else {
        let mut words: Vec<String> = Vec::new();
        if let Some(handle) = unsafe { engine.as_ref() } {
            if query.bytes().all(|b| b.is_ascii_alphabetic() || b == b'\'') {
                let outcome =
                    panic::catch_unwind(panic::AssertUnwindSafe(|| handle.engine.try_input(query)));
                if let Ok(Ok(candidates)) = outcome {
                    words = candidates
                        .into_iter()
                        .filter(|c| c.consumed >= query.len() && !c.text.is_ascii())
                        .map(|c| c.text)
                        .take(8)
                        .collect();
                }
            } else if !query.is_ascii() {
                words.push(query.to_string());
            }
        }
        let refs: Vec<&str> = words.iter().map(String::as_str).collect();
        let mut out: Vec<String> = Vec::new();
        let add = |items: Vec<&str>, out: &mut Vec<String>| {
            for item in items {
                if !out.iter().any(|x| x == item) {
                    out.push(item.to_string());
                }
            }
        };
        let symbols = crate::symbols::search(query, limit);
        let (named, prefixed): (Vec<_>, Vec<_>) =
            symbols.into_iter().partition(|s| s.names.contains(&query));
        // 符号名完全对上的、关键词完全对上的 emoji、以名字开头的符号、英文、关键词里包含这个词的
        add(named.into_iter().map(|s| s.text).collect(), &mut out);
        add(crate::emoji::search_words_exact(&refs, limit), &mut out);
        add(prefixed.into_iter().map(|s| s.text).collect(), &mut out);
        add(crate::emoji::search_english(query, limit), &mut out);
        add(
            crate::emoji::search_words_containing(&refs, limit),
            &mut out,
        );
        out
    };
    let texts: Vec<CString> = results
        .into_iter()
        .take(limit.max(1))
        .filter_map(|t| CString::new(t).ok())
        .collect();
    let scores = vec![0.0; texts.len()];
    Box::into_raw(Box::new(WowtypeCandidates::plain(texts, scores)))
}

/// 选中后上屏的文字：emoji 推荐的结果可能和显示的不一样，其他候选就是 text。越界返回 NULL。
#[no_mangle]
pub unsafe extern "C" fn wowtype_candidates_commit(
    candidates: *const WowtypeCandidates,
    index: usize,
) -> *const c_char {
    unsafe { candidates.as_ref() }
        .and_then(|c| c.commits.get(index).or_else(|| c.texts.get(index)))
        .map(|text| text.as_ptr())
        .unwrap_or(ptr::null())
}

/// 记录选词并可携带拼音：选够次数后自动加入用户词库。pinyin 可为 NULL。
#[no_mangle]
pub unsafe extern "C" fn wowtype_engine_record_selection_ex(
    engine: *mut WowtypeEngineHandle,
    word: *const c_char,
    pinyin: *const c_char,
) -> c_int {
    if engine.is_null() {
        set_last_error("engine is null");
        return -1;
    }
    let Some(word) = (unsafe { cstr_to_str(word) }) else {
        set_last_error("word is null or not utf8");
        return -1;
    };
    let pinyin = unsafe { cstr_to_str(pinyin) }.filter(|p| !p.is_empty());
    unsafe { &*engine }
        .engine
        .record_selection_with_pinyin(word, pinyin);
    0
}

/// 候选数量。
#[no_mangle]
pub unsafe extern "C" fn wowtype_candidates_len(candidates: *const WowtypeCandidates) -> usize {
    unsafe { candidates.as_ref() }
        .map(|c| c.texts.len())
        .unwrap_or(0)
}

/// 候选文本（UTF-8，以 NUL 结尾）。索引越界返回 NULL。
#[no_mangle]
pub unsafe extern "C" fn wowtype_candidates_text(
    candidates: *const WowtypeCandidates,
    index: usize,
) -> *const c_char {
    unsafe { candidates.as_ref() }
        .and_then(|c| c.texts.get(index))
        .map(|text| text.as_ptr())
        .unwrap_or(ptr::null())
}

/// 候选分数。索引越界返回 0。
#[no_mangle]
pub unsafe extern "C" fn wowtype_candidates_score(
    candidates: *const WowtypeCandidates,
    index: usize,
) -> f64 {
    unsafe { candidates.as_ref() }
        .and_then(|c| c.scores.get(index).copied())
        .unwrap_or(0.0)
}

/// 候选的拼音（空格分隔，按纠错后的音节）；没有拼音或越界返回空串/NULL。
#[no_mangle]
pub unsafe extern "C" fn wowtype_candidates_pinyin(
    candidates: *const WowtypeCandidates,
    index: usize,
) -> *const c_char {
    unsafe { candidates.as_ref() }
        .and_then(|c| c.pinyins.get(index))
        .map(|text| text.as_ptr())
        .unwrap_or(ptr::null())
}

/// 候选用掉了输入的多少字节。小于输入长度时是句首词候选：
/// 上屏这个词后，剩下的输入（从这个偏移开始）继续留在编辑区。越界返回 0。
#[no_mangle]
pub unsafe extern "C" fn wowtype_candidates_consumed(
    candidates: *const WowtypeCandidates,
    index: usize,
) -> usize {
    unsafe { candidates.as_ref() }
        .and_then(|c| c.consumed.get(index).copied())
        .unwrap_or(0)
}

/// 释放候选集合。
#[no_mangle]
pub unsafe extern "C" fn wowtype_candidates_free(candidates: *mut WowtypeCandidates) {
    if !candidates.is_null() {
        drop(unsafe { Box::from_raw(candidates) });
    }
}

thread_local! {
    static SEGMENTED: RefCell<CString> = RefCell::new(CString::default());
}

/// 按候选拼音（空格分隔）给输入分词：hengxiang + "heng xiang" -> heng'xiang。
/// 切不了（英文、双拼、差得太多）返回 NULL；返回的串在同一线程下一次调用前有效。
#[no_mangle]
pub unsafe extern "C" fn wowtype_segment_input(
    input: *const c_char,
    pinyin: *const c_char,
) -> *const c_char {
    let (Some(input), Some(pinyin)) = (unsafe { cstr_to_str(input) }, unsafe {
        cstr_to_str(pinyin)
    }) else {
        return ptr::null();
    };
    match crate::preedit::segment_input(input, pinyin).and_then(|s| CString::new(s).ok()) {
        Some(text) => SEGMENTED.with(|slot| {
            *slot.borrow_mut() = text;
            slot.borrow().as_ptr()
        }),
        None => ptr::null(),
    }
}

/// 最近一次错误。返回的指针在下一次调用本 API 前有效。
#[no_mangle]
pub extern "C" fn wowtype_last_error() -> *const c_char {
    LAST_ERROR.with(|slot| slot.borrow().as_ptr())
}

/// C 接口版本：主版本 * 1000 + 次版本。
#[no_mangle]
pub extern "C" fn wowtype_abi_version() -> c_int {
    ABI_VERSION_MAJOR * 1000 + ABI_VERSION_MINOR
}

/// C 接口版本，和 `wowtype_core.h` 里的 `WOWTYPE_ABI_VERSION_*` 一致（测试会比对）。
pub const ABI_VERSION_MAJOR: c_int = 1;
pub const ABI_VERSION_MINOR: c_int = 9;

/// WowType 版本号字符串。
#[no_mangle]
pub extern "C" fn wowtype_version() -> *const c_char {
    concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr() as *const c_char
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ffi_roundtrip() {
        let lexicon = concat!(env!("CARGO_MANIFEST_DIR"), "/data/demo_lexicon.tsv");
        let lexicon = CString::new(lexicon).unwrap();
        let engine = unsafe { wowtype_engine_new(lexicon.as_ptr(), ptr::null()) };
        assert!(!engine.is_null(), "engine_new 失败: 见 last_error");

        let input = CString::new("xian").unwrap();
        let candidates = unsafe { wowtype_engine_input(engine, input.as_ptr()) };
        assert!(!candidates.is_null());
        let count = unsafe { wowtype_candidates_len(candidates) };
        assert!(count >= 2, "xian 应至少有 先/西安 两个候选，实际 {count}");

        let first = unsafe { wowtype_candidates_text(candidates, 0) };
        assert!(!first.is_null());
        let first = unsafe { CStr::from_ptr(first) }.to_str().unwrap();
        assert!(!first.is_empty());

        let len = unsafe { wowtype_candidates_consumed(candidates, 0) };
        assert_eq!(len, 4, "整句候选用掉全部输入");
        let pinyin = unsafe { wowtype_candidates_pinyin(candidates, 0) };
        assert!(!pinyin.is_null());
        assert!(unsafe { wowtype_candidates_pinyin(candidates, 99) }.is_null());

        unsafe {
            wowtype_candidates_free(candidates);
            wowtype_engine_free(engine);
        }

        let version = wowtype_version();
        assert!(!version.is_null());
    }

    #[test]
    fn abi_version_matches_header() {
        let header = include_str!("../include/wowtype_core.h");
        let read = |name: &str| -> c_int {
            header
                .lines()
                .find_map(|line| line.strip_prefix(&format!("#define {name} ")))
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(-1)
        };
        assert_eq!(read("WOWTYPE_ABI_VERSION_MAJOR"), ABI_VERSION_MAJOR);
        assert_eq!(read("WOWTYPE_ABI_VERSION_MINOR"), ABI_VERSION_MINOR);
        assert_eq!(wowtype_abi_version(), 1009);
    }
}

#[cfg(test)]
mod toolbar_tests {
    use super::*;

    unsafe fn texts(c: *mut WowtypeCandidates) -> Vec<(String, String, String)> {
        assert!(!c.is_null());
        let n = unsafe { wowtype_candidates_len(c) };
        let get = |p: *const c_char| unsafe { CStr::from_ptr(p) }.to_str().unwrap().to_string();
        let out = (0..n)
            .map(|i| unsafe {
                (
                    get(wowtype_candidates_text(c, i)),
                    get(wowtype_candidates_pinyin(c, i)),
                    get(wowtype_candidates_commit(c, i)),
                )
            })
            .collect();
        unsafe { wowtype_candidates_free(c) };
        out
    }

    #[test]
    fn custom_phrases_in_file_order() {
        let lexicon = concat!(env!("CARGO_MANIFEST_DIR"), "/data/demo_lexicon.tsv");
        let lexicon = CString::new(lexicon).unwrap();
        let engine = unsafe { wowtype_engine_new(lexicon.as_ptr(), ptr::null()) };
        assert!(!engine.is_null());
        unsafe { &*engine }.engine.lexicon().set_custom_tsv(
            "# 注释\ndz\t北京市海淀区\\n100080\nni hao\t妮好\n\t空码\nbad1\t不收\n",
        );
        let got = unsafe { texts(wowtype_engine_custom_phrases(engine)) };
        assert_eq!(
            got,
            vec![
                (
                    "北京市海淀区\\n100080".to_string(),
                    "dz".to_string(),
                    "北京市海淀区\n100080".to_string()
                ),
                ("妮好".to_string(), "ni hao".to_string(), "妮好".to_string()),
            ]
        );
        assert!(unsafe { wowtype_engine_custom_phrases(ptr::null()) }.is_null());
        unsafe { wowtype_engine_free(engine) };
    }

    #[test]
    fn clipboard_and_symbols() {
        let _guard = crate::clipboard::TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        wowtype_clipboard_clear();
        let a = CString::new("第一条").unwrap();
        let b = CString::new("second").unwrap();
        assert_eq!(unsafe { wowtype_clipboard_remember(a.as_ptr()) }, 1);
        assert_eq!(unsafe { wowtype_clipboard_remember(b.as_ptr()) }, 1);
        assert_eq!(unsafe { wowtype_clipboard_remember(ptr::null()) }, 0);
        let got: Vec<String> = unsafe { texts(wowtype_clipboard_history()) }
            .into_iter()
            .map(|t| t.0)
            .collect();
        assert_eq!(got, vec!["second", "第一条"]);
        wowtype_clipboard_clear();
        assert!(unsafe { texts(wowtype_clipboard_history()) }.is_empty());

        let symbols = unsafe { texts(wowtype_symbol_list()) };
        assert_eq!(symbols.len(), crate::symbols::SYMBOLS.len());
        assert_eq!(symbols[0].0, "，");
    }
}
