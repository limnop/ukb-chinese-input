//! 二元语言模型接口与内存实现。
//!
//! 打分使用自然对数概率（<= 0）；未登录词回退到 unigram，再对未知词使用
//! 加零点五平滑。骨架阶段只有 bigram，后续可替换为 WeType 的 lmcnmac.bin
//! 解析器或神经语言模型。

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use anyhow::{bail, Context, Result};

/// 语言模型接口。
pub trait LanguageModel: Send + Sync {
    /// 返回 ln P(cur | prev)。prev 为 None 表示句首。
    fn score(&self, prev: Option<&str>, cur: &str) -> f64;

    /// 是否存在「显式」的二元组。上下文打分只认这个，
    /// 避免回退到一元概率时用错分布（例如从二元计数反推的一元）。
    fn has_bigram(&self, _prev: &str, _cur: &str) -> bool {
        false
    }

    /// 词表里有没有这个词；None 表示这个模型没有词表可查（字级、测试用的模型）。
    fn knows_word(&self, _word: &str) -> Option<bool> {
        None
    }

    /// 一句话的累计对数概率。
    fn sequence_score(&self, words: &[&str]) -> f64 {
        let mut total = 0.0;
        for (i, word) in words.iter().enumerate() {
            let prev = if i == 0 { None } else { Some(words[i - 1]) };
            total += self.score(prev, word);
        }
        total
    }

    /// 给定上一个词，返回最可能的后续词（按概率降序，最多 limit 个）。
    ///
    /// 这是「预测词库」的接口：默认实现不提供预测，接入了 bigram/神经网络
    /// 的模型自行覆写。
    fn top_next(&self, _prev: &str, _limit: usize) -> Vec<(String, f64)> {
        Vec::new()
    }
}

/// 内存二元语言模型。
#[derive(Clone, Debug)]
pub struct BigramLm {
    unigram: HashMap<String, f64>,
    bigram: HashMap<String, HashMap<String, f64>>,
    total: f64,
    vocab: usize,
    /// 回退权重，乘在 unigram 概率上；默认 0.4。
    backoff: f64,
}

impl Default for BigramLm {
    fn default() -> Self {
        Self {
            unigram: HashMap::new(),
            bigram: HashMap::new(),
            total: 0.0,
            vocab: 0,
            backoff: 0.4,
        }
    }
}

impl BigramLm {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_backoff(mut self, backoff: f64) -> Self {
        if backoff > 0.0 && backoff <= 1.0 {
            self.backoff = backoff;
        }
        self
    }

    pub fn add_unigram(&mut self, word: &str, count: f64) {
        if count <= 0.0 {
            return;
        }
        let slot = self.unigram.entry(word.to_string()).or_insert(0.0);
        if *slot == 0.0 {
            self.vocab += 1;
        }
        *slot += count;
        self.total += count;
    }

    pub fn add_bigram(&mut self, prev: &str, cur: &str, count: f64) {
        if count <= 0.0 {
            return;
        }
        self.bigram
            .entry(prev.to_string())
            .or_default()
            .entry(cur.to_string())
            .and_modify(|c| *c += count)
            .or_insert(count);
    }

    pub fn vocab(&self) -> usize {
        self.vocab
    }

    pub fn total(&self) -> f64 {
        self.total
    }

    /// 粗略的 bigram TSV 加载：每行 prev<TAB>cur<TAB>count。
    ///
    /// 为了能直接查询，这里把每行的 prev/cur 也累加成 unigram 计数；
    /// 与真实词频不同，仅用于打通流程。
    pub fn from_bigram_tsv_str(data: &str) -> Result<Self> {
        let mut lm = Self::new();
        for (line_no, raw) in data.lines().enumerate() {
            let line = raw.trim_end();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut cols = line.split('\t');
            let prev = cols.next().unwrap_or("");
            let cur = cols.next().unwrap_or("");
            let count = cols.next().unwrap_or("");
            if cols.next().is_some() || prev.is_empty() || cur.is_empty() || count.is_empty() {
                bail!("第 {} 行应为 3 列：prev<TAB>cur<TAB>count", line_no + 1);
            }
            let count: f64 = count
                .parse()
                .with_context(|| format!("第 {} 行计数不是数字: {count}", line_no + 1))?;
            lm.add_bigram(prev, cur, count);
            lm.add_unigram(prev, count);
            lm.add_unigram(cur, count);
        }
        Ok(lm)
    }

    pub fn from_bigram_tsv_path<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();
        let data = fs::read_to_string(path)
            .with_context(|| format!("读取语言模型失败: {}", path.display()))?;
        Self::from_bigram_tsv_str(&data)
    }

    fn unigram_logprob(&self, word: &str) -> f64 {
        if self.total <= 0.0 {
            return 0.0;
        }
        let denom = self.total + 0.5 * self.vocab as f64;
        let count = self.unigram.get(word).copied().unwrap_or(0.0);
        ((count + 0.5) / denom).ln()
    }
}

impl LanguageModel for BigramLm {
    fn has_bigram(&self, prev: &str, cur: &str) -> bool {
        self.bigram
            .get(prev)
            .map(|next| next.contains_key(cur))
            .unwrap_or(false)
    }

    fn score(&self, prev: Option<&str>, cur: &str) -> f64 {
        // 未加载任何数据时保持中性，避免骨架的排序被常数惩罚主导。
        if self.total <= 0.0 {
            return 0.0;
        }

        // 词表里没有的词（例如字级模型评多字词）当作「无信息」，
        // 否则会被按近零概率惩罚，反而把正确的长词压下去。
        let unigram = if self.unigram.contains_key(cur) {
            self.unigram_logprob(cur)
        } else {
            0.0
        };
        // 句首用 <s> 查 bigram；没有 <s> 时自然回退到 unigram。
        let prev_word = prev.unwrap_or("<s>");
        let bigram_count = self.bigram.get(prev_word).and_then(|m| m.get(cur)).copied();
        if let Some(count) = bigram_count {
            if count > 0.0 {
                if let Some(prev_count) = self.unigram.get(prev_word).copied() {
                    if prev_count > 0.0 {
                        return (count / prev_count).ln();
                    }
                }
            }
        }
        self.backoff.ln() + unigram
    }

    fn top_next(&self, prev: &str, limit: usize) -> Vec<(String, f64)> {
        let Some(row) = self.bigram.get(prev) else {
            return Vec::new();
        };
        let prev_count = self.unigram.get(prev).copied().unwrap_or(0.0);
        if prev_count <= 0.0 {
            return Vec::new();
        }
        let mut items: Vec<(String, f64)> = row
            .iter()
            .filter(|(_, count)| **count > 0.0)
            .map(|(word, count)| (word.clone(), (count / prev_count).ln()))
            .collect();
        items.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
        });
        items.truncate(limit);
        items
    }
}

/// ARPA 格式的二元语言模型（kenlm / SRILM 通用格式）。
///
/// 只解析 \\1-grams: 和 \\2-grams: 两段，够中文输入法用：
///
/// ```text
/// \\data\\
/// ngram 1=3
/// ngram 2=2
/// \\1-grams:
/// -1.5\t你好\t-0.3
/// \\2-grams:
/// -0.7\t你好 世界
/// \\end\\
/// ```
///
/// 数值都是 log10 概率，内部转成自然对数。
#[derive(Clone, Debug, Default)]
pub struct ArpaLm {
    unigram: HashMap<String, (f64, f64)>,
    bigram: HashMap<(String, String), f64>,
    /// 未登录词的 log10 概率，缺省 -6。
    unk_log10: f64,
}

impl ArpaLm {
    pub fn new() -> Self {
        Self {
            unigram: HashMap::new(),
            bigram: HashMap::new(),
            unk_log10: -6.0,
        }
    }

    pub fn vocab(&self) -> usize {
        self.unigram.len()
    }

    pub fn bigram_count(&self) -> usize {
        self.bigram.len()
    }

    pub fn from_arpa_str(data: &str) -> Result<Self> {
        let mut lm = Self::new();
        let mut section = 0u8;
        for (line_no, raw) in data.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() {
                continue;
            }
            if line.starts_with('\\') {
                if line.starts_with("\\1-grams") {
                    section = 1;
                } else if line.starts_with("\\2-grams") {
                    section = 2;
                } else if line.starts_with("\\end") {
                    section = 0;
                }
                continue;
            }
            match section {
                1 => {
                    let cols: Vec<&str> = line.split('\t').collect();
                    if cols.len() < 2 {
                        continue;
                    }
                    let log10: f64 = cols[0]
                        .trim()
                        .parse()
                        .with_context(|| format!("ARPA 第 {} 行一元概率非法", line_no + 1))?;
                    let backoff = cols
                        .get(2)
                        .and_then(|s| s.trim().parse::<f64>().ok())
                        .unwrap_or(0.0);
                    lm.unigram.insert(cols[1].to_string(), (log10, backoff));
                }
                2 => {
                    let cols: Vec<&str> = line.split('\t').collect();
                    if cols.len() < 2 {
                        continue;
                    }
                    let log10: f64 = cols[0]
                        .trim()
                        .parse()
                        .with_context(|| format!("ARPA 第 {} 行二元概率非法", line_no + 1))?;
                    let mut words = cols[1].split_whitespace();
                    let (Some(a), Some(b)) = (words.next(), words.next()) else {
                        continue;
                    };
                    lm.bigram.insert((a.to_string(), b.to_string()), log10);
                }
                _ => {}
            }
        }
        Ok(lm)
    }

    pub fn from_arpa_path<P: AsRef<Path>>(path: P) -> Result<Self> {
        let data = fs::read_to_string(path.as_ref())
            .with_context(|| format!("读取 ARPA 失败: {}", path.as_ref().display()))?;
        Self::from_arpa_str(&data)
    }

    fn uni(&self, word: &str) -> f64 {
        self.unigram
            .get(word)
            .map(|(p, _)| p * std::f64::consts::LN_10)
            .unwrap_or(self.unk_log10 * std::f64::consts::LN_10)
    }
}

impl LanguageModel for ArpaLm {
    fn has_bigram(&self, prev: &str, cur: &str) -> bool {
        self.bigram
            .contains_key(&(prev.to_string(), cur.to_string()))
    }

    fn score(&self, prev: Option<&str>, cur: &str) -> f64 {
        if let Some(prev) = prev {
            if let Some(log10) = self.bigram.get(&(prev.to_string(), cur.to_string())) {
                return log10 * std::f64::consts::LN_10;
            }
            let bow = self
                .unigram
                .get(prev)
                .map(|(_, b)| b * std::f64::consts::LN_10)
                .unwrap_or(0.0);
            return bow + self.uni(cur);
        }
        self.uni(cur)
    }

    fn top_next(&self, prev: &str, limit: usize) -> Vec<(String, f64)> {
        let mut items: Vec<(String, f64)> = self
            .bigram
            .iter()
            .filter(|((a, _), _)| a == prev)
            .map(|((_, b), log10)| (b.clone(), log10 * std::f64::consts::LN_10))
            .collect();
        items.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
        });
        items.truncate(limit);
        items
    }
}

/// 按文件内容载入语言模型：`wowtype-lmbuild` 生成的二进制（开头是 `WTLM`）用
/// [`crate::wordlm::WordLm`]，否则当 `前词<TAB>后词<TAB>次数` 的 TSV。
pub fn load_lm<P: AsRef<Path>>(path: P) -> Result<std::sync::Arc<dyn LanguageModel>> {
    let path = path.as_ref();
    let mut head = [0u8; 4];
    let is_binary = fs::File::open(path)
        .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut head))
        .map(|_| &head == crate::wordlm::MAGIC)
        .unwrap_or(false);
    if is_binary {
        Ok(std::sync::Arc::new(crate::wordlm::WordLm::load(path)?))
    } else {
        Ok(std::sync::Arc::new(BigramLm::from_bigram_tsv_path(path)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_lm_is_neutral() {
        let lm = BigramLm::default();
        assert_eq!(lm.score(None, "你"), 0.0);
        assert_eq!(lm.sequence_score(&["你", "好"]), 0.0);
    }

    #[test]
    fn bigram_and_backoff() {
        let mut lm = BigramLm::new().with_backoff(0.4);
        lm.add_unigram("你", 10.0);
        lm.add_unigram("好", 5.0);
        lm.add_bigram("你", "好", 4.0);

        let hit = lm.score(Some("你"), "好");
        assert!((hit - (4.0f64 / 10.0).ln()).abs() < 1e-9);

        let backoff = lm.score(Some("好"), "你");
        let uni = lm.unigram_logprob("你");
        assert!((backoff - (0.4f64.ln() + uni)).abs() < 1e-9);
        assert!(backoff < 0.0);
    }

    #[test]
    fn sentence_start_bigram() {
        let mut lm = BigramLm::new();
        lm.add_unigram("<s>", 10.0);
        lm.add_unigram("你", 5.0);
        lm.add_bigram("<s>", "你", 3.0);
        assert!((lm.score(None, "你") - (0.3f64).ln()).abs() < 1e-9);
    }

    #[test]
    fn arpa_scores_and_predicts() {
        let text = r#"\data\
ngram 1=3
ngram 2=2

\1-grams:
-1.0	你	-0.3
-1.3	好	-0.2
-2.0	世界

\2-grams:
-0.3	你 好
-1.0	好 世界

\end\
"#;
        let lm = ArpaLm::from_arpa_str(text).unwrap();
        assert_eq!(lm.vocab(), 3);
        assert_eq!(lm.bigram_count(), 2);
        let hit = lm.score(Some("你"), "好");
        assert!((hit - (-0.3f64 * std::f64::consts::LN_10)).abs() < 1e-9);
        let top = lm.top_next("你", 5);
        assert_eq!(top[0].0, "好");
        // 未登录词回退到 unigram
        let unseen = lm.score(Some("好"), "你");
        assert!(unseen < 0.0);
    }

    #[test]
    fn predicts_following_words() {
        let mut lm = BigramLm::new();
        lm.add_unigram("中国", 100.0);
        lm.add_bigram("中国", "人", 60.0);
        lm.add_bigram("中国", "队", 30.0);
        lm.add_bigram("中国", "的", 10.0);

        let top = lm.top_next("中国", 2);
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].0, "人");
        assert_eq!(top[1].0, "队");
        // 概率是 ln(count/prev_count)
        assert!((top[0].1 - (0.6f64).ln()).abs() < 1e-9);
        assert!(lm.top_next("不存在", 3).is_empty());
    }

    #[test]
    fn tsv_loader() {
        let lm = BigramLm::from_bigram_tsv_str("你\t好\t4\n好\t你\t1\n").unwrap();
        assert!(lm.total() > 0.0);
        assert_eq!(lm.vocab(), 2);
    }
}

/// 领域词级模型的默认权重（[`MixedLm`]）。Tatoeba 上 0.2–0.4 都试过，0.4 在领域内最好、领域外不掉。
pub const DOMAIN_LM_WEIGHT: f64 = 0.4;

/// 通用词级模型混上一个领域模型（用户拿自己的语料用 `wowtype-lmbuild` 训的）：
/// `score = (1 − w) · 通用 + w · 领域`。
///
/// 词级模型的分数是「搭配比各自随便出现常见多少」的对数（`ln P(cur|prev) − ln P(cur)`，见 [`crate::wordlm`]），
/// 这样加权就是两个模型对这个比值的几何平均。领域模型不认识 prev 时给 0（没有搭配可说），
/// 等于把通用模型的分数按 1 − w 收一点。也试过「领域模型不认识 prev 或 cur 时只用通用模型」
/// 和按比值线性插值，领域内的收益都更小，领域外（日常句、上文、热词三组加起来）也没有更好，所以就用最简单的加权。
///
/// 词表（`knows_word`，上文特征拿它判断认不认识这个词）和联想（`top_next`）按通用模型：
/// 领域模型只有几千到几万个词，不能拿它判断一个词认不认识。
pub struct MixedLm {
    general: Arc<dyn LanguageModel>,
    domain: Arc<dyn LanguageModel>,
    weight: f64,
}

impl MixedLm {
    /// `weight` 是领域模型的权重，夹在 0..=1。
    pub fn new(general: Arc<dyn LanguageModel>, domain: Arc<dyn LanguageModel>, weight: f64) -> Self {
        Self {
            general,
            domain,
            weight: weight.clamp(0.0, 1.0),
        }
    }
}

impl LanguageModel for MixedLm {
    fn score(&self, prev: Option<&str>, cur: &str) -> f64 {
        (1.0 - self.weight) * self.general.score(prev, cur) + self.weight * self.domain.score(prev, cur)
    }

    fn has_bigram(&self, prev: &str, cur: &str) -> bool {
        self.general.has_bigram(prev, cur) || self.domain.has_bigram(prev, cur)
    }

    fn knows_word(&self, word: &str) -> Option<bool> {
        self.general.knows_word(word)
    }

    fn top_next(&self, prev: &str, limit: usize) -> Vec<(String, f64)> {
        self.general.top_next(prev, limit)
    }
}

#[cfg(test)]
mod mixed_lm_tests {
    use super::*;

    struct Fixed(f64, bool);
    impl LanguageModel for Fixed {
        fn score(&self, _prev: Option<&str>, _cur: &str) -> f64 {
            self.0
        }
        fn knows_word(&self, _word: &str) -> Option<bool> {
            Some(self.1)
        }
    }

    #[test]
    fn mixed_score_is_weighted() {
        let m = MixedLm::new(Arc::new(Fixed(-2.0, true)), Arc::new(Fixed(-4.0, false)), 0.25);
        assert!((m.score(None, "x") - (-2.5)).abs() < 1e-9);
        // 认不认识一个词按通用模型：混上领域模型以后上文特征照样有
        assert_eq!(m.knows_word("x"), Some(true));
    }
}
