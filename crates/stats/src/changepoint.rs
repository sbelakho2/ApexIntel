//! PELT (Pruned Exact Linear Time) change-point detection.
//!
//! # Time complexity
//!
//! Standard PELT is O(n) in the best case (when pruning aggressively) but
//! degrades to O(n²) in the worst case when no candidates are pruned.
//! The optimization below bounds the worst case:
//!
//! - **`max_candidates`** — caps the candidate set size to `K`, bounding the
//!   inner loop to O(K·n). Default: `64`.  This is the primary mechanism:
//!   instead of scanning all O(n) candidates, we keep only the most recent
//!   `K` candidates, which prunes the search space to O(K·n).

/// Configuration for the PELT change-point detector (B289).
///
/// # Default values
/// | Field                  | Default | Rationale                                                           |
/// |-----------------------|---------|--------------------------------------------------------------------|
/// | `penalty`             | 3.0     | Fixed penalty fallback when adaptive mode is disabled              |
/// | `adaptive_penalty`    | true    | Use modified BIC penalty `β = c · log(n)` with variance-adaptive `c` |
/// | `min_segment`         | 2       | Minimum observations per segment; must be ≥ 1                      |
/// | `max_candidates`      | 200     | Bounds PELT worst-case inner loop to O(200·n) instead of O(n²)    |
///
/// Increase `penalty` to suppress noise-driven change-points on smooth series;
/// decrease it for volatile series where subtle shifts matter.
#[derive(Debug, Clone)]
pub struct PeltConfig {
    /// Fixed log-likelihood change-point penalty fallback. Default: `3.0`.
    pub penalty: f64,
    /// Whether to use the variance-adaptive modified BIC penalty.
    pub adaptive_penalty: bool,
    /// Minimum data points in a valid segment.  Default: `2`.
    pub min_segment: usize,
    /// Maximum number of candidate changepoints kept in the pruning set.
    /// Bounds the inner loop to O(max_candidates·n) instead of O(n²).
    /// Default: `200`.
    pub max_candidates: usize,
}

impl Default for PeltConfig {
    fn default() -> Self {
        Self {
            penalty: 3.0,
            adaptive_penalty: true,
            min_segment: 2,
            max_candidates: 200,
        }
    }
}

#[derive(Debug, Clone)]
pub struct BocpdConfig {
    /// Constant hazard rate for a changepoint at any step.
    pub hazard: f64,
    /// Prior mean for the Gaussian mean model.
    pub prior_mean: f64,
    /// Effective sample size of the prior.
    pub prior_strength: f64,
    /// Known observation variance used by the predictive density.
    pub observation_variance: f64,
    /// Posterior probability threshold for emitting a changepoint.
    pub changepoint_threshold: f64,
    /// Minimum number of points between consecutive emitted changepoints.
    pub min_distance: usize,
}

impl Default for BocpdConfig {
    fn default() -> Self {
        Self {
            hazard: 1.0 / 24.0,
            prior_mean: 0.0,
            prior_strength: 1.0,
            observation_variance: 1.0,
            changepoint_threshold: 0.35,
            min_distance: 2,
        }
    }
}

impl PeltConfig {
    /// Validate that all numeric fields are within their valid operating ranges.
    ///
    /// Returns an empty `Vec` when the config is valid.  Each entry in a
    /// non-empty return value is a human-readable error string.
    ///
    /// # Valid ranges
    /// | Field         | Constraint | Rationale                                    |
    /// |--------------|------------|---------------------------------------------|
    /// | `penalty`    | `> 0.0`    | Negative penalties invert the objective       |
    /// | `min_segment`| `>= 2`     | Gaussian cost requires ≥2 points per segment  |
    /// | `max_candidates` | `>= 2` | Need at least 2 candidates for pruning       |
    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();

        if !self.penalty.is_finite() || self.penalty <= 0.0 {
            errors.push(format!(
                "PeltConfig.penalty = {} must be a positive finite number",
                self.penalty
            ));
        }
        if self.min_segment < 2 {
            errors.push(format!(
                "PeltConfig.min_segment = {} must be >= 2 (gaussian_cost requires ≥2 points)",
                self.min_segment
            ));
        }
        if self.max_candidates < 2 {
            errors.push(format!(
                "PeltConfig.max_candidates = {} must be >= 2",
                self.max_candidates
            ));
        }

        errors
    }
}

/// Detect change-points in a time series using the PELT algorithm.
///
/// Returns indices where the distribution of data changes.
///
/// # Time complexity
///
/// Best case: O(n) with aggressive PELT pruning.
/// Worst case: O(max_candidates · n) bounded by the candidate cap,
/// instead of the unbounded O(n²) of standard PELT.
pub fn detect_changepoints(data: &[f64], config: &PeltConfig) -> Vec<usize> {
    // Reject invalid configurations instead of panicking on arithmetic
    // under/overflow (e.g. `max_candidates = 0`, `min_segment = usize::MAX`).
    if !config.validate().is_empty() {
        return vec![];
    }

    let n = data.len();
    if n < config.min_segment.saturating_mul(2) {
        return vec![];
    }

    let penalty = effective_penalty(data, config);

    let mut cost = vec![0.0_f64; n + 1];
    // Standard PELT initialization: F(0) = -β so that the no-changepoint
    // baseline has cost 0 and each split adds exactly β to the objective.
    cost[0] = -penalty;
    let mut cp_trace: Vec<Vec<usize>> = vec![vec![]; n + 1];
    // PELT candidate set — properly pruned each iteration
    let mut candidates: Vec<usize> = vec![0];

    for t in config.min_segment..=n {
        let mut best_cost = f64::MAX;
        let mut best_s = 0usize;

        // OPTIMIZATION: Bound the inner loop to max_candidates candidates.
        // This prevents the O(n²) worst case when PELT pruning is ineffective.
        // We take the most recent `max_candidates` candidates because older ones
        // are likely to have been fully explored already.
        let candidate_limit = candidates.len().min(config.max_candidates);
        let candidates_slice = &candidates[candidates.len().saturating_sub(candidate_limit)..];

        for &s in candidates_slice {
            if t < s + config.min_segment {
                continue;
            }
            let seg_cost = gaussian_cost(&data[s..t]);
            let total = cost[s] + seg_cost + penalty;
            if total < best_cost {
                best_cost = total;
                best_s = s;
            }
        }

        cost[t] = best_cost;
        let mut new_cps = cp_trace[best_s].clone();
        if best_s > 0 {
            new_cps.push(best_s);
        }
        cp_trace[t] = new_cps;

        // PELT pruning rule (Killick et al. 2012): keep s if F(s) <= F(t),
        // meaning it may still yield a better cost for some future t'.
        // Use <= to avoid premature pruning of equal-cost candidates (e.g.
        // a changepoint exactly at the segment boundary).
        candidates.retain(|&s| cost[s] <= best_cost);
        // Always keep at least one candidate
        if candidates.is_empty() {
            candidates.push(t);
        }
        // OPTIMIZATION: Bound candidate set size to max_candidates.
        // Keep the oldest (most conservative) candidates because they see
        // longer segments, which naturally suppresses false positives in noise.
        // Leave room for t, which is appended after truncation so the newest
        // candidate is always available for future iterations.
        if candidates.len() > config.max_candidates.saturating_sub(1) {
            candidates.truncate(config.max_candidates.saturating_sub(1));
        }
        candidates.push(t);
    }

    cp_trace[n].clone()
}

pub fn effective_penalty(data: &[f64], config: &PeltConfig) -> f64 {
    if config.adaptive_penalty {
        modified_bic_penalty(data)
    } else {
        config.penalty
    }
}

pub fn modified_bic_penalty(data: &[f64]) -> f64 {
    if data.len() < 2 {
        return 1.0;
    }

    let variance = sample_variance(data);
    let normalized_variance = (variance / (variance + 1.0)).clamp(0.0, 1.0);
    let c = (1.0 + 2.0 * normalized_variance).clamp(1.0, 3.0);
    c * (data.len() as f64).ln().max(1.0)
}

pub fn detect_changepoints_bocpd(data: &[f64], config: &BocpdConfig) -> Vec<usize> {
    if data.len() < 2 || !config.hazard.is_finite() || config.hazard <= 0.0 || config.hazard >= 1.0
    {
        return vec![];
    }

    let observation_variance = config.observation_variance.max(1e-6);
    let mut run_probs = vec![1.0_f64];
    let mut means = vec![config.prior_mean];
    let mut strengths = vec![config.prior_strength.max(1e-6)];
    let mut changepoints = Vec::new();

    for (index, &value) in data.iter().enumerate() {
        let predictive = means
            .iter()
            .zip(strengths.iter())
            .map(|(&mean, &strength)| {
                let variance = observation_variance * (1.0 + 1.0 / strength.max(1e-6));
                gaussian_pdf(value, mean, variance)
            })
            .collect::<Vec<_>>();

        let mut next_run_probs = vec![0.0_f64; run_probs.len() + 1];
        let mut next_means = vec![config.prior_mean; means.len() + 1];
        let mut next_strengths = vec![config.prior_strength.max(1e-6); strengths.len() + 1];

        let cp_prob = run_probs
            .iter()
            .zip(predictive.iter())
            .map(|(&prob, &pred)| prob * pred * config.hazard)
            .sum::<f64>();
        next_run_probs[0] = cp_prob;

        for run_length in 0..run_probs.len() {
            let growth_prob =
                run_probs[run_length] * predictive[run_length] * (1.0 - config.hazard);
            next_run_probs[run_length + 1] = growth_prob;

            let posterior_strength = strengths[run_length] + 1.0;
            let posterior_mean =
                ((strengths[run_length] * means[run_length]) + value) / posterior_strength;
            next_means[run_length + 1] = posterior_mean;
            next_strengths[run_length + 1] = posterior_strength;
        }

        let normalizer = next_run_probs.iter().sum::<f64>();
        if normalizer <= f64::EPSILON || !normalizer.is_finite() {
            continue;
        }
        for probability in &mut next_run_probs {
            *probability /= normalizer;
        }

        let short_run_mass = next_run_probs
            .iter()
            .take(config.min_distance.max(1) + 1)
            .sum::<f64>();

        if index >= config.min_distance
            && short_run_mass >= config.changepoint_threshold
            && changepoints
                .last()
                .is_none_or(|last| index.saturating_sub(*last) >= config.min_distance)
        {
            changepoints.push(index);
        }

        run_probs = next_run_probs;
        means = next_means;
        strengths = next_strengths;
    }

    changepoints
}

/// Gaussian cost function for a segment: sum of squared deviations (always ≥ 0).
#[inline]
fn gaussian_cost(data: &[f64]) -> f64 {
    if data.is_empty() {
        return 0.0;
    }
    let n = data.len() as f64;
    let mean = data.iter().sum::<f64>() / n;
    data.iter().map(|x| (x - mean).powi(2)).sum::<f64>()
}

fn sample_variance(data: &[f64]) -> f64 {
    if data.len() < 2 {
        return 0.0;
    }
    let mean = data.iter().sum::<f64>() / data.len() as f64;
    data.iter().map(|value| (value - mean).powi(2)).sum::<f64>() / (data.len() - 1) as f64
}

fn gaussian_pdf(x: f64, mean: f64, variance: f64) -> f64 {
    let variance = variance.max(1e-9);
    let norm = (2.0 * std::f64::consts::PI * variance).sqrt();
    ((-0.5 * (x - mean).powi(2) / variance).exp() / norm).max(1e-12)
}

/// Simplified CUSUM detector for online change-point detection.
pub fn cusum(data: &[f64], threshold: f64, drift: f64) -> Vec<usize> {
    let mut s_pos = 0.0_f64;
    let mut s_neg = 0.0_f64;
    let mut alarms = Vec::new();

    let mean = if data.len() >= 10 {
        data[..10].iter().sum::<f64>() / 10.0
    } else if !data.is_empty() {
        data.iter().sum::<f64>() / data.len() as f64
    } else {
        return vec![];
    };

    for (i, &x) in data.iter().enumerate() {
        s_pos = (s_pos + x - mean - drift).max(0.0);
        s_neg = (s_neg - x + mean - drift).max(0.0);

        if s_pos > threshold || s_neg > threshold {
            alarms.push(i);
            s_pos = 0.0;
            s_neg = 0.0;
        }
    }

    alarms
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_changepoints_constant() {
        let data: Vec<f64> = vec![1.0; 20];
        let config = PeltConfig {
            penalty: 5.0,
            adaptive_penalty: false,
            min_segment: 3,
            max_candidates: 200,
        };
        let cps = detect_changepoints(&data, &config);
        assert!(cps.is_empty(), "Constant data should have no changepoints");
    }

    #[test]
    fn test_detect_changepoints_clear_shift() {
        let mut data: Vec<f64> = vec![1.0; 30];
        data.extend(vec![10.0; 30]);

        let config = PeltConfig {
            penalty: 10.0,
            adaptive_penalty: false,
            min_segment: 3,
            max_candidates: 200,
        };
        let cps = detect_changepoints(&data, &config);
        assert!(!cps.is_empty(), "Should detect shift");
        // The changepoint should be near index 30
        let closest = cps
            .iter()
            .min_by_key(|&&cp| (cp as i64 - 30).unsigned_abs())
            .unwrap_or_else(|| panic!("shift test should produce at least one changepoint"));
        assert!(
            (*closest as i64 - 30).unsigned_abs() <= 5,
            "Changepoint at {} not near 30",
            closest
        );
    }

    #[test]
    fn test_detect_changepoints_too_short() {
        let data = vec![1.0, 2.0];
        let config = PeltConfig {
            penalty: 1.0,
            adaptive_penalty: false,
            min_segment: 3,
            max_candidates: 200,
        };
        let cps = detect_changepoints(&data, &config);
        assert!(cps.is_empty());
    }

    #[test]
    fn adaptive_penalty_tracks_signal_variance() {
        let low_variance = vec![1.0, 1.1, 0.9, 1.05, 0.95, 1.02, 0.98, 1.0];
        let high_variance = vec![1.0, 4.0, -2.0, 5.0, -3.0, 6.0, -4.0, 7.0];

        let low_penalty = modified_bic_penalty(&low_variance);
        let high_penalty = modified_bic_penalty(&high_variance);

        assert!(high_penalty > low_penalty);
        assert!(low_penalty >= (low_variance.len() as f64).ln());
    }

    #[test]
    fn adaptive_penalty_detects_single_known_changepoint() {
        let mut data = Vec::new();
        for index in 0..120 {
            let seasonal = (((index * 17) % 9) as f64 - 4.0) * 0.05;
            let baseline = if index < 60 { 2.0 } else { 5.5 };
            data.push(baseline + seasonal);
        }

        let config = PeltConfig {
            penalty: 3.0,
            adaptive_penalty: true,
            min_segment: 4,
            max_candidates: 200,
        };
        let cps = detect_changepoints(&data, &config);
        let closest = cps
            .iter()
            .min_by_key(|&&cp| (cp as i64 - 60).unsigned_abs())
            .copied();

        assert!(
            closest.is_some(),
            "adaptive penalty should detect the synthetic shift"
        );
        assert!(matches!(closest, Some(value) if (value as i64 - 60).unsigned_abs() <= 6));
    }

    #[test]
    fn adaptive_penalty_rejects_pure_noise() {
        let data = (0..120)
            .map(|index| ((((index * 37) % 101) as f64) / 100.0 - 0.5) * 0.6)
            .collect::<Vec<_>>();
        let config = PeltConfig {
            penalty: 3.0,
            adaptive_penalty: true,
            min_segment: 4,
            max_candidates: 200,
        };
        let cps = detect_changepoints(&data, &config);
        assert!(
            cps.is_empty(),
            "variance-adaptive MBIC should suppress pure noise changepoints"
        );
    }

    #[test]
    fn test_gaussian_cost_constant() {
        let data = vec![5.0; 10];
        let cost = gaussian_cost(&data);
        assert!((cost - 0.0).abs() < 1e-10);
    }

    #[test]
    fn test_gaussian_cost_varying() {
        let data = vec![1.0, 5.0, 1.0, 5.0];
        let cost = gaussian_cost(&data);
        assert!(cost > 0.0);
    }

    #[test]
    fn test_cusum_no_change() {
        let data: Vec<f64> = (0..30).map(|i| 5.0 + (i as f64 * 0.01)).collect();
        let alarms = cusum(&data, 10.0, 0.5);
        assert!(alarms.is_empty());
    }

    #[test]
    fn test_cusum_detects_shift() {
        let mut data: Vec<f64> = vec![5.0; 20];
        data.extend(vec![15.0; 20]);
        let alarms = cusum(&data, 5.0, 0.5);
        assert!(!alarms.is_empty(), "CUSUM should detect the shift");
    }

    #[test]
    fn test_cusum_empty() {
        let alarms = cusum(&[], 5.0, 0.5);
        assert!(alarms.is_empty());
    }

    // B289: PeltConfig default stability
    #[test]
    fn test_pelt_config_default_is_stable_and_valid() {
        let a = PeltConfig::default();
        let b = PeltConfig::default();
        // Identical across two calls (deterministic)
        assert_eq!(a.min_segment, b.min_segment);
        assert!((a.penalty - b.penalty).abs() < f64::EPSILON);
        assert_eq!(a.adaptive_penalty, b.adaptive_penalty);
        // Values are in valid ranges
        assert!(
            a.penalty > 0.0,
            "penalty must be positive; got {}",
            a.penalty
        );
        assert!(
            a.min_segment >= 1,
            "min_segment must be >= 1; got {}",
            a.min_segment
        );
    }

    #[test]
    fn test_pelt_config_default_values_match_documented_defaults() {
        let cfg = PeltConfig::default();
        assert!(
            (cfg.penalty - 3.0).abs() < f64::EPSILON,
            "default penalty should be 3.0"
        );
        assert!(
            cfg.adaptive_penalty,
            "adaptive penalty should be enabled by default"
        );
        assert_eq!(cfg.min_segment, 2, "default min_segment should be 2");
    }

    // B291: PeltConfig::validate
    #[test]
    fn test_pelt_config_default_passes_validation() {
        assert!(
            PeltConfig::default().validate().is_empty(),
            "default PeltConfig must be valid out of the box"
        );
    }

    #[test]
    fn test_pelt_config_zero_penalty_is_invalid() {
        let cfg = PeltConfig {
            penalty: 0.0,
            ..PeltConfig::default()
        };
        let errs = cfg.validate();
        assert!(errs.iter().any(|e| e.contains("penalty")));
    }

    #[test]
    fn test_pelt_config_negative_penalty_is_invalid() {
        let cfg = PeltConfig {
            penalty: -1.0,
            ..PeltConfig::default()
        };
        let errs = cfg.validate();
        assert!(errs.iter().any(|e| e.contains("penalty")));
    }

    #[test]
    fn test_pelt_config_nan_penalty_is_invalid() {
        let cfg = PeltConfig {
            penalty: f64::NAN,
            ..PeltConfig::default()
        };
        let errs = cfg.validate();
        assert!(errs.iter().any(|e| e.contains("penalty")));
    }

    #[test]
    fn test_pelt_config_min_segment_one_is_invalid() {
        let cfg = PeltConfig {
            min_segment: 1,
            ..PeltConfig::default()
        };
        let errs = cfg.validate();
        assert!(errs.iter().any(|e| e.contains("min_segment")));
    }

    #[test]
    fn test_pelt_config_all_invalid_fields_all_reported() {
        let cfg = PeltConfig {
            penalty: f64::NEG_INFINITY,
            adaptive_penalty: true,
            min_segment: 0,
            max_candidates: 0,
        };
        let errs = cfg.validate();
        assert!(errs.iter().any(|e| e.contains("penalty")));
        assert!(errs.iter().any(|e| e.contains("min_segment")));
        assert!(errs.iter().any(|e| e.contains("max_candidates")));
    }

    #[test]
    fn bocpd_detects_streaming_shift() {
        let mut data = vec![0.5; 40];
        data.extend(vec![4.0; 40]);
        let config = BocpdConfig {
            hazard: 1.0 / 30.0,
            prior_mean: 0.0,
            prior_strength: 1.0,
            observation_variance: 0.5,
            changepoint_threshold: 0.25,
            min_distance: 3,
        };

        let cps = detect_changepoints_bocpd(&data, &config);
        let closest = cps
            .iter()
            .min_by_key(|&&cp| (cp as i64 - 40).unsigned_abs())
            .copied();
        assert!(closest.is_some(), "BOCPD should detect the shift");
        assert!(matches!(closest, Some(value) if (value as i64 - 40).unsigned_abs() <= 6));
    }

    #[test]
    fn bocpd_rejects_pure_noise() {
        let data = (0..80)
            .map(|index| ((((index * 19) % 97) as f64) / 97.0 - 0.5) * 0.5)
            .collect::<Vec<_>>();
        let config = BocpdConfig {
            observation_variance: 0.5,
            changepoint_threshold: 0.4,
            min_distance: 3,
            ..BocpdConfig::default()
        };

        let cps = detect_changepoints_bocpd(&data, &config);
        assert!(
            cps.is_empty(),
            "BOCPD should stay quiet on stationary noise"
        );
    }

    #[test]
    fn changepoint_adaptive_penalty() {
        let mut shifted = vec![1.0; 100];
        shifted.extend(vec![6.0; 100]);
        let shifted_cfg = PeltConfig {
            penalty: 3.0,
            adaptive_penalty: true,
            min_segment: 4,
            max_candidates: 200,
        };
        let shifted_cps = detect_changepoints(&shifted, &shifted_cfg);
        assert!(shifted_cps
            .iter()
            .any(|cp| (*cp as i64 - 100).unsigned_abs() <= 5));

        let noise = (0..200)
            .map(|index| ((((index * 97) % 113) as f64) / 113.0 - 0.5) * 0.4)
            .collect::<Vec<_>>();
        let noise_cps = detect_changepoints(&noise, &shifted_cfg);
        assert!(noise_cps.is_empty());
    }

    #[test]
    fn changepoint_sensitivity_specificity_pair() {
        let mut shifted = vec![0.0; 100];
        shifted.extend(vec![5.0; 100]);
        let config = PeltConfig {
            penalty: 6.0,
            adaptive_penalty: false,
            min_segment: 4,
            max_candidates: 200,
        };
        let cps = detect_changepoints(&shifted, &config);
        assert!(cps.iter().any(|cp| (*cp as i64 - 100).unsigned_abs() <= 5));

        let pure_noise = (0..200)
            .map(|index| ((((index * 29) % 89) as f64) / 89.0 - 0.5) * 0.3)
            .collect::<Vec<_>>();
        assert!(detect_changepoints(&pure_noise, &config).is_empty());
    }
}
