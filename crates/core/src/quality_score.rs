//! Observation quality score calculator.
//!
//! Each observation gets a quality score (0.0–1.0) computed from:
//! - Source reliability (crawl source trustworthiness)
//! - Extraction confidence (NLP/parsing confidence)
//! - Freshness (age decay — newer = higher)
//!
//! Low-quality observations are deprioritized in recipe evaluation.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ─── Source reliability tiers ───────────────────────────────────────────

/// Reliability rating for a data source.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum SourceReliability {
    /// Verified official sources (SEC filings, government registries)
    Official,
    /// Established media, company websites
    Established,
    /// Industry publications, trade press
    TradePress,
    /// Social media, forums
    Social,
    /// Scraped data of uncertain origin
    Unknown,
}

impl SourceReliability {
    /// Numeric score 0.0–1.0.
    pub fn score(&self) -> f64 {
        match self {
            Self::Official => 1.0,
            Self::Established => 0.85,
            Self::TradePress => 0.70,
            Self::Social => 0.45,
            Self::Unknown => 0.25,
        }
    }

    /// Classify a source URL into a reliability tier.
    pub fn from_url(url: &str) -> Self {
        let lower = url.to_lowercase();
        if lower.contains("sec.gov")
            || lower.contains("edinet")
            || lower.contains("registre")
            || lower.contains("innopi.tn")
            || lower.contains("data.gov")
        {
            Self::Official
        } else if lower.contains("reuters.com")
            || lower.contains("bloomberg.com")
            || lower.contains(".gov.")
            || lower.contains("linkedin.com")
        {
            Self::Established
        } else if lower.contains("eetimes.com")
            || lower.contains("edn.com")
            || lower.contains("electronicsweekly")
            || lower.contains("semi.org")
        {
            Self::TradePress
        } else if lower.contains("twitter.com")
            || lower.contains("x.com")
            || lower.contains("facebook.com")
            || lower.contains("reddit.com")
            || lower.contains("discord")
        {
            Self::Social
        } else {
            Self::Unknown
        }
    }
}

// ─── Quality score computation ──────────────────────────────────────────

/// Quality score configuration.
#[derive(Debug, Clone)]
pub struct QualityConfig {
    /// Weight for source reliability (0.0–1.0)
    pub source_weight: f64,
    /// Weight for extraction confidence (0.0–1.0)
    pub confidence_weight: f64,
    /// Weight for freshness (0.0–1.0)
    pub freshness_weight: f64,
    /// Half-life for freshness decay (days)
    pub freshness_halflife_days: f64,
    /// Minimum quality score to include in recipe evaluation
    pub min_quality_threshold: f64,
}

impl Default for QualityConfig {
    fn default() -> Self {
        Self {
            source_weight: 0.40,
            confidence_weight: 0.35,
            freshness_weight: 0.25,
            freshness_halflife_days: 30.0,
            min_quality_threshold: 0.30,
        }
    }
}

/// Input data for quality computation.
#[derive(Debug, Clone)]
pub struct ObservationInput {
    pub source_url: String,
    pub extraction_confidence: f64,
    pub observed_at: DateTime<Utc>,
    /// Override source reliability (if known)
    pub source_reliability: Option<SourceReliability>,
}

/// Computed quality score with breakdown.
#[derive(Debug, Clone, Serialize)]
pub struct QualityScore {
    pub total: f64,
    pub source_score: f64,
    pub confidence_score: f64,
    pub freshness_score: f64,
    pub source_reliability: String,
    pub passes_threshold: bool,
}

/// Compute the quality score for an observation.
pub fn compute_quality(
    input: &ObservationInput,
    config: &QualityConfig,
    now: DateTime<Utc>,
) -> QualityScore {
    // Source reliability
    let reliability = input
        .source_reliability
        .unwrap_or_else(|| SourceReliability::from_url(&input.source_url));
    let source_score = reliability.score();

    // Extraction confidence (clamp to 0–1)
    let confidence_score = input.extraction_confidence.clamp(0.0, 1.0);

    // Freshness — exponential decay with half-life
    let age_days = (now - input.observed_at).num_hours() as f64 / 24.0;
    let freshness_score = if age_days <= 0.0 {
        1.0
    } else {
        0.5_f64.powf(age_days / config.freshness_halflife_days)
    };

    // Weighted total
    let total = (source_score * config.source_weight
        + confidence_score * config.confidence_weight
        + freshness_score * config.freshness_weight)
        .clamp(0.0, 1.0);

    QualityScore {
        total,
        source_score,
        confidence_score,
        freshness_score,
        source_reliability: format!("{:?}", reliability),
        passes_threshold: total >= config.min_quality_threshold,
    }
}

/// Batch-compute quality scores and sort by descending quality.
pub fn rank_observations(
    inputs: &[ObservationInput],
    config: &QualityConfig,
    now: DateTime<Utc>,
) -> Vec<(usize, QualityScore)> {
    let mut scored: Vec<(usize, QualityScore)> = inputs
        .iter()
        .enumerate()
        .map(|(i, input)| (i, compute_quality(input, config, now)))
        .collect();
    scored.sort_by(|a, b| b.1.total.partial_cmp(&a.1.total).unwrap_or(std::cmp::Ordering::Equal));
    scored
}

/// Filter observations that pass the quality threshold.
pub fn filter_quality(
    inputs: &[ObservationInput],
    config: &QualityConfig,
    now: DateTime<Utc>,
) -> Vec<(usize, QualityScore)> {
    rank_observations(inputs, config, now)
        .into_iter()
        .filter(|(_, q)| q.passes_threshold)
        .collect()
}

/// SQL to update quality_score column on observations.
pub fn update_quality_sql() -> &'static str {
    r#"
    UPDATE observations SET quality_score = (
        CASE
            WHEN provenance->>'source_reliability' = 'Official' THEN 1.0
            WHEN provenance->>'source_reliability' = 'Established' THEN 0.85
            WHEN provenance->>'source_reliability' = 'TradePress' THEN 0.70
            WHEN provenance->>'source_reliability' = 'Social' THEN 0.45
            ELSE 0.25
        END * 0.40
        + COALESCE((provenance->>'extraction_confidence')::float, 0.5) * 0.35
        + POWER(0.5, EXTRACT(EPOCH FROM (NOW() - observed_at)) / 86400.0 / 30.0) * 0.25
    )
    WHERE quality_score IS NULL OR quality_score = 0
    "#
}

// ─── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn make_input(url: &str, confidence: f64, days_ago: i64) -> ObservationInput {
        ObservationInput {
            source_url: url.into(),
            extraction_confidence: confidence,
            observed_at: Utc::now() - Duration::days(days_ago),
            source_reliability: None,
        }
    }

    #[test]
    fn test_source_reliability_classification() {
        assert_eq!(
            SourceReliability::from_url("https://www.sec.gov/cgi-bin/browse-edgar"),
            SourceReliability::Official
        );
        assert_eq!(
            SourceReliability::from_url("https://www.reuters.com/business"),
            SourceReliability::Established
        );
        assert_eq!(
            SourceReliability::from_url("https://www.eetimes.com/"),
            SourceReliability::TradePress
        );
        assert_eq!(
            SourceReliability::from_url("https://twitter.com/somebody"),
            SourceReliability::Social
        );
        assert_eq!(
            SourceReliability::from_url("https://unknown-blog.com"),
            SourceReliability::Unknown
        );
    }

    #[test]
    fn test_high_quality_official_fresh() {
        let config = QualityConfig::default();
        let input = make_input("https://sec.gov/filing", 0.95, 1);
        let score = compute_quality(&input, &config, Utc::now());
        assert!(score.total > 0.85);
        assert!(score.passes_threshold);
    }

    #[test]
    fn test_low_quality_old_social() {
        let config = QualityConfig::default();
        let input = make_input("https://twitter.com/xyz", 0.3, 90);
        let score = compute_quality(&input, &config, Utc::now());
        assert!(score.total < 0.40);
    }

    #[test]
    fn test_freshness_decay() {
        let config = QualityConfig::default();
        let fresh = make_input("https://sec.gov/filing", 0.9, 0);
        let old = make_input("https://sec.gov/filing", 0.9, 60);
        let fresh_score = compute_quality(&fresh, &config, Utc::now());
        let old_score = compute_quality(&old, &config, Utc::now());
        assert!(fresh_score.freshness_score > old_score.freshness_score);
        assert!(fresh_score.total > old_score.total);
    }

    #[test]
    fn test_ranking() {
        let config = QualityConfig::default();
        let inputs = vec![
            make_input("https://twitter.com/x", 0.3, 90),
            make_input("https://sec.gov/filing", 0.95, 1),
            make_input("https://reuters.com/article", 0.7, 15),
        ];
        let ranked = rank_observations(&inputs, &config, Utc::now());
        assert_eq!(ranked[0].0, 1); // SEC filing should rank first
        assert_eq!(ranked[2].0, 0); // Twitter should rank last
    }

    #[test]
    fn test_threshold_filtering() {
        let config = QualityConfig {
            min_quality_threshold: 0.50,
            ..QualityConfig::default()
        };
        let inputs = vec![
            make_input("https://twitter.com/x", 0.1, 120), // low quality
            make_input("https://sec.gov/filing", 0.95, 1), // high quality
        ];
        let filtered = filter_quality(&inputs, &config, Utc::now());
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].0, 1);
    }

    #[test]
    fn test_sql_not_empty() {
        assert!(update_quality_sql().contains("UPDATE observations"));
    }
}
