//! Reusable evidence-quality model (audit P1-10).
//!
//! The ten dimensions below are the shared semantics for "how good is this
//! evidence set?", so consumers (source-coverage reporting, insight memos,
//! warning grounding) stop inventing per-feature thresholds:
//!
//! * `evidence_count` — number of evidence records in the set.
//! * `independent_origin_count` — distinct origins (source id or URL host);
//!   two records from the same domain never count as corroboration.
//! * `primary_source_count` — records whose source is a primary/official one
//!   (filings, registries, courts) rather than commentary.
//! * `source_type_diversity` — distinct source types / evidence count.
//! * `geographic_diversity` — distinct regions / evidence count.
//! * `freshness` — mean exponential age decay (`exp(-age_days / 30)`), so a
//!   set of month-old records scores ~0.37 regardless of who collected it.
//! * `contradiction_ratio` — relevance-weighted contradicting share.
//! * `corroboration_score` — independent support relative to the full set.
//! * `coverage_completeness` — share of the caller's expected coverage
//!   dimensions that the evidence set covers.
//! * `parser_confidence` — mean parser confidence of the records that carry
//!   one (0.5, a neutral prior, when none do).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use url::Url;

pub use crate::analysis::EvidenceStance;

/// Source types treated as primary/official even when the caller did not set
/// [`EvidenceItem::primary`].
const PRIMARY_SOURCE_TYPES: &[&str] = &[
    "filing",
    "filings",
    "sec_filing",
    "government_registry",
    "government",
    "regulatory",
    "court",
    "official",
    "primary",
    "tender_award",
];

/// One evidence record fed to [`assess_evidence_quality`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceItem {
    /// Stable origin: a source id when known, otherwise a URL. Only the
    /// origin (URL host) is used for independence, never the path.
    pub origin: Option<String>,
    pub source_type: Option<String>,
    pub region: Option<String>,
    /// Caller-asserted primary/official source.
    pub primary: bool,
    pub observed_at: Option<DateTime<Utc>>,
    /// Relevance weight in `[0, 1]`; clamped during assessment.
    pub relevance: f64,
    pub stance: EvidenceStance,
    /// Parser confidence for this record when the producing parser reports
    /// one (e.g. parse contract checks).
    pub parser_confidence: Option<f64>,
    /// Coverage dimensions this record is evidence for (e.g. capability
    /// families). Compared case-insensitively against the expected set.
    pub coverage_tags: Vec<String>,
}

impl EvidenceItem {
    pub fn new(relevance: f64, stance: EvidenceStance) -> Self {
        Self {
            origin: None,
            source_type: None,
            region: None,
            primary: false,
            observed_at: None,
            relevance,
            stance,
            parser_confidence: None,
            coverage_tags: Vec::new(),
        }
    }

    pub fn with_origin(mut self, origin: impl Into<String>) -> Self {
        self.origin = Some(origin.into());
        self
    }

    pub fn with_source_type(mut self, source_type: impl Into<String>) -> Self {
        self.source_type = Some(source_type.into());
        self
    }

    pub fn with_region(mut self, region: impl Into<String>) -> Self {
        self.region = Some(region.into());
        self
    }

    pub fn primary(mut self) -> Self {
        self.primary = true;
        self
    }

    pub fn with_observed_at(mut self, observed_at: DateTime<Utc>) -> Self {
        self.observed_at = Some(observed_at);
        self
    }

    pub fn with_parser_confidence(mut self, parser_confidence: f64) -> Self {
        self.parser_confidence = Some(parser_confidence);
        self
    }

    pub fn with_coverage_tag(mut self, tag: impl Into<String>) -> Self {
        self.coverage_tags.push(tag.into());
        self
    }

    /// True when the record is primary either by flag or by source type.
    pub fn is_primary(&self) -> bool {
        if self.primary {
            return true;
        }
        self.source_type
            .as_deref()
            .map(|source_type| {
                let normalized = source_type.trim().to_ascii_lowercase();
                PRIMARY_SOURCE_TYPES.contains(&normalized.as_str())
            })
            .unwrap_or(false)
    }

    /// Normalized independence origin: explicit origin (lowercased) when
    /// present, otherwise the URL host; `None` when neither yields one.
    pub fn independence_origin(&self) -> Option<String> {
        let origin = self.origin.as_deref()?;
        let origin = origin.trim();
        if origin.is_empty() {
            return None;
        }
        if let Ok(url) = Url::parse(origin) {
            if let Some(host) = url.host_str() {
                let host = host.trim().to_ascii_lowercase();
                if !host.is_empty() {
                    return Some(host);
                }
            }
        }
        Some(origin.to_ascii_lowercase())
    }
}

/// The shared ten-dimension evidence-quality assessment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct EvidenceQuality {
    pub evidence_count: usize,
    pub independent_origin_count: usize,
    pub primary_source_count: usize,
    pub source_type_diversity: f64,
    pub geographic_diversity: f64,
    pub freshness: f64,
    pub contradiction_ratio: f64,
    pub corroboration_score: f64,
    pub coverage_completeness: f64,
    pub parser_confidence: f64,
}

impl EvidenceQuality {
    /// Independent origins as a share of the evidence set.
    pub fn independence_ratio(&self) -> f64 {
        if self.evidence_count == 0 {
            0.0
        } else {
            (self.independent_origin_count as f64 / self.evidence_count as f64).clamp(0.0, 1.0)
        }
    }

    /// Single composite score in `[0, 1]` for ranking/UI, derived from the ten
    /// shared dimensions rather than feature-local weights.
    pub fn composite_score(&self) -> f64 {
        if self.evidence_count == 0 {
            return 0.0;
        }
        let raw = 0.30 * self.corroboration_score
            + 0.15 * self.source_type_diversity
            + 0.15 * self.geographic_diversity
            + 0.15 * self.independence_ratio()
            + 0.15 * self.freshness
            + 0.10 * self.coverage_completeness;
        (raw * (1.0 - 0.6 * self.contradiction_ratio)).clamp(0.0, 1.0)
    }

    /// Shared quality label used by UI badges.
    pub fn quality_label(&self) -> &'static str {
        let score = self.composite_score();
        if score >= 0.8 && self.contradiction_ratio < 0.15 {
            "high"
        } else if score >= 0.6 {
            "moderate"
        } else if score >= 0.35 {
            "emerging"
        } else {
            "insufficient"
        }
    }
}

fn normalized_tag(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

/// Assess an evidence set against the caller's expected coverage dimensions.
///
/// `expected_coverage` is the set of dimensions the caller needs evidence
/// for (regions, capability families, claim types); pass an empty slice when
/// no completeness contract applies and `coverage_completeness` is `1.0`.
pub fn assess_evidence_quality(
    items: &[EvidenceItem],
    expected_coverage: &[String],
    now: DateTime<Utc>,
) -> EvidenceQuality {
    if items.is_empty() {
        return EvidenceQuality::default();
    }

    let mut origins = HashSet::new();
    let mut source_types = HashSet::new();
    let mut regions = HashSet::new();
    let mut covered_tags = HashSet::new();
    let mut freshness_total = 0.0;
    let mut freshness_count = 0usize;
    let mut parser_total = 0.0;
    let mut parser_count = 0usize;
    let mut support_weight = 0.0;
    let mut contradiction_weight = 0.0;
    let mut support_count = 0usize;
    let mut primary_count = 0usize;

    for item in items {
        let relevance = item.relevance.clamp(0.0, 1.0);
        match item.stance {
            EvidenceStance::Supports => {
                support_weight += relevance.max(0.2);
                support_count += 1;
            }
            EvidenceStance::Contradicts => {
                contradiction_weight += relevance.max(0.2);
            }
            EvidenceStance::Neutral => {
                support_weight += relevance * 0.35;
            }
        }
        if let Some(origin) = item.independence_origin() {
            origins.insert(origin);
        }
        if let Some(source_type) = item.source_type.as_deref() {
            let normalized = normalized_tag(source_type);
            if !normalized.is_empty() {
                source_types.insert(normalized);
            }
        }
        if let Some(region) = item.region.as_deref() {
            let normalized = normalized_tag(region);
            if !normalized.is_empty() {
                regions.insert(normalized);
            }
        }
        for tag in &item.coverage_tags {
            let normalized = normalized_tag(tag);
            if !normalized.is_empty() {
                covered_tags.insert(normalized);
            }
        }
        if let Some(observed_at) = item.observed_at {
            let age_days = (now - observed_at).num_days().max(0) as f64;
            freshness_total += (-age_days / 30.0).exp();
            freshness_count += 1;
        }
        if let Some(parser_confidence) = item.parser_confidence {
            parser_total += parser_confidence.clamp(0.0, 1.0);
            parser_count += 1;
        }
        if item.is_primary() {
            primary_count += 1;
        }
    }

    let evidence_count = items.len();
    let independent_origin_count = origins.len();
    let source_type_diversity = (source_types.len() as f64 / evidence_count as f64).clamp(0.0, 1.0);
    let geographic_diversity = (regions.len() as f64 / evidence_count as f64).clamp(0.0, 1.0);
    let freshness = if freshness_count == 0 {
        0.5
    } else {
        freshness_total / freshness_count as f64
    };
    let contradiction_ratio = if support_weight + contradiction_weight <= f64::EPSILON {
        0.0
    } else {
        (contradiction_weight / (support_weight + contradiction_weight)).clamp(0.0, 1.0)
    };
    let corroboration_score = if support_count == 0 {
        0.0
    } else {
        let independent_support = (independent_origin_count.min(support_count) as f64
            / evidence_count as f64)
            .clamp(0.0, 1.0);
        let support_strength = (support_weight / support_count as f64).clamp(0.0, 1.0);
        (0.55 * independent_support + 0.45 * support_strength).clamp(0.0, 1.0)
    };
    let coverage_completeness = if expected_coverage.is_empty() {
        1.0
    } else {
        let expected: HashSet<String> = expected_coverage
            .iter()
            .map(|tag| normalized_tag(tag))
            .filter(|tag| !tag.is_empty())
            .collect();
        if expected.is_empty() {
            1.0
        } else {
            let covered = expected
                .iter()
                .filter(|tag| covered_tags.contains(*tag))
                .count();
            (covered as f64 / expected.len() as f64).clamp(0.0, 1.0)
        }
    };
    let parser_confidence = if parser_count == 0 {
        0.5
    } else {
        (parser_total / parser_count as f64).clamp(0.0, 1.0)
    };

    EvidenceQuality {
        evidence_count,
        independent_origin_count,
        primary_source_count: primary_count,
        source_type_diversity,
        geographic_diversity,
        freshness,
        contradiction_ratio,
        corroboration_score,
        coverage_completeness,
        parser_confidence,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn now() -> DateTime<Utc> {
        Utc::now()
    }

    #[test]
    fn independence_counts_distinct_origins_not_records() {
        let quality = assess_evidence_quality(
            &[
                EvidenceItem::new(0.9, EvidenceStance::Supports)
                    .with_origin("https://alpha.example.com/report-a"),
                EvidenceItem::new(0.9, EvidenceStance::Supports)
                    .with_origin("https://alpha.example.com/report-b"),
                EvidenceItem::new(0.8, EvidenceStance::Supports)
                    .with_origin("https://beta.example.org/report"),
            ],
            &[],
            now(),
        );

        assert_eq!(quality.evidence_count, 3);
        assert_eq!(
            quality.independent_origin_count, 2,
            "two records from alpha.example.com must count as one origin"
        );

        let unsourced = assess_evidence_quality(
            &[EvidenceItem::new(0.9, EvidenceStance::Supports)],
            &[],
            now(),
        );
        assert_eq!(
            unsourced.independent_origin_count, 0,
            "an unsourced record must not earn independence credit"
        );
    }

    #[test]
    fn evidence_quality_carries_all_ten_dimensions() {
        let quality = assess_evidence_quality(
            &[
                EvidenceItem::new(0.9, EvidenceStance::Supports)
                    .with_origin("https://alpha.example.com/a")
                    .with_source_type("filing")
                    .with_region("europe")
                    .primary()
                    .with_observed_at(now() - Duration::days(1))
                    .with_parser_confidence(0.95)
                    .with_coverage_tag("procurement"),
                EvidenceItem::new(0.7, EvidenceStance::Contradicts)
                    .with_origin("https://beta.example.org/b")
                    .with_source_type("news")
                    .with_region("china")
                    .with_observed_at(now() - Duration::days(5))
                    .with_parser_confidence(0.8)
                    .with_coverage_tag("patents"),
            ],
            &["procurement".to_string(), "patents".to_string()],
            now(),
        );

        assert_eq!(quality.evidence_count, 2);
        assert_eq!(quality.independent_origin_count, 2);
        assert_eq!(quality.primary_source_count, 1);
        assert!((quality.source_type_diversity - 1.0).abs() < f64::EPSILON);
        assert!((quality.geographic_diversity - 1.0).abs() < f64::EPSILON);
        assert!(quality.freshness > 0.5);
        assert!(quality.contradiction_ratio > 0.0);
        assert!(quality.corroboration_score > 0.0);
        assert!((quality.coverage_completeness - 1.0).abs() < f64::EPSILON);
        assert!((quality.parser_confidence - 0.875).abs() < 1e-9);
        assert!(quality.composite_score() > 0.0);
    }

    #[test]
    fn coverage_completeness_measures_missing_dimensions() {
        let quality = assess_evidence_quality(
            &[EvidenceItem::new(0.8, EvidenceStance::Supports)
                .with_origin("https://alpha.example.com/a")
                .with_coverage_tag("procurement")],
            &[
                "procurement".to_string(),
                "patents".to_string(),
                "regulatory".to_string(),
                "hiring".to_string(),
            ],
            now(),
        );

        assert!((quality.coverage_completeness - 0.25).abs() < 1e-9);
        assert!(
            quality.composite_score() < 0.6,
            "missing three of four coverage dimensions must not score as moderate: {}",
            quality.composite_score()
        );
    }

    #[test]
    fn empty_evidence_is_insufficient() {
        let quality = assess_evidence_quality(&[], &["procurement".to_string()], now());
        assert_eq!(quality, EvidenceQuality::default());
        assert_eq!(quality.quality_label(), "insufficient");
        assert_eq!(quality.composite_score(), 0.0);
    }
}
