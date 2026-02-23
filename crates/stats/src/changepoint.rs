/// PELT (Pruned Exact Linear Time) change-point detection.

#[derive(Debug, Clone)]
pub struct PeltConfig {
    pub penalty: f64,
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

/// Detect change-points in a time series using the PELT algorithm.
///
/// Returns indices where the distribution of data changes.
pub fn detect_changepoints(data: &[f64], config: &PeltConfig) -> Vec<usize> {
    let n = data.len();
    if n < config.min_segment * 2 {
        return vec![];
    }

    let mut cost = vec![0.0_f64; n + 1];
    let mut cp_trace: Vec<Vec<usize>> = vec![vec![]; n + 1];

    for t in config.min_segment..=n {
        let mut best_cost = f64::MAX;
        let mut best_s = 0;

        let start = if t > config.min_segment {
            t - n.min(t) // start from 0
        } else {
            0
        };

        for s in start..=(t.saturating_sub(config.min_segment)) {
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
        if t < cp_trace.len() {
            cp_trace[t] = new_cps;
        } else {
            cp_trace.push(new_cps);
        }
    }

    cp_trace[n].clone()
}

/// Gaussian cost function for a segment: n * (ln(var) + 1).
fn gaussian_cost(data: &[f64]) -> f64 {
    if data.is_empty() {
        return 0.0;
    }
    let n = data.len() as f64;
    let mean = data.iter().sum::<f64>() / n;
    let var = data.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n;
    if var < 1e-12 {
        return 0.0;
    }
    n * (var.ln() + 1.0)
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
        let closest = cps.iter().min_by_key(|&&cp| (cp as i64 - 30).unsigned_abs()).unwrap();
        assert!((*closest as i64 - 30).unsigned_abs() <= 5, "Changepoint at {} not near 30", closest);
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
}
