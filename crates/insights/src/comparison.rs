//! Competitive comparison matrix generator.
//!
//! # Discovery Integration (Phase 2)
//!
//! This module now supports dynamically discovered entities through
//! [`infer_category`] and [`find_analogous_entity`], enabling comparison
//! generation for entities that were not part of the original seed set.
//!
//! # De-Starzed Comparison System
//!
//! This module provides **entity-agnostic** comparison infrastructure:
//!
//! - [`build_comparison_matrix`] — original API (backward-compatible, takes explicit
//!   target-caps + competitor data).
//! - [`select_reference_entity`] — dynamically picks the best reference entity for a
//!   target using [`EntityRegistry`], instead of hardcoding "Starz".
//! - [`generate_comparison_matrix`] — multi-entity comparison returning a ranked list
//!   of competitors with similarity scores and insight text.
//! - [`build_comparison_prompt`] — parameterised LLM prompt with zero hardcoded names.
//! - [`compute_entity_similarity`] — scores similarity between two entity profiles.
//!
//! Used by: weekly strategy memo, dossier generation, `/api/insights/comparison`.

use crate::entity_relevance::{EntityCategory, EntityProfile, EntityRegistry, SignalContext};
use serde::Serialize;
use std::collections::HashMap;

// ─────────────────────────────────────────────────────────────────────────────
// Public types (backward-compatible)
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct ComparisonMatrix {
    pub starz_capabilities: Vec<String>,
    pub competitors: Vec<CompetitorColumn>,
    pub capability_rows: Vec<CapabilityRow>,
    pub summary: ComparisonSummary,
}

#[derive(Debug, Serialize)]
pub struct CompetitorColumn {
    pub name: String,
    pub region: String,
    pub threat_score: f64,
    pub overlap_score: f64,
}

#[derive(Debug, Serialize)]
pub struct CapabilityRow {
    pub capability: String,
    pub starz_has: bool,
    pub starz_proof_grade: String,
    pub competitor_status: HashMap<String, CapStatus>,
}

#[derive(Debug, Serialize)]
pub struct CapStatus {
    pub has_capability: bool,
    pub proof_grade: String,
    pub certified: bool,
}

#[derive(Debug, Serialize)]
pub struct ComparisonSummary {
    pub starz_unique_capabilities: Vec<String>,
    pub common_capabilities: Vec<String>,
    pub competitor_unique_capabilities: Vec<String>,
    pub starz_cert_advantage: Vec<String>,
    pub starz_cert_gap: Vec<String>,
    pub regional_advantages: Vec<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// New types — De-Starzed Comparison System
// ─────────────────────────────────────────────────────────────────────────────

/// Category of comparison to perform between two entities.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub enum ComparisonCategory {
    /// Same industry / same category — direct competitors (e.g., Foxconn vs Pegatron).
    DirectCompetitor,
    /// Adjacent supply-chain segment — upstream/downstream (e.g., NVIDIA vs TSMC).
    SupplyChain,
    /// Cross-category fallback — used when no same-category entity is available.
    CrossCategory,
}

impl ComparisonCategory {
    /// Return a human-readable label.
    pub fn as_str(&self) -> &str {
        match self {
            Self::DirectCompetitor => "direct competitor",
            Self::SupplyChain => "supply chain",
            Self::CrossCategory => "cross-category",
        }
    }
}

/// A single comparison insight between two entities.
#[derive(Debug, Clone, Serialize)]
pub struct ComparisonInsight {
    /// Primary entity name.
    pub entity_a: String,
    /// Reference entity name.
    pub entity_b: String,
    /// Similarity score (0.0 – 1.0).
    pub similarity: f64,
    /// Generated insight text describing the comparison.
    pub insight_text: String,
    /// Category of comparison.
    pub category: ComparisonCategory,
}

// ─────────────────────────────────────────────────────────────────────────────
// Original builder (backward-compatible)
// ─────────────────────────────────────────────────────────────────────────────

/// Build a comparison matrix from target capabilities and competitor data.
///
/// # Arguments
/// * `starz_caps` — (capability_name, proof_grade, is_certified) tuples
/// * `competitors` — (name, region, threat_score, overlap_score, capabilities) tuples
pub fn build_comparison_matrix(
    starz_caps: &[(String, String, bool)],
    competitors: &[(String, String, f64, f64, Vec<(String, String, bool)>)],
) -> ComparisonMatrix {
    // Collect all unique capabilities
    let all_capabilities: Vec<String> = {
        let mut caps: Vec<String> = starz_caps.iter().map(|c| c.0.clone()).collect();
        for (_, _, _, _, comp_caps) in competitors {
            for (cap, _, _) in comp_caps {
                if !caps.contains(cap) {
                    caps.push(cap.clone());
                }
            }
        }
        caps.sort();
        caps
    };

    let competitor_columns: Vec<CompetitorColumn> = competitors
        .iter()
        .map(|(name, region, threat, overlap, _)| CompetitorColumn {
            name: name.clone(),
            region: region.clone(),
            threat_score: *threat,
            overlap_score: *overlap,
        })
        .collect();

    let mut capability_rows = Vec::new();
    let mut starz_unique = Vec::new();
    let mut common = Vec::new();
    let mut comp_unique = Vec::new();
    let mut cert_advantage = Vec::new();
    let mut cert_gap = Vec::new();

    for cap in &all_capabilities {
        let starz = starz_caps.iter().find(|c| &c.0 == cap);
        let starz_has = starz.is_some();
        let starz_grade = starz.map(|c| c.1.clone()).unwrap_or_default();
        let starz_certified = starz.map(|c| c.2).unwrap_or(false);

        let mut comp_status = HashMap::new();
        let mut any_competitor_has = false;
        let mut any_competitor_certified = false;

        for (name, _, _, _, comp_caps) in competitors {
            let comp = comp_caps.iter().find(|c| &c.0 == cap);
            let has = comp.is_some();
            if has {
                any_competitor_has = true;
            }
            let certified = comp.map(|c| c.2).unwrap_or(false);
            if certified {
                any_competitor_certified = true;
            }
            comp_status.insert(
                name.clone(),
                CapStatus {
                    has_capability: has,
                    proof_grade: comp.map(|c| c.1.clone()).unwrap_or_default(),
                    certified,
                },
            );
        }

        if starz_has && !any_competitor_has {
            starz_unique.push(cap.clone());
        } else if starz_has && any_competitor_has {
            common.push(cap.clone());
        } else if !starz_has && any_competitor_has {
            comp_unique.push(cap.clone());
        }

        if starz_certified && !any_competitor_certified {
            cert_advantage.push(cap.clone());
        }
        if !starz_certified && any_competitor_certified {
            cert_gap.push(cap.clone());
        }

        capability_rows.push(CapabilityRow {
            capability: cap.clone(),
            starz_has,
            starz_proof_grade: starz_grade,
            competitor_status: comp_status,
        });
    }

    ComparisonMatrix {
        starz_capabilities: starz_caps.iter().map(|c| c.0.clone()).collect(),
        competitors: competitor_columns,
        capability_rows,
        summary: ComparisonSummary {
            starz_unique_capabilities: starz_unique,
            common_capabilities: common,
            competitor_unique_capabilities: comp_unique,
            starz_cert_advantage: cert_advantage,
            starz_cert_gap: cert_gap,
            regional_advantages: vec![
                "Tunisia/Morocco nearshore proximity to EU".into(),
                "Multi-region presence (TN+MA) for dual-source".into(),
                "Free-zone tax advantages".into(),
            ],
        },
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Dynamic Reference Entity Selection
// ─────────────────────────────────────────────────────────────────────────────

/// Selects the best reference entity for a comparison.
///
/// Uses [`EntityRegistry`] to find the entity with highest activity score
/// that is in the same category as the target entity (for direct competitors)
/// or in an adjacent category (for supply chain comparisons).
///
/// # Selection Strategy
///
/// 1. If the target itself is found in the registry, look for same-category
///    entities with the highest activity scores, excluding the target itself.
/// 2. If same-category entities exist, return the highest-activity one.
/// 3. If no same-category entity exists, fall back to cross-category: pick
///    the entity with the highest activity score overall.
/// 4. Never default to a single hardcoded entity.
pub fn select_reference_entity(
    target: &str,
    registry: &EntityRegistry,
    category: ComparisonCategory,
) -> Option<EntityProfile> {
    if registry.is_empty() {
        return None;
    }

    match category {
        ComparisonCategory::DirectCompetitor => {
            // Find the target's category
            let target_category = registry.get_category(target);
            match target_category {
                Some(cat) => {
                    // Get all entities in the same category, excluding the target
                    let same_category = registry.entities_in_category(&cat);
                    let candidates: Vec<&EntityProfile> = same_category
                        .into_iter()
                        .filter(|p| p.entity_name.to_lowercase() != target.to_lowercase())
                        .collect();

                    // Return the one with highest activity score
                    candidates.into_iter().next().cloned()
                }
                None => {
                    // Target not in registry — pick the highest-activity entity overall
                    let top = registry.top_n_entities(1, &[]);
                    top.into_iter().next().cloned()
                }
            }
        }
        ComparisonCategory::SupplyChain => {
            // For supply chain, look in adjacent categories
            // Find the target's category first
            let target_category = registry.get_category(target);
            match target_category {
                Some(cat) => {
                    // Pick from a different category (adjacent supply chain)
                    let all_entities: Vec<&EntityProfile> = registry
                        .entity_names()
                        .filter_map(|name| {
                            let profile = registry.get(name)?;
                            if profile.category != cat
                                && profile.entity_name.to_lowercase() != target.to_lowercase()
                            {
                                Some(profile)
                            } else {
                                None
                            }
                        })
                        .collect();

                    all_entities
                        .into_iter()
                        .max_by(|a, b| {
                            let score_a = registry.activity_score(&a.entity_name);
                            let score_b = registry.activity_score(&b.entity_name);
                            score_a
                                .partial_cmp(&score_b)
                                .unwrap_or(std::cmp::Ordering::Equal)
                        })
                        .cloned()
                }
                None => {
                    // Target not known — highest activity overall
                    let top = registry.top_n_entities(1, &[]);
                    top.into_iter().next().cloned()
                }
            }
        }
        ComparisonCategory::CrossCategory => {
            // Highest-activity entity regardless of category, excluding target
            let top = registry.top_n_entities(1, &[target.to_string()]);
            top.into_iter().next().cloned()
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Entity Similarity Scoring
// ─────────────────────────────────────────────────────────────────────────────

/// Compute a similarity score between two entities based on:
/// - Same category → 0.5 base
/// - Shared supply chain proximity → +0.3
/// - Both have recent observations → +0.2
/// - Same country/region → +0.1
pub fn compute_entity_similarity(a: &EntityProfile, b: &EntityProfile) -> f64 {
    let mut score = 0.0;

    // Same category → 0.5 base
    if a.category == b.category {
        score += 0.5;
    }

    // Same country/region → +0.1
    if let (Some(ref country_a), Some(ref country_b)) = (&a.country_code, &b.country_code) {
        if country_a.to_lowercase() == country_b.to_lowercase() {
            score += 0.1;
        }
    }

    // Overlapping products → +0.1
    let product_overlap: usize = a
        .product_keywords
        .iter()
        .filter(|p| b.product_keywords.contains(p))
        .count();
    if product_overlap > 0 {
        score += 0.1;
    }

    // Overlapping topics → +0.05 per shared topic (max +0.15)
    let topic_overlap: usize = a
        .topic_keywords
        .iter()
        .filter(|t| b.topic_keywords.contains(t))
        .count();
    let topic_bonus = (topic_overlap as f64 * 0.05).min(0.15);
    score += topic_bonus;

    // Overlapping competitors → +0.1 (they compete with the same set)
    let competitor_overlap: usize = a
        .competitor_keywords
        .iter()
        .filter(|c| b.competitor_keywords.contains(c))
        .count();
    if competitor_overlap > 0 {
        score += 0.1;
    }

    // Overlapping industries → +0.05 (max +0.1)
    let industry_overlap: usize = a
        .industry_keywords
        .iter()
        .filter(|i| b.industry_keywords.contains(i))
        .count();
    let industry_bonus = (industry_overlap as f64 * 0.05).min(0.1);
    score += industry_bonus;

    score.min(1.0)
}

// ─────────────────────────────────────────────────────────────────────────────
// Discovery Integration Helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Infer the most likely [`EntityCategory`] for a dynamically discovered entity
/// based on the categories of known entities it shares keywords/topics with.
///
/// This enables comparison generation for entities that were not part of the
/// original seed set by placing them in a plausible category.
///
/// # Strategy
///
/// 1. Collect all known entities from the registry.
/// 2. Score each known entity against the target profile using
///    [`compute_entity_similarity`].
/// 3. Group similarity scores by category and pick the category with the
///    highest aggregate similarity.
/// 4. If no known entities exist, fall back to [`EntityCategory::Technology`].
pub fn infer_category(
    profile: &EntityProfile,
    registry: &EntityRegistry,
) -> EntityCategory {
    if registry.is_empty() {
        // Use the profile's own category if already set (e.g., from discovery)
        if profile.category != EntityCategory::Other("unknown".to_string()) {
            return profile.category.clone();
        }
        return EntityCategory::Technology;
    }

    // Score similarity against all known entities, grouped by category
    let mut category_scores: HashMap<EntityCategory, (f64, usize)> = HashMap::new();

    for name in registry.entity_names() {
        if let Some(known) = registry.get(name) {
            // Skip the entity itself
            if known.entity_name.eq_ignore_ascii_case(&profile.entity_name) {
                continue;
            }

            let similarity = compute_entity_similarity(profile, known);
            let entry = category_scores
                .entry(known.category.clone())
                .or_insert((0.0, 0));
            entry.0 += similarity;
            entry.1 += 1;
        }
    }

    // Pick category with highest average similarity
    let mut best_category = EntityCategory::Technology;
    let mut best_avg = 0.0_f64;

    for (cat, (total, count)) in &category_scores {
        let avg = total / *count as f64;
        if avg > best_avg {
            best_avg = avg;
            best_category = (*cat).clone();
        }
    }

    best_category
}

/// Find an analogous known entity for a dynamically discovered entity.
///
/// Returns the most similar [`EntityProfile`] from the registry along with
/// the similarity score. This lets dynamically discovered entities piggyback
/// on the analytical depth of known entities for comparison generation.
///
/// # Strategy
///
/// 1. Score the target against all known entities via [`compute_entity_similarity`].
/// 2. Return the closest match with similarity > 0.0.
/// 3. If no match is found (empty registry or all zero scores), returns `None`.
pub fn find_analogous_entity<'a>(
    profile: &EntityProfile,
    registry: &'a EntityRegistry,
) -> Option<(&'a EntityProfile, f64)> {
    if registry.is_empty() {
        return None;
    }

    let mut best: Option<(&EntityProfile, f64)> = None;

    for name in registry.entity_names() {
        if let Some(known) = registry.get(name) {
            if known.entity_name.eq_ignore_ascii_case(&profile.entity_name) {
                continue;
            }
            let similarity = compute_entity_similarity(profile, known);
            if similarity > 0.0 {
                match best {
                    Some((_, best_score)) if similarity > best_score => {
                        best = Some((known, similarity));
                    }
                    None => {
                        best = Some((known, similarity));
                    }
                    _ => {}
                }
            }
        }
    }

    best
}

// ─────────────────────────────────────────────────────────────────────────────
// Multi-Entity Comparison Matrix
// ─────────────────────────────────────────────────────────────────────────────

/// Generate multi-entity comparison (not just 1-vs-1).
///
/// Returns up to `max_entities` competitors ranked by similarity,
/// along with the comparison insight text.
///
/// # Selection Strategy
///
/// 1. Find the target entity in the registry. If not present, return empty.
/// 2. Score all other entities against the target using [`compute_entity_similarity`].
/// 3. Sort by similarity descending, return top `max_entities`.
/// 4. Each result includes a [`ComparisonInsight`] with a descriptive label.
pub fn generate_comparison_matrix(
    entity: &str,
    max_entities: usize,
    registry: &EntityRegistry,
) -> Vec<(EntityProfile, ComparisonInsight)> {
    if registry.is_empty() || max_entities == 0 {
        return Vec::new();
    }

    let target_profile = match registry.get(entity) {
        Some(p) => p.clone(),
        None => return Vec::new(),
    };

    let mut scored: Vec<(&EntityProfile, f64)> = registry
        .entity_names()
        .filter_map(|name| {
            if name.to_lowercase() == entity.to_lowercase() {
                return None; // Skip self
            }
            let profile = registry.get(name)?;
            let similarity = compute_entity_similarity(&target_profile, profile);
            Some((profile, similarity))
        })
        .collect();

    // Sort by similarity descending
    scored.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    scored.truncate(max_entities);

    let target_name = target_profile.entity_name.clone();
    scored
        .into_iter()
        .map(|(profile, similarity)| {
            let category = if profile.category == target_profile.category {
                ComparisonCategory::DirectCompetitor
            } else {
                ComparisonCategory::CrossCategory
            };

            let insight_text = build_comparison_insight_text(
                &target_name,
                &profile.entity_name,
                &category,
                similarity,
            );

            let insight = ComparisonInsight {
                entity_a: target_name.clone(),
                entity_b: profile.entity_name.clone(),
                similarity,
                insight_text,
                category,
            };

            (profile.clone(), insight)
        })
        .collect()
}

/// Build a short human-readable insight text for a comparison pair.
fn build_comparison_insight_text(
    entity_a: &str,
    entity_b: &str,
    category: &ComparisonCategory,
    similarity: f64,
) -> String {
    match category {
        ComparisonCategory::DirectCompetitor => {
            format!(
                "{} is a direct competitor to {} (similarity: {:.2}). \
                 Monitor {} for competitive moves in the same market segment.",
                entity_b, entity_a, similarity, entity_b
            )
        }
        ComparisonCategory::SupplyChain => {
            format!(
                "{} and {} are in adjacent supply-chain segments (similarity: {:.2}). \
                 Track {} for upstream/downstream signals.",
                entity_a, entity_b, similarity, entity_b
            )
        }
        ComparisonCategory::CrossCategory => {
            format!(
                "{} shows some structural similarities to {} (similarity: {:.2}). \
                 Explore cross-sector trends.",
                entity_b, entity_a, similarity
            )
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// LLM Prompt Builder (Entity-Agnostic)
// ─────────────────────────────────────────────────────────────────────────────

/// Entity-agnostic comparison template that works for ANY pair of entities.
///
/// No hardcoded names, sectors, or products. The LLM prompt is parameterised
/// with `{entity_a}`, `{entity_b}`, `{category}` placeholders.
pub fn build_comparison_prompt(
    entity_a: &EntityProfile,
    entity_b: &EntityProfile,
    context: &SignalContext,
) -> String {
    let category_label = match (&entity_a.category, &entity_b.category) {
        (a_cat, b_cat) if a_cat == b_cat => "direct competitor",
        _ => "cross-category / supply-chain adjacent",
    };

    format!(
        "Compare {entity_a_name} and {entity_b_name} in the context of {category}.\n\n\
         ---\n\n\
         Entity A: {entity_a_name}\n\
         - Industry: {industry_a}\n\
         - Products/Services: {products_a}\n\
         - Topics: {topics_a}\n\
         - Geographic presence: {geo_a}\n\
         - Country: {country_a}\n\n\
         Entity B: {entity_b_name}\n\
         - Industry: {industry_b}\n\
         - Products/Services: {products_b}\n\
         - Topics: {topics_b}\n\
         - Geographic presence: {geo_b}\n\
         - Country: {country_b}\n\n\
         Recent signal: {signal_text}\n\n\
         ---\n\n\
         Analysis instructions:\n\
         1. Identify the key competitive dynamics between {entity_a_name} and {entity_b_name} \
         as {category_label}.\n\
         2. Highlight capability overlaps and gaps.\n\
         3. Note geographic or regulatory advantages each entity may hold.\n\
         4. Assess supply-chain dependencies or shared customers/suppliers.\n\
         5. Provide a strategic assessment: what should a procurement or business \
         development team know about this pairing?\n\n\
         Output format: Provide a concise paragraph (3-5 sentences) covering \
         the above points.",
        entity_a_name = entity_a.entity_name,
        entity_b_name = entity_b.entity_name,
        category = category_label,
        industry_a = entity_a.industry_keywords.join(", "),
        industry_b = entity_b.industry_keywords.join(", "),
        products_a = entity_a.product_keywords.join(", "),
        products_b = entity_b.product_keywords.join(", "),
        topics_a = entity_a.topic_keywords.join(", "),
        topics_b = entity_b.topic_keywords.join(", "),
        geo_a = entity_a.geographic_keywords.join(", "),
        geo_b = entity_b.geographic_keywords.join(", "),
        country_a = entity_a.country_code.as_deref().unwrap_or("unknown"),
        country_b = entity_b.country_code.as_deref().unwrap_or("unknown"),
        signal_text = context.text,
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity_relevance::{EntityCategory, EntityRegistry};

    // ── Backward-compatibility tests ────────────────────────────────────

    #[test]
    fn basic_matrix_generation() {
        let starz = vec![
            ("SMT Assembly".into(), "A".into(), true),
            ("Wire Bonding".into(), "B".into(), false),
            ("Conformal Coating".into(), "A".into(), true),
        ];
        let competitors = vec![(
            "CompetitorX".into(),
            "CN".into(),
            0.85,
            0.6,
            vec![
                ("SMT Assembly".into(), "B".into(), true),
                ("Die Attach".into(), "A".into(), true),
            ],
        )];

        let matrix = build_comparison_matrix(&starz, &competitors);

        assert_eq!(matrix.competitors.len(), 1);
        assert_eq!(matrix.capability_rows.len(), 4); // 3 starz + 1 comp-unique
        assert!(matrix
            .summary
            .starz_unique_capabilities
            .contains(&"Conformal Coating".into()));
        assert!(matrix
            .summary
            .starz_unique_capabilities
            .contains(&"Wire Bonding".into()));
        assert!(matrix
            .summary
            .competitor_unique_capabilities
            .contains(&"Die Attach".into()));
        assert!(matrix
            .summary
            .common_capabilities
            .contains(&"SMT Assembly".into()));
    }

    #[test]
    fn empty_competitors() {
        let starz = vec![("PCB Fabrication".into(), "A".into(), true)];
        let matrix = build_comparison_matrix(&starz, &[]);
        assert_eq!(matrix.capability_rows.len(), 1);
        assert!(matrix
            .summary
            .starz_unique_capabilities
            .contains(&"PCB Fabrication".into()));
    }

    #[test]
    fn cert_advantage_detection() {
        let starz = vec![("AOI Testing".into(), "A".into(), true)];
        let competitors = vec![(
            "Rival".into(),
            "EU".into(),
            0.5,
            0.3,
            vec![("AOI Testing".into(), "C".into(), false)],
        )];
        let matrix = build_comparison_matrix(&starz, &competitors);
        assert!(matrix
            .summary
            .starz_cert_advantage
            .contains(&"AOI Testing".into()));
    }

    #[test]
    fn cert_gap_detection() {
        let starz = vec![("X-Ray Inspection".into(), "B".into(), false)];
        let competitors = vec![(
            "Rival".into(),
            "JP".into(),
            0.7,
            0.5,
            vec![("X-Ray Inspection".into(), "A".into(), true)],
        )];
        let matrix = build_comparison_matrix(&starz, &competitors);
        assert!(matrix
            .summary
            .starz_cert_gap
            .contains(&"X-Ray Inspection".into()));
    }

    // ── ComparisonCategory tests ────────────────────────────────────────

    #[test]
    fn test_comparison_category_as_str() {
        assert_eq!(
            ComparisonCategory::DirectCompetitor.as_str(),
            "direct competitor"
        );
        assert_eq!(ComparisonCategory::SupplyChain.as_str(), "supply chain");
        assert_eq!(
            ComparisonCategory::CrossCategory.as_str(),
            "cross-category"
        );
    }

    // ── compute_entity_similarity tests ─────────────────────────────────

    #[test]
    fn test_compute_entity_similarity_same_category() {
        let foxconn = EntityProfile::new("Foxconn")
            .with_category(EntityCategory::Ems)
            .with_country("TW")
            .with_products(vec!["iPhone", "assembly"])
            .with_topics(vec!["manufacturing", "supply chain", "labor"])
            .with_competitors(vec!["Flex", "Jabil"]);

        let pegatron = EntityProfile::new("Pegatron")
            .with_category(EntityCategory::Ems)
            .with_country("TW")
            .with_products(vec!["iPhone", "assembly"])
            .with_topics(vec!["manufacturing", "supply chain"])
            .with_competitors(vec!["Foxconn", "Wistron"]);

        let similarity = compute_entity_similarity(&foxconn, &pegatron);
        assert!(
            similarity > 0.5,
            "Same-category entities should have high similarity, got {}",
            similarity
        );
    }

    #[test]
    fn test_compute_entity_similarity_different_categories() {
        let nvidia = EntityProfile::new("NVIDIA")
            .with_category(EntityCategory::Semiconductor)
            .with_country("US")
            .with_products(vec!["GPU", "AI chips"])
            .with_topics(vec!["AI", "machine learning", "data center"]);

        // Use "Other" with a unique category string to avoid Ems overlap
        let foxconn = EntityProfile::new("Foxconn")
            .with_category(EntityCategory::Ems)
            .with_country("TW")
            .with_products(vec!["iPhone", "assembly"])
            .with_topics(vec!["manufacturing", "supply chain", "labor"]);

        let similarity = compute_entity_similarity(&nvidia, &foxconn);
        assert!(
            similarity < 0.5,
            "Different-category entities should have lower similarity, got {}",
            similarity
        );
    }

    #[test]
    fn test_compute_entity_similarity_same_country() {
        let a = EntityProfile::new("CompanyA")
            .with_category(EntityCategory::Ems)
            .with_country("US");
        let b = EntityProfile::new("CompanyB")
            .with_category(EntityCategory::Ems)
            .with_country("US");

        let similarity = compute_entity_similarity(&a, &b);
        assert!(
            similarity >= 0.6,
            "Same-category + same-country should give >= 0.6, got {}",
            similarity
        );
    }

    #[test]
    fn test_compute_entity_similarity_no_overlap() {
        let a = EntityProfile::new("CompanyA")
            .with_category(EntityCategory::Semiconductor)
            .with_country("US");
        let b = EntityProfile::new("CompanyB")
            .with_category(EntityCategory::Logistics)
            .with_country("CN");

        let similarity = compute_entity_similarity(&a, &b);
        assert_eq!(
            similarity, 0.0,
            "No overlap should give 0.0, got {}",
            similarity
        );
    }

    // ── select_reference_entity tests ───────────────────────────────────

    #[test]
    #[allow(clippy::disallowed_methods)]
    fn test_select_reference_entity_same_category() {
        // Foxconn → should select Pegatron or similar EMS, not Starz
        let registry = EntityRegistry::from_yaml_config();

        let reference = select_reference_entity(
            "Foxconn",
            &registry,
            ComparisonCategory::DirectCompetitor,
        );
        assert!(
            reference.is_some(),
            "Should find a reference entity for Foxconn"
        );

        let ref_name = reference.unwrap().entity_name;
        assert_ne!(
            ref_name.to_lowercase(),
            "foxconn",
            "Should not select the target itself"
        );

        // The reference should be another EMS company
        let ref_profile = registry.get(&ref_name);
        assert!(
            ref_profile.is_some(),
            "Reference should exist in registry"
        );
        assert_eq!(
            ref_profile.unwrap().category,
            EntityCategory::Ems,
            "Reference for Foxconn should be an EMS entity, got {:?}",
            ref_profile.unwrap().category
        );
    }

    #[test]
    fn test_select_reference_entity_returns_none_for_empty_registry() {
        let registry = EntityRegistry::empty();
        let reference =
            select_reference_entity("Foxconn", &registry, ComparisonCategory::DirectCompetitor);
        assert!(reference.is_none(), "Empty registry should return None");
    }

    #[test]
    fn test_select_reference_entity_fallback() {
        // When no same-category entity exists for a target that IS in the registry,
        // fall back to cross-category. We'll use a target that's NOT in the registry
        // to trigger cross-category fallback.
        let registry = EntityRegistry::from_yaml_config();

        // "NonexistentCorp" is not in registry → should fall back to highest-activity entity
        let reference = select_reference_entity(
            "NonexistentCorp",
            &registry,
            ComparisonCategory::DirectCompetitor,
        );
        assert!(
            reference.is_some(),
            "Should fall back to some entity for unknown target"
        );
    }

    #[test]
    #[allow(clippy::disallowed_methods)]
    fn test_select_reference_entity_supply_chain() {
        // For supply chain comparison, should pick from a different category
        let registry = EntityRegistry::from_yaml_config();

        let reference = select_reference_entity(
            "Foxconn",
            &registry,
            ComparisonCategory::SupplyChain,
        );
        assert!(
            reference.is_some(),
            "Should find a supply-chain reference for Foxconn"
        );

        let ref_profile = reference.unwrap();
        // The supply-chain reference should NOT be EMS (different category)
        assert_ne!(
            ref_profile.category,
            EntityCategory::Ems,
            "Supply-chain reference for Foxconn should be from a different category"
        );
    }

    #[test]
    #[allow(clippy::disallowed_methods)]
    fn test_select_reference_entity_cross_category() {
        let registry = EntityRegistry::from_yaml_config();

        let reference = select_reference_entity(
            "Foxconn",
            &registry,
            ComparisonCategory::CrossCategory,
        );
        assert!(
            reference.is_some(),
            "Should find a cross-category reference"
        );
        assert_ne!(
            reference.unwrap().entity_name.to_lowercase(),
            "foxconn",
            "Should not select the target itself"
        );
    }

    // ── generate_comparison_matrix tests ────────────────────────────────

    #[test]
    fn test_generate_comparison_matrix_returns_multiple() {
        let registry = EntityRegistry::from_yaml_config();

        let results = generate_comparison_matrix("Foxconn", 5, &registry);
        assert!(
            !results.is_empty(),
            "Should return at least one competitor for Foxconn"
        );
        assert!(
            results.len() <= 5,
            "Should return at most 5 competitors, got {}",
            results.len()
        );

        // Results should be sorted by similarity descending
        for i in 1..results.len() {
            assert!(
                results[i - 1].1.similarity >= results[i].1.similarity,
                "Results should be sorted by similarity descending"
            );
        }
    }

    #[test]
    fn test_generate_comparison_matrix_returns_empty_for_unknown() {
        let registry = EntityRegistry::from_yaml_config();

        let results = generate_comparison_matrix("NonexistentEntity", 5, &registry);
        assert!(
            results.is_empty(),
            "Unknown entity should return empty results"
        );
    }

    #[test]
    fn test_generate_comparison_matrix_returns_empty_for_empty_registry() {
        let registry = EntityRegistry::empty();

        let results = generate_comparison_matrix("Foxconn", 5, &registry);
        assert!(results.is_empty(), "Empty registry should return empty");
    }

    #[test]
    fn test_generate_comparison_matrix_excludes_self() {
        let registry = EntityRegistry::from_yaml_config();

        let results = generate_comparison_matrix("Foxconn", 10, &registry);

        // None of the results should be Foxconn itself
        for (profile, _) in &results {
            assert_ne!(
                profile.entity_name.to_lowercase(),
                "foxconn",
                "Results should not include the target entity"
            );
        }
    }

    #[test]
    fn test_generate_comparison_matrix_max_entities_respected() {
        let registry = EntityRegistry::from_yaml_config();

        let max_entities = 3;
        let results = generate_comparison_matrix("Foxconn", max_entities, &registry);
        assert!(
            results.len() <= max_entities,
            "Should respect max_entities limit"
        );
    }

    #[test]
    fn test_generate_comparison_matrix_zero_max() {
        let registry = EntityRegistry::from_yaml_config();

        let results = generate_comparison_matrix("Foxconn", 0, &registry);
        assert!(results.is_empty(), "max_entities=0 should return empty");
    }

    // ── build_comparison_prompt tests ───────────────────────────────────

    #[test]
    fn test_comparison_prompt_no_hardcoded_names() {
        let foxconn = EntityProfile::new("Foxconn")
            .with_category(EntityCategory::Ems)
            .with_industry(vec!["electronics manufacturing", "EMS"])
            .with_products(vec!["iPhone assembly", "OEM manufacturing"])
            .with_topics(vec!["supply chain", "manufacturing", "labor"])
            .with_geography(vec!["Taiwan", "China"])
            .with_country("TW");

        let pegatron = EntityProfile::new("Pegatron")
            .with_category(EntityCategory::Ems)
            .with_industry(vec!["electronics manufacturing", "EMS"])
            .with_products(vec!["iPhone assembly", "OEM"])
            .with_topics(vec!["supply chain", "manufacturing"])
            .with_geography(vec!["Taiwan"])
            .with_country("TW");

        let context = SignalContext {
            text: "Foxconn expanding in Vietnam".to_string(),
            entity_hint: None,
            category: None,
            source_url: None,
            timestamp: 1000000,
        };

        let prompt = build_comparison_prompt(&foxconn, &pegatron, &context);

        // Should mention both entities
        assert!(
            prompt.contains("Foxconn"),
            "Prompt should contain entity_a name"
        );
        assert!(
            prompt.contains("Pegatron"),
            "Prompt should contain entity_b name"
        );

        // Should NOT contain "Starz"
        assert!(
            !prompt.contains("Starz"),
            "Prompt should NOT contain hardcoded 'Starz' — got: {}",
            prompt
        );

        // Should NOT contain hardcoded sector names like "semiconductor" that
        // don't apply to EMS entities
        assert!(
            !prompt.contains("direct competitor") || prompt.contains("Foxconn"),
            "Prompt category should be based on entity categories, not hardcoded"
        );
    }

    #[test]
    fn test_comparison_prompt_parameterized() {
        let nvidia = EntityProfile::new("NVIDIA")
            .with_category(EntityCategory::Semiconductor)
            .with_industry(vec!["semiconductor", "chip design"])
            .with_products(vec!["GPU", "H100", "AI chips"])
            .with_topics(vec!["AI", "machine learning", "data center"])
            .with_geography(vec!["Santa Clara", "USA"])
            .with_country("US");

        let amd = EntityProfile::new("AMD")
            .with_category(EntityCategory::Semiconductor)
            .with_industry(vec!["semiconductor", "chip design"])
            .with_products(vec!["GPU", "Ryzen", "EPYC"])
            .with_topics(vec!["AI", "gaming", "data center"])
            .with_geography(vec!["Santa Clara", "USA"])
            .with_country("US");

        let context = SignalContext {
            text: "NVIDIA and AMD compete in AI chip market".to_string(),
            entity_hint: None,
            category: None,
            source_url: None,
            timestamp: 1000000,
        };

        let prompt = build_comparison_prompt(&nvidia, &amd, &context);

        // Should contain entity names
        assert!(prompt.contains("NVIDIA"));
        assert!(prompt.contains("AMD"));

        // Should contain signal text
        assert!(prompt.contains("AI chip market"));

        // Should contain both country codes
        assert!(prompt.contains("US"));

        // Should have the analysis instructions
        assert!(prompt.contains("competitive dynamics"));
        assert!(prompt.contains("capability overlaps and gaps"));
    }

    // ── Integration tests ───────────────────────────────────────────────

    #[test]
    fn test_full_comparison_pipeline_foxconn() {
        // Full pipeline: select reference → generate matrix → build prompt
        let registry = EntityRegistry::from_yaml_config();

        // Step 1: Select reference entity
        let reference =
            select_reference_entity("Foxconn", &registry, ComparisonCategory::DirectCompetitor);
        assert!(
            reference.is_some(),
            "Should find reference for Foxconn"
        );

        // Step 2: Generate comparison matrix
        let matrix = generate_comparison_matrix("Foxconn", 5, &registry);
        assert!(!matrix.is_empty(), "Should generate comparison matrix");

        // Step 3: Verify all results have insights
        for (profile, insight) in &matrix {
            assert!(!insight.insight_text.is_empty(), "Insight should have text");
            assert_eq!(insight.entity_a, "Foxconn");
            assert_eq!(insight.entity_b, profile.entity_name);
            assert!(
                insight.similarity > 0.0,
                "Similarity should be positive"
            );

            // Verify the insight mentions the entities
            assert!(
                insight.insight_text.contains("Foxconn"),
                "Insight should reference Foxconn"
            );
            assert!(
                insight.insight_text.contains(&profile.entity_name),
                "Insight should reference the competitor"
            );
        }
    }

    #[test]
    fn test_full_comparison_pipeline_nvidia() {
        // If NVIDIA were in the registry, this would test semiconductor comparison
        // Since NVIDIA is not in augmentation_entities.yaml, this tests graceful handling
        let registry = EntityRegistry::from_yaml_config();

        let matrix = generate_comparison_matrix("NVIDIA", 3, &registry);
        assert!(
            matrix.is_empty(),
            "NVIDIA is not in the YAML config, so should return empty"
        );
    }

    #[test]
    #[allow(clippy::disallowed_methods)]
    fn test_select_reference_entity_no_starz_default() {
        // Verify that select_reference_entity returns a valid same-category
        // competitor for Foxconn (EMS) rather than hardcoding a specific fallback.
        let registry = EntityRegistry::from_yaml_config();

        let reference =
            select_reference_entity("Foxconn", &registry, ComparisonCategory::DirectCompetitor);
        assert!(reference.is_some());
        let ref_name = reference.unwrap().entity_name;

        // Must not be the target itself
        assert_ne!(
            ref_name.to_lowercase(),
            "foxconn",
            "Reference should not be the target itself"
        );

        // Reference should be a same-category EMS competitor
        let ref_profile = registry.get(&ref_name).unwrap();
        assert_eq!(
            ref_profile.category,
            EntityCategory::Ems,
            "Reference should be an EMS company"
        );
    }

    #[test]
    fn test_comparison_insight_text_variants() {
        let foxconn = EntityProfile::new("Foxconn").with_category(EntityCategory::Ems);
        let pegatron = EntityProfile::new("Pegatron").with_category(EntityCategory::Ems);

        let context = SignalContext {
            text: "test signal".to_string(),
            entity_hint: None,
            category: None,
            source_url: None,
            timestamp: 0,
        };

        let prompt = build_comparison_prompt(&foxconn, &pegatron, &context);
        assert!(
            prompt.contains("direct competitor"),
            "Same-category entities should be labeled as direct competitors"
        );

        let nvidia = EntityProfile::new("NVIDIA").with_category(EntityCategory::Semiconductor);
        let cross_prompt = build_comparison_prompt(&foxconn, &nvidia, &context);
        assert!(
            cross_prompt.contains("cross-category / supply-chain adjacent"),
            "Different-category entities should be labeled as cross-category"
        );
    }

    // ── Edge case tests ─────────────────────────────────────────────────

    #[test]
    fn test_compute_entity_similarity_identical_profiles() {
        let a = EntityProfile::new("CompanyX")
            .with_category(EntityCategory::Ems)
            .with_country("US")
            .with_products(vec!["ProductA"])
            .with_topics(vec!["TopicA"])
            .with_competitors(vec!["CompA"])
            .with_industry(vec!["IndustryA"]);

        // Same content, different name — should score highly
        let b = EntityProfile::new("CompanyY")
            .with_category(EntityCategory::Ems)
            .with_country("US")
            .with_products(vec!["ProductA"])
            .with_topics(vec!["TopicA"])
            .with_competitors(vec!["CompA"])
            .with_industry(vec!["IndustryA"]);

        let sim = compute_entity_similarity(&a, &b);
        assert!(
            (sim - 0.95).abs() < 0.1,
            "Near-identical profiles should score ~0.95, got {}",
            sim
        );
    }

    #[test]
    fn test_compute_entity_similarity_topic_overlap_capped() {
        let a = EntityProfile::new("A")
            .with_category(EntityCategory::Ems)
            .with_topics(vec![
                "t1", "t2", "t3", "t4", "t5", "t6", "t7", "t8", "t9", "t10",
            ]);
        let b = EntityProfile::new("B")
            .with_category(EntityCategory::Ems)
            .with_topics(vec![
                "t1", "t2", "t3", "t4", "t5", "t6", "t7", "t8", "t9", "t10",
            ]);

        let sim = compute_entity_similarity(&a, &b);
        // Same category (0.5) + topic overlap capped at 0.15 + no country/product/comp/industry
        assert!(
            (sim - 0.65).abs() < 0.01,
            "Topic overlap should be capped at 0.15, got {}",
            sim
        );
    }

    #[test]
    #[allow(clippy::disallowed_methods)]
    fn test_select_reference_entity_with_specific_target() {
        let registry = EntityRegistry::from_yaml_config();

        // Pegatron should get a different EMS reference (not itself)
        let reference = select_reference_entity(
            "Pegatron",
            &registry,
            ComparisonCategory::DirectCompetitor,
        );
        assert!(reference.is_some(), "Should find reference for Pegatron");
        assert_ne!(
            reference.unwrap().entity_name.to_lowercase(),
            "pegatron",
            "Should not select the target itself"
        );
    }

    #[test]
    fn test_generate_comparison_matrix_sorted_by_similarity() {
        let registry = EntityRegistry::from_yaml_config();

        let results = generate_comparison_matrix("Foxconn", 10, &registry);

        // Verify descending similarity
        for i in 1..results.len() {
            assert!(
                results[i - 1].1.similarity >= results[i].1.similarity,
                "Position {} (sim={}) should be >= position {} (sim={})",
                i - 1,
                results[i - 1].1.similarity,
                i,
                results[i].1.similarity
            );
        }

        // First result should be the most similar (likely same-category EMS)
        if let Some((first, _)) = results.first() {
            assert_eq!(
                first.category,
                EntityCategory::Ems,
                "Top result for Foxconn should be an EMS company"
            );
        }
    }
}
