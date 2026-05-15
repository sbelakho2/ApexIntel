//! Competitor capability gap analyzer.
//!
//! Compares a target entity's capability and certification profile against
//! a set of competitor profiles to identify:
//! - Capabilities the target lacks (competitive vulnerabilities)
//! - Capabilities the target has that competitors don't (differentiation)
//! - Certification gaps that block market access
//! - LLM-generated strategic narrative explaining the gap picture
//!
//! # Design
//! - Pure computation: no I/O; all data is passed in
//! - Optional: LLM narration for turning raw gap lists into prose recommendations

use anyhow::{Context, Result};
use apex_core::entities::{Capability, Certification};
use apex_llm::LlmClient;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use tracing::warn;

// ─────────────────────────────────────────────────────────────────────────────
// Input / output types
// ─────────────────────────────────────────────────────────────────────────────

/// Summary of a capability / certification possessed by an entity.
/// Uses the capability slug string from `Capability::capability`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct CapabilityTag {
    /// Slug, e.g. "SMT", "die_casting" — maps to `Capability::capability`.
    pub code: String,
    /// Human-readable label (may equal `code` if no alias is available).
    pub label: String,
    /// Optional category / domain string (e.g., "manufacturing", "software").
    pub category: String,
}

impl From<&Capability> for CapabilityTag {
    fn from(c: &Capability) -> Self {
        Self {
            code: c.capability.clone(),
            label: c.capability.clone(), // capability slug doubles as label
            category: c.proof_grade.as_str().to_string(),
        }
    }
}

/// Profile of a single entity for gap analysis purposes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityCapabilityProfile {
    pub entity_id: String,
    pub entity_name: String,
    pub capabilities: HashSet<CapabilityTag>,
    /// Certification standard strings, e.g. "ISO9001", "AS9100".
    pub certifications: HashSet<String>,
}

impl EntityCapabilityProfile {
    /// Construct from pre-fetched `Capability` and `Certification` records.
    pub fn from_records(
        entity_id: impl Into<String>,
        entity_name: impl Into<String>,
        capabilities: &[Capability],
        certifications: &[Certification],
    ) -> Self {
        let cap_tags: HashSet<CapabilityTag> =
            capabilities.iter().map(CapabilityTag::from).collect();
        let cert_stds: HashSet<String> =
            certifications.iter().map(|c| c.standard.clone()).collect();
        Self {
            entity_id: entity_id.into(),
            entity_name: entity_name.into(),
            capabilities: cap_tags,
            certifications: cert_stds,
        }
    }

    /// Construct from raw string slices (useful in tests and synthetic data).
    pub fn from_strings(
        entity_id: impl Into<String>,
        entity_name: impl Into<String>,
        caps: impl IntoIterator<Item = impl Into<String>>,
        certs: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        let capabilities = caps
            .into_iter()
            .map(|s| {
                let code = s.into();
                CapabilityTag {
                    label: code.clone(),
                    category: String::new(),
                    code,
                }
            })
            .collect();
        Self {
            entity_id: entity_id.into(),
            entity_name: entity_name.into(),
            capabilities,
            certifications: certs.into_iter().map(|s| s.into()).collect(),
        }
    }
}

/// A single gap entry: something one entity has that another doesn't.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GapEntry {
    pub tag: CapabilityTag,
    /// Which competitor(s) possess this capability.
    pub possessed_by: Vec<String>,
    /// Estimated strategic weight [0, 1].
    pub weight: f64,
}

/// Result of a gap analysis for one target entity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GapAnalysisResult {
    /// The analysed entity.
    pub entity_id: String,
    pub entity_name: String,
    /// Capabilities this entity lacks but ≥1 competitor has.
    pub defensive_gaps: Vec<GapEntry>,
    /// Capabilities this entity has that no competitor has.
    pub differentiators: Vec<GapEntry>,
    /// Certifications this entity lacks but ≥1 competitor holds.
    pub certification_gaps: Vec<String>,
    /// Certifications this entity holds that no competitor has.
    pub unique_certifications: Vec<String>,
    /// Overall gap score: larger = more gaps relative to competition.
    pub gap_score: f64,
    /// LLM-generated narrative (optional).
    pub narrative: Option<String>,
    /// Number of competitors analysed.
    pub competitor_count: usize,
}

// ─────────────────────────────────────────────────────────────────────────────
// Analyzer
// ─────────────────────────────────────────────────────────────────────────────

/// Performs capability gap analysis between a target and its competitors.
pub struct GapAnalyzer {
    /// Optional LLM for narrative generation.
    llm: Option<std::sync::Arc<dyn LlmClient>>,
    /// Weight multiplier for unique capabilities not held by any competitor.
    differentiator_weight: f64,
    /// Weight multiplier for gaps held by all competitors.
    universal_gap_weight: f64,
}

impl GapAnalyzer {
    /// Create an analyzer without LLM narration.
    pub fn new() -> Self {
        Self {
            llm: None,
            differentiator_weight: 1.5,
            universal_gap_weight: 2.0,
        }
    }

    /// Create an analyzer with LLM narration enabled.
    pub fn with_llm(llm: std::sync::Arc<dyn LlmClient>) -> Self {
        Self {
            llm: Some(llm),
            ..Self::new()
        }
    }

    /// Analyse the gap between `target` and `competitors`.
    pub async fn analyse(
        &self,
        target: &EntityCapabilityProfile,
        competitors: &[EntityCapabilityProfile],
    ) -> GapAnalysisResult {
        // Collect all capability tags that competitors possess
        let mut competitor_caps: HashMap<CapabilityTag, Vec<String>> = HashMap::new();
        for comp in competitors {
            for cap in &comp.capabilities {
                competitor_caps
                    .entry(cap.clone())
                    .or_default()
                    .push(comp.entity_name.clone());
            }
        }

        // Collect all certification codes competitors hold
        let mut competitor_certs: HashMap<String, usize> = HashMap::new();
        for comp in competitors {
            for cert in &comp.certifications {
                *competitor_certs.entry(cert.clone()).or_insert(0) += 1;
            }
        }

        // ── Defensive gaps: competitor has it, target doesn't ───
        let defensive_gaps: Vec<GapEntry> = competitor_caps
            .iter()
            .filter(|(cap, _)| !target.capabilities.contains(*cap))
            .map(|(cap, possessors)| {
                let coverage_ratio = possessors.len() as f64 / competitors.len().max(1) as f64;
                let weight = if coverage_ratio >= 1.0 {
                    self.universal_gap_weight
                } else {
                    coverage_ratio * self.universal_gap_weight
                };
                GapEntry {
                    tag: cap.clone(),
                    possessed_by: possessors.clone(),
                    weight,
                }
            })
            .collect();

        // ── Differentiators: target has it, no competitor does ──
        let differentiators: Vec<GapEntry> = target
            .capabilities
            .iter()
            .filter(|cap| !competitor_caps.contains_key(*cap))
            .map(|cap| GapEntry {
                tag: cap.clone(),
                possessed_by: vec![],
                weight: self.differentiator_weight,
            })
            .collect();

        // ── Certification gaps ────────────────────────────────────
        let certification_gaps: Vec<String> = competitor_certs
            .keys()
            .filter(|cert| !target.certifications.contains(*cert))
            .cloned()
            .collect();

        let unique_certifications: Vec<String> = target
            .certifications
            .iter()
            .filter(|cert| !competitor_certs.contains_key(*cert))
            .cloned()
            .collect();

        // ── Gap score ─────────────────────────────────────────────
        let gap_score = if competitors.is_empty() {
            0.0
        } else {
            let total_competitor_caps: f64 = competitor_caps.len() as f64;
            let target_missing: f64 = defensive_gaps.len() as f64;
            (target_missing / total_competitor_caps.max(1.0)).min(1.0)
        };

        // ── LLM narrative ──────────────────────────────────────────
        let narrative = if let Some(ref llm) = self.llm {
            match self
                .generate_narrative(llm.as_ref(), target, &defensive_gaps, &differentiators)
                .await
            {
                Ok(narr) => Some(narr),
                Err(e) => {
                    warn!(entity=%target.entity_name, error=%e, "Gap narrative generation failed");
                    None
                }
            }
        } else {
            None
        };

        GapAnalysisResult {
            entity_id: target.entity_id.clone(),
            entity_name: target.entity_name.clone(),
            defensive_gaps,
            differentiators,
            certification_gaps,
            unique_certifications,
            gap_score,
            narrative,
            competitor_count: competitors.len(),
        }
    }

    async fn generate_narrative(
        &self,
        llm: &dyn LlmClient,
        target: &EntityCapabilityProfile,
        gaps: &[GapEntry],
        differentiators: &[GapEntry],
    ) -> Result<String> {
        let gap_list = gaps
            .iter()
            .take(10)
            .map(|g| format!("  - {} ({})", g.tag.label, g.tag.category))
            .collect::<Vec<_>>()
            .join("\n");

        let diff_list = differentiators
            .iter()
            .take(5)
            .map(|g| format!("  + {}", g.tag.label))
            .collect::<Vec<_>>()
            .join("\n");

        let system = "You are an OSINT analyst writing a competitive capability gap brief. \
            Be concise, specific, and actionable. Max 4 sentences.";

        let user = format!(
            "Entity: {}\n\
            Capability gaps vs competitors:\n{}\n\
            Differentiators (unique to this entity):\n{}\n\
            Write a gap analysis paragraph with strategic implications.",
            target.entity_name,
            if gap_list.is_empty() {
                "  (none identified)".to_string()
            } else {
                gap_list
            },
            if diff_list.is_empty() {
                "  (none identified)".to_string()
            } else {
                diff_list
            },
        );

        llm.generate_text(system, &user)
            .await
            .context("Gap narrative LLM call failed")
    }
}

impl Default for GapAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Batch helper
// ─────────────────────────────────────────────────────────────────────────────

/// Analyse gaps for multiple targets against a shared competitor pool.
pub async fn batch_analyse(
    analyzer: &GapAnalyzer,
    targets: &[EntityCapabilityProfile],
    competitors: &[EntityCapabilityProfile],
) -> Vec<GapAnalysisResult> {
    let mut results = Vec::with_capacity(targets.len());
    for target in targets {
        // Exclude the target itself from its own competitor pool
        let comps: Vec<_> = competitors
            .iter()
            .filter(|c| c.entity_id != target.entity_id)
            .cloned()
            .collect();
        results.push(analyzer.analyse(target, &comps).await);
    }
    results
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    #![allow(
        clippy::disallowed_methods,
        clippy::field_reassign_with_default,
        clippy::manual_range_contains,
        clippy::needless_borrows_for_generic_args,
        clippy::cloned_ref_to_slice_refs
    )]

    use super::*;

    fn make_profile(id: &str, name: &str, caps: &[(&str, &str, &str)]) -> EntityCapabilityProfile {
        EntityCapabilityProfile {
            entity_id: id.to_string(),
            entity_name: name.to_string(),
            capabilities: caps
                .iter()
                .map(|(code, label, cat)| CapabilityTag {
                    code: code.to_string(),
                    label: label.to_string(),
                    category: cat.to_string(),
                })
                .collect(),
            certifications: HashSet::new(),
        }
    }

    #[tokio::test]
    async fn gap_identified_correctly() {
        let target = make_profile("t1", "Target Corp", &[("CAP_A", "Capability A", "Defence")]);
        let competitor = make_profile(
            "c1",
            "Competitor Inc",
            &[
                ("CAP_A", "Capability A", "Defence"),
                ("CAP_B", "Capability B", "Electronics"),
            ],
        );

        let analyzer = GapAnalyzer::new();
        let result = analyzer.analyse(&target, &[competitor]).await;

        assert_eq!(result.defensive_gaps.len(), 1);
        assert_eq!(result.defensive_gaps[0].tag.code, "CAP_B");
        assert!(result.differentiators.is_empty());
    }

    #[tokio::test]
    async fn differentiator_identified() {
        let target = make_profile(
            "t1",
            "Target",
            &[("UNIQUE_CAP", "Unique Capability", "Tech")],
        );
        let competitor = make_profile(
            "c1",
            "Competitor",
            &[("DIFFERENT_CAP", "Different", "Tech")],
        );

        let analyzer = GapAnalyzer::new();
        let result = analyzer.analyse(&target, &[competitor]).await;

        assert_eq!(result.differentiators.len(), 1);
        assert_eq!(result.differentiators[0].tag.code, "UNIQUE_CAP");
    }

    #[tokio::test]
    async fn no_competitors_returns_zero_gap_score() {
        let target = make_profile("t1", "Target", &[("CAP_A", "Cap A", "Defence")]);
        let analyzer = GapAnalyzer::new();
        let result = analyzer.analyse(&target, &[]).await;
        assert!((result.gap_score).abs() < f64::EPSILON);
        assert_eq!(result.competitor_count, 0);
    }

    #[tokio::test]
    async fn gap_score_between_zero_and_one() {
        let target = make_profile("t1", "Target", &[]);
        let competitors: Vec<_> = (0..5)
            .map(|i| {
                make_profile(
                    &format!("c{}", i),
                    &format!("Comp{}", i),
                    &[(&format!("CAP_{}", i), &format!("Cap {}", i), "Defence")],
                )
            })
            .collect();

        let analyzer = GapAnalyzer::new();
        let result = analyzer.analyse(&target, &competitors).await;
        assert!(result.gap_score >= 0.0 && result.gap_score <= 1.0);
    }
}
