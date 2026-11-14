//! 从语料训练词级二元语言模型（[`wowtype_core::wordlm`] 的格式）。
//!
//! ```
//! cargo run --release -p wowtype-core --bin wowtype-lmbuild -- \
//!     --lexicon lexicon.tsv --s2t s2t.tsv --out words.lm corpus1.txt corpus2.txt
//! ```
//!
//! 语料是纯文本，一行一段。繁体字按 s2t.tsv 反查转成简体；只看汉字，别的字符当断句。
//! 分词用词库：每段汉字取词频乘积最大的切法（最长 8 个字一个词）。
//! 句号、问号、叹号、分号、换行之后算句首（前一个词是 `<s>`）；逗号等只断开、不算句首。
//!
//! 选项：`--threads N`、`--min-count N`、`--max-bigrams N`、`--min-unigram N`、
//! `--max-chars N`（只读前 N 个字，试跑用）、`--save-counts 文件`（把计数存下来）、
//! `--from-counts 文件`（不读语料，用存下来的计数换参数重新出模型）。

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::io::{BufRead, BufReader};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Instant;

use wowtype_core::wordlm::{build, BuildConfig, Counts, BOS};

fn fail(msg: &str) -> ! {
    eprintln!("{msg}");
    std::process::exit(1);
}

/// u64 键用的快速哈希（FxHash 的乘法）。
#[derive(Default)]
struct Fx(u64);
impl Hasher for Fx {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0.rotate_left(5) ^ b as u64).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
        }
    }
    fn write_u64(&mut self, v: u64) {
        self.0 = (self.0.rotate_left(5) ^ v).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
}
type FxMap<K, V> = HashMap<K, V, BuildHasherDefault<Fx>>;

const MAX_WORD_CHARS: usize = 8;
const OOV: u32 = u32::MAX;

struct Dict {
    /// 词 -> (编号, ln 概率)
    words: HashMap<String, (u32, f64)>,
    names: Vec<String>,
    t2s: HashMap<char, char>,
}

fn is_han(c: char) -> bool {
    matches!(c, '\u{4e00}'..='\u{9fff}' | '\u{3400}'..='\u{4dbf}')
}

fn load_dict(lexicon: &str, s2t: Option<&str>) -> Dict {
    let text = std::fs::read_to_string(lexicon).unwrap_or_else(|e| fail(&format!("{e}")));
    let mut freq: HashMap<String, f64> = HashMap::new();
    for line in text.lines() {
        if line.starts_with('#') {
            continue;
        }
        let mut cols = line.split('\t');
        let (_, Some(word), Some(f)) = (cols.next(), cols.next(), cols.next()) else {
            continue;
        };
        let n = word.chars().count();
        if n == 0 || n > MAX_WORD_CHARS || !word.chars().all(is_han) {
            continue;
        }
        let f: f64 = f.trim().parse().unwrap_or(1.0);
        *freq.entry(word.to_string()).or_default() += f.max(1.0);
    }
    let total: f64 = freq.values().sum();
    let mut names = vec![BOS.to_string()];
    let mut sorted: Vec<(String, f64)> = freq.into_iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    let mut words = HashMap::with_capacity(sorted.len());
    for (w, f) in sorted {
        words.insert(w.clone(), (names.len() as u32, (f / total).ln()));
        names.push(w);
    }
    let mut t2s = HashMap::new();
    if let Some(path) = s2t {
        let text = std::fs::read_to_string(path).unwrap_or_else(|e| fail(&format!("{e}")));
        for line in text.lines() {
            if line.starts_with('#') {
                continue;
            }
            let mut cols = line.split('\t');
            let (Some(s), Some(t)) = (cols.next(), cols.next()) else {
                continue;
            };
            let (mut s, mut t) = (s.chars(), t.chars());
            if let (Some(sc), None, Some(tc), None) = (s.next(), s.next(), t.next(), t.next()) {
                if sc != tc {
                    t2s.entry(tc).or_insert(sc);
                }
            }
        }
    }
    Dict { words, names, t2s }
}

/// 一段汉字按词频最大切分，返回词编号（不认识的单字是 OOV）。
fn segment(dict: &Dict, run: &str, out: &mut Vec<u32>) {
    let offs: Vec<usize> = run
        .char_indices()
        .map(|(i, _)| i)
        .chain(std::iter::once(run.len()))
        .collect();
    let n = offs.len() - 1;
    let mut best = vec![f64::NEG_INFINITY; n + 1];
    let mut back = vec![(0usize, OOV); n + 1];
    best[0] = 0.0;
    for i in 1..=n {
        for l in 1..=MAX_WORD_CHARS.min(i) {
            let j = i - l;
            if best[j] == f64::NEG_INFINITY {
                continue;
            }
            let (id, lp) = match dict.words.get(&run[offs[j]..offs[i]]) {
                Some(&(id, lp)) => (id, lp),
                None if l == 1 => (OOV, -30.0),
                None => continue,
            };
            let s = best[j] + lp;
            if s > best[i] {
                best[i] = s;
                back[i] = (j, id);
            }
        }
    }
    let start = out.len();
    let mut i = n;
    while i > 0 {
        let (j, id) = back[i];
        out.push(id);
        i = j;
    }
    out[start..].reverse();
}

#[derive(Default)]
struct Local {
    unigram: Vec<u64>,
    bigram: FxMap<u64, u32>,
    tokens: u64,
}

fn count_line(dict: &Dict, line: &str, local: &mut Local, run: &mut String, ids: &mut Vec<u32>) {
    let mut sentence_start = true;
    let flush = |run: &mut String, start: bool, local: &mut Local, ids: &mut Vec<u32>| {
        if run.is_empty() {
            return;
        }
        ids.clear();
        segment(dict, run, ids);
        run.clear();
        let mut prev = if start { Some(0u32) } else { None };
        if start {
            local.unigram[0] += 1;
        }
        for &id in ids.iter() {
            if id == OOV {
                prev = None;
                continue;
            }
            local.unigram[id as usize] += 1;
            local.tokens += 1;
            if let Some(p) = prev {
                *local
                    .bigram
                    .entry((p as u64) << 32 | id as u64)
                    .or_insert(0) += 1;
            }
            prev = Some(id);
        }
    };
    for c in line.chars() {
        let c = dict.t2s.get(&c).copied().unwrap_or(c);
        if is_han(c) {
            run.push(c);
            continue;
        }
        let was_start = sentence_start;
        if !run.is_empty() {
            flush(run, was_start, local, ids);
            sentence_start = false;
        }
        if matches!(c, '。' | '！' | '？' | '；' | '!' | '?' | ';') {
            sentence_start = true;
        }
    }
    flush(run, sentence_start, local, ids);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut lexicon = String::new();
    let mut s2t: Option<String> = None;
    let mut out = String::new();
    let mut corpora = Vec::new();
    let mut threads = 8usize;
    let mut max_chars = u64::MAX;
    let mut config = BuildConfig::default();
    let mut save_counts: Option<String> = None;
    let mut from_counts: Option<String> = None;
    let mut i = 1;
    while i < args.len() {
        let flag = args[i].clone();
        let mut take = || {
            i += 1;
            args.get(i).cloned().unwrap_or_else(|| fail("参数缺值"))
        };
        match flag.as_str() {
            "--lexicon" => lexicon = take(),
            "--s2t" => s2t = Some(take()),
            "--out" => out = take(),
            "--threads" => threads = take().parse().unwrap_or(8).max(1),
            "--max-chars" => max_chars = take().parse().unwrap_or(u64::MAX),
            "--min-count" => config.min_count = take().parse().unwrap_or(3),
            "--max-bigrams" => config.max_bigrams = take().parse().unwrap_or(4_000_000),
            "--min-unigram" => config.min_unigram = take().parse().unwrap_or(20),
            "--save-counts" => save_counts = Some(take()),
            "--from-counts" => from_counts = Some(take()),
            "-h" | "--help" => {
                println!("用法：wowtype-lmbuild --lexicon lexicon.tsv [--s2t s2t.tsv] --out words.lm 语料...");
                return;
            }
            other => corpora.push(other.to_string()),
        }
        i += 1;
    }
    let started = Instant::now();
    if let Some(path) = &from_counts {
        if out.is_empty() {
            fail("需要 --out");
        }
        let counts = read_counts(path).unwrap_or_else(|e| fail(&format!("{path}: {e}")));
        write_model(&counts, config, &out, started);
        return;
    }
    if lexicon.is_empty() || out.is_empty() || corpora.is_empty() {
        fail("需要 --lexicon、--out 和至少一个语料文件");
    }
    let dict = Arc::new(load_dict(&lexicon, s2t.as_deref()));
    eprintln!(
        "词库 {} 个词，繁简对照 {} 个字（{:?}）",
        dict.names.len() - 1,
        dict.t2s.len(),
        started.elapsed()
    );

    let (tx, rx) = mpsc::sync_channel::<Vec<String>>(threads * 4);
    let rx = Arc::new(std::sync::Mutex::new(rx));
    let workers: Vec<_> = (0..threads)
        .map(|_| {
            let rx = rx.clone();
            let dict = dict.clone();
            std::thread::spawn(move || {
                let mut local = Local {
                    unigram: vec![0; dict.names.len()],
                    ..Local::default()
                };
                let (mut run, mut ids) = (String::new(), Vec::new());
                loop {
                    let batch = match rx.lock().unwrap().recv() {
                        Ok(b) => b,
                        Err(_) => break,
                    };
                    for line in batch {
                        count_line(&dict, &line, &mut local, &mut run, &mut ids);
                    }
                }
                local
            })
        })
        .collect();

    let mut read = 0u64;
    'files: for path in &corpora {
        let file = std::fs::File::open(path).unwrap_or_else(|e| fail(&format!("{path}: {e}")));
        let mut batch = Vec::with_capacity(512);
        for line in BufReader::with_capacity(1 << 20, file).lines() {
            let Ok(line) = line else { continue };
            read += line.chars().count() as u64;
            batch.push(line);
            if batch.len() == 512 {
                tx.send(std::mem::replace(&mut batch, Vec::with_capacity(512)))
                    .unwrap();
            }
            if read >= max_chars {
                tx.send(batch).unwrap();
                break 'files;
            }
        }
        tx.send(batch).unwrap();
    }
    drop(tx);

    let mut unigram = vec![0u64; dict.names.len()];
    let mut bigram: FxMap<u64, u32> = FxMap::default();
    let mut tokens = 0u64;
    for w in workers {
        let local = w.join().unwrap();
        tokens += local.tokens;
        for (a, b) in unigram.iter_mut().zip(&local.unigram) {
            *a += b;
        }
        if bigram.is_empty() {
            bigram = local.bigram;
        } else {
            for (k, v) in local.bigram {
                let e = bigram.entry(k).or_insert(0);
                *e = e.saturating_add(v);
            }
        }
    }
    eprintln!(
        "读了 {read} 个字，{tokens} 个词，{} 句，{} 种搭配（{:?}）",
        unigram[0],
        bigram.len(),
        started.elapsed()
    );
    let mut history = vec![0u64; dict.names.len()];
    for (&k, &c) in &bigram {
        history[(k >> 32) as usize] += c as u64;
    }
    // 存计数时多留一些（次数 ≥ 2），之后可以换 --min-count 重新出模型
    let min = if save_counts.is_some() {
        config.min_count.min(2)
    } else {
        config.min_count
    };
    let counts = Counts {
        words: dict.names.clone(),
        unigram,
        history,
        bigram: bigram.into_iter().filter(|e| e.1 >= min).collect(),
    };
    if let Some(path) = &save_counts {
        write_counts(&counts, path).unwrap_or_else(|e| fail(&format!("{path}: {e}")));
        eprintln!("计数存到 {path}");
    }
    write_model(&counts, config, &out, started);
}

fn write_model(counts: &Counts, config: BuildConfig, out: &str, started: Instant) {
    let (bytes, n, m) = build(counts, config).unwrap_or_else(|e| fail(&format!("{e:#}")));
    std::fs::write(out, &bytes).unwrap_or_else(|e| fail(&format!("{out}: {e}")));
    eprintln!(
        "写出 {out}：{n} 个词，{m} 个搭配，{:.1} MB（{:?}）",
        bytes.len() as f64 / 1e6,
        started.elapsed()
    );
}

/// 计数的中间文件（只给 --from-counts 用，格式随时可能变）：
/// u32 词数，每个词 u16 长 + UTF-8 + u64 一元 + u64 history，u64 搭配数，每个 u64 键 + u32 次数。
fn write_counts(c: &Counts, path: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut w = std::io::BufWriter::new(std::fs::File::create(path)?);
    w.write_all(&(c.words.len() as u32).to_le_bytes())?;
    for (i, word) in c.words.iter().enumerate() {
        w.write_all(&(word.len() as u16).to_le_bytes())?;
        w.write_all(word.as_bytes())?;
        w.write_all(&c.unigram[i].to_le_bytes())?;
        w.write_all(&c.history[i].to_le_bytes())?;
    }
    w.write_all(&(c.bigram.len() as u64).to_le_bytes())?;
    for &(k, v) in &c.bigram {
        w.write_all(&k.to_le_bytes())?;
        w.write_all(&v.to_le_bytes())?;
    }
    w.flush()
}

fn read_counts(path: &str) -> std::io::Result<Counts> {
    use std::io::Read;
    let mut r = std::io::BufReader::new(std::fs::File::open(path)?);
    let mut b4 = [0u8; 4];
    let mut b8 = [0u8; 8];
    let mut b2 = [0u8; 2];
    r.read_exact(&mut b4)?;
    let n = u32::from_le_bytes(b4) as usize;
    let (mut words, mut unigram, mut history) = (Vec::new(), Vec::new(), Vec::new());
    for _ in 0..n {
        r.read_exact(&mut b2)?;
        let mut s = vec![0u8; u16::from_le_bytes(b2) as usize];
        r.read_exact(&mut s)?;
        words.push(String::from_utf8_lossy(&s).into_owned());
        r.read_exact(&mut b8)?;
        unigram.push(u64::from_le_bytes(b8));
        r.read_exact(&mut b8)?;
        history.push(u64::from_le_bytes(b8));
    }
    r.read_exact(&mut b8)?;
    let m = u64::from_le_bytes(b8) as usize;
    let mut bigram = Vec::with_capacity(m);
    for _ in 0..m {
        r.read_exact(&mut b8)?;
        r.read_exact(&mut b4)?;
        bigram.push((u64::from_le_bytes(b8), u32::from_le_bytes(b4)));
    }
    Ok(Counts {
        words,
        unigram,
        history,
        bigram,
    })
}
