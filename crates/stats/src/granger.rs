use crate::fdr;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

const MIN_OBSERVATIONS: usize = 50;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GrangerLagResult {
    pub lag: usize,
    pub f_stat: f64,
    pub p_value: f64,
    pub rss_restricted: f64,
    pub rss_full: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PredictiveSignalEdge {
    pub source_signal: String,
    pub target_signal: String,
    pub lag: usize,
    pub f_stat: f64,
    pub p_value: f64,
    pub q_value: f64,
}

pub fn granger_causality(x: &[f64], y: &[f64], lag: usize) -> Option<GrangerLagResult> {
    if lag == 0 {
        return None;
    }
    let n = x.len().min(y.len());
    if n < MIN_OBSERVATIONS || n <= (lag + 3) {
        return None;
    }

    let mut restricted_design = Vec::new();
    let mut full_design = Vec::new();
    let mut targets = Vec::new();

    for t in lag..n {
        let target = y[t];
        if !target.is_finite() {
            continue;
        }

        let mut restricted_row = vec![1.0];
        let mut full_row = vec![1.0];
        let mut valid = true;

        for offset in 1..=lag {
            let y_lag = y[t - offset];
            if !y_lag.is_finite() {
                valid = false;
                break;
            }
            restricted_row.push(y_lag);
            full_row.push(y_lag);
        }
        if !valid {
            continue;
        }
        let exogenous_lag = x[t - lag];
        if !exogenous_lag.is_finite() {
            continue;
        }
        full_row.push(exogenous_lag);

        restricted_design.push(restricted_row);
        full_design.push(full_row);
        targets.push(target);
    }

    if targets.len() < MIN_OBSERVATIONS || targets.len() <= (lag + 3) {
        return None;
    }

    let restricted_beta = fit_ols(&restricted_design, &targets)?;
    let full_beta = fit_ols(&full_design, &targets)?;
    let rss_restricted = residual_sum_squares(&restricted_design, &targets, &restricted_beta);
    let rss_full = residual_sum_squares(&full_design, &targets, &full_beta);
    if rss_full <= 1e-12 || rss_restricted + 1e-12 < rss_full {
        return None;
    }

    let df_num = 1.0;
    let df_den = (targets.len() as isize - (lag + 2) as isize) as f64;
    if df_den <= 0.0 {
        return None;
    }

    let numerator = ((rss_restricted - rss_full).max(0.0)) / df_num;
    let denominator = rss_full / df_den;
    if denominator <= 0.0 {
        return None;
    }
    let f_stat = numerator / denominator;
    let p_value = f_distribution_survival(f_stat, df_num, df_den);

    Some(GrangerLagResult {
        lag,
        f_stat,
        p_value,
        rss_restricted,
        rss_full,
    })
}

pub fn best_granger_lag(x: &[f64], y: &[f64], lags: &[usize]) -> Option<GrangerLagResult> {
    lags.iter()
        .filter_map(|&lag| granger_causality(x, y, lag))
        .min_by(|left, right| {
            left.p_value
                .partial_cmp(&right.p_value)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left.lag.cmp(&right.lag))
        })
}

pub fn build_predictive_signal_graph(
    signal_series: &HashMap<String, Vec<f64>>,
    lags: &[usize],
    fdr_alpha: f64,
) -> Vec<PredictiveSignalEdge> {
    let mut candidates = Vec::new();
    let signal_names = signal_series.keys().cloned().collect::<Vec<_>>();

    for source_signal in &signal_names {
        for target_signal in &signal_names {
            if source_signal == target_signal {
                continue;
            }
            let Some(source_series) = signal_series.get(source_signal) else {
                continue;
            };
            let Some(target_series) = signal_series.get(target_signal) else {
                continue;
            };
            if let Some(best) = best_granger_lag(source_series, target_series, lags) {
                candidates.push((source_signal.clone(), target_signal.clone(), best));
            }
        }
    }

    let q_values = fdr::bh_correct(
        &candidates
            .iter()
            .map(|(_, _, result)| result.p_value)
            .collect::<Vec<_>>(),
    );

    let mut edges = candidates
        .into_iter()
        .zip(q_values)
        .filter_map(|((source_signal, target_signal, result), q_value)| {
            if result.p_value < 0.05 && q_value <= fdr_alpha {
                Some(PredictiveSignalEdge {
                    source_signal,
                    target_signal,
                    lag: result.lag,
                    f_stat: result.f_stat,
                    p_value: result.p_value,
                    q_value,
                })
            } else {
                None
            }
        })
        .collect::<Vec<_>>();

    edges.sort_by(|left, right| {
        left.q_value
            .partial_cmp(&right.q_value)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.source_signal.cmp(&right.source_signal))
            .then_with(|| left.target_signal.cmp(&right.target_signal))
    });
    edges
}

pub fn predictive_alerts_for_signal(
    fired_signal: &str,
    edges: &[PredictiveSignalEdge],
) -> Vec<String> {
    let mut alerts = edges
        .iter()
        .filter(|edge| edge.source_signal == fired_signal)
        .map(|edge| {
            format!(
                "Signal {} just fired -> {} historically follows within {} days (q={:.3})",
                edge.source_signal, edge.target_signal, edge.lag, edge.q_value
            )
        })
        .collect::<Vec<_>>();
    alerts.sort();
    alerts
}

fn fit_ols(design: &[Vec<f64>], targets: &[f64]) -> Option<Vec<f64>> {
    let parameter_count = design.first()?.len();
    let mut xtx = vec![vec![0.0; parameter_count]; parameter_count];
    let mut xty = vec![0.0; parameter_count];

    for (row, target) in design.iter().zip(targets.iter()) {
        for i in 0..parameter_count {
            xty[i] += row[i] * target;
            for j in 0..parameter_count {
                xtx[i][j] += row[i] * row[j];
            }
        }
    }

    solve_linear_system(xtx, xty)
}

fn residual_sum_squares(design: &[Vec<f64>], targets: &[f64], beta: &[f64]) -> f64 {
    design
        .iter()
        .zip(targets.iter())
        .map(|(row, target)| {
            let prediction = row.iter().zip(beta.iter()).map(|(x, b)| x * b).sum::<f64>();
            (target - prediction).powi(2)
        })
        .sum::<f64>()
}

fn solve_linear_system(mut a: Vec<Vec<f64>>, mut b: Vec<f64>) -> Option<Vec<f64>> {
    let n = b.len();
    for pivot in 0..n {
        let best_row = (pivot..n)
            .max_by(|left, right| {
                a[*left][pivot]
                    .abs()
                    .partial_cmp(&a[*right][pivot].abs())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })?;
        if a[best_row][pivot].abs() < 1e-12 {
            return None;
        }
        if best_row != pivot {
            a.swap(best_row, pivot);
            b.swap(best_row, pivot);
        }

        let pivot_value = a[pivot][pivot];
        for col in pivot..n {
            a[pivot][col] /= pivot_value;
        }
        b[pivot] /= pivot_value;

        for row in 0..n {
            if row == pivot {
                continue;
            }
            let factor = a[row][pivot];
            for col in pivot..n {
                a[row][col] -= factor * a[pivot][col];
            }
            b[row] -= factor * b[pivot];
        }
    }
    Some(b)
}

fn f_distribution_survival(f_stat: f64, df_num: f64, df_den: f64) -> f64 {
    if !f_stat.is_finite() || f_stat <= 0.0 || df_num <= 0.0 || df_den <= 0.0 {
        return 1.0;
    }
    let x = (df_num * f_stat) / (df_num * f_stat + df_den);
    (1.0 - regularized_incomplete_beta(df_num * 0.5, df_den * 0.5, x)).clamp(0.0, 1.0)
}

fn regularized_incomplete_beta(a: f64, b: f64, x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }

    let bt = (log_gamma(a + b) - log_gamma(a) - log_gamma(b)
        + a * x.ln()
        + b * (1.0 - x).ln())
        .exp();

    if x < (a + 1.0) / (a + b + 2.0) {
        bt * beta_continued_fraction(a, b, x) / a
    } else {
        1.0 - bt * beta_continued_fraction(b, a, 1.0 - x) / b
    }
}

fn beta_continued_fraction(a: f64, b: f64, x: f64) -> f64 {
    let max_iterations = 200;
    let epsilon = 3.0e-7;
    let fp_min = 1.0e-30;

    let qab = a + b;
    let qap = a + 1.0;
    let qam = a - 1.0;
    let mut c = 1.0;
    let mut d = 1.0 - qab * x / qap;
    if d.abs() < fp_min {
        d = fp_min;
    }
    d = 1.0 / d;
    let mut h = d;

    for m in 1..=max_iterations {
        let m_f = m as f64;
        let m2 = 2.0 * m_f;

        let aa = m_f * (b - m_f) * x / ((qam + m2) * (a + m2));
        d = 1.0 + aa * d;
        if d.abs() < fp_min {
            d = fp_min;
        }
        c = 1.0 + aa / c;
        if c.abs() < fp_min {
            c = fp_min;
        }
        d = 1.0 / d;
        h *= d * c;

        let aa = -(a + m_f) * (qab + m_f) * x / ((a + m2) * (qap + m2));
        d = 1.0 + aa * d;
        if d.abs() < fp_min {
            d = fp_min;
        }
        c = 1.0 + aa / c;
        if c.abs() < fp_min {
            c = fp_min;
        }
        d = 1.0 / d;
        let delta = d * c;
        h *= delta;

        if (delta - 1.0).abs() < epsilon {
            break;
        }
    }

    h
}

fn log_gamma(z: f64) -> f64 {
    let coefficients = [
        676.5203681218851,
        -1259.1392167224028,
        771.3234287776531,
        -176.6150291621406,
        12.507343278686905,
        -0.13857109526572012,
        9.984369578019572e-6,
        1.5056327351493116e-7,
    ];

    if z < 0.5 {
        return std::f64::consts::PI.ln()
            - (std::f64::consts::PI * z).sin().ln()
            - log_gamma(1.0 - z);
    }

    let z = z - 1.0;
    let mut x = 0.9999999999998099;
    for (index, coefficient) in coefficients.iter().enumerate() {
        x += coefficient / (z + index as f64 + 1.0);
    }
    let t = z + coefficients.len() as f64 - 0.5;
    0.5 * (2.0 * std::f64::consts::PI).ln() + (z + 0.5) * t.ln() - t + x.ln()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic_granger_pair() -> (Vec<f64>, Vec<f64>) {
        let n = 160;
        let mut source = vec![0.0; n];
        let mut target = vec![0.0; n];
        let mut seed = 17_u64;
        for idx in 1..n {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let source_noise = (((seed >> 16) % 2_000) as f64 / 1_000.0) - 1.0;
            let noise = (((idx * 19 + 7) % 11) as f64 - 5.0) / 80.0;
            source[idx] = source_noise;
            let lagged_source = if idx >= 7 { source[idx - 7] } else { 0.0 };
            target[idx] = 0.98 * lagged_source + noise;
        }
        (source, target)
    }

    #[test]
    fn granger_causality_known_lag() {
        let (source, target) = synthetic_granger_pair();
        let best = best_granger_lag(&source, &target, &[1, 7, 14, 30]).unwrap();

        assert_eq!(best.lag, 7);
        assert!(best.p_value < 0.05, "p={}", best.p_value);
    }

    #[test]
    fn predictive_signal_graph_detects_directed_edge() {
        let (source, target) = synthetic_granger_pair();
        let independent = (0..160)
            .map(|idx| (idx as f64 / 8.0).cos() * 0.1)
            .collect::<Vec<_>>();
        let mut signals = HashMap::new();
        signals.insert("commodity_spike".to_string(), source.clone());
        signals.insert("patent_filing".to_string(), target.clone());
        signals.insert("independent_noise".to_string(), independent);

        let edges = build_predictive_signal_graph(&signals, &[1, 7, 14, 30], 0.05);
        assert!(edges.iter().any(|edge| {
            edge.source_signal == "commodity_spike"
                && edge.target_signal == "patent_filing"
                && edge.lag == 7
        }));
        let alerts = predictive_alerts_for_signal("commodity_spike", &edges);
        assert!(alerts.iter().any(|alert| alert.contains("patent_filing")));
    }
}