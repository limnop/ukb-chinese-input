//! 语音输入的 C 接口：按住（或点一下）热键说话，松开后把识别结果交给插件上屏。
//!
//! 识别用微信输入法的离线模型（`wowtype_voice::wtlocal`）。模型不随软件分发，
//! 用户在设置界面（WowType 设置 → 语音）里下载；这里只读，不下载。
//! 没有模型时按热键只给一句提示（VOICE_ERROR + MODEL_MISSING），什么也不做。
//!
//! 线程：
//! - 插件线程只调 `wowtype_voice_*`，这些函数都不阻塞（不开设备、不解码）；
//! - 每次说话起一个工作线程：开麦克风、分帧、VAD、喂识别、出中间结果，结束时整段定稿；
//! - 模型的加载在单独的线程里，只做一次。说话时模型还没读完，音频先攒着，好了再补喂。
//!
//! 插件定时调 `wowtype_voice_poll`（几十毫秒一次）取状态与文本。
//!
//! 测试：设了环境变量 `WOWTYPE_VOICE_WAV` 时不开麦克风，改读这个 WAV 文件当作说的话。

#![allow(clippy::missing_safety_doc)]

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use wowtype_voice::asr::AsrChannel;
use wowtype_voice::audio::{downmix, to_i16, Framer, Resampler, TARGET_RATE};
use wowtype_voice::hotkey::{Action, Mode, TriggerConfig, VoiceTrigger};
use wowtype_voice::vad::Vad;
use wowtype_voice::wtlocal::{self, download, WtLocalChannel};

/// 没在录音。
pub const VOICE_IDLE: c_int = 0;
/// 正在听；文本是中间结果（可能为空）。
pub const VOICE_LISTENING: c_int = 1;
/// 录完了，正在出最终结果。
pub const VOICE_RECOGNIZING: c_int = 2;
/// 最终结果；文本即要上屏的字。只返回一次，之后回到 IDLE。
pub const VOICE_RESULT: c_int = 3;
/// 出错或没听清；文本是给用户看的原因。只返回一次。
pub const VOICE_ERROR: c_int = 4;

/// 模型状态。
pub const MODEL_NONE: c_int = 0;
/// 不再用：模型改在设置界面里下载，这里不会返回它（留着编号免得插件和核心对不上）。
#[allow(dead_code)]
pub const MODEL_DOWNLOADING: c_int = 1;
pub const MODEL_LOADING: c_int = 2;
pub const MODEL_READY: c_int = 3;
pub const MODEL_FAILED: c_int = 4;
/// 按了热键，但模型还没下载。
pub const MODEL_MISSING: c_int = 5;

/// 没有模型时按热键显示的提示。
const MISSING_HINT: &str = "没有语音模型，请在「WowType 设置 → 语音」里下载";

/// 单次最长录音。和 SenseVoice 那套一样，误触不会一直录下去。
const MAX_RECORDING: Duration = Duration::from_secs(120);
/// 等模型加载的上限（已下载好、只是还在读进内存）。
const LOAD_WAIT: Duration = Duration::from_secs(60);

static EMPTY: [u8; 1] = [0];

enum ModelState {
    None,
    Missing,
    Loading,
    Ready,
    Failed(String),
}

struct ModelSlot {
    state: Mutex<ModelState>,
    changed: Condvar,
}

impl ModelSlot {
    fn code(&self) -> c_int {
        match &*self.state.lock().unwrap() {
            ModelState::None => MODEL_NONE,
            ModelState::Missing => MODEL_MISSING,
            ModelState::Loading => MODEL_LOADING,
            ModelState::Ready => MODEL_READY,
            ModelState::Failed(_) => MODEL_FAILED,
        }
    }

    /// 还没载入（或上次失败）就起线程载入。模型不在磁盘上返回 false，不下载。
    fn prepare(self: &Arc<Self>, dir: &Path) -> bool {
        {
            let mut state = self.state.lock().unwrap();
            if !matches!(*state, ModelState::None | ModelState::Missing | ModelState::Failed(_)) {
                return true;
            }
            // 每次都看一眼磁盘：设置界面可能刚下载好，也可能刚删掉
            if !download::is_ready(dir) {
                *state = ModelState::Missing;
                return false;
            }
            *state = ModelState::Loading;
        }
        let slot = self.clone();
        let dir = dir.to_path_buf();
        std::thread::Builder::new()
            .name("wowtype-voice-model".into())
            .spawn(move || {
                let result = wtlocal::load(&dir).map(|_| ());
                *slot.state.lock().unwrap() = match result {
                    Ok(()) => ModelState::Ready,
                    Err(e) => {
                        eprintln!("wowtype: 语音模型不可用：{e:#}");
                        ModelState::Failed(format!("{e}"))
                    }
                };
                slot.changed.notify_all();
            })
            .expect("spawn model thread");
        true
    }

    /// 等模型读进内存。
    fn wait_ready(&self) -> Result<(), String> {
        let deadline = Instant::now() + LOAD_WAIT;
        let mut state = self.state.lock().unwrap();
        loop {
            match &*state {
                ModelState::Ready => return Ok(()),
                ModelState::Failed(e) => return Err(format!("语音模型不可用：{e}")),
                ModelState::None => return Err("语音模型没有准备".into()),
                ModelState::Missing => return Err(MISSING_HINT.into()),
                ModelState::Loading => {
                    let left = deadline.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        return Err("语音模型加载超时".into());
                    }
                    state = self.changed.wait_timeout(state, left).unwrap().0;
                }
            }
        }
    }

    fn is_ready(&self) -> bool {
        matches!(*self.state.lock().unwrap(), ModelState::Ready)
    }
}

/// 工作线程与插件线程共享的状态。
struct Shared {
    /// 当前这次说话的编号；旧会话的结果对不上号就丢掉。
    session: u64,
    phase: c_int,
    text: String,
    /// 最近一次检测到说话的时间（点按模式靠它判断说完了）。
    voice_at: Option<Instant>,
    /// 最近一块音频的音量，0–100（给插件画波形）。
    level: u8,
}

/// 一块 PCM 的音量：-60 dB（安静房间）到 -10 dB（大声说话）映射到 0–100。
fn level_of(pcm: &[i16]) -> u8 {
    if pcm.is_empty() {
        return 0;
    }
    let energy = pcm
        .iter()
        .map(|&v| (v as f64 / 32768.0).powi(2))
        .sum::<f64>()
        / pcm.len() as f64;
    let db = 10.0 * energy.max(1e-10).log10();
    (((db + 60.0) / 50.0).clamp(0.0, 1.0) * 100.0).round() as u8
}

enum Msg {
    Pcm(Vec<i16>),
    Stop,
    Cancel,
}

/// 语音句柄。只在插件线程用。
pub struct WowtypeVoice {
    dir: PathBuf,
    wav: Option<PathBuf>,
    model: Arc<ModelSlot>,
    shared: Arc<Mutex<Shared>>,
    trigger: VoiceTrigger,
    session: u64,
    tx: Option<Sender<Msg>>,
    text: CString,
}

impl WowtypeVoice {
    fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            wav: std::env::var_os("WOWTYPE_VOICE_WAV").map(PathBuf::from),
            model: Arc::new(ModelSlot {
                state: Mutex::new(ModelState::None),
                changed: Condvar::new(),
            }),
            shared: Arc::new(Mutex::new(Shared {
                session: 0,
                phase: VOICE_IDLE,
                text: String::new(),
                voice_at: None,
                level: 0,
            })),
            trigger: VoiceTrigger::new(TriggerConfig::default()),
            session: 0,
            tx: None,
            text: CString::default(),
        }
    }

    fn apply(&mut self, actions: Vec<Action>) {
        for action in actions {
            match action {
                Action::Start => self.start(),
                Action::Stop => self.stop(),
                Action::Continue => {}
            }
        }
    }

    fn start(&mut self) {
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(Msg::Cancel);
        }
        if !self.model.prepare(&self.dir) {
            // 没有模型：只提示去设置里下载，不开麦克风、不下载
            self.session += 1;
            {
                let mut shared = self.shared.lock().unwrap();
                shared.session = self.session;
                shared.voice_at = None;
                shared.level = 0;
            }
            self.fail(MISSING_HINT);
            return;
        }
        self.session += 1;
        {
            let mut shared = self.shared.lock().unwrap();
            shared.session = self.session;
            shared.phase = VOICE_LISTENING;
            shared.text.clear();
            shared.voice_at = Some(Instant::now());
            shared.level = 0;
        }
        let (tx, rx) = mpsc::channel();
        let worker = Worker {
            session: self.session,
            dir: self.dir.clone(),
            wav: self.wav.clone(),
            model: self.model.clone(),
            shared: self.shared.clone(),
            tx: tx.clone(),
        };
        let spawned = std::thread::Builder::new()
            .name("wowtype-voice".into())
            .spawn(move || worker.run(rx));
        if spawned.is_err() {
            self.fail("开不了录音线程");
            return;
        }
        self.tx = Some(tx);
    }

    fn stop(&mut self) {
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(Msg::Stop);
        }
    }

    fn cancel(&mut self) {
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(Msg::Cancel);
        }
        self.trigger = VoiceTrigger::new(TriggerConfig::default());
        self.session += 1;
        let mut shared = self.shared.lock().unwrap();
        shared.session = self.session;
        shared.phase = VOICE_IDLE;
        shared.text.clear();
    }

    fn fail(&mut self, message: &str) {
        let mut shared = self.shared.lock().unwrap();
        shared.phase = VOICE_ERROR;
        shared.text = message.to_string();
    }

    fn poll(&mut self) -> c_int {
        let now = Instant::now();
        let (phase, text, voice_at) = {
            let shared = self.shared.lock().unwrap();
            (shared.phase, shared.text.clone(), shared.voice_at)
        };
        if let Some(at) = voice_at {
            self.trigger.note_voice(at);
        }
        if let Some(action) = self.trigger.tick(now) {
            self.apply(vec![action]);
        }
        // 工作线程自己停了（到了时长上限、开不了麦克风）：热键状态机跟着复位
        if phase != VOICE_LISTENING && self.trigger.is_recording() {
            self.trigger = VoiceTrigger::new(TriggerConfig::default());
            self.tx = None;
        }
        if phase == VOICE_RESULT || phase == VOICE_ERROR {
            let mut shared = self.shared.lock().unwrap();
            if shared.phase == phase {
                shared.phase = VOICE_IDLE;
            }
        }
        self.text = CString::new(text.replace('\0', "")).unwrap_or_default();
        phase
    }
}

struct Worker {
    session: u64,
    dir: PathBuf,
    wav: Option<PathBuf>,
    model: Arc<ModelSlot>,
    shared: Arc<Mutex<Shared>>,
    tx: Sender<Msg>,
}

/// 麦克风（或测试用的 WAV）。丢弃即停。
enum Source {
    #[allow(dead_code)]
    Mic(wowtype_voice::capture::Capture),
    Wav,
}

impl Worker {
    /// 改共享状态；这次说话已经被新的一次顶掉（或取消）就什么也不做。
    fn update(&self, f: impl FnOnce(&mut Shared)) {
        let mut shared = self.shared.lock().unwrap();
        if shared.session == self.session {
            f(&mut shared);
        }
    }

    fn open(&self) -> anyhow::Result<Source> {
        if let Some(path) = &self.wav {
            let (rate, channels, pcm) = wowtype_voice::read_wav(path)?;
            let mono: Vec<f32> = downmix(
                &pcm.iter().map(|&v| v as f32 / 32768.0).collect::<Vec<_>>(),
                channels,
            );
            let pcm = to_i16(&Resampler::new(rate, TARGET_RATE).process(&mono));
            for chunk in pcm.chunks(1600) {
                let _ = self.tx.send(Msg::Pcm(chunk.to_vec()));
            }
            return Ok(Source::Wav);
        }
        let tx = self.tx.clone();
        let capture = wowtype_voice::capture::start(None, move |pcm| {
            let _ = tx.send(Msg::Pcm(pcm));
        })?;
        Ok(Source::Mic(capture))
    }

    fn run(self, rx: Receiver<Msg>) {
        let source = match self.open() {
            Ok(source) => source,
            Err(e) => {
                eprintln!("wowtype: 开麦克风失败：{e:#}");
                self.update(|s| {
                    s.phase = VOICE_ERROR;
                    s.text = "开不了麦克风".into();
                });
                return;
            }
        };
        let started = Instant::now();
        let mut framer = Framer::default();
        let mut vad = Vad::default();
        let mut pending: Vec<i16> = Vec::new();
        let mut asr: Option<WtLocalChannel> = None;
        let mut shown = String::new();

        let stopped = loop {
            if started.elapsed() >= MAX_RECORDING {
                break true;
            }
            let msg = rx.recv_timeout(Duration::from_millis(50));
            if asr.is_none() && self.model.is_ready() {
                match WtLocalChannel::new(&self.dir) {
                    Ok(mut ch) => {
                        ch.feed(&pending);
                        pending = Vec::new();
                        asr = Some(ch);
                    }
                    Err(e) => {
                        self.update(|s| {
                            s.phase = VOICE_ERROR;
                            s.text = format!("语音模型不可用：{e}");
                        });
                        return;
                    }
                }
            }
            let mut batch: Vec<Vec<i16>> = Vec::new();
            let mut end: Option<bool> = None;
            match msg {
                Ok(Msg::Pcm(pcm)) => batch.push(pcm),
                Ok(Msg::Stop) => end = Some(true),
                Ok(Msg::Cancel) | Err(RecvTimeoutError::Disconnected) => end = Some(false),
                Err(RecvTimeoutError::Timeout) => {}
            }
            // 算中间结果的时候麦克风还在送音频：先把积压的都收下，再用最新的音频算一次，
            // 中间结果就不会越来越落后
            while end.is_none() {
                match rx.try_recv() {
                    Ok(Msg::Pcm(pcm)) => batch.push(pcm),
                    Ok(Msg::Stop) => end = Some(true),
                    Ok(Msg::Cancel) | Err(TryRecvError::Disconnected) => end = Some(false),
                    Err(TryRecvError::Empty) => break,
                }
            }
            for pcm in &batch {
                let level = level_of(pcm);
                self.update(|s| s.level = level);
                let mut voiced = false;
                for frame in framer.push(pcm) {
                    voiced |= vad.push(&frame);
                }
                if voiced {
                    self.update(|s| s.voice_at = Some(Instant::now()));
                }
                match asr.as_mut() {
                    Some(ch) => ch.push(pcm),
                    None => pending.extend_from_slice(pcm),
                }
            }
            if let Some(stopped) = end {
                break stopped;
            }
            if let Some(ch) = asr.as_mut() {
                if ch.refresh() {
                    if let Some(text) = ch.partial() {
                        if text != shown {
                            shown = text.to_string();
                            self.update(|s| s.text = shown.clone());
                        }
                    }
                }
            }
        };
        drop(source);
        if !stopped {
            return;
        }
        // 停麦克风之前已经送出来的音频也要算上
        while let Ok(msg) = rx.try_recv() {
            match msg {
                Msg::Pcm(pcm) => match asr.as_mut() {
                    Some(ch) => ch.push(&pcm),
                    None => pending.extend_from_slice(&pcm),
                },
                Msg::Cancel => return,
                Msg::Stop => {}
            }
        }
        self.update(|s| s.phase = VOICE_RECOGNIZING);

        let mut asr = match asr {
            Some(ch) => ch,
            None => {
                let ready = self
                    .model
                    .wait_ready()
                    .and_then(|()| WtLocalChannel::new(&self.dir).map_err(|e| e.to_string()));
                match ready {
                    Ok(mut ch) => {
                        ch.feed(&pending);
                        ch
                    }
                    Err(e) => {
                        self.update(|s| {
                            s.phase = VOICE_ERROR;
                            s.text = e;
                        });
                        return;
                    }
                }
            }
        };
        let text = asr.finish().unwrap_or_default();
        self.update(|s| {
            if text.trim().is_empty() {
                s.phase = VOICE_ERROR;
                s.text = "没听清".into();
            } else {
                s.phase = VOICE_RESULT;
                s.text = text;
            }
        });
    }
}

/// 创建语音句柄；model_dir 为 NULL/空串时用默认目录（~/.local/share/wowtype/wtlocal）。
/// 不碰麦克风、不读模型，第一次按热键时才读；从来不下载（模型在设置界面里下载）。
#[no_mangle]
pub unsafe extern "C" fn wowtype_voice_new(model_dir: *const c_char) -> *mut WowtypeVoice {
    let dir = if model_dir.is_null() {
        None
    } else {
        unsafe { CStr::from_ptr(model_dir) }
            .to_str()
            .ok()
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
    };
    let dir = dir.unwrap_or_else(download::default_dir);
    Box::into_raw(Box::new(WowtypeVoice::new(dir)))
}

#[no_mangle]
pub unsafe extern "C" fn wowtype_voice_free(voice: *mut WowtypeVoice) {
    if !voice.is_null() {
        let mut voice = unsafe { Box::from_raw(voice) };
        voice.cancel();
    }
}

/// 热键按下。
#[no_mangle]
pub unsafe extern "C" fn wowtype_voice_key_down(voice: *mut WowtypeVoice) {
    if let Some(voice) = unsafe { voice.as_mut() } {
        let actions = voice.trigger.on_down(Instant::now());
        voice.apply(actions);
    }
}

/// 热键松开。按住超过 300ms 算长按，松开即结束；更短算点按，继续听到说完（静音 1.5 秒）或再按一次。
#[no_mangle]
pub unsafe extern "C" fn wowtype_voice_key_up(voice: *mut WowtypeVoice) {
    if let Some(voice) = unsafe { voice.as_mut() } {
        let actions = voice.trigger.on_up(Instant::now());
        voice.apply(actions);
    }
}

/// 不要这次的结果（按了 Esc、切走了输入框）。
#[no_mangle]
pub unsafe extern "C" fn wowtype_voice_cancel(voice: *mut WowtypeVoice) {
    if let Some(voice) = unsafe { voice.as_mut() } {
        voice.cancel();
    }
}

/// 定时调用：推进状态机（点按模式的静音结束），返回 VOICE_* 状态。
/// 文本用 wowtype_voice_text 取。
#[no_mangle]
pub unsafe extern "C" fn wowtype_voice_poll(voice: *mut WowtypeVoice) -> c_int {
    match unsafe { voice.as_mut() } {
        Some(voice) => voice.poll(),
        None => VOICE_IDLE,
    }
}

/// 上一次 poll 对应的文本；下一次 poll 或释放前有效。
#[no_mangle]
pub unsafe extern "C" fn wowtype_voice_text(voice: *const WowtypeVoice) -> *const c_char {
    match unsafe { voice.as_ref() } {
        Some(voice) => voice.text.as_ptr(),
        None => EMPTY.as_ptr() as *const c_char,
    }
}

/// 正在听时最近的音量，0–100。
#[no_mangle]
pub unsafe extern "C" fn wowtype_voice_level(voice: *const WowtypeVoice) -> c_int {
    match unsafe { voice.as_ref() } {
        Some(voice) => c_int::from(voice.shared.lock().unwrap().level),
        None => 0,
    }
}

/// 模型状态，MODEL_*。
#[no_mangle]
pub unsafe extern "C" fn wowtype_voice_model_state(voice: *const WowtypeVoice) -> c_int {
    match unsafe { voice.as_ref() } {
        Some(voice) => voice.model.code(),
        None => MODEL_NONE,
    }
}

/// 是否点按模式（松开热键后还在听）。
#[no_mangle]
pub unsafe extern "C" fn wowtype_voice_is_tap(voice: *const WowtypeVoice) -> c_int {
    match unsafe { voice.as_ref() } {
        Some(voice) => (voice.trigger.is_recording() && voice.trigger.mode() == Mode::Tap) as c_int,
        None => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_until_key_and_cancel_resets() {
        let dir = std::env::temp_dir().join(format!("wowtype-voice-none-{}", std::process::id()));
        let mut voice = WowtypeVoice::new(dir);
        assert_eq!(voice.poll(), VOICE_IDLE);
        assert_eq!(voice.model.code(), MODEL_NONE);
        voice.cancel();
        assert_eq!(voice.poll(), VOICE_IDLE);
        assert!(!voice.trigger.is_recording());
    }

    #[test]
    fn key_without_model_hints_and_downloads_nothing() {
        let root = std::env::temp_dir().join(format!("wowtype-nomodel-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let dir = root.join("wtlocal");
        let mut voice = WowtypeVoice::new(dir.clone());
        let actions = voice.trigger.on_down(Instant::now());
        voice.apply(actions);
        assert!(voice.tx.is_none(), "没有模型不该开录音线程");
        assert_eq!(voice.poll(), VOICE_ERROR);
        assert!(voice.text.to_str().unwrap().contains("设置"));
        assert_eq!(voice.model.code(), MODEL_MISSING);
        assert!(!voice.trigger.is_recording());
        let actions = voice.trigger.on_up(Instant::now());
        voice.apply(actions);
        assert_eq!(voice.poll(), VOICE_IDLE);
        // 什么也没下载：目录里没有模型、没有 .apk / .part
        assert!(!dir.exists());
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn level_maps_silence_and_speech() {
        assert_eq!(level_of(&[0; 320]), 0);
        assert!(level_of(&[8000, -8000].repeat(160)) > 70);
        let quiet = level_of(&[60, -60].repeat(160));
        assert!(quiet > 0 && quiet < 40, "{quiet}");
    }
}
