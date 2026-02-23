/// Anomaly detection using MAD z-score and EWMA control charts.

/// Detect anomalies using Median Absolute Deviation (MAD) z-scores.
///
/// Returns Vec<(index, z_score)> for points exceeding the threshold.
pub fn mad_zscore(data: &[f64], threshold: f64) -> Vec<(usize, f64)> {
    if data.len() < 3 {
        return vec![];
    }

    let mut sorted = data.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median = sorted[sorted.len() / 2];

    let mut abs_devs: Vec<f64> = data.iter().map(|x| (x - median).abs()).collect();
    abs_devs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mad = abs_devs[abs_devs.len() / 2] * 1.4826; // consistency constant for normality

    if mad < 1e-12 {
        return vec![];
    }

    data.iter()
        .enumerate()
        .filter_map(|(i, x)| {
            let z = (x - median) / mad;
            if z.abs() > threshold {
                Some((i, z))
            } else {
                None
            }
        })
        .collect()
}

/// Detect anomalies using Exponentially Weighted Moving Average (EWMA) control chart.
///
/// Returns Vec<(index, deviation_in_sigmas)> for out-of-control points.
pub fn ewma_control(data: &[f64], alpha: f64, sigma_mult: f64) -> Vec<(usize, f64)> {
    if data.len() < 10 {
        return vec![];
    }

    let mean = data.iter().sum::<f64>() / data.len() as f64;
    let var = data.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / data.len() as f64;
    let sigma = var.sqrt();

    if sigma < 1e-12 {
        return vec![];
    }

    let mut ewma = mean;
    let mut anomalies = vec![];

    for (i, &x) in data.iter().enumerate() {
        ewma = alpha * x + (1.0 - alpha) * ewma;
        let limit = sigma * sigma_mult * (alpha / (2.0 - alpha)).sqrt();
        if (ewma - mean).abs() > limit {
            anomalies.push((i, (ewma - mean) / sigma));
        }
    }

    anomalies
}

/// Simple IQR-based outlier detection.
///
/// Returns indices of outlier points (below Q1 - k*IQR or above Q3 + k*IQR).
pub fn iqr_outliers(data: &[f64], k: f64) -> Vec<usize> {
    if data.len() < 4 {
        return vec![];
    }

    let mut sorted = data.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());

    let q1 = sorted[sorted.len() / 4];
    let q3 = sorted[3 * sorted.len() / 4];
    let iqr = q3 - q1;

    if iqr < 1e-12 {
        return vec![];
    }

    let lower = q1 - k * iqr;
    let upper = q3 + k * iqr;

    data.iter()
        .enumerate()
        .filter(|(_, x)| **x < lower || **x > upper)
        .map(|(i, _)| i)
        .collect()
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
}
