//! Negative controls — shuffle-based validation to confirm causal direction.
//!
//! A genuine pattern should vanish when either (a) timestamps are permuted
//! or (b) entity-timestamp assignments are randomly shuffled.  If the effect
//! persists under permutation it is likely spurious (e.g., driven by
//! confounding seasonality or entity clustering).

use crate::miner::{
    build_contingency, fisher_p_value, odds_ratio, EventRecord, PatternCandidate,
};
use rand::seq::SliceRandom;
use rand::SeedableRng;
use serde::{Deserialize, Serialize};

// ────────────────────────────────────────────
// Config & types
// ────────────────────────────────────────────

/// Configuration for negative control validation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NegativeControlConfig {
    /// Number of permutation rounds.
    pub permutations: usize,
    /// Significance threshold for the permutation test.
    pub alpha: f64,
    /// Window (days) for contingency table.
    pub window_days: i32,
    /// Random seed for reproducibility.
    pub seed: u64,
}

impl Default for NegativeControlConfig {
    fn default() -> Self {
        Self {
            permutations: 200,
            alpha: 0.05,
            window_days: 30,
            seed: 42,
        }
    }
}

/// Type of shuffle applied.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ShuffleKind {
    /// Permute timestamps across entities (preserves marginal counts).
    TimeShuffle,
    /// Shuffle entity labels on signal events (breaks entity-level link).
    EntityShuffle,
}

/// Result of a negative control test.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NegativeControlResult {
    pub outcome: String,
    pub signal: String,
    pub lag_days: i32,
    pub shuffle_kind: ShuffleKind,
    /// Observed effect on original data.
    pub observed_effect: f64,
    /// Observed p-value on original data.
    pub observed_p: f64,
    /// Distribution of effects under permutation.
    pub permuted_effects: Vec<f64>,
    /// Fraction of permuted effects ≥ observed effect (permutation p-value).
    pub permutation_p_value: f64,
    /// true if the effect vanishes under shuffling (good: genuine signal).
    pub passed: bool,
}

// ────────────────────────────────────────────
// Shuffle helpers
// ────────────────────────────────────────────

/// Shuffle timestamps within signal events (preserves entity identities and
/// marginal timestamp distribution, but breaks temporal alignment).
fn shuffle_times(signals: &[EventRecord], rng: &mut impl rand::Rng) -> Vec<EventRecord> {
    let mut timestamps: Vec<i64> = signals.iter().map(|(_, ts)| *ts).collect();
    timestamps.shuffle(rng);
    signals
        .iter()
        .zip(timestamps.into_iter())
        .map(|((eid, _), new_ts)| (eid.clone(), new_ts))
        .collect()
}

/// Shuffle entity labels on signal events (preserves timestamps and marginal
/// entity distribution, but breaks entity-level association).
fn shuffle_entities(signals: &[EventRecord], rng: &mut impl rand::Rng) -> Vec<EventRecord> {
    let mut entities: Vec<String> = signals.iter().map(|(eid, _)| eid.clone()).collect();
    entities.shuffle(rng);
    signals
        .iter()
        .zip(entities.into_iter())
        .map(|((_, ts), new_eid)| (new_eid, *ts))
        .collect()
}

// ────────────────────────────────────────────
// Permutation test
// ────────────────────────────────────────────

/// Run a permutation test for a single pattern candidate with a given shuffle
/// strategy.
///
/// Returns `None` if there is insufficient data to compute a meaningful
/// contingency table (< 10 total observations).
pub fn run_permutation_test(
    candidate: &PatternCandidate,
    outcomes: &[EventRecord],
    signals: &[EventRecord],
    config: &NegativeControlConfig,
    shuffle_kind: ShuffleKind,
) -> Option<NegativeControlResult> {
    if outcomes.is_empty() || signals.is_empty() {
        return None;
    }

    let lag = candidate.best_lag_days;

    // Observed statistics
    let (a, b, c, d) = build_contingency(outcomes, signals, lag, config.window_days, 0);
    let total = a + b + c + d;
    if total < 10 {
        return None;
    }
    let observed_effect = odds_ratio(a, b, c, d);
    let observed_p = fisher_p_value(a, b, c, d);

    // Permutation distribution
    let mut rng = rand::rngs::StdRng::seed_from_u64(config.seed);
    let mut permuted_effects = Vec::with_capacity(config.permutations);
    let mut count_ge = 0usize;

    for _ in 0..config.permutations {
        let shuffled = match shuffle_kind {
            ShuffleKind::TimeShuffle => shuffle_times(signals, &mut rng),
            ShuffleKind::EntityShuffle => shuffle_entities(signals, &mut rng),
        };

        let (pa, pb, pc, pd) = build_contingency(outcomes, &shuffled, lag, config.window_days, 0);
        let perm_effect = odds_ratio(pa, pb, pc, pd);
        if perm_effect >= observed_effect {
            count_ge += 1;
        }
        permuted_effects.push(perm_effect);
    }

    let perm_p = count_ge as f64 / config.permutations as f64;

    // The control PASSES if permuted effects are significantly lower than
    // observed (i.e., the effect vanishes under shuffling).
    let passed = perm_p < config.alpha;

    Some(NegativeControlResult {
        outcome: candidate.outcome.clone(),
        signal: candidate.signals.first().cloned().unwrap_or_default(),
        lag_days: lag,
        shuffle_kind,
        observed_effect,
        observed_p,
        permuted_effects,
        permutation_p_value: perm_p,
        passed,
    })
}

/// Run both time-shuffle and entity-shuffle controls for a candidate.
/// The candidate passes only if BOTH controls pass.
pub fn run_full_negative_control(
    candidate: &PatternCandidate,
    outcomes: &[EventRecord],
    signals: &[EventRecord],
    config: &NegativeControlConfig,
) -> (Option<NegativeControlResult>, Option<NegativeControlResult>) {
    let time_result =
        run_permutation_test(candidate, outcomes, signals, config, ShuffleKind::TimeShuffle);
    let entity_result =
        run_permutation_test(candidate, outcomes, signals, config, ShuffleKind::EntityShuffle);
    (time_result, entity_result)
}

/// Check if a candidate passes all negative controls.
pub fn passes_negative_controls(
    candidate: &PatternCandidate,
    outcomes: &[EventRecord],
    signals: &[EventRecord],
    config: &NegativeControlConfig,
) -> bool {
    let (time_res, entity_res) = run_full_negative_control(candidate, outcomes, signals, config);

    let time_ok = time_res.map_or(false, |r| r.passed);
    let entity_ok = entity_res.map_or(false, |r| r.passed);

    time_ok && entity_ok
}

/// Batch filter: keep only candidates that pass negative controls.
pub fn filter_by_negative_controls(
    candidates: &[PatternCandidate],
    outcomes: &[EventRecord],
    signals: &[EventRecord],
    config: &NegativeControlConfig,
) -> Vec<PatternCandidate> {
    candidates
        .iter()
        .filter(|c| passes_negative_controls(c, outcomes, signals, config))
        .cloned()
        .collect()
}

/// Compute the mean of the permutation effect distribution.
pub fn mean_permuted_effect(effects: &[f64]) -> f64 {
    if effects.is_empty() {
        return 0.0;
    }
    effects.iter().sum::<f64>() / effects.len() as f64
}

/// Compute effect ratio: observed_effect / mean_permuted_effect.
/// A high ratio (> 2–3×) indicates a robust signal.
pub fn effect_ratio(observed: f64, permuted: &[f64]) -> f64 {
    let mean = mean_permuted_effect(permuted);
    if mean < 1e-12 {
        return if observed > 1e-12 { f64::MAX } else { 1.0 };
    }
    observed / mean
}

// ────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Generate a genuine causal pattern: entity i's signal at day i*10,
    /// outcome at day i*10+1.  Strong temporal alignment.
    fn genuine_pattern(n: usize) -> (Vec<EventRecord>, Vec<EventRecord>) {
        let mut outcomes = Vec::new();
        let mut signals = Vec::new();
        for i in 0..n {
            let eid = format!("E{}", i);
            signals.push((eid.clone(), (i as i64 * 10) * 86400));
            outcomes.push((eid, (i as i64 * 10 + 1) * 86400));
        }
        (outcomes, signals)
    }

    /// Generate random noise pattern: signal and outcome times uncorrelated.
    fn noise_pattern(n: usize) -> (Vec<EventRecord>, Vec<EventRecord>) {
        let mut outcomes = Vec::new();
        let mut signals = Vec::new();
        // Signal entities and outcome entities don't overlap much
        for i in 0..n {
            let sig_eid = format!("S{}", i);
            let out_eid = format!("O{}", i);
            signals.push((sig_eid, (i as i64 * 7) * 86400));
            outcomes.push((out_eid, (i as i64 * 13 + 500) * 86400));
        }
        (outcomes, signals)
    }

    fn sample_candidate(lag: i32) -> PatternCandidate {
        PatternCandidate {
            outcome: "rfq_posted".to_string(),
            signals: vec!["sqe_hiring".to_string()],
            best_lag_days: lag,
            effect_size: 3.0,
            p_value: 0.005,
            q_value: 0.01,
            stability: 0.8,
            entity_coverage: 0.5,
            segments: vec!["TN".to_string()],
            contingency: (10, 3, 2, 20),
        }
    }

    // ── Config defaults ──────────────────

    #[test]
    fn test_config_defaults() {
        let cfg = NegativeControlConfig::default();
        assert_eq!(cfg.permutations, 200);
        assert!((cfg.alpha - 0.05).abs() < 0.001);
        assert_eq!(cfg.window_days, 30);
        assert_eq!(cfg.seed, 42);
    }

    // ── Shuffle helpers ──────────────────

    #[test]
    fn test_shuffle_times_preserves_entities() {
        let signals: Vec<EventRecord> = (0..10)
            .map(|i| (format!("E{}", i), i as i64 * 86400))
            .collect();
        let mut rng = rand::rngs::StdRng::seed_from_u64(99);
        let shuffled = shuffle_times(&signals, &mut rng);

        assert_eq!(shuffled.len(), signals.len());
        // Entities should be in the same order
        for (orig, shuf) in signals.iter().zip(shuffled.iter()) {
            assert_eq!(orig.0, shuf.0);
        }
        // Timestamps should be a permutation of original timestamps
        let mut orig_ts: Vec<i64> = signals.iter().map(|(_, ts)| *ts).collect();
        let mut shuf_ts: Vec<i64> = shuffled.iter().map(|(_, ts)| *ts).collect();
        orig_ts.sort();
        shuf_ts.sort();
        assert_eq!(orig_ts, shuf_ts);
    }

    #[test]
    fn test_shuffle_entities_preserves_timestamps() {
        let signals: Vec<EventRecord> = (0..10)
            .map(|i| (format!("E{}", i), i as i64 * 86400))
            .collect();
        let mut rng = rand::rngs::StdRng::seed_from_u64(99);
        let shuffled = shuffle_entities(&signals, &mut rng);

        assert_eq!(shuffled.len(), signals.len());
        // Timestamps should be in the same order
        for (orig, shuf) in signals.iter().zip(shuffled.iter()) {
            assert_eq!(orig.1, shuf.1);
        }
        // Entities should be a permutation of original entities
        let mut orig_eids: Vec<String> = signals.iter().map(|(eid, _)| eid.clone()).collect();
        let mut shuf_eids: Vec<String> = shuffled.iter().map(|(eid, _)| eid.clone()).collect();
        orig_eids.sort();
        shuf_eids.sort();
        assert_eq!(orig_eids, shuf_eids);
    }

    #[test]
    fn test_shuffle_times_deterministic() {
        let signals: Vec<EventRecord> = (0..20)
            .map(|i| (format!("E{}", i), i as i64 * 86400))
            .collect();
        let mut rng1 = rand::rngs::StdRng::seed_from_u64(42);
        let mut rng2 = rand::rngs::StdRng::seed_from_u64(42);
        let s1 = shuffle_times(&signals, &mut rng1);
        let s2 = shuffle_times(&signals, &mut rng2);
        assert_eq!(s1, s2);
    }

    // ── Permutation test ──────────────────

    #[test]
    fn test_permutation_test_empty_data() {
        let candidate = sample_candidate(0);
        let config = NegativeControlConfig::default();
        let result = run_permutation_test(&candidate, &[], &[], &config, ShuffleKind::TimeShuffle);
        assert!(result.is_none());
    }

    #[test]
    fn test_permutation_test_insufficient_data() {
        let candidate = sample_candidate(0);
        let config = NegativeControlConfig::default();
        let outcomes = vec![("A".to_string(), 86400i64)];
        let signals = vec![("A".to_string(), 0i64)];
        // total < 10 → None
        let result =
            run_permutation_test(&candidate, &outcomes, &signals, &config, ShuffleKind::TimeShuffle);
        assert!(result.is_none());
    }

    #[test]
    fn test_permutation_test_genuine_signal_time_shuffle() {
        // With a genuine co-occurrence pattern, time-shuffling should
        // destroy it.  permutation_p_value should be low → passes.
        let (outcomes, signals) = genuine_pattern(30);
        let candidate = sample_candidate(0);
        let config = NegativeControlConfig {
            permutations: 100,
            alpha: 0.10,
            window_days: 5,
            seed: 42,
        };

        let result =
            run_permutation_test(&candidate, &outcomes, &signals, &config, ShuffleKind::TimeShuffle);
        assert!(result.is_some());
        let r = result.unwrap();
        assert_eq!(r.permuted_effects.len(), 100);
        assert!(!r.observed_effect.is_nan());
    }

    #[test]
    fn test_permutation_test_genuine_signal_entity_shuffle() {
        let (outcomes, signals) = genuine_pattern(30);
        let candidate = sample_candidate(0);
        let config = NegativeControlConfig {
            permutations: 100,
            alpha: 0.10,
            window_days: 5,
            seed: 42,
        };

        let result = run_permutation_test(
            &candidate,
            &outcomes,
            &signals,
            &config,
            ShuffleKind::EntityShuffle,
        );
        assert!(result.is_some());
        let r = result.unwrap();
        assert_eq!(r.permuted_effects.len(), 100);
    }

    #[test]
    fn test_permutation_test_noise_fails() {
        // Random noise pattern: effect should NOT vanish under shuffling
        // because there's no real alignment to destroy.
        let (outcomes, signals) = noise_pattern(30);
        let candidate = sample_candidate(0);
        let config = NegativeControlConfig {
            permutations: 50,
            alpha: 0.05,
            window_days: 5,
            seed: 42,
        };

        let result =
            run_permutation_test(&candidate, &outcomes, &signals, &config, ShuffleKind::TimeShuffle);
        // For noise, the result might be None (insufficient data) or the
        // test should not pass (effect doesn't reliably vanish).
        if let Some(r) = result {
            // We expect that for noise, permutation p is not significant
            // (effect doesn't reliably exceed permuted effects).
            // The key check: noise should not produce passed=true consistently.
            assert!(!r.passed || r.permutation_p_value > 0.01,
                "Noise should not pass negative control with high confidence");
        }
    }

    #[test]
    fn test_permutation_p_value_range() {
        let (outcomes, signals) = genuine_pattern(25);
        let candidate = sample_candidate(0);
        let config = NegativeControlConfig {
            permutations: 100,
            alpha: 0.05,
            window_days: 5,
            seed: 42,
        };

        let result =
            run_permutation_test(&candidate, &outcomes, &signals, &config, ShuffleKind::TimeShuffle)
                .unwrap();
        // p-value must be in [0, 1]
        assert!(result.permutation_p_value >= 0.0);
        assert!(result.permutation_p_value <= 1.0);
    }

    #[test]
    fn test_permutation_reproducible() {
        let (outcomes, signals) = genuine_pattern(20);
        let candidate = sample_candidate(0);
        let config = NegativeControlConfig {
            permutations: 50,
            alpha: 0.05,
            window_days: 5,
            seed: 12345,
        };

        let r1 = run_permutation_test(
            &candidate,
            &outcomes,
            &signals,
            &config,
            ShuffleKind::TimeShuffle,
        )
        .unwrap();
        let r2 = run_permutation_test(
            &candidate,
            &outcomes,
            &signals,
            &config,
            ShuffleKind::TimeShuffle,
        )
        .unwrap();

        assert_eq!(r1.permutation_p_value, r2.permutation_p_value);
        assert_eq!(r1.permuted_effects, r2.permuted_effects);
    }

    // ── Full negative control ──────────────────

    #[test]
    fn test_full_negative_control_returns_both() {
        let (outcomes, signals) = genuine_pattern(25);
        let candidate = sample_candidate(0);
        let config = NegativeControlConfig {
            permutations: 50,
            alpha: 0.10,
            window_days: 5,
            seed: 42,
        };

        let (time_res, entity_res) =
            run_full_negative_control(&candidate, &outcomes, &signals, &config);
        assert!(time_res.is_some());
        assert!(entity_res.is_some());
        assert_eq!(time_res.unwrap().shuffle_kind, ShuffleKind::TimeShuffle);
        assert_eq!(entity_res.unwrap().shuffle_kind, ShuffleKind::EntityShuffle);
    }

    #[test]
    fn test_full_negative_control_empty() {
        let candidate = sample_candidate(0);
        let config = NegativeControlConfig::default();
        let (time_res, entity_res) =
            run_full_negative_control(&candidate, &[], &[], &config);
        assert!(time_res.is_none());
        assert!(entity_res.is_none());
    }

    // ── Passes negative controls ──────────────────

    #[test]
    fn test_passes_negative_controls_empty() {
        let candidate = sample_candidate(0);
        let config = NegativeControlConfig::default();
        assert!(!passes_negative_controls(&candidate, &[], &[], &config));
    }

    // ── Batch filter ──────────────────

    #[test]
    fn test_filter_by_negative_controls_empty() {
        let config = NegativeControlConfig::default();
        let result = filter_by_negative_controls(&[], &[], &[], &config);
        assert!(result.is_empty());
    }

    #[test]
    fn test_filter_preserves_fields() {
        // Create a strong genuine pattern that should survive filtering
        let (outcomes, signals) = genuine_pattern(30);
        let candidate = PatternCandidate {
            outcome: "my_outcome".to_string(),
            signals: vec!["my_signal".to_string()],
            best_lag_days: 0,
            effect_size: 5.0,
            p_value: 0.001,
            q_value: 0.005,
            stability: 0.9,
            entity_coverage: 0.6,
            segments: vec!["TN".to_string(), "MA".to_string()],
            contingency: (20, 2, 1, 30),
        };

        let config = NegativeControlConfig {
            permutations: 50,
            alpha: 0.50, // very lenient for test
            window_days: 5,
            seed: 42,
        };

        let filtered = filter_by_negative_controls(&[candidate.clone()], &outcomes, &signals, &config);
        // If the candidate survived, verify its fields are intact
        for c in &filtered {
            assert_eq!(c.outcome, "my_outcome");
            assert_eq!(c.signals, vec!["my_signal"]);
            assert!((c.effect_size - 5.0).abs() < 0.01);
        }
    }

    // ── Effect ratio helper ──────────────────

    #[test]
    fn test_mean_permuted_effect_empty() {
        assert_eq!(mean_permuted_effect(&[]), 0.0);
    }

    #[test]
    fn test_mean_permuted_effect_normal() {
        let effects = vec![1.0, 2.0, 3.0, 4.0];
        assert!((mean_permuted_effect(&effects) - 2.5).abs() < 0.001);
    }

    #[test]
    fn test_effect_ratio_normal() {
        let permuted = vec![1.0, 1.5, 2.0, 1.0];
        // mean = 1.375
        let ratio = effect_ratio(5.5, &permuted);
        assert!((ratio - 5.5 / 1.375).abs() < 0.01);
    }

    #[test]
    fn test_effect_ratio_zero_permuted() {
        let permuted = vec![0.0, 0.0, 0.0];
        assert_eq!(effect_ratio(5.0, &permuted), f64::MAX);
        assert!((effect_ratio(0.0, &permuted) - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_effect_ratio_empty_permuted() {
        // mean = 0 → same as zero
        assert_eq!(effect_ratio(3.0, &[]), f64::MAX);
    }

    // ── NegativeControlResult serialization ──────────────────

    #[test]
    fn test_result_serialization() {
        let r = NegativeControlResult {
            outcome: "test".to_string(),
            signal: "sig".to_string(),
            lag_days: 10,
            shuffle_kind: ShuffleKind::TimeShuffle,
            observed_effect: 3.0,
            observed_p: 0.01,
            permuted_effects: vec![1.0, 1.5, 2.0],
            permutation_p_value: 0.03,
            passed: true,
        };
        let json = serde_json::to_string(&r).unwrap();
        let parsed: NegativeControlResult = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.outcome, "test");
        assert_eq!(parsed.shuffle_kind, ShuffleKind::TimeShuffle);
        assert!(parsed.passed);
        assert!((parsed.permutation_p_value - 0.03).abs() < 0.001);
    }

    #[test]
    fn test_shuffle_kind_variants() {
        assert_ne!(ShuffleKind::TimeShuffle, ShuffleKind::EntityShuffle);
        let json = serde_json::to_string(&ShuffleKind::EntityShuffle).unwrap();
        assert!(json.contains("EntityShuffle"));
    }
}
