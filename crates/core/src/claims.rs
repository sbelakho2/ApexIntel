//! Claim-level evidence (audit P0 #23).
//!
//! An insight is not a single atomic statement: it mixes observed facts,
//! system inference, and recommendations. [`InsightClaim`] decomposes an
//! insight into discrete claims so the UI can cite evidence per claim and
//! label what the system actually knows.
//!
//! [`backfill_claims`] is the migration/backfill policy: it only re-uses
//! claims that were already structured with their evidence ids, and otherwise
//! returns a single `unknown` claim with **no** evidence ids. It never
//! fabricates citations for an insight whose provenance is not recorded.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// What kind of statement a claim is.
///
/// The distinction is user-facing: an observed fact is grounded in the cited
/// evidence; a system inference is the platform's interpretation and can be
/// wrong; a recommendation is a suggested action, not a fact; `Unknown` means
/// the platform could not establish the claim's provenance.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaimKind {
    /// A fact stated in the cited evidence.
    Observed,
    /// The system's interpretation of the evidence.
    Inference,
    /// A suggested action, not a factual statement.
    Recommendation,
    /// Provenance could not be established; rendered as clearly unresolved.
    #[default]
    Unknown,
}

impl ClaimKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ClaimKind::Observed => "observed",
            ClaimKind::Inference => "inference",
            ClaimKind::Recommendation => "recommendation",
            ClaimKind::Unknown => "unknown",
        }
    }

    /// Parse a persisted/database string; unrecognised values map to
    /// [`ClaimKind::Unknown`] rather than guessing.
    pub fn from_db(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "observed" | "fact" => ClaimKind::Observed,
            "inference" | "inferred" => ClaimKind::Inference,
            "recommendation" | "recommended" | "action" => ClaimKind::Recommendation,
            _ => ClaimKind::Unknown,
        }
    }

    /// Human label used on the insight detail page.
    pub fn label(self) -> &'static str {
        match self {
            ClaimKind::Observed => "Observed fact",
            ClaimKind::Inference => "System inference",
            ClaimKind::Recommendation => "Recommendation",
            ClaimKind::Unknown => "Evidence unknown",
        }
    }
}

impl std::fmt::Display for ClaimKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A single claim backed by zero or more evidence row ids.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InsightClaim {
    pub claim: String,
    #[serde(default)]
    pub evidence_ids: Vec<Uuid>,
    pub confidence: Option<f64>,
    #[serde(default)]
    pub kind: ClaimKind,
}

impl InsightClaim {
    pub fn new(
        claim: impl Into<String>,
        evidence_ids: Vec<Uuid>,
        confidence: Option<f64>,
        kind: ClaimKind,
    ) -> Self {
        Self {
            claim: claim.into(),
            evidence_ids,
            confidence,
            kind,
        }
    }

    /// A claim whose provenance is unrecorded. Deliberately carries no evidence
    /// ids: callers must not invent them.
    pub fn unknown(claim: impl Into<String>) -> Self {
        Self {
            claim: claim.into(),
            evidence_ids: Vec::new(),
            confidence: None,
            kind: ClaimKind::Unknown,
        }
    }
}

/// Plan the backfill claims for an insight that has none yet.
///
/// - Structured claims already stored in `metadata["claims"]` (with evidence
///   ids) are re-used verbatim, except that a claim with no evidence ids is
///   downgraded to [`ClaimKind::Unknown`] — its evidence cannot be vouched for.
/// - Otherwise a single `unknown` claim derived from the insight title is
///   returned, with no evidence ids. Fabricating per-sentence claims or
///   citations here would invent provenance that was never recorded.
pub fn backfill_claims(title: &str, metadata: Option<&serde_json::Value>) -> Vec<InsightClaim> {
    if let Some(claims) = metadata
        .and_then(|m| m.get("claims"))
        .and_then(|v| v.as_array())
    {
        let parsed: Vec<InsightClaim> = claims.iter().filter_map(parse_structured_claim).collect();
        if !parsed.is_empty() {
            return parsed;
        }
    }

    let text = title.trim();
    let fallback = if text.is_empty() {
        "Untitled insight (no record of evidence)"
    } else {
        text
    };
    vec![InsightClaim::unknown(fallback)]
}

/// Parse one entry of a structured `metadata["claims"]` array.
///
/// Returns `None` when the claim text is missing/empty. Entries whose
/// evidence ids cannot be parsed keep the claim as `unknown` with no ids
/// rather than dropping evidence silently.
pub fn parse_structured_claim(value: &serde_json::Value) -> Option<InsightClaim> {
    let claim = value.get("claim")?.as_str()?.trim().to_string();
    if claim.is_empty() {
        return None;
    }
    let evidence_ids: Vec<Uuid> = value
        .get("evidence_ids")
        .and_then(|v| v.as_array())
        .map(|ids| {
            ids.iter()
                .filter_map(|id| id.as_str().and_then(|s| Uuid::parse_str(s).ok()))
                .collect()
        })
        .unwrap_or_default();
    let confidence = value.get("confidence").and_then(|v| v.as_f64());
    let kind = value
        .get("kind")
        .or_else(|| value.get("claim_kind"))
        .and_then(|v| v.as_str())
        .map(ClaimKind::from_db)
        .unwrap_or(ClaimKind::Unknown);
    // Never claim observed/inferred/recommended status without evidence.
    let kind = if evidence_ids.is_empty() && kind != ClaimKind::Unknown {
        ClaimKind::Unknown
    } else {
        kind
    };
    Some(InsightClaim {
        claim,
        evidence_ids,
        confidence,
        kind,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn backfill_marks_evidence_less_insights_unknown_without_ids() {
        let claims = backfill_claims("Company X won a tender", None);
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0].kind, ClaimKind::Unknown);
        assert!(claims[0].evidence_ids.is_empty());
        assert!(claims[0].confidence.is_none());
        assert!(claims[0].claim.contains("tender"));
    }

    #[test]
    fn backfill_reuses_structured_metadata_claims() {
        let id = Uuid::nil();
        let metadata = json!({
            "claims": [
                {"claim": "Observed: plant opened", "evidence_ids": [id.to_string()], "confidence": 0.9, "kind": "observed"},
                {"claim": "Likely to expand", "evidence_ids": [id.to_string()], "confidence": 0.6, "kind": "inference"}
            ]
        });
        let claims = backfill_claims("ignored", Some(&metadata));
        assert_eq!(claims.len(), 2);
        assert_eq!(claims[0].kind, ClaimKind::Observed);
        assert_eq!(claims[0].evidence_ids, vec![id]);
        assert_eq!(claims[1].kind, ClaimKind::Inference);
    }

    #[test]
    fn backfill_downgrades_metadata_claims_without_evidence_to_unknown() {
        let metadata = json!({
            "claims": [{"claim": "Unverifiable assertion", "confidence": 0.99, "kind": "observed"}]
        });
        let claims = backfill_claims("title", Some(&metadata));
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0].kind, ClaimKind::Unknown);
        assert!(claims[0].evidence_ids.is_empty());
    }

    #[test]
    fn backfill_ignores_malformed_claims_and_falls_back_to_unknown() {
        let metadata = json!({"claims": [{"confidence": 0.4}, "not-an-object"]});
        let claims = backfill_claims("Title", Some(&metadata));
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0].kind, ClaimKind::Unknown);
        assert_eq!(claims[0].claim, "Title");
    }

    #[test]
    fn claim_kind_round_trips_and_unknown_is_default() {
        for kind in [
            ClaimKind::Observed,
            ClaimKind::Inference,
            ClaimKind::Recommendation,
            ClaimKind::Unknown,
        ] {
            assert_eq!(ClaimKind::from_db(kind.as_str()), kind);
        }
        assert_eq!(ClaimKind::from_db("nonsense"), ClaimKind::Unknown);
        assert_eq!(ClaimKind::default(), ClaimKind::Unknown);
    }
}
