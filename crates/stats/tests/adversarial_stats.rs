//! Adversarial regression tests for the statistics layer.
//!
//! These deliberately feed hostile inputs (u64::MAX counts, usize::MAX bin
//! counts, NaN/Inf, inverted ranges) to public APIs and assert they stay
//! finite, in-range, and panic-free.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;

use apex_stats::bayesian::{fuse_signals, fuse_signals_detailed, BetaUpdater};
use apex_stats::calibration::{fit_platt_scaling, legacy_score_to_probability, CalibrationSample};
use apex_stats::changepoint::{detect_changepoints, PeltConfig};
use apex_stats::correlation::lagged_xcorr;
use apex_stats::fisher;
use apex_stats::graph_risk::contagion_score;
use apex_stats::mutual_info;

#[test]
fn fisher_survives_overflowing_tables() {
    // Previously: `a + b + c + d` overflowed u64 (debug panic) or wrapped.
    let p = fisher::p_value(u64::MAX, 1, 1, 1);
    assert!(p.is_finite() && (0.0..=1.0).contains(&p), "p={p}");
    assert!(fisher::p_value(u64::MAX, u64::MAX, u64::MAX, u64::MAX).is_finite());
    // A zero table is defined as 1.0.
    assert_eq!(fisher::p_value(0, 0, 0, 0), 1.0);
}

#[test]
fn fisher_odds_ratio_distinguishes_undefined_from_infinite() {
    // 0/0 is undefined, not infinitely strong evidence.
    assert!(fisher::odds_ratio(0, 0, 5, 5).is_nan());
    assert!(fisher::odds_ratio(5, 0, 3, 0).is_nan());
    // Positive numerator over zero denominator is genuinely +inf.
    assert!(fisher::odds_ratio(5, 0, 3, 4).is_infinite());
    // Overflowing totals yield None rather than panicking.
    assert!(fisher::woolf_odds_ratio_confidence_interval(u64::MAX, u64::MAX, 1, 1).is_none());
}

#[test]
fn mutual_info_caps_bins_and_never_panics() {
    let x: Vec<f64> = (0..200).map(|i| (i as f64 * 0.37).sin()).collect();
    let y: Vec<f64> = (0..200).map(|i| (i as f64 * 0.11).cos()).collect();
    // usize::MAX bins previously overflowed `bins * 2` and attempted a bins²
    // allocation.
    let mi = mutual_info::estimate(&x, &y, usize::MAX);
    assert!(mi.is_finite() && mi >= 0.0, "mi={mi}");
    // Degenerate/constant series must not blow up adaptive bin selection.
    let constant = vec![1.0; 500];
    assert!(mutual_info::estimate_adaptive(&constant, &constant).is_finite());
    // Extreme outlier skew.
    let mut skewed = x.clone();
    skewed[0] = 1e300;
    assert!(mutual_info::estimate_adaptive(&skewed, &y).is_finite());
}

#[test]
fn changepoint_rejects_invalid_configs_without_panicking() {
    let data: Vec<f64> = (0..60).map(|i| if i < 30 { 1.0 } else { 5.0 }).collect();

    let zero_candidates = PeltConfig {
        penalty: 3.0,
        adaptive_penalty: true,
        min_segment: 2,
        max_candidates: 0,
    };
    assert!(detect_changepoints(&data, &zero_candidates).is_empty());

    let huge_segment = PeltConfig {
        penalty: 3.0,
        adaptive_penalty: true,
        min_segment: usize::MAX,
        max_candidates: 64,
    };
    assert!(detect_changepoints(&data, &huge_segment).is_empty());

    // A valid config still finds the step.
    let valid = PeltConfig::default();
    assert!(!detect_changepoints(&data, &valid).is_empty());
}

#[test]
fn calibration_treats_non_finite_scores_conservatively() {
    // Unknown score must not map to the strongest alert probability.
    assert!(legacy_score_to_probability(f64::NAN) <= 0.5);
    assert!(legacy_score_to_probability(f64::INFINITY) <= 1.0);

    // A single NaN sample previously poisoned the fitted model.
    let samples = vec![
        CalibrationSample {
            raw_score: f64::NAN,
            actual_outcome: true,
        },
        CalibrationSample {
            raw_score: 1.0,
            actual_outcome: false,
        },
        CalibrationSample {
            raw_score: 2.0,
            actual_outcome: true,
        },
        CalibrationSample {
            raw_score: 3.0,
            actual_outcome: false,
        },
    ];
    if let Some(model) = fit_platt_scaling(&samples) {
        assert!(model.slope.is_finite() && model.intercept.is_finite());
    }
}

#[test]
fn lagged_xcorr_filters_nan_and_rejects_hostile_lags() {
    let x = vec![1.0, f64::NAN, 3.0, 4.0, 5.0, 6.0];
    let y = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
    let lag0 = lagged_xcorr(&x, &y, 0);
    assert_eq!(lag0.len(), 1);
    // The NaN pair is dropped rather than zeroing the whole lag.
    assert!(
        lag0[0].1.is_finite() && lag0[0].1 > 0.9,
        "r={:?}",
        lag0[0].1
    );

    // i32::MIN previously overflowed on negation; huge lags must not allocate.
    assert!(lagged_xcorr(&x, &y, i32::MIN).is_empty());
    assert!(lagged_xcorr(&x, &y, i32::MAX).is_empty());
}

#[test]
fn contagion_score_is_clamped_to_unit_interval() {
    let mut adjacency: HashMap<String, Vec<(String, f64)>> = HashMap::new();
    adjacency.insert("a".to_string(), vec![("b".to_string(), -1.0)]);
    let mut risk: HashMap<String, f64> = HashMap::new();
    risk.insert("a".to_string(), 0.9);
    let score = contagion_score(&adjacency, &risk, "b");
    assert!(
        (0.0..=1.0).contains(&score),
        "negative weight score={score}"
    );

    // NaN risk previously inflated to 1.0 via `NaN.min(1.0)`.
    risk.insert("a".to_string(), f64::NAN);
    let score = contagion_score(&adjacency, &risk, "b");
    assert!(
        (0.0..=1.0).contains(&score) && score.is_finite(),
        "nan score={score}"
    );
}

#[test]
fn bayesian_fusion_and_beta_updater_never_return_nan() {
    let fused = fuse_signals(f64::NAN, &[(0.9, 0.1)]);
    assert!(
        fused.is_finite() && (0.0..=1.0).contains(&fused),
        "fused={fused}"
    );

    let with_nan_likelihood = fuse_signals(0.5, &[(f64::NAN, 0.2), (0.8, 0.3)]);
    assert!(!with_nan_likelihood.is_nan());

    let detailed = fuse_signals_detailed(f64::NAN, &[(0.9, 0.1)]);
    assert!(detailed.posterior.is_finite());

    let mean = BetaUpdater::new(0.0, 0.0).mean();
    assert!(
        (0.0..=1.0).contains(&mean) && mean.is_finite(),
        "mean={mean}"
    );
    let var = BetaUpdater::new(f64::NAN, -1.0).variance();
    assert!(var.is_finite() && var >= 0.0, "var={var}");
}
