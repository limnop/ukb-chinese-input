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
use crate::symbols::PunctStyle;
use crate::session::{
    ArrowKeys, Candidate, CandidateKind, Config, Frame, Key, KeyAction, Mode, Panel, SelectKeys,
    Session, ShiftToggle, Tool,
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
    /// 1.10 起：`WOWTYPE_KEY_HINT_*`。
    pub hints: u32,
    /// 1.10 起：按键码，不知道给 0。
    pub code: u32,
    /// 1.10 起：按住不放的自动重复。
    pub repeat: u32,
    /// 1.10 起：按键时刻（毫秒，单调递增），不知道给 0。
    pub time_ms: u64,
    /// 1.12 起：应用里光标后一个字符，读不到给 0（带 `WOWTYPE_KEY_HINT_NEXT_CHAR` 时 0 是光标后没有字）。
    pub next_char: u32,
}

pub const WOWTYPE_KEY_HINT_NO_AUTO_SPACE: u32 = crate::session::key_hints::NO_AUTO_SPACE;
pub const WOWTYPE_KEY_HINT_PANEL_OPEN: u32 = crate::session::key_hints::PANEL_OPEN;
pub const WOWTYPE_KEY_HINT_NO_TRANSLATE: u32 = crate::session::key_hints::NO_TRANSLATE;
pub const WOWTYPE_KEY_HINT_SURROUNDING: u32 = crate::session::key_hints::SURROUNDING;
pub const WOWTYPE_KEY_HINT_NO_LEARN: u32 = crate::session::key_hints::NO_LEARN;
pub const WOWTYPE_KEY_HINT_NEXT_CHAR: u32 = crate::session::key_hints::NEXT_CHAR;

/// 外壳自己的翻译器（C 布局，1.10 起）：macOS 用它把自己的翻译器（测试里是假的）交给会话。
/// 函数都不阻塞。poll 把这次的原文、译文、出错原因写进三个出参（在下一次 poll 前有效），
/// 返回 `WOWTYPE_TRANSLATE_*`；tag 返回方向标签（下一次 tag 前有效）。
#[repr(C)]
#[derive(Clone, Copy)]
pub struct WowtypeTranslateBackend {
    pub size: u32,
    pub user: *mut std::ffi::c_void,
    pub start: Option<unsafe extern "C" fn(user: *mut std::ffi::c_void, text: *const c_char)>,
    pub cancel: Option<unsafe extern "C" fn(user: *mut std::ffi::c_void)>,
    pub poll: Option<
        unsafe extern "C" fn(
            user: *mut std::ffi::c_void,
            source: *mut *const c_char,
            text: *mut *const c_char,
            error: *mut *const c_char,
        ) -> c_int,
    >,
    pub tag: Option<unsafe extern "C" fn(user: *mut std::ffi::c_void, text: *const c_char) -> *const c_char>,
}

impl Default for WowtypeTranslateBackend {
    fn default() -> Self {
        Self {
            size: 0,
            user: ptr::null_mut(),
            start: None,
            cancel: None,
            poll: None,
            tag: None,
        }
    }
}

/// 把外壳的函数表包成会话用的翻译器。
struct CBackend(WowtypeTranslateBackend);

impl crate::session::TranslateBackend for CBackend {
    fn start(&self, text: &str) {
        if let Some(start) = self.0.start {
            let text = to_c(text);
            unsafe { start(self.0.user, text.as_ptr()) };
        }
    }
    fn cancel(&self) {
        if let Some(cancel) = self.0.cancel {
            unsafe { cancel(self.0.user) };
        }
    }
    fn poll(&self) -> crate::translate::Snapshot {
        use crate::translate::JobState;
        let (mut source, mut text, mut error) = (ptr::null(), ptr::null(), ptr::null());
        let state = match self.0.poll {
            Some(poll) => unsafe { poll(self.0.user, &mut source, &mut text, &mut error) },
            None => 0,
        };
        crate::translate::Snapshot {
            state: match state {
                1 => JobState::Running,
                2 => JobState::Done,
                3 => JobState::Error,
                _ => JobState::Idle,
            },
            source: unsafe { arg(source) },
            text: unsafe { arg(text) },
            error: unsafe { arg(error) },
        }
    }
    fn tag(&self, text: &str) -> String {
        match self.0.tag {
            Some(tag) => {
                let text = to_c(text);
                unsafe { arg(tag(self.0.user, text.as_ptr())) }
            }
            None => String::new(),
        }
    }
}

pub const WOWTYPE_PANEL_NORMAL: c_int = 0;
pub const WOWTYPE_PANEL_EXPANDED: c_int = 1;
pub const WOWTYPE_PANEL_EMOJI: c_int = 2;
pub const WOWTYPE_PANEL_TOOL: c_int = 3;
/// 1.11 起：上屏以后的联想（`WowtypeSessionConfig.association` 开着时才有）。
pub const WOWTYPE_PANEL_ASSOCIATION: c_int = 4;

pub const WOWTYPE_TOOL_NONE: c_int = 0;
pub const WOWTYPE_TOOL_MENU: c_int = 1;
pub const WOWTYPE_TOOL_CLIPBOARD: c_int = 2;
pub const WOWTYPE_TOOL_PHRASES: c_int = 3;
pub const WOWTYPE_TOOL_SYMBOLS: c_int = 4;
pub const WOWTYPE_TOOL_EMOJI: c_int = 5;

pub const WOWTYPE_MODE_CHINESE: c_int = 0;
pub const WOWTYPE_MODE_ENGLISH: c_int = 1;
pub const WOWTYPE_MODE_TRANSLATE: c_int = 2;

fn mode_to_c(mode: Mode) -> c_int {
    match mode {
        Mode::Chinese => WOWTYPE_MODE_CHINESE,
        Mode::English => WOWTYPE_MODE_ENGLISH,
        Mode::Translate => WOWTYPE_MODE_TRANSLATE,
    }
}

fn mode_from_c(raw: c_int) -> Mode {
    match raw {
        WOWTYPE_MODE_ENGLISH => Mode::English,
        WOWTYPE_MODE_TRANSLATE => Mode::Translate,
        _ => Mode::Chinese,
    }
}

pub const WOWTYPE_SHIFT_TOGGLE_BOTH: c_int = 0;
pub const WOWTYPE_SHIFT_TOGGLE_LEFT: c_int = 1;
pub const WOWTYPE_SHIFT_TOGGLE_RIGHT: c_int = 2;
pub const WOWTYPE_SHIFT_TOGGLE_OFF: c_int = 3;

pub const WOWTYPE_KEY_ACTION_NONE: u32 = 0;
pub const WOWTYPE_KEY_ACTION_PREV_PAGE: u32 = 1;
pub const WOWTYPE_KEY_ACTION_NEXT_PAGE: u32 = 2;
pub const WOWTYPE_KEY_ACTION_PREV_CANDIDATE: u32 = 3;
pub const WOWTYPE_KEY_ACTION_NEXT_CANDIDATE: u32 = 4;
pub const WOWTYPE_KEY_ACTION_OBSERVE: u32 = 5;
pub const WOWTYPE_KEY_ACTION_TOGGLE_MODE: u32 = 6;
pub const WOWTYPE_KEY_ACTION_TOGGLE_TRANSLATE: u32 = 7;

fn key_action(raw: u32) -> KeyAction {
    match raw {
        WOWTYPE_KEY_ACTION_PREV_PAGE => KeyAction::PrevPage,
        WOWTYPE_KEY_ACTION_NEXT_PAGE => KeyAction::NextPage,
        WOWTYPE_KEY_ACTION_PREV_CANDIDATE => KeyAction::PrevCandidate,
        WOWTYPE_KEY_ACTION_NEXT_CANDIDATE => KeyAction::NextCandidate,
        WOWTYPE_KEY_ACTION_OBSERVE => KeyAction::Observe,
        WOWTYPE_KEY_ACTION_TOGGLE_MODE => KeyAction::ToggleMode,
        WOWTYPE_KEY_ACTION_TOGGLE_TRANSLATE => KeyAction::ToggleTranslate,
        _ => KeyAction::None,
    }
}

pub const WOWTYPE_SELECT_KEYS_OFF: c_int = 0;
pub const WOWTYPE_SELECT_KEYS_SEMICOLON: c_int = 1;
pub const WOWTYPE_SELECT_KEYS_SEMICOLON_QUOTE: c_int = 2;

pub const WOWTYPE_ARROW_KEYS_CANDIDATES: c_int = 0;
pub const WOWTYPE_ARROW_KEYS_CARET: c_int = 1;

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
    /// 1.10 起：0 全角、1 半角。
    pub punct_style: c_int,
    /// 1.10 起：自动补空格，`autospace::LETTERS | DIGITS`，0 关。
    pub auto_space: u32,
    /// 1.10 起：`WOWTYPE_SHIFT_TOGGLE_*`，默认两边都行。
    pub shift_toggle: c_int,
    /// 1.10 起：按住 Shift 多久以内松开算单按（毫秒），0 用默认（500）。
    pub shift_tap_ms: u32,
    /// 1.10 起：单按 Shift 的循环里有「译」。
    pub translate_mode: c_int,
    /// 1.10 起：只打一个 v 再按空格打开工具栏，默认 1。
    pub toolbar: c_int,
    /// 1.10 起：记剪贴板历史（工具栏「剪贴板」页的提示看它），默认 0。
    pub clipboard_history: c_int,
    /// 1.10 起：双拼时编辑区显示打的键，不翻成全拼，默认 0。
    pub shuangpin_keys: c_int,
    /// 1.10 起：打字途中方向键做什么，`WOWTYPE_ARROW_KEYS_*`（#129），默认挑候选。
    pub arrow_keys: c_int,
    /// 1.10 起：五笔（引擎要载入五笔码表），默认 0。
    pub wubi: c_int,
    /// 1.10 起：五笔四码唯一自动上屏，默认 1。
    pub wubi_auto_commit: c_int,
    /// 1.10 起：五笔编码提示，默认 1。
    pub wubi_hint: c_int,
    /// 1.10 起：另设了进出「译」的快捷键，单按 Shift 只在中 ↔ 英之间切，默认 0。
    pub translate_key: c_int,
    /// 1.11 起：上屏以后的联想，默认 0（旧外壳不认识联想、画不出来；设置里的默认值是开，外壳照设置填）。
    pub association: c_int,
    /// 1.12 起：中文标点自动配对，默认 0（外壳要会按 frame 的 `cursor_delta`、`delete_next` 挪光标、删字才开）。
    pub auto_pair_punct: c_int,
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
            punct_style: c_int::from(config.punct_style == PunctStyle::Halfwidth),
            auto_space: config.auto_space,
            shift_toggle: match config.shift_toggle {
                ShiftToggle::Both => WOWTYPE_SHIFT_TOGGLE_BOTH,
                ShiftToggle::Left => WOWTYPE_SHIFT_TOGGLE_LEFT,
                ShiftToggle::Right => WOWTYPE_SHIFT_TOGGLE_RIGHT,
                ShiftToggle::Off => WOWTYPE_SHIFT_TOGGLE_OFF,
            },
            shift_tap_ms: config.shift_tap_ms,
            translate_mode: c_int::from(config.translate_mode),
            toolbar: c_int::from(config.toolbar),
            clipboard_history: c_int::from(config.clipboard_history),
            shuangpin_keys: c_int::from(config.shuangpin_keys),
            arrow_keys: match config.arrow_keys {
                ArrowKeys::Candidates => WOWTYPE_ARROW_KEYS_CANDIDATES,
                ArrowKeys::Caret => WOWTYPE_ARROW_KEYS_CARET,
            },
            wubi: c_int::from(config.wubi),
            wubi_auto_commit: c_int::from(config.wubi_auto_commit),
            wubi_hint: c_int::from(config.wubi_hint),
            translate_key: c_int::from(config.translate_key),
            association: c_int::from(config.association),
            auto_pair_punct: c_int::from(config.auto_pair_punct),
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
            punct_style: if self.punct_style == 1 {
                PunctStyle::Halfwidth
            } else {
                PunctStyle::Fullwidth
            },
            auto_space: self.auto_space,
            shift_toggle: match self.shift_toggle {
                WOWTYPE_SHIFT_TOGGLE_LEFT => ShiftToggle::Left,
                WOWTYPE_SHIFT_TOGGLE_RIGHT => ShiftToggle::Right,
                WOWTYPE_SHIFT_TOGGLE_OFF => ShiftToggle::Off,
                _ => ShiftToggle::Both,
            },
            shift_tap_ms: if self.shift_tap_ms == 0 {
                Config::default().shift_tap_ms
            } else {
                self.shift_tap_ms
            },
            translate_mode: self.translate_mode != 0,
            // 旧外壳给的结构体短、没有这一项时，read_sized 留着 Default 的值（开着）
            toolbar: self.toolbar != 0,
            clipboard_history: self.clipboard_history != 0,
            shuangpin_keys: self.shuangpin_keys != 0,
            arrow_keys: match self.arrow_keys {
                WOWTYPE_ARROW_KEYS_CARET => ArrowKeys::Caret,
                _ => ArrowKeys::Candidates,
            },
            wubi: self.wubi != 0,
            wubi_auto_commit: self.wubi_auto_commit != 0,
            wubi_hint: self.wubi_hint != 0,
            translate_key: self.translate_key != 0,
            association: self.association != 0,
            auto_pair_punct: self.auto_pair_punct != 0,
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
    /// 1.10 起：候选后面的小字（五笔还要打的码、反查的五笔码），没有是空串。
    pub comment: *const c_char,
    /// 1.12 起：候选在列表里的位置（1 起）；0 表示不知道。
    pub index: usize,
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
    comment: CString,
    consumed: usize,
    kind: c_int,
    index: usize,
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
            comment: self.comment.as_ptr(),
            index: self.index,
        }
    }

    fn new(c: &Candidate, index: usize) -> Self {
        Self {
            text: to_c(&c.text),
            pinyin: to_c(&c.pinyin),
            commit: to_c(&c.commit),
            emoji: to_c(&c.emoji),
            comment: to_c(&c.comment),
            consumed: c.consumed,
            index,
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
    /// 1.10 起：这次上屏里有引擎学到的选词。
    pub learned: c_int,
    /// 1.10 起：这次调用以后的输入模式，`WOWTYPE_MODE_*`。
    pub mode: c_int,
    /// 1.10 起：这次调用切换了输入模式。
    pub mode_changed: c_int,
    /// 1.10 起：互译中（下面四个字符串有效，不在互译时都是空串）。
    pub translating: c_int,
    pub translate_source: *const c_char,
    pub translate_text: *const c_char,
    pub translate_title: *const c_char,
    pub translate_status: *const c_char,
    /// 1.10 起：translate_text 是译完的最终结果。
    pub translate_done: c_int,
    /// 1.12 起：上屏以后让外壳把应用里的光标挪几格（负数左移、正数右移），自动配对用。
    pub cursor_delta: c_int,
    /// 1.12 起：1 表示这次没接的退格要连光标后的右标点一起删（成对删）。
    pub delete_next: c_int,
    /// 1.12 起：这次上屏的候选在列表里的位置（1 起）；不是选候选为 0。
    pub choice_index: usize,
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
            learned: 0,
            mode: WOWTYPE_MODE_CHINESE,
            mode_changed: 0,
            translating: 0,
            translate_source: EMPTY.as_ptr(),
            translate_text: EMPTY.as_ptr(),
            translate_title: EMPTY.as_ptr(),
            translate_status: EMPTY.as_ptr(),
            translate_done: 0,
            cursor_delta: 0,
            delete_next: 0,
            choice_index: 0,
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
    /// 上一次上屏是选的哪个候选（以前的 `wowtype_session_last_choice` 用，现在只给调试看）。
    last_choice: Option<CCandidate>,
    /// 过渡函数返回的字符串。
    text: CString,
    /// 互译要画的四段字（原文、译文、标题、提示）。
    translate: [CString; 4],
    /// 候选框上方的提示（工具栏、表情面板）。
    aux: CString,
    /// 外壳给的翻译器（函数表或核心的翻译器），会话借用它。
    backend: Option<Box<dyn crate::session::TranslateBackend>>,
}

static EMPTY: &std::ffi::CStr = c"";

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

/// 按 size 读一个带 size 字段的结构体：外壳给的比我们认识的短时，缺的字段是 `T::default()` 的值。
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
        self.last_choice = self
            .session
            .last_choice()
            .map(|c| CCandidate::new(c, self.session.last_choice_index()));
        self.frame = WowtypeFrame {
            handled: c_int::from(frame.handled),
            commit: self.commit.as_ref().map_or(ptr::null(), |c| c.as_ptr()),
            preedit: self.preedit.as_ptr(),
            preedit_cursor: frame.preedit_cursor.try_into().unwrap_or(c_int::MAX),
            panel: match frame.panel {
                Panel::Normal => WOWTYPE_PANEL_NORMAL,
                Panel::Expanded => WOWTYPE_PANEL_EXPANDED,
                Panel::Emoji => WOWTYPE_PANEL_EMOJI,
                Panel::Tool => WOWTYPE_PANEL_TOOL,
                Panel::Association => WOWTYPE_PANEL_ASSOCIATION,
            },
            cand_count: frame.cand_count,
            cand_cursor: frame
                .cand_cursor
                .and_then(|c| c_int::try_from(c).ok())
                .unwrap_or(-1),
            has_prev: c_int::from(frame.has_prev),
            has_next: c_int::from(frame.has_next),
            forgot: c_int::from(frame.forgot),
            learned: c_int::from(frame.learned),
            mode: mode_to_c(frame.mode),
            mode_changed: c_int::from(frame.mode_changed),
            wake_after_ms: frame.wake_after_ms,
            ..WowtypeFrame::default()
        };
        self.frame.choice_index = self.session.last_choice_index();
        if !frame.aux.is_empty() {
            self.aux = to_c(&frame.aux);
            self.frame.aux = self.aux.as_ptr();
        }
        if let Some(view) = &frame.translate {
            self.translate = [
                to_c(&view.source),
                to_c(&view.text),
                to_c(&view.title),
                to_c(&view.status),
            ];
            self.frame.translating = 1;
            self.frame.translate_source = self.translate[0].as_ptr();
            self.frame.translate_text = self.translate[1].as_ptr();
            self.frame.translate_title = self.translate[2].as_ptr();
            self.frame.translate_status = self.translate[3].as_ptr();
            self.frame.translate_done = c_int::from(view.done);
        }
        self.frame.cursor_delta = frame.cursor_delta as c_int;
        self.frame.delete_next = c_int::from(frame.delete_next);
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
            inner.display_candidates();
        }));
        if computed.is_err() {
            set_last_error("session panicked");
            self.c_candidates.clear();
            return &self.c_candidates;
        }
        if self.session.generation() != self.c_generation {
            self.c_candidates = self
                .session
                .display_candidates()
                .iter()
                .enumerate()
                .map(|(index, candidate)| CCandidate::new(candidate, index + 1))
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
        translate: Default::default(),
        aux: CString::default(),
        backend: None,
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
        hints: raw.hints,
        code: raw.code,
        repeat: raw.repeat != 0,
        time_ms: raw.time_ms,
        next_char: char::from_u32(raw.next_char).filter(|&c| c != '\0'),
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
    unsafe { shown_candidate(session, index) }.map_or(ptr::null(), |c| c.text.as_ptr())
}

/// 上一次返回的 frame 里显示的第 index 个候选后面的小字（1.10，五笔的编码提示），没有是空串。
/// 越界或 NULL 返回 NULL。
#[no_mangle]
pub unsafe extern "C" fn wowtype_session_candidate_comment(
    session: *mut WowtypeSession,
    index: usize,
) -> *const c_char {
    unsafe { shown_candidate(session, index) }.map_or(ptr::null(), |c| c.comment.as_ptr())
}

/// 上一次的 frame 里显示的第 index 个候选。
unsafe fn shown_candidate<'a>(session: *mut WowtypeSession, index: usize) -> Option<&'a CCandidate> {
    let handle = unsafe { session.as_mut() }?;
    if index >= handle.frame.cand_count {
        return None;
    }
    let at = handle.shown_start + index;
    handle.c_candidates().get(at)
}

/// 清空编辑区；commit_pending 非 0 时把已选的词和原始输入放进 frame->commit，为 0 是放弃（上文也作废）。
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

/// 切换输入模式（`WOWTYPE_MODE_*`，外部命令、默认模式、外壳全局的模式用）。切到英文时打到一半的放进
/// frame->commit。（1.10 起）
#[no_mangle]
pub unsafe extern "C" fn wowtype_session_set_mode(
    session: *mut WowtypeSession,
    mode: c_int,
) -> *const WowtypeFrame {
    let Some(handle) = (unsafe { session.as_mut() }) else {
        return ptr::null();
    };
    handle.run("set_mode", |session| session.set_mode(mode_from_c(mode)))
}

/// 现在的输入模式，`WOWTYPE_MODE_*`；session 为 NULL 时是中文。（1.10 起）
#[no_mangle]
pub unsafe extern "C" fn wowtype_session_mode(session: *const WowtypeSession) -> c_int {
    unsafe { session.as_ref() }.map_or(WOWTYPE_MODE_CHINESE, |handle| mode_to_c(handle.session.mode()))
}

/// 互译用核心的翻译器（Linux 外壳）；NULL 表示没有翻译器。翻译器要比会话活得久，
/// 释放前先设成 NULL。换掉时攒着的原文丢掉。（1.10 起）
#[no_mangle]
pub unsafe extern "C" fn wowtype_session_set_translator(
    session: *mut WowtypeSession,
    translator: *mut crate::translate_ffi::WowtypeTranslator,
) {
    let Some(handle) = (unsafe { session.as_mut() }) else {
        return;
    };
    // 生命周期由外壳保证（和引擎一样）
    let backend: Option<&'static dyn crate::session::TranslateBackend> =
        unsafe { translator.as_ref() }.map(|t| {
            let inner: &dyn crate::session::TranslateBackend = &t.translator;
            unsafe { &*(inner as *const dyn crate::session::TranslateBackend) }
        });
    handle.session.set_translator(backend);
    handle.backend = None;
}

/// 互译用外壳自己的翻译器（函数表，macOS 外壳）；NULL 表示没有翻译器。表里的 user 要比会话活得久，
/// 释放前先设成 NULL。换掉时攒着的原文丢掉。（1.10 起）
#[no_mangle]
pub unsafe extern "C" fn wowtype_session_set_translate_backend(
    session: *mut WowtypeSession,
    backend: *const WowtypeTranslateBackend,
) {
    let Some(handle) = (unsafe { session.as_mut() }) else {
        return;
    };
    let boxed: Option<Box<dyn crate::session::TranslateBackend>> = if backend.is_null() {
        None
    } else {
        let raw = unsafe { read_sized(backend, (*backend).size) };
        Some(Box::new(CBackend(raw)))
    };
    // 会话借用盒子里的翻译器：盒子放进 handle 以后地址不变；先换会话的，再放掉旧盒子
    let borrowed: Option<&'static dyn crate::session::TranslateBackend> = boxed
        .as_deref()
        .map(|b| unsafe { &*(b as *const dyn crate::session::TranslateBackend) });
    handle.session.set_translator(borrowed);
    handle.backend = boxed;
}

/// 定时器到了（frame->wake_after_ms 排的）：该发翻译请求的发，按过回车且译好了的放进 frame->commit。
/// now_ms 是单调时钟的毫秒数（和 WowtypeKey.time_ms 同一个时钟）。（1.10 起）
#[no_mangle]
pub unsafe extern "C" fn wowtype_session_tick(
    session: *mut WowtypeSession,
    now_ms: u64,
) -> *const WowtypeFrame {
    let Some(handle) = (unsafe { session.as_mut() }) else {
        return ptr::null();
    };
    handle.run("tick", |session| session.tick(now_ms))
}

/// 外壳要上屏一段与拼音无关的字（工具栏、表情面板、语音识别结果）：互译时进原文，否则补好空格放进
/// frame->commit。prev_char 是应用里光标前的字，读不到给 0。（1.10 起）
#[no_mangle]
pub unsafe extern "C" fn wowtype_session_commit_text(
    session: *mut WowtypeSession,
    text: *const c_char,
    prev_char: u32,
) -> *const WowtypeFrame {
    let Some(handle) = (unsafe { session.as_mut() }) else {
        return ptr::null();
    };
    let text = unsafe { arg(text) };
    let prev = char::from_u32(prev_char).filter(|&c| c != '\0');
    handle.run("commit_text", |session| session.commit_text(&text, prev))
}

/// 表情面板快捷键（外壳按自己的设置认出来）：开着就关上；打了一半的拼音拿来当搜索词。（1.10 起）
#[no_mangle]
pub unsafe extern "C" fn wowtype_session_toggle_emoji_panel(
    session: *mut WowtypeSession,
) -> *const WowtypeFrame {
    let Some(handle) = (unsafe { session.as_mut() }) else {
        return ptr::null();
    };
    handle.run("toggle_emoji_panel", |session| session.toggle_emoji_panel())
}

/// 打开工具栏的一页或表情面板（`WOWTYPE_TOOL_*`，外壳的菜单、按钮用；v + 空格会话自己开）。
/// `WOWTYPE_TOOL_NONE` 关上。（1.10 起）
#[no_mangle]
pub unsafe extern "C" fn wowtype_session_open_tool(
    session: *mut WowtypeSession,
    tool: c_int,
) -> *const WowtypeFrame {
    let Some(handle) = (unsafe { session.as_mut() }) else {
        return ptr::null();
    };
    let tool = match tool {
        WOWTYPE_TOOL_MENU => Some(Tool::Menu),
        WOWTYPE_TOOL_CLIPBOARD => Some(Tool::Clipboard),
        WOWTYPE_TOOL_PHRASES => Some(Tool::Phrases),
        WOWTYPE_TOOL_SYMBOLS => Some(Tool::Emoji { symbols: true }),
        WOWTYPE_TOOL_EMOJI => Some(Tool::Emoji { symbols: false }),
        _ => None,
    };
    handle.run("open_tool", |session| match tool {
        Some(tool) => session.open_tool(tool),
        None => session.close_panel(),
    })
}

/// 开着的面板，`WOWTYPE_TOOL_*`（没开是 NONE）。（1.10 起）
#[no_mangle]
pub unsafe extern "C" fn wowtype_session_open_panel(session: *const WowtypeSession) -> c_int {
    let Some(handle) = (unsafe { session.as_ref() }) else {
        return WOWTYPE_TOOL_NONE;
    };
    match handle.session.open_panel() {
        None => WOWTYPE_TOOL_NONE,
        Some(Tool::Menu) => WOWTYPE_TOOL_MENU,
        Some(Tool::Clipboard) => WOWTYPE_TOOL_CLIPBOARD,
        Some(Tool::Phrases) => WOWTYPE_TOOL_PHRASES,
        Some(Tool::Emoji { symbols: true }) => WOWTYPE_TOOL_SYMBOLS,
        Some(Tool::Emoji { symbols: false }) => WOWTYPE_TOOL_EMOJI,
    }
}

/// 联想开着（上屏以后的联想行，`WOWTYPE_PANEL_ASSOCIATION`）返回 1，没有是 0。不改会话，
/// 之前拿到的 frame 照样有效。（1.11 起）
#[no_mangle]
pub unsafe extern "C" fn wowtype_session_association(session: *const WowtypeSession) -> c_int {
    unsafe { session.as_ref() }.map_or(0, |handle| c_int::from(handle.session.association_open()))
}

/// 最近用过的 emoji 存在这个文件（一行一个，进程内所有会话共用；读进已有的）。
/// NULL 或空串：只记在内存里，并清空。（1.10 起）
#[no_mangle]
pub unsafe extern "C" fn wowtype_recent_emoji_path(path: *const c_char) {
    crate::recent_emoji::set_path(&unsafe { arg(path) });
}

/// 最近用过的 emoji，最近的在前；用 wowtype_candidates_* 读取并释放。（1.10 起）
#[no_mangle]
pub extern "C" fn wowtype_recent_emoji_list() -> *mut crate::ffi::WowtypeCandidates {
    let texts: Vec<CString> = crate::recent_emoji::list().iter().map(|t| to_c(t)).collect();
    let scores = vec![0.0; texts.len()];
    Box::into_raw(Box::new(crate::ffi::WowtypeCandidates::plain(texts, scores)))
}

/// 攒着的原文（互译），没有是空串。在同一个会话下一次调用前有效。（1.10 起）
#[no_mangle]
pub unsafe extern "C" fn wowtype_session_translate_source(session: *mut WowtypeSession) -> *const c_char {
    let Some(handle) = (unsafe { session.as_mut() }) else {
        return EMPTY.as_ptr();
    };
    let source = handle.session.translate_source().to_owned();
    handle.text(&source)
}

/// 丢掉互译攒着的原文、取消这个会话的翻译请求（输入框失去焦点）。（1.10 起）
#[no_mangle]
pub unsafe extern "C" fn wowtype_session_translate_clear(
    session: *mut WowtypeSession,
) -> *const WowtypeFrame {
    let Some(handle) = (unsafe { session.as_mut() }) else {
        return ptr::null();
    };
    handle.run("translate_clear", |session| {
        session.clear_translation();
        session.frame(true)
    })
}

// ---- 读会话状态、整个候选列表（fcitx5 的候选框要整个列表），见 docs/ABI.md「输入会话」----

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
            hints: 0,
            code: 0,
            repeat: 0,
            time_ms: 0,
            next_char: 0,
        };
        let frame = unsafe { &*wowtype_session_key(session, &key) };
        assert_eq!(frame.handled, 1);
        assert_eq!(c(frame.preedit), "a");
        unsafe { wowtype_session_free(session) };
    }

    #[test]
    fn short_config_struct_keeps_defaults() {
        // 旧外壳的 WowtypeSessionConfig 没有 toolbar 这一项：读出来照默认开着，不是 0
        let mut config = WowtypeSessionConfig::default();
        config.size = std::mem::offset_of!(WowtypeSessionConfig, toolbar) as u32;
        config.toolbar = 0;
        config.shuangpin_keys = 1;
        let read = unsafe { read_sized(&config, config.size) }.to_config();
        assert!(read.toolbar);
        assert!(!read.shuangpin_keys, "size 之外的字段不该读进来");
        // 1.10 加的五笔两个开关默认开着，五笔本身默认关
        assert!(!read.wubi && read.wubi_auto_commit && read.wubi_hint);
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
        unsafe { wowtype_session_free(ptr::null_mut()) };
    }
}
