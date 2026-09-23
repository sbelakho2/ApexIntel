//! Pattern mining — contingency tables, odds ratios, stability checks.
//!
//! Pure functions for mining signal-outcome pairs from in-memory data.
//! No database dependencies; all data arrives as typed vectors.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Default observation window in days for contingency tables (B220).
pub const DEFAULT_WINDOW_DAYS: i32 = 30;

/// Maximum sweep lag to prevent extremely long runtimes (B222).
pub const MAX_SWEEP_LAG_DAYS: i32 = 365;

type LagSweepBest = (
    i32,
    apex_stats::fisher::FisherExactResult,
    f64,
    (u64, u64, u64, u64),
);

// ────────────────────────────────────────────
// Config & types
// ────────────────────────────────────────────

/// Configuration for the causal signal miner (B289).
///
/// All fields have well-tested defaults via [`Default`]; override only when you
/// have domain-specific knowledge that warrants it.
///
/// # Default values
/// | Field               | Default | Rationale                                                   |
/// |--------------------|---------|------------------------------------------------------------|
/// | `max_lag_days`    | 90      | Covers a full business quarter; longer lags are noisy      |
/// | `min_effect`      | 1.5     | Requires at least 50% uplift in odds ratio                 |
/// | `max_p`           | 0.01    | 1% significance threshold (stricter than the usual 5%)     |
/// | `min_stability`   | 0.6     | 60% cross-time-slice consistency required                  |
/// | `time_splits`     | 4       | Quarterly splits for stability validation                  |
/// | `entity_min_count`| 5       | Exclude pairs with too few observations                     |
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MinerConfig {
    /// Maximum causal lag in calendar days to evaluate.  Default: `90`.
    pub max_lag_days: i32,
    /// Minimum required odds ratio (effect size).  Default: `1.5`.
    pub min_effect: f64,
    /// Maximum p-value for the Fisher exact test.  Default: `0.01`.
    pub max_p: f64,
    /// Minimum cross-split stability score (0–1).  Default: `0.6`.
    pub min_stability: f64,
    /// Number of time splits used for stability validation.  Default: `4`.
    pub time_splits: usize,
    /// Minimum entity count required for a valid contingency table.  Default: `5`.
    pub entity_min_count: usize,
}

impl Default for MinerConfig {
    fn default() -> Self {
        Self {
            max_lag_days: 90,
            min_effect: 1.5,
            max_p: 0.01,
            min_stability: 0.6,
            time_splits: 4,
            entity_min_count: 5,
        }
    }
}

impl MinerConfig {
    /// Validate that all numeric fields are within their valid operating ranges.
    ///
    /// Returns an empty `Vec` when the config is valid.  Each entry in a
    /// non-empty return value is a human-readable error string suitable for
    /// logging at startup or returning from an API validation endpoint.
    ///
    /// # Valid ranges
    /// | Field              | Constraint                  | Rationale                          |
    /// |-------------------|-----------------------------|------------------------------------||
    /// | `max_lag_days`    | `>= 1`                      | At least one day lag required      |
    /// | `min_effect`      | `> 1.0`                     | Effect must exceed baseline         |
    /// | `max_p`           | `(0.0, 1.0]`                | Valid probability                   |
    /// | `min_stability`   | `[0.0, 1.0]`                | Fraction; unity means perfectly stable |
    /// | `time_splits`     | `>= 2`                      | Need ≥2 splits for cross-validation |
    /// | `entity_min_count`| `>= 1`                      | At least one entity required        |
    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();

        if self.max_lag_days < 1 {
            errors.push(format!(
                "MinerConfig.max_lag_days = {} must be >= 1",
                self.max_lag_days
            ));
        }
        if self.min_effect <= 1.0 || !self.min_effect.is_finite() {
            errors.push(format!(
                "MinerConfig.min_effect = {} must be > 1.0 (represents uplift over baseline)",
                self.min_effect
            ));
        }
        if !self.max_p.is_finite() || self.max_p <= 0.0 || self.max_p > 1.0 {
            errors.push(format!(
                "MinerConfig.max_p = {} must be in (0.0, 1.0]",
                self.max_p
            ));
        }
        if !self.min_stability.is_finite() || self.min_stability < 0.0 || self.min_stability > 1.0 {
            errors.push(format!(
                "MinerConfig.min_stability = {} must be in [0.0, 1.0]",
                self.min_stability
            ));
        }
        if self.time_splits < 2 {
            errors.push(format!(
                "MinerConfig.time_splits = {} must be >= 2 for meaningful cross-validation",
                self.time_splits
            ));
        }
        if self.entity_min_count < 1 {
            errors.push(format!(
                "MinerConfig.entity_min_count = {} must be >= 1",
                self.entity_min_count
            ));
        }

        errors
    }
}

/// A timestamped event for an entity: (entity_id, epoch_seconds).
pub type EventRecord = (String, i64);

/// A mined pattern candidate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternCandidate {
    pub outcome: String,
    pub signals: Vec<String>,
    pub best_lag_days: i32,
    pub effect_size: f64,
    pub odds_ratio_ci_low: Option<f64>,
    pub odds_ratio_ci_high: Option<f64>,
    pub minimum_detectable_effect: f64,
    pub p_value: f64,
    pub q_value: f64,
    pub stability: f64,
    pub entity_coverage: f64,
    pub segments: Vec<String>,
    pub contingency: (u64, u64, u64, u64),
}

// ────────────────────────────────────────────
// Contingency table construction
// ────────────────────────────────────────────

/// Build a 2×2 contingency table from outcome and signal events at a given lag.
///
/// For each entity, determines:
/// - Has signal? (any event in signal list)
/// - Has outcome within window? (any outcome event within `window_days` of any
///   signal event shifted by `lag_days`)
///
/// `total_entities` accounts for the full population — entities that appear
/// in neither outcome nor signal lists are counted as d (neither).  Pass 0
/// to use only entities observed in the data.
///
/// Returns (a, b, c, d) where:
/// - a = signal present AND outcome present
/// - b = signal present AND outcome absent
/// - c = signal absent AND outcome present
/// - d = signal absent AND outcome absent
///
/// # Time complexity
///
/// O(E · (log S + log O)) per call, where E = unique entities,
/// S = avg signal timestamps per entity, O = avg outcome timestamps
/// per entity.  Uses sorted timestamps + binary search instead of
/// O(S · O) nested iteration.
pub fn build_contingency(
    outcomes: &[EventRecord],
    signals: &[EventRecord],
    lag_days: i32,
    window_days: i32,
    total_entities: usize,
) -> (u64, u64, u64, u64) {
    let lag_secs = lag_days as i64 * 86400;
    let window_secs = window_days as i64 * 86400;

    let mut outcome_map: HashMap<&str, Vec<i64>> = HashMap::new();
    for (eid, ts) in outcomes {
        outcome_map.entry(eid.as_str()).or_default().push(*ts);
    }
    // Sort timestamps for binary-search-based window checks
    for v in outcome_map.values_mut() {
        v.sort_unstable();
    }

    let mut signal_map: HashMap<&str, Vec<i64>> = HashMap::new();
    for (eid, ts) in signals {
        signal_map.entry(eid.as_str()).or_default().push(*ts);
    }
    for v in signal_map.values_mut() {
        v.sort_unstable();
    }

    let all_entities: HashSet<&str> = outcome_map
        .keys()
        .chain(signal_map.keys())
        .copied()
        .collect();

    // Compute study observation period from signal timestamps for c-cell symmetry
    let all_signal_times: Vec<i64> = signal_map
        .values()
        .flat_map(|v| v.iter().copied())
        .collect();
    let study_start = all_signal_times.iter().copied().min().unwrap_or(0) + lag_secs;
    let study_end = all_signal_times.iter().copied().max().unwrap_or(0) + lag_secs + window_secs;

    let (mut a, mut b, mut c, mut d) = (0u64, 0u64, 0u64, 0u64);

    for entity in &all_entities {
        let signal_timestamps = signal_map
            .get(*entity)
            .filter(|ts_list| !ts_list.is_empty());
        let has_signal = signal_timestamps.is_some();

        let has_outcome_in_window = if has_signal {
            let Some(s_ts) = signal_timestamps else {
                continue;
            };
            outcome_map.get(*entity).is_some_and(|o_ts| {
                // OPTIMIZATION: Binary search over sorted signal timestamps
                // instead of O(S · O) nested iteration.
                // For each outcome timestamp, check if any signal timestamp
                // falls within [ot - lag_secs - window_secs, ot - lag_secs].
                o_ts.iter().any(|ot| {
                    let window_start = *ot - lag_secs - window_secs;
                    let window_end = *ot - lag_secs;
                    // Binary search: find first signal timestamp >= window_start
                    let idx = s_ts.partition_point(|st| *st < window_start);
                    idx < s_ts.len() && s_ts[idx] <= window_end
                })
            })
        } else {
            // No signal: require outcome within the study observation period
            outcome_map
                .get(*entity)
                .is_some_and(|o_ts| o_ts.iter().any(|ot| *ot >= study_start && *ot <= study_end))
        };

        match (has_signal, has_outcome_in_window) {
            (true, true) => a += 1,
            (true, false) => b += 1,
            (false, true) => c += 1,
            (false, false) => d += 1,
        }
    }

    // Account for background population not present in data.
    // If total_entities is smaller than observed entities, ignore it and use observed size.
    if total_entities > all_entities.len() {
        d += (total_entities - all_entities.len()) as u64;
    } else if total_entities > 0 && total_entities < all_entities.len() {
        tracing::warn!(
            total_entities,
            observed_entities = all_entities.len(),
            "build_contingency: total_entities smaller than observed; using observed entities"
        );
    }

    (a, b, c, d)
}

/// Compute odds ratio from a 2×2 contingency table.
#[inline]
pub fn odds_ratio(a: u64, b: u64, c: u64, d: u64) -> f64 {
    let num = (a as f64) * (d as f64);
    let den = (b as f64) * (c as f64);
    if den < 1e-12 {
        return 100.0; // cap instead of f64::MAX to prevent Infinity propagation in rank scoring
    }
    num / den
}

/// Compute Fisher's exact test p-value using our stats crate.
#[inline]
pub fn fisher_p_value(a: u64, b: u64, c: u64, d: u64) -> f64 {
    apex_stats::fisher::p_value(a, b, c, d)
}

pub fn fisher_result(a: u64, b: u64, c: u64, d: u64) -> apex_stats::fisher::FisherExactResult {
    apex_stats::fisher::analyze(a, b, c, d)
}

// ────────────────────────────────────────────
// Stability analysis
// ────────────────────────────────────────────

/// Compute temporal stability: fraction of time splits with a positive effect.
pub fn compute_stability(
    outcomes: &[EventRecord],
    signals: &[EventRecord],
    lag_days: i32,
    splits: usize,
) -> f64 {
    const MIN_VALID_TIME_SLICES: usize = 2;
    if outcomes.is_empty() || signals.is_empty() || splits == 0 {
        return 0.0;
    }

    let all_ts: Vec<i64> = outcomes
        .iter()
        .chain(signals.iter())
        .map(|(_, ts)| *ts)
        .collect();
    let min_ts = match all_ts.iter().min() {
        Some(v) => *v,
        None => return 0.0,
    };
    let max_ts = match all_ts.iter().max() {
        Some(v) => *v,
        None => return 0.0,
    };

    if max_ts <= min_ts {
        return 0.0;
    }

    let split_size = (max_ts - min_ts) / splits as i64;
    if split_size == 0 {
        return 0.0;
    }

    let mut positive_splits = 0;
    let mut valid_splits = 0;

    for i in 0..splits {
        let start = min_ts + i as i64 * split_size;
        // Last split extends to max_ts+1 to include boundary events.
        let end = if i == splits - 1 {
            max_ts + 1
        } else {
            start + split_size
        };

        let split_outcomes: Vec<EventRecord> = outcomes
            .iter()
            .filter(|(_, ts)| *ts >= start && *ts < end)
            .cloned()
            .collect();
        let split_signals: Vec<EventRecord> = signals
            .iter()
            .filter(|(_, ts)| *ts >= start && *ts < end)
            .cloned()
            .collect();

        if split_outcomes.len() < 3 || split_signals.len() < 3 {
            continue;
        }

        valid_splits += 1;
        let (a, b, c, d) = build_contingency(&split_outcomes, &split_signals, lag_days, 30, 0);
        let effect = odds_ratio(a, b, c, d);
        let p = fisher_p_value(a, b, c, d);

        if effect > 1.0 && p < 0.1 {
            positive_splits += 1;
        }
    }

    if valid_splits == 0 {
        return 0.0;
    }

    if valid_splits < MIN_VALID_TIME_SLICES {
        tracing::warn!(
            requested_splits = splits,
            valid_slices = valid_splits,
            min_required = MIN_VALID_TIME_SLICES,
            "compute_stability: valid time slices below minimum required"
        );
        return 0.0;
    }

    positive_splits as f64 / valid_splits as f64
}

/// Compute entity coverage: fraction of entities where the signal fires.
pub fn entity_coverage(signals: &[EventRecord], all_entity_count: usize) -> f64 {
    if all_entity_count == 0 {
        return 0.0;
    }
    let unique: HashSet<&str> = signals.iter().map(|(eid, _)| eid.as_str()).collect();
    unique.len() as f64 / all_entity_count as f64
}

// ────────────────────────────────────────────
// Lag sweep
// ────────────────────────────────────────────

/// Sweep lags to find the best one for a signal-outcome pair.
/// `max_lag_days` is clamped to `MAX_SWEEP_LAG_DAYS` to prevent excessive runtimes (B222).
///
/// # Time complexity
///
/// O(L · E · (log S + log O)) where L = number of lags, E = unique entities,
/// S = signal timestamps per entity, O = outcome timestamps per entity.
///
/// # Optimizations
///
/// - **Early exit on consecutive non-significant lags**: if `EARLY_EXIT_STREAK`
///   consecutive lags on both the negative and positive sides show no significant
///   effect, the sweep stops scanning further away.  The best lag for causal
///   signals is typically close to zero; very distant lags are rarely meaningful.
/// - `max_lag_days` clamping via [`MAX_SWEEP_LAG_DAYS`] (B222).
pub fn sweep_lags(
    outcomes: &[EventRecord],
    signals: &[EventRecord],
    config: &MinerConfig,
    window_days: i32,
) -> Option<PatternCandidate> {
    /// Number of consecutive non-significant lags before early exit.
    const EARLY_EXIT_STREAK: i32 = 30;

    let effective_max = config.max_lag_days.min(MAX_SWEEP_LAG_DAYS); // B222
    let mut best: Option<LagSweepBest> = None;

    // OPTIMIZATION: Sweep negative and positive sides separately with
    // independent early-exit streaks.  This avoids the bug where breaking
    // out of a unified loop on the negative side would skip all positive lags.
    let mut streak = 0i32;

    // Phase 1: negative lags (most distant → close to zero)
    for lag in (-effective_max..0).rev() {
        let (a, b, c, d) = build_contingency(outcomes, signals, lag, window_days, 0);
        let total = a + b + c + d;
        if total < 20 {
            continue;
        }

        let fisher = fisher_result(a, b, c, d);
        let effect = odds_ratio(a, b, c, d);
        let mde = apex_stats::fisher::minimum_detectable_odds_ratio(a, b, c, d, config.max_p, 0.80);

        let is_sig = effect >= config.min_effect && fisher.p_value <= config.max_p;
        if is_sig {
            streak = 0;
            if best
                .as_ref()
                .is_none_or(|(_, bf, _, _)| effect > bf.odds_ratio.min(100.0))
            {
                best = Some((lag, fisher, mde, (a, b, c, d)));
            }
        } else {
            streak += 1;
            if streak >= EARLY_EXIT_STREAK {
                break; // remaining negative lags closer to zero unlikely to be significant
            }
        }
    }

    // Phase 2: zero and positive lags (0 → +max)
    streak = 0;
    for lag in 0..=effective_max {
        let (a, b, c, d) = build_contingency(outcomes, signals, lag, window_days, 0);
        let total = a + b + c + d;
        if total < 20 {
            continue;
        }

        let fisher = fisher_result(a, b, c, d);
        let effect = odds_ratio(a, b, c, d);
        let mde = apex_stats::fisher::minimum_detectable_odds_ratio(a, b, c, d, config.max_p, 0.80);

        let is_sig = effect >= config.min_effect && fisher.p_value <= config.max_p;
        if is_sig {
            streak = 0;
            if best
                .as_ref()
                .is_none_or(|(_, bf, _, _)| effect > bf.odds_ratio.min(100.0))
            {
                best = Some((lag, fisher, mde, (a, b, c, d)));
            }
        } else {
            streak += 1;
            if streak >= EARLY_EXIT_STREAK {
                break; // remaining positive lags too far from zero
            }
        }
    }

    best.map(|(lag, fisher, mde, contingency)| PatternCandidate {
        outcome: "unknown_outcome".to_string(),
        signals: vec!["unknown_signal".to_string()],
        best_lag_days: lag,
        effect_size: odds_ratio(contingency.0, contingency.1, contingency.2, contingency.3),
        odds_ratio_ci_low: fisher.odds_ratio_ci_low,
        odds_ratio_ci_high: fisher.odds_ratio_ci_high,
        minimum_detectable_effect: mde,
        p_value: fisher.p_value,
        q_value: 0.0,
        stability: 0.0,
        entity_coverage: 0.0,
        segments: Vec::new(),
        contingency,
    })
}

/// Mine one signal-outcome pair: sweep lags, check stability.
pub fn mine_one_pair(
    outcome_name: &str,
    signal_name: &str,
    outcomes: &[EventRecord],
    signals: &[EventRecord],
    config: &MinerConfig,
) -> Option<PatternCandidate> {
    if outcomes.len() < config.entity_min_count || signals.len() < config.entity_min_count {
        return None;
    }

    let mut candidate = sweep_lags(outcomes, signals, config, 30)?;

    // Check stability
    let stability = compute_stability(
        outcomes,
        signals,
        candidate.best_lag_days,
        config.time_splits,
    );
    if stability < config.min_stability {
        return None;
    }

    // Count unique entities across both
    let all_entities: HashSet<&str> = outcomes
        .iter()
        .chain(signals.iter())
        .map(|(eid, _)| eid.as_str())
        .collect();

    candidate.outcome = outcome_name.to_string();
    candidate.signals = vec![signal_name.to_string()];
    candidate.stability = stability;
    candidate.entity_coverage = entity_coverage(signals, all_entities.len());

    Some(candidate)
}

// ────────────────────────────────────────────
// Candidate ranking
// ────────────────────────────────────────────

/// Rank candidates by composite score: effect_size * stability * (1 - p_value).
pub fn rank_candidates(candidates: &mut [PatternCandidate]) {
    candidates.sort_by(|a, b| {
        let score_a = a.effect_size * a.stability * (1.0 - a.p_value);
        let score_b = b.effect_size * b.stability * (1.0 - b.p_value);
        // Primary: composite score DESC
        score_b
            .partial_cmp(&score_a)
            .unwrap_or(std::cmp::Ordering::Equal)
            // Secondary: outcome ASC — deterministic tiebreak (B292)
            .then_with(|| a.outcome.cmp(&b.outcome))
            // Tertiary: signals join ASC — fully deterministic across equal-score candidates
            .then_with(|| a.signals.join(",").cmp(&b.signals.join(",")))
    });
}

/// Apply FDR correction to a set of candidates using Benjamini-Hochberg.
pub fn apply_fdr_correction(candidates: &mut Vec<PatternCandidate>, max_q: f64) {
    let p_values: Vec<f64> = candidates.iter().map(|c| c.p_value).collect();
    let q_values = apex_stats::fdr::bh_correct(&p_values);
    for (i, q) in q_values.into_iter().enumerate() {
        candidates[i].q_value = q;
    }
    candidates.retain(|c| c.q_value <= max_q);
}

/// Deduplicate candidates: if two share the same outcome and overlapping signals,
/// keep the one with higher effect size.
pub fn deduplicate_candidates(candidates: &mut Vec<PatternCandidate>) {
    candidates.sort_by(|a, b| {
        b.effect_size
            .partial_cmp(&a.effect_size)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut seen: HashSet<String> = HashSet::new();
    candidates.retain(|c| {
        let key = format!("{}:{}", c.outcome, c.signals.join(","));
        seen.insert(key)
    });
}

// ────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[allow(dead_code)]
    fn make_events(entity_id: &str, timestamps: &[i64]) -> Vec<EventRecord> {
        timestamps
            .iter()
            .map(|ts| (entity_id.to_string(), *ts))
            .collect()
    }

    #[test]
    fn test_miner_config_defaults() {
        let cfg = MinerConfig::default();
        assert_eq!(cfg.max_lag_days, 90);
        assert!((cfg.min_effect - 1.5).abs() < 0.01);
        assert!((cfg.max_p - 0.01).abs() < 0.001);
        assert!((cfg.min_stability - 0.6).abs() < 0.01);
        assert_eq!(cfg.time_splits, 4);
        assert_eq!(cfg.entity_min_count, 5);
    }

    // B289: MinerConfig default stability
    #[test]
    fn test_miner_config_default_is_stable_across_two_calls() {
        let a = MinerConfig::default();
        let b = MinerConfig::default();
        assert_eq!(a.max_lag_days, b.max_lag_days);
        assert!((a.min_effect - b.min_effect).abs() < f64::EPSILON);
        assert!((a.max_p - b.max_p).abs() < f64::EPSILON);
        assert!((a.min_stability - b.min_stability).abs() < f64::EPSILON);
        assert_eq!(a.time_splits, b.time_splits);
        assert_eq!(a.entity_min_count, b.entity_min_count);
    }

    #[test]
    fn test_miner_config_default_values_are_in_valid_ranges() {
        let cfg = MinerConfig::default();
        assert!(cfg.max_lag_days > 0, "max_lag_days must be positive");
        assert!(
            cfg.min_effect > 1.0,
            "min_effect must be > 1.0 (odds ratio uplift)"
        );
        assert!(cfg.max_p > 0.0 && cfg.max_p < 1.0, "max_p must be in (0,1)");
        assert!(
            cfg.min_stability >= 0.0 && cfg.min_stability <= 1.0,
            "min_stability in [0,1]"
        );
        assert!(
            cfg.time_splits >= 2,
            "time_splits must be >= 2 for meaningful cross-validation"
        );
        assert!(cfg.entity_min_count >= 1, "entity_min_count must be >= 1");
    }

    #[test]
    fn test_build_contingency_basic() {
        // Entity A: has signal and outcome within window
        // Entity B: has signal but no outcome
        // Entity C: has outcome but no signal
        // Entity D: neither
        let outcomes = vec![("A".to_string(), 1000i64), ("C".to_string(), 2000i64)];
        let signals = vec![("A".to_string(), 1000i64), ("B".to_string(), 3000i64)];
        // With lag=0, window=30 days (2592000 seconds)
        // 4 entities known in population: A, B, C, D (D has neither)
        let (a, b, c, d) = build_contingency(&outcomes, &signals, 0, 30, 4);
        assert_eq!(a, 1); // A: signal + outcome in window
        assert_eq!(b, 1); // B: signal but no outcome
        assert_eq!(c, 1); // C: outcome but no signal
        assert_eq!(d, 1); // D: neither (background population)
    }

    #[test]
    fn test_build_contingency_with_lag() {
        // Signal at day 0, outcome at day 10 — with lag=10 they should match
        let outcomes = vec![("A".to_string(), 864000i64)]; // day 10
        let signals = vec![("A".to_string(), 0i64)]; // day 0
        let (a, _b, _c, _d) = build_contingency(&outcomes, &signals, 10, 5, 0);
        assert_eq!(a, 1);
    }

    #[test]
    fn test_build_contingency_outside_window() {
        // Signal at day 0, outcome at day 100 — lag=0, window=5 days
        let outcomes = vec![("A".to_string(), 8640000i64)]; // day 100
        let signals = vec![("A".to_string(), 0i64)];
        let (a, b, _c, _d) = build_contingency(&outcomes, &signals, 0, 5, 0);
        assert_eq!(a, 0); // too far apart
        assert_eq!(b, 1); // signal present, no outcome in window
    }

    #[test]
    fn test_odds_ratio_perfect() {
        // All signal entities have outcome, no non-signal entities do.
        // Returns 100.0 (capped) instead of f64::MAX to prevent Infinity in rank scoring.
        assert_eq!(odds_ratio(10, 0, 0, 10), 100.0);
    }

    #[test]
    fn test_odds_ratio_normal() {
        // a=10, b=5, c=3, d=20 → OR = (10*20)/(5*3) = 200/15 ≈ 13.33
        let or = odds_ratio(10, 5, 3, 20);
        assert!((or - 13.333).abs() < 0.01);
    }

    #[test]
    fn test_odds_ratio_no_effect() {
        // a=5, b=5, c=5, d=5 → OR = 1.0
        let or = odds_ratio(5, 5, 5, 5);
        assert!((or - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_fisher_p_value_significant() {
        // Strong association
        let p = fisher_p_value(20, 2, 1, 20);
        assert!(p < 0.01);
    }

    #[test]
    fn test_fisher_p_value_not_significant() {
        // No association
        let p = fisher_p_value(5, 5, 5, 5);
        assert!(p > 0.1);
    }

    #[test]
    fn test_entity_coverage() {
        let signals = vec![
            ("A".to_string(), 100),
            ("A".to_string(), 200),
            ("B".to_string(), 300),
        ];
        let cov = entity_coverage(&signals, 5);
        assert!((cov - 0.4).abs() < 0.01); // 2 unique out of 5
    }

    #[test]
    fn test_entity_coverage_empty() {
        assert_eq!(entity_coverage(&[], 10), 0.0);
        assert_eq!(entity_coverage(&[("A".to_string(), 1)], 0), 0.0);
    }

    #[test]
    fn test_compute_stability_empty() {
        assert_eq!(compute_stability(&[], &[], 0, 4), 0.0);
    }

    #[test]
    fn test_rank_candidates() {
        let mut candidates = vec![
            PatternCandidate {
                outcome: "A".to_string(),
                signals: vec!["s1".to_string()],
                best_lag_days: 0,
                effect_size: 2.0,
                odds_ratio_ci_low: Some(1.1),
                odds_ratio_ci_high: Some(3.5),
                minimum_detectable_effect: 1.5,
                p_value: 0.01,
                q_value: 0.01,
                stability: 0.8,
                entity_coverage: 0.5,
                segments: vec![],
                contingency: (10, 5, 3, 20),
            },
            PatternCandidate {
                outcome: "B".to_string(),
                signals: vec!["s2".to_string()],
                best_lag_days: 5,
                effect_size: 5.0,
                odds_ratio_ci_low: Some(2.4),
                odds_ratio_ci_high: Some(9.2),
                minimum_detectable_effect: 1.7,
                p_value: 0.001,
                q_value: 0.002,
                stability: 0.9,
                entity_coverage: 0.7,
                segments: vec![],
                contingency: (15, 3, 2, 25),
            },
        ];
        rank_candidates(&mut candidates);
        // B should rank higher: 5.0*0.9*0.999 > 2.0*0.8*0.99
        assert_eq!(candidates[0].outcome, "B");
    }

    #[test]
    fn test_apply_fdr_correction() {
        let mut candidates = vec![
            PatternCandidate {
                outcome: "A".to_string(),
                signals: vec![],
                best_lag_days: 0,
                effect_size: 2.0,
                odds_ratio_ci_low: Some(1.1),
                odds_ratio_ci_high: Some(3.5),
                minimum_detectable_effect: 1.5,
                p_value: 0.001,
                q_value: 0.0,
                stability: 0.8,
                entity_coverage: 0.5,
                segments: vec![],
                contingency: (0, 0, 0, 0),
            },
            PatternCandidate {
                outcome: "B".to_string(),
                signals: vec![],
                best_lag_days: 0,
                effect_size: 1.5,
                odds_ratio_ci_low: Some(0.9),
                odds_ratio_ci_high: Some(2.8),
                minimum_detectable_effect: 1.5,
                p_value: 0.5,
                q_value: 0.0,
                stability: 0.6,
                entity_coverage: 0.3,
                segments: vec![],
                contingency: (0, 0, 0, 0),
            },
        ];

        apply_fdr_correction(&mut candidates, 0.05);
        // B has p=0.5 which after FDR should still be > 0.05
        // Only A should remain
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].outcome, "A");
    }

    #[test]
    fn test_deduplicate_candidates() {
        let mut candidates = vec![
            PatternCandidate {
                outcome: "A".to_string(),
                signals: vec!["s1".to_string()],
                best_lag_days: 0,
                effect_size: 2.0,
                odds_ratio_ci_low: Some(1.1),
                odds_ratio_ci_high: Some(3.5),
                minimum_detectable_effect: 1.5,
                p_value: 0.01,
                q_value: 0.01,
                stability: 0.8,
                entity_coverage: 0.5,
                segments: vec![],
                contingency: (0, 0, 0, 0),
            },
            PatternCandidate {
                outcome: "A".to_string(),
                signals: vec!["s1".to_string()],
                best_lag_days: 5,
                effect_size: 1.5, // lower, should be removed
                odds_ratio_ci_low: Some(0.9),
                odds_ratio_ci_high: Some(2.8),
                minimum_detectable_effect: 1.5,
                p_value: 0.01,
                q_value: 0.01,
                stability: 0.7,
                entity_coverage: 0.4,
                segments: vec![],
                contingency: (0, 0, 0, 0),
            },
        ];

        deduplicate_candidates(&mut candidates);
        assert_eq!(candidates.len(), 1);
        assert!((candidates[0].effect_size - 2.0).abs() < 0.01);
    }

    #[test]
    fn test_sweep_lags_no_data() {
        let cfg = MinerConfig::default();
        let result = sweep_lags(&[], &[], &cfg, 30);
        assert!(result.is_none());
    }

    #[test]
    fn test_sweep_lags_no_valid_lag_exists() {
        let outcomes: Vec<EventRecord> = (0..30)
            .map(|i| (format!("E{}", i), i as i64 * 86400))
            .collect();
        let signals: Vec<EventRecord> = (0..30)
            .map(|i| (format!("S{}", i), i as i64 * 86400))
            .collect();

        let cfg = MinerConfig {
            min_effect: 50.0,
            max_p: 1e-10,
            ..Default::default()
        };

        let result = sweep_lags(&outcomes, &signals, &cfg, 30);
        assert!(result.is_none());
    }

    #[test]
    fn test_sweep_lags_populates_fisher_metadata() {
        let mut outcomes = Vec::new();
        let mut signals = Vec::new();

        for i in 0..20 {
            let entity = format!("E{}", i);
            signals.push((entity.clone(), (i * 5) as i64 * 86400));
            outcomes.push((entity, (i * 5 + 1) as i64 * 86400));
        }
        for i in 20..28 {
            let entity = format!("E{}", i);
            signals.push((entity, (i * 5) as i64 * 86400));
        }
        for i in 28..36 {
            let entity = format!("E{}", i);
            outcomes.push((entity, (i * 5) as i64 * 86400));
        }

        let config = MinerConfig {
            min_effect: 1.1,
            max_p: 0.05,
            ..MinerConfig::default()
        };
        let candidate = sweep_lags(&outcomes, &signals, &config, 5)
            .expect("sweep_lags should find a valid candidate");

        assert!(candidate.odds_ratio_ci_low.is_some());
        assert!(candidate.odds_ratio_ci_high.is_some());
        assert!(candidate.odds_ratio_ci_low.unwrap() < candidate.effect_size);
        assert!(candidate.odds_ratio_ci_high.unwrap() > candidate.effect_size);
        assert!(candidate.minimum_detectable_effect > 1.0);
    }

    #[test]
    fn test_build_contingency_total_entities_smaller_than_observed_uses_observed() {
        let outcomes = vec![("A".to_string(), 0i64), ("B".to_string(), 0i64)];
        let signals = vec![("A".to_string(), 0i64), ("C".to_string(), 0i64)];

        let observed = build_contingency(&outcomes, &signals, 0, 30, 0);
        let undersized = build_contingency(&outcomes, &signals, 0, 30, 1);

        assert_eq!(
            observed, undersized,
            "undersized total_entities must not distort counts"
        );
    }

    // ── B216: build_contingency with empty data ──────────
    #[test]
    fn test_build_contingency_empty_outcomes() {
        let signals = vec![("A".to_string(), 100i64)];
        let (a, b, c, _d) = build_contingency(&[], &signals, 0, 30, 0);
        assert_eq!(a, 0);
        assert_eq!(c, 0);
        assert!(b >= 1); // A is signal-only
    }

    #[test]
    fn test_build_contingency_empty_signals() {
        let outcomes = vec![("A".to_string(), 100i64)];
        let (a, b, _c, _d) = build_contingency(&outcomes, &[], 0, 30, 0);
        assert_eq!(a, 0);
        assert_eq!(b, 0);
    }

    #[test]
    fn test_build_contingency_both_empty() {
        let (a, b, c, d) = build_contingency(&[], &[], 0, 30, 0);
        assert_eq!((a, b, c, d), (0, 0, 0, 0));
    }

    #[test]
    fn test_build_contingency_both_empty_with_population() {
        let (a, _b, _c, d) = build_contingency(&[], &[], 0, 30, 100);
        assert_eq!(a, 0);
        assert_eq!(d, 100); // all population goes to d
    }

    // ── B218: odds_ratio small denominator ──────────
    #[test]
    fn test_odds_ratio_zero_denominator_capped() {
        // b=0, c=0 → den near zero, should return 100.0 (not infinity)
        let or = odds_ratio(5, 0, 0, 10);
        assert_eq!(or, 100.0);
        assert!(or.is_finite());
    }

    #[test]
    fn test_odds_ratio_all_zeros() {
        let or = odds_ratio(0, 0, 0, 0);
        assert!(or.is_finite());
    }

    // ── B219: compute_stability small splits ──────────
    #[test]
    fn test_compute_stability_one_split() {
        let outcomes: Vec<EventRecord> = (0..10)
            .map(|i| (format!("E{}", i), i as i64 * 86400 * 10))
            .collect();
        let signals: Vec<EventRecord> = (0..10)
            .map(|i| (format!("E{}", i), i as i64 * 86400 * 10))
            .collect();
        let stability = compute_stability(&outcomes, &signals, 0, 1);
        // With 1 split, stability is 0.0 or 1.0
        assert!((0.0..=1.0).contains(&stability));
    }

    #[test]
    fn test_compute_stability_two_splits() {
        let outcomes: Vec<EventRecord> = (0..20)
            .map(|i| (format!("E{}", i), i as i64 * 86400 * 5))
            .collect();
        let signals: Vec<EventRecord> = (0..20)
            .map(|i| (format!("E{}", i), i as i64 * 86400 * 5))
            .collect();
        let stability = compute_stability(&outcomes, &signals, 0, 2);
        assert!((0.0..=1.0).contains(&stability));
    }

    // ── B220: DEFAULT_WINDOW_DAYS constant ──────────
    #[test]
    fn test_default_window_days_constant() {
        assert_eq!(DEFAULT_WINDOW_DAYS, 30);
    }

    // ── B221: entity_coverage with zero entities ──────────
    #[test]
    fn test_entity_coverage_zero_total() {
        assert_eq!(entity_coverage(&[("A".to_string(), 1)], 0), 0.0);
    }

    #[test]
    fn test_entity_coverage_zero_signals_zero_total() {
        assert_eq!(entity_coverage(&[], 0), 0.0);
    }

    // ── B222: sweep_lags large max_lag_days clamped ──────────
    #[test]
    fn test_sweep_lags_large_max_lag_clamped() {
        let cfg = MinerConfig {
            max_lag_days: 10_000, // exceeds MAX_SWEEP_LAG_DAYS
            ..Default::default()
        };
        // Should not panic or take forever — clamped to MAX_SWEEP_LAG_DAYS
        let result = sweep_lags(&[], &[], &cfg, 30);
        assert!(result.is_none());
    }

    #[test]
    fn test_max_sweep_lag_days_constant() {
        assert_eq!(MAX_SWEEP_LAG_DAYS, 365);
    }

    // ── B223: pattern_emerges is not exposed publicly but we test
    //    through compute_stability with small counts ──────────
    #[test]
    fn test_compute_stability_very_few_events() {
        // Only 2 events each — too few per split
        let outcomes = vec![("A".to_string(), 0i64), ("B".to_string(), 86400)];
        let signals = vec![("A".to_string(), 0i64), ("B".to_string(), 86400)];
        let stability = compute_stability(&outcomes, &signals, 0, 4);
        assert_eq!(stability, 0.0);
    }

    // B291: MinerConfig::validate
    #[test]
    fn test_miner_config_default_passes_validation() {
        assert!(
            MinerConfig::default().validate().is_empty(),
            "default config must be valid out of the box"
        );
    }

    #[test]
    fn test_miner_config_zero_lag_is_invalid() {
        let cfg = MinerConfig {
            max_lag_days: 0,
            ..MinerConfig::default()
        };
        let errs = cfg.validate();
        assert!(errs.iter().any(|e| e.contains("max_lag_days")));
    }

    #[test]
    fn test_miner_config_min_effect_exactly_one_is_invalid() {
        let cfg = MinerConfig {
            min_effect: 1.0,
            ..MinerConfig::default()
        };
        let errs = cfg.validate();
        assert!(errs.iter().any(|e| e.contains("min_effect")));
    }

    #[test]
    fn test_miner_config_max_p_zero_is_invalid() {
        let cfg = MinerConfig {
            max_p: 0.0,
            ..MinerConfig::default()
        };
        let errs = cfg.validate();
        assert!(errs.iter().any(|e| e.contains("max_p")));
    }

    #[test]
    fn test_miner_config_min_stability_above_one_is_invalid() {
        let cfg = MinerConfig {
            min_stability: 1.01,
            ..MinerConfig::default()
        };
        let errs = cfg.validate();
        assert!(errs.iter().any(|e| e.contains("min_stability")));
    }

    #[test]
    fn test_miner_config_one_time_split_is_invalid() {
        let cfg = MinerConfig {
            time_splits: 1,
            ..MinerConfig::default()
        };
        let errs = cfg.validate();
        assert!(errs.iter().any(|e| e.contains("time_splits")));
    }

    #[test]
    fn test_miner_config_all_invalid_fields_all_reported() {
        let cfg = MinerConfig {
            max_lag_days: 0,
            min_effect: 0.5,
            max_p: 1.5,
            min_stability: -0.1,
            time_splits: 1,
            entity_min_count: 0,
        };
        let errs = cfg.validate();
        // Every broken field must appear in the error list
        assert!(errs.iter().any(|e| e.contains("max_lag_days")));
        assert!(errs.iter().any(|e| e.contains("min_effect")));
        assert!(errs.iter().any(|e| e.contains("max_p")));
        assert!(errs.iter().any(|e| e.contains("min_stability")));
        assert!(errs.iter().any(|e| e.contains("time_splits")));
        assert!(errs.iter().any(|e| e.contains("entity_min_count")));
    }

    // B292: rank_candidates deterministic tie-breaking
    #[test]
    fn test_rank_candidates_equal_scores_ordered_by_outcome_asc() {
        // Two candidates with identical computed score (effect × stability × (1 - p))
        // must be ordered by outcome ASC, then signals ASC
        let make = |outcome: &str, sig: &str| PatternCandidate {
            outcome: outcome.to_string(),
            signals: vec![sig.to_string()],
            best_lag_days: 7,
            effect_size: 2.0,
            odds_ratio_ci_low: Some(1.1),
            odds_ratio_ci_high: Some(3.5),
            minimum_detectable_effect: 1.5,
            p_value: 0.01,
            q_value: 0.01,
            stability: 0.8,
            entity_coverage: 0.5,
            segments: vec![],
            contingency: (10, 5, 5, 80),
        };
        let mut candidates = vec![
            make("zzz_outcome", "signal_a"), // same score, should sort last
            make("aaa_outcome", "signal_a"), // same score, should sort first
        ];
        rank_candidates(&mut candidates);
        assert_eq!(
            candidates[0].outcome, "aaa_outcome",
            "with equal scores, lower outcome string must come first"
        );
    }

    #[test]
    fn test_rank_candidates_deterministic_across_calls() {
        let make = |outcome: &str, effect: f64| PatternCandidate {
            outcome: outcome.to_string(),
            signals: vec!["sig".to_string()],
            best_lag_days: 7,
            effect_size: effect,
            odds_ratio_ci_low: Some(1.1),
            odds_ratio_ci_high: Some(3.5),
            minimum_detectable_effect: 1.5,
            p_value: 0.01,
            q_value: 0.01,
            stability: 0.8,
            entity_coverage: 0.5,
            segments: vec![],
            contingency: (10, 5, 5, 80),
        };
        let mut c1 = vec![make("out_b", 2.0), make("out_a", 2.0)];
        let mut c2 = vec![make("out_b", 2.0), make("out_a", 2.0)];
        rank_candidates(&mut c1);
        rank_candidates(&mut c2);
        let outcomes1: Vec<&str> = c1.iter().map(|c| c.outcome.as_str()).collect();
        let outcomes2: Vec<&str> = c2.iter().map(|c| c.outcome.as_str()).collect();
        assert_eq!(
            outcomes1, outcomes2,
            "rank_candidates must be deterministic"
        );
    }

    #[test]
    fn compute_stability_empty_outcomes_returns_zero() {
        let signals: Vec<EventRecord> = vec![("e1".into(), 100)];
        assert!((compute_stability(&[], &signals, 1, 3) - 0.0).abs() < 1e-10);
    }

    #[test]
    fn compute_stability_empty_signals_returns_zero() {
        let outcomes: Vec<EventRecord> = vec![("e1".into(), 100)];
        assert!((compute_stability(&outcomes, &[], 1, 3) - 0.0).abs() < 1e-10);
    }

    #[test]
    fn compute_stability_zero_splits_returns_zero() {
        let data: Vec<EventRecord> = vec![("e1".into(), 100)];
        assert!((compute_stability(&data, &data, 1, 0) - 0.0).abs() < 1e-10);
    }

    #[test]
    fn compute_stability_single_timestamp_returns_zero() {
        let data: Vec<EventRecord> = vec![("e1".into(), 100)];
        assert!((compute_stability(&data, &data, 1, 3) - 0.0).abs() < 1e-10);
    }
}
