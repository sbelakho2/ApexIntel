//! Pattern mining — contingency tables, odds ratios, stability checks.
//!
//! Pure functions for mining signal-outcome pairs from in-memory data.
//! No database dependencies; all data arrives as typed vectors.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

// ────────────────────────────────────────────
// Config & types
// ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MinerConfig {
    pub max_lag_days: i32,
    pub min_effect: f64,    // minimum odds ratio
    pub max_p: f64,
    pub min_stability: f64,
    pub time_splits: usize,
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

/// A timestamped event for an entity: (entity_id, epoch_seconds).
pub type EventRecord = (String, i64);

/// A mined pattern candidate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternCandidate {
    pub outcome: String,
    pub signals: Vec<String>,
    pub best_lag_days: i32,
    pub effect_size: f64,
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

    let mut signal_map: HashMap<&str, Vec<i64>> = HashMap::new();
    for (eid, ts) in signals {
        signal_map.entry(eid.as_str()).or_default().push(*ts);
    }

    let all_entities: HashSet<&str> = outcome_map
        .keys()
        .chain(signal_map.keys())
        .copied()
        .collect();

    let (mut a, mut b, mut c, mut d) = (0u64, 0u64, 0u64, 0u64);

    for entity in &all_entities {
        let has_signal = signal_map
            .get(*entity)
            .map_or(false, |ts_list| !ts_list.is_empty());

        let has_outcome_in_window = if has_signal {
            let s_ts = signal_map.get(*entity).unwrap();
            outcome_map.get(*entity).map_or(false, |o_ts| {
                o_ts.iter().any(|ot| {
                    s_ts.iter()
                        .any(|st| (*ot - (st + lag_secs)).abs() <= window_secs)
                })
            })
        } else {
            // No signal: check if entity has any outcome at all
            outcome_map
                .get(*entity)
                .map_or(false, |o_ts| !o_ts.is_empty())
        };

        match (has_signal, has_outcome_in_window) {
            (true, true) => a += 1,
            (true, false) => b += 1,
            (false, true) => c += 1,
            (false, false) => d += 1,
        }
    }

    // Account for background population not present in data
    if total_entities > all_entities.len() {
        d += (total_entities - all_entities.len()) as u64;
    }

    (a, b, c, d)
}

/// Compute odds ratio from a 2×2 contingency table.
pub fn odds_ratio(a: u64, b: u64, c: u64, d: u64) -> f64 {
    let num = (a as f64) * (d as f64);
    let den = (b as f64) * (c as f64);
    if den < 1e-12 {
        return f64::MAX;
    }
    num / den
}

/// Compute Fisher's exact test p-value using our stats crate.
pub fn fisher_p_value(a: u64, b: u64, c: u64, d: u64) -> f64 {
    apex_stats::fisher::p_value(a, b, c, d)
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
    if outcomes.is_empty() || signals.is_empty() || splits == 0 {
        return 0.0;
    }

    let all_ts: Vec<i64> = outcomes
        .iter()
        .chain(signals.iter())
        .map(|(_, ts)| *ts)
        .collect();
    let min_ts = *all_ts.iter().min().unwrap();
    let max_ts = *all_ts.iter().max().unwrap();

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
        let end = start + split_size;

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
pub fn sweep_lags(
    outcomes: &[EventRecord],
    signals: &[EventRecord],
    config: &MinerConfig,
    window_days: i32,
) -> Option<PatternCandidate> {
    let mut best: Option<(i32, f64, f64, (u64, u64, u64, u64))> = None;

    for lag in -config.max_lag_days..=config.max_lag_days {
        let (a, b, c, d) = build_contingency(outcomes, signals, lag, window_days, 0);
        let total = a + b + c + d;
        if total < 20 {
            continue;
        }

        let p = fisher_p_value(a, b, c, d);
        let effect = odds_ratio(a, b, c, d);

        if effect >= config.min_effect && p <= config.max_p {
            if best.as_ref().map_or(true, |(_, best_effect, _, _)| effect > *best_effect) {
                best = Some((lag, effect, p, (a, b, c, d)));
            }
        }
    }

    best.map(|(lag, effect, p, contingency)| PatternCandidate {
        outcome: String::new(),
        signals: Vec::new(),
        best_lag_days: lag,
        effect_size: effect,
        p_value: p,
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
    let stability = compute_stability(outcomes, signals, candidate.best_lag_days, config.time_splits);
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
        score_b
            .partial_cmp(&score_a)
            .unwrap_or(std::cmp::Ordering::Equal)
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
    use super::*;

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

    #[test]
    fn test_build_contingency_basic() {
        // Entity A: has signal and outcome within window
        // Entity B: has signal but no outcome
        // Entity C: has outcome but no signal
        // Entity D: neither
        let outcomes = vec![
            ("A".to_string(), 1000i64),
            ("C".to_string(), 2000i64),
        ];
        let signals = vec![
            ("A".to_string(), 1000i64),
            ("B".to_string(), 3000i64),
        ];
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
        let signals = vec![("A".to_string(), 0i64)];       // day 0
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
        // All signal entities have outcome, no non-signal entities do
        assert_eq!(odds_ratio(10, 0, 0, 10), f64::MAX);
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
}
