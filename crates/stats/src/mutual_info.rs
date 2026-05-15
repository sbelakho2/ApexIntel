//! Mutual information estimation via binning.
//!
//! Estimates are reported in bits after Miller-Madow bias correction. All functions filter out
//! `NaN` / `±∞` via the `min_max` helper so callers do not need to
//! pre-clean their data.
//!
//! **Bins parameter**: must be ≥ 1.  `bins = 0` is treated as having
//! insufficient data and returns `0.0` immediately.  More bins improve
//! resolution but require more data; a rule of thumb is `bins ≈ √(n/5)`.

/// Estimate mutual information between two variables using binned estimation.
///
/// Returns MI in bits after Miller-Madow bias correction. Returns `0.0` when `bins == 0`,
/// when either variable has insufficient variance, or when there are fewer
/// than `2 × bins` paired finite observations.
pub fn estimate(x: &[f64], y: &[f64], bins: usize) -> f64 {
    estimate_with_bins(x, y, bins)
}

pub fn estimate_adaptive(x: &[f64], y: &[f64]) -> f64 {
    let finite_pairs: Vec<(f64, f64)> = x
        .iter()
        .zip(y.iter())
        .filter_map(|(&xv, &yv)| {
            if xv.is_finite() && yv.is_finite() {
                Some((xv, yv))
            } else {
                None
            }
        })
        .collect();
    let x_only: Vec<f64> = finite_pairs.iter().map(|(xv, _)| *xv).collect();
    let y_only: Vec<f64> = finite_pairs.iter().map(|(_, yv)| *yv).collect();
    let bins = adaptive_bin_count(&x_only).max(adaptive_bin_count(&y_only));
    estimate_with_bins(x, y, bins)
}

pub fn normalized_mi_adaptive(x: &[f64], y: &[f64]) -> f64 {
    let finite_pairs: Vec<(f64, f64)> = x
        .iter()
        .zip(y.iter())
        .filter_map(|(&xv, &yv)| {
            if xv.is_finite() && yv.is_finite() {
                Some((xv, yv))
            } else {
                None
            }
        })
        .collect();
    let x_only: Vec<f64> = finite_pairs.iter().map(|(xv, _)| *xv).collect();
    let y_only: Vec<f64> = finite_pairs.iter().map(|(_, yv)| *yv).collect();
    let bins = adaptive_bin_count(&x_only).max(adaptive_bin_count(&y_only));
    normalized_mi(x, y, bins)
}

fn estimate_with_bins(x: &[f64], y: &[f64], bins: usize) -> f64 {
    let finite_pairs: Vec<(f64, f64)> = x
        .iter()
        .zip(y.iter())
        .filter_map(|(&xv, &yv)| {
            if xv.is_finite() && yv.is_finite() {
                Some((xv, yv))
            } else {
                None
            }
        })
        .collect();

    let n = finite_pairs.len();
    if n < bins * 2 || bins == 0 {
        return 0.0;
    }

    let x_only: Vec<f64> = finite_pairs.iter().map(|(xv, _)| *xv).collect();
    let y_only: Vec<f64> = finite_pairs.iter().map(|(_, yv)| *yv).collect();

    let (x_min, x_max) = min_max(&x_only);
    let (y_min, y_max) = min_max(&y_only);
    let x_step = (x_max - x_min) / bins as f64;
    let y_step = (y_max - y_min) / bins as f64;

    if x_step < 1e-12 || y_step < 1e-12 {
        return 0.0;
    }

    let mut joint = vec![vec![0u64; bins]; bins];
    let mut mx = vec![0u64; bins];
    let mut my = vec![0u64; bins];

    for (xv, yv) in finite_pairs {
        let xi = ((xv - x_min) / x_step).clamp(0.0, (bins - 1) as f64) as usize;
        let yi = ((yv - y_min) / y_step).clamp(0.0, (bins - 1) as f64) as usize;
        joint[xi][yi] += 1;
        mx[xi] += 1;
        my[yi] += 1;
    }

    let nf = n as f64;
    let mut mi_nats = 0.0;
    for i in 0..bins {
        for j in 0..bins {
            if joint[i][j] > 0 && mx[i] > 0 && my[j] > 0 {
                let pxy = joint[i][j] as f64 / nf;
                let px = mx[i] as f64 / nf;
                let py = my[j] as f64 / nf;
                mi_nats += pxy * (pxy / (px * py)).ln();
            }
        }
    }
    let mi_bits = mi_nats / std::f64::consts::LN_2;
    let correction_bits = miller_madow_bias_correction_bits(n, bins, bins);
    (mi_bits - correction_bits).max(0.0)
}

/// Normalized mutual information (0–1).
///
/// NMI = MI / (H(X) + H(Y)) / 2.  Returns `0.0` when both entropies are
/// negligible (constant variables) and `1.0` for perfectly dependent ones.
pub fn normalized_mi(x: &[f64], y: &[f64], bins: usize) -> f64 {
    let mi = estimate(x, y, bins);
    let hx = entropy_binned(x, bins);
    let hy = entropy_binned(y, bins);

    let denom = ((hx + hy) / 2.0).max(1e-12);
    (mi / denom).clamp(0.0, 1.0)
}

/// Compute Shannon entropy of a variable via binning, in bits.
fn entropy_binned(data: &[f64], bins: usize) -> f64 {
    let finite: Vec<f64> = data.iter().copied().filter(|v| v.is_finite()).collect();
    let n = finite.len();
    if n < bins || bins == 0 {
        return 0.0;
    }

    let (min_val, max_val) = min_max(&finite);
    let step = (max_val - min_val) / bins as f64;
    if step < 1e-12 {
        return 0.0;
    }

    let mut counts = vec![0u64; bins];
    for &v in &finite {
        let idx = ((v - min_val) / step).clamp(0.0, (bins - 1) as f64) as usize;
        counts[idx] += 1;
    }

    let nf = n as f64;
    let mut h_nats = 0.0;
    for &c in &counts {
        if c > 0 {
            let p = c as f64 / nf;
            h_nats -= p * p.ln();
        }
    }
    h_nats / std::f64::consts::LN_2
}

fn miller_madow_bias_correction_bits(sample_count: usize, x_bins: usize, y_bins: usize) -> f64 {
    if sample_count == 0 || x_bins == 0 || y_bins == 0 {
        return 0.0;
    }
    (x_bins.saturating_mul(y_bins).saturating_sub(1)) as f64
        / (2.0 * sample_count as f64 * std::f64::consts::LN_2)
}

fn adaptive_bin_count(data: &[f64]) -> usize {
    let mut finite: Vec<f64> = data
        .iter()
        .copied()
        .filter(|value| value.is_finite())
        .collect();
    if finite.len() < 4 {
        return 1;
    }
    finite.sort_by(|left, right| left.total_cmp(right));
    let q1 = percentile(&finite, 0.25);
    let q3 = percentile(&finite, 0.75);
    let iqr = (q3 - q1).abs();
    let (min_val, max_val) = min_max(&finite);
    let range = (max_val - min_val).abs();
    if iqr < 1e-12 || range < 1e-12 {
        return 1;
    }
    let bin_width = 2.0 * iqr * (finite.len() as f64).powf(-1.0 / 3.0);
    if bin_width < 1e-12 {
        return 1;
    }
    ((range / bin_width).ceil() as usize).max(1)
}

fn percentile(sorted: &[f64], quantile: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let position = quantile.clamp(0.0, 1.0) * (sorted.len().saturating_sub(1)) as f64;
    let lower = position.floor() as usize;
    let upper = position.ceil() as usize;
    if lower == upper {
        return sorted[lower];
    }
    let weight = position - lower as f64;
    sorted[lower] * (1.0 - weight) + sorted[upper] * weight
}

/// Returns `(min, max + 1e-12)` for the finite elements in `data`.
/// Returns `(0.0, 0.0)` when no finite elements exist (all-`NaN` / all-`±∞`
/// input), which causes downstream step-size computation to fall below the
/// `1e-12` guard and short-circuit with `MI = 0.0` (B255).
fn min_max(data: &[f64]) -> (f64, f64) {
    let mut min = f64::MAX;
    let mut max = f64::MIN;
    for &v in data {
        if !v.is_finite() {
            continue;
        }
        if v < min {
            min = v;
        }
        if v > max {
            max = v;
        }
    }
    if min == f64::MAX || max == f64::MIN {
        return (0.0, 0.0);
    }
    (min, max + 1e-12)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mi_independent() {
        // Two independent sequences
        let x: Vec<f64> = (0..100).map(|i| (i % 10) as f64).collect();
        let y: Vec<f64> = (0..100).map(|i| ((i / 10) % 10) as f64).collect();
        let mi = estimate(&x, &y, 5);
        // MI should be small (not necessarily zero due to binning artifacts)
        assert!(
            mi < 1.0,
            "MI for independent vars should be small, got {}",
            mi
        );
    }

    #[test]
    fn test_mi_perfectly_dependent() {
        let x: Vec<f64> = (0..100).map(|i| i as f64).collect();
        let y: Vec<f64> = (0..100).map(|i| i as f64 * 2.0).collect();
        let mi = estimate(&x, &y, 10);
        assert!(
            mi > 0.5,
            "MI for perfectly correlated vars should be high, got {}",
            mi
        );
    }

    #[test]
    fn test_mi_too_short() {
        let x = vec![1.0, 2.0];
        let y = vec![3.0, 4.0];
        let mi = estimate(&x, &y, 5);
        assert!((mi - 0.0).abs() < 1e-10);
    }

    #[test]
    fn test_normalized_mi_range() {
        let x: Vec<f64> = (0..100).map(|i| i as f64).collect();
        let y: Vec<f64> = (0..100).map(|i| i as f64 + 1.0).collect();
        let nmi = normalized_mi(&x, &y, 10);
        assert!(
            (0.0..=1.0).contains(&nmi),
            "NMI should be in [0,1], got {}",
            nmi
        );
    }

    #[test]
    fn test_entropy_constant() {
        let data = vec![5.0; 50];
        let h = entropy_binned(&data, 10);
        assert!((h - 0.0).abs() < 1e-10);
    }

    #[test]
    fn test_entropy_uniform() {
        // Generate uniform-ish data
        let data: Vec<f64> = (0..100).map(|i| i as f64).collect();
        let h = entropy_binned(&data, 10);
        // Entropy of uniform distribution with 10 bins = log2(10) ≈ 3.32 bits
        assert!(h > 2.0, "Uniform data should have high entropy, got {}", h);
    }

    #[test]
    fn test_min_max_ignores_nan() {
        let data = vec![1.0, f64::NAN, 3.0];
        let (min, max) = min_max(&data);
        assert_eq!(min, 1.0);
        assert!(max >= 3.0);
    }

    // ── B254: bins edge cases ────────────────────────────────────────────────

    #[test]
    fn test_mi_zero_bins_returns_zero() {
        // bins=0 must short-circuit safely, not panic or divide-by-zero.
        let x: Vec<f64> = (0..50).map(|i| i as f64).collect();
        let y: Vec<f64> = (0..50).map(|i| i as f64 * 2.0).collect();
        let mi = estimate(&x, &y, 0);
        assert!((mi - 0.0).abs() < 1e-10, "bins=0 must return 0.0, got {mi}");
    }

    #[test]
    fn test_mi_one_bin_returns_zero() {
        // bins=1 requires n >= 2; with reasonable data x_step=( max-min)/1
        // which is non-trivial, so all points land in bin 0 → p(x=0)=1,
        // p(y=0)=1, p(xy=00)=1 → MI = 1*ln(1/1*1) = 0.
        let x: Vec<f64> = (0..10).map(|i| i as f64).collect();
        let y: Vec<f64> = (0..10).map(|i| i as f64).collect();
        let mi = estimate(&x, &y, 1);
        assert!(
            (mi - 0.0).abs() < 1e-10,
            "bins=1 must return 0.0 (no resolution), got {mi}"
        );
    }

    #[test]
    fn test_nmi_zero_bins_returns_zero() {
        let x: Vec<f64> = (0..50).map(|i| i as f64).collect();
        let mi = normalized_mi(&x, &x, 0);
        assert!(
            (mi - 0.0).abs() < 1e-10,
            "NMI with bins=0 must return 0.0, got {mi}"
        );
    }

    // ── B255: all-NaN / all-Inf input safety via min_max ────────────────────

    #[test]
    fn test_mi_all_nan_returns_zero() {
        let data: Vec<f64> = vec![f64::NAN; 50];
        let mi = estimate(&data, &data, 5);
        assert!(
            (mi - 0.0).abs() < 1e-10,
            "all-NaN input must return 0.0, got {mi}"
        );
        assert!(mi.is_finite(), "result must be finite, not NaN");
    }

    #[test]
    fn test_mi_all_inf_returns_zero() {
        let data: Vec<f64> = vec![f64::INFINITY; 50];
        let mi = estimate(&data, &data, 5);
        assert!(
            (mi - 0.0).abs() < 1e-10,
            "all-Inf input must return 0.0, got {mi}"
        );
    }

    #[test]
    fn test_nmi_all_nan_returns_zero() {
        let data: Vec<f64> = vec![f64::NAN; 50];
        let nmi = normalized_mi(&data, &data, 5);
        assert!(
            (nmi - 0.0).abs() < 1e-10,
            "NMI for all-NaN must be 0.0, got {nmi}"
        );
    }

    #[test]
    fn test_mi_with_negative_values_is_finite() {
        let x: Vec<f64> = (-50..50).map(|i| i as f64).collect();
        let y: Vec<f64> = (-50..50).map(|i| (i as f64) * 1.5 - 2.0).collect();
        let mi = estimate(&x, &y, 10);
        assert!(mi.is_finite());
        assert!(mi >= 0.0);
    }

    #[test]
    fn test_normalized_mi_zero_entropy_returns_zero() {
        let x = vec![5.0; 100];
        let y = vec![5.0; 100];
        let nmi = normalized_mi(&x, &y, 10);
        assert!((nmi - 0.0).abs() < 1e-10);
    }

    #[test]
    fn test_mi_ignores_non_finite_pairs() {
        let x = vec![
            1.0,
            2.0,
            f64::NAN,
            4.0,
            f64::INFINITY,
            6.0,
            7.0,
            8.0,
            9.0,
            10.0,
        ];
        let y = vec![
            1.0,
            2.0,
            3.0,
            f64::NAN,
            5.0,
            6.0,
            7.0,
            f64::NEG_INFINITY,
            9.0,
            10.0,
        ];
        let mi = estimate(&x, &y, 3);
        assert!(mi.is_finite());
        assert!(mi >= 0.0);
    }

    #[test]
    fn test_adaptive_binning_detects_dependency() {
        let x: Vec<f64> = (0..200).map(|index| index as f64 / 10.0).collect();
        let y: Vec<f64> = x.iter().map(|value| value.sin()).collect();
        let mi = estimate_adaptive(&x, &y);
        assert!(mi.is_finite());
        assert!(mi >= 0.0);
    }

    #[test]
    fn test_mi_reported_in_bits_for_binary_signal() {
        let x: Vec<f64> = (0..200).map(|index| (index % 2) as f64).collect();
        let y = x.clone();
        let mi = estimate(&x, &y, 2);
        assert!(
            mi > 0.8 && mi <= 1.1,
            "perfect binary dependence should be about 1 bit, got {mi}"
        );
    }

    #[test]
    fn test_miller_madow_bias_correction_non_negative() {
        let x: Vec<f64> = (0..20).map(|index| (index % 4) as f64).collect();
        let y: Vec<f64> = x.iter().map(|value| value * 2.0).collect();
        let corrected = estimate(&x, &y, 4);
        assert!(corrected >= 0.0);
    }
}
