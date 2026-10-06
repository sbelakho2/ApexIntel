//! Argumentation-based warrants (QBAF-style contestable analysis).
//!
//! Every claim carries support arguments (independent, reliability-weighted
//! evidence families) and attack arguments (verification failures, thin
//! causality, coordinated-placement flags, external red-team findings). The
//! saturating resolution yields a per-claim warrant, and the insight's overall
//! warrant is weakest-link constrained: a chain of reasoning is only as strong
//! as its weakest asserted step, so one unsupported central claim cannot be
//! averaged away by volume elsewhere.

use super::{source_families, EvidenceRecord};
use crate::analytical::verification::{ClaimVerdict, HardViolation, SoftFlag, VerificationReport};
use apex_core::claims::{ClaimKind, InsightClaim};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// An external attack from another subsystem (adversarial quarantine,
/// coordinated-placement detection, counter-evidence, red team).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExternalAttack {
    /// Claim index the attack targets; `None` attacks the insight as a whole.
    pub claim_index: Option<usize>,
    /// Attack strength in `0..=1`.
    pub strength: f64,
    pub rationale: String,
}

/// Per-claim warrant result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClaimWarrant {
    pub claim: String,
    pub kind: ClaimKind,
    /// Saturated support from independent evidence families (`0..=1`).
    pub support: f64,
    /// Saturated attack points (`0..=1`).
    pub attack: f64,
    /// Resolved warrant (`0..=1`).
    pub warrant: f64,
    /// Distinct evidence families backing this claim.
    pub family_count: usize,
    /// Human-readable reasons that lowered the warrant.
    pub attack_reasons: Vec<String>,
}

/// Aggregate warrant result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WarrantReport {
    pub per_claim: Vec<ClaimWarrant>,
    /// Weakest-link-constrained overall warrant (`0..=1`).
    pub overall: f64,
    /// The claim with the lowest warrant among observed/inference claims.
    pub weakest_claim: Option<String>,
}

fn saturate(value: f64, half: f64) -> f64 {
    if value <= 0.0 {
        0.0
    } else {
        value / (value + half)
    }
}

/// Build warrant arguments for every claim.
pub fn build_warrants(
    claims: &[InsightClaim],
    evidence: &[EvidenceRecord],
    verification: &VerificationReport,
    external_attacks: &[ExternalAttack],
) -> WarrantReport {
    let families = source_families(evidence);
    let mut per_claim = Vec::with_capacity(claims.len());
    let mut insight_attacks = 0.0_f64;

    for (index, claim) in claims.iter().enumerate() {
        // Support: independent families that back the cited evidence for this
        // claim. A family counts once (duplicate copies are not corroboration).
        let cited: HashSet<uuid::Uuid> = claim.evidence_ids.iter().copied().collect();
        let mut family_strength = 0.0;
        let mut family_count = 0usize;
        let mut seen_families: HashSet<String> = HashSet::new();
        for record in evidence
            .iter()
            .filter(|record| cited.contains(&record.evidence_id))
        {
            let family = record.source_family();
            if seen_families.insert(family) {
                family_count += 1;
                family_strength += record.reliability.clamp(0.0, 1.0);
            }
        }
        if family_count == 0 && !families.is_empty() && !cited.is_empty() {
            // Cited ids that are not in the supplied evidence: treat as one
            // weak unattributed support — never as corroboration.
            family_count = 1;
            family_strength = 0.2;
        }
        let mut support = saturate(family_strength, 1.0);

        let verdict = verification
            .claims
            .get(index)
            .map(|entry| entry.verdict)
            .unwrap_or(ClaimVerdict::Unverifiable);
        support *= match verdict {
            ClaimVerdict::Supported => 1.0,
            ClaimVerdict::PartiallySupported => 0.7,
            ClaimVerdict::Unverifiable => 0.4,
            ClaimVerdict::Unsupported => 0.1,
        };
        if claim.kind == ClaimKind::Recommendation {
            // Recommendations are actions, not assertions: warrant measures
            // their grounding in cited evidence, at a discount.
            support *= 0.5;
        }

        // Attack: hard violations that name this claim, soft flags, external
        // attacks targeted at this claim.
        let mut attack_points = 0.0_f64;
        let mut attack_reasons: Vec<String> = Vec::new();
        for violation in &verification.hard_violations {
            let names_claim = match violation {
                HardViolation::FabricatedNumber { claim: flagged, .. }
                | HardViolation::UnverifiedDate { claim: flagged, .. }
                | HardViolation::CitationOutOfRange { claim: flagged, .. } => {
                    flagged == &claim.claim
                }
            };
            if names_claim {
                attack_points += 1.0;
                attack_reasons.push(match violation {
                    HardViolation::FabricatedNumber { value, .. } => {
                        format!("fabricated number {value}")
                    }
                    HardViolation::UnverifiedDate { date, .. } => format!("unverified date {date}"),
                    HardViolation::CitationOutOfRange { ordinal, .. } => {
                        format!("citation [{ordinal}] out of range")
                    }
                });
            }
        }
        for flag in &verification.soft_flags {
            let (names_claim, weight, reason) = match flag {
                SoftFlag::ThinCausality { claim: flagged, .. } => {
                    (flagged == &claim.claim, 0.3, "thin causality")
                }
                SoftFlag::UnhedgedProjection { claim: flagged } => {
                    (flagged == &claim.claim, 0.25, "unhedged projection")
                }
                SoftFlag::UnverifiedEntity { claim: flagged, .. } => {
                    (flagged == &claim.claim, 0.2, "unverified entity")
                }
                SoftFlag::InferredNumber { claim: flagged, .. } => {
                    (flagged == &claim.claim, 0.1, "inferred number")
                }
                SoftFlag::NoClaims => (false, 0.0, ""),
            };
            if names_claim {
                attack_points += weight;
                if !reason.is_empty() && !attack_reasons.iter().any(|existing| existing == reason) {
                    attack_reasons.push(reason.to_string());
                }
            }
        }
        for external in external_attacks {
            match external.claim_index {
                Some(target) if target == index => {
                    attack_points += external.strength.clamp(0.0, 1.0);
                    attack_reasons.push(external.rationale.clone());
                }
                None => insight_attacks += external.strength.clamp(0.0, 1.0),
                _ => {}
            }
        }

        let attack = saturate(attack_points, 0.8);
        let warrant = ((support + 0.05) / (support + attack + 0.1)).clamp(0.0, 1.0);
        per_claim.push(ClaimWarrant {
            claim: claim.claim.clone(),
            kind: claim.kind,
            support,
            attack,
            warrant,
            family_count,
            attack_reasons,
        });
    }

    let insight_attack = saturate(insight_attacks, 0.8);

    // Weakest-link: observed claims' minimum bounds the whole insight; only
    // if no observed claim exists do inferences define the floor.
    let observed: Vec<&ClaimWarrant> = per_claim
        .iter()
        .filter(|entry| entry.kind == ClaimKind::Observed)
        .collect();
    let floor_set: Vec<&ClaimWarrant> = if observed.is_empty() {
        per_claim
            .iter()
            .filter(|entry| entry.kind == ClaimKind::Inference)
            .collect()
    } else {
        observed
    };
    let weakest = floor_set.iter().min_by(|a, b| {
        a.warrant
            .partial_cmp(&b.warrant)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let weakest_warrant = weakest.map(|entry| entry.warrant).unwrap_or(0.0);
    let weakest_claim = weakest.map(|entry| entry.claim.clone());

    let weighted_mean = if per_claim.is_empty() {
        0.0
    } else {
        let total_weight: f64 = per_claim
            .iter()
            .map(|entry| match entry.kind {
                ClaimKind::Observed => 1.0,
                ClaimKind::Inference => 0.7,
                ClaimKind::Recommendation => 0.3,
                ClaimKind::Unknown => 0.5,
            })
            .sum();
        per_claim
            .iter()
            .map(|entry| {
                entry.warrant
                    * match entry.kind {
                        ClaimKind::Observed => 1.0,
                        ClaimKind::Inference => 0.7,
                        ClaimKind::Recommendation => 0.3,
                        ClaimKind::Unknown => 0.5,
                    }
            })
            .sum::<f64>()
            / total_weight.max(f64::EPSILON)
    };

    // 0.55 mean + 0.45 weakest link, then insight-level attacks discount.
    let mut overall = 0.55 * weighted_mean + 0.45 * weakest_warrant;
    if per_claim.is_empty() {
        overall *= 0.5;
    }
    overall *= 1.0 - 0.7 * insight_attack;

    WarrantReport {
        per_claim,
        overall: overall.clamp(0.0, 1.0),
        weakest_claim,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::analytical::verification::{verify_claims, VerificationConfig};
    use crate::analytical::EvidenceRecord;
    use chrono::{DateTime, Utc};
    use uuid::Uuid;

    fn as_of() -> DateTime<Utc> {
        DateTime::from_timestamp(1_780_000_000, 0).unwrap()
    }

    fn evidence(n: usize) -> Vec<EvidenceRecord> {
        (0..n)
            .map(|i| EvidenceRecord {
                evidence_id: Uuid::from_u128(i as u128 + 1),
                title: format!("Report {i}"),
                text: "Duty rates rose 12% in 2025.".into(),
                source_name: format!("publisher{i}.example"),
                source_url: Some(format!("https://publisher{i}.example/a")),
                signal_type: "news".into(),
                observed_at: None,
                reliability: 0.8,
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

    #[test]
    fn duplicate_sources_do_not_add_support() {
        let mut records = evidence(3);
        // All three from the same family despite distinct source names.
        for record in &mut records {
            record.source_url = Some("https://reuters.com/a".into());
        }
        let claims = vec![claim(
            "Duty rates rose 12% in 2025 [1][2][3].",
            ClaimKind::Observed,
            &[1, 2, 3],
        )];
        let verification =
            verify_claims(&claims, &records, &VerificationConfig::default(), as_of());
        let report = build_warrants(&claims, &records, &verification, &[]);
        assert_eq!(report.per_claim[0].family_count, 1);
        assert!(
            report.per_claim[0].support < 0.5,
            "support {}",
            report.per_claim[0].support
        );
    }

    #[test]
    fn weakest_link_bounds_overall() {
        let records = evidence(4);
        let claims = vec![
            claim(
                "Duty rates rose 12% in 2025 [1][2].",
                ClaimKind::Observed,
                &[1, 2],
            ),
            claim(
                "Zenith Dynamics Ltd will certainly capture 87% of the market [3].",
                ClaimKind::Observed,
                &[3],
            ),
        ];
        let verification =
            verify_claims(&claims, &records, &VerificationConfig::default(), as_of());
        assert!(!verification.hard_violations.is_empty());
        let report = build_warrants(&claims, &records, &verification, &[]);
        assert!(report.overall < 0.45, "overall {}", report.overall);
        assert!(report.per_claim[1].warrant < report.per_claim[0].warrant);
    }

    #[test]
    fn external_insight_attack_discounts_overall() {
        let records = evidence(2);
        let claims = vec![claim(
            "Duty rates rose 12% in 2025 [1][2].",
            ClaimKind::Observed,
            &[1, 2],
        )];
        let verification =
            verify_claims(&claims, &records, &VerificationConfig::default(), as_of());
        let clean = build_warrants(&claims, &records, &verification, &[]);
        let attacked = build_warrants(
            &claims,
            &records,
            &verification,
            &[ExternalAttack {
                claim_index: None,
                strength: 0.9,
                rationale: "coordinated placement cluster".into(),
            }],
        );
        assert!(attacked.overall < clean.overall);
    }

    #[test]
    fn unsupported_claims_have_near_zero_warrant() {
        let records = evidence(1);
        let claims = vec![claim(
            "Something happened somewhere important.",
            ClaimKind::Unknown,
            &[],
        )];
        let verification =
            verify_claims(&claims, &records, &VerificationConfig::default(), as_of());
        let report = build_warrants(&claims, &records, &verification, &[]);
        assert!(report.per_claim[0].warrant < 0.25);
    }
}
