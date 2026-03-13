use crate::correlation::spearman;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalibrationSample {
    pub raw_score: f64,
    pub actual_outcome: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlattScalingModel {
    pub slope: f64,
    pub intercept: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IsotonicCalibrationModel {
    pub thresholds: Vec<f64>,
    pub predictions: Vec<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum AlertCalibrationModel {
    Legacy,
    Platt(PlattScalingModel),
    Isotonic(IsotonicCalibrationModel),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReliabilityBin {
    pub lower_bound: f64,
    pub upper_bound: f64,
    pub predicted_mean: f64,
    pub observed_frequency: f64,
    pub count: usize,
}

fn stable_logistic(value: f64) -> f64 {
    if value >= 0.0 {
        let z = (-value).exp();
        1.0 / (1.0 + z)
    } else {
        let z = value.exp();
        z / (1.0 + z)
    }
}

fn has_both_classes(samples: &[CalibrationSample]) -> bool {
    let positives = samples.iter().filter(|sample| sample.actual_outcome).count();
    positives > 0 && positives < samples.len()
}

pub fn legacy_score_to_probability(score: f64) -> f64 {
    if score < 1.0 {
        0.05
    } else if score < 3.0 {
        0.15
    } else if score < 5.0 {
        0.50
    } else {
        0.80
    }
}

pub fn probability_to_alert_level(probability: f64) -> &'static str {
    if probability >= 0.70 {
        "high"
    } else if probability >= 0.30 {
        "medium"
    } else if probability >= 0.10 {
        "low"
    } else {
        "none"
    }
}

pub fn fit_platt_scaling(samples: &[CalibrationSample]) -> Option<PlattScalingModel> {
    if samples.len() < 2 || !has_both_classes(samples) {
        return None;
    }

    let mut slope = 1.0_f64;
    let mut intercept = 0.0_f64;
    let learning_rate = 0.01 / (samples.len() as f64).sqrt().max(1.0);

    for _ in 0..4000 {
        let mut grad_slope = 0.0;
        let mut grad_intercept = 0.0;
        for sample in samples {
            let probability = stable_logistic(slope * sample.raw_score + intercept);
            let outcome = if sample.actual_outcome { 1.0 } else { 0.0 };
            let error = probability - outcome;
            grad_slope += error * sample.raw_score;
            grad_intercept += error;
        }
        slope -= learning_rate * grad_slope;
        intercept -= learning_rate * grad_intercept;
        slope = slope.max(1e-6);
    }

    Some(PlattScalingModel { slope, intercept })
}

pub fn fit_isotonic_regression(samples: &[CalibrationSample]) -> Option<IsotonicCalibrationModel> {
    if samples.len() < 2 || !has_both_classes(samples) {
        return None;
    }

    #[derive(Debug, Clone)]
    struct Block {
        end: f64,
        weighted_sum: f64,
        weight: f64,
    }

    impl Block {
        fn mean(&self) -> f64 {
            if self.weight <= f64::EPSILON {
                0.0
            } else {
                self.weighted_sum / self.weight
            }
        }
    }

    let mut ordered = samples.to_vec();
    ordered.sort_by(|left, right| left.raw_score.total_cmp(&right.raw_score));

    let mut blocks: Vec<Block> = ordered
        .into_iter()
        .map(|sample| Block {
            end: sample.raw_score,
            weighted_sum: if sample.actual_outcome { 1.0 } else { 0.0 },
            weight: 1.0,
        })
        .collect();

    let mut index = 0usize;
    while index + 1 < blocks.len() {
        if blocks[index].mean() > blocks[index + 1].mean() {
            let right = blocks.remove(index + 1);
            blocks[index].end = right.end;
            blocks[index].weighted_sum += right.weighted_sum;
            blocks[index].weight += right.weight;
            index = index.saturating_sub(1);
        } else {
            index += 1;
        }
    }

    Some(IsotonicCalibrationModel {
        thresholds: blocks.iter().map(|block| block.end).collect(),
        predictions: blocks.iter().map(Block::mean).collect(),
    })
}

pub fn fit_best_alert_calibration_model(samples: &[CalibrationSample]) -> AlertCalibrationModel {
    if samples.len() >= 200 {
        if let Some(model) = fit_isotonic_regression(samples) {
            return AlertCalibrationModel::Isotonic(model);
        }
    }
    if samples.len() >= 100 {
        if let Some(model) = fit_platt_scaling(samples) {
            return AlertCalibrationModel::Platt(model);
        }
    }
    AlertCalibrationModel::Legacy
}

impl AlertCalibrationModel {
    pub fn method_name(&self) -> &'static str {
        match self {
            Self::Legacy => "legacy",
            Self::Platt(_) => "platt",
            Self::Isotonic(_) => "isotonic",
        }
    }

    pub fn predict(&self, raw_score: f64) -> f64 {
        match self {
            Self::Legacy => legacy_score_to_probability(raw_score),
            Self::Platt(model) => stable_logistic(model.slope * raw_score + model.intercept),
            Self::Isotonic(model) => {
                for (threshold, prediction) in model.thresholds.iter().zip(model.predictions.iter()) {
                    if raw_score <= *threshold {
                        return *prediction;
                    }
                }
                model.predictions.last().copied().unwrap_or_else(|| legacy_score_to_probability(raw_score))
            }
        }
    }
}

pub fn calibration_curve(
    samples: &[CalibrationSample],
    model: &AlertCalibrationModel,
    bin_count: usize,
) -> Vec<ReliabilityBin> {
    if samples.is_empty() || bin_count == 0 {
        return Vec::new();
    }

    let width = 1.0 / bin_count as f64;
    let mut totals = vec![0usize; bin_count];
    let mut predicted_sum = vec![0.0f64; bin_count];
    let mut observed_sum = vec![0.0f64; bin_count];

    for sample in samples {
        let predicted = model.predict(sample.raw_score).clamp(0.0, 1.0);
        let mut index = (predicted / width).floor() as usize;
        if index >= bin_count {
            index = bin_count - 1;
        }
        totals[index] += 1;
        predicted_sum[index] += predicted;
        observed_sum[index] += if sample.actual_outcome { 1.0 } else { 0.0 };
    }

    (0..bin_count)
        .filter_map(|index| {
            let count = totals[index];
            if count == 0 {
                return None;
            }
            Some(ReliabilityBin {
                lower_bound: index as f64 * width,
                upper_bound: ((index + 1) as f64 * width).min(1.0),
                predicted_mean: predicted_sum[index] / count as f64,
                observed_frequency: observed_sum[index] / count as f64,
                count,
            })
        })
        .collect()
}

pub fn predicted_probabilities(
    samples: &[CalibrationSample],
    model: &AlertCalibrationModel,
) -> Vec<f64> {
    samples
        .iter()
        .map(|sample| model.predict(sample.raw_score).clamp(0.0, 1.0))
        .collect()
}

pub fn alert_score_rank_correlation(samples: &[CalibrationSample], model: &AlertCalibrationModel) -> f64 {
    if samples.len() < 2 || !has_both_classes(samples) {
        return 0.0;
    }
    let predictions = predicted_probabilities(samples, model);
    let actuals = samples
        .iter()
        .map(|sample| if sample.actual_outcome { 1.0 } else { 0.0 })
        .collect::<Vec<_>>();
    spearman(&predictions, &actuals)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic_samples(count: usize) -> Vec<CalibrationSample> {
        (0..count)
            .map(|index| {
                let raw_score = index as f64 / 14.0;
                let actual_outcome = raw_score >= 3.5 || (raw_score >= 2.0 && index % 4 == 0);
                CalibrationSample {
                    raw_score,
                    actual_outcome,
                }
            })
            .collect()
    }

    #[test]
    fn platt_scaling_monotonicity() {
        let model = fit_platt_scaling(&synthetic_samples(120)).expect("platt model");
        let low = stable_logistic(model.slope * 1.0 + model.intercept);
        let high = stable_logistic(model.slope * 4.0 + model.intercept);
        assert!(low < high);
    }

    #[test]
    fn alert_score_rank_correlation_exceeds_threshold() {
        let samples = synthetic_samples(100);
        let model = fit_best_alert_calibration_model(&samples);
        let correlation = alert_score_rank_correlation(&samples, &model);
        assert!(correlation > 0.5, "expected rank correlation > 0.5, got {correlation}");
    }

    #[test]
    fn isotonic_model_is_selected_for_large_sample_sets() {
        let model = fit_best_alert_calibration_model(&synthetic_samples(240));
        assert!(matches!(model, AlertCalibrationModel::Isotonic(_)));
    }

    #[test]
    fn calibration_curve_bins_are_bounded() {
        let samples = synthetic_samples(120);
        let model = fit_best_alert_calibration_model(&samples);
        let bins = calibration_curve(&samples, &model, 10);
        assert!(!bins.is_empty());
        for bin in bins {
            assert!(bin.predicted_mean >= 0.0 && bin.predicted_mean <= 1.0);
            assert!(bin.observed_frequency >= 0.0 && bin.observed_frequency <= 1.0);
        }
    }
}
