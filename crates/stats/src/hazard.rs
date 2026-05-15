//! Hazard rate estimation and survival analysis utilities.
//!
//! All time values must be non-negative.  Functions that accept a time axis
//! silently filter out negative-time observations (Kaplan-Meier) or treat
//! negative interval widths as degenerate inputs (hazard rate).
use crate::utils::safe_div;

const COX_MAX_ITERATIONS: usize = 50;
const COX_TOLERANCE: f64 = 1e-6;
const COX_REGULARIZATION: f64 = 1e-8;
const TIME_TIE_TOLERANCE: f64 = 1e-12;

#[derive(Debug, Clone, PartialEq)]
pub struct CoxObservation {
    pub time: f64,
    pub event: bool,
    pub covariates: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CoxPHModel {
    pub coefficients: Vec<f64>,
    pub hazard_ratios: Vec<f64>,
    pub standard_errors: Vec<f64>,
    pub z_scores: Vec<f64>,
    pub concordance_index: f64,
    pub log_partial_likelihood: f64,
    pub iterations: usize,
    pub converged: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LogRankResult {
    pub chi_square: f64,
    pub z_score: f64,
    pub p_value: f64,
    pub observed_minus_expected: f64,
    pub variance: f64,
}

/// Kaplan-Meier survival function.
///
/// Takes events as `Vec<(time, is_event)>` where `is_event = true` means the
/// event occurred (not censored).  Returns `Vec<(time, survival_probability)>`
/// in chronological order.
///
/// # Negative times
/// Observations with `time < 0.0` are silently discarded before estimation.
/// This guards against malformed inputs (e.g. negative duration from a clock
/// skew) without panicking; callers should validate data upstream.
pub fn kaplan_meier(events: &[(f64, bool)]) -> Vec<(f64, f64)> {
    if events.is_empty() {
        return vec![];
    }

    // B261: reject negative-time observations rather than silently including
    // them at the front of the survival curve where they corrupt at-risk counts.
    let mut sorted: Vec<(f64, bool)> = events.iter().filter(|(t, _)| *t >= 0.0).cloned().collect();

    // If all observations had negative times, return empty rather than panic.
    if sorted.is_empty() {
        return vec![];
    }

    sorted.sort_by(|a, b| a.0.total_cmp(&b.0));

    let mut result = Vec::new();
    let mut at_risk = sorted.len() as f64;
    let mut survival = 1.0;

    let mut i = 0;
    while i < sorted.len() {
        let time = sorted[i].0;
        let mut events_at_time = 0.0;
        let mut censored_at_time = 0.0;

        while i < sorted.len() && (sorted[i].0 - time).abs() < 1e-12 {
            if sorted[i].1 {
                events_at_time += 1.0;
            } else {
                censored_at_time += 1.0;
            }
            i += 1;
        }

        if events_at_time > 0.0 {
            survival *= 1.0 - events_at_time / at_risk;
        }
        result.push((time, survival));
        at_risk -= events_at_time + censored_at_time;
    }

    result
}

/// Simple hazard rate at a time point.
///
/// `hazard = events / (at_risk × interval_width)`
///
/// Returns `0.0` if `at_risk == 0` or `interval_width <= 0.0`.
pub fn hazard_rate(events: usize, at_risk: usize, interval_width: f64) -> f64 {
    if at_risk == 0 || interval_width <= 0.0 {
        return 0.0;
    }
    safe_div(events as f64, at_risk as f64 * interval_width)
}

/// Cumulative hazard from survival probabilities.
///
/// H(t) = −ln S(t).  Returns `f64::INFINITY` for time points where `S(t) = 0`.
pub fn cumulative_hazard(survival_curve: &[(f64, f64)]) -> Vec<(f64, f64)> {
    survival_curve
        .iter()
        .map(|(t, s)| {
            let h = if *s > 0.0 { -s.ln() } else { f64::INFINITY };
            (*t, h)
        })
        .collect()
}

/// Estimate median survival time from a Kaplan-Meier survival curve.
///
/// Returns the smallest time `t` where `S(t) ≤ 0.5`, consistent with the
/// standard KM step-function convention (closed interval on the right).
/// Returns `None` when the curve never drops to or below 50% — this indicates
/// that fewer than half the cohort experienced the event within the observation
/// window (right-censoring plateau above 50%).
pub fn median_survival(survival_curve: &[(f64, f64)]) -> Option<f64> {
    for &(t, s) in survival_curve {
        if s <= 0.5 {
            return Some(t);
        }
    }
    // Survival never drops to or below 50%
    None
}

pub fn fit_cox_ph(observations: &[CoxObservation]) -> Option<CoxPHModel> {
    let filtered = validated_cox_observations(observations)?;
    let covariate_count = filtered.first()?.covariates.len();
    if covariate_count == 0 || filtered.iter().filter(|obs| obs.event).count() == 0 {
        return None;
    }

    let mut coefficients = vec![0.0; covariate_count];
    let mut converged = false;
    let mut iterations = 0;
    let mut current_log_partial_likelihood =
        cox_score_information_loglik(&filtered, &coefficients).2;

    for iteration in 0..COX_MAX_ITERATIONS {
        let (score, information, _) = cox_score_information_loglik(&filtered, &coefficients);
        let delta = solve_linear_system(&regularized_matrix(&information), &score)?;
        let mut step_scale = 1.0;
        let mut accepted = None;

        while step_scale >= 1.0 / 1024.0 {
            let candidate_coefficients: Vec<f64> = coefficients
                .iter()
                .zip(delta.iter())
                .map(|(coefficient, update)| coefficient + step_scale * update)
                .collect();
            let candidate_log_partial_likelihood =
                cox_score_information_loglik(&filtered, &candidate_coefficients).2;
            if candidate_log_partial_likelihood.is_finite()
                && candidate_log_partial_likelihood >= current_log_partial_likelihood - 1e-10
            {
                accepted = Some((candidate_coefficients, candidate_log_partial_likelihood));
                break;
            }
            step_scale *= 0.5;
        }

        let Some((candidate_coefficients, candidate_log_partial_likelihood)) = accepted else {
            break;
        };

        let max_step = delta
            .iter()
            .map(|value| (step_scale * value).abs())
            .fold(0.0_f64, f64::max);
        coefficients = candidate_coefficients;
        current_log_partial_likelihood = candidate_log_partial_likelihood;
        iterations = iteration + 1;
        if max_step <= COX_TOLERANCE {
            converged = true;
            break;
        }
    }

    let (_, information, log_partial_likelihood) =
        cox_score_information_loglik(&filtered, &coefficients);
    let covariance = invert_matrix(&regularized_matrix(&information))?;
    let standard_errors: Vec<f64> = covariance
        .iter()
        .enumerate()
        .map(|(idx, row)| row.get(idx).copied().unwrap_or(0.0).max(0.0).sqrt())
        .collect();
    let z_scores: Vec<f64> = coefficients
        .iter()
        .zip(standard_errors.iter())
        .map(|(&beta, &se)| safe_div(beta, se))
        .collect();
    let hazard_ratios: Vec<f64> = coefficients.iter().map(|beta| beta.exp()).collect();
    let risk_scores: Vec<f64> = filtered
        .iter()
        .map(|obs| dot(&coefficients, &obs.covariates))
        .collect();

    Some(CoxPHModel {
        coefficients,
        hazard_ratios,
        standard_errors,
        z_scores,
        concordance_index: concordance_index(&filtered, &risk_scores),
        log_partial_likelihood,
        iterations,
        converged,
    })
}

pub fn log_rank_test(group_a: &[(f64, bool)], group_b: &[(f64, bool)]) -> Option<LogRankResult> {
    let a = validated_group_events(group_a);
    let b = validated_group_events(group_b);
    if a.is_empty() || b.is_empty() {
        return None;
    }

    let mut event_times: Vec<f64> = a
        .iter()
        .chain(b.iter())
        .filter_map(|(time, event)| if *event { Some(*time) } else { None })
        .collect();
    if event_times.is_empty() {
        return None;
    }

    event_times.sort_by(|left, right| left.total_cmp(right));
    event_times.dedup_by(|left, right| (*left - *right).abs() < TIME_TIE_TOLERANCE);

    let mut observed_minus_expected = 0.0;
    let mut variance = 0.0;

    for &time in &event_times {
        let n1 = a
            .iter()
            .filter(|(t, _)| *t >= time - TIME_TIE_TOLERANCE)
            .count() as f64;
        let n2 = b
            .iter()
            .filter(|(t, _)| *t >= time - TIME_TIE_TOLERANCE)
            .count() as f64;
        let d1 = a
            .iter()
            .filter(|(t, event)| *event && (*t - time).abs() < TIME_TIE_TOLERANCE)
            .count() as f64;
        let d2 = b
            .iter()
            .filter(|(t, event)| *event && (*t - time).abs() < TIME_TIE_TOLERANCE)
            .count() as f64;
        let n = n1 + n2;
        let d = d1 + d2;
        if n <= 1.0 || d <= 0.0 {
            continue;
        }

        let expected_group_a = d * safe_div(n1, n);
        let hypergeometric_variance = safe_div(n1 * n2 * d * (n - d), n * n * (n - 1.0));
        observed_minus_expected += d1 - expected_group_a;
        variance += hypergeometric_variance;
    }

    if variance <= 0.0 {
        return None;
    }

    let z_score = safe_div(observed_minus_expected, variance.sqrt());
    let chi_square = z_score * z_score;
    let p_value = 2.0 * (1.0 - normal_cdf(z_score.abs()));

    Some(LogRankResult {
        chi_square,
        z_score,
        p_value: p_value.clamp(0.0, 1.0),
        observed_minus_expected,
        variance,
    })
}

pub fn concordance_index(observations: &[CoxObservation], risk_scores: &[f64]) -> f64 {
    let filtered = validated_cox_observations(observations);
    let Some(filtered) = filtered else {
        return 0.5;
    };
    if filtered.len() != risk_scores.len() {
        return 0.5;
    }

    let mut comparable_pairs = 0.0;
    let mut concordant_pairs = 0.0;

    for i in 0..filtered.len() {
        for j in (i + 1)..filtered.len() {
            let left = &filtered[i];
            let right = &filtered[j];

            let ordering = if left.time + TIME_TIE_TOLERANCE < right.time && left.event {
                Some((i, j))
            } else if right.time + TIME_TIE_TOLERANCE < left.time && right.event {
                Some((j, i))
            } else {
                None
            };

            let Some((earlier_idx, later_idx)) = ordering else {
                continue;
            };

            comparable_pairs += 1.0;
            let earlier_score = risk_scores[earlier_idx];
            let later_score = risk_scores[later_idx];
            if earlier_score > later_score {
                concordant_pairs += 1.0;
            } else if (earlier_score - later_score).abs() < TIME_TIE_TOLERANCE {
                concordant_pairs += 0.5;
            }
        }
    }

    if comparable_pairs <= 0.0 {
        0.5
    } else {
        concordant_pairs / comparable_pairs
    }
}

fn validated_group_events(events: &[(f64, bool)]) -> Vec<(f64, bool)> {
    let mut filtered: Vec<(f64, bool)> = events
        .iter()
        .copied()
        .filter(|(time, _)| time.is_finite() && *time >= 0.0)
        .collect();
    filtered.sort_by(|left, right| left.0.total_cmp(&right.0));
    filtered
}

fn validated_cox_observations(observations: &[CoxObservation]) -> Option<Vec<CoxObservation>> {
    let covariate_count = observations.first()?.covariates.len();
    if covariate_count == 0 {
        return None;
    }

    let mut filtered = Vec::new();
    for observation in observations {
        if !observation.time.is_finite() || observation.time < 0.0 {
            continue;
        }
        if observation.covariates.len() != covariate_count {
            return None;
        }
        if observation
            .covariates
            .iter()
            .any(|value| !value.is_finite())
        {
            continue;
        }
        filtered.push(observation.clone());
    }

    if filtered.len() <= covariate_count {
        return None;
    }

    filtered.sort_by(|left, right| left.time.total_cmp(&right.time));
    Some(filtered)
}

#[allow(clippy::needless_range_loop)]
fn cox_score_information_loglik(
    observations: &[CoxObservation],
    coefficients: &[f64],
) -> (Vec<f64>, Vec<Vec<f64>>, f64) {
    let covariate_count = coefficients.len();
    let mut score = vec![0.0; covariate_count];
    let mut information = vec![vec![0.0; covariate_count]; covariate_count];
    let mut log_partial_likelihood = 0.0;
    let mut event_times: Vec<f64> = observations
        .iter()
        .filter_map(|obs| if obs.event { Some(obs.time) } else { None })
        .collect();

    event_times.sort_by(|left, right| left.total_cmp(right));
    event_times.dedup_by(|left, right| (*left - *right).abs() < TIME_TIE_TOLERANCE);

    for &time in &event_times {
        let mut risk_sum = 0.0;
        let mut weighted_covariates = vec![0.0; covariate_count];
        let mut weighted_outer = vec![vec![0.0; covariate_count]; covariate_count];
        let mut event_covariates = vec![0.0; covariate_count];
        let mut event_count = 0.0;
        let mut event_linear_predictor_sum = 0.0;

        for observation in observations {
            if observation.time + TIME_TIE_TOLERANCE < time {
                continue;
            }

            let linear_predictor = dot(coefficients, &observation.covariates).clamp(-30.0, 30.0);
            let weight = linear_predictor.exp();
            risk_sum += weight;
            for row in 0..covariate_count {
                weighted_covariates[row] += weight * observation.covariates[row];
                for col in 0..covariate_count {
                    weighted_outer[row][col] +=
                        weight * observation.covariates[row] * observation.covariates[col];
                }
            }

            if observation.event && (observation.time - time).abs() < TIME_TIE_TOLERANCE {
                event_count += 1.0;
                event_linear_predictor_sum += dot(coefficients, &observation.covariates);
                for (sum, value) in event_covariates
                    .iter_mut()
                    .zip(observation.covariates.iter())
                {
                    *sum += *value;
                }
            }
        }

        if event_count <= 0.0 || risk_sum <= 0.0 {
            continue;
        }

        for row in 0..covariate_count {
            score[row] +=
                event_covariates[row] - event_count * safe_div(weighted_covariates[row], risk_sum);
            for col in 0..covariate_count {
                let mean_cross = safe_div(weighted_outer[row][col], risk_sum);
                let mean_row = safe_div(weighted_covariates[row], risk_sum);
                let mean_col = safe_div(weighted_covariates[col], risk_sum);
                information[row][col] += event_count * (mean_cross - mean_row * mean_col);
            }
        }

        log_partial_likelihood += event_linear_predictor_sum - event_count * risk_sum.ln();
    }

    (score, information, log_partial_likelihood)
}

fn dot(left: &[f64], right: &[f64]) -> f64 {
    left.iter().zip(right.iter()).map(|(a, b)| a * b).sum()
}

fn regularized_matrix(matrix: &[Vec<f64>]) -> Vec<Vec<f64>> {
    let mut regularized = matrix.to_vec();
    for (idx, row) in regularized.iter_mut().enumerate() {
        if let Some(value) = row.get_mut(idx) {
            *value += COX_REGULARIZATION;
        }
    }
    regularized
}

#[allow(clippy::needless_range_loop)]
fn solve_linear_system(matrix: &[Vec<f64>], rhs: &[f64]) -> Option<Vec<f64>> {
    let n = matrix.len();
    if n == 0 || rhs.len() != n || matrix.iter().any(|row| row.len() != n) {
        return None;
    }

    let mut augmented: Vec<Vec<f64>> = matrix
        .iter()
        .zip(rhs.iter())
        .map(|(row, value)| {
            let mut augmented_row = row.clone();
            augmented_row.push(*value);
            augmented_row
        })
        .collect();

    for pivot_idx in 0..n {
        let pivot_row = (pivot_idx..n).max_by(|&left, &right| {
            augmented[left][pivot_idx]
                .abs()
                .total_cmp(&augmented[right][pivot_idx].abs())
        })?;
        if augmented[pivot_row][pivot_idx].abs() < COX_REGULARIZATION {
            return None;
        }
        if pivot_row != pivot_idx {
            augmented.swap(pivot_row, pivot_idx);
        }

        let pivot = augmented[pivot_idx][pivot_idx];
        for col in pivot_idx..=n {
            augmented[pivot_idx][col] /= pivot;
        }

        for row in 0..n {
            if row == pivot_idx {
                continue;
            }
            let factor = augmented[row][pivot_idx];
            if factor.abs() < COX_REGULARIZATION {
                continue;
            }
            for col in pivot_idx..=n {
                augmented[row][col] -= factor * augmented[pivot_idx][col];
            }
        }
    }

    Some(augmented.iter().map(|row| row[n]).collect())
}

#[allow(clippy::needless_range_loop)]
fn invert_matrix(matrix: &[Vec<f64>]) -> Option<Vec<Vec<f64>>> {
    let n = matrix.len();
    if n == 0 || matrix.iter().any(|row| row.len() != n) {
        return None;
    }

    let mut inverse = vec![vec![0.0; n]; n];
    for column in 0..n {
        let mut basis = vec![0.0; n];
        basis[column] = 1.0;
        let solution = solve_linear_system(matrix, &basis)?;
        for row in 0..n {
            inverse[row][column] = solution[row];
        }
    }
    Some(inverse)
}

fn normal_cdf(value: f64) -> f64 {
    0.5 * (1.0 + erf_approx(value / std::f64::consts::SQRT_2))
}

fn erf_approx(value: f64) -> f64 {
    let sign = if value < 0.0 { -1.0 } else { 1.0 };
    let x = value.abs();
    let t = 1.0 / (1.0 + 0.3275911 * x);
    let y = 1.0
        - (((((1.061405429 * t - 1.453152027) * t) + 1.421413741) * t - 0.284496736) * t
            + 0.254829592)
            * t
            * (-x * x).exp();
    sign * y
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kaplan_meier_basic() {
        let events = vec![
            (1.0, true),
            (2.0, true),
            (3.0, false), // censored
            (4.0, true),
            (5.0, true),
        ];
        let km = kaplan_meier(&events);
        assert_eq!(km.len(), 5);
        // First event: 1 - 1/5 = 0.8
        assert!((km[0].1 - 0.8).abs() < 1e-10);
        // Survival should be monotonically decreasing
        for w in km.windows(2) {
            assert!(w[0].1 >= w[1].1);
        }
    }

    #[test]
    fn test_kaplan_meier_all_censored() {
        let events = vec![(1.0, false), (2.0, false), (3.0, false)];
        let km = kaplan_meier(&events);
        // No events → survival stays at 1.0
        for (_, s) in &km {
            assert!((*s - 1.0).abs() < 1e-10);
        }
    }

    #[test]
    fn test_hazard_functions_all_censored_events() {
        let events = vec![(1.0, false), (2.0, false), (3.0, false), (4.0, false)];
        let km = kaplan_meier(&events);
        assert!(!km.is_empty());
        assert!(km.iter().all(|(_, s)| (*s - 1.0).abs() < 1e-10));

        let cumulative = cumulative_hazard(&km);
        assert!(cumulative.iter().all(|(_, h)| h.abs() < 1e-10));
    }

    #[test]
    fn test_kaplan_meier_empty() {
        let km = kaplan_meier(&[]);
        assert!(km.is_empty());
    }

    #[test]
    fn test_hazard_rate_basic() {
        let h = hazard_rate(5, 100, 1.0);
        assert!((h - 0.05).abs() < 1e-10);
    }

    #[test]
    fn test_hazard_rate_zero_risk() {
        let h = hazard_rate(5, 0, 1.0);
        assert!((h - 0.0).abs() < 1e-10);
    }

    #[test]
    fn test_cumulative_hazard() {
        let curve = vec![(1.0, 0.8), (2.0, 0.5), (3.0, 0.2)];
        let ch = cumulative_hazard(&curve);
        assert_eq!(ch.len(), 3);
        // H(t) at S=0.5 should be ln(2) ≈ 0.693
        assert!((ch[1].1 - 0.693).abs() < 0.01);
    }

    #[test]
    fn test_median_survival() {
        let curve = vec![(1.0, 0.9), (2.0, 0.7), (3.0, 0.4), (4.0, 0.2)];
        let median = median_survival(&curve);
        assert!(median.is_some());
        let m = median.unwrap_or_else(|| panic!("median should exist"));
        // KM step function: median = smallest t where S(t) <= 0.5.
        // S(2.0)=0.7 > 0.5, S(3.0)=0.4 <= 0.5 → median = 3.0.
        assert!((m - 3.0).abs() < 1e-10, "expected 3.0, got {}", m);
    }

    #[test]
    fn test_median_survival_never_below_50() {
        let curve = vec![(1.0, 0.9), (2.0, 0.8), (3.0, 0.7)];
        let median = median_survival(&curve);
        assert!(median.is_none());
    }

    #[test]
    fn test_median_survival_no_events_all_censored_is_none() {
        let events = vec![(1.0, false), (2.0, false), (3.0, false)];
        let km = kaplan_meier(&events);
        let median = median_survival(&km);
        assert!(median.is_none());
    }

    // ── B261: kaplan_meier rejects negative times ───────────────────────────

    #[test]
    fn test_kaplan_meier_negative_times_discarded() {
        // Negative-time observations must not corrupt at-risk counts or
        // appear in the output curve.
        let events = vec![
            (-1.0, true), // invalid — negative time
            (1.0, true),
            (2.0, false),
            (3.0, true),
        ];
        let km = kaplan_meier(&events);
        // Only 3 non-negative observations should produce output
        assert_eq!(km.len(), 3, "negative-time obs should be dropped");
        // No output entry should have a negative time
        for (t, _) in &km {
            assert!(*t >= 0.0, "negative time {t} in KM output");
        }
        // At-risk count at t=1 should be 3 (not 4), so S(1) = 1 - 1/3 ≈ 0.667
        let s1 = km
            .iter()
            .find(|(time, _)| (*time - 1.0).abs() < 1e-12)
            .map(|(_, survival)| *survival)
            .unwrap_or_else(|| panic!("KM output should contain t=1"));
        assert!(
            (s1 - 2.0 / 3.0).abs() < 1e-10,
            "S(1) should be 2/3 when negative obs dropped; got {s1}"
        );
    }

    #[test]
    fn test_kaplan_meier_all_negative_returns_empty() {
        let events = vec![(-3.0, true), (-1.0, false)];
        let km = kaplan_meier(&events);
        assert!(km.is_empty(), "all-negative times should yield empty curve");
    }

    #[test]
    fn test_kaplan_meier_mixed_negative_and_zero() {
        let events = vec![(-1.0, true), (0.0, true), (1.0, false)];
        let km = kaplan_meier(&events);
        // t=0.0 is valid (≥ 0.0), t=-1.0 must be dropped
        assert_eq!(km.len(), 2);
        assert!((km[0].0 - 0.0).abs() < 1e-12, "first time should be 0.0");
    }

    #[test]
    fn test_kaplan_meier_unsorted_decreasing_times() {
        let events = vec![(5.0, true), (3.0, false), (1.0, true)];
        let km = kaplan_meier(&events);
        assert_eq!(km.len(), 3);
        assert!(km[0].0 <= km[1].0 && km[1].0 <= km[2].0);
    }

    #[test]
    fn test_hazard_rate_zero_interval() {
        let h = hazard_rate(3, 10, 0.0);
        assert!((h - 0.0).abs() < 1e-12);
    }

    #[test]
    fn test_cumulative_hazard_zero_survival_is_infinite() {
        let curve = vec![(1.0, 0.8), (2.0, 0.0)];
        let ch = cumulative_hazard(&curve);
        assert!((ch[0].1 - 0.223143551).abs() < 1e-6);
        assert!(ch[1].1.is_infinite());
    }

    // ── B262: median_survival with survival plateaus ────────────────────────

    #[test]
    fn test_median_survival_plateau_above_50() {
        // S(t) stays above 0.5 for multiple steps, then drops abruptly past it
        let curve = vec![
            (1.0, 0.9),
            (2.0, 0.9), // plateau at 0.9
            (3.0, 0.9), // plateau at 0.9
            (4.0, 0.3), // sudden drop below 0.5
        ];
        let median = median_survival(&curve);
        assert!(median.is_some());
        assert!(
            matches!(median, Some(value) if (value - 4.0).abs() < 1e-10),
            "median should be 4.0; got {:?}",
            median
        );
    }

    #[test]
    fn test_median_survival_plateau_exactly_at_50() {
        // S(t) reaches exactly 0.5 — the standard KM convention accepts S ≤ 0.5
        let curve = vec![
            (1.0, 0.8),
            (2.0, 0.5), // exactly 0.5 → this is the median
            (3.0, 0.3),
        ];
        let median = median_survival(&curve);
        assert!(median.is_some());
        assert!(
            matches!(median, Some(value) if (value - 2.0).abs() < 1e-10),
            "median should be 2.0; got {:?}",
            median
        );
    }

    #[test]
    fn test_median_survival_empty_curve() {
        // Empty input → None (no observations)
        assert!(median_survival(&[]).is_none());
    }

    #[test]
    fn test_median_survival_plateau_never_crosses_50() {
        // Long plateau above 0.5 with no subsequent drop → None
        let curve = vec![(1.0, 0.8), (2.0, 0.8), (3.0, 0.8), (4.0, 0.8)];
        assert!(median_survival(&curve).is_none());
    }

    #[test]
    fn test_log_rank_detects_group_difference() {
        let fast_fail = vec![
            (1.0, true),
            (1.2, true),
            (1.4, true),
            (1.6, true),
            (1.8, true),
        ];
        let slow_fail = vec![
            (5.0, true),
            (6.0, true),
            (7.0, true),
            (8.0, true),
            (9.0, true),
        ];
        let result = log_rank_test(&fast_fail, &slow_fail)
            .unwrap_or_else(|| panic!("log-rank result should exist"));
        assert!(
            result.chi_square > 5.0,
            "expected separation, got {result:?}"
        );
        assert!(
            result.p_value < 0.05,
            "expected directional evidence, got {result:?}"
        );
        assert!(result.observed_minus_expected > 0.0);
    }

    #[test]
    fn test_log_rank_identical_groups_are_not_significant() {
        let group_a = vec![(1.0, true), (3.0, false), (4.0, true), (6.0, false)];
        let group_b = group_a.clone();
        let result = log_rank_test(&group_a, &group_b)
            .unwrap_or_else(|| panic!("log-rank result should exist"));
        assert!(result.chi_square < 1e-6);
        assert!(result.p_value > 0.95);
    }

    #[test]
    fn test_concordance_index_half_credit_for_ties() {
        let observations = vec![
            CoxObservation {
                time: 1.0,
                event: true,
                covariates: vec![1.0],
            },
            CoxObservation {
                time: 2.0,
                event: true,
                covariates: vec![1.0],
            },
            CoxObservation {
                time: 3.0,
                event: false,
                covariates: vec![0.0],
            },
        ];
        let c_index = concordance_index(&observations, &[0.8, 0.8, 0.2]);
        assert!(
            (c_index - (5.0 / 6.0)).abs() < 1e-10,
            "expected half-credit for one tie"
        );
    }

    #[test]
    fn test_cox_ph_concordance() {
        let observations = vec![
            CoxObservation {
                time: 1.0,
                event: true,
                covariates: vec![2.0, 1.0],
            },
            CoxObservation {
                time: 1.4,
                event: true,
                covariates: vec![1.8, 1.0],
            },
            CoxObservation {
                time: 2.2,
                event: true,
                covariates: vec![1.7, 0.0],
            },
            CoxObservation {
                time: 2.8,
                event: true,
                covariates: vec![1.4, 0.0],
            },
            CoxObservation {
                time: 3.4,
                event: false,
                covariates: vec![1.2, 0.0],
            },
            CoxObservation {
                time: 4.5,
                event: true,
                covariates: vec![1.1, 0.0],
            },
            CoxObservation {
                time: 5.5,
                event: false,
                covariates: vec![0.8, 0.0],
            },
            CoxObservation {
                time: 6.5,
                event: false,
                covariates: vec![0.4, 0.0],
            },
            CoxObservation {
                time: 7.5,
                event: false,
                covariates: vec![0.2, 0.0],
            },
        ];

        let model = fit_cox_ph(&observations)
            .unwrap_or_else(|| panic!("cox fit should converge for synthetic observations"));
        assert!(
            model.coefficients[0] > 0.0,
            "higher risk covariate should increase hazard"
        );
        assert!(model.hazard_ratios[0] > 1.0);
        assert!(model.iterations > 0);
        assert!(
            model.concordance_index > 0.6,
            "expected useful discrimination: {model:?}"
        );
    }
}
