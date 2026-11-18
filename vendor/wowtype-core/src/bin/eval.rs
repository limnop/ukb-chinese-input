//! 排序评测：给定 (输入, 期望) 的 TSV，统计 top-1 / top-3 / top-9 命中率。
//!
//! ```
//! # 默认用「外壳」配置：候选上限 1000、词库旁边的 words.lm 和 english.txt、纠错开、模糊音关……
//! #（逐项见 src/bin/common/preset.rs），和 fcitx5 插件、macOS 默认设置下一样
//! cargo run --release -p wowtype-core --bin wowtype-eval -- \
//!     --data <数据目录> --eval tools/eval/common.tsv [--eval 另一组.tsv ...]
//!
//! # 和基线比（CI 用的就是这个，见 scripts/eval.sh、tools/eval/README.md）
//! wowtype-eval --data <数据目录> --eval ... --strict regress \
//!     --baseline tools/eval/baseline.tsv [--markdown 汇总.md]
//!
//! # 旧的默认（候选上限 10，不自动带 words.lm / english.txt，上文不截）
//! wowtype-eval --legacy --lexicon lexicon.tsv --eval cases.tsv [--lm words.lm] [--weights coverage_score=1.5,prob=0]
//! ```
//!
//! 用例格式：`输入<TAB>期望[<TAB>上文[<TAB>属性]]`，`#` 开头是注释。
//! - 期望：多个可接受答案用 `|` 分隔。
//! - 上文（可选）：光标前已经上屏的文字（`tools/make_sentence_eval.py --context` 生成）。没有上文、要写属性时留空。
//! - 属性（可选，空格或逗号分隔）：
//!   - `top=N`：期望在前 N 名就算通过（默认 `top=1`）；
//!   - `at=N`：期望正好在第 N 名；
//!   - `before-single`：期望里列的**每一个**词都排在第一个单字候选之前（同音词排序）；
//!   - `sp=方案`：这一条按双拼打（`xiaohe`、`ziranma`……，见 `shuangpin::Scheme::from_name`）。
//!
//! 每组（每个用例文件，组名是文件名去掉 .tsv）报 top1 / top3 / top9 和「通过」（按上面的属性判断；
//! 没写属性的用例「通过」就是 top1）。这是调排序权重时唯一靠谱的依据——没有数据就是盲调。

#[path = "common/preset.rs"]
mod preset;

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;
use std::sync::Arc;

use wowtype_core::lm::{ArpaLm, BigramLm, LanguageModel, MixedLm};
use wowtype_core::rank::{FeatureWeights, LinearRanker};
use wowtype_core::shuangpin::Scheme;
use wowtype_core::{Engine, EngineConfig, Lexicon, MemoryLexicon};

fn fail(msg: &str) -> ! {
    eprintln!("{msg}");
    std::process::exit(1);
}

/// 一条用例怎么算通过。
#[derive(Clone, Copy, Debug, PartialEq)]
enum Check {
    /// 期望在前 N 名。
    Top(usize),
    /// 期望正好在第 N 名（从 1 数）。
    At(usize),
    /// 期望里的每个词都在第一个单字候选前面。
    BeforeSingle,
}

struct Case {
    line_no: usize,
    input: String,
    accepted: Vec<String>,
    context: String,
    check: Check,
    scheme: Option<Scheme>,
}

fn parse_cases(path: &str, data: &str) -> Vec<Case> {
    let mut cases = Vec::new();
    for (index, line) in data.lines().enumerate() {
        let line = line.trim_end_matches(['\r', '\n']);
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let mut cols = line.split('\t');
        let (Some(input), Some(expected)) = (cols.next(), cols.next()) else {
            continue;
        };
        let accepted: Vec<String> = expected
            .split('|')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        if input.trim().is_empty() || accepted.is_empty() {
            fail(&format!("{path}:{}: 输入或期望是空的", index + 1));
        }
        let context = cols.next().unwrap_or("").to_string();
        let mut check = Check::Top(1);
        let mut scheme = None;
        for attr in cols
            .next()
            .unwrap_or("")
            .split([' ', ','])
            .filter(|s| !s.is_empty())
        {
            let bad = || -> ! { fail(&format!("{path}:{}: 不认识的属性 {attr}", index + 1)) };
            let number = |v: &str| {
                v.parse::<usize>()
                    .ok()
                    .filter(|n| *n >= 1)
                    .unwrap_or_else(|| bad())
            };
            if attr == "before-single" {
                check = Check::BeforeSingle;
            } else if let Some(v) = attr.strip_prefix("top=") {
                check = Check::Top(number(v));
            } else if let Some(v) = attr.strip_prefix("at=") {
                check = Check::At(number(v));
            } else if let Some(v) = attr
                .strip_prefix("sp=")
                .or_else(|| attr.strip_prefix("shuangpin="))
            {
                scheme = Some(Scheme::from_name(v).unwrap_or_else(|| bad()));
            } else {
                bad();
            }
        }
        cases.push(Case {
            line_no: index + 1,
            input: input.trim().to_string(),
            accepted,
            context,
            check,
            scheme,
        });
    }
    cases
}

/// 一组用例的统计。
#[derive(Default, Clone)]
struct Stats {
    total: usize,
    top1: usize,
    top3: usize,
    topk: usize,
    top9: usize,
    pass: usize,
}

impl Stats {
    fn pct(&self, n: usize) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            n as f64 * 100.0 / self.total as f64
        }
    }
}

struct GroupResult {
    name: String,
    stats: Stats,
    /// 没通过的用例（已经排好版的一行）。
    failures: Vec<String>,
    /// 没进前 k（`--k`）的用例，单文件时照旧打出来。
    misses: Vec<String>,
    causes: BTreeMap<&'static str, usize>,
}

/// 基线文件里的一行。
struct BaselineRow {
    total: usize,
    top1: usize,
    pass: usize,
}

struct Baseline {
    data: Option<String>,
    rows: BTreeMap<String, BaselineRow>,
}

fn read_baseline(path: &str) -> Baseline {
    let text =
        std::fs::read_to_string(path).unwrap_or_else(|e| fail(&format!("读不了基线 {path}: {e}")));
    let mut baseline = Baseline {
        data: None,
        rows: BTreeMap::new(),
    };
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("# 数据\t") {
            baseline.data = Some(rest.trim().to_string());
            continue;
        }
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() < 6 {
            fail(&format!("基线 {path} 的这一行列数不对：{line}"));
        }
        let num = |s: &str| -> usize {
            s.parse()
                .unwrap_or_else(|_| fail(&format!("基线 {path} 的这一行不是数字：{line}")))
        };
        baseline.rows.insert(
            cols[0].to_string(),
            BaselineRow {
                total: num(cols[1]),
                top1: num(cols[2]),
                pass: num(cols[5]),
            },
        );
    }
    baseline
}

/// 数据目录的版本：`VERSION`（词库包版本）和 `SOURCE.txt` 里 rime-ice 的提交。
fn data_identity(lexicon: &str) -> Option<String> {
    let dir = Path::new(lexicon).parent()?;
    let version = std::fs::read_to_string(dir.join("VERSION")).ok()?;
    let commit = std::fs::read_to_string(dir.join("SOURCE.txt"))
        .ok()
        .and_then(|s| {
            s.lines()
                .find_map(|l| l.strip_prefix("版本："))
                .map(|l| l.split('（').next().unwrap_or(l).trim().to_string())
        })
        .unwrap_or_default();
    Some(format!("{}\t{}", version.trim(), commit))
}

fn group_name(path: &str) -> String {
    Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut lexicon_path = String::new();
    let mut eval_paths: Vec<String> = Vec::new();
    let mut lm_path: Option<String> = None;
    let mut domain_lm_path: Option<String> = None;
    let mut domain_lm_weight = wowtype_core::lm::DOMAIN_LM_WEIGHT;
    let mut arpa_path: Option<String> = None;
    let mut beam: Option<usize> = None;
    let mut max_candidates: Option<usize> = None;
    let mut top_k = 3usize;
    let mut weights = FeatureWeights::default();
    let mut fuzzy = false;
    let mut shuangpin = false;
    let mut scheme: Option<Scheme> = None;
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
    let mut legacy = false;
    let mut no_lm = false;
    let mut no_english = false;
    let mut baseline_path: Option<String> = None;
    let mut tolerance = 0.5f64;
    let mut write_baseline: Option<String> = None;
    let mut markdown_path: Option<String> = None;
    let mut strict: Vec<String> = Vec::new();
    let mut list_misses = false;

    let mut i = 1;
    while i < args.len() {
        let value = |i: &mut usize, name: &str| -> String {
            *i += 1;
            args.get(*i)
                .cloned()
                .unwrap_or_else(|| fail(&format!("{name} 需要一个值")))
        };
        match args[i].as_str() {
            "--lexicon" => lexicon_path = value(&mut i, "--lexicon"),
            "--data" => {
                let dir = value(&mut i, "--data");
                lexicon_path = Path::new(&dir)
                    .join("lexicon.tsv")
                    .to_string_lossy()
                    .into_owned();
            }
            "--eval" => eval_paths.push(value(&mut i, "--eval")),
            "--lm" => lm_path = Some(value(&mut i, "--lm")),
            "--no-lm" => no_lm = true,
            "--domain-lm" => domain_lm_path = Some(value(&mut i, "--domain-lm")),
            "--domain-lm-weight" => {
                domain_lm_weight = value(&mut i, "--domain-lm-weight")
                    .parse()
                    .unwrap_or(domain_lm_weight)
            }
            "--arpa" => arpa_path = Some(value(&mut i, "--arpa")),
            "--beam" => beam = value(&mut i, "--beam").parse().ok(),
            "--max" => max_candidates = value(&mut i, "--max").parse().ok(),
            "--k" => top_k = value(&mut i, "--k").parse().unwrap_or(3).max(1),
            "--weights" => {
                let spec = value(&mut i, "--weights");
                if let Err(e) = weights.apply_pairs(&spec) {
                    fail(&e);
                }
            }
            "--abbrev-cost" => abbrev_cost = value(&mut i, "--abbrev-cost").parse().ok(),
            "--tail-abbrev-cost" => {
                tail_abbrev_cost = value(&mut i, "--tail-abbrev-cost").parse().ok()
            }
            "--typo-cost" => typo_cost = value(&mut i, "--typo-cost").parse().ok(),
            "--completion-cost" => {
                completion_cost = value(&mut i, "--completion-cost").parse().ok()
            }
            "--no-typo" => no_typo = true,
            "--english" => english_path = Some(value(&mut i, "--english")),
            "--no-english" => no_english = true,
            "--show-top1" => show_top1 = true,
            "--nnlm" => nnlm_path = Some(value(&mut i, "--nnlm")),
            "--lm-weight" => lm_weight = value(&mut i, "--lm-weight").parse().ok(),
            "--nnlm-rerank" => nnlm_rerank = value(&mut i, "--nnlm-rerank").parse().ok(),
            "--learn" => learn = true,
            "--learn-from" => {
                learn_from = value(&mut i, "--learn-from");
                learn = true;
            }
            "--fuzzy" => fuzzy = true,
            "--shuangpin" => shuangpin = true,
            "--scheme" => {
                let name = value(&mut i, "--scheme");
                scheme = Some(
                    Scheme::from_name(&name)
                        .unwrap_or_else(|| fail(&format!("不认识的双拼方案 {name}"))),
                );
            }
            "--legacy" => legacy = true,
            "--preset" => match value(&mut i, "--preset").as_str() {
                "shell" => legacy = false,
                "legacy" => legacy = true,
                other => fail(&format!("不认识的预设 {other}（shell 或 legacy）")),
            },
            "--baseline" => baseline_path = Some(value(&mut i, "--baseline")),
            "--tolerance" => {
                tolerance = value(&mut i, "--tolerance")
                    .parse()
                    .unwrap_or_else(|_| fail("--tolerance 要一个数（百分点）"))
            }
            "--write-baseline" => write_baseline = Some(value(&mut i, "--write-baseline")),
            "--markdown" => markdown_path = Some(value(&mut i, "--markdown")),
            "--strict" => strict.push(value(&mut i, "--strict")),
            "--misses" => list_misses = true,
            "-h" | "--help" => {
                println!("wowtype-eval (--data <目录> | --lexicon <tsv>) --eval <tsv> [--eval <tsv> ...]");
                println!("  配置：默认和外壳一致（候选上限 1000、词库旁边的 words.lm / english.txt、纠错开、模糊音关）");
                println!("            [--legacy | --preset shell|legacy] [--lm|--arpa <file>] [--no-lm] [--english <file>] [--no-english]");
                println!("            [--max n] [--beam n] [--weights name=v,...] [--fuzzy] [--shuangpin [--scheme 方案]] [--domain-lm <file> [--domain-lm-weight w]]");
                println!("            [--typo-cost x] [--completion-cost x] [--abbrev-cost x] [--tail-abbrev-cost x] [--no-typo]");
                println!("            [--nnlm <file>] [--nnlm-rerank n] [--lm-weight w] [--learn | --learn-from <tsv>]");
                println!("  输出：[--k n] [--show-top1] [--misses]");
                println!("  门槛：[--strict 组名]... [--baseline <tsv> [--tolerance 百分点]] [--write-baseline <tsv>] [--markdown <file>]");
                return;
            }
            other => fail(&format!("未知参数: {other}")),
        }
        i += 1;
    }

    if lexicon_path.is_empty() || eval_paths.is_empty() {
        fail("需要 --lexicon（或 --data）与 --eval");
    }
    if learn && eval_paths.len() > 1 {
        fail("--learn 一次只测一个用例文件");
    }

    // 外壳在数据目录里找 words.lm、english.txt；旧的默认只用命令行给的
    if !legacy {
        if lm_path.is_none() && arpa_path.is_none() && !no_lm {
            lm_path = preset::sibling(&lexicon_path, "words.lm");
        }
        if english_path.is_none() && !no_english {
            english_path = preset::sibling(&lexicon_path, "english.txt");
        }
    }

    let lexicon = match MemoryLexicon::from_tsv_path_cached(&lexicon_path) {
        Ok(v) => v,
        Err(e) => fail(&format!("{lexicon_path}: {e:#}")),
    };
    let lm: Arc<dyn LanguageModel> = if let Some(path) = &arpa_path {
        match ArpaLm::from_arpa_path(path) {
            Ok(v) => Arc::new(v),
            Err(e) => fail(&format!("{e:#}")),
        }
    } else if let Some(path) = lm_path.as_ref().filter(|_| !no_lm) {
        match wowtype_core::lm::load_lm(path) {
            Ok(v) => v,
            Err(e) => fail(&format!("{e:#}")),
        }
    } else {
        Arc::new(BigramLm::default())
    };

    // 领域模型：和插件里一样混在通用模型上（插件的权重固定是 DOMAIN_LM_WEIGHT）
    let lm: Arc<dyn LanguageModel> = match &domain_lm_path {
        Some(path) => {
            let domain =
                wowtype_core::lm::load_lm(path).unwrap_or_else(|e| fail(&format!("{e:#}")));
            Arc::new(MixedLm::new(lm, domain, domain_lm_weight))
        }
        None => lm,
    };
    let lex: Arc<dyn Lexicon> = Arc::new(lexicon);
    let ranker = Arc::new(LinearRanker::new(weights));
    let mut config = if legacy {
        EngineConfig {
            max_candidates: preset::LEGACY_MAX_CANDIDATES,
            ..EngineConfig::default()
        }
    } else {
        preset::shell_config()
    };
    if let Some(n) = max_candidates {
        config.max_candidates = n.max(1);
    }
    config.fuzzy |= fuzzy;
    config.shuangpin |= shuangpin;
    if let Some(s) = scheme {
        config.shuangpin_scheme = s;
    }
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
    let base_shuangpin = (config.shuangpin, config.shuangpin_scheme);
    eprintln!(
        "配置：{}，候选上限 {}，语言模型 {}，英文词表 {}，纠错{}，模糊音{}，{}",
        if legacy {
            "旧默认（--legacy）"
        } else {
            "外壳"
        },
        config.max_candidates,
        arpa_path
            .as_deref()
            .or(lm_path.as_deref().filter(|_| !no_lm))
            .unwrap_or("无"),
        english_path.as_deref().unwrap_or("无"),
        if config.typo_correction { "开" } else { "关" },
        if config.fuzzy { "开" } else { "关" },
        if config.shuangpin {
            format!("双拼 {}", config.shuangpin_scheme.name())
        } else {
            "全拼".to_string()
        }
    );
    let mut engine = Engine::new(lex, lm).with_config(config).with_ranker(ranker);
    engine.prepare_fuzzy();
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

    let mut results: Vec<GroupResult> = Vec::new();
    for eval_path in &eval_paths {
        let data = match std::fs::read_to_string(eval_path) {
            Ok(v) => v,
            Err(e) => fail(&format!("读取用例 {eval_path} 失败: {e}")),
        };
        if learn {
            replay(&engine, &data, &learn_from);
        }
        let cases = parse_cases(eval_path, &data);
        if cases.is_empty() {
            fail(&format!("{eval_path}：没有可用的用例"));
        }
        let result = run_group(
            &mut engine,
            group_name(eval_path),
            &cases,
            !legacy,
            top_k,
            show_top1,
            base_shuangpin,
        );
        results.push(result);
    }

    let single = results.len() == 1 && baseline_path.is_none() && markdown_path.is_none();
    let mut failed = false;
    let mut report = String::new();
    for r in &results {
        let s = &r.stats;
        let name = &r.name;
        if !single {
            println!("== {name}");
        }
        println!(
            "用例 {}  top1 {} ({:.1}%)  top{top_k} {} ({:.1}%)  top9 {} ({:.1}%)  通过 {} ({:.1}%)",
            s.total,
            s.top1,
            s.pct(s.top1),
            s.topk,
            s.pct(s.topk),
            s.top9,
            s.pct(s.top9),
            s.pass,
            s.pct(s.pass)
        );
        if !r.causes.is_empty() {
            let parts: Vec<String> = r.causes.iter().map(|(k, v)| format!("{k} {v}")).collect();
            println!("top1 被谁占了：{}", parts.join("  "));
        }
        let is_strict = strict.iter().any(|g| g == name);
        if is_strict || list_misses {
            if !r.failures.is_empty() {
                println!("没通过：");
                for line in &r.failures {
                    println!("{line}");
                }
            }
        } else if single && !r.misses.is_empty() {
            println!("未进前 {top_k}：");
            for line in &r.misses {
                println!("{line}");
            }
        }
    }

    // 汇总表（多组、比基线或者要写 Markdown 时）
    if !single {
        let baseline = baseline_path.as_deref().map(read_baseline);
        let current_data = data_identity(&lexicon_path);
        let mut verdicts: Vec<String> = Vec::new();
        let _ = writeln!(
            report,
            "| 组 | 用例 | top1 | top3 | top9 | 通过 | 基线 top1 | 差 | |"
        );
        let _ = writeln!(report, "|---|---:|---:|---:|---:|---:|---:|---:|---|");
        for r in &results {
            let s = &r.stats;
            let is_strict = strict.iter().any(|g| g == &r.name);
            let mut base_cell = "—".to_string();
            let mut diff_cell = "—".to_string();
            let mut mark = "";
            if is_strict && s.pass < s.total {
                failed = true;
                mark = "没过（必须全过）";
                verdicts.push(format!(
                    "{}：{} 条里有 {} 条没通过（这一组必须全过）",
                    r.name,
                    s.total,
                    s.total - s.pass
                ));
            } else if is_strict {
                mark = "全过";
            }
            if let Some(b) = baseline.as_ref().and_then(|b| b.rows.get(&r.name)) {
                let base = Stats {
                    total: b.total,
                    top1: b.top1,
                    pass: b.pass,
                    ..Stats::default()
                };
                let d_top1 = s.pct(s.top1) - base.pct(base.top1);
                let d_pass = s.pct(s.pass) - base.pct(base.pass);
                base_cell = format!("{:.1}%", base.pct(base.top1));
                diff_cell = format!("{d_top1:+.1}");
                if b.total != s.total {
                    verdicts.push(format!(
                        "{}：用例数 {} 和基线的 {} 不一样（用例集改了要重写基线）",
                        r.name, s.total, b.total
                    ));
                }
                if !is_strict && (d_top1 < -tolerance || d_pass < -tolerance) {
                    failed = true;
                    mark = "比基线低";
                    verdicts.push(format!(
                        "{}：top1 {:.1}% → {:.1}%（{d_top1:+.1}），通过 {:.1}% → {:.1}%（{d_pass:+.1}），超过允许的 {tolerance} 个百分点",
                        r.name,
                        base.pct(base.top1),
                        s.pct(s.top1),
                        base.pct(base.pass),
                        s.pct(s.pass)
                    ));
                } else if !is_strict && (d_top1 > tolerance || d_pass > tolerance) {
                    mark = "比基线高";
                    verdicts.push(format!(
                        "{}：top1 {d_top1:+.1}、通过 {d_pass:+.1} 个百分点，比基线好，可以更新基线（见 tools/eval/README.md）",
                        r.name
                    ));
                } else if !is_strict {
                    mark = "持平";
                }
            } else if baseline.is_some() && !is_strict {
                verdicts.push(format!("{}：基线里没有这一组", r.name));
            }
            let _ = writeln!(
                report,
                "| {} | {} | {:.1}% | {:.1}% | {:.1}% | {:.1}% | {base_cell} | {diff_cell} | {mark} |",
                r.name,
                s.total,
                s.pct(s.top1),
                s.pct(s.top3),
                s.pct(s.top9),
                s.pct(s.pass)
            );
        }
        if let Some(b) = &baseline {
            for name in b.rows.keys() {
                if !results.iter().any(|r| &r.name == name) {
                    verdicts.push(format!("{name}：基线里有，这次没跑"));
                }
            }
            if b.data.is_some() && b.data != current_data {
                verdicts.push(format!(
                    "数据和基线的不一样：基线 {}，这次 {}（数据换了要重写基线）",
                    b.data.as_deref().unwrap_or("?").replace('\t', " "),
                    current_data.as_deref().unwrap_or("未知").replace('\t', " ")
                ));
            }
        }
        if !verdicts.is_empty() {
            report.push('\n');
            for v in &verdicts {
                let _ = writeln!(report, "- {v}");
            }
        }
        // 必须全过的组没过时，把没过的用例写进汇总
        for r in &results {
            if strict.iter().any(|g| g == &r.name) && !r.failures.is_empty() {
                let _ = writeln!(report, "\n{} 没通过的用例：\n\n```", r.name);
                for line in &r.failures {
                    let _ = writeln!(report, "{}", line.trim_start());
                }
                let _ = writeln!(report, "```");
            }
        }
        println!();
        print!("{report}");
        if let Some(path) = &markdown_path {
            let mut md = String::from("## 排序评测\n\n");
            if let Some(data) = &current_data {
                let _ = writeln!(md, "数据：{}\n", data.replace('\t', "，rime-ice "));
            }
            md.push_str(&report);
            if failed {
                md.push_str("\n**没过门槛。**\n");
            }
            if let Err(e) = std::fs::write(path, md) {
                fail(&format!("写不了 {path}: {e}"));
            }
        }
    }

    if let Some(path) = &write_baseline {
        let mut out = String::new();
        out.push_str("# 排序评测基线：scripts/eval.sh 用当时的代码跑出来的（各列是条数）。\n");
        out.push_str("# 改了排序、要更新它：scripts/eval.sh <数据目录> --update-baseline，提交说明里贴前后对比（见 tools/eval/README.md）。\n");
        if let Some(data) = data_identity(&lexicon_path) {
            let _ = writeln!(out, "# 数据\t{data}");
        }
        out.push_str("# 组\t用例\ttop1\ttop3\ttop9\t通过\n");
        for r in &results {
            let s = &r.stats;
            let _ = writeln!(
                out,
                "{}\t{}\t{}\t{}\t{}\t{}",
                r.name, s.total, s.top1, s.top3, s.top9, s.pass
            );
        }
        if let Err(e) = std::fs::write(path, out) {
            fail(&format!("写不了 {path}: {e}"));
        }
        eprintln!("基线写到了 {path}");
    }

    if failed {
        eprintln!("没过门槛");
        std::process::exit(1);
    }
}

/// --learn：先把每个用例「打一遍、上屏正确答案」，再测。看用户用过一次以后能不能记住。
fn replay(engine: &Engine, data: &str, learn_from: &str) {
    let replay = if learn_from.is_empty() {
        data.to_string()
    } else {
        std::fs::read_to_string(learn_from)
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

fn run_group(
    engine: &mut Engine,
    name: String,
    cases: &[Case],
    shell: bool,
    top_k: usize,
    show_top1: bool,
    base_shuangpin: (bool, Scheme),
) -> GroupResult {
    let mut stats = Stats::default();
    let mut failures = Vec::new();
    let mut misses = Vec::new();
    // top1 没中时，排在第一的是什么东西抢走了位置
    let mut causes: BTreeMap<&'static str, usize> = BTreeMap::new();

    for case in cases {
        let input = case.input.as_str();
        if let Some(scheme) = case.scheme {
            engine.set_shuangpin(true);
            engine.set_shuangpin_scheme(scheme);
        }
        // 第三列（可选）是光标前已经上屏的文字：测「接着前文打」时的排序
        let context = if shell {
            preset::context_tail(&case.context)
        } else {
            case.context.as_str()
        };
        engine.set_context(context);
        let candidates = engine.input(input);
        if case.scheme.is_some() {
            engine.set_shuangpin(base_shuangpin.0);
            engine.set_shuangpin_scheme(base_shuangpin.1);
        }
        stats.total += 1;
        let texts: Vec<&str> = candidates.iter().map(|c| c.text.as_str()).collect();
        let accepted = &case.accepted;
        let rank = texts.iter().position(|t| accepted.iter().any(|a| a == t));
        let within = |n: usize| rank.is_some_and(|r| r < n);
        if within(1) {
            stats.top1 += 1;
        } else if let Some(first) = candidates.first() {
            if show_top1 {
                println!(
                    "  top1 未中 [{}] {input:<14} 期望 {:<6} 实际 {}",
                    miss_cause(input, first),
                    accepted.join("|"),
                    texts.iter().take(4).cloned().collect::<Vec<_>>().join(", ")
                );
            }
            *causes.entry(miss_cause(input, first)).or_insert(0) += 1;
        }
        if within(3) {
            stats.top3 += 1;
        }
        if within(9) {
            stats.top9 += 1;
        }
        if within(top_k) {
            stats.topk += 1;
        } else {
            misses.push(format!(
                "  {input:<12} 期望 {:<6} 实际 {}",
                accepted.join("|"),
                texts
                    .iter()
                    .take(12)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }

        let (pass, why) = match case.check {
            Check::Top(n) => (within(n), format!("前 {n}")),
            Check::At(n) => (rank == Some(n - 1), format!("第 {n}")),
            Check::BeforeSingle => {
                let first_single = texts
                    .iter()
                    .position(|t| t.chars().count() == 1)
                    .unwrap_or(texts.len());
                let late: Vec<String> = accepted
                    .iter()
                    .filter_map(|a| match texts.iter().position(|t| t == a) {
                        Some(p) if p < first_single => None,
                        Some(p) => Some(format!("{a}(第 {})", p + 1)),
                        None => Some(format!("{a}(没有)")),
                    })
                    .collect();
                (
                    late.is_empty(),
                    format!(
                        "都在第一个单字（第 {}）前，晚了：{}",
                        first_single + 1,
                        late.join(" ")
                    ),
                )
            }
        };
        if pass {
            stats.pass += 1;
        } else {
            let shown = match case.check {
                Check::Top(n) | Check::At(n) => n.max(9),
                Check::BeforeSingle => 12,
            };
            let mut line = format!("  {name}:{} {input}", case.line_no);
            if !case.context.is_empty() {
                let _ = write!(line, "（上文「{}」）", case.context);
            }
            if let Some(s) = case.scheme {
                let _ = write!(line, "（{}）", s.name());
            }
            let _ = write!(
                line,
                " 期望 {} {why}；实际{} {}",
                accepted.join("|"),
                rank.map(|r| format!("第 {}", r + 1))
                    .unwrap_or_else(|| "没有".into()),
                texts
                    .iter()
                    .take(shown)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            failures.push(line);
        }
    }

    GroupResult {
        name,
        stats,
        failures,
        misses,
        causes,
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
