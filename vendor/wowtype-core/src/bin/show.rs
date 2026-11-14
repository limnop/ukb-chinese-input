//! 打印几条输入的候选，配置和 fcitx5 插件一致（候选上限 18、英文词表、纠错）。
//!
//! ```
//! wowtype-show --lexicon lexicon.tsv [--english english.txt] [--max 18] [--context 文本] 拼音...
//! ```
//!
//! 每个候选标出类型：`句` 整句（多个词拼的）、`词` 整个输入是一个词、
//! `首` 句首词（只用掉输入的开头）、`原` 原始串/英文。

use std::sync::Arc;

use wowtype_core::lm::BigramLm;
use wowtype_core::{Engine, EngineConfig, Lexicon, MemoryLexicon};

fn fail(msg: &str) -> ! {
    eprintln!("{msg}");
    std::process::exit(1);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut lexicon_path = String::new();
    let mut english_path = String::new();
    let mut lm_path = String::new();
    let mut context = String::new();
    let mut max_candidates = 18usize;
    let mut scores = false;
    let mut inputs = Vec::new();
    let mut i = 1;
    while i < args.len() {
        let take = |i: &mut usize| -> String {
            *i += 1;
            args.get(*i).cloned().unwrap_or_default()
        };
        match args[i].as_str() {
            "--lexicon" => lexicon_path = take(&mut i),
            "--english" => english_path = take(&mut i),
            "--lm" => lm_path = take(&mut i),
            "--context" => context = take(&mut i),
            "--max" => max_candidates = take(&mut i).parse().unwrap_or(18),
            "--scores" => scores = true,
            "-h" | "--help" => {
                println!("wowtype-show --lexicon <tsv> [--lm <words.lm>] [--english <txt>] [--max n] [--context 文本] [--scores] 拼音...");
                return;
            }
            other => inputs.push(other.to_string()),
        }
        i += 1;
    }
    if lexicon_path.is_empty() {
        fail("需要 --lexicon");
    }
    let lexicon = MemoryLexicon::from_tsv_path_cached(&lexicon_path)
        .unwrap_or_else(|e| fail(&format!("词库载入失败: {e:#}")));
    let lex: Arc<dyn Lexicon> = Arc::new(lexicon);
    let config = EngineConfig {
        max_candidates,
        ..EngineConfig::default()
    };
    let lm: Arc<dyn wowtype_core::lm::LanguageModel> = if !lm_path.is_empty() {
        wowtype_core::lm::load_lm(&lm_path)
            .unwrap_or_else(|e| fail(&format!("语言模型载入失败: {e:#}")))
    } else {
        Arc::new(BigramLm::default())
    };
    let engine = Engine::new(lex, lm).with_config(config);
    if !english_path.is_empty() {
        engine
            .load_english(&english_path)
            .unwrap_or_else(|e| fail(&format!("英文词表载入失败: {e:#}")));
    }
    engine.set_context(&context);
    for input in &inputs {
        let candidates = engine.input(input);
        let line: Vec<String> = candidates
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let kind = if c.consumed < input.len() {
                    "首"
                } else if c.words.len() == 1 && c.words[0].pinyin.is_empty() {
                    "原"
                } else if c.words.len() >= 2 {
                    "句"
                } else {
                    "词"
                };
                if scores {
                    format!("{}.{}{}({:.1})", i + 1, c.text, kind, c.score)
                } else {
                    format!("{}.{}{}", i + 1, c.text, kind)
                }
            })
            .collect();
        // 编辑区的分词和插件一样：按第一个用完整个输入的候选切
        let shown = candidates
            .iter()
            .find(|c| c.consumed >= input.len())
            .and_then(|c| wowtype_core::preedit::segment_input(input, &c.pinyin))
            .unwrap_or_else(|| input.clone());
        println!("{input} [{shown}]: {}", line.join(" "));
    }
}
