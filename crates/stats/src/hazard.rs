/// Hazard rate estimation and survival analysis utilities.
///
/// All time values must be non-negative.  Functions that accept a time axis
/// silently filter out negative-time observations (Kaplan-Meier) or treat
/// negative interval widths as degenerate inputs (hazard rate).

use crate::utils::safe_div;

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
    let mut sorted: Vec<(f64, bool)> = events
        .iter()
        .filter(|(t, _)| *t >= 0.0)
        .cloned()
        .collect();

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
        let events = vec![
            (1.0, false),
            (2.0, false),
            (3.0, false),
        ];
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
        let curve = vec![
            (1.0, 0.9),
            (2.0, 0.7),
            (3.0, 0.4),
            (4.0, 0.2),
        ];
        let median = median_survival(&curve);
        assert!(median.is_some());
        let m = median.unwrap();
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
            (-1.0, true),  // invalid — negative time
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
        let s1 = km.iter().find(|(t, _)| (*t - 1.0).abs() < 1e-12).unwrap().1;
        assert!((s1 - 2.0 / 3.0).abs() < 1e-10, "S(1) should be 2/3 when negative obs dropped; got {s1}");
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
        assert!((median.unwrap() - 4.0).abs() < 1e-10, "median should be 4.0; got {:?}", median);
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
        assert!((median.unwrap() - 2.0).abs() < 1e-10, "median should be 2.0; got {:?}", median);
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
}
