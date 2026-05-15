//! Lagged cross-correlation analysis.
//!
//! All correlation functions return values in `[−1, 1]`. `NaN` and `±∞`
//! input values are filtered before Pearson computation. When too few pairs
//! remain (< 2 for Pearson/Spearman, < 3 per-lag for lagged xcorr), the
//! function returns `0.0` rather than panicking.

/// Compute lagged cross-correlation between two time series.
///
/// Returns `Vec<(lag, correlation)>` for lags from `−max_lag` to `+max_lag`
/// (always `2 × max_lag + 1` entries).
///
/// Uses per-lag Pearson normalisation over the overlapping window, requiring
/// at least 3 overlapping points to report a non-zero correlation.
/// When `max_lag ≥ series length`, many lags will have fewer than 3
/// overlapping points and their correlation is reported as `0.0` (B256).
pub fn lagged_xcorr(x: &[f64], y: &[f64], max_lag: i32) -> Vec<(i32, f64)> {
    let n = x.len().min(y.len());
    if n < 3 {
        return vec![];
    }

    (-max_lag..=max_lag)
        .map(|lag| {
            // Collect overlapping indices
            let pairs: Vec<(f64, f64)> = (0..n)
                .filter_map(|i| {
                    let j = i as i32 + lag;
                    if j >= 0 && (j as usize) < n {
                        Some((x[i], y[j as usize]))
                    } else {
                        None
                    }
                })
                .collect();

            let count = pairs.len();
            if count < 3 {
                return (lag, 0.0);
            }

            let mx = pairs.iter().map(|(a, _)| a).sum::<f64>() / count as f64;
            let my = pairs.iter().map(|(_, b)| b).sum::<f64>() / count as f64;

            let mut num = 0.0;
            let mut sx2 = 0.0;
            let mut sy2 = 0.0;
            for &(xi, yi) in &pairs {
                let dx = xi - mx;
                let dy = yi - my;
                num += dx * dy;
                sx2 += dx * dx;
                sy2 += dy * dy;
            }

            let denom = (sx2 * sy2).sqrt();
            let r = if denom > 1e-12 { num / denom } else { 0.0 };
            (lag, r)
        })
        .collect()
}

/// Pearson correlation coefficient between two series.
///
/// Filters out pairs where either value is `NaN` or `±∞`.  Returns `0.0`
/// when fewer than 2 finite pairs remain or when either variable has zero
/// variance.
pub fn pearson(x: &[f64], y: &[f64]) -> f64 {
    let n = x.len().min(y.len());
    let pairs: Vec<(f64, f64)> = x[..n]
        .iter()
        .zip(&y[..n])
        .filter(|(a, b)| a.is_finite() && b.is_finite())
        .map(|(a, b)| (*a, *b))
        .collect();
    if pairs.len() < 2 {
        return 0.0;
    }

    let mx = pairs.iter().map(|(a, _)| a).sum::<f64>() / pairs.len() as f64;
    let my = pairs.iter().map(|(_, b)| b).sum::<f64>() / pairs.len() as f64;

    let mut cov = 0.0;
    let mut var_x = 0.0;
    let mut var_y = 0.0;

    for (xi, yi) in pairs {
        let dx = xi - mx;
        let dy = yi - my;
        cov += dx * dy;
        var_x += dx * dx;
        var_y += dy * dy;
    }

    let denom = (var_x * var_y).sqrt();
    if denom < 1e-12 {
        0.0
    } else {
        cov / denom
    }
}

/// Spearman rank correlation coefficient.
///
/// Filters `NaN` pairs before ranking (B257 — avoids rank corruption for
/// all-NaN series).  Ties receive averaged ranks following the standard
/// convention.  When all values in one series are identical (all-ties),
/// rank variance is zero and `0.0` is returned (B257).
pub fn spearman(x: &[f64], y: &[f64]) -> f64 {
    let n = x.len().min(y.len());
    // Filter out pairs where either value is NaN to prevent rank corruption.
    let pairs: Vec<(f64, f64)> = x[..n]
        .iter()
        .zip(&y[..n])
        .filter(|(a, b)| !a.is_nan() && !b.is_nan())
        .map(|(a, b)| (*a, *b))
        .collect();
    if pairs.len() < 2 {
        return 0.0;
    }

    let fx: Vec<f64> = pairs.iter().map(|(a, _)| *a).collect();
    let fy: Vec<f64> = pairs.iter().map(|(_, b)| *b).collect();

    let rank_x = ranks(&fx);
    let rank_y = ranks(&fy);

    pearson(&rank_x, &rank_y)
}

fn ranks(data: &[f64]) -> Vec<f64> {
    let n = data.len();
    let mut indexed: Vec<(usize, f64)> = data.iter().cloned().enumerate().collect();
    indexed.sort_by(|a, b| a.1.total_cmp(&b.1));

    let mut result = vec![0.0; n];
    let mut i = 0;
    while i < n {
        let mut j = i;
        while j < n - 1 && (indexed[j + 1].1 - indexed[j].1).abs() < 1e-12 {
            j += 1;
        }
        // Average rank for ties
        let avg_rank = (i + j) as f64 / 2.0 + 1.0;
        for k in i..=j {
            result[indexed[k].0] = avg_rank;
        }
        i = j + 1;
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pearson_perfect_positive() {
        let x: Vec<f64> = (0..10).map(|i| i as f64).collect();
        let y: Vec<f64> = (0..10).map(|i| i as f64 * 2.0 + 1.0).collect();
        let r = pearson(&x, &y);
        assert!((r - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_pearson_perfect_negative() {
        let x: Vec<f64> = (0..10).map(|i| i as f64).collect();
        let y: Vec<f64> = (0..10).map(|i| -(i as f64)).collect();
        let r = pearson(&x, &y);
        assert!((r - (-1.0)).abs() < 1e-10);
    }

    #[test]
    fn test_pearson_zero_variance() {
        let x = vec![5.0; 10];
        let y: Vec<f64> = (0..10).map(|i| i as f64).collect();
        let r = pearson(&x, &y);
        assert!((r - 0.0).abs() < 1e-10);
    }

    #[test]
    fn test_pearson_ignores_nan() {
        let x = vec![1.0, f64::NAN, 3.0, 4.0];
        let y = vec![1.0, 2.0, 3.0, 4.0];
        let r = pearson(&x, &y);
        assert!(r.is_finite());
    }

    #[test]
    fn test_spearman_monotonic() {
        let x = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let y = vec![1.0, 4.0, 9.0, 16.0, 25.0]; // monotonic but not linear
        let r = spearman(&x, &y);
        assert!(
            (r - 1.0).abs() < 1e-10,
            "Monotonic data should have Spearman = 1.0"
        );
    }

    #[test]
    fn test_lagged_xcorr_self() {
        let x: Vec<f64> = (0..20).map(|i| (i as f64 * 0.5).sin()).collect();
        let results = lagged_xcorr(&x, &x, 5);
        // At lag 0, autocorrelation should be ~1.0
        let lag_0 = results
            .iter()
            .find(|(lag, _)| *lag == 0)
            .unwrap_or_else(|| panic!("lag 0 should be present"));
        assert!(
            (lag_0.1 - 1.0).abs() < 0.01,
            "Self-correlation at lag 0 should be ~1.0, got {}",
            lag_0.1
        );
    }

    #[test]
    fn test_lagged_xcorr_returns_correct_lags() {
        let x: Vec<f64> = (0..20).map(|i| i as f64).collect();
        let y: Vec<f64> = (0..20).map(|i| (i as f64) * 2.0).collect();
        let results = lagged_xcorr(&x, &y, 3);
        assert_eq!(results.len(), 7); // -3, -2, -1, 0, 1, 2, 3
    }

    #[test]
    fn test_lagged_xcorr_too_short() {
        let x = vec![1.0, 2.0];
        let y = vec![3.0, 4.0];
        let results = lagged_xcorr(&x, &y, 1);
        assert!(results.is_empty());
    }

    #[test]
    fn test_ranks() {
        let data = vec![4.0, 2.0, 3.0, 1.0];
        let r = ranks(&data);
        assert!((r[0] - 4.0).abs() < 1e-10);
        assert!((r[1] - 2.0).abs() < 1e-10);
        assert!((r[2] - 3.0).abs() < 1e-10);
        assert!((r[3] - 1.0).abs() < 1e-10);
    }

    // ── B256: lagged_xcorr with max_lag ≥ series length ─────────────────────

    #[test]
    fn test_lagged_xcorr_max_lag_larger_than_series() {
        // Series of length 5, max_lag = 10 → many lags have < 3 overlapping
        // points and should report 0.0 without panicking.
        let x: Vec<f64> = (0..5).map(|i| i as f64).collect();
        let y: Vec<f64> = (0..5).map(|i| i as f64 * 2.0).collect();
        let results = lagged_xcorr(&x, &y, 10);

        // Must still return 2*max_lag+1 = 21 entries
        assert_eq!(
            results.len(),
            21,
            "lagged_xcorr must return 2*max_lag+1 entries"
        );

        // Lags with abs > 2 have < 3 overlapping points → must be 0.0
        for (lag, corr) in &results {
            if lag.abs() > 2 {
                assert!(
                    corr.abs() < 1e-10,
                    "lag {lag} with <3 overlap must give corr≈0; got {corr}"
                );
            }
        }
    }

    #[test]
    fn test_lagged_xcorr_max_lag_equals_series_length_minus_one() {
        // max_lag = n - 1: only lag 0 has full overlap, boundary lags have 1 point.
        let x: Vec<f64> = (0..6).map(|i| i as f64).collect();
        let y = x.clone();
        let results = lagged_xcorr(&x, &y, 5);
        assert_eq!(results.len(), 11);
        // Lag 0 should still show high autocorrelation
        let lag0 = results
            .iter()
            .find(|(lag, _)| *lag == 0)
            .map(|(_, value)| *value)
            .unwrap_or_else(|| panic!("lag 0 should be present"));
        assert!(
            (lag0 - 1.0).abs() < 1e-10,
            "lag-0 autocorr should be 1.0; got {lag0}"
        );
    }

    // ── B257: spearman when all values are identical (all-ties) ─────────────

    #[test]
    fn test_spearman_all_ties_one_series_returns_zero() {
        // All values in X are the same → all get the same rank → variance = 0
        // → Pearson of constant ranks = 0.0.
        let x = vec![5.0, 5.0, 5.0, 5.0, 5.0];
        let y = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let r = spearman(&x, &y);
        assert!(
            (r - 0.0).abs() < 1e-10,
            "all-ties in one series must give Spearman=0; got {r}"
        );
    }

    #[test]
    fn test_spearman_all_ties_both_series_returns_zero() {
        let x = vec![3.0; 5];
        let y = vec![7.0; 5];
        let r = spearman(&x, &y);
        assert!(
            (r - 0.0).abs() < 1e-10,
            "all-ties in both series must give Spearman=0; got {r}"
        );
    }

    #[test]
    fn test_spearman_partial_ties_still_finite() {
        // Several ties but not all → result must be in [-1,1] and finite.
        let x = vec![1.0, 1.0, 2.0, 2.0, 3.0];
        let y = vec![1.0, 2.0, 2.0, 3.0, 3.0];
        let r = spearman(&x, &y);
        assert!(r.is_finite(), "Spearman with ties must be finite; got {r}");
        assert!(
            (-1.0..=1.0).contains(&r),
            "Spearman must be in [-1,1]; got {r}"
        );
    }

    #[test]
    fn test_lagged_xcorr_with_nan_inputs_is_finite() {
        let x = vec![1.0, f64::NAN, 3.0, 4.0, 5.0, 6.0];
        let y = vec![1.0, 2.0, 3.0, f64::NAN, 5.0, 6.0];
        let results = lagged_xcorr(&x, &y, 2);
        assert_eq!(results.len(), 5);
        assert!(results.iter().all(|(_, c)| c.is_finite()));
    }

    #[test]
    fn test_spearman_nan_pairs_dropped() {
        let x = vec![1.0, f64::NAN, 3.0, 4.0];
        let y = vec![1.0, 2.0, 3.0, 4.0];
        let r = spearman(&x, &y);
        assert!(
            (r - 1.0).abs() < 1e-10,
            "Expected perfect monotonic rank after NaN-pair drop, got {r}"
        );
    }
}
