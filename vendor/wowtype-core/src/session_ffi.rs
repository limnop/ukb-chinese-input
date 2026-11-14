//! 输入会话的 C 接口（`wowtype_session_*`），见 core/include/wowtype_core.h 和 docs/ABI.md。
//!
//! 会话返回的指针（`WowtypeFrame` 和里面的字符串、过渡函数返回的字符串）归会话所有，
//! 在同一个会话的下一次调用前有效。

#![allow(clippy::missing_safety_doc)]

use std::ffi::{CStr, CString};
use std::mem::size_of;
use std::os::raw::{c_char, c_int};
use std::panic;
use std::ptr;

use crate::ffi::{set_last_error, WowtypeEngineHandle};
use crate::session::{
    Candidate, CandidateKind, Config, Frame, Key, KeyAction, Panel, SelectKeys, Session,
};

/// 一次按键（C 布局）。`size` 是外壳编译时的 `sizeof(WowtypeKey)`：
/// 以后在末尾加字段，旧外壳传的 size 小，没传的字段当 0。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct WowtypeKey {
    pub size: u32,
    pub keysym: u32,
    pub modifiers: u32,
    pub release: u32,
    pub prev_char: u32,
    /// `WOWTYPE_KEY_ACTION_*`：外壳认出来的全局热键、候选框上的翻页按钮。
    pub action: u32,
}

pub const WOWTYPE_KEY_ACTION_NONE: u32 = 0;
pub const WOWTYPE_KEY_ACTION_PREV_PAGE: u32 = 1;
pub const WOWTYPE_KEY_ACTION_NEXT_PAGE: u32 = 2;
pub const WOWTYPE_KEY_ACTION_PREV_CANDIDATE: u32 = 3;
pub const WOWTYPE_KEY_ACTION_NEXT_CANDIDATE: u32 = 4;

fn key_action(raw: u32) -> KeyAction {
    match raw {
        WOWTYPE_KEY_ACTION_PREV_PAGE => KeyAction::PrevPage,
        WOWTYPE_KEY_ACTION_NEXT_PAGE => KeyAction::NextPage,
        WOWTYPE_KEY_ACTION_PREV_CANDIDATE => KeyAction::PrevCandidate,
        WOWTYPE_KEY_ACTION_NEXT_CANDIDATE => KeyAction::NextCandidate,
        _ => KeyAction::None,
    }
}

pub const WOWTYPE_SELECT_KEYS_OFF: c_int = 0;
pub const WOWTYPE_SELECT_KEYS_SEMICOLON: c_int = 1;
pub const WOWTYPE_SELECT_KEYS_SEMICOLON_QUOTE: c_int = 2;

/// 会话的设置（C 布局）。外壳没填的字段（size 比我们认识的短）用默认值。
#[repr(C)]
#[derive(Clone, Copy)]
pub struct WowtypeSessionConfig {
    pub size: u32,
    /// 一页几个候选；0 用默认（9）。
    pub page_size: u32,
    pub calculator: c_int,
    pub date_time: c_int,
    pub emoji_suggest: c_int,
    /// 候选横排（默认 1）。
    pub horizontal: c_int,
    /// 翻页键，`WOWTYPE_PAGE_KEYS_*` 位掩码（默认 - =）。
    pub page_keys: u32,
    /// `WOWTYPE_SELECT_KEYS_*`（默认关）。
    pub select_keys: c_int,
    /// 1.8 起：以词定字（`[ ]` 取候选的第一个、最后一个字），默认 1。
    pub pick_char: c_int,
}

impl Default for WowtypeSessionConfig {
    fn default() -> Self {
        let config = Config::default();
        Self {
            size: size_of::<WowtypeSessionConfig>() as u32,
            page_size: config.page_size as u32,
            calculator: c_int::from(config.calculator),
            date_time: c_int::from(config.date_time),
            emoji_suggest: c_int::from(config.emoji_suggest),
            horizontal: c_int::from(config.horizontal),
            page_keys: config.page_keys,
            select_keys: match config.select_keys {
                SelectKeys::Off => WOWTYPE_SELECT_KEYS_OFF,
                SelectKeys::Semicolon => WOWTYPE_SELECT_KEYS_SEMICOLON,
                SelectKeys::SemicolonQuote => WOWTYPE_SELECT_KEYS_SEMICOLON_QUOTE,
            },
            pick_char: c_int::from(config.pick_char),
        }
    }
}

impl WowtypeSessionConfig {
    fn to_config(self) -> Config {
        Config {
            page_size: if self.page_size == 0 {
                Config::default().page_size
            } else {
                self.page_size as usize
            },
            calculator: self.calculator != 0,
            date_time: self.date_time != 0,
            emoji_suggest: self.emoji_suggest != 0,
            horizontal: self.horizontal != 0,
            page_keys: self.page_keys,
            select_keys: match self.select_keys {
                WOWTYPE_SELECT_KEYS_SEMICOLON => SelectKeys::Semicolon,
                WOWTYPE_SELECT_KEYS_SEMICOLON_QUOTE => SelectKeys::SemicolonQuote,
                _ => SelectKeys::Off,
            },
            pick_char: self.pick_char != 0,
        }
    }
}

/// 过渡：候选框的状态（C 布局），fcitx5 外壳按它摆自己的候选列表。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct WowtypeSessionView {
    pub size: u32,
    pub page: usize,
    pub cursor: usize,
    pub expanded: c_int,
    pub grid_cursor: usize,
    /// 候选重算过几次：变了就要重新抄候选。
    pub generation: u64,
}

/// 过渡：一个候选的全部信息（C 布局），字段含义见头文件。
#[repr(C)]
#[derive(Clone, Copy)]
pub struct WowtypeCandidateInfo {
    pub size: u32,
    pub text: *const c_char,
    pub pinyin: *const c_char,
    pub consumed: usize,
    pub kind: c_int,
    pub commit: *const c_char,
    pub emoji: *const c_char,
}

pub const WOWTYPE_CAND_WORD: c_int = 0;
pub const WOWTYPE_CAND_PLAIN: c_int = 1;
pub const WOWTYPE_CAND_EMOJI: c_int = 2;

/// 一个候选的 C 字符串（随候选重算重建）。
struct CCandidate {
    text: CString,
    pinyin: CString,
    commit: CString,
    emoji: CString,
    consumed: usize,
    kind: c_int,
}

impl CCandidate {
    fn info(&self) -> WowtypeCandidateInfo {
        WowtypeCandidateInfo {
            size: 0,
            text: self.text.as_ptr(),
            pinyin: self.pinyin.as_ptr(),
            consumed: self.consumed,
            kind: self.kind,
            commit: self.commit.as_ptr(),
            emoji: self.emoji.as_ptr(),
        }
    }

    fn new(c: &Candidate) -> Self {
        Self {
            text: to_c(&c.text),
            pinyin: to_c(&c.pinyin),
            commit: to_c(&c.commit),
            emoji: to_c(&c.emoji),
            consumed: c.consumed,
            kind: match c.kind {
                CandidateKind::Word => WOWTYPE_CAND_WORD,
                CandidateKind::Plain => WOWTYPE_CAND_PLAIN,
                CandidateKind::Emoji => WOWTYPE_CAND_EMOJI,
            },
        }
    }
}

/// 一次调用之后外壳要做的事（C 布局），字段含义见头文件。
#[repr(C)]
pub struct WowtypeFrame {
    pub size: u32,
    pub handled: c_int,
    pub commit: *const c_char,
    pub preedit: *const c_char,
    pub preedit_cursor: c_int,
    pub aux: *const c_char,
    pub panel: c_int,
    pub cand_count: usize,
    pub cand_cursor: c_int,
    pub has_prev: c_int,
    pub has_next: c_int,
    pub wake_after_ms: u32,
    /// 1.7 起：这次调用忘掉了一个学过的词（Ctrl+Delete），外壳该马上存用户词库和输入习惯。
    pub forgot: c_int,
}

impl Default for WowtypeFrame {
    fn default() -> Self {
        Self {
            size: size_of::<WowtypeFrame>() as u32,
            handled: 0,
            commit: ptr::null(),
            preedit: ptr::null(),
            preedit_cursor: 0,
            aux: ptr::null(),
            panel: 0,
            cand_count: 0,
            cand_cursor: 0,
            has_prev: 0,
            has_next: 0,
            wake_after_ms: 0,
            forgot: 0,
        }
    }
}

/// 会话句柄。引擎借用的是外壳的 `WowtypeEngineHandle`，外壳保证它比会话活得久
/// （或者先 set_engine(NULL)）。
pub struct WowtypeSession {
    session: Session<'static>,
    /// 候选的 C 字符串，和 `c_generation` 那次重算的候选对应。
    c_candidates: Vec<CCandidate>,
    c_generation: u64,
    frame: WowtypeFrame,
    commit: Option<CString>,
    preedit: CString,
    /// frame 里显示的候选在全部候选里的起点（`wowtype_session_candidate` 用）。
    shown_start: usize,
    /// 上一次上屏是选的哪个候选（`wowtype_session_last_choice` 用）。
    last_choice: Option<CCandidate>,
    /// 过渡函数返回的字符串。
    text: CString,
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

/// 按 size 读一个带 size 字段的结构体：外壳给的比我们认识的短，缺的字段当 0。
unsafe fn read_sized<T: Copy + Default>(ptr: *const T, size: u32) -> T {
    let mut value = T::default();
    let len = (size as usize).min(size_of::<T>());
    unsafe {
        ptr::copy_nonoverlapping(ptr as *const u8, &mut value as *mut T as *mut u8, len);
    }
    value
}

/// 按外壳的 out->size 写一个带 size 字段的结构体（size 字段写成实际写了多少字节）。
unsafe fn write_sized<T: Copy>(out: *mut T, mut value: T) {
    let len = (unsafe { *(out as *const u32) } as usize).min(size_of::<T>());
    unsafe {
        *(&mut value as *mut T as *mut u32) = len as u32;
        ptr::copy_nonoverlapping(&value as *const T as *const u8, out as *mut u8, len);
    }
}

impl WowtypeSession {
    fn publish(&mut self, frame: Frame) -> *const WowtypeFrame {
        self.commit = frame.commit.as_deref().map(to_c);
        self.preedit = to_c(&frame.preedit);
        self.shown_start = frame.cand_start;
        self.last_choice = self.session.last_choice().map(CCandidate::new);
        self.frame = WowtypeFrame {
            handled: c_int::from(frame.handled),
            commit: self.commit.as_ref().map_or(ptr::null(), |c| c.as_ptr()),
            preedit: self.preedit.as_ptr(),
            preedit_cursor: frame.preedit_cursor.try_into().unwrap_or(c_int::MAX),
            panel: match frame.panel {
                Panel::Normal => 0,
                Panel::Expanded => 1,
            },
            cand_count: frame.cand_count,
            cand_cursor: frame
                .cand_cursor
                .and_then(|c| c_int::try_from(c).ok())
                .unwrap_or(-1),
            has_prev: c_int::from(frame.has_prev),
            has_next: c_int::from(frame.has_next),
            forgot: c_int::from(frame.forgot),
            ..WowtypeFrame::default()
        };
        &self.frame
    }

    /// 跑一个会改会话的调用，出了 panic 就当没处理。
    fn run(
        &mut self,
        what: &str,
        f: impl FnOnce(&mut Session<'static>) -> Frame,
    ) -> *const WowtypeFrame {
        let inner = &mut self.session;
        match panic::catch_unwind(panic::AssertUnwindSafe(|| f(inner))) {
            Ok(frame) => self.publish(frame),
            Err(_) => {
                set_last_error(&format!("session {what} panicked"));
                self.current(false)
            }
        }
    }

    fn text(&mut self, text: &str) -> *const c_char {
        self.text = to_c(text);
        self.text.as_ptr()
    }

    /// 现在的编辑区；算候选时出了 panic 就当没有候选。
    fn current(&mut self, handled: bool) -> *const WowtypeFrame {
        let inner = &mut self.session;
        let frame = match panic::catch_unwind(panic::AssertUnwindSafe(|| inner.frame(handled))) {
            Ok(frame) => frame,
            Err(_) => {
                set_last_error("session panicked");
                let preedit = self.session.selected_text() + self.session.remaining();
                Frame {
                    handled,
                    preedit_cursor: preedit.len(),
                    preedit,
                    ..Frame::default()
                }
            }
        };
        self.publish(frame)
    }

    fn c_candidates(&mut self) -> &[CCandidate] {
        let inner = &mut self.session;
        let computed = panic::catch_unwind(panic::AssertUnwindSafe(|| {
            inner.candidates();
        }));
        if computed.is_err() {
            set_last_error("session panicked");
            self.c_candidates.clear();
            return &self.c_candidates;
        }
        if self.session.generation() != self.c_generation {
            self.c_candidates = self
                .session
                .candidates()
                .iter()
                .map(CCandidate::new)
                .collect();
            self.c_generation = self.session.generation();
        }
        &self.c_candidates
    }
}

unsafe fn engine_ref(engine: *mut WowtypeEngineHandle) -> Option<&'static crate::Engine> {
    unsafe { engine.as_ref() }.map(|handle| &handle.engine)
}

/// 建会话。engine 可以传 NULL（外壳的词库在后台载入，载入好之前也要能打字，载入好了
/// 用 set_engine 补上）；传了的话要比会话活得久。config 可以传 NULL（全用默认）。
#[no_mangle]
pub unsafe extern "C" fn wowtype_session_new(
    engine: *mut WowtypeEngineHandle,
    config: *const WowtypeSessionConfig,
) -> *mut WowtypeSession {
    let mut session = Session::new();
    session.set_engine(unsafe { engine_ref(engine) });
    if !config.is_null() {
        session.configure(unsafe { read_sized(config, (*config).size) }.to_config());
    }
    let mut handle = Box::new(WowtypeSession {
        session,
        c_candidates: Vec::new(),
        c_generation: 0,
        frame: WowtypeFrame::default(),
        commit: None,
        preedit: CString::default(),
        shown_start: 0,
        last_choice: None,
        text: CString::default(),
    });
    handle.current(false);
    Box::into_raw(handle)
}

/// 换引擎（词库在后台载入好了），可以传 NULL。
#[no_mangle]
pub unsafe extern "C" fn wowtype_session_set_engine(
    session: *mut WowtypeSession,
    engine: *mut WowtypeEngineHandle,
) {
    if let Some(handle) = unsafe { session.as_mut() } {
        handle.session.set_engine(unsafe { engine_ref(engine) });
    }
}

/// 改设置。config 为 NULL 时全用默认。
#[no_mangle]
pub unsafe extern "C" fn wowtype_session_configure(
    session: *mut WowtypeSession,
    config: *const WowtypeSessionConfig,
) {
    let Some(handle) = (unsafe { session.as_mut() }) else {
        return;
    };
    let config = if config.is_null() {
        Config::default()
    } else {
        unsafe { read_sized(config, (*config).size) }.to_config()
    };
    handle.session.configure(config);
}

/// 现在的编辑区（不处理按键，handled 为 0）。session 为 NULL 时返回 NULL。
#[no_mangle]
pub unsafe extern "C" fn wowtype_session_frame(
    session: *mut WowtypeSession,
) -> *const WowtypeFrame {
    match unsafe { session.as_mut() } {
        Some(handle) => handle.current(false),
        None => ptr::null(),
    }
}

#[no_mangle]
pub unsafe extern "C" fn wowtype_session_free(session: *mut WowtypeSession) {
    if !session.is_null() {
        drop(unsafe { Box::from_raw(session) });
    }
}

/// 处理一次按键。session 或 key 为 NULL 时返回 NULL。
#[no_mangle]
pub unsafe extern "C" fn wowtype_session_key(
    session: *mut WowtypeSession,
    key: *const WowtypeKey,
) -> *const WowtypeFrame {
    let Some(handle) = (unsafe { session.as_mut() }) else {
        return ptr::null();
    };
    if key.is_null() {
        set_last_error("key is null");
        return ptr::null();
    }
    let raw = unsafe { read_sized(key, (*key).size) };
    let key = Key {
        keysym: raw.keysym,
        modifiers: raw.modifiers,
        release: raw.release != 0,
        prev_char: char::from_u32(raw.prev_char).filter(|&c| c != '\0'),
        action: key_action(raw.action),
    };
    handle.run("key", |session| session.key(&key))
}

/// 选候选框里显示的第 index 个（一页，或展开面板的一屏；鼠标点候选）。序号不对时 handled 为 0。
#[no_mangle]
pub unsafe extern "C" fn wowtype_session_select(
    session: *mut WowtypeSession,
    index: usize,
) -> *const WowtypeFrame {
    let Some(handle) = (unsafe { session.as_mut() }) else {
        return ptr::null();
    };
    handle.run("select", |session| session.select(index))
}

/// 上一次返回的 frame 里显示的第 index 个候选（0 起，小于 cand_count）。越界或 NULL 返回 NULL。
#[no_mangle]
pub unsafe extern "C" fn wowtype_session_candidate(
    session: *mut WowtypeSession,
    index: usize,
) -> *const c_char {
    let Some(handle) = (unsafe { session.as_mut() }) else {
        return ptr::null();
    };
    if index >= handle.frame.cand_count {
        return ptr::null();
    }
    let at = handle.shown_start + index;
    handle
        .c_candidates()
        .get(at)
        .map_or(ptr::null(), |c| c.text.as_ptr())
}

/// 清空编辑区；commit_pending 非 0 时把已选的词和原始输入放进 frame->commit。
#[no_mangle]
pub unsafe extern "C" fn wowtype_session_reset(
    session: *mut WowtypeSession,
    commit_pending: c_int,
) -> *const WowtypeFrame {
    let Some(handle) = (unsafe { session.as_mut() }) else {
        return ptr::null();
    };
    handle.run("reset", |session| session.reset(commit_pending != 0))
}

// ---- 过渡函数：外壳还没挪完时用，挪完以后删掉（见 docs/ABI.md）----

/// 全部原始输入（含已选词用掉的部分）。
#[no_mangle]
pub unsafe extern "C" fn wowtype_session_input(session: *mut WowtypeSession) -> *const c_char {
    let Some(handle) = (unsafe { session.as_mut() }) else {
        return ptr::null();
    };
    let text = handle.session.input().to_owned();
    handle.text(&text)
}

/// 还没选词的那部分原始输入。
#[no_mangle]
pub unsafe extern "C" fn wowtype_session_remaining(session: *mut WowtypeSession) -> *const c_char {
    let Some(handle) = (unsafe { session.as_mut() }) else {
        return ptr::null();
    };
    let text = handle.session.remaining().to_owned();
    handle.text(&text)
}

/// 已选的词连起来。
#[no_mangle]
pub unsafe extern "C" fn wowtype_session_selected_text(
    session: *mut WowtypeSession,
) -> *const c_char {
    let Some(handle) = (unsafe { session.as_mut() }) else {
        return ptr::null();
    };
    let text = handle.session.selected_text();
    handle.text(&text)
}

/// 选第 index 个候选（全部候选里的序号，不分页）：句首词进已选栈，其余上屏并记选词，
/// 见 `Session::choose`。序号不对时 handled 为 0。
#[no_mangle]
pub unsafe extern "C" fn wowtype_session_choose(
    session: *mut WowtypeSession,
    index: usize,
) -> *const WowtypeFrame {
    let Some(handle) = (unsafe { session.as_mut() }) else {
        return ptr::null();
    };
    handle.run("choose", |session| session.choose(index))
}

/// 候选框的状态写进 out（按 out->size 写）。返回 0 成功，-1 参数为 NULL。
#[no_mangle]
pub unsafe extern "C" fn wowtype_session_view(
    session: *mut WowtypeSession,
    out: *mut WowtypeSessionView,
) -> c_int {
    let (Some(handle), false) = (unsafe { session.as_mut() }, out.is_null()) else {
        return -1;
    };
    let inner = &mut handle.session;
    let Ok(view) = panic::catch_unwind(panic::AssertUnwindSafe(|| inner.view())) else {
        set_last_error("session view panicked");
        return -1;
    };
    let value = WowtypeSessionView {
        size: 0,
        page: view.page,
        cursor: view.cursor,
        expanded: c_int::from(view.expanded),
        grid_cursor: view.grid_cursor,
        generation: handle.session.generation(),
    };
    unsafe { write_sized(out, value) };
    0
}

/// 上一次返回的 frame 上屏的字是选哪个候选来的，写进 out（按 out->size 写）。
/// 返回 0 是选候选上屏的；-1：原样上屏输入、没上屏，或参数为 NULL。
#[no_mangle]
pub unsafe extern "C" fn wowtype_session_last_choice(
    session: *mut WowtypeSession,
    out: *mut WowtypeCandidateInfo,
) -> c_int {
    let (Some(handle), false) = (unsafe { session.as_mut() }, out.is_null()) else {
        return -1;
    };
    let Some(c) = handle.last_choice.as_ref() else {
        return -1;
    };
    unsafe { write_sized(out, c.info()) };
    0
}

/// 往原始输入后面加字（计算器、双拼分号、表情面板搜索词）。
#[no_mangle]
pub unsafe extern "C" fn wowtype_session_append(session: *mut WowtypeSession, text: *const c_char) {
    if let Some(handle) = unsafe { session.as_mut() } {
        let text = unsafe { arg(text) };
        handle.session.append(&text);
    }
}

/// 光标前已经上屏的字（外壳记的上文）。
#[no_mangle]
pub unsafe extern "C" fn wowtype_session_set_context(
    session: *mut WowtypeSession,
    text: *const c_char,
) {
    if let Some(handle) = unsafe { session.as_mut() } {
        let text = unsafe { arg(text) };
        handle.session.set_context(&text);
    }
}

/// 全部候选的个数（不分页）。
#[no_mangle]
pub unsafe extern "C" fn wowtype_session_candidate_total(session: *mut WowtypeSession) -> usize {
    unsafe { session.as_mut() }.map_or(0, |handle| handle.c_candidates().len())
}

/// 第 index 个候选（不分页）写进 out（按 out->size 写）。返回 0 成功，-1 越界或参数为 NULL。
/// 字符串在同一会话下一次改了编辑区或设置的调用前有效。
#[no_mangle]
pub unsafe extern "C" fn wowtype_session_candidate_info(
    session: *mut WowtypeSession,
    index: usize,
    out: *mut WowtypeCandidateInfo,
) -> c_int {
    let Some(handle) = (unsafe { session.as_mut() }) else {
        return -1;
    };
    if out.is_null() {
        return -1;
    }
    let Some(c) = handle.c_candidates().get(index) else {
        return -1;
    };
    unsafe { write_sized(out, c.info()) };
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::keysym;

    fn press(session: *mut WowtypeSession, keysym: u32) -> &'static WowtypeFrame {
        let key = WowtypeKey {
            size: size_of::<WowtypeKey>() as u32,
            keysym,
            ..WowtypeKey::default()
        };
        unsafe { &*wowtype_session_key(session, &key) }
    }

    fn c(ptr: *const c_char) -> String {
        assert!(!ptr.is_null());
        unsafe { CStr::from_ptr(ptr) }.to_str().unwrap().to_owned()
    }

    #[test]
    fn roundtrip() {
        let session = unsafe { wowtype_session_new(ptr::null_mut(), ptr::null()) };
        assert!(!session.is_null());
        for ch in "nihaoma".bytes() {
            assert_eq!(press(session, u32::from(ch)).handled, 1);
        }
        assert_eq!(c(unsafe { wowtype_session_remaining(session) }), "nihaoma");
        // 没有引擎：没有候选，选不了
        assert_eq!(unsafe { wowtype_session_candidate_total(session) }, 0);
        assert_eq!(unsafe { &*wowtype_session_choose(session, 0) }.handled, 0);

        let frame = press(session, keysym::BACKSPACE);
        assert_eq!(frame.handled, 1);
        assert!(frame.commit.is_null());
        assert_eq!(c(frame.preedit), "nihaom");
        assert_eq!(frame.size as usize, size_of::<WowtypeFrame>());

        let frame = unsafe { &*wowtype_session_reset(session, 1) };
        assert_eq!(c(frame.commit), "nihaom");
        assert_eq!(c(frame.preedit), "");
        unsafe { wowtype_session_free(session) };
    }

    #[test]
    fn short_key_struct_is_zero_extended() {
        // 旧外壳的 WowtypeKey 只到 keysym：后面的字段当 0（没有修饰键、按下）
        let session = unsafe { wowtype_session_new(ptr::null_mut(), ptr::null()) };
        let key = WowtypeKey {
            size: 8,
            keysym: u32::from(b'a'),
            modifiers: 0xffff,
            release: 1,
            prev_char: 0,
            action: 0,
        };
        let frame = unsafe { &*wowtype_session_key(session, &key) };
        assert_eq!(frame.handled, 1);
        assert_eq!(c(frame.preedit), "a");
        unsafe { wowtype_session_free(session) };
    }

    #[test]
    fn null_handles() {
        assert!(unsafe { wowtype_session_key(ptr::null_mut(), ptr::null()) }.is_null());
        assert!(unsafe { wowtype_session_reset(ptr::null_mut(), 0) }.is_null());
        assert_eq!(
            unsafe { wowtype_session_candidate_total(ptr::null_mut()) },
            0
        );
        assert!(unsafe { wowtype_session_choose(ptr::null_mut(), 0) }.is_null());
        assert!(unsafe { wowtype_session_select(ptr::null_mut(), 0) }.is_null());
        assert!(unsafe { wowtype_session_candidate(ptr::null_mut(), 0) }.is_null());
        assert_eq!(
            unsafe { wowtype_session_view(ptr::null_mut(), ptr::null_mut()) },
            -1
        );
        assert_eq!(
            unsafe { wowtype_session_last_choice(ptr::null_mut(), ptr::null_mut()) },
            -1
        );
        unsafe { wowtype_session_free(ptr::null_mut()) };
    }
}
