//! 用「输入 → 用户选中的词」拟合排序权重。
//!
//! ```
//! cargo run --release -p wowtype-core --bin wowtype-train -- \
//!     --lexicon lexicon.tsv --cases cases.tsv --epochs 300 --out weights.txt
//! ```
//!
//! cases.tsv 格式：输入<TAB>选中的词。工具会用当前引擎生成候选（含特征），
//! 找出被选中的那条，再用 softmax 交叉熵拟合权重。

use std::sync::Arc;

use wowtype_core::lm::{ArpaLm, BigramLm, LanguageModel};
use wowtype_core::rank::{FeatureWeights, FEATURE_NAMES};
use wowtype_core::train::{accuracy, train, Sample, TrainConfig};
use wowtype_core::{Engine, EngineConfig, Lexicon, MemoryLexicon};

fn fail(msg: &str) -> ! {
    eprintln!("{msg}");
    std::process::exit(1);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut lexicon_path = String::new();
    let mut cases_path = String::new();
    let mut out_path = String::new();
    let mut config = TrainConfig::default();
    let mut arpa: Option<String> = None;
    let mut lm_path: Option<String> = None;

    let mut i = 1;
    while i < args.len() {
        let take = |i: &mut usize| -> String {
            *i += 1;
            args.get(*i).cloned().unwrap_or_default()
        };
        match args[i].as_str() {
            "--lexicon" => lexicon_path = take(&mut i),
            "--cases" => cases_path = take(&mut i),
            "--out" => out_path = take(&mut i),
            "--arpa" => arpa = Some(take(&mut i)),
            "--lm" => lm_path = Some(take(&mut i)),
            "--epochs" => config.epochs = take(&mut i).parse().unwrap_or(config.epochs),
            "--lr" => config.learning_rate = take(&mut i).parse().unwrap_or(config.learning_rate),
            "--l2" => config.l2 = take(&mut i).parse().unwrap_or(config.l2),
            "-h" | "--help" => {
                println!("wowtype-train --lexicon <tsv> --cases <tsv> [--lm words.lm|--arpa model.arpa] [--out weights.txt] [--epochs n] [--lr v] [--l2 v]");
                return;
            }
            other => fail(&format!("未知参数: {other}")),
        }
        i += 1;
    }
    if lexicon_path.is_empty() || cases_path.is_empty() {
        fail("需要 --lexicon 与 --cases");
    }

    let lexicon = MemoryLexicon::from_tsv_path_cached(&lexicon_path)
        .unwrap_or_else(|e| fail(&format!("词库载入失败: {e:#}")));
    let lm: Arc<dyn LanguageModel> = if let Some(path) = &arpa {
        Arc::new(ArpaLm::from_arpa_path(path).unwrap_or_else(|e| fail(&format!("{e:#}"))))
    } else if let Some(path) = &lm_path {
        wowtype_core::lm::load_lm(path).unwrap_or_else(|e| fail(&format!("{e:#}")))
    } else {
        Arc::new(BigramLm::default())
    };
    let lex: Arc<dyn Lexicon> = Arc::new(lexicon);
    let engine_config = EngineConfig {
        max_candidates: 10,
        ..EngineConfig::default()
    };
    let engine = Engine::new(lex, lm).with_config(engine_config);

    let data = std::fs::read_to_string(&cases_path)
        .unwrap_or_else(|e| fail(&format!("读取用例失败: {e}")));

    let mut samples = Vec::new();
    let mut missing = 0usize;
    for line in data.lines() {
        let line = line.trim_end();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut cols = line.split('\t');
        let (Some(input), Some(want)) = (cols.next(), cols.next()) else {
            continue;
        };
        let candidates = engine.input(input);
        let features: Vec<_> = candidates.iter().map(|c| c.features).collect();
        match candidates.iter().position(|c| c.text == want) {
            Some(chosen) => samples.push(Sample { features, chosen }),
            None => missing += 1,
        }
    }

    if samples.is_empty() {
        fail("没有可用样本（选中的词都不在候选里）");
    }

    let initial = FeatureWeights::default();
    let before = accuracy(&samples, initial);
    let trained = train(&samples, config, initial);
    let after = accuracy(&samples, trained);

    println!(
        "样本 {}（{} 条目标词不在候选里，已跳过）",
        samples.len(),
        missing
    );
    println!(
        "训练前 top1 {:.1}%  训练后 top1 {:.1}%",
        before * 100.0,
        after * 100.0
    );

    let values = trained.to_array();
    let mut spec = String::new();
    for (name, value) in FEATURE_NAMES.iter().zip(values.iter()) {
        if !spec.is_empty() {
            spec.push(',');
        }
        spec.push_str(&format!("{name}={value:.4}"));
    }
    println!("\n权重（可用于 wowtype-eval --weights）:\n{spec}");

    if !out_path.is_empty() {
        match std::fs::write(&out_path, &spec) {
            Ok(()) => println!("已写入 {out_path}"),
            Err(e) => fail(&format!("写入失败: {e}")),
        }
    }
}
