//! 词库压测：载入耗时与整条输入的延迟。
//!
//! 合成词库：cargo run --release -p wowtype-core --bin wowtype-bench
//! 真实词库：cargo run --release -p wowtype-core --bin wowtype-bench -- --lexicon lexicon.tsv
//! 混上领域词级模型：... -- --lexicon lexicon.tsv --lm words.lm --domain-lm words.domain.lm
//! 逐键延迟：... -- --lexicon lexicon.tsv --typing cases.tsv
//!   （cases.tsv 每行第一列是输入，逐个前缀模拟打字，统计每次按键的耗时分布）
//!   加 --lm words.lm 带上词级语言模型，--nnlm nlrgpt2_nnlm.bin 带上神经语言模型重排；加 --no-prefix 关掉句首词候选，对比它的开销；--max N 设候选总数（输入法里是 300）；
//!   --s2t s2t.tsv 载入简繁表并打开繁体输出
//!   --fuzzy 分组 打开模糊音（分组是 syllable::fuzzy 的位掩码，比如 0xFFFF 是全部分组）
//! 五笔：... -- --lexicon lexicon.tsv --wubi wubi86.tsv
//!   码表载入耗时和内存；按输入会话逐键打码表里的码（四码唯一上屏、顶字都照真的走）和几句 z 临时拼音，
//!   统计每次按键（会话处理 + 取这一页候选）的耗时分布

use std::sync::Arc;
use std::time::Instant;

use wowtype_core::lm::BigramLm;
use wowtype_core::session::{Config, Key, Session};
use wowtype_core::{Engine, EngineConfig, Lexicon, MemoryLexicon, SYLLABLES};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut lexicon_path: Option<String> = None;
    let mut total = 100_000usize;
    let mut typing_path: Option<String> = None;
    let mut no_prefix = false;
    let mut domain_lm_path: Option<String> = None;
    let mut max_candidates: Option<usize> = None;
    let mut nnlm_path: Option<String> = None;
    let mut lm_path: Option<String> = None;
    let mut s2t_path: Option<String> = None;
    let mut fuzzy_groups: Option<u32> = None;
    let mut wubi_path: Option<String> = None;

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
            "--domain-lm" => {
                i += 1;
                domain_lm_path = args.get(i).cloned();
            }
            "--nnlm" => {
                i += 1;
                nnlm_path = args.get(i).cloned();
            }
            "--fuzzy" => {
                i += 1;
                fuzzy_groups = args.get(i).and_then(|v| {
                    v.strip_prefix("0x")
                        .map_or_else(|| v.parse().ok(), |hex| u32::from_str_radix(hex, 16).ok())
                });
            }
            "--max" => {
                i += 1;
                max_candidates = args.get(i).and_then(|v| v.parse().ok());
            }
            "--s2t" => {
                i += 1;
                s2t_path = args.get(i).cloned();
            }
            "--wubi" => {
                i += 1;
                wubi_path = args.get(i).cloned();
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
    if let Some(groups) = fuzzy_groups {
        config.fuzzy = true;
        config.fuzzy_groups = groups & wowtype_core::syllable::fuzzy::ALL;
    }
    config.traditional = s2t_path.is_some();
    let mut engine = Engine::new(lex, lm).with_config(config);
    if let Some(path) = &domain_lm_path {
        engine.set_domain_lm(Some(wowtype_core::lm::load_lm(path).expect("载入领域模型")));
    }
    if let Some(path) = &s2t_path {
        let started = Instant::now();
        let n = engine.load_conversion(path).expect("载入简繁表");
        println!("简繁表 {n} 条，载入 {:?}", started.elapsed());
    }
    // 和 wowtype_engine_new 一样在载入时建好模糊音索引，不算进第一次按键
    engine.prepare_fuzzy();
    if let Some(path) = &nnlm_path {
        let started = Instant::now();
        engine.load_nnlm(path).expect("载入神经语言模型");
        println!("神经语言模型载入 {:?}", started.elapsed());
    }

    if let Some(path) = wubi_path {
        wubi_latency(&engine, &path);
        return;
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

/// 五笔：码表载入，再按会话逐键打码（每 7 条取一条的码，加几句 z 临时拼音）。
fn wubi_latency(engine: &Engine, path: &str) {
    let started = Instant::now();
    let n = engine.load_wubi(path).expect("载入五笔码表");
    let table = engine.wubi().expect("码表");
    println!(
        "五笔码表 {n} 条，载入 {:?}，约占内存 {:.1} MB",
        started.elapsed(),
        table.heap_bytes() as f64 / 1048576.0
    );
    let data = std::fs::read_to_string(path).expect("读码表");
    let mut cases: Vec<String> = data
        .lines()
        .step_by(7)
        .filter_map(|line| line.split('\t').next().map(str::to_owned))
        .collect();
    cases.extend(["znihao", "zwoshizhongguoren", "zshurufa", "zriqi"].map(String::from));
    let mut session = Session::with_engine(engine);
    session.configure(Config { wubi: true, ..Config::default() });
    let mut samples: Vec<(f64, String)> = Vec::new();
    for code in &cases {
        for (i, c) in code.chars().enumerate() {
            let t = Instant::now();
            session.key(&Key::char(c));
            let _ = session.shown().len();
            samples.push((t.elapsed().as_secs_f64() * 1000.0, code[..=i].to_owned()));
        }
        session.reset(false);
    }
    samples.sort_by(|a, b| a.0.total_cmp(&b.0));
    let pick = |q: f64| samples[((samples.len() - 1) as f64 * q) as usize].0;
    println!(
        "五笔按键 {} 次  p50 {:.3} ms  p95 {:.3}  p99 {:.3}  最慢 {:.3} ms",
        samples.len(),
        pick(0.5),
        pick(0.95),
        pick(0.99),
        samples.last().map(|s| s.0).unwrap_or(0.0)
    );
    for (ms, keys) in samples.iter().rev().take(5) {
        println!("  {ms:7.3} ms  {keys}");
    }
}
