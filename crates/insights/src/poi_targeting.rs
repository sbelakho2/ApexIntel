//! POI-aware recommendation targeting for insights.
//!
//! Replaces generic "contact CEO/director" recommendations with
//! specific, role-appropriate POI names and titles sourced from
//! the entity's known persons of interest.
//!
//! # How It Works
//!
//! 1. When rendering an insight for an entity, this module looks up
//!    all known POIs for that entity.
//! 2. It maps the insight's category/signal to the most appropriate
//!    role function (procurement, supply chain, quality, etc.).
//! 3. It finds the best-matching POI(s) by role family and seniority.
//! 4. It generates specific, actionable contact recommendations.
//!
//! # Fallback
//!
//! If no POIs exist for the entity, the module returns generic-but-
//! appropriate role recommendations (e.g., "contact the procurement
//! manager" instead of "contact the CEO").

use std::collections::HashMap;
use serde::{Deserialize, Serialize};

/// A lightweight POI reference used for insight targeting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoiRef {
    pub person_id: String,
    pub name: String,
    pub title: String,
    pub role_family: String,
    pub seniority: String,
    pub org: String,
    pub relevance_score: f64,
}

/// Mapping from insight category/signal to the appropriate target role.
#[derive(Debug, Clone)]
pub struct CategoryRoleMapping {
    pub category: &'static str,
    pub preferred_roles: &'static [&'static str],
    pub fallback_action: &'static str,
}

/// The full category-to-role mapping table.
/// Ordered by specificity — more specific categories checked first.
static CATEGORY_ROLE_MAP: &[CategoryRoleMapping] = &[
    // Supply Chain / Procurement
    CategoryRoleMapping {
        category: "supply_chain",
        preferred_roles: &["Supply Chain", "Procurement", "Sourcing", "Logistics"],
        fallback_action: "Contact the supply chain or procurement manager for this entity.",
    },
    CategoryRoleMapping {
        category: "procurement",
        preferred_roles: &["Procurement", "Sourcing", "Purchasing", "Supply Chain"],
        fallback_action: "Reach out to the procurement or sourcing manager for this entity.",
    },
    CategoryRoleMapping {
        category: "logistics",
        preferred_roles: &["Logistics", "Supply Chain", "Operations", "Warehouse"],
        fallback_action: "Contact the logistics or supply chain manager for this entity.",
    },
    // Quality / Compliance
    CategoryRoleMapping {
        category: "quality",
        preferred_roles: &["Quality", "Compliance", "Regulatory", "Engineering"],
        fallback_action: "Contact the quality or compliance manager for this entity.",
    },
    CategoryRoleMapping {
        category: "compliance",
        preferred_roles: &["Compliance", "Regulatory", "Legal", "Quality"],
        fallback_action: "Contact the compliance or regulatory lead for this entity.",
    },
    CategoryRoleMapping {
        category: "certification",
        preferred_roles: &["Quality", "Compliance", "Engineering", "Operations"],
        fallback_action: "Contact the quality or engineering manager for this entity.",
    },
    // Engineering / Manufacturing
    CategoryRoleMapping {
        category: "manufacturing",
        preferred_roles: &["Engineering", "Operations", "Production", "Plant"],
        fallback_action: "Contact the manufacturing or operations lead for this entity.",
    },
    CategoryRoleMapping {
        category: "engineering",
        preferred_roles: &["Engineering", "Technical", "R&D", "Operations"],
        fallback_action: "Contact the engineering or technical lead for this entity.",
    },
    // Sales / Business Development
    CategoryRoleMapping {
        category: "sales",
        preferred_roles: &["Sales", "Business Development", "Marketing", "Commercial"],
        fallback_action: "Contact the sales or business development manager for this entity.",
    },
    CategoryRoleMapping {
        category: "market",
        preferred_roles: &["Sales", "Marketing", "Business Development", "Strategy"],
        fallback_action: "Contact the sales or marketing lead for this entity.",
    },
    // Security / Threat
    CategoryRoleMapping {
        category: "security",
        preferred_roles: &["Security", "Compliance", "IT", "Operations"],
        fallback_action: "Contact the security or compliance lead for this entity.",
    },
    CategoryRoleMapping {
        category: "threat",
        preferred_roles: &["Security", "Compliance", "Operations", "Defense/Government"],
        fallback_action: "Contact the security lead for this entity.",
    },
    // Financial
    CategoryRoleMapping {
        category: "financial",
        preferred_roles: &["Finance", "C-Suite", "Strategy", "Operations"],
        fallback_action: "Contact the finance or strategy lead for this entity.",
    },
    // Demand / Sourcing
    CategoryRoleMapping {
        category: "demand",
        preferred_roles: &["Procurement", "Sourcing", "Sales", "Supply Chain"],
        fallback_action: "Reach out to the procurement or sourcing manager for this entity.",
    },
    // Competitor
    CategoryRoleMapping {
        category: "competitor",
        preferred_roles: &["Strategy", "Sales", "Marketing", "C-Suite"],
        fallback_action: "Contact the strategy or competitive intelligence lead for this entity.",
    },
    // Strategic
    CategoryRoleMapping {
        category: "strategic",
        preferred_roles: &["C-Suite", "Strategy", "VP/Director", "Business Development"],
        fallback_action: "Contact the relevant VP or director for this entity.",
    },
    // Default catch-all — always use entity-specific POIs if available
    CategoryRoleMapping {
        category: "general",
        preferred_roles: &["VP/Director", "C-Suite", "Manager", "Operations"],
        fallback_action: "Contact the relevant department lead for this entity.",
    },
];

/// Look up the preferred roles for a given insight category.
pub fn preferred_roles_for_category(category: &str) -> &'static [&'static str] {
    let lower = category.to_lowercase();
    for mapping in CATEGORY_ROLE_MAP {
        if lower.contains(mapping.category) {
            return mapping.preferred_roles;
        }
    }
    // Default: prefer VP/Director level or operations
    &["VP/Director", "C-Suite", "Manager", "Operations"]
}

/// Look up the fallback action text for a given insight category.
pub fn fallback_action_for_category(category: &str) -> &'static str {
    let lower = category.to_lowercase();
    for mapping in CATEGORY_ROLE_MAP {
        if lower.contains(mapping.category) {
            return mapping.fallback_action;
        }
    }
    "Contact the relevant department lead for this entity."
}

/// Score a POI's relevance to a set of preferred roles.
/// Returns a score 0.0-1.0 where higher is better.
pub fn score_poi_for_roles(poi: &PoiRef, preferred_roles: &[&str]) -> f64 {
    let mut score: f64 = 0.0;

    // 1. Role family match (primary weight)
    for (i, role) in preferred_roles.iter().enumerate() {
        if poi.role_family.eq_ignore_ascii_case(role) {
            // Earlier in the list = higher weight
            let position_weight = 1.0 - (i as f64 * 0.1).min(0.5);
            score += 0.5 * position_weight;
            break;
        }
    }

    // 2. Seniority bonus
    match poi.seniority.to_lowercase().as_str() {
        "c-level" => score += 0.15,
        "executive" => score += 0.15,
        "vp" => score += 0.25,
        "director" => score += 0.20,
        "manager" => score += 0.15,
        "senior" => score += 0.10,
        _ => score += 0.05,
    }

    // 3. Has actual job title (not just a name)
    if !poi.title.is_empty() && poi.title != "Unknown" {
        score += 0.10;
    }

    // 4. Relevance score from upstream (if provided)
    score += poi.relevance_score * 0.10;

    score.min(1.0)
}

/// Find the best POI(s) to recommend for a given category and entity.
/// Returns up to `max_count` POIs sorted by relevance.
pub fn find_target_pois(
    pois: &[PoiRef],
    category: &str,
    max_count: usize,
) -> Vec<PoiRef> {
    if pois.is_empty() {
        return Vec::new();
    }

    let preferred_roles = preferred_roles_for_category(category);

    let mut scored: Vec<(f64, &PoiRef)> = pois
        .iter()
        .map(|p| (score_poi_for_roles(p, preferred_roles), p))
        .collect();

    // Sort by relevance score descending
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    // Take top N, but only those with meaningful relevance
    let threshold = if pois.len() <= 3 { 0.0 } else { 0.15 };
    scored
        .into_iter()
        .filter(|(score, _)| *score >= threshold)
        .take(max_count)
        .map(|(_, poi)| poi.clone())
        .collect()
}

/// Generate a specific contact recommendation for an insight.
///
/// If POIs exist for the entity, returns recommendations like:
///   "Contact Ahmed Ben Ali (VP Procurement) at Foxconn Tunisia"
///
/// If no POIs exist, returns role-appropriate generic recommendations:
///   "Contact the procurement or sourcing manager for Foxconn Tunisia"
pub fn generate_contact_recommendation(
    entity_name: &str,
    category: &str,
    pois: &[PoiRef],
    max_contacts: usize,
) -> Vec<String> {
    let targets = find_target_pois(pois, category, max_contacts);

    if targets.is_empty() {
        // No POIs — use role-appropriate generic recommendation
        let fallback = fallback_action_for_category(category);
        return vec![format!("{} for {}.", fallback, entity_name)];
    }

    let mut recommendations = Vec::new();

    for (i, poi) in targets.iter().enumerate() {
        let title_display = if poi.title.is_empty() || poi.title == "Unknown" {
            String::new()
        } else {
            format!(" ({})", poi.title)
        };

        if i == 0 {
            recommendations.push(format!(
                "Contact {} at {} for {}{}.",
                poi.name,
                entity_name,
                category.replace('_', " "),
                title_display
            ));
        } else {
            recommendations.push(format!(
                "Alternatively, reach out to {} at {}{}.",
                poi.name, entity_name, title_display
            ));
        }
    }

    // Add a context note if we have POIs
    if !targets.is_empty() {
        let role_note = if category.to_lowercase().contains("procurement") {
            "This contact is directly responsible for procurement decisions."
        } else if category.to_lowercase().contains("supply") {
            "This contact oversees supply chain operations."
        } else if category.to_lowercase().contains("quality") {
            "This contact manages quality and compliance."
        } else if category.to_lowercase().contains("security") || category.to_lowercase().contains("threat") {
            "This contact handles security matters."
        } else {
            "This is the most relevant contact for this insight category."
        };
        recommendations.push(role_note.to_string());
    }

    recommendations
}

/// Replace generic "contact CEO/director" text in an action list with
/// POI-aware recommendations.
pub fn enhance_actions_with_poi_targeting(
    actions: &[String],
    entity_name: &str,
    category: &str,
    pois: &[PoiRef],
) -> Vec<String> {
    let targets = find_target_pois(pois, category, 2);

    if targets.is_empty() {
        // Replace generic "contact management" with role-appropriate text
        let fallback = fallback_action_for_category(category);
        return actions
            .iter()
            .map(|action| {
                // Replace common generic patterns
                let lower = action.to_lowercase();
                if lower.contains("contact the ceo")
                    || lower.contains("contact ceo")
                    || lower.contains("contact management")
                    || lower.contains("contact the director")
                    || lower.contains("reach out to the ceo")
                    || lower.contains("reach out to management")
                    || lower.contains("contact leadership")
                {
                    format!("{} for {}.", fallback, entity_name)
                } else if lower.contains("engage with the ceo")
                    || lower.contains("engage with management")
                {
                    format!("{} for {}.", fallback, entity_name)
                } else {
                    action.clone()
                }
            })
            .collect();
    }

    // We have POIs — insert specific contact recommendations
    let mut enhanced = Vec::new();

    // Add POI-specific contact recommendations first
    for poi in &targets {
        let title_display = if poi.title.is_empty() || poi.title == "Unknown" {
            String::new()
        } else {
            format!(" ({})", poi.title)
        };
        enhanced.push(format!(
            "Contact {}{} at {}",
            poi.name, title_display, entity_name
        ));
    }

    // Then add the original actions, with generic contact text replaced
    for action in actions {
        let lower = action.to_lowercase();
        if lower.contains("contact the ceo")
            || lower.contains("contact ceo")
            || lower.contains("contact management")
            || lower.contains("contact the director")
            || lower.contains("reach out to the ceo")
            || lower.contains("reach out to management")
            || lower.contains("engage with the ceo")
            || lower.contains("engage with management")
            || lower.contains("contact leadership")
        {
            // Skip — replaced by specific POI recommendations above
            continue;
        }
        enhanced.push(action.clone());
    }

    enhanced
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_pois() -> Vec<PoiRef> {
        vec![
            PoiRef {
                person_id: "p1".into(),
                name: "Ahmed Ben Ali".into(),
                title: "VP Procurement".into(),
                role_family: "Procurement".into(),
                seniority: "VP".into(),
                org: "Foxconn Tunisia".into(),
                relevance_score: 0.9,
            },
            PoiRef {
                person_id: "p2".into(),
                name: "Marie Dupont".into(),
                title: "Director of Supply Chain".into(),
                role_family: "Supply Chain".into(),
                seniority: "Director".into(),
                org: "Foxconn Tunisia".into(),
                relevance_score: 0.85,
            },
            PoiRef {
                person_id: "p3".into(),
                name: "Jean Martin".into(),
                title: "CEO".into(),
                role_family: "C-Suite".into(),
                seniority: "C-Level".into(),
                org: "Foxconn Tunisia".into(),
                relevance_score: 0.5,
            },
            PoiRef {
                person_id: "p4".into(),
                name: "Sophie Laurent".into(),
                title: "Quality Manager".into(),
                role_family: "Quality".into(),
                seniority: "Manager".into(),
                org: "Foxconn Tunisia".into(),
                relevance_score: 0.7,
            },
        ]
    }

    #[test]
    fn test_preferred_roles_for_procurement() {
        let roles = preferred_roles_for_category("procurement");
        assert!(roles.contains(&"Procurement"));
        assert!(roles.contains(&"Sourcing"));
    }

    #[test]
    fn test_find_target_pois_for_procurement() {
        let pois = sample_pois();
        let targets = find_target_pois(&pois, "procurement", 2);
        assert!(!targets.is_empty());
        // The VP Procurement should be first
        assert_eq!(targets[0].name, "Ahmed Ben Ali");
        assert_eq!(targets[0].role_family, "Procurement");
    }

    #[test]
    fn test_find_target_pois_for_supply_chain() {
        let pois = sample_pois();
        let targets = find_target_pois(&pois, "supply_chain", 2);
        assert!(!targets.is_empty());
        // Supply chain director should be first
        assert!(targets.iter().any(|p| p.role_family == "Supply Chain"));
    }

    #[test]
    fn test_find_target_pois_for_quality() {
        let pois = sample_pois();
        let targets = find_target_pois(&pois, "quality", 2);
        assert!(!targets.is_empty());
        assert!(targets.iter().any(|p| p.role_family == "Quality"));
    }

    #[test]
    fn test_generate_contact_recommendation_with_pois() {
        let pois = sample_pois();
        let recs = generate_contact_recommendation("Foxconn Tunisia", "procurement", &pois, 2);
        assert!(!recs.is_empty());
        assert!(recs[0].contains("Ahmed Ben Ali"));
        assert!(recs[0].contains("VP Procurement"));
        assert!(!recs[0].contains("CEO")); // Should NOT recommend CEO for procurement
    }

    #[test]
    fn test_generate_contact_recommendation_no_pois() {
        let recs = generate_contact_recommendation("Foxconn Tunisia", "procurement", &[], 2);
        assert!(!recs.is_empty());
        // Should give role-appropriate generic, NOT CEO
        assert!(!recs[0].to_lowercase().contains("ceo"));
        assert!(recs[0].to_lowercase().contains("procurement") || recs[0].to_lowercase().contains("sourcing"));
    }

    #[test]
    fn test_enhance_actions_replaces_ceo_text() {
        let actions = vec![
            "Review evidence and define immediate next step".to_string(),
            "Contact the CEO or director directly".to_string(),
            "Monitor supplier portal for updates".to_string(),
        ];
        let pois = sample_pois();
        let enhanced = enhance_actions_with_poi_targeting(&actions, "Foxconn Tunisia", "procurement", &pois);
        // Should NOT contain "CEO" anymore
        assert!(!enhanced.iter().any(|a| a.to_lowercase().contains("ceo")));
        // Should contain specific POI recommendations
        assert!(enhanced.iter().any(|a| a.contains("Ahmed Ben Ali")));
    }

    #[test]
    fn test_enhance_actions_no_pois_uses_fallback() {
        let actions = vec![
            "Contact the CEO or director directly".to_string(),
        ];
        let enhanced = enhance_actions_with_poi_targeting(&actions, "Foxconn Tunisia", "procurement", &[]);
        assert!(!enhanced.is_empty());
        // Should replace CEO with procurement-appropriate recommendation
        assert!(!enhanced[0].to_lowercase().contains("ceo"));
        assert!(enhanced[0].to_lowercase().contains("procurement") || enhanced[0].to_lowercase().contains("sourcing"));
    }

    #[test]
    fn test_fallback_action_does_not_mention_ceo() {
        // Every fallback action should be role-appropriate, not default to CEO
        for mapping in CATEGORY_ROLE_MAP {
            assert!(
                !mapping.fallback_action.to_lowercase().contains("ceo"),
                "Category '{}' fallback mentions CEO: {}",
                mapping.category,
                mapping.fallback_action
            );
        }
    }
}