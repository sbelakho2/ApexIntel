//! POI profile model and supporting types.

use serde::{Deserialize, Serialize};

/// Role families for POI classification.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum RoleFamily {
    Procurement,
    SupplierQuality,
    Engineering,
    Operations,
    Security,
    Executive,
    Government,
    FreeZoneAuthority,
    PortLogistics,
    CertificationBody,
    IndustryAssociation,
    Distributor,
    Finance,
    Legal,
    Military,
    Intelligence,
    Other(String),
}

/// Decision style inferred from artifacts and behavior.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum DecisionStyle {
    CostFirst,
    QualityFirst,
    SpeedFirst,
    RiskFirst,
    ComplianceFirst,
    BalancedAnalytical,
}

/// Change appetite classification.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ChangeAppetite {
    EarlyAdopter,
    Pragmatist,
    Conservative,
    Laggard,
}

/// Proof type preferences.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ProofType {
    KpiMetrics,
    Certifications,
    CaseStudies,
    AuditReadiness,
    TechDemos,
    CostTransparency,
}

/// Priority vector representing what matters most to this POI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriorityVector {
    pub cost: f64,
    pub quality: f64,
    pub speed: f64,
    pub resilience: f64,
    pub compliance: f64,
    pub security: f64,
    pub confidence: f64,
}

impl PriorityVector {
    pub fn zero() -> Self {
        Self {
            cost: 0.0,
            quality: 0.0,
            speed: 0.0,
            resilience: 0.0,
            compliance: 0.0,
            security: 0.0,
            confidence: 0.0,
        }
    }

    /// Return the dominant priority dimension.
    pub fn dominant(&self) -> &'static str {
        let pairs = [
            ("cost", self.cost),
            ("quality", self.quality),
            ("speed", self.speed),
            ("resilience", self.resilience),
            ("compliance", self.compliance),
            ("security", self.security),
        ];
        let mut best = pairs[0];
        for &p in &pairs[1..] {
            if p.1 > best.1 {
                best = p;
            }
        }
        best.0
    }
}

/// Psychological profile for engagement optimization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PsychProfile {
    pub decision_style: DecisionStyle,
    pub change_appetite: ChangeAppetite,
    pub pain_index: f64,
    pub preferred_proof: Vec<ProofType>,
    pub risk_tolerance: f64,
}

impl PsychProfile {
    pub fn default_profile() -> Self {
        Self {
            decision_style: DecisionStyle::BalancedAnalytical,
            change_appetite: ChangeAppetite::Pragmatist,
            pain_index: 0.3,
            preferred_proof: vec![ProofType::KpiMetrics, ProofType::Certifications],
            risk_tolerance: 0.5,
        }
    }
}

/// Influence profile derived from graph analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InfluenceProfile {
    pub influence_score: f64,
    pub graph_centrality: f64,
    pub public_recurrence: f64,
    pub role_seniority_score: f64,
    pub network_size: usize,
}

/// Engagement strategy profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngagementProfile {
    pub what_they_want_to_hear: Vec<String>,
    pub opening_topics: Vec<String>,
    pub avoid_topics: Vec<String>,
    pub best_channel: String,
    pub best_timing: String,
    pub recommended_proof_pack: Vec<String>,
}

/// A public artifact associated with a POI (talk, patent, article, etc.).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoiArtifact {
    pub artifact_type: String,
    pub title: String,
    pub content_summary: String,
    pub source_url: Option<String>,
    pub ts_utc: i64,
}

/// Role history entry for tracking career moves.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoleHistoryEntry {
    pub org: String,
    pub title: String,
    pub role_family: RoleFamily,
    pub start_ts: i64,
    pub end_ts: Option<i64>,
}

/// Full POI profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoiProfile {
    pub person_id: String,
    pub name: String,
    pub name_variants: Vec<String>,
    pub org: String,
    pub org_id: Option<String>,
    pub current_role: String,
    pub role_family: RoleFamily,
    pub region: String,
    pub country_code: String,
    pub public_bio: String,
    pub public_email: Option<String>,
    pub artifacts: Vec<PoiArtifact>,
    pub priority_vector: PriorityVector,
    pub psychological: PsychProfile,
    pub influence: InfluenceProfile,
    pub engagement: Option<EngagementProfile>,
    pub role_history: Vec<RoleHistoryEntry>,
    pub last_updated_utc: i64,
    pub profile_completeness: f64,
}

impl PoiProfile {
    /// Compute profile completeness (0-1) based on filled fields.
    pub fn compute_completeness(&self) -> f64 {
        let mut filled = 0.0;
        let total = 10.0;

        if !self.name.is_empty() { filled += 1.0; }
        if !self.org.is_empty() { filled += 1.0; }
        if !self.current_role.is_empty() { filled += 1.0; }
        if !self.public_bio.is_empty() { filled += 1.0; }
        if self.public_email.is_some() { filled += 1.0; }
        if !self.artifacts.is_empty() { filled += 1.0; }
        if !self.role_history.is_empty() { filled += 1.0; }
        if !self.region.is_empty() { filled += 1.0; }
        if self.priority_vector.confidence > 0.0 { filled += 1.0; }
        if self.influence.influence_score > 0.0 { filled += 1.0; }

        filled / total
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_profile() -> PoiProfile {
        PoiProfile {
            person_id: "poi_001".to_string(),
            name: "Ahmed Ben Ali".to_string(),
            name_variants: vec!["أحمد بن علي".to_string()],
            org: "Foxconn Tunisia".to_string(),
            org_id: Some("org_fox_tn".to_string()),
            current_role: "VP Procurement".to_string(),
            role_family: RoleFamily::Procurement,
            region: "TN".to_string(),
            country_code: "TN".to_string(),
            public_bio: "20 years in EMS procurement".to_string(),
            public_email: Some("ahmed@foxconn.tn".to_string()),
            artifacts: vec![PoiArtifact {
                artifact_type: "conference_talk".to_string(),
                title: "Cost optimization in EMS".to_string(),
                content_summary: "Discussed cost reduction strategies".to_string(),
                source_url: Some("https://example.com/talk".to_string()),
                ts_utc: 1700000000,
            }],
            priority_vector: PriorityVector {
                cost: 0.4,
                quality: 0.2,
                speed: 0.1,
                resilience: 0.1,
                compliance: 0.1,
                security: 0.1,
                confidence: 0.5,
            },
            psychological: PsychProfile::default_profile(),
            influence: InfluenceProfile {
                influence_score: 72.0,
                graph_centrality: 60.0,
                public_recurrence: 45.0,
                role_seniority_score: 85.0,
                network_size: 15,
            },
            engagement: None,
            role_history: vec![RoleHistoryEntry {
                org: "Foxconn Tunisia".to_string(),
                title: "VP Procurement".to_string(),
                role_family: RoleFamily::Procurement,
                start_ts: 1600000000,
                end_ts: None,
            }],
            last_updated_utc: 1700000000,
            profile_completeness: 0.0,
        }
    }

    #[test]
    fn test_priority_vector_dominant() {
        let pv = PriorityVector {
            cost: 0.4,
            quality: 0.3,
            speed: 0.1,
            resilience: 0.1,
            compliance: 0.05,
            security: 0.05,
            confidence: 0.8,
        };
        assert_eq!(pv.dominant(), "cost");
    }

    #[test]
    fn test_priority_vector_zero() {
        let pv = PriorityVector::zero();
        // All zero, first element wins
        assert_eq!(pv.dominant(), "cost");
    }

    #[test]
    fn test_profile_completeness_full() {
        let p = sample_profile();
        let c = p.compute_completeness();
        assert!(c > 0.8, "Well-filled profile should have high completeness, got {}", c);
    }

    #[test]
    fn test_profile_completeness_minimal() {
        let p = PoiProfile {
            person_id: "poi_empty".to_string(),
            name: "Unknown".to_string(),
            name_variants: vec![],
            org: String::new(),
            org_id: None,
            current_role: String::new(),
            role_family: RoleFamily::Other("Unknown".to_string()),
            region: String::new(),
            country_code: String::new(),
            public_bio: String::new(),
            public_email: None,
            artifacts: vec![],
            priority_vector: PriorityVector::zero(),
            psychological: PsychProfile::default_profile(),
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
        };
        let c = p.compute_completeness();
        assert!(c < 0.3, "Empty profile should have low completeness, got {}", c);
    }

    #[test]
    fn test_psych_profile_default() {
        let p = PsychProfile::default_profile();
        assert_eq!(p.decision_style, DecisionStyle::BalancedAnalytical);
        assert_eq!(p.change_appetite, ChangeAppetite::Pragmatist);
    }

    #[test]
    fn test_role_family_serialization() {
        let rf = RoleFamily::Procurement;
        let json = serde_json::to_string(&rf).unwrap();
        assert!(json.contains("Procurement"));
        let back: RoleFamily = serde_json::from_str(&json).unwrap();
        assert_eq!(back, rf);
    }
}
