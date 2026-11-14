//! 运行时自检：确认词库、语言模型、简繁表、用户词等数据是否就位。
//!
//! ```
//! wowtype-check [--lexicon path] [--lm tsv] [--arpa file] [--s2t tsv] [--user-dict tsv]
//! ```
//!
//! 默认读与插件相同的环境变量（WOWTYPE_LEXICON / WOWTYPE_LM / WOWTYPE_S2T / WOWTYPE_USER_DICT），
//! 所以在 Linux 上装完后跑一遍就能知道配置有没有生效。

use std::path::Path;
use std::sync::Arc;

use wowtype_core::lm::{ArpaLm, BigramLm, LanguageModel};
use wowtype_core::{symbols, Engine, Lexicon, MemoryLexicon};

struct Report {
    ok: usize,
    warn: usize,
    fail: usize,
}

impl Report {
    fn new() -> Self {
        Self {
            ok: 0,
            warn: 0,
            fail: 0,
        }
    }
    fn ok(&mut self, what: &str, detail: &str) {
        self.ok += 1;
        println!("  [ok]   {what}: {detail}");
    }
    fn warn(&mut self, what: &str, detail: &str) {
        self.warn += 1;
        println!("  [warn] {what}: {detail}");
    }
    fn fail(&mut self, what: &str, detail: &str) {
        self.fail += 1;
        println!("  [fail] {what}: {detail}");
    }
}

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name)
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| default.to_string())
}

fn main() {
    let mut lexicon = env_or("WOWTYPE_LEXICON", "/usr/share/wowtype/lexicon.tsv");
    let default_lm = if Path::new("/usr/share/wowtype/words.lm").exists() {
        "/usr/share/wowtype/words.lm"
    } else {
        ""
    };
    let mut lm = env_or("WOWTYPE_LM", default_lm);
    let mut arpa = env_or("WOWTYPE_ARPA", "");
    let mut s2t = env_or("WOWTYPE_S2T", "/usr/share/wowtype/s2t.tsv");
    let mut user_dict = env_or("WOWTYPE_USER_DICT", "");

    let args: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i < args.len() {
        let take = |i: &mut usize| -> String {
            *i += 1;
            args.get(*i).cloned().unwrap_or_default()
        };
        match args[i].as_str() {
            "--lexicon" => lexicon = take(&mut i),
            "--lm" => lm = take(&mut i),
            "--arpa" => arpa = take(&mut i),
            "--s2t" => s2t = take(&mut i),
            "--user-dict" => user_dict = take(&mut i),
            "-h" | "--help" => {
                println!("wowtype-check [--lexicon path] [--lm tsv] [--arpa file] [--s2t tsv] [--user-dict tsv]");
                return;
            }
            other => {
                eprintln!("未知参数: {other}");
                std::process::exit(2);
            }
        }
        i += 1;
    }

    let mut report = Report::new();
    println!("WowType 自检  (core {})", env!("CARGO_PKG_VERSION"));

    // 1. 词库（必需）
    println!("\n[词库] {lexicon}");
    let mut lexicon_len = 0usize;
    let lex: Arc<dyn Lexicon> = if !Path::new(&lexicon).exists() {
        report.fail("词库", "文件不存在；先跑 tools/rime2tsv.py 生成");
        Arc::new(MemoryLexicon::new())
    } else {
        match MemoryLexicon::from_tsv_path_cached(&lexicon) {
            Ok(v) => {
                lexicon_len = v.len();
                if v.len() == 0 {
                    report.warn("词库", "文件为空");
                } else {
                    report.ok(
                        "词库",
                        &format!(
                            "{} 条词，总频次 {:.0}，跳过 {} 行",
                            v.len(),
                            v.total_freq(),
                            v.skipped()
                        ),
                    );
                }
                Arc::new(v)
            }
            Err(e) => {
                report.fail("词库", &format!("解析失败: {e:#}"));
                Arc::new(MemoryLexicon::new())
            }
        }
    };

    // 2. 语言模型（可选）
    println!("\n[语言模型]");
    let lm_arc: Arc<dyn LanguageModel> = if !arpa.is_empty() {
        match ArpaLm::from_arpa_path(&arpa) {
            Ok(v) => {
                report.ok(
                    "ARPA",
                    &format!("{arpa}: {} 词 {} 二元组", v.vocab(), v.bigram_count()),
                );
                Arc::new(v)
            }
            Err(e) => {
                report.warn("ARPA", &format!("载入失败: {e:#}"));
                Arc::new(BigramLm::default())
            }
        }
    } else if !lm.is_empty() {
        match wowtype_core::lm::load_lm(&lm) {
            Ok(v) => {
                report.ok("语言模型", &format!("{lm}"));
                v
            }
            Err(e) => {
                report.warn("语言模型", &format!("载入失败: {e:#}"));
                Arc::new(BigramLm::default())
            }
        }
    } else {
        report.warn("语言模型", "未配置，排序只用词频");
        Arc::new(BigramLm::default())
    };

    let engine = Engine::new(lex, lm_arc);

    // 3. 简繁表（可选）
    println!("\n[简繁表] {s2t}");
    if Path::new(&s2t).exists() {
        match engine.load_conversion(&s2t) {
            Ok(n) => report.ok("简繁表", &format!("{n} 条映射")),
            Err(e) => report.warn("简繁表", &format!("载入失败: {e:#}")),
        }
    } else {
        report.warn("简繁表", "文件不存在，繁体模式不可用");
    }

    // 4. 用户词库（可选）
    println!("\n[用户词库]");
    if user_dict.is_empty() {
        report.warn(
            "用户词库",
            "未配置（可由插件自动使用 XDG_DATA_HOME/wowtype/user.tsv）",
        );
    } else if Path::new(&user_dict).exists() {
        match engine.load_user_dict(&user_dict) {
            Ok(n) => report.ok("用户词库", &format!("{user_dict}: {n} 条")),
            Err(e) => report.warn("用户词库", &format!("载入失败: {e:#}")),
        }
    } else {
        report.warn("用户词库", "文件不存在（首次使用时自动创建）");
    }

    // 5. 功能自检
    println!("\n[功能]");
    let cands = engine.input("shi");
    if lexicon_len == 0 {
        report.warn("基本输入", "词库为空，跳过");
    } else if cands.is_empty() {
        report.warn("基本输入", "shi 没有任何候选（词库可能不含这些音节）");
    } else {
        let top: Vec<&str> = cands.iter().take(3).map(|c| c.text.as_str()).collect();
        report.ok("基本输入", &format!("shi -> {}", top.join(" / ")));
    }

    let symbols = symbols::search("juhao", 3);
    if symbols.is_empty() {
        report.fail("符号表", "检索 juhao 失败");
    } else {
        report.ok(
            "符号表",
            &format!(
                "{} 个符号，juhao -> {}",
                symbols::SYMBOLS.len(),
                symbols[0].text
            ),
        );
    }

    match wowtype_core::shuangpin::encode(wowtype_core::shuangpin::Scheme::Ziranma, "zhong").first()
    {
        Some([a, b]) => report.ok("双拼", &format!("zhong -> {}{}", *a as char, *b as char)),
        None => report.fail("双拼", "zhong 无法编码"),
    }

    let canon = wowtype_core::syllable::fuzzy_canon("zhang");
    if canon == "zan" {
        report.ok("模糊音", "zhang -> zan");
    } else {
        report.fail("模糊音", &format!("zhang -> {canon}，期望 zan"));
    }

    println!(
        "\n结果：{} 项通过，{} 项警告，{} 项失败",
        report.ok, report.warn, report.fail
    );
    if report.fail > 0 {
        std::process::exit(1);
    }
}
