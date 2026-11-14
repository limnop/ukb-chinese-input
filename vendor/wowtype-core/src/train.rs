//! 线性排序模型训练（逻辑回归 / softmax）。
//!
//! 给定每个输入下的候选特征和用户实际选中的那个，用 softmax 交叉熵拟合权重：
//! 让被选中的候选得分高于同组其它候选。这就是 WeType 里 `CalculateScoreByModel(LRModel)`
//! 那一层能自建的最小版本——我们没有它的训练数据，但可以自己标。

use crate::rank::{Feature, FeatureWeights, FEATURE_COUNT};

/// 一条训练样本：同一输入下的候选特征 + 用户选中的下标。
pub struct Sample {
    pub features: Vec<Feature>,
    pub chosen: usize,
}

/// 训练参数。
#[derive(Clone, Copy, Debug)]
pub struct TrainConfig {
    pub epochs: usize,
    pub learning_rate: f32,
    /// L2 正则，防止权重被少量样本拉爆。
    pub l2: f32,
}

impl Default for TrainConfig {
    fn default() -> Self {
        Self {
            epochs: 300,
            learning_rate: 0.05,
            l2: 0.01,
        }
    }
}

fn softmax(scores: &[f32]) -> Vec<f32> {
    let max = scores.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let exps: Vec<f32> = scores.iter().map(|s| (s - max).exp()).collect();
    let sum: f32 = exps.iter().sum();
    if sum <= 0.0 {
        return vec![1.0 / scores.len().max(1) as f32; scores.len()];
    }
    exps.into_iter().map(|e| e / sum).collect()
}

/// 用样本拟合权重；`initial` 为起点（通常是当前默认权重）。
pub fn train(samples: &[Sample], config: TrainConfig, initial: FeatureWeights) -> FeatureWeights {
    let mut w = initial.to_array();
    if samples.is_empty() {
        return initial;
    }

    let mut grad = [0f32; FEATURE_COUNT];
    for _ in 0..config.epochs {
        for g in grad.iter_mut() {
            *g = 0.0;
        }
        let mut used = 0usize;
        for sample in samples {
            if sample.features.len() < 2 || sample.chosen >= sample.features.len() {
                continue;
            }
            used += 1;
            let vectors: Vec<[f32; FEATURE_COUNT]> =
                sample.features.iter().map(|f| f.to_vector()).collect();
            let scores: Vec<f32> = vectors
                .iter()
                .map(|v| {
                    let mut s = 0.0;
                    for i in 0..FEATURE_COUNT {
                        s += w[i] * v[i];
                    }
                    s
                })
                .collect();
            let probs = softmax(&scores);
            for (idx, v) in vectors.iter().enumerate() {
                let target = if idx == sample.chosen { 1.0 } else { 0.0 };
                let diff = probs[idx] - target;
                for i in 0..FEATURE_COUNT {
                    grad[i] += diff * v[i];
                }
            }
        }
        if used == 0 {
            break;
        }
        let scale = config.learning_rate / used as f32;
        for i in 0..FEATURE_COUNT {
            w[i] -= scale * (grad[i] + config.l2 * w[i]);
        }
    }
    FeatureWeights::from_array(w)
}

/// 评估：返回 top-1 准确率。
pub fn accuracy(samples: &[Sample], weights: FeatureWeights) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let mut hit = 0usize;
    let mut total = 0usize;
    for sample in samples {
        if sample.features.len() < 2 || sample.chosen >= sample.features.len() {
            continue;
        }
        total += 1;
        let mut best = 0usize;
        let mut best_score = f32::NEG_INFINITY;
        for (i, feature) in sample.features.iter().enumerate() {
            let score = weights.dot(feature);
            if score > best_score {
                best_score = score;
                best = i;
            }
        }
        if best == sample.chosen {
            hit += 1;
        }
    }
    if total == 0 {
        0.0
    } else {
        hit as f64 / total as f64
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::rank::Feature;

    // 用默认权重为 0 的特征，才能看出训练效果
    fn pair(flag_a: f32, flag_b: f32) -> (Feature, Feature) {
        let a = Feature {
            prefix_completion: flag_a,
            ..Feature::default()
        };
        let b = Feature {
            prefix_completion: flag_b,
            ..Feature::default()
        };
        (a, b)
    }

    #[test]
    fn learns_to_rank_by_feature() {
        // 每个样本里，覆盖率为 1 的候选才是被选中的那个
        let mut samples = Vec::new();
        for _ in 0..20 {
            let (a, b) = pair(0.0, 1.0);
            samples.push(Sample {
                features: vec![a, b],
                chosen: 1,
            });
        }
        let initial = FeatureWeights::default();
        let before = accuracy(&samples, initial);
        let trained = train(&samples, TrainConfig::default(), initial);
        let after = accuracy(&samples, trained);
        assert!(after > before, "训练后准确率应提升: {before} -> {after}");
        assert!(
            trained.prefix_completion > 0.0,
            "应当学到 prefix_completion 的正权重"
        );
    }

    #[test]
    fn empty_samples_keep_initial_weights() {
        let initial = FeatureWeights::default();
        let trained = train(&[], TrainConfig::default(), initial);
        assert_eq!(trained.prob, initial.prob);
        assert_eq!(accuracy(&[], initial), 0.0);
    }

    #[test]
    fn skips_samples_with_single_candidate() {
        let samples = vec![Sample {
            features: vec![Feature::default()],
            chosen: 0,
        }];
        let initial = FeatureWeights::default();
        let trained = train(&samples, TrainConfig::default(), initial);
        assert_eq!(trained.coverage_score, initial.coverage_score);
    }
}
