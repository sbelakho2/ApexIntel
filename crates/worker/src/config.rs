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
pub static LLM_MAX_RETRIES: LazyLock<u32> = LazyLock::new(|| {
    parse_env_with_warning("LLM_MAX_RETRIES", 3)
});

/// LLM request timeout in seconds.
/// Override with `LLM_TIMEOUT_SECS` env var.
pub static LLM_TIMEOUT_SECS: LazyLock<u64> = LazyLock::new(|| {
    parse_env_with_warning("LLM_TIMEOUT_SECS", 180u64).clamp(10, 600)
});

/// LLM request timeout as Duration.
pub fn llm_timeout() -> Duration {
    Duration::from_secs(*LLM_TIMEOUT_SECS)
}

/// Title deduplication Jaccard similarity threshold.
/// If two titles have Jaccard similarity >= this value, they are considered duplicates.
/// Override with `DEDUP_TITLE_THRESHOLD` env var.
pub static DEDUP_TITLE_THRESHOLD: LazyLock<f64> = LazyLock::new(|| {
    parse_f64_env_clamped("DEDUP_TITLE_THRESHOLD", 0.78, 0.0, 1.0)
});

/// Summary deduplication Jaccard similarity threshold.
/// Override with `DEDUP_SUMMARY_THRESHOLD` env var.
pub static DEDUP_SUMMARY_THRESHOLD: LazyLock<f64> = LazyLock::new(|| {
    parse_f64_env_clamped("DEDUP_SUMMARY_THRESHOLD", 0.72, 0.0, 1.0)
});

/// Freshness half-life in days for exponential decay.
/// Evidence older than this number of days has half the freshness weight.
/// Override with `FRESHNESS_HALFLIFE_DAYS` env var.
pub static FRESHNESS_HALFLIFE_DAYS: LazyLock<u32> = LazyLock::new(|| {
    parse_env_with_warning("FRESHNESS_HALFLIFE_DAYS", 30u32).max(1)
});

/// Freshness half-life as f64 for calculations.
pub fn freshness_halflife_days() -> f64 {
    *FRESHNESS_HALFLIFE_DAYS as f64
}

/// Entity fuzzy match threshold.
/// Names with similarity >= this value are considered matches.
/// Override with `ENTITY_MATCH_THRESHOLD` env var.
pub static ENTITY_MATCH_THRESHOLD: LazyLock<f64> = LazyLock::new(|| {
    parse_f64_env_clamped("ENTITY_MATCH_THRESHOLD", 0.6, 0.0, 1.0)
});

/// PELT change-point detection default penalty.
/// Higher values produce fewer change points; lower values produce more.
/// Override with `PELT_DEFAULT_PENALTY` env var.
pub static PELT_DEFAULT_PENALTY: LazyLock<f64> = LazyLock::new(|| {
    parse_f64_env_clamped("PELT_DEFAULT_PENALTY", 3.0, 0.1, 100.0)
});

/// Minimum quality threshold for insight acceptance.
/// Insights below this confidence are rejected.
/// Override with `MIN_QUALITY_THRESHOLD` env var.
pub static MIN_QUALITY_THRESHOLD: LazyLock<f64> = LazyLock::new(|| {
    parse_f64_env_clamped("MIN_QUALITY_THRESHOLD", 0.30, 0.0, 1.0)
});

/// Maximum evidence signals to include in LLM prompts.
/// Override with `LLM_MAX_EVIDENCE_SIGNALS` env var.
pub static LLM_MAX_EVIDENCE_SIGNALS: LazyLock<usize> = LazyLock::new(|| {
    parse_env_with_warning("LLM_MAX_EVIDENCE_SIGNALS", 12usize).clamp(1, 100)
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
