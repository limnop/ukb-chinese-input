//! 词级二元语言模型（紧凑二进制格式，`wowtype-lmbuild` 从语料训练）。
//!
//! 和 [`crate::lm::BigramLm`] 不同，这里的分数是**相对一元的提升**：
//! `ln P(cur | prev) - ln P(cur)`。词本身有多常用已经由词库词频管了（束搜索里的 lexical），
//! 语言模型只负责「这两个词连着出现是不是比各自随便出现更常见」，不重复计一元。
//!
//! P(cur | prev) 用绝对折扣插值：`max(c - d, 0) / H(prev) + λ(prev) · P(cur)`，
//! 没见过的搭配就是 `ln λ(prev)`（≤ 0，常接各种词的 prev 罚得轻）；模型里没有的词给 0。
//!
//! 文件格式（小端）：
//!
//! ```text
//! "WTLM" u32 版本=1
//! u32 词数 n  u32 二元组数 m
//! n × { u8 字节长 + UTF-8 }        词表，0 号是 <s>
//! n × f32  ln λ(prev)
//! (n+1) × u32  每个 prev 的二元组起点
//! m × u32  cur 编号（同一 prev 内升序）
//! m × f32  分数
//! ```

use std::collections::HashMap;
use std::path::Path;

use anyhow::{bail, Context, Result};

use crate::lm::LanguageModel;

pub const MAGIC: &[u8; 4] = b"WTLM";
pub const VERSION: u32 = 1;
pub const BOS: &str = "<s>";

pub struct WordLm {
    ids: HashMap<String, u32>,
    words: Vec<String>,
    backoff: Vec<f32>,
    starts: Vec<u32>,
    curs: Vec<u32>,
    scores: Vec<f32>,
}

impl WordLm {
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();
        let data = std::fs::read(path).with_context(|| format!("读取 {}", path.display()))?;
        Self::from_bytes(&data).with_context(|| format!("解析 {}", path.display()))
    }

    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        let mut r = Reader { d: data, at: 0 };
        if r.take(4)? != MAGIC {
            bail!("不是 WowType 词级语言模型");
        }
        let version = r.u32()?;
        if version != VERSION {
            bail!("版本 {version} 不认识");
        }
        let n = r.u32()? as usize;
        let m = r.u32()? as usize;
        let mut words = Vec::with_capacity(n);
        let mut ids = HashMap::with_capacity(n);
        for i in 0..n {
            let len = r.take(1)?[0] as usize;
            let w = std::str::from_utf8(r.take(len)?)
                .context("词表不是 UTF-8")?
                .to_string();
            ids.insert(w.clone(), i as u32);
            words.push(w);
        }
        let backoff = r.f32s(n)?;
        let starts = r.u32s(n + 1)?;
        let curs = r.u32s(m)?;
        let scores = r.f32s(m)?;
        if starts.last().copied() != Some(m as u32) || starts.windows(2).any(|w| w[0] > w[1]) {
            bail!("二元组索引坏了");
        }
        if r.at != data.len() {
            bail!("文件末尾多了 {} 字节", data.len() - r.at);
        }
        Ok(Self {
            ids,
            words,
            backoff,
            starts,
            curs,
            scores,
        })
    }

    pub fn words(&self) -> usize {
        self.words.len()
    }

    pub fn bigrams(&self) -> usize {
        self.curs.len()
    }

    fn pair(&self, prev: u32, cur: u32) -> Option<f32> {
        let (a, b) = (
            self.starts[prev as usize] as usize,
            self.starts[prev as usize + 1] as usize,
        );
        let row = &self.curs[a..b];
        row.binary_search(&cur).ok().map(|i| self.scores[a + i])
    }
}

impl LanguageModel for WordLm {
    fn score(&self, prev: Option<&str>, cur: &str) -> f64 {
        // 前一个词不在模型里：没有可用的信息，给 0
        let Some(&p) = self.ids.get(prev.unwrap_or(BOS)) else {
            return 0.0;
        };
        // 当前词不在模型里（语料里太少见、被裁掉了）：按「没见过的搭配」给回退分（≤ 0），
        // 和模型认识、但没跟 prev 搭配过的词一样。以前给 0，比回退分高，
        // 结果模型不认识的冷僻词、错字词反而比常用词分高：
        // fenbenglixi 首选成了「分崩离兮」，pingwenguodu 成了「平稳过度」。
        // 不再额外罚分：实测再罚会把这类词所在的整句挤出前几名（日常句 90.6% -> 87.7%）。
        let Some(&c) = self.ids.get(cur) else {
            return self.backoff[p as usize] as f64;
        };
        match self.pair(p, c) {
            Some(s) => s as f64,
            None => self.backoff[p as usize] as f64,
        }
    }

    fn has_bigram(&self, prev: &str, cur: &str) -> bool {
        match (self.ids.get(prev), self.ids.get(cur)) {
            (Some(&p), Some(&c)) => self.pair(p, c).is_some(),
            _ => false,
        }
    }

    fn top_next(&self, prev: &str, limit: usize) -> Vec<(String, f64)> {
        let Some(&p) = self.ids.get(prev) else {
            return Vec::new();
        };
        let (a, b) = (
            self.starts[p as usize] as usize,
            self.starts[p as usize + 1] as usize,
        );
        let mut items: Vec<(String, f64)> = (a..b)
            .map(|i| {
                (
                    self.words[self.curs[i] as usize].clone(),
                    self.scores[i] as f64,
                )
            })
            .collect();
        items.sort_by(|x, y| y.1.total_cmp(&x.1).then_with(|| x.0.cmp(&y.0)));
        items.truncate(limit);
        items
    }
}

struct Reader<'a> {
    d: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let s = self.d.get(self.at..self.at + n).context("文件截断")?;
        self.at += n;
        Ok(s)
    }
    fn u32(&mut self) -> Result<u32> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn u32s(&mut self, n: usize) -> Result<Vec<u32>> {
        let b = self.take(n * 4)?;
        Ok(b.chunks_exact(4)
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect())
    }
    fn f32s(&mut self, n: usize) -> Result<Vec<f32>> {
        let b = self.take(n * 4)?;
        Ok(b.chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect())
    }
}

/// 训练用：一元、二元计数 -> 模型文件。
pub struct Counts {
    /// 词表（0 号是 `<s>`）。
    pub words: Vec<String>,
    /// 每个词出现的次数（`<s>` 是句子数）。
    pub unigram: Vec<u64>,
    /// 每个词后面接了多少次别的词（含没进 `bigram` 的搭配），用来算折扣后剩下的概率。
    pub history: Vec<u64>,
    /// (prev << 32 | cur, 次数)。可以只给次数够 `min_count` 的。
    pub bigram: Vec<(u64, u32)>,
}

/// 训练参数。
#[derive(Clone, Copy, Debug)]
pub struct BuildConfig {
    /// 至少出现几次的搭配才存。
    pub min_count: u32,
    /// 最多存多少个搭配（按次数从多到少取）。
    pub max_bigrams: usize,
    /// 绝对折扣。
    pub discount: f64,
    /// 分数上限（自然对数），免得罕见词对把分数拉得太高。
    pub max_score: f64,
    /// 不在任何搭配里的词，至少出现几次才进词表。
    pub min_unigram: u64,
}

impl Default for BuildConfig {
    fn default() -> Self {
        Self {
            min_count: 3,
            max_bigrams: 4_000_000,
            discount: 0.75,
            max_score: 6.0,
            min_unigram: 20,
        }
    }
}

/// 把计数写成模型文件，返回 (词数, 二元组数)。
pub fn build(counts: &Counts, config: BuildConfig) -> Result<(Vec<u8>, usize, usize)> {
    let n_all = counts.words.len();
    let history = &counts.history;
    let mut kept: Vec<(u64, u32)> = counts
        .bigram
        .iter()
        .filter(|e| e.1 >= config.min_count)
        .copied()
        .collect();
    if kept.len() > config.max_bigrams {
        kept.select_nth_unstable_by(config.max_bigrams, |a, b| b.1.cmp(&a.1));
        kept.truncate(config.max_bigrams);
    }
    // 只留在搭配里出现过、或者自己够常见的词，重新编号（<s> 永远是 0）
    let mut in_pair = vec![false; n_all];
    for &(key, _) in &kept {
        in_pair[(key >> 32) as usize] = true;
        in_pair[(key & 0xffff_ffff) as usize] = true;
    }
    let mut remap = vec![u32::MAX; n_all];
    let mut words = Vec::new();
    for (i, w) in counts.words.iter().enumerate() {
        if i == 0 || in_pair[i] || counts.unigram[i] >= config.min_unigram.max(1) {
            if w.len() > 255 {
                continue;
            }
            remap[i] = words.len() as u32;
            words.push(i);
        }
    }
    let total: f64 = counts.unigram.iter().skip(1).sum::<u64>() as f64;
    if total <= 0.0 {
        bail!("语料里没有一个词");
    }
    let vocab = words.len() as f64;
    let p_uni = |i: usize| (counts.unigram[i] as f64 + 0.5) / (total + 0.5 * vocab);

    let mut rows: Vec<Vec<(u32, f32)>> = vec![Vec::new(); words.len()];
    let mut mass = vec![0.0f64; n_all];
    for &(key, c) in &kept {
        let (p, q) = ((key >> 32) as usize, (key & 0xffff_ffff) as usize);
        if remap[p] == u32::MAX || remap[q] == u32::MAX {
            continue;
        }
        mass[p] += (c as f64 - config.discount).max(0.0);
    }
    let lambda: Vec<f64> = (0..n_all)
        .map(|p| {
            if history[p] == 0 {
                1.0
            } else {
                (1.0 - mass[p] / history[p] as f64).clamp(1e-6, 1.0)
            }
        })
        .collect();
    for &(key, c) in &kept {
        let (p, q) = ((key >> 32) as usize, (key & 0xffff_ffff) as usize);
        if remap[p] == u32::MAX || remap[q] == u32::MAX {
            continue;
        }
        let pu = p_uni(q);
        let prob = (c as f64 - config.discount).max(0.0) / history[p] as f64 + lambda[p] * pu;
        let score = (prob / pu).ln().min(config.max_score);
        rows[remap[p] as usize].push((remap[q], score as f32));
    }
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&(words.len() as u32).to_le_bytes());
    let m: usize = rows.iter().map(Vec::len).sum();
    out.extend_from_slice(&(m as u32).to_le_bytes());
    for &i in &words {
        let w = counts.words[i].as_bytes();
        out.push(w.len() as u8);
        out.extend_from_slice(w);
    }
    for &i in &words {
        out.extend_from_slice(&(lambda[i].ln() as f32).to_le_bytes());
    }
    let mut start = 0u32;
    out.extend_from_slice(&start.to_le_bytes());
    for row in rows.iter_mut() {
        row.sort_by_key(|e| e.0);
        start += row.len() as u32;
        out.extend_from_slice(&start.to_le_bytes());
    }
    for row in &rows {
        for &(c, _) in row {
            out.extend_from_slice(&c.to_le_bytes());
        }
    }
    for row in &rows {
        for &(_, s) in row {
            out.extend_from_slice(&s.to_le_bytes());
        }
    }
    Ok((out, words.len(), m))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_scores() {
        let words: Vec<String> = ["<s>", "我们", "明天", "见", "面"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let key = |a: u64, b: u64| a << 32 | b;
        let bigram = vec![
            (key(0, 1), 10),
            (key(1, 2), 8),
            (key(2, 3), 6),
            (key(2, 4), 1), // 少于 min_count，裁掉
        ];
        let counts = Counts {
            words,
            unigram: vec![10, 10, 9, 6, 50],
            history: vec![10, 8, 7, 0, 0],
            bigram,
        };
        let (bytes, n, m) = build(&counts, BuildConfig::default()).unwrap();
        assert_eq!((n, m), (5, 3));
        let lm = WordLm::from_bytes(&bytes).unwrap();
        assert!(lm.has_bigram("我们", "明天"));
        assert!(!lm.has_bigram("明天", "面"));
        // 见过的搭配比没见过的好；没见过的 ≤ 0
        assert!(lm.score(Some("明天"), "见") > 0.0);
        assert!(lm.score(Some("明天"), "面") <= 0.0);
        assert!(lm.score(None, "我们") > 0.0);
        assert_eq!(lm.score(Some("不认识"), "见"), 0.0);
        // 不认识的词和「认识但没搭配过」的词同分，不会比常用词高
        assert_eq!(
            lm.score(Some("明天"), "不认识"),
            lm.score(Some("明天"), "面")
        );
        assert_eq!(lm.top_next("明天", 1)[0].0, "见");
    }
}
