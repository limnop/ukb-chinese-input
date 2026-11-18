//! WowType 拼音引擎骨架。
//!
//! 流水线：
//! 输入预处理 -> 音节图 -> 束搜索（词典 + 二元 LM） -> 特征/线性打分 -> 候选。
//! LTR / 神经重排 / MixRank 只在接口上预留，尚未接入真实模型。

pub mod autospace;
pub mod beam;
pub mod calc;
pub mod clipboard;
pub mod convert;
pub mod datamark;
pub mod datetime;
pub mod emoji;
pub mod ffi;
pub mod fsutil;
pub mod graph;
pub mod learn;
pub mod lexicon;
pub mod llama;
pub mod lm;
pub mod nnlm;
pub mod preedit;
pub mod rank;
pub mod recent_emoji;
pub mod saver;
pub mod session;
pub mod session_ffi;
pub mod shuangpin;
pub mod stats;
pub mod stats_ffi;
pub mod syllable;
pub mod symbols;
pub mod train;
pub mod translate;
pub mod translate_ffi;
#[cfg(feature = "voice")]
pub mod voice_ffi;
#[cfg(not(feature = "voice"))]
pub mod voice_stub;
pub mod wordlm;
pub mod wubi;

use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;

pub use crate::beam::{beam_search, BeamConfig, BeamPath, BeamWord};
pub use crate::convert::Converter;
pub use crate::graph::{Edge, EdgeKind, GraphMode, GraphOptions, SyllableGraph};
pub use crate::learn::{LearnedLm, UserPairs};
pub use crate::lexicon::{
    FuzzyLexicon, LexEntry, Lexicon, MemoryLexicon, SyllableKey, UserLexicon,
};
pub use crate::lm::{ArpaLm, BigramLm, LanguageModel};
pub use crate::rank::{
    Feature, FeatureWeights, LinearRanker, LtrModel, Ranker, FEATURE_COUNT, FEATURE_NAMES,
};
pub use crate::syllable::{
    is_valid, parse, split, ParseError, ParsedInput, Syllable, SyllableId, SyllableQuery, SYLLABLES,
};

/// 最终候选。
#[derive(Clone, Debug)]
pub struct Candidate {
    pub text: String,
    pub pinyin: String,
    pub score: f64,
    pub features: Feature,
    pub words: Vec<CandidateWord>,
    /// 这个候选用掉了原始输入的前多少字节。
    ///
    /// 整句候选等于输入长度；句首词候选（长句先选「你好」，剩下的接着打）更短，
    /// 前端上屏后应把剩余部分留在输入框里继续转换。
    pub consumed: usize,
}

/// 候选内的一个分词。
#[derive(Clone, Debug)]
pub struct CandidateWord {
    pub text: String,
    pub pinyin: String,
    pub freq: f64,
    pub known: bool,
}

/// 引擎配置。
#[derive(Clone, Debug)]
pub struct EngineConfig {
    pub beam: BeamConfig,
    pub max_candidates: usize,
    /// 全拼走不通时是否尝试简拼（zg -> 中国）。
    pub jianpin_fallback: bool,
    /// 是否启用模糊音（总开关）。
    pub fuzzy: bool,
    /// 打开的模糊音分组（`syllable::fuzzy::*` 位掩码），默认平翘舌 + 前后鼻音。
    pub fuzzy_groups: u32,
    /// 每用一处模糊音（一个声母或韵母）多出的代价（自然对数），越大越靠后。
    pub fuzzy_penalty: f64,
    /// 模糊音关着时的兜底：精确读音只能凑出硬拼的结果（yingai -> 因该），而按模糊音
    /// 是个常用词（应该）时，把这个词也给出来。见 [`Engine::fuzzy_fallback`]。
    pub fuzzy_fallback: bool,
    /// 是否使用双拼。
    pub shuangpin: bool,
    /// 双拼方案（自然码、小鹤……）。
    pub shuangpin_scheme: crate::shuangpin::Scheme,
    /// 是否在候选末尾追加原始输入（英文/未登录串）。
    pub english_candidate: bool,
    /// 是否把候选转成繁体。
    pub traditional: bool,
    /// 选词达到阈值后自动加入用户词库。
    pub auto_learn: bool,
    /// 自动学习阈值（选多少次后进用户词库）。
    pub auto_learn_threshold: u32,
    /// 按错键纠错（相邻键、两键按反、多按/漏按）。双拼下不生效。
    pub typo_correction: bool,
    /// 句首词候选最多几个（0 表示不给）。长句打错时可以先选开头的词。
    pub prefix_candidates: usize,
    /// 前面几页之后把剩下的词全补上（整个输入的同音词、句首词、单字），直到
    /// `max_candidates`。关掉时只给最可能的那几页。
    pub more_candidates: bool,
    /// 有神经语言模型时，按束搜索分数取前几条整句用它重排（0 表示不用）。
    pub nnlm_rerank: usize,
}

/// 上文末尾和候选连起来本身是一个词（我 + 要 = 我要）时，上文排序特征至少给这么多
/// （自然对数，和语言模型的分数同一个尺度）。见 [`Engine::word_lm_context_score`]。
pub const MERGED_PAIR_SCORE: f64 = 1.5;

/// 分几次选出来的词，多长以内当成用户造的词（人名、术语、成语大多在四个字以内）。
pub const COMPOSED_WORD_MAX_CHARS: usize = 4;

/// 能拿来联想的词：都是字母、数字、汉字（没有标点、符号、`</s>` 这类记号），不是单个英文字母。
/// 有字级神经语言模型时，联想先从词级模型取这么多个后续词（差不多是前一个词的全部后续词），
/// 按字级模型给第一个字的概率留下 [`ASSOCIATE_RESCORE`] 个，再算整词的概率排出一页。
const ASSOCIATE_POOL: usize = 2000;
/// 见 [`ASSOCIATE_POOL`]：整词的概率每个要算一两步前向（约 1 ms 一步），留得太多上屏时会卡。
const ASSOCIATE_RESCORE: usize = 12;
/// 字级模型认为「下一个字是这个词的第一个字」的 log 概率低于这个（约 1/400）就不拿来联想（学到的搭配不受限）。
const ASSOCIATE_MIN_LOGPROB: f64 = -6.0;

fn associable(word: &str) -> bool {
    let n = word.chars().count();
    n > 0 && word.chars().all(char::is_alphanumeric) && !(n == 1 && word.is_ascii())
}

/// 热度够不够收词：一会儿工夫连着选三次，热度会因为这几毫秒的衰减差一点点到 3，不能因此不收。
fn reached(heat: f64, threshold: f64) -> bool {
    heat + 0.01 >= threshold
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            beam: BeamConfig::default(),
            max_candidates: 10,
            jianpin_fallback: true,
            fuzzy: false,
            fuzzy_groups: crate::syllable::fuzzy::DEFAULT,
            fuzzy_penalty: 2.5,
            fuzzy_fallback: true,
            shuangpin: false,
            shuangpin_scheme: crate::shuangpin::Scheme::Ziranma,
            english_candidate: true,
            traditional: false,
            auto_learn: true,
            auto_learn_threshold: 3,
            typo_correction: true,
            prefix_candidates: 16,
            more_candidates: true,
            nnlm_rerank: 8,
        }
    }
}

/// 用户历史里的一条记录。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HistoryEntry {
    /// 热度：上次使用时的次数，已经按半衰期衰减过，可以带小数。到某个时刻的热度用 [`learn::decayed`] 算。
    pub count: f64,
    /// 上次使用时间（epoch 毫秒）。
    pub last_used_ms: u64,
}

/// 当前时间（epoch 毫秒）。
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 拼音引擎。
pub struct Engine {
    /// 原始词库（只读）。
    base_lexicon: Arc<dyn Lexicon>,
    /// 基础词库 + 用户词的合并视图，束搜索实际用它。
    lexicon: Arc<UserLexicon>,
    /// 实际用的词级语言模型：通用模型，或者混上了领域模型（[`Engine::set_domain_lm`]）。
    lm: Arc<dyn LanguageModel>,
    /// 通用模型本身：换掉、去掉领域模型时从它重新混。
    general_lm: Arc<dyn LanguageModel>,
    ranker: Arc<dyn Ranker>,
    config: EngineConfig,
    /// 用户选词历史：候选文本 -> (次数, 上次时间)。
    history: RwLock<HashMap<String, HistoryEntry>>,
    /// 光标前已上屏的文本，用于上下文 bigram 特征。
    context: RwLock<String>,
    /// 用户学到的词搭配（上一个词 -> 这个词）。
    pairs: RwLock<UserPairs>,
    /// 最近一次选的词的最后一个词；光标前的文本以它结尾时，下一个词和它组成搭配。
    last_word: RwLock<String>,
    /// 上一次 input() 给出的候选：文本 -> 分词，选词时用它知道整句是哪几个词。
    /// 分词是词库里的写法；开着繁体时显示的文本和转换前的文本都能查到。
    last_words: RwLock<HashMap<String, Vec<String>>>,
    /// 简繁字符映射表。
    converter: RwLock<Arc<Converter>>,
    /// 英文词表：小写编码 -> 上屏形式（github -> GitHub）。
    english: RwLock<Arc<HashMap<String, String>>>,
    /// 字级神经语言模型（微信输入法的 nlrgpt2，本机有才加载），给整句候选重排。
    nnlm: RwLock<Option<Arc<nnlm::Nnlm>>>,
    /// 五笔码表（选了五笔才载入，见 [`Engine::load_wubi`]）。
    wubi: RwLock<Option<Arc<wubi::WubiTable>>>,
    /// 候选新词：分几次选出来拼成、第一次出现的词，第二次再拼出来才收进用户词库。
    new_words: RwLock<HashMap<String, learn::NewWord>>,
    /// 最近的学习改动（[`Engine::undo_learning`] 用）。
    journal: Mutex<learn::Journal>,
    /// 后台存盘（[`Engine::save_user_dict_async`]）。引擎释放时先写完。
    saver: saver::Saver,
}

impl Engine {
    pub fn new(lexicon: Arc<dyn Lexicon>, lm: Arc<dyn LanguageModel>) -> Self {
        let merged = Arc::new(UserLexicon::new(lexicon.clone()));
        Self {
            base_lexicon: lexicon,
            lexicon: merged,
            general_lm: lm.clone(),
            lm,
            ranker: Arc::new(LinearRanker::default()),
            config: EngineConfig::default(),
            history: RwLock::new(HashMap::new()),
            context: RwLock::new(String::new()),
            pairs: RwLock::new(UserPairs::default()),
            last_word: RwLock::new(String::new()),
            last_words: RwLock::new(HashMap::new()),
            converter: RwLock::new(Arc::new(Converter::new())),
            english: RwLock::new(Arc::new(HashMap::new())),
            nnlm: RwLock::new(None),
            wubi: RwLock::new(None),
            new_words: RwLock::new(HashMap::new()),
            journal: Mutex::new(learn::Journal::default()),
            saver: saver::Saver::default(),
        }
    }

    /// 加载微信输入法的字级神经语言模型（`nlrgpt2_nnlm.bin`），返回字表大小。
    /// 模型不随 WowType 分发，见 `docs/DATA.md`。
    pub fn load_nnlm<P: AsRef<std::path::Path>>(&self, path: P) -> Result<usize> {
        let model = nnlm::Nnlm::load(path)?;
        let size = model.vocab_size();
        self.set_nnlm(Some(Arc::new(model)));
        Ok(size)
    }

    pub fn set_nnlm(&self, model: Option<Arc<nnlm::Nnlm>>) {
        if let Ok(mut slot) = self.nnlm.write() {
            *slot = model;
        }
    }

    /// 混上一个领域词级模型（用户拿自己的语料用 `wowtype-lmbuild` 训的），None 去掉。
    /// 再设一次是换掉上一个，不会一层层叠着混。权重是 [`lm::DOMAIN_LM_WEIGHT`]。
    pub fn set_domain_lm(&mut self, model: Option<Arc<dyn LanguageModel>>) {
        self.lm = match model {
            Some(domain) => Arc::new(lm::MixedLm::new(
                self.general_lm.clone(),
                domain,
                lm::DOMAIN_LM_WEIGHT,
            )),
            None => self.general_lm.clone(),
        };
    }

    pub fn has_nnlm(&self) -> bool {
        self.nnlm.read().map(|m| m.is_some()).unwrap_or(false)
    }

    /// 设置光标前的文本（上一次上屏的内容）。
    pub fn set_context(&self, text: &str) {
        if let Ok(mut slot) = self.context.write() {
            slot.clear();
            slot.push_str(text);
        }
    }

    pub fn context(&self) -> String {
        self.context.read().map(|s| s.clone()).unwrap_or_default()
    }

    /// 记录一次选词，用于历史特征。时间戳取当前时间。
    pub fn record_selection(&self, word: &str) {
        self.record_selection_at(word, now_ms());
    }

    /// 记录选词；当同一个词选够次数后自动加入用户词库。
    ///
    /// pinyin 是这次上屏时输入的拼音（用于生成用户词条）。词库里已有的词也会被
    /// 加成用户词——这等价于「记住用户的偏好」，不会覆盖原词条。
    /// 「选够次数」看的是衰减后的热度（[`learn::decayed`]），很久以前零零散散选过几次的不算。
    /// 是候选新词（[`Engine::record_composed_word`] 第一次拼出来的）时，这是第二次用，也收进去。
    pub fn record_selection_with_pinyin(&self, word: &str, pinyin: Option<&str>) {
        self.record_selection_with_pinyin_at(word, pinyin, now_ms());
    }

    /// 带时间戳的 [`Engine::record_selection_with_pinyin`]。
    pub fn record_selection_with_pinyin_at(&self, word: &str, pinyin: Option<&str>, at_ms: u64) {
        let word = &self.lexicon_form(word);
        let heat = self.record_selection_at(word, at_ms);
        if !self.config.auto_learn || word.is_empty() {
            return;
        }
        let Some(pinyin) = pinyin.filter(|p| !p.trim().is_empty()) else {
            return;
        };
        let threshold = f64::from(self.config.auto_learn_threshold.max(1));
        if reached(heat, threshold) {
            self.add_learned_word(word, pinyin, heat);
        } else if self.take_new_word(word, at_ms) {
            self.add_learned_word(word, pinyin, heat.max(threshold));
        }
    }

    /// 学到的词收进用户词库。heat 是选用热度，决定频次。
    fn add_learned_word(&self, word: &str, pinyin: &str, heat: f64) {
        // 候选带的拼音已经是全拼（双拼也翻好了）；以前的前端会传双拼键位串（如 "vsgo"），
        // 那样的要先展开，否则用户词会带着一串无效音节存进去，永远也打不出来。
        let pinyin = if self.config.shuangpin && !pinyin.trim().contains(' ') {
            crate::shuangpin::to_full_pinyin(self.config.shuangpin_scheme, pinyin.trim())
        } else {
            pinyin.to_string()
        };
        if pinyin.trim().is_empty() {
            return;
        }
        // 频次随选用次数增长但有上限，避免用户词压过通用高频词
        let freq = (1000.0 * heat.round().max(1.0)).min(50_000.0);
        let before = self.lexicon.entry(word);
        if self.lexicon.add(&pinyin, word, freq).is_ok() {
            self.journal(vec![learn::Change::UserWord {
                word: word.to_string(),
                before,
                after: self.lexicon.entry(word),
            }]);
        }
    }

    /// 用户分几次选出来、拼成的一个词（最多 [`COMPOSED_WORD_MAX_CHARS`] 个字）（「霜」「频」「阁」→ 霜频阁）：
    /// 记一次选词；第一次只记成「候选新词」，[`learn::NEW_WORD_TTL_MS`]（30 天）里再用一次（再这样拼出来，
    /// 或者直接选了它）才收进用户词库，不用等选够 [`EngineConfig::auto_learn_threshold`] 次。
    ///
    /// 用户特意一段一段挑出来的，多半就是想要这个词；可一次就收的话，挑错一个字（「你好事竣」）
    /// 也成了用户词，所以要第二次确认。一次就打出来的整句不走这里，还是按次数学。
    /// 反馈见 https://x.com/so1v3/status/2120415675969896729
    pub fn record_composed_word(&self, word: &str, pinyin: &str) {
        self.record_composed_word_at(word, pinyin, now_ms());
    }

    /// 带时间戳的 [`Engine::record_composed_word`]。
    pub fn record_composed_word_at(&self, word: &str, pinyin: &str, at_ms: u64) {
        let word = &self.lexicon_form(word);
        // 超过四个字的多半是先选句首词、再选剩下的长句，不是要造的词：照常按次数学
        if word.chars().count() > COMPOSED_WORD_MAX_CHARS {
            self.record_selection_with_pinyin_at(word, Some(pinyin), at_ms);
            return;
        }
        let heat = self.record_selection_at(word, at_ms);
        if !self.config.auto_learn || word.is_empty() || pinyin.trim().is_empty() {
            return;
        }
        // 收的时候至少给到「选够次数」那一档的频次，下次一打就排前面
        let threshold = f64::from(self.config.auto_learn_threshold.max(1));
        if self.lexicon.entry(word).is_some()
            || reached(heat, threshold)
            || self.take_new_word(word, at_ms)
        {
            self.add_learned_word(word, pinyin, heat.max(threshold));
            return;
        }
        let Ok(mut new_words) = self.new_words.write() else {
            return;
        };
        let before = new_words.get(word.as_str()).cloned();
        let after = learn::NewWord {
            pinyin: pinyin.trim().to_string(),
            at_ms,
        };
        new_words.insert(word.clone(), after.clone());
        drop(new_words);
        self.journal(vec![learn::Change::NewWord {
            word: word.clone(),
            before,
            after: Some(after),
        }]);
    }

    /// word 是候选新词：从候选新词里拿掉（第二次用了，要么收进用户词库，要么过期了重新算），
    /// 没过期时返回 true。
    fn take_new_word(&self, word: &str, at_ms: u64) -> bool {
        let Ok(mut new_words) = self.new_words.write() else {
            return false;
        };
        let Some(seen) = new_words.remove(word) else {
            return false;
        };
        drop(new_words);
        let fresh = at_ms.saturating_sub(seen.at_ms) <= learn::NEW_WORD_TTL_MS;
        self.journal(vec![learn::Change::NewWord {
            word: word.to_string(),
            before: Some(seen),
            after: None,
        }]);
        fresh
    }

    /// 是不是还在等第二次的候选新词。
    pub fn is_new_word(&self, word: &str) -> bool {
        let word = self.lexicon_form(word);
        self.new_words.read().is_ok_and(|n| n.contains_key(&word))
    }

    /// 带时间戳的选词记录（方便测试），返回这次以后的热度。
    ///
    /// 记两样：这个词的热度，和它跟前后词的搭配（见 [`learn`]）。改了什么记进学习日志，
    /// [`Engine::undo_learning`] 能退回去。
    pub fn record_selection_at(&self, word: &str, at_ms: u64) -> f64 {
        if word.is_empty() {
            return 0.0;
        }
        let word = &self.lexicon_form(word);
        let mut changes = Vec::new();
        let mut heat = 0.0;
        if let Ok(mut history) = self.history.write() {
            // 先按时间衰减再加 1：很久以前用得多的词不会一直顶着
            let before = history.get(word.as_str()).copied();
            let after = learn::bumped(before, at_ms);
            heat = after.count;
            history.insert(word.to_string(), after);
            changes.push(learn::Change::Word {
                word: word.to_string(),
                before,
                after: Some(after),
            });
        }
        // 这个候选是哪几个词拼的（整句「我发给你」= 我 | 发给 | 你）
        let words: Vec<String> = self
            .last_words
            .read()
            .ok()
            .and_then(|m| m.get(word).cloned())
            .filter(|w| !w.is_empty())
            .unwrap_or_else(|| vec![word.to_string()]);
        let start = self.start_word();
        if let Ok(mut pairs) = self.pairs.write() {
            if let Some(prev) = &start {
                changes.extend(pairs.learn(prev, &words[0], at_ms));
            }
            for pair in words.windows(2) {
                changes.extend(pairs.learn(&pair[0], &pair[1], at_ms));
            }
        }
        if let (Ok(mut last), Some(tail)) = (self.last_word.write(), words.last()) {
            if *last != *tail {
                changes.push(learn::Change::LastWord {
                    before: std::mem::replace(&mut *last, tail.clone()),
                    after: tail.clone(),
                });
            }
        }
        self.journal(changes);
        heat
    }

    fn journal(&self, changes: Vec<learn::Change>) {
        if let Ok(mut journal) = self.journal.lock() {
            for change in changes {
                journal.push(change);
            }
        }
    }

    /// 学习日志的位置：学习前后各取一次，[`Engine::undo_learning`] 拿这两个数退回中间学到的。
    pub fn learn_mark(&self) -> u64 {
        self.journal.lock().map(|j| j.mark()).unwrap_or(0)
    }

    /// 撤销 [start, end)（[`Engine::learn_mark`] 取的）之间学到的东西：选词热度、搭配、自动收进用户词库的词、
    /// 候选新词都回到学之前。只改回后来没再被改过的（别的输入框接着学了同一个词，那份不动）。
    /// 不重算：学的时候记下了改之前的值。start 已经挤出日志（隔了上千条改动）时什么也不做。
    /// 返回是不是真的退了什么。
    pub fn undo_learning(&self, start: u64, end: u64) -> bool {
        let Some(changes) = self.journal.lock().ok().and_then(|mut j| j.take(start, end)) else {
            return false;
        };
        let mut undone = false;
        for change in changes {
            undone |= self.revert(change);
        }
        undone
    }

    /// 把一条改动改回去（现在的值还是那次改成的才改）。
    fn revert(&self, change: learn::Change) -> bool {
        match change {
            learn::Change::Word { word, before, after } => {
                let Ok(mut history) = self.history.write() else {
                    return false;
                };
                if history.get(&word).copied() != after {
                    return false;
                }
                match before {
                    Some(entry) => history.insert(word, entry),
                    None => history.remove(&word),
                };
            }
            learn::Change::Pair {
                prev,
                cur,
                before,
                after,
            } => {
                let Ok(mut pairs) = self.pairs.write() else {
                    return false;
                };
                if pairs.get(&prev, &cur) != after {
                    return false;
                }
                pairs.set(&prev, &cur, before);
            }
            learn::Change::LastWord { before, after } => {
                let Ok(mut last) = self.last_word.write() else {
                    return false;
                };
                if *last != after {
                    return false;
                }
                *last = before;
            }
            learn::Change::UserWord { word, before, after } => {
                if self.lexicon.entry(&word) != after {
                    return false;
                }
                match before {
                    Some((pinyin, freq)) => {
                        let _ = self.lexicon.add(&pinyin, &word, freq);
                    }
                    None => {
                        self.lexicon.remove(&word);
                    }
                }
            }
            learn::Change::NewWord { word, before, after } => {
                let Ok(mut new_words) = self.new_words.write() else {
                    return false;
                };
                if new_words.get(&word) != after.as_ref() {
                    return false;
                }
                match before {
                    Some(seen) => new_words.insert(word, seen),
                    None => new_words.remove(&word),
                };
            }
        }
        true
    }

    /// 光标前的最后一个词：上一次选的词，而且光标前的文本正好以它结尾
    /// （中间换了输入框、删了字，就不算）。
    fn start_word(&self) -> Option<String> {
        let last = self.last_word.read().ok()?.clone();
        if last.is_empty() {
            return None;
        }
        // 记的是词库里的写法，光标前是上屏的（开着繁体就是转换过的）字
        let shown = self.shown_text(&last);
        let context = self.context.read().ok()?;
        context.trim_end().ends_with(&shown).then_some(last)
    }

    /// 候选显示的字在词库里怎么写。开着繁体时候选是转换过的，学词、记热度和搭配、
    /// 忘词都得用转换前的写法：词库、热度、搭配都按它查，关掉繁体后学过的词也照样能用。
    /// 不是上一次 input() 的候选就原样返回。
    pub fn lexicon_form(&self, shown: &str) -> String {
        if self.config.traditional {
            if let Some(words) = self.last_words.read().ok().and_then(|m| m.get(shown).cloned()) {
                if !words.is_empty() {
                    return words.concat();
                }
            }
        }
        shown.to_string()
    }

    /// 词库里的写法在候选里显示成什么：开着繁体就转一次。
    pub(crate) fn shown_text(&self, text: &str) -> String {
        match self.converter.read() {
            Ok(conv) if self.config.traditional && !conv.is_empty() => conv.convert(text),
            _ => text.to_string(),
        }
    }

    /// 学到的搭配数。
    pub fn pair_count(&self) -> usize {
        self.pairs.read().map(|p| p.len()).unwrap_or(0)
    }

    /// 把用户输入习惯（词的热度、词搭配、候选新词）存到 TSV。热度已经衰减得几乎没有的不写。
    /// 在调用的线程上写完才返回（要等 fsync）；按键的线程上用 [`Engine::save_history_async`]。
    pub fn save_history<P: AsRef<std::path::Path>>(&self, path: P) -> Result<()> {
        // 后台还排着、正在写的旧数据先写完，别等这次写好以后再盖上去
        self.saver.flush();
        let (history, pairs, new_words) = self.history_snapshot()?;
        let text = crate::learn::history_text(&history, &pairs, &new_words, now_ms());
        crate::fsutil::write_atomic(path.as_ref(), text.as_bytes())
    }

    /// 在后台存输入习惯：这里只在锁里抄一份，排成文字、写文件（两次 fsync）在存盘的线程上做。
    /// 连着几次还没写的合并成一次。[`Engine::save_wait`] 等写完。
    pub fn save_history_async<P: AsRef<std::path::Path>>(&self, path: P) -> Result<()> {
        let (history, pairs, new_words) = self.history_snapshot()?;
        let now = now_ms();
        self.saver.submit(
            path.as_ref().to_path_buf(),
            Box::new(move || Ok(crate::learn::history_text(&history, &pairs, &new_words, now))),
        );
        Ok(())
    }

    /// 在后台存用户词库，见 [`Engine::save_history_async`]。
    pub fn save_user_dict_async<P: AsRef<std::path::Path>>(&self, path: P) -> Result<()> {
        let words = self.lexicon.words();
        self.saver.submit(
            path.as_ref().to_path_buf(),
            Box::new(move || Ok(crate::lexicon::user_dict_text(&words))),
        );
        Ok(())
    }

    /// 等后台存盘写完：timeout 是 None 时一直等（进程退出、换词库前），Some(0) 只看一眼。
    /// 写失败的原因只报一次。引擎释放时也会等写完。
    pub fn save_wait(&self, timeout: Option<std::time::Duration>) -> saver::SaveStatus {
        self.saver.wait(timeout)
    }

    /// 输入习惯的一份拷贝（在锁里抄，存盘用）。
    #[allow(clippy::type_complexity)]
    fn history_snapshot(
        &self,
    ) -> Result<(HashMap<String, HistoryEntry>, UserPairs, HashMap<String, learn::NewWord>)> {
        let history = self
            .history
            .read()
            .map_err(|_| anyhow::anyhow!("history lock poisoned"))?
            .clone();
        let pairs = self
            .pairs
            .read()
            .map_err(|_| anyhow::anyhow!("pairs lock poisoned"))?
            .clone();
        let new_words = self
            .new_words
            .read()
            .map_err(|_| anyhow::anyhow!("new words lock poisoned"))?
            .clone();
        Ok((history, pairs, new_words))
    }

    /// 读回 [`Engine::save_history`] 存的输入习惯，返回条数。
    pub fn load_history<P: AsRef<std::path::Path>>(&self, path: P) -> Result<usize> {
        let mut history = self
            .history
            .write()
            .map_err(|_| anyhow::anyhow!("history lock poisoned"))?;
        let mut pairs = self
            .pairs
            .write()
            .map_err(|_| anyhow::anyhow!("pairs lock poisoned"))?;
        let mut new_words = self
            .new_words
            .write()
            .map_err(|_| anyhow::anyhow!("new words lock poisoned"))?;
        crate::learn::load_history(path, &mut history, &mut pairs, &mut new_words)
    }

    /// 上次选这个词时的次数（衰减过、四舍五入），没选过是 0。
    pub fn history_count(&self, word: &str) -> u32 {
        self.history
            .read()
            .ok()
            .and_then(|h| h.get(word).map(|e| e.count.round() as u32))
            .unwrap_or(0)
    }

    /// 这个词 at_ms 时的选词热度（衰减到那时），没选过是 0。
    pub fn history_heat(&self, word: &str, at_ms: u64) -> f64 {
        self.history
            .read()
            .ok()
            .and_then(|h| h.get(word).map(|e| learn::decayed(*e, at_ms)))
            .unwrap_or(0.0)
    }

    pub fn with_config(mut self, config: EngineConfig) -> Self {
        self.config = config;
        self
    }

    pub fn with_ranker(mut self, ranker: Arc<dyn Ranker>) -> Self {
        self.ranker = ranker;
        self
    }

    pub fn set_beam_size(&mut self, beam_size: usize) {
        self.config.beam.beam_size = beam_size.max(1);
    }

    /// 开关模糊音。
    pub fn set_fuzzy(&mut self, on: bool) {
        self.config.fuzzy = on;
    }

    /// 把模糊音索引先建好。模糊音打开或者兜底（默认开）都要用；不先建的话，第一次碰到
    /// 要查模糊音的输入时现建，会卡一下（55 万条约 0.2 秒）。载入词库的后台线程里调。
    pub fn prepare_fuzzy(&self) {
        self.base_lexicon.prepare_fuzzy();
    }

    /// 模糊音关着时的兜底（见 [`EngineConfig::fuzzy_fallback`]），默认开。
    pub fn set_fuzzy_fallback(&mut self, on: bool) {
        self.config.fuzzy_fallback = on;
    }

    /// 设置打开的模糊音分组（`syllable::fuzzy::*` 位掩码）。不用重建索引。
    pub fn set_fuzzy_groups(&mut self, groups: u32) {
        self.config.fuzzy_groups = groups & crate::syllable::fuzzy::ALL;
    }

    pub fn set_typo_correction(&mut self, on: bool) {
        self.config.typo_correction = on;
    }

    /// 是否也出 CJK 扩展 B 区及以后的单字（默认不出，常见字体显示不了；通用规范汉字表里的照常）。
    pub fn set_rare_ext_chars(&mut self, on: bool) {
        self.config.beam.rare_ext_chars = on;
    }

    /// 开关双拼。
    pub fn set_shuangpin(&mut self, on: bool) {
        self.config.shuangpin = on;
    }

    /// 选双拼方案（不改开关）。
    pub fn set_shuangpin_scheme(&mut self, scheme: crate::shuangpin::Scheme) {
        self.config.shuangpin_scheme = scheme;
    }

    /// 编辑区里怎么显示还没选的输入：双拼翻成全拼（`vsgo` -> `zhong'guo`），全拼返回 None。
    pub fn shuangpin_display(&self, input: &str) -> Option<String> {
        self.config
            .shuangpin
            .then(|| crate::shuangpin::display(self.config.shuangpin_scheme, input))
    }

    /// 开关繁体输出。
    pub fn set_traditional(&mut self, on: bool) {
        self.config.traditional = on;
    }

    /// 载入简繁映射表，返回条目数。
    /// 载入英文词表，返回词条数。
    ///
    /// 每行一个词；也直接接受 rime 的 en.dict.yaml（「上屏<TAB>编码」，
    /// 注释和 YAML 头会被跳过）。只收 2~32 个字母的编码。
    pub fn load_english<P: AsRef<std::path::Path>>(&self, path: P) -> Result<usize> {
        let text = std::fs::read_to_string(path)?;
        Ok(self.load_english_str(&text))
    }

    /// 从字符串载入英文词表（追加），返回累计词条数。
    pub fn load_english_str(&self, text: &str) -> usize {
        let mut map: HashMap<String, String> = self
            .english
            .read()
            .map(|m| m.as_ref().clone())
            .unwrap_or_default();
        for line in text.lines() {
            let line = line.trim_end_matches('\r');
            if line.starts_with('#') || line.trim().is_empty() {
                continue;
            }
            let mut cols = line.split('\t');
            let word = cols.next().unwrap_or("").trim();
            let code = cols.next().map(str::trim).unwrap_or(word);
            if code.len() < 2 || code.len() > 32 || !code.bytes().all(|b| b.is_ascii_alphabetic()) {
                continue;
            }
            if word.is_empty() || !word.eq_ignore_ascii_case(code) {
                // 缩写（January<TAB>Jan）只认上屏与编码一致的条目
                continue;
            }
            let key = code.to_ascii_lowercase();
            match map.entry(key) {
                Entry::Vacant(slot) => {
                    slot.insert(word.to_string());
                }
                Entry::Occupied(mut slot) => {
                    // 同一个词有大小写两种写法时，留全小写的（march 而不是 March）
                    if word.bytes().all(|b| !b.is_ascii_uppercase()) {
                        slot.insert(word.to_string());
                    }
                }
            }
        }
        let len = map.len();
        if let Ok(mut slot) = self.english.write() {
            *slot = Arc::new(map);
        }
        len
    }

    pub fn english_len(&self) -> usize {
        self.english.read().map(|m| m.len()).unwrap_or(0)
    }

    pub fn load_conversion<P: AsRef<std::path::Path>>(&self, path: P) -> Result<usize> {
        let conv = Converter::from_tsv_path(path)?;
        let len = conv.len();
        if let Ok(mut slot) = self.converter.write() {
            *slot = Arc::new(conv);
        }
        Ok(len)
    }

    /// 直接装载一张映射表（测试或程序内生成时用）。
    pub fn install_converter(&self, conv: Converter) {
        if let Ok(mut slot) = self.converter.write() {
            *slot = Arc::new(conv);
        }
    }

    pub fn conversion_len(&self) -> usize {
        self.converter.read().map(|c| c.len()).unwrap_or(0)
    }

    /// 把候选转成繁体，并合并转换后重复的候选。分词不转，还是词库里的写法（学词用）。
    fn apply_traditional(&self, candidates: &mut Vec<Candidate>) {
        let conv = match self.converter.read() {
            Ok(slot) if !slot.is_empty() => slot.clone(),
            _ => return,
        };
        let mut best: HashMap<String, Candidate> = HashMap::new();
        for mut candidate in std::mem::take(candidates) {
            candidate.text = conv.convert(&candidate.text);
            match best.entry(candidate.text.clone()) {
                Entry::Vacant(slot) => {
                    slot.insert(candidate);
                }
                Entry::Occupied(mut slot) => {
                    if candidate.score > slot.get().score {
                        slot.insert(candidate);
                    }
                }
            }
        }
        let mut list: Vec<Candidate> = best.into_values().collect();
        list.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then_with(|| a.text.cmp(&b.text))
        });
        list.truncate(self.config.max_candidates.max(1));
        *candidates = list;
    }

    /// 载入五笔码表（`wubi86.tsv`），替换上一次载入的，返回条数。
    pub fn load_wubi<P: AsRef<std::path::Path>>(&self, path: P) -> Result<usize> {
        let table = wubi::WubiTable::load_tsv(path)?;
        let len = table.len();
        self.set_wubi(Some(Arc::new(table)));
        Ok(len)
    }

    /// 换五笔码表，None 卸掉。
    pub fn set_wubi(&self, table: Option<Arc<wubi::WubiTable>>) {
        if let Ok(mut slot) = self.wubi.write() {
            *slot = table;
        }
    }

    /// 载入了的五笔码表。
    pub fn wubi(&self) -> Option<Arc<wubi::WubiTable>> {
        self.wubi.read().ok().and_then(|t| t.clone())
    }

    pub fn set_max_candidates(&mut self, max_candidates: usize) {
        self.config.max_candidates = max_candidates.max(1);
    }

    pub fn lexicon(&self) -> &Arc<UserLexicon> {
        &self.lexicon
    }

    pub fn base_lexicon(&self) -> &Arc<dyn Lexicon> {
        &self.base_lexicon
    }

    /// 添加用户词（拼音可以是 ni hao / nihao / xi'an）。
    pub fn add_user_word(&self, pinyin: &str, word: &str, freq: f64) -> Result<()> {
        self.lexicon.add(pinyin, word, freq)
    }

    /// 删除用户词，返回是否删掉了。
    pub fn remove_user_word(&self, word: &str) -> bool {
        self.lexicon.remove(word)
    }

    /// 忘掉学过的一个词（候选框里 Ctrl+Delete）：用户词库里的这个词、它的选词热度、
    /// 和它有关的搭配都去掉。词库里本来就有的词删不掉，去掉学到的加分后回到原来的位置。
    /// 设置里加的自定义词不动。返回是否真的忘掉了什么。
    pub fn forget_word(&self, word: &str) -> bool {
        let word = self.lexicon_form(word);
        let word = word.as_str();
        if word.is_empty() || self.lexicon.is_custom(word) {
            return false;
        }
        let mut forgot = self.lexicon.remove(word);
        if let Ok(mut history) = self.history.write() {
            forgot |= history.remove(word).is_some();
        }
        if let Ok(mut new_words) = self.new_words.write() {
            forgot |= new_words.remove(word).is_some();
        }
        if let Ok(mut pairs) = self.pairs.write() {
            forgot |= pairs.remove_word(word) > 0;
        }
        // 下一次上屏别再拿刚忘掉的词接搭配
        if let Ok(mut last) = self.last_word.write() {
            if *last == word {
                last.clear();
            }
        }
        forgot
    }

    pub fn user_words(&self) -> Vec<(String, String, f64)> {
        self.lexicon.words()
    }

    /// 存用户词库（在调用的线程上写完才返回）；按键的线程上用 [`Engine::save_user_dict_async`]。
    pub fn save_user_dict<P: AsRef<std::path::Path>>(&self, path: P) -> Result<()> {
        self.saver.flush();
        self.lexicon.save_tsv(path)
    }

    pub fn load_user_dict<P: AsRef<std::path::Path>>(&self, path: P) -> Result<usize> {
        self.lexicon.load_tsv(path)
    }

    /// 读自定义词文件（custom.tsv），整体替换上一次读到的；文件不存在就清空。
    pub fn load_custom_words<P: AsRef<std::path::Path>>(&self, path: P) -> Result<usize> {
        self.lexicon.load_custom(path)
    }

    pub fn lm(&self) -> &Arc<dyn LanguageModel> {
        &self.lm
    }

    pub fn config(&self) -> &EngineConfig {
        &self.config
    }

    /// 基于上一词的候选预测（「预测词库」）：先是用户学到的搭配（prev 后面选过的，热的在前，
    /// 热度不到 [`learn::MIN_PICK_HEAT`] 的不算），再是词级语言模型里常跟在 prev 后面的词。
    ///
    /// 返回 (词, 分数)，按分数降序：模型的是对数概率（不大于 0），学到的搭配是 ln(1+热度)（大于 0，排在前面）。
    /// 都是词库里的写法。语言模型没接入预测能力、也没学过时返回空。
    pub fn predict_next(&self, prev: &str, limit: usize) -> Vec<(String, f64)> {
        let mut out: Vec<(String, f64)> = self
            .pairs
            .read()
            .map(|pairs| pairs.next_words(prev, now_ms()))
            .unwrap_or_default()
            .into_iter()
            .filter(|(_, heat)| *heat >= learn::MIN_PICK_HEAT)
            .map(|(word, heat)| (word, heat.ln_1p()))
            .take(limit)
            .collect();
        for (word, score) in self.lm.top_next(prev, limit) {
            if out.len() >= limit {
                break;
            }
            if !out.iter().any(|(w, _)| *w == word) {
                out.push((word, score));
            }
        }
        out
    }

    /// 上屏以后的联想（会话用）：before 是光标前已上屏的字（末尾是刚上屏的 just），返回最多 limit 个
    /// 常跟在后面的词（词库里的写法）。
    ///
    /// 按上文末尾的词查 [`Engine::predict_next`]：先试最近一次选的词（光标前正好以它结尾），再试末尾 4、3、2、1 个字，
    /// 用第一个查得到的（短的字查出来的多半接不上，不拿来凑数）。去掉标点、符号、单个字母，和刚上屏的一样的。
    pub fn associate(&self, before: &str, just: &str, limit: usize) -> Vec<String> {
        let mut keys: Vec<String> = Vec::new();
        if let Ok(last) = self.last_word.read() {
            if !last.is_empty() && before.ends_with(&self.shown_text(&last)) {
                keys.push(last.clone());
            }
        }
        let chars: Vec<char> = before.chars().collect();
        for n in (1..=chars.len().min(4)).rev() {
            let tail: String = chars[chars.len() - n..].iter().collect();
            if !keys.contains(&tail) {
                keys.push(tail);
            }
        }
        let just_plain = self.lexicon_form(just);
        let nnlm = self.nnlm.read().ok().and_then(|m| m.clone());
        let learned: Vec<String> = self
            .pairs
            .read()
            .map(|p| p.next_words(keys.first().map_or("", String::as_str), now_ms()))
            .unwrap_or_default()
            .into_iter()
            .filter(|(_, heat)| *heat >= learn::MIN_PICK_HEAT)
            .map(|(w, _)| w)
            .collect();
        for key in &keys {
            // 有字级模型时多取一些再按它重排（见 ASSOCIATE_POOL）
            let pool = if nnlm.is_some() { ASSOCIATE_POOL.max(limit) } else { limit.saturating_mul(2) };
            let mut out: Vec<(String, f64)> = Vec::new();
            for (word, score) in self.predict_next(key, pool) {
                if !associable(&word) || word == *key || word == just || word == just_plain {
                    continue;
                }
                if out.iter().any(|(w, _)| *w == word) {
                    continue;
                }
                out.push((word, score));
            }
            if let Some(model) = &nnlm {
                // 词级模型给的是 ln(P(词|前词) / P(词))，按它排偏向罕见的固定搭配（你好 -> 久放、谢谢 -> 抬爱）；
                // 字级模型给的是真正的条件概率：按「光标前的字接这个词的第一个字」重排，学到的搭配照旧在前
                // 先按第一个字的概率粗筛（一次前向，之后每个词只是一次点积），再给留下的算整词的概率
                let context = model.context_tokens(before);
                let mut scored: Vec<(String, f64, bool)> = out
                    .into_iter()
                    .map(|(w, _)| {
                        let first = w.chars().next().unwrap_or(' ');
                        let r = f64::from(model.next_char_logprob(&context, first));
                        let mine = learned.contains(&w);
                        (w, r, mine)
                    })
                    .filter(|(_, r, mine)| *mine || *r >= ASSOCIATE_MIN_LOGPROB)
                    .collect();
                scored.sort_by(|a, b| b.2.cmp(&a.2).then(b.1.total_cmp(&a.1)));
                scored.truncate(ASSOCIATE_RESCORE);
                // 整词的 log 概率按字数的平方根折算：不全按总和（总偏向单字）也不全按平均（总偏向长词）
                for (w, r, _) in scored.iter_mut() {
                    let n = w.chars().count().max(1) as f64;
                    *r = model.score(&context, w) / n.sqrt();
                }
                scored.sort_by(|a, b| b.2.cmp(&a.2).then(b.1.total_cmp(&a.1)));
                out = scored.into_iter().map(|(w, r, _)| (w, r)).collect();
            }
            out.truncate(limit);
            if !out.is_empty() {
                return out.into_iter().map(|(w, _)| w).collect();
            }
        }
        Vec::new()
    }

    pub fn input(&self, raw: &str) -> Vec<Candidate> {
        self.input_at(raw, now_ms())
    }

    /// 带时间戳的输入：历史特征的时间衰减按 at_ms 计算，便于测试与回放。
    pub fn input_at(&self, raw: &str, at_ms: u64) -> Vec<Candidate> {
        self.try_input_at(raw, at_ms).unwrap_or_default()
    }

    /// 可返回错误的版本，供 CLI/FFI 使用。
    pub fn try_input(&self, raw: &str) -> Result<Vec<Candidate>> {
        self.try_input_at(raw, now_ms())
    }

    pub fn try_input_at(&self, raw: &str, at_ms: u64) -> Result<Vec<Candidate>> {
        let mut result = if self.config.shuangpin {
            self.shuangpin_candidates(raw, at_ms)?
        } else {
            self.pinyin_candidates(raw, None, true, None, at_ms)?
        };
        self.prepend_custom_codes(raw, &mut result);
        if let Ok(mut last) = self.last_words.write() {
            last.clear();
            for c in &result {
                if !is_raw_candidate(c) {
                    let words: Vec<String> = c.words.iter().map(|w| w.text.clone()).collect();
                    if self.config.traditional {
                        last.insert(words.concat(), words.clone());
                    }
                    last.insert(c.text.clone(), words);
                }
            }
        }
        // 自定义词里的 \n、\t 到这里才换成真的换行、制表符（词库是一行一条的 TSV）
        for c in &mut result {
            if c.text.contains('\\') {
                c.text = unescape_custom(&c.text);
            }
        }
        Ok(result)
    }

    /// 输入正好是某个自定义编码（mail、wdyx）时，把对应的词放在最前面。
    fn prepend_custom_codes(&self, raw: &str, result: &mut Vec<Candidate>) {
        let words = self.lexicon.custom_code_words(raw.trim());
        if words.is_empty() {
            return;
        }
        result.retain(|c| !words.contains(&c.text));
        let front = words.into_iter().map(|text| Candidate {
            pinyin: String::new(),
            score: 0.0,
            features: Feature::default(),
            words: vec![CandidateWord {
                text: text.clone(),
                pinyin: String::new(),
                freq: crate::lexicon::CUSTOM_WORD_FREQ,
                known: true,
            }],
            consumed: raw.len(),
            text,
        });
        result.splice(0..0, front);
    }

    /// 双拼：先把键翻成全拼（`vsgo` -> `zhong'guo`），再走全拼那一套，
    /// 最后把候选用掉的全拼字节数换回双拼键的字节数。
    fn shuangpin_candidates(&self, raw: &str, at_ms: u64) -> Result<Vec<Candidate>> {
        let display = raw.trim();
        let leading = raw.len() - raw.trim_start().len();
        let scheme = self.config.shuangpin_scheme;
        let variants = crate::shuangpin::decode(scheme, display);
        let Some(first) = variants.first() else {
            // 翻不了（不是双拼）：只给原始串
            let mut result = Vec::new();
            self.append_raw_candidate(display, &mut result);
            for c in &mut result {
                c.consumed = raw.len();
            }
            return Ok(result);
        };
        // 双拼里每个音节固定两键：配成对的音节已经打全，不能再当前缀补成别的音节
        // （lo'ji 的 ji 不是 jiu 的前缀）；只有奇数键末尾悬空的声母才需要补全（vsg -> zhong'g）。
        let completion = display
            .bytes()
            .filter(|&b| scheme.is_key(b.to_ascii_lowercase()))
            .count()
            % 2
            == 1;
        // 有重码（lo / luo）时，各读法的束搜索结果在这里按分数合并，再一起走全拼后面那套按位置排的
        // 步骤（选过的提前、原始串和英文的位置、整句和句首词的穿插、翻页长尾）。不能等排完再按分数
        // 重排或尾追加：重排会把这些位置全打乱，尾追加会让前一组的长尾把候选数填满（#226：loji 里
        // lo 读法硬凑的「咯 | 就」排在 luo 读法打全的「逻辑」前面）。
        let (primary, found) = if variants.len() == 1 {
            (first, None)
        } else {
            let max = self.config.max_candidates.max(1);
            let mut best: Option<(usize, f64)> = None;
            let mut merged = Vec::new();
            for (i, v) in variants.iter().enumerate() {
                let found = self.query_plain(&v.pinyin.to_ascii_lowercase(), completion, at_ms)?;
                let top = found.first().map_or(f64::NEG_INFINITY, |c| c.score);
                if best.is_none_or(|(_, s)| top > s) {
                    best = Some((i, top));
                }
                merged.extend(found);
            }
            let mut merged = dedupe_by_text(merged);
            merged.sort_by(|a, b| {
                b.score
                    .total_cmp(&a.score)
                    .then_with(|| a.text.cmp(&b.text))
            });
            merged.truncate(max);
            // 第一名所在的读法当主读法：句首词、翻页长尾和用掉的键数都按它算
            (&variants[best.map_or(0, |b| b.0)], Some(merged))
        };
        let mut result =
            self.pinyin_candidates(&primary.pinyin, Some(display), completion, found, at_ms)?;
        result.retain_mut(|c| {
            let used = if is_raw_candidate(c) {
                display.len()
            } else {
                primary.raw_bytes(c.consumed, display.len())
            };
            c.consumed = if used >= display.len() {
                raw.len()
            } else {
                leading + used
            };
            used > 0
        });
        Ok(result)
    }

    /// 全拼的完整流程。`raw_display` 给了的话（双拼），原始串候选用它而不是 raw。
    /// `completion` 控制末音节前缀补全：双拼配成对的音节已打全，只有悬空声母才开。
    /// `found` 给了的话（双拼几种读法合并好的束搜索结果）不再查一遍，直接接着排。
    fn pinyin_candidates(
        &self,
        raw: &str,
        raw_display: Option<&str>,
        completion: bool,
        found: Option<Vec<Candidate>>,
        at_ms: u64,
    ) -> Result<Vec<Candidate>> {
        let display = raw.trim();
        let normalized = display.to_ascii_lowercase();
        let mut result = match found {
            Some(found) => found,
            None => self.query_plain(&normalized, completion, at_ms)?,
        };

        protect_exact_input(&mut result);
        promote_picked(&mut result);
        pin_single_initial(&normalized, &mut result);
        self.promote_custom(&mut result);
        self.append_raw_candidate(raw_display.unwrap_or(display), &mut result);
        let english_only = drop_forced_chinese(&mut result);
        if self.config.traditional {
            self.apply_traditional(&mut result);
        }
        for candidate in &mut result {
            candidate.consumed = raw.len();
        }
        if english_only {
            // 打的是英文词（iphone、hello）：硬解出来的中文都去掉了，也不补句首词
            return Ok(result);
        }
        if self.config.prefix_candidates > 0 {
            let leading = raw.len() - raw.trim_start().len();
            let mut prefix = self.prefix_candidates(&normalized)?;
            let _ = prefix_on_syllable_boundaries(&result, &normalized, &mut prefix);
            for candidate in &mut prefix {
                candidate.consumed = leading + letters_to_bytes(display, candidate.consumed);
                if self.config.traditional {
                    if let Ok(conv) = self.converter.read() {
                        if !conv.is_empty() {
                            candidate.text = conv.convert(&candidate.text);
                        }
                    }
                }
            }
            if let Some(head) = sentence_head(&result, &normalized) {
                let mut head = head;
                head.text = self.shown_text(&head.text);
                head.consumed = leading + letters_to_bytes(display, head.consumed);
                prefix.insert(0, head);
            }
            compose_candidates(&mut result, prefix, self.config.max_candidates.max(1));
        }
        if self.config.more_candidates {
            self.append_more_candidates(raw, display, &normalized, completion, &mut result)?;
        }
        Ok(result)
    }

    /// 句首词候选：从输入开头起、但没有用完整个输入的词。
    ///
    /// candidate.consumed 先存「用掉的字母数」，由调用方换算成字节。
    fn prefix_candidates(&self, normalized: &str) -> Result<Vec<Candidate>> {
        let Ok(parsed) = parse(normalized) else {
            return Ok(Vec::new());
        };
        if parsed.len() < 2 {
            return Ok(Vec::new());
        }
        let typo = self.config.typo_correction && !self.config.shuangpin;
        let full = SyllableGraph::build(
            &parsed,
            GraphOptions {
                mode: GraphMode::Full,
                allow_erhua: true,
                typo,
                completion: false,
            },
        );
        let graph = if full.has_exact_path() || !self.config.jianpin_fallback {
            full
        } else {
            let completion_explains = SyllableGraph::build(
                &parsed,
                GraphOptions {
                    mode: GraphMode::Full,
                    allow_erhua: true,
                    typo: false,
                    completion: true,
                },
            )
            .has_path_without_typo();
            abbrev_graph(&parsed, completion_explains).unwrap_or(full)
        };
        let fuzzy_view;
        let lexicon: &dyn Lexicon = if self.config.fuzzy {
            fuzzy_view = FuzzyLexicon::new(
                self.lexicon.as_ref(),
                self.config.fuzzy_groups,
                self.config.fuzzy_penalty,
            );
            &fuzzy_view
        } else {
            self.lexicon.as_ref()
        };
        let words = crate::beam::first_words(
            &graph,
            lexicon,
            &self.config.beam,
            self.config.prefix_candidates,
        );
        Ok(words
            .into_iter()
            .map(|(word, cost)| Candidate {
                text: word.text.clone(),
                pinyin: word.pinyin.clone(),
                score: -cost,
                features: Feature {
                    typo_count: word.typos as f32,
                    typo: if word.typos > 0 { 1.0 } else { 0.0 },
                    ..Feature::default()
                },
                consumed: word.end,
                words: vec![CandidateWord {
                    text: word.text,
                    pinyin: word.pinyin,
                    freq: word.freq,
                    known: true,
                }],
            })
            .collect())
    }

    /// 翻页用的长尾：前面几页之后，把输入能对上的词全补上，直到 `max_candidates`。
    ///
    /// 顺序：整个输入的同音词（按词频）→ 句首词（长的在前）→ 第一个音节的常用单字 →
    /// 换一种切法才对得上的词（houxuanci 的「后续」）→ 第一个音节的生僻字 →
    /// 换一种切法才对得上的单字。前面已经有的不重复；排第一的结果没纠错时不补纠错出来的词。
    ///
    /// 输入框里显示的切法（ling'lian'jie）是用户看着打的那一种：整句不对时接下来多半是
    /// 按这个切法一个字一个字选，所以它的常用字排在换切法的词（ling li | an jie 的「凌厉」）
    /// 前面，换切法的单字放到最后。
    fn append_more_candidates(
        &self,
        raw: &str,
        display: &str,
        normalized: &str,
        completion: bool,
        result: &mut Vec<Candidate>,
    ) -> Result<()> {
        let max = self.config.max_candidates.max(1);
        let Ok(parsed) = parse(normalized) else {
            return Ok(());
        };
        if parsed.is_empty() {
            return Ok(());
        }
        let typo = self.config.typo_correction && !self.config.shuangpin;
        let full = SyllableGraph::build(
            &parsed,
            GraphOptions {
                mode: GraphMode::Full,
                allow_erhua: true,
                typo,
                completion,
            },
        );
        let graph = if full.has_exact_path() || !self.config.jianpin_fallback {
            full
        } else {
            let completion_explains = full.has_path_without_typo();
            abbrev_graph(&parsed, completion_explains).unwrap_or(full)
        };
        let fuzzy_view;
        let lexicon: &dyn Lexicon = if self.config.fuzzy {
            fuzzy_view = FuzzyLexicon::new(
                self.lexicon.as_ref(),
                self.config.fuzzy_groups,
                self.config.fuzzy_penalty,
            );
            &fuzzy_view
        } else {
            self.lexicon.as_ref()
        };
        let words = crate::beam::all_first_words(&graph, lexicon, &self.config.beam, max);
        if words.is_empty() {
            return Ok(());
        }
        let total = graph.len();
        let exact_top = result
            .iter()
            .find(|c| !is_raw_candidate(c))
            .map(|c| c.features.typo_count == 0.0)
            .unwrap_or(false);
        let to_candidate = |word: BeamWord, cost: f64| Candidate {
            text: word.text.clone(),
            pinyin: word.pinyin.clone(),
            score: -cost,
            features: Feature {
                typo_count: word.typos as f32,
                typo: if word.typos > 0 { 1.0 } else { 0.0 },
                ..Feature::default()
            },
            consumed: word.end,
            words: vec![CandidateWord {
                text: word.text,
                pinyin: word.pinyin,
                freq: word.freq,
                known: true,
            }],
        };
        let mut whole = Vec::new();
        let mut homophones = Vec::new();
        let mut heads = Vec::new();
        for (word, cost) in words {
            if exact_top && word.typos > 0 {
                continue;
            }
            if word.end >= total {
                // 打全了、整个输入正好是词库里的一个词（两个音节以上）：同音词
                let exact = word.typos == 0
                    && word.completion == 0
                    && word.abbrev == 0
                    && !word.jianpin
                    && word.syllable_count >= 2;
                if exact && exact_top {
                    homophones.push(to_candidate(word, cost));
                } else {
                    whole.push(to_candidate(word, cost));
                }
            } else if word.completion == 0 {
                heads.push(to_candidate(word, cost));
            }
        }
        // 句首词只留在音节边界上的（不把一个音节拆开）
        let mut other_splits = prefix_on_syllable_boundaries(result, normalized, &mut heads);
        if self.config.shuangpin {
            // 双拼的音节边界由键对定死（ie | il 就是 che | chuang），不能换一种切法：
            // 换切法的「撤出」[che chu] 只用掉 ie 两个键，选了它会多上屏一个字
            other_splits.clear();
        }
        let leading = raw.len() - raw.trim_start().len();
        let mut seen: HashMap<String, ()> = result.iter().map(|c| (c.text.clone(), ())).collect();
        let converter = if self.config.traditional {
            self.converter.read().ok()
        } else {
            None
        };
        // 同音词全部排在句首词和单字前面，接在束搜索给出的整词后面。束搜索只留分数最高的
        // 十来条，名额常被纠错、补全出来的高频词占掉（tongde 的「用的、听得、功德」），
        // 词库里打全了就对得上的词（通的、大创、超梦）反而掉到单字后面，只能一个字一个字选。
        // 两个音节的同音词最多三十来个（按使用频率算 90% 的输入不超过 8 个），不会把单字挤得太远。
        // 原始串/英文候选先拿出来，插完再放回原来的位置（英文词排第 6 的还排第 6）
        let mut raws: Vec<(usize, Candidate)> = Vec::new();
        let mut i = 0;
        while i < result.len() {
            if is_raw_candidate(&result[i]) {
                raws.push((i + raws.len(), result.remove(i)));
            } else {
                i += 1;
            }
        }
        let mut at = result
            .iter()
            .rposition(|c| c.consumed >= raw.len() && c.features.typo_count == 0.0)
            .map_or(0, |i| i + 1);
        // 三个音节以上的同音词还要排在读音一样、几个词拼出来的整句前面（qidongqi：启动器 在 启动 | 其
        // 前面），和 protect_exact_input 对束搜索结果做的一样；用户选过的整句不动。
        // 有音节更少、够常用的整词时（jieran 的「截然」），换切法才对得上的长词（己二胺）不往前提
        let fewest = fewest_exact_syllables(result)
            .into_iter()
            .chain(
                homophones
                    .iter()
                    .filter(|c| c.words[0].freq >= EXACT_PROTECT_MIN_FREQ)
                    .map(|c| c.pinyin.split_whitespace().count()),
            )
            .min();
        for mut candidate in homophones {
            let syllables = candidate.pinyin.split_whitespace().count();
            let long = syllables >= EXACT_LONG_SYLLABLES
                && candidate.words[0].freq >= EXACT_LONG_MIN_FREQ
                && fewest.is_none_or(|n| syllables <= n);
            if let Some(conv) = converter.as_ref().filter(|c| !c.is_empty()) {
                candidate.text = conv.convert(&candidate.text);
            }
            if seen.insert(candidate.text.clone(), ()).is_some() {
                continue;
            }
            candidate.consumed = raw.len();
            let stitched = result[..at].iter().position(|c| {
                long && c.consumed >= raw.len()
                    && c.words.len() >= 2
                    && c.features.history_use_count == 0.0
                    && same_syllables(c, &candidate)
            });
            result.insert(stitched.unwrap_or(at), candidate);
            at += 1;
        }
        for (i, c) in raws {
            let i = i.min(result.len());
            result.insert(i, c);
        }
        // 超出上限时从后面去掉（原始串/英文候选不动）
        while result.len() > max {
            match result.iter().rposition(|c| !is_raw_candidate(c)) {
                Some(i) if result.len() > 1 => {
                    result.remove(i);
                }
                _ => break,
            }
        }
        // 长尾按 TailTier 分层排：层与层之间按层，同一层里保持束搜索给的顺序
        let single = |c: &Candidate| c.text.chars().count() == 1;
        let mut tail: Vec<(TailTier, Candidate)> = whole
            .into_iter()
            .map(|c| (TailTier::Whole, c))
            .collect();
        for c in heads {
            let tier = match (single(&c), is_rare_char(&c)) {
                (false, _) => TailTier::HeadWord,
                (true, false) => TailTier::HeadChar,
                (true, true) => TailTier::RareChar,
            };
            tail.push((tier, c));
        }
        for c in other_splits {
            let tier = if single(&c) {
                TailTier::OtherSplitChar
            } else {
                TailTier::OtherSplitWord
            };
            tail.push((tier, c));
        }
        tail.sort_by_key(|(tier, _)| *tier); // 稳定排序：同一层里的先后不变
        for (_, mut candidate) in tail {
            if result.len() >= max {
                break;
            }
            if let Some(conv) = converter.as_ref().filter(|c| !c.is_empty()) {
                candidate.text = conv.convert(&candidate.text);
            }
            if seen.insert(candidate.text.clone(), ()).is_some() {
                continue;
            }
            candidate.consumed = if candidate.consumed >= total {
                raw.len()
            } else {
                leading + letters_to_bytes(display, candidate.consumed)
            };
            result.push(candidate);
        }
        for i in 1..result.len() {
            if result[i].score >= result[i - 1].score {
                result[i].score = result[i - 1].score - 1e-3;
            }
        }
        Ok(())
    }

    /// 全拼下输入能否不靠纠错、补全切成完整音节。
    fn is_plain_pinyin(&self, display: &str) -> bool {
        if self.config.shuangpin {
            return false;
        }
        let Ok(parsed) = parse(&display.to_ascii_lowercase()) else {
            return false;
        };
        SyllableGraph::build(
            &parsed,
            GraphOptions {
                mode: GraphMode::Full,
                allow_erhua: true,
                typo: false,
                completion: false,
            },
        )
        .has_complete_path()
    }

    /// 英文/未登录输入：保证原始输入本身可以作为候选上屏。
    ///
    /// 放在第几位：
    /// - 拼音完整、没纠错的中文排前面，原始串垫底（she -> 社 设 ……）；
    /// - 输入是已知英文词，而中文要靠纠错才凑得出来：英文排第一
    ///   （hello、github、python 不该被改成乱码中文）；中文只差最后一个音节没打完
    ///   （bug、diy）时排第二；拼音本身就完整（she、long）时排第六；
    /// - 不认识的串：中文全是未登录兜底时排第一，中文纠了 2 处以上排第二，其余垫底。
    ///
    /// 原始串一定留在列表里，不会被截断挤掉。
    fn append_raw_candidate(&self, display: &str, candidates: &mut Vec<Candidate>) {
        if !self.config.english_candidate || display.is_empty() || display.len() > 32 {
            return;
        }
        if !display.bytes().any(|b| b.is_ascii_alphabetic()) {
            return;
        }
        // 全小写输入且词表里有标准写法时用标准写法（github -> GitHub）
        let english = self
            .english
            .read()
            .ok()
            .and_then(|m| m.get(&display.to_ascii_lowercase()).cloned());
        let text = match &english {
            Some(word) if display.bytes().all(|b| !b.is_ascii_uppercase()) => word.clone(),
            _ => display.to_string(),
        };
        if candidates
            .iter()
            .any(|c| c.text.eq_ignore_ascii_case(&text))
        {
            return;
        }
        // 输入本身是不是一串完整的拼音（she、me、long），或者已经有不靠纠错和
        // 补全的完整中文候选（双拼下只能看后者）
        // 全拼混简拼（he l l o -> 合理了哦）是硬凑出来的，不算
        let exact = self.is_plain_pinyin(display)
            || candidates.iter().any(|c| {
                c.features.coverage_score >= 1.0
                    && c.features.typo_count == 0.0
                    && c.features.prefix_completion == 0.0
                    && !c.words.iter().any(|w| !w.known)
                    && !is_mixed_jianpin(c)
            });
        // 第一个中文候选在中间就有只打了声母的音节（hello -> he l l o）：
        // 不像是还在打拼音，英文排第一
        let inner_abbrev = candidates
            .first()
            .is_some_and(|c| is_mixed_jianpin(c) && abbrev_before_last(c));
        // 打了大写（Hello）却只能凑成混简拼（he l l o -> 合理了哦）：要的是原样上屏
        let literal = display.bytes().any(|b| b.is_ascii_uppercase())
            && candidates.first().is_some_and(is_mixed_jianpin);
        let top_typos = candidates
            .first()
            .map(|c| c.features.typo_count)
            .unwrap_or(0.0);
        let has_known_word = candidates
            .first()
            .map(|c| c.words.iter().any(|w| w.known))
            .unwrap_or(false);
        // 第一名是几个词凑的整句（ok -> 哦 | 可）：不是「一个词还没打完」
        let top_sentence = candidates.first().is_some_and(is_sentence);
        let position = if english.is_some() {
            if exact {
                // 本身就是拼音的英文词（fan、long、she）：前 5 个留给中文
                Some(5)
            } else if top_typos == 0.0 && has_known_word && !inner_abbrev && !top_sentence {
                // 中文只是最后一个音节没打完（bug -> 不过、diy -> 第一），
                // 用户多半还在打拼音，英文排第二
                Some(1)
            } else {
                Some(0)
            }
        } else if exact {
            // 拼音完整的中文，原始拼音串没必要出现
            return;
        } else if !has_known_word {
            Some(0)
        } else if literal {
            Some(0)
        } else if top_typos >= 2.0 {
            Some(1)
        } else {
            None
        };
        let max = self.config.max_candidates.max(1);
        let at = position.unwrap_or(candidates.len()).min(candidates.len());
        // 分数夹在前后两个候选之间，后面无论怎么重新排序（繁体转换会重排）位置都不变
        let score = match (
            at.checked_sub(1).and_then(|i| candidates.get(i)),
            candidates.get(at),
        ) {
            (None, None) => -1.0,
            (None, Some(next)) => next.score + 1.0,
            (Some(prev), None) => prev.score - 1.0,
            (Some(prev), Some(next)) => (prev.score + next.score) / 2.0,
        };
        let raw = Candidate {
            text,
            pinyin: display.to_ascii_lowercase(),
            score,
            features: Feature::default(),
            words: vec![CandidateWord {
                text: display.to_string(),
                pinyin: String::new(),
                freq: 0.0,
                known: english.is_some(),
            }],
            consumed: display.len(),
        };
        candidates.insert(at, raw);
        if candidates.len() > max {
            // 截掉的是原始串以外的最后一个
            let drop = if at == candidates.len() - 1 {
                candidates.len() - 2
            } else {
                candidates.len() - 1
            };
            candidates.remove(drop);
        }
    }

    /// 单条全拼输入的完整查询（精确 + 可选模糊音）。
    ///
    /// 模糊音视图里精确读音和模糊读音都有（模糊词扣 `fuzzy_penalty`），一次搜索就够了，
    /// 两种读音的词按各自的分数排在一起。
    fn query_plain(
        &self,
        normalized: &str,
        completion: bool,
        at_ms: u64,
    ) -> Result<Vec<Candidate>> {
        self.run_query(normalized, at_ms, self.config.fuzzy, completion)
    }

    /// 跑一次查询；fuzzy=true 时精确读音和模糊音读音的词一起搜。
    ///
    /// 末音节补全（zhongg -> 中国、zhonggu -> 中国）和纠错都做成了音节图里的边，
    /// 一次搜索就能同时给出精确、补全、纠错三类候选。
    fn run_query(
        &self,
        normalized: &str,
        at_ms: u64,
        fuzzy: bool,
        completion: bool,
    ) -> Result<Vec<Candidate>> {
        let parsed = parse(normalized)?;
        if parsed.is_empty() {
            return Ok(Vec::new());
        }
        let typo = self.config.typo_correction && !self.config.shuangpin;
        let full = SyllableGraph::build(
            &parsed,
            GraphOptions {
                mode: GraphMode::Full,
                allow_erhua: true,
                typo,
                completion,
            },
        );
        // 不靠补全、纠错就能完整解析：直接用全拼。
        // 但能切成音节不等于词库里有词：hm、hng 这类叹词音节词库不收，以前只给出原样的 hm，
        // 打 hm（后面、好吗）一个中文候选都没有。一个认识的词都凑不出来就接着试简拼。
        if full.has_exact_path() {
            let mut found = self.query_once(&full, at_ms, fuzzy)?;
            self.keep_exact_syllable(&full, &mut found, at_ms, fuzzy)?;
            if !fuzzy && self.config.fuzzy_fallback && !self.config.shuangpin {
                self.fuzzy_fallback(&full, &mut found);
            }
            let any_known = found.iter().any(|c| !c.words.is_empty() && c.words.iter().all(|w| w.known));
            if !self.config.jianpin_fallback || any_known {
                return Ok(found);
            }
        }
        // 否则简拼（zg、jzm）、全拼混简拼（kyi、yongzwen）与全拼补全/纠错都试，按分数合并
        let mut merged: Vec<Candidate> = Vec::new();
        if self.config.jianpin_fallback {
            if let Some(jianpin) = abbrev_graph(&parsed, full.has_path_without_typo()) {
                let mut found = self.query_once(&jianpin, at_ms, fuzzy)?;
                if jianpin.mode() == GraphMode::Mixed {
                    // 混简拼里拿声母、单个字母兜底凑出来的（h e l l o -> "hello"）没有意义
                    found.retain(|c| c.words.iter().all(|w| w.known));
                }
                merged.extend(found);
            }
        }
        if full.has_complete_path() {
            merged.extend(self.query_once(&full, at_ms, fuzzy)?);
        }
        let mut result = dedupe_by_text(merged);
        result.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then_with(|| a.text.cmp(&b.text))
        });
        result.truncate(self.config.max_candidates.max(1));
        Ok(result)
    }

    /// 输入正好是一个完整音节，但束搜索的名额全被补全出来的字占了、本音节的字一个也没留下时
    /// （lia：lian、liang、liao 的字几百个，「俩」只有词频 99，以前排第 60 个；dia 的「嗲」也是），
    /// 只按这个音节（不补全、不纠错）再搜一遍，把本音节不是生僻字的字补进来，
    /// 再照 [`exact_syllable_first`] 排在补全出来的字前面（和 den 的「扽」、nou 的「耨」一样）。
    ///
    /// 束搜索里已经有本音节的字时什么都不做，多花的只是很少见的这一种输入上的一次单音节搜索。
    fn keep_exact_syllable(
        &self,
        graph: &SyllableGraph,
        found: &mut Vec<Candidate>,
        at_ms: u64,
        fuzzy: bool,
    ) -> Result<()> {
        let n = graph.len();
        let one_syllable = graph.edges_from(0).iter().any(|e| {
            e.end == n
                && !e.typo
                && e.completion == 0
                && matches!(e.kind, EdgeKind::Full { erhua: false, .. })
        });
        let exact_char = |c: &Candidate| {
            c.features.typo_count == 0.0
                && c.features.prefix_completion == 0.0
                && c.words.len() == 1
                && c.words[0].known
                && !c.pinyin.contains(' ')
        };
        if !one_syllable || found.iter().any(exact_char) {
            return Ok(());
        }
        let exact = SyllableGraph::build(
            graph.input(),
            GraphOptions {
                mode: GraphMode::Full,
                allow_erhua: false,
                typo: false,
                completion: false,
            },
        );
        let mut extra: Vec<Candidate> = self
            .query_once(&exact, at_ms, fuzzy)?
            .into_iter()
            .filter(|c| exact_char(c) && !is_rare_char(c))
            .filter(|c| !found.iter().any(|f| f.text == c.text))
            .collect();
        if extra.is_empty() {
            return Ok(());
        }
        // 放在最前面，分数抬到刚好高过原来的第一名：原来那些候选之间的分差不动
        // （compose_candidates 按分差判断硬凑的整句）；不再整体走 exact_syllable_first，
        // 它会把换切法凑出来的整句（li | a 的「理啊」）也当成本音节的提到前面
        if let Some(top) = found.first().map(|c| c.score) {
            let k = extra.len();
            for (i, c) in extra.iter_mut().enumerate() {
                c.score = top + 1e-3 * (k - i) as f64;
            }
        }
        found.splice(0..0, extra);
        found.truncate(self.config.max_candidates.max(1));
        Ok(())
    }

    /// 模糊音关着时的兜底：精确读音的第一名偏弱（几个词硬凑的、纠错或补全出来的），
    /// 而输入按模糊音读是个常用词（yingai -> 应该、shengme -> 什么、zidao -> 知道、
    /// penyou -> 朋友）时，把这个词（最多两个）加进候选，比第一名高出
    /// [`FUZZY_FALLBACK_LEAD`] 才加。
    ///
    /// 只查 2–4 个音节的精确切分，每种切分一次模糊查表，不再跑一遍束搜索。
    fn fuzzy_fallback(&self, graph: &SyllableGraph, found: &mut Vec<Candidate>) {
        let weak = found.first().is_none_or(|c| {
            c.words.len() >= 2
                || c.features.typo_count > 0.0
                || c.features.prefix_completion > 0.0
                || c.words.iter().any(|w| !w.known)
        });
        let syllables = found
            .first()
            .map_or(0, |c| c.pinyin.split_whitespace().count());
        if !weak || !(2..=4).contains(&syllables) {
            return;
        }
        let top = found.first().map_or(f64::NEG_INFINITY, |c| c.score);
        let view = FuzzyLexicon::new(
            self.lexicon.as_ref(),
            self.config.fuzzy_groups,
            self.config.fuzzy_penalty,
        );
        // 接着前文打时第一个词不在句首，和精确候选一样不加句首分
        let mid_sentence = !self.context().trim().is_empty();
        let mut segmentations = Vec::new();
        exact_segmentations(graph, 0, 4, &mut Vec::new(), &mut segmentations);
        let mut extra: Vec<Candidate> = Vec::new();
        for segmentation in segmentations.iter().filter(|s| (2..=4).contains(&s.len())) {
            let query: Vec<SyllableQuery> = segmentation.iter().map(|e| e.query()).collect();
            let exact = self.lexicon.lookup_common(&query);
            let pinyin = query.iter().map(|q| q.display()).collect::<Vec<_>>().join(" ");
            for entry in view.lookup_common(&query) {
                // 精确读音也有这个词、而且更常用：不是靠模糊音才有的
                if exact.iter().any(|e| e.word == entry.word && e.freq >= entry.freq) {
                    continue;
                }
                if found.iter().chain(&extra).any(|c| c.text == entry.word) {
                    continue;
                }
                let lm = if mid_sentence { 0.0 } else { self.lm.score(None, &entry.word) };
                let feature = Feature {
                    coverage_score: 1.0,
                    lm_score_with_previous_txt: lm as f32,
                    ..Feature::default()
                };
                let score = -view.cost(&entry)
                    + self.config.beam.lm_weight * lm
                    + self.ranker.score(&feature) as f64;
                if score < top + FUZZY_FALLBACK_LEAD {
                    continue;
                }
                extra.push(Candidate {
                    text: entry.word.clone(),
                    pinyin: pinyin.clone(),
                    score,
                    features: feature,
                    words: vec![CandidateWord {
                        text: entry.word.clone(),
                        pinyin: pinyin.clone(),
                        freq: entry.freq,
                        known: true,
                    }],
                    consumed: 0,
                });
            }
        }
        if extra.is_empty() {
            return;
        }
        extra.sort_by(|a, b| b.score.total_cmp(&a.score));
        extra.truncate(2);
        found.extend(extra);
        found.sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| a.text.cmp(&b.text)));
        found.truncate(self.config.max_candidates.max(1));
    }

    fn query_once(&self, graph: &SyllableGraph, at_ms: u64, fuzzy: bool) -> Result<Vec<Candidate>> {
        // fuzzy 时用包装视图：精确读音之外再加上模糊音读音的词。
        let fuzzy_view;
        let lexicon: &dyn Lexicon = if fuzzy {
            fuzzy_view = FuzzyLexicon::new(
                self.lexicon.as_ref(),
                self.config.fuzzy_groups,
                self.config.fuzzy_penalty,
            );
            &fuzzy_view
        } else {
            self.lexicon.as_ref()
        };
        let start = self.start_word();
        let pairs = self
            .pairs
            .read()
            .map_err(|_| anyhow::anyhow!("pairs lock poisoned"))?;
        // 接着前文打：第一个词不在句首，不加句首分（上文搭配由排序特征 context_bigram_score 管）
        let mid_sentence = !self.context().trim().is_empty();
        let lm = LearnedLm {
            base: self.lm.as_ref(),
            pairs: &pairs,
            start: start.as_deref(),
            at_ms,
            mid_sentence,
        };
        let mut paths = beam_search(graph, lexicon, &lm, &self.config.beam);
        drop(pairs);
        if paths.is_empty() {
            return Ok(Vec::new());
        }

        // 有完整覆盖路径时只保留完整覆盖，避免 西an / xi安 这类半拼音候选。
        // 否则退而求其次，保留有部分词典命中的路径；全未登录时保留兜底路径。
        if paths.iter().any(|p| p.coverage >= 1.0) {
            paths.retain(|p| p.coverage >= 1.0);
        } else if paths.iter().any(|p| p.coverage > 0.0) {
            paths.retain(|p| p.coverage > 0.0);
        }

        let best_score = paths
            .iter()
            .map(|p| p.score)
            .fold(f64::NEG_INFINITY, f64::max);

        let nnlm = self.nnlm_scores(&paths);
        let candidates: Vec<Candidate> = paths
            .into_iter()
            .zip(nnlm)
            .map(|(path, nnlm)| self.make_candidate(path, best_score, at_ms, nnlm))
            .collect();

        let mut deduped = dedupe_by_text(candidates);
        deduped.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then_with(|| a.text.cmp(&b.text))
        });
        let mut deduped = exact_syllable_first(deduped);
        deduped.truncate(self.config.max_candidates.max(1));
        Ok(deduped)
    }

    /// 每条路径的神经语言模型特征：log P(整句 | 光标前的文本) 减去这批里最好的那条，≤ 0。
    ///
    /// 只算束搜索分数最高的 `nnlm_rerank` 条（模型一步约 1 ms，同一前缀会缓存）；
    /// 其余的给比算过的最差那条还低一点的值。没有模型时全是 0。
    fn nnlm_scores(&self, paths: &[BeamPath]) -> Vec<f32> {
        let mut out = vec![0.0f32; paths.len()];
        let limit = self.config.nnlm_rerank;
        if limit == 0 || paths.len() < 2 {
            return out;
        }
        let Some(model) = self.nnlm.read().ok().and_then(|m| m.clone()) else {
            return out;
        };
        let mut order: Vec<usize> = (0..paths.len()).collect();
        order.sort_by(|&a, &b| paths[b].score.total_cmp(&paths[a].score));
        // 没有上文时只给长输入重排：单独一个词（kexue、jieran）时字级模型只会比较「哪串字常见」，
        // 盖过词库对整词的判断；几个词连起来顺不顺才是它擅长的。见 NNLM_MIN_SYLLABLES
        let syllables: usize = paths[order[0]]
            .words
            .iter()
            .map(|w| w.pinyin.split_whitespace().count())
            .sum();
        if syllables < NNLM_MIN_SYLLABLES && self.context().trim().is_empty() {
            return out;
        }
        // 长输入里一半以上的音节是只打了声母、纠错出来或词库不认识的（sdsjdsdsd、asdfghjkl 这类乱按）：
        // 束搜索的整句是硬凑的，字级模型挑哪串更顺也没有意义，每键却要多算几十个字（#227）。
        // 短输入（带上文时也会重排）照算，代价小
        if syllables >= NNLM_MIN_SYLLABLES && weak_syllables(&paths[order[0]]) * 2 > syllables {
            return out;
        }
        let picked = nnlm_picks(paths, &order, limit);
        let context = model.context_tokens(&self.context());
        let scored: Vec<(usize, f64)> = picked
            .iter()
            .map(|&i| (i, model.score(&context, &paths[i].text)))
            .collect();
        let best = scored.iter().map(|s| s.1).fold(f64::NEG_INFINITY, f64::max);
        let worst = scored.iter().map(|s| s.1).fold(f64::INFINITY, f64::min);
        for &(i, s) in &scored {
            out[i] = (s - best).max(-NNLM_FLOOR) as f32;
        }
        let rest = ((worst - best) - 2.0).max(-NNLM_FLOOR) as f32;
        for &i in order.iter().skip(picked.len()) {
            out[i] = rest;
        }
        out
    }

    /// 词级语言模型下，光标前的文字接这个词的分数（上文排序特征）。
    ///
    /// 上文末尾 1–4 个字里，模型认识的每个后缀都当一次前一个词，取最高分。以前只试「整段上文」
    /// 和「最后一个字」：整段上文模型不认识时得 0，反倒把负分抹平了（「今天我」碰巧正常、「我」不正常）。
    ///
    /// 「后缀 + 这个词」本身是模型里的一个词时（我 + 要 = 我要、不 + 知道 = 不知道），
    /// 语料分词时它们被并成了一个词，这个搭配从没被计过数，模型给的是「没见过」的负分，
    /// 还不如偶然见过几次的「我腰」「我悔」。这时按见过的常用搭配算，至少给 [`MERGED_PAIR_SCORE`]。
    fn word_lm_context_score(&self, context: &str, word: &str) -> f32 {
        let chars: Vec<char> = context.chars().collect();
        let merged_floor = MERGED_PAIR_SCORE;
        let mut best: Option<f64> = None;
        for n in 1..=chars.len().min(4) {
            let prev: String = chars[chars.len() - n..].iter().collect();
            if self.lm.knows_word(&prev) == Some(true) {
                let s = self.lm.score(Some(&prev), word);
                best = Some(best.map_or(s, |b| b.max(s)));
            }
            if self.lm.knows_word(&format!("{prev}{word}")) == Some(true) {
                best = Some(best.map_or(merged_floor, |b| b.max(merged_floor)));
            }
        }
        best.unwrap_or(0.0) as f32
    }

    fn make_candidate(&self, path: BeamPath, best_score: f64, at_ms: u64, nnlm: f32) -> Candidate {
        let mut feature = self.make_feature(&path, best_score, at_ms);
        feature.head_word_nnlm = nnlm;
        let rank_score = self.ranker.score(&feature) as f64;
        // 单个音节打全时只有一个字：句首 LM 分反映的是语料里谁常出现在句首，
        // 没有上下文时不该压过字频（否则 he 会先出「何」）。
        // 只减基础模型的句首分：path.lm_score 里还有用户学过的搭配加成，学过的字要照样靠前。
        let single_word = path.words.len() == 1 && path.words[0].syllable_count == 1;
        // 接着前文打时句首分本来就没加（见 run_query 的 mid_sentence），不用再减
        let mid_sentence = !self.context().trim().is_empty();
        let score = if single_word && !mid_sentence {
            let start = self.lm.score(None, &path.words[0].text);
            path.score - self.config.beam.lm_weight * start + rank_score
        } else {
            path.score + rank_score
        };
        Candidate {
            text: path.text,
            pinyin: path.pinyin,
            score,
            features: feature,
            words: path
                .words
                .into_iter()
                .map(|w| CandidateWord {
                    text: w.text,
                    pinyin: w.pinyin,
                    freq: w.freq,
                    known: w.known,
                })
                .collect(),
            consumed: 0,
        }
    }

    fn make_feature(&self, path: &BeamPath, best_score: f64, at_ms: u64) -> Feature {
        // 历史：整句命中与末词命中都算。
        //
        // 排序用热度（30 天半衰期衰减后的次数，取 ln(1+热度)），见 [`learn`]。
        // 末词只算多字词，而且算一半：选过「种地」，「我想种地」也该靠前；
        // 选过「你」不能把所有以「你」结尾的整句都抬上来。
        let history_hot_val = self
            .history
            .read()
            .ok()
            .map(|history| {
                let exact = history
                    .get(&path.text)
                    .map(|e| crate::learn::decayed(*e, at_ms))
                    .unwrap_or(0.0);
                let tail = path
                    .words
                    .last()
                    .filter(|w| path.words.len() >= 2 && w.text.chars().count() >= 2)
                    .and_then(|w| history.get(&w.text))
                    .map(|e| crate::learn::decayed(*e, at_ms))
                    .unwrap_or(0.0);
                ((1.0 + exact + 0.5 * tail).ln() as f32, exact as f32)
            })
            .unwrap_or((0.0, 0.0));
        let (history_hot_val, picked) = history_hot_val;
        let (history_count, history_time_diff) = self
            .history
            .read()
            .ok()
            .map(|history| {
                let exact = history.get(&path.text).copied();
                let tail = path
                    .words
                    .last()
                    .and_then(|w| history.get(&w.text))
                    .copied();
                let merged = match (exact, tail) {
                    (Some(a), Some(b)) if b.count > a.count => b,
                    (Some(a), _) => a,
                    (None, Some(b)) => b,
                    (None, None) => HistoryEntry {
                        count: 0.0,
                        last_used_ms: at_ms,
                    },
                };
                let seconds = at_ms.saturating_sub(merged.last_used_ms) as f64 / 1000.0;
                (merged.count as f32, (1.0 + seconds).ln() as f32)
            })
            .unwrap_or((0.0, 0.0));

        // 上下文：用候选首词去查二元分。
        //
        // 取键要同时试「上一个词」和「上一个字」：词级 LM 用前者，
        // 字级 LM（例如从语料统计的字二元）用后者。中文上下文通常没有空格，
        // 只取 split_whitespace().last() 会把整段文本当成一个词，查不到任何 bigram。
        let (with_context, context_bigram_score) = self
            .context
            .read()
            .map(|context| {
                let context = context.trim();
                let Some(candidate) = path.words.first() else {
                    return (0.0, 0.0);
                };
                if context.is_empty() {
                    return (0.0, 0.0);
                }
                if self.lm.knows_word(&candidate.text).is_some() {
                    return (1.0, self.word_lm_context_score(context, &candidate.text));
                }
                let mut best: Option<f32> = None;
                let mut consider = |prev: &str| {
                    if prev.is_empty() {
                        return;
                    }
                    let score = self.lm.score(Some(prev), &candidate.text) as f32;
                    best = Some(best.map_or(score, |b: f32| b.max(score)));
                };
                if let Some(word) = context.split_whitespace().last() {
                    consider(word);
                }
                let mut buf = [0u8; 4];
                if let Some(ch) = context.chars().last() {
                    consider(ch.encode_utf8(&mut buf));
                }
                (1.0, best.unwrap_or(0.0))
            })
            .unwrap_or((0.0, 0.0));

        Feature {
            kind: if path.words.len() <= 1 { 0.0 } else { 1.0 },
            prob: (path.score - best_score).exp().clamp(0.0, 1.0) as f32,
            coverage_score: path.coverage,
            lm_score_with_previous_txt: path.lm_score as f32,
            full_jianpin: if path.jianpin { 1.0 } else { 0.0 },
            ltr_valid: if self.ranker.has_ltr() { 1.0 } else { 0.0 },
            history_count,
            // 这个候选本身（整个文本）被选过的热度，不含末词命中
            history_use_count: picked,
            history_time_diff,
            history_hot_val,
            with_context,
            context_bigram_score,
            typo_count: path.typos as f32,
            prefix_completion: (path.completion as f32).min(3.0),
            typo: if path.typos > 0 { 1.0 } else { 0.0 },
            // 用户词等特征还没有数据源，保持 0；NNLM 在 make_candidate 里填。
            ..Feature::default()
        }
    }
}

/// 输入正好是一个完整音节（xian、ji、zhe）时：本音节的字排在补全出来的字（想、就、真）前面，
/// 纠错出来的（gei → 被、飞；si → 是，漏按 h）再往后。
///
/// 补全每个字母只罚 1.4 左右，高频字补全以后常常比本音节的字分还高；已经打全的音节，
/// 用户要的是这个音节的字。多音节输入、没打全的音节不动。
fn exact_syllable_first(candidates: Vec<Candidate>) -> Vec<Candidate> {
    let exact = |c: &Candidate| c.features.typo_count == 0.0 && c.features.prefix_completion == 0.0;
    let one_syllable = candidates
        .iter()
        .any(|c| exact(c) && c.words.len() == 1 && c.words[0].known && !c.pinyin.contains(' '));
    if !one_syllable {
        return candidates;
    }
    let (mut out, rest): (Vec<Candidate>, Vec<Candidate>) = candidates.into_iter().partition(exact);
    lift_split_word(&mut out);
    // 补全的在前、纠错的在后，各自按分数
    let (completions, typos): (Vec<Candidate>, Vec<Candidate>) =
        rest.into_iter().partition(|c| c.features.typo_count == 0.0);
    // 分数也压到前面的下面，后面按分数合并、排序的地方不会再把它们换上来
    for c in completions.into_iter().chain(typos) {
        let floor = out.last().map_or(0.0, |last| last.score);
        out.push(Candidate {
            score: c.score.min(floor - 1e-3),
            ..c
        });
    }
    out
}

/// 一个音节也能切成两个音节的整词（xian → 西安）：分数和第一名差得不多时，至少排到第 [`SPLIT_WORD_SLOT`] 位。
/// 单字太多，按分数它常常排在十几位以后，第一页看不到。
fn lift_split_word(exact: &mut Vec<Candidate>) {
    let Some(top) = exact.first().map(|c| c.score) else {
        return;
    };
    let found = exact.iter().position(|c| {
        c.words.len() == 1 && c.pinyin.contains(' ') && top - c.score <= SPLIT_WORD_MARGIN
    });
    let Some(at) = found.filter(|&at| at > SPLIT_WORD_SLOT) else {
        return;
    };
    let mut word = exact.remove(at);
    // 分数夹在前后两个之间，后面再按分数排也不会变
    let (above, below) = (exact[SPLIT_WORD_SLOT - 1].score, exact[SPLIT_WORD_SLOT].score);
    word.score = (above + below) / 2.0;
    exact.insert(SPLIT_WORD_SLOT, word);
}

/// [`lift_split_word`]：最多和第一名差多少（自然对数）。实测 xian 的「西安」差 3.0，
/// 别的音节能切出来的整词（提案、彼岸、立案、堤岸）差 4 以上，不抬。
const SPLIT_WORD_MARGIN: f64 = 3.5;
/// [`lift_split_word`]：抬到第几位（从 0 数，4 是第五个）。
const SPLIT_WORD_SLOT: usize = 4;

/// 第 k 个字母之后在原始串里的字节位置（跳过紧随其后的隔音符）。
fn letters_to_bytes(raw: &str, letters: usize) -> usize {
    let bytes = raw.as_bytes();
    let mut seen = 0usize;
    let mut idx = 0usize;
    while idx < bytes.len() && seen < letters {
        if bytes[idx].is_ascii_alphabetic() {
            seen += 1;
        }
        idx += 1;
    }
    while idx < bytes.len() && bytes[idx] == b'\'' {
        idx += 1;
    }
    idx
}

/// 把句首词候选并入整句候选。
///
/// 整句是一个词时（shijie -> 世界）句首词放在最后；整句由多个词拼成时
/// （长句）第二名往后的整句只是小改动，价值不如句首词，所以句首词紧跟第一名。
/// 词频低于这个数的词不受「精确优先」保护（rime-ice 里约排在 20 万名以后，多是生僻或凑出来的条目）。
const EXACT_PROTECT_MIN_FREQ: f64 = 1000.0;
/// 整个输入是一个词、音节数到这个数时，「精确优先」不看词频，几个词拼出来的候选也排它后面。
const EXACT_LONG_SYLLABLES: usize = 3;
/// 上面这条规则要的最低词频：只有个位数词频的条目（赤锦腕 1、大李窑 3）多是人名地名，
/// 撞上常用词拼出来的整句（吃 | 今晚、搭理 | 要）时不该抢第一；它们照样在第一页的同音词里。
/// 占位词频是 100，手工补的冷门词（发一版 65）也在这条线以上。
const EXACT_LONG_MIN_FREQ: f64 = 10.0;
/// 没有上文时，输入至少这么多个音节才用字级神经语言模型重排（有上文时都用）。
/// scripts/eval.sh 实测（WowType 自己的 nnlm.bin，权重 0.5）：门槛 0 时日常整句 +2.2、上文 +4.9，
/// 但常用词 -3.5、简拼 -6.7、截断 -8.7；门槛 4 时冷门词还掉 1.6；门槛 5 时单独打词的各组和不带模型时一样，
/// 日常整句 90.6% -> 94.2%、上文 89.1% -> 94.0%。
const NNLM_MIN_SYLLABLES: usize = 5;
/// 神经语言模型特征最低到多少（自然对数），免得一条特别离谱的把分数拉得太开。
const NNLM_FLOOR: f64 = 20.0;
/// 一次重排最多让神经语言模型算多少个字（几条路径按公共前缀合并后数），给每键的开销定个上限（约 0.1 ms 一个字）。
/// 实测日常整句逐键打（tools/eval/daily.tsv 的 138 句、3114 键）一次最多 59 个字、平均 20 个，
/// 上限 24 时排序评测也一样；只有特别长、几条路径又各说各的输入会被截。
const NNLM_MAX_CHARS: usize = 64;

/// [`Engine::nnlm_scores`] 要让模型打分的路径：按束搜索分数从高到低最多 `limit` 条，
/// 要算的字数（几条路径按公共前缀合并后数，同一前缀模型只算一次）到 [`NNLM_MAX_CHARS`] 为止，
/// 至少两条才有得比。返回的是 `order` 的前几个。
fn nnlm_picks(paths: &[BeamPath], order: &[usize], limit: usize) -> Vec<usize> {
    let mut picked: Vec<usize> = Vec::with_capacity(limit);
    let mut texts: Vec<Vec<char>> = Vec::with_capacity(limit);
    let mut chars = 0usize;
    for &i in order.iter().take(limit) {
        let text: Vec<char> = paths[i].text.chars().collect();
        let shared = texts
            .iter()
            .map(|t| t.iter().zip(&text).take_while(|(a, b)| a == b).count())
            .max()
            .unwrap_or(0);
        let cost = text.len() - shared;
        if picked.len() >= 2 && chars + cost > NNLM_MAX_CHARS {
            break;
        }
        chars += cost;
        picked.push(i);
        texts.push(text);
    }
    picked
}

/// 一条路径里「不实在」的音节数：只打了声母的音节，加上纠错出来的、词库不认识的词的全部音节。
/// [`Engine::nnlm_scores`] 用它跳过乱按出来的长输入。
fn weak_syllables(path: &BeamPath) -> usize {
    path.words
        .iter()
        .map(|w| {
            if w.typos > 0 || !w.known {
                w.pinyin.split_whitespace().count().max(1)
            } else {
                w.abbrev
            }
        })
        .sum()
}

/// 拼音打全了、而且正好是一个实在的词时，这个词必须排第一。
///
/// 补全和纠错的分数里带着词频，高频词补出来以后可能比用户打全了的低频词还高：
/// `zhongdi` 会出「重点」（zhong dian）而不是「种地」，`ka` 出「看」，`beiji` 出「北京」。
/// 用户按空格上屏的是第一个——打到一半时补全排第二只是多按一个键，
/// 打全了却被换成别的词就是打错字。所以排在这个词前面的补全/纠错候选都挪到它后面。
///
/// 同理，整个输入就是一个词时，几个单字硬凑出来的候选（`shidan` 拆成「是 | 但」）
/// 也不该压过它（「实弹」）。
///
/// 「实在的词」：精确匹配（不靠纠错和补全）、要么整个输入是一个词，要么最后一个词
/// 至少两个字（这时只和「前面的词一样、只有最后一个词不同」的候选比）；每个词的词频不低于 [`EXACT_PROTECT_MIN_FREQ`]，或者是用户选过的。
/// 最后是个单字的精确结果（`zhonggu` 拆成「中 | 骨」）不算，用户多半还没打完，
/// 补全照常可以排第一。
fn protect_exact_input(candidates: &mut Vec<Candidate>) {
    let multi = |w: &CandidateWord| w.pinyin.split_whitespace().count() >= 2;
    let is_exact = |c: &Candidate| {
        c.features.coverage_score >= 1.0
            && c.features.typo_count == 0.0
            && c.features.prefix_completion == 0.0
            && !c.words.is_empty()
            && c.words.iter().all(|w| w.known)
    };
    // 三个音节以上、整个输入正好是词库里的一个词：读音这么长还能对上，同音的词库词平均不到两个，
    // 不看词频也算实在的词（前后端、启动器、候选词在 rime-ice 里都只有占位的词频）
    // 切法得是最自然的那种：有个音节更少、够常用的精确整词时（jieran 的「截然」jie ran），
    // 换一种切法才对得上的长词（己二胺 ji er an）不算
    let fewest = fewest_exact_syllables(candidates);
    let long_word = |c: &Candidate| {
        let syllables = c.pinyin.split_whitespace().count();
        c.words.len() == 1
            && c.words[0].freq >= EXACT_LONG_MIN_FREQ
            && syllables >= EXACT_LONG_SYLLABLES
            && fewest.is_none_or(|n| syllables <= n)
    };
    let solid = |c: &Candidate| {
        is_exact(c)
            && (c.words.len() == 1 || c.words.last().map(multi).unwrap_or(false))
            && (c.features.history_count > 0.0
                || long_word(c)
                || c.words.iter().all(|w| w.freq >= EXACT_PROTECT_MIN_FREQ))
    };
    // 长的整词先提到拼出来的整句前面：fayiban 里 法 | 一般 本身也算「实在的」（最后一个词两个字），
    // 下面按第一个实在的候选当锚点时轮不到「发一版」。用户选过的整句不动
    // 只压过读音完全一样的拼凑（启动 | 其 = qi dong qi）；分数只抬到刚好高过它，后面的不动
    // （整段压成等差会让 compose_candidates 按分差去掉硬凑整句的判断失效）
    if let Some(word_at) = candidates.iter().position(|c| is_exact(c) && long_word(c)) {
        let stitched = candidates[..word_at].iter().position(|c| {
            c.words.len() >= 2
                && c.features.history_use_count == 0.0
                && same_syllables(c, &candidates[word_at])
        });
        if let Some(at) = stitched {
            let mut word = candidates.remove(word_at);
            word.score = word.score.max(candidates[at].score + 1e-3);
            candidates.insert(at, word);
            for i in 1..candidates.len() {
                if candidates[i].score > candidates[i - 1].score {
                    candidates[i].score = candidates[i - 1].score - 1e-3;
                }
            }
        }
    }
    let Some(anchor) = candidates.iter().position(solid) else {
        return;
    };
    if anchor == 0 {
        return;
    }
    let whole_word = candidates[anchor].words.len() == 1;
    let long_anchor = long_word(&candidates[anchor]);
    let anchor_pinyin = candidates[anchor].pinyin.clone();
    // 精确结果是几个词拼的（我在 | 种地）时，只管开头一样、只是后面换了词的拼凑结果
    // （我 | 在重点）；一个整词盖住全部输入的（从善如流 vs 从善 | 入理）是更强的证据，不动
    let head_text: String = {
        let words = &candidates[anchor].words;
        words[..words.len() - 1]
            .iter()
            .map(|w| w.text.as_str())
            .collect()
    };
    let same_head =
        |c: &Candidate| whole_word || (c.words.len() >= 2 && c.text.starts_with(&head_text));
    let yields = |c: &Candidate| {
        let corrected = c.features.typo_count > 0.0 || c.features.prefix_completion > 0.0;
        // 长的整词前面，几个词拼出来的都让位（启动 | 其、前后 | 断）；短的整词只压过全是单字拼的
        (corrected && same_head(c))
            || (whole_word
                && c.words.len() >= 2
                && ((long_anchor && c.pinyin.split_whitespace().eq(anchor_pinyin.split_whitespace()))
                    || !c.words.iter().any(multi)))
    };
    if !candidates[..anchor].iter().any(yields) {
        return;
    }
    let mut ahead = Vec::new();
    let mut keep = Vec::with_capacity(candidates.len());
    for (i, c) in candidates.drain(..).enumerate() {
        if i < anchor && yields(&c) {
            ahead.push(c);
        } else {
            keep.push(c);
        }
    }
    let at = keep
        .iter()
        .position(solid)
        .map(|i| i + 1)
        .unwrap_or(keep.len());
    keep.splice(at..at, ahead);
    // 分数改成单调递减，后面再按分数重排（繁体转换）顺序也不变
    for i in 1..keep.len() {
        if keep[i].score >= keep[i - 1].score {
            keep[i].score = keep[i - 1].score - 1e-3;
        }
    }
    *candidates = keep;
}

/// 两个候选的音节一样（整词和拼凑出来的整句：启动器 / 启动 | 其）。
fn same_syllables(a: &Candidate, b: &Candidate) -> bool {
    a.pinyin.split_whitespace().eq(b.pinyin.split_whitespace())
}

/// 不靠纠错、补全、整个输入是一个够常用（[`EXACT_PROTECT_MIN_FREQ`]）的词库词的候选里，最少几个音节。
fn fewest_exact_syllables(candidates: &[Candidate]) -> Option<usize> {
    candidates
        .iter()
        .filter(|c| {
            c.words.len() == 1
                && c.words[0].known
                && c.words[0].freq >= EXACT_PROTECT_MIN_FREQ
                && c.features.typo_count == 0.0
                && c.features.prefix_completion == 0.0
                && c.features.coverage_score >= 1.0
        })
        .map(|c| c.pinyin.split_whitespace().count())
        .min()
}

/// 用户在设置里加的自定义词排最前（语言模型不认识它们，光靠词频排不上来）。
impl Engine {
    fn promote_custom(&self, candidates: &mut [Candidate]) {
        let custom = |c: &Candidate| c.words.len() == 1 && self.lexicon.is_custom(&c.text);
        if candidates.iter().any(custom) {
            // 稳定排序：自定义词之间、其余候选之间都保持原来的顺序
            candidates.sort_by_key(|c| !custom(c));
        }
    }
}

/// 自定义词的转义：`\n` 换行、`\t` 制表符、`\\` 反斜杠，其余原样。
pub fn unescape_custom(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// 只打一个声母（l、b、zh）时固定排第一的字：词库按读音分的词频把它们排得太后
/// （了 le 348 万，来 1484 万；不 299 万，把 380 万），而且第一个候选不该随上文、选过的字变来变去
/// （#156）。没列的声母照词频排。
const SINGLE_INITIAL_TOP: &[(&str, &str)] = &[
    ("b", "不"),
    ("d", "的"),
    ("g", "个"),
    ("h", "和"),
    ("j", "就"),
    ("l", "了"),
    ("n", "你"),
    ("q", "去"),
    ("r", "人"),
    ("s", "是"),
    ("sh", "是"),
    ("t", "他"),
    ("w", "我"),
    ("y", "一"),
    ("z", "在"),
    ("zh", "这"),
];

/// 输入只是一个声母时，把 [`SINGLE_INITIAL_TOP`] 里的字挪到第一个（在选过最多的字前面；
/// 自定义词、自定义编码之后还会排到它前面）。候选里没有这个字就不动。
fn pin_single_initial(input: &str, candidates: &mut Vec<Candidate>) {
    let Some(&(_, top)) = SINGLE_INITIAL_TOP.iter().find(|(key, _)| *key == input) else {
        return;
    };
    if let Some(i) = candidates
        .iter()
        .position(|c| c.text == top && c.words.len() == 1)
    {
        let pinned = candidates.remove(i);
        candidates.insert(0, pinned);
    }
}

/// 用户上次给这串输入选过的候选往前放（微信输入法的 FirstUserCandidateRanker /
/// SingleUserWordCanbeSecond）。
///
/// - 词和整句：选得最多的那个放第一（第一个候选被选得更多时不动）。
/// - 单字：第一个候选不动——偶尔选一次生僻字，不该让它以后一直顶掉这个音节最常用的字；
///   选过的字按热度从第二位起排（#161：找过一次的字下次就在第一页，不只提选得最多的那一个），
///   最多 [`PICKED_CHARS_MAX`] 个，免得几次手滑把常用字挤到第二页。
///   两样都有时（xian 选过「西安」也选过「先」）词照样放第一，字跟在后面。
///
/// 只看「这个候选本身被选过」（不算末词命中），而且要精确匹配（不是纠错、补全出来的）。
/// 热度（`history_use_count`，衰减过的）不到 [`learn::MIN_PICK_HEAT`] 的不算：
/// 误选一次的词一个月以后就不再一直顶在前面。
fn promote_picked(candidates: &mut Vec<Candidate>) {
    let single = |c: &Candidate| c.text.chars().count() <= 1;
    let heat = |c: &Candidate| c.features.history_use_count;
    let mut changed = false;
    // 词和整句：选得最多的放第一（一样多时取靠前的）
    let best_word = candidates
        .iter()
        .enumerate()
        .filter(|(_, c)| !single(c) && is_picked(c))
        .max_by(|a, b| heat(a.1).total_cmp(&heat(b.1)).then_with(|| b.0.cmp(&a.0)))
        .map(|(i, _)| i);
    if let Some(best) = best_word.filter(|&i| i > 0) {
        // 第一个候选被选得更多（比如常选的单字），就不动它
        if heat(&candidates[0]) < heat(&candidates[best]) {
            let c = candidates.remove(best);
            candidates.insert(0, c);
            changed = true;
        }
    }
    // 单字：第一个候选之后，按热度排选过的字，最多 PICKED_CHARS_MAX 个
    let mut picked: Vec<usize> = (1..candidates.len())
        .filter(|&i| single(&candidates[i]) && is_picked(&candidates[i]))
        .collect();
    // 稳定排序：热度一样时保持原来（按词频）的先后
    picked.sort_by(|&a, &b| heat(&candidates[b]).total_cmp(&heat(&candidates[a])));
    picked.truncate(PICKED_CHARS_MAX);
    if !picked.is_empty() {
        let moved: Vec<Candidate> = picked.iter().map(|&i| candidates[i].clone()).collect();
        let mut rest = Vec::with_capacity(candidates.len());
        for (i, c) in std::mem::take(candidates).into_iter().enumerate() {
            if !picked.contains(&i) {
                rest.push(c);
            }
        }
        let at = rest.len().min(1);
        rest.splice(at..at, moved);
        *candidates = rest;
        changed = true;
    }
    if changed {
        make_scores_monotonic(candidates);
    }
}

/// [`promote_picked`]：选过的单字最多提前几个。
const PICKED_CHARS_MAX: usize = 3;

/// 这个候选本身被精确选过（不是靠末词命中、纠错或补全出来的），而且热度够
/// [`learn::MIN_PICK_HEAT`]（误选一次、过了一个月就不算）。
fn is_picked(c: &Candidate) -> bool {
    f64::from(c.features.history_use_count) >= learn::MIN_PICK_HEAT
        && c.features.typo_count == 0.0
        && c.features.prefix_completion == 0.0
        && c.features.full_jianpin == 0.0
}

/// 分数改成单调递减，后面再按分数重排（繁体转换）顺序也不变。
fn make_scores_monotonic(candidates: &mut [Candidate]) {
    for i in 1..candidates.len() {
        if candidates[i].score >= candidates[i - 1].score {
            candidates[i].score = candidates[i - 1].score - 1e-3;
        }
    }
}

/// 句首词只在多音节输入里有意义，而且只能在音节边界上切。
///
/// 单音节的 `dan` 不该出「大」（da + 剩个 n），`xian` 不该出「下」：
/// 那是把一个打完的音节拆开了，只会把第一页塞满没用的字。
/// 第一个整句候选是精确全拼时，句首词必须正好用完它的前几个音节；
/// 是简拼或纠错结果时没法对齐字母，只要求它至少两个音节。
///
/// 返回被筛掉、但换一种切法也说得通的词：词本身是完整全拼，剩下的字母也能整个切成
/// 音节（houxuanci 里的「后续」= hou xu | an ci）。它们不该挤进前几页，长尾里补上
/// （位置见 [`Engine::append_more_candidates`]）。
fn prefix_on_syllable_boundaries(
    result: &[Candidate],
    normalized: &str,
    prefix: &mut Vec<Candidate>,
) -> Vec<Candidate> {
    let Some(top) = result.iter().find(|c| !is_raw_candidate(c)) else {
        prefix.clear();
        return Vec::new();
    };
    // 儿化的音节（wanr）按「音节 + r」两段算：第一名是「玩儿」时，wan 的单字（玩、万）照样是句首词
    let syllables: Vec<&str> = top
        .pinyin
        .split_whitespace()
        .flat_map(|s| match s.strip_suffix('r') {
            Some(base) if !base.is_empty() && s != "er" => vec![base, "r"],
            _ => vec![s],
        })
        .collect();
    if syllables.len() < 2 {
        prefix.clear();
        return Vec::new();
    }
    let letters: String = normalized
        .chars()
        .filter(|c| c.is_ascii_alphabetic())
        .collect();
    if top.features.full_jianpin > 0.0 || syllables.concat().len() > letters.len() + 2 {
        // 简拼（zgrm）字母和音节对不上，不按边界筛
        return Vec::new();
    }
    // 从头对齐：补全和纠错只改动对不上的那一段，前面对得上的音节边界照样可信
    let mut boundaries = Vec::new();
    let mut at = 0;
    for syllable in &syllables[..syllables.len() - 1] {
        if !letters[at..].starts_with(syllable) {
            break;
        }
        at += syllable.len();
        boundaries.push(at);
    }
    let exact = top.features.typo_count == 0.0;
    let mut other_splits = Vec::new();
    // consumed 此时是字母数（音节图里的位置，不含隔音符）
    prefix.retain(|p| {
        let used = p.consumed;
        if exact && p.features.typo_count > 0.0 {
            return false;
        }
        if boundaries.contains(&used) {
            return true;
        }
        if exact
            && used < letters.len()
            && p.pinyin.split_whitespace().collect::<String>() == letters[..used]
            && splits_into_syllables(&letters[used..])
        {
            other_splits.push(p.clone());
        }
        false
    });
    // 整句候选本身的开头（你好中国 -> 你好）最可能是对的，越长越省事，排最前
    let head = |p: &Candidate| top.text.starts_with(&p.text);
    prefix.sort_by_key(|p| std::cmp::Reverse(if head(p) { p.text.chars().count() } else { 0 }));
    // 长的在前（同样长的保持束搜索给的顺序）
    other_splits.sort_by_key(|p| std::cmp::Reverse(p.text.chars().count()));
    other_splits
}

/// 这串字母能不能整个切成合法的全拼音节（an ci、ci）。
///
/// 没有元音的音节（ng、hm、hng，「嗯」「噷」「哼」）不算：linglianjie 切成 li | ng lian jie
/// 没人会这么打，算进去的话 li 的几百个单字都成了「换一种切法」的候选。
fn splits_into_syllables(letters: &str) -> bool {
    let voiced = |s: &str| s.bytes().any(|b| b"aeiouv".contains(&b));
    let mut ok = vec![false; letters.len() + 1];
    ok[0] = true;
    for end in 1..=letters.len() {
        for len in 1..=6.min(end) {
            let syllable = &letters[end - len..end];
            if ok[end - len] && voiced(syllable) && is_valid(syllable) {
                ok[end] = true;
                break;
            }
        }
    }
    ok[letters.len()]
}

/// 全拼对不上时用的简拼图；走不通时返回 None。
///
/// - 全是声母（zg、jzm）：纯简拼，和以前一样；
/// - 有韵母（kyi、yongzwen）：全拼混简拼，每个音节可以打全也可以只打声母。
///   全拼加补全已经能解释时（打到一半的 yanzhon）不用：结果一样，混简拼的路径却多得多，
///   长句逐键会从几毫秒涨到几十毫秒。`completion_explains` 是「全拼加补全、不靠纠错能走完」。
fn abbrev_graph(
    parsed: &crate::syllable::ParsedInput,
    completion_explains: bool,
) -> Option<SyllableGraph> {
    let all_initials = parsed
        .letters
        .iter()
        .all(|&b| crate::syllable::is_jianpin_letter(b));
    if !all_initials && completion_explains {
        return None;
    }
    let graph = SyllableGraph::build(
        parsed,
        GraphOptions {
            mode: if all_initials {
                GraphMode::Jianpin
            } else {
                GraphMode::Mixed
            },
            allow_erhua: false,
            typo: false,
            completion: false,
        },
    );
    graph.has_complete_path().then_some(graph)
}

/// 翻页长尾（[`Engine::append_more_candidates`] 补在前几页后面的）分几层，按这里的先后排。
///
/// 整套候选的层次（从前往后）：束搜索的整句 / 整词（[`protect_exact_input`]、[`compose_candidates`] 定先后，
/// 用户选过的提前），整个输入对得上的词库同音词（插在束搜索的整词后面），句首候选，然后是这里的长尾。
/// 输入框里显示的切法是用户看着打的那一种：整句不对时接下来多半是按它一个字一个字选，所以它的常用字排在
/// 换一种切法才对得上的词（ling li | an jie 的「凌厉」）前面。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum TailTier {
    /// 用完整个输入、束搜索没留下的（纠错、补全出来的整词等）
    Whole,
    /// 按显示的切法、从开头起的多字词
    HeadWord,
    /// 按显示的切法、第一个音节的常用字
    HeadChar,
    /// 换一种切法才对得上的多字词（houxuanci 的「后续」= hou xu | an ci）
    OtherSplitWord,
    /// 第一个音节的生僻字（频次小于 1，见 [`crate::lexicon::LexEntry::is_rare`]）
    RareChar,
    /// 换一种切法才对得上的单字
    OtherSplitChar,
}

/// 长尾里的生僻字（[`crate::lexicon::LexEntry::is_rare`]：频次不到 1 的单字）。
fn is_rare_char(c: &Candidate) -> bool {
    c.words.len() == 1 && crate::lexicon::is_rare_char(&c.words[0].text, c.words[0].freq)
}

/// 全拼和只打声母的音节混着的候选（k yi -> 可以，yong z wen -> 用中文）。
fn is_mixed_jianpin(c: &Candidate) -> bool {
    c.features.full_jianpin > 0.0
        && c.pinyin
            .split_whitespace()
            .any(|s| !crate::syllable::is_abbrev_syllable(s))
}

/// 最后一个音节之前就有只打了声母的音节。
fn abbrev_before_last(c: &Candidate) -> bool {
    let syllables: Vec<&str> = c.pinyin.split_whitespace().collect();
    syllables.len() >= 2
        && syllables[..syllables.len() - 1]
            .iter()
            .any(|s| crate::syllable::is_abbrev_syllable(s))
}

/// 原始串 / 英文候选（append_raw_candidate 插进来的）。
fn is_raw_candidate(c: &Candidate) -> bool {
    c.words.len() == 1 && c.words[0].pinyin.is_empty() && c.features.coverage_score == 0.0
}

/// 整个输入是不是只有一个音节、两个音节……（按第一个非原始串候选的拼音数）。
fn syllable_count(result: &[Candidate]) -> usize {
    result
        .iter()
        .find(|c| !is_raw_candidate(c))
        .map(|c| c.pinyin.split_whitespace().count())
        .unwrap_or(0)
}

/// 整句候选：几个词拼起来的、用完整个输入的候选。
fn is_sentence(c: &Candidate) -> bool {
    c.words.len() >= 2 && !is_raw_candidate(c)
}

/// 两个候选只差最后一个字（横向候选词 / 横向候选此）。
///
/// 这种候选没必要单独占位置：用户选整句的开头（横向候选），再选最后一个字就行。
fn differs_only_at_tail(a: &str, b: &str) -> bool {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    a.len() == b.len() && a.len() >= 2 && a[..a.len() - 1] == b[..b.len() - 1]
}

/// 整句候选最多留几个：越长的输入越可能要换个整句，短输入的「整句」多半是两个单字的拼凑。
fn sentence_cap(syllables: usize) -> usize {
    match syllables {
        0..=2 => 1,
        3 => 2,
        _ => 3,
    }
}

/// 第一个整句候选去掉最后一个词（横向候选词 -> 横向候选），作为句首候选。
///
/// 整句只是最后一个词错了时，选它再选最后一个词，比翻页找整句快。
/// 只给精确全拼的整句，开头至少两个音节（一个音节的开头本来就在句首词里）。
/// consumed 先存「用掉的字母数」（不含隔音符），由调用方换算成字节。
fn sentence_head(result: &[Candidate], normalized: &str) -> Option<Candidate> {
    let top = result.iter().find(|c| !is_raw_candidate(c))?;
    if !is_sentence(top)
        || top.features.typo_count > 0.0
        || top.features.prefix_completion > 0.0
        || top.features.full_jianpin > 0.0
    {
        return None;
    }
    let head_words = &top.words[..top.words.len() - 1];
    let syllables: Vec<&str> = head_words
        .iter()
        .flat_map(|w| w.pinyin.split_whitespace())
        .collect();
    if syllables.len() < 2 || head_words.iter().any(|w| !w.known) {
        return None;
    }
    // 拼音要和输入从头对得上，才能知道切在第几个字母
    let letters: String = normalized
        .chars()
        .filter(|c| c.is_ascii_alphabetic())
        .collect();
    let joined = syllables.concat();
    if !letters.starts_with(&joined) {
        return None;
    }
    // 用分词（词库里的写法）拼出前半截，不按字数切整句：繁体词组转换可能改变字数
    // （计算机 -> 電腦）。开着繁体时由调用方再转一次
    Some(Candidate {
        text: head_words.iter().map(|w| w.text.as_str()).collect(),
        pinyin: syllables.join(" "),
        score: top.score,
        features: Feature::default(),
        words: head_words.to_vec(),
        // 字母数，不含隔音符：调用方用 letters_to_bytes 换算，那边只数字母。
        // 以前这里存的是带隔音符的字符数，换算时每个隔音符多吃掉后面一个字母
        // （xi'an'shi'ji'chang 选「西安市」后只剩 chang；双拼解码后音节间都有隔音符，长句都会中）
        consumed: joined.len(),
    })
}

/// 把整句候选和句首候选排成最终列表，总数不超过 max（原始串/英文候选不会被截掉，位置不变）。
///
/// 照着微信输入法的做法：整句只留几个，而且只差最后一个字的整句不留；
/// 然后是整个输入正好是一个词的候选（同音词），再是句首候选——长的在前、单字在后。
/// 长输入（4 个音节以上）的第一个整句常常不全对，最好的句首候选放第二位。
fn compose_candidates(result: &mut Vec<Candidate>, prefix: Vec<Candidate>, max: usize) {
    let syllables = syllable_count(result);
    let cap = sentence_cap(syllables);
    // 原始串记下原来的位置，排完再插回去
    let raw: Vec<(usize, Candidate)> = result
        .iter()
        .enumerate()
        .filter(|(_, c)| is_raw_candidate(c))
        .map(|(i, c)| (i, c.clone()))
        .collect();
    let whole: Vec<Candidate> = std::mem::take(result)
        .into_iter()
        .filter(|c| !is_raw_candidate(c))
        .collect();

    let mut kept: Vec<Candidate> = Vec::new();
    let mut sentences = 0usize;
    // 第一名是打全了的、词库里的整词（你好；纠错出来的也算：zhognguo -> 中国）时，比它差
    // JUNK_SENTENCE_MARGIN 以上的整句（泥 | 好、点 | 脑、钟 | 国）是硬凑的，不占位置；用户选过的照留
    let top_word = whole
        .first()
        .filter(|c| {
            c.words.len() == 1 && c.words[0].known && c.features.prefix_completion == 0.0
        })
        .map(|c| c.score);
    for c in whole {
        if is_sentence(&c) {
            let junk = top_word.is_some_and(|top| top - c.score > JUNK_SENTENCE_MARGIN);
            if junk && c.features.history_use_count == 0.0 {
                continue;
            }
            let first = kept.is_empty();
            if !first
                && (sentences >= cap || kept.iter().any(|k| differs_only_at_tail(&k.text, &c.text)))
            {
                continue;
            }
            sentences += 1;
        }
        kept.push(c);
    }

    let mut seen: HashMap<String, ()> = HashMap::new();
    let prefix: Vec<Candidate> = prefix
        .into_iter()
        .filter(|p| !kept.iter().any(|r| r.text == p.text))
        .filter(|p| seen.insert(p.text.clone(), ()).is_none())
        .collect();

    // 排第一的没纠错时，纠错出来的整句/整词放到句首词后面：`bujianyi` 打的就是
    // 不建议，「古剑一」（b 当 g）不该排在「不见」前面
    let exact_first = kept.first().is_some_and(|c| c.features.typo <= 0.0);
    let (mut kept, mut corrected): (Vec<Candidate>, Vec<Candidate>) = if exact_first {
        kept.into_iter().partition(|c| c.features.typo <= 0.0)
    } else {
        (kept, Vec::new())
    };
    // 但第一名只是几个单字硬凑的整句（tianqu -> 天 | 去），而纠一处错就是个词（天气）时，
    // 这个词放第二位：打的多半是这个词，只是按错了一个键
    let weak_top = kept.first().is_some_and(|c| {
        is_sentence(c)
            && c.features.history_use_count == 0.0
            && c.words.iter().all(|w| w.pinyin.split_whitespace().count() == 1)
    });
    if weak_top {
        let top = kept[0].score;
        let found = corrected.iter().position(|c| {
            c.words.len() == 1 && c.features.typo_count == 1.0 && top - c.score <= TYPO_WORD_GAP
        });
        if let Some(i) = found {
            let word = corrected.remove(i);
            kept.insert(1, word);
        }
    }
    let long_sentence = syllables >= 4 && kept.first().map(is_sentence).unwrap_or(false);
    let mut merged: Vec<Candidate> =
        Vec::with_capacity(kept.len() + prefix.len() + corrected.len());
    let mut prefix = prefix.into_iter();
    let mut kept = kept.into_iter();
    if long_sentence {
        merged.extend(kept.by_ref().take(1));
        merged.extend(prefix.by_ref().take(1));
    }
    merged.extend(kept);
    merged.extend(prefix);
    merged.extend(corrected);
    merged.truncate(max.saturating_sub(raw.len()).max(1));
    for (at, c) in raw {
        let at = at.min(merged.len());
        merged.insert(at, c);
    }
    // 分数改成单调递减，后面再按分数重排（繁体转换）顺序也不变
    for i in 1..merged.len() {
        if merged[i].score >= merged[i - 1].score {
            merged[i].score = merged[i - 1].score - 1e-3;
        }
    }
    *result = merged;
}

/// [`compose_candidates`]：整句比排第一的整词差多少（自然对数）就算硬凑的。
/// 实测 你好 −6.6 vs 泥 | 好 −15.5，电脑 −7.6 vs 点 | 脑 −14.8；整句评测不受影响。
const JUNK_SENTENCE_MARGIN: f64 = 3.0;

/// [`compose_candidates`]：纠一处错得到的词最多比单字硬凑的第一名差多少，还能排第二。
/// 实测 tianqu：天 | 去 −11.1，天气（纠一处）−15.9，差 4.8。
const TYPO_WORD_GAP: f64 = 6.5;

/// 第一个候选是英文词表里的词（iPhone、hello）时，去掉硬解出来的中文：两处纠错的（屁 | 好呢）、
/// 中间夹着只打了声母的音节的（合理[he l] | 了哦）。返回是不是只剩英文了。
fn drop_forced_chinese(result: &mut Vec<Candidate>) -> bool {
    let english_first = result.len() > 1
        && result
            .first()
            .is_some_and(|c| is_raw_candidate(c) && c.words[0].known);
    if !english_first {
        return false;
    }
    result.retain(|c| {
        is_raw_candidate(c)
            || !(c.features.typo_count >= 2.0 || (is_mixed_jianpin(c) && abbrev_before_last(c)))
    });
    result.len() == 1
}

/// [`Engine::fuzzy_fallback`]：模糊音读出的词要比精确读音的第一名高出多少（自然对数）才给出来。
const FUZZY_FALLBACK_LEAD: f64 = 2.0;

/// 只走精确全拼边（不纠错、不补全、不带儿化）、最多 depth 个音节的切法，最多 8 种。
fn exact_segmentations(
    graph: &SyllableGraph,
    pos: usize,
    depth: usize,
    path: &mut Vec<crate::graph::Edge>,
    out: &mut Vec<Vec<crate::graph::Edge>>,
) {
    if pos == graph.len() {
        out.push(path.clone());
        return;
    }
    if depth == 0 || out.len() >= 8 {
        return;
    }
    for edge in graph.edges_from(pos) {
        let plain = matches!(edge.kind, crate::graph::EdgeKind::Full { erhua: false, .. });
        if edge.typo || edge.completion > 0 || !plain {
            continue;
        }
        path.push(*edge);
        exact_segmentations(graph, edge.end, depth - 1, path, out);
        path.pop();
    }
}

fn dedupe_by_text(candidates: Vec<Candidate>) -> Vec<Candidate> {
    let mut best: HashMap<String, Candidate> = HashMap::new();
    for candidate in candidates {
        match best.entry(candidate.text.clone()) {
            Entry::Vacant(slot) => {
                slot.insert(candidate);
            }
            Entry::Occupied(mut slot) => {
                if candidate.score > slot.get().score {
                    slot.insert(candidate);
                }
            }
        }
    }
    best.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexicon::MemoryLexicon;
    use crate::lm::BigramLm;

    fn engine(tsv: &str) -> Engine {
        let lexicon: Arc<dyn Lexicon> = Arc::new(MemoryLexicon::from_tsv_str(tsv).unwrap());
        let lm: Arc<dyn LanguageModel> = Arc::new(BigramLm::default());
        Engine::new(lexicon, lm)
    }

    fn default_engine() -> Engine {
        engine(concat!(
            "xian\t先\t1000\n",
            "xi an\t西安\t500\n",
            "xi\t西\t400\n",
            "an\t安\t300\n",
            "zhong guo\t中国\t800\n",
            "ni hao\t你好\t100000\n",
            "hua r\t花儿\t100\n",
        ))
    }

    #[test]
    fn history_boosts_selection() {
        let e = engine("shi\t是\t900\nshi\t事\t800\n");
        let first = e.input_at("shi", 1_000_000);
        assert_eq!(first[0].text, "是", "默认按频次");

        for i in 0..5 {
            e.record_selection_at("事", 1_000_000 + i);
        }
        assert_eq!(e.history_count("事"), 5);

        let after = e.input_at("shi", 1_000_100);
        assert_eq!(
            after[0].text,
            "事",
            "选过多次的词应当排到前面: {:?}",
            after.iter().map(|c| c.text.as_str()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn all_picked_chars_follow_the_first_candidate() {
        let e = engine(concat!(
            "ma\t吗\t100000\n",
            "ma\t麻\t50000\n",
            "ma\t骂\t1\n",
            "ma\t马\t1\n",
            "ma\t妈\t1\n",
        ));
        assert_eq!(
            e.input_at("ma", 1_000_000)[0].text,
            "吗",
            "没选过时按词频"
        );
        // 粤语用户常选的几个生僻字各选几次：都要排到第一页，不只提最常选的那个
        e.record_selection_at("马", 1_000_001);
        e.record_selection_at("妈", 1_000_002);
        e.record_selection_at("妈", 1_000_003);
        e.record_selection_at("骂", 1_000_004);
        e.record_selection_at("骂", 1_000_005);
        e.record_selection_at("骂", 1_000_006);
        let after = e.input_at("ma", 1_000_100);
        let texts: Vec<&str> = after.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(
            &texts[..4],
            &["吗", "骂", "妈", "马"],
            "选过的字按次数排在第一个候选后面: {texts:?}"
        );
    }

    #[test]
    fn picked_chars_are_capped_and_decay() {
        let e = engine(concat!(
            "ma\t吗\t100000\n",
            "ma\t麻\t50000\n",
            "ma\t嘛\t40000\n",
            "ma\t骂\t1\n",
            "ma\t马\t1\n",
            "ma\t妈\t1\n",
            "ma\t蚂\t1\n",
            "ma\t玛\t1\n",
        ));
        let day = 24 * 3600 * 1000;
        // 五个生僻字各选几次：只提热度最高的三个，常用的「麻」「嘛」不被挤到后面
        for (i, (ch, n)) in [("骂", 5), ("马", 4), ("妈", 3), ("蚂", 2), ("玛", 2)]
            .into_iter()
            .enumerate()
        {
            for k in 0..n {
                e.record_selection_at(ch, (i * 10 + k) as u64);
            }
        }
        let texts: Vec<String> = e.input_at("ma", 100).into_iter().map(|c| c.text).collect();
        assert_eq!(&texts[..6], &["吗", "骂", "马", "妈", "麻", "嘛"], "{texts:?}");
        // 误选一次的字：一个半月以后热度不到半次，不再提前
        let e = engine("ma\t吗\t100000\nma\t麻\t50000\nma\t嘛\t40000\nma\t骂\t1\n");
        e.record_selection_at("骂", 0);
        let texts: Vec<String> = e.input_at("ma", day).into_iter().map(|c| c.text).collect();
        assert_eq!(texts[1], "骂", "刚选过的提前: {texts:?}");
        let texts: Vec<String> = e.input_at("ma", 45 * day).into_iter().map(|c| c.text).collect();
        assert_eq!(&texts[..3], &["吗", "麻", "嘛"], "一个半月以后回到原位: {texts:?}");
    }

    #[test]
    fn picked_word_still_first_when_a_char_is_picked_more() {
        // 「先」最常用，选过的「鲜」（选得比「西安」多）排第二，选过的词「西安」照样排第一
        let e = engine("xian\t先\t100000\nxian\t线\t50000\nxi an\t西安\t500\nxian\t鲜\t1\n");
        for t in 0..3 {
            e.record_selection_at("鲜", t);
        }
        e.record_selection_at("西安", 10);
        e.record_selection_at("西安", 11);
        let texts: Vec<String> = e.input_at("xian", 100).into_iter().map(|c| c.text).collect();
        assert_eq!(&texts[..3], &["西安", "鲜", "先"], "{texts:?}");
    }

    #[test]
    fn old_selection_still_ranks_above_unused_word() {
        // 以前的历史特征按 ln(1+秒) 扣分：一天前选过的词反而排在没选过的后面
        let e = engine("shi\t是\t900\nshi\t事\t800\n");
        e.record_selection_at("事", 0);
        e.record_selection_at("事", 1);
        let day = 24 * 3600 * 1000;
        let after = e.input_at("shi", day);
        assert_eq!(
            after[0].text,
            "事",
            "{:?}",
            after.iter().map(|c| &c.text).collect::<Vec<_>>()
        );
        // 半年不用就退回去
        let later = e.input_at("shi", 180 * day);
        assert_eq!(later[0].text, "是");
    }

    #[test]
    fn custom_words_rank_first_and_reload_replaces() {
        let e = engine(concat!(
            "ni hao\t你好\t332885\n",
            "zhang san\t张三\t20000\n",
            "zhang\t张\t500000\n",
            "san\t三\t500000\n",
        ));
        e.lexicon()
            .set_custom_tsv("# 注释\nni hao\t妮好\nwo de you xiang\t我的邮箱\nbad xx\t坏\n");
        assert_eq!(e.lexicon().custom_words().len(), 2, "不合法的拼音跳过");
        assert_eq!(e.input_at("nihao", 1)[0].text, "妮好");
        assert_eq!(e.input_at("wodeyouxiang", 1)[0].text, "我的邮箱");
        // 自定义词不进用户词库，保存时不会写出去
        assert!(e.user_words().is_empty());

        e.lexicon().set_custom_tsv("zhang san\t章三\n");
        assert_eq!(e.input_at("nihao", 1)[0].text, "你好", "旧的自定义词去掉了");
        assert_eq!(e.input_at("zhangsan", 1)[0].text, "章三");
        e.lexicon().set_custom_tsv("");
        assert_eq!(e.input_at("zhangsan", 1)[0].text, "张三");
    }

    #[test]
    fn learned_pair_fixes_homophone_inside_sentence() {
        let e = engine(concat!(
            "wo men\t我们\t900000\n",
            "zai\t在\t900000\n",
            "zai\t再\t300000\n",
            "tao lun\t讨论\t50000\n",
        ));
        let first = e.input_at("womenzaitaolun", 10);
        assert_eq!(first[0].text, "我们在讨论", "没学之前按词频");
        // 用户一段一段选出「我们再讨论」：我们 -> 再 -> 讨论
        e.set_context("");
        e.record_selection_at("我们", 10);
        e.set_context("我们");
        e.input_at("zaitaolun", 10);
        e.record_selection_at("再", 11);
        e.set_context("我们再");
        e.input_at("taolun", 11);
        e.record_selection_at("讨论", 12);
        e.set_context("");

        let after = e.input_at("womenzaitaolun", 20);
        assert_eq!(
            after[0].text,
            "我们再讨论",
            "{:?}",
            after.iter().map(|c| &c.text).collect::<Vec<_>>()
        );
    }

    #[test]
    fn previous_commit_is_context_for_next_word() {
        let e = engine("zai\t在\t900000\nzai\t再\t300000\nhui tou\t回头\t1000\nhui\t回\t1000\n");
        // 上一次上屏的是「回头」，这次打 zai：学过「回头 -> 再」以后排第一
        e.input_at("huitou", 1);
        e.record_selection_at("回头", 1);
        e.set_context("回头");
        e.input_at("zai", 1);
        e.record_selection_at("再", 2);

        e.set_context("回头");
        e.input_at("huitou", 3);
        e.record_selection_at("回头", 3);
        let with_context = e.input_at("zai", 4);
        assert_eq!(with_context[0].text, "再");

        // 光标前不是「回头」时不受影响（「再」本身选过一次，热度还在，所以换个更远的时间）
        e.set_context("别的");
        let without = e.input_at("zai", 4 + 365 * 24 * 3600 * 1000);
        assert_eq!(without[0].text, "在");
    }

    #[test]
    fn history_survives_save_and_load() {
        let tsv = "shi\t是\t900\nshi\t事\t800\n";
        let e = engine(tsv);
        e.record_selection_at("事", now_ms());
        let dir = std::env::temp_dir().join(format!("wowtype-hist-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("history.tsv");
        e.save_history(&path).unwrap();

        let fresh = engine(tsv);
        assert!(fresh.load_history(&path).unwrap() >= 1);
        assert_eq!(fresh.input("shi")[0].text, "事");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rare_chars_listed_last_and_never_in_sentences() {
        // 频次小于 1 的单字是大字表补进来的（tools/rime2tsv.py --extra-chars）：
        // 一个音节的字要全列出来，但只排在最后，也不拿来组句
        let mut e = engine(concat!(
            "shi\t是\t9000\nshi\t十\t8000\nshi\t时\t7000\n",
            "shi shi\t事实\t5000\n",
            "shi\t𠯜\t0.7\nshi\t䊓\t0.8\nshi\t戠\t0.9\n",
        ));
        e.set_max_candidates(1000);
        // 默认不列扩展 B 区以后的字（常见字体显示成方块），基本区、扩展 A 照常
        let texts: Vec<String> = e.input("shi").into_iter().map(|c| c.text).collect();
        assert!(!texts.iter().any(|t| t == "𠯜"), "{texts:?}");
        assert!(
            texts.iter().any(|t| t == "䊓") && texts.iter().any(|t| t == "戠"),
            "{texts:?}"
        );

        e.set_rare_ext_chars(true);
        let list = e.input("shi");
        let texts: Vec<&str> = list.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(texts.first(), Some(&"是"), "{texts:?}");
        let at = |t: &str| texts.iter().position(|x| *x == t);
        for ch in ["是", "十", "时", "戠", "䊓", "𠯜"] {
            assert!(at(ch).is_some(), "{ch} 要列出来：{texts:?}");
        }
        assert!(at("时") < at("戠"), "常用字在前：{texts:?}");
        assert!(
            at("戠") < at("䊓") && at("䊓") < at("𠯜"),
            "基本区、扩展 A、扩展 B：{texts:?}"
        );

        let list = e.input("shishi");
        let texts: Vec<&str> = list.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(texts.first(), Some(&"事实"), "{texts:?}");
        let rare = ['𠯜', '䊓', '戠'];
        for c in &list {
            if c.text.chars().count() > 1 {
                assert!(
                    !c.text.chars().any(|ch| rare.contains(&ch)),
                    "整句不用生僻字：{texts:?}"
                );
            }
        }
        assert!(texts.contains(&"𠯜"), "第一个音节的字也要列全：{texts:?}");

        // 简拼不带生僻字
        let texts: Vec<String> = e.input("sh").into_iter().map(|c| c.text).collect();
        assert!(!texts.iter().any(|t| t == "𠯜"), "{texts:?}");
    }

    #[test]
    fn ext_chars_hidden_by_char_not_by_freq() {
        // 通用规范汉字表里的扩展区字（𬭩）照常出；别的扩展区单字不管频次多少都藏（万象词库
        // 把它们记成普通单字）；含这些字的词不受影响
        let mut e = engine(concat!(
            "yang\t氧\t9000\nyang\t𬭩\t0.7\nyang\t𪦲\t600\n",
            "an kang\t𩽾𩾌\t50\n",
        ));
        e.set_max_candidates(100);
        let texts: Vec<String> = e.input("yang").into_iter().map(|c| c.text).collect();
        assert!(texts.iter().any(|t| t == "𬭩"), "{texts:?}");
        assert!(!texts.iter().any(|t| t == "𪦲"), "{texts:?}");
        let texts: Vec<String> = e.input("ankang").into_iter().map(|c| c.text).collect();
        assert_eq!(texts.first().map(String::as_str), Some("𩽾𩾌"), "{texts:?}");
        e.set_rare_ext_chars(true);
        let texts: Vec<String> = e.input("yang").into_iter().map(|c| c.text).collect();
        assert!(texts.iter().any(|t| t == "𪦲"), "{texts:?}");
    }

    #[test]
    fn lookup_common_drops_rare_chars() {
        let lex = MemoryLexicon::from_tsv_str("shi\t是\t9000\nshi\t𬤊\t0.7\nshi shi\t事实\t0.5\n")
            .unwrap();
        let shi = crate::syllable::id_of("shi").unwrap();
        let q = [SyllableQuery::Full {
            id: shi,
            erhua: false,
        }];
        let all: Vec<String> = lex.lookup(&q).into_iter().map(|e| e.word).collect();
        let common: Vec<String> = lex.lookup_common(&q).into_iter().map(|e| e.word).collect();
        assert_eq!(all, ["是", "𬤊"]);
        assert_eq!(common, ["是"]);
        // 只看单字：低频的词照样能组句
        let two: Vec<String> = lex
            .lookup_common(&[q[0], q[0]])
            .into_iter()
            .map(|e| e.word)
            .collect();
        assert_eq!(two, ["事实"]);
    }

    #[test]
    fn other_split_heads_come_before_chars() {
        // houxuanci 也能切成 hou xu | an ci：「后续」排在生僻字前面（常用字之后）
        let mut e = engine(concat!(
            "hou xuan ci\t候选词\t3000\nhou xuan\t候选\t4000\n",
            "hou xu\t后续\t3000\nan\t按\t5000\n",
            "hou\t后\t9000\nhou\t侯\t300\nhou\t㕈\t0.8\n",
            "xuan\t选\t8000\nci\t此\t90000\nci\t词\t70000\n",
        ));
        e.set_max_candidates(1000);
        let list = e.input("houxuanci");
        let texts: Vec<&str> = list.iter().map(|c| c.text.as_str()).collect();
        let at = |t: &str| texts.iter().position(|x| *x == t);
        assert!(at("后续").is_some(), "{texts:?}");
        assert!(at("后续") < at("㕈"), "{texts:?}");
        assert!(at("候选") < at("后续"), "正常切分的词在前：{texts:?}");
    }

    #[test]
    fn first_syllable_chars_come_before_other_splits() {
        // linglianjie：输入框里切的是 ling'lian'jie。第一页放不下的 ling 的常用字（零）要排在
        // 换一种切法的词（ling li | an jie 的「凌厉」）前面；li | ng lian jie 不算一种切法
        let mut e = engine(concat!(
            "ling\t令\t9000\nling\t灵\t8000\nling\t领\t7000\nling\t另\t6000\n",
            "ling\t凌\t5000\nling\t玲\t4000\nling\t零\t3000\nling\t龄\t2000\n",
            "ling\t㖫\t0.8\n",
            "ling li\t凌厉\t3000\nling li\t伶俐\t2000\n",
            "li\t里\t90000\nli\t力\t80000\nng\t嗯\t5000\n",
            "lian jie\t连接\t5000\nlian\t连\t5000\njie\t接\t5000\nan\t按\t5000\n",
        ));
        e.set_max_candidates(1000);
        let list = e.input("linglianjie");
        let texts: Vec<&str> = list.iter().map(|c| c.text.as_str()).collect();
        let at = |t: &str| texts.iter().position(|x| *x == t);
        for common in ["零", "龄"] {
            assert!(at(common).is_some(), "{texts:?}");
            assert!(at(common) < at("凌厉"), "{common} 应在换切法的词前面：{texts:?}");
        }
        assert!(at("凌厉").is_some() && at("伶俐").is_some(), "{texts:?}");
        assert!(at("凌厉") < at("㖫"), "换切法的词在生僻字前面：{texts:?}");
        assert!(at("里").is_none() && at("力").is_none(), "li | ng… 不是一种切法：{texts:?}");
    }

    #[test]
    fn other_split_chars_come_last() {
        // fangan 也能切成 fan | gan：「反」排在 fang 的所有单字（包括生僻字）后面
        let mut e = engine(concat!(
            "fang an\t方案\t9000\n",
            "fang\t放\t9000\nfang\t方\t8000\nfang\t房\t7000\nfang\t防\t6000\n",
            "fang\t访\t5000\nfang\t仿\t4000\nfang\t纺\t3000\nfang\t㑂\t0.8\n",
            "fan\t反\t90000\ngan\t干\t9000\nan\t按\t5000\n",
        ));
        e.set_max_candidates(1000);
        let list = e.input("fangan");
        let texts: Vec<&str> = list.iter().map(|c| c.text.as_str()).collect();
        let at = |t: &str| texts.iter().position(|x| *x == t);
        assert!(at("反").is_some(), "换切法的单字还在：{texts:?}");
        assert!(at("纺") < at("反"), "{texts:?}");
        assert!(at("㑂") < at("反"), "{texts:?}");
    }

    #[test]
    fn long_input_keeps_few_sentences_then_words_and_chars() {
        let e = engine(concat!(
            "heng xiang\t横向\t5000\n",
            "hou xuan ci\t候选词\t3000\n",
            "hou xuan\t候选\t4000\n",
            "heng\t横\t3000\nheng\t恒\t2000\nheng\t衡\t1000\n",
            "ci\t此\t90000\nci\t次\t80000\nci\t词\t70000\nci\t刺\t20000\n",
            "xiang\t向\t5000\nhou\t后\t9000\nxuan\t选\t8000\n",
        ));
        let list = e.input("hengxianghouxuanci");
        let texts: Vec<&str> = list.iter().map(|c| c.text.as_str()).collect();
        let sentences = list
            .iter()
            .filter(|c| c.words.len() >= 2 && c.consumed == 18)
            .count();
        assert!(sentences <= 3, "整句最多 3 个：{texts:?}");
        // 只差最后一个字的整句不重复出现
        let tail_variants = texts
            .iter()
            .filter(|t| t.starts_with("横向候选") && t.chars().count() == 5)
            .count();
        assert_eq!(tail_variants, 1, "{texts:?}");
        assert!(texts.contains(&"横向"), "{texts:?}");
        assert!(texts.contains(&"横"), "后面要有单字：{texts:?}");
        let word = texts.iter().position(|t| *t == "横向").unwrap();
        let single = texts.iter().position(|t| *t == "横").unwrap();
        assert!(word < single, "词在单字前面：{texts:?}");
    }

    #[test]
    fn sentence_head_is_offered_for_long_input() {
        let e = engine(concat!(
            "wo\t我\t900000\n",
            "jin tian\t今天\t100000\n",
            "wan shang\t晚上\t100000\n",
            "yao\t要\t500000\n",
            "jia ban\t加班\t20000\n",
            "jia ban\t甲板\t19000\n",
        ));
        let list = e.input("wojintianwanshangyaojiaban");
        let head = list
            .iter()
            .find(|c| c.text == "我今天晚上要")
            .unwrap_or_else(|| panic!("{:?}", list.iter().map(|c| &c.text).collect::<Vec<_>>()));
        assert_eq!(head.consumed, "wojintianwanshangyao".len());
    }

    /// 模拟词级 LM 的句首（`<s>`）分：只给「何」一个很大的提升。
    struct SentenceStartBoost;

    impl LanguageModel for SentenceStartBoost {
        fn score(&self, prev: Option<&str>, cur: &str) -> f64 {
            if prev.is_none() && cur == "何" {
                5.0
            } else {
                0.0
            }
        }
    }

    #[test]
    fn single_syllable_candidates_follow_word_frequency() {
        // 打 he 时「和」比「何」常用，句首 LM 分不该把「何」抬上来。
        let lexicon: Arc<dyn Lexicon> = Arc::new(
            MemoryLexicon::from_tsv_str("he\t和\t100000\nhe\t何\t10000\nhe\t合\t1000\n").unwrap(),
        );
        let e = Engine::new(lexicon, Arc::new(SentenceStartBoost));
        let texts: Vec<String> = e.input("he").into_iter().map(|c| c.text).collect();
        let at = |word: &str| texts.iter().position(|t| t == word);
        assert_eq!(texts.first().map(String::as_str), Some("和"), "{texts:?}");
        assert!(at("何") < at("合"), "{texts:?}");
    }

    #[test]
    fn single_syllable_ranking_ignores_lm_weight() {
        // 句首分加的时候乘了 lm_weight，减的时候也要乘同一个数：不管权重多少，
        // 单音节的候选和分数都只看字频（#200 按长度调权重时减的是没乘系数的，shi 的「十」掉到第 9）
        let ranked = |lm_weight: f64| {
            let lexicon: Arc<dyn Lexicon> = Arc::new(
                MemoryLexicon::from_tsv_str("he\t和\t100000\nhe\t何\t10000\nhe\t合\t1000\n")
                    .unwrap(),
            );
            let mut config = EngineConfig::default();
            config.beam.lm_weight = lm_weight;
            Engine::new(lexicon, Arc::new(SentenceStartBoost))
                .with_config(config)
                .input("he")
                .into_iter()
                .map(|c| (c.text, c.score))
                .collect::<Vec<_>>()
        };
        let reference = ranked(1.0);
        for lm_weight in [0.5, 0.7, 1.5] {
            let got = ranked(lm_weight);
            let texts = |r: &[(String, f64)]| r.iter().map(|c| c.0.clone()).collect::<Vec<_>>();
            assert_eq!(texts(&got), texts(&reference), "lm_weight {lm_weight}");
            for (a, b) in got.iter().zip(&reference) {
                assert!((a.1 - b.1).abs() < 1e-9, "lm_weight {lm_weight}: {a:?} vs {b:?}");
            }
        }
    }

    #[test]
    fn single_syllable_keeps_learned_pair_bonus() {
        // 去掉的只是基础模型的句首分：用户在「快」后面选过「乐」，这个搭配加成要留着。
        // 两个引擎都选过一次「乐」（热度一样），只有 a 是在「快」后面选的。
        let make = || {
            let lexicon: Arc<dyn Lexicon> = Arc::new(
                MemoryLexicon::from_tsv_str("le\t了\t100000\nle\t乐\t10000\n").unwrap(),
            );
            Engine::new(lexicon, Arc::new(SentenceStartBoost))
        };
        // 搭配只认「上一次选的词，且光标前正好以它结尾」，所以先选前一个字
        let pick = |e: &Engine, prev: &str, word: &str| {
            e.set_context("");
            e.record_selection(prev);
            e.set_context(prev);
            e.record_selection(word);
        };
        let (a, b) = (make(), make());
        pick(&a, "快", "乐");
        pick(&b, "慢", "乐");
        let score = |e: &Engine| {
            e.set_context("");
            e.record_selection("快");
            e.set_context("快");
            e.input("le").into_iter().find(|c| c.text == "乐").map(|c| c.score)
        };
        let (sa, sb) = (score(&a).unwrap(), score(&b).unwrap());
        assert!(sa > sb, "学过的搭配没起作用：{sa} vs {sb}");
    }

    /// 模拟词级 LM 的分词缺口：「我要」是一个词，(我, 要) 从没计过数（回退分 −2.5），
    /// 偶然见过几次的 (我, 腰) 反而有 +1.5。
    struct MergedPairLm;

    impl LanguageModel for MergedPairLm {
        fn score(&self, prev: Option<&str>, cur: &str) -> f64 {
            match (prev, cur) {
                (Some("我"), "腰") => 1.5,
                (Some("我"), _) => -2.5,
                _ => 0.0,
            }
        }

        fn knows_word(&self, word: &str) -> Option<bool> {
            Some(matches!(word, "我" | "要" | "腰" | "我要" | "今天"))
        }
    }

    #[test]
    fn domain_lm_replaces_instead_of_stacking() {
        // 领域模型：jin tian 后面接「天」比「田」常见得多
        struct Domain;
        impl LanguageModel for Domain {
            fn score(&self, prev: Option<&str>, cur: &str) -> f64 {
                if prev == Some("今天") && cur == "天" { 2.0 } else { 0.0 }
            }
        }
        let mut e = engine("jin tian\t今天\t1000\ntian\t天\t1000\ntian\t田\t1000\n");
        let scores = |e: &Engine| -> Vec<(String, f64)> {
            e.set_context("今天");
            e.input("tian").into_iter().map(|c| (c.text, c.score)).collect()
        };
        let general = scores(&e);
        e.set_domain_lm(Some(Arc::new(Domain)));
        let once = scores(&e);
        assert_ne!(once, general, "混上领域模型要起作用");
        e.set_domain_lm(Some(Arc::new(Domain)));
        assert_eq!(scores(&e), once, "再设一次是换掉，不是再混一层");
        e.set_domain_lm(None);
        assert_eq!(scores(&e), general, "去掉以后回到通用模型");
    }

    #[test]
    fn context_merged_into_one_word_counts_as_common_pair() {
        let lexicon: Arc<dyn Lexicon> = Arc::new(
            MemoryLexicon::from_tsv_str("yao\t要\t1000000\nyao\t腰\t30000\n").unwrap(),
        );
        let e = Engine::new(lexicon, Arc::new(MergedPairLm));
        // 上文只有「我」和「今天我」（整段不是模型里的词）都要一样：取末尾模型认识的后缀
        for context in ["我", "今天我"] {
            e.set_context(context);
            let texts: Vec<String> = e.input("yao").into_iter().map(|c| c.text).collect();
            assert_eq!(texts.first().map(String::as_str), Some("要"), "{context}: {texts:?}");
        }
    }

    #[test]
    fn complete_syllable_keeps_its_own_characters_first() {
        // 「想」字频高得多，但 xian 已经打全了：本音节的字都排在补全出来的「想」前面；
        // 切成 xi'an 的整词「西安」分数差得不多，抬到第五个
        let mut tsv = String::from("xiang\t想\t50000000\nxi an\t西安\t400000\nxiao\t笑\t100\n");
        for (i, c) in "先现显线险仙咸鲜".chars().enumerate() {
            tsv.push_str(&format!("xian\t{c}\t{}\n", 1_000_000 - i * 100_000));
        }
        let lexicon: Arc<dyn Lexicon> = Arc::new(MemoryLexicon::from_tsv_str(&tsv).unwrap());
        let e = Engine::new(lexicon, Arc::new(BigramLm::default()));
        let texts: Vec<String> = e.input("xian").into_iter().map(|c| c.text).collect();
        let at = |w: &str| texts.iter().position(|t| t == w);
        assert_eq!(texts.first().map(String::as_str), Some("先"), "{texts:?}");
        assert!(at("鲜") < at("想"), "补全的字排到了本音节的字前面：{texts:?}");
        assert_eq!(at("西安"), Some(4), "{texts:?}");
        // 没打全时照样出补全
        let texts: Vec<String> = e.input("xia").into_iter().map(|c| c.text).collect();
        assert!(texts.iter().any(|t| t == "想"), "{texts:?}");
    }

    #[test]
    fn complete_syllable_chars_survive_crowded_completions() {
        // lia 只有「俩」（词频 99）：lian、liang、liao 补全出来的字比它常用得多，以前把束搜索
        // 这一段的名额全占了，「俩」排到第 60 个（#172）。打全的音节，本音节的字照样排在补全前面
        let mut tsv = String::from("lia\t俩\t99\nlia\t倆\t0.5\nli a\t理啊\t50\n");
        let chars: Vec<char> = ('\u{4E00}'..).take(60).collect();
        for (i, c) in chars.iter().enumerate() {
            let syllable = ["lian", "liang", "liao"][i % 3];
            tsv.push_str(&format!("{syllable}\t{c}\t{}\n", 100_000 - i * 1000));
        }
        let lexicon: Arc<dyn Lexicon> = Arc::new(MemoryLexicon::from_tsv_str(&tsv).unwrap());
        let mut e = Engine::new(lexicon, Arc::new(BigramLm::default()));
        e.set_max_candidates(300);
        let texts: Vec<String> = e.input("lia").into_iter().map(|c| c.text).collect();
        assert_eq!(texts.first().map(String::as_str), Some("俩"), "{texts:?}");
        // 补全出来的字接着排，生僻的异体字（倆）不提前，换切法凑的「理啊」也不提前
        assert_eq!(texts.get(1), Some(&chars[0].to_string()), "{texts:?}");
        let at = |w: &str| texts.iter().position(|t| t == w);
        assert!(at("倆").is_none_or(|i| i > 10), "{texts:?}");
        assert!(at("理啊").is_none_or(|i| i > 10), "{texts:?}");
        // 没打全的音节不受影响：lian 的字照样按词频
        let texts: Vec<String> = e.input("lian").into_iter().map(|c| c.text).collect();
        assert_eq!(texts.first(), Some(&chars[0].to_string()), "{texts:?}");
    }

    #[test]
    fn context_bigram_boosts_first_word() {
        let mut lm = BigramLm::new().with_backoff(0.001);
        lm.add_unigram("今天", 10.0);
        lm.add_unigram("天", 5.0);
        lm.add_unigram("田", 5.0);
        lm.add_bigram("今天", "天", 8.0);

        let lexicon: Arc<dyn Lexicon> =
            Arc::new(MemoryLexicon::from_tsv_str("tian\t田\t200\ntian\t天\t100\n").unwrap());
        let e = Engine::new(lexicon, Arc::new(lm));

        let before = e.input_at("tian", 1);
        assert_eq!(before[0].text, "田", "没有上下文时按频次");

        e.set_context("今天");
        let after = e.input_at("tian", 1);
        assert_eq!(
            after[0].text,
            "天",
            "上下文应当把 天 提上来: {:?}",
            after.iter().map(|c| c.text.as_str()).collect::<Vec<_>>()
        );
        let tian = after.iter().find(|c| c.text == "天").unwrap();
        let tian_farm = after.iter().find(|c| c.text == "田").unwrap();
        assert!(tian.features.with_context == 1.0);
        assert!(
            tian.features.context_bigram_score > tian_farm.features.context_bigram_score,
            "{} vs {}",
            tian.features.context_bigram_score,
            tian_farm.features.context_bigram_score
        );
    }

    #[test]
    fn user_word_is_usable() {
        let e = engine("ni hao\t你好\t10\n");
        assert!(e.add_user_word("wo wo te", "我特", 5000.0).is_ok());
        assert_eq!(e.user_words().len(), 1);
        let cands = e.input("wowote");
        assert!(
            cands.iter().any(|c| c.text == "我特"),
            "用户词应当能输入: {:?}",
            cands.iter().map(|c| c.text.as_str()).collect::<Vec<_>>()
        );
        assert!(e.remove_user_word("我特"));
        let after = e.input("wowote");
        assert!(!after.iter().any(|c| c.text == "我特"));
    }

    #[test]
    fn forget_word_clears_learning_and_saves_empty_dict() {
        let e = engine("ni hao\t你好\t10\n");
        e.add_user_word("wo wo te", "我特", 5000.0).unwrap();
        e.record_selection("我特");
        e.set_context("我特");
        e.record_selection("你好");
        assert_eq!(e.pair_count(), 1, "我特 -> 你好");
        assert!(e.forget_word("我特"));
        assert!(e.user_words().is_empty());
        assert_eq!(e.history_count("我特"), 0);
        assert_eq!(e.pair_count(), 0, "和它有关的搭配也去掉");
        assert!(!e.forget_word("我特"), "第二次没东西可忘");

        // 用户词全删光了也要存盘，不然旧的 user.tsv 留着，重启又回来
        let dir = std::env::temp_dir().join(format!("wowtype-forget-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("user.tsv");
        std::fs::write(&path, "wo wo te\t我特\t5000\n").unwrap();
        e.save_user_dict(&path).unwrap();
        let e2 = engine("ni hao\t你好\t10\n");
        assert_eq!(e2.load_user_dict(&path).unwrap(), 0);
        std::fs::remove_dir_all(&dir).ok();

        // 自定义词不忘
        e.lexicon().set_custom_tsv("ni hao\t妮好\n");
        e.record_selection("妮好");
        assert!(!e.forget_word("妮好"));
        assert_eq!(e.history_count("妮好"), 1);
    }

    #[test]
    fn async_save_writes_in_background() {
        let e = engine("ni hao\t你好\t10\n");
        e.add_user_word("wo te", "我特", 5000.0).unwrap();
        e.record_selection("我特");
        let dir = std::env::temp_dir().join(format!("wowtype-async-save-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (dict, history) = (dir.join("user.tsv"), dir.join("history.tsv"));
        e.save_user_dict_async(&dict).unwrap();
        e.save_history_async(&history).unwrap();
        assert_eq!(e.save_wait(None), saver::SaveStatus::Done);
        let again = engine("ni hao\t你好\t10\n");
        assert_eq!(again.load_user_dict(&dict).unwrap(), 1);
        assert!(again.load_history(&history).unwrap() >= 1);
        assert_eq!(again.history_count("我特"), 1);

        // 后台排着旧的、接着同步存新的：同步的先等旧的写完，文件里是新的
        e.remove_user_word("我特");
        e.save_user_dict_async(&dict).unwrap();
        e.add_user_word("wo te", "沃特", 5000.0).unwrap();
        e.save_user_dict(&dict).unwrap();
        let text = std::fs::read_to_string(&dict).unwrap();
        assert!(text.contains("沃特") && !text.contains("我特"), "{text}");

        // 写不了的报一次错
        e.save_history_async(dir.join("没有这个目录/history.tsv")).unwrap();
        assert!(matches!(e.save_wait(None), saver::SaveStatus::Failed(_)));
        assert_eq!(e.save_wait(Some(std::time::Duration::ZERO)), saver::SaveStatus::Done);

        // 引擎释放时先写完
        e.save_history_async(&history).unwrap();
        drop(e);
        assert!(std::fs::read_to_string(&history).unwrap().contains("我特"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn invalid_user_pinyin_rejected() {
        let e = engine("ni hao\t你好\t10\n");
        assert!(e.add_user_word("zzz", "坏词", 1.0).is_err());
        assert!(e.user_words().is_empty());
    }

    #[test]
    fn partial_tail_yields_completions() {
        let e = default_engine();
        let c = e.input("nih");
        assert!(
            c.iter().any(|x| x.text == "你好"),
            "nih 应当补全出 你好: {:?}",
            c.iter().map(|x| x.text.as_str()).collect::<Vec<_>>()
        );
        let c = e.input("zhongg");
        assert!(
            c.iter().any(|x| x.text == "中国"),
            "zhongg 应当补全出 中国: {:?}",
            c.iter().map(|x| x.text.as_str()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn incomplete_last_syllable_prefers_real_word() {
        let e = default_engine();
        let c = e.input("niha");
        assert_eq!(c[0].text, "你好", "niha 应当优先给出 你好");
    }

    #[test]
    fn fuzzy_matches_zh_z() {
        let lexicon: Arc<dyn Lexicon> =
            Arc::new(MemoryLexicon::from_tsv_str("zhi dao\t知道\t5000\n").unwrap());
        let lm: Arc<dyn LanguageModel> = Arc::new(BigramLm::default());
        let mut e = Engine::new(lexicon, lm);
        // 纠错会把 zidao 当成漏按 h，这里只测模糊音开关；关着时的兜底另测
        e.set_typo_correction(false);
        e.set_fuzzy_fallback(false);
        assert!(
            !e.input("zidao").iter().any(|c| c.text == "知道"),
            "默认关闭模糊音时不应命中"
        );
        e.set_fuzzy(true);
        assert!(
            e.input("zidao").iter().any(|c| c.text == "知道"),
            "开启后 zi dao 应当匹配 zhi dao"
        );
    }

    #[test]
    fn fuzzy_matches_nasal() {
        let lexicon: Arc<dyn Lexicon> =
            Arc::new(MemoryLexicon::from_tsv_str("shang dian\t商店\t5000\n").unwrap());
        let lm: Arc<dyn LanguageModel> = Arc::new(BigramLm::default());
        let mut e = Engine::new(lexicon, lm);
        e.set_fuzzy(true);
        assert!(
            e.input("sangdian").iter().any(|c| c.text == "商店"),
            "sang dian 应当匹配 shang dian"
        );
    }

    #[test]
    fn custom_codes_and_newlines() {
        let e = default_engine();
        e.lexicon().set_custom_tsv(
            "mail\tme@example.com\nwdyx\t我的邮箱\ndi zhi\t四川省\\n宜宾市\nbad1\t不收\n",
        );
        let c = e.input("mail");
        assert_eq!(c[0].text, "me@example.com");
        assert_eq!(c[0].consumed, 4);
        assert_eq!(e.input("wdyx")[0].text, "我的邮箱");
        assert_eq!(
            e.input("dizhi")[0].text,
            "四川省\n宜宾市",
            "\\n 换成真的换行"
        );
        assert!(
            !e.input("mai").iter().any(|c| c.text == "me@example.com"),
            "要整个编码对上"
        );
        assert_eq!(unescape_custom("a\\tb\\\\c\\x"), "a\tb\\c\\x");
    }

    #[test]
    fn duplicate_letter_typo() {
        // 音节中间同一个键多按一次（nihhao）也能纠成 ni hao
        let e = engine("ni hao\t你好\t10000\n");
        assert!(e.input("nihhao").iter().any(|c| c.text == "你好"));
    }

    #[test]
    fn fuzzy_groups_filter() {
        let lexicon: Arc<dyn Lexicon> = Arc::new(
            MemoryLexicon::from_tsv_str(
                "nan ren\t男人\t5000\nfu jian\t福建\t5000\nzhi dao\t知道\t5000\n",
            )
            .unwrap(),
        );
        let lm: Arc<dyn LanguageModel> = Arc::new(BigramLm::default());
        let mut e = Engine::new(lexicon, lm);
        e.set_typo_correction(false);
        e.set_fuzzy(true);
        let has =
            |e: &Engine, input: &str, word: &str| e.input(input).iter().any(|c| c.text == word);
        // 默认只开平翘舌和前后鼻音
        assert!(has(&e, "zidao", "知道"));
        assert!(!has(&e, "lanren", "男人"), "n/l 默认不开");
        assert!(!has(&e, "hujian", "福建"), "f/h 默认不开");
        e.set_fuzzy_groups(crate::syllable::fuzzy::NL | crate::syllable::fuzzy::FH);
        assert!(has(&e, "lanren", "男人"));
        assert!(has(&e, "hujian", "福建"));
        assert!(!has(&e, "zidao", "知道"), "关掉平翘舌后 zi 不再等于 zhi");
        // n 和 r 都归到 l，但 n/r 本身不是一组
        e.set_fuzzy_groups(crate::syllable::fuzzy::ALL);
        assert!(!has(&e, "ranren", "男人"), "n 和 r 不该互相模糊");
        // 几个分组一起用：lan（n/l）+ leng（r/l、en/eng）
        assert!(has(&e, "lanleng", "男人"));
    }

    #[test]
    fn fuzzy_new_groups() {
        use crate::syllable::fuzzy;
        let lexicon: Arc<dyn Lexicon> = Arc::new(
            MemoryLexicon::from_tsv_str(
                "dong xi\t东西\t5000\nguang zhou\t广州\t5000\nxiang fa\t想法\t5000\n",
            )
            .unwrap(),
        );
        let lm: Arc<dyn LanguageModel> = Arc::new(BigramLm::default());
        let mut e = Engine::new(lexicon, lm);
        e.set_typo_correction(false);
        e.set_fuzzy(true);
        let has =
            |e: &Engine, input: &str, word: &str| e.input(input).iter().any(|c| c.text == word);
        // 默认：ian/iang、uan/uang 开，un/ong 不开
        assert!(has(&e, "guanzhou", "广州"));
        assert!(has(&e, "xianfa", "想法"));
        assert!(!has(&e, "dunxi", "东西"), "un/ong 默认不开");
        e.set_fuzzy_groups(fuzzy::UN);
        assert!(has(&e, "dunxi", "东西"));
        assert!(
            !has(&e, "guanzhou", "广州"),
            "关掉 uan/uang 后 guan 不再等于 guang"
        );
        // an/ang 和 ian/iang 分开：只开 an/ang 时 xian 不等于 xiang
        e.set_fuzzy_groups(fuzzy::AN);
        assert!(!has(&e, "xianfa", "想法"));
    }

    #[test]
    fn fuzzy_pair_groups() {
        use crate::syllable::fuzzy;
        let lexicon: Arc<dyn Lexicon> = Arc::new(
            MemoryLexicon::from_tsv_str(
                "huang se\t黄色\t5000\njun\t军\t5000\nzhong\t中\t5000\ndan\t单\t5000\nyun\t云\t5000\n",
            )
            .unwrap(),
        );
        let lm: Arc<dyn LanguageModel> = Arc::new(BigramLm::default());
        let mut e = Engine::new(lexicon, lm);
        e.set_typo_correction(false);
        e.set_fuzzy(true);
        e.set_fuzzy_fallback(false);
        let has =
            |e: &Engine, input: &str, word: &str| e.input(input).iter().any(|c| c.text == word);
        // 只开另外十二组时都对不上（yun 是 ün：yun = yong 算 un = iong，不算 un = ong）
        let new_groups = fuzzy::AN_AI | fuzzy::HUANG_WANG | fuzzy::UN_IONG | fuzzy::ENG_ONG;
        e.set_fuzzy_groups(fuzzy::ALL & !new_groups);
        let pairs = [("wangse", "黄色"), ("jiong", "军"), ("zheng", "中"), ("dai", "单"), ("yong", "云")];
        for (input, word) in pairs {
            assert!(!has(&e, input, word), "{input} 不该出 {word}");
        }
        e.set_fuzzy_groups(fuzzy::HUANG_WANG);
        assert!(has(&e, "wangse", "黄色"));
        e.set_fuzzy_groups(fuzzy::UN_IONG);
        assert!(has(&e, "jiong", "军"));
        assert!(has(&e, "yong", "云"), "yun 是 ün，和 yong 是 un = iong，不是 un = ong");
        e.set_fuzzy_groups(fuzzy::ENG_ONG);
        assert!(has(&e, "zheng", "中"));
        e.set_fuzzy_groups(fuzzy::AN_AI);
        assert!(has(&e, "dai", "单"));
        // 和换声母的分组叠着用：zeng 换声母 z -> zh、换韵母 eng -> ong
        e.set_fuzzy_groups(fuzzy::ENG_ONG);
        assert!(!has(&e, "zeng", "中"));
        e.set_fuzzy_groups(fuzzy::ENG_ONG | fuzzy::ZH);
        assert!(has(&e, "zeng", "中"));
        // 整个音节换完还能再换韵母：wan -> wang -> huang
        e.set_fuzzy_groups(fuzzy::HUANG_WANG | fuzzy::AN);
        assert!(has(&e, "wanse", "黄色"));
    }

    /// #93：精确读音本身有词时（zong 有「总」，dan si 有「单丝」），模糊音的词也要列出来、
    /// 按频次和精确读音的词排在一起。
    fn fuzzy_rank_engine() -> Engine {
        engine(concat!(
            "zong\t总\t3000\n",
            "zong\t宗\t800\n",
            "zhong\t中\t20000\n",
            "si\t四\t3000\n",
            "si\t似\t2000\n",
            "shi\t是\t50000\n",
            "sou\t搜\t1000\n",
            "shou\t手\t8000\n",
            "dan\t单\t3000\n",
            "dan\t但\t5000\n",
            "dan si\t单丝\t5\n",
            "dan shi\t但是\t30000\n",
            "cu\t粗\t1000\n",
            "chu\t出\t8000\n",
            "lai\t来\t9000\n",
            "chu lai\t出来\t20000\n",
            "ji\t机\t5000\n",
            "sou ji\t搜集\t800\n",
            "shou ji\t手机\t20000\n",
        ))
    }

    fn position(e: &Engine, input: &str, word: &str) -> Option<usize> {
        e.input(input).iter().position(|c| c.text == word)
    }

    #[test]
    fn fuzzy_off_still_offers_common_word_when_exact_reading_is_junk() {
        // 模糊音关着：yingai 精确读音只能凑出「因 | 该」，按模糊音是「应该」，给出来排第一；
        // 精确读音本身有词时（dansi -> 单丝）不兜底
        let mut e = Engine::new(
            Arc::new(
                MemoryLexicon::from_tsv_str(concat!(
                    "yin\t因\t50000\n",
                    "gai\t该\t50000\n",
                    "ying gai\t应该\t5000000\n",
                    "dan si\t单丝\t5\n",
                    "dan shi\t但是\t3000000\n",
                ))
                .unwrap(),
            ),
            Arc::new(BigramLm::default()),
        );
        e.set_typo_correction(false);
        assert_eq!(position(&e, "yingai", "应该"), Some(0));
        assert_eq!(position(&e, "dansi", "但是"), None);
        e.set_fuzzy_fallback(false);
        assert_eq!(position(&e, "yingai", "应该"), None);
    }

    #[test]
    fn fuzzy_competes_with_exact_reading() {
        let mut e = fuzzy_rank_engine();
        e.set_typo_correction(false);
        e.set_fuzzy_fallback(false);
        // 关掉模糊音（也不兜底）时照旧只有精确读音
        assert_eq!(position(&e, "zong", "中"), None);
        assert_eq!(position(&e, "si", "是"), None);
        assert_eq!(position(&e, "dansi", "但是"), None);
        assert_eq!(position(&e, "culai", "出来"), None);
        e.set_fuzzy(true);
        // 单音节：模糊音的字也列出来，常用的排在前面
        assert!(
            position(&e, "zong", "中").is_some_and(|p| p < 3),
            "zong 要有 中"
        );
        assert!(
            position(&e, "si", "是").is_some_and(|p| p < 3),
            "si 要有 是"
        );
        assert!(
            position(&e, "sou", "手").is_some_and(|p| p < 3),
            "sou 要有 手"
        );
        assert_eq!(e.input("zong")[0].text, "总", "差不多常用时精确读音在前");
        // 两个音节：常用词压过精确读音凑出来的冷门词
        assert_eq!(e.input("dansi")[0].text, "但是");
        assert_eq!(e.input("culai")[0].text, "出来");
        assert_eq!(e.input("souji")[0].text, "手机");
        // 精确读音的词还在
        assert!(position(&e, "dansi", "单丝").is_some());
        assert!(position(&e, "souji", "搜集").is_some());
    }

    #[test]
    fn fuzzy_competes_with_exact_reading_shuangpin() {
        use crate::shuangpin::Scheme;
        let mut e = fuzzy_rank_engine();
        e.set_shuangpin(true);
        e.set_shuangpin_scheme(Scheme::Xiaohe);
        // 小鹤：zs = zong，sz = sou，si = si，culd = cu lai，djsi = dan si
        assert_eq!(position(&e, "zs", "中"), None);
        assert_eq!(position(&e, "djsi", "但是"), None);
        e.set_fuzzy(true);
        assert!(position(&e, "zs", "中").is_some_and(|p| p < 3));
        assert!(position(&e, "si", "是").is_some_and(|p| p < 3));
        assert!(position(&e, "sz", "手").is_some_and(|p| p < 3));
        assert_eq!(e.input("djsi")[0].text, "但是");
        assert_eq!(e.input("culd")[0].text, "出来");
        assert_eq!(e.input("szji")[0].text, "手机");
    }

    #[test]
    fn fuzzy_penalty_counts_each_rule() {
        // 用的模糊音越多扣得越多：dan shi 一处，dang shi 两处
        let mut e = engine("dan shi\t但是\t1000\ndang shi\t当时\t1000\n");
        e.set_typo_correction(false);
        e.set_fuzzy(true);
        let c = e.input("dansi");
        let at = |w: &str| c.iter().position(|x| x.text == w);
        assert!(at("但是").unwrap() < at("当时").unwrap());
        // 同一个音节里声母、韵母各算一处：san -> shan 一处，san -> shang 两处
        let mut e = engine("dai san\t带伞\t1000\ndai shan\t带山\t1000\ndai shang\t带上\t1000\n");
        e.set_typo_correction(false);
        e.set_fuzzy(true);
        let c = e.input("daisan");
        let at = |w: &str| c.iter().position(|x| x.text == w);
        assert!(at("带伞").unwrap() < at("带山").unwrap());
        assert!(at("带山").unwrap() < at("带上").unwrap());
    }

    #[test]
    fn fuzzy_groups_between_pairs() {
        use crate::syllable::{fuzzy, fuzzy_groups_between, id_of};
        let g = |a: &str, b: &str| fuzzy_groups_between(id_of(a).unwrap(), id_of(b).unwrap());
        assert_eq!(g("zhang", "zan"), Some(fuzzy::ZH | fuzzy::AN));
        assert_eq!(g("niang", "lian"), Some(fuzzy::NL | fuzzy::IAN));
        assert_eq!(g("guang", "guan"), Some(fuzzy::UAN));
        assert_eq!(g("dong", "dun"), Some(fuzzy::UN));
        assert_eq!(g("chong", "cun"), Some(fuzzy::CH | fuzzy::UN));
        assert_eq!(g("jiong", "jun"), Some(fuzzy::UN_IONG));
        assert_eq!(g("yong", "yun"), Some(fuzzy::UN_IONG), "yong 按 iong 算");
        assert_eq!(g("zheng", "zhong"), Some(fuzzy::ENG_ONG));
        assert_eq!(g("keng", "kong"), Some(fuzzy::ENG_ONG));
        assert_eq!(g("zeng", "zhong"), Some(fuzzy::ZH | fuzzy::ENG_ONG));
        assert_eq!(g("dai", "dan"), Some(fuzzy::AN_AI));
        assert_eq!(g("huang", "wang"), Some(fuzzy::HUANG_WANG));
        assert_eq!(g("wan", "huang"), Some(fuzzy::HUANG_WANG | fuzzy::AN));
        assert_eq!(g("wang", "huan"), Some(fuzzy::HUANG_WANG | fuzzy::UAN));
        // 韵母只能换一步：要换两步的不算
        assert_eq!(g("dai", "dang"), None);
        assert_eq!(g("zhen", "zhong"), None);
        assert_eq!(g("dun", "deng"), None);
        assert_eq!(g("wan", "huan"), None);
        assert_eq!(g("huang", "fang"), None, "uang 和 ang 不是一组");
        assert_eq!(g("ren", "leng"), Some(fuzzy::RL | fuzzy::EN));
        assert_eq!(g("nu", "ru"), None);
        assert_eq!(g("ying", "yin"), Some(fuzzy::IN));
        assert_eq!(g("shi", "shi"), Some(0));
    }

    #[test]
    fn shuangpin_input() {
        let lexicon: Arc<dyn Lexicon> = Arc::new(
            MemoryLexicon::from_tsv_str("zhong guo\t中国\t8000\nni hao\t你好\t10000\n").unwrap(),
        );
        let lm: Arc<dyn LanguageModel> = Arc::new(BigramLm::default());
        let mut e = Engine::new(lexicon, lm);
        assert!(
            !e.input("vsgo").iter().any(|c| c.text == "中国"),
            "默认全拼模式下 vsgo 不应命中"
        );
        e.set_shuangpin(true);
        assert!(
            e.input("vsgo").iter().any(|c| c.text == "中国"),
            "双拼 vsgo 应当命中 中国"
        );
        assert!(
            e.input("nihk").iter().any(|c| c.text == "你好"),
            "双拼 nihk 应当命中 你好"
        );
    }

    #[test]
    fn shuangpin_colliding_readings_rank_together() {
        use crate::shuangpin::Scheme;
        let lexicon: Arc<dyn Lexicon> = Arc::new(
            MemoryLexicon::from_tsv_str(
                "luo ji\t逻辑\t100000\nlian luo\t联络\t100000\n\
                 luo\t洛\t20000\nluo\t络\t30000\nlo\t咯\t5000\n\
                 ji\t机\t80000\njiu\t就\t90000\n\
                 lian\t连\t80000\nlian\t练\t60000\nlian\t脸\t50000\nlong\t龙\t70000\n",
            )
            .unwrap(),
        );
        let lm: Arc<dyn LanguageModel> = Arc::new(BigramLm::default());
        let mut e = Engine::new(lexicon, lm);
        e.set_shuangpin(true);
        e.set_shuangpin_scheme(Scheme::Ziranma);
        let texts = |input: &str| -> Vec<String> {
            e.input(input).into_iter().map(|c| c.text).collect()
        };
        // lo 和 luo 撞码：两个读法的候选要按分数一起排，luo 读法打全的「逻辑」不能被
        // lo 读法的长尾压在后面（#226）
        let loji = texts("loji");
        assert_eq!(loji.first().map(String::as_str), Some("逻辑"), "{loji:?}");
        // 配成对的 ji 已经打全，不能再补成 jiu：两个读法都会补出「咯就」「络就」这类硬凑词
        assert!(!loji.iter().any(|t| t.contains('就')), "{loji:?}");
        // lian 和 lo/luo 同理，不能让单字长尾把「联络」挤出候选
        let lmlo = texts("lmlo");
        assert_eq!(lmlo.first().map(String::as_str), Some("联络"), "{lmlo:?}");
    }

    fn shuangpin_collision_engine() -> Engine {
        use crate::shuangpin::Scheme;
        let mut e = engine(concat!(
            "luo ji\t逻辑\t100000\n",
            "luo ji\t落籍\t10\n",
            "luo\t落\t40000\n",
            "luo\t罗\t30000\n",
            "luo\t络\t20000\n",
            "lo\t咯\t5000\n",
            "ji\t机\t80000\n",
            "ji\t几\t60000\n",
        ));
        e.set_shuangpin(true);
        e.set_shuangpin_scheme(Scheme::Ziranma);
        e.set_max_candidates(300);
        e
    }

    #[test]
    fn shuangpin_collision_keeps_learned_word_first() {
        // 两种读法合并以后照样走「选过的提前」：不能再按分数重排，把选过的冷门词又压回去
        let e = shuangpin_collision_engine();
        let t0 = 1_000_000;
        assert_eq!(e.input_at("loji", t0)[0].text, "逻辑");
        e.record_selection_at("落籍", t0 + 1);
        e.record_selection_at("落籍", t0 + 2);
        let texts: Vec<String> = e.input_at("loji", t0 + 10).into_iter().map(|c| c.text).collect();
        assert_eq!(texts[0], "落籍", "{texts:?}");
        // 冷门读法（lo）的单字选过也一样，至少排到第二
        let before: Vec<String> = e.input_at("lo", t0 + 3).into_iter().map(|c| c.text).collect();
        assert!(before.iter().position(|t| t == "咯") > Some(1), "{before:?}");
        e.record_selection_at("咯", t0 + 3);
        e.record_selection_at("咯", t0 + 4);
        let texts: Vec<String> = e.input_at("lo", t0 + 10).into_iter().map(|c| c.text).collect();
        assert!(texts[..2].iter().any(|t| t == "咯"), "{texts:?}");
    }

    #[test]
    fn shuangpin_collision_keeps_more_candidates_tail() {
        // 合并发生在排位置之前：翻页长尾（第一个音节的单字）不被截掉，也不按分数插到整词中间
        let e = shuangpin_collision_engine();
        let cands = e.input("loji");
        let texts: Vec<&str> = cands.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(texts[0], "逻辑", "{texts:?}");
        let heads: Vec<usize> = (0..cands.len()).filter(|&i| cands[i].consumed < 4).collect();
        for ch in ["落", "罗", "络"] {
            let at = texts.iter().position(|t| *t == ch);
            assert!(at.is_some_and(|i| cands[i].consumed == 2), "{ch}: {texts:?}");
        }
        // 句首的单字都排在用完整个输入的候选后面
        let first_head = heads[0];
        assert!(
            cands[first_head..].iter().all(|c| c.consumed < 4),
            "{texts:?}"
        );
    }

    #[test]
    fn shuangpin_prefix_and_trailing_key() {
        use crate::shuangpin::Scheme;
        let lexicon: Arc<dyn Lexicon> = Arc::new(
            MemoryLexicon::from_tsv_str(
                "zhong guo\t中国\t8000\nren\t人\t9000\nzhong guo ren\t中国人\t3000\n\
                 ni hao\t你好\t10000\nni\t你\t9000\nhao\t好\t9000\nzhong\t中\t9000\n",
            )
            .unwrap(),
        );
        let lm: Arc<dyn LanguageModel> = Arc::new(BigramLm::default());
        let mut e = Engine::new(lexicon, lm);
        e.set_shuangpin(true);
        // 自然码 vs go rf：句首词「中」用掉前 2 个键
        let c = e.input("vsgorf");
        assert!(
            c.iter().any(|x| x.text == "中国人" && x.consumed == 6),
            "{c:?}"
        );
        assert!(c.iter().any(|x| x.text == "中" && x.consumed == 2), "{c:?}");
        // 句首词「你好」用掉前 4 个键（隔音符也算进去）
        let c = e.input("ni'hkvs");
        assert!(
            c.iter().any(|x| x.text == "你好" && x.consumed == 5),
            "{c:?}"
        );
        // 最后单一个键当声母
        assert!(e.input("vsg").iter().any(|x| x.text == "中国"));
        assert_eq!(e.shuangpin_display("vsgo").as_deref(), Some("zhong'guo"));
        // 换成小鹤：hao 是 hc
        e.set_shuangpin_scheme(Scheme::Xiaohe);
        assert!(e.input("nihc").iter().any(|x| x.text == "你好"));
        // 学习时传进来的拼音是全拼
        assert!(e.input("nihk").iter().all(|x| x.text != "你好"));
    }

    #[test]
    fn english_input_yields_raw_candidate() {
        let e = default_engine();
        let c = e.input("hello");
        assert_eq!(c.first().map(|x| x.text.as_str()), Some("hello"));
        assert!(c.iter().any(|x| x.text == "hello"));

        // 原始大小写要保留
        let c = e.input("Hello");
        assert_eq!(c[0].text, "Hello");

        // 只有未登录兜底时，原始串排最前
        let c = e.input("ok");
        assert_eq!(
            c[0].text,
            "ok",
            "{:?}",
            c.iter().map(|x| &x.text).collect::<Vec<_>>()
        );
    }

    #[test]
    fn known_english_word_beats_typo_corrected_chinese() {
        let e = engine(concat!(
            "he\t和\t9000\n",
            "lo\t咯\t500\n",
            "she\t设\t5000\n",
            "shen\t神\t9000\n",
            "ni hao\t你好\t100000\n",
        ));
        assert_eq!(
            e.load_english_str("hello\thello\nshe\tshe\nGitHub\tGitHub\nJanuary\tJan\n"),
            3
        );
        let texts =
            |input: &str| -> Vec<String> { e.input(input).into_iter().map(|c| c.text).collect() };

        // 中文只能靠纠错凑出来时，已知英文词排第一
        assert_eq!(texts("hello")[0], "hello");
        // 全小写输入用词表里的写法，自己打了大写就照原样
        assert_eq!(texts("github")[0], "GitHub");
        assert_eq!(texts("Github")[0], "Github");
        // 本身就是完整拼音的英文词，中文在前，英文排第 6（候选不足时排最后）
        let she = texts("she");
        let pos = she.iter().position(|t| t == "she").expect("应当有 she");
        assert!(pos == 5 || (pos >= 1 && pos == she.len() - 1), "{she:?}");
        // 缩写编码（Jan -> January）不当作英文词
        assert!(!texts("jan").iter().any(|t| t == "January"));
    }

    #[test]
    fn english_word_beats_chinese_sentence_made_of_abbreviations() {
        // ok -> 哦 | 可（o + k）是两个词硬凑的；bug -> 不过（bu + g）是一个词还没打完
        let e = engine("o\t哦\t900000\nke\t可\t900000\nbu guo\t不过\t500000\nbu\t不\t1000000\n");
        e.load_english_str("ok\nbug\n");
        let texts =
            |input: &str| -> Vec<String> { e.input(input).into_iter().map(|c| c.text).collect() };
        assert_eq!(texts("ok").first().map(String::as_str), Some("ok"), "{:?}", texts("ok"));
        let bug = texts("bug");
        assert_eq!(bug.first().map(String::as_str), Some("不过"), "{bug:?}");
        assert_eq!(bug.get(1).map(String::as_str), Some("bug"), "{bug:?}");
    }

    #[test]
    fn junk_sentence_does_not_follow_a_clear_word() {
        // 你好比「泥 | 好」高得多：硬凑的整句不占第二位；英文词后面不出两处纠错硬解的中文
        let e = engine("ni hao\t你好\t1000000\nni\t泥\t10000\nhao\t好\t100000\n");
        let texts: Vec<String> = e.input("nihao").into_iter().map(|c| c.text).collect();
        assert_eq!(texts.first().map(String::as_str), Some("你好"), "{texts:?}");
        assert!(!texts.iter().any(|t| t == "泥好"), "{texts:?}");
    }

    #[test]
    fn swapped_keys_beat_a_wrong_key() {
        // meiyuo：两键按反是「没有」，按错一个键（y 当 g）是「美国」；美国词频高一些，
        // 但打反比按错常见，没有排第一
        let e = engine("mei you\t没有\t500000\nmei guo\t美国\t900000\n");
        let texts: Vec<String> = e.input("meiyuo").into_iter().map(|c| c.text).collect();
        assert_eq!(texts.first().map(String::as_str), Some("没有"), "{texts:?}");
    }

    #[test]
    fn one_typo_word_follows_a_sentence_of_single_characters() {
        // tianqu：第一名「天 | 去」是两个单字硬凑的，纠一处错（u 当 i）就是「天气」，排第二
        let e = engine("tian\t天\t100000\nqu\t去\t100000\ntian qi\t天气\t1000000\n");
        let texts: Vec<String> = e.input("tianqu").into_iter().map(|c| c.text).collect();
        assert_eq!(texts.first().map(String::as_str), Some("天去"), "{texts:?}");
        assert_eq!(texts.get(1).map(String::as_str), Some("天气"), "{texts:?}");
    }

    #[test]
    fn exact_word_is_not_overridden_by_completion_or_typo() {
        // 重点的词频比种地高 20 倍，补全（zhongdi -> zhong dian）的分数会比精确词高
        let e = engine(concat!(
            "zhong dian\t重点\t500988\n",
            "zhong di\t种地\t25085\n",
            "zhong\t中\t900000\n",
            "di\t地\t800000\n",
            "ka\t卡\t402256\n",
            "kan\t看\t1500000\n",
            "zhong guo\t中国\t500000\n",
            "gu\t骨\t90000\n",
            "shi\t是\t9000000\n",
            "dan\t但\t900000\n",
            "shi dan\t实弹\t3175\n",
            "shi dang\t适当\t300000\n",
            "ta\t他\t3000000\n",
            "ta me\t他么\t333\n",
            "ta men\t他们\t3000000\n",
        ));
        let texts =
            |input: &str| -> Vec<String> { e.input(input).into_iter().map(|c| c.text).collect() };
        let zhongdi = texts("zhongdi");
        assert_eq!(zhongdi[0], "种地", "{zhongdi:?}");
        assert!(
            zhongdi.iter().any(|t| t == "重点"),
            "补全仍然在候选里：{zhongdi:?}"
        );
        assert_eq!(texts("ka")[0], "卡");
        // 整个输入是一个词时，单字凑出来的（是 | 但）也不能压过它
        assert_eq!(texts("shidan")[0], "实弹");
        // 精确结果只是单字拼凑（中 | 骨）时用户多半没打完，补全照常排第一
        assert_eq!(texts("zhonggu")[0], "中国");
        // 生僻词（词频低于下限）不受保护
        assert_eq!(texts("tame")[0], "他们");
    }

    #[test]
    fn prefix_candidates_do_not_split_a_syllable() {
        let e = engine(concat!(
            "dan\t但\t900000\n",
            "da\t大\t900000\n",
            "ni hao\t你好\t100000\n",
            "ni\t你\t900000\n",
            "zhong guo\t中国\t500000\n",
        ));
        let dan: Vec<String> = e.input("dan").into_iter().map(|c| c.text).collect();
        assert!(!dan.iter().any(|t| t == "大"), "单音节不出句首词：{dan:?}");
        let long = e.input("nihaozhongguo");
        assert_eq!(long[0].text, "你好中国");
        let head = long
            .iter()
            .position(|c| c.text == "你好")
            .expect("应当有句首词 你好");
        assert!(
            head <= 1,
            "{:?}",
            long.iter().map(|c| &c.text).collect::<Vec<_>>()
        );
        assert!(long.len() <= e.config.max_candidates.max(1));
    }

    /// 句首候选用掉的输入必须正好是它自己的拼音：带隔音符、双拼都一样。
    #[test]
    fn head_candidates_consume_exactly_their_own_pinyin() {
        let tsv = concat!(
            "xi an\t西安\t500000\n",
            "shi\t市\t900000\n",
            "ji chang\t机场\t500000\n",
            "xi\t西\t400\n",
            "an\t安\t300\n",
            "ji\t机\t300\n",
            "chang\t场\t300\n",
            "wo men\t我们\t900000\n",
            "de\t的\t900000\n",
            "wo\t我\t900000\n",
            "xi huan\t喜欢\t500000\n",
        );
        let mut e = engine(tsv);
        e.set_max_candidates(60);
        for input in ["xianshijichang", "xi'an'shi'ji'chang", "xi'anshi'jichang"] {
            let list = e.input(input);
            let heads: Vec<&Candidate> = list.iter().filter(|c| c.consumed < input.len()).collect();
            assert!(heads.iter().any(|c| c.text == "西安市"), "{input}: 缺句首候选 西安市");
            assert!(heads.iter().any(|c| c.text == "西安"), "{input}: 缺句首词 西安");
            for c in heads {
                let used = input[..c.consumed].replace('\'', "");
                assert_eq!(used, c.pinyin.replace(' ', ""), "{input}: {} 用掉了 {used}", c.text);
            }
        }
        // 小鹤双拼：每个音节两个键，解码出来的全拼音节之间都有隔音符
        e.set_shuangpin(true);
        e.set_shuangpin_scheme(crate::shuangpin::Scheme::Xiaohe);
        let input = "womfdewoxihr";
        let list = e.input(input);
        assert_eq!(list[0].text, "我们的我喜欢");
        let heads: Vec<&Candidate> = list.iter().filter(|c| c.consumed < input.len()).collect();
        assert!(heads.iter().any(|c| c.text == "我们"), "缺句首词 我们");
        for c in heads {
            assert_eq!(
                c.consumed,
                2 * c.pinyin.split_whitespace().count(),
                "{} 用掉了 {:?}",
                c.text,
                &input[..c.consumed]
            );
        }
    }

    /// 换一种切法才对得上的长词（jieran 切成 ji er an 的「己二胺」）不能压过自然切法的常用词（截然）。
    #[test]
    fn long_word_from_another_split_does_not_beat_common_word() {
        let mut e = engine(concat!(
            "jie ran\t截然\t52375\n",
            "jie\t介\t90000\n",
            "ran\t然\t900000\n",
            "ji er an\t己二胺\t760\n",
            "ji\t几\t900000\n",
            "er\t二\t900000\n",
            "an\t安\t900000\n",
        ));
        e.set_max_candidates(100);
        let list: Vec<String> = e.input("jieran").into_iter().map(|c| c.text).collect();
        assert_eq!(list[0], "截然", "{list:?}");
        let at = |w: &str| list.iter().position(|t| t == w);
        assert!(at("己二胺") > at("截然"), "{list:?}");
    }

    /// 双拼的音节边界由键对定死：句首词用掉的键数必须是音节数的两倍，不出换切法的词
    /// （ie il ce = che chuang ce，「撤出」che chu 只用掉 ie 两个键，选了会多上屏一个字）。
    #[test]
    fn shuangpin_heads_never_resplit_syllables() {
        let mut e = engine(concat!(
            "che\t车\t900000\n",
            "che\t撤\t90000\n",
            "chuang\t窗\t900000\n",
            "ce\t测\t900000\n",
            "che chu\t撤出\t50000\n",
            "chu\t出\t900000\n",
            "chuang ce\t窗测\t10\n",
        ));
        e.set_max_candidates(200);
        e.set_shuangpin(true);
        e.set_shuangpin_scheme(crate::shuangpin::Scheme::Xiaohe);
        let input = "ieilce";
        for c in e.input(input).iter().filter(|c| c.consumed < input.len()) {
            assert_eq!(
                c.consumed,
                2 * c.pinyin.split_whitespace().count(),
                "{} [{}] 用掉了 {:?}",
                c.text,
                c.pinyin,
                &input[..c.consumed]
            );
        }
    }

    /// 整个输入对得上的词库词（同音词）全部排在句首词和单字前面，不受束搜索名额限制。
    #[test]
    fn exact_homophones_come_before_head_candidates() {
        let mut tsv = String::from("tong\t同\t900000\ntong\t通\t800000\nde\t的\t9000000\n");
        for i in 0..60 {
            tsv.push_str(&format!("tong de\t同音{i}\t{}\n", 5000 - i));
        }
        // 纠错能凑出来的高频词（t 和 y 相邻）：以前它们占掉名额，把上面的词挤到单字后面
        for i in 0..40 {
            tsv.push_str(&format!("yong de\t用的{i}\t{}\n", 900000 - i));
        }
        let mut e = engine(&tsv);
        e.set_max_candidates(300);
        let input = "tongde";
        let list = e.input(input);
        let first_head = list
            .iter()
            .position(|c| c.consumed < input.len())
            .expect("应当有句首的单字");
        for i in 0..60 {
            let word = format!("同音{i}");
            let at = list.iter().position(|c| c.text == word);
            assert!(
                at.is_some_and(|at| at < first_head),
                "{word} 在第 {at:?} 位，第一个单字在第 {first_head} 位"
            );
        }
        // 上限小的时候同音词也优先留下，不是先留单字
        e.set_max_candidates(20);
        let list = e.input(input);
        assert!(list.len() <= 20);
        assert!(list.iter().filter(|c| c.text.starts_with("同音")).count() >= 15, "{list:?}");
    }

    /// 三个音节以上、整个输入正好是一个词库词：排在几个词拼出来的候选前面，不看词频。
    #[test]
    fn long_exact_word_beats_stitched_sentence() {
        let e = engine(concat!(
            "qi dong\t启动\t500000\n",
            "qi\t其\t900000\n",
            "qi\t器\t5000\n",
            "dong\t动\t50000\n",
            "qi dong qi\t启动器\t100\n",
        ));
        let list = e.input("qidongqi");
        assert_eq!(list[0].text, "启动器", "{:?}", list.iter().map(|c| &c.text).collect::<Vec<_>>());
        assert!(list.iter().any(|c| c.text == "启动其"));
    }

    #[test]
    fn later_pages_list_every_homophone_and_head_word() {
        let mut tsv = String::new();
        for i in 0..40 {
            tsv.push_str(&format!("shi\t字{i}\t{}\n", 1000 - i));
        }
        tsv.push_str("shi jie\t世界\t9000\nshi jie\t视界\t50\njie\t界\t500\n");
        let mut e = engine(&tsv);
        e.set_max_candidates(300);
        let one: Vec<String> = e.input("shi").into_iter().map(|c| c.text).collect();
        for i in 0..40 {
            assert!(one.contains(&format!("字{i}")), "缺 字{i}: {one:?}");
        }
        let two = e.input("shijie");
        assert_eq!(two[0].text, "世界");
        let texts: Vec<&str> = two.iter().map(|c| c.text.as_str()).collect();
        assert!(texts.contains(&"视界"), "{texts:?}");
        // 翻到后面能选第一个音节的任意一个字，只用掉 shi
        let head = two.iter().find(|c| c.text == "字39").expect("单字在后面");
        assert_eq!(head.consumed, 3);

        let mut off = engine(&tsv);
        off.set_max_candidates(300);
        off.config.more_candidates = false;
        assert!(off.input("shi").len() < 40);
    }

    #[test]
    fn raw_candidate_survives_a_full_list() {
        let mut e = engine(concat!(
            "he\t和\t9000\nhe\t何\t8000\nhe\t合\t7000\n",
            "lo\t咯\t500\nluo\t落\t400\n",
        ));
        e.set_max_candidates(3);
        // hrlo 不是拼音（r 是 e 的邻键），中文全靠纠错，列表还是满的
        let c = e.input("hrlo");
        assert!(
            c.iter().any(|x| x.text == "hrlo"),
            "原始串不该被截掉: {:?}",
            c.iter().map(|x| &x.text).collect::<Vec<_>>()
        );
    }

    #[test]
    fn full_pinyin_does_not_add_raw_candidate() {
        let e = default_engine();
        assert!(
            !e.input("nihao").iter().any(|c| c.text == "nihao"),
            "完整覆盖时不应插入原始拼音串"
        );
    }

    #[test]
    fn traditional_candidates() {
        let lexicon: Arc<dyn Lexicon> =
            Arc::new(MemoryLexicon::from_tsv_str("zhong guo\t中国\t8000\n").unwrap());
        let lm: Arc<dyn LanguageModel> = Arc::new(BigramLm::default());
        let mut e = Engine::new(lexicon, lm);
        e.install_converter(Converter::from_tsv_str("国\t國\n").unwrap());
        assert_eq!(e.input("zhongguo")[0].text, "中国");
        e.set_traditional(true);
        assert_eq!(e.input("zhongguo")[0].text, "中國");
        // 分词不转，还是词库里的写法（学词、记搭配用）
        assert_eq!(e.input("zhongguo")[0].words[0].text, "中国");
    }

    #[test]
    fn traditional_phrase_candidates() {
        let lexicon: Arc<dyn Lexicon> = Arc::new(
            MemoryLexicon::from_tsv_str(
                "tou fa\t头发\t8000\ngan jing\t干净\t8000\nruan jian\t软件\t8000\n",
            )
            .unwrap(),
        );
        let lm: Arc<dyn LanguageModel> = Arc::new(BigramLm::default());
        let mut e = Engine::new(lexicon, lm);
        e.install_converter(
            Converter::from_tsv_str(
                "发\t發\n干\t幹\n头\t頭\n净\t淨\n软\t軟\n头发\t頭髮\n干净\t乾淨\n软件\t軟體\n",
            )
            .unwrap(),
        );
        e.set_traditional(true);
        assert_eq!(e.input("toufa")[0].text, "頭髮");
        assert_eq!(e.input("ganjing")[0].text, "乾淨");
        // 词组可以换成台湾用语，字数也可以不同
        assert_eq!(e.input("ruanjian")[0].text, "軟體");
        assert_eq!(e.input("ruanjian")[0].words[0].text, "软件");
    }

    #[test]
    fn auto_learn_promotes_word() {
        let lexicon: Arc<dyn Lexicon> =
            Arc::new(MemoryLexicon::from_tsv_str("ni hao\t你好\t10000\n").unwrap());
        let lm: Arc<dyn LanguageModel> = Arc::new(BigramLm::default());
        let e = Engine::new(lexicon, lm);

        // 词库里没有「我特」，初始只能拿到原始串
        assert!(!e.input("wote").iter().any(|c| c.text == "我特"));

        // 选不够次数不学
        e.record_selection_with_pinyin("我特", Some("wo te"));
        e.record_selection_with_pinyin("我特", Some("wo te"));
        assert!(!e.input("wote").iter().any(|c| c.text == "我特"));

        // 第三次达到阈值
        e.record_selection_with_pinyin("我特", Some("wo te"));
        assert_eq!(e.user_words().len(), 1);
        assert!(
            e.input("wote").iter().any(|c| c.text == "我特"),
            "自动学习后应当能打出 我特"
        );
    }

    fn shuangpinge_engine() -> Engine {
        engine(
            "shuang\t双\t10000\nshuang\t霜\t500\npin\t拼\t8000\npin\t频\t500\n\
             ge\t个\t20000\nge\t阁\t300\nshuang pin\t双拼\t3000\n",
        )
    }

    #[test]
    fn composed_word_is_learned_on_second_use() {
        // 以前分几次选出来的词用一次就收进用户词库，挑错一个字（你好事竣）也成了用户词：
        // 现在第一次只记成候选新词，30 天里第二次这样拼出来才收
        let e = shuangpinge_engine();
        assert!(!e.input("shuangpinge").iter().any(|c| c.text == "霜频阁"));
        let day = 24 * 3600 * 1000;
        e.record_composed_word_at("霜频阁", "shuang pin ge", day);
        assert!(e.user_words().is_empty(), "第一次只是候选新词");
        assert!(e.is_new_word("霜频阁"));
        e.record_composed_word_at("霜频阁", "shuang pin ge", 20 * day);
        assert_eq!(e.user_words().len(), 1, "30 天里第二次就收");
        assert!(!e.is_new_word("霜频阁"));
        assert_eq!(e.input("shuangpinge")[0].text, "霜频阁");

        // 第二次直接选了整个词（学到的搭配让它排上来了）也算
        let e = shuangpinge_engine();
        e.record_composed_word_at("霜频阁", "shuang pin ge", day);
        e.record_selection_with_pinyin_at("霜频阁", Some("shuang pin ge"), 2 * day);
        assert_eq!(e.user_words().len(), 1);

        // 隔了 30 天以上：重新算第一次
        let e = shuangpinge_engine();
        e.record_composed_word_at("霜频阁", "shuang pin ge", day);
        e.record_composed_word_at("霜频阁", "shuang pin ge", 40 * day);
        assert!(e.user_words().is_empty());
        assert!(e.is_new_word("霜频阁"));
    }

    #[test]
    fn auto_learn_uses_decayed_heat() {
        // 很久以前零零散散选过的不算数：每隔 60 天选一次，热度一直到不了 3
        let e = engine("ni hao\t你好\t10000\n");
        let day = 24 * 3600 * 1000;
        for i in 0..5 {
            e.record_selection_with_pinyin_at("我特", Some("wo te"), i * 60 * day);
        }
        assert!(e.user_words().is_empty(), "{:?}", e.user_words());
        // 一天里选三次就够
        for i in 0..3 {
            e.record_selection_with_pinyin_at("沃特", Some("wo te"), 400 * day + i);
        }
        assert_eq!(e.user_words().len(), 1);
    }

    #[test]
    fn one_old_pick_stops_promoting() {
        // 误选一次的词以前会一直排第一（热度衰减没有下限）；热度掉到半次以下就不再提前。
        // 频次差得远，排序特征里的热度本身翻不过来，排第一靠的是「上次选过的放第一」
        let e = engine("shi jian\t时间\t9000\nshi jian\t事件\t800\n");
        e.record_selection_at("事件", 0);
        let day = 24 * 3600 * 1000;
        assert_eq!(e.input_at("shijian", day)[0].text, "事件", "刚选过的提前");
        assert_eq!(e.input_at("shijian", 29 * day)[0].text, "事件", "一个月以内还提前");
        assert_eq!(e.input_at("shijian", 45 * day)[0].text, "时间", "一个半月以后回到原位");
    }

    #[test]
    fn undo_learning_rolls_back_one_selection() {
        let e = shuangpinge_engine();
        e.add_user_word("wo te", "我特", 5000.0).unwrap();
        e.record_selection("我特");
        e.set_context("我特");
        let start = e.learn_mark();
        // 选够次数自动收词、学搭配、记候选新词
        for _ in 0..3 {
            e.record_selection_with_pinyin("双拼", Some("shuang pin"));
        }
        e.record_composed_word("霜频阁", "shuang pin ge");
        let end = e.learn_mark();
        assert!(e.user_words().iter().any(|(_, w, _)| w == "双拼"));
        assert!(e.pair_count() >= 1);
        assert!(e.is_new_word("霜频阁"));
        assert!(e.undo_learning(start, end));
        assert_eq!(e.history_count("双拼"), 0);
        assert_eq!(e.history_count("霜频阁"), 0);
        assert!(!e.is_new_word("霜频阁"));
        assert_eq!(e.pair_count(), 0, "我特 -> 双拼 退掉");
        let words: Vec<String> = e.user_words().into_iter().map(|(_, w, _)| w).collect();
        assert_eq!(words, ["我特"], "自动收的退掉，原来的用户词不动");
        assert_eq!(e.history_count("我特"), 1, "之前的学习不动");
        assert!(!e.undo_learning(start, end), "撤销过的不再撤销");

        // 撤销以后又被别处学过的不动
        let start = e.learn_mark();
        e.record_selection("双拼");
        let end = e.learn_mark();
        e.record_selection("双拼");
        e.undo_learning(start, end);
        assert_eq!(e.history_count("双拼"), 2, "后来又选的一次在，撤销不去碰");
    }

    #[test]
    fn long_composed_sentence_still_needs_repeats() {
        let lexicon: Arc<dyn Lexicon> =
            Arc::new(MemoryLexicon::from_tsv_str("ni hao\t你好\t10000\n").unwrap());
        let lm: Arc<dyn LanguageModel> = Arc::new(BigramLm::default());
        let e = Engine::new(lexicon, lm);
        // 先选句首词再选剩下的长句：不当造词，一次不进词库
        e.record_composed_word("我今天去学校了", "wo jin tian qu xue xiao le");
        assert!(e.user_words().is_empty());
        e.record_composed_word("我今天去学校了", "wo jin tian qu xue xiao le");
        e.record_composed_word("我今天去学校了", "wo jin tian qu xue xiao le");
        assert_eq!(e.user_words().len(), 1);
    }

    #[test]
    fn composed_word_respects_auto_learn_off() {
        let lexicon: Arc<dyn Lexicon> =
            Arc::new(MemoryLexicon::from_tsv_str("ni hao\t你好\t10000\n").unwrap());
        let lm: Arc<dyn LanguageModel> = Arc::new(BigramLm::default());
        let config = EngineConfig {
            auto_learn: false,
            ..EngineConfig::default()
        };
        let e = Engine::new(lexicon, lm).with_config(config);
        e.record_composed_word("我特", "wo te");
        assert!(e.user_words().is_empty());
    }

    #[test]
    fn auto_learn_can_be_disabled() {
        let lexicon: Arc<dyn Lexicon> =
            Arc::new(MemoryLexicon::from_tsv_str("ni hao\t你好\t10000\n").unwrap());
        let lm: Arc<dyn LanguageModel> = Arc::new(BigramLm::default());
        let config = EngineConfig {
            auto_learn: false,
            ..EngineConfig::default()
        };
        let e = Engine::new(lexicon, lm).with_config(config);
        for _ in 0..5 {
            e.record_selection_with_pinyin("我特", Some("wo te"));
        }
        assert!(e.user_words().is_empty());
    }

    #[test]
    fn context_disambiguates_homophones() {
        // 同一拼音，两个候选：先（高频）、西安（低频）
        let lexicon: Arc<dyn Lexicon> =
            Arc::new(MemoryLexicon::from_tsv_str("xian\t先\t50000\nxi an\t西安\t1000\n").unwrap());
        // TSV 加载器会把二元组计数也记进一元，所以这里额外给「先」补一条
        // 高频搭配，让它在**没有上下文**时仍然靠词频胜出。
        let lm: Arc<dyn LanguageModel> = Arc::new(
            BigramLm::from_bigram_tsv_str("我\t西安\t9990\n我\t先\t10\n先\t的\t100000\n").unwrap(),
        );
        let e = Engine::new(lexicon, lm);

        // 没有上下文时，词频决定：先
        assert_eq!(e.input("xian")[0].text, "先");

        // 有上下文「我」之后，语言模型应当把 西安 顶上来
        e.set_context("我");
        assert_eq!(e.input("xian")[0].text, "西安", "上下文应当消除同音词歧义");
    }

    #[test]
    fn arpa_context_disambiguates() {
        let lexicon: Arc<dyn Lexicon> =
            Arc::new(MemoryLexicon::from_tsv_str("xian\t先\t50000\nxi an\t西安\t1000\n").unwrap());
        let arpa = "\\data\\\nngram 1=3\nngram 2=2\n\n\\1-grams:\n-1.0\t我\n-1.0\t先\n-1.0\t西安\n\n\\2-grams:\n-0.01\t我 西安\n-4.0\t我 先\n\n\\end\\\n";
        let lm: Arc<dyn LanguageModel> =
            Arc::new(ArpaLm::from_arpa_str(arpa).expect("ARPA 解析失败"));
        let e = Engine::new(lexicon, lm);
        assert_eq!(e.input("xian")[0].text, "先");
        e.set_context("我");
        assert_eq!(e.input("xian")[0].text, "西安");
    }

    #[test]
    fn context_uses_last_character() {
        // 中文上下文通常没有空格，必须能退到「最后一个字」去查 bigram
        let lexicon: Arc<dyn Lexicon> =
            Arc::new(MemoryLexicon::from_tsv_str("shi\t是\t50000\nshi\t事\t30000\n").unwrap());
        let lm: Arc<dyn LanguageModel> = Arc::new(
            BigramLm::from_bigram_tsv_str(
                "做\t事\t9990\n做\t是\t10\n是\t的\t100000\n事\t的\t100000\n",
            )
            .unwrap(),
        );
        let e = Engine::new(lexicon, lm);
        assert_eq!(e.input("shi")[0].text, "是");

        e.set_context("我今天做");
        assert_eq!(
            e.input("shi")[0].text,
            "事",
            "多字上下文应当用最后一个字查二元组"
        );
    }

    #[test]
    fn completion_does_not_extend_unknown_head() {
        // shesh：只能是 she + sh*，不能把 shesh 整体当成能解析的头
        let e = engine("she shi\t设施\t500\nshe\t设\t100\n");
        assert!(e.input("shesh").iter().any(|c| c.text == "设施"));
    }

    #[test]
    fn completion_extends_the_last_syllable() {
        // 「shesh」尾巴是 sh，应当能补成 设施(she shi)
        let lexicon: Arc<dyn Lexicon> = Arc::new(
            MemoryLexicon::from_tsv_str(
                "she shi\t设施\t9000\nshe sha\t射杀\t500\nshe shou\t射手\t400\n",
            )
            .unwrap(),
        );
        let e = Engine::new(lexicon, Arc::new(BigramLm::default()));
        let list = e.input("shesh");
        assert!(
            list.iter().take(3).any(|c| c.text == "设施"),
            "应当补出 设施: {:?}",
            list.iter().map(|c| &c.text).collect::<Vec<_>>()
        );
    }

    #[test]
    fn input_xian() {
        let e = default_engine();
        let cands = e.input("xian");
        let texts: Vec<&str> = cands.iter().map(|c| c.text.as_str()).collect();
        assert!(texts.contains(&"先"), "{:?}", texts);
        assert!(texts.contains(&"西安"), "{:?}", texts);
    }

    #[test]
    fn apostrophe_does_not_yield_xian_word() {
        let e = default_engine();
        let cands = e.input("xi'an");
        let texts: Vec<&str> = cands.iter().map(|c| c.text.as_str()).collect();
        assert!(texts.contains(&"西安"), "{:?}", texts);
        // 强制切分后不应再出现单音节 xian 对应的 先。
        assert!(!texts.contains(&"先"), "{:?}", texts);
    }

    #[test]
    fn jianpin_fallback() {
        let e = default_engine();
        let cands = e.input("zg");
        assert!(cands.iter().any(|c| c.text == "中国"), "{:?}", cands);
    }

    #[test]
    fn single_initial_pins_the_common_char_first() {
        // #156：只打 l，词频更高的「来」「里」不能排在「了」前面；选过别的字也不换掉第一个
        let e = engine(concat!(
            "lai\t来\t14837007\n",
            "li\t里\t7329328\n",
            "le\t了\t3481243\n",
            "liao\t了\t999\n",
            "ba\t把\t3798288\n",
            "bu\t不\t2985809\n",
            "fa\t发\t3000000\n",
            "fang\t放\t4000000\n",
        ));
        let texts = |raw: &str| e.input(raw).into_iter().map(|c| c.text).collect::<Vec<_>>();
        assert_eq!(texts("l")[0], "了", "{:?}", texts("l"));
        assert_eq!(texts("b")[0], "不", "{:?}", texts("b"));
        // 没列的声母照词频
        assert_eq!(texts("f")[0], "放", "{:?}", texts("f"));
        // 打全的拼音不受影响
        assert_eq!(texts("lai")[0], "来");
        for _ in 0..5 {
            e.record_selection("来");
        }
        let l = texts("l");
        assert_eq!(&l[..2], ["了", "来"], "选过的字排第二，第一个不变：{l:?}");
    }

    #[test]
    fn single_letters_and_dead_syllables_still_give_candidates() {
        // 以前这几种输入一个候选都没有：
        // - 单独的 r 被当成儿化标记，整个输入报错
        // - hm 能切成叹词音节，可词库里没有 hm 的词，也不去试简拼
        // - qv、xv、yv 是扩展音节，词库却记在 qu、xu、yu 下面
        let e = engine(concat!(
            "ren\t人\t50000\n",
            "ri\t日\t30000\n",
            "hou mian\t后面\t20000\n",
            "hao ma\t好吗\t10000\n",
            "qu\t去\t80000\n",
            "xu yao\t需要\t40000\n",
            "yu\t与\t30000\n",
            "ju zi\t句子\t9000\n",
            "quan bu\t全部\t9000\n",
            "hua\t花\t9000\n",
            "lv\t绿\t9000\n",
            "lu\t路\t9000\n",
        ));
        let has = |raw: &str, want: &str| {
            let c = e.input(raw);
            assert!(c.iter().any(|c| c.text == want), "{raw} 应有 {want}：{:?}", c.iter().map(|c| &c.text).collect::<Vec<_>>());
        };
        has("r", "人");
        has("r", "日");
        has("hm", "后面");
        has("hm", "好吗");
        has("qv", "去");
        has("xvyao", "需要");
        has("yv", "与");
        has("jvzi", "句子");
        has("qvanbu", "全部");
        // 儿化照旧
        has("hua'r", "花");
        // lv 还是绿，不变成路
        assert_eq!(e.input("lv").first().map(|c| c.text.as_str()), Some("绿"));
    }

    #[test]
    fn mixed_full_and_initial_only_syllables() {
        // 有的音节只打声母（省略韵母）：k yi -> 可以，yong z wen -> 用中文
        let e = engine(concat!(
            "ke yi\t可以\t50000\n",
            "ke\t可\t20000\n",
            "kui\t亏\t3000\n",
            "yi\t一\t100000\n",
            "yong\t用\t30000\n",
            "zhong wen\t中文\t20000\n",
            "zheng wen\t正文\t2000\n",
            "zen\t怎\t5000\n",
            "wen\t文\t10000\n",
        ));
        assert_eq!(e.input("kyi")[0].text, "可以");
        assert_eq!(e.input("yongzwen")[0].text, "用中文");
        // 打全了照旧
        assert_eq!(e.input("keyi")[0].text, "可以");
        assert_eq!(e.input("yongzhongwen")[0].text, "用中文");
    }

    #[test]
    fn stretched_mixed_parse_does_not_bury_raw_input() {
        // 没有英文词表时，Hello 被混简拼凑成「合理了哦」，原样输入不能垫底
        let e = engine(concat!(
            "he li\t合理\t50000\n",
            "le o\t了哦\t30000\n",
            "ke yi\t可以\t50000\n",
        ));
        assert_eq!(e.input("Hello")[0].text, "Hello");
        // 省略在词里面的照旧
        assert_eq!(e.input("kyi")[0].text, "可以");
    }

    #[test]
    fn erhua() {
        let e = default_engine();
        let cands = e.input("huar");
        assert!(cands.iter().any(|c| c.text == "花儿"), "{:?}", cands);
    }

    #[test]
    fn empty_input() {
        let e = default_engine();
        assert!(e.input("").is_empty());
        assert!(e.input("   ").is_empty());
    }

    #[test]
    fn unknown_input_falls_back_to_pinyin() {
        let e = engine("");
        let cands = e.input("xian");
        assert!(!cands.is_empty());
        assert_eq!(cands[0].text, "xian");
    }

    fn beam_path(words: &[(&str, &str, usize, usize, bool)]) -> BeamPath {
        // (字, 拼音, 只打了声母的音节数, 纠错数, 词库认识)
        let words: Vec<BeamWord> = words
            .iter()
            .map(|&(text, pinyin, abbrev, typos, known)| BeamWord {
                text: text.into(),
                pinyin: pinyin.into(),
                freq: 1.0,
                known,
                syllable_count: pinyin.split_whitespace().count(),
                jianpin: abbrev > 0,
                abbrev,
                ends_abbrev: false,
                typos,
                swaps: 0,
                completion: 0,
                start: 0,
                end: 0,
            })
            .collect();
        BeamPath {
            text: words.iter().map(|w| w.text.as_str()).collect(),
            pinyin: words.iter().map(|w| w.pinyin.as_str()).collect::<Vec<_>>().join(" "),
            words,
            score: 0.0,
            lm_score: 0.0,
            lexical_cost: 0.0,
            coverage: 1.0,
            jianpin: false,
            typos: 0,
            completion: 0,
        }
    }

    #[test]
    fn nnlm_skips_garbage_paths() {
        // sdsjdsds：每个字母都只是声母，凑出来的整句不值得让模型重排（#227）
        let garbage = beam_path(&[
            ("说的", "s d", 2, 0, true),
            ("是", "s", 1, 0, true),
            ("假的", "j d", 2, 0, true),
            ("山东省", "s d s", 3, 0, true),
        ]);
        assert_eq!(weak_syllables(&garbage), 8);
        // 打全的长句，最后一个音节只打了声母、中间纠了一处错：照样重排
        let sentence = beam_path(&[
            ("我想", "wo xiang", 0, 0, true),
            ("去", "qu", 0, 0, true),
            ("字节", "zi jie", 0, 1, true),
            ("上班", "shang b", 1, 0, true),
        ]);
        assert_eq!(weak_syllables(&sentence), 3);
        assert!(weak_syllables(&sentence) * 2 <= 7);
        // 词库不认识的段整段算
        let raw = beam_path(&[("qwer", "q w e r", 0, 0, false), ("你", "ni", 0, 0, true)]);
        assert_eq!(weak_syllables(&raw), 4);
    }

    #[test]
    fn nnlm_picks_cap_scored_chars() {
        let long = "一二三四五六七八九十".repeat(4); // 40 个字
        let paths: Vec<BeamPath> = (0..8)
            .map(|i| {
                // 前四条只在最后一个字上不同（共用前缀，几乎不多算），后四条整句都不一样
                let text = if i < 4 {
                    format!("{long}{i}")
                } else {
                    format!("{i}{long}")
                };
                beam_path(&[(text.as_str(), "x", 0, 0, true)])
            })
            .collect();
        let order: Vec<usize> = (0..8).collect();
        // 41 + 1 + 1 + 1 = 44 个字，再来一条就 85 个，超过上限
        assert_eq!(nnlm_picks(&paths, &order, 8), vec![0, 1, 2, 3]);
        // 至少两条
        let order = vec![4, 5, 6];
        assert_eq!(nnlm_picks(&paths, &order, 8), vec![4, 5]);
        assert_eq!(nnlm_picks(&paths, &order, 1), vec![4]);
    }

    #[test]
    fn ltr_valid_reflects_ranker() {
        struct NullLtr;
        impl crate::rank::LtrModel for NullLtr {
            fn score(&self, _feature: &Feature) -> f32 {
                0.0
            }
        }
        let lexicon: Arc<dyn Lexicon> =
            Arc::new(MemoryLexicon::from_tsv_str("xian\t先\t1000\n").unwrap());
        let lm: Arc<dyn LanguageModel> = Arc::new(BigramLm::default());
        let ranker = Arc::new(LinearRanker::default().with_ltr(Arc::new(NullLtr), 1.0));
        let e = Engine::new(lexicon, lm).with_ranker(ranker);
        let cands = e.input("xian");
        assert!(!cands.is_empty());
        assert_eq!(cands[0].features.ltr_valid, 1.0);
    }
}
