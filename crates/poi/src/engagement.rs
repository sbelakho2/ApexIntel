//! Engagement profile generation — "What to say" recommendation engine.

use crate::model::*;

/// Generate a full engagement profile for a POI.
pub fn generate_engagement_profile(poi: &PoiProfile) -> EngagementProfile {
    let what_they_want_to_hear = role_talking_points(&poi.role_family);
    let opening_topics = decision_style_openers(&poi.psychological.decision_style);
    let avoid_topics = compute_avoid_topics(&poi.psychological);
    let inferred_channel = infer_best_channel(&poi.psychological.change_appetite);
    let best_channel = if is_valid_best_channel(&inferred_channel) {
        inferred_channel
    } else {
        "trade_show_referral".to_string()
    };
    let best_timing = infer_best_timing(poi);
    let recommended_proof_pack = proof_pack(&poi.psychological.preferred_proof);

    EngagementProfile {
        what_they_want_to_hear,
        opening_topics,
        avoid_topics,
        best_channel,
        best_timing,
        recommended_proof_pack,
    }
}

fn is_valid_best_channel(channel: &str) -> bool {
    matches!(
        channel,
        "direct_outreach"
            | "trade_show_referral"
            | "referral_trusted_partner"
            | "existing_relationship_only"
    )
}

/// Get role-specific talking points.
fn role_talking_points(role: &RoleFamily) -> Vec<String> {
    match role {
        RoleFamily::Procurement => vec![
            "Total cost of ownership breakdown".to_string(),
            "Lead time certainty with SLA".to_string(),
            "Risk removal: dual source, buffer stock".to_string(),
            "Compliance readiness (IATF/ISO/RoHS)".to_string(),
        ],
        RoleFamily::SupplierQuality => vec![
            "PPM performance data".to_string(),
            "Control plan & process capability (Cpk)".to_string(),
            "Traceability system overview".to_string(),
            "Audit readiness package".to_string(),
            "8D/corrective action responsiveness".to_string(),
        ],
        RoleFamily::Engineering => vec![
            "DFM/DFT collaboration process".to_string(),
            "Fast iteration on prototypes".to_string(),
            "Test strategy (ICT/AOI/functional)".to_string(),
            "BOM optimization capability".to_string(),
        ],
        RoleFamily::Operations => vec![
            "Line stability and changeover discipline".to_string(),
            "Escalation path clarity".to_string(),
            "OTD performance metrics".to_string(),
            "Capacity flexibility (surge/ramp)".to_string(),
        ],
        RoleFamily::Security => vec![
            "DMARC/SPF/DKIM posture".to_string(),
            "Vendor access control policy".to_string(),
            "Incident response and transparency".to_string(),
            "Third-party risk management".to_string(),
        ],
        RoleFamily::Executive => vec![
            "Strategic partnership value".to_string(),
            "Growth capacity and roadmap".to_string(),
            "Regional advantage (nearshore/compliance)".to_string(),
        ],
        RoleFamily::Government | RoleFamily::FreeZoneAuthority => vec![
            "Job creation and investment plans".to_string(),
            "Technology transfer potential".to_string(),
            "Export growth contribution".to_string(),
            "Compliance with local content requirements".to_string(),
        ],
        _ => vec!["Reliability and competence".to_string()],
    }
}

/// Get opening topics based on decision style.
fn decision_style_openers(style: &DecisionStyle) -> Vec<String> {
    match style {
        DecisionStyle::CostFirst => vec![
            "TCO analysis".to_string(),
            "Cost optimization track record".to_string(),
        ],
        DecisionStyle::QualityFirst => vec![
            "Quality metrics".to_string(),
            "Zero-defect philosophy".to_string(),
        ],
        DecisionStyle::SpeedFirst => vec![
            "Rapid proto capability".to_string(),
            "NPI speed".to_string(),
        ],
        DecisionStyle::RiskFirst => vec![
            "Supply chain resilience".to_string(),
            "Dual-source strategy".to_string(),
        ],
        DecisionStyle::ComplianceFirst => vec![
            "Certification portfolio".to_string(),
            "Audit history".to_string(),
        ],
        DecisionStyle::BalancedAnalytical => vec![
            "Data-driven partnership".to_string(),
            "Balanced scorecard".to_string(),
        ],
    }
}

/// Determine topics to avoid.
fn compute_avoid_topics(psych: &PsychProfile) -> Vec<String> {
    let mut avoid = Vec::new();
    if psych.pain_index > 0.5 {
        avoid.push("Don't remind them of recent failures".to_string());
    }
    if psych.risk_tolerance < 0.3 {
        avoid.push("Avoid discussing disruptive changes".to_string());
    }
    avoid
}

/// Infer best contact channel from change appetite.
fn infer_best_channel(appetite: &ChangeAppetite) -> String {
    match appetite {
        ChangeAppetite::EarlyAdopter => "direct_outreach".to_string(),
        ChangeAppetite::Pragmatist => "trade_show_referral".to_string(),
        ChangeAppetite::Conservative => "referral_trusted_partner".to_string(),
        ChangeAppetite::Laggard => "existing_relationship_only".to_string(),
    }
}

/// Infer best timing for engagement.
fn infer_best_timing(poi: &PoiProfile) -> String {
    if poi.psychological.pain_index > 0.6 {
        return "immediately_pain_driven".to_string();
    }
    match &poi.role_family {
        RoleFamily::Procurement => "budget_cycle_q4_q1".to_string(),
        RoleFamily::SupplierQuality => "pre_audit_season".to_string(),
        RoleFamily::Engineering => "npi_phase_early".to_string(),
        _ => "anytime_with_trigger".to_string(),
    }
}

/// Build recommended proof pack from preferences.
fn proof_pack(preferred: &[ProofType]) -> Vec<String> {
    preferred
        .iter()
        .map(|p| match p {
            ProofType::KpiMetrics => "PPM/OTD/yield dashboard snapshot".to_string(),
            ProofType::Certifications => "Certificate portfolio (IATF/AS9100/ISO13485)".to_string(),
            ProofType::CaseStudies => "Relevant customer case studies".to_string(),
            ProofType::AuditReadiness => "Pre-audit self-assessment results".to_string(),
            ProofType::TechDemos => "Equipment list + process capability demo video".to_string(),
            ProofType::CostTransparency => "Should-cost model breakdown".to_string(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_poi(role: RoleFamily, decision: DecisionStyle, pain: f64) -> PoiProfile {
        PoiProfile {
            person_id: "poi_test".to_string(),
            name: "Test Person".to_string(),
            name_variants: vec![],
            org: "Test Corp".to_string(),
            org_id: None,
            current_role: "VP".to_string(),
            role_family: role,
            region: "TN".to_string(),
            country_code: "TN".to_string(),
            public_bio: String::new(),
            public_email: None,
            artifacts: vec![],
            priority_vector: PriorityVector::zero(),
            psychological: PsychProfile {
                decision_style: decision,
                change_appetite: ChangeAppetite::Pragmatist,
                pain_index: pain,
                preferred_proof: vec![ProofType::KpiMetrics, ProofType::Certifications],
                risk_tolerance: 0.5,
            },
            influence: InfluenceProfile {
                influence_score: 0.0,
                graph_centrality: 0.0,
                public_recurrence: 0.0,
                role_seniority_score: 0.0,
                network_size: 0,
            },
            engagement: None,
            role_history: vec![],
            last_updated_utc: 0,
            profile_completeness: 0.0,
        }
    }

    #[test]
    fn test_generate_procurement_engagement() {
        let poi = sample_poi(RoleFamily::Procurement, DecisionStyle::CostFirst, 0.3);
        let eng = generate_engagement_profile(&poi);
        assert!(eng.what_they_want_to_hear.iter().any(|t| t.contains("cost")));
        assert!(eng.opening_topics.iter().any(|t| t.contains("TCO")));
        assert_eq!(eng.best_channel, "trade_show_referral");
        assert_eq!(eng.best_timing, "budget_cycle_q4_q1");
    }

    #[test]
    fn test_generate_engineering_engagement() {
        let poi = sample_poi(RoleFamily::Engineering, DecisionStyle::SpeedFirst, 0.2);
        let eng = generate_engagement_profile(&poi);
        assert!(eng.what_they_want_to_hear.iter().any(|t| t.contains("DFM")));
        assert_eq!(eng.best_timing, "npi_phase_early");
    }

    #[test]
    fn test_high_pain_timing() {
        let poi = sample_poi(RoleFamily::Operations, DecisionStyle::BalancedAnalytical, 0.8);
        let eng = generate_engagement_profile(&poi);
        assert_eq!(eng.best_timing, "immediately_pain_driven");
        assert!(eng.avoid_topics.iter().any(|t| t.contains("failures")));
    }

    #[test]
    fn test_proof_pack() {
        let pack = proof_pack(&[ProofType::KpiMetrics, ProofType::CaseStudies]);
        assert_eq!(pack.len(), 2);
        assert!(pack[0].contains("PPM"));
        assert!(pack[1].contains("case studies"));
    }

    #[test]
    fn test_proof_pack_empty_list() {
        let pack = proof_pack(&[]);
        assert!(pack.is_empty());
    }

    #[test]
    fn test_best_channel_values_are_valid() {
        assert!(is_valid_best_channel("direct_outreach"));
        assert!(is_valid_best_channel("trade_show_referral"));
        assert!(is_valid_best_channel("referral_trusted_partner"));
        assert!(is_valid_best_channel("existing_relationship_only"));
        assert!(!is_valid_best_channel("email_blast"));
    }

    #[test]
    fn test_government_talking_points() {
        let poi = sample_poi(RoleFamily::Government, DecisionStyle::BalancedAnalytical, 0.1);
        let eng = generate_engagement_profile(&poi);
        assert!(eng.what_they_want_to_hear.iter().any(|t| t.contains("Job creation")));
    }

    #[test]
    fn test_security_role() {
        let poi = sample_poi(RoleFamily::Security, DecisionStyle::RiskFirst, 0.4);
        let eng = generate_engagement_profile(&poi);
        assert!(eng.what_they_want_to_hear.iter().any(|t| t.contains("DMARC")));
    }

    #[test]
    fn test_low_risk_tolerance_avoid() {
        let mut poi = sample_poi(RoleFamily::Executive, DecisionStyle::BalancedAnalytical, 0.3);
        poi.psychological.risk_tolerance = 0.2;
        let eng = generate_engagement_profile(&poi);
        assert!(eng.avoid_topics.iter().any(|t| t.contains("disruptive")));
    }
}
