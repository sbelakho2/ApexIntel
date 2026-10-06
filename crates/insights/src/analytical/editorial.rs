//! The editorial board — the single gate every analytical product passes
//! before publication.
//!
//! The board runs the full review stack (depth → verification → warrants →
//! calibration) and returns a [`EditorialReview`] with a verdict, the final
//! (recalibrated, evidence-capped) confidence, and every reason the verdict
//! was reached. Hard violations reject; soft shortfalls demand revision; only
//! a product meeting all standards publishes.
//!
//! Verdict semantics:
//! - `Publish`: all standards met; `final_confidence` is the product's
//!   measured, calibrated confidence.
//! - `Revise`: the analysis is salvageable but below standard — it must not
//!   be published as-is. Callers either drop it, or publish with the returned
//!   `final_confidence` and the `editorial:revise` tag when the product class
//!   tolerates a revision-flagged release.
//! - `Reject`: hard integrity failure; never publish.

use super::argumentation::{build_warrants, ExternalAttack, WarrantReport};
use super::calibration::CalibrationCurve;
use super::depth::{self, DepthConfig, DepthReport};
use super::verification::{self, VerificationConfig, VerificationReport};
use super::EvidenceRecord;
use apex_core::claims::InsightClaim;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Editorial verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EditorialVerdict {
    Publish,
    Revise,
    Reject,
}

/// Publication thresholds (all independently enforceable).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EditorialConfig {
    pub depth: DepthConfig,
    pub verification: VerificationConfig,
    /// Minimum overall warrant (default 0.55).
    pub min_warrant: f64,
    /// Minimum evidence-family independence score, unless the evidence is
    /// primarily official (a single primary register is itself authoritative).
    pub min_source_independence: f64,
    /// Verdict becomes `Reject` (not merely `Revise`) when warrant is this low.
    pub reject_warrant_floor: f64,
    /// Verdict becomes `Reject` when factuality is this low.
    pub reject_factuality_floor: f64,
}

impl Default for EditorialConfig {
    fn default() -> Self {
        Self {
            depth: DepthConfig::default(),
            verification: VerificationConfig::default(),
            min_warrant: 0.55,
            min_source_independence: 0.45,
            reject_warrant_floor: 0.35,
            reject_factuality_floor: 0.50,
        }
    }
}

/// Everything the board reviews.
#[derive(Debug, Clone)]
pub struct EditorialInputs<'a> {
    pub headline: &'a str,
    pub narrative: &'a str,
    pub recommendations: &'a str,
    pub claims: &'a [InsightClaim],
    pub evidence: &'a [EvidenceRecord],
    /// Model/analyst stated confidence before review.
    pub stated_confidence: f64,
    /// Alternative hypotheses attached to the product.
    pub alternative_hypotheses: usize,
    /// Red-team attacks from other subsystems.
    pub external_attacks: &'a [ExternalAttack],
    /// Fitted calibration curve, when the system has one.
    pub calibration: Option<&'a CalibrationCurve>,
    /// Deterministic review clock.
    pub as_of: DateTime<Utc>,
}

/// The complete, persistable review.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EditorialReview {
    pub verdict: EditorialVerdict,
    pub depth: DepthReport,
    pub verification: VerificationReport,
    pub warrants: WarrantReport,
    /// Recalibrated and evidence-capped confidence.
    pub final_confidence: f64,
    /// The confidence cap implied by evidence and warrant (before calibration).
    pub evidence_cap: f64,
    /// Reasons the verdict is not `Publish` (empty on publish).
    pub reasons: Vec<String>,
    /// Machine-readable release tags to attach (e.g. `editorial:revise`).
    pub tags: Vec<String>,
}

/// Run the full editorial review.
pub fn review(inputs: &EditorialInputs<'_>, config: &EditorialConfig) -> EditorialReview {
    let depth_inputs = depth::DepthInputs {
        headline: inputs.headline,
        narrative: inputs.narrative,
        recommendations: inputs.recommendations,
        claims: inputs.claims,
        evidence: inputs.evidence,
        confidence: inputs.stated_confidence,
        alternative_hypotheses: inputs.alternative_hypotheses,
    };
    let depth_report = depth::assess_depth(&depth_inputs, &config.depth);

    let verification_report = verification::verify_claims(
        inputs.claims,
        inputs.evidence,
        &config.verification,
        inputs.as_of,
    );

    let warrants = build_warrants(
        inputs.claims,
        inputs.evidence,
        &verification_report,
        inputs.external_attacks,
    );

    let independence_score = depth_report
        .components
        .iter()
        .find(|component| component.name == "source_independence")
        .map(|component| component.score)
        .unwrap_or(0.0);
    let official_only = inputs.evidence.iter().all(|record| {
        super::SourceClass::classify(&record.signal_type, &record.source_name)
            == super::SourceClass::Official
    }) && !inputs.evidence.is_empty();

    let mut reasons: Vec<String> = Vec::new();
    let mut tags: Vec<String> = Vec::new();
    let mut reject = false;

    if !verification_report.hard_violations.is_empty() {
        reject = true;
        reasons.push(format!(
            "{} hard integrity violation(s): fabricated figures, unverified dates, or out-of-range citations",
            verification_report.hard_violations.len()
        ));
        tags.push("editorial:integrity_violation".to_string());
    }
    if warrants.overall < config.reject_warrant_floor {
        reject = true;
        reasons.push(format!(
            "overall warrant {:.2} below the reject floor {:.2}",
            warrants.overall, config.reject_warrant_floor
        ));
    }
    if verification_report.factuality_score < config.reject_factuality_floor {
        reject = true;
        reasons.push(format!(
            "factuality {:.2} below the reject floor {:.2}",
            verification_report.factuality_score, config.reject_factuality_floor
        ));
    }

    let mut revise = false;
    if depth_report.index < config.depth.min_index {
        revise = true;
        reasons.push(format!(
            "depth index {:.2} below standard {:.2} (tier {})",
            depth_report.index,
            config.depth.min_index,
            depth_report.tier.as_str()
        ));
        tags.push("editorial:revise".to_string());
    }
    if verification_report.factuality_score < config.verification.min_factuality {
        revise = true;
        reasons.push(format!(
            "factuality {:.2} below standard {:.2}",
            verification_report.factuality_score, config.verification.min_factuality
        ));
        tags.push("editorial:revise".to_string());
    }
    if warrants.overall < config.min_warrant {
        revise = true;
        if let Some(weakest) = warrants.weakest_claim.as_deref() {
            reasons.push(format!(
                "overall warrant {:.2} below standard {:.2}; weakest link: {}",
                warrants.overall,
                config.min_warrant,
                truncate(weakest, 140)
            ));
        } else {
            reasons.push(format!(
                "overall warrant {:.2} below standard {:.2}",
                warrants.overall, config.min_warrant
            ));
        }
        tags.push("editorial:revise".to_string());
    }
    if independence_score < config.min_source_independence && !official_only {
        revise = true;
        reasons.push(format!(
            "source independence {:.2} below standard {:.2}",
            independence_score, config.min_source_independence
        ));
        tags.push("editorial:revise".to_string());
    }
    for attack in inputs.external_attacks {
        if attack.claim_index.is_none() && attack.strength >= 0.6 {
            revise = true;
            reasons.push(format!(
                "red-team finding: {}",
                truncate(&attack.rationale, 140)
            ));
            tags.push("editorial:red_team".to_string());
        }
    }
    if !verification_report.soft_flags.is_empty() && reasons.is_empty() {
        // Soft flags alone do not block, but they are surfaced for the record.
        tags.push("editorial:soft_flags".to_string());
    }

    // Evidence cap: confidence can never exceed what the evidence and the
    // argumentation support. The warrant term makes certainty injections
    // strictly non-raising: weakening the reasoning lowers the cap, so a
    // corrupted variant can never end up more confident than a clean one.
    let support_cap = (0.25
        + 0.30 * independence_score
        + 0.20 * verification_report.factuality_score
        + 0.25 * warrants.overall)
        .clamp(0.0, 0.95);
    let mut final_confidence = inputs.stated_confidence.clamp(0.0, 1.0);
    if let Some(curve) = inputs.calibration {
        final_confidence = curve.adjust(final_confidence);
    }
    // Uncertainty discipline gates stated confidence: absolute language
    // (proves/certainly/inevitable) with thin independence lowers the
    // released confidence. This makes certainty injections strictly
    // non-raising — a corrupted variant can never end up more confident
    // than the clean analysis it mutated.
    let discipline = depth_report
        .components
        .iter()
        .find(|component| component.name == "uncertainty_discipline")
        .map(|component| component.score)
        .unwrap_or(0.5);
    // Absolute-certainty wording is an explicit confidence penalty on top of
    // the discipline factor: certainty injections must strictly lower the
    // released confidence. (The clean dossier has zero certainty hits.)
    let certainty_penalty = (depth_report.certainty_hits as f64 * 0.04).min(0.20);
    final_confidence = (final_confidence.min(support_cap)
        * (0.80 + 0.20 * discipline)
        * (1.0 - certainty_penalty))
        .clamp(0.0, support_cap);

    let verdict = if reject {
        EditorialVerdict::Reject
    } else if revise {
        EditorialVerdict::Revise
    } else {
        EditorialVerdict::Publish
    };
    if verdict == EditorialVerdict::Publish {
        reasons.clear();
        tags.retain(|tag| tag != "editorial:revise");
    }

    EditorialReview {
        verdict,
        depth: depth_report,
        verification: verification_report,
        warrants,
        final_confidence,
        evidence_cap: support_cap,
        reasons,
        tags,
    }
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use apex_core::claims::{ClaimKind, InsightClaim};
    use uuid::Uuid;

    fn as_of() -> DateTime<Utc> {
        DateTime::from_timestamp(1_780_000_000, 0).unwrap()
    }

    fn evidence(n: usize) -> Vec<EvidenceRecord> {
        (0..n)
            .map(|i| EvidenceRecord {
                evidence_id: Uuid::from_u128(i as u128 + 1),
                title: format!("Filing {i}"),
                text: "Duty rates rose 12% in 2025, forcing buyers to re-quote.".into(),
                source_name: format!("publisher{i}.example"),
                source_url: Some(format!("https://publisher{i}.example/a")),
                signal_type: "government_registry".into(),
                observed_at: None,
                reliability: 0.85,
            })
            .collect()
    }

    fn claim(text: &str, kind: ClaimKind, ids: &[u128]) -> InsightClaim {
        InsightClaim::new(
            text.to_string(),
            ids.iter().map(|id| Uuid::from_u128(*id)).collect(),
            Some(0.8),
            kind,
        )
    }

    fn good_inputs<'a>(
        narrative: &'a str,
        recommendations: &'a str,
        claims: &'a [InsightClaim],
        records: &'a [EvidenceRecord],
    ) -> EditorialInputs<'a> {
        EditorialInputs {
            headline: "Tariff pressure reshapes North African electronics sourcing",
            narrative,
            recommendations,
            claims,
            evidence: records,
            stated_confidence: 0.8,
            alternative_hypotheses: 2,
            external_attacks: &[],
            calibration: None,
            as_of: as_of(),
        }
    }

    #[test]
    fn deep_grounded_analysis_publishes() {
        let narrative = "Tariff increases on HS 8534 drove a 12% landed-cost rise in 2025 because \
            ministry filings show duty rates rising in Q1 and Q3. This forced buyers to re-quote; \
            in turn, nearshore suppliers gained share. However, an alternative explanation is that \
            freight rates, not tariffs, drove the shift. The evidence suggests roughly even odds \
            that both factors compounded. In the US and EU, second-order effects would include \
            supplier consolidation which could reduce choice for customers.";
        let recommendations =
            "We recommend contacting the top three buyers within two weeks to lock pricing. \
            Monitor regulator gazette updates this month and re-forecast margins next quarter.";
        let claims = vec![
            claim(
                "Tariff increases drove a 12% landed-cost rise in 2025 [1][2].",
                ClaimKind::Observed,
                &[1, 2],
            ),
            claim(
                "Buyers likely shifted toward nearshore suppliers [1].",
                ClaimKind::Inference,
                &[1],
            ),
            claim(
                "An alternative explanation is freight rates [2].",
                ClaimKind::Inference,
                &[2],
            ),
        ];
        let records = evidence(3);
        let review = review(
            &good_inputs(narrative, recommendations, &claims, &records),
            &EditorialConfig::default(),
        );
        assert_eq!(
            review.verdict,
            EditorialVerdict::Publish,
            "reasons {:?}",
            review.reasons
        );
        assert!(review.final_confidence <= review.evidence_cap + f64::EPSILON);
        assert!(review.final_confidence > 0.0);
    }

    #[test]
    fn fabricated_number_rejects() {
        let claims = vec![claim(
            "Duty rates rose 87% in 2025 [1][2].",
            ClaimKind::Observed,
            &[1, 2],
        )];
        let records = evidence(2);
        let review = review(
            &good_inputs("Duty rates rose 87% in 2025 [1][2].", "", &claims, &records),
            &EditorialConfig::default(),
        );
        assert_eq!(review.verdict, EditorialVerdict::Reject);
        assert!(review
            .reasons
            .iter()
            .any(|reason| reason.contains("hard integrity")));
    }

    #[test]
    fn shallow_draft_revises() {
        let claims = vec![claim(
            "The market changed in ways that matter.",
            ClaimKind::Unknown,
            &[],
        )];
        let records = evidence(1);
        let review = review(
            &good_inputs(
                "The market changed. This is important.",
                "",
                &claims,
                &records,
            ),
            &EditorialConfig::default(),
        );
        assert!(
            matches!(
                review.verdict,
                EditorialVerdict::Reject | EditorialVerdict::Revise
            ),
            "unexpected {:?}",
            review.verdict
        );
        assert!(!review.reasons.is_empty());
    }

    #[test]
    fn red_team_finding_forces_revision() {
        let narrative = "Tariff increases drove a 12% landed-cost rise in 2025 because filings show duty \
            rates rising in Q1 and Q3, forcing buyers to re-quote; in turn nearshore suppliers gained \
            share. However, an alternative explanation is freight. Second-order effects would include \
            consolidation. The evidence suggests roughly even odds.";
        let recommendations = "We recommend contacting buyers within two weeks to lock pricing.";
        let claims = vec![
            claim(
                "Tariff increases drove a 12% landed-cost rise in 2025 [1][2].",
                ClaimKind::Observed,
                &[1, 2],
            ),
            claim(
                "Nearshore suppliers likely gained [1].",
                ClaimKind::Inference,
                &[1],
            ),
        ];
        let records = evidence(3);
        let mut inputs = good_inputs(narrative, recommendations, &claims, &records);
        let attacks = vec![ExternalAttack {
            claim_index: None,
            strength: 0.8,
            rationale: "coordinated placement cluster across 5 sources".into(),
        }];
        inputs.external_attacks = &attacks;
        let review = review(&inputs, &EditorialConfig::default());
        assert_ne!(review.verdict, EditorialVerdict::Publish);
        assert!(review.tags.iter().any(|tag| tag == "editorial:red_team"));
    }

    #[test]
    fn calibration_curve_caps_overconfidence() {
        let claims = vec![claim(
            "Tariff increases drove a 12% landed-cost rise in 2025 [1][2].",
            ClaimKind::Observed,
            &[1, 2],
        )];
        let records = evidence(2);
        let pairs: Vec<crate::analytical::calibration::CalibrationPair> = (0..40)
            .map(|index| crate::analytical::calibration::CalibrationPair {
                stated: 0.9,
                outcome: index % 2 == 0,
            })
            .collect();
        let curve = crate::analytical::calibration::CalibrationCurve::fit(&pairs, 10);
        let mut inputs = good_inputs(
            "Tariff increases drove a 12% landed-cost rise in 2025 [1][2]. In turn buyers re-quoted.",
            "We recommend contacting buyers within two weeks.",
            &claims,
            &records,
        );
        inputs.stated_confidence = 0.95;
        inputs.calibration = curve.as_ref();
        let review = review(&inputs, &EditorialConfig::default());
        assert!(
            review.final_confidence < 0.75,
            "final {}",
            review.final_confidence
        );
    }
}
