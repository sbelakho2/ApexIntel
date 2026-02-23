/// Lagged cross-correlation analysis.

/// Compute lagged cross-correlation between two time series.
///
/// Returns Vec<(lag, correlation)> for lags from -max_lag to +max_lag.
pub fn lagged_xcorr(x: &[f64], y: &[f64], max_lag: i32) -> Vec<(i32, f64)> {
    let n = x.len().min(y.len());
    if n < 3 {
        return vec![];
    }

    let mx = x[..n].iter().sum::<f64>() / n as f64;
    let my = y[..n].iter().sum::<f64>() / n as f64;
    let sx: f64 = x[..n].iter().map(|v| (v - mx).powi(2)).sum::<f64>().sqrt();
    let sy: f64 = y[..n].iter().map(|v| (v - my).powi(2)).sum::<f64>().sqrt();

    if sx < 1e-12 || sy < 1e-12 {
        return vec![];
    }

    (-max_lag..=max_lag)
        .map(|lag| {
            let mut num = 0.0;
            let mut count = 0;
            for i in 0..n {
                let j = i as i32 + lag;
                if j >= 0 && (j as usize) < n {
                    num += (x[i] - mx) * (y[j as usize] - my);
                    count += 1;
                }
            }
            let r = if count > 0 { num / (sx * sy) } else { 0.0 };
            (lag, r)
        })
        .collect()
}

/// Pearson correlation coefficient between two series.
pub fn pearson(x: &[f64], y: &[f64]) -> f64 {
    let n = x.len().min(y.len());
    if n < 2 {
        return 0.0;
    }

    let mx = x[..n].iter().sum::<f64>() / n as f64;
    let my = y[..n].iter().sum::<f64>() / n as f64;

    let mut cov = 0.0;
    let mut var_x = 0.0;
    let mut var_y = 0.0;

    for i in 0..n {
        let dx = x[i] - mx;
        let dy = y[i] - my;
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
pub fn spearman(x: &[f64], y: &[f64]) -> f64 {
    let n = x.len().min(y.len());
    if n < 2 {
        return 0.0;
    }

    let rank_x = ranks(&x[..n]);
    let rank_y = ranks(&y[..n]);

    pearson(&rank_x, &rank_y)
}

fn ranks(data: &[f64]) -> Vec<f64> {
    let n = data.len();
    let mut indexed: Vec<(usize, f64)> = data.iter().cloned().enumerate().collect();
    indexed.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());

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
    fn test_spearman_monotonic() {
        let x = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let y = vec![1.0, 4.0, 9.0, 16.0, 25.0]; // monotonic but not linear
        let r = spearman(&x, &y);
        assert!((r - 1.0).abs() < 1e-10, "Monotonic data should have Spearman = 1.0");
    }

    #[test]
    fn test_lagged_xcorr_self() {
        let x: Vec<f64> = (0..20).map(|i| (i as f64 * 0.5).sin()).collect();
        let results = lagged_xcorr(&x, &x, 5);
        // At lag 0, autocorrelation should be ~1.0
        let lag_0 = results.iter().find(|(lag, _)| *lag == 0).unwrap();
        assert!((lag_0.1 - 1.0).abs() < 0.01, "Self-correlation at lag 0 should be ~1.0, got {}", lag_0.1);
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
}
