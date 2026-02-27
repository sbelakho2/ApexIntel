//! Dossiers route — request/response types for company and person dossier endpoints.

use apex_core::validation::validate_uuid;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ────────────────────────────────────────────
// Response types
// ────────────────────────────────────────────

/// Company dossier — comprehensive profile for analyst review.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompanyDossier {
    pub company_id: String,
    pub company_name: String,
    pub generated_at: DateTime<Utc>,
    pub sections: Vec<DossierSection>,
    pub risk_assessment: RiskAssessment,
    pub source_count: u32,
    pub confidence_score: f64,
}

/// Person dossier — comprehensive POI profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonDossier {
    pub person_id: String,
    pub person_name: String,
    pub generated_at: DateTime<Utc>,
    pub sections: Vec<DossierSection>,
    pub engagement_assessment: EngagementAssessment,
    pub source_count: u32,
    pub confidence_score: f64,
}

/// A section within a dossier.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DossierSection {
    pub title: String,
    pub content: String,
    pub section_type: DossierSectionType,
    pub sources: Vec<DossierSource>,
    pub confidence: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum DossierSectionType {
    Overview,
    Capabilities,
    Financial,
    Personnel,
    Competitive,
    Risk,
    Timeline,
    Network,
    Engagement,
    Custom,
}

impl DossierSectionType {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Overview => "overview",
            Self::Capabilities => "capabilities",
            Self::Financial => "financial",
            Self::Personnel => "personnel",
            Self::Competitive => "competitive",
            Self::Risk => "risk",
            Self::Timeline => "timeline",
            Self::Network => "network",
            Self::Engagement => "engagement",
            Self::Custom => "custom",
        }
    }

    pub fn company_defaults() -> Vec<Self> {
        vec![
            Self::Overview,
            Self::Capabilities,
            Self::Financial,
            Self::Personnel,
            Self::Competitive,
            Self::Risk,
            Self::Timeline,
        ]
    }

    pub fn person_defaults() -> Vec<Self> {
        vec![
            Self::Overview,
            Self::Network,
            Self::Timeline,
            Self::Engagement,
            Self::Risk,
        ]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DossierSource {
    pub url: String,
    pub title: Option<String>,
    pub crawled_at: DateTime<Utc>,
    pub reliability: f64,
}

/// Risk assessment within a company dossier.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskAssessment {
    pub overall_risk: RiskLevel,
    pub supply_chain_risk: f64,
    pub competitive_risk: f64,
    pub regulatory_risk: f64,
    pub security_risk: f64,
    pub factors: Vec<RiskFactor>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

impl RiskLevel {
    pub fn from_score(score: f64) -> Self {
        if score >= 0.75 {
            Self::Critical
        } else if score >= 0.5 {
            Self::High
        } else if score >= 0.25 {
            Self::Medium
        } else {
            Self::Low
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Critical => "critical",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskFactor {
    pub category: String,
    pub description: String,
    pub severity: f64,
    pub mitigable: bool,
}

/// Engagement assessment within a person dossier.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngagementAssessment {
    pub readiness: f64,
    pub approach_strategy: String,
    pub key_interests: Vec<String>,
    pub barriers: Vec<String>,
    pub recommended_channels: Vec<String>,
}

// ────────────────────────────────────────────
// Logic
// ────────────────────────────────────────────

/// Compute an overall risk score from individual dimensions.
pub fn compute_overall_risk(
    supply_chain: f64,
    competitive: f64,
    regulatory: f64,
    security: f64,
) -> (f64, RiskLevel) {
    let weights = [0.30, 0.25, 0.20, 0.25];
    let values = [supply_chain, competitive, regulatory, security];
    let score: f64 = weights.iter().zip(values.iter()).map(|(w, v)| w * v).sum();
    let clamped = score.clamp(0.0, 1.0);
    let level = RiskLevel::from_score(clamped);
    (clamped, level)
}

/// Compute dossier completeness (fraction of sections that have content).
pub fn dossier_completeness(sections: &[DossierSection]) -> f64 {
    if sections.is_empty() {
        return 0.0;
    }
    let filled = sections
        .iter()
        .filter(|s| !s.content.trim().is_empty())
        .count();
    filled as f64 / sections.len() as f64
}

/// Compute average source reliability across a dossier.
pub fn avg_source_reliability(sections: &[DossierSection]) -> f64 {
    let all_sources: Vec<f64> = sections
        .iter()
        .flat_map(|s| s.sources.iter().map(|src| src.reliability))
        .collect();
    if all_sources.is_empty() {
        return 0.0;
    }
    all_sources.iter().sum::<f64>() / all_sources.len() as f64
}

/// Validate a dossier ID (company or person).
pub fn validate_dossier_id(id: &str) -> Result<uuid::Uuid, String> {
    validate_uuid(id, "dossier_id").map_err(|e| e.to_string())?;
    uuid::Uuid::parse_str(id.trim()).map_err(|_| format!("Invalid dossier ID: '{}'", id))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_section(section_type: DossierSectionType, content: &str, reliability: f64) -> DossierSection {
        DossierSection {
            title: format!("{} Section", section_type.label()),
            content: content.to_string(),
            section_type,
            sources: vec![DossierSource {
                url: "https://example.com".to_string(),
                title: Some("Test".to_string()),
                crawled_at: Utc::now(),
                reliability,
            }],
            confidence: 0.8,
        }
    }

    #[test]
    fn test_risk_level_from_score() {
        assert_eq!(RiskLevel::from_score(0.1), RiskLevel::Low);
        assert_eq!(RiskLevel::from_score(0.3), RiskLevel::Medium);
        assert_eq!(RiskLevel::from_score(0.6), RiskLevel::High);
        assert_eq!(RiskLevel::from_score(0.9), RiskLevel::Critical);
    }

    #[test]
    fn test_risk_level_boundaries() {
        assert_eq!(RiskLevel::from_score(0.25), RiskLevel::Medium);
        assert_eq!(RiskLevel::from_score(0.50), RiskLevel::High);
        assert_eq!(RiskLevel::from_score(0.75), RiskLevel::Critical);
    }

    #[test]
    fn test_compute_overall_risk() {
        let (score, level) = compute_overall_risk(0.8, 0.6, 0.3, 0.7);
        // 0.30*0.8 + 0.25*0.6 + 0.20*0.3 + 0.25*0.7 = 0.24 + 0.15 + 0.06 + 0.175 = 0.625
        assert!((score - 0.625).abs() < 0.001);
        assert_eq!(level, RiskLevel::High);
    }

    #[test]
    fn test_compute_overall_risk_low() {
        let (score, level) = compute_overall_risk(0.1, 0.1, 0.1, 0.1);
        assert!((score - 0.1).abs() < 0.01);
        assert_eq!(level, RiskLevel::Low);
    }

    #[test]
    fn test_dossier_completeness_all_filled() {
        let sections = vec![
            make_section(DossierSectionType::Overview, "Content here", 0.9),
            make_section(DossierSectionType::Risk, "Risk info", 0.8),
        ];
        assert!((dossier_completeness(&sections) - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_dossier_completeness_half_empty() {
        let sections = vec![
            make_section(DossierSectionType::Overview, "Content", 0.9),
            make_section(DossierSectionType::Risk, "", 0.8),
        ];
        assert!((dossier_completeness(&sections) - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_dossier_completeness_empty() {
        assert_eq!(dossier_completeness(&[]), 0.0);
    }

    #[test]
    fn test_avg_source_reliability() {
        let sections = vec![
            make_section(DossierSectionType::Overview, "A", 0.9),
            make_section(DossierSectionType::Risk, "B", 0.7),
        ];
        let avg = avg_source_reliability(&sections);
        assert!((avg - 0.8).abs() < 0.001);
    }

    #[test]
    fn test_avg_source_reliability_empty() {
        let sections: Vec<DossierSection> = vec![];
        assert_eq!(avg_source_reliability(&sections), 0.0);
    }

    #[test]
    fn test_validate_dossier_id() {
        let id = uuid::Uuid::new_v4().to_string();
        assert!(validate_dossier_id(&id).is_ok());
        assert!(validate_dossier_id("bad-id").is_err());
    }

    #[test]
    fn test_section_type_defaults() {
        let company = DossierSectionType::company_defaults();
        assert!(company.len() >= 5);
        assert!(company.contains(&DossierSectionType::Overview));
        assert!(company.contains(&DossierSectionType::Risk));

        let person = DossierSectionType::person_defaults();
        assert!(person.len() >= 4);
        assert!(person.contains(&DossierSectionType::Engagement));
    }

    #[test]
    fn test_company_dossier_serialization() {
        let dossier = CompanyDossier {
            company_id: "c-1".to_string(),
            company_name: "Test Corp".to_string(),
            generated_at: Utc::now(),
            sections: vec![make_section(DossierSectionType::Overview, "Test content", 0.9)],
            risk_assessment: RiskAssessment {
                overall_risk: RiskLevel::Medium,
                supply_chain_risk: 0.3,
                competitive_risk: 0.4,
                regulatory_risk: 0.2,
                security_risk: 0.1,
                factors: vec![RiskFactor {
                    category: "supply_chain".to_string(),
                    description: "Single source dependency".to_string(),
                    severity: 0.6,
                    mitigable: true,
                }],
            },
            source_count: 15,
            confidence_score: 0.85,
        };
        let json = serde_json::to_string(&dossier).unwrap();
        assert!(json.contains("Test Corp"));
        assert!(json.contains("Single source"));
    }

    #[test]
    fn test_person_dossier_serialization() {
        let dossier = PersonDossier {
            person_id: "p-1".to_string(),
            person_name: "Ahmed".to_string(),
            generated_at: Utc::now(),
            sections: vec![make_section(DossierSectionType::Overview, "Bio here", 0.8)],
            engagement_assessment: EngagementAssessment {
                readiness: 0.7,
                approach_strategy: "Professional introduction".to_string(),
                key_interests: vec!["EMS trends".to_string()],
                barriers: vec![],
                recommended_channels: vec!["LinkedIn".to_string()],
            },
            source_count: 8,
            confidence_score: 0.75,
        };
        let json = serde_json::to_string(&dossier).unwrap();
        assert!(json.contains("Ahmed"));
        assert!(json.contains("Professional introduction"));
    }

    #[test]
    fn test_risk_factor_serialization() {
        let factor = RiskFactor {
            category: "competitive".to_string(),
            description: "Market entry threat".to_string(),
            severity: 0.7,
            mitigable: false,
        };
        let json = serde_json::to_string(&factor).unwrap();
        let back: RiskFactor = serde_json::from_str(&json).unwrap();
        assert!((back.severity - 0.7).abs() < 0.001);
        assert!(!back.mitigable);
    }
}
