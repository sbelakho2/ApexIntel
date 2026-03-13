#![allow(dead_code)]

//! Runtime configuration constants.
//!
//! All constants can be overridden via environment variables.
//!
//! This module centralizes hardcoded numerical constants and makes them configurable
//! at runtime via environment variables while providing sensible defaults.

use std::sync::LazyLock;
use std::time::Duration;

/// Maximum number of LLM retries before giving up.
/// Override with `LLM_MAX_RETRIES` env var.
pub static LLM_MAX_RETRIES: LazyLock<u32> = LazyLock::new(|| {
    std::env::var("LLM_MAX_RETRIES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3)
});

/// LLM request timeout in seconds.
/// Override with `LLM_TIMEOUT_SECS` env var.
pub static LLM_TIMEOUT_SECS: LazyLock<u64> = LazyLock::new(|| {
    std::env::var("LLM_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(180)
});

/// LLM request timeout as Duration.
pub fn llm_timeout() -> Duration {
    Duration::from_secs(*LLM_TIMEOUT_SECS)
}

/// Title deduplication Jaccard similarity threshold.
/// If two titles have Jaccard similarity >= this value, they are considered duplicates.
/// Override with `DEDUP_TITLE_THRESHOLD` env var.
pub static DEDUP_TITLE_THRESHOLD: LazyLock<f64> = LazyLock::new(|| {
    std::env::var("DEDUP_TITLE_THRESHOLD")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.78)
});

/// Summary deduplication Jaccard similarity threshold.
/// Override with `DEDUP_SUMMARY_THRESHOLD` env var.
pub static DEDUP_SUMMARY_THRESHOLD: LazyLock<f64> = LazyLock::new(|| {
    std::env::var("DEDUP_SUMMARY_THRESHOLD")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.72)
});

/// Freshness half-life in days for exponential decay.
/// Evidence older than this number of days has half the freshness weight.
/// Override with `FRESHNESS_HALFLIFE_DAYS` env var.
pub static FRESHNESS_HALFLIFE_DAYS: LazyLock<u32> = LazyLock::new(|| {
    std::env::var("FRESHNESS_HALFLIFE_DAYS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(30)
});

/// Freshness half-life as f64 for calculations.
pub fn freshness_halflife_days() -> f64 {
    *FRESHNESS_HALFLIFE_DAYS as f64
}

/// Entity fuzzy match threshold.
/// Names with similarity >= this value are considered matches.
/// Override with `ENTITY_MATCH_THRESHOLD` env var.
pub static ENTITY_MATCH_THRESHOLD: LazyLock<f64> = LazyLock::new(|| {
    std::env::var("ENTITY_MATCH_THRESHOLD")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.6)
});

/// PELT change-point detection default penalty.
/// Higher values produce fewer change points; lower values produce more.
/// Override with `PELT_DEFAULT_PENALTY` env var.
pub static PELT_DEFAULT_PENALTY: LazyLock<f64> = LazyLock::new(|| {
    std::env::var("PELT_DEFAULT_PENALTY")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3.0)
});

/// Minimum quality threshold for insight acceptance.
/// Insights below this confidence are rejected.
/// Override with `MIN_QUALITY_THRESHOLD` env var.
pub static MIN_QUALITY_THRESHOLD: LazyLock<f64> = LazyLock::new(|| {
    std::env::var("MIN_QUALITY_THRESHOLD")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.30)
});

/// Maximum evidence signals to include in LLM prompts.
/// Override with `LLM_MAX_EVIDENCE_SIGNALS` env var.
pub static LLM_MAX_EVIDENCE_SIGNALS: LazyLock<usize> = LazyLock::new(|| {
    std::env::var("LLM_MAX_EVIDENCE_SIGNALS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(12)
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
}
