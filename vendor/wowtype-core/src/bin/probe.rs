//! 逐键输入探针：模拟一个字母一个字母打，看目标词在每个前缀位置是否还在候选里。
//!
//! ```
//! wowtype-probe --lexicon lexicon.tsv --cases cases.tsv [--max 10]
//! ```
//!
//! cases.tsv 是 `拼音<TAB>词`。对每个用例从第 2 个字母开始逐位截断，
//! 统计「目标词还在不在前 N 个候选里」。这个指标能发现「打到一半词就消失」
//! 这类只在实际输入时才会遇到的问题。

use std::collections::BTreeMap;
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
    let mut cases_path = String::new();
    let mut max_candidates = 10usize;
    let mut worst_show = 10usize;

    let mut i = 1;
    while i < args.len() {
        let take = |i: &mut usize| -> String {
            *i += 1;
            args.get(*i).cloned().unwrap_or_default()
        };
        match args[i].as_str() {
            "--lexicon" => lexicon_path = take(&mut i),
            "--cases" => cases_path = take(&mut i),
            "--max" => max_candidates = take(&mut i).parse().unwrap_or(10),
            "--show" => worst_show = take(&mut i).parse().unwrap_or(10),
            "-h" | "--help" => {
                println!("wowtype-probe --lexicon <tsv> --cases <tsv> [--max n] [--show n]");
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
    let lex: Arc<dyn Lexicon> = Arc::new(lexicon);
    let config = EngineConfig {
        max_candidates,
        ..EngineConfig::default()
    };
    let engine = Engine::new(lex, Arc::new(BigramLm::default())).with_config(config);

    let data = std::fs::read_to_string(&cases_path)
        .unwrap_or_else(|e| fail(&format!("读取用例失败: {e}")));

    // 前缀长度 -> (用例数, 命中数)
    let mut stats: BTreeMap<usize, (usize, usize)> = BTreeMap::new();
    let mut misses: Vec<String> = Vec::new();

    for line in data.lines() {
        let line = line.trim_end();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut cols = line.split('\t');
        let (Some(pinyin), Some(word)) = (cols.next(), cols.next()) else {
            continue;
        };
        let joined: String = pinyin.split_whitespace().collect();
        if joined.len() < 3 {
            continue;
        }
        // 真正值得关注的不是「短前缀时排不进去」（那是正常的),
        // 而是「已经出现过、再打一个字母反而消失了」——这说明补全被挤掉了。
        let mut seen_hit = false;
        let mut regressed_at: Option<usize> = None;
        for prefix_len in 2..=joined.len() {
            let prefix: String = joined.chars().take(prefix_len).collect();
            let candidates = engine.input(&prefix);
            let hit = candidates
                .iter()
                .take(max_candidates)
                .any(|c| c.text == word);
            let entry = stats.entry(prefix_len).or_default();
            entry.0 += 1;
            if hit {
                entry.1 += 1;
                seen_hit = true;
            } else if seen_hit && regressed_at.is_none() {
                regressed_at = Some(prefix_len);
            }
        }
        if let Some(pos) = regressed_at {
            if misses.len() < worst_show {
                misses.push(format!(
                    "  {word}（{pinyin}）在第 {pos} 个字母处掉了（之前出现过）"
                ));
            }
        }
    }

    println!("前缀长度  用例数  命中   比例");
    for (len, (total, hit)) in &stats {
        let pct = *hit as f64 * 100.0 / (*total).max(1) as f64;
        println!("{len:>8}  {total:>6}  {hit:>5}  {pct:5.1}%");
    }
    if !misses.is_empty() {
        println!("\n出现过又消失的例子：");
        for line in misses {
            println!("{line}");
        }
    }
}
