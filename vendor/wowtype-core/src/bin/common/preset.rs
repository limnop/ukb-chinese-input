//! 评测工具（`wowtype-eval`、`wowtype-show`）共用的「和外壳一致」的引擎配置。
//!
//! 两个外壳（fcitx5 插件 `fcitx5-addon/src/wowtype_engine.cpp` 的 `loadCore` / `applyCoreSettings`，
//! macOS `macos/Sources/Shared/Composer.swift` 的 `Engine`）在默认设置下都这样配引擎：
//!
//! | 项 | 外壳 | 这里 |
//! |---|---|---|
//! | 候选上限 | `kMaxCandidates` / `Engine.maxCandidates` = 1000 | [`SHELL_MAX_CANDIDATES`] |
//! | 词级语言模型 | 数据目录里的 `words.lm` | 词库旁边的 `words.lm` |
//! | 英文词表 | 数据目录里的 `english.txt` | 词库旁边的 `english.txt` |
//! | 模糊音 | 默认关（`Fuzzy=False`），分组默认 zh ch sh an en in ian uan | `fuzzy = false`、`syllable::fuzzy::DEFAULT` |
//! | 模糊音兜底 | 引擎默认开 | 同 |
//! | 按错键纠错 | 默认开（`TypoCorrection=True`） | `typo_correction = true` |
//! | 扩展 B 区以后的生僻字 | 默认关（`RareExtChars=False`） | `rare_ext_chars = false` |
//! | 双拼 | 默认关，方案默认自然码 | `shuangpin = false`、`Ziranma` |
//! | 繁体 | 默认关 | `traditional = false` |
//! | 补全、句首词、后几页补全同音词和单字 | 引擎默认（都开） | 同 |
//! | 模糊音索引 | `wowtype_engine_new` 里预建（`prepare_fuzzy`） | 同 |
//! | 上文 | 会话只给最后 24 个字节（`session.rs` 的 `CONTEXT_BYTES`） | [`context_tail`] |
//!
//! 外壳里有、这里没有的：用户词库、输入习惯（`history.tsv`）、自定义词、领域模型 `words.domain.lm`、
//! 本机才有的神经语言模型（都是用户自己的数据，评测不带），以及会话在引擎候选之外插的 emoji 推荐、
//! 日期时间（不改引擎给出的相对顺序，评测只看引擎的候选）。
//!
//! 外壳的默认值改了，这里跟着改。

#![allow(dead_code)]

use std::path::{Path, PathBuf};

use wowtype_core::EngineConfig;

/// 两个外壳的候选上限：翻页时一个音节的字要列全（`yi` 有八百多个字）。
pub const SHELL_MAX_CANDIDATES: usize = 1000;

/// 会话给引擎的上文最多这么多字节（`core/src/session.rs` 的 `CONTEXT_BYTES`）。
pub const CONTEXT_BYTES: usize = 24;

/// 旧版 `wowtype-eval` 的默认候选上限（`--legacy`）。
pub const LEGACY_MAX_CANDIDATES: usize = 10;

/// 外壳默认设置下的引擎配置。
pub fn shell_config() -> EngineConfig {
    let mut config = EngineConfig {
        max_candidates: SHELL_MAX_CANDIDATES,
        fuzzy: false,
        fuzzy_groups: wowtype_core::syllable::fuzzy::DEFAULT,
        shuangpin: false,
        shuangpin_scheme: wowtype_core::shuangpin::Scheme::Ziranma,
        typo_correction: true,
        traditional: false,
        ..EngineConfig::default()
    };
    config.beam.rare_ext_chars = false;
    config
}

/// 词库旁边的数据文件（外壳在同一个数据目录里找 `words.lm`、`english.txt`）。
pub fn sibling(lexicon: &str, name: &str) -> Option<String> {
    let dir = Path::new(lexicon)
        .parent()
        .unwrap_or_else(|| Path::new("."));
    let path: PathBuf = dir.join(name);
    path.is_file().then(|| path.to_string_lossy().into_owned())
}

/// 上文只留最后 [`CONTEXT_BYTES`] 个字节（在字符边界上切），和会话一样。
pub fn context_tail(text: &str) -> &str {
    if text.len() <= CONTEXT_BYTES {
        return text;
    }
    let mut start = text.len() - CONTEXT_BYTES;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    &text[start..]
}
