//! 输入会话：一个输入框里的编辑状态和按键规则（见 docs/PORTING.md）。
//!
//! 目标是让 fcitx5 和 macOS 两个外壳只做「平台按键 → 会话 → 按结果画界面、上屏」，
//! 输入行为都在这里。现在是一块一块从 fcitx5 外壳（`fcitx5-addon/src/wowtype_engine.cpp`）
//! 挪过来的，已经挪进来的：
//!
//! - 编辑区里的原始输入、已选词栈（长句先选开头的词）；
//! - 字母和隔音符进编辑区，退格（先撤销最后一次选词，没有选词了才删字母），Esc 清空；
//! - Ctrl+Delete（还有 Ctrl+退格、Shift+Delete）忘掉光标处学错的词，候选原地重算；
//! - 候选列表（引擎的候选 + 计算器、emoji 推荐、数字按键 emoji、日期时间），
//!   编辑区的分音节显示（`nihao` 显示成 `ni hao`，双拼显示成全拼）。
//!
//! - 选词（句首词进已选栈，整句上屏）和学习（选词记录、分几次选出来的整句也记一次）；
//! - 空格、回车、数字、分号引号选词、计算器和双拼分号进编辑区，翻页、移光标和
//!   横排下键展开的候选面板（页和光标照 fcitx5 的候选列表：各管各的，光标可以不在这一页上）。
//!
//! - 上屏以后的联想（[`Config::association`]）：一页常跟在后面的词，Tab 进去选（见 [`Session::key`]）。
//!
//! 还没挪的键 [`Session::key`] 返回 `handled = false`，外壳接着用自己的旧逻辑处理
//! （打字途中的标点、其他可见字符、吃掉的光标键）。鼠标点候选按全局序号调
//! [`Session::choose`]，表情面板往编辑区加搜索词用 [`Session::append`]，这些过渡用法挪完以后会删掉。

use crate::datetime::LocalTime;
pub use crate::symbols::PunctStyle;
use crate::translate::JobState;
use crate::Engine;

/// 平台无关的键码：XKB keysym 的子集（fcitx5 直接用，macOS 外壳把 kVK_* 翻成这个）。
pub mod keysym {
    pub const SHIFT_L: u32 = 0xffe1;
    pub const SHIFT_R: u32 = 0xffe2;
    pub const BACKSPACE: u32 = 0xff08;
    pub const TAB: u32 = 0xff09;
    pub const RETURN: u32 = 0xff0d;
    pub const ESCAPE: u32 = 0xff1b;
    pub const LEFT: u32 = 0xff51;
    pub const UP: u32 = 0xff52;
    pub const RIGHT: u32 = 0xff53;
    pub const DOWN: u32 = 0xff54;
    pub const PAGE_UP: u32 = 0xff55;
    pub const PAGE_DOWN: u32 = 0xff56;
    pub const KP_ENTER: u32 = 0xff8d;
    pub const KP_DELETE: u32 = 0xff9f;
    pub const DELETE: u32 = 0xffff;
    pub const ISO_LEFT_TAB: u32 = 0xfe20;
    pub const KP_MULTIPLY: u32 = 0xffaa;
    pub const KP_ADD: u32 = 0xffab;
    pub const KP_SUBTRACT: u32 = 0xffad;
    pub const KP_DECIMAL: u32 = 0xffae;
    pub const KP_DIVIDE: u32 = 0xffaf;
    pub const KP_0: u32 = 0xffb0;
    pub const KP_9: u32 = 0xffb9;
    pub const SPACE: u32 = 0x20;
    pub const HOME: u32 = 0xff50;
    pub const END: u32 = 0xff57;
    pub const PERCENT: u32 = 0x25;
    pub const APOSTROPHE: u32 = 0x27;
    pub const PAREN_LEFT: u32 = 0x28;
    pub const PAREN_RIGHT: u32 = 0x29;
    pub const ASTERISK: u32 = 0x2a;
    pub const PLUS: u32 = 0x2b;
    pub const COMMA: u32 = 0x2c;
    pub const MINUS: u32 = 0x2d;
    pub const PERIOD: u32 = 0x2e;
    pub const SLASH: u32 = 0x2f;
    pub const DIGIT_0: u32 = 0x30;
    pub const DIGIT_9: u32 = 0x39;
    pub const SEMICOLON: u32 = 0x3b;
    pub const EQUAL: u32 = 0x3d;
    pub const BRACKET_LEFT: u32 = 0x5b;
    pub const BRACKET_RIGHT: u32 = 0x5d;
    pub const ASCII_CIRCUM: u32 = 0x5e;
}

/// 修饰键位掩码，和 X11/fcitx5 的 KeyState 取值一样（fcitx5 外壳可以直接传）。
pub mod modifier {
    pub const SHIFT: u32 = 1 << 0;
    pub const CAPS: u32 = 1 << 1;
    pub const CTRL: u32 = 1 << 2;
    pub const ALT: u32 = 1 << 3;
    pub const NUMLOCK: u32 = 1 << 4;
    pub const SUPER: u32 = 1 << 6;
}

/// 外壳按平台的全局热键认出来的候选动作（fcitx5 的「上一页」「下一个候选」热键，
/// 候选框上的翻页按钮）。方向键不走这个，按横排 / 竖排照会话自己的规则。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KeyAction {
    #[default]
    None,
    PrevPage,
    NextPage,
    PrevCandidate,
    NextCandidate,
    /// 外壳只是告诉会话有这个键（松开的、外壳自己处理掉的）：单按 Shift 的判断要看到按住
    /// Shift 时的每个键；会话不处理它。
    Observe,
    /// 设置里另设的切换键（ToggleKey）：中 ↔ 英，在「译」里回中文（和单按 Shift 一样）。
    ToggleMode,
    /// 设置里进出「译」的快捷键（TranslateModeKey）：不在「译」里进「译」，在「译」里回到进来前的模式。
    /// 进不了「译」（没配好翻译、[`key_hints::NO_TRANSLATE`] 的输入框）时不接，键交给应用。
    ToggleTranslate,
}

/// 一次按键。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Key {
    pub keysym: u32,
    pub modifiers: u32,
    /// 松开（按下是 false）。
    pub release: bool,
    /// 应用里光标前一个字符（外壳读周围文字得到），读不到是 None（带 [`key_hints::SURROUNDING`] 时是光标前没有字）。
    /// 数字后面的标点保持英文、自动空格、核对上文都看它；读不到时标点看会话自己记的上一个字，自动空格不补。
    pub prev_char: Option<char>,
    /// 外壳认出来的全局热键（翻页、移光标），没有是 None。
    pub action: KeyAction,
    /// 输入框的情况，[`key_hints`] 的位掩码。
    pub hints: u32,
    /// 按键码（硬件键码），不知道是 0。单按 Shift 时按下、松开按键码对上才算同一个键：
    /// xkb 选项或输入法布局重新翻译时，松开那一下可能翻成别的符号。
    pub code: u32,
    /// 按住不放的自动重复。
    pub repeat: bool,
    /// 按键时刻（毫秒，单调递增的任意起点），不知道是 0。单按 Shift 看按住多久。
    pub time_ms: u64,
    /// 应用里光标后一个字符，读不到是 None（带 [`key_hints::NEXT_CHAR`] 时是光标后没有字）。
    /// 自动配对跳过右标点、成对删之前拿它核对右边确实是补好的那个。
    pub next_char: Option<char>,
}

/// 外壳告诉会话的输入框情况（[`Key::hints`]）。
pub mod key_hints {
    /// 密码框、网址框、邮箱框：不自动补空格。
    pub const NO_AUTO_SPACE: u32 = 1 << 0;
    /// 外壳开着自己的面板（表情面板、工具栏）：这时按 Shift 不切中英文。
    pub const PANEL_OPEN: u32 = 1 << 1;
    /// 不能翻译的输入框（密码框）：「译」里也照常上屏，不攒原文、不往外发。
    pub const NO_TRANSLATE: u32 = 1 << 2;
    /// 应用告诉了光标周围的文字：[`Key::prev_char`] 是 None 时光标前确实没有字（不是读不到）。
    pub const SURROUNDING: u32 = 1 << 3;
    /// 密码框：选的词也不学（不进 history.tsv 和用户词库）。回车上屏的原始字母串哪里都不学，不用这个标记。
    /// 和 NO_TRANSLATE 分开：macOS 只知道系统全局的「安全输入」开没开（终端开了安全键盘输入时所有输入框都算），
    /// 拿它判断会让学习在所有地方静默停掉。
    pub const NO_LEARN: u32 = 1 << 4;
    /// 应用告诉了光标后面的字：[`Key::next_char`] 是 None 时光标后确实没有字。有选区时外壳别带。
    pub const NEXT_CHAR: u32 = 1 << 5;
}

/// 互译用的翻译器（[`crate::translate::Translator`]，或外壳自己的实现）。都不阻塞。
pub trait TranslateBackend {
    /// 开始翻译 text，取消别的请求；正在译或已经译好同一句话时什么也不做。
    fn start(&self, text: &str);
    /// 取消现在的请求，不管是谁发的。
    fn cancel(&self);
    /// 现在的请求还是翻译 source 的才取消。一个进程只有一个翻译器、所有输入框共用：
    /// 输入框 A 清空时，翻译器上跑的可能已经是输入框 B 后来发的请求，不能把它取消掉。
    /// 会话都在同一个线程上调翻译器（插件的事件循环、macOS 的主线程），先看再取消之间不会有别人 start。
    fn cancel_source(&self, source: &str) {
        if self.poll().source == source {
            self.cancel();
        }
    }
    fn poll(&self) -> crate::translate::Snapshot;
    /// 方向标签（「中→英」「英→中」），按 text 判断。
    fn tag(&self, text: &str) -> String;
}

impl TranslateBackend for crate::translate::Translator {
    fn start(&self, text: &str) {
        crate::translate::Translator::start(self, text);
    }
    fn cancel(&self) {
        crate::translate::Translator::cancel(self);
    }
    fn cancel_source(&self, source: &str) {
        crate::translate::Translator::cancel_source(self, source);
    }
    fn poll(&self) -> crate::translate::Snapshot {
        crate::translate::Translator::poll(self)
    }
    fn tag(&self, text: &str) -> String {
        crate::translate::direction_tag(&self.config(), text)
    }
}

/// 原文停下来这么久才发请求（句末标点立刻发）。
pub const TRANSLATE_DELAY_MS: u64 = 500;
/// 等译文时多久再叫一次会话（[`Frame::wake_after_ms`]）。
pub const TRANSLATE_POLL_MS: u32 = 50;

/// 互译时外壳要画的（[`Frame::translate`]）。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TranslateView {
    /// 攒着的原文（编辑区里画在拼音前面）。
    pub source: String,
    /// 显示的译文：原文改了、新译文还没写到上一句那么长时先留着上一句，免得一闪一闪。
    pub text: String,
    /// 译完了（text 是最终结果）。
    pub done: bool,
    /// 候选框上方：方向标签 + 「互译：」。
    pub title: String,
    /// 候选框下方的一行：出错原因、「翻译中…」、怎么上屏；没有是空串。
    pub status: String,
}

#[derive(Clone, Debug, Default)]
struct Translation {
    source: String,
    shown: String,
    /// 按过回车：译好了就上屏。
    pending: bool,
    /// 什么时候发请求（毫秒）；None 不用发。
    due_ms: Option<u64>,
    /// 这个会话发出去、还没取消的请求的原文。多个输入框共用一个翻译器：清空时只取消翻译器上
    /// 还是这句话的请求，别的输入框后来发的不动。
    requested: Option<String>,
}

/// 输入模式：中文、英文（按键原样交给应用）、互译（打中文出外文，上屏由外壳接管）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Chinese,
    English,
    Translate,
}

/// 用哪边的 Shift 单按切换中英文（[`Config::shift_toggle`]）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ShiftToggle {
    #[default]
    Both,
    Left,
    Right,
    Off,
}

/// 单按 Shift：按下以后、松开以前没按别的键，也没按太久（[`Config::shift_tap_ms`]）。
#[derive(Clone, Copy, Debug, Default)]
struct ShiftTap {
    /// 按下的是哪个 Shift（keysym）、按键码、什么时候按下的。
    down: Option<(u32, u32, u64)>,
}

impl Key {
    /// 按下一个键，不带修饰键。
    pub fn press(keysym: u32) -> Self {
        Self {
            keysym,
            ..Self::default()
        }
    }

    /// 按下一个可见 ASCII 字符（大写字母带上 Shift，和 fcitx5 收到的一样）。
    pub fn char(c: char) -> Self {
        let modifiers = if c.is_ascii_uppercase() {
            modifier::SHIFT
        } else {
            0
        };
        Self {
            keysym: c as u32,
            modifiers,
            ..Self::default()
        }
    }

    /// 候选框上的翻页按钮、全局热键：不是按键，只带动作。
    pub fn action(action: KeyAction) -> Self {
        Self {
            action,
            ..Self::default()
        }
    }

    /// 忘掉光标处的候选：Ctrl+Delete、Ctrl+退格（Mac 笔记本没有向前删除键）、Shift+Delete。
    /// 带 Alt、Super 的不算。
    fn is_forget(&self) -> bool {
        let m = self.modifiers & !(modifier::CAPS | modifier::NUMLOCK);
        let delete = matches!(self.keysym, keysym::DELETE | keysym::KP_DELETE);
        match m {
            modifier::CTRL => delete || self.keysym == keysym::BACKSPACE,
            modifier::SHIFT => delete,
            _ => false,
        }
    }

    fn has_command_modifier(&self) -> bool {
        self.modifiers & (modifier::CTRL | modifier::ALT | modifier::SUPER) != 0
    }

    /// 数字键（主键盘和小键盘）的值；带了 CapsLock、NumLock 以外的修饰键不算（和 fcitx5 一样）。
    fn digit(&self) -> Option<usize> {
        if self.modifiers & !(modifier::CAPS | modifier::NUMLOCK) != 0 {
            return None;
        }
        match self.keysym {
            k @ keysym::DIGIT_0..=keysym::DIGIT_9 => Some((k - keysym::DIGIT_0) as usize),
            k @ keysym::KP_0..=keysym::KP_9 => Some((k - keysym::KP_0) as usize),
            _ => None,
        }
    }
}

/// V 模式计算器认的字符（数字、运算符、括号，小键盘也算；不看修饰键）。
fn calc_char(sym: u32) -> Option<char> {
    Some(match sym {
        keysym::DIGIT_0..=keysym::DIGIT_9 => char::from(b'0' + (sym - keysym::DIGIT_0) as u8),
        keysym::KP_0..=keysym::KP_9 => char::from(b'0' + (sym - keysym::KP_0) as u8),
        keysym::PLUS | keysym::KP_ADD => '+',
        keysym::MINUS | keysym::KP_SUBTRACT => '-',
        keysym::ASTERISK | keysym::KP_MULTIPLY => '*',
        keysym::SLASH | keysym::KP_DIVIDE => '/',
        keysym::PERIOD | keysym::KP_DECIMAL => '.',
        keysym::PERCENT => '%',
        keysym::ASCII_CIRCUM => '^',
        keysym::PAREN_LEFT => '(',
        keysym::PAREN_RIGHT => ')',
        _ => return None,
    })
}

/// 已经选好、还没上屏的一个词（长句先选开头的词）。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    pub text: String,
    /// 词库里的写法：开着繁体时 text 是转换过的字，拼成词去学时用这个。空的当和 text 一样。
    pub plain: String,
    pub pinyin: String,
    /// 用掉的原始输入字节数。
    pub bytes: usize,
    /// 选它时引擎学到的东西在学习日志里的区间（[`Engine::learn_mark`]）：退格把它退回编辑区、
    /// Esc 放弃时按这个撤销（[`Engine::undo_learning`]）。没学（以词定字、五笔）是空的。
    pub learn: std::ops::Range<u64>,
}

/// 一次调用之后外壳要做的事。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Frame {
    /// 会话处理了这个键。false：会话还不管这个键（外壳接着处理，挪完以后就是交给应用）。
    pub handled: bool,
    /// 要上屏的字。
    pub commit: Option<String>,
    /// 编辑区：已选的词 + 还没选的输入（分好音节，双拼显示成全拼）。
    pub preedit: String,
    /// 编辑区光标，UTF-8 字节偏移。
    pub preedit_cursor: usize,
    /// 候选框是普通的一页还是展开的面板。
    pub panel: Panel,
    /// 候选框里显示的是全部候选里从 cand_start 起的 cand_count 个（一页，或展开面板的一屏）。
    pub cand_start: usize,
    pub cand_count: usize,
    /// 光标在显示的这些里的位置；不在这一页上是 None（翻页按钮翻走以后会这样）。
    pub cand_cursor: Option<usize>,
    pub has_prev: bool,
    pub has_next: bool,
    /// 这次调用忘掉了一个学过的词（Ctrl+Delete）：外壳该马上存用户词库和输入习惯。
    pub forgot: bool,
    /// 这次上屏里有引擎学到的选词（选了引擎的候选、原样上屏输入）：外壳攒够次数存一次用户词库。
    /// 只上屏了标点、emoji、计算结果、日期时间的是 false。
    pub learned: bool,
    /// 这次调用以后的输入模式。
    pub mode: Mode,
    /// 这次调用切换了输入模式（单按 Shift、set_mode）：外壳更新状态栏、提示。
    pub mode_changed: bool,
    /// 互译中（「译」、有翻译器、输入框能翻译）：外壳画原文、译文、提示。
    pub translate: Option<TranslateView>,
    /// 大于 0：过这么久（毫秒）调一次 [`Session::tick`]（等译文、等发请求）。
    pub wake_after_ms: u32,
    /// 候选框上方的提示（工具栏、表情面板），没有是空串。
    pub aux: String,
    /// 上屏以后（commit 落到应用以后）让外壳把应用里的光标挪几格：负数左移、正数右移，0 不动。
    /// 自动配对上屏「（）」后是 -1（光标放到中间），跳过补好的右标点是 +1（这时可能还带着先上屏的候选）。
    pub cursor_delta: i32,
    /// 退格成对删：这次的退格没接（handled 为 false），外壳除了让应用删光标前的左标点，
    /// 还要删光标后的右标点（一个 Delete）。两下按顺序发，别和挪光标的键乱了先后。
    pub delete_next: bool,
}

/// 候选框的样子。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Panel {
    /// 一页候选（横排或竖排）。
    #[default]
    Normal,
    /// 横排时按下键展开的面板：[`GRID_COLS`] 列，一屏 [`GRID_ROWS`] 行，光标所在行标 1–6。
    Expanded,
    /// 表情面板（表情快捷键、工具栏的「符号」「表情」）：和展开面板一样的网格，编辑区是搜索词。
    Emoji,
    /// v + 空格的工具栏（菜单、剪贴板、常用语）：竖排一列，一页 [`TOOL_PAGE`] 个。
    Tool,
    /// 上屏以后的联想（没在打字）：一页常跟在后面的词，照普通候选框画（横排 / 竖排照设置），
    /// 提示在 [`Frame::aux`]。按 Tab 进了联想行时 [`Frame::cand_cursor`] 是光标，没进是 None（不画光标、不画序号）。
    Association,
}

/// 编辑区里还没选词的部分最多几个键，见 [`Session::push_input`]。
/// 日常的整句四五十个字母（实测 51 个字母一个键 16 ms），64 留了余量。
pub const MAX_PENDING_BYTES: usize = 64;

/// 工具栏一页几个。
pub const TOOL_PAGE: usize = 9;
/// 表情面板最多列这么多个（不搜索时是全部 emoji）。
const MAX_EMOJI: usize = 2000;
const TOOL_MENU: [&str; 4] = ["剪贴板", "常用语", "符号", "表情"];

/// 开着的面板（[`Session::open_tool`]）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    /// v + 空格的菜单：剪贴板、常用语、符号、表情
    Menu,
    Clipboard,
    Phrases,
    /// 表情面板；symbols 是从菜单「符号」进来的（不搜索时列全部符号，选的不记进最近用过）
    Emoji { symbols: bool },
}

#[derive(Clone, Debug)]
struct ToolState {
    tool: Tool,
    items: Vec<Candidate>,
    /// 光标在 items 里的位置（不分页）
    cursor: usize,
    /// 表情面板的搜索词（打拼音或英文）
    query: String,
}

/// 上屏以后的联想。
#[derive(Clone, Debug)]
struct Association {
    /// 显示的词（开着繁体时是转换过的）。
    items: Vec<Candidate>,
    /// 词库里的写法，选了以后拿它学。
    plain: Vec<String>,
    /// 按 Tab 进了联想行：空格、回车、数字选，方向键、Tab 移光标，Esc 退出。没进时这些键照原来的规则处理。
    focused: bool,
    cursor: usize,
}

/// 联想行上方的提示。
const ASSOCIATION_HINT: &str = "联想：Tab 选词";
const ASSOCIATION_FOCUSED_HINT: &str = "联想：空格或数字上屏，Esc 退出";

/// 展开面板一行几个候选。
pub const GRID_COLS: usize = 6;
/// 展开面板一屏几行。
pub const GRID_ROWS: usize = 6;
const GRID_SCREEN: usize = GRID_COLS * GRID_ROWS;

/// 设置里能选的翻页键（[`Config::page_keys`] 的位掩码）。
pub mod page_keys {
    /// - =
    pub const MINUS: u32 = 1 << 0;
    /// [ ]
    pub const BRACKET: u32 = 1 << 1;
    /// , .
    pub const COMMA: u32 = 1 << 2;
    /// Shift+Tab Tab
    pub const TAB: u32 = 1 << 3;
}

/// 分号、引号选词。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SelectKeys {
    #[default]
    Off,
    /// 分号选第 2 个。
    Semicolon,
    /// 分号选第 2 个、引号选第 3 个。
    SemicolonQuote,
}

/// 打字途中方向键做什么（#129）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ArrowKeys {
    /// 都用来挑候选，跟着候选排列：横排左右键选词、上键上一页、下键展开面板；竖排上下键选词、左右键翻页。
    #[default]
    Candidates,
    /// 上下键选词（到了一页的两头接着翻页），左右键在还没选词的拼音里移动光标：
    /// 字母插在光标处，退格删光标前一个，Home、End、Delete 也管光标。方向键不翻页。
    /// 「译」里拼音在原文框里，没有光标，左右键不动。
    Caret,
}

/// 候选框的状态（过渡：fcitx5 外壳按它摆自己的候选列表）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct View {
    /// 普通候选框在第几页。
    pub page: usize,
    /// 普通候选框的光标（全部候选里的序号，可以不在这一页上）。
    pub cursor: usize,
    pub expanded: bool,
    /// 展开面板的光标（全部候选里的序号）。
    pub grid_cursor: usize,
}

/// 会话的设置（外壳从自己的配置里抄过来）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    /// 一页几个候选（数字按键 emoji 只在第一页里找数字字）。
    pub page_size: usize,
    /// V 模式计算器（v 后面打算式）。双拼、五笔时用大写 V（小写 v 是声母键、五笔码）。
    pub calculator: bool,
    /// 打 riqi、shijian 这类时给日期时间候选。
    pub date_time: bool,
    /// 打字时推荐 emoji（开心 → 😊，八 → 8️⃣）。
    pub emoji_suggest: bool,
    /// 候选横排；false 竖排。方向键怎么用见 [`ArrowKeys`]。
    pub horizontal: bool,
    /// 翻页键，[`page_keys`] 的位掩码（PgUp PgDn 一直能用）。
    pub page_keys: u32,
    pub select_keys: SelectKeys,
    /// 以词定字：打拼音途中 `[` 上屏光标处候选的第一个字、`]` 上屏最后一个字（#83）。
    /// `[ ]` 设成翻页键时让给翻页。
    pub pick_char: bool,
    /// 标点样式：全角（中文标点）或半角（原样交给应用）。
    pub punct_style: PunctStyle,
    /// 中英文之间自动补空格，[`crate::autospace`] 的 `LETTERS | DIGITS` 位掩码，0 是关。
    pub auto_space: u32,
    /// 用哪边的 Shift 单按切换中英文。
    pub shift_toggle: ShiftToggle,
    /// 按住 Shift 多久以内松开才算单按（毫秒）：按久了多半是想打大写或者犹豫了。
    pub shift_tap_ms: u32,
    /// 有「译」（外壳配好了翻译服务才开）：单按 Shift 的循环里有它（中 → 英 → 译 → 中），
    /// [`KeyAction::ToggleTranslate`] 进得去。
    pub translate_mode: bool,
    /// 另设了进出「译」的快捷键（[`KeyAction::ToggleTranslate`]）：单按 Shift 只在中 ↔ 英之间切
    /// （在「译」里单按回中文），不再绕到「译」。
    pub translate_key: bool,
    /// 只打一个 v 再按空格打开工具栏（双拼、五笔、互译时不开）。
    pub toolbar: bool,
    /// 记剪贴板历史（设置里开了）：工具栏「剪贴板」页的提示看它。
    pub clipboard_history: bool,
    /// 双拼时编辑区显示打的键（uivs），不翻成全拼（shi'zhong）；中英混打时看得出打了什么（#122）。
    pub shuangpin_keys: bool,
    /// 打字途中方向键做什么（#129）。
    pub arrow_keys: ArrowKeys,
    /// 五笔（86 版）：字母是五笔码，z 开头临时打拼音。引擎没载入五笔码表时照全拼打，编辑区上方提示。
    pub wubi: bool,
    /// 五笔打满四码、只有一个候选时直接上屏。
    pub wubi_auto_commit: bool,
    /// 五笔的逐码提示（以打的码开头的更长的码的词）后面显示还要打的码。
    pub wubi_hint: bool,
    /// 上屏以后的联想：选词上屏、整句上屏以后给出一页常跟在后面的词（[`Engine::associate`]）。
    /// 设置里默认开；这里默认关（外壳不认识联想就画不出来，C 接口里旧外壳没给这一项时也是关）。
    pub association: bool,
    /// 中文标点自动配对：全角标点下打 `( [ < { " '` 上屏成对的（）【】《》「」“”‘’、光标放中间
    /// （[`Frame::cursor_delta`]），在里面打完再按右半边跳过补好的那个，空的一对按退格两个一起删
    /// （[`Frame::delete_next`]）。默认关（外壳要会挪光标才开，设置里也是默认关）。
    pub auto_pair_punct: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            page_size: 9,
            calculator: true,
            date_time: true,
            emoji_suggest: true,
            horizontal: true,
            page_keys: page_keys::MINUS,
            select_keys: SelectKeys::Off,
            pick_char: true,
            punct_style: PunctStyle::Fullwidth,
            auto_space: 0,
            shift_toggle: ShiftToggle::Both,
            shift_tap_ms: 500,
            translate_mode: false,
            translate_key: false,
            toolbar: true,
            clipboard_history: false,
            shuangpin_keys: false,
            arrow_keys: ArrowKeys::Candidates,
            wubi: false,
            wubi_auto_commit: true,
            wubi_hint: true,
            association: false,
            auto_pair_punct: false,
        }
    }
}

/// 候选的种类：选中以后怎么上屏、学不学。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CandidateKind {
    /// 引擎的候选：选中要学习。
    #[default]
    Word,
    /// 计算结果、日期时间、带换行的自定义词：原样上屏，不学习。
    Plain,
    /// emoji：上屏 [`Candidate::commit`]，记进最近用过的 emoji，不学习。
    Emoji,
}

/// 会话给出的一个候选。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Candidate {
    /// 候选框里显示的字。
    pub text: String,
    /// 拼音（空格分音节）；计算结果、emoji 这类为空。
    pub pinyin: String,
    /// 用掉了还没选词那部分输入的多少字节；比它短是句首词（选了以后剩下的接着选）。
    pub consumed: usize,
    pub kind: CandidateKind,
    /// 选中时上屏的字：emoji 推荐在句尾对上时是「整句 + emoji」，其余和 text 一样。
    pub commit: String,
    /// emoji 本身（推荐候选显示的是「开心😊」，这里是「😊」）；不是 emoji 为空。
    pub emoji: String,
    /// 候选后面的小字（不上屏）：五笔逐码提示还要打的码、z 临时拼音反查的五笔码；没有为空。
    pub comment: String,
}

impl Candidate {
    fn plain(text: String, consumed: usize) -> Self {
        Self {
            commit: text.clone(),
            text,
            consumed,
            kind: CandidateKind::Plain,
            ..Self::default()
        }
    }
}

/// 读本地时间的函数（测试里换成固定时间）。
pub type Clock = fn() -> Option<LocalTime>;

/// 选了开头的词以后，拿已上屏的字 + 已选的词当上文，最多这么多字节。
const CONTEXT_BYTES: usize = 24;
/// 会话自己记的上文最多留几个字（排序用的上文、学搭配都只看最后几个字）。
const CONTEXT_CHARS: usize = 20;
/// emoji 推荐插在第几个（首选和次选后面）。
const EMOJI_SLOT: usize = 2;
/// 五笔的逐码提示最多列几个（只打一码时以它开头的有四五千条，列多了没用）。
const WUBI_MORE: usize = 50;

/// 一个输入框的会话。只能在一个线程里用。
///
/// 引擎是后台载入的，载入好之前 `engine` 是 None（照样能打字，只是没有候选）。
#[derive(Clone)]
pub struct Session<'e> {
    /// 全部原始输入（包括已经选过词的部分）。
    input: String,
    selections: Vec<Selection>,
    /// 编辑区光标在 input 里的字节位置；None 是在末尾（没移动过，或移回了末尾）。
    /// 只在还没选词的那段里，读的时候用 [`Session::caret`] 夹进去。
    caret: Option<usize>,
    engine: Option<&'e Engine>,
    config: Config,
    /// 光标前已经上屏的字（外壳记的）。
    context: String,
    clock: Clock,
    /// 候选缓存：输入、已选词、引擎、上文、设置变了就作废，用的时候再算。
    candidates: Vec<Candidate>,
    fresh: bool,
    /// 每重算一次候选加一（C 接口按它判断要不要重建字符串）。
    generation: u64,
    /// 候选框：页、光标（全局序号）、展开面板和它的光标。候选重算时回到开头。
    page: usize,
    cursor: usize,
    expanded: bool,
    grid_cursor: usize,
    /// 这次调用要上屏的字（frame 取走）。
    pending: Option<String>,
    /// 这次调用上屏的字是选哪个候选来的；原样上屏输入时是 None。
    last_choice: Option<Candidate>,
    /// 这个候选在列表里的位置（1 起）；没选候选时是 0。
    last_choice_index: usize,
    /// 这次调用忘掉了学过的词（frame 取走）。
    forgot: bool,
    /// 这次调用上屏的里有学到的选词（frame 取走）。
    learned: bool,
    /// 双引号、单引号各自上一次给的是不是左引号（每个会话一份）。
    quotes: [bool; 2],
    /// 应用里光标前的字（这次按键时外壳给的；上屏以后是上屏的最后一个字）。自动空格看它，不知道是 None。
    prev_char: Option<char>,
    /// 会话自己记的上一个字：上屏的最后一个字，或者交给应用的数字、标点。
    /// 外壳读不到周围文字时，标点看它（3.14 的点）。
    last_char: Option<char>,
    /// 这次按键时外壳给的输入框情况（[`key_hints`]）。
    hints: u32,
    mode: Mode,
    /// 这次调用切换了模式（frame 取走）。
    mode_changed: bool,
    /// 进「译」之前的模式：进出「译」的快捷键在「译」里按时回到这里。
    translate_from: Mode,
    shift: ShiftTap,
    translator: Option<&'e dyn TranslateBackend>,
    translation: Translation,
    /// 开着的工具栏 / 表情面板；None 是没开。
    tool: Option<ToolState>,
    /// 最近一次按键或 tick 的时刻（毫秒）。
    now_ms: u64,
    /// 上一次上屏学到的东西（学习日志的区间，含选句首词时学的）：上屏后紧接着按退格时撤销。
    /// 之后按了别的键、又上屏了别的字（标点、空格）、放弃时清掉。
    undo_commit: Option<std::ops::Range<u64>>,
    /// 这次上屏开始学习前的学习日志位置（没有已选的词时，上屏的学习从这里算）。
    commit_start: Option<u64>,
    /// 上屏以后的联想，没有是 None。
    association: Option<Association>,
    /// 这次调用选词上屏了（后面没再上屏标点之类）：调用结束时出联想。
    association_due: bool,
    /// 应用里光标后的字（这次按键时外壳给的，带 [`key_hints::NEXT_CHAR`] 时才算数）。
    next_char: Option<char>,
    /// 自动配对补上、还没跳过的右标点，外面的在前（可以套着：（“”））。光标一直在最后一个的右半边前面；
    /// 光标可能挪走了（方向键、回车、快捷键、失去焦点、重置）就清空。
    pairs: Vec<Pair>,
    /// 这次调用以后外壳要挪的应用光标（frame 取走）。
    cursor_delta: i32,
    /// 这次的退格要外壳连光标后的右标点一起删（frame 取走）。
    delete_next: bool,
}

/// 自动配对补上的一对标点（[`Config::auto_pair_punct`]）。
#[derive(Clone, Copy, Debug)]
struct Pair {
    open: char,
    close: char,
    /// 左半边和光标之间有几个字：上屏的、交给应用的字和空格，里面跳过了的一对整个算上。
    /// 退格减一；是 0 时左半边就在光标前，退格两个一起删。
    inner: usize,
}

/// 最多记几层配对（再多就丢掉最外面的）。
const MAX_PAIRS: usize = 32;

impl Default for Session<'_> {
    fn default() -> Self {
        Self {
            input: String::new(),
            selections: Vec::new(),
            caret: None,
            engine: None,
            config: Config::default(),
            context: String::new(),
            clock: system_time,
            candidates: Vec::new(),
            fresh: false,
            generation: 0,
            page: 0,
            cursor: 0,
            expanded: false,
            grid_cursor: 0,
            pending: None,
            last_choice: None,
            last_choice_index: 0,
            forgot: false,
            learned: false,
            quotes: [false; 2],
            prev_char: None,
            last_char: None,
            hints: 0,
            mode: Mode::Chinese,
            mode_changed: false,
            translate_from: Mode::Chinese,
            shift: ShiftTap::default(),
            translator: None,
            translation: Translation::default(),
            tool: None,
            now_ms: 0,
            undo_commit: None,
            commit_start: None,
            association: None,
            association_due: false,
            next_char: None,
            pairs: Vec::new(),
            cursor_delta: 0,
            delete_next: false,
        }
    }
}

impl std::fmt::Debug for Session<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("input", &self.input)
            .field("selections", &self.selections)
            .field("has_engine", &self.engine.is_some())
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl<'e> Session<'e> {
    pub fn new() -> Self {
        Self::default()
    }

    /// 带引擎建会话。
    pub fn with_engine(engine: &'e Engine) -> Self {
        Self {
            engine: Some(engine),
            ..Self::default()
        }
    }

    /// 换引擎（外壳的词库在后台载入好了）。
    pub fn set_engine(&mut self, engine: Option<&'e Engine>) {
        let same = match (self.engine, engine) {
            (Some(a), Some(b)) => std::ptr::eq(a, b),
            (None, None) => true,
            _ => false,
        };
        if !same {
            self.engine = engine;
            self.fresh = false;
        }
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn configure(&mut self, config: Config) {
        if self.config != config {
            if config.arrow_keys == ArrowKeys::Candidates {
                self.caret = None; // 方向键换回选词：拼音里的光标回到末尾
            }
            self.config = config;
            self.fresh = false;
            if !self.config.association {
                self.close_association();
            }
            if !self.config.auto_pair_punct {
                self.pairs.clear();
            }
        }
    }

    /// 光标前已经上屏的字（选了开头的词以后，剩下的部分拿它和已选的词当上文排序）。
    pub fn set_context(&mut self, text: &str) {
        if self.context != text {
            self.context.clear();
            self.context.push_str(text);
            self.fresh = false;
        }
    }

    /// 换掉读本地时间的函数（测试用）。
    pub fn set_clock(&mut self, clock: Clock) {
        self.clock = clock;
        self.fresh = false;
    }

    /// 处理一次按键。
    pub fn key(&mut self, key: &Key) -> Frame {
        self.last_choice = None;
        self.last_choice_index = 0;
        if key.time_ms != 0 {
            self.now_ms = key.time_ms;
        }
        if let Some(handled) = self.shift_key(key) {
            return self.frame(handled);
        }
        if key.action == KeyAction::Observe {
            return self.frame(false);
        }
        if key.release {
            return self.frame(false);
        }
        self.prev_char = key.prev_char;
        self.next_char = key.next_char;
        self.hints = key.hints;
        if !is_modifier_keysym(key.keysym) {
            // 上屏后紧接着按退格（编辑区空着，退格要交给应用删字）：刚才那次上屏是打错了，
            // 学到的东西退回去。按了别的键就不算了（只修最近一次、只在紧接着时）
            let undo = self.undo_commit.take();
            if let Some(span) = undo.filter(|_| self.backspace_undoes(key)) {
                self.unlearn(&span);
            }
        }
        let surrounding = key.hints & key_hints::SURROUNDING != 0;
        if key.prev_char.is_some() || surrounding {
            self.last_char = key.prev_char;
        }
        if surrounding {
            self.check_context(key.prev_char);
        }
        let handled = match self.association_key(key) {
            Some(handled) => handled,
            None => self.handle_key(key),
        };
        if !handled && key.keysym != 0 && !is_modifier_keysym(key.keysym) {
            // 这个键交给了应用（退格、Delete、方向键、回车、快捷键、英文和半角的字……），
            // 应用里的字或光标可能变了：会话记的光标前的字、上文都不算数了。
            // 交给应用的数字、标点会话已经记了（3.14 的点），单按修饰键不算
            if key.prev_char.is_none() && visible_symbol(key.keysym).is_none() {
                self.last_char = None;
            }
            if !self.composing() {
                self.set_context("");
            }
            self.pair_key_passed(key);
        }
        self.settle_association();
        self.frame(handled)
    }

    /// 联想开着时的键。Some：联想处理完了；None：联想关了（或者是单按的修饰键，联想留着），照原来的规则处理。
    ///
    /// 联想出来以后最常做的是接着打拼音、数字、标点，按空格、回车交给应用，这些都不能被联想抢走：
    /// 没进联想行时只有 Tab（进联想行）和 Esc（关掉，吃掉）是联想的，别的键先关掉联想再照常处理——
    /// 数字也是（中文后面接着打数字很常见，见 AutoSpaceDigits），所以数字不选联想。
    /// 进联想行用 Tab 不用下键：没在打字时方向键是挪应用里的光标（上屏后发现前面打错了，正要挪过去改），
    /// 下键在打字途中还管展开面板、竖排时挪候选，Tab 在中文后面几乎不会紧接着用。
    /// 进了联想行：空格、回车、数字选，左右（上下）键、Tab、Shift+Tab 移光标，Esc 退出；别的键退出联想再照常处理。
    fn association_key(&mut self, key: &Key) -> Option<bool> {
        let state = self.association.as_ref()?;
        if is_modifier_keysym(key.keysym) {
            return None;
        }
        let mods = key.modifiers & !(modifier::CAPS | modifier::NUMLOCK);
        let plain = mods & (modifier::CTRL | modifier::ALT | modifier::SUPER) == 0 && key.action == KeyAction::None;
        let n = state.items.len();
        if state.focused && plain {
            let cursor = state.cursor.min(n.saturating_sub(1));
            let shift = mods & modifier::SHIFT != 0;
            let moved = match key.keysym {
                keysym::SPACE | keysym::RETURN | keysym::KP_ENTER => {
                    self.choose_association(cursor);
                    return Some(true);
                }
                keysym::ESCAPE => {
                    self.close_association();
                    return Some(true);
                }
                keysym::LEFT | keysym::UP | keysym::ISO_LEFT_TAB => Some(cursor.saturating_sub(1)),
                keysym::TAB if shift => Some(cursor.saturating_sub(1)),
                keysym::RIGHT | keysym::DOWN | keysym::TAB => Some((cursor + 1).min(n.saturating_sub(1))),
                _ => None,
            };
            if let Some(cursor) = moved {
                if let Some(state) = self.association.as_mut() {
                    state.cursor = cursor;
                }
                return Some(true);
            }
            if let Some(digit) = key.digit() {
                if (1..=n).contains(&digit) {
                    self.choose_association(digit - 1);
                }
                return Some(true);
            }
        } else if plain && mods == 0 {
            match key.keysym {
                keysym::TAB => {
                    if let Some(state) = self.association.as_mut() {
                        state.focused = true;
                        state.cursor = 0;
                    }
                    return Some(true);
                }
                keysym::ESCAPE => {
                    self.close_association();
                    return Some(true);
                }
                _ => {}
            }
        }
        self.close_association();
        None
    }

    /// 关掉联想。
    fn close_association(&mut self) {
        if self.association.take().is_some() {
            self.generation += 1;
        }
    }

    /// 联想开着（外壳画联想行、鼠标点了按 [`Session::choose`] 选）。
    pub fn association_open(&self) -> bool {
        self.association.is_some()
    }

    /// 能不能出联想：设置开着、有引擎、中文模式、没在打字、没开面板；五笔、「译」、密码框（不能翻译的输入框）不出。
    fn association_allowed(&self) -> bool {
        self.config.association
            && self.engine.is_some()
            && self.mode == Mode::Chinese
            && !self.config.wubi
            && !self.composing()
            && self.tool.is_none()
            && self.hints & key_hints::NO_TRANSLATE == 0
            && self.translation.source.is_empty()
    }

    /// 一次调用结束：这次选词上屏了（后面没再上屏别的）就按上文出联想。
    fn settle_association(&mut self) {
        if !std::mem::take(&mut self.association_due) || !self.association_allowed() {
            return;
        }
        let Some(engine) = self.engine else {
            return;
        };
        let just = self.last_choice.as_ref().map(|c| c.text.clone()).unwrap_or_default();
        let plain = engine.associate(&self.context, &just, self.page_size().min(9));
        if plain.is_empty() {
            return;
        }
        let items = plain
            .iter()
            .map(|word| {
                let text = engine.shown_text(word);
                Candidate {
                    commit: text.clone(),
                    text,
                    kind: CandidateKind::Word,
                    ..Candidate::default()
                }
            })
            .collect();
        self.association = Some(Association {
            items,
            plain,
            focused: false,
            cursor: 0,
        });
        self.generation += 1;
    }

    /// 选第 index 个联想：上屏，和普通选词一样记一次选词（学搭配），接着出下一轮联想。
    fn choose_association(&mut self, index: usize) -> bool {
        // 序号不对什么也不做（联想留着）
        if !self.association.as_ref().is_some_and(|a| index < a.items.len()) {
            return false;
        }
        let Some(state) = self.association.take() else {
            return false;
        };
        self.generation += 1;
        let (Some(item), Some(plain)) = (state.items.get(index), state.plain.get(index)) else {
            return false;
        };
        self.commit_start = Some(self.learn_mark());
        if self.engine.is_some() {
            self.learn(plain, "");
            self.learned = true;
        }
        self.finish(item.commit.clone());
        self.last_choice = Some(item.clone());
        self.association_due = true;
        true
    }

    /// 这个键是上屏后紧接着的退格：不带修饰键（CapsLock、NumLock 不算），没在打字、没开面板，
    /// 也没在「译」里攒原文（那时退格删的是原文）。
    fn backspace_undoes(&self, key: &Key) -> bool {
        key.keysym == keysym::BACKSPACE
            && key.action == KeyAction::None
            && key.modifiers & !(modifier::CAPS | modifier::NUMLOCK) == 0
            && !self.composing()
            && self.tool.is_none()
            && self.translation.source.is_empty()
    }

    /// 学习日志现在的位置；没有引擎是 0。
    fn learn_mark(&self) -> u64 {
        self.engine.map_or(0, Engine::learn_mark)
    }

    /// 撤销学习日志 span 里学到的东西。
    fn unlearn(&self, span: &std::ops::Range<u64>) {
        if let Some(engine) = self.engine.filter(|_| !span.is_empty()) {
            engine.undo_learning(span.start, span.end);
        }
    }

    /// 应用告诉了光标前的字（读得到周围文字）、没在打字：会话记的上文要和它对得上。
    /// 对不上（在应用里删过、改过、点到别处）就换成应用里的那个字，光标前没有字就清空。
    /// 「译」里上文是攒着的原文，应用里是译文，不核对。
    fn check_context(&mut self, before: Option<char>) {
        if self.composing() || self.tool.is_some() || self.translating() {
            return;
        }
        if self.context.chars().next_back() != before {
            self.set_context(&before.map(String::from).unwrap_or_default());
        }
    }

    fn handle_key(&mut self, key: &Key) -> bool {
        if self.tool.is_some() {
            return self.tool_key(key);
        }
        match key.action {
            KeyAction::ToggleMode => {
                self.switch_mode(if self.mode == Mode::Chinese { Mode::English } else { Mode::Chinese });
                return true;
            }
            KeyAction::ToggleTranslate => return self.toggle_translate(),
            _ => {}
        }
        if self.mode == Mode::English && !self.composing() {
            return self.english_key(key);
        }
        if !self.translation.source.is_empty() && !self.translating() {
            // 「译」里攒着原文时换到了不能翻译的输入框（密码框）：原文先原样上屏
            self.commit_source();
        }
        if self.translating() && !self.composing() {
            if let Some(handled) = self.translate_key(key) {
                return handled;
            }
        }
        if self.composing() && key.is_forget() {
            self.forget_current();
            return true;
        }
        if key.has_command_modifier() {
            return false;
        }
        if self.composing() {
            if let Some(handled) = self.composing_key(key) {
                return handled;
            }
        }
        let sym = key.keysym;
        // 打拼音途中的隔音符（xi'an）下面单独处理；打五笔码时它是标点
        if let Some(c) = visible_symbol(sym).filter(|&c| !(c == '\'' && self.composing() && !self.wubi_code())) {
            return self.symbol_key(c, key);
        }
        if (u32::from(b'a')..=u32::from(b'z')).contains(&sym)
            || (u32::from(b'A')..=u32::from(b'Z')).contains(&sym)
        {
            let c = char::from(sym as u8);
            if self.wubi() && !self.temp_pinyin() {
                self.wubi_letter(c);
            } else {
                // 大写字母原样保留：Hello、iPhone 这类英文候选要用到
                self.push_input(c);
            }
            return true;
        }
        if self.composing() && sym == keysym::APOSTROPHE {
            // 隔音符（xi'an）；开头的隔音符没有意义
            self.push_input('\'');
            return true;
        }
        false
    }

    /// 换翻译器（None：没有翻译器，「译」里也照常上屏）。换掉时攒着的原文丢掉。
    pub fn set_translator(&mut self, translator: Option<&'e dyn TranslateBackend>) {
        let same = match (self.translator, translator) {
            (Some(a), Some(b)) => std::ptr::addr_eq(a as *const dyn TranslateBackend, b as *const dyn TranslateBackend),
            (None, None) => true,
            _ => false,
        };
        if same {
            return;
        }
        self.clear_translation();
        self.translator = translator;
    }

    /// 互译中：「译」、有翻译器、输入框能翻译（不是密码框）。
    pub fn translating(&self) -> bool {
        self.mode == Mode::Translate
            && self.translator.is_some()
            && self.hints & key_hints::NO_TRANSLATE == 0
    }

    /// 攒着的原文（互译）。
    pub fn translate_source(&self) -> &str {
        &self.translation.source
    }

    /// 丢掉攒着的原文，取消这个会话发的请求（输入框失去焦点、放弃输入）。
    pub fn clear_translation(&mut self) {
        let t = &mut self.translation;
        t.source.clear();
        t.shown.clear();
        t.pending = false;
        t.due_ms = None;
        if let Some(sent) = t.requested.take() {
            if let Some(translator) = self.translator {
                translator.cancel_source(&sent);
            }
        }
    }

    /// 定时器到了（外壳按 frame 的 wake_after_ms 排的）：该发请求的发，按过回车且译好了的上屏。
    pub fn tick(&mut self, now_ms: u64) -> Frame {
        self.last_choice = None;
        self.now_ms = now_ms;
        self.poll_translation();
        self.frame(true)
    }

    fn poll_translation(&mut self) {
        let Some(translator) = self.translator else {
            return;
        };
        if self.translation.source.is_empty() {
            self.translation.due_ms = None;
            if let Some(sent) = self.translation.requested.take() {
                translator.cancel_source(&sent);
            }
            return;
        }
        if !self.translating() {
            return;
        }
        if self.translation.due_ms.is_some_and(|due| self.now_ms >= due) {
            self.translation.due_ms = None;
            self.request_translation();
        }
        let snap = translator.poll();
        let fresh = snap.source == self.translation.source;
        if fresh && snap.state == JobState::Error {
            self.translation.pending = false;
        }
        if self.translation.pending && fresh && snap.state == JobState::Done {
            self.commit_translation();
        }
    }

    fn request_translation(&mut self) {
        if let Some(translator) = self.translator {
            translator.start(&self.translation.source);
            self.translation.requested = Some(self.translation.source.clone());
        }
    }

    /// 原文变了：过 TRANSLATE_DELAY_MS（now 时立刻）再发请求。
    fn source_changed(&mut self, now: bool) {
        self.translation.pending = false;
        if self.translation.source.is_empty() {
            self.clear_translation();
            return;
        }
        let delay = if now { 0 } else { TRANSLATE_DELAY_MS };
        self.translation.due_ms = Some(self.now_ms + delay);
    }

    /// 回车、空格：译好了就上屏译文；还没译好（或出错了，回车就是重试）记下来，好了再上屏。
    fn commit_translation(&mut self) {
        let Some(translator) = self.translator else {
            return;
        };
        if self.translation.source.is_empty() {
            return;
        }
        let snap = translator.poll();
        let fresh = snap.source == self.translation.source;
        if fresh && snap.state == JobState::Done {
            self.translation.requested = None;
            self.clear_translation();
            self.push_commit(&snap.text);
            return;
        }
        self.translation.pending = true;
        if !fresh || matches!(snap.state, JobState::Error | JobState::Idle) {
            self.translation.due_ms = None;
            self.request_translation();
        }
    }

    /// Shift+回车、离开「译」：原文原样上屏。
    fn commit_source(&mut self) {
        let source = std::mem::take(&mut self.translation.source);
        self.clear_translation();
        self.push_commit(&source);
    }

    /// 互译、没在打拼音时的键。Some：处理完了；None：照常处理（字母开始拼音，标点、数字进原文）。
    fn translate_key(&mut self, key: &Key) -> Option<bool> {
        if key.modifiers & (modifier::CTRL | modifier::ALT | modifier::SUPER) != 0 {
            return None;
        }
        if self.translation.source.is_empty() {
            return None;
        }
        match key.keysym {
            keysym::RETURN | keysym::KP_ENTER => {
                if key.modifiers & modifier::SHIFT != 0 {
                    self.commit_source();
                } else {
                    self.commit_translation();
                }
                Some(true)
            }
            keysym::SPACE => {
                self.commit_translation();
                Some(true)
            }
            keysym::ESCAPE => {
                self.clear_translation();
                Some(true)
            }
            keysym::BACKSPACE => {
                self.translation.source.pop();
                self.source_changed(false);
                Some(true)
            }
            keysym::LEFT | keysym::RIGHT | keysym::UP | keysym::DOWN | keysym::HOME | keysym::END
            | keysym::DELETE | keysym::PAGE_UP | keysym::PAGE_DOWN | keysym::TAB
            | keysym::ISO_LEFT_TAB => {
                // 原文里没有光标，别让应用里的光标乱跑
                Some(true)
            }
            _ => None,
        }
    }

    /// 互译时外壳要画的，顺便看要不要再叫会话（等发请求、等译文）。
    fn translate_view(&mut self) -> Option<(TranslateView, u32)> {
        if !self.translating() {
            return None;
        }
        let translator = self.translator?;
        let snap = translator.poll();
        let t = &mut self.translation;
        let fresh = !t.source.is_empty() && snap.source == t.source;
        let failed = fresh && snap.state == JobState::Error;
        let done = fresh && snap.state == JobState::Done;
        let text = if fresh && !failed { snap.text.as_str() } else { "" };
        if t.source.is_empty() || failed {
            t.shown.clear();
        } else if done || text.len() >= t.shown.len() {
            t.shown = text.to_owned();
        }
        let status = if failed {
            format!("{}（回车重试）", snap.error)
        } else if t.pending {
            "翻译中…好了就上屏".to_owned()
        } else if !t.source.is_empty() && !done {
            "翻译中…".to_owned()
        } else if !t.source.is_empty() && self.input.is_empty() {
            "回车上屏 · Shift+回车上屏原文 · Esc 清空".to_owned()
        } else {
            String::new()
        };
        // 翻译器上跑的是这个会话发的请求才接着轮询（别的输入框的请求不用这边盯）
        let running =
            t.requested.as_deref() == Some(snap.source.as_str()) && snap.state == JobState::Running;
        let wake = if t.due_ms.is_some() || t.pending || running {
            TRANSLATE_POLL_MS
        } else {
            0
        };
        let view = TranslateView {
            source: t.source.clone(),
            text: t.shown.clone(),
            done,
            title: format!("{} 互译：", translator.tag(&t.source)),
            status,
        };
        Some((view, wake))
    }

    /// 开着的面板，None 是没开。
    pub fn open_panel(&self) -> Option<Tool> {
        self.tool.as_ref().map(|t| t.tool)
    }

    /// 打开工具栏的一页或表情面板（外壳的菜单、按钮用；v + 空格会话自己开）。打到一半的丢掉。
    pub fn open_tool(&mut self, tool: Tool) -> Frame {
        self.last_choice = None;
        self.open_tool_now(tool, String::new());
        self.frame(true)
    }

    /// 关上工具栏 / 表情面板。
    pub fn close_panel(&mut self) -> Frame {
        self.last_choice = None;
        self.close_tool();
        self.frame(true)
    }

    /// 表情面板快捷键：开着就关上。打了一半的拼音拿来当搜索词（xiao + 快捷键 → 笑相关的 emoji）；
    /// 已经选了词的，先把打的原样上屏（和回车一样）。
    pub fn toggle_emoji_panel(&mut self) -> Frame {
        self.last_choice = None;
        if matches!(self.open_panel(), Some(Tool::Emoji { .. })) {
            self.close_tool();
            return self.frame(true);
        }
        let mut query = String::new();
        if self.wubi_code() {
            // 五笔码不能拿来搜：先顶字（空码丢掉），和打码途中按别的键一样
            self.commit_current();
        } else if self.composing() {
            if self.selections.is_empty() {
                query = self.remaining()[self.pinyin_skip()..]
                    .chars()
                    .filter(char::is_ascii_alphabetic)
                    .map(|c| c.to_ascii_lowercase())
                    .collect();
                self.clear();
            } else {
                self.commit_input_now();
            }
        }
        self.open_tool_now(Tool::Emoji { symbols: false }, query);
        self.frame(true)
    }

    fn opens_toolbar(&self) -> bool {
        self.config.toolbar
            && !self.shuangpin()
            && !self.wubi()
            && !self.translating()
            && !self.expanded
            && self.input == "v"
            && self.selections.is_empty()
    }

    fn open_tool_now(&mut self, tool: Tool, query: String) {
        self.close_association();
        if self.composing() {
            self.clear();
        }
        let mut state = ToolState {
            tool,
            items: Vec::new(),
            cursor: 0,
            query,
        };
        let item = |shown: String, commit: String| Candidate {
            text: shown,
            commit,
            kind: CandidateKind::Plain,
            ..Candidate::default()
        };
        match tool {
            Tool::Menu => {
                state.items = TOOL_MENU.iter().map(|m| item((*m).to_owned(), String::new())).collect();
            }
            Tool::Clipboard => {
                if self.config.clipboard_history {
                    state.items = crate::clipboard::history()
                        .into_iter()
                        .map(|text| item(tool_line(&text), text))
                        .collect();
                }
            }
            Tool::Phrases => {
                if let Some(engine) = self.engine {
                    // 自定义词按 custom.tsv 的顺序：「编码　词」
                    state.items = engine
                        .lexicon()
                        .custom_phrases()
                        .into_iter()
                        .map(|(code, word)| {
                            let shown = if code.is_empty() {
                                tool_line(&word)
                            } else {
                                format!("{code}\u{3000}{}", tool_line(&word))
                            };
                            item(shown, crate::unescape_custom(&word))
                        })
                        .collect();
                }
            }
            Tool::Emoji { .. } => {}
        }
        self.tool = Some(state);
        if matches!(tool, Tool::Emoji { .. }) {
            self.search_emoji();
        }
        self.generation += 1;
    }

    fn close_tool(&mut self) {
        if self.tool.take().is_some() {
            self.generation += 1;
        }
    }

    /// 按搜索词重新列表情面板，光标回到第一个。
    fn search_emoji(&mut self) {
        let engine = self.engine;
        let Some(state) = self.tool.as_mut() else {
            return;
        };
        let Tool::Emoji { symbols } = state.tool else {
            return;
        };
        let mut texts: Vec<String> = Vec::new();
        let mut add = |text: String| {
            if !texts.contains(&text) {
                texts.push(text);
            }
        };
        if state.query.is_empty() && symbols {
            crate::symbols::SYMBOLS.iter().for_each(|s| add(s.text.to_owned()));
        } else {
            if state.query.is_empty() {
                crate::recent_emoji::list().into_iter().for_each(&mut add);
            }
            // 拼音（经引擎转成中文词）、英文、符号名一起找；引擎还没载入好时只按英文和符号名
            crate::emoji::panel_search(engine, &state.query, MAX_EMOJI)
                .into_iter()
                .for_each(&mut add);
        }
        state.items = texts
            .into_iter()
            .map(|text| Candidate {
                commit: text.clone(),
                emoji: text.clone(),
                text,
                kind: CandidateKind::Emoji,
                ..Candidate::default()
            })
            .collect();
        state.cursor = 0;
        self.generation += 1;
    }

    /// 工具栏或表情面板开着时的按键。返回 false 的只有修饰键本身和带 Ctrl / Alt / Super 的组合键
    /// （收起面板，交给应用：复制粘贴等）。别的键都吃掉，免得打进应用。
    fn tool_key(&mut self, key: &Key) -> bool {
        if is_modifier_keysym(key.keysym) {
            return false;
        }
        if key.modifiers & (modifier::CTRL | modifier::ALT | modifier::SUPER) != 0 {
            self.close_tool();
            return false;
        }
        let Some(state) = self.tool.clone() else {
            return false;
        };
        if matches!(state.tool, Tool::Emoji { .. }) {
            self.emoji_key(key, state);
        } else {
            self.list_key(key, state);
        }
        true
    }

    fn list_key(&mut self, key: &Key, state: ToolState) {
        let n = state.items.len();
        let page = state.cursor / TOOL_PAGE * TOOL_PAGE;
        let mut cursor = state.cursor;
        match key.keysym {
            keysym::ESCAPE => return self.close_tool(),
            keysym::BACKSPACE => {
                // 退格返回上一层，在菜单里就关上
                if state.tool == Tool::Menu {
                    self.close_tool();
                } else {
                    self.open_tool_now(Tool::Menu, String::new());
                }
                return;
            }
            keysym::SPACE | keysym::RETURN | keysym::KP_ENTER => {
                if n == 0 {
                    self.close_tool();
                } else {
                    self.select_tool(cursor.min(n - 1));
                }
                return;
            }
            keysym::DOWN | keysym::RIGHT if n > 0 => cursor = (cursor + 1).min(n - 1),
            keysym::UP | keysym::LEFT => cursor = cursor.saturating_sub(1),
            keysym::PAGE_DOWN | keysym::EQUAL | keysym::PERIOD | keysym::BRACKET_RIGHT => {
                if page + TOOL_PAGE < n {
                    cursor = page + TOOL_PAGE;
                }
            }
            keysym::PAGE_UP | keysym::MINUS | keysym::COMMA | keysym::BRACKET_LEFT => {
                cursor = page.saturating_sub(TOOL_PAGE);
            }
            sym => {
                if let Some(d) = digit_of(sym).filter(|&d| d >= 1) {
                    if page + d - 1 < n {
                        self.select_tool(page + d - 1);
                    }
                }
                return;
            }
        }
        if let Some(state) = self.tool.as_mut() {
            state.cursor = cursor;
        }
    }

    fn emoji_key(&mut self, key: &Key, state: ToolState) {
        let n = state.items.len();
        let sym = key.keysym;
        match sym {
            keysym::ESCAPE => return self.close_tool(),
            keysym::BACKSPACE => {
                // 删搜索词的最后一个字母，删空了再按就关上
                if state.query.is_empty() {
                    return self.close_tool();
                }
                if let Some(state) = self.tool.as_mut() {
                    state.query.pop();
                }
                return self.search_emoji();
            }
            keysym::SPACE | keysym::RETURN | keysym::KP_ENTER => {
                if n == 0 {
                    self.close_tool();
                } else {
                    self.commit_tool(state.cursor.min(n - 1));
                }
                return;
            }
            _ => {}
        }
        if let Some(c) = char::from_u32(sym).filter(char::is_ascii_alphabetic) {
            if let Some(state) = self.tool.as_mut() {
                state.query.push(c.to_ascii_lowercase());
            }
            return self.search_emoji();
        }
        if n == 0 {
            return;
        }
        let screen = GRID_SCREEN;
        let mut cursor = state.cursor;
        match sym {
            keysym::LEFT => cursor = cursor.saturating_sub(1),
            keysym::RIGHT => cursor = (cursor + 1).min(n - 1),
            // 第一行再往上：不动（不像展开面板那样收起）
            keysym::UP if cursor >= GRID_COLS => cursor -= GRID_COLS,
            keysym::UP => {}
            keysym::DOWN => {
                if cursor + GRID_COLS < n {
                    cursor += GRID_COLS;
                } else if cursor / GRID_COLS + 1 < n.div_ceil(GRID_COLS) {
                    cursor = n - 1; // 最后一行不满时落到最后一个
                }
            }
            keysym::PAGE_DOWN | keysym::EQUAL => cursor = (cursor + screen).min(n - 1),
            keysym::PAGE_UP | keysym::MINUS => {
                cursor = if cursor >= screen {
                    cursor - screen
                } else {
                    cursor % GRID_COLS
                };
            }
            sym => {
                if let Some(d) = digit_of(sym).filter(|d| (1..=GRID_COLS).contains(d)) {
                    let index = cursor / GRID_COLS * GRID_COLS + d - 1;
                    if index < n {
                        self.commit_tool(index);
                    }
                }
                return;
            }
        }
        if let Some(state) = self.tool.as_mut() {
            state.cursor = cursor;
        }
    }

    /// 选工具栏里的第 index 项：菜单进下一层，剪贴板、常用语、表情上屏。
    fn select_tool(&mut self, index: usize) {
        let Some(state) = self.tool.as_ref() else {
            return;
        };
        if index >= state.items.len() {
            return;
        }
        if state.tool != Tool::Menu {
            return self.commit_tool(index);
        }
        let next = match index {
            0 => Tool::Clipboard,
            1 => Tool::Phrases,
            2 => Tool::Emoji { symbols: true },
            _ => Tool::Emoji { symbols: false },
        };
        self.open_tool_now(next, String::new());
    }

    fn commit_tool(&mut self, index: usize) {
        let Some(state) = self.tool.take() else {
            return;
        };
        self.generation += 1;
        let Some(item) = state.items.get(index) else {
            return;
        };
        let text = item.commit.clone();
        self.emit(&text);
        if state.tool == (Tool::Emoji { symbols: false }) {
            crate::recent_emoji::remember(&text);
        }
        self.last_choice = Some(item.clone());
    }

    /// 工具栏 / 表情面板开着时的 frame：候选是面板的条目，编辑区是搜索词。
    fn tool_frame(&self, frame: &mut Frame) {
        let Some(state) = &self.tool else {
            return;
        };
        let n = state.items.len();
        let emoji = matches!(state.tool, Tool::Emoji { .. });
        let size = if emoji { GRID_SCREEN } else { TOOL_PAGE };
        let cursor = state.cursor.min(n.saturating_sub(1));
        let start = cursor / size * size;
        frame.panel = if emoji { Panel::Emoji } else { Panel::Tool };
        frame.cand_start = start;
        frame.cand_count = n.saturating_sub(start).min(size);
        frame.cand_cursor = (n > 0).then_some(cursor - start);
        frame.has_prev = start > 0;
        frame.has_next = start + size < n;
        frame.preedit = state.query.clone();
        frame.preedit_cursor = state.query.len();
        frame.aux = self.tool_hint(state);
    }

    fn tool_hint(&self, state: &ToolState) -> String {
        let empty = state.items.is_empty();
        match state.tool {
            Tool::Menu => "工具栏：数字键选择，Esc 关闭".to_owned(),
            Tool::Clipboard if !empty => "剪贴板：选一条上屏，退格返回".to_owned(),
            Tool::Clipboard if self.config.clipboard_history => {
                "剪贴板：还没有记录，复制过的文字会出现在这里".to_owned()
            }
            Tool::Clipboard => "剪贴板：没打开，在设置里打开「记录剪贴板历史」".to_owned(),
            Tool::Phrases if empty => "常用语：还没有，在设置的「自定义词」里添加".to_owned(),
            Tool::Phrases => "常用语：选一条上屏，退格返回".to_owned(),
            Tool::Emoji { symbols } => {
                let name = if symbols { "符号" } else { "表情" };
                if state.query.is_empty() {
                    format!("{name}：打拼音或英文搜索，Esc 关闭")
                } else if empty {
                    format!("{name}：没找到「{}」", state.query)
                } else {
                    format!("{name}：{}", state.query)
                }
            }
        }
    }

    /// 候选框里能选的全部条目：开着工具栏 / 表情面板时是面板的，否则是候选（C 接口读这个）。
    pub fn display_candidates(&mut self) -> &[Candidate] {
        if self.tool.is_some() {
            return self.tool.as_ref().map_or(&[], |t| t.items.as_slice());
        }
        if self.association.is_some() {
            return self.association.as_ref().map_or(&[], |a| a.items.as_slice());
        }
        self.candidates()
    }

    /// 外壳要上屏一段与拼音无关的字（工具栏、表情面板、语音识别结果）：互译时进原文，
    /// 否则照会话的规矩（补空格、记上文）放进 frame 的 commit。prev_char 是应用里光标前的字（读不到 None）。
    pub fn commit_text(&mut self, text: &str, prev_char: Option<char>) -> Frame {
        self.last_choice = None;
        self.prev_char = prev_char;
        self.emit(text);
        self.frame(true)
    }

    /// 现在的输入模式。
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// 切到 mode（外部命令、设置里的默认模式、外壳全局的模式）。切到英文时打到一半的原样上屏。
    pub fn set_mode(&mut self, mode: Mode) -> Frame {
        self.last_choice = None;
        self.switch_mode(mode);
        self.frame(true)
    }

    fn switch_mode(&mut self, mode: Mode) {
        if mode == self.mode {
            return;
        }
        if self.mode == Mode::Translate || mode == Mode::Translate {
            // 「译」里打的字进原文，上屏的是译文：数不清配对里有几个字。中 ↔ 英照旧（括号里打英文）
            self.pairs.clear();
        }
        self.close_association();
        if self.mode == Mode::Translate {
            // 离开「译」：攒着的原文原样上屏，正在打的拼音留着接着打
            self.commit_source();
        }
        if mode == Mode::English && self.composing() {
            // 打到一半切英文：已选的词 + 打的字母原样上屏（和微信输入法一样）
            self.commit_input_now();
        }
        if mode == Mode::Translate {
            self.translate_from = self.mode;
        }
        self.mode = mode;
        self.mode_changed = true;
    }

    /// 单按 Shift 循环切换：中 → 英 →（有「译」、没另设进出「译」的键时）译 → 中。
    fn toggle_mode(&mut self) {
        let next = match self.mode {
            Mode::Chinese => Mode::English,
            Mode::English if self.config.translate_mode && !self.config.translate_key => Mode::Translate,
            Mode::English | Mode::Translate => Mode::Chinese,
        };
        self.switch_mode(next);
    }

    /// 进出「译」的快捷键：在「译」里回到进来前的模式（翻译不能用了也回得去）；不在就进「译」，
    /// 打到一半的拼音带进去接着打（和单按 Shift 一样）。进不了时返回 false，键交给应用。
    fn toggle_translate(&mut self) -> bool {
        if self.mode == Mode::Translate {
            self.switch_mode(self.translate_from);
        } else if self.config.translate_mode && self.hints & key_hints::NO_TRANSLATE == 0 {
            self.switch_mode(Mode::Translate);
        } else {
            return false;
        }
        true
    }

    /// Shift 的按下、松开，以及打断单按的别的键。Some：处理完了（handled 是里面的值，
    /// 单按切换了模式时为 true）；None：接着照常处理（Shift 的按下也接着处理：展开面板要收起）。
    ///
    /// 和以前 Linux 外壳的规矩一样：按下时带着 Ctrl / Alt / Super、外壳开着面板时不算；
    /// 按住不放的自动重复不重新计时；松开时按键码认（没有键码时按符号）；
    /// 按下以后按了别的键（按下或松开）就不算；按住超过 shift_tap_ms 不算。
    fn shift_key(&mut self, key: &Key) -> Option<bool> {
        let sym = key.keysym;
        let is_shift = sym == keysym::SHIFT_L || sym == keysym::SHIFT_R;
        if !key.release {
            if !is_shift {
                self.shift.down = None;
                return None;
            }
            let wanted = match self.config.shift_toggle {
                ShiftToggle::Both => true,
                ShiftToggle::Left => sym == keysym::SHIFT_L,
                ShiftToggle::Right => sym == keysym::SHIFT_R,
                ShiftToggle::Off => false,
            };
            let other = key.modifiers & (modifier::CTRL | modifier::ALT | modifier::SUPER) != 0;
            let panel = key.hints & key_hints::PANEL_OPEN != 0 || self.tool.is_some();
            if wanted && !other && !panel {
                let held = matches!(self.shift.down, Some((s, _, _)) if s == sym) && key.repeat;
                if !held {
                    self.shift.down = Some((sym, key.code, key.time_ms));
                }
            } else {
                self.shift.down = None;
            }
            return None;
        }
        let Some((down_sym, down_code, down_at)) = self.shift.down else {
            return is_shift.then_some(false);
        };
        let same = if key.code != 0 && down_code != 0 {
            key.code == down_code
        } else {
            sym == down_sym
        };
        if !same {
            if !is_shift {
                self.shift.down = None;
                return None;
            }
            return Some(false);
        }
        self.shift.down = None;
        let held_ms = key.time_ms.saturating_sub(down_at);
        if key.time_ms != 0 && down_at != 0 && held_ms > u64::from(self.config.shift_tap_ms) {
            return Some(false);
        }
        self.toggle_mode();
        Some(true)
    }

    /// 英文模式、没在打字：按键交给应用；汉字后面打字母、数字要补空格时自己连空格一起上屏。
    fn english_key(&mut self, key: &Key) -> bool {
        if key.modifiers & (modifier::CTRL | modifier::ALT | modifier::SUPER) != 0 {
            return false;
        }
        let Some(c) = char::from_u32(key.keysym).filter(|c| c.is_ascii_graphic()) else {
            return false;
        };
        if c.is_ascii_alphanumeric() && self.space_before(c) {
            self.emit(&c.to_string());
            return true;
        }
        self.last_char = Some(c);
        false
    }

    /// 字母、隔音符以外的可见 ASCII 字符（标点、符号、数字），会话的选词、翻页规则都没用上它时。
    ///
    /// - 正在输入：先上屏候选（光标停在句首词上时接着选，直到整句上屏），再上屏这个字符
    ///   （全角样式下有中文标点的换成中文标点，数字后面的保持英文）
    /// - 没在输入：有中文标点的换成中文标点上屏；数字前面要补空格（汉字后面打数字）就自己上屏；
    ///   别的交给应用
    fn symbol_key(&mut self, c: char, key: &Key) -> bool {
        if key.modifiers & (modifier::CTRL | modifier::ALT | modifier::SUPER) != 0 {
            return false;
        }
        if self.composing() {
            for _ in 0..16 {
                if !self.composing() {
                    break;
                }
                self.commit_current();
            }
            let prev = self.last_char;
            if self.auto_pair_key(c, prev) {
                return true;
            }
            let text = self.punct_text(c, prev).unwrap_or_else(|| c.to_string());
            self.emit(&text);
            return true;
        }
        if self.translating() {
            // 互译时打的字都进原文：数字、没有中文写法的符号、数字后面的点（3.14）也是
            let text = self.punct_text(c, self.last_char).unwrap_or_else(|| c.to_string());
            self.emit(&text);
            return true;
        }
        if c.is_ascii_digit() {
            if self.space_before(c) {
                self.emit(&c.to_string());
                return true;
            }
            self.last_char = Some(c);
            return false;
        }
        if self.auto_pair_key(c, self.last_char) {
            return true;
        }
        match self.punct_text(c, self.last_char) {
            Some(text) => {
                self.emit(&text);
                true
            }
            None => {
                self.last_char = Some(c);
                false
            }
        }
    }

    /// 自动配对（[`Config::auto_pair_punct`]，全角标点、不在「译」里）。处理了返回 true。
    ///
    /// - 右标点正好是最里面那一对的右半边：不上屏，光标右移跳过补好的那个。不管前面是什么字，
    ///   数字后面也跳（（3）），应用说光标后面不是它时（光标早挪走了）配对作废、照常上屏
    /// - 左标点（数字后面的照常是半角，不配对）：上屏成对的，光标左移一格放到中间
    fn auto_pair_key(&mut self, c: char, prev: Option<char>) -> bool {
        if !self.config.auto_pair_punct || self.config.punct_style != PunctStyle::Fullwidth || self.translating() {
            return false;
        }
        if let Some(top) = self.pairs.last().copied() {
            if crate::symbols::closing_punct(c) == Some(top.close) {
                if self.hints & key_hints::NEXT_CHAR == 0 || self.next_char == Some(top.close) {
                    self.skip_pair(top);
                    return true;
                }
                self.pairs.clear();
            }
        }
        if prev.is_some_and(|p| p.is_ascii_digit()) {
            return false;
        }
        let Some((open, close)) = crate::symbols::auto_pair(c) else {
            return false;
        };
        self.emit(&format!("{open}{close}"));
        // 光标在两个中间：光标前的字、上文是左半边；右半边不算进外面那一对（跳过它时才算）
        self.context.pop();
        self.prev_char = Some(open);
        self.last_char = Some(open);
        if let Some(outer) = self.pairs.last_mut() {
            outer.inner = outer.inner.saturating_sub(1);
        }
        if self.pairs.len() >= MAX_PAIRS {
            self.pairs.remove(0);
        }
        self.pairs.push(Pair { open, close, inner: 0 });
        self.cursor_delta -= 1;
        true
    }

    /// 跳过最里面那一对补好的右半边 top（光标右移一格）。
    fn skip_pair(&mut self, top: Pair) {
        self.pairs.pop();
        if let Some(outer) = self.pairs.last_mut() {
            outer.inner += top.inner + 1;
        }
        self.cursor_delta += 1;
        self.prev_char = Some(top.close);
        self.last_char = Some(top.close);
        self.context = char_tail(&format!("{}{}", self.context, top.close), CONTEXT_CHARS);
        self.fresh = false;
        // 和上屏了一个标点一样：退格不再撤销刚学的词，联想也不出了
        self.undo_commit = None;
        self.association_due = false;
        self.close_association();
    }

    /// 上屏了 n 个字（光标前多了 n 个字）：算进最里面那一对。
    fn pair_typed(&mut self, n: usize) {
        if let Some(top) = self.pairs.last_mut() {
            top.inner += n;
        }
    }

    /// 交给应用的键对配对的影响。打的字、空格算进最里面那一对；退格少算一个字，空的一对两个一起删
    /// （[`Frame::delete_next`]）；Esc 不动光标；别的键（方向键、Home、End、翻页、回车、Tab、Delete、
    /// 带 Ctrl、Alt 的快捷键……）光标可能挪走了、右半边可能没了，配对作废。
    fn pair_key_passed(&mut self, key: &Key) {
        if self.pairs.is_empty() {
            return;
        }
        let mods = key.modifiers & !(modifier::CAPS | modifier::NUMLOCK);
        let plain = key.action == KeyAction::None && mods & (modifier::CTRL | modifier::ALT | modifier::SUPER) == 0;
        if plain && typed_key(key.keysym) {
            self.pair_typed(1);
        } else if plain && mods == 0 && key.keysym == keysym::BACKSPACE && !self.composing() && self.tool.is_none() {
            self.pair_backspace(key);
        } else if !(plain && key.keysym == keysym::ESCAPE) {
            self.pairs.clear();
        }
    }

    /// 交给应用的退格：最里面那一对是空的（左半边就在光标前）就两个一起删，不然少算一个字。
    /// 应用告诉了光标两边的字时先核对，对不上是光标早挪走了，配对作废。
    fn pair_backspace(&mut self, key: &Key) {
        let Some(&top) = self.pairs.last() else {
            return;
        };
        let after_ok = key.hints & key_hints::NEXT_CHAR == 0 || key.next_char == Some(top.close);
        if !after_ok {
            self.pairs.clear();
        } else if top.inner > 0 {
            if let Some(top) = self.pairs.last_mut() {
                top.inner -= 1;
            }
        } else if key.hints & key_hints::SURROUNDING == 0 || key.prev_char == Some(top.open) {
            self.pairs.pop();
            // 外面那一对少了这个左半边
            if let Some(outer) = self.pairs.last_mut() {
                outer.inner = outer.inner.saturating_sub(1);
            }
            self.delete_next = true;
        } else {
            self.pairs.clear();
        }
    }

    /// c 在全角样式下换成的中文标点；半角样式、数字后面、没有中文标点的是 None。
    fn punct_text(&mut self, c: char, prev: Option<char>) -> Option<String> {
        crate::symbols::punct_with(c, self.config.punct_style, prev, &mut self.quotes).map(str::to_owned)
    }

    /// 上屏 text 之前要不要补一个空格（汉字和字母、数字之间，设置里的 AutoSpace）。
    /// 只看应用里光标前那个字，读不到（外壳没给）、密码框网址框邮箱框都不补。
    fn space_before(&self, first: char) -> bool {
        self.config.auto_space != 0
            && self.hints & key_hints::NO_AUTO_SPACE == 0
            && self
                .prev_char
                .is_some_and(|prev| crate::autospace::needs_space(prev, first, self.config.auto_space))
    }

    /// 上屏一段字（接在这次调用已经要上屏的后面）：要补空格就补，记下光标前的字和上文。
    fn emit(&mut self, text: &str) {
        let Some(first) = text.chars().next() else {
            return;
        };
        // 后面又上屏了别的（标点、空格）：退格删的是它，不是刚才学的词；联想也不出了
        self.undo_commit = None;
        self.association_due = false;
        self.close_association();
        if self.translating() {
            // 互译：上屏的字攒成原文，过一会儿（句末标点立刻）发请求
            self.translation.source.push_str(text);
            self.source_changed(ends_sentence(text));
            self.remember(text);
            return;
        }
        let spaced = if self.space_before(first) {
            format!(" {text}")
        } else {
            text.to_owned()
        };
        self.pair_typed(spaced.chars().count());
        self.pending.get_or_insert_with(String::new).push_str(&spaced);
        self.remember(text);
    }

    /// 上屏了 text（或进了原文）：记下光标前的字和上文。
    fn remember(&mut self, text: &str) {
        let last = text.chars().last();
        self.prev_char = last;
        self.last_char = last;
        self.context = char_tail(&(self.context.clone() + text), CONTEXT_CHARS);
        self.fresh = false;
    }

    /// 直接上屏（译文、原文），不进原文、不补空格。
    fn push_commit(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.undo_commit = None;
        self.association_due = false;
        self.close_association();
        self.pair_typed(text.chars().count());
        self.pending.get_or_insert_with(String::new).push_str(text);
        self.remember(text);
    }

    /// 正在输入时的键（字母、隔音符除外）。Some：处理完了（handled 是里面的值）；
    /// None：接着当字母、隔音符看，都不是就交给外壳。
    fn composing_key(&mut self, key: &Key) -> Option<bool> {
        let sym = key.keysym;
        // 先把候选算好：候选重算时候选框回到开头，别在读页和光标以后才重算
        let n = self.candidates().len();
        if self.expanded {
            if n == 0 {
                self.expanded = false;
            } else if let Some(handled) = self.grid_key(key, n) {
                return Some(handled);
            }
        }
        match sym {
            keysym::BACKSPACE => {
                // 选过词先撤销最后一次选词，拼音回到编辑区（后面还没选的拼音不动），
                // 和微信输入法一样；没有选词了才删字母
                let at = self.caret();
                if at < self.input.len() {
                    // 光标在拼音中间（#129）：删光标前一个；光标在最前面时撤销选词，没有就不动
                    if at > self.rest_start() {
                        let prev = self.input[..at].chars().next_back().map_or(1, char::len_utf8);
                        self.input.replace_range(at - prev..at, "");
                        self.caret = Some(at - prev);
                    } else {
                        self.pop_selection();
                    }
                } else if !self.pop_selection() {
                    self.input.pop();
                }
                self.fresh = false;
                if self.input.is_empty() {
                    self.clear();
                }
                return Some(true);
            }
            keysym::ESCAPE => {
                self.clear();
                return Some(true);
            }
            keysym::SPACE if self.opens_toolbar() => {
                // 只打了一个 v 再按空格：打开工具栏（vim、vscode 这些照常打字，v1+2*3 照常算算式）
                self.clear();
                self.open_tool(Tool::Menu);
                return Some(true);
            }
            keysym::SPACE => return Some(self.commit_current()),
            keysym::RETURN | keysym::KP_ENTER => {
                // 回车：已选的词 + 原始字母上屏
                self.commit_input_now();
                return Some(true);
            }
            _ => {}
        }
        if self.arrow_moves_caret() {
            if let Some(handled) = self.caret_key(sym) {
                return Some(handled);
            }
        }
        if self.config.calculator {
            // V 模式：v 后面的数字、运算符进编辑区（不选词、不翻页、不当标点）。双拼时是大写 V
            if sym == keysym::EQUAL && self.calc_active() && self.input.len() > 1 {
                // = 上屏计算结果（第一个候选）
                return Some(self.commit_current());
            }
            if let Some(c) = calc_char(sym) {
                if self.calc_active() {
                    self.push_input(c);
                    return Some(true);
                }
            }
        }
        if let Some(nth) = self.select_key_index(sym) {
            // 分号选第 2 个、引号选第 3 个；这一页没这么多就什么都不做
            let (start, len) = self.page_range(n);
            if nth <= len {
                self.choose_now(start + nth - 1);
            }
            return Some(true);
        }
        if let Some(last) = self.pick_char_key(sym) {
            // 以词定字：没有能拆的候选时交给外壳（照常当标点）
            if self.pick_char(last) {
                return Some(true);
            }
            return None;
        }
        if let Some(digit) = key.digit() {
            // 数字按当前页选词
            let (start, len) = self.page_range(n);
            if (1..=len).contains(&digit) {
                self.choose_now(start + digit - 1);
            }
            return Some(true);
        }
        if sym == keysym::SEMICOLON && self.semicolon_is_key() {
            // 微软、搜狗、紫光双拼用分号当 ing：等第二个键时分号进编辑区，不当标点
            self.push_input(';');
            return Some(true);
        }
        if n == 0 {
            return None;
        }
        let arrow = matches!(
            sym,
            keysym::UP | keysym::DOWN | keysym::LEFT | keysym::RIGHT
        );
        if !arrow && key.action != KeyAction::None {
            // 全局的翻页、移光标热键：翻不动也吃掉，不让它变成标点把字上屏
            match key.action {
                KeyAction::PrevPage => self.prev_page(),
                KeyAction::NextPage => self.next_page(),
                KeyAction::PrevCandidate => self.move_cursor(true),
                KeyAction::NextCandidate => self.move_cursor(false),
                // OBSERVE 在 key() 里就返回了，切模式的在 handle_key 开头，到不了这里
                KeyAction::None | KeyAction::Observe | KeyAction::ToggleMode | KeyAction::ToggleTranslate => {}
            }
            return Some(true);
        }
        // 横排时左右键移动候选、上下键翻页；竖排反过来。左右键用来移拼音里的光标时（上面 caret_key
        // 处理完了，到不了这里）和竖排一样上下键选词，下键也不展开面板
        let horizontal = self.config.horizontal && !self.arrow_moves_caret();
        let (prev_key, next_key, prev_page_key, next_page_key) = if horizontal {
            (keysym::LEFT, keysym::RIGHT, keysym::UP, keysym::DOWN)
        } else {
            (keysym::UP, keysym::DOWN, keysym::LEFT, keysym::RIGHT)
        };
        if horizontal && sym == keysym::DOWN && n > 1 {
            // 横排时下键展开候选面板（照微信输入法），光标停在当前选中的词上
            self.expanded = true;
            self.grid_cursor = self.cursor;
            return Some(true);
        }
        if sym == prev_key {
            self.move_cursor(true);
            return Some(true);
        }
        if sym == next_key {
            self.move_cursor(false);
            return Some(true);
        }
        // 设置里能选的翻页键（- =、[ ]、, .、Shift+Tab Tab）
        let keys = self.config.page_keys;
        let shift = key.modifiers & modifier::SHIFT != 0;
        let tab = keys & page_keys::TAB != 0;
        let char_prev = (keys & page_keys::MINUS != 0 && sym == keysym::MINUS)
            || (keys & page_keys::BRACKET != 0 && sym == keysym::BRACKET_LEFT)
            || (keys & page_keys::COMMA != 0 && sym == keysym::COMMA)
            || (tab && (sym == keysym::ISO_LEFT_TAB || (sym == keysym::TAB && shift)));
        let char_next = (keys & page_keys::MINUS != 0 && sym == keysym::EQUAL)
            || (keys & page_keys::BRACKET != 0 && sym == keysym::BRACKET_RIGHT)
            || (keys & page_keys::COMMA != 0 && sym == keysym::PERIOD)
            || (tab && sym == keysym::TAB && !shift);
        let (has_prev, has_next) = (self.has_prev_page(), self.has_next_page(n));
        if (sym == keysym::PAGE_UP || sym == prev_page_key || char_prev) && has_prev {
            self.prev_page();
            return Some(true);
        }
        if (sym == keysym::PAGE_DOWN || sym == next_page_key || char_next) && has_next {
            self.next_page();
            return Some(true);
        }
        if (char_prev || char_next) && (has_prev || has_next) {
            // 已经在第一页 / 最后一页：吃掉，不当标点上屏（只有一页时照常当标点）
            return Some(true);
        }
        None
    }

    /// 展开面板里的键。Some：处理完了；None：收起了面板，接着照常处理这个键。
    fn grid_key(&mut self, key: &Key, n: usize) -> Option<bool> {
        let mut cursor = self.grid_cursor.min(n - 1);
        match key.keysym {
            keysym::LEFT => cursor = cursor.saturating_sub(1),
            keysym::RIGHT => {
                if cursor + 1 < n {
                    cursor += 1;
                }
            }
            keysym::UP => {
                if cursor < GRID_COLS {
                    // 第一行再往上：收起
                    self.collapse();
                    return Some(true);
                }
                cursor -= GRID_COLS;
            }
            keysym::DOWN => {
                if cursor + GRID_COLS < n {
                    cursor += GRID_COLS;
                } else if cursor / GRID_COLS + 1 < n.div_ceil(GRID_COLS) {
                    cursor = n - 1; // 最后一行不满时落到最后一个
                }
            }
            keysym::PAGE_DOWN | keysym::EQUAL => cursor = (cursor + GRID_SCREEN).min(n - 1),
            keysym::PAGE_UP | keysym::MINUS => {
                cursor = if cursor >= GRID_SCREEN {
                    cursor - GRID_SCREEN
                } else {
                    cursor % GRID_COLS
                };
            }
            keysym::ESCAPE => {
                self.collapse();
                return Some(true);
            }
            keysym::SPACE => {
                self.choose_now(cursor);
                return Some(true);
            }
            _ => {
                if let Some(digit) = key.digit() {
                    // 数字选光标所在行的第几个
                    if (1..=GRID_COLS).contains(&digit) {
                        let index = cursor / GRID_COLS * GRID_COLS + digit - 1;
                        if index < n {
                            self.choose_now(index);
                        }
                    }
                    return Some(true);
                }
                // 打字、退格、回车、标点：先收起，再照常处理
                self.collapse();
                return None;
            }
        }
        self.grid_cursor = cursor;
        Some(true)
    }

    /// 收起展开面板：光标是面板里的光标，翻到它所在的那一页。
    fn collapse(&mut self) {
        self.expanded = false;
        self.cursor = self.grid_cursor;
        self.page = self.cursor / self.page_size();
    }

    fn page_size(&self) -> usize {
        self.config.page_size.max(1)
    }

    /// 当前页在全部候选里的起点和个数。
    fn page_range(&self, n: usize) -> (usize, usize) {
        let start = self.page * self.page_size();
        (start, n.saturating_sub(start).min(self.page_size()))
    }

    /// 光标在当前页的第几个；不在这一页上是 None。
    fn cursor_on_page(&self) -> Option<usize> {
        let size = self.page_size();
        (self.cursor / size == self.page).then_some(self.cursor % size)
    }

    fn has_prev_page(&self) -> bool {
        self.page > 0
    }

    fn has_next_page(&self, n: usize) -> bool {
        self.page + 1 < n.div_ceil(self.page_size())
    }

    /// 上一页。光标原来在这一页上的话跟着到新的一页的第一个（和 fcitx5 一样）。
    fn prev_page(&mut self) {
        if self.has_prev_page() {
            let on_page = self.cursor_on_page().is_some();
            self.page -= 1;
            if on_page {
                self.cursor = self.page * self.page_size();
            }
        }
    }

    fn next_page(&mut self) {
        if self.has_next_page(self.candidates.len()) {
            let on_page = self.cursor_on_page().is_some();
            self.page += 1;
            if on_page {
                self.cursor = self.page * self.page_size();
            }
        }
    }

    /// 移光标：光标不在这一页上时落到这一页的头 / 尾；否则走一格，跨页时翻到光标所在的页。
    /// 到了全部候选的两头就停住，不绕到另一头：第一个往前一按就跳到末页的生僻字，
    /// 再顺手按空格就上屏了它（展开面板、工具栏到头也是停住）。
    fn move_cursor(&mut self, prev: bool) {
        let n = self.candidates.len();
        if n == 0 {
            return;
        }
        let (start, len) = self.page_range(n);
        if len == 0 {
            return;
        }
        if self.cursor_on_page().is_none() {
            self.cursor = start + if prev { len - 1 } else { 0 };
            return;
        }
        self.cursor = if prev {
            self.cursor.saturating_sub(1)
        } else {
            (self.cursor + 1).min(n - 1)
        };
        self.page = self.cursor / self.page_size();
    }

    /// 空格：选光标处的候选（光标不在这一页上时选这一页第一个），没有候选就原样上屏。
    /// 五笔空码不上屏字母，丢掉（空格清空；标点、顶字只上屏后面的）。
    fn commit_current(&mut self) -> bool {
        let n = self.candidates().len();
        if n == 0 {
            if self.wubi_code() {
                self.clear();
            } else {
                self.commit_input_now();
            }
            return true;
        }
        let index = self.page * self.page_size() + self.cursor_on_page().unwrap_or(0);
        self.choose_now(index.min(n - 1));
        true
    }

    /// 光标处的候选（空格选的那个）：展开面板时是面板的光标。没有候选是 None。
    fn current_index(&mut self) -> Option<usize> {
        let n = self.candidates().len();
        if n == 0 {
            return None;
        }
        let index = if self.expanded {
            self.grid_cursor
        } else {
            self.page * self.page_size() + self.cursor_on_page().unwrap_or(0)
        };
        Some(index.min(n - 1))
    }

    /// Ctrl+Delete：让引擎忘掉光标处的候选（用户词库里的词、选词热度、搭配），
    /// 候选原地重算，编辑区不动，候选框停在原来的页和位置。只管引擎的词（计算结果、
    /// emoji、日期时间这类没学过）；什么也没学过、是自定义词时什么也不做。
    /// 五笔码的候选是码表里的，五笔也不学，没有可忘的（同一个词拼音学过的不能顺手删掉）。
    fn forget_current(&mut self) {
        let Some(engine) = self.engine else {
            return;
        };
        if self.wubi_code() {
            return;
        }
        let Some(index) = self.current_index() else {
            return;
        };
        let chosen = &self.candidates[index];
        if chosen.kind != CandidateKind::Word || !engine.forget_word(&chosen.text) {
            return;
        }
        self.forgot = true;
        let (page, cursor, expanded, grid_cursor) =
            (self.page, self.cursor, self.expanded, self.grid_cursor);
        self.fresh = false;
        let n = self.candidates().len();
        if n == 0 {
            return;
        }
        let last_page = (n - 1) / self.page_size();
        self.page = page.min(last_page);
        self.cursor = cursor.min(n - 1);
        self.expanded = expanded && n > 1;
        self.grid_cursor = grid_cursor.min(n - 1);
    }

    /// 以词定字的键：`[` 是 Some(false)（第一个字），`]` 是 Some(true)（最后一个字）。
    fn pick_char_key(&self, sym: u32) -> Option<bool> {
        if !self.config.pick_char || self.config.page_keys & page_keys::BRACKET != 0 {
            return None;
        }
        match sym {
            keysym::BRACKET_LEFT => Some(false),
            keysym::BRACKET_RIGHT => Some(true),
            _ => None,
        }
    }

    /// 以词定字：光标处的候选只取第一个（last 时最后一个）字。没用完输入的候选（句首词）
    /// 取的字进已选栈，剩下的拼音接着选；用完输入的连同已选的词上屏。取出来的字不记选词
    /// （学的是词，不是这个字）。光标处不是引擎的多字词时返回 false。
    fn pick_char(&mut self, last: bool) -> bool {
        let Some(index) = self.current_index() else {
            return false;
        };
        let chosen = self.candidates[index].clone();
        let chars: Vec<char> = chosen.text.chars().collect();
        if chosen.kind != CandidateKind::Word || chars.len() < 2 {
            return false;
        }
        let pick = if last { chars[chars.len() - 1] } else { chars[0] };
        // 开着繁体时也取词库写法里的同一个字（字数对不上就不知道是哪个，留空）
        let plain: Vec<char> = self.plain_of(&chosen.text).chars().collect();
        let plain = if plain.len() == chars.len() {
            (if last { plain[plain.len() - 1] } else { plain[0] }).to_string()
        } else {
            String::new()
        };
        // 拼音按音节对上字数时带上这个字的读音（以后拼成词能学），对不上就不带
        let syllables: Vec<&str> = chosen.pinyin.split_whitespace().collect();
        let pinyin = if syllables.len() == chars.len() {
            (if last { syllables[syllables.len() - 1] } else { syllables[0] }).to_string()
        } else {
            String::new()
        };
        self.last_choice = None;
        self.last_choice_index = 0;
        if chosen.consumed < self.remaining().len() {
            let mark = self.learn_mark();
            self.push_selection(Selection {
                text: pick.to_string(),
                plain,
                pinyin,
                bytes: chosen.consumed,
                learn: mark..mark,
            });
        } else {
            let text = self.selected_text() + &pick.to_string();
            self.finish(text);
            self.association_due = true;
        }
        true
    }

    /// 分号、引号选第几个；不是选词键是 None。
    fn select_key_index(&self, sym: u32) -> Option<usize> {
        match (self.config.select_keys, sym) {
            (SelectKeys::Off, _) => None,
            // 微软、搜狗、紫光双拼里分号是 ing 键，不能拿来选词
            (_, keysym::SEMICOLON) => (!self.shuangpin_semicolon()).then_some(2),
            (SelectKeys::SemicolonQuote, keysym::APOSTROPHE) => Some(3),
            _ => None,
        }
    }

    fn shuangpin(&self) -> bool {
        self.engine.is_some_and(|e| e.config().shuangpin)
    }

    /// 五笔：设置里选了五笔，引擎也载入了码表。没有码表时照全拼打（编辑区上方提示），不会打不了字。
    fn wubi(&self) -> bool {
        self.config.wubi && self.engine.is_some_and(|e| e.wubi().is_some())
    }

    /// 五笔下 z 开头的临时拼音。看全部输入：选了句首词以后，剩下的就不以 z 开头了。
    fn temp_pinyin(&self) -> bool {
        self.wubi() && self.input.starts_with('z')
    }

    /// 正在打五笔码（不是临时拼音、计算器）。
    fn wubi_code(&self) -> bool {
        self.composing() && self.wubi() && !self.temp_pinyin() && !self.calc_active()
    }

    /// 还没选词的输入里，拼音前面要跳过几个字节：临时拼音还没选过词时是开头的 z。
    fn pinyin_skip(&self) -> usize {
        usize::from(self.temp_pinyin() && self.selections.is_empty())
    }

    /// 五笔下的字母（临时拼音以外）：
    /// - 已经打了四码、空码、按了 z 或大写字母、在算算式：先顶字（光标处的候选上屏，空码丢掉）；
    /// - 大写字母原样上屏（V 开头是计算器）；别的字母开始新码，z 开始临时拼音；
    /// - 打满四码、只有一个候选时直接上屏（设置里能关）。四码没有更长的码，emoji 推荐不算。
    fn wubi_letter(&mut self, c: char) {
        if self.composing()
            && (c == 'z'
                || c.is_ascii_uppercase()
                || self.calc_active()
                || self.remaining().len() >= crate::wubi::MAX_CODE
                || self.candidates().is_empty())
        {
            self.commit_current();
        }
        if c.is_ascii_uppercase() && !(c == 'V' && self.config.calculator) {
            self.emit(&c.to_string());
            return;
        }
        self.push_input(c);
        if self.config.wubi_auto_commit
            && self.wubi_code()
            && self.input.len() == crate::wubi::MAX_CODE
            && self.candidates().iter().filter(|c| c.kind != CandidateKind::Emoji).count() == 1
        {
            self.choose_now(0);
        }
    }

    /// 双拼方案拿分号当韵母键。
    fn shuangpin_semicolon(&self) -> bool {
        self.engine.is_some_and(|e| {
            let config = e.config();
            config.shuangpin && config.shuangpin_scheme.uses_semicolon()
        })
    }

    /// 双拼等第二个键（还没选词的部分按了奇数个键）时，分号是韵母键。
    fn semicolon_is_key(&self) -> bool {
        self.shuangpin_semicolon()
            && self.remaining().bytes().filter(|&c| c != b'\'').count() % 2 == 1
    }

    fn push_input(&mut self, c: char) {
        // 还没选词的部分最多 MAX_PENDING_BYTES 个键：再长的键吃掉不收。候选是每按一个键重算的，
        // 一长串只打声母的输入（zhzhzh…）耗时涨得比长度快，56 个字母时一个键要 200 ms
        if self.remaining().len() >= MAX_PENDING_BYTES && !self.calc_active() {
            return;
        }
        let at = self.caret();
        if at < self.input.len() {
            // 光标在拼音中间（#129）：插在光标处，光标跟着往后
            self.input.insert(at, c);
            self.caret = Some(at + c.len_utf8());
        } else {
            self.input.push(c);
        }
        self.fresh = false;
    }

    /// 清空编辑区。commit_pending：把已选的词和还没选的原始输入上屏（切换输入法时），
    /// 和回车一样也记一次选词；互译时拼音先进原文，原文再原样上屏。
    ///
    /// 不上屏是放弃（失去焦点、应用重置了输入框、换了词库）：打到一半的丢掉，会话记的光标前的字、
    /// 上文也不算数了（回来时光标可能在别处）。互译攒着的原文不动，要丢掉用 [`Session::clear_translation`]。
    pub fn reset(&mut self, commit_pending: bool) -> Frame {
        self.last_choice = None;
        // 切走输入法、失去焦点、点鼠标挪了光标（应用会重置输入法）：会话看不到之后光标去了哪
        self.pairs.clear();
        self.close_association();
        self.last_choice_index = 0;
        if commit_pending && self.composing() {
            self.commit_input_now();
        } else {
            self.clear();
        }
        if !commit_pending {
            self.set_context("");
            self.prev_char = None;
            self.last_char = None;
            // 工具栏、表情面板跟着编辑区一起收：失去焦点后面板已经看不见了，会话里还开着的话，
            // 回来打的字母进了表情搜索，空格和数字上屏的是表情，单按 Shift 也切不了中英文
            self.close_tool();
            // 按着 Shift 时切走的，回来松开不算「单按 Shift」
            self.shift.down = None;
            self.undo_commit = None;
        }
        if commit_pending && !self.translation.source.is_empty() {
            self.commit_source();
        }
        self.frame(true)
    }

    /// 现在的编辑区和候选框（不处理按键）。这次调用要上屏的字也在里面。
    pub fn frame(&mut self, handled: bool) -> Frame {
        let n = self.candidates().len();
        let selected = self.selected_text();
        let (shown, shown_caret) = self.segmented_remaining();
        let preedit_cursor = selected.len() + shown_caret;
        let preedit = selected + &shown;
        let mut frame = Frame {
            handled,
            commit: self.pending.take(),
            forgot: std::mem::take(&mut self.forgot),
            learned: std::mem::take(&mut self.learned),
            mode: self.mode,
            mode_changed: std::mem::take(&mut self.mode_changed),
            translate: None,
            wake_after_ms: 0,
            aux: String::new(),
            preedit_cursor,
            preedit,
            cursor_delta: std::mem::take(&mut self.cursor_delta),
            delete_next: std::mem::take(&mut self.delete_next),
            ..Frame::default()
        };
        if self.tool.is_some() {
            self.tool_frame(&mut frame);
            return frame;
        }
        if let Some(state) = &self.association {
            frame.panel = Panel::Association;
            frame.cand_count = state.items.len();
            frame.cand_cursor = state.focused.then_some(state.cursor.min(state.items.len().saturating_sub(1)));
            frame.aux = if state.focused { ASSOCIATION_FOCUSED_HINT } else { ASSOCIATION_HINT }.to_owned();
            return frame;
        }
        if let Some((view, wake)) = self.translate_view() {
            frame.translate = Some(view);
            frame.wake_after_ms = wake;
        }
        frame.aux = self.notice(n);
        if n == 0 {
            return frame;
        }
        if self.expanded {
            let cursor = self.grid_cursor.min(n - 1);
            let start = cursor / GRID_SCREEN * GRID_SCREEN;
            frame.panel = Panel::Expanded;
            frame.cand_start = start;
            frame.cand_count = (n - start).min(GRID_SCREEN);
            frame.cand_cursor = Some(cursor - start);
            frame.has_prev = start > 0;
            frame.has_next = start + GRID_SCREEN < n;
        } else {
            let (start, len) = self.page_range(n);
            frame.cand_start = start;
            frame.cand_count = len;
            frame.cand_cursor = self.cursor_on_page();
            frame.has_prev = self.has_prev_page();
            frame.has_next = self.has_next_page(n);
        }
        frame
    }

    /// 打字时候选框上方的提示（n 是候选个数）：选了五笔却没有码表（照全拼打，说清楚为什么）、五笔空码。
    fn notice(&self, n: usize) -> String {
        if !self.composing() || !self.config.wubi {
            String::new()
        } else if self.engine.is_some_and(|e| e.wubi().is_none()) {
            "没有五笔码表，先按全拼打（重装 WowType 可以修好）".to_owned()
        } else if n == 0 && self.wubi_code() {
            "空码".to_owned()
        } else {
            String::new()
        }
    }

    /// 候选框的状态（过渡：fcitx5 外壳按它摆自己的候选列表）。
    pub fn view(&mut self) -> View {
        if let Some(state) = &self.tool {
            let emoji = matches!(state.tool, Tool::Emoji { .. });
            return View {
                page: state.cursor / TOOL_PAGE,
                cursor: state.cursor,
                expanded: emoji,
                grid_cursor: state.cursor,
            };
        }
        if let Some(state) = &self.association {
            return View {
                page: 0,
                cursor: if state.focused { state.cursor } else { 0 },
                expanded: false,
                grid_cursor: 0,
            };
        }
        self.candidates();
        View {
            page: self.page,
            cursor: self.cursor,
            expanded: self.expanded,
            grid_cursor: self.grid_cursor,
        }
    }

    /// 候选框里显示的候选（一页，或展开面板的一屏）。
    pub fn shown(&mut self) -> &[Candidate] {
        let frame = self.frame(false);
        // frame 会取走要上屏的字和几个标记；这里只是看，放回去
        self.pending = frame.commit;
        self.forgot = frame.forgot;
        self.learned = frame.learned;
        self.mode_changed = frame.mode_changed;
        let (start, count) = (frame.cand_start, frame.cand_count);
        &self.display_candidates()[start..start + count]
    }

    /// 上一次调用上屏的字是选哪个候选来的；原样上屏输入、没上屏是 None。
    pub fn last_choice(&self) -> Option<&Candidate> {
        self.last_choice.as_ref()
    }

    /// 上一次上屏的候选在列表里的位置（1 起）；不是选候选上屏时为 0。
    pub fn last_choice_index(&self) -> usize {
        self.last_choice_index
    }

    fn set_last_choice(&mut self, chosen: Candidate, index: usize) {
        self.last_choice = Some(chosen);
        self.last_choice_index = index.saturating_add(1);
    }

    /// 放弃编辑区（Esc、删光、失去焦点）：已选的句首词学到的东西也退回去（选错了才会放弃）。
    fn clear(&mut self) {
        while self.pop_selection() {}
        self.input.clear();
        self.caret = None;
        self.fresh = false;
    }

    /// 退回最后一个已选的词（退格、放弃），撤销选它时学到的。没有已选的词返回 false。
    fn pop_selection(&mut self) -> bool {
        let Some(selection) = self.selections.pop() else {
            return false;
        };
        self.unlearn(&selection.learn);
        self.fresh = false;
        true
    }

    /// 还没选词的那段在 input 里从哪个字节开始。
    fn rest_start(&self) -> usize {
        self.selected_bytes().min(self.input.len())
    }

    /// 编辑区光标在 input 里的字节位置：在还没选词的那段里（临时拼音的 z 后面），没移动过时在末尾。
    fn caret(&self) -> usize {
        self.caret.map_or(self.input.len(), |at| at.clamp(self.caret_start(), self.input.len()))
    }

    /// 光标最多往前挪到哪：还没选词的那段的开头；五笔临时拼音的 z 前面不去（在 z 前面打字就不是拼音了）。
    fn caret_start(&self) -> usize {
        self.rest_start() + self.pinyin_skip()
    }

    /// 方向键挪拼音里的光标（[`ArrowKeys::Caret`]）。打五笔码时不挪：最多四码，在里面挪光标没什么用，
    /// 方向键照「挑候选」用；五笔的临时拼音里照常挪。
    fn arrow_moves_caret(&self) -> bool {
        self.config.arrow_keys == ArrowKeys::Caret && !self.wubi_code()
    }

    /// 左右键移动拼音里的光标（[`ArrowKeys::Caret`]），Home、End、Delete 也管光标；到了两头照样吃掉。
    /// Some：处理完了；None：不是这几个键。
    fn caret_key(&mut self, sym: u32) -> Option<bool> {
        if !matches!(sym, keysym::LEFT | keysym::RIGHT | keysym::HOME | keysym::END | keysym::DELETE) {
            return None;
        }
        if self.translating() {
            // 「译」里拼音显示在候选框的原文框里，那里画不出光标：光标留在末尾
            return Some(true);
        }
        let start = self.caret_start();
        let at = self.caret();
        let len = self.input.len();
        let prev = self.input[..at].chars().next_back().map_or(0, char::len_utf8);
        let next = self.input[at..].chars().next().map_or(0, char::len_utf8);
        match sym {
            keysym::LEFT if at > start => self.caret = Some(at - prev),
            keysym::RIGHT if at < len => self.caret = (at + next < len).then_some(at + next),
            keysym::HOME => self.caret = Some(start),
            keysym::END => self.caret = None,
            keysym::DELETE if at < len => {
                self.input.replace_range(at..at + next, "");
                self.caret = (at < self.input.len()).then_some(at);
                self.fresh = false;
                if self.input.is_empty() {
                    self.clear();
                }
            }
            _ => {} // 到了两头
        }
        Some(true)
    }

    /// 光标在拼音里的位置换算到显示的样子（ni hao 这种分了音节的）里：显示的字是原始输入按顺序
    /// 加上分隔符时才换得出来，换不出来（双拼显示成全拼）返回 None。
    fn shown_caret(raw: &str, shown: &str, caret: usize) -> Option<usize> {
        if caret >= raw.len() {
            return Some(shown.len());
        }
        let mut consumed = 0;
        for (i, ch) in shown.char_indices() {
            if consumed == caret {
                return Some(i);
            }
            if raw[consumed..].starts_with(ch) {
                consumed += ch.len_utf8();
            }
        }
        None
    }

    /// 还没选词那部分输入的候选（全部，不分页）。没有引擎、没在打字时为空。
    /// 重算时候选框回到第一页、光标在第一个、收起展开面板。
    pub fn candidates(&mut self) -> &[Candidate] {
        if !self.fresh {
            self.candidates = self.compute_candidates();
            self.fresh = true;
            self.generation += 1;
            self.page = 0;
            self.cursor = 0;
            self.expanded = false;
            self.grid_cursor = 0;
        }
        &self.candidates
    }

    /// 候选重算过几次（C 接口用）。
    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    fn compute_candidates(&self) -> Vec<Candidate> {
        let input = self.remaining();
        let Some(engine) = self.engine else {
            return Vec::new();
        };
        if input.is_empty() {
            return Vec::new();
        }
        if self.calc_active() && input.len() > 1 {
            // V 模式：v 后面是算式，候选是结果和「算式=结果」
            return crate::calc::candidates(&input[1..])
                .into_iter()
                .map(|text| Candidate::plain(text, input.len()))
                .collect();
        }
        if self.wubi() && !self.temp_pinyin() {
            // 只打了一个 V（计算器）时还没有候选
            return if self.calc_active() {
                Vec::new()
            } else {
                self.wubi_candidates(engine, input)
            };
        }

        let skip = self.pinyin_skip();
        let pinyin = &input[skip..];
        if pinyin.is_empty() {
            return Vec::new();
        }
        // 已经选了开头的词时，把它当上文，剩下的部分排序更准
        let context = self.context.clone() + &self.selected_text();
        engine.set_context(utf8_tail(&context, CONTEXT_BYTES));
        let Ok(found) = engine.try_input(pinyin) else {
            return Vec::new();
        };
        let mut all: Vec<Candidate> = found
            .into_iter()
            .map(|c| {
                let consumed = if c.consumed == 0 || c.consumed > pinyin.len() {
                    pinyin.len()
                } else {
                    c.consumed
                };
                // 自定义词里带换行的，不往用户词库里记（词库是一行一条）
                let kind = if c.text.contains(['\n', '\t']) {
                    CandidateKind::Plain
                } else {
                    CandidateKind::Word
                };
                Candidate {
                    commit: c.text.clone(),
                    text: c.text,
                    pinyin: c.pinyin,
                    consumed: skip + consumed,
                    kind,
                    ..Candidate::default()
                }
            })
            .collect();
        if self.temp_pinyin() {
            // 临时拼音反查：候选后面是它的五笔码，用来学码
            if let Some(table) = engine.wubi() {
                for c in all.iter_mut().filter(|c| c.kind == CandidateKind::Word) {
                    c.comment = table.hint(&c.text).unwrap_or_default();
                }
            }
        }
        if self.config.emoji_suggest {
            add_emoji_suggestion(&mut all, input.len());
            add_keycap(&mut all, input.len(), self.config.page_size);
        }
        if self.config.date_time && self.selections.is_empty() {
            self.add_date_time(&mut all, input, skip, engine.config().shuangpin);
        }
        all
    }

    /// 五笔码的候选：自定义编码（用户定义的排最前）、码正好对上的码表词（码表的顺序），
    /// 再是 emoji 推荐（不占重码的码位），最后是逐码提示（以这个码开头的更长的码的词，后面带还要打的码）。
    /// 日期时间不在这里出：rq、sj 都是五笔码，日期、时间、星期的码又是四码唯一、一打就上屏了；
    /// 要日期时间打 z 临时拼音（zriqi）。
    fn wubi_candidates(&self, engine: &Engine, code: &str) -> Vec<Candidate> {
        let len = code.len();
        let candidate = |text: String, comment: String| Candidate {
            // 自定义词里带换行的原样上屏（和拼音一样）
            kind: if text.contains(['\n', '\t']) {
                CandidateKind::Plain
            } else {
                CandidateKind::Word
            },
            commit: text.clone(),
            text,
            consumed: len,
            comment,
            ..Candidate::default()
        };
        // custom.tsv 里编码正好是这个码的，编码像不像拼音都算（dd → 地址）；分了音节的（li si → 李四）
        // 是拼音的读法，不是要打的码，不算，免得占了码位
        let mut all: Vec<Candidate> = engine
            .lexicon()
            .custom_phrases()
            .into_iter()
            .filter(|(custom, _)| !custom.contains(' ') && custom.chars().filter(|&c| c != '\'').eq(code.chars()))
            .map(|(_, word)| candidate(crate::unescape_custom(&word), String::new()))
            .collect();
        let mut longer = Vec::new();
        for hit in engine.wubi().iter().flat_map(|table| table.lookup(code, WUBI_MORE)) {
            // 繁体输出时转完可能重复，自定义词也可能和码表的一样：只留前面那个，码位不动
            let text = engine.shown_text(hit.word);
            if all.iter().chain(&longer).any(|c: &Candidate| c.text == text) {
                continue;
            }
            if hit.code.len() == len {
                all.push(candidate(text, String::new()));
            } else {
                let rest = if self.config.wubi_hint { &hit.code[len..] } else { "" };
                longer.push(candidate(text, rest.to_owned()));
            }
        }
        if self.config.emoji_suggest {
            let mut emoji = all
                .first()
                .map(|top| emoji_suggestions(&top.text, len))
                .unwrap_or_default();
            emoji.extend(all.iter().find_map(|c| keycap_candidate(&c.text, len)));
            all.extend(emoji);
        }
        all.extend(longer);
        all
    }

    /// 日期时间：首选还是那个词（日期、时间），各种写法从第二个开始。
    /// skip：input 开头不算拼音的字节数（临时拼音的 z）。
    fn add_date_time(&self, all: &mut Vec<Candidate>, input: &str, skip: usize, shuangpin: bool) {
        let Some(first) = all.first() else {
            return;
        };
        // 全拼看打的字母（riqi、rq、date 等）；双拼看首选的拼音
        let key = if shuangpin {
            if first.consumed < input.len() {
                return;
            }
            first.pinyin.clone()
        } else {
            input[skip..].to_owned()
        };
        let Some(now) = (self.clock)() else {
            return;
        };
        let extra: Vec<Candidate> = crate::datetime::candidates(&key, &now)
            .into_iter()
            .map(|text| Candidate::plain(text, input.len()))
            .collect();
        if extra.is_empty() {
            return;
        }
        // 写法是给「时间」这个词的：接着前文打时它可能不在第一（日期后面接 shijian，
        // 语料里「…日事件」多，事件排第一），先把它挪到第一，写法再跟在后面
        if let Some(word) = crate::datetime::trigger_word(&key) {
            if let Some(at) = all.iter().position(|c| c.text == word).filter(|&at| at > 0) {
                let trigger = all.remove(at);
                all.insert(0, trigger);
            }
        }
        all.splice(1..1, extra);
    }

    /// V 模式（计算器）：没选过词、v 开头、后面只有数字和运算符。
    /// 双拼时 v 是声母键（v1 是「选第一个候选」）、五笔时 v 是码（一级简码「发」），改用大写 V（#160）。
    fn calc_active(&self) -> bool {
        if !self.config.calculator || !self.selections.is_empty() {
            return false;
        }
        let prefix = if self.shuangpin() || self.wubi() { 'V' } else { 'v' };
        let Some(rest) = self.input.strip_prefix(prefix) else {
            return false;
        };
        rest.bytes()
            .all(|c| c.is_ascii_digit() || b"+-*/%^.()".contains(&c))
    }

    /// 还没选词那部分输入的显示：全拼分音节（nihao → ni hao），双拼显示成全拼
    /// （vsgo → zhong'guo），和微信输入法一样；设置了 `shuangpin_keys` 时双拼显示打的键。
    /// 五笔码原样，临时拼音是 z + 分好音节的拼音。分不出来就原样。
    /// 连同光标在显示里的字节位置一起给。
    fn segmented_remaining(&mut self) -> (String, usize) {
        let shown = self.segmented_remaining_shown();
        let input = self.remaining();
        let rest_caret = self.caret() - self.rest_start();
        match Self::shown_caret(input, &shown, rest_caret) {
            Some(at) => (shown, at),
            // 光标在拼音中间、显示的样子换不回光标位置（双拼显示成全拼）：显示打的键
            None => (input.to_owned(), rest_caret),
        }
    }

    fn segmented_remaining_shown(&mut self) -> String {
        let input = self.remaining().to_owned();
        if self.calc_active() || self.wubi_code() {
            // 算式原样显示（双拼时不能当成按键换成全拼）
            return input;
        }
        if let Some(engine) = self.engine {
            if engine.config().shuangpin {
                if self.config.shuangpin_keys {
                    return input;
                }
                return engine
                    .shuangpin_display(&input)
                    .filter(|shown| !shown.is_empty())
                    .unwrap_or(input);
            }
        }
        let full = self
            .candidates()
            .iter()
            .find(|c| c.consumed >= input.len())
            .map(|c| c.pinyin.clone());
        let skip = self.pinyin_skip();
        match full.and_then(|full| crate::preedit::segment_input(&input[skip..], &full)) {
            Some(shown) => format!("{}{shown}", &input[..skip]),
            None => input,
        }
    }

    /// 正在输入（编辑区不空）。
    pub fn composing(&self) -> bool {
        !self.input.is_empty()
    }

    /// 全部原始输入。
    pub fn input(&self) -> &str {
        &self.input
    }

    /// 已选的词用掉的字节数。
    pub fn selected_bytes(&self) -> usize {
        self.selections.iter().map(|s| s.bytes).sum()
    }

    /// 还没选词的那部分输入。
    pub fn remaining(&self) -> &str {
        self.input.get(self.selected_bytes()..).unwrap_or("")
    }

    /// 已选的词连起来。
    pub fn selected_text(&self) -> String {
        self.selections.iter().map(|s| s.text.as_str()).collect()
    }

    pub fn selections(&self) -> &[Selection] {
        &self.selections
    }

    /// 选第 index 个候选（全部候选里的序号，不分页）。
    ///
    /// - emoji：上屏已选的词 + emoji（推荐在句尾对上时是整句 + emoji），不学习；
    /// - 计算结果、日期时间这类用完输入的：已选的词 + 它原样上屏，不学习；
    /// - 句首词（没用完输入）：进已选栈，记一次选词，剩下的接着选；
    /// - 其余：已选的词 + 它上屏，记选词（分几次选出来的整句也记一次，下次能一次打出来）。
    ///
    /// 序号不对时什么也不做（handled = false）。过渡：外壳的鼠标点候选、展开面板点一行用。
    pub fn choose(&mut self, index: usize) -> Frame {
        self.last_choice = None;
        self.last_choice_index = 0;
        if self.tool.is_some() {
            let ok = self.tool.as_ref().is_some_and(|t| index < t.items.len());
            if ok {
                self.select_tool(index);
            }
            return self.frame(ok);
        }
        if self.association.is_some() {
            // 联想行：鼠标点了一个（序号就是显示的第几个）
            let handled = self.choose_association(index);
            self.settle_association();
            return self.frame(handled);
        }
        let handled = self.choose_now(index);
        self.settle_association();
        self.frame(handled)
    }

    /// 选候选框里显示的第 index 个（一页，或展开面板的一屏），见 [`Session::choose`]。
    pub fn select(&mut self, index: usize) -> Frame {
        self.last_choice = None;
        self.last_choice_index = 0;
        if self.tool.is_some() {
            let shown = self.frame(false);
            return self.choose(shown.cand_start + index);
        }
        let shown = self.frame(false);
        self.pending = shown.commit;
        self.forgot = shown.forgot;
        self.learned = shown.learned;
        self.mode_changed = shown.mode_changed;
        if self.association.is_some() {
            let handled = self.choose_association(index);
            self.settle_association();
            return self.frame(handled);
        }
        let handled = index < shown.cand_count && self.choose_now(shown.cand_start + index);
        self.settle_association();
        self.frame(handled)
    }

    fn choose_now(&mut self, index: usize) -> bool {
        let Some(chosen) = self.candidates().get(index).cloned() else {
            return false;
        };
        let remaining = self.remaining().len();
        match chosen.kind {
            CandidateKind::Emoji => {
                let text = self.selected_text() + &chosen.commit;
                self.finish(text);
                // 打字推荐的 emoji：记进最近用过（表情面板里排最前）
                crate::recent_emoji::remember(&chosen.emoji);
                self.set_last_choice(chosen, index);
            }
            CandidateKind::Plain if chosen.consumed >= remaining => {
                let text = self.selected_text() + &chosen.text;
                self.finish(text);
                self.set_last_choice(chosen, index);
            }
            _ if chosen.consumed < remaining => {
                // 句首词：先放进已选栈，剩下的拼音继续选。学到的记下区间：退格退回、Esc 放弃时撤销
                let start = self.learn_mark();
                self.learn(&chosen.text, &chosen.pinyin);
                let end = self.learn_mark();
                self.push_selection(Selection {
                    plain: self.plain_of(&chosen.text),
                    text: chosen.text,
                    pinyin: chosen.pinyin,
                    bytes: chosen.consumed,
                    learn: start..end,
                });
            }
            _ => {
                self.commit_with(&chosen.text, &chosen.pinyin);
                // 选了引擎的词上屏（不是原样上屏的字母、英文）：出联想
                self.association_due = !chosen.pinyin.is_empty();
                self.set_last_choice(chosen, index);
            }
        }
        true
    }

    /// 上屏已选的词 + 还没选的原始输入（和回车、`reset(true)` 一样）。原始字母串不记选词。
    pub fn commit_input(&mut self) -> Frame {
        self.last_choice = None;
        self.last_choice_index = 0;
        self.commit_input_now();
        self.frame(true)
    }

    fn commit_input_now(&mut self) {
        let tail = self.remaining().to_owned();
        self.commit_with(&tail, "");
    }

    /// 已选的词 + tail 上屏，记选词，清空。
    fn commit_with(&mut self, tail: &str, tail_pinyin: &str) {
        let text = self.selected_text() + tail;
        self.commit_start = Some(self.learn_mark());
        // 回车、打字途中按标点上屏的原始字母串（没有拼音）不记：排序用不上它（原始串和英文候选的位置
        // 不看选词记录，没有拼音也收不进用户词库），记下来只是在 history.tsv 里留一份明文——
        // 终端里的 sudo 提示没有密码框标记，中文模式下打密码再回车，密码就这样进了输入习惯
        let raw_tail = tail_pinyin.is_empty() && tail.is_ascii();
        let learns = !self.selections.is_empty() || (!tail.is_empty() && !raw_tail);
        if self.engine.is_some() && learns && !self.wubi() && !self.sensitive() {
            self.learned = true;
            if !tail.is_empty() && !raw_tail {
                // 带上拼音：选够次数后引擎会自动把这个词收进用户词库
                self.learn(tail, tail_pinyin);
            }
            // 以词定字取出来的字对不上读音时拼音是空的，这样拼成的不学
            if !self.selections.is_empty()
                && !tail.is_empty()
                && !tail_pinyin.is_empty()
                && self.selections.iter().all(|s| !s.pinyin.is_empty())
            {
                // 分几次选出来的也记一次，下次能一次打出来。四个字以内的当成用户造的词，
                // 第二次这样拼出来就收进用户词库（「霜」「频」「阁」→ 霜频阁）；更长的是整句，照常按次数学。
                let mut pinyin = String::new();
                for selection in &self.selections {
                    pinyin.push_str(&selection.pinyin);
                    pinyin.push(' ');
                }
                pinyin.push_str(tail_pinyin);
                // 学词库里的写法（开着繁体时上屏的是转换过的字）
                let mut plain: String = self
                    .selections
                    .iter()
                    .map(|s| if s.plain.is_empty() { s.text.as_str() } else { s.plain.as_str() })
                    .collect();
                plain += &self.plain_of(tail);
                self.learn_composed(&plain, &pinyin);
            }
        }
        self.finish(text);
    }

    /// 密码框（外壳给的 [`key_hints::NO_LEARN`]）：什么都不学。
    fn sensitive(&self) -> bool {
        self.hints & key_hints::NO_LEARN != 0
    }

    /// 记一次选词。上文和算候选时一样（已上屏的字 + 已选的词）。
    /// 五笔（连同临时拼音）什么都不学：码位固定，选的词也不该进拼音的选词热度和用户词库。
    fn learn(&mut self, word: &str, pinyin: &str) {
        let Some(engine) = self.engine.filter(|_| !self.wubi() && !self.sensitive()) else {
            return;
        };
        let context = self.context.clone() + &self.selected_text();
        engine.set_context(utf8_tail(&context, CONTEXT_BYTES));
        engine.record_selection_with_pinyin(word, Some(pinyin).filter(|p| !p.is_empty()));
        // 外壳靠它数「有几次学习还没存」：选句首词进已选栈时也学了，之后按 Esc、失去焦点放弃，
        // 不报的话这次学习不计数，退出时可能不存
        self.learned = true;
    }

    /// 候选在词库里的写法（开着繁体时候选是转换过的），见 [`Engine::lexicon_form`]。
    fn plain_of(&self, shown: &str) -> String {
        self.engine
            .map_or_else(|| shown.to_string(), |engine| engine.lexicon_form(shown))
    }

    /// 记分几次选出来拼成的词，见 [`Engine::record_composed_word`]。上文是还没选词时的上文。
    fn learn_composed(&self, word: &str, pinyin: &str) {
        let Some(engine) = self.engine.filter(|_| !self.wubi() && !self.sensitive()) else {
            return;
        };
        engine.set_context(utf8_tail(&self.context, CONTEXT_BYTES));
        engine.record_composed_word(word, pinyin);
    }

    /// 上屏 text（空的不上屏，frame 取走），清空编辑区。已选的词连同这次学到的都算这次上屏的，
    /// 上屏后紧接着按退格时一起撤销。
    fn finish(&mut self, text: String) {
        let end = self.learn_mark();
        let start = self
            .selections
            .first()
            .map(|s| s.learn.start)
            .or(self.commit_start)
            .unwrap_or(end);
        self.commit_start = None;
        self.selections.clear();
        self.clear();
        self.emit(&text);
        if !text.is_empty() {
            self.undo_commit = (start < end).then_some(start..end);
        }
    }

    /// 选了句首词，压进已选栈（测试里不带引擎时用）。
    pub fn push_selection(&mut self, selection: Selection) {
        self.selections.push(selection);
        self.caret = None;
        self.fresh = false;
    }

    /// 过渡：外壳往原始输入后面加字（计算器的算式、双拼的分号、表情面板的搜索词）。
    pub fn append(&mut self, text: &str) {
        self.input.push_str(text);
        self.fresh = false;
    }
}

/// 首选是整句时按它推荐一个 emoji，插在第 [`EMOJI_SLOT`] 个。
fn add_emoji_suggestion(all: &mut Vec<Candidate>, input_len: usize) {
    let Some(first) = all.first() else {
        return;
    };
    if first.consumed < input_len {
        return;
    }
    let emoji = emoji_suggestions(&first.text, input_len);
    let at = EMOJI_SLOT.min(all.len());
    all.splice(at..at, emoji);
}

/// 按 top 推荐的 emoji 候选（最多一个）。
fn emoji_suggestions(top: &str, input_len: usize) -> Vec<Candidate> {
    crate::emoji::recommend(top, 1)
        .into_iter()
        .map(|s| Candidate {
            // 句尾对上时 commit 是「整句 + emoji」
            emoji: s.commit.strip_prefix(top).unwrap_or(&s.commit).to_owned(),
            text: s.display,
            consumed: input_len,
            kind: CandidateKind::Emoji,
            commit: s.commit,
            ..Candidate::default()
        })
        .collect()
}

/// 第一页里有数字字就在它后面跟按键 emoji：打 ba 是「吧 八 8️⃣ 把」，和手机输入法一样。
fn add_keycap(all: &mut Vec<Candidate>, input_len: usize, page_size: usize) {
    let window = all.len().min(page_size);
    for i in 0..window {
        let c = &all[i];
        if c.kind == CandidateKind::Emoji || c.consumed < input_len {
            continue;
        }
        if let Some(keycap) = keycap_candidate(&c.text, input_len) {
            all.insert(i + 1, keycap);
            return;
        }
    }
}

/// 数字字（八、捌）的按键 emoji 候选；不是数字字是 None。
fn keycap_candidate(text: &str, input_len: usize) -> Option<Candidate> {
    let keycap = crate::emoji::keycap(text)?;
    Some(Candidate {
        text: keycap.to_owned(),
        consumed: input_len,
        kind: CandidateKind::Emoji,
        commit: keycap.to_owned(),
        emoji: keycap.to_owned(),
        ..Candidate::default()
    })
}

/// 只留尾部 max_bytes 字节，起点对齐到字符边界。
/// text 最后 max_chars 个字。
fn char_tail(text: &str, max_chars: usize) -> String {
    let skip = text.chars().count().saturating_sub(max_chars);
    text.chars().skip(skip).collect()
}

/// 工具栏列表里一行显示的样子：换行显示成 ⏎，制表符换成空格，太长的截断（40 个字）。
fn tool_line(text: &str) -> String {
    const MAX: usize = 40;
    let mut out = String::new();
    for (i, c) in text.chars().enumerate() {
        if i == MAX {
            out.push('…');
            break;
        }
        match c {
            '\n' => out.push('⏎'),
            '\t' | '\r' => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

/// 数字键（主键盘、小键盘）的数字。
fn digit_of(sym: u32) -> Option<usize> {
    match sym {
        0x30..=0x39 => Some((sym - 0x30) as usize),
        keysym::KP_0..=0xffb9 => Some((sym - keysym::KP_0) as usize),
        _ => None,
    }
}

/// 句末（。！？…. ! ? 换行）：原文打到这里立刻发翻译请求。
fn ends_sentence(text: &str) -> bool {
    ["。", "！", "？", "…", ".", "!", "?", "\n"]
        .iter()
        .any(|end| text.ends_with(end))
}

/// 单独的修饰键（Shift、Ctrl、Alt、Super、CapsLock、AltGr……）。
fn is_modifier_keysym(sym: u32) -> bool {
    (0xffe1..=0xffee).contains(&sym) || (0xfe01..=0xfe13).contains(&sym)
}

/// 字母、空格以外的可见 ASCII 字符（标点、符号、数字）。
/// 交给应用后会打出一个字的键：可见 ASCII 字符、空格、小键盘的数字和运算符。
fn typed_key(sym: u32) -> bool {
    char::from_u32(sym).is_some_and(|c| c == ' ' || c.is_ascii_graphic())
        || (keysym::KP_0..=keysym::KP_9).contains(&sym)
        || matches!(
            sym,
            keysym::KP_MULTIPLY | keysym::KP_ADD | keysym::KP_SUBTRACT | keysym::KP_DECIMAL | keysym::KP_DIVIDE
        )
}

fn visible_symbol(sym: u32) -> Option<char> {
    let c = char::from_u32(sym).filter(|c| c.is_ascii_graphic())?;
    (!c.is_ascii_alphabetic()).then_some(c)
}

fn utf8_tail(text: &str, max_bytes: usize) -> &str {
    if text.len() <= max_bytes {
        return text;
    }
    let mut start = text.len() - max_bytes;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    &text[start..]
}

/// 本地时间（日期时间候选用）。
#[cfg(unix)]
pub fn system_time() -> Option<LocalTime> {
    let mut now: libc::time_t = 0;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: time 和 localtime_r 只写我们给的两个变量
    unsafe {
        libc::time(&mut now);
        if libc::localtime_r(&now, &mut tm).is_null() {
            return None;
        }
    }
    Some(LocalTime {
        year: tm.tm_year + 1900,
        month: (tm.tm_mon + 1).max(0) as u32,
        day: tm.tm_mday.max(0) as u32,
        hour: tm.tm_hour.max(0) as u32,
        minute: tm.tm_min.max(0) as u32,
        second: tm.tm_sec.max(0) as u32,
        weekday: tm.tm_wday.max(0) as u32,
    })
}

#[cfg(not(unix))]
pub fn system_time() -> Option<LocalTime> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(keys: &str) -> Session<'static> {
        let mut session = Session::new();
        for c in keys.chars() {
            session.key(&Key::char(c));
        }
        session
    }

    #[test]
    fn letters_and_apostrophe() {
        let session = typed("xi'an");
        assert_eq!(session.input(), "xi'an");
        // 开头的隔音符不进编辑区：没在打字时它是单引号，换成中文引号上屏
        let mut session = Session::new();
        let frame = session.key(&Key::char('\''));
        assert!(!session.composing());
        assert_eq!(frame.commit.as_deref(), Some("‘"));
    }

    #[test]
    fn command_modifiers_and_release_are_not_ours() {
        let mut session = typed("ni");
        let ctrl_a = Key {
            keysym: u32::from(b'a'),
            modifiers: modifier::CTRL,
            ..Key::default()
        };
        assert!(!session.key(&ctrl_a).handled);
        let release = Key {
            release: true,
            ..Key::press(keysym::BACKSPACE)
        };
        assert!(!session.key(&release).handled);
        assert_eq!(session.input(), "ni");
    }

    #[test]
    fn keys_outside_composition_are_not_ours() {
        let mut session = Session::new();
        assert!(!session.key(&Key::press(keysym::BACKSPACE)).handled);
        assert!(!session.key(&Key::press(keysym::ESCAPE)).handled);
    }

    #[test]
    fn reset_commits_selected_and_raw() {
        let mut session = typed("nihaozhongguo");
        session.push_selection(Selection {
            text: "你好".into(),
            pinyin: "ni hao".into(),
            bytes: 5,
            ..Default::default()
        });
        let frame = session.reset(true);
        assert_eq!(frame.commit.as_deref(), Some("你好zhongguo"));
        assert!(frame.preedit.is_empty());
        assert!(!session.composing());
        assert_eq!(session.reset(true).commit, None);
    }

    #[test]
    fn preedit_cursor_is_byte_offset() {
        let mut session = typed("nihaoma");
        session.push_selection(Selection {
            text: "你好".into(),
            pinyin: "ni hao".into(),
            bytes: 5,
            ..Default::default()
        });
        let frame = session.frame(true);
        assert_eq!(frame.preedit, "你好ma");
        assert_eq!(frame.preedit_cursor, "你好ma".len());
    }
}
