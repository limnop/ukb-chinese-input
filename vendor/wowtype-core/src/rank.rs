//! 候选特征、基础线性打分，以及 LTR 钩子。
//!
//! 特征名严格采用 WeType 排序链路中已确认的 23 个名字。默认权重只是让
//! 骨架能跑通的占位值，并非从 WeType 超参数文件还原的数值；接入
//! hpmac_base.bin / LightGBM 模型后再标定。

use std::sync::Arc;

/// 特征名列表，顺序必须与 Feature 字段一致（也用于 LTR 的向量化）。
pub const FEATURE_NAMES: [&str; 23] = [
    "kind",
    "typo_count",
    "history_count",
    "complete_count",
    "history_time_diff",
    "context_bigram_score",
    "user_word_rollback_score",
    "lm_score_with_previous_txt",
    "head_word_nnlm",
    "hot_val_conf",
    "with_context",
    "user_tail_conf",
    "coverage_score",
    "ltr_valid",
    "history_use_count",
    "history_hot_val",
    "prob",
    "context_flag",
    "full_jianpin",
    "prefix_completion",
    "other_completion",
    "typo",
    "force_emit",
];

pub const FEATURE_COUNT: usize = FEATURE_NAMES.len();

/// 候选特征。除 kind 外都按浮点数处理，方便直接喂给线性/LTR 模型。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Feature {
    /// 候选类型：0=词，1=整句；分类特征。
    pub kind: f32,
    pub typo_count: f32,
    pub history_count: f32,
    pub complete_count: f32,
    pub history_time_diff: f32,
    pub context_bigram_score: f32,
    pub user_word_rollback_score: f32,
    pub lm_score_with_previous_txt: f32,
    pub head_word_nnlm: f32,
    pub hot_val_conf: f32,
    pub with_context: f32,
    pub user_tail_conf: f32,
    pub coverage_score: f32,
    pub ltr_valid: f32,
    pub history_use_count: f32,
    pub history_hot_val: f32,
    pub prob: f32,
    pub context_flag: f32,
    pub full_jianpin: f32,
    pub prefix_completion: f32,
    pub other_completion: f32,
    pub typo: f32,
    pub force_emit: f32,
}

impl Feature {
    pub fn to_vector(self) -> [f32; FEATURE_COUNT] {
        [
            self.kind,
            self.typo_count,
            self.history_count,
            self.complete_count,
            self.history_time_diff,
            self.context_bigram_score,
            self.user_word_rollback_score,
            self.lm_score_with_previous_txt,
            self.head_word_nnlm,
            self.hot_val_conf,
            self.with_context,
            self.user_tail_conf,
            self.coverage_score,
            self.ltr_valid,
            self.history_use_count,
            self.history_hot_val,
            self.prob,
            self.context_flag,
            self.full_jianpin,
            self.prefix_completion,
            self.other_completion,
            self.typo,
            self.force_emit,
        ]
    }
}

/// 线性权重。字段与 Feature 一一对应。
#[derive(Clone, Copy, Debug)]
pub struct FeatureWeights {
    pub kind: f32,
    pub typo_count: f32,
    pub history_count: f32,
    pub complete_count: f32,
    pub history_time_diff: f32,
    pub context_bigram_score: f32,
    pub user_word_rollback_score: f32,
    pub lm_score_with_previous_txt: f32,
    pub head_word_nnlm: f32,
    pub hot_val_conf: f32,
    pub with_context: f32,
    pub user_tail_conf: f32,
    pub coverage_score: f32,
    pub ltr_valid: f32,
    pub history_use_count: f32,
    pub history_hot_val: f32,
    pub prob: f32,
    pub context_flag: f32,
    pub full_jianpin: f32,
    pub prefix_completion: f32,
    pub other_completion: f32,
    pub typo: f32,
    pub force_emit: f32,
}

impl Default for FeatureWeights {
    fn default() -> Self {
        // 占位权重：只保证“更可能、覆盖率更高、纠错更少”的词排前面。
        Self {
            kind: 0.0,
            typo_count: -1.5,
            // 历史只看热度（history_hot_val）：次数线性加分会无限涨，
            // 时间差的 ln(1+秒) 会让「一小时前选过」比「从没选过」还靠后
            history_count: 0.0,
            complete_count: 0.2,
            history_time_diff: 0.0,
            // 上下文二元分是 path.score 之外的新信息，权重给足
            context_bigram_score: 2.0,
            user_word_rollback_score: 0.8,
            // 这一项已经计入 path.score，再叠加就是重复计权
            lm_score_with_previous_txt: 0.0,
            head_word_nnlm: 1.0,
            hot_val_conf: 0.3,
            with_context: 0.1,
            user_tail_conf: 0.3,
            coverage_score: 2.0,
            ltr_valid: 0.0,
            history_use_count: 0.0,
            // ln(1+热度)：选过一次约 +2.1，两次 +3.3，五次 +5.4
            history_hot_val: 3.0,
            // prob 是 path.score 的单调变换，再乘权重只会放大基础分
            prob: 0.0,
            context_flag: 0.0,
            full_jianpin: -0.3,
            prefix_completion: -0.2,
            other_completion: -0.2,
            typo: -1.0,
            force_emit: 0.0,
        }
    }
}

impl FeatureWeights {
    pub fn dot(&self, f: &Feature) -> f32 {
        self.kind * f.kind
            + self.typo_count * f.typo_count
            + self.history_count * f.history_count
            + self.complete_count * f.complete_count
            + self.history_time_diff * f.history_time_diff
            + self.context_bigram_score * f.context_bigram_score
            + self.user_word_rollback_score * f.user_word_rollback_score
            + self.lm_score_with_previous_txt * f.lm_score_with_previous_txt
            + self.head_word_nnlm * f.head_word_nnlm
            + self.hot_val_conf * f.hot_val_conf
            + self.with_context * f.with_context
            + self.user_tail_conf * f.user_tail_conf
            + self.coverage_score * f.coverage_score
            + self.ltr_valid * f.ltr_valid
            + self.history_use_count * f.history_use_count
            + self.history_hot_val * f.history_hot_val
            + self.prob * f.prob
            + self.context_flag * f.context_flag
            + self.full_jianpin * f.full_jianpin
            + self.prefix_completion * f.prefix_completion
            + self.other_completion * f.other_completion
            + self.typo * f.typo
            + self.force_emit * f.force_emit
    }
}

/// LTR 模型钩子（LightGBM / 神经重排都可以实现它）。
pub trait LtrModel: Send + Sync {
    fn score(&self, feature: &Feature) -> f32;
}

/// 排序器接口。
pub trait Ranker: Send + Sync {
    fn base_score(&self, feature: &Feature) -> f32;

    /// 返回 None 表示没有 LTR 模型。
    fn ltr(&self, _feature: &Feature) -> Option<f32> {
        None
    }

    fn has_ltr(&self) -> bool {
        false
    }

    fn score(&self, feature: &Feature) -> f32 {
        self.base_score(feature) + self.ltr(feature).unwrap_or(0.0)
    }
}

/// 线性打分器，预留 LTR 钩子。
pub struct LinearRanker {
    pub weights: FeatureWeights,
    ltr_model: Option<Arc<dyn LtrModel>>,
    ltr_weight: f32,
}

impl Default for LinearRanker {
    fn default() -> Self {
        Self {
            weights: FeatureWeights::default(),
            ltr_model: None,
            ltr_weight: 1.0,
        }
    }
}

impl LinearRanker {
    pub fn new(weights: FeatureWeights) -> Self {
        Self {
            weights,
            ltr_model: None,
            ltr_weight: 1.0,
        }
    }

    /// 接入 LTR 模型；分数按 ltr_weight 叠加到线性分上。
    pub fn with_ltr(mut self, model: Arc<dyn LtrModel>, ltr_weight: f32) -> Self {
        self.ltr_model = Some(model);
        self.ltr_weight = ltr_weight;
        self
    }
}

impl Ranker for LinearRanker {
    fn base_score(&self, feature: &Feature) -> f32 {
        self.weights.dot(feature)
    }

    fn ltr(&self, feature: &Feature) -> Option<f32> {
        self.ltr_model
            .as_ref()
            .map(|model| self.ltr_weight * model.score(feature))
    }

    fn has_ltr(&self) -> bool {
        self.ltr_model.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ConstantLtr(f32);

    impl LtrModel for ConstantLtr {
        fn score(&self, _feature: &Feature) -> f32 {
            self.0
        }
    }

    #[test]
    fn feature_names_match_count() {
        assert_eq!(FEATURE_NAMES.len(), FEATURE_COUNT);
    }

    #[test]
    fn linear_dot_product() {
        let f = Feature {
            prob: 0.5,
            coverage_score: 1.0,
            ..Feature::default()
        };
        let weights = FeatureWeights::default();
        let ranker = LinearRanker::new(weights);
        let expected = 0.5 * weights.prob + weights.coverage_score;
        assert!((ranker.base_score(&f) - expected).abs() < 1e-6);
    }

    #[test]
    fn ltr_hook_adds_score() {
        let ranker = LinearRanker::default().with_ltr(Arc::new(ConstantLtr(2.5)), 1.0);
        let f = Feature::default();
        assert_eq!(ranker.ltr(&f), Some(2.5));
        assert!(ranker.has_ltr());
        assert!((ranker.score(&f) - 2.5).abs() < 1e-6);
    }

    #[test]
    fn vector_order() {
        let f = Feature {
            kind: 1.0,
            force_emit: 2.0,
            ..Default::default()
        };
        let v = f.to_vector();
        assert_eq!(v[0], 1.0);
        assert_eq!(v[FEATURE_COUNT - 1], 2.0);
    }
}

impl FeatureWeights {
    /// 按特征名改权重，返回是否命中。用于命令行调参。
    pub fn set_by_name(&mut self, name: &str, value: f32) -> bool {
        match name {
            "kind" => self.kind = value,
            "typo_count" => self.typo_count = value,
            "history_count" => self.history_count = value,
            "complete_count" => self.complete_count = value,
            "history_time_diff" => self.history_time_diff = value,
            "context_bigram_score" => self.context_bigram_score = value,
            "user_word_rollback_score" => self.user_word_rollback_score = value,
            "lm_score_with_previous_txt" => self.lm_score_with_previous_txt = value,
            "head_word_nnlm" => self.head_word_nnlm = value,
            "hot_val_conf" => self.hot_val_conf = value,
            "with_context" => self.with_context = value,
            "user_tail_conf" => self.user_tail_conf = value,
            "coverage_score" => self.coverage_score = value,
            "ltr_valid" => self.ltr_valid = value,
            "history_use_count" => self.history_use_count = value,
            "history_hot_val" => self.history_hot_val = value,
            "prob" => self.prob = value,
            "context_flag" => self.context_flag = value,
            "full_jianpin" => self.full_jianpin = value,
            "prefix_completion" => self.prefix_completion = value,
            "other_completion" => self.other_completion = value,
            "typo" => self.typo = value,
            "force_emit" => self.force_emit = value,
            _ => return false,
        }
        true
    }

    /// 解析 `a=1,b=-0.5` 形式的覆盖串。
    pub fn apply_pairs(&mut self, spec: &str) -> Result<(), String> {
        for pair in spec.split(',').filter(|p| !p.trim().is_empty()) {
            let (name, value) = pair
                .split_once('=')
                .ok_or_else(|| format!("权重项应为 名称=数值: {pair}"))?;
            let value: f32 = value
                .trim()
                .parse()
                .map_err(|_| format!("权重不是数字: {pair}"))?;
            if !self.set_by_name(name.trim(), value) {
                return Err(format!("未知特征名: {name}"));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod weight_tests {
    use super::*;

    #[test]
    fn applies_pairs_by_name() {
        let mut w = FeatureWeights::default();
        w.apply_pairs("prob=1.5, coverage_score=-0.5").unwrap();
        assert_eq!(w.prob, 1.5);
        assert_eq!(w.coverage_score, -0.5);
    }

    #[test]
    fn rejects_unknown_names() {
        let mut w = FeatureWeights::default();
        assert!(w.apply_pairs("nope=1").is_err());
        assert!(w.apply_pairs("prob=abc").is_err());
        assert!(w.apply_pairs("prob").is_err());
    }
}

impl FeatureWeights {
    /// 按 FEATURE_NAMES 顺序导出成数组，便于做数值优化。
    pub fn to_array(&self) -> [f32; FEATURE_COUNT] {
        [
            self.kind,
            self.typo_count,
            self.history_count,
            self.complete_count,
            self.history_time_diff,
            self.context_bigram_score,
            self.user_word_rollback_score,
            self.lm_score_with_previous_txt,
            self.head_word_nnlm,
            self.hot_val_conf,
            self.with_context,
            self.user_tail_conf,
            self.coverage_score,
            self.ltr_valid,
            self.history_use_count,
            self.history_hot_val,
            self.prob,
            self.context_flag,
            self.full_jianpin,
            self.prefix_completion,
            self.other_completion,
            self.typo,
            self.force_emit,
        ]
    }

    pub fn from_array(w: [f32; FEATURE_COUNT]) -> Self {
        Self {
            kind: w[0],
            typo_count: w[1],
            history_count: w[2],
            complete_count: w[3],
            history_time_diff: w[4],
            context_bigram_score: w[5],
            user_word_rollback_score: w[6],
            lm_score_with_previous_txt: w[7],
            head_word_nnlm: w[8],
            hot_val_conf: w[9],
            with_context: w[10],
            user_tail_conf: w[11],
            coverage_score: w[12],
            ltr_valid: w[13],
            history_use_count: w[14],
            history_hot_val: w[15],
            prob: w[16],
            context_flag: w[17],
            full_jianpin: w[18],
            prefix_completion: w[19],
            other_completion: w[20],
            typo: w[21],
            force_emit: w[22],
        }
    }
}
