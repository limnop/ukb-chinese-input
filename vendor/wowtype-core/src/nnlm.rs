//! 字级神经语言模型：读微信输入法 Linux 版自带的 `nlrgpt2_nnlm.bin`，给整句候选重排。
//!
//! 模型文件是腾讯的，**不随 WowType 分发**：用户本机装过微信输入法（或自己下载官方安装包）
//! 才有。这里只放读取和推理代码。文件格式和模型结构见 `docs/DATA.md` 的 NNLM 一节，简单说：
//!
//! - 文件 = 1 字节标志 + u32 长度 + TFLite 模型 + 字表哈希表 + 末尾一段 JSON；
//! - 模型是 3 层 GPT-2（d=256，4 头，词表 5594 个字，最长 512 位置），权重全是 float32；
//! - 字表的值里直接带了字的码位和编号，所以不用重算哈希就能还原出「字 -> 编号」。
//!
//! 推理是纯 Rust 的逐字前向（带 KV 缓存），一步约 4M 次乘加。同一个前缀算过就缓存起来，
//! 用户接着打字时，前面的字不用再算。

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use anyhow::{bail, Context, Result};

const D: usize = 256;
const HEADS: usize = 4;
const HEAD_DIM: usize = 64;
const FF: usize = 1024;
const MAX_POS: usize = 512;
const LAYERS: usize = 3;

/// 特殊编号（模型训练时的约定）。
pub const UNK: u32 = 1;
pub const BOS: u32 = 2;

/// 上下文最多取光标前多少个字。
const MAX_CONTEXT: usize = 24;
/// 前缀缓存最多存多少个状态（每个约 7 KB）。
const CACHE_CAP: usize = 6000;

struct Layer {
    ln1_g: Vec<f32>,
    ln1_b: Vec<f32>,
    attn_w: Vec<f32>, // [3D, D]
    attn_b: Vec<f32>,
    proj_w: Vec<f32>, // [D, D]
    proj_b: Vec<f32>,
    ln2_g: Vec<f32>,
    ln2_b: Vec<f32>,
    fc_w: Vec<f32>, // [FF, D]
    fc_b: Vec<f32>,
    out_w: Vec<f32>, // [D, FF]
    out_b: Vec<f32>,
}

/// 一个前缀算完后留下的东西：最后一个字在各层的 k、v，以及最后一层的输出和 log-sum-exp。
/// 下一个字的 log 概率 = wte[字]·y - lse。
struct State {
    kv: Vec<f32>, // [LAYERS][2][D]
    y: Vec<f32>,  // [D]
    lse: f32,
    used: u64,
}

pub struct Nnlm {
    vocab: HashMap<char, u32>,
    wte: Vec<f32>, // [V, D]
    wpe: Vec<f32>, // [MAX_POS, D]
    layers: Vec<Layer>,
    lnf_g: Vec<f32>,
    lnf_b: Vec<f32>,
    vocab_size: usize,
    cache: Mutex<Cache>,
}

#[derive(Default)]
struct Cache {
    states: HashMap<Vec<u32>, State>,
    clock: u64,
}

impl Nnlm {
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();
        let data = std::fs::read(path).with_context(|| format!("读取 {}", path.display()))?;
        Self::from_bytes(&data).with_context(|| format!("解析 {}", path.display()))
    }

    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        if data.len() < 16 || data[0] != 1 {
            bail!("不是 nlrgpt2_nnlm.bin（文件头不对）");
        }
        let tfl_len = u32_at(data, 1)? as usize;
        let tfl = data.get(5..5 + tfl_len).context("TFLite 段越界")?;
        let mut tensors = read_tflite_tensors(tfl)?;
        let vocab = read_vocab(data, 5 + tfl_len)?;

        const P: &str = "tfgpt2lm_head_custom_model/transformer/";
        // 词表大小从 wte 的长度推出来
        let wte_key = tensors
            .keys()
            .find(|k| k.contains(&format!("{P}wte/Gather/resource")))
            .cloned()
            .context("找不到 wte")?;
        let wte = tensors.remove(&wte_key).unwrap().1;
        if wte.len() % D != 0 {
            bail!("wte 长度 {} 不是 {D} 的倍数", wte.len());
        }
        let vocab_size = wte.len() / D;
        if vocab_size < 1000 || vocab.values().any(|&id| id as usize >= vocab_size) {
            bail!("字表和模型对不上（词表 {vocab_size}）");
        }
        let mut take = |pattern: &[&str], shape: &[usize]| -> Result<Vec<f32>> {
            let hits: Vec<String> = tensors
                .keys()
                .filter(|k| pattern.iter().all(|p| k.contains(p)))
                .cloned()
                .collect();
            if hits.len() != 1 {
                bail!("找不到唯一的权重 {pattern:?}（命中 {}）", hits.len());
            }
            let (got_shape, values) = tensors.remove(&hits[0]).unwrap();
            if values.len() != shape.iter().product::<usize>() {
                bail!("权重 {pattern:?} 形状 {got_shape:?}，期望 {shape:?}");
            }
            Ok(values)
        };
        let wpe = take(&[&format!("{P}Gather/resource")], &[MAX_POS, D])?;
        let mut layers = Vec::with_capacity(LAYERS);
        for l in 0..LAYERS {
            let h = format!("h_._{l}/");
            let h = h.as_str();
            layers.push(Layer {
                ln1_g: take(&[h, "ln_1/batchnorm/mul/"], &[D])?,
                ln1_b: take(&[h, "ln_1/batchnorm/ReadVariableOp/"], &[D])?,
                attn_w: take(&[h, "attn/c_attn/MatMul"], &[3 * D, D])?,
                attn_b: take(&[h, "attn/c_attn/add/"], &[3 * D])?,
                proj_w: take(&[h, "attn/c_proj/MatMul"], &[D, D])?,
                proj_b: take(&[h, "attn/c_proj/add/"], &[D])?,
                ln2_g: take(&[h, "ln_2/batchnorm/mul/"], &[D])?,
                ln2_b: take(&[h, "ln_2/batchnorm/ReadVariableOp/"], &[D])?,
                fc_w: take(&[h, "mlp/c_fc/MatMul"], &[FF, D])?,
                fc_b: take(&[h, "mlp/c_fc/add/"], &[FF])?,
                out_w: take(&[h, "mlp/c_proj/MatMul"], &[D, FF])?,
                out_b: take(&[h, "mlp/c_proj/add/"], &[D])?,
            });
        }
        let lnf_g = take(&[&format!("{P}ln_f/batchnorm/mul/")], &[D])?;
        let lnf_b = take(&[&format!("{P}ln_f/batchnorm/ReadVariableOp/")], &[D])?;
        Ok(Self {
            vocab,
            wte,
            wpe,
            layers,
            lnf_g,
            lnf_b,
            vocab_size,
            cache: Mutex::new(Cache::default()),
        })
    }

    pub fn vocab_size(&self) -> usize {
        self.vocab_size
    }

    /// 字 -> 编号；不认识的字给 `<unk>`。
    pub fn token(&self, ch: char) -> u32 {
        self.vocab.get(&ch).copied().unwrap_or(UNK)
    }

    pub fn knows(&self, ch: char) -> bool {
        self.vocab.contains_key(&ch)
    }

    /// 光标前的文本转成编号前缀：`<s>` + 最后至多 24 个字。遇到换行就从换行后开始。
    pub fn context_tokens(&self, context: &str) -> Vec<u32> {
        let tail = context.rsplit('\n').next().unwrap_or("").trim_start();
        let chars: Vec<char> = tail.chars().filter(|c| !c.is_whitespace()).collect();
        let start = chars.len().saturating_sub(MAX_CONTEXT);
        let mut out = Vec::with_capacity(chars.len() - start + 1);
        out.push(BOS);
        out.extend(chars[start..].iter().map(|&c| self.token(c)));
        out
    }

    /// log P(text | prefix)，逐字相加（自然对数）。prefix 必须以 `<s>` 开头。
    pub fn score(&self, prefix: &[u32], text: &str) -> f64 {
        let tokens: Vec<u32> = text
            .chars()
            .filter(|c| !c.is_whitespace())
            .map(|c| self.token(c))
            .collect();
        self.score_tokens(prefix, &tokens)
    }

    pub fn score_tokens(&self, prefix: &[u32], tokens: &[u32]) -> f64 {
        if prefix.is_empty() || tokens.is_empty() {
            return 0.0;
        }
        let mut seq: Vec<u32> = prefix.to_vec();
        // 太长就截掉开头的上下文（保留 <s>）
        let over = (seq.len() + tokens.len()).saturating_sub(MAX_POS);
        if over > 0 {
            let keep = seq.len().saturating_sub(over).max(1);
            let cut = seq.len() - keep;
            seq.drain(1..1 + cut.min(seq.len() - 1));
            if seq.len() + tokens.len() > MAX_POS {
                return 0.0;
            }
        }
        let Ok(mut cache) = self.cache.lock() else {
            return 0.0;
        };
        cache.clock += 1;
        let mut total = 0.0f64;
        for &tok in tokens {
            self.ensure(&mut cache, &seq);
            let state = cache.states.get(&seq).expect("刚算过");
            total += (self.logit(tok, &state.y) - state.lse) as f64;
            seq.push(tok);
        }
        if cache.states.len() > CACHE_CAP {
            cache.evict();
        }
        total
    }

    /// 下一个字的 log 概率分布里最大的几个（调试用）。
    pub fn top_next(&self, prefix: &[u32], k: usize) -> Vec<(char, f32)> {
        let Ok(mut cache) = self.cache.lock() else {
            return Vec::new();
        };
        cache.clock += 1;
        self.ensure(&mut cache, prefix);
        let state = cache.states.get(prefix).expect("刚算过");
        let inverse: HashMap<u32, char> = self.vocab.iter().map(|(&c, &i)| (i, c)).collect();
        let mut all: Vec<(u32, f32)> = (0..self.vocab_size as u32)
            .map(|t| (t, self.logit(t, &state.y) - state.lse))
            .collect();
        all.sort_by(|a, b| b.1.total_cmp(&a.1));
        all.into_iter()
            .filter_map(|(t, lp)| inverse.get(&t).map(|&c| (c, lp)))
            .take(k)
            .collect()
    }

    fn logit(&self, tok: u32, y: &[f32]) -> f32 {
        let t = (tok as usize).min(self.vocab_size - 1);
        dot(&self.wte[t * D..(t + 1) * D], y)
    }

    /// 保证 seq 和它所有前缀的状态都在缓存里。
    fn ensure(&self, cache: &mut Cache, seq: &[u32]) {
        let clock = cache.clock;
        let mut have = seq.len();
        while have > 0 {
            if let Some(s) = cache.states.get_mut(&seq[..have]) {
                s.used = clock;
                break;
            }
            have -= 1;
        }
        // 前面的状态也标成刚用过，免得马上被清掉
        for i in 1..have {
            if let Some(s) = cache.states.get_mut(&seq[..i]) {
                s.used = clock;
            }
        }
        for end in (have + 1)..=seq.len() {
            let state = {
                let past: Vec<&State> = (1..end)
                    .map(|i| cache.states.get(&seq[..i]).expect("前缀已算"))
                    .collect();
                self.step(seq[end - 1], end - 1, &past, clock)
            };
            cache.states.insert(seq[..end].to_vec(), state);
        }
    }

    fn step(&self, tok: u32, pos: usize, past: &[&State], clock: u64) -> State {
        let t = (tok as usize).min(self.vocab_size - 1);
        let mut x: Vec<f32> = (0..D)
            .map(|i| self.wte[t * D + i] + self.wpe[pos * D + i])
            .collect();
        let mut kv = vec![0.0f32; LAYERS * 2 * D];
        let mut a = vec![0.0f32; D];
        let mut qkv = vec![0.0f32; 3 * D];
        let mut o = vec![0.0f32; D];
        let mut h = vec![0.0f32; FF];
        let mut scores = vec![0.0f32; past.len() + 1];
        for (l, layer) in self.layers.iter().enumerate() {
            layer_norm(&x, &layer.ln1_g, &layer.ln1_b, &mut a);
            matvec(&layer.attn_w, &layer.attn_b, &a, &mut qkv);
            let (q, rest) = qkv.split_at(D);
            let (k, v) = rest.split_at(D);
            kv[(l * 2) * D..(l * 2 + 1) * D].copy_from_slice(k);
            kv[(l * 2 + 1) * D..(l * 2 + 2) * D].copy_from_slice(v);
            for head in 0..HEADS {
                let r = head * HEAD_DIM..(head + 1) * HEAD_DIM;
                let qh = &q[r.clone()];
                for (j, p) in past.iter().enumerate() {
                    let pk = &p.kv[(l * 2) * D..(l * 2 + 1) * D];
                    scores[j] = dot(qh, &pk[r.clone()]) * 0.125;
                }
                scores[past.len()] = dot(qh, &k[r.clone()]) * 0.125;
                let max = scores.iter().copied().fold(f32::NEG_INFINITY, f32::max);
                let mut sum = 0.0;
                for s in scores.iter_mut() {
                    *s = (*s - max).exp();
                    sum += *s;
                }
                let oh = &mut o[r.clone()];
                oh.iter_mut().for_each(|x| *x = 0.0);
                for (j, p) in past.iter().enumerate() {
                    let pv = &p.kv[(l * 2 + 1) * D..(l * 2 + 2) * D];
                    let w = scores[j] / sum;
                    for (dst, src) in oh.iter_mut().zip(&pv[r.clone()]) {
                        *dst += w * src;
                    }
                }
                let w = scores[past.len()] / sum;
                for (dst, src) in oh.iter_mut().zip(&v[r.clone()]) {
                    *dst += w * src;
                }
            }
            matvec_add(&layer.proj_w, &layer.proj_b, &o, &mut x);
            layer_norm(&x, &layer.ln2_g, &layer.ln2_b, &mut a);
            matvec(&layer.fc_w, &layer.fc_b, &a, &mut h);
            h.iter_mut().for_each(|v| *v = gelu(*v));
            matvec_add(&layer.out_w, &layer.out_b, &h, &mut x);
        }
        let mut y = vec![0.0f32; D];
        layer_norm(&x, &self.lnf_g, &self.lnf_b, &mut y);
        // log-sum-exp over 全部词表
        let mut max = f32::NEG_INFINITY;
        let logits: Vec<f32> = self
            .wte
            .chunks_exact(D)
            .map(|row| {
                let v = dot(row, &y);
                max = max.max(v);
                v
            })
            .collect();
        let sum: f32 = logits.iter().map(|v| (v - max).exp()).sum();
        State {
            kv,
            y,
            lse: max + sum.ln(),
            used: clock,
        }
    }
}

impl Cache {
    /// 清掉最久没用的一半。
    fn evict(&mut self) {
        let mut ages: Vec<u64> = self.states.values().map(|s| s.used).collect();
        ages.sort_unstable();
        let cut = ages[ages.len() / 2];
        self.states.retain(|_, s| s.used > cut);
    }
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    let mut acc = [0.0f32; 8];
    let (ca, ra) = a.split_at(a.len() / 8 * 8);
    let (cb, rb) = b.split_at(ca.len());
    for (x, y) in ca.chunks_exact(8).zip(cb.chunks_exact(8)) {
        for i in 0..8 {
            acc[i] += x[i] * y[i];
        }
    }
    let mut s: f32 = acc.iter().sum();
    for (x, y) in ra.iter().zip(rb) {
        s += x * y;
    }
    s
}

/// out = W·x + b，W 按 [out, in] 行存。
fn matvec(w: &[f32], b: &[f32], x: &[f32], out: &mut [f32]) {
    for ((o, row), bias) in out.iter_mut().zip(w.chunks_exact(x.len())).zip(b) {
        *o = dot(row, x) + bias;
    }
}

/// out += W·x + b。
fn matvec_add(w: &[f32], b: &[f32], x: &[f32], out: &mut [f32]) {
    for ((o, row), bias) in out.iter_mut().zip(w.chunks_exact(x.len())).zip(b) {
        *o += dot(row, x) + bias;
    }
}

fn layer_norm(x: &[f32], g: &[f32], b: &[f32], out: &mut [f32]) {
    let n = x.len() as f32;
    let mean = x.iter().sum::<f32>() / n;
    let var = x.iter().map(|v| (v - mean) * (v - mean)).sum::<f32>() / n;
    let inv = 1.0 / (var + 1e-5).sqrt();
    for i in 0..x.len() {
        out[i] = (x[i] - mean) * inv * g[i] + b[i];
    }
}

fn gelu(x: f32) -> f32 {
    const C: f32 = 0.797_884_6; // sqrt(2/pi)
    0.5 * x * (1.0 + (C * (x + 0.044715 * x * x * x)).tanh())
}

fn u32_at(d: &[u8], at: usize) -> Result<u32> {
    let b = d.get(at..at + 4).context("越界")?;
    Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn u16_at(d: &[u8], at: usize) -> Result<u16> {
    let b = d.get(at..at + 2).context("越界")?;
    Ok(u16::from_le_bytes([b[0], b[1]]))
}

/// 字表哈希表：TFLite 段后面第一个 `flags=0x80000001` 的表。
///
/// 表头：flags u32、桶数 u32，然后每桶一个相对表头的 u32 偏移。每桶：n、n 个哈希、
/// n 个 u16（各值的累计结束偏移）、值字节。字的值是 `04 01 <u16 码位> <u8 编号>` 或
/// `14 01 <u16 码位> <u16 编号>`；`<s>` 这类特殊符号的值是别的格式，跳过（编号是约定好的）。
fn read_vocab(data: &[u8], from: usize) -> Result<HashMap<char, u32>> {
    let mut at = from;
    let table = loop {
        if at + 12 > data.len() || at > from + 4096 {
            bail!("找不到字表");
        }
        let flags = u32_at(data, at)?;
        let buckets = u32_at(data, at + 4)? as usize;
        if flags == 0x8000_0001 && (1000..100_000).contains(&buckets) {
            break at;
        }
        at += 1;
    };
    let buckets = u32_at(data, table + 4)? as usize;
    let mut vocab = HashMap::new();
    for b in 0..buckets {
        let off = u32_at(data, table + 8 + 4 * b)? as usize;
        let base = table + off;
        let n = u32_at(data, base)? as usize;
        let ends = base + 4 + 4 * n;
        let values = ends + 2 * n;
        let mut prev = 0usize;
        for i in 0..n {
            let end = u16_at(data, ends + 2 * i)? as usize;
            let v = data
                .get(values + prev..values + end)
                .context("字表值越界")?;
            prev = end;
            let (cp, id) = match v {
                [0x04, 0x01, a, b, id] => (u16::from_le_bytes([*a, *b]) as u32, *id as u32),
                [0x14, 0x01, a, b, i0, i1] => (
                    u16::from_le_bytes([*a, *b]) as u32,
                    u16::from_le_bytes([*i0, *i1]) as u32,
                ),
                _ => continue,
            };
            if let Some(ch) = char::from_u32(cp) {
                vocab.insert(ch, id);
            }
        }
    }
    if vocab.len() < 1000 {
        bail!("字表只读出 {} 个字", vocab.len());
    }
    Ok(vocab)
}

// ---- 最小的 TFLite（FlatBuffers）读取：只取 float32 常量张量的名字、形状和数据 ----

struct Fb<'a> {
    d: &'a [u8],
}

impl<'a> Fb<'a> {
    fn u32(&self, at: usize) -> Result<u32> {
        u32_at(self.d, at)
    }
    /// 表的第 field 个字段的位置（没有这个字段返回 None）。
    fn field(&self, table: usize, field: usize) -> Result<Option<usize>> {
        let soff = self.u32(table)? as i32;
        let vt = (table as i64 - soff as i64) as usize;
        let vt_len = u16_at(self.d, vt)? as usize;
        let slot = 4 + 2 * field;
        if slot + 2 > vt_len {
            return Ok(None);
        }
        let off = u16_at(self.d, vt + slot)? as usize;
        Ok((off != 0).then_some(table + off))
    }
    fn deref(&self, at: usize) -> Result<usize> {
        Ok(at + self.u32(at)? as usize)
    }
    fn table_field(&self, table: usize, field: usize) -> Result<Option<usize>> {
        match self.field(table, field)? {
            Some(at) => Ok(Some(self.deref(at)?)),
            None => Ok(None),
        }
    }
    /// 向量：(元素起点, 长度)。
    fn vector(&self, table: usize, field: usize) -> Result<Option<(usize, usize)>> {
        match self.table_field(table, field)? {
            Some(v) => Ok(Some((v + 4, self.u32(v)? as usize))),
            None => Ok(None),
        }
    }
}

#[allow(clippy::type_complexity)]
fn read_tflite_tensors(d: &[u8]) -> Result<HashMap<String, (Vec<usize>, Vec<f32>)>> {
    let fb = Fb { d };
    let root = fb.deref(0)?;
    let (subgraphs, n_sub) = fb.vector(root, 2)?.context("没有 subgraphs")?;
    let (buffers, n_buf) = fb.vector(root, 4)?.context("没有 buffers")?;
    if n_sub == 0 {
        bail!("subgraphs 为空");
    }
    let sub = fb.deref(subgraphs)?;
    let (tensors, n_t) = fb.vector(sub, 0)?.context("没有 tensors")?;
    let mut out = HashMap::new();
    for i in 0..n_t {
        let t = fb.deref(tensors + 4 * i)?;
        // Tensor: shape(0) type(1) buffer(2) name(3)
        let ty = match fb.field(t, 1)? {
            Some(at) => *d.get(at).context("越界")?,
            None => 0,
        };
        if ty != 0 {
            continue; // 只要 FLOAT32
        }
        let buf_idx = match fb.field(t, 2)? {
            Some(at) => fb.u32(at)? as usize,
            None => 0,
        };
        if buf_idx == 0 || buf_idx >= n_buf {
            continue;
        }
        let buffer = fb.deref(buffers + 4 * buf_idx)?;
        let Some((data_at, len)) = fb.vector(buffer, 0)? else {
            continue;
        };
        if len == 0 || len % 4 != 0 {
            continue;
        }
        let name = match fb.table_field(t, 3)? {
            Some(s) => {
                let n = fb.u32(s)? as usize;
                String::from_utf8_lossy(d.get(s + 4..s + 4 + n).context("越界")?).into_owned()
            }
            None => continue,
        };
        let shape = match fb.vector(t, 0)? {
            Some((at, n)) => (0..n)
                .map(|j| fb.u32(at + 4 * j).map(|v| v as usize))
                .collect::<Result<Vec<_>>>()?,
            None => Vec::new(),
        };
        let bytes = d.get(data_at..data_at + len).context("张量数据越界")?;
        let values: Vec<f32> = bytes
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect();
        out.insert(name, (shape, values));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 本机有模型文件时才跑（模型不随仓库分发）。WOWTYPE_NNLM 可指定路径。
    fn model() -> Option<Nnlm> {
        let path = std::env::var("WOWTYPE_NNLM").ok().or_else(|| {
            let home = std::env::var("HOME").ok()?;
            Some(format!(
                "{home}/.local/share/wowtype/wetype/nlrgpt2_nnlm.bin"
            ))
        })?;
        if !Path::new(&path).exists() {
            eprintln!("没有 {path}，跳过");
            return None;
        }
        Some(Nnlm::load(path).expect("加载模型"))
    }

    #[test]
    fn matches_reference_scores() {
        let Some(m) = model() else { return };
        assert_eq!(m.vocab_size(), 5594);
        assert_eq!(m.token('，'), 4);
        assert_eq!(m.token('的'), 5);
        // 数值来自 TFLite 官方运行时（LiteRT）
        let bos = [BOS];
        let a = m.score(&bos, "我今天很开心");
        assert!((a + 15.28).abs() < 0.05, "{a}");
        let b = m.score(&bos, "我今天很开薪");
        assert!(b < a - 15.0, "{b}");
        assert!(m.score(&bos, "我在家里") > m.score(&bos, "我再家里") + 5.0);
        let ctx = m.context_tokens("今天天气");
        let top: String = m.top_next(&ctx, 3).into_iter().map(|(c, _)| c).collect();
        assert!(top.contains('好'), "{top}");
    }

    #[test]
    fn cache_gives_same_result() {
        let Some(m) = model() else { return };
        let ctx = m.context_tokens("我们明天");
        let first = m.score(&ctx, "一起去吃饭");
        let again = m.score(&ctx, "一起去吃饭");
        assert_eq!(first, again);
        m.cache.lock().unwrap().states.clear();
        let fresh = m.score(&ctx, "一起去吃饭");
        assert!((first - fresh).abs() < 1e-4);
    }
}
