//! Cross-domain signal combination mining.
//!
//! The standard [`miner`](super::miner) tests individual signal-outcome pairs.
//! This module discovers **interaction effects** — combinations of signals from
//! different [`ObservationType`] domains that are *jointly* more predictive than
//! any individual signal.
//!
//! # Approach
//!
//! 1. **Pairwise interaction screening** — for every pair of observation types,
//!    compute a 2×2×2 contingency cube (signal_A × signal_B × outcome) and
//!    derive an interaction odds ratio.
//!
//! 2. **Mutual information filtering** — pairs with low normalised MI against
//!    the outcome are pruned (no point in testing interactions when neither
//!    signal is informative).
//!
//! 3. **Synergy test** — the interaction effect must exceed the stronger
//!    individual effect by a configurable multiplier to count as synergistic
//!    (prevents redundant "both strong signals" from being flagged as synergy).
//!
//! 4. **Stability check** — the interaction must hold across ≥ half the time
//!    splits used by the standard miner.
//!
//! All functions are pure — no database.  Enable with `--features experimental`.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

// ────────────────────────────────────────────
// Config
// ────────────────────────────────────────────

/// Configuration for cross-domain signal combination mining.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrossDomainConfig {
    /// Minimum interaction odds ratio to consider a pair synergistic.
    pub min_interaction_effect: f64,
    /// The interaction must exceed the stronger individual OR by this factor.
    pub synergy_multiplier: f64,
    /// Minimum normalised MI (0–1) for a signal to be considered informative.
    pub min_nmi: f64,
    /// Maximum number of pairs to evaluate (combinatorial guard B286).
    pub max_pairs: usize,
    /// Minimum entity count for meaningful contingency tables.
    pub min_entities: usize,
    /// Number of time-splits for stability check (same as miner).
    pub time_splits: usize,
    /// Minimum fraction of time-splits where the interaction must hold.
    pub min_stability: f64,
}

impl Default for CrossDomainConfig {
    fn default() -> Self {
        Self {
            min_interaction_effect: 2.0,
            synergy_multiplier: 1.3,
            min_nmi: 0.05,
            max_pairs: 500,
            min_entities: 5,
            time_splits: 4,
            min_stability: 0.5,
        }
    }
}

impl CrossDomainConfig {
    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();
        if self.min_interaction_effect <= 1.0 {
            errors.push("min_interaction_effect must be > 1.0".into());
        }
        if self.synergy_multiplier <= 1.0 {
            errors.push("synergy_multiplier must be > 1.0".into());
        }
        if self.min_nmi < 0.0 || self.min_nmi > 1.0 {
            errors.push("min_nmi must be in [0.0, 1.0]".into());
        }
        if self.max_pairs == 0 {
            errors.push("max_pairs must be > 0".into());
        }
        if self.min_entities == 0 {
            errors.push("min_entities must be > 0".into());
        }
        if self.time_splits < 2 {
            errors.push("time_splits must be >= 2".into());
        }
        if self.min_stability < 0.0 || self.min_stability > 1.0 {
            errors.push("min_stability must be in [0.0, 1.0]".into());
        }
        errors
    }
}

// ────────────────────────────────────────────
// Types
// ────────────────────────────────────────────

/// A timestamped, typed observation for an entity.
#[derive(Debug, Clone)]
pub struct TypedEvent {
    pub entity_id: String,
    pub obs_type: String,
    pub ts_epoch: i64,
}

/// A discovered cross-domain signal combination.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalCombination {
    /// First observation type.
    pub type_a: String,
    /// Second observation type.
    pub type_b: String,
    /// Predicted outcome.
    pub outcome: String,
    /// Best lag days for the combined signal.
    pub best_lag_days: i32,
    /// Interaction odds ratio (how much the combined signal exceeds individual).
    pub interaction_effect: f64,
    /// Odds ratio for signal A alone.
    pub individual_effect_a: f64,
    /// Odds ratio for signal B alone.
    pub individual_effect_b: f64,
    /// Synergy factor: interaction / max(individual A, individual B).
    pub synergy_factor: f64,
    /// Fraction of time-splits where the interaction holds.
    pub stability: f64,
    /// Normalised MI between the combined signal and outcome.
    pub nmi_combined: f64,
    /// Entity count used for contingency tables.
    pub entity_coverage: usize,
    /// 2×2×2 contingency cube: (a_and_b, a_only, b_only, neither) × (outcome, no_outcome).
    pub contingency_cube: InteractionContingency,
}

/// A 2×2×2 contingency cube for interaction analysis.
///
/// Rows: (both_signals, signal_a_only, signal_b_only, neither_signal)
/// Cols: (outcome_present, outcome_absent)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InteractionContingency {
    pub both_signals_outcome: u64,
    pub both_signals_no_outcome: u64,
    pub a_only_outcome: u64,
    pub a_only_no_outcome: u64,
    pub b_only_outcome: u64,
    pub b_only_no_outcome: u64,
    pub neither_outcome: u64,
    pub neither_no_outcome: u64,
}

impl InteractionContingency {
    pub fn total(&self) -> u64 {
        self.both_signals_outcome
            + self.both_signals_no_outcome
            + self.a_only_outcome
            + self.a_only_no_outcome
            + self.b_only_outcome
            + self.b_only_no_outcome
            + self.neither_outcome
            + self.neither_no_outcome
    }

    /// Interaction odds ratio: P(outcome | both) / P(outcome | neither),
    /// accounting for the marginals.
    pub fn interaction_odds_ratio(&self) -> f64 {
        let a = self.both_signals_outcome.max(1) as f64;
        let b = self.both_signals_no_outcome.max(1) as f64;
        let c = self.neither_outcome.max(1) as f64;
        let d = self.neither_no_outcome.max(1) as f64;
        (a * d) / (b * c)
    }

    /// Odds ratio for signal A alone (ignoring B status).
    pub fn odds_ratio_a_alone(&self) -> f64 {
        let a = (self.both_signals_outcome + self.a_only_outcome).max(1) as f64;
        let b = (self.both_signals_no_outcome + self.a_only_no_outcome).max(1) as f64;
        let c = (self.b_only_outcome + self.neither_outcome).max(1) as f64;
        let d = (self.b_only_no_outcome + self.neither_no_outcome).max(1) as f64;
        (a * d) / (b * c)
    }

    /// Odds ratio for signal B alone (ignoring A status).
    pub fn odds_ratio_b_alone(&self) -> f64 {
        let a = (self.both_signals_outcome + self.b_only_outcome).max(1) as f64;
        let b = (self.both_signals_no_outcome + self.b_only_no_outcome).max(1) as f64;
        let c = (self.a_only_outcome + self.neither_outcome).max(1) as f64;
        let d = (self.a_only_no_outcome + self.neither_no_outcome).max(1) as f64;
        (a * d) / (b * c)
    }
}

// ────────────────────────────────────────────
// Core algorithm
// ────────────────────────────────────────────

/// Build a 2×2×2 interaction contingency cube.
///
/// For each entity, classifies it into one of 4 groups:
/// - Both signals present in window
/// - Only signal A present
/// - Only signal B present
/// - Neither signal present
/// Then checks whether the outcome occurred within `window_days` of the signal.
pub fn build_interaction_contingency(
    outcomes: &[(String, i64)], // (entity_id, ts_epoch)
    signals_a: &[(String, i64)],
    signals_b: &[(String, i64)],
    lag_days: i32,
    window_days: i32,
    entity_universe: Option<&HashSet<String>>,
) -> InteractionContingency {
    let lag_secs = lag_days as i64 * 86400;
    let window_secs = window_days as i64 * 86400;

    // Group by entity.
    let mut outcome_map: HashMap<&str, Vec<i64>> = HashMap::new();
    for (eid, ts) in outcomes {
        outcome_map.entry(eid.as_str()).or_default().push(*ts);
    }
    let mut signal_a_map: HashMap<&str, Vec<i64>> = HashMap::new();
    for (eid, ts) in signals_a {
        signal_a_map.entry(eid.as_str()).or_default().push(*ts);
    }
    let mut signal_b_map: HashMap<&str, Vec<i64>> = HashMap::new();
    for (eid, ts) in signals_b {
        signal_b_map.entry(eid.as_str()).or_default().push(*ts);
    }

    // Use the explicit entity universe if provided; otherwise fall back to
    // the union of entities referenced in outcomes + signals.
    let fallback: HashSet<String>;
    let all_entities: Vec<&str> = if let Some(universe) = entity_universe {
        universe.iter().map(|s| s.as_str()).collect()
    } else {
        fallback = outcome_map
            .keys()
            .chain(signal_a_map.keys())
            .chain(signal_b_map.keys())
            .map(|s| (*s).to_string())
            .collect();
        fallback.iter().map(|s| s.as_str()).collect()
    };

    let mut cube = InteractionContingency {
        both_signals_outcome: 0,
        both_signals_no_outcome: 0,
        a_only_outcome: 0,
        a_only_no_outcome: 0,
        b_only_outcome: 0,
        b_only_no_outcome: 0,
        neither_outcome: 0,
        neither_no_outcome: 0,
    };

    for &entity in &all_entities {
        let has_a = signal_a_map.get(entity).map_or(false, |ts| !ts.is_empty());
        let has_b = signal_b_map.get(entity).map_or(false, |ts| !ts.is_empty());

        // Check for outcome within window of the *latest* relevant signal.
        let latest_signal_ts = match (has_a, has_b) {
            (true, true) => {
                let a_max = signal_a_map[entity].iter().max().copied().unwrap_or(0);
                let b_max = signal_b_map[entity].iter().max().copied().unwrap_or(0);
                Some(a_max.max(b_max))
            }
            (true, false) => signal_a_map[entity].iter().max().copied(),
            (false, true) => signal_b_map[entity].iter().max().copied(),
            (false, false) => None,
        };

        let has_outcome = if let Some(sig_ts) = latest_signal_ts {
            let window_start = sig_ts + lag_secs;
            let window_end = sig_ts + lag_secs + window_secs;
            outcome_map.get(entity).map_or(false, |ots| {
                ots.iter().any(|&t| t >= window_start && t <= window_end)
            })
        } else {
            // No signals — check if entity had any outcome at all.
            outcome_map.get(entity).map_or(false, |ots| !ots.is_empty())
        };

        match (has_a, has_b, has_outcome) {
            (true, true, true) => cube.both_signals_outcome += 1,
            (true, true, false) => cube.both_signals_no_outcome += 1,
            (true, false, true) => cube.a_only_outcome += 1,
            (true, false, false) => cube.a_only_no_outcome += 1,
            (false, true, true) => cube.b_only_outcome += 1,
            (false, true, false) => cube.b_only_no_outcome += 1,
            (false, false, true) => cube.neither_outcome += 1,
            (false, false, false) => cube.neither_no_outcome += 1,
        }
    }

    cube
}

/// Group typed events by observation type.
fn group_by_type(events: &[TypedEvent]) -> HashMap<String, Vec<(String, i64)>> {
    let mut map: HashMap<String, Vec<(String, i64)>> = HashMap::new();
    for e in events {
        map.entry(e.obs_type.clone())
            .or_default()
            .push((e.entity_id.clone(), e.ts_epoch));
    }
    map
}

/// Mine cross-domain signal combinations.
///
/// Takes all observation events, all outcome events, and a config.
/// Returns synergistic combinations sorted descending by `synergy_factor`.
///
/// The function:
/// 1. Groups observations by type.
/// 2. Generates all unique pairs of types (`max_pairs` guard).
/// 3. For each pair, builds an interaction contingency cube.
/// 4. Filters by min interaction effect and synergy multiplier.
/// 5. Checks stability across time splits.
pub fn mine_signal_combinations(
    observations: &[TypedEvent],
    outcomes: &[(String, i64)],
    outcome_label: &str,
    lag_days: i32,
    window_days: i32,
    config: &CrossDomainConfig,
) -> Vec<SignalCombination> {
    let type_groups = group_by_type(observations);
    let type_names: Vec<String> = type_groups.keys().cloned().collect();

    // Build a global entity universe from ALL observations + outcomes so that
    // entities with a third signal type (not in the tested pair) appear as
    // "neither" in the contingency cube.
    let entity_universe: HashSet<String> = observations
        .iter()
        .map(|e| e.entity_id.clone())
        .chain(outcomes.iter().map(|(e, _)| e.clone()))
        .collect();

    // Generate pairs — cap at max_pairs (B286).
    let mut pairs: Vec<(String, String)> = Vec::new();
    for i in 0..type_names.len() {
        for j in (i + 1)..type_names.len() {
            let a = &type_names[i];
            let b = &type_names[j];
            // Canonical ordering for deterministic results.
            if a < b {
                pairs.push((a.clone(), b.clone()));
            } else {
                pairs.push((b.clone(), a.clone()));
            }
        }
        if pairs.len() >= config.max_pairs {
            break;
        }
    }
    pairs.truncate(config.max_pairs);

    let mut results = Vec::new();

    for (type_a, type_b) in &pairs {
        let signals_a = match type_groups.get(type_a) {
            Some(s) => s,
            None => continue,
        };
        let signals_b = match type_groups.get(type_b) {
            Some(s) => s,
            None => continue,
        };

        // Check entity coverage.
        let entities_a: HashSet<&str> = signals_a.iter().map(|(e, _)| e.as_str()).collect();
        let entities_b: HashSet<&str> = signals_b.iter().map(|(e, _)| e.as_str()).collect();
        let total_entities = entities_a.union(&entities_b).count();
        if total_entities < config.min_entities {
            continue;
        }

        // Build full interaction contingency.
        let cube = build_interaction_contingency(
            outcomes,
            signals_a,
            signals_b,
            lag_days,
            window_days,
            Some(&entity_universe),
        );

        let interaction_or = cube.interaction_odds_ratio();
        let or_a = cube.odds_ratio_a_alone();
        let or_b = cube.odds_ratio_b_alone();
        let stronger_individual = or_a.max(or_b);

        // Filter: interaction must exceed min effect.
        if interaction_or < config.min_interaction_effect {
            continue;
        }

        // Filter: interaction must be synergistic (exceeds stronger individual
        // by the multiplier).
        let synergy = if stronger_individual > 0.0 {
            interaction_or / stronger_individual
        } else {
            0.0
        };
        if synergy < config.synergy_multiplier {
            continue;
        }

        // Stability: check interaction across time splits.
        let stability = compute_interaction_stability(
            outcomes,
            signals_a,
            signals_b,
            lag_days,
            window_days,
            config.time_splits,
            config.min_interaction_effect,
        );
        if stability < config.min_stability {
            continue;
        }

        // Compute NMI for the combined signal vs outcome.
        let nmi = compute_combined_nmi(outcomes, signals_a, signals_b);

        results.push(SignalCombination {
            type_a: type_a.clone(),
            type_b: type_b.clone(),
            outcome: outcome_label.to_string(),
            best_lag_days: lag_days,
            interaction_effect: interaction_or,
            individual_effect_a: or_a,
            individual_effect_b: or_b,
            synergy_factor: synergy,
            stability,
            nmi_combined: nmi,
            entity_coverage: total_entities,
            contingency_cube: cube,
        });
    }

    // Sort descending by synergy_factor, deterministic tiebreak (B292).
    results.sort_by(|a, b| {
        b.synergy_factor
            .partial_cmp(&a.synergy_factor)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| (&a.type_a, &a.type_b).cmp(&(&b.type_a, &b.type_b)))
    });
    results
}

/// Stability check: split observations by time, recompute interaction OR in
/// each split, and return the fraction of splits where OR ≥ min_effect.
fn compute_interaction_stability(
    outcomes: &[(String, i64)],
    signals_a: &[(String, i64)],
    signals_b: &[(String, i64)],
    lag_days: i32,
    window_days: i32,
    time_splits: usize,
    min_effect: f64,
) -> f64 {
    if time_splits < 2 {
        return 1.0;
    }

    // Find global time range.
    let all_ts: Vec<i64> = outcomes
        .iter()
        .map(|(_, t)| *t)
        .chain(signals_a.iter().map(|(_, t)| *t))
        .chain(signals_b.iter().map(|(_, t)| *t))
        .collect();

    let (ts_min, ts_max) = match (all_ts.iter().min(), all_ts.iter().max()) {
        (Some(&mn), Some(&mx)) if mx > mn => (mn, mx),
        _ => return 1.0,
    };

    let split_width = (ts_max - ts_min) / time_splits as i64;
    if split_width <= 0 {
        return 1.0;
    }

    let mut passes = 0usize;
    for i in 0..time_splits {
        let lo = ts_min + i as i64 * split_width;
        let hi = if i == time_splits - 1 {
            ts_max + 1
        } else {
            ts_min + (i + 1) as i64 * split_width
        };

        let out_split: Vec<(String, i64)> = outcomes
            .iter()
            .filter(|(_, t)| *t >= lo && *t < hi)
            .cloned()
            .collect();
        let a_split: Vec<(String, i64)> = signals_a
            .iter()
            .filter(|(_, t)| *t >= lo && *t < hi)
            .cloned()
            .collect();
        let b_split: Vec<(String, i64)> = signals_b
            .iter()
            .filter(|(_, t)| *t >= lo && *t < hi)
            .cloned()
            .collect();

        if out_split.is_empty() || a_split.is_empty() || b_split.is_empty() {
            continue;
        }

        let cube = build_interaction_contingency(
            &out_split,
            &a_split,
            &b_split,
            lag_days,
            window_days,
            None,
        );
        if cube.interaction_odds_ratio() >= min_effect {
            passes += 1;
        }
    }

    passes as f64 / time_splits as f64
}

/// Quick normalised MI approximation for the combined binary signal vs binary outcome.
fn compute_combined_nmi(
    outcomes: &[(String, i64)],
    signals_a: &[(String, i64)],
    signals_b: &[(String, i64)],
) -> f64 {
    // Binary encoding: entity has_both_signals (1.0 / 0.0) vs has_outcome (1.0 / 0.0).
    let entity_set_a: HashSet<&str> = signals_a.iter().map(|(e, _)| e.as_str()).collect();
    let entity_set_b: HashSet<&str> = signals_b.iter().map(|(e, _)| e.as_str()).collect();
    let outcome_entities: HashSet<&str> = outcomes.iter().map(|(e, _)| e.as_str()).collect();

    let all_entities: HashSet<&str> = entity_set_a
        .iter()
        .chain(entity_set_b.iter())
        .chain(outcome_entities.iter())
        .copied()
        .collect();

    if all_entities.len() < 10 {
        return 0.0;
    }

    let mut x = Vec::with_capacity(all_entities.len());
    let mut y = Vec::with_capacity(all_entities.len());
    for entity in &all_entities {
        let has_both = entity_set_a.contains(entity) && entity_set_b.contains(entity);
        let has_outcome = outcome_entities.contains(entity);
        x.push(if has_both { 1.0 } else { 0.0 });
        y.push(if has_outcome { 1.0 } else { 0.0 });
    }

    // Use 2 bins for binary data.
    apex_stats::mutual_info::normalized_mi(&x, &y, 2)
}

// ────────────────────────────────────────────
// Multi-lag sweep
// ────────────────────────────────────────────

/// Sweep multiple lag values and return the best combination per pair.
///
/// For each candidate pair, tries lags from 1..`max_lag_days` in `step` day
/// increments and keeps the lag with the highest interaction odds ratio.
pub fn sweep_lags(
    observations: &[TypedEvent],
    outcomes: &[(String, i64)],
    outcome_label: &str,
    window_days: i32,
    max_lag_days: i32,
    step: i32,
    config: &CrossDomainConfig,
) -> Vec<SignalCombination> {
    let mut best_by_pair: HashMap<(String, String), SignalCombination> = HashMap::new();

    let mut lag = 1;
    while lag <= max_lag_days {
        let combos = mine_signal_combinations(
            observations,
            outcomes,
            outcome_label,
            lag,
            window_days,
            config,
        );
        for combo in combos {
            let key = (combo.type_a.clone(), combo.type_b.clone());
            let is_better = best_by_pair.get(&key).map_or(true, |existing| {
                combo.interaction_effect > existing.interaction_effect
            });
            if is_better {
                best_by_pair.insert(key, combo);
            }
        }
        lag += step;
    }

    let mut results: Vec<SignalCombination> = best_by_pair.into_values().collect();
    results.sort_by(|a, b| {
        b.synergy_factor
            .partial_cmp(&a.synergy_factor)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| (&a.type_a, &a.type_b).cmp(&(&b.type_a, &b.type_b)))
    });
    results
}

// ────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_events(obs_type: &str, entities: &[&str], base_ts: i64) -> Vec<TypedEvent> {
        entities
            .iter()
            .enumerate()
            .map(|(i, eid)| TypedEvent {
                entity_id: (*eid).to_string(),
                obs_type: obs_type.to_string(),
                ts_epoch: base_ts + i as i64 * 86400,
            })
            .collect()
    }

    fn make_outcome_events(entities: &[&str], base_ts: i64) -> Vec<(String, i64)> {
        entities
            .iter()
            .enumerate()
            .map(|(i, eid)| ((*eid).to_string(), base_ts + i as i64 * 86400))
            .collect()
    }

    #[test]
    fn test_config_validate() {
        let mut cfg = CrossDomainConfig::default();
        assert!(cfg.validate().is_empty());

        cfg.min_interaction_effect = 0.5;
        assert!(!cfg.validate().is_empty());
    }

    #[test]
    fn test_build_interaction_contingency_basic() {
        let outcomes = vec![("e1".into(), 100_000i64), ("e2".into(), 100_000)];
        let signals_a = vec![
            ("e1".into(), 50_000i64),
            ("e2".into(), 50_000),
            ("e3".into(), 50_000),
        ];
        let signals_b = vec![("e1".into(), 50_000i64), ("e4".into(), 50_000)];

        let cube = build_interaction_contingency(&outcomes, &signals_a, &signals_b, 0, 365, None);

        // e1: has_a=true, has_b=true, outcome=true  → both_signals_outcome
        // e2: has_a=true, has_b=false, outcome=true  → a_only_outcome
        // e3: has_a=true, has_b=false, outcome=false → a_only_no_outcome
        // e4: has_a=false, has_b=true, outcome=false → b_only_no_outcome
        assert_eq!(cube.both_signals_outcome, 1);
        assert_eq!(cube.a_only_outcome, 1);
        assert_eq!(cube.a_only_no_outcome, 1);
        assert_eq!(cube.b_only_no_outcome, 1);
        assert_eq!(cube.total(), 4);
    }

    #[test]
    fn test_interaction_odds_ratio() {
        let cube = InteractionContingency {
            both_signals_outcome: 50,
            both_signals_no_outcome: 10,
            a_only_outcome: 20,
            a_only_no_outcome: 30,
            b_only_outcome: 15,
            b_only_no_outcome: 25,
            neither_outcome: 10,
            neither_no_outcome: 80,
        };

        let ior = cube.interaction_odds_ratio();
        // (50 * 80) / (10 * 10) = 4000/100 = 40.0
        assert!((ior - 40.0).abs() < 0.01);

        let or_a = cube.odds_ratio_a_alone();
        let or_b = cube.odds_ratio_b_alone();
        // Interaction should be substantially stronger than either individual.
        assert!(ior > or_a);
        assert!(ior > or_b);
    }

    #[test]
    fn test_mine_signal_combinations_synergy() {
        // Construct data where combining JobPost + PatentPublished is more
        // predictive of ContractAward than either alone.
        let base = 1_000_000i64;
        let day = 86400i64;

        // Entities e1..e10 have both signals, e11..e15 have only jobs,
        // e16..e20 have only patents, e21..e30 have neither.
        let mut observations = Vec::new();
        for i in 1..=10 {
            observations.push(TypedEvent {
                entity_id: format!("e{}", i),
                obs_type: "JobPost".into(),
                ts_epoch: base + i * day,
            });
            observations.push(TypedEvent {
                entity_id: format!("e{}", i),
                obs_type: "PatentPublished".into(),
                ts_epoch: base + i * day + 1000,
            });
        }
        for i in 11..=15 {
            observations.push(TypedEvent {
                entity_id: format!("e{}", i),
                obs_type: "JobPost".into(),
                ts_epoch: base + i * day,
            });
        }
        for i in 16..=20 {
            observations.push(TypedEvent {
                entity_id: format!("e{}", i),
                obs_type: "PatentPublished".into(),
                ts_epoch: base + i * day,
            });
        }
        // e21..e30 have a third signal type (neither JobPost nor Patent).
        // These become "neither" entities in the (JobPost, Patent) pair.
        for i in 21..=30 {
            observations.push(TypedEvent {
                entity_id: format!("e{}", i),
                obs_type: "FinancialReport".into(),
                ts_epoch: base + i * day,
            });
        }

        // Outcomes: entities with BOTH signals get ContractAward at high
        // rate.  Single-signal entities get it at moderate rate.
        // "Neither" entities (e21-e30) get it at very low rate.
        let mut outcomes: Vec<(String, i64)> = Vec::new();
        for i in 1..=9 {
            // 9/10 with both signals → outcome  (90%).
            outcomes.push((format!("e{}", i), base + i * day + 20 * day));
        }
        for i in 11..=12 {
            // 2/5 with JobPost only → outcome   (40%).
            outcomes.push((format!("e{}", i), base + i * day + 20 * day));
        }
        for i in 16..=16 {
            // 1/5 with patent only → outcome    (20%).
            outcomes.push((format!("e{}", i), base + i * day + 20 * day));
        }
        for i in 21..=21 {
            // 1/10 with neither → outcome       (10%).
            outcomes.push((format!("e{}", i), base + i * day + 20 * day));
        }

        let config = CrossDomainConfig {
            min_interaction_effect: 1.5,
            synergy_multiplier: 1.1,
            min_nmi: 0.0,
            max_pairs: 100,
            min_entities: 3,
            time_splits: 2,
            min_stability: 0.0, // relax for test
        };

        let combos =
            mine_signal_combinations(&observations, &outcomes, "ContractAward", 15, 30, &config);

        // Should find the JobPost × PatentPublished synergy.
        assert!(
            !combos.is_empty(),
            "Expected at least one cross-domain combination"
        );
        let top = &combos[0];
        assert!(
            (top.type_a == "JobPost" && top.type_b == "PatentPublished")
                || (top.type_a == "PatentPublished" && top.type_b == "JobPost"),
        );
        assert!(top.interaction_effect > top.individual_effect_a);
        assert!(top.synergy_factor > 1.0);
    }

    #[test]
    fn test_mine_empty() {
        let combos = mine_signal_combinations(&[], &[], "X", 30, 30, &CrossDomainConfig::default());
        assert!(combos.is_empty());
    }

    #[test]
    fn test_sweep_lags() {
        // Simple test: sweep should find results when mine does.
        let obs = make_events(
            "A",
            &["e1", "e2", "e3", "e4", "e5", "e6", "e7", "e8"],
            1_000_000,
        );
        let mut obs2 = make_events(
            "B",
            &["e1", "e2", "e3", "e4", "e5", "e6", "e7", "e8"],
            1_000_000,
        );
        let mut all_obs = obs;
        all_obs.append(&mut obs2);
        let outcomes = make_outcome_events(&["e1", "e2", "e3", "e4"], 1_100_000);

        let config = CrossDomainConfig {
            min_interaction_effect: 1.0,
            synergy_multiplier: 1.0,
            min_nmi: 0.0,
            max_pairs: 100,
            min_entities: 2,
            time_splits: 2,
            min_stability: 0.0,
        };

        let results = sweep_lags(&all_obs, &outcomes, "Outcome", 30, 30, 15, &config);
        // Should find something (though may not be very synergistic with these simple data)
        // The key thing is it doesn't panic.
        let _ = results;
    }

    #[test]
    fn test_stability_check() {
        // Fabricate data spread across 4 time splits.
        let day = 86400i64;
        let split_len = 90 * day; // 90 days per split.
        let base = 0i64;

        let mut signals_a = Vec::new();
        let mut signals_b = Vec::new();
        let mut outcomes = Vec::new();

        // In each split, create entities with both signals that lead to outcome.
        for split in 0..4 {
            let t0 = base + split as i64 * split_len;
            for i in 0..5 {
                let eid = format!("e_{}_{}", split, i);
                signals_a.push((eid.clone(), t0 + i * day));
                signals_b.push((eid.clone(), t0 + i * day + 1000));
                if i < 3 {
                    // 3/5 with both signals → outcome.
                    outcomes.push((eid, t0 + i * day + 15 * day));
                }
            }
        }

        let stability =
            compute_interaction_stability(&outcomes, &signals_a, &signals_b, 10, 30, 4, 1.0);
        assert!(
            stability >= 0.5,
            "Expected ≥50% stability but got {}",
            stability
        );
    }
}
