//! 排序评测：给定 (输入, 期望) 的 TSV，统计 top-1 / top-3 命中率。
//!
//! ```
//! cargo run --release -p wowtype-core --bin wowtype-eval -- \
//!     --lexicon lexicon.tsv --eval cases.tsv [--weights coverage_score=1.5,prob=0]
//! ```
//!
//! 用例格式：输入<TAB>期望词（多个可接受答案用 | 分隔）。
//! 这是调排序权重时唯一靠谱的依据——没有数据就是盲调。

use std::sync::Arc;

use wowtype_core::lm::{ArpaLm, BigramLm, LanguageModel};
use wowtype_core::rank::{FeatureWeights, LinearRanker};
use wowtype_core::{Engine, EngineConfig, Lexicon, MemoryLexicon};

fn fail(msg: &str) -> ! {
    eprintln!("{msg}");
    std::process::exit(1);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut lexicon_path = String::new();
    let mut eval_path = String::new();
    let mut lm_path: Option<String> = None;
    let mut arpa_path: Option<String> = None;
    let mut beam: Option<usize> = None;
    let mut max_candidates = 10usize;
    let mut top_k = 3usize;
    let mut weights = FeatureWeights::default();
    let mut fuzzy = false;
    let mut shuangpin = false;
    let mut typo_cost: Option<f64> = None;
    let mut abbrev_cost: Option<f64> = None;
    let mut tail_abbrev_cost: Option<f64> = None;
    let mut completion_cost: Option<f64> = None;
    let mut no_typo = false;
    let mut english_path: Option<String> = None;
    let mut show_top1 = false;
    let mut nnlm_path: Option<String> = None;
    let mut nnlm_rerank: Option<usize> = None;
    let mut lm_weight: Option<f64> = None;
    let mut learn = false;
    let mut learn_from = String::new();

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--lexicon" => {
                i += 1;
                lexicon_path = args
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| fail("--lexicon 需要路径"));
            }
            "--eval" => {
                i += 1;
                eval_path = args
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| fail("--eval 需要路径"));
            }
            "--lm" => {
                i += 1;
                lm_path = args.get(i).cloned();
            }
            "--arpa" => {
                i += 1;
                arpa_path = args.get(i).cloned();
            }
            "--beam" => {
                i += 1;
                beam = args.get(i).and_then(|v| v.parse().ok());
            }
            "--max" => {
                i += 1;
                max_candidates = args.get(i).and_then(|v| v.parse().ok()).unwrap_or(10);
            }
            "--k" => {
                i += 1;
                top_k = args.get(i).and_then(|v| v.parse().ok()).unwrap_or(3).max(1);
            }
            "--weights" => {
                i += 1;
                let spec = args.get(i).cloned().unwrap_or_default();
                if let Err(e) = weights.apply_pairs(&spec) {
                    fail(&e);
                }
            }
            "--abbrev-cost" => {
                i += 1;
                abbrev_cost = args.get(i).and_then(|v| v.parse().ok());
            }
            "--tail-abbrev-cost" => {
                i += 1;
                tail_abbrev_cost = args.get(i).and_then(|v| v.parse().ok());
            }
            "--typo-cost" => {
                i += 1;
                typo_cost = args.get(i).and_then(|v| v.parse().ok());
            }
            "--completion-cost" => {
                i += 1;
                completion_cost = args.get(i).and_then(|v| v.parse().ok());
            }
            "--no-typo" => no_typo = true,
            "--english" => {
                i += 1;
                english_path = args.get(i).cloned();
            }
            "--show-top1" => show_top1 = true,
            "--nnlm" => {
                i += 1;
                nnlm_path = args.get(i).cloned();
            }
            "--lm-weight" => {
                i += 1;
                lm_weight = args.get(i).and_then(|v| v.parse().ok());
            }
            "--nnlm-rerank" => {
                i += 1;
                nnlm_rerank = args.get(i).and_then(|v| v.parse().ok());
            }
            "--learn" => learn = true,
            "--learn-from" => {
                i += 1;
                learn_from = args.get(i).cloned().unwrap_or_default();
                learn = true;
            }
            "--fuzzy" => fuzzy = true,
            "--shuangpin" => shuangpin = true,
            "-h" | "--help" => {
                println!(
                    "wowtype-eval --lexicon <tsv> --eval <tsv> [--lm|--arpa <file>] [--beam n] [--k n]"
                );
                println!("            [--max n] [--weights name=v,...] [--fuzzy] [--shuangpin]");
                println!("            [--typo-cost x] [--completion-cost x] [--abbrev-cost x] [--tail-abbrev-cost x] [--no-typo] [--english <file>] [--show-top1]");
                return;
            }
            other => fail(&format!("未知参数: {other}")),
        }
        i += 1;
    }

    if lexicon_path.is_empty() || eval_path.is_empty() {
        fail("需要 --lexicon 与 --eval");
    }

    let lexicon = match MemoryLexicon::from_tsv_path_cached(&lexicon_path) {
        Ok(v) => v,
        Err(e) => fail(&format!("{e:#}")),
    };
    let lm: Arc<dyn LanguageModel> = if let Some(path) = &arpa_path {
        match ArpaLm::from_arpa_path(path) {
            Ok(v) => Arc::new(v),
            Err(e) => fail(&format!("{e:#}")),
        }
    } else if let Some(path) = &lm_path {
        match wowtype_core::lm::load_lm(path) {
            Ok(v) => v,
            Err(e) => fail(&format!("{e:#}")),
        }
    } else {
        Arc::new(BigramLm::default())
    };

    let lex: Arc<dyn Lexicon> = Arc::new(lexicon);
    let ranker = Arc::new(LinearRanker::new(weights));
    let mut config = EngineConfig {
        max_candidates,
        fuzzy,
        shuangpin,
        ..EngineConfig::default()
    };
    if let Some(size) = beam {
        config.beam.beam_size = size.max(1);
    }
    if let Some(cost) = abbrev_cost {
        config.beam.abbrev_cost = cost;
    }
    if let Some(cost) = tail_abbrev_cost {
        config.beam.tail_abbrev_cost = cost;
    }
    if let Some(cost) = typo_cost {
        config.beam.typo_cost = cost;
    }
    if let Some(cost) = completion_cost {
        config.beam.completion_cost = cost;
    }
    if no_typo {
        config.typo_correction = false;
    }
    if let Some(w) = lm_weight {
        config.beam.lm_weight = w;
    }
    if let Some(n) = nnlm_rerank {
        config.nnlm_rerank = n;
    }
    let engine = Engine::new(lex, lm).with_config(config).with_ranker(ranker);
    if let Some(path) = &nnlm_path {
        if let Err(e) = engine.load_nnlm(path) {
            fail(&format!("{e:#}"));
        }
    }
    if let Some(path) = &english_path {
        if let Err(e) = engine.load_english(path) {
            fail(&format!("{e:#}"));
        }
    }

    let data = match std::fs::read_to_string(&eval_path) {
        Ok(v) => v,
        Err(e) => fail(&format!("读取用例失败: {e}")),
    };

    // --learn：先把每个用例「打一遍、上屏正确答案」，再测。看用户用过一次以后能不能记住。
    if learn {
        let replay = if learn_from.is_empty() {
            data.clone()
        } else {
            std::fs::read_to_string(&learn_from)
                .unwrap_or_else(|e| fail(&format!("读不了 {learn_from}: {e}")))
        };
        for line in replay.lines() {
            let mut cols = line.trim_end().split('\t');
            let (Some(input), Some(expected)) = (cols.next(), cols.next()) else {
                continue;
            };
            if input.starts_with('#') {
                continue;
            }
            let expected = expected.split('|').next().unwrap_or("").trim();
            // 照插件里真实的选法：整句对就选整句；不对就选最长的、对得上的句首候选，
            // 剩下的拼音接着打；最后把拼出来的整句也记一次
            engine.set_context("");
            let mut rest = input.to_string();
            let mut done = String::new();
            for _ in 0..16 {
                let candidates = engine.input(&rest);
                let want = &expected[done.len().min(expected.len())..];
                if let Some(c) = candidates
                    .iter()
                    .find(|c| c.text == want && c.consumed >= rest.len())
                {
                    engine.record_selection(&c.text);
                    done.push_str(&c.text);
                    break;
                }
                let head = candidates
                    .iter()
                    .filter(|c| c.consumed < rest.len() && want.starts_with(c.text.as_str()))
                    .max_by_key(|c| c.text.len());
                let Some(head) = head else {
                    break;
                };
                engine.record_selection(&head.text);
                done.push_str(&head.text);
                rest = rest[head.consumed..].to_string();
                engine.set_context(&done);
            }
            if done == expected && done != input {
                engine.set_context("");
                engine.record_selection(expected);
            }
        }
        engine.set_context("");
    }

    let mut total = 0usize;
    let mut top1 = 0usize;
    let mut top3 = 0usize;
    let mut misses: Vec<String> = Vec::new();
    // top1 没中时，排在第一的是什么东西抢走了位置
    let mut causes: std::collections::BTreeMap<&'static str, usize> = Default::default();

    for line in data.lines() {
        let line = line.trim_end();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut cols = line.split('\t');
        let (Some(input), Some(expected)) = (cols.next(), cols.next()) else {
            continue;
        };
        let accepted: Vec<&str> = expected.split('|').map(str::trim).collect();
        let candidates = engine.input(input);
        total += 1;
        let texts: Vec<&str> = candidates.iter().map(|c| c.text.as_str()).collect();
        let hit1 = texts.first().map(|t| accepted.contains(t)).unwrap_or(false);
        if hit1 {
            top1 += 1;
        } else if let Some(first) = candidates.first() {
            if show_top1 {
                println!(
                    "  top1 未中 [{}] {input:<14} 期望 {expected:<6} 实际 {}",
                    miss_cause(input, first),
                    texts.iter().take(4).cloned().collect::<Vec<_>>().join(", ")
                );
            }
            causes
                .entry(miss_cause(input, first))
                .and_modify(|n| *n += 1)
                .or_insert(1);
        }
        let hit3 = texts.iter().take(top_k).any(|t| accepted.contains(t));
        if hit3 {
            top3 += 1;
        }
        if !hit3 {
            misses.push(format!(
                "  {input:<12} 期望 {:<6} 实际 {}",
                expected,
                texts.join(", ")
            ));
        }
    }

    if total == 0 {
        fail("没有可用的用例");
    }
    let pct = |n: usize| n as f64 * 100.0 / total as f64;
    println!(
        "用例 {total}  top1 {top1} ({:.1}%)  top{top_k} {top3} ({:.1}%)",
        pct(top1),
        pct(top3)
    );
    if !causes.is_empty() {
        let parts: Vec<String> = causes.iter().map(|(k, v)| format!("{k} {v}")).collect();
        println!("top1 被谁占了：{}", parts.join("  "));
    }
    if !misses.is_empty() {
        println!("未进前 {top_k}：");
        for line in misses {
            println!("{line}");
        }
    }
}

/// 排第一的候选是怎么来的（只在 top1 没中时统计）。
fn miss_cause(input: &str, first: &wowtype_core::Candidate) -> &'static str {
    if first.consumed < input.len() {
        "句首词"
    } else if first.words.len() == 1 && first.words[0].pinyin.is_empty() {
        "原始串/英文"
    } else if first.features.typo_count > 0.0 {
        "纠错"
    } else if first.features.prefix_completion > 0.0 {
        "补全"
    } else {
        "同音排序"
    }
}
