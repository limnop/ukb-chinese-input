//! 编辑区里的拼音分词：`hengxiang` 显示成 `heng'xiang`。
//!
//! 按候选的拼音（`heng xiang`，空格分隔）把用户实际打的字母切开。显示的仍是用户打的字母
//! （大小写、按错的键都原样保留），只在音节之间加隔音符：
//!
//! - 打了一半的最后一个音节照样切：`zhongg` + `zhong guo` -> `zhong'g`
//! - 简拼、省略韵母：`bjdx` + `b j d x` -> `b'j'd'x`，`kyi` + `k yi` -> `k'yi`
//! - 按错键：`zhognguo` + `zhong guo` -> `zhogn'guo`（对齐时容许少量编辑）
//! - 用户自己打的隔音符留着，不重复加
//!
//! 对不上（英文候选、双拼、差得太多）返回 None，前端照原样显示。

/// 音节之间显示的分隔符。
pub const SEPARATOR: char = '\'';

/// 整段对齐最多容许的编辑次数（引擎对一个词最多纠两处错）。
const MAX_EDITS: usize = 3;

/// 按候选拼音切分输入；切不了返回 None。
pub fn segment_input(raw: &str, pinyin: &str) -> Option<String> {
    let syllables: Vec<Vec<u8>> = pinyin
        .split_whitespace()
        .map(|s| s.to_ascii_lowercase().into_bytes())
        .collect();
    if syllables.len() < 2
        || syllables
            .iter()
            .any(|s| !s.iter().all(u8::is_ascii_lowercase))
    {
        return None;
    }

    // 用户打的字母（小写，用来对齐）和用户自己打的隔音符位置（按字母下标）
    let mut letters = Vec::new();
    let mut forced = Vec::new();
    for b in raw.bytes() {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' => letters.push(b.to_ascii_lowercase()),
            b'\'' => forced.push(letters.len()),
            _ => return None,
        }
    }
    let n = letters.len();
    let k = syllables.len();
    if n < k {
        return None;
    }

    // dp[j][i]：前 j 个音节对齐前 i 个字母的最少编辑次数
    const INF: usize = usize::MAX / 2;
    let mut dp = vec![vec![INF; n + 1]; k + 1];
    let mut from = vec![vec![0usize; n + 1]; k + 1];
    dp[0][0] = 0;
    for j in 1..=k {
        let syl = &syllables[j - 1];
        let last = j == k;
        for i in j..=n {
            // 最后一个音节可能只打了开头几个字母
            let max_len = syl.len() + 2;
            for a in (j - 1)..i {
                if dp[j - 1][a] >= INF || i - a > max_len {
                    continue;
                }
                // 用户打的隔音符不能落在一个音节中间
                if forced.iter().any(|&f| f > a && f < i) {
                    continue;
                }
                let seg = &letters[a..i];
                let cost = if last {
                    (1..=syl.len())
                        .map(|p| osa(seg, &syl[..p]))
                        .min()
                        .unwrap_or(INF)
                } else {
                    osa(seg, syl)
                };
                let total = dp[j - 1][a] + cost;
                if total < dp[j][i] {
                    dp[j][i] = total;
                    from[j][i] = a;
                }
            }
        }
    }
    if dp[k][n] > MAX_EDITS {
        return None;
    }

    // 回溯出切分点（字母下标）
    let mut cuts = Vec::with_capacity(k - 1);
    let mut i = n;
    for j in (2..=k).rev() {
        i = from[j][i];
        cuts.push(i);
    }
    cuts.reverse();

    let mut out = String::with_capacity(raw.len() + cuts.len());
    let mut seen = 0usize;
    let mut cut = cuts.iter().peekable();
    let mut last_was_sep = true;
    for ch in raw.chars() {
        if ch == '\'' {
            if !last_was_sep {
                out.push(SEPARATOR);
                last_was_sep = true;
            }
            continue;
        }
        while cut.peek().is_some_and(|&&c| c == seen) {
            cut.next();
            if !last_was_sep {
                out.push(SEPARATOR);
                last_was_sep = true;
            }
        }
        out.push(ch);
        last_was_sep = false;
        seen += 1;
    }
    if raw.ends_with('\'') && !out.ends_with(SEPARATOR) {
        out.push(SEPARATOR);
    }
    Some(out)
}

/// 编辑距离（相邻两字母互换算一次）。
fn osa(a: &[u8], b: &[u8]) -> usize {
    let (n, m) = (a.len(), b.len());
    let mut d = vec![vec![0usize; m + 1]; n + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for j in 0..=m {
        d[0][j] = j;
    }
    for i in 1..=n {
        for j in 1..=m {
            let sub = usize::from(a[i - 1] != b[j - 1]);
            let mut v = (d[i - 1][j] + 1)
                .min(d[i][j - 1] + 1)
                .min(d[i - 1][j - 1] + sub);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                v = v.min(d[i - 2][j - 2] + 1);
            }
            d[i][j] = v;
        }
    }
    d[n][m]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(raw: &str, pinyin: &str) -> Option<String> {
        segment_input(raw, pinyin)
    }

    #[test]
    fn full_pinyin() {
        assert_eq!(
            seg("hengxiang", "heng xiang").as_deref(),
            Some("heng'xiang")
        );
        assert_eq!(
            seg("wojintianwanshang", "wo jin tian wan shang").as_deref(),
            Some("wo'jin'tian'wan'shang")
        );
        // 同样的字母，按候选的切法：方案 fang an
        assert_eq!(seg("fangan", "fang an").as_deref(), Some("fang'an"));
        assert_eq!(seg("fangan", "fan gan").as_deref(), Some("fan'gan"));
    }

    #[test]
    fn partial_abbrev_and_typos() {
        assert_eq!(seg("zhongg", "zhong guo").as_deref(), Some("zhong'g"));
        assert_eq!(seg("bjdx", "b j d x").as_deref(), Some("b'j'd'x"));
        assert_eq!(seg("kyi", "k yi").as_deref(), Some("k'yi"));
        assert_eq!(seg("yongzwen", "yong z wen").as_deref(), Some("yong'z'wen"));
        assert_eq!(seg("zhognguo", "zhong guo").as_deref(), Some("zhogn'guo"));
        assert_eq!(seg("nihoa", "ni hao").as_deref(), Some("ni'hoa"));
        assert_eq!(seg("zhchi", "zh chi").as_deref(), Some("zh'chi"));
        assert_eq!(seg("zhch", "zh ch").as_deref(), Some("zh'ch"));
    }

    #[test]
    fn keeps_user_input() {
        assert_eq!(seg("xi'an", "xi an").as_deref(), Some("xi'an"));
        assert_eq!(seg("NiHao", "ni hao").as_deref(), Some("Ni'Hao"));
        assert_eq!(seg("nihao'", "ni hao").as_deref(), Some("ni'hao'"));
    }

    #[test]
    fn gives_up_when_it_does_not_fit() {
        assert_eq!(seg("nihao", "nihao"), None); // 一个音节（或英文候选）
        assert_eq!(seg("vsgo", "zhong guo"), None); // 双拼
        assert_eq!(seg("/juhao", "ju hao"), None);
    }
}
