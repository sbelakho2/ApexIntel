//! Claim extraction for generated insights (audit P0 #23).
//!
//! The insight generator writes a narrative using numeric citations (`[1]`,
//! `[2]`, …) that index the evidence signals supplied to the model. This
//! module turns that narrative into structured [`InsightClaim`]s:
//!
//! - each claim keeps the evidence **row ids** cited in its sentence, so the
//!   UI can render inline citations that resolve to real source rows;
//! - each claim is labelled `observed`, `inference`, `recommendation`, or
//!   `unknown`, so a statement the model made without citing evidence is never
//!   presented as a fact;
//! - citation ordinals that do not resolve to a supplied evidence row are
//!   dropped. The extractor never invents evidence ids.

use apex_core::claims::{ClaimKind, InsightClaim};
use uuid::Uuid;

/// One piece of evidence available at generation time.
///
/// `evidence_id` is the stable database id (e.g. an `observations.id`) that a
/// citation ordinal maps to; `source_url` is the human-facing URL, persisted
/// separately as an evidence reference so the UI can link citations.
#[derive(Clone, Debug, PartialEq)]
pub struct ClaimEvidenceRef {
    pub evidence_id: Uuid,
    pub source_url: Option<String>,
}

impl ClaimEvidenceRef {
    pub fn new(evidence_id: Uuid, source_url: Option<String>) -> Self {
        Self {
            evidence_id,
            source_url,
        }
    }
}

const MIN_CLAIM_CHARS: usize = 20;
const DEFAULT_MAX_CLAIMS: usize = 12;

const INFERENCE_MARKERS: [&str; 12] = [
    "likely",
    "suggests",
    "suggesting",
    "indicates",
    "might",
    "could",
    "expected to",
    "appears to",
    "seems to",
    "probably",
    "implies",
    "points to",
];

const RECOMMENDATION_MARKERS: [&str; 12] = [
    "recommend",
    "should ",
    "next step",
    "we suggest",
    "prioritize",
    "engage ",
    "reach out",
    "outreach",
    "consider ",
    "schedule ",
    "target ",
    "action:",
];

/// Split prose into claim-sized sentences.
///
/// Deliberately simple: sentence terminators plus line breaks, with a minimum
/// length so headings and list fragments do not become claims.
pub fn split_claim_sentences(text: &str) -> Vec<String> {
    let mut sentences = Vec::new();
    for line in text.lines() {
        let mut current = String::new();
        for ch in line.chars() {
            current.push(ch);
            if matches!(ch, '.' | '!' | '?') {
                push_sentence(&mut sentences, &mut current);
            }
        }
        push_sentence(&mut sentences, &mut current);
    }
    sentences
}

fn push_sentence(out: &mut Vec<String>, current: &mut String) {
    let trimmed = current.trim();
    if trimmed.chars().count() >= MIN_CLAIM_CHARS {
        out.push(trimmed.to_string());
    }
    current.clear();
}

/// Parse the 1-based citation ordinals in a sentence (`[1]`, `[2][3]`).
///
/// Returns ordinals in first-seen order; malformed brackets are ignored.
pub fn citation_ordinals(sentence: &str) -> Vec<usize> {
    let bytes = sentence.as_bytes();
    let mut ordinals: Vec<usize> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'[' {
            let start = i + 1;
            let mut end = start;
            while end < bytes.len() && bytes[end].is_ascii_digit() {
                end += 1;
            }
            if end > start && end < bytes.len() && bytes[end] == b']' {
                if let Ok(value) = sentence[start..end].parse::<usize>() {
                    if !ordinals.contains(&value) {
                        ordinals.push(value);
                    }
                }
                i = end + 1;
                continue;
            }
        }
        i += 1;
    }
    ordinals
}

/// Classify a narrative sentence.
///
/// Hedged language wins over citations (a hedged statement is an inference,
/// even when it cites evidence); a plain cited statement is observed; an
/// uncited statement without hedging is `unknown`, never observed.
pub fn classify_claim(sentence: &str, cited_evidence: usize) -> ClaimKind {
    let lower = sentence.to_ascii_lowercase();
    if RECOMMENDATION_MARKERS
        .iter()
        .any(|marker| lower.contains(marker))
    {
        return ClaimKind::Recommendation;
    }
    if INFERENCE_MARKERS
        .iter()
        .any(|marker| lower.contains(marker))
    {
        return ClaimKind::Inference;
    }
    if cited_evidence > 0 {
        ClaimKind::Observed
    } else {
        ClaimKind::Unknown
    }
}

fn resolve_citations(ordinals: &[usize], evidence: &[ClaimEvidenceRef]) -> Vec<Uuid> {
    let mut ids: Vec<Uuid> = Vec::new();
    for ordinal in ordinals {
        if let Some(reference) = ordinal.checked_sub(1).and_then(|index| evidence.get(index)) {
            if !ids.contains(&reference.evidence_id) {
                ids.push(reference.evidence_id);
            }
        }
    }
    ids
}

/// Extract structured claims from a generated narrative and recommendation.
///
/// Recommendation-section sentences are always labelled `recommendation`
/// (they are suggested actions, not facts). Narrative sentences are labelled
/// by [`classify_claim`]. Citations that do not resolve to a supplied evidence
/// row are dropped, and uncited claims carry an empty `evidence_ids` list.
pub fn extract_claims(
    narrative: &str,
    recommendation: &str,
    evidence: &[ClaimEvidenceRef],
    confidence: f64,
    max_claims: Option<usize>,
) -> Vec<InsightClaim> {
    let max_claims = max_claims.unwrap_or(DEFAULT_MAX_CLAIMS).max(1);
    let mut claims: Vec<InsightClaim> = Vec::new();

    for sentence in split_claim_sentences(narrative) {
        let cited = resolve_citations(&citation_ordinals(&sentence), evidence);
        let kind = classify_claim(&sentence, cited.len());
        claims.push(InsightClaim::new(sentence, cited, Some(confidence), kind));
        if claims.len() >= max_claims {
            return claims;
        }
    }

    for sentence in split_claim_sentences(recommendation) {
        let cited = resolve_citations(&citation_ordinals(&sentence), evidence);
        claims.push(InsightClaim::new(
            sentence,
            cited,
            Some(confidence),
            ClaimKind::Recommendation,
        ));
        if claims.len() >= max_claims {
            return claims;
        }
    }

    claims
}

/// Serialize claims for `insights.metadata["claims"]`.
///
/// Persisting the structured claims alongside the insight lets backfills
/// recover real evidence ids instead of downgrading to `unknown`.
pub fn claims_to_metadata(claims: &[InsightClaim]) -> serde_json::Value {
    serde_json::Value::Array(
        claims
            .iter()
            .map(|claim| {
                serde_json::json!({
                    "claim": claim.claim,
                    "evidence_ids": claim
                        .evidence_ids
                        .iter()
                        .map(Uuid::to_string)
                        .collect::<Vec<String>>(),
                    "confidence": claim.confidence,
                    "kind": claim.kind.as_str(),
                })
            })
            .collect(),
    )
}

/// Evidence reference index for `insights.metadata["evidence_refs"]`.
///
/// Each entry maps an evidence row id to its source URL so the detail page can
/// turn a claim's `evidence_ids` into inline citation links without guessing.
pub fn evidence_refs_to_metadata(evidence: &[ClaimEvidenceRef]) -> serde_json::Value {
    serde_json::Value::Array(
        evidence
            .iter()
            .filter_map(|reference| {
                reference.source_url.as_ref().map(|url| {
                    serde_json::json!({
                        "id": reference.evidence_id.to_string(),
                        "url": url,
                    })
                })
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn evidence(n: usize) -> Vec<ClaimEvidenceRef> {
        (0..n)
            .map(|i| ClaimEvidenceRef::new(Uuid::from_u128(i as u128 + 1), None))
            .collect()
    }

    #[test]
    fn cited_sentences_become_observed_claims_with_evidence_ids() {
        let refs = evidence(2);
        let claims = extract_claims(
            "The company announced a new plant in Tunis [1]. Output is likely to double [2].",
            "",
            &refs,
            0.8,
            None,
        );
        assert_eq!(claims.len(), 2);
        assert_eq!(claims[0].kind, ClaimKind::Observed);
        assert_eq!(claims[0].evidence_ids, vec![refs[0].evidence_id]);
        // "expected to" is an inference marker, not an observed fact.
        assert_eq!(claims[1].kind, ClaimKind::Inference);
        assert_eq!(claims[1].evidence_ids, vec![refs[1].evidence_id]);
    }

    #[test]
    fn uncited_sentences_are_unknown_without_evidence_ids() {
        let claims = extract_claims(
            "Something happened here without any citation attached to it.",
            "",
            &evidence(3),
            0.9,
            None,
        );
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0].kind, ClaimKind::Unknown);
        assert!(claims[0].evidence_ids.is_empty());
    }

    #[test]
    fn out_of_range_citations_are_dropped_not_invented() {
        let refs = evidence(1);
        let claims = extract_claims(
            "A reported fact cites a source that was never supplied [7].",
            "",
            &refs,
            0.7,
            None,
        );
        assert_eq!(claims.len(), 1);
        assert!(claims[0].evidence_ids.is_empty());
        assert_eq!(claims[0].kind, ClaimKind::Unknown);
    }

    #[test]
    fn recommendation_section_is_labelled_recommendation() {
        let claims = extract_claims(
            "The company opened a facility in Casablanca [1].",
            "We recommend scheduling a discovery call with the procurement lead.",
            &evidence(1),
            0.75,
            None,
        );
        assert_eq!(claims.len(), 2);
        assert_eq!(claims[0].kind, ClaimKind::Observed);
        assert_eq!(claims[1].kind, ClaimKind::Recommendation);
    }

    #[test]
    fn claims_are_capped() {
        let text = "Alpha statement with a citation [1]. Bravo statement with a citation [1]. Charlie statement with a citation [1]. Delta statement with a citation [1].";
        let claims = extract_claims(text, "", &evidence(1), 0.5, Some(2));
        assert_eq!(claims.len(), 2);
    }

    #[test]
    fn metadata_round_trips_through_core_backfill() {
        let refs = evidence(1);
        let claims = extract_claims(
            "The supplier filed for certification last week [1].",
            "",
            &refs,
            0.8,
            None,
        );
        let metadata = serde_json::json!({ "claims": claims_to_metadata(&claims) });
        let recovered = apex_core::claims::backfill_claims("ignored", Some(&metadata));
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].kind, ClaimKind::Observed);
        assert_eq!(recovered[0].evidence_ids, vec![refs[0].evidence_id]);
    }

    #[test]
    fn evidence_ref_metadata_lists_only_url_backed_rows() {
        let with_url = ClaimEvidenceRef::new(Uuid::from_u128(7), Some("https://x.test/a".into()));
        let without = ClaimEvidenceRef::new(Uuid::from_u128(8), None);
        let value = evidence_refs_to_metadata(&[with_url, without]);
        let arr = value.as_array().expect("array");
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["id"], Uuid::from_u128(7).to_string());
        assert_eq!(arr[0]["url"], "https://x.test/a");
    }
}
