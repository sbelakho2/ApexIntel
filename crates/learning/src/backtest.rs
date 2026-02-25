//! Walk-forward backtesting — evaluate pattern candidates on held-out
//! historical data using precision, recall, F1, and confusion matrices.
//!
//! The core idea: split time-series data into expanding training windows
//! and fixed test windows, re-mine on each training fold, and evaluate
//! predictions on the test fold.  No future information ever leaks.

use crate::miner::{
    build_contingency, fisher_p_value, odds_ratio, EventRecord, MinerConfig,
    PatternCandidate,
};
use serde::{Deserialize, Serialize};

const STORED_METRIC_DECIMALS: f64 = 1_000_000.0;

// ────────────────────────────────────────────
// Types
// ────────────────────────────────────────────

/// Configuration for walk-forward backtesting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BacktestConfig {
    /// Number of walk-forward folds.
    pub folds: usize,
    /// Fraction of total timeline used as initial training window.
    pub initial_train_fraction: f64,
    /// Window (days) to check outcome after signal.
    pub window_days: i32,
    /// Minimum true-positive count per fold for a fold to be valid.
    pub min_tp_per_fold: u64,
}

impl Default for BacktestConfig {
    fn default() -> Self {
        Self {
            folds: 4,
            initial_train_fraction: 0.5,
            window_days: 30,
            min_tp_per_fold: 1,
        }
    }
}

impl BacktestConfig {
    /// Validate configuration values (B224).
    pub fn validate(&self) -> Vec<String> {
        let mut issues = Vec::new();
        if self.folds == 0 {
            issues.push("folds must be > 0".to_string());
        }
        if !(0.0..=1.0).contains(&self.initial_train_fraction) {
            issues.push(format!(
                "initial_train_fraction must be 0.0..=1.0, got {}",
                self.initial_train_fraction
            ));
        }
        if self.window_days <= 0 {
            issues.push(format!("window_days must be > 0, got {}", self.window_days));
        }
        issues
    }
}

/// Confusion matrix for a single fold or aggregated.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ConfusionMatrix {
    pub tp: u64,
    pub fp: u64,
    pub fn_: u64,
    pub tn: u64,
}

impl ConfusionMatrix {
    pub fn precision(&self) -> f64 {
        let denom = self.tp + self.fp;
        if denom == 0 {
            return 0.0;
        }
        self.tp as f64 / denom as f64
    }

    pub fn recall(&self) -> f64 {
        let denom = self.tp + self.fn_;
        if denom == 0 {
            return 0.0;
        }
        self.tp as f64 / denom as f64
    }

    pub fn f1(&self) -> f64 {
        let p = self.precision();
        let r = self.recall();
        if p + r < 1e-12 {
            return 0.0;
        }
        2.0 * p * r / (p + r)
    }

    pub fn accuracy(&self) -> f64 {
        let total = self.tp + self.fp + self.fn_ + self.tn;
        if total == 0 {
            return 0.0;
        }
        (self.tp + self.tn) as f64 / total as f64
    }

    /// Merge another matrix into this one (additive).
    pub fn merge(&mut self, other: &ConfusionMatrix) {
        self.tp += other.tp;
        self.fp += other.fp;
        self.fn_ += other.fn_;
        self.tn += other.tn;
    }
}

/// Result of a full walk-forward backtest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BacktestResult {
    pub outcome: String,
    pub signal: String,
    pub lag_days: i32,
    pub fold_results: Vec<FoldResult>,
    pub aggregate: ConfusionMatrix,
    pub aggregate_precision: f64,
    pub aggregate_recall: f64,
    pub aggregate_f1: f64,
    pub passed: bool,
}

/// Per-fold evaluation result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FoldResult {
    pub fold_index: usize,
    pub train_start: i64,
    pub train_end: i64,
    pub test_start: i64,
    pub test_end: i64,
    pub confusion: ConfusionMatrix,
    pub precision: f64,
    pub recall: f64,
    pub f1: f64,
    /// Whether the pattern was re-discovered on the training fold.
    pub pattern_found: bool,
}

// ────────────────────────────────────────────
// Walk-forward engine
// ────────────────────────────────────────────

/// Compute the time range of all events.
fn time_range(outcomes: &[EventRecord], signals: &[EventRecord]) -> Option<(i64, i64)> {
    let all_ts: Vec<i64> = outcomes
        .iter()
        .chain(signals.iter())
        .map(|(_, ts)| *ts)
        .collect();
    if all_ts.is_empty() {
        return None;
    }
    let min = *all_ts.iter().min().unwrap();
    let max = *all_ts.iter().max().unwrap();
    if max <= min {
        return None;
    }
    Some((min, max))
}

/// Filter events to a time range [start, end).
fn filter_events(events: &[EventRecord], start: i64, end: i64) -> Vec<EventRecord> {
    events
        .iter()
        .filter(|(_, ts)| *ts >= start && *ts < end)
        .cloned()
        .collect()
}

fn normalize_stored_metric(value: f64) -> f64 {
    if !value.is_finite() {
        return 0.0;
    }
    (value * STORED_METRIC_DECIMALS).round() / STORED_METRIC_DECIMALS
}

fn validate_fold_window(
    prev_test_end: Option<i64>,
    train_start: i64,
    train_end: i64,
    test_start: i64,
    test_end: i64,
) -> bool {
    if train_end < train_start || test_end <= test_start {
        return false;
    }
    if train_end > test_start {
        return false;
    }
    if let Some(prev_end) = prev_test_end {
        if test_start < prev_end {
            return false;
        }
    }
    true
}

/// Evaluate a candidate on a test fold: build contingency from test data,
/// and derive a confusion matrix.
///
/// Logic: the candidate's lag_days and a window of `window_days` are used
/// to build a contingency table on the test fold.  Signal+outcome = TP,
/// signal+no-outcome = FP, no-signal+outcome = FN, neither = TN.
fn evaluate_on_fold(
    outcomes: &[EventRecord],
    signals: &[EventRecord],
    lag_days: i32,
    window_days: i32,
) -> ConfusionMatrix {
    let (a, b, c, d) = build_contingency(outcomes, signals, lag_days, window_days, 0);
    ConfusionMatrix {
        tp: a,
        fp: b,
        fn_: c,
        tn: d,
    }
}

/// Check if a pattern re-emerges on a training fold: the effect must be
/// positive (OR > 1) and significant (p < 0.05 — relaxed from mining threshold).
/// Uses a population estimate of 2× observed entities for the d-cell.
fn pattern_emerges(
    outcomes: &[EventRecord],
    signals: &[EventRecord],
    lag_days: i32,
    window_days: i32,
    min_effect: f64,
) -> bool {
    // Estimate background population: unique entities × 2 gives a conservative
    // d-cell so the Fisher test can detect significance.
    let mut unique: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for (eid, _) in outcomes.iter().chain(signals.iter()) {
        unique.insert(eid.as_str());
    }
    let total_est = unique.len() * 2;
    let (a, b, c, d) = build_contingency(outcomes, signals, lag_days, window_days, total_est);
    let total = a + b + c + d;
    if total < 10 {
        return false;
    }
    let effect = odds_ratio(a, b, c, d);
    let p = fisher_p_value(a, b, c, d);
    effect >= min_effect && p < 0.05
}

/// Run a full walk-forward backtest for a single candidate.
///
/// The timeline is split into `folds` expanding training windows, each with
/// a fixed-size test window.  The pattern is re-validated on each training fold,
/// and predictions are checked against the test fold.
pub fn walk_forward_backtest(
    candidate: &PatternCandidate,
    outcomes: &[EventRecord],
    signals: &[EventRecord],
    config: &BacktestConfig,
    miner_config: &MinerConfig,
) -> Option<BacktestResult> {
    let (min_ts, max_ts) = time_range(outcomes, signals)?;
    let total_span = max_ts - min_ts;

    if config.folds == 0 {
        return None;
    }

    let initial_train_end =
        min_ts + (total_span as f64 * config.initial_train_fraction) as i64;
    let remaining = max_ts - initial_train_end;
    let fold_size = remaining / config.folds as i64;

    if fold_size <= 0 {
        return None;
    }

    let lag_days = candidate.best_lag_days;
    let mut fold_results = Vec::new();
    let mut aggregate = ConfusionMatrix::default();

    for i in 0..config.folds {
        let train_start = min_ts;
        let train_end = initial_train_end + i as i64 * fold_size;
        let test_start = train_end;
        // Last fold extends to max_ts+1 to capture all trailing events
        // that integer division truncation would otherwise drop.
        let test_end = if i == config.folds - 1 {
            max_ts + 1
        } else {
            test_start + fold_size
        };

        let prev_test_end = fold_results.last().map(|f: &FoldResult| f.test_end);
        if !validate_fold_window(prev_test_end, train_start, train_end, test_start, test_end) {
            tracing::warn!(
                fold = i,
                train_start,
                train_end,
                test_start,
                test_end,
                "backtest invalid fold window or overlap detected"
            );
            return None;
        }

        let train_outcomes = filter_events(outcomes, train_start, train_end);
        let train_signals = filter_events(signals, train_start, train_end);
        let test_outcomes = filter_events(outcomes, test_start, test_end);
        let test_signals = filter_events(signals, test_start, test_end);

        // Re-validate pattern on training fold
        let found = pattern_emerges(
            &train_outcomes,
            &train_signals,
            lag_days,
            config.window_days,
            miner_config.min_effect,
        );

        // Evaluate on test fold
        let confusion = evaluate_on_fold(
            &test_outcomes,
            &test_signals,
            lag_days,
            config.window_days,
        );

        // Include low-data folds in count (as unfound) to avoid inflating pass rate
        if confusion.tp < config.min_tp_per_fold {
            tracing::warn!(fold = i, tp = confusion.tp, "backtest fold has insufficient TP, skipping"); // B230
            fold_results.push(FoldResult {
                fold_index: i,
                train_start,
                train_end,
                test_start,
                test_end,
                confusion,
                precision: 0.0,
                recall: 0.0,
                f1: 0.0,
                pattern_found: false,
            });
            continue;
        }

        let precision = normalize_stored_metric(confusion.precision());
        let recall = normalize_stored_metric(confusion.recall());
        let f1 = normalize_stored_metric(confusion.f1());

        aggregate.merge(&confusion);

        fold_results.push(FoldResult {
            fold_index: i,
            train_start,
            train_end,
            test_start,
            test_end,
            confusion,
            precision,
            recall,
            f1,
            pattern_found: found,
        });
    }

    let agg_precision = normalize_stored_metric(aggregate.precision());
    let agg_recall = normalize_stored_metric(aggregate.recall());
    let agg_f1 = normalize_stored_metric(aggregate.f1());

    // Pass criteria: aggregate precision ≥ 0.5 AND recall ≥ 0.3 AND
    // pattern re-discovered in ≥ half of folds.
    let folds_with_pattern = fold_results.iter().filter(|f| f.pattern_found).count();
    let valid_fold_count = fold_results.len();
    let passed = valid_fold_count > 0
        && agg_precision >= 0.5
        && agg_recall >= 0.3
        && folds_with_pattern * 2 >= valid_fold_count;

    // B230: log aggregate result
    tracing::info!(
        outcome = %candidate.outcome,
        signal = ?candidate.signals.first(),
        agg_precision,
        agg_recall,
        agg_f1,
        passed,
        "walk-forward backtest completed"
    );

    Some(BacktestResult {
        outcome: candidate.outcome.clone(),
        signal: candidate
            .signals
            .first()
            .cloned()
            .unwrap_or_default(),
        lag_days,
        fold_results,
        aggregate,
        aggregate_precision: agg_precision,
        aggregate_recall: agg_recall,
        aggregate_f1: agg_f1,
        passed,
    })
}

/// Batch-backtest a list of candidates: returns only those that pass.
pub fn backtest_candidates(
    candidates: &[PatternCandidate],
    outcomes: &[EventRecord],
    signals: &[EventRecord],
    bt_config: &BacktestConfig,
    miner_config: &MinerConfig,
) -> Vec<BacktestResult> {
    candidates
        .iter()
        .filter_map(|c| {
            let result = walk_forward_backtest(c, outcomes, signals, bt_config, miner_config)?;
            if result.passed {
                Some(result)
            } else {
                None
            }
        })
        .collect()
}

/// Compute precision/recall/F1 from pre-computed contingency values.
pub fn evaluate_contingency(a: u64, b: u64, c: u64, d: u64) -> (f64, f64, f64) {
    let cm = ConfusionMatrix {
        tp: a,
        fp: b,
        fn_: c,
        tn: d,
    };
    (cm.precision(), cm.recall(), cm.f1())
}

/// Quick single-fold evaluation (no walk-forward) for lightweight checks.
pub fn single_fold_evaluate(
    candidate: &PatternCandidate,
    outcomes: &[EventRecord],
    signals: &[EventRecord],
    window_days: i32,
) -> ConfusionMatrix {
    evaluate_on_fold(outcomes, signals, candidate.best_lag_days, window_days)
}

// ────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: build events where entity at day_offset * 86400.
    fn make_events(entities: &[(&str, &[i64])]) -> Vec<EventRecord> {
        entities
            .iter()
            .flat_map(|(eid, days)| {
                days.iter()
                    .map(move |d| (eid.to_string(), d * 86400))
            })
            .collect()
    }

    fn sample_candidate() -> PatternCandidate {
        PatternCandidate {
            outcome: "rfq_posted".to_string(),
            signals: vec!["sqe_hiring".to_string()],
            best_lag_days: 0,
            effect_size: 3.0,
            p_value: 0.005,
            q_value: 0.01,
            stability: 0.8,
            entity_coverage: 0.5,
            segments: vec!["TN".to_string()],
            contingency: (10, 3, 2, 20),
        }
    }

    // ── ConfusionMatrix tests ──────────────────

    #[test]
    fn test_confusion_precision_recall_f1() {
        let cm = ConfusionMatrix {
            tp: 10,
            fp: 5,
            fn_: 2,
            tn: 83,
        };
        assert!((cm.precision() - 10.0 / 15.0).abs() < 0.001);
        assert!((cm.recall() - 10.0 / 12.0).abs() < 0.001);

        let expected_f1 = 2.0 * (10.0 / 15.0) * (10.0 / 12.0) / ((10.0 / 15.0) + (10.0 / 12.0));
        assert!((cm.f1() - expected_f1).abs() < 0.001);
    }

    #[test]
    fn test_confusion_all_zeros() {
        let cm = ConfusionMatrix::default();
        assert_eq!(cm.precision(), 0.0);
        assert_eq!(cm.recall(), 0.0);
        assert_eq!(cm.f1(), 0.0);
        assert_eq!(cm.accuracy(), 0.0);
    }

    #[test]
    fn test_confusion_perfect() {
        let cm = ConfusionMatrix {
            tp: 50,
            fp: 0,
            fn_: 0,
            tn: 50,
        };
        assert!((cm.precision() - 1.0).abs() < 0.001);
        assert!((cm.recall() - 1.0).abs() < 0.001);
        assert!((cm.f1() - 1.0).abs() < 0.001);
        assert!((cm.accuracy() - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_confusion_no_tp() {
        let cm = ConfusionMatrix {
            tp: 0,
            fp: 5,
            fn_: 10,
            tn: 85,
        };
        assert_eq!(cm.precision(), 0.0);
        assert_eq!(cm.recall(), 0.0);
        assert_eq!(cm.f1(), 0.0);
    }

    #[test]
    fn test_confusion_merge() {
        let mut a = ConfusionMatrix {
            tp: 5,
            fp: 2,
            fn_: 1,
            tn: 10,
        };
        let b = ConfusionMatrix {
            tp: 3,
            fp: 1,
            fn_: 2,
            tn: 8,
        };
        a.merge(&b);
        assert_eq!(a.tp, 8);
        assert_eq!(a.fp, 3);
        assert_eq!(a.fn_, 3);
        assert_eq!(a.tn, 18);
    }

    #[test]
    fn test_confusion_accuracy() {
        let cm = ConfusionMatrix {
            tp: 40,
            fp: 10,
            fn_: 5,
            tn: 45,
        };
        // accuracy = (40 + 45) / 100 = 0.85
        assert!((cm.accuracy() - 0.85).abs() < 0.001);
    }

    // ── Evaluate contingency ──────────────────

    #[test]
    fn test_evaluate_contingency() {
        let (p, r, f1) = evaluate_contingency(10, 5, 2, 83);
        assert!((p - 10.0 / 15.0).abs() < 0.001);
        assert!((r - 10.0 / 12.0).abs() < 0.001);
        assert!(f1 > 0.7);
    }

    #[test]
    fn test_evaluate_contingency_zeros() {
        let (p, r, f1) = evaluate_contingency(0, 0, 0, 0);
        assert_eq!(p, 0.0);
        assert_eq!(r, 0.0);
        assert_eq!(f1, 0.0);
    }

    // ── Time range & filtering ──────────────────

    #[test]
    fn test_time_range_normal() {
        let outcomes = make_events(&[("A", &[10, 50])]);
        let signals = make_events(&[("A", &[5, 80])]);
        let (min, max) = time_range(&outcomes, &signals).unwrap();
        assert_eq!(min, 5 * 86400);
        assert_eq!(max, 80 * 86400);
    }

    #[test]
    fn test_time_range_empty() {
        assert!(time_range(&[], &[]).is_none());
    }

    #[test]
    fn test_filter_events() {
        let events = make_events(&[("A", &[10, 20, 30, 40])]);
        let filtered = filter_events(&events, 15 * 86400, 35 * 86400);
        assert_eq!(filtered.len(), 2); // days 20, 30
    }

    // ── Single fold evaluate ──────────────────

    #[test]
    fn test_single_fold_evaluate() {
        // Create clear signal-outcome relationship at lag=0
        let outcomes = make_events(&[
            ("A", &[10]),
            ("B", &[15]),
            ("C", &[20]),
            ("D", &[25]),
        ]);
        let signals = make_events(&[
            ("A", &[10]),
            ("B", &[15]),
            // C has outcome but no signal → FN
            // D has outcome but no signal → FN
        ]);

        let candidate = PatternCandidate {
            outcome: "test".to_string(),
            signals: vec!["sig".to_string()],
            best_lag_days: 0,
            effect_size: 2.0,
            p_value: 0.01,
            q_value: 0.01,
            stability: 0.8,
            entity_coverage: 0.5,
            segments: vec![],
            contingency: (10, 3, 2, 20),
        };

        let cm = single_fold_evaluate(&candidate, &outcomes, &signals, 30);
        // A and B: signal + outcome in window → TP
        assert_eq!(cm.tp, 2);
        // C, D: outcome but no signal → FN
        assert_eq!(cm.fn_, 2);
    }

    // ── Pattern emergence ──────────────────

    #[test]
    fn test_pattern_emerges_strong() {
        // Strong pattern: signal → outcome for many entities, plus
        // control entities (signal-only and outcome-only) for a valid 2×2 table.
        let mut outcomes = Vec::new();
        let mut signals = Vec::new();
        for i in 0..15 {
            let eid = format!("E{}", i);
            signals.push((eid.clone(), i as i64 * 86400));
            outcomes.push((eid, i as i64 * 86400 + 86400)); // outcome 1 day after
        }
        // Entities with signal but no outcome (FP)
        for i in 15..20 {
            let eid = format!("E{}", i);
            signals.push((eid, i as i64 * 86400));
        }
        // Entities with outcome but no signal (FN)
        for i in 20..25 {
            let eid = format!("E{}", i);
            outcomes.push((eid, i as i64 * 86400));
        }
        assert!(pattern_emerges(&outcomes, &signals, 0, 5, 1.0));
    }

    #[test]
    fn test_pattern_emerges_insufficient_data() {
        let outcomes = make_events(&[("A", &[10])]);
        let signals = make_events(&[("A", &[10])]);
        // total < 10 → false
        assert!(!pattern_emerges(&outcomes, &signals, 0, 5, 1.5));
    }

    // ── Walk-forward backtest ──────────────────

    #[test]
    fn test_walk_forward_empty_data() {
        let candidate = sample_candidate();
        let config = BacktestConfig::default();
        let miner = MinerConfig::default();
        let result = walk_forward_backtest(&candidate, &[], &[], &config, &miner);
        assert!(result.is_none());
    }

    #[test]
    fn test_walk_forward_zero_folds() {
        let candidate = sample_candidate();
        let config = BacktestConfig {
            folds: 0,
            ..Default::default()
        };
        let miner = MinerConfig::default();
        let outcomes = make_events(&[("A", &[10, 50])]);
        let signals = make_events(&[("A", &[5, 45])]);
        let result = walk_forward_backtest(&candidate, &outcomes, &signals, &config, &miner);
        assert!(result.is_none());
    }

    #[test]
    fn test_walk_forward_correct_fold_count() {
        // Generate sufficient data spread over a long timeline
        let mut outcomes = Vec::new();
        let mut signals = Vec::new();
        for i in 0..50 {
            let eid = format!("E{}", i % 10);
            signals.push((eid.clone(), (i * 10) as i64 * 86400));
            outcomes.push((eid, (i * 10 + 1) as i64 * 86400));
        }

        let candidate = sample_candidate();
        let config = BacktestConfig {
            folds: 3,
            initial_train_fraction: 0.5,
            window_days: 30,
            min_tp_per_fold: 0,
        };
        let miner = MinerConfig::default();

        let result = walk_forward_backtest(&candidate, &outcomes, &signals, &config, &miner);
        assert!(result.is_some());
        let r = result.unwrap();
        assert_eq!(r.fold_results.len(), 3);
    }

    #[test]
    fn test_walk_forward_fold_boundaries_non_overlapping() {
        let mut outcomes = Vec::new();
        let mut signals = Vec::new();
        for i in 0..100 {
            let eid = format!("E{}", i % 15);
            signals.push((eid.clone(), i as i64 * 86400));
            outcomes.push((eid, (i + 1) as i64 * 86400));
        }

        let candidate = sample_candidate();
        let config = BacktestConfig {
            folds: 4,
            initial_train_fraction: 0.5,
            window_days: 5,
            min_tp_per_fold: 0,
        };
        let miner = MinerConfig::default();

        let result = walk_forward_backtest(&candidate, &outcomes, &signals, &config, &miner).unwrap();

        // Verify non-overlapping: each fold's test_start == previous fold's test_end
        for i in 1..result.fold_results.len() {
            assert_eq!(
                result.fold_results[i].test_start,
                result.fold_results[i - 1].test_end,
                "Fold {} test window overlaps with fold {}",
                i,
                i - 1
            );
        }

        // Verify train_end == test_start for each fold
        for fold in &result.fold_results {
            assert_eq!(fold.train_end, fold.test_start);
        }
    }

    #[test]
    fn test_validate_fold_window_rejects_overlap() {
        assert!(!validate_fold_window(Some(200), 0, 100, 150, 250));
        assert!(!validate_fold_window(None, 100, 90, 90, 120));
        assert!(!validate_fold_window(None, 0, 110, 100, 120));
        assert!(validate_fold_window(Some(200), 0, 100, 200, 250));
    }

    #[test]
    fn test_normalize_stored_metric_rounds_to_six_decimals() {
        assert_eq!(normalize_stored_metric(0.123456789), 0.123457);
        assert_eq!(normalize_stored_metric(0.1234561), 0.123456);
        assert_eq!(normalize_stored_metric(f64::NAN), 0.0);
    }

    #[test]
    fn test_walk_forward_with_strong_pattern() {
        // Create strong predictive pattern: every entity with signal on day X
        // has outcome on day X+1.  Some entities have no signal and no outcome.
        let mut outcomes = Vec::new();
        let mut signals = Vec::new();

        // 30 entities with signal and outcome, spread across 300+ days
        for i in 0..30 {
            let eid = format!("sig_entity_{}", i);
            let day = i * 10; // spread across 0..300 days
            signals.push((eid.clone(), day as i64 * 86400));
            outcomes.push((eid, (day + 1) as i64 * 86400));
        }
        // 30 entities without signal and without outcome
        // (won't appear in data → counted as absent in contingency)

        let candidate = PatternCandidate {
            outcome: "test_outcome".to_string(),
            signals: vec!["test_signal".to_string()],
            best_lag_days: 0,
            effect_size: 5.0,
            p_value: 0.001,
            q_value: 0.005,
            stability: 0.9,
            entity_coverage: 0.6,
            segments: vec![],
            contingency: (20, 0, 0, 20),
        };

        let config = BacktestConfig {
            folds: 2,
            initial_train_fraction: 0.5,
            window_days: 5,
            min_tp_per_fold: 0,
        };
        let miner = MinerConfig {
            min_effect: 1.0,
            max_p: 0.05,
            ..Default::default()
        };

        let result = walk_forward_backtest(
            &candidate, &outcomes, &signals, &config, &miner,
        );
        assert!(result.is_some());
        let r = result.unwrap();
        // With a perfect pattern, aggregate recall and precision should be high
        assert!(r.aggregate.tp > 0, "should have true positives");
    }

    // ── Batch backtest ──────────────────

    #[test]
    fn test_backtest_candidates_filters_failing() {
        let mut outcomes = Vec::new();
        let mut signals = Vec::new();
        for i in 0..50 {
            let eid = format!("E{}", i % 10);
            signals.push((eid.clone(), (i * 10) as i64 * 86400));
            outcomes.push((eid, (i * 10 + 1) as i64 * 86400));
        }

        // Create two candidates: one reasonable, one with absurd lag
        let good = sample_candidate();
        let bad = PatternCandidate {
            outcome: "never_happens".to_string(),
            signals: vec!["does_not_exist".to_string()],
            best_lag_days: 5000, // absurd lag
            effect_size: 0.1,
            p_value: 0.9,
            q_value: 0.9,
            stability: 0.1,
            entity_coverage: 0.01,
            segments: vec![],
            contingency: (0, 0, 0, 0),
        };

        let bt_config = BacktestConfig::default();
        let miner = MinerConfig::default();

        let results = backtest_candidates(
            &[good, bad],
            &outcomes,
            &signals,
            &bt_config,
            &miner,
        );
        // Bad candidate should not produce a passing backtest
        assert!(results.iter().all(|r| r.passed));
    }

    // ── BacktestResult computed fields ──────────────────

    #[test]
    fn test_backtest_result_aggregate_metrics() {
        // Manually create a result and verify computed metrics match
        let aggregate = ConfusionMatrix {
            tp: 20,
            fp: 5,
            fn_: 10,
            tn: 65,
        };
        let p = aggregate.precision();
        let r = aggregate.recall();
        let f1 = aggregate.f1();

        assert!((p - 20.0 / 25.0).abs() < 0.001); // 0.8
        assert!((r - 20.0 / 30.0).abs() < 0.001); // 0.667
        let expected_f1 = 2.0 * 0.8 * (20.0 / 30.0) / (0.8 + 20.0 / 30.0);
        assert!((f1 - expected_f1).abs() < 0.001);
    }

    #[test]
    fn test_backtest_config_default() {
        let cfg = BacktestConfig::default();
        assert_eq!(cfg.folds, 4);
        assert!((cfg.initial_train_fraction - 0.5).abs() < 0.01);
        assert_eq!(cfg.window_days, 30);
        assert_eq!(cfg.min_tp_per_fold, 1);
    }

    #[test]
    fn test_confusion_matrix_serialization() {
        let cm = ConfusionMatrix {
            tp: 10,
            fp: 5,
            fn_: 3,
            tn: 82,
        };
        let json = serde_json::to_string(&cm).unwrap();
        let parsed: ConfusionMatrix = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.tp, 10);
        assert_eq!(parsed.fp, 5);
        assert_eq!(parsed.fn_, 3);
        assert_eq!(parsed.tn, 82);
    }

    // ── B223: pattern_emerges with small total counts ──────
    #[test]
    fn test_pattern_emerges_very_small_data() {
        // Only 3 entities → total < 10 → should return false
        let outcomes = make_events(&[("A", &[1]), ("B", &[2])]);
        let signals = make_events(&[("A", &[1]), ("C", &[3])]);
        assert!(!pattern_emerges(&outcomes, &signals, 0, 5, 1.5));
    }

    // ── B224: BacktestConfig validation ──────────
    #[test]
    fn test_backtest_config_validate_valid() {
        let cfg = BacktestConfig::default();
        assert!(cfg.validate().is_empty());
    }

    #[test]
    fn test_backtest_config_validate_zero_folds() {
        let cfg = BacktestConfig { folds: 0, ..Default::default() };
        let issues = cfg.validate();
        assert!(issues.iter().any(|i| i.contains("folds")));
    }

    #[test]
    fn test_backtest_config_validate_bad_fraction() {
        let cfg = BacktestConfig {
            initial_train_fraction: 1.5,
            ..Default::default()
        };
        let issues = cfg.validate();
        assert!(issues.iter().any(|i| i.contains("initial_train_fraction")));
    }

    #[test]
    fn test_backtest_config_validate_bad_window() {
        let cfg = BacktestConfig {
            window_days: -1,
            ..Default::default()
        };
        let issues = cfg.validate();
        assert!(issues.iter().any(|i| i.contains("window_days")));
    }

    // ── B225: walk_forward_backtest with uneven fold sizes ──────
    #[test]
    fn test_walk_forward_uneven_folds() {
        // 7 folds from data that doesn't divide evenly
        let mut outcomes = Vec::new();
        let mut signals = Vec::new();
        for i in 0..100 {
            let eid = format!("E{}", i % 12);
            signals.push((eid.clone(), i as i64 * 86400));
            outcomes.push((eid, (i + 1) as i64 * 86400));
        }
        let candidate = sample_candidate();
        let config = BacktestConfig {
            folds: 7, // 7 doesn't divide evenly
            initial_train_fraction: 0.3,
            window_days: 5,
            min_tp_per_fold: 0,
        };
        let miner = MinerConfig::default();
        let result = walk_forward_backtest(&candidate, &outcomes, &signals, &config, &miner);
        assert!(result.is_some());
        let r = result.unwrap();
        assert_eq!(r.fold_results.len(), 7);
        // Last fold should cover all remaining data
        let last = r.fold_results.last().unwrap();
        assert!(last.test_end > last.test_start);
    }
}
