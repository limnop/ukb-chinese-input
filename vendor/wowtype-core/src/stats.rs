//! 打字统计：默认关，打开以后只在本机记每天的聚合计数，不保存输入内容。
//!
//! 记什么（stats.json 里只有这些）：每天上屏的字符数、按类别（汉字 / 字母 / 数字 / 标点 / 表情 /
//! 其他）的字符数、活跃毫秒数、每个物理键当天按下的次数、按输入模式和双拼方案分的字符数、
//! 候选位置 1–9 和「第 10 个以后」的次数。
//!
//! 不记什么：上屏的文字、词、拼音、按键顺序、每次按键或上屏的时刻（算活跃时长用的上一次上屏时刻
//! 只在内存里）、哪个应用。密码框（外壳传 [`SENSITIVE`]）里的按键和上屏整个不算。
//!
//! 开关在 wowtype.conf 的 `TypingStats`（默认 False），外壳读到以后调 [`Recorder::set_enabled`]。
//! 关着时 [`Recorder`] 什么都不做：不计数、不起线程、不碰文件。
//!
//! 文件只有输入法进程（这里的写文件线程）写。按键和上屏只改内存里的一份，攒够了交给写文件线程：
//! 它读文件、加上这批、去掉 [`KEEP_DAYS`] 天以前的、原子写回（0600）。按键的线程不读写文件。
//!
//! 清空：设置界面调 [`clear`]（或照它的做法）——旁边的 `stats.clear` 里的整数加一，再删掉
//! `stats.json`。写文件线程每次写之前看这个数：和攒这批时看到的不一样，说明这批里有清空之前打的，
//! 整批丢掉；文件里记的 `cleared` 和它不一样（清空的同时正好写回了旧数据），文件当成空的。
//! 设置界面显示时也一样：`cleared` 和 `stats.clear` 对不上就当没有记录。

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

use serde_json::{json, Map, Value};

/// 两次上屏间隔不超过这个值，这段间隔算作活跃输入时间。
pub const ACTIVE_GAP_MS: i64 = 10_000;
/// 攒够这么久（或 [`FLUSH_COMMITS`] 次上屏）交给写文件线程一次。
const FLUSH_INTERVAL_MS: i64 = 30_000;
const FLUSH_COMMITS: u32 = 20;
/// 只留最近这么多天；更早的写的时候删掉。
pub const KEEP_DAYS: i64 = 400;
/// 文件大小上限。超过的（不是我们写的）挪到 `stats.json.bad`，从空的重新记；
/// 我们自己写的超过时从最早的一天开始删到写得下（留 400 天、键名有限，正常到不了）。
pub const MAX_FILE_BYTES: usize = 16 * 1024 * 1024;
const MAX_COUNT: u64 = 9_000_000_000_000_000;
const DAY_MS: i64 = 24 * 60 * 60 * 1000;
/// 一张计数表（键位、模式、方案）最多这么多项，名字最长这么多字节（只认字母、数字、下划线）。
const MAX_MAP_ENTRIES: usize = 256;
const MAX_NAME_BYTES: usize = 32;
const VERSION: u32 = 2;

/// 外壳传的标志：这是密码框（或别的敏感输入框），这次按键 / 上屏不算。
pub const SENSITIVE: u32 = 1;

/// 认的输入模式名（上屏来源）；别的名字不记。
const SOURCES: [&str; 5] = ["quanpin", "shuangpin", "wubi", "english", "translate"];

/// 一天里的字符分类。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Kinds {
    pub han: u64,
    pub latin: u64,
    pub number: u64,
    pub punct: u64,
    pub emoji: u64,
    pub other: u64,
}

impl Kinds {
    pub fn add(&mut self, kind: Kind, amount: u64) {
        let slot = match kind {
            Kind::Han => &mut self.han,
            Kind::Latin => &mut self.latin,
            Kind::Number => &mut self.number,
            Kind::Punct => &mut self.punct,
            Kind::Emoji => &mut self.emoji,
            Kind::Other => &mut self.other,
        };
        *slot = add_count(*slot, amount);
    }

    pub fn readable(&self) -> u64 {
        self.han.saturating_add(self.latin)
    }

    fn merge(&mut self, other: &Self) {
        self.han = add_count(self.han, other.han);
        self.latin = add_count(self.latin, other.latin);
        self.number = add_count(self.number, other.number);
        self.punct = add_count(self.punct, other.punct);
        self.emoji = add_count(self.emoji, other.emoji);
        self.other = add_count(self.other, other.other);
    }

    fn to_json(&self) -> Value {
        json!({
            "han": self.han,
            "latin": self.latin,
            "number": self.number,
            "punct": self.punct,
            "emoji": self.emoji,
            "other": self.other,
        })
    }

    fn from_json(value: Option<&Value>) -> Self {
        let Some(map) = value.and_then(Value::as_object) else {
            return Self::default();
        };
        Self {
            han: number_of(map.get("han")),
            latin: number_of(map.get("latin")),
            number: number_of(map.get("number")),
            punct: number_of(map.get("punct")),
            emoji: number_of(map.get("emoji")),
            other: number_of(map.get("other")),
        }
    }
}

/// 一个本地日的计数。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Day {
    pub chars: u64,
    pub kinds: Kinds,
    pub active_ms: u64,
    /// 键位 id（W3C `KeyboardEvent.code`）→ 当天按下的次数。
    pub keys: BTreeMap<String, u64>,
    /// 输入模式（[`SOURCES`]）→ 字符数。
    pub sources: BTreeMap<String, u64>,
    /// 双拼方案 → 字符数。
    pub schemes: BTreeMap<String, u64>,
    /// 候选位置 1..9 的上屏次数。
    pub selections: [u64; 9],
    /// 比第 9 个更靠后的候选的上屏次数。
    pub beyond: u64,
}

impl Day {
    fn merge(&mut self, other: &Self) {
        self.chars = add_count(self.chars, other.chars);
        self.kinds.merge(&other.kinds);
        self.active_ms = self.active_ms.saturating_add(other.active_ms).min(DAY_MS as u64);
        merge_map(&mut self.keys, &other.keys);
        merge_map(&mut self.sources, &other.sources);
        merge_map(&mut self.schemes, &other.schemes);
        for (slot, count) in self.selections.iter_mut().zip(other.selections) {
            *slot = add_count(*slot, count);
        }
        self.beyond = add_count(self.beyond, other.beyond);
    }

    fn to_json(&self) -> Value {
        json!({
            "chars": self.chars,
            "kinds": self.kinds.to_json(),
            "activeMs": self.active_ms,
            "keys": map_json(&self.keys),
            "sources": map_json(&self.sources),
            "schemes": map_json(&self.schemes),
            "selections": self.selections.to_vec(),
            "beyond": self.beyond,
        })
    }

    fn from_json(value: &Value) -> Self {
        let Some(map) = value.as_object() else {
            return Self::default();
        };
        let mut out = Self {
            chars: number_of(map.get("chars")),
            kinds: Kinds::from_json(map.get("kinds")),
            active_ms: number_of(map.get("activeMs")).min(DAY_MS as u64),
            keys: map_of_numbers(map.get("keys")),
            sources: map_of_numbers(map.get("sources")),
            schemes: map_of_numbers(map.get("schemes")),
            selections: [0; 9],
            beyond: number_of(map.get("beyond")),
        };
        if let Some(ranks) = map.get("selections").and_then(Value::as_array) {
            for (slot, value) in out.selections.iter_mut().zip(ranks) {
                *slot = number_of(Some(value));
            }
        }
        out
    }
}

/// 按天的计数（内存里攒着的一批，或整份文件）。
pub type Days = BTreeMap<String, Day>;

/// 整份统计文件。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Document {
    /// 写这份文件时 `stats.clear` 里的数；和现在的对不上，说明之后清空过，整份不算。
    pub cleared: u64,
    pub days: Days,
}

/// 读统计文件出的错。
#[derive(Debug)]
pub enum LoadError {
    /// 读不了（权限、不是普通文件……）：不动它，这次不写。
    Io(String),
    /// 内容坏了或太大：挪开，从空的重新记。
    Bad(String),
}

impl Document {
    /// 读统计文件；不存在时是空的。
    pub fn load(path: &Path) -> Result<Self, LoadError> {
        let metadata = match fs::metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(LoadError::Io(format!("读统计文件失败：{error}"))),
        };
        if !metadata.is_file() {
            return Err(LoadError::Io("统计文件不是普通文件".to_string()));
        }
        if metadata.len() > MAX_FILE_BYTES as u64 {
            return Err(LoadError::Bad("统计文件超过 16 MB".to_string()));
        }
        let bytes = fs::read(path).map_err(|error| LoadError::Io(format!("读统计文件失败：{error}")))?;
        Self::from_bytes(&bytes).map_err(LoadError::Bad)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_FILE_BYTES {
            return Err("统计文件超过 16 MB".to_string());
        }
        let value: Value =
            serde_json::from_slice(bytes).map_err(|_| "统计文件不是合法 JSON".to_string())?;
        let Some(map) = value.as_object() else {
            return Err("统计文件不是对象".to_string());
        };
        let mut days = Days::new();
        if let Some(entries) = map.get("days").and_then(Value::as_object) {
            for (day, value) in entries {
                if is_valid_day(day) {
                    days.insert(day.clone(), Day::from_json(value));
                }
            }
        }
        Ok(Self {
            cleared: map.get("cleared").and_then(Value::as_u64).unwrap_or(0),
            days,
        })
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let days: Map<String, Value> = self
            .days
            .iter()
            .map(|(day, value)| (day.clone(), value.to_json()))
            .collect();
        let document = json!({
            "version": VERSION,
            "cleared": self.cleared,
            "days": days,
        });
        serde_json::to_vec_pretty(&document).unwrap_or_default()
    }

    fn merge(&mut self, days: &Days) {
        for (day, value) in days {
            self.days.entry(day.clone()).or_default().merge(value);
        }
    }

    /// 去掉 [`KEEP_DAYS`] 天以前的。
    pub fn prune(&mut self, now_ms: i64) {
        let cutoff = local_day(now_ms.saturating_sub(KEEP_DAYS * DAY_MS));
        if cutoff.is_empty() {
            return;
        }
        self.days.retain(|day, _| day.as_str() >= cutoff.as_str());
    }

    /// 排成要写的字节；超过 [`MAX_FILE_BYTES`] 时从最早的一天删起。
    fn to_bytes_capped(&mut self) -> Vec<u8> {
        loop {
            let bytes = self.to_bytes();
            if bytes.len() <= MAX_FILE_BYTES || self.days.is_empty() {
                return bytes;
            }
            // 按超出的比例一次删掉最早的几天（至少一天），再排一次
            let over = (bytes.len() - MAX_FILE_BYTES) as f64 / bytes.len() as f64;
            let drop = ((self.days.len() as f64 * over).ceil() as usize).max(1);
            match self.days.keys().nth(drop).cloned() {
                Some(first_kept) => self.days = self.days.split_off(&first_kept),
                None => self.days.clear(),
            }
        }
    }
}

/// `stats.json` 旁边的清空计数文件（`stats.clear`）。
pub fn clear_marker_path(path: &Path) -> PathBuf {
    path.with_extension("clear")
}

/// 坏了的统计文件挪到这里（`stats.json.bad`），只留最近的一份。
pub fn bad_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(".bad");
    path.with_file_name(name)
}

/// 读清空计数；没有文件、读不出数时是 0。
pub fn read_clear_marker(path: &Path) -> u64 {
    fs::read_to_string(clear_marker_path(path))
        .ok()
        .and_then(|text| text.trim().parse::<u64>().ok())
        .unwrap_or(0)
}

/// 清空统计（设置界面用，输入法开着关着都行）：清空计数加一，再删掉统计文件。
///
/// 先加计数：输入法这时正好在写的话，写完会发现计数变了、自己删掉；攒着没写的那批下次写时丢掉。
pub fn clear(path: &Path) -> Result<(), String> {
    let next = read_clear_marker(path).wrapping_add(1).max(1);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("建统计目录失败：{error}"))?;
    }
    write_private(&clear_marker_path(path), format!("{next}\n").as_bytes())?;
    for file in [path.to_path_buf(), bad_path(path)] {
        match fs::remove_file(&file) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("删统计文件失败：{error}")),
        }
    }
    Ok(())
}

/// 设置界面读统计用：读文件，清空过（`cleared` 对不上）或读不出来时是空的。
pub fn load_current(path: &Path) -> Document {
    let marker = read_clear_marker(path);
    match Document::load(path) {
        Ok(document) if document.cleared == marker => document,
        _ => Document::default(),
    }
}

// ---------------- 输入法进程里的记录器

/// X11 键码（0..512）按下没松开的位图：长按的自动重复只算一次。
#[derive(Clone, Debug, Default)]
struct HeldKeys([u64; 8]);

impl HeldKeys {
    const LIMIT: u32 = 8 * 64;

    fn slot(code: u32) -> Option<(usize, u64)> {
        (code < Self::LIMIT).then(|| ((code / 64) as usize, 1_u64 << (code % 64)))
    }

    /// 记下按下；原来就按着时返回 false。
    fn press(&mut self, code: u32) -> bool {
        let Some((word, bit)) = Self::slot(code) else {
            return false;
        };
        let fresh = self.0[word] & bit == 0;
        self.0[word] |= bit;
        fresh
    }

    fn release(&mut self, code: u32) {
        if let Some((word, bit)) = Self::slot(code) {
            self.0[word] &= !bit;
        }
    }

    fn clear(&mut self) {
        self.0 = [0; 8];
    }

    #[cfg(test)]
    fn is_empty(&self) -> bool {
        self.0.iter().all(|word| *word == 0)
    }
}

/// 输入法进程里的记录器：按键与上屏只改内存，攒够了交给写文件线程。
///
/// 新建时是关着的：什么都不记、不起线程、不碰文件，外壳读到 `TypingStats=True` 才打开。
pub struct Recorder {
    enabled: bool,
    pending: Days,
    pending_commits: u32,
    last_submit_ms: i64,
    /// 上一次上屏的时刻，只用来算活跃时长，不落盘；关掉统计时清零。
    last_commit_ms: i64,
    held: HeldKeys,
    /// 上一次松开的 X11 键和时刻，用来识破没有 detectable-repeat 时合成的「松开 + 按下」。只在内存里。
    last_release: Option<(u32, i64)>,
    writer: Writer,
}

impl Recorder {
    /// 建一个记录器（关着）。不读文件、不建文件。
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            enabled: false,
            pending: Days::new(),
            pending_commits: 0,
            last_submit_ms: 0,
            last_commit_ms: 0,
            held: HeldKeys::default(),
            last_release: None,
            writer: Writer::new(path.into()),
        }
    }

    pub fn path(&self) -> &Path {
        &self.writer.path
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// 打开或关上统计。关上时丢掉内存里攒着、还没交出去的计数，以后什么都不记。
    pub fn set_enabled(&mut self, enabled: bool, now_ms: i64) {
        if enabled == self.enabled {
            return;
        }
        self.reset_memory();
        self.enabled = enabled;
        if enabled {
            // 写文件线程记下现在的清空计数：之后清空过，这之后攒的第一批也要丢
            self.writer.send(Message::Start);
            self.last_submit_ms = now_ms;
        }
    }

    fn reset_memory(&mut self) {
        self.pending.clear();
        self.pending_commits = 0;
        self.last_commit_ms = 0;
        self.held.clear();
        self.last_release = None;
    }

    /// 一次物理按键。`down` 为假时只松开（用来识别长按和合成重复）。
    ///
    /// `platform`：0 = X11/fcitx5（`code` 是 XKB 键码，内部减 8 成 evdev），1 = macOS（`NSEvent.keyCode`）。
    /// `flags` 带 [`SENSITIVE`]（密码框）时不算。
    pub fn key(&mut self, platform: i32, code: u32, down: bool, flags: u32, now_ms: i64) {
        if !self.enabled {
            return;
        }
        match platform {
            0 => {
                if !down {
                    self.held.release(code);
                    self.last_release = Some((code, now_ms));
                    return;
                }
                // 长按的自动重复；键码超出范围（不是物理键）也在这里挡掉
                if !self.held.press(code) {
                    return;
                }
                if let Some((released, at)) = self.last_release {
                    if released == code && now_ms >= at && now_ms - at <= 2 {
                        return; // X11 合成的一次重复
                    }
                }
            }
            // macOS 的 InputMethodKit 只送按下（没有松开），自动重复由外壳的 `isARepeat` 挡掉
            1 if down => {}
            _ => return,
        }
        if flags & SENSITIVE != 0 {
            return;
        }
        let Some(id) = key_id(platform, code) else {
            return;
        };
        let day = local_day(now_ms);
        if day.is_empty() {
            return;
        }
        let slot = self.pending.entry(day).or_default().keys.entry(id.to_string()).or_default();
        *slot = add_count(*slot, 1);
        if now_ms - self.last_submit_ms >= FLUSH_INTERVAL_MS {
            self.flush(now_ms);
        }
    }

    /// 一次上屏。`text` 只在这里数一数各类字符有几个，不留下。
    ///
    /// `source` 是输入模式（[`SOURCES`] 之一，别的不记），`scheme` 是双拼方案名（只认字母数字）。
    /// `cand_index` 是候选在列表里的位置（1 起）；不是选候选上屏时为 0。
    pub fn commit(
        &mut self,
        text: &str,
        source: &str,
        scheme: &str,
        cand_index: u32,
        flags: u32,
        now_ms: i64,
    ) {
        if !self.enabled || flags & SENSITIVE != 0 {
            return;
        }
        let (chars, kinds) = classify(text);
        if chars == 0 {
            return;
        }
        let day = local_day(now_ms);
        if day.is_empty() {
            return;
        }
        let entry = self.pending.entry(day).or_default();
        entry.chars = add_count(entry.chars, chars);
        entry.kinds.merge(&kinds);
        if SOURCES.contains(&source) {
            let slot = entry.sources.entry(source.to_string()).or_default();
            *slot = add_count(*slot, chars);
        }
        let scheme = scheme.to_ascii_lowercase();
        if is_plain_name(&scheme) {
            let slot = entry.schemes.entry(scheme).or_default();
            *slot = add_count(*slot, chars);
        }
        match cand_index {
            1..=9 => {
                let slot = &mut entry.selections[cand_index as usize - 1];
                *slot = add_count(*slot, 1);
            }
            10.. => entry.beyond = add_count(entry.beyond, 1),
            _ => {}
        }
        let gap = now_ms - self.last_commit_ms;
        if self.last_commit_ms > 0 && gap > 0 && gap <= ACTIVE_GAP_MS {
            entry.active_ms = entry.active_ms.saturating_add(gap as u64).min(DAY_MS as u64);
        }
        self.last_commit_ms = now_ms;
        self.pending_commits += 1;
        if self.pending_commits >= FLUSH_COMMITS || now_ms - self.last_submit_ms >= FLUSH_INTERVAL_MS {
            self.flush(now_ms);
        }
    }

    /// 把攒着的计数交给写文件线程（不等它写）。关着、没有攒着的时什么都不做。
    pub fn flush(&mut self, now_ms: i64) {
        self.last_submit_ms = now_ms;
        self.pending_commits = 0;
        if !self.enabled || self.pending.is_empty() {
            return;
        }
        let batch = std::mem::take(&mut self.pending);
        self.writer.send(Message::Write(batch, now_ms));
    }

    /// 等交出去的都写完。返回这期间写失败的原因；`timeout` 是 None 时一直等。
    pub fn wait(&mut self, timeout: Option<Duration>) -> Result<(), String> {
        self.writer.wait(timeout)
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        // 关着时 pending 是空的，flush 什么都不做；开着时把最后这批交出去，Writer 被 drop 时等它写完
        self.flush(now_ms());
    }
}

// ---------------- 写文件线程

enum Message {
    /// 统计刚打开：记下现在的清空计数。
    Start,
    /// 一批计数和交出来的时刻（删旧数据按它算今天）。
    Write(Days, i64),
    /// 等前面的都写完，回报这期间的错误。
    Sync(mpsc::Sender<Option<String>>),
}

struct Writer {
    path: PathBuf,
    sender: Option<mpsc::Sender<Message>>,
    handle: Option<JoinHandle<()>>,
}

impl Writer {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            sender: None,
            handle: None,
        }
    }

    /// 交给写文件线程；第一次交时才起线程。起不了线程时这批丢掉（统计不要紧，不在按键线程上写）。
    fn send(&mut self, message: Message) {
        if self.sender.is_none() {
            let (sender, receiver) = mpsc::channel();
            let path = self.path.clone();
            match std::thread::Builder::new()
                .name("wowtype-stats".into())
                .spawn(move || run_writer(path, receiver))
            {
                Ok(handle) => {
                    self.sender = Some(sender);
                    self.handle = Some(handle);
                }
                Err(error) => {
                    eprintln!("wowtype: 起不了写打字统计的线程：{error}");
                    return;
                }
            }
        }
        if let Some(sender) = &self.sender {
            let _ = sender.send(message);
        }
    }

    fn wait(&mut self, timeout: Option<Duration>) -> Result<(), String> {
        let Some(sender) = &self.sender else {
            return Ok(());
        };
        let (reply, answer) = mpsc::channel();
        if sender.send(Message::Sync(reply)).is_err() {
            return Err("写打字统计的线程已经退出".to_string());
        }
        let result = match timeout {
            None => answer.recv().map_err(|_| "写打字统计的线程已经退出".to_string()),
            Some(timeout) => answer
                .recv_timeout(timeout)
                .map_err(|_| "等写打字统计超时".to_string()),
        };
        match result? {
            None => Ok(()),
            Some(error) => Err(error),
        }
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        // 关掉发送端，线程把排着的写完就退
        self.sender = None;
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn run_writer(path: PathBuf, receiver: mpsc::Receiver<Message>) {
    let mut state = WriterState {
        path,
        known_clear: None,
        errors: Vec::new(),
    };
    while let Ok(message) = receiver.recv() {
        match message {
            Message::Start => state.known_clear = Some(read_clear_marker(&state.path)),
            Message::Write(batch, now_ms) => {
                if let Err(error) = state.write(&batch, now_ms) {
                    eprintln!("wowtype: 打字统计写入失败：{error}");
                    state.errors.push(error);
                }
            }
            Message::Sync(reply) => {
                let errors = std::mem::take(&mut state.errors);
                let _ = reply.send((!errors.is_empty()).then(|| errors.join("；")));
            }
        }
    }
}

struct WriterState {
    path: PathBuf,
    /// 开始攒这批时的清空计数。
    known_clear: Option<u64>,
    errors: Vec<String>,
}

impl WriterState {
    fn write(&mut self, batch: &Days, now_ms: i64) -> Result<(), String> {
        let marker = read_clear_marker(&self.path);
        let known = *self.known_clear.get_or_insert(marker);
        if known != marker {
            // 攒这批的时候清空过：这批里有清空之前打的，整批丢掉；清空时正好写回去的旧数据也删掉
            self.known_clear = Some(marker);
            if matches!(Document::load(&self.path), Ok(document) if document.cleared != marker) {
                let _ = fs::remove_file(&self.path);
            }
            return Ok(());
        }
        if batch.is_empty() {
            return Ok(());
        }
        let mut document = match Document::load(&self.path) {
            Ok(document) => document,
            Err(LoadError::Io(error)) => return Err(error),
            Err(LoadError::Bad(reason)) => {
                // 坏了、太大（不是我们写的）：挪开留一份，从空的重新记
                let bad = bad_path(&self.path);
                eprintln!("wowtype: {reason}，挪到 {} 重新记", bad.display());
                fs::rename(&self.path, &bad).map_err(|error| format!("挪开坏的统计文件失败：{error}"))?;
                Document::default()
            }
        };
        if document.cleared != marker {
            document = Document::default();
        }
        document.cleared = marker;
        document.merge(batch);
        document.prune(now_ms);
        let bytes = document.to_bytes_capped();
        write_private(&self.path, &bytes)?;
        if read_clear_marker(&self.path) != marker {
            // 写的时候正好清空了：刚写回去的有清空之前的，删掉（下一批也会因为计数变了丢掉）
            let _ = fs::remove_file(&self.path);
        }
        Ok(())
    }
}

// ---------------- 字符分类

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Han,
    Latin,
    Number,
    Punct,
    Emoji,
    Other,
}

/// 按字符分类；返回（计数字符数, 各类别计数）。空白不计，组合表情算一个字符。
pub fn classify(text: &str) -> (u64, Kinds) {
    let mut kinds = Kinds::default();
    let mut count = 0_u64;
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch.is_whitespace() {
            continue;
        }
        if is_emoji(ch) {
            // 吞掉变体选择符、肤色修饰、键帽组合、零宽连接符和它后面的表情；
            // 没有连接符的两个表情仍然算两个。
            loop {
                let Some(&next) = chars.peek() else { break };
                let code = next as u32;
                if code == 0xfe0f || code == 0x20e3 || (0x1f3fb..=0x1f3ff).contains(&code) {
                    chars.next();
                    continue;
                }
                if code == 0x200d {
                    chars.next();
                    if chars.peek().is_some_and(|c| is_emoji(*c)) {
                        chars.next();
                    }
                    continue;
                }
                break;
            }
            kinds.add(Kind::Emoji, 1);
            count += 1;
            continue;
        }
        kinds.add(kind_of(ch), 1);
        count += 1;
    }
    (count, kinds)
}

fn kind_of(ch: char) -> Kind {
    let code = ch as u32;
    if is_han(code) {
        return Kind::Han;
    }
    if is_latin(code) {
        return Kind::Latin;
    }
    if is_number(code) {
        return Kind::Number;
    }
    if is_punct(code) {
        return Kind::Punct;
    }
    Kind::Other
}

fn is_han(code: u32) -> bool {
    (0x3400..=0x4dbf).contains(&code)
        || (0x4e00..=0x9fff).contains(&code)
        || (0xf900..=0xfaff).contains(&code)
        || (0x20000..=0x323af).contains(&code)
}

fn is_latin(code: u32) -> bool {
    (0x41..=0x5a).contains(&code)
        || (0x61..=0x7a).contains(&code)
        || (0xc0..=0x24f).contains(&code)
        || (0x1e00..=0x1eff).contains(&code)
        || (0xff21..=0xff3a).contains(&code)
        || (0xff41..=0xff5a).contains(&code)
}

fn is_number(code: u32) -> bool {
    (0x30..=0x39).contains(&code) || (0xff10..=0xff19).contains(&code)
}

fn is_punct(code: u32) -> bool {
    (0x21..=0x2f).contains(&code)
        || (0x3a..=0x40).contains(&code)
        || (0x5b..=0x60).contains(&code)
        || (0x7b..=0x7e).contains(&code)
        || (0x3000..=0x303f).contains(&code)
        || (0xfe10..=0xfe1f).contains(&code)
        || (0xfe30..=0xfe4f).contains(&code)
        || (0xff01..=0xff20).contains(&code)
        || (0xff3b..=0xff40).contains(&code)
        || (0xff5b..=0xff65).contains(&code)
}

fn is_emoji(ch: char) -> bool {
    let code = ch as u32;
    (0x1f000..=0x1faff).contains(&code)
        || (0x2600..=0x27bf).contains(&code)
        || (0x2b00..=0x2bff).contains(&code)
        || (0x1f1e6..=0x1f1ff).contains(&code)
}

// ---------------- 键位

/// 物理键码 → `KeyboardEvent.code` 风格的 id。
///
/// `platform`：0 = X11（XKB 键码，evdev + 8；fcitx5 给的就是这个），1 = macOS（`NSEvent.keyCode`）。
/// 热力图画不出的键（F13 以后、媒体键、打印屏幕……）返回 `None`，不计入。
pub fn key_id(platform: i32, code: u32) -> Option<&'static str> {
    match platform {
        0 => x11_key_id(code.checked_sub(8)?),
        1 => macos_key_id(u16::try_from(code).ok()?),
        _ => None,
    }
}

fn x11_key_id(evdev: u32) -> Option<&'static str> {
    Some(match evdev {
        1 => "Escape",
        2 => "Digit1",
        3 => "Digit2",
        4 => "Digit3",
        5 => "Digit4",
        6 => "Digit5",
        7 => "Digit6",
        8 => "Digit7",
        9 => "Digit8",
        10 => "Digit9",
        11 => "Digit0",
        12 => "Minus",
        13 => "Equal",
        14 => "Backspace",
        15 => "Tab",
        16 => "KeyQ",
        17 => "KeyW",
        18 => "KeyE",
        19 => "KeyR",
        20 => "KeyT",
        21 => "KeyY",
        22 => "KeyU",
        23 => "KeyI",
        24 => "KeyO",
        25 => "KeyP",
        26 => "BracketLeft",
        27 => "BracketRight",
        28 => "Enter",
        29 => "ControlLeft",
        30 => "KeyA",
        31 => "KeyS",
        32 => "KeyD",
        33 => "KeyF",
        34 => "KeyG",
        35 => "KeyH",
        36 => "KeyJ",
        37 => "KeyK",
        38 => "KeyL",
        39 => "Semicolon",
        40 => "Quote",
        41 => "Backquote",
        42 => "ShiftLeft",
        43 => "Backslash",
        44 => "KeyZ",
        45 => "KeyX",
        46 => "KeyC",
        47 => "KeyV",
        48 => "KeyB",
        49 => "KeyN",
        50 => "KeyM",
        51 => "Comma",
        52 => "Period",
        53 => "Slash",
        54 => "ShiftRight",
        55 => "NumpadMultiply",
        56 => "AltLeft",
        57 => "Space",
        58 => "CapsLock",
        59 => "F1",
        60 => "F2",
        61 => "F3",
        62 => "F4",
        63 => "F5",
        64 => "F6",
        65 => "F7",
        66 => "F8",
        67 => "F9",
        68 => "F10",
        69 => "NumLock",
        70 => "ScrollLock",
        71 => "Numpad7",
        72 => "Numpad8",
        73 => "Numpad9",
        74 => "NumpadSubtract",
        75 => "Numpad4",
        76 => "Numpad5",
        77 => "Numpad6",
        78 => "NumpadAdd",
        79 => "Numpad1",
        80 => "Numpad2",
        81 => "Numpad3",
        82 => "Numpad0",
        83 => "NumpadDecimal",
        86 => "IntlBackslash",
        87 => "F11",
        88 => "F12",
        89 => "IntlRo",
        92 => "Convert",
        93 => "KanaMode",
        94 => "NonConvert",
        96 => "NumpadEnter",
        97 => "ControlRight",
        98 => "NumpadDivide",
        100 => "AltRight",
        102 => "Home",
        103 => "ArrowUp",
        104 => "PageUp",
        105 => "ArrowLeft",
        106 => "ArrowRight",
        107 => "End",
        108 => "ArrowDown",
        109 => "PageDown",
        110 => "Insert",
        111 => "Delete",
        122 => "Lang1",
        123 => "Lang2",
        124 => "IntlYen",
        125 => "MetaLeft",
        126 => "MetaRight",
        127 => "ContextMenu",
        464 => "Fn",
        _ => return None,
    })
}

fn macos_key_id(code: u16) -> Option<&'static str> {
    Some(match code {
        0x00 => "KeyA",
        0x01 => "KeyS",
        0x02 => "KeyD",
        0x03 => "KeyF",
        0x04 => "KeyH",
        0x05 => "KeyG",
        0x06 => "KeyZ",
        0x07 => "KeyX",
        0x08 => "KeyC",
        0x09 => "KeyV",
        0x0a => "IntlBackslash",
        0x0b => "KeyB",
        0x0c => "KeyQ",
        0x0d => "KeyW",
        0x0e => "KeyE",
        0x0f => "KeyR",
        0x10 => "KeyY",
        0x11 => "KeyT",
        0x12 => "Digit1",
        0x13 => "Digit2",
        0x14 => "Digit3",
        0x15 => "Digit4",
        0x16 => "Digit6",
        0x17 => "Digit5",
        0x18 => "Equal",
        0x19 => "Digit9",
        0x1a => "Digit7",
        0x1b => "Minus",
        0x1c => "Digit8",
        0x1d => "Digit0",
        0x1e => "BracketRight",
        0x1f => "KeyO",
        0x20 => "KeyU",
        0x21 => "BracketLeft",
        0x22 => "KeyI",
        0x23 => "KeyP",
        0x24 => "Enter",
        0x25 => "KeyL",
        0x26 => "KeyJ",
        0x27 => "Quote",
        0x28 => "KeyK",
        0x29 => "Semicolon",
        0x2a => "Backslash",
        0x2b => "Comma",
        0x2c => "Slash",
        0x2d => "KeyN",
        0x2e => "KeyM",
        0x2f => "Period",
        0x30 => "Tab",
        0x31 => "Space",
        0x32 => "Backquote",
        0x33 => "Backspace",
        0x35 => "Escape",
        0x36 => "MetaRight",
        0x37 => "MetaLeft",
        0x38 => "ShiftLeft",
        0x39 => "CapsLock",
        0x3a => "AltLeft",
        0x3b => "ControlLeft",
        0x3c => "ShiftRight",
        0x3d => "AltRight",
        0x3e => "ControlRight",
        0x3f => "Fn",
        0x41 => "NumpadDecimal",
        0x43 => "NumpadMultiply",
        0x45 => "NumpadAdd",
        0x47 => "NumLock",
        0x4b => "NumpadDivide",
        0x4c => "NumpadEnter",
        0x4e => "NumpadSubtract",
        0x52 => "Numpad0",
        0x53 => "Numpad1",
        0x54 => "Numpad2",
        0x55 => "Numpad3",
        0x56 => "Numpad4",
        0x57 => "Numpad5",
        0x58 => "Numpad6",
        0x59 => "Numpad7",
        0x5b => "Numpad8",
        0x5c => "Numpad9",
        0x5d => "IntlYen",
        0x5e => "IntlRo",
        0x60 => "F5",
        0x61 => "F6",
        0x62 => "F7",
        0x63 => "F3",
        0x64 => "F8",
        0x65 => "F9",
        0x66 => "Lang2",
        0x67 => "F11",
        0x68 => "Lang1",
        0x6d => "F10",
        0x6e => "ContextMenu",
        0x6f => "F12",
        0x72 => "Insert",
        0x73 => "Home",
        0x74 => "PageUp",
        0x75 => "Delete",
        0x76 => "F4",
        0x77 => "End",
        0x78 => "F2",
        0x79 => "PageDown",
        0x7a => "F1",
        0x7b => "ArrowLeft",
        0x7c => "ArrowRight",
        0x7d => "ArrowDown",
        0x7e => "ArrowUp",
        _ => return None,
    })
}

// ---------------- 工具

/// Unix 毫秒 → 本地日期 `YYYY-MM-DD`。
pub fn local_day(now_ms: i64) -> String {
    let seconds = now_ms.div_euclid(1000);
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let time = seconds as libc::time_t;
    if unsafe { libc::localtime_r(&time, &mut tm) }.is_null() {
        return String::new();
    }
    format!("{:04}-{:02}-{:02}", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday)
}

pub fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

fn is_valid_day(day: &str) -> bool {
    let bytes = day.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[..4].iter().all(u8::is_ascii_digit)
        && bytes[5..7].iter().all(u8::is_ascii_digit)
        && bytes[8..].iter().all(u8::is_ascii_digit)
}

/// 计数表里的名字（键位 id、模式、方案）：只认字母、数字、下划线，不长。
fn is_plain_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME_BYTES
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

fn add_count(count: u64, amount: u64) -> u64 {
    count.saturating_add(amount).min(MAX_COUNT)
}

fn merge_map(into: &mut BTreeMap<String, u64>, from: &BTreeMap<String, u64>) {
    for (key, count) in from {
        if into.len() >= MAX_MAP_ENTRIES && !into.contains_key(key) {
            continue;
        }
        let slot = into.entry(key.clone()).or_default();
        *slot = add_count(*slot, *count);
    }
}

fn number_of(value: Option<&Value>) -> u64 {
    value.and_then(Value::as_u64).unwrap_or(0).min(MAX_COUNT)
}

fn map_of_numbers(value: Option<&Value>) -> BTreeMap<String, u64> {
    let mut out = BTreeMap::new();
    let Some(map) = value.and_then(Value::as_object) else {
        return out;
    };
    for (key, value) in map {
        if out.len() >= MAX_MAP_ENTRIES {
            break;
        }
        let count = number_of(Some(value));
        if count > 0 && is_plain_name(key) {
            out.insert(key.clone(), count);
        }
    }
    out
}

fn map_json(map: &BTreeMap<String, u64>) -> Value {
    Value::Object(map.iter().map(|(key, value)| (key.clone(), json!(value))).collect())
}

/// 原子写，文件只给自己读写（0600）：同目录临时文件写完、刷到盘上再 rename。
fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let Some(parent) = path.parent() else {
        return Err("统计文件没有目录".to_string());
    };
    fs::create_dir_all(parent).map_err(|error| format!("建统计目录失败：{error}"))?;
    let name = path.file_name().and_then(|name| name.to_str()).unwrap_or("stats.json");
    let temporary = parent.join(format!(".{name}.{}.tmp", std::process::id()));
    let result = (|| -> std::io::Result<()> {
        let mut file = {
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                fs::OpenOptions::new()
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .mode(0o600)
                    .open(&temporary)?
            }
            #[cfg(not(unix))]
            {
                fs::File::create(&temporary)?
            }
        };
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if let Err(error) = result {
        let _ = fs::remove_file(&temporary);
        return Err(format!("写统计文件失败：{error}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
