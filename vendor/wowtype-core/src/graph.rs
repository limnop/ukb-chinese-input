//! 音节图：把输入串切成所有合法音节序列（DAG）。
//!
//! 每条边消耗一个音节（全拼）或一个首字母（简拼）。隔音符产生的强制边界
//! 会禁止跨过该位置的边；儿化边会消耗掉尾部的 r，并把音节标记为儿化。

use crate::syllable::{
    is_jianpin_letter, jianpin_digraph, parse, ParsedInput, Syllable, SyllableQuery,
    MAX_SYLLABLE_LEN,
};

/// 建图模式。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphMode {
    /// 只允许完整音节。
    Full,
    /// 只允许每个音节取首字母。
    Jianpin,
    /// 同时允许两种边（会产生较多路径，默认不用于 Engine）。
    Mixed,
}

/// 建图选项。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GraphOptions {
    pub mode: GraphMode,
    pub allow_erhua: bool,
    /// 是否加纠错边（按错相邻键、两键按反、多按/漏按一个键）。只对全拼生效。
    pub typo: bool,
    /// 是否给尾部没打完的音节加补全边（nih -> ni + hao）。只对全拼生效。
    pub completion: bool,
}

impl Default for GraphOptions {
    fn default() -> Self {
        Self {
            mode: GraphMode::Full,
            allow_erhua: true,
            typo: false,
            completion: false,
        }
    }
}

/// 一条边。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Edge {
    pub start: usize,
    pub end: usize,
    pub kind: EdgeKind,
    /// 这条边是纠错得来的（与实际按键差一处）。
    pub typo: bool,
    /// 补全边：音节比实际输入多出的字母数（0 表示不是补全）。
    pub completion: u8,
}

/// 边的类型。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EdgeKind {
    Full {
        syllable: Syllable,
        erhua: bool,
    },
    Jianpin {
        letter: u8,
    },
    /// 跨音节的两键按反（nhiao -> ni hao）：一条边消耗两个音节。
    Pair {
        first: Syllable,
        second: Syllable,
    },
}

impl Edge {
    /// 第一个音节的查询（Pair 边还有第二个，见 push_queries）。
    pub fn query(&self) -> SyllableQuery {
        match self.kind {
            EdgeKind::Full { syllable, erhua } => SyllableQuery::Full {
                id: syllable.id,
                erhua,
            },
            EdgeKind::Jianpin { letter } => SyllableQuery::Jianpin { letter },
            EdgeKind::Pair { first, .. } => SyllableQuery::Full {
                id: first.id,
                erhua: false,
            },
        }
    }

    /// 把这条边的所有音节查询追加到 out。
    pub fn push_queries(&self, out: &mut Vec<SyllableQuery>) {
        out.push(self.query());
        if let EdgeKind::Pair { second, .. } = self.kind {
            out.push(SyllableQuery::Full {
                id: second.id,
                erhua: false,
            });
        }
    }

    pub fn is_jianpin(&self) -> bool {
        matches!(self.kind, EdgeKind::Jianpin { .. })
    }

    /// 供日志/未知词兜底使用的文本。
    pub fn display(&self) -> String {
        let mut queries = Vec::with_capacity(2);
        self.push_queries(&mut queries);
        queries
            .into_iter()
            .map(SyllableQuery::display)
            .collect::<Vec<_>>()
            .join("")
    }

    pub fn syllable_count(&self) -> usize {
        match self.kind {
            EdgeKind::Pair { .. } => 2,
            _ => 1,
        }
    }
}

/// 音节图。
#[derive(Clone, Debug)]
pub struct SyllableGraph {
    input: ParsedInput,
    edges: Vec<Vec<Edge>>,
    mode: GraphMode,
}

impl SyllableGraph {
    /// 从输入串直接建图。
    pub fn from_str(
        input: &str,
        options: GraphOptions,
    ) -> Result<Self, crate::syllable::ParseError> {
        let parsed = parse(input)?;
        Ok(Self::build(&parsed, options))
    }

    /// 用已解析的输入建图。
    pub fn build(input: &ParsedInput, options: GraphOptions) -> Self {
        let n = input.len();
        let mut edges = vec![Vec::new(); n + 1];

        if matches!(options.mode, GraphMode::Full | GraphMode::Mixed) {
            build_full_edges(input, options.allow_erhua, &mut edges);
        }
        if matches!(options.mode, GraphMode::Jianpin | GraphMode::Mixed) {
            build_jianpin_edges(input, &mut edges);
        }
        if options.typo && matches!(options.mode, GraphMode::Full | GraphMode::Mixed) {
            build_typo_edges(input, &mut edges);
        }
        if options.completion && matches!(options.mode, GraphMode::Full | GraphMode::Mixed) {
            build_completion_edges(input, &mut edges);
        }

        Self {
            input: input.clone(),
            edges,
            mode: options.mode,
        }
    }

    /// 输入长度（节点数 = len + 1）。
    pub fn len(&self) -> usize {
        self.input.len()
    }

    pub fn is_empty(&self) -> bool {
        self.input.is_empty()
    }

    pub fn mode(&self) -> GraphMode {
        self.mode
    }

    pub fn input(&self) -> &ParsedInput {
        &self.input
    }

    pub fn edges_from(&self, pos: usize) -> &[Edge] {
        self.edges.get(pos).map(Vec::as_slice).unwrap_or(&[])
    }

    /// 是否存在从 0 到 n 的完整路径。
    pub fn has_complete_path(&self) -> bool {
        self.has_path(|_| true)
    }

    /// 不靠纠错边能否走完（补全可以）。
    pub fn has_path_without_typo(&self) -> bool {
        self.has_path(|e| !e.typo)
    }

    /// 不靠纠错边、补全边能否走完。
    pub fn has_exact_path(&self) -> bool {
        self.has_path(|e| !e.typo && e.completion == 0)
    }

    fn has_path(&self, allow: impl Fn(&Edge) -> bool) -> bool {
        let n = self.input.len();
        if n == 0 {
            return true;
        }
        let mut reachable = vec![false; n + 1];
        reachable[n] = true;
        for pos in (0..n).rev() {
            reachable[pos] = self.edges[pos].iter().any(|e| allow(e) && reachable[e.end]);
        }
        reachable[0]
    }

    /// 枚举全部完整切分（最多 limit 条，避免歧义输入爆炸）。
    pub fn segmentations(&self, limit: usize) -> Vec<Vec<Edge>> {
        let mut out = Vec::new();
        if limit == 0 || !self.has_complete_path() {
            return out;
        }
        let mut path = Vec::new();
        self.dfs(0, &mut path, &mut out, limit);
        out
    }

    fn dfs(&self, pos: usize, path: &mut Vec<Edge>, out: &mut Vec<Vec<Edge>>, limit: usize) {
        if out.len() >= limit {
            return;
        }
        if pos == self.input.len() {
            out.push(path.clone());
            return;
        }
        for edge in self.edges_from(pos) {
            path.push(*edge);
            self.dfs(edge.end, path, out, limit);
            path.pop();
            if out.len() >= limit {
                return;
            }
        }
    }
}

fn add_edge(edges: &mut [Vec<Edge>], edge: Edge) {
    let bucket = &mut edges[edge.start];
    // 同一段、同一个音节只留一条：精确边先加，纠错边遇到同样的音节就不再加
    if !bucket
        .iter()
        .any(|e| e.end == edge.end && e.kind == edge.kind)
    {
        bucket.push(edge);
    }
}

fn full_edge(start: usize, end: usize, syllable: Syllable, erhua: bool) -> Edge {
    Edge {
        start,
        end,
        kind: EdgeKind::Full { syllable, erhua },
        typo: false,
        completion: 0,
    }
}

/// 补全边：尾部 [start, n) 是某些音节的真前缀时，连一条到终点的边。
///
/// 一次搜索就覆盖所有补全，不必像以前那样对每个补全串各跑一遍。
fn build_completion_edges(input: &ParsedInput, edges: &mut [Vec<Edge>]) {
    let n = input.len();
    for start in n.saturating_sub(MAX_SYLLABLE_LEN - 1)..n {
        if input.forced_between(start, n) {
            continue;
        }
        let tail = &input.letters[start..n];
        for text in crate::syllable::SYLLABLES {
            let bytes = text.as_bytes();
            if bytes.len() > tail.len() && bytes.starts_with(tail) {
                if let Some(syllable) = Syllable::from_text(text) {
                    let mut edge = full_edge(start, n, syllable, false);
                    edge.completion = (bytes.len() - tail.len()) as u8;
                    add_edge(edges, edge);
                }
            }
        }
    }
}

/// QWERTY 键盘上的相邻键（同排左右 + 上下排斜邻）。
pub fn keyboard_neighbors(letter: u8) -> &'static [u8] {
    match letter {
        b'q' => b"wa",
        b'w' => b"qesa",
        b'e' => b"wrds",
        b'r' => b"etfd",
        b't' => b"rygf",
        b'y' => b"tuhg",
        b'u' => b"yijh",
        b'i' => b"uokj",
        b'o' => b"iplk",
        b'p' => b"ol",
        b'a' => b"sqwz",
        b's' => b"adwezx",
        b'd' => b"sferxc",
        b'f' => b"dgrtcv",
        b'g' => b"fhtyvb",
        b'h' => b"gjyubn",
        b'j' => b"hkuinm",
        b'k' => b"jliom",
        b'l' => b"kop",
        b'z' => b"xas",
        b'x' => b"zcsd",
        b'c' => b"xvdf",
        b'v' => b"cbfg",
        b'b' => b"vngh",
        b'n' => b"bmhj",
        b'm' => b"njk",
        _ => b"",
    }
}

fn is_neighbor(a: u8, b: u8) -> bool {
    keyboard_neighbors(a).contains(&b)
}

fn syllable_of(bytes: &[u8]) -> Option<Syllable> {
    std::str::from_utf8(bytes)
        .ok()
        .and_then(Syllable::from_text)
}

/// 纠错边：每条边只纠一处。
///
/// - 替换：某个字母换成键盘上的相邻键（wrnti -> wenti）
/// - 对调：音节内相邻两键按反（zhnog -> zhong）
/// - 多按：多出来的那个键和前后某个键相同或相邻（haoo / haio -> hao）
/// - 漏按：补一个字母（zhon -> zhong 由补全负责，这里管中间漏的：zhng -> zhong）
/// - 跨音节对调：nhiao -> ni hao（Pair 边）
fn build_typo_edges(input: &ParsedInput, edges: &mut [Vec<Edge>]) {
    let n = input.len();
    let letters = &input.letters;
    let mut buf: Vec<u8> = Vec::with_capacity(MAX_SYLLABLE_LEN + 1);
    let add = |edges: &mut [Vec<Edge>], start: usize, end: usize, syllable: Syllable| {
        let mut edge = full_edge(start, end, syllable, false);
        edge.typo = true;
        add_edge(edges, edge);
    };

    for start in 0..n {
        let max_end = (start + MAX_SYLLABLE_LEN + 1).min(n);
        for end in (start + 1)..=max_end {
            if input.forced_between(start, end) {
                continue;
            }
            let text = &letters[start..end];
            let len = text.len();

            if len <= MAX_SYLLABLE_LEN {
                // 替换
                for i in 0..len {
                    for &c in keyboard_neighbors(text[i]) {
                        buf.clear();
                        buf.extend_from_slice(text);
                        buf[i] = c;
                        if let Some(s) = syllable_of(&buf) {
                            add(edges, start, end, s);
                        }
                    }
                }
                // 音节内对调
                for i in 0..len.saturating_sub(1) {
                    if text[i] == text[i + 1] {
                        continue;
                    }
                    buf.clear();
                    buf.extend_from_slice(text);
                    buf.swap(i, i + 1);
                    if let Some(s) = syllable_of(&buf) {
                        add(edges, start, end, s);
                    }
                }
            }

            // 多按一个键：只删和左右邻居相同或相邻的键，避免把正常字母当成多余
            if len >= 2 {
                for i in 0..len {
                    let c = text[i];
                    let left = (i > 0).then(|| text[i - 1]);
                    let right = (i + 1 < len).then(|| text[i + 1]);
                    let stray = [left, right]
                        .into_iter()
                        .flatten()
                        .any(|o| o == c || is_neighbor(o, c));
                    if !stray {
                        continue;
                    }
                    buf.clear();
                    buf.extend_from_slice(&text[..i]);
                    buf.extend_from_slice(&text[i + 1..]);
                    if let Some(s) = syllable_of(&buf) {
                        add(edges, start, end, s);
                    }
                }
            }

            // 漏按一个键：只补在音节中间（末尾漏的交给补全，开头漏的歧义太大）
            if (2..MAX_SYLLABLE_LEN).contains(&len) {
                for i in 1..len {
                    for c in b'a'..=b'z' {
                        buf.clear();
                        buf.extend_from_slice(&text[..i]);
                        buf.push(c);
                        buf.extend_from_slice(&text[i..]);
                        if let Some(s) = syllable_of(&buf) {
                            add(edges, start, end, s);
                        }
                    }
                }
            }
        }
    }

    // 跨音节对调：交换 k 与 k+1 后，在两者之间切开得到两个音节
    for k in 0..n.saturating_sub(1) {
        if letters[k] == letters[k + 1] || input.forced_between(k, k + 2) {
            continue;
        }
        let mut swapped = letters.clone();
        swapped.swap(k, k + 1);
        let a_min = (k + 1).saturating_sub(MAX_SYLLABLE_LEN);
        for a in a_min..=k {
            if input.forced_between(a, k + 1) {
                continue;
            }
            let Some(first) = syllable_of(&swapped[a..=k]) else {
                continue;
            };
            let b_max = (k + 1 + MAX_SYLLABLE_LEN).min(n);
            for b in (k + 2)..=b_max {
                if input.forced_between(k + 1, b) {
                    continue;
                }
                if let Some(second) = syllable_of(&swapped[k + 1..b]) {
                    add_edge(
                        edges,
                        Edge {
                            start: a,
                            end: b,
                            kind: EdgeKind::Pair { first, second },
                            typo: true,
                            completion: 0,
                        },
                    );
                }
            }
        }
    }
}

fn build_full_edges(input: &ParsedInput, allow_erhua: bool, edges: &mut [Vec<Edge>]) {
    let n = input.len();
    for start in 0..n {
        let max_end = (start + MAX_SYLLABLE_LEN).min(n);
        for end in (start + 1)..=max_end {
            if input.forced_between(start, end) {
                continue;
            }
            // parse 保证是 ASCII，这里可以安全地做字节切片。
            let text = std::str::from_utf8(&input.letters[start..end]).expect("ASCII 输入");
            if let Some(syllable) = Syllable::from_text(text) {
                add_edge(edges, full_edge(start, end, syllable, false));
                if input.has_erhua(end) {
                    add_edge(edges, full_edge(start, end, syllable, true));
                }
            }

            // 尾部 r 儿化：huar -> hua + 儿化。er 本身是独立音节，不当作 e 的儿化。
            if allow_erhua && text.len() > 1 && text.ends_with('r') && text != "er" {
                let stem = &text[..text.len() - 1];
                if let Some(syllable) = Syllable::from_text(stem) {
                    add_edge(edges, full_edge(start, end, syllable, true));
                }
            }
        }
    }
}

fn build_jianpin_edges(input: &ParsedInput, edges: &mut [Vec<Edge>]) {
    for (pos, &letter) in input.letters.iter().enumerate() {
        // zh/ch/sh 也可以只打声母：zhch -> 支持（z h 拆开的那条路照样保留）
        if let Some(&next) = input.letters.get(pos + 1) {
            if let Some(digraph) = jianpin_digraph(letter, next) {
                if !input.forced_between(pos, pos + 2) {
                    add_edge(
                        edges,
                        Edge {
                            start: pos,
                            end: pos + 2,
                            kind: EdgeKind::Jianpin { letter: digraph },
                            typo: false,
                            completion: 0,
                        },
                    );
                }
            }
        }
        if is_jianpin_letter(letter) {
            add_edge(
                edges,
                Edge {
                    start: pos,
                    end: pos + 1,
                    kind: EdgeKind::Jianpin { letter },
                    typo: false,
                    completion: 0,
                },
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph(text: &str, mode: GraphMode) -> SyllableGraph {
        let parsed = parse(text).unwrap();
        SyllableGraph::build(
            &parsed,
            GraphOptions {
                mode,
                allow_erhua: true,
                typo: false,
                completion: false,
            },
        )
    }

    fn texts(path: &[Edge]) -> Vec<String> {
        path.iter().map(Edge::display).collect()
    }

    #[test]
    fn xian_has_two_segmentations() {
        let g = graph("xian", GraphMode::Full);
        let segs = g.segmentations(16);
        assert_eq!(
            segs.len(),
            2,
            "xian 应有 xian 与 xi+an 两种切分: {:?}",
            segs
        );
        assert!(segs.iter().any(|p| texts(p) == vec!["xian"]));
        assert!(segs.iter().any(|p| texts(p) == vec!["xi", "an"]));
    }

    #[test]
    fn apostrophe_forces_xi_an() {
        let g = graph("xi'an", GraphMode::Full);
        let segs = g.segmentations(16);
        assert_eq!(segs.len(), 1);
        assert_eq!(texts(&segs[0]), vec!["xi", "an"]);
    }

    #[test]
    fn jianpin_zg() {
        let g = graph("zg", GraphMode::Jianpin);
        assert!(g.has_complete_path());
        let segs = g.segmentations(8);
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].len(), 2);
        assert_eq!(segs[0][0].query(), SyllableQuery::Jianpin { letter: b'z' });
        assert_eq!(segs[0][1].query(), SyllableQuery::Jianpin { letter: b'g' });
    }

    #[test]
    fn jianpin_digraph_initials() {
        // zh 既可以是一个声母，也可以是 z、h 两个
        let g = graph("zhch", GraphMode::Jianpin);
        let segs = g.segmentations(16);
        let zh_ch = vec![
            SyllableQuery::Jianpin { letter: b'Z' },
            SyllableQuery::Jianpin { letter: b'C' },
        ];
        assert!(segs
            .iter()
            .any(|p| p.iter().map(|e| e.query()).collect::<Vec<_>>() == zh_ch));
        assert!(segs.iter().any(|p| p.len() == 4));
        // 隔音符隔开就不算 zh
        let g = graph("z'h", GraphMode::Jianpin);
        assert!(g.segmentations(8).iter().all(|p| p.len() == 2));
        // 混着全拼：zh + chi
        let g = graph("zhchi", GraphMode::Mixed);
        assert!(g
            .segmentations(32)
            .iter()
            .any(|p| p.len() == 2 && p[0].query() == SyllableQuery::Jianpin { letter: b'Z' }));
    }

    #[test]
    fn jianpin_rejects_non_initial() {
        let g = graph("za", GraphMode::Jianpin);
        assert!(!g.has_complete_path(), "a 不能作简拼首字母");
    }

    #[test]
    fn trailing_r_marks_erhua() {
        let g = graph("huar", GraphMode::Full);
        let segs = g.segmentations(16);
        assert!(
            segs.iter()
                .any(|p| p.len() == 1 && texts(p) == vec!["huar"]),
            "huar 应产生 hua 的儿化边: {:?}",
            segs
        );
    }

    #[test]
    fn apostrophe_r_marks_erhua() {
        let g = graph("hua'r", GraphMode::Full);
        let segs = g.segmentations(16);
        assert!(
            segs.iter()
                .any(|p| p.len() == 1 && texts(p) == vec!["huar"]),
            "hua'r 应产生 hua 的儿化边: {:?}",
            segs
        );
    }

    fn typo_graph(text: &str) -> SyllableGraph {
        SyllableGraph::build(
            &parse(text).unwrap(),
            GraphOptions {
                mode: GraphMode::Full,
                allow_erhua: true,
                typo: true,
                completion: false,
            },
        )
    }

    fn has_typo_path(text: &str, want: &[&str]) -> bool {
        let g = typo_graph(text);
        g.segmentations(4096).iter().any(|p| {
            let mut q = Vec::new();
            for e in p {
                e.push_queries(&mut q);
            }
            q.iter().map(|x| x.display()).collect::<Vec<_>>() == want
        })
    }

    #[test]
    fn typo_edges_cover_common_mistakes() {
        assert!(has_typo_path("nihso", &["ni", "hao"]), "相邻键替换");
        assert!(has_typo_path("zhnog", &["zhong"]), "音节内对调");
        assert!(has_typo_path("nhiao", &["ni", "hao"]), "跨音节对调");
        assert!(has_typo_path("haoo", &["hao"]), "多按");
        assert!(has_typo_path("zhng", &["zhong"]), "漏按");
    }

    #[test]
    fn typo_edges_are_off_by_default_and_marked() {
        assert!(!graph("nihso", GraphMode::Full).has_complete_path());
        let g = typo_graph("nihso");
        assert!(g.has_complete_path());
        assert!(!g.has_exact_path());
        assert!(typo_graph("nihao").has_exact_path());
    }
}
