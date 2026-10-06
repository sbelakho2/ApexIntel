//! Calibration and evidence fusion (CHAIN / competence-gating / noisy-OR).
//!
//! Stated confidence is only trustworthy if it has been measured against
//! resolved outcomes. This module provides:
//!
//! - [`combine_independent_evidence`]: noisy-OR fusion of genuinely
//!   independent evidence chains (never a sum of correlated copies);
//! - [`CalibrationCurve::fit`] / [`CalibrationCurve::adjust`]: isotonic
//!   (PAVA) recalibration fitted on resolved predictions, with Brier score,
//!   expected/ max calibration error and log loss as the hard error metrics;
//! - [`competence_weight`]: outcome-estimated, shrunk domain competence
//!   weights for selective model/domain trust (verbal confidence is not
//!   trusted as a competence signal).

use serde::{Deserialize, Serialize};

/// One resolved prediction: stated confidence + whether the event occurred.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CalibrationPair {
    pub stated: f64,
    pub outcome: bool,
}

/// One reliability-diagram bin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CalibrationBin {
    pub lower: f64,
    pub upper: f64,
    pub count: usize,
    /// Mean stated confidence in the bin (0 when empty).
    pub mean_stated: f64,
    /// Observed frequency (0 when empty).
    pub observed: f64,
}

/// Hard error metrics plus the monotone adjustment map.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CalibrationCurve {
    pub bins: Vec<CalibrationBin>,
    pub sample_size: usize,
    /// Mean squared error of stated confidence (proper score; lower is better).
    pub brier: f64,
    /// Expected calibration error (count-weighted mean |stated − observed|).
    pub ece: f64,
    /// Maximum calibration error over non-empty bins.
    pub mce: f64,
    /// Binary log loss of stated confidence.
    pub log_loss: f64,
    /// Monotone piecewise-linear adjustment points `(stated, adjusted)`,
    /// always including `(0,0)` and `(1,1)` anchors.
    pub adjustment_points: Vec<(f64, f64)>,
}

/// Minimum resolved predictions before a curve may be fitted. Below this the
/// curve would memorize noise; callers must fall back to raw confidence.
pub const MIN_CALIBRATION_SAMPLES: usize = 20;

impl CalibrationCurve {
    /// Fit a curve from resolved pairs. Returns `None` when there are fewer
    /// than [`MIN_CALIBRATION_SAMPLES`] pairs.
    pub fn fit(pairs: &[CalibrationPair], bin_count: usize) -> Option<Self> {
        if pairs.len() < MIN_CALIBRATION_SAMPLES {
            return None;
        }
        let bin_count = bin_count.clamp(2, 20);
        let mut bins: Vec<CalibrationBin> = (0..bin_count)
            .map(|index| {
                let lower = index as f64 / bin_count as f64;
                let upper = (index + 1) as f64 / bin_count as f64;
                CalibrationBin {
                    lower,
                    upper,
                    count: 0,
                    mean_stated: 0.0,
                    observed: 0.0,
                }
            })
            .collect();

        let mut stated_sum = vec![0.0_f64; bin_count];
        let mut observed_sum = vec![0.0_f64; bin_count];
        let mut brier_sum = 0.0;
        let mut log_loss_sum = 0.0;
        for pair in pairs {
            let stated = pair.stated.clamp(0.0, 1.0);
            let index = ((stated * bin_count as f64).floor() as usize).min(bin_count - 1);
            bins[index].count += 1;
            stated_sum[index] += stated;
            observed_sum[index] += if pair.outcome { 1.0 } else { 0.0 };
            let actual = if pair.outcome { 1.0 } else { 0.0 };
            brier_sum += (stated - actual).powi(2);
            let clamped = stated.clamp(1e-6, 1.0 - 1e-6);
            log_loss_sum += if pair.outcome {
                -clamped.ln()
            } else {
                -(1.0 - clamped).ln()
            };
        }

        let mut ece = 0.0;
        let mut mce: f64 = 0.0;
        for (index, bin) in bins.iter_mut().enumerate() {
            if bin.count == 0 {
                continue;
            }
            bin.mean_stated = stated_sum[index] / bin.count as f64;
            bin.observed = observed_sum[index] / bin.count as f64;
            let gap = (bin.mean_stated - bin.observed).abs();
            ece += gap * bin.count as f64 / pairs.len() as f64;
            mce = mce.max(gap);
        }

        // PAVA over non-empty bins (weighted by count), anchored at (0,0)/(1,1).
        let mut blocks: Vec<(f64, f64, f64)> = bins
            .iter()
            .filter(|bin| bin.count > 0)
            .map(|bin| {
                let center = (bin.lower + bin.upper) / 2.0;
                (center, bin.observed, bin.count as f64)
            })
            .collect();
        blocks = pava(blocks);
        let mut adjustment_points: Vec<(f64, f64)> = Vec::with_capacity(blocks.len() + 2);
        adjustment_points.push((0.0, 0.0));
        for (center, value, _) in &blocks {
            adjustment_points.push((*center, value.clamp(0.0, 1.0)));
        }
        adjustment_points.push((1.0, 1.0));
        adjustment_points
            .sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        adjustment_points.dedup_by(|a, b| (a.0 - b.0).abs() < 1e-9);

        Some(Self {
            bins,
            sample_size: pairs.len(),
            brier: brier_sum / pairs.len() as f64,
            ece,
            mce,
            log_loss: log_loss_sum / pairs.len() as f64,
            adjustment_points,
        })
    }

    /// Apply the monotone recalibration map to a stated confidence.
    pub fn adjust(&self, stated: f64) -> f64 {
        let stated = stated.clamp(0.0, 1.0);
        let points = &self.adjustment_points;
        if points.len() < 2 {
            return stated;
        }
        for window in points.windows(2) {
            let (x0, y0) = window[0];
            let (x1, y1) = window[1];
            if stated >= x0 && stated <= x1 {
                if (x1 - x0).abs() < f64::EPSILON {
                    return y1;
                }
                let t = (stated - x0) / (x1 - x0);
                return (y0 + t * (y1 - y0)).clamp(0.0, 1.0);
            }
        }
        points
            .last()
            .map(|(_, y)| y.clamp(0.0, 1.0))
            .unwrap_or(stated)
    }
}

/// Pool-adjacent-violators: enforce non-decreasing weighted means.
fn pava(blocks: Vec<(f64, f64, f64)>) -> Vec<(f64, f64, f64)> {
    let mut stack: Vec<(f64, f64, f64)> = Vec::new();
    for (center, value, weight) in blocks {
        stack.push((center, value, weight));
        while stack.len() >= 2 {
            let last = stack[stack.len() - 1];
            let prev = stack[stack.len() - 2];
            if prev.1 <= last.1 {
                break;
            }
            let combined_weight = prev.2 + last.2;
            let combined_value = (prev.1 * prev.2 + last.1 * last.2) / combined_weight;
            let combined_center = (prev.0 * prev.2 + last.0 * last.2) / combined_weight;
            stack.pop();
            stack.pop();
            stack.push((combined_center, combined_value, combined_weight));
        }
    }
    stack
}

/// Noisy-OR fusion of independent evidence chain probabilities.
///
/// Duplicated or correlated chains must be collapsed by the caller before
/// fusion ([`super::source_families`] does exactly that over evidence rows).
/// With no evidence the function returns the uniform prior `0.5`.
pub fn combine_independent_evidence(chain_probabilities: &[f64]) -> f64 {
    if chain_probabilities.is_empty() {
        return 0.5;
    }
    let mut remaining = 1.0_f64;
    for probability in chain_probabilities {
        remaining *= 1.0 - probability.clamp(0.0, 1.0);
    }
    (1.0 - remaining).clamp(0.0, 1.0)
}

/// Outcome-estimated competence weight for a domain, shrunk toward the global
/// rate with a Beta pseudo-count prior (competence gating).
pub fn competence_weight(
    domain_successes: u64,
    domain_total: u64,
    global_successes: u64,
    global_total: u64,
    prior_strength: f64,
) -> f64 {
    let global_rate = if global_total == 0 {
        0.5
    } else {
        global_successes as f64 / global_total as f64
    };
    let prior = prior_strength.max(0.0);
    let posterior = (domain_successes as f64 + prior * global_rate)
        / (domain_total as f64 + prior).max(f64::EPSILON);
    posterior.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn pairs(stated: f64, positive_rate: f64, n: usize) -> Vec<CalibrationPair> {
        let positives = (positive_rate * n as f64).round() as usize;
        (0..n)
            .map(|index| CalibrationPair {
                stated,
                outcome: index < positives,
            })
            .collect()
    }

    #[test]
    fn overconfident_curve_is_adjusted_downward() {
        // 0.9 stated, only 50% occurred: adjustment must pull 0.9 down.
        let data = pairs(0.9, 0.5, 100);
        let curve = CalibrationCurve::fit(&data, 10).expect("curve");
        assert!(curve.ece > 0.3, "ece {}", curve.ece);
        let adjusted = curve.adjust(0.9);
        assert!(adjusted < 0.75, "adjusted {adjusted}");
        assert!(adjusted > 0.3, "adjusted {adjusted}");
    }

    #[test]
    fn calibrated_curve_is_near_identity() {
        let mut data = Vec::new();
        for _ in 0..70 {
            data.push(CalibrationPair {
                stated: 0.7,
                outcome: true,
            });
        }
        for _ in 0..30 {
            data.push(CalibrationPair {
                stated: 0.7,
                outcome: false,
            });
        }
        let curve = CalibrationCurve::fit(&data, 10).expect("curve");
        assert!(curve.ece < 0.05, "ece {}", curve.ece);
        let adjusted = curve.adjust(0.7);
        assert!((adjusted - 0.7).abs() < 0.05, "adjusted {adjusted}");
    }

    #[test]
    fn brier_and_log_loss_are_proper() {
        let confident_wrong = vec![
            CalibrationPair {
                stated: 0.95,
                outcome: false
            };
            30
        ];
        let curve = CalibrationCurve::fit(&confident_wrong, 10).expect("curve");
        assert!(curve.brier > 0.85, "brier {}", curve.brier);
        assert!(curve.log_loss > 2.5, "log loss {}", curve.log_loss);
    }

    #[test]
    fn insufficient_samples_yield_no_curve() {
        let data = vec![
            CalibrationPair {
                stated: 0.5,
                outcome: true
            };
            MIN_CALIBRATION_SAMPLES - 1
        ];
        assert!(CalibrationCurve::fit(&data, 10).is_none());
    }

    #[test]
    fn adjustment_is_monotone() {
        let data = pairs(0.9, 0.5, 100);
        let curve = CalibrationCurve::fit(&data, 10).expect("curve");
        let mut previous = curve.adjust(0.0);
        for step in 1..=100 {
            let current = curve.adjust(step as f64 / 100.0);
            assert!(current + 1e-9 >= previous, "not monotone at {step}");
            previous = current;
        }
    }

    #[test]
    fn curve_survives_json_round_trip() {
        // The worker persists the curve as JSON in the model registry and
        // deserializes it at recipe-fire time; the round trip must be exact.
        let data = pairs(0.8, 0.6, 50);
        let curve = CalibrationCurve::fit(&data, 10).expect("curve");
        let value = serde_json::to_value(&curve).expect("serialize");
        let restored: CalibrationCurve = serde_json::from_value(value).expect("deserialize");
        assert_eq!(restored, curve);
        assert!((restored.adjust(0.8) - curve.adjust(0.8)).abs() < f64::EPSILON);
    }

    #[test]
    fn noisy_or_combines_independent_chains() {
        assert!((combine_independent_evidence(&[0.5, 0.5]) - 0.75).abs() < 1e-9);
        assert!((combine_independent_evidence(&[]) - 0.5).abs() < 1e-9);
        assert!((combine_independent_evidence(&[1.0]) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn competence_shrinks_toward_global_for_small_domains() {
        let small = competence_weight(1, 2, 700, 1000, 10.0);
        let large = competence_weight(90, 100, 700, 1000, 10.0);
        assert!(small < 0.8, "small {small}");
        assert!(large > 0.85, "large {large}");
        // A tiny sample cannot drag the estimate to certainty.
        assert!(small > 0.5);
    }
}
