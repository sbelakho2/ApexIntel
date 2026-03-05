//! POI profile model and supporting types.

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

// Re-export the canonical RoleFamily from core — no longer defined locally.
pub use apex_core::entities::RoleFamily;

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

/// Maximum allowed name variants to prevent unbounded growth (B116).
pub const MAX_NAME_VARIANTS: usize = 50;

/// Maximum network size to prevent unreasonable values (B128).
pub const MAX_NETWORK_SIZE: usize = 10_000;

/// Maximum artifacts retained per POI to avoid unbounded profile growth.
pub const MAX_PROFILE_ARTIFACTS: usize = 2_000;

static PUBLIC_EMAIL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}$").unwrap()
});

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

    /// Validate that dimension values are in [0,1] and sum is within expected range (B111).
    pub fn validate(&self) -> Vec<String> {
        let mut issues = Vec::new();
        let fields = [
            ("cost", self.cost), ("quality", self.quality), ("speed", self.speed),
            ("resilience", self.resilience), ("compliance", self.compliance), ("security", self.security),
        ];
        for (name, val) in &fields {
            if val.is_nan() || val.is_infinite() {
                issues.push(format!("{} is NaN/Inf", name));
            } else if *val < 0.0 || *val > 1.0 {
                issues.push(format!("{} out of [0,1]: {}", name, val));
            }
        }
        let sum: f64 = fields.iter().map(|(_, v)| v).filter(|v| v.is_finite()).sum();
        if sum > 1.5 {
            issues.push(format!("dimension sum {:.2} > 1.5", sum));
        }
        if self.confidence.is_nan() || self.confidence.is_infinite() {
            issues.push("confidence is NaN/Inf".to_string());
        } else if !(0.0..=1.0).contains(&self.confidence) {
            issues.push(format!("confidence out of [0,1]: {}", self.confidence));
        }
        issues
    }

    /// Return the dominant priority dimension.
    ///
    /// NaN-safe: NaN values are skipped so they cannot silently "win"
    /// due to IEEE 754 comparison semantics.
    pub fn dominant(&self) -> &'static str {
        let pairs = [
            ("cost", self.cost),
            ("quality", self.quality),
            ("speed", self.speed),
            ("resilience", self.resilience),
            ("compliance", self.compliance),
            ("security", self.security),
        ];
        if pairs.iter().all(|(_, value)| !value.is_finite()) {
            return "unknown";
        }
        let mut best = pairs[0];
        for &p in &pairs[1..] {
            if p.1.is_nan() {
                continue;
            }
            if best.1.is_nan() || p.1 > best.1 {
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

impl InfluenceProfile {
    /// Clamp network_size to MAX_NETWORK_SIZE (B128).
    pub fn clamp_network_size(&mut self) {
        if self.network_size > MAX_NETWORK_SIZE {
            self.network_size = MAX_NETWORK_SIZE;
        }
    }
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
    /// Returns normalized org_id: trimmed non-empty string, otherwise None.
    pub fn normalized_org_id(&self) -> Option<&str> {
        self.org_id.as_deref().map(str::trim).filter(|id| !id.is_empty())
    }

    /// Compute profile completeness (0-1) based on filled fields.
    /// Accounts for org_id and region presence consistently (B113).
    pub fn compute_completeness(&self) -> f64 {
        let mut filled = 0.0;
        let total = 12.0;

        if !self.name.is_empty() { filled += 1.0; }
        if !self.org.is_empty() { filled += 1.0; }
        if self.normalized_org_id().is_some() { filled += 1.0; } // B113/B402
        if !self.current_role.is_empty() { filled += 1.0; }
        if !self.public_bio.is_empty() { filled += 1.0; }
        if self.public_email.is_some() { filled += 1.0; }
        if !self.artifacts.is_empty() { filled += 1.0; }
        if !self.role_history.is_empty() { filled += 1.0; }
        if !self.region.is_empty() { filled += 1.0; } // B113
        if !self.country_code.is_empty() { filled += 1.0; } // B113
        if self.priority_vector.confidence > 0.0 { filled += 1.0; }
        if self.influence.influence_score > 0.0 { filled += 1.0; }

        let raw: f64 = filled / total;
        raw.clamp(0.0, 1.0) // B129: prevent exceeding 1.0
    }

    /// Truncate name_variants to MAX_NAME_VARIANTS (B116).
    pub fn clamp_name_variants(&mut self) {
        if self.name_variants.len() > MAX_NAME_VARIANTS {
            self.name_variants.truncate(MAX_NAME_VARIANTS);
        }
    }

    /// Truncate artifacts to a strict cap to prevent huge profiles.
    pub fn clamp_artifacts(&mut self) {
        if self.artifacts.len() > MAX_PROFILE_ARTIFACTS {
            self.artifacts.truncate(MAX_PROFILE_ARTIFACTS);
        }
    }

    /// Validate optional public email format when present.
    pub fn has_valid_public_email(&self) -> bool {
        match &self.public_email {
            None => true,
            Some(email) => {
                let normalized = email.trim();
                !normalized.is_empty() && PUBLIC_EMAIL_RE.is_match(normalized)
            }
        }
    }

    /// Validate role_history timestamps are in ascending order (B117).
    pub fn validate_role_history_order(&self) -> bool {
        self.role_history.windows(2).all(|pair| pair[0].start_ts <= pair[1].start_ts)
    }

    /// Detect identical consecutive role history entries (B118).
    pub fn has_consecutive_duplicates(&self) -> bool {
        self.role_history.windows(2).any(|pair| {
            pair[0].org == pair[1].org && pair[0].title == pair[1].title
        })
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

    #[test]
    fn test_role_family_canonical_labels_consistent_casing() {
        assert_eq!(RoleFamily::Procurement.canonical_label(), "procurement");
        assert_eq!(RoleFamily::SupplierQuality.canonical_label(), "supplier_quality");
        assert_eq!(RoleFamily::FreeZoneAuthority.canonical_label(), "free_zone_authority");
        assert_eq!(RoleFamily::Other("MiXeD Case".to_string()).canonical_label(), "mixed_case");
    }

    #[test]
    fn test_psych_profile_rejects_invalid_proof_type() {
        let bad = serde_json::json!({
            "decision_style": "BalancedAnalytical",
            "change_appetite": "Pragmatist",
            "pain_index": 0.3,
            "preferred_proof": ["NotARealProofType"],
            "risk_tolerance": 0.5
        });
        let parsed: Result<PsychProfile, _> = serde_json::from_value(bad);
        assert!(parsed.is_err());
    }

    // B111: PriorityVector validation
    #[test]
    fn test_priority_vector_validate() {
        let pv = PriorityVector {
            cost: 0.3, quality: 0.3, speed: 0.1, resilience: 0.1,
            compliance: 0.1, security: 0.1, confidence: 0.8,
        };
        assert!(pv.validate().is_empty());
    }

    #[test]
    fn test_priority_vector_validate_nan() {
        let pv = PriorityVector {
            cost: f64::NAN, quality: 0.3, speed: 0.1, resilience: 0.1,
            compliance: 0.1, security: 0.1, confidence: 0.8,
        };
        assert!(!pv.validate().is_empty());
    }

    #[test]
    fn test_priority_vector_validate_confidence_out_of_range() {
        let pv = PriorityVector {
            cost: 0.2,
            quality: 0.2,
            speed: 0.2,
            resilience: 0.2,
            compliance: 0.2,
            security: 0.2,
            confidence: 1.2,
        };
        let issues = pv.validate();
        assert!(issues.iter().any(|m| m.contains("confidence")));
    }

    // B115: dominant with NaN values
    #[test]
    fn test_dominant_all_nan() {
        let pv = PriorityVector {
            cost: f64::NAN, quality: f64::NAN, speed: f64::NAN,
            resilience: f64::NAN, compliance: f64::NAN, security: f64::NAN,
            confidence: 0.0,
        };
        // Should not panic, returns some valid string
        let _ = pv.dominant();
    }

    // B116: Clamp name variants
    #[test]
    fn test_clamp_name_variants() {
        let mut p = sample_profile();
        p.name_variants = (0..100).map(|i| format!("variant_{}", i)).collect();
        p.clamp_name_variants();
        assert!(p.name_variants.len() <= MAX_NAME_VARIANTS);
    }

    // B117: Role history timestamp ordering
    #[test]
    fn test_validate_role_history_order() {
        let p = sample_profile();
        assert!(p.validate_role_history_order());
    }

    // B118: Consecutive duplicate detection
    #[test]
    fn test_has_consecutive_duplicates() {
        let mut p = sample_profile();
        assert!(!p.has_consecutive_duplicates());
        p.role_history.push(p.role_history.last().unwrap().clone());
        assert!(p.has_consecutive_duplicates());
    }

    // B129: Completeness never exceeds 1.0
    #[test]
    fn test_completeness_capped() {
        let p = sample_profile();
        assert!(p.compute_completeness() <= 1.0);
    }

    #[test]
    fn test_org_id_empty_and_missing_treated_consistently() {
        let mut missing = sample_profile();
        missing.org_id = None;

        let mut empty = sample_profile();
        empty.org_id = Some("   ".to_string());

        assert_eq!(missing.normalized_org_id(), None);
        assert_eq!(empty.normalized_org_id(), None);
        assert_eq!(missing.compute_completeness(), empty.compute_completeness());
    }

    #[test]
    fn test_clamp_artifacts_caps_large_profiles() {
        let mut p = sample_profile();
        p.artifacts = (0..(MAX_PROFILE_ARTIFACTS + 50))
            .map(|i| PoiArtifact {
                artifact_type: "article".to_string(),
                title: format!("A{i}"),
                content_summary: "x".to_string(),
                source_url: None,
                ts_utc: 0,
            })
            .collect();
        p.clamp_artifacts();
        assert_eq!(p.artifacts.len(), MAX_PROFILE_ARTIFACTS);
    }

    #[test]
    fn test_has_valid_public_email() {
        let mut p = sample_profile();
        p.public_email = Some("valid.email+tag@example.com".to_string());
        assert!(p.has_valid_public_email());

        p.public_email = Some("not-an-email".to_string());
        assert!(!p.has_valid_public_email());
    }
}
