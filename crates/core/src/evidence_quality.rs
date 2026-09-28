//! Canonical evidence-quality model (audit P1 measurement).
//!
//! This module is the single answer to "how does ApexIntel calculate evidence
//! quality". `crate::analysis` re-exports it; there is no second
//! implementation.
//!
//! The assessment is split into two layers that are never conflated:
//!
//! * [`CorpusQuality`] — properties of the evidence set itself: evidence
//!   count, independent origin *clusters* (records sharing a content hash,
//!   near-duplicate text within a timestamp window, canonical publisher, or
//!   syndication/quoted-upstream publisher count as one origin; distinct
//!   hosts that merely republish the same story never count as
//!   corroboration), primary-source share, provenance completeness,
//!   source-type and geographic diversity, freshness, parser confidence and
//!   coverage completeness.
//! * [`ClaimAssessment`] — how the evidence bears on an *actual claim*
//!   (supports / contradicts / neutral, corroboration, contradiction ratio).
//!   Without a claim there is nothing to contradict, so
//!   `contradiction_ratio` stays [`Measurement::NotMeasured`] — it is never
//!   reported as "no contradictions" merely because every record was created
//!   with `Supports`.
//!
//! Every derived metric is a [`Measurement`]: an absent timestamp does not
//! become `0.5`, and a family with no parser results does not become a neutral
//! prior. [`EvidenceQuality::composite_score`] scores only the dimensions that
//! were actually measured, re-normalizing the weights, and
//! [`EvidenceQuality::completeness`] reports separately how much of the model
//! was measurable. It never synthesizes a midpoint for an unknown dimension.
//!
//! `parser_confidence` and `freshness` are the dimensions most often missing
//! in practice, so they are typed rather than defaulted; a caller that really
//! wants a prior must state it explicitly and it will be visible as a measured
//! input.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use url::Url;

use crate::measurement::Measurement;

/// Knowledge stance of one evidence record relative to a claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceStance {
    Supports,
    Contradicts,
    Neutral,
}

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

/// One evidence record fed to the assessment functions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceItem {
    /// Stable origin: a source id when known, otherwise a URL. The origin host
    /// is one clustering signal; content hash, publisher and syndication
    /// signals take precedence so a syndicated story is one origin even
    /// across hosts.
    pub origin: Option<String>,
    pub source_type: Option<String>,
    pub region: Option<String>,
    /// Caller-asserted primary/official source.
    pub primary: bool,
    pub observed_at: Option<DateTime<Utc>>,
    /// Relevance weight in `[0, 1]` used for claim-level weighting.
    /// [`Measurement::NotMeasured`] means the producer had no measured
    /// relevance/confidence for this record: it still counts for corpus
    /// dimensions, but never contributes a synthesized weight.
    pub relevance: Measurement<f64>,
    pub stance: EvidenceStance,
    /// Parser confidence for this record when the producing parser reports one
    /// (e.g. parse contract checks). Missing stays missing.
    pub parser_confidence: Measurement<f64>,
    /// Source-quality tier already computed elsewhere.
    pub source_reliability_tier: Option<String>,
    /// True when this record is derived intelligence (a generated insight or
    /// inference) rather than a directly observed fact.
    pub derived: bool,
    /// Coverage dimensions this record is evidence for (e.g. capability
    /// families). Compared case-insensitively against the expected set.
    pub coverage_tags: Vec<String>,
    /// Content hash for exact-duplicate origin clustering.
    #[serde(default)]
    pub content_hash: Option<String>,
    /// Canonical publisher of the record (e.g. `reuters`), used for origin
    /// clustering across syndicating sites.
    #[serde(default)]
    pub canonical_publisher: Option<String>,
    /// Upstream publisher/wire this record was syndicated from or quotes.
    #[serde(default)]
    pub syndication_of: Option<String>,
    /// Title text used for near-duplicate origin clustering.
    #[serde(default)]
    pub title: Option<String>,
}

impl EvidenceItem {
    /// A record with a measured relevance weight.
    pub fn new(relevance: f64, stance: EvidenceStance) -> Self {
        Self {
            origin: None,
            source_type: None,
            region: None,
            primary: false,
            observed_at: None,
            relevance: Measurement::measured(relevance),
            stance,
            parser_confidence: Measurement::not_measured(),
            source_reliability_tier: None,
            derived: false,
            coverage_tags: Vec::new(),
            content_hash: None,
            canonical_publisher: None,
            syndication_of: None,
            title: None,
        }
    }

    /// A record whose relevance/confidence was never measured.
    pub fn new_unmeasured(stance: EvidenceStance) -> Self {
        Self {
            relevance: Measurement::not_measured(),
            ..Self::new(0.0, stance)
        }
    }

    /// A record whose relevance is the optional measured confidence of the
    /// producing observation/insight: `None` stays `NotMeasured`.
    pub fn new_optional(relevance: Option<f64>, stance: EvidenceStance) -> Self {
        match relevance {
            Some(value) => Self::new(value, stance),
            None => Self::new_unmeasured(stance),
        }
    }

    /// Override the relevance weight with an explicit measurement state.
    pub fn with_relevance(mut self, relevance: Measurement<f64>) -> Self {
        self.relevance = relevance;
        self
    }

    /// Set the relevance weight from an optional measured value.
    pub fn with_optional_relevance(mut self, relevance: Option<f64>) -> Self {
        self.relevance = match relevance {
            Some(value) => Measurement::measured(value),
            None => Measurement::not_measured(),
        };
        self
    }

    pub fn with_origin(mut self, origin: impl Into<String>) -> Self {
        self.origin = Some(origin.into());
        self
    }

    /// Alias for [`Self::with_origin`] for source-id call sites.
    pub fn with_source_id(mut self, source_id: impl Into<String>) -> Self {
        self.origin = Some(source_id.into());
        self
    }

    /// Alias for [`Self::with_origin`] for URL call sites.
    pub fn with_source_url(mut self, source_url: impl Into<String>) -> Self {
        self.origin = Some(source_url.into());
        self
    }

    pub fn with_content_hash(mut self, content_hash: impl Into<String>) -> Self {
        self.content_hash = Some(content_hash.into());
        self
    }

    pub fn with_canonical_publisher(mut self, publisher: impl Into<String>) -> Self {
        self.canonical_publisher = Some(publisher.into());
        self
    }

    pub fn with_syndication_of(mut self, upstream: impl Into<String>) -> Self {
        self.syndication_of = Some(upstream.into());
        self
    }

    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
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
        self.parser_confidence = Measurement::measured(parser_confidence);
        self
    }

    /// Parser confidence from an optional measured value; `None` stays
    /// `NotMeasured`.
    pub fn with_optional_parser_confidence(mut self, parser_confidence: Option<f64>) -> Self {
        self.parser_confidence = match parser_confidence {
            Some(value) => Measurement::measured(value),
            None => Measurement::not_measured(),
        };
        self
    }

    pub fn with_source_reliability(mut self, tier: Option<String>) -> Self {
        self.source_reliability_tier = tier;
        self
    }

    pub fn derived(mut self) -> Self {
        self.derived = true;
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

    /// Normalized independence origin (registrable domain for URLs/hosts,
    /// lowercased origin otherwise); `None` when no origin is known.
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

/// Corpus-level metrics: what the evidence set is, independent of any claim.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct CorpusQuality {
    pub evidence_count: usize,
    pub independent_origin_count: usize,
    /// Independent origins / evidence count.
    pub independence_ratio: f64,
    pub primary_source_count: usize,
    /// Primary/official sources / evidence count.
    pub primary_source_share: f64,
    /// Share of records carrying the minimum provenance (an origin and an
    /// observation timestamp).
    pub provenance_completeness: f64,
    /// Distinct source types / evidence count.
    pub source_type_diversity: f64,
    /// Distinct regions / evidence count.
    pub geographic_diversity: f64,
    /// Mean exponential age decay (`exp(-age_days / 30)`). `NotMeasured` when
    /// no record carries a timestamp.
    pub freshness: Measurement<f64>,
    /// Mean parser confidence of the records that report one. `NotMeasured`
    /// when none do.
    pub parser_confidence: Measurement<f64>,
    /// Share of the caller's expected coverage dimensions that are covered.
    /// `NotMeasured` when the caller declared no coverage contract.
    pub coverage_completeness: Measurement<f64>,
    /// Records that are directly observed facts.
    pub direct_evidence_count: usize,
    /// Records that are derived intelligence (generated insight/inference).
    pub derived_evidence_count: usize,
    /// How many records cite each source-quality tier; records without
    /// measured source quality are counted under `unknown`.
    pub source_reliability_distribution: BTreeMap<String, usize>,
}

/// Claim-level assessment: how the evidence bears on an actual claim.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ClaimAssessment {
    /// The claim the evidence was evaluated against. `None` means no claim was
    /// assessed: corroboration and contradiction stay unmeasured.
    pub claim: Option<String>,
    pub supporting_count: usize,
    pub neutral_count: usize,
    pub contradicting_count: usize,
    /// Independent support relative to the set. Measured only when a claim was
    /// assessed; `InsufficientEvidence` when supporting records exist but none
    /// carried a measured relevance weight.
    pub corroboration_score: Measurement<f64>,
    /// Relevance-weighted contradicting share. Measured only when a claim was
    /// assessed and at least one stance-bearing record carried a measured
    /// weight; never `Measured(0.0)` merely because records were created as
    /// `Supports`.
    pub contradiction_ratio: Measurement<f64>,
}

/// How much of the quality model could actually be measured.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MeasurementCompleteness {
    pub measured_dimensions: usize,
    pub total_dimensions: usize,
    /// `measured_dimensions / total_dimensions`.
    pub ratio: f64,
    /// Names of the tracked dimensions that stayed unmeasured.
    pub missing_dimensions: Vec<String>,
}

impl Default for MeasurementCompleteness {
    fn default() -> Self {
        Self {
            measured_dimensions: 0,
            total_dimensions: TRACKED_DIMENSIONS.len(),
            ratio: 0.0,
            missing_dimensions: TRACKED_DIMENSIONS
                .iter()
                .map(|name| (*name).to_string())
                .collect(),
        }
    }
}

/// Dimensions tracked by [`MeasurementCompleteness`]. Six are scored by
/// [`EvidenceQuality::composite_score`]; `parser_confidence` is reported but
/// not scored.
pub const TRACKED_DIMENSIONS: [&str; 7] = [
    "independence_ratio",
    "source_type_diversity",
    "geographic_diversity",
    "freshness",
    "parser_confidence",
    "coverage_completeness",
    "corroboration",
];

/// The full evidence-quality assessment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct EvidenceQuality {
    pub corpus: CorpusQuality,
    pub claim: ClaimAssessment,
    pub completeness: MeasurementCompleteness,
}

impl EvidenceQuality {
    /// Single composite score in `[0, 1]` over the **measured** dimensions
    /// only. Weights are re-normalized over what was measurable, so a missing
    /// freshness timestamp or parser confidence removes that term instead of
    /// substituting a midpoint. When a claim was assessed, the measured
    /// contradiction ratio applies a `1 - 0.6 * ratio` penalty.
    pub fn composite_score(&self) -> f64 {
        if self.corpus.evidence_count == 0 {
            return 0.0;
        }
        let dimensions = self.scored_dimensions();
        let weight_total: f64 = dimensions.iter().map(|(_, weight, _)| weight).sum();
        if weight_total <= f64::EPSILON {
            return 0.0;
        }
        let weighted: f64 = dimensions
            .iter()
            .map(|(_, weight, value)| weight * value.clamp(0.0, 1.0))
            .sum();
        let base = weighted / weight_total;
        let penalty = self
            .claim
            .contradiction_ratio
            .value_copied()
            .map(|ratio| 1.0 - 0.6 * ratio.clamp(0.0, 1.0))
            .unwrap_or(1.0);
        (base * penalty).clamp(0.0, 1.0)
    }

    /// The measured score dimensions as `(name, weight, value)`.
    fn scored_dimensions(&self) -> Vec<(&'static str, f64, f64)> {
        let mut dimensions = vec![
            (
                "independence_ratio",
                INDEPENDENCE_WEIGHT,
                self.corpus.independence_ratio,
            ),
            (
                "source_type_diversity",
                DIVERSITY_WEIGHT,
                self.corpus.source_type_diversity,
            ),
            (
                "geographic_diversity",
                DIVERSITY_WEIGHT,
                self.corpus.geographic_diversity,
            ),
        ];
        if let Some(freshness) = self.corpus.freshness.value_copied() {
            dimensions.push(("freshness", FRESHNESS_WEIGHT, freshness));
        }
        if let Some(coverage) = self.corpus.coverage_completeness.value_copied() {
            dimensions.push(("coverage_completeness", COVERAGE_WEIGHT, coverage));
        }
        if self.claim.claim.is_some() {
            if let Some(corroboration) = self.claim.corroboration_score.value_copied() {
                dimensions.push(("corroboration", CORROBORATION_WEIGHT, corroboration));
            }
        }
        dimensions
    }

    /// Shared quality label used by UI badges. The "high" gate only applies
    /// the contradiction rule when the ratio was actually measured.
    pub fn quality_label(&self) -> &'static str {
        let score = self.composite_score();
        let contradiction_blocks_high = self
            .claim
            .contradiction_ratio
            .value_copied()
            .map(|ratio| ratio >= 0.15)
            .unwrap_or(false);
        if score >= 0.8 && !contradiction_blocks_high {
            "high"
        } else if score >= 0.6 {
            "moderate"
        } else if score >= 0.35 {
            "emerging"
        } else {
            "insufficient"
        }
    }

    pub fn independence_ratio(&self) -> f64 {
        self.corpus.independence_ratio
    }
}

const CORROBORATION_WEIGHT: f64 = 0.30;
const INDEPENDENCE_WEIGHT: f64 = 0.15;
const DIVERSITY_WEIGHT: f64 = 0.15;
const FRESHNESS_WEIGHT: f64 = 0.15;
const COVERAGE_WEIGHT: f64 = 0.10;

fn normalized_tag(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

/// Assess the corpus-level quality of an evidence set.
///
/// `expected_coverage` is the set of dimensions the caller needs evidence for
/// (regions, capability families, claim types); pass an empty slice when no
/// completeness contract applies and `coverage_completeness` stays
/// `NotMeasured`.
pub fn assess_corpus_quality(
    items: &[EvidenceItem],
    expected_coverage: &[String],
    now: DateTime<Utc>,
) -> CorpusQuality {
    if items.is_empty() {
        return CorpusQuality::default();
    }

    let mut source_types = HashSet::new();
    let mut regions = HashSet::new();
    let mut covered_tags = HashSet::new();
    let mut freshness_total = 0.0;
    let mut freshness_count = 0usize;
    let mut parser_total = 0.0;
    let mut parser_count = 0usize;
    let mut primary_count = 0usize;
    let mut provenance_count = 0usize;
    let mut direct_count = 0usize;
    let mut derived_count = 0usize;
    let mut reliability_distribution: BTreeMap<String, usize> = BTreeMap::new();

    for item in items {
        let has_origin = item.independence_origin().is_some();
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
        if let Some(parser_confidence) = item.parser_confidence.value_copied() {
            parser_total += parser_confidence.clamp(0.0, 1.0);
            parser_count += 1;
        }
        if item.is_primary() {
            primary_count += 1;
        }
        if has_origin && item.observed_at.is_some() {
            provenance_count += 1;
        }
        if item.derived {
            derived_count += 1;
        } else {
            direct_count += 1;
        }
        let tier = item
            .source_reliability_tier
            .as_deref()
            .map(str::trim)
            .filter(|tier| !tier.is_empty())
            .unwrap_or("unknown")
            .to_ascii_lowercase();
        *reliability_distribution.entry(tier).or_default() += 1;
    }

    let evidence_count = items.len();
    // Independence is counted over origin clusters (content hash, publisher,
    // syndication upstream, near-duplicate text within a timestamp window,
    // then origin host) — never over registrable domains.
    let origin_records: Vec<crate::origin_cluster::OriginRecord> = items
        .iter()
        .map(|item| crate::origin_cluster::OriginRecord {
            origin: item.independence_origin(),
            canonical_publisher: item.canonical_publisher.clone(),
            syndication_of: item.syndication_of.clone(),
            content_hash: item.content_hash.clone(),
            title: item.title.clone(),
            body: None,
            observed_at: item.observed_at,
        })
        .collect();
    let independent_origin_count = crate::origin_cluster::independent_origin_count(&origin_records);
    let source_type_diversity = (source_types.len() as f64 / evidence_count as f64).clamp(0.0, 1.0);
    let geographic_diversity = (regions.len() as f64 / evidence_count as f64).clamp(0.0, 1.0);
    let freshness = if freshness_count == 0 {
        Measurement::not_measured()
    } else {
        Measurement::measured((freshness_total / freshness_count as f64).clamp(0.0, 1.0))
    };
    let parser_confidence = if parser_count == 0 {
        Measurement::not_measured()
    } else {
        Measurement::measured((parser_total / parser_count as f64).clamp(0.0, 1.0))
    };
    let coverage_completeness = {
        let expected: HashSet<String> = expected_coverage
            .iter()
            .map(|tag| normalized_tag(tag))
            .filter(|tag| !tag.is_empty())
            .collect();
        if expected.is_empty() {
            Measurement::not_measured()
        } else {
            let covered = expected
                .iter()
                .filter(|tag| covered_tags.contains(*tag))
                .count();
            Measurement::measured((covered as f64 / expected.len() as f64).clamp(0.0, 1.0))
        }
    };

    CorpusQuality {
        evidence_count,
        independent_origin_count,
        independence_ratio: if evidence_count == 0 {
            0.0
        } else {
            (independent_origin_count as f64 / evidence_count as f64).clamp(0.0, 1.0)
        },
        primary_source_count: primary_count,
        primary_source_share: if evidence_count == 0 {
            0.0
        } else {
            (primary_count as f64 / evidence_count as f64).clamp(0.0, 1.0)
        },
        provenance_completeness: (provenance_count as f64 / evidence_count as f64).clamp(0.0, 1.0),
        source_type_diversity,
        geographic_diversity,
        freshness,
        parser_confidence,
        coverage_completeness,
        direct_evidence_count: direct_count,
        derived_evidence_count: derived_count,
        source_reliability_distribution: reliability_distribution,
    }
}

/// Assess how an evidence set bears on `claim`.
///
/// `claim = None` produces count-only output: corroboration and contradiction
/// stay `NotMeasured` because there is no claim to support or contradict.
pub fn assess_claim(claim: Option<&str>, items: &[EvidenceItem]) -> ClaimAssessment {
    if items.is_empty() {
        return ClaimAssessment {
            claim: claim.map(str::to_string),
            ..ClaimAssessment::default()
        };
    }

    let mut support_weight = 0.0;
    let mut contradiction_weight = 0.0;
    let mut weighted_support_count = 0usize;
    let mut weighted_contradiction_count = 0usize;
    let mut support_count = 0usize;
    let mut contradiction_count = 0usize;
    let mut neutral_count = 0usize;
    let mut independent_origins = HashSet::new();

    for item in items {
        if let Some(origin) = item.independence_origin() {
            independent_origins.insert(origin);
        }
        let relevance = item.relevance.value_copied();
        match item.stance {
            EvidenceStance::Supports => {
                support_count += 1;
                if let Some(relevance) = relevance {
                    support_weight += relevance.clamp(0.0, 1.0).max(0.2);
                    weighted_support_count += 1;
                }
            }
            EvidenceStance::Contradicts => {
                contradiction_count += 1;
                if let Some(relevance) = relevance {
                    contradiction_weight += relevance.clamp(0.0, 1.0).max(0.2);
                    weighted_contradiction_count += 1;
                }
            }
            EvidenceStance::Neutral => {
                neutral_count += 1;
                if let Some(relevance) = relevance {
                    support_weight += relevance.clamp(0.0, 1.0) * 0.35;
                    weighted_support_count += 1;
                }
            }
        }
    }

    let claim_present = claim.is_some();
    let corroboration_score = if !claim_present {
        Measurement::not_measured()
    } else if support_count == 0 {
        Measurement::measured(0.0)
    } else if weighted_support_count == 0 {
        Measurement::insufficient_evidence()
    } else {
        let independent_support = (independent_origins.len().min(support_count) as f64
            / items.len() as f64)
            .clamp(0.0, 1.0);
        let support_strength = (support_weight / support_count as f64).clamp(0.0, 1.0);
        Measurement::measured(
            (0.55 * independent_support + 0.45 * support_strength).clamp(0.0, 1.0),
        )
    };

    let contradiction_ratio = if !claim_present {
        Measurement::not_measured()
    } else if weighted_support_count + weighted_contradiction_count == 0 {
        Measurement::insufficient_evidence()
    } else {
        let total_weight = support_weight + contradiction_weight;
        if total_weight <= f64::EPSILON {
            Measurement::insufficient_evidence()
        } else {
            Measurement::measured((contradiction_weight / total_weight).clamp(0.0, 1.0))
        }
    };

    ClaimAssessment {
        claim: claim.map(str::to_string),
        supporting_count: support_count,
        neutral_count,
        contradicting_count: contradiction_count,
        corroboration_score,
        contradiction_ratio,
    }
}

/// Measure how much of the model was actually measurable.
pub fn measurement_completeness(
    corpus: &CorpusQuality,
    claim: &ClaimAssessment,
) -> MeasurementCompleteness {
    if corpus.evidence_count == 0 {
        return MeasurementCompleteness::default();
    }
    let states: [(&'static str, bool); 7] = [
        ("independence_ratio", true),
        ("source_type_diversity", true),
        ("geographic_diversity", true),
        ("freshness", corpus.freshness.is_measured()),
        ("parser_confidence", corpus.parser_confidence.is_measured()),
        (
            "coverage_completeness",
            corpus.coverage_completeness.is_measured(),
        ),
        ("corroboration", claim.corroboration_score.is_measured()),
    ];
    let measured_dimensions = states.iter().filter(|(_, measured)| *measured).count();
    let missing_dimensions = states
        .iter()
        .filter(|(_, measured)| !measured)
        .map(|(name, _)| (*name).to_string())
        .collect();
    MeasurementCompleteness {
        measured_dimensions,
        total_dimensions: TRACKED_DIMENSIONS.len(),
        ratio: measured_dimensions as f64 / TRACKED_DIMENSIONS.len() as f64,
        missing_dimensions,
    }
}

/// Assess an evidence set without a claim: corpus quality plus stance counts.
/// Corroboration and contradiction are unmeasured.
pub fn assess_evidence_quality(
    items: &[EvidenceItem],
    expected_coverage: &[String],
    now: DateTime<Utc>,
) -> EvidenceQuality {
    build_assessment(items, expected_coverage, None, now)
}

/// Assess an evidence set against an actual claim so corroboration and the
/// contradiction ratio are measured.
pub fn assess_evidence_quality_for_claim(
    items: &[EvidenceItem],
    expected_coverage: &[String],
    claim: &str,
    now: DateTime<Utc>,
) -> EvidenceQuality {
    build_assessment(items, expected_coverage, Some(claim), now)
}

fn build_assessment(
    items: &[EvidenceItem],
    expected_coverage: &[String],
    claim: Option<&str>,
    now: DateTime<Utc>,
) -> EvidenceQuality {
    let corpus = assess_corpus_quality(items, expected_coverage, now);
    let claim_assessment = assess_claim(claim, items);
    let completeness = measurement_completeness(&corpus, &claim_assessment);
    EvidenceQuality {
        corpus,
        claim: claim_assessment,
        completeness,
    }
}

/// Registrable domain (eTLD+1 approximation) for a URL or bare host.
///
/// Independence must count distinct organisations, not distinct URLs or
/// subdomains: `news.example.com` and `blog.example.com` are one origin. This
/// strips the scheme/path, lowercases, removes `www.`, and reduces the host to
/// its last label plus the public suffix when the suffix is a known multi-label
/// one (for example `co.uk`); otherwise it keeps the last two labels. IP
/// addresses and single-label hosts are returned as-is (lowercased), because
/// there is no registrable domain to reduce them to.
pub fn registrable_domain(url_or_host: &str) -> Option<String> {
    let raw = url_or_host.trim();
    if raw.is_empty() {
        return None;
    }
    let host = match Url::parse(raw) {
        Ok(parsed) => parsed.host_str().map(str::to_string),
        Err(_) => {
            // Bare host (possibly with port/path): strip everything after the
            // first `/`, `?`, or `#`, then the port.
            let cut = raw
                .find(['/', '?', '#'])
                .map(|index| &raw[..index])
                .unwrap_or(raw);
            Some(
                cut.rsplit_once(':')
                    .map(|(host, _)| host)
                    .unwrap_or(cut)
                    .to_string(),
            )
        }
    }?;
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty() {
        return None;
    }
    if host.parse::<std::net::IpAddr>().is_ok() {
        return Some(host);
    }
    let labels: Vec<&str> = host.split('.').filter(|label| !label.is_empty()).collect();
    if labels.len() < 2 {
        return Some(host);
    }
    let suffix_len = multi_label_suffix_len(&labels);
    let keep = suffix_len + 1;
    Some(labels[labels.len() - keep..].join("."))
}

/// Number of trailing labels forming a known multi-label public suffix.
fn multi_label_suffix_len(labels: &[&str]) -> usize {
    const MULTI_LABEL_SUFFIXES: &[&[&str]] = &[
        &["co", "uk"],
        &["org", "uk"],
        &["ac", "uk"],
        &["gov", "uk"],
        &["co", "jp"],
        &["or", "jp"],
        &["ne", "jp"],
        &["com", "au"],
        &["net", "au"],
        &["org", "au"],
        &["co", "nz"],
        &["com", "br"],
        &["com", "cn"],
        &["com", "hk"],
        &["com", "sg"],
        &["com", "tw"],
        &["co", "in"],
        &["com", "mx"],
        &["co", "za"],
        &["co", "kr"],
    ];
    if labels.len() < 3 {
        return 1;
    }
    let last_two = &labels[labels.len() - 2..];
    if MULTI_LABEL_SUFFIXES.contains(&last_two) {
        2
    } else {
        1
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
    fn syndicated_story_across_three_sites_is_one_origin() {
        let now = now();
        let quality = assess_evidence_quality(
            &[
                EvidenceItem::new(0.9, EvidenceStance::Supports)
                    .with_origin("https://site-a.example/tech/story")
                    .with_canonical_publisher("reuters")
                    .with_title("Chipmaker unveils new plant in Arizona")
                    .with_observed_at(now),
                EvidenceItem::new(0.9, EvidenceStance::Supports)
                    .with_origin("https://site-b.example/business/story")
                    .with_canonical_publisher("reuters")
                    .with_title("Chipmaker unveils new plant in Arizona")
                    .with_observed_at(now),
                EvidenceItem::new(0.9, EvidenceStance::Supports)
                    .with_origin("https://site-c.example/wires/story")
                    .with_syndication_of("reuters")
                    .with_title("Chipmaker unveils new plant in Arizona")
                    .with_observed_at(now),
            ],
            &[],
            now,
        );

        assert_eq!(quality.corpus.evidence_count, 3);
        assert_eq!(
            quality.corpus.independent_origin_count, 1,
            "Reuters via three sites is one origin"
        );
        assert!(
            quality.independence_ratio() < 0.5,
            "syndicated copies must not inflate independence: {}",
            quality.independence_ratio()
        );
    }

    #[test]
    fn independence_counts_distinct_origins_not_records() {
        let quality = assess_evidence_quality(
            &[
                EvidenceItem::new(0.9, EvidenceStance::Supports)
                    .with_origin("https://news.example.com/report-a"),
                EvidenceItem::new(0.9, EvidenceStance::Supports)
                    .with_origin("https://blog.example.com/report-b"),
                EvidenceItem::new(0.8, EvidenceStance::Supports)
                    .with_origin("https://beta.example.org/report"),
            ],
            &[],
            now(),
        );

        assert_eq!(quality.corpus.evidence_count, 3);
        assert_eq!(
            quality.corpus.independent_origin_count, 3,
            "three distinct origin hosts are three origin clusters"
        );

        let unsourced = assess_evidence_quality(
            &[EvidenceItem::new(0.9, EvidenceStance::Supports)],
            &[],
            now(),
        );
        assert_eq!(
            unsourced.corpus.independent_origin_count, 0,
            "an unsourced record must not earn independence credit"
        );
    }

    #[test]
    fn missing_freshness_and_parser_stay_not_measured() {
        let quality = assess_evidence_quality(
            &[
                EvidenceItem::new(0.9, EvidenceStance::Supports)
                    .with_origin("https://alpha.example.com/a"),
                EvidenceItem::new(0.8, EvidenceStance::Supports)
                    .with_origin("https://beta.example.org/b"),
            ],
            &[],
            now(),
        );

        assert_eq!(
            quality.corpus.freshness,
            Measurement::not_measured(),
            "no timestamp must not synthesize a 0.5 freshness midpoint"
        );
        assert_eq!(
            quality.corpus.parser_confidence,
            Measurement::not_measured(),
            "no parser results must not synthesize a 0.5 confidence midpoint"
        );
        assert!(
            quality
                .completeness
                .missing_dimensions
                .iter()
                .any(|name| name == "freshness"),
            "freshness must be reported as a missing measurement"
        );
        assert!(
            quality
                .completeness
                .missing_dimensions
                .iter()
                .any(|name| name == "parser_confidence"),
            "parser confidence must be reported as a missing measurement"
        );
        assert!(quality.completeness.ratio < 1.0);
        assert!(quality.composite_score() > 0.0);
    }

    #[test]
    fn measured_freshness_and_parser_are_reported() {
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

        assert_eq!(quality.corpus.evidence_count, 2);
        assert_eq!(quality.corpus.independent_origin_count, 2);
        assert_eq!(quality.corpus.primary_source_count, 1);
        assert!((quality.corpus.primary_source_share - 0.5).abs() < f64::EPSILON);
        assert!((quality.corpus.source_type_diversity - 1.0).abs() < f64::EPSILON);
        assert!((quality.corpus.geographic_diversity - 1.0).abs() < f64::EPSILON);
        assert!(quality.corpus.freshness.value_copied().unwrap_or_default() > 0.5);
        assert!(
            (quality
                .corpus
                .parser_confidence
                .value_copied()
                .unwrap_or_default()
                - 0.875)
                .abs()
                < 1e-9
        );
        assert!(
            (quality
                .corpus
                .coverage_completeness
                .value_copied()
                .unwrap_or_default()
                - 1.0)
                .abs()
                < 1e-9
        );
        assert!(quality.composite_score() > 0.0);
    }

    #[test]
    fn contradiction_ratio_is_undefined_without_a_claim() {
        let items = [
            EvidenceItem::new(0.9, EvidenceStance::Supports)
                .with_origin("https://alpha.example.com/a"),
            EvidenceItem::new(0.9, EvidenceStance::Supports)
                .with_origin("https://beta.example.org/b"),
        ];

        let unclaimed = assess_evidence_quality(&items, &[], now());
        assert!(unclaimed.claim.claim.is_none());
        assert_eq!(
            unclaimed.claim.contradiction_ratio,
            Measurement::not_measured(),
            "no claim means there is nothing to contradict; a Supports-only set \
             must not report a measured zero ratio"
        );
        assert_eq!(
            unclaimed.claim.corroboration_score,
            Measurement::not_measured()
        );

        let claimed = assess_evidence_quality_for_claim(&items, &[], "alpha grows", now());
        assert_eq!(claimed.claim.claim.as_deref(), Some("alpha grows"));
        assert_eq!(
            claimed.claim.contradiction_ratio,
            Measurement::measured(0.0)
        );
        assert!(claimed.claim.corroboration_score.is_measured());
    }

    #[test]
    fn claim_assessment_measures_contradiction_weight() {
        let quality = assess_evidence_quality_for_claim(
            &[
                EvidenceItem::new(0.9, EvidenceStance::Supports)
                    .with_origin("https://alpha.example.com/a"),
                EvidenceItem::new(0.9, EvidenceStance::Contradicts)
                    .with_origin("https://beta.example.org/b"),
                EvidenceItem::new(0.5, EvidenceStance::Neutral)
                    .with_origin("https://gamma.example.net/c"),
            ],
            &[],
            "alpha grows",
            now(),
        );

        assert_eq!(quality.claim.supporting_count, 1);
        assert_eq!(quality.claim.contradicting_count, 1);
        assert_eq!(quality.claim.neutral_count, 1);
        let ratio = quality
            .claim
            .contradiction_ratio
            .value_copied()
            .expect("claim present, weighted records present");
        assert!(ratio > 0.0 && ratio < 1.0);
        assert!(quality.composite_score() < 1.0);
    }

    #[test]
    fn claim_without_measured_weights_is_insufficient_evidence() {
        let quality = assess_evidence_quality_for_claim(
            &[EvidenceItem::new_unmeasured(EvidenceStance::Supports)
                .with_origin("https://alpha.example.com/a")],
            &[],
            "alpha grows",
            now(),
        );

        assert_eq!(
            quality.claim.corroboration_score,
            Measurement::insufficient_evidence(),
            "supporting records with no measured weight cannot produce a strength score"
        );
        assert_eq!(
            quality.claim.contradiction_ratio,
            Measurement::insufficient_evidence()
        );
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

        assert!(
            (quality
                .corpus
                .coverage_completeness
                .value_copied()
                .unwrap_or_default()
                - 0.25)
                .abs()
                < 1e-9
        );
    }

    #[test]
    fn no_coverage_contract_stays_not_measured() {
        let quality = assess_evidence_quality(
            &[EvidenceItem::new(0.8, EvidenceStance::Supports)
                .with_origin("https://alpha.example.com/a")],
            &[],
            now(),
        );

        assert_eq!(
            quality.corpus.coverage_completeness,
            Measurement::not_measured(),
            "no declared coverage contract means coverage completeness was not measured"
        );
        assert!(quality
            .completeness
            .missing_dimensions
            .iter()
            .any(|name| name == "coverage_completeness"));
    }

    #[test]
    fn composite_skips_unmeasured_dimensions_instead_of_substituting() {
        // Two corpora identical except the second has no timestamp and no
        // parser confidence. The first scores freshness/parser into the
        // composite; the second renormalizes over the remaining dimensions
        // rather than injecting a 0.5.
        let measured = assess_evidence_quality(
            &[EvidenceItem::new(1.0, EvidenceStance::Supports)
                .with_origin("https://alpha.example.com/a")
                .with_observed_at(now())
                .with_parser_confidence(1.0)],
            &[],
            now(),
        );
        let unmeasured = assess_evidence_quality(
            &[EvidenceItem::new(1.0, EvidenceStance::Supports)
                .with_origin("https://alpha.example.com/a")],
            &[],
            now(),
        );

        assert!(measured.composite_score() > 0.0);
        assert!(unmeasured.composite_score() > 0.0);
        assert!(measured.completeness.ratio > unmeasured.completeness.ratio);
        assert_eq!(unmeasured.completeness.measured_dimensions, 3);
        assert_eq!(
            unmeasured.completeness.total_dimensions,
            TRACKED_DIMENSIONS.len()
        );
    }

    #[test]
    fn empty_evidence_is_insufficient() {
        let quality = assess_evidence_quality(&[], &["procurement".to_string()], now());
        assert_eq!(quality, EvidenceQuality::default());
        assert_eq!(quality.quality_label(), "insufficient");
        assert_eq!(quality.composite_score(), 0.0);
        assert_eq!(quality.completeness.ratio, 0.0);
    }

    #[test]
    fn registrable_domain_reduces_hosts_to_etld_plus_one() {
        assert_eq!(
            registrable_domain("https://news.example.com/story").as_deref(),
            Some("example.com")
        );
        assert_eq!(
            registrable_domain("www.example.co.uk").as_deref(),
            Some("example.co.uk")
        );
        assert_eq!(
            registrable_domain("https://sub.example.com:8443/path").as_deref(),
            Some("example.com")
        );
        assert_eq!(
            registrable_domain("http://192.168.0.1/x").as_deref(),
            Some("192.168.0.1")
        );
        assert_eq!(
            registrable_domain("localhost").as_deref(),
            Some("localhost")
        );
        assert_eq!(registrable_domain(""), None);
    }

    #[test]
    fn corpus_reports_provenance_and_derived_counts() {
        let quality = assess_evidence_quality(
            &[
                EvidenceItem::new(0.9, EvidenceStance::Supports)
                    .with_origin("https://alpha.example.com/a")
                    .with_observed_at(now()),
                EvidenceItem::new(0.9, EvidenceStance::Supports)
                    .with_origin("https://beta.example.org/b"),
                EvidenceItem::new(0.9, EvidenceStance::Supports)
                    .with_origin("https://gamma.example.net/c")
                    .with_observed_at(now())
                    .derived(),
            ],
            &[],
            now(),
        );

        assert!((quality.corpus.provenance_completeness - 2.0 / 3.0).abs() < 1e-9);
        assert_eq!(quality.corpus.direct_evidence_count, 2);
        assert_eq!(quality.corpus.derived_evidence_count, 1);
        assert_eq!(
            quality
                .corpus
                .source_reliability_distribution
                .get("unknown"),
            Some(&3)
        );
    }
}
