//! 打印几条输入的候选，配置和两个外壳默认设置下一样（候选上限 1000、词库旁边的 words.lm 和
//! english.txt、纠错开、模糊音关……逐项见 src/bin/common/preset.rs）。
//!
//! ```
//! wowtype-show (--data <目录> | --lexicon lexicon.tsv) [--lm words.lm | --no-lm] [--english english.txt | --no-english]
//!              [--max 1000] [--show 18] [--context 文本] [--shuangpin 方案] [--scores] [--legacy] 拼音...
//! ```
//!
//! 只打印前 `--show` 个（默认 18，外壳的前两页）。`--legacy` 是以前的配置：候选上限 18、
//! 只用命令行给的语言模型和英文词表、上文不截。
//!
//! 每个候选标出类型：`句` 整句（多个词拼的）、`词` 整个输入是一个词、
//! `首` 句首词（只用掉输入的开头）、`原` 原始串/英文。

#[path = "common/preset.rs"]
mod preset;

use std::path::Path;
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
    let mut max_candidates: Option<usize> = None;
    let mut show = 18usize;
    let mut scores = false;
    let mut legacy = false;
    let mut no_lm = false;
    let mut no_english = false;
    let mut scheme = None;
    let mut inputs = Vec::new();
    let mut i = 1;
    while i < args.len() {
        let take = |i: &mut usize| -> String {
            *i += 1;
            args.get(*i).cloned().unwrap_or_default()
        };
        match args[i].as_str() {
            "--lexicon" => lexicon_path = take(&mut i),
            "--data" => {
                lexicon_path = Path::new(&take(&mut i))
                    .join("lexicon.tsv")
                    .to_string_lossy()
                    .into_owned()
            }
            "--english" => english_path = take(&mut i),
            "--no-english" => no_english = true,
            "--lm" => lm_path = take(&mut i),
            "--no-lm" => no_lm = true,
            "--context" => context = take(&mut i),
            "--max" => max_candidates = take(&mut i).parse().ok(),
            "--show" => show = take(&mut i).parse().unwrap_or(18),
            "--shuangpin" => {
                let name = take(&mut i);
                scheme = Some(
                    wowtype_core::shuangpin::Scheme::from_name(&name)
                        .unwrap_or_else(|| fail(&format!("不认识的双拼方案 {name}"))),
                );
            }
            "--scores" => scores = true,
            "--legacy" => legacy = true,
            "-h" | "--help" => {
                println!("wowtype-show (--data <目录> | --lexicon <tsv>) [--lm <words.lm> | --no-lm] [--english <txt> | --no-english]");
                println!("             [--max n] [--show n] [--context 文本] [--shuangpin 方案] [--scores] [--legacy] 拼音...");
                return;
            }
            other => inputs.push(other.to_string()),
        }
        i += 1;
    }
    if lexicon_path.is_empty() {
        fail("需要 --lexicon 或 --data");
    }
    if !legacy {
        if lm_path.is_empty() && !no_lm {
            lm_path = preset::sibling(&lexicon_path, "words.lm").unwrap_or_default();
        }
        if english_path.is_empty() && !no_english {
            english_path = preset::sibling(&lexicon_path, "english.txt").unwrap_or_default();
        }
    }
    if no_lm {
        lm_path.clear();
    }
    if no_english {
        english_path.clear();
    }
    let lexicon = MemoryLexicon::from_tsv_path_cached(&lexicon_path)
        .unwrap_or_else(|e| fail(&format!("词库载入失败: {e:#}")));
    let lex: Arc<dyn Lexicon> = Arc::new(lexicon);
    let mut config = if legacy {
        EngineConfig {
            max_candidates: 18,
            ..EngineConfig::default()
        }
    } else {
        preset::shell_config()
    };
    if let Some(n) = max_candidates {
        config.max_candidates = n.max(1);
    }
    if let Some(s) = scheme {
        config.shuangpin = true;
        config.shuangpin_scheme = s;
    }
    let lm: Arc<dyn wowtype_core::lm::LanguageModel> = if !lm_path.is_empty() {
        wowtype_core::lm::load_lm(&lm_path)
            .unwrap_or_else(|e| fail(&format!("语言模型载入失败: {e:#}")))
    } else {
        Arc::new(BigramLm::default())
    };
    let engine = Engine::new(lex, lm).with_config(config);
    engine.prepare_fuzzy();
    if !english_path.is_empty() {
        engine
            .load_english(&english_path)
            .unwrap_or_else(|e| fail(&format!("英文词表载入失败: {e:#}")));
    }
    engine.set_context(if legacy {
        context.as_str()
    } else {
        preset::context_tail(&context)
    });
    for input in &inputs {
        let candidates = engine.input(input);
        let line: Vec<String> = candidates
            .iter()
            .take(show.max(1))
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
        // 编辑区的分词和插件一样：按第一个用完整个输入的候选切；双拼显示翻成的全拼
        let shown = engine.shuangpin_display(input).unwrap_or_else(|| {
            candidates
                .iter()
                .find(|c| c.consumed >= input.len())
                .and_then(|c| wowtype_core::preedit::segment_input(input, &c.pinyin))
                .unwrap_or_else(|| input.clone())
        });
        let more = if candidates.len() > show {
            format!(" …（共 {} 个）", candidates.len())
        } else {
            String::new()
        };
        println!("{input} [{shown}]: {}{more}", line.join(" "));
    }
}
