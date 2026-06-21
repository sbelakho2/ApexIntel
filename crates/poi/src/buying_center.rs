//! Buying Center Role Mapping for Accurate Contact Recommendations
//!
//! This module solves the CEO/director recommendation bias by mapping
//! each role family to the correct buying center role, ensuring insights
//! recommend contacting the *right* person (procurement manager, supply
//! chain director, quality engineer) instead of always defaulting to
//! CEO/executive level.
//!
//! The buying center model (Webster & Wind, 1972) identifies 6 roles:
//! - Initiator: identifies the need
//! - Gatekeeper: controls information flow
//! - Influencer: shapes specifications and evaluation criteria
//! - Decider: makes the final decision
//! - Buyer: manages the purchasing process
//! - User: consumes the product/service
//!
//! In EMS/manufacturing, procurement managers and supply chain directors
//! are typically the Buyers and Influencers — the people you actually
//! need to reach. C-suite executives are usually Deciders only for
//! strategic partnerships, not day-to-day purchasing.

use crate::model::RoleFamily;

/// Maps a role family to the buying center role it most likely plays
/// in an EMS/manufacturing procurement decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuyingCenterRole {
    /// Initiates the purchase process — identifies the need
    Initiator,
    /// Controls information flow to other participants
    Gatekeeper,
    /// Shapes specifications, requirements, and evaluation criteria
    Influencer,
    /// Makes or approves the final decision
    Decider,
    /// Manages the purchasing/contracting process
    Buyer,
    /// Uses the product or service
    User,
}

impl BuyingCenterRole {
    /// Returns the recommended engagement priority (lower = higher priority).
    pub fn priority(&self) -> u8 {
        match self {
            // Buyer is highest priority — they run the RFP/RFQ process
            Self::Buyer => 1,
            // Influencer shapes the decision — second priority
            Self::Influencer => 2,
            // Initiator identifies the need — third priority
            Self::Initiator => 3,
            // Gatekeeper controls access — fourth priority
            Self::Gatekeeper => 4,
            // Decider approves — fifth priority (only needed at final stage)
            Self::Decider => 5,
            // User — lowest priority for initial outreach
            Self::User => 6,
        }
    }

    /// Human-readable label for display.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Initiator => "Initiator",
            Self::Gatekeeper => "Gatekeeper",
            Self::Influencer => "Influencer",
            Self::Decider => "Decider",
            Self::Buyer => "Buyer",
            Self::User => "User",
        }
    }

    /// Recommended outreach strategy for this role.
    pub fn outreach_strategy(&self) -> &'static str {
        match self {
            Self::Buyer => "Reach out first — they run the procurement process. Present TCO analysis, SLA terms, and compliance certifications.",
            Self::Influencer => "Engage second — they shape requirements. Present technical capabilities, quality metrics, and case studies.",
            Self::Initiator => "Engage early — they identified the need. Present capability overview and how you solve their pain point.",
            Self::Gatekeeper => "Build relationship — they control access. Be professional, direct, and respect their process.",
            Self::Decider => "Engage last — they approve the final decision. Present strategic value, risk mitigation, and partnership vision.",
            Self::User => "Engage for feedback — they'll use what you build. Present reliability, support, and ease of integration.",
        }
    }
}

/// Maps a RoleFamily to the most likely BuyingCenterRole.
///
/// This is the **critical fix** for the CEO/director recommendation bias.
/// Previously, `Executive` role family was the fallback/default, causing
/// insights to always suggest contacting C-suite. Now we correctly map
/// procurement, supply chain, engineering, and quality roles to their
/// actual buying center positions.
pub fn role_to_buying_center(role: &RoleFamily) -> BuyingCenterRole {
    match role {
        // Procurement/Sourcing/Purchasing → BUYER (highest priority)
        RoleFamily::Procurement => BuyingCenterRole::Buyer,

        // Quality (general) → INFLUENCER
        RoleFamily::Quality => BuyingCenterRole::Influencer,

        // Supplier Quality → INFLUENCER
        RoleFamily::SupplierQuality => BuyingCenterRole::Influencer,

        // Engineering/R&D → INFLUENCER
        RoleFamily::Engineering => BuyingCenterRole::Influencer,

        // Operations/Manufacturing → USER
        RoleFamily::Operations => BuyingCenterRole::User,

        // Security/InfoSec → GATEKEEPER
        RoleFamily::Security => BuyingCenterRole::Gatekeeper,

        // Executive/C-Suite → DECIDER
        RoleFamily::Executive => BuyingCenterRole::Decider,

        // Government → DECIDER
        RoleFamily::Government => BuyingCenterRole::Decider,

        // Free Zone Authority → DECIDER
        RoleFamily::FreeZoneAuthority => BuyingCenterRole::Decider,

        // Logistics → INFLUENCER (supply chain execution)
        RoleFamily::Logistics => BuyingCenterRole::Influencer,

        // Port Logistics → INFLUENCER
        RoleFamily::PortLogistics => BuyingCenterRole::Influencer,

        // Certification Body → INFLUENCER (quality/compliance gate)
        RoleFamily::CertificationBody => BuyingCenterRole::Influencer,

        // Industry Association → INFLUENCER
        RoleFamily::IndustryAssociation => BuyingCenterRole::Influencer,

        // Distributor → BUYER (purchasing channel)
        RoleFamily::Distributor => BuyingCenterRole::Buyer,

        // Finance → GATEKEEPER (budget approval)
        RoleFamily::Finance => BuyingCenterRole::Gatekeeper,

        // Legal → GATEKEEPER (contract review)
        RoleFamily::Legal => BuyingCenterRole::Gatekeeper,

        // Military → DECIDER
        RoleFamily::Military => BuyingCenterRole::Decider,

        // Intelligence → GATEKEEPER
        RoleFamily::Intelligence => BuyingCenterRole::Gatekeeper,

        // Other → varies by label
        RoleFamily::Other(label) => match label.to_lowercase().as_str() {
            s if s.contains("sales") || s.contains("marketing") => BuyingCenterRole::Influencer,
            s if s.contains("finance") => BuyingCenterRole::Gatekeeper,
            s if s.contains("hr") || s.contains("human resources") => BuyingCenterRole::User,
            s if s.contains("legal") => BuyingCenterRole::Gatekeeper,
            s if s.contains("procurement") || s.contains("sourcing") || s.contains("purchasing") || s.contains("buyer") => BuyingCenterRole::Buyer,
            s if s.contains("supply chain") || s.contains("logistics") => BuyingCenterRole::Influencer,
            s if s.contains("quality") => BuyingCenterRole::Influencer,
            s if s.contains("engineer") || s.contains("r&d") => BuyingCenterRole::Influencer,
            _ => BuyingCenterRole::Influencer,
        },
    }
}

/// Returns the recommended contact person for a given entity's POIs,
/// prioritizing procurement/supply chain over executive contacts.
///
/// # Priority order:
/// 1. Procurement Manager / Sourcing Director (BUYER)
/// 2. Supplier Quality Manager (INFLUENCER)
/// 3. Supply Chain Director (INFLUENCER)
/// 4. Engineering Lead (INFLUENCER)
/// 5. Operations Manager (USER)
/// 6. CEO / Managing Director (DECIDER — last resort)
pub struct ContactRecommendation {
    /// The name of the recommended person to contact
    pub person_name: String,
    /// Their role title
    pub role_title: String,
    /// Their role family
    pub role_family: String,
    /// Their buying center role
    pub buying_center_role: BuyingCenterRole,
    /// Priority (1 = highest)
    pub priority: u8,
    /// Why this person is recommended
    pub reason: String,
}

/// Convert a RoleFamily to a human-readable display name.
fn role_family_to_display_name(role: &RoleFamily) -> String {
    match role {
        RoleFamily::Procurement => "Procurement".to_string(),
        RoleFamily::Quality => "Quality".to_string(),
        RoleFamily::SupplierQuality => "Supplier Quality".to_string(),
        RoleFamily::Engineering => "Engineering".to_string(),
        RoleFamily::Operations => "Operations".to_string(),
        RoleFamily::Security => "Security".to_string(),
        RoleFamily::Executive => "Executive".to_string(),
        RoleFamily::Government => "Government".to_string(),
        RoleFamily::FreeZoneAuthority => "Free Zone Authority".to_string(),
        RoleFamily::Logistics => "Logistics".to_string(),
        RoleFamily::PortLogistics => "Port Logistics".to_string(),
        RoleFamily::CertificationBody => "Certification Body".to_string(),
        RoleFamily::IndustryAssociation => "Industry Association".to_string(),
        RoleFamily::Distributor => "Distributor".to_string(),
        RoleFamily::Finance => "Finance".to_string(),
        RoleFamily::Legal => "Legal".to_string(),
        RoleFamily::Military => "Military".to_string(),
        RoleFamily::Intelligence => "Intelligence".to_string(),
        RoleFamily::Other(label) => label.clone(),
    }
}

/// Given a list of POIs, returns ordered contact recommendations
/// with procurement/supply chain prioritized over executives.
pub fn recommend_contacts(pois: &[crate::model::PoiProfile]) -> Vec<ContactRecommendation> {
    let mut recommendations: Vec<ContactRecommendation> = pois
        .iter()
        .map(|poi| {
            let bc_role = role_to_buying_center(&poi.role_family);
            let role_str = role_family_to_display_name(&poi.role_family);

            ContactRecommendation {
                person_name: poi.name.clone(),
                role_title: poi.current_role.clone(),
                role_family: role_str,
                buying_center_role: bc_role,
                priority: bc_role.priority(),
                reason: format!(
                    "{} — {} at {}. {}",
                    bc_role.label(),
                    poi.current_role,
                    poi.org,
                    bc_role.outreach_strategy(),
                ),
            }
        })
        .collect();

    // Sort by buying center priority (Buyer=1 first, Decider=5 last)
    recommendations.sort_by_key(|r| r.priority);
    recommendations
}

/// Generate a contact recommendation paragraph for inclusion in insights.
///
/// This replaces the old pattern of "Contact the CEO or Managing Director
/// directly" with accurate, role-appropriate recommendations.
pub fn contact_recommendation_text(recommendations: &[ContactRecommendation]) -> String {
    if recommendations.is_empty() {
        return "No key contacts identified for this entity.".to_string();
    }

    let top = &recommendations[0];
    let mut text = format!(
        "**Recommended contact:** {} ({}) — {} at {}. {}",
        top.person_name,
        top.buying_center_role.label(),
        top.role_title,
        top.role_family,
        top.reason,
    );

    // Add secondary contacts if available
    if recommendations.len() > 1 {
        let others: Vec<String> = recommendations[1..]
            .iter()
            .take(2) // max 2 additional
            .map(|r| format!("{} ({})", r.person_name, r.buying_center_role.label()))
            .collect();
        if !others.is_empty() {
            text.push_str(&format!(
                "\n\n**Also relevant:** {}.",
                others.join(", ")
            ));
        }
    }

    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{PoiProfile, PsychProfile, InfluenceProfile, PriorityVector, ChangeAppetite, DecisionStyle, ProofType};

    fn make_poi(name: &str, role: &str, org: &str, role_family: RoleFamily) -> PoiProfile {
        PoiProfile {
            person_id: format!("poi_{}", name.to_lowercase()),
            name: name.to_string(),
            name_variants: vec![],
            org: org.to_string(),
            org_id: None,
            current_role: role.to_string(),
            role_family,
            region: "US".to_string(),
            country_code: "US".to_string(),
            public_bio: String::new(),
            public_email: None,
            artifacts: vec![],
            priority_vector: PriorityVector::zero(),
            psychological: PsychProfile {
                decision_style: DecisionStyle::BalancedAnalytical,
                change_appetite: ChangeAppetite::Pragmatist,
                pain_index: 0.3,
                preferred_proof: vec![ProofType::KpiMetrics],
                risk_tolerance: 0.5,
            },
            influence: InfluenceProfile {
                influence_score: 0.5,
                graph_centrality: 0.3,
                public_recurrence: 0.2,
                role_seniority_score: 0.5,
                network_size: 10,
            },
            engagement: None,
            role_history: vec![],
            last_updated_utc: 0,
            profile_completeness: 0.5,
        }
    }

    #[test]
    fn test_procurement_maps_to_buyer() {
        let role = role_to_buying_center(&RoleFamily::Procurement);
        assert_eq!(role, BuyingCenterRole::Buyer);
        assert_eq!(role.priority(), 1); // Highest priority
    }

    #[test]
    fn test_executive_maps_to_decider() {
        let role = role_to_buying_center(&RoleFamily::Executive);
        assert_eq!(role, BuyingCenterRole::Decider);
        assert_eq!(role.priority(), 5); // Lowest priority (except User)
    }

    #[test]
    fn test_quality_maps_to_influencer() {
        let role = role_to_buying_center(&RoleFamily::SupplierQuality);
        assert_eq!(role, BuyingCenterRole::Influencer);
    }

    #[test]
    fn test_recommend_contacts_prioritizes_buyer_over_executive() {
        let pois = vec![
            make_poi("CEO Person", "CEO", "Target Corp", RoleFamily::Executive),
            make_poi("Procurement Manager", "Global Sourcing Director", "Target Corp", RoleFamily::Procurement),
            make_poi("Quality Lead", "Supplier Quality Manager", "Target Corp", RoleFamily::SupplierQuality),
        ];

        let recs = recommend_contacts(&pois);

        // First recommendation should be Procurement (Buyer, priority 1)
        assert_eq!(recs[0].person_name, "Procurement Manager");
        assert_eq!(recs[0].buying_center_role, BuyingCenterRole::Buyer);

        // Second should be Quality (Influencer, priority 2)
        assert_eq!(recs[1].person_name, "Quality Lead");
        assert_eq!(recs[1].buying_center_role, BuyingCenterRole::Influencer);

        // Third should be CEO (Decider, priority 5)
        assert_eq!(recs[2].person_name, "CEO Person");
        assert_eq!(recs[2].buying_center_role, BuyingCenterRole::Decider);
    }

    #[test]
    fn test_contact_recommendation_text_mentions_buyer_first() {
        let recs = vec![
            ContactRecommendation {
                person_name: "Procurement Manager".to_string(),
                role_title: "Global Sourcing Director".to_string(),
                role_family: "Procurement".to_string(),
                buying_center_role: BuyingCenterRole::Buyer,
                priority: 1,
                reason: "Buyer — they run the procurement process".to_string(),
            },
            ContactRecommendation {
                person_name: "CEO Person".to_string(),
                role_title: "CEO".to_string(),
                role_family: "Executive".to_string(),
                buying_center_role: BuyingCenterRole::Decider,
                priority: 5,
                reason: "Decider — approves final decision".to_string(),
            },
        ];

        let text = contact_recommendation_text(&recs);

        // Should mention procurement manager first
        assert!(text.contains("Procurement Manager"));
        assert!(text.contains("Buyer"));

        // Should mention CEO as secondary
        assert!(text.contains("Also relevant"));
        assert!(text.contains("CEO Person"));
    }

    #[test]
    fn test_empty_recommendations() {
        let text = contact_recommendation_text(&[]);
        assert!(text.contains("No key contacts identified"));
    }

    #[test]
    fn test_buyer_has_highest_priority() {
        assert!(BuyingCenterRole::Buyer.priority() < BuyingCenterRole::Influencer.priority());
        assert!(BuyingCenterRole::Influencer.priority() < BuyingCenterRole::Decider.priority());
        assert!(BuyingCenterRole::Decider.priority() < BuyingCenterRole::User.priority());
    }
}