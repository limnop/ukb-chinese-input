//! 词库压测：载入耗时与整条输入的延迟。
//!
//! 合成词库：cargo run --release -p wowtype-core --bin wowtype-bench
//! 真实词库：cargo run --release -p wowtype-core --bin wowtype-bench -- --lexicon lexicon.tsv
//! 逐键延迟：... -- --lexicon lexicon.tsv --typing cases.tsv
//!   （cases.tsv 每行第一列是输入，逐个前缀模拟打字，统计每次按键的耗时分布）
//!   加 --lm words.lm 带上词级语言模型，--nnlm nlrgpt2_nnlm.bin 带上神经语言模型重排；加 --no-prefix 关掉句首词候选，对比它的开销；--max N 设候选总数（输入法里是 300）

use std::sync::Arc;
use std::time::Instant;

use wowtype_core::lm::BigramLm;
use wowtype_core::{Engine, EngineConfig, Lexicon, MemoryLexicon, SYLLABLES};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut lexicon_path: Option<String> = None;
    let mut total = 100_000usize;
    let mut typing_path: Option<String> = None;
    let mut no_prefix = false;
    let mut max_candidates: Option<usize> = None;
    let mut nnlm_path: Option<String> = None;
    let mut lm_path: Option<String> = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--lexicon" => {
                i += 1;
                lexicon_path = args.get(i).cloned();
            }
            "--typing" => {
                i += 1;
                typing_path = args.get(i).cloned();
            }
            "--no-prefix" => no_prefix = true,
            "--lm" => {
                i += 1;
                lm_path = args.get(i).cloned();
            }
            "--nnlm" => {
                i += 1;
                nnlm_path = args.get(i).cloned();
            }
            "--max" => {
                i += 1;
                max_candidates = args.get(i).and_then(|v| v.parse().ok());
            }
            other => {
                if let Ok(n) = other.parse() {
                    total = n;
                } else {
                    eprintln!("未知参数: {other}");
                    std::process::exit(2);
                }
            }
        }
        i += 1;
    }

    let t0 = Instant::now();
    let (lexicon, label) = match &lexicon_path {
        Some(path) => {
            let lex = MemoryLexicon::from_tsv_path_cached(path).expect("载入词库失败");
            (lex, path.clone())
        }
        None => {
            // 合成词库：音节序列尽量分散，模拟真实分布
            let mut tsv = String::with_capacity(total * 24);
            let n = SYLLABLES.len();
            for k in 0..total {
                let a = SYLLABLES[k % n];
                let b = SYLLABLES[(k * 7 + 3) % n];
                tsv.push_str(a);
                tsv.push(' ');
                tsv.push_str(b);
                tsv.push('\t');
                tsv.push_str(&format!("词{k}"));
                tsv.push('\t');
                tsv.push_str(&format!("{}", 1000 + (k % 5000)));
                tsv.push('\n');
            }
            (
                MemoryLexicon::from_tsv_str(&tsv).expect("构造词库失败"),
                format!("合成 {total} 条"),
            )
        }
    };
    let load = t0.elapsed();
    println!(
        "载入 {label}：{} 条，{:?}（{:.1} 万条/秒）",
        lexicon.len(),
        load,
        lexicon.len() as f64 / load.as_secs_f64() / 10_000.0
    );

    let lm: Arc<dyn wowtype_core::LanguageModel> = match &lm_path {
        Some(path) => {
            let started = Instant::now();
            let lm = wowtype_core::lm::load_lm(path).expect("载入语言模型");
            println!("语言模型载入 {:?}", started.elapsed());
            lm
        }
        None => Arc::new(BigramLm::default()),
    };
    let lex: Arc<dyn Lexicon> = Arc::new(lexicon);
    let mut config = EngineConfig::default();
    if let Some(max) = max_candidates {
        config.max_candidates = max;
    }
    if no_prefix {
        config.prefix_candidates = 0;
    }
    let engine = Engine::new(lex, lm).with_config(config);
    if let Some(path) = &nnlm_path {
        let started = Instant::now();
        engine.load_nnlm(path).expect("载入神经语言模型");
        println!("神经语言模型载入 {:?}", started.elapsed());
    }

    if let Some(path) = typing_path {
        typing_latency(&engine, &path);
        return;
    }

    let queries = ["a", "zhong", "zhongguo", "nihao", "shurufa", "zhongg", "zg"];
    let rounds = 50;
    let mut worst = 0.0f64;
    for q in queries {
        let t = Instant::now();
        for _ in 0..rounds {
            let _ = engine.input(q);
        }
        let per = t.elapsed().as_secs_f64() * 1000.0 / rounds as f64;
        worst = worst.max(per);
        println!("input({q:>10}) : {per:8.3} ms/次");
    }
    println!("最慢一次输入：{worst:.3} ms");
}

/// 模拟逐键输入：对每条用例的每个前缀计时。
fn typing_latency(engine: &Engine, path: &str) {
    let data = std::fs::read_to_string(path).expect("读取用例失败");
    let mut samples: Vec<(f64, String)> = Vec::new();
    for line in data.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let input = line.split('\t').next().unwrap_or("");
        for end in 1..=input.len() {
            let prefix = &input[..end];
            let t = Instant::now();
            let _ = engine.input(prefix);
            samples.push((t.elapsed().as_secs_f64() * 1000.0, prefix.to_string()));
        }
    }
    if samples.is_empty() {
        println!("没有用例");
        return;
    }
    samples.sort_by(|a, b| a.0.total_cmp(&b.0));
    let pick = |q: f64| samples[((samples.len() - 1) as f64 * q) as usize].0;
    let mean = samples.iter().map(|s| s.0).sum::<f64>() / samples.len() as f64;
    println!(
        "按键 {} 次  平均 {:.2} ms  p50 {:.2}  p95 {:.2}  p99 {:.2}  最慢 {:.2} ms",
        samples.len(),
        mean,
        pick(0.5),
        pick(0.95),
        pick(0.99),
        samples.last().map(|s| s.0).unwrap_or(0.0)
    );
    for (ms, prefix) in samples.iter().rev().take(5) {
        println!("  {ms:7.2} ms  {prefix}");
    }
}
