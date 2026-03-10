/// PELT (Pruned Exact Linear Time) change-point detection.

/// Configuration for the PELT change-point detector (B289).
///
/// # Default values
/// | Field          | Default | Rationale                                                           |
/// |---------------|---------|--------------------------------------------------------------------|
/// | `penalty`     | 3.0     | BIC-like penalty; lower detects more (potentially spurious) points |
/// | `min_segment` | 2       | Minimum observations per segment; must be ≥ 1                      |
///
/// Increase `penalty` to suppress noise-driven change-points on smooth series;
/// decrease it for volatile series where subtle shifts matter.
#[derive(Debug, Clone)]
pub struct PeltConfig {
    /// Log-likelihood change-point penalty.  Default: `3.0`.
    pub penalty: f64,
    /// Minimum data points in a valid segment.  Default: `2`.
    pub min_segment: usize,
}

impl Default for PeltConfig {
    fn default() -> Self {
        Self {
            penalty: 3.0,
            min_segment: 2,
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

        errors
    }
}

/// Detect change-points in a time series using the PELT algorithm.
///
/// Returns indices where the distribution of data changes.
pub fn detect_changepoints(data: &[f64], config: &PeltConfig) -> Vec<usize> {
    let n = data.len();
    if n < config.min_segment * 2 {
        return vec![];
    }

    let mut cost = vec![0.0_f64; n + 1];
    // Standard PELT initialization: F(0) = -β so that the no-changepoint
    // baseline has cost 0 and each split adds exactly β to the objective.
    cost[0] = -config.penalty;
    let mut cp_trace: Vec<Vec<usize>> = vec![vec![]; n + 1];
    // PELT candidate set — properly pruned each iteration
    let mut candidates: Vec<usize> = vec![0];

    for t in config.min_segment..=n {
        let mut best_cost = f64::MAX;
        let mut best_s = 0usize;

        for &s in &candidates {
            if t < s + config.min_segment {
                continue;
            }
            let seg_cost = gaussian_cost(&data[s..t]);
            let total = cost[s] + seg_cost + config.penalty;
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
        candidates.push(t);
        candidates.retain(|&s| cost[s] <= best_cost);
        // Always keep at least one candidate
        if candidates.is_empty() {
            candidates.push(t);
        }
    }

    cp_trace[n].clone()
}

/// Gaussian cost function for a segment: sum of squared deviations (always ≥ 0).
fn gaussian_cost(data: &[f64]) -> f64 {
    if data.is_empty() {
        return 0.0;
    }
    let n = data.len() as f64;
    let mean = data.iter().sum::<f64>() / n;
    data.iter().map(|x| (x - mean).powi(2)).sum::<f64>()
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
            min_segment: 3,
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
            min_segment: 3,
        };
        let cps = detect_changepoints(&data, &config);
        assert!(!cps.is_empty(), "Should detect shift");
        // The changepoint should be near index 30
        let closest = cps
            .iter()
            .min_by_key(|&&cp| (cp as i64 - 30).unsigned_abs())
            .unwrap();
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
            min_segment: 3,
        };
        let cps = detect_changepoints(&data, &config);
        assert!(cps.is_empty());
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
            min_segment: 0,
        };
        let errs = cfg.validate();
        assert!(errs.iter().any(|e| e.contains("penalty")));
        assert!(errs.iter().any(|e| e.contains("min_segment")));
    }
}
