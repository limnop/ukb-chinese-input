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
//!
//! 版本标记（见 [`crate::datamark`]）：官方发布的模型词表 1 号词是 `wowtype-data: words.lm <版本> <来源>`，
//! 没有搭配、回退分 0（λ = 1）。格式版本还是 1：0.4.0 及以前的读取代码把它当成一个普通的词，
//! 输入里永远打不出这样的词，查不到它，读新文件照常；这里读到时把它认成标记、不放进词表的查找。
//! 加标记以前发布的 lm-2026.11.09 没有这个词，按字节数认（[`crate::datamark::LEGACY_OFFICIAL_LM`]）。
//! 训练完用 [`set_data_marker`] 加标记（`tools/datamark.py stamp` 是同样的做法）。

use std::path::Path;

use anyhow::{bail, Context, Result};

use crate::datamark::{self, DataMarker};
use crate::lm::LanguageModel;

pub const MAGIC: &[u8; 4] = b"WTLM";
pub const VERSION: u32 = 1;
pub const BOS: &str = "<s>";

/// 模型里没有的词，回退分最低给到这里（自然对数）。
/// 实测（rime-ice + words.lm）：不设下限时中低频词 top1 82.5%、整句 84.8%，−1.5 时 87.8%、86.2%；
/// 再高（−0.5）中低频 88.8%，但整句回落到 84.8%、简拼也降。设成 0 以前出过「分崩离兮」。
pub const OOV_FLOOR: f64 = -1.5;
const EMPTY_SLOT: u32 = 0;

pub struct WordLm {
    /// 词 -> 编号的开放寻址表：存「编号 + 1」，0 是空位。和词本身分开放，
    /// 束搜索每次查词不用为 String 分配内存、算 SipHash（#227）
    slots: Vec<u32>,
    words: Vec<String>,
    backoff: Vec<f32>,
    starts: Vec<u32>,
    curs: Vec<u32>,
    scores: Vec<f32>,
    marker: Option<DataMarker>,
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
        let mut marker = None;
        for i in 0..n {
            let len = r.take(1)?[0] as usize;
            let w = std::str::from_utf8(r.take(len)?)
                .context("词表不是 UTF-8")?
                .to_string();
            if i == 1 && w.starts_with(datamark::KEY) {
                // 版本标记：不当词查（下面建查找表时跳过它）
                marker = DataMarker::parse(&w);
            }
            words.push(w);
        }
        let mut slot_count = 1usize;
        while slot_count < n.saturating_mul(2).max(2) {
            slot_count = slot_count.checked_mul(2).context("词表太大")?;
        }
        let mut slots = vec![EMPTY_SLOT; slot_count];
        for (id, word) in words.iter().enumerate() {
            // 版本标记留在 words 里（搭配的编号不变），但不进查找表
            if id == 1 && word.starts_with(datamark::KEY) {
                continue;
            }
            let mut slot = hash_word(word) as usize & (slot_count - 1);
            loop {
                let stored = slots[slot];
                // 空位放下；同一个词出现两次时用后面的编号（和原来 HashMap::insert 一样）
                if stored == EMPTY_SLOT || words[stored as usize - 1] == *word {
                    slots[slot] = id as u32 + 1;
                    break;
                }
                slot = (slot + 1) & (slot_count - 1);
            }
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
            slots,
            words,
            backoff,
            starts,
            curs,
            scores,
            marker,
        })
    }

    /// 词表里的版本标记（官方发布的模型才有；加标记以前的 lm-2026.11.09 没有）。
    pub fn data_marker(&self) -> Option<&DataMarker> {
        self.marker.as_ref()
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

    fn id(&self, word: &str) -> Option<u32> {
        let mask = self.slots.len() - 1;
        let mut slot = hash_word(word) as usize & mask;
        loop {
            let stored = self.slots[slot];
            if stored == EMPTY_SLOT {
                return None;
            }
            let id = stored - 1;
            if self.words[id as usize].as_bytes() == word.as_bytes() {
                return Some(id);
            }
            slot = (slot + 1) & mask;
        }
    }
}

fn hash_word(word: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in word.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x1000_0000_01b3);
    }
    hash
}

impl LanguageModel for WordLm {
    fn score(&self, prev: Option<&str>, cur: &str) -> f64 {
        // 前一个词不在模型里：没有可用的信息，给 0
        let Some(p) = self.id(prev.unwrap_or(BOS)) else {
            return 0.0;
        };
        // 当前词不在模型里（语料里太少见、被裁掉了）：按「没见过的搭配」给回退分（≤ 0），
        // 和模型认识、但没跟 prev 搭配过的词一样。以前给 0，比回退分高，
        // 结果模型不认识的冷僻词、错字词反而比常用词分高：
        // fenbenglixi 首选成了「分崩离兮」，pingwenguodu 成了「平稳过度」。
        // 不再额外罚分：实测再罚会把这类词所在的整句挤出前几名（日常句 90.6% -> 87.7%）。
        // 回退分再设个下限 OOV_FLOOR：句首的回退分低到 −4.2，热词表里补的新词（大模型、黑神话、
        // 永远的神）都不在模型里，30000 的词频实际只顶 450 用。
        let Some(c) = self.id(cur) else {
            return (self.backoff[p as usize] as f64).max(OOV_FLOOR);
        };
        match self.pair(p, c) {
            Some(s) => s as f64,
            None => self.backoff[p as usize] as f64,
        }
    }

    fn knows_word(&self, word: &str) -> Option<bool> {
        Some(self.id(word).is_some())
    }

    fn has_bigram(&self, prev: &str, cur: &str) -> bool {
        match (self.id(prev), self.id(cur)) {
            (Some(p), Some(c)) => self.pair(p, c).is_some(),
            _ => false,
        }
    }

    fn top_next(&self, prev: &str, limit: usize) -> Vec<(String, f64)> {
        let Some(p) = self.id(prev) else {
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

/// 给模型文件加上（或换掉）版本标记：词表 1 号插入标记，原来 1 号及以后的词编号加一；
/// 已经有标记时只换掉那个词。格式版本不变，0.4.0 的读取代码照样能读。
pub fn set_data_marker(data: &[u8], marker: &DataMarker) -> Result<Vec<u8>> {
    let body = marker.body();
    if body.len() > 255 {
        bail!("标记太长（{} 字节，最多 255）", body.len());
    }
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
    if n == 0 {
        bail!("词表是空的（没有 <s>）");
    }
    let mut words = Vec::with_capacity(n);
    for _ in 0..n {
        let len = r.take(1)?[0] as usize;
        words.push(r.take(len)?);
    }
    let backoff = r.take(n * 4)?;
    let starts = r.u32s(n + 1)?;
    let curs = r.u32s(m)?;
    let scores = r.take(m * 4)?;
    if r.at != data.len() {
        bail!("文件末尾多了 {} 字节", data.len() - r.at);
    }
    let has = n > 1 && words[1].starts_with(datamark::KEY.as_bytes());

    let mut out = Vec::with_capacity(data.len() + body.len() + 16);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&((n + usize::from(!has)) as u32).to_le_bytes());
    out.extend_from_slice(&(m as u32).to_le_bytes());
    for (i, w) in words.iter().enumerate() {
        if i == 1 && has {
            continue;
        }
        out.push(w.len() as u8);
        out.extend_from_slice(w);
        if i == 0 {
            out.push(body.len() as u8);
            out.extend_from_slice(body.as_bytes());
        }
    }
    out.extend_from_slice(&backoff[..4]);
    if !has {
        out.extend_from_slice(&0f32.to_le_bytes());
    }
    out.extend_from_slice(&backoff[4..]);
    for (i, s) in starts.iter().enumerate() {
        out.extend_from_slice(&s.to_le_bytes());
        if i == 1 && !has {
            out.extend_from_slice(&s.to_le_bytes()); // 标记那一行是空的
        }
    }
    for &c in &curs {
        let c = if has || c == 0 { c } else { c + 1 };
        out.extend_from_slice(&c.to_le_bytes());
    }
    out.extend_from_slice(scores);
    Ok(out)
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

    /// 加了标记的模型分数一样；标记能读出来、能换；文件开头就能认出标记。
    #[test]
    fn data_marker_round_trip() {
        let words: Vec<String> = ["<s>", "我们", "明天", "见", "面"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let key = |a: u64, b: u64| a << 32 | b;
        let counts = Counts {
            words,
            unigram: vec![10, 10, 9, 6, 50],
            history: vec![10, 8, 7, 0, 0],
            bigram: vec![(key(0, 1), 10), (key(1, 2), 8), (key(2, 3), 6), (key(2, 1), 3)],
        };
        let (bytes, _, _) = build(&counts, BuildConfig::default()).unwrap();
        let plain = WordLm::from_bytes(&bytes).unwrap();
        assert!(plain.data_marker().is_none());
        assert!(datamark::scan(&bytes).is_none());

        let marker = DataMarker::new("words.lm", "2026.11.20.0", "lm-2026.11.20 测试").unwrap();
        let marked = set_data_marker(&bytes, &marker).unwrap();
        let lm = WordLm::from_bytes(&marked).unwrap();
        assert_eq!(lm.data_marker(), Some(&marker));
        assert_eq!(datamark::scan(&marked).as_ref(), Some(&marker));
        assert_eq!(lm.words(), plain.words() + 1);
        assert_eq!(lm.bigrams(), plain.bigrams());
        let probes = [
            (None, "我们"),
            (Some("我们"), "明天"),
            (Some("明天"), "见"),
            (Some("明天"), "我们"),
            (Some("明天"), "面"),
            (Some("见"), "面"),
            (Some("明天"), "不认识"),
            (Some("不认识"), "见"),
        ];
        for (prev, cur) in probes {
            assert_eq!(lm.score(prev, cur), plain.score(prev, cur), "{prev:?} {cur}");
        }
        assert_eq!(lm.top_next("明天", 5), plain.top_next("明天", 5));
        assert_eq!(lm.top_next("<s>", 5), plain.top_next("<s>", 5));
        assert_eq!(lm.knows_word(&marker.body()), Some(false));

        // 换标记：词数不变
        let newer = DataMarker::new("words.lm", "2026.12.01.0", "").unwrap();
        let again = set_data_marker(&marked, &newer).unwrap();
        let lm2 = WordLm::from_bytes(&again).unwrap();
        assert_eq!(lm2.data_marker(), Some(&newer));
        assert_eq!(lm2.words(), lm.words());
        assert_eq!(lm2.score(Some("明天"), "见"), plain.score(Some("明天"), "见"));
        // 坏文件、多出来的字节：报错，不乱写
        assert!(set_data_marker(&bytes[..bytes.len() - 1], &marker).is_err());
        let mut longer = bytes.clone();
        longer.push(0);
        assert!(set_data_marker(&longer, &marker).is_err());
    }
}
