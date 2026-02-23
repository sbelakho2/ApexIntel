/// Hazard rate estimation and survival analysis utilities.

/// Kaplan-Meier survival function.
///
/// Takes events as Vec<(time, is_event)> where is_event=true means the event occurred
/// (not censored). Returns Vec<(time, survival_probability)>.
pub fn kaplan_meier(events: &[(f64, bool)]) -> Vec<(f64, f64)> {
    if events.is_empty() {
        return vec![];
    }

    let mut sorted: Vec<(f64, bool)> = events.to_vec();
    sorted.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());

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
/// hazard = events / (at_risk * interval_width)
pub fn hazard_rate(events: usize, at_risk: usize, interval_width: f64) -> f64 {
    if at_risk == 0 || interval_width <= 0.0 {
        return 0.0;
    }
    events as f64 / (at_risk as f64 * interval_width)
}

/// Cumulative hazard from survival probabilities.
///
/// H(t) = -ln(S(t))
pub fn cumulative_hazard(survival_curve: &[(f64, f64)]) -> Vec<(f64, f64)> {
    survival_curve
        .iter()
        .map(|(t, s)| {
            let h = if *s > 0.0 { -s.ln() } else { f64::INFINITY };
            (*t, h)
        })
        .collect()
}

/// Estimate median survival time from a survival curve.
pub fn median_survival(survival_curve: &[(f64, f64)]) -> Option<f64> {
    for window in survival_curve.windows(2) {
        if window[0].1 >= 0.5 && window[1].1 < 0.5 {
            // Linear interpolation
            let t0 = window[0].0;
            let t1 = window[1].0;
            let s0 = window[0].1;
            let s1 = window[1].1;
            let frac = (s0 - 0.5) / (s0 - s1);
            return Some(t0 + frac * (t1 - t0));
        }
    }
    // If survival never drops below 50%
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
        assert!(m > 2.0 && m < 3.0);
    }

    #[test]
    fn test_median_survival_never_below_50() {
        let curve = vec![(1.0, 0.9), (2.0, 0.8), (3.0, 0.7)];
        let median = median_survival(&curve);
        assert!(median.is_none());
    }
}
