#![allow(dead_code)]

//! Runtime configuration constants.
//!
//! All constants can be overridden via environment variables.
//!
//! This module centralizes hardcoded numerical constants and makes them configurable
//! at runtime via environment variables while providing sensible defaults.

use std::sync::LazyLock;
use std::time::Duration;

fn parse_env_with_warning<T: std::str::FromStr + std::fmt::Display>(key: &str, default: T) -> T {
    match std::env::var(key) {
        Ok(raw) => match raw.parse::<T>() {
            Ok(val) => val,
            Err(_) => {
                tracing::warn!(
                    env_var = key,
                    raw_value = %raw,
                    default_value = %default,
                    "invalid value for env var, using default"
                );
                default
            }
        },
        Err(_) => default,
    }
}

fn parse_f64_env_clamped(key: &str, default: f64, min: f64, max: f64) -> f64 {
    parse_env_with_warning(key, default).clamp(min, max)
}

/// Maximum number of LLM retries before giving up.
/// Override with `LLM_MAX_RETRIES` env var.
pub static LLM_MAX_RETRIES: LazyLock<u32> =
    LazyLock::new(|| parse_env_with_warning("LLM_MAX_RETRIES", 3));

/// LLM request timeout in seconds.
/// Override with `LLM_TIMEOUT_SECS` env var.
pub static LLM_TIMEOUT_SECS: LazyLock<u64> =
    LazyLock::new(|| parse_env_with_warning("LLM_TIMEOUT_SECS", 180u64).clamp(10, 600));

/// LLM request timeout as Duration.
pub fn llm_timeout() -> Duration {
    Duration::from_secs(*LLM_TIMEOUT_SECS)
}

/// Title deduplication Jaccard similarity threshold.
/// If two titles have Jaccard similarity >= this value, they are considered duplicates.
/// Override with `DEDUP_TITLE_THRESHOLD` env var.
pub static DEDUP_TITLE_THRESHOLD: LazyLock<f64> =
    LazyLock::new(|| parse_f64_env_clamped("DEDUP_TITLE_THRESHOLD", 0.65, 0.0, 1.0));

/// Summary deduplication Jaccard similarity threshold.
/// Override with `DEDUP_SUMMARY_THRESHOLD` env var.
pub static DEDUP_SUMMARY_THRESHOLD: LazyLock<f64> =
    LazyLock::new(|| parse_f64_env_clamped("DEDUP_SUMMARY_THRESHOLD", 0.60, 0.0, 1.0));

/// Freshness half-life in days for exponential decay.
/// Evidence older than this number of days has half the freshness weight.
/// Override with `FRESHNESS_HALFLIFE_DAYS` env var.
pub static FRESHNESS_HALFLIFE_DAYS: LazyLock<u32> =
    LazyLock::new(|| parse_env_with_warning("FRESHNESS_HALFLIFE_DAYS", 30u32).max(1));

/// Freshness half-life as f64 for calculations.
pub fn freshness_halflife_days() -> f64 {
    *FRESHNESS_HALFLIFE_DAYS as f64
}

/// Entity fuzzy match threshold.
/// Names with similarity >= this value are considered matches.
/// Override with `ENTITY_MATCH_THRESHOLD` env var.
pub static ENTITY_MATCH_THRESHOLD: LazyLock<f64> =
    LazyLock::new(|| parse_f64_env_clamped("ENTITY_MATCH_THRESHOLD", 0.6, 0.0, 1.0));

/// PELT change-point detection default penalty.
/// Higher values produce fewer change points; lower values produce more.
/// Override with `PELT_DEFAULT_PENALTY` env var.
pub static PELT_DEFAULT_PENALTY: LazyLock<f64> =
    LazyLock::new(|| parse_f64_env_clamped("PELT_DEFAULT_PENALTY", 3.0, 0.1, 100.0));

/// Minimum quality threshold for insight acceptance.
/// Insights below this confidence are rejected.
/// Override with `MIN_QUALITY_THRESHOLD` env var.
pub static MIN_QUALITY_THRESHOLD: LazyLock<f64> =
    LazyLock::new(|| parse_f64_env_clamped("MIN_QUALITY_THRESHOLD", 0.30, 0.0, 1.0));

/// Maximum evidence signals to include in LLM prompts.
/// Override with `LLM_MAX_EVIDENCE_SIGNALS` env var.
pub static LLM_MAX_EVIDENCE_SIGNALS: LazyLock<usize> =
    LazyLock::new(|| parse_env_with_warning("LLM_MAX_EVIDENCE_SIGNALS", 12usize).clamp(1, 100));

/// Maximum number of insights generated about a single entity within a rolling
/// 7-day window. Combined with the per-run cap and coverage-weighted ranking,
/// this prevents a handful of high-signal companies from monopolizing the
/// insight stream night after night.
/// Override with `WEEKLY_ENTITY_INSIGHT_BUDGET` env var.
pub static WEEKLY_ENTITY_INSIGHT_BUDGET: LazyLock<usize> =
    LazyLock::new(|| parse_env_with_warning("WEEKLY_ENTITY_INSIGHT_BUDGET", 8usize).clamp(1, 100));

/// Coverage-damping factor `k` for anti-repetition ranking. An entity that
/// already has `n` insights in the recent window receives a ranking multiplier
/// of `1 / (1 + k * n)`, so heavily-covered entities yield priority to
/// under-covered ones. Larger values dampen more aggressively; `0` disables it.
/// Override with `COVERAGE_DAMPING_FACTOR` env var.
pub static COVERAGE_DAMPING_FACTOR: LazyLock<f64> =
    LazyLock::new(|| parse_f64_env_clamped("COVERAGE_DAMPING_FACTOR", 0.2, 0.0, 5.0));

/// Ranking multiplier applied to demand-side entities (customers / prospects /
/// buyers) located in a *primary* sales market. Starz sells battery packs mostly
/// into Morocco, Tunisia and Egypt, so opportunity entities there are boosted.
/// Competitors and upstream suppliers are monitored globally and are never
/// affected by this weight. Override with `GEO_PRIMARY_DEMAND_WEIGHT`.
pub static GEO_PRIMARY_DEMAND_WEIGHT: LazyLock<f64> =
    LazyLock::new(|| parse_f64_env_clamped("GEO_PRIMARY_DEMAND_WEIGHT", 1.35, 1.0, 3.0));

/// Ranking multiplier applied to demand-side entities located in a *secondary*
/// sales market (the European Union). Starz has a smaller EU focus, so these
/// receive a mild boost above neutral. Override with `GEO_SECONDARY_DEMAND_WEIGHT`.
pub static GEO_SECONDARY_DEMAND_WEIGHT: LazyLock<f64> =
    LazyLock::new(|| parse_f64_env_clamped("GEO_SECONDARY_DEMAND_WEIGHT", 1.10, 0.5, 3.0));

/// Ranking multiplier applied to demand-side entities located *outside* every
/// target market. Starz does not sell packs outside Morocco, Tunisia, Egypt and
/// the EU, so off-target buyers are de-prioritised (never dropped — competitors
/// and suppliers there are still monitored at full weight). Override with
/// `GEO_OFFTARGET_DEMAND_WEIGHT`.
pub static GEO_OFFTARGET_DEMAND_WEIGHT: LazyLock<f64> =
    LazyLock::new(|| parse_f64_env_clamped("GEO_OFFTARGET_DEMAND_WEIGHT", 0.55, 0.05, 1.0));

/// Observation lookback window (in days) for the pattern-mining stage. Mining
/// needs a long history to detect *lagged* cross-signal correlations, so this is
/// far wider than the 24-hour analytics reporting window. Override with
/// `PATTERN_MINING_LOOKBACK_DAYS`.
pub static PATTERN_MINING_LOOKBACK_DAYS: LazyLock<i64> =
    LazyLock::new(|| parse_env_with_warning("PATTERN_MINING_LOOKBACK_DAYS", 120i64).clamp(14, 365));

/// Minimum number of events an observation-type stream must contain over the
/// lookback window to be eligible for mining. Streams below this are dropped to
/// avoid spurious correlations on sparse data. Override with
/// `PATTERN_MINING_MIN_EVENTS`.
pub static PATTERN_MINING_MIN_EVENTS: LazyLock<usize> =
    LazyLock::new(|| parse_env_with_warning("PATTERN_MINING_MIN_EVENTS", 8usize).clamp(5, 10_000));

/// Hard cap on the number of observation rows loaded into memory for a single
/// mining run (oldest first). Bounds memory and query time on large histories.
/// Override with `PATTERN_MINING_MAX_OBSERVATIONS`.
pub static PATTERN_MINING_MAX_OBSERVATIONS: LazyLock<i64> = LazyLock::new(|| {
    parse_env_with_warning("PATTERN_MINING_MAX_OBSERVATIONS", 200_000i64).clamp(10_000, 5_000_000)
});

/// Maximum number of ranked, statistically-robust candidates carried forward to
/// LLM hypothesis generation per run. Override with
/// `PATTERN_MINING_MAX_CANDIDATES`.
pub static PATTERN_MINING_MAX_CANDIDATES: LazyLock<usize> =
    LazyLock::new(|| parse_env_with_warning("PATTERN_MINING_MAX_CANDIDATES", 12usize).clamp(1, 200));

/// Benjamini-Hochberg false-discovery-rate q-value ceiling for mined candidates.
/// Candidates with q above this are discarded to control multiple-comparison
/// false positives across the enumerated signal pairs. Override with
/// `PATTERN_MINING_MAX_Q`.
pub static PATTERN_MINING_MAX_Q: LazyLock<f64> =
    LazyLock::new(|| parse_f64_env_clamped("PATTERN_MINING_MAX_Q", 0.10, 0.0001, 1.0));

// ---------------------------------------------------------------------------
// Statistical miner gates (`apex_learning::miner::MinerConfig`).
//
// Defaults are byte-for-byte identical to `MinerConfig::default()` and are
// intentionally strict: they exist to suppress false-positive correlations, not
// to be relaxed casually. They are externalised here purely so operators can
// *calibrate* sensitivity (and run principled diagnostics) without recompiling
// the worker. Each value is clamped to the valid range enforced by
// `MinerConfig::validate()`.
// ---------------------------------------------------------------------------

/// Maximum causal lag, in calendar days, evaluated by the miner's lag sweep.
/// Default `90`. Override with `PATTERN_MINING_MAX_LAG_DAYS`.
pub static PATTERN_MINING_MAX_LAG_DAYS: LazyLock<i64> = LazyLock::new(|| {
    parse_env_with_warning("PATTERN_MINING_MAX_LAG_DAYS", 90i64).clamp(1, 365)
});

/// Minimum odds-ratio (effect size) a candidate must clear. Default `1.5`.
/// Must exceed `1.0` (an effect at or below baseline is no effect). Override
/// with `PATTERN_MINING_MIN_EFFECT`.
pub static PATTERN_MINING_MIN_EFFECT: LazyLock<f64> =
    LazyLock::new(|| parse_f64_env_clamped("PATTERN_MINING_MIN_EFFECT", 1.5, 1.0001, 100.0));

/// Maximum Fisher-exact p-value a candidate may have. Default `0.01`. Override
/// with `PATTERN_MINING_MAX_P`.
pub static PATTERN_MINING_MAX_P: LazyLock<f64> =
    LazyLock::new(|| parse_f64_env_clamped("PATTERN_MINING_MAX_P", 0.01, 0.0001, 1.0));

/// Minimum cross-split stability (0–1) a candidate must hold. Default `0.6`.
/// Override with `PATTERN_MINING_MIN_STABILITY`.
pub static PATTERN_MINING_MIN_STABILITY: LazyLock<f64> =
    LazyLock::new(|| parse_f64_env_clamped("PATTERN_MINING_MIN_STABILITY", 0.6, 0.0, 1.0));

/// Number of chronological splits used to measure stability. Default `4`.
/// At least `2` are required for cross-validation. Override with
/// `PATTERN_MINING_TIME_SPLITS`.
pub static PATTERN_MINING_TIME_SPLITS: LazyLock<usize> = LazyLock::new(|| {
    parse_env_with_warning("PATTERN_MINING_TIME_SPLITS", 4usize).clamp(2, 52)
});

/// Minimum distinct-entity count required for a valid contingency table.
/// Default `5`. Override with `PATTERN_MINING_ENTITY_MIN_COUNT`.
pub static PATTERN_MINING_ENTITY_MIN_COUNT: LazyLock<usize> = LazyLock::new(|| {
    parse_env_with_warning("PATTERN_MINING_ENTITY_MIN_COUNT", 5usize).clamp(1, 10_000)
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sensible() {
        assert_eq!(*LLM_MAX_RETRIES, 3);
        assert_eq!(*LLM_TIMEOUT_SECS, 180);
        assert!(*DEDUP_TITLE_THRESHOLD > 0.5 && *DEDUP_TITLE_THRESHOLD < 1.0);
        assert!(*DEDUP_SUMMARY_THRESHOLD > 0.5 && *DEDUP_SUMMARY_THRESHOLD < 1.0);
        assert!(*FRESHNESS_HALFLIFE_DAYS > 0);
        assert!(*ENTITY_MATCH_THRESHOLD > 0.0 && *ENTITY_MATCH_THRESHOLD < 1.0);
        assert!(*PELT_DEFAULT_PENALTY > 0.0);
        assert!(*MIN_QUALITY_THRESHOLD > 0.0 && *MIN_QUALITY_THRESHOLD < 1.0);
        assert!(*WEEKLY_ENTITY_INSIGHT_BUDGET >= 1);
        assert!(*COVERAGE_DAMPING_FACTOR >= 0.0 && *COVERAGE_DAMPING_FACTOR <= 5.0);
        // Geographic demand weighting: primary > secondary > neutral > off-target.
        assert!(*GEO_PRIMARY_DEMAND_WEIGHT >= 1.0);
        assert!(*GEO_SECONDARY_DEMAND_WEIGHT >= 0.5);
        assert!(*GEO_OFFTARGET_DEMAND_WEIGHT > 0.0 && *GEO_OFFTARGET_DEMAND_WEIGHT <= 1.0);
        assert!(*GEO_PRIMARY_DEMAND_WEIGHT >= *GEO_SECONDARY_DEMAND_WEIGHT);
        assert!(*GEO_SECONDARY_DEMAND_WEIGHT >= *GEO_OFFTARGET_DEMAND_WEIGHT);
        // Pattern-mining tunables.
        assert!(*PATTERN_MINING_LOOKBACK_DAYS >= 14 && *PATTERN_MINING_LOOKBACK_DAYS <= 365);
        assert!(*PATTERN_MINING_MIN_EVENTS >= 5);
        assert!(*PATTERN_MINING_MAX_OBSERVATIONS >= 10_000);
        assert!(*PATTERN_MINING_MAX_CANDIDATES >= 1);
        assert!(*PATTERN_MINING_MAX_Q > 0.0 && *PATTERN_MINING_MAX_Q <= 1.0);
        // Statistical miner gates must default to the strict library defaults.
        assert_eq!(*PATTERN_MINING_MAX_LAG_DAYS, 90);
        assert!((*PATTERN_MINING_MIN_EFFECT - 1.5).abs() < 1e-9);
        assert!(*PATTERN_MINING_MIN_EFFECT > 1.0);
        assert!((*PATTERN_MINING_MAX_P - 0.01).abs() < 1e-9);
        assert!(*PATTERN_MINING_MAX_P > 0.0 && *PATTERN_MINING_MAX_P <= 1.0);
        assert!((*PATTERN_MINING_MIN_STABILITY - 0.6).abs() < 1e-9);
        assert_eq!(*PATTERN_MINING_TIME_SPLITS, 4);
        assert!(*PATTERN_MINING_TIME_SPLITS >= 2);
        assert_eq!(*PATTERN_MINING_ENTITY_MIN_COUNT, 5);
    }

    #[test]
    fn llm_timeout_returns_duration() {
        let timeout = llm_timeout();
        assert_eq!(timeout.as_secs(), *LLM_TIMEOUT_SECS);
    }

    #[test]
    fn parse_env_with_warning_returns_default_for_missing() {
        // Uses a key that won't exist in test environment
        let val = parse_env_with_warning("__NONEXISTENT_TEST_KEY_XYZ__", 42u32);
        assert_eq!(val, 42);
    }

    #[test]
    fn parse_f64_env_clamped_clamps_to_range() {
        // With no env var, default should be returned and clamped
        let val = parse_f64_env_clamped("__NONEXISTENT_F64__", 0.5, 0.0, 1.0);
        assert!((val - 0.5).abs() < 1e-10);
    }

    #[test]
    fn evidence_signals_within_bounds() {
        assert!(*LLM_MAX_EVIDENCE_SIGNALS >= 1);
        assert!(*LLM_MAX_EVIDENCE_SIGNALS <= 100);
    }
}
