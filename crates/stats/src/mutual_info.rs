/// Mutual information estimation via binning.

/// Estimate mutual information between two variables using binned estimation.
///
/// Returns MI in nats (natural log).
pub fn estimate(x: &[f64], y: &[f64], bins: usize) -> f64 {
    let n = x.len().min(y.len());
    if n < bins * 2 || bins == 0 {
        return 0.0;
    }

    let (x_min, x_max) = min_max(&x[..n]);
    let (y_min, y_max) = min_max(&y[..n]);
    let x_step = (x_max - x_min) / bins as f64;
    let y_step = (y_max - y_min) / bins as f64;

    if x_step < 1e-12 || y_step < 1e-12 {
        return 0.0;
    }

    let mut joint = vec![vec![0u64; bins]; bins];
    let mut mx = vec![0u64; bins];
    let mut my = vec![0u64; bins];

    for i in 0..n {
        let xi = ((x[i] - x_min) / x_step).min((bins - 1) as f64) as usize;
        let yi = ((y[i] - y_min) / y_step).min((bins - 1) as f64) as usize;
        joint[xi][yi] += 1;
        mx[xi] += 1;
        my[yi] += 1;
    }

    let nf = n as f64;
    let mut mi = 0.0;
    for i in 0..bins {
        for j in 0..bins {
            if joint[i][j] > 0 && mx[i] > 0 && my[j] > 0 {
                let pxy = joint[i][j] as f64 / nf;
                let px = mx[i] as f64 / nf;
                let py = my[j] as f64 / nf;
                mi += pxy * (pxy / (px * py)).ln();
            }
        }
    }
    mi.max(0.0)
}

/// Normalized mutual information (0-1).
pub fn normalized_mi(x: &[f64], y: &[f64], bins: usize) -> f64 {
    let mi = estimate(x, y, bins);
    let hx = entropy_binned(x, bins);
    let hy = entropy_binned(y, bins);

    let denom = ((hx + hy) / 2.0).max(1e-12);
    (mi / denom).clamp(0.0, 1.0)
}

/// Compute Shannon entropy of a variable via binning.
fn entropy_binned(data: &[f64], bins: usize) -> f64 {
    let n = data.len();
    if n < bins || bins == 0 {
        return 0.0;
    }

    let (min_val, max_val) = min_max(data);
    let step = (max_val - min_val) / bins as f64;
    if step < 1e-12 {
        return 0.0;
    }

    let mut counts = vec![0u64; bins];
    for &v in data {
        let idx = ((v - min_val) / step).min((bins - 1) as f64) as usize;
        counts[idx] += 1;
    }

    let nf = n as f64;
    let mut h = 0.0;
    for &c in &counts {
        if c > 0 {
            let p = c as f64 / nf;
            h -= p * p.ln();
        }
    }
    h
}

fn min_max(data: &[f64]) -> (f64, f64) {
    let mut min = f64::MAX;
    let mut max = f64::MIN;
    for &v in data {
        if v < min {
            min = v;
        }
        if v > max {
            max = v;
        }
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
        let y: Vec<f64> = (0..100).map(|i| ((i * 7) % 10) as f64).collect();
        let mi = estimate(&x, &y, 5);
        // MI should be small (not necessarily zero due to binning artifacts)
        assert!(mi < 1.0, "MI for independent vars should be small, got {}", mi);
    }

    #[test]
    fn test_mi_perfectly_dependent() {
        let x: Vec<f64> = (0..100).map(|i| i as f64).collect();
        let y: Vec<f64> = (0..100).map(|i| i as f64 * 2.0).collect();
        let mi = estimate(&x, &y, 10);
        assert!(mi > 0.5, "MI for perfectly correlated vars should be high, got {}", mi);
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
        assert!(nmi >= 0.0 && nmi <= 1.0, "NMI should be in [0,1], got {}", nmi);
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
        // Entropy of uniform distribution with 10 bins = ln(10) ≈ 2.30
        assert!(h > 1.5, "Uniform data should have high entropy, got {}", h);
    }
}
