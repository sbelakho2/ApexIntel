//! Anomaly detection using MAD z-score, EWMA control charts, and IQR fences.
//!
//! All functions filter out `NaN` and `±∞` inputs before computing statistics,
//! so callers need not pre-clean their data. When too few finite observations
//! remain (typically < 3–10 depending on the algorithm), an empty `Vec` is
//! returned instead of panicking.

/// Detect anomalies using Median Absolute Deviation (MAD) z-scores.
///
/// Requires ≥ 3 finite observations; returns `vec![]` for smaller inputs.
/// Skips `NaN` / `±∞` values in `data` before computing the median and MAD.
///
/// Returns `Vec<(index, z_score)>` for points exceeding `threshold`.
/// `threshold` is typically 3.0 (≈ 3σ equivalent for Gaussian data).
pub fn mad_zscore(data: &[f64], threshold: f64) -> Vec<(usize, f64)> {
    let clean: Vec<(usize, f64)> = data
        .iter()
        .enumerate()
        .filter(|(_, v)| v.is_finite())
        .map(|(i, v)| (i, *v))
        .collect();
    if clean.len() < 3 {
        return vec![];
    }

    let mut sorted: Vec<f64> = clean.iter().map(|(_, v)| *v).collect();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let median = median_of_sorted(&sorted);

    let mut abs_devs: Vec<f64> = clean.iter().map(|(_, x)| (x - median).abs()).collect();
    abs_devs.sort_by(|a, b| a.total_cmp(b));
    let mad = median_of_sorted(&abs_devs) * 1.4826; // consistency constant for normality

    if mad < 1e-12 {
        return vec![];
    }

    clean
        .iter()
        .filter_map(|(i, x)| {
            let z = (x - median) / mad;
            if z.abs() > threshold {
                Some((*i, z))
            } else {
                None
            }
        })
        .collect()
}

/// Detect anomalies using Exponentially Weighted Moving Average (EWMA)
/// control chart (Montgomery, 6th ed. §9.2).
///
/// # Parameters
/// - `data`: time-series of observations.  `NaN` / `±∞` values are filtered out.
/// - `alpha`: smoothing factor in `(0, 1]`.  Out-of-range values produce
///   an empty result (B251).  Values very close to 1 converge to the
///   single-point memory case and are handled without overflow by an
///   asymptotic guard (B252).
/// - `sigma_mult`: control-limit multiplier (> 0).  A typical value is 3.0.
///   Returns `vec![]` when ≤ 0 (B251).
///
/// Requires ≥ 10 finite observations; returns `vec![]` for smaller inputs.
///
/// Returns `Vec<(index, deviation_in_sigmas)>` for out-of-control points.
pub fn ewma_control(data: &[f64], alpha: f64, sigma_mult: f64) -> Vec<(usize, f64)> {
    // B251: validate parameter ranges before any computation.
    if !alpha.is_finite() || alpha <= 0.0 || alpha > 1.0 {
        return vec![];
    }
    if !sigma_mult.is_finite() || sigma_mult <= 0.0 {
        return vec![];
    }
    let clean: Vec<(usize, f64)> = data
        .iter()
        .enumerate()
        .filter(|(_, v)| v.is_finite())
        .map(|(i, v)| (i, *v))
        .collect();
    if clean.len() < 10 {
        return vec![];
    }

    let mean = clean.iter().map(|(_, v)| v).sum::<f64>() / clean.len() as f64;
    let var = clean.iter().map(|(_, x)| (x - mean).powi(2)).sum::<f64>() / (clean.len() - 1) as f64;
    let sigma = var.sqrt();

    if sigma < 1e-12 {
        return vec![];
    }

    let mut ewma = mean;
    let mut anomalies = vec![];

    for (pos, x) in clean.iter() {
        ewma = alpha * *x + (1.0 - alpha) * ewma;
        // Exact time-varying EWMA control limit (Montgomery, 6th ed.):
        // Var(Z_i) = σ² · (λ/(2−λ)) · [1 − (1−λ)^{2(i+1)}]
        // i is 0-based, so observation number is i+1.
        // For large i the term (1-α)^{2(i+1)} is negligible and the
        // exponent 2*(i+1) would overflow i32.  Short-circuit once the
        // correction is below f64 precision (~53 doublings of (1-α)²).
        let exponent = 2 * (*pos + 1);
        let time_factor = if exponent > 1074 {
            1.0
        } else {
            1.0 - (1.0 - alpha).powi(exponent as i32)
        };
        let limit = sigma * sigma_mult * (alpha / (2.0 - alpha) * time_factor).sqrt();
        if (ewma - mean).abs() > limit {
            anomalies.push((*pos, (ewma - mean) / sigma));
        }
    }

    anomalies
}

/// Simple IQR-based outlier detection.
///
/// Returns indices of outlier points (below `Q1 − k·IQR` or above `Q3 + k·IQR`).
/// Requires ≥ 4 finite observations.  `k = 1.5` is the classic Tukey fence.
pub fn iqr_outliers(data: &[f64], k: f64) -> Vec<usize> {
    let clean: Vec<(usize, f64)> = data
        .iter()
        .enumerate()
        .filter(|(_, v)| v.is_finite())
        .map(|(i, v)| (i, *v))
        .collect();
    if clean.len() < 4 {
        return vec![];
    }

    let mut sorted: Vec<f64> = clean.iter().map(|(_, v)| *v).collect();
    sorted.sort_by(|a, b| a.total_cmp(b));

    let q1 = percentile_linear(&sorted, 0.25);
    let q3 = percentile_linear(&sorted, 0.75);
    let iqr = q3 - q1;

    if iqr < 1e-12 {
        return vec![];
    }

    let lower = q1 - k * iqr;
    let upper = q3 + k * iqr;

    clean
        .iter()
        .filter(|(_, x)| *x < lower || *x > upper)
        .map(|(i, _)| *i)
        .collect()
}

/// Compute the median of a pre-sorted slice.
///
/// For even-length slices, returns the average of the two middle elements
/// (the standard statistical median).
fn median_of_sorted(sorted: &[f64]) -> f64 {
    let n = sorted.len();
    if n == 0 {
        return 0.0;
    }
    if n.is_multiple_of(2) {
        (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0
    } else {
        sorted[n / 2]
    }
}

/// Linear interpolation percentile on a pre-sorted slice (R type 7 / NumPy default).
fn percentile_linear(sorted: &[f64], p: f64) -> f64 {
    let n = sorted.len();
    if n == 0 {
        return 0.0;
    }
    if n == 1 {
        return sorted[0];
    }
    let idx = p * (n - 1) as f64;
    let lo = idx.floor() as usize;
    let hi = (lo + 1).min(n - 1);
    let frac = idx - lo as f64;
    sorted[lo] * (1.0 - frac) + sorted[hi] * frac
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mad_zscore_detects_outlier() {
        let mut data: Vec<f64> = vec![1.0, 1.1, 0.9, 1.0, 1.05, 0.95, 1.0, 1.02, 0.98, 1.0];
        data.push(50.0); // extreme outlier
        let anomalies = mad_zscore(&data, 3.0);
        assert!(!anomalies.is_empty());
        assert!(anomalies.iter().any(|(i, _)| *i == 10));
    }

    #[test]
    fn test_mad_zscore_no_outliers() {
        let data: Vec<f64> = vec![1.0, 1.1, 0.9, 1.0, 1.05, 0.95];
        let anomalies = mad_zscore(&data, 5.0);
        assert!(anomalies.is_empty());
    }

    #[test]
    fn test_mad_zscore_too_short() {
        let data = vec![1.0, 2.0];
        let anomalies = mad_zscore(&data, 3.0);
        assert!(anomalies.is_empty());
    }

    #[test]
    fn test_ewma_detects_shift() {
        let mut data: Vec<f64> = vec![5.0; 30];
        data.extend(vec![50.0; 30]);
        let anomalies = ewma_control(&data, 0.3, 2.0);
        assert!(!anomalies.is_empty());
    }

    #[test]
    fn test_ewma_stable_data() {
        let data: Vec<f64> = vec![5.0; 30];
        let anomalies = ewma_control(&data, 0.2, 3.0);
        assert!(anomalies.is_empty());
    }

    #[test]
    fn test_ewma_too_short() {
        let data = vec![1.0; 5];
        let anomalies = ewma_control(&data, 0.2, 3.0);
        assert!(anomalies.is_empty());
    }

    #[test]
    fn test_iqr_outliers_detects() {
        let mut data: Vec<f64> = (0..20).map(|i| 10.0 + i as f64 * 0.1).collect();
        data.push(100.0); // outlier
        data.push(-50.0); // outlier
        let outliers = iqr_outliers(&data, 1.5);
        assert!(outliers.contains(&20));
        assert!(outliers.contains(&21));
    }

    #[test]
    fn test_iqr_no_outliers() {
        let data: Vec<f64> = (0..20).map(|i| 10.0 + i as f64 * 0.01).collect();
        let outliers = iqr_outliers(&data, 1.5);
        assert!(outliers.is_empty());
    }

    #[test]
    fn test_nan_inputs_are_ignored() {
        let data = vec![1.0, f64::NAN, 1.1, 0.9, f64::INFINITY];
        let outliers = mad_zscore(&data, 3.0);
        assert!(outliers.is_empty() || outliers.iter().all(|(_, v)| v.is_finite()));
    }

    // ── B251: alpha / sigma_mult validation ─────────────────────────────────

    #[test]
    fn test_ewma_alpha_zero_returns_empty() {
        let data: Vec<f64> = vec![5.0; 30];
        assert!(
            ewma_control(&data, 0.0, 3.0).is_empty(),
            "alpha=0 should be rejected"
        );
    }

    #[test]
    fn test_ewma_alpha_negative_returns_empty() {
        let data: Vec<f64> = vec![5.0; 30];
        assert!(
            ewma_control(&data, -0.1, 3.0).is_empty(),
            "negative alpha should be rejected"
        );
    }

    #[test]
    fn test_ewma_alpha_above_one_returns_empty() {
        let data: Vec<f64> = vec![5.0; 30];
        assert!(
            ewma_control(&data, 1.01, 3.0).is_empty(),
            "alpha > 1 should be rejected"
        );
    }

    #[test]
    fn test_ewma_alpha_exactly_one_is_valid() {
        // alpha=1 collapses to a point-memory detector (e.g. exact-value comparison);
        // it is a degenerate but valid boundary case.
        let mut data: Vec<f64> = vec![5.0; 30];
        data.extend(vec![50.0; 10]);
        let result = ewma_control(&data, 1.0, 3.0);
        // Must not panic and result must be finite
        for (_, dev) in &result {
            assert!(dev.is_finite(), "deviation must be finite for alpha=1");
        }
    }

    #[test]
    fn test_ewma_sigma_mult_zero_returns_empty() {
        let data: Vec<f64> = vec![5.0; 30];
        assert!(
            ewma_control(&data, 0.3, 0.0).is_empty(),
            "sigma_mult=0 should be rejected"
        );
    }

    #[test]
    fn test_ewma_sigma_mult_negative_returns_empty() {
        let data: Vec<f64> = vec![5.0; 30];
        assert!(
            ewma_control(&data, 0.3, -1.0).is_empty(),
            "negative sigma_mult should be rejected"
        );
    }

    // ── B252: alpha close to 1 does not overflow ─────────────────────────────

    #[test]
    fn test_ewma_alpha_close_to_one_no_overflow() {
        // alpha=0.999 makes (1−α)^{2i} extremely small for large i;
        // the existing exponent > 1074 guard should handle this without panic
        // and all deviation values must be finite.
        let mut data: Vec<f64> = vec![5.0; 500];
        data.extend(vec![100.0; 50]);
        let result = ewma_control(&data, 0.999, 3.0);
        for &(_, dev) in &result {
            assert!(
                dev.is_finite(),
                "deviation must be finite for alpha close to 1"
            );
        }
        // The shift at index 500 should be detected
        assert!(
            !result.is_empty(),
            "large shift must be detected with alpha=0.999"
        );
    }

    // ── B253: mad_zscore with NaN-heavy inputs ───────────────────────────────

    #[test]
    fn test_mad_zscore_all_nan_returns_empty() {
        let data = vec![f64::NAN; 20];
        assert!(
            mad_zscore(&data, 3.0).is_empty(),
            "all-NaN input must return empty vec"
        );
    }

    #[test]
    fn test_mad_zscore_all_inf_returns_empty() {
        let data = vec![f64::INFINITY; 20];
        assert!(
            mad_zscore(&data, 3.0).is_empty(),
            "all-Inf input must return empty vec"
        );
    }

    #[test]
    fn test_mad_zscore_mostly_nan_still_detects_outlier() {
        // Only 4 finite values; 3 clustered around 1.0, 1 extreme outlier.
        let mut data: Vec<f64> = vec![f64::NAN; 50];
        data[5] = 1.0;
        data[10] = 1.1;
        data[15] = 0.9;
        data[20] = 100.0; // extreme outlier
        let anomalies = mad_zscore(&data, 3.0);
        assert!(
            !anomalies.is_empty(),
            "outlier at index 20 must be detected despite NaN majority"
        );
        assert!(anomalies.iter().any(|(i, _)| *i == 20));
    }

    #[test]
    fn test_mad_zscore_nan_result_indices_are_valid() {
        // All returned indices must correspond to finite values in the original slice.
        let mut data: Vec<f64> = (0..30).map(|i| i as f64 * 0.1).collect();
        data[5] = f64::NAN;
        data[10] = f64::NAN;
        data[25] = 999.0; // outlier
        let anomalies = mad_zscore(&data, 3.0);
        for (idx, _) in &anomalies {
            assert!(
                data[*idx].is_finite(),
                "anomaly index {idx} points to a non-finite value"
            );
        }
    }
}
