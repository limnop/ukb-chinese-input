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
//! 还没挪的键 [`Session::key`] 返回 `handled = false`，外壳接着用自己的旧逻辑处理
//! （打字途中的标点、其他可见字符、吃掉的光标键）。鼠标点候选按全局序号调
//! [`Session::choose`]，表情面板往编辑区加搜索词用 [`Session::append`]，这些过渡用法挪完以后会删掉。

use crate::datetime::LocalTime;
use crate::Engine;

/// 平台无关的键码：XKB keysym 的子集（fcitx5 直接用，macOS 外壳把 kVK_* 翻成这个）。
pub mod keysym {
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
}

/// 一次按键。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Key {
    pub keysym: u32,
    pub modifiers: u32,
    /// 松开（按下是 false）。
    pub release: bool,
    /// 光标前一个字符，不知道是 None（以后挪标点、小数点时用）。
    pub prev_char: Option<char>,
    /// 外壳认出来的全局热键（翻页、移光标），没有是 None。
    pub action: KeyAction,
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
    pub pinyin: String,
    /// 用掉的原始输入字节数。
    pub bytes: usize,
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
}

/// 候选框的样子。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Panel {
    /// 一页候选（横排或竖排）。
    #[default]
    Normal,
    /// 横排时按下键展开的面板：[`GRID_COLS`] 列，一屏 [`GRID_ROWS`] 行，光标所在行标 1–6。
    Expanded,
}

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
    /// V 模式计算器（v 后面打算式）。双拼时不开（v 是韵母键）。
    pub calculator: bool,
    /// 打 riqi、shijian 这类时给日期时间候选。
    pub date_time: bool,
    /// 打字时推荐 emoji（开心 → 😊，八 → 8️⃣）。
    pub emoji_suggest: bool,
    /// 候选横排（左右键移光标、上下键翻页、下键展开面板）；竖排时上下移光标、左右翻页。
    pub horizontal: bool,
    /// 翻页键，[`page_keys`] 的位掩码（PgUp PgDn 一直能用）。
    pub page_keys: u32,
    pub select_keys: SelectKeys,
    /// 以词定字：打拼音途中 `[` 上屏光标处候选的第一个字、`]` 上屏最后一个字（#83）。
    /// `[ ]` 设成翻页键时让给翻页。
    pub pick_char: bool,
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
/// emoji 推荐插在第几个（首选和次选后面）。
const EMOJI_SLOT: usize = 2;

/// 一个输入框的会话。只能在一个线程里用。
///
/// 引擎是后台载入的，载入好之前 `engine` 是 None（照样能打字，只是没有候选）。
#[derive(Clone)]
pub struct Session<'e> {
    /// 全部原始输入（包括已经选过词的部分）。
    input: String,
    selections: Vec<Selection>,
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
    /// 这次调用忘掉了学过的词（frame 取走）。
    forgot: bool,
}

impl Default for Session<'_> {
    fn default() -> Self {
        Self {
            input: String::new(),
            selections: Vec::new(),
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
            forgot: false,
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
            self.config = config;
            self.fresh = false;
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
        let handled = self.handle_key(key);
        self.frame(handled)
    }

    fn handle_key(&mut self, key: &Key) -> bool {
        if key.release {
            return false;
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
        if (u32::from(b'a')..=u32::from(b'z')).contains(&sym)
            || (u32::from(b'A')..=u32::from(b'Z')).contains(&sym)
        {
            // 大写字母原样保留：Hello、iPhone 这类英文候选要用到
            self.push_input(char::from(sym as u8));
            return true;
        }
        if self.composing() && sym == keysym::APOSTROPHE {
            // 隔音符（xi'an）；开头的隔音符没有意义
            self.push_input('\'');
            return true;
        }
        false
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
                if self.selections.pop().is_none() {
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
            keysym::SPACE => return Some(self.commit_current()),
            keysym::RETURN | keysym::KP_ENTER => {
                // 回车：已选的词 + 原始字母上屏
                self.commit_input_now();
                return Some(true);
            }
            _ => {}
        }
        if self.config.calculator && !self.shuangpin() {
            // V 模式：v 后面的数字、运算符进编辑区（不选词、不翻页、不当标点）
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
                KeyAction::None => {}
            }
            return Some(true);
        }
        // 横排时左右键移动候选、上下键翻页；竖排反过来
        let horizontal = self.config.horizontal;
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

    /// 移光标（和 fcitx5 的候选列表一样）：光标不在这一页上时落到这一页的头 / 尾；
    /// 否则走一格，走过头绕到全部候选的另一头，翻到光标所在的页。
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
        self.cursor = match (prev, self.cursor) {
            (true, 0) => n - 1,
            (true, c) => c - 1,
            (false, c) if c + 1 >= n => 0,
            (false, c) => c + 1,
        };
        self.page = self.cursor / self.page_size();
    }

    /// 空格：选光标处的候选（光标不在这一页上时选这一页第一个），没有候选就原样上屏。
    fn commit_current(&mut self) -> bool {
        let n = self.candidates().len();
        if n == 0 {
            self.commit_input_now();
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
    fn forget_current(&mut self) {
        let Some(engine) = self.engine else {
            return;
        };
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
        // 拼音按音节对上字数时带上这个字的读音（以后拼成词能学），对不上就不带
        let syllables: Vec<&str> = chosen.pinyin.split_whitespace().collect();
        let pinyin = if syllables.len() == chars.len() {
            (if last { syllables[syllables.len() - 1] } else { syllables[0] }).to_string()
        } else {
            String::new()
        };
        self.last_choice = None;
        if chosen.consumed < self.remaining().len() {
            self.push_selection(Selection {
                text: pick.to_string(),
                pinyin,
                bytes: chosen.consumed,
            });
        } else {
            let text = self.selected_text() + &pick.to_string();
            self.finish(text);
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
        self.input.push(c);
        self.fresh = false;
    }

    /// 清空编辑区。commit_pending：把已选的词和还没选的原始输入上屏（切换输入法时），
    /// 和回车一样也记一次选词。
    pub fn reset(&mut self, commit_pending: bool) -> Frame {
        self.last_choice = None;
        if commit_pending && self.composing() {
            self.commit_input_now();
        } else {
            self.clear();
        }
        self.frame(true)
    }

    /// 现在的编辑区和候选框（不处理按键）。这次调用要上屏的字也在里面。
    pub fn frame(&mut self, handled: bool) -> Frame {
        let n = self.candidates().len();
        let preedit = self.selected_text() + &self.segmented_remaining();
        let mut frame = Frame {
            handled,
            commit: self.pending.take(),
            forgot: std::mem::take(&mut self.forgot),
            preedit_cursor: preedit.len(),
            preedit,
            ..Frame::default()
        };
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

    /// 候选框的状态（过渡：fcitx5 外壳按它摆自己的候选列表）。
    pub fn view(&mut self) -> View {
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
        // frame 会取走要上屏的字；这里只是看，放回去
        self.pending = frame.commit;
        self.forgot = frame.forgot;
        &self.candidates[frame.cand_start..frame.cand_start + frame.cand_count]
    }

    /// 上一次调用上屏的字是选哪个候选来的；原样上屏输入、没上屏是 None。
    pub fn last_choice(&self) -> Option<&Candidate> {
        self.last_choice.as_ref()
    }

    fn clear(&mut self) {
        self.input.clear();
        self.selections.clear();
        self.fresh = false;
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

        // 已经选了开头的词时，把它当上文，剩下的部分排序更准
        let context = self.context.clone() + &self.selected_text();
        engine.set_context(utf8_tail(&context, CONTEXT_BYTES));
        let Ok(found) = engine.try_input(input) else {
            return Vec::new();
        };
        let mut all: Vec<Candidate> = found
            .into_iter()
            .map(|c| {
                let consumed = if c.consumed == 0 || c.consumed > input.len() {
                    input.len()
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
                    consumed,
                    kind,
                    emoji: String::new(),
                }
            })
            .collect();
        if self.config.emoji_suggest {
            add_emoji_suggestion(&mut all, input.len());
            add_keycap(&mut all, input.len(), self.config.page_size);
        }
        if self.config.date_time && self.selections.is_empty() {
            self.add_date_time(&mut all, input, engine.config().shuangpin);
        }
        all
    }

    /// 日期时间：首选还是那个词（日期、时间），各种写法从第二个开始。
    fn add_date_time(&self, all: &mut Vec<Candidate>, input: &str, shuangpin: bool) {
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
            input.to_owned()
        };
        let Some(now) = (self.clock)() else {
            return;
        };
        let extra = crate::datetime::candidates(&key, &now)
            .into_iter()
            .map(|text| Candidate::plain(text, input.len()));
        all.splice(1..1, extra);
    }

    /// V 模式（计算器）：没选过词、v 开头、后面只有数字和运算符。双拼时 v 是韵母键，不开。
    fn calc_active(&self) -> bool {
        let shuangpin = self.engine.is_some_and(|e| e.config().shuangpin);
        if !self.config.calculator || shuangpin || !self.selections.is_empty() {
            return false;
        }
        let Some(rest) = self.input.strip_prefix('v') else {
            return false;
        };
        rest.bytes()
            .all(|c| c.is_ascii_digit() || b"+-*/%^.()".contains(&c))
    }

    /// 还没选词那部分输入的显示：全拼分音节（nihao → ni hao），双拼显示成全拼
    /// （vsgo → zhong'guo），和微信输入法一样。分不出来就原样。
    fn segmented_remaining(&mut self) -> String {
        let input = self.remaining().to_owned();
        if let Some(engine) = self.engine {
            if engine.config().shuangpin {
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
        full.and_then(|pinyin| crate::preedit::segment_input(&input, &pinyin))
            .unwrap_or(input)
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
        let handled = self.choose_now(index);
        self.frame(handled)
    }

    /// 选候选框里显示的第 index 个（一页，或展开面板的一屏），见 [`Session::choose`]。
    pub fn select(&mut self, index: usize) -> Frame {
        self.last_choice = None;
        let shown = self.frame(false);
        self.pending = shown.commit;
        self.forgot = shown.forgot;
        let handled = index < shown.cand_count && self.choose_now(shown.cand_start + index);
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
                self.last_choice = Some(chosen);
            }
            CandidateKind::Plain if chosen.consumed >= remaining => {
                let text = self.selected_text() + &chosen.text;
                self.finish(text);
                self.last_choice = Some(chosen);
            }
            _ if chosen.consumed < remaining => {
                // 句首词：先放进已选栈，剩下的拼音继续选
                self.learn(&chosen.text, &chosen.pinyin);
                self.push_selection(Selection {
                    text: chosen.text,
                    pinyin: chosen.pinyin,
                    bytes: chosen.consumed,
                });
            }
            _ => {
                self.commit_with(&chosen.text, &chosen.pinyin);
                self.last_choice = Some(chosen);
            }
        }
        true
    }

    /// 上屏已选的词 + 还没选的原始输入，原始输入也记一次选词（和回车、`reset(true)` 一样）。
    pub fn commit_input(&mut self) -> Frame {
        self.last_choice = None;
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
        if self.engine.is_some() && !text.is_empty() {
            if !tail.is_empty() {
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
                // 用一次就收进用户词库（「霜」「频」「阁」→ 霜频阁）；更长的是整句，照常按次数学。
                let mut pinyin = String::new();
                for selection in &self.selections {
                    pinyin.push_str(&selection.pinyin);
                    pinyin.push(' ');
                }
                pinyin.push_str(tail_pinyin);
                self.learn_composed(&text, &pinyin);
            }
        }
        self.finish(text);
    }

    /// 记一次选词。上文和算候选时一样（已上屏的字 + 已选的词）。
    fn learn(&self, word: &str, pinyin: &str) {
        let Some(engine) = self.engine else {
            return;
        };
        let context = self.context.clone() + &self.selected_text();
        engine.set_context(utf8_tail(&context, CONTEXT_BYTES));
        engine.record_selection_with_pinyin(word, Some(pinyin).filter(|p| !p.is_empty()));
    }

    /// 记分几次选出来拼成的词，见 [`Engine::record_composed_word`]。上文是还没选词时的上文。
    fn learn_composed(&self, word: &str, pinyin: &str) {
        let Some(engine) = self.engine else {
            return;
        };
        engine.set_context(utf8_tail(&self.context, CONTEXT_BYTES));
        engine.record_composed_word(word, pinyin);
    }

    /// 上屏 text（空的不上屏，frame 取走），清空编辑区。
    fn finish(&mut self, text: String) {
        self.clear();
        self.pending = (!text.is_empty()).then_some(text);
    }

    /// 选了句首词，压进已选栈（测试里不带引擎时用）。
    pub fn push_selection(&mut self, selection: Selection) {
        self.selections.push(selection);
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
    let top = first.text.clone();
    let emoji: Vec<Candidate> = crate::emoji::recommend(&top, 1)
        .into_iter()
        .map(|s| Candidate {
            // 句尾对上时 commit 是「整句 + emoji」
            emoji: s
                .commit
                .strip_prefix(top.as_str())
                .unwrap_or(&s.commit)
                .to_owned(),
            text: s.display,
            pinyin: String::new(),
            consumed: input_len,
            kind: CandidateKind::Emoji,
            commit: s.commit,
        })
        .collect();
    let at = EMOJI_SLOT.min(all.len());
    all.splice(at..at, emoji);
}

/// 第一页里有数字字就在它后面跟按键 emoji：打 ba 是「吧 八 8️⃣ 把」，和手机输入法一样。
fn add_keycap(all: &mut Vec<Candidate>, input_len: usize, page_size: usize) {
    let window = all.len().min(page_size);
    for i in 0..window {
        let c = &all[i];
        if c.kind == CandidateKind::Emoji || c.consumed < input_len {
            continue;
        }
        let Some(keycap) = crate::emoji::keycap(&c.text) else {
            continue;
        };
        all.insert(
            i + 1,
            Candidate {
                text: keycap.to_owned(),
                pinyin: String::new(),
                consumed: input_len,
                kind: CandidateKind::Emoji,
                commit: keycap.to_owned(),
                emoji: keycap.to_owned(),
            },
        );
        return;
    }
}

/// 只留尾部 max_bytes 字节，起点对齐到字符边界。
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
        // 开头的隔音符不进编辑区
        let mut session = Session::new();
        assert!(!session.key(&Key::char('\'')).handled);
        assert!(!session.composing());
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
        });
        let frame = session.frame(true);
        assert_eq!(frame.preedit, "你好ma");
        assert_eq!(frame.preedit_cursor, "你好ma".len());
    }
}
