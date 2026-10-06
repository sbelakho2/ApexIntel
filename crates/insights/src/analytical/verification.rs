//! Type-routed atomic claim verification (CoVe / FActScore / FinGround-style).
//!
//! Each claim is decomposed by fact type and verified with the mechanism that
//! fits it — numbers against evidence numbers with tolerance, dates against
//! the evidence timeline, citations against the supplied evidence range,
//! causal assertions against multi-source support, and org-like entities
//! against evidence text. The supported fraction of atomic facts is the
//! [`VerificationReport::factuality_score`]; hard violations (fabricated
//! numbers, invented dates, out-of-range citations) are reported separately
//! and block publication outright.
//!
//! Deterministic and evidence-scoped: the verifier never has network access
//! and never "fixes" a claim, it only judges it against the supplied records.

use super::EvidenceRecord;
use apex_core::claims::{ClaimKind, InsightClaim};
use chrono::{DateTime, Datelike, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::sync::OnceLock;

/// Per-claim verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaimVerdict {
    Supported,
    PartiallySupported,
    Unsupported,
    Unverifiable,
}

/// Hard violation kinds — any one is fatal to publication.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HardViolation {
    /// A figure asserted as observed that matches no evidence number within
    /// tolerance.
    FabricatedNumber { claim: String, value: String },
    /// A past date asserted as observed that appears nowhere in the evidence.
    UnverifiedDate { claim: String, date: String },
    /// A numeric citation index outside the supplied evidence range.
    CitationOutOfRange {
        claim: String,
        ordinal: usize,
        evidence_count: usize,
    },
}

/// Soft flags — quality signals that feed depth/warrant, never fatal alone.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SoftFlag {
    /// A causal statement supported by fewer than two evidence items.
    ThinCausality {
        claim: String,
        evidence_count: usize,
    },
    /// A future projection stated as certainty.
    UnhedgedProjection { claim: String },
    /// An org-like proper noun not found in the evidence text.
    UnverifiedEntity { claim: String, entity: String },
    /// Numeric claim that is an inference (allowed but discounted).
    InferredNumber { claim: String, value: String },
    /// The draft produced no extractable claims.
    NoClaims,
}

/// One claim's verification result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClaimVerification {
    pub claim: String,
    pub kind: ClaimKind,
    pub verdict: ClaimVerdict,
    /// Numbers checked in this claim.
    pub numbers_checked: usize,
    /// Numbers that matched evidence within tolerance.
    pub numbers_matched: usize,
    pub evidence_count: usize,
}

/// Aggregate verification report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerificationReport {
    /// Weighted supported fraction over non-recommendation claims (0..=1).
    pub factuality_score: f64,
    pub claims: Vec<ClaimVerification>,
    pub hard_violations: Vec<HardViolation>,
    pub soft_flags: Vec<SoftFlag>,
    pub evidence_numbers: usize,
    pub as_of: DateTime<Utc>,
}

impl VerificationReport {
    pub fn is_clean(&self) -> bool {
        self.hard_violations.is_empty()
    }
}

/// Verification configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerificationConfig {
    /// Relative tolerance when matching figures (default 10%).
    pub number_tolerance: f64,
    /// Absolute tolerance for small percentages/amounts (default 0.5).
    pub number_abs_tolerance: f64,
    /// Minimum factuality for publication (default 0.75).
    pub min_factuality: f64,
}

impl Default for VerificationConfig {
    fn default() -> Self {
        Self {
            number_tolerance: 0.10,
            number_abs_tolerance: 0.5,
            min_factuality: 0.75,
        }
    }
}

const CAUSAL_STARTS: [&str; 14] = [
    "because",
    "drove",
    "drives",
    "led to",
    "leads to",
    "triggered",
    "trigger",
    "forced",
    "forces",
    "due to",
    "as a result",
    "caused",
    "enabled",
    "pressures",
];

const FUTURE_MARKERS: [&str; 8] = [
    "will ",
    "expects to",
    "expected to",
    "plans to",
    "projected",
    "forecast",
    "by 20",
    "next year",
];

const ORG_SUFFIXES: [&str; 12] = [
    " inc",
    " ltd",
    " llc",
    " plc",
    " corp",
    " gmbh",
    " sarl",
    " sa",
    " ag",
    " co.",
    " group",
    " holdings",
];

const COMMON_CAPS: [&str; 41] = [
    "the", "a", "an", "in", "on", "at", "by", "for", "with", "and", "or", "but", "this", "that",
    "these", "those", "us", "u.s.", "uk", "u.k.", "eu", "e.u.", "un", "u.n.", "china", "india",
    "europe", "africa", "asia", "middle", "east", "north", "south", "west", "global", "q1", "q2",
    "q3", "q4", "h1", "h2",
];

fn percent_regex() -> &'static regex::Regex {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(r"(-?\d+(?:[.,]\d+)?)\s*%")
            .unwrap_or_else(|error| panic!("valid static regex: {error}"))
    })
}

fn currency_regex() -> &'static regex::Regex {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(r"[$€£]\s?(\d+(?:[.,]\d+)?)\s?(k|m|bn|billion|million|thousand)?")
            .unwrap_or_else(|error| panic!("valid static regex: {error}"))
    })
}

fn integer_regex() -> &'static regex::Regex {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(r"\b(\d{2,})\b")
            .unwrap_or_else(|error| panic!("valid static regex: {error}"))
    })
}

fn year_regex() -> &'static regex::Regex {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(r"\b(19\d{2}|20\d{2})\b")
            .unwrap_or_else(|error| panic!("valid static regex: {error}"))
    })
}

fn citation_regex() -> &'static regex::Regex {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(r"\[(\d{1,2})\]")
            .unwrap_or_else(|error| panic!("valid static regex: {error}"))
    })
}

fn parse_number(raw: &str) -> Option<f64> {
    raw.replace(',', ".").parse::<f64>().ok()
}

fn scale_suffix(suffix: Option<&str>) -> f64 {
    match suffix.unwrap_or("") {
        "k" | "thousand" => 1_000.0,
        "m" | "million" => 1_000_000.0,
        "bn" | "billion" => 1_000_000_000.0,
        _ => 1.0,
    }
}

/// Extract canonical numeric values from free text: percentages, currency
/// amounts (scaled), and standalone integers ≥ 10. Years are returned
/// separately by [`years_in`]; integer extraction intentionally skips them.
pub fn numbers_in(text: &str) -> Vec<(String, f64)> {
    let mut out: Vec<(String, f64)> = Vec::new();
    for captures in percent_regex().captures_iter(text) {
        if let Some(value) = captures.get(1).and_then(|m| parse_number(m.as_str())) {
            out.push((captures[0].trim().to_string(), value));
        }
    }
    for captures in currency_regex().captures_iter(text) {
        if let Some(value) = captures.get(1).and_then(|m| parse_number(m.as_str())) {
            let scaled = value * scale_suffix(captures.get(2).map(|m| m.as_str()));
            out.push((captures[0].trim().to_string(), scaled));
        }
    }
    for captures in integer_regex().captures_iter(text) {
        let raw = captures
            .get(1)
            .map(|m| m.as_str())
            .unwrap_or("")
            .to_string();
        let value = match raw.parse::<f64>() {
            Ok(value) => value,
            Err(_) => continue,
        };
        if (1900.0..=2100.0).contains(&value) {
            continue;
        }
        if value < 10.0 {
            continue;
        }
        out.push((raw, value));
    }
    out
}

/// Calendar years mentioned in text.
pub fn years_in(text: &str) -> BTreeSet<i32> {
    year_regex()
        .find_iter(text)
        .filter_map(|m| m.as_str().parse::<i32>().ok())
        .collect()
}

fn contains_causal(text_lower: &str) -> bool {
    CAUSAL_STARTS
        .iter()
        .any(|marker| text_lower.contains(marker))
}

fn is_projection(text_lower: &str) -> bool {
    FUTURE_MARKERS
        .iter()
        .any(|marker| text_lower.contains(marker))
}

/// Org-like capitalized tokens in the claim (multi-word runs ending in a
/// corporate suffix, or capitalized tokens ≥ 4 chars not in the common list).
fn org_like_tokens(text: &str) -> Vec<String> {
    let lower = text.to_ascii_lowercase();
    let mut out = Vec::new();
    for suffix in ORG_SUFFIXES {
        if lower.contains(suffix) {
            // Capture the preceding capitalized run: e.g. "Globex Ltd".
            let idx = lower.find(suffix).unwrap_or(0);
            let before = &text[..idx];
            let run: Vec<&str> = before
                .split_whitespace()
                .rev()
                .take(2)
                .take_while(|token| {
                    token
                        .chars()
                        .next()
                        .is_some_and(|first| first.is_uppercase())
                })
                .collect();
            if !run.is_empty() {
                let mut words: Vec<&str> = run.into_iter().rev().collect();
                let suffix_text = suffix.trim();
                let phrase = format!("{} {}", words.join(" "), capitalize(suffix_text));
                out.push(phrase.trim().to_string());
                words.clear();
            }
        }
    }
    for token in text.split_whitespace() {
        let clean: String = token
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '-')
            .collect();
        if clean.chars().count() >= 4
            && token
                .chars()
                .next()
                .is_some_and(|first| first.is_uppercase())
            && !COMMON_CAPS.contains(&clean.to_ascii_lowercase().as_str())
        {
            out.push(clean);
        }
    }
    out.sort();
    out.dedup();
    out
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str()),
        None => String::new(),
    }
}

/// Verify a set of atomic claims against evidence records.
pub fn verify_claims(
    claims: &[InsightClaim],
    evidence: &[EvidenceRecord],
    config: &VerificationConfig,
    as_of: DateTime<Utc>,
) -> VerificationReport {
    let evidence_text: String = evidence
        .iter()
        .map(|record| record.searchable_text())
        .collect::<Vec<_>>()
        .join("\n");
    let evidence_text_lower = evidence_text.to_ascii_lowercase();
    let evidence_numbers: Vec<f64> = numbers_in(&evidence_text)
        .into_iter()
        .map(|(_, value)| value)
        .collect();
    let evidence_years = years_in(&evidence_text);

    let mut verifications: Vec<ClaimVerification> = Vec::with_capacity(claims.len());
    let mut hard_violations: Vec<HardViolation> = Vec::new();
    let mut soft_flags: Vec<SoftFlag> = Vec::new();
    let mut supported_weight: f64 = 0.0;
    let mut total_weight: f64 = 0.0;

    for claim in claims {
        if claim.kind == ClaimKind::Recommendation {
            // Recommendations are exempt from support scoring, but fabricated
            // figures inside them are still fatal: a recommendation must not
            // smuggle invented statistics.
            let claim_numbers = numbers_in(&claim.claim);
            for (raw, value) in &claim_numbers {
                if !matches_any(*value, &evidence_numbers, config) && !evidence_numbers.is_empty() {
                    hard_violations.push(HardViolation::FabricatedNumber {
                        claim: claim.claim.clone(),
                        value: raw.clone(),
                    });
                }
            }
            verifications.push(ClaimVerification {
                claim: claim.claim.clone(),
                kind: claim.kind,
                verdict: ClaimVerdict::Unverifiable,
                numbers_checked: claim_numbers.len(),
                numbers_matched: 0,
                evidence_count: claim.evidence_ids.len(),
            });
            continue;
        }

        let weight = match claim.kind {
            ClaimKind::Observed => 1.0,
            ClaimKind::Inference => 0.7,
            ClaimKind::Unknown => 0.3,
            ClaimKind::Recommendation => 0.0,
        };
        total_weight += weight;

        let claim_lower = claim.claim.to_ascii_lowercase();
        let claim_numbers = numbers_in(&claim.claim);
        let mut numbers_matched = 0usize;
        let mut fabricated = false;
        for (raw, value) in &claim_numbers {
            if matches_any(*value, &evidence_numbers, config) {
                numbers_matched += 1;
            } else if !evidence_numbers.is_empty() {
                match claim.kind {
                    ClaimKind::Observed => {
                        fabricated = true;
                        hard_violations.push(HardViolation::FabricatedNumber {
                            claim: claim.claim.clone(),
                            value: raw.clone(),
                        });
                    }
                    ClaimKind::Inference => {
                        soft_flags.push(SoftFlag::InferredNumber {
                            claim: claim.claim.clone(),
                            value: raw.clone(),
                        });
                    }
                    _ => {
                        soft_flags.push(SoftFlag::InferredNumber {
                            claim: claim.claim.clone(),
                            value: raw.clone(),
                        });
                    }
                }
            }
        }

        // Citations must resolve within the supplied evidence range.
        for captures in citation_regex().captures_iter(&claim.claim) {
            if let Some(ordinal) = captures
                .get(1)
                .and_then(|m| m.as_str().parse::<usize>().ok())
            {
                if ordinal == 0 || ordinal > evidence.len() {
                    hard_violations.push(HardViolation::CitationOutOfRange {
                        claim: claim.claim.clone(),
                        ordinal,
                        evidence_count: evidence.len(),
                    });
                }
            }
        }

        // Dates: past years asserted as observed must appear in evidence.
        for year in years_in(&claim.claim) {
            if year <= as_of.year()
                && !evidence_years.contains(&year)
                && claim.kind == ClaimKind::Observed
            {
                hard_violations.push(HardViolation::UnverifiedDate {
                    claim: claim.claim.clone(),
                    date: year.to_string(),
                });
            }
        }

        let causal = contains_causal(&claim_lower);
        if causal && claim.kind == ClaimKind::Observed && claim.evidence_ids.len() < 2 {
            soft_flags.push(SoftFlag::ThinCausality {
                claim: claim.claim.clone(),
                evidence_count: claim.evidence_ids.len(),
            });
        }
        if (causal || is_projection(&claim_lower)) && claim.kind == ClaimKind::Observed {
            let absolute = ["will ", "certainly", "proves", "guarantees"]
                .iter()
                .any(|marker| claim_lower.contains(marker));
            if absolute {
                soft_flags.push(SoftFlag::UnhedgedProjection {
                    claim: claim.claim.clone(),
                });
            }
        }

        // Org-like entities must be grounded in the evidence text (soft flag:
        // proper-noun heuristics have false positives).
        if !evidence.is_empty() {
            for entity in org_like_tokens(&claim.claim) {
                if !evidence_text_lower.contains(&entity.to_ascii_lowercase()) {
                    soft_flags.push(SoftFlag::UnverifiedEntity {
                        claim: claim.claim.clone(),
                        entity,
                    });
                }
            }
        }

        let verdict = if fabricated {
            ClaimVerdict::Unsupported
        } else {
            match claim.kind {
                ClaimKind::Observed => {
                    let partial = (!claim_numbers.is_empty()
                        && numbers_matched < claim_numbers.len())
                        || (causal && claim.evidence_ids.len() < 2);
                    if claim.evidence_ids.is_empty() {
                        ClaimVerdict::Unverifiable
                    } else if partial {
                        ClaimVerdict::PartiallySupported
                    } else {
                        ClaimVerdict::Supported
                    }
                }
                ClaimKind::Inference => {
                    // An inference's factual basis is its cited evidence; the
                    // hedge is honest analysis, not a support deficit. An
                    // explicitly-hedged judgment without a citation keeps
                    // partial credit (it is labelled as judgment, not fact);
                    // an unhedged uncited assertion is Unknown and scores
                    // zero.
                    if claim.evidence_ids.is_empty() {
                        ClaimVerdict::PartiallySupported
                    } else {
                        ClaimVerdict::Supported
                    }
                }
                ClaimKind::Unknown => ClaimVerdict::Unsupported,
                ClaimKind::Recommendation => ClaimVerdict::Unverifiable,
            }
        };

        supported_weight += weight
            * match verdict {
                ClaimVerdict::Supported => 1.0,
                ClaimVerdict::PartiallySupported => 0.5,
                ClaimVerdict::Unsupported | ClaimVerdict::Unverifiable => 0.0,
            };

        verifications.push(ClaimVerification {
            claim: claim.claim.clone(),
            kind: claim.kind,
            verdict,
            numbers_checked: claim_numbers.len(),
            numbers_matched,
            evidence_count: claim.evidence_ids.len(),
        });
    }

    if claims.is_empty() {
        soft_flags.push(SoftFlag::NoClaims);
    }

    let factuality_score = if total_weight <= 0.0 {
        if claims.is_empty() {
            0.5
        } else {
            1.0
        }
    } else {
        (supported_weight / total_weight).clamp(0.0, 1.0)
    };

    VerificationReport {
        factuality_score,
        claims: verifications,
        hard_violations,
        soft_flags,
        evidence_numbers: evidence_numbers.len(),
        as_of,
    }
}

fn matches_any(value: f64, candidates: &[f64], config: &VerificationConfig) -> bool {
    candidates.iter().any(|candidate| {
        let abs = (candidate - value).abs();
        abs <= config.number_abs_tolerance
            || (value.abs() > f64::EPSILON && abs / value.abs() <= config.number_tolerance)
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::analytical::EvidenceRecord;
    use uuid::Uuid;

    fn evidence() -> Vec<EvidenceRecord> {
        vec![
            EvidenceRecord {
                evidence_id: Uuid::from_u128(1),
                title: "Ministry filing".into(),
                text: "Duty rates rose 12% in 2025 while imports reached $3.5m according to Globex Ltd.".into(),
                source_name: "trade.gov".into(),
                source_url: Some("https://trade.gov/a".into()),
                signal_type: "government_registry".into(),
                observed_at: None,
                reliability: 0.9,
            },
            EvidenceRecord {
                evidence_id: Uuid::from_u128(2),
                title: "Wire coverage".into(),
                text: "Buyers re-quoted contracts after the tariff change in 2025.".into(),
                source_name: "Reuters".into(),
                source_url: Some("https://reuters.com/b".into()),
                signal_type: "news".into(),
                observed_at: None,
                reliability: 0.8,
            },
        ]
    }

    fn claim(text: &str, kind: ClaimKind, evidence_ids: Vec<Uuid>) -> InsightClaim {
        InsightClaim::new(text.to_string(), evidence_ids, Some(0.8), kind)
    }

    fn as_of() -> DateTime<Utc> {
        DateTime::from_timestamp(1_780_000_000, 0).unwrap_or_else(|| panic!("valid timestamp"))
    }

    #[test]
    fn fabricated_number_is_a_hard_violation() {
        let claims = vec![claim(
            "Duty rates rose 47% in 2025 [1].",
            ClaimKind::Observed,
            vec![Uuid::from_u128(1)],
        )];
        let report = verify_claims(
            &claims,
            &evidence(),
            &VerificationConfig::default(),
            as_of(),
        );
        assert!(report
            .hard_violations
            .iter()
            .any(|violation| matches!(violation, HardViolation::FabricatedNumber { .. })));
        assert!((report.factuality_score - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn number_within_tolerance_is_supported() {
        let claims = vec![claim(
            "Duty rates rose 12% in 2025 [1].",
            ClaimKind::Observed,
            vec![Uuid::from_u128(1)],
        )];
        let report = verify_claims(
            &claims,
            &evidence(),
            &VerificationConfig::default(),
            as_of(),
        );
        assert!(report.hard_violations.is_empty());
        assert_eq!(report.claims[0].verdict, ClaimVerdict::Supported);
        assert!((report.factuality_score - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn out_of_range_citation_is_hard() {
        let claims = vec![claim(
            "Rates rose 12% [7].",
            ClaimKind::Observed,
            vec![Uuid::from_u128(1)],
        )];
        let report = verify_claims(
            &claims,
            &evidence(),
            &VerificationConfig::default(),
            as_of(),
        );
        assert!(report.hard_violations.iter().any(|violation| matches!(
            violation,
            HardViolation::CitationOutOfRange { ordinal: 7, .. }
        )));
    }

    #[test]
    fn past_year_not_in_evidence_is_hard_but_future_projection_is_not() {
        let bad = vec![claim(
            "The plant opened in 2019 [1].",
            ClaimKind::Observed,
            vec![Uuid::from_u128(2)],
        )];
        let report = verify_claims(&bad, &evidence(), &VerificationConfig::default(), as_of());
        assert!(report
            .hard_violations
            .iter()
            .any(|violation| matches!(violation, HardViolation::UnverifiedDate { date, .. } if date == "2019")));

        let projection = vec![claim(
            "Output will rise in 2027 [1].",
            ClaimKind::Observed,
            vec![Uuid::from_u128(2)],
        )];
        let report = verify_claims(
            &projection,
            &evidence(),
            &VerificationConfig::default(),
            as_of(),
        );
        assert!(report.hard_violations.is_empty());
        assert!(report
            .soft_flags
            .iter()
            .any(|flag| matches!(flag, SoftFlag::UnhedgedProjection { .. })));
    }

    #[test]
    fn single_source_causality_is_partially_supported() {
        let claims = vec![claim(
            "Tariffs drove the re-quote wave in 2025 [2].",
            ClaimKind::Observed,
            vec![Uuid::from_u128(2)],
        )];
        let report = verify_claims(
            &claims,
            &evidence(),
            &VerificationConfig::default(),
            as_of(),
        );
        assert_eq!(report.claims[0].verdict, ClaimVerdict::PartiallySupported);
        assert!(report
            .soft_flags
            .iter()
            .any(|flag| matches!(flag, SoftFlag::ThinCausality { .. })));
    }

    #[test]
    fn inference_numbers_are_soft_not_hard() {
        let claims = vec![claim(
            "Costs could reach 33% by 2027 [1].",
            ClaimKind::Inference,
            vec![Uuid::from_u128(1)],
        )];
        let report = verify_claims(
            &claims,
            &evidence(),
            &VerificationConfig::default(),
            as_of(),
        );
        assert!(report.hard_violations.is_empty());
        assert!(report
            .soft_flags
            .iter()
            .any(|flag| matches!(flag, SoftFlag::InferredNumber { .. })));
    }

    #[test]
    fn recommendation_cannot_smuggle_fabricated_statistics() {
        let claims = vec![claim(
            "We recommend locking 87% of volume now.",
            ClaimKind::Recommendation,
            vec![],
        )];
        let report = verify_claims(
            &claims,
            &evidence(),
            &VerificationConfig::default(),
            as_of(),
        );
        assert!(report
            .hard_violations
            .iter()
            .any(|violation| matches!(violation, HardViolation::FabricatedNumber { .. })));
    }

    #[test]
    fn unverified_org_like_entity_is_soft() {
        let claims = vec![claim(
            "Zenith Dynamics Ltd won the tender in 2025 [1].",
            ClaimKind::Observed,
            vec![Uuid::from_u128(1)],
        )];
        let report = verify_claims(
            &claims,
            &evidence(),
            &VerificationConfig::default(),
            as_of(),
        );
        assert!(report.hard_violations.is_empty());
        assert!(report
            .soft_flags
            .iter()
            .any(|flag| matches!(flag, SoftFlag::UnverifiedEntity { .. })));
    }

    #[test]
    fn unknown_claims_are_unsupported() {
        let claims = vec![claim(
            "Something significant happened somewhere.",
            ClaimKind::Unknown,
            vec![],
        )];
        let report = verify_claims(
            &claims,
            &evidence(),
            &VerificationConfig::default(),
            as_of(),
        );
        assert_eq!(report.claims[0].verdict, ClaimVerdict::Unsupported);
        assert!((report.factuality_score - 0.0).abs() < f64::EPSILON);
    }
}
