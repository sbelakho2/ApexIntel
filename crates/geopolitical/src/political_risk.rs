//! # Political Risk Assessment Module
//!
//! Provides comprehensive political risk intelligence including:
//! - Country stability scoring
//! - Policy change detection
//! - Conflict zone monitoring
//! - Infrastructure risk analysis

use crate::models::{
    Confidence, CountryCode, GeopoliticalConfig, IntelligenceAlert,
    IntelligenceSource, RiskScore, Severity, TimeSeriesPoint, TrendDirection,
};
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Country stability score
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StabilityScore {
    pub country: CountryCode,
    pub overall_score: f32,
    pub economic_stability: f32,
    pub political_stability: f32,
    pub social_cohesion: f32,
    pub governance: f32,
    pub rule_of_law: f32,
    pub corruption_index: f32,
    pub conflict_risk: f32,
    pub confidence: Confidence,
    pub trend: TrendDirection,
    pub historical_scores: Vec<TimeSeriesPoint<f32>>,
    pub last_updated: DateTime<Utc>,
    pub next_update: DateTime<Utc>,
    pub methodology_version: String,
    pub data_sources: Vec<IntelligenceSource>,
}

impl StabilityScore {
    /// Create a new stability score
    pub fn new(country: CountryCode) -> Self {
        Self {
            country,
            overall_score: 0.5,
            economic_stability: 0.5,
            political_stability: 0.5,
            social_cohesion: 0.5,
            governance: 0.5,
            rule_of_law: 0.5,
            corruption_index: 0.5,
            conflict_risk: 0.0,
            confidence: Confidence::Medium,
            trend: TrendDirection::Stable,
            historical_scores: Vec::new(),
            last_updated: Utc::now(),
            next_update: Utc::now() + chrono::Duration::hours(24),
            methodology_version: "1.0".to_string(),
            data_sources: Vec::new(),
        }
    }

    /// Calculate overall score from components
    pub fn calculate_overall(&mut self) {
        self.overall_score =
            self.economic_stability * 0.2 +
            self.political_stability * 0.25 +
            self.social_cohesion * 0.15 +
            self.governance * 0.15 +
            self.rule_of_law * 0.15 +
            (1.0 - self.corruption_index) * 0.1;

        // Update confidence based on data quality
        self.confidence = Confidence::from_value(self.overall_score);
    }

    /// Get risk level description
    pub fn risk_level(&self) -> &'static str {
        match self.overall_score {
            s if s >= 0.8 => "Very Stable",
            s if s >= 0.6 => "Stable",
            s if s >= 0.4 => "Moderate",
            s if s >= 0.2 => "Unstable",
            _ => "Very Unstable",
        }
    }

    /// Check if country is safe for investment
    pub fn is_investment_grade(&self) -> bool {
        self.overall_score >= 0.5 && self.conflict_risk < 0.3
    }

    /// Add historical data point
    pub fn add_historical(&mut self, score: f32, timestamp: DateTime<Utc>) {
        self.historical_scores.push(TimeSeriesPoint {
            timestamp,
            value: score,
        });
        // Keep only last 12 months
        let cutoff = Utc::now() - chrono::Duration::days(365);
        self.historical_scores.retain(|p| p.timestamp > cutoff);
    }
}

/// Conflict zone information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictZone {
    pub id: Uuid,
    pub name: String,
    pub location: GeoLocation,
    pub conflict_type: ConflictType,
    pub intensity: ConflictIntensity,
    pub status: ConflictStatus,
    pub parties_involved: Vec<ConflictParty>,
    pub start_date: DateTime<Utc>,
    pub last_escalation: Option<DateTime<Utc>>,
    pub affected_regions: Vec<String>,
    pub civilian_impact: CivilianImpact,
    pub humanitarian_situation: HumanitarianSituation,
    pub economic_impact: EconomicImpact,
    pub risk_score: f32,
    pub trajectory: ConflictTrajectory,
    pub sources: Vec<IntelligenceSource>,
    pub last_verified: DateTime<Utc>,
}

impl ConflictZone {
    /// Create a new conflict zone
    pub fn new(name: String, conflict_type: ConflictType, location: GeoLocation) -> Self {
        Self {
            id: Uuid::new_v4(),
            name,
            location,
            conflict_type,
            intensity: ConflictIntensity::Low,
            status: ConflictStatus::Active,
            parties_involved: Vec::new(),
            start_date: Utc::now(),
            last_escalation: None,
            affected_regions: Vec::new(),
            civilian_impact: CivilianImpact::default(),
            humanitarian_situation: HumanitarianSituation::default(),
            economic_impact: EconomicImpact::default(),
            risk_score: 0.5,
            trajectory: ConflictTrajectory::Unchanged,
            sources: Vec::new(),
            last_verified: Utc::now(),
        }
    }

    /// Add party to conflict
    pub fn add_party(&mut self, party: ConflictParty) {
        self.parties_involved.push(party);
    }

    /// Update intensity
    pub fn update_intensity(&mut self, new_intensity: ConflictIntensity) {
        if new_intensity > self.intensity {
            self.last_escalation = Some(Utc::now());
        }
        self.intensity = new_intensity;
    }

    /// Calculate current risk
    pub fn calculate_risk(&self) -> f32 {
        let base = self.intensity.weight() * 0.4;
        let humanitarian = self.humanitarian_situation.severity() * 0.3;
        let economic = self.economic_impact.impact_score() * 0.2;
        let trajectory = match self.trajectory {
            ConflictTrajectory::Escalating => 0.2,
            ConflictTrajectory::Unchanged => 0.1,
            ConflictTrajectory::DeEscalating => 0.0,
        };

        (base + humanitarian + economic + trajectory).min(1.0)
    }

    /// Check if conflict affects a country
    pub fn affects_country(&self, country: &CountryCode) -> bool {
        self.parties_involved.iter().any(|p| &p.country == country) ||
        self.location.countries.contains(country)
    }
}

/// Geographic location
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeoLocation {
    pub countries: Vec<CountryCode>,
    pub region: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub description: String,
}

impl GeoLocation {
    /// Create from countries
    pub fn from_countries(countries: Vec<CountryCode>) -> Self {
        Self {
            countries,
            region: None,
            latitude: None,
            longitude: None,
            description: String::new(),
        }
    }
}

/// Conflict types
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ConflictType {
    ArmedConflict,
    CivilWar,
    InterstateWar,
    Insurgency,
    Terrorism,
    Protest,
    EthnicConflict,
    ReligiousConflict,
    ResourceConflict,
    BorderDispute,
    CyberConflict,
    HybridWarfare,
}

impl ConflictType {
    pub fn description(&self) -> &'static str {
        match self {
            ConflictType::ArmedConflict => "Armed Conflict",
            ConflictType::CivilWar => "Civil War",
            ConflictType::InterstateWar => "Interstate War",
            ConflictType::Insurgency => "Insurgency",
            ConflictType::Terrorism => "Terrorism",
            ConflictType::Protest => "Mass Protest",
            ConflictType::EthnicConflict => "Ethnic Conflict",
            ConflictType::ReligiousConflict => "Religious Conflict",
            ConflictType::ResourceConflict => "Resource Conflict",
            ConflictType::BorderDispute => "Border Dispute",
            ConflictType::CyberConflict => "Cyber Conflict",
            ConflictType::HybridWarfare => "Hybrid Warfare",
        }
    }
}

/// Conflict intensity levels
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum ConflictIntensity {
    Negligible,
    Low,
    Medium,
    High,
    VeryHigh,
    Extreme,
}

impl ConflictIntensity {
    pub fn weight(&self) -> f32 {
        match self {
            ConflictIntensity::Negligible => 0.0,
            ConflictIntensity::Low => 0.2,
            ConflictIntensity::Medium => 0.4,
            ConflictIntensity::High => 0.6,
            ConflictIntensity::VeryHigh => 0.8,
            ConflictIntensity::Extreme => 1.0,
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            ConflictIntensity::Negligible => "Negligible",
            ConflictIntensity::Low => "Low",
            ConflictIntensity::Medium => "Medium",
            ConflictIntensity::High => "High",
            ConflictIntensity::VeryHigh => "Very High",
            ConflictIntensity::Extreme => "Extreme",
        }
    }
}

/// Conflict status
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ConflictStatus {
    Dormant,
    LowActivity,
    Active,
    Escalating,
    Ceasefire,
    PeaceProcess,
    Resolved,
}

impl ConflictStatus {
    pub fn description(&self) -> &'static str {
        match self {
            ConflictStatus::Dormant => "Dormant",
            ConflictStatus::LowActivity => "Low Activity",
            ConflictStatus::Active => "Active",
            ConflictStatus::Escalating => "Escalating",
            ConflictStatus::Ceasefire => "Ceasefire",
            ConflictStatus::PeaceProcess => "Peace Process",
            ConflictStatus::Resolved => "Resolved",
        }
    }
}

/// Conflict party
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictParty {
    pub name: String,
    pub country: CountryCode,
    pub party_type: ConflictPartyType,
    pub involvement_level: InvolvementLevel,
    pub casualties_estimate: Option<u32>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ConflictPartyType {
    Government,
    RebelGroup,
    Militia,
    ForeignForces,
    NonStateActor,
    MultinationalForce,
    PeacekeepingForce,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum InvolvementLevel {
    Direct,
    Indirect,
    Backing,
    Mediating,
}

/// Civilian impact assessment
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CivilianImpact {
    pub casualties_estimate: u32,
    pub displaced_estimate: u32,
    pub infrastructure_damage: DamageLevel,
    pub human_rights_concerns: Vec<String>,
}

impl Default for CivilianImpact {
    fn default() -> Self {
        Self {
            casualties_estimate: 0,
            displaced_estimate: 0,
            infrastructure_damage: DamageLevel::None,
            human_rights_concerns: Vec::new(),
        }
    }
}

impl CivilianImpact {
    pub fn severity(&self) -> f32 {
        let casualty_score = (self.casualties_estimate as f32 / 1000.0).min(1.0);
        let displaced_score = (self.displaced_estimate as f32 / 100000.0).min(1.0);
        let damage_score = self.infrastructure_damage.to_score();

        (casualty_score * 0.4 + displaced_score * 0.4 + damage_score * 0.2).min(1.0)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[derive(Default)]
pub enum DamageLevel {
    #[default]
    None,
    Minor,
    Moderate,
    Severe,
    Total,
}


impl DamageLevel {
    pub fn to_score(&self) -> f32 {
        match self {
            DamageLevel::None => 0.0,
            DamageLevel::Minor => 0.2,
            DamageLevel::Moderate => 0.4,
            DamageLevel::Severe => 0.7,
            DamageLevel::Total => 1.0,
        }
    }
}

/// Humanitarian situation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HumanitarianSituation {
    pub severity_score: f32,
    pub food_insecurity_level: FoodSecurityLevel,
    pub medical_access: MedicalAccessLevel,
    pub refugee_count: u32,
    pub internally_displaced: u32,
}

impl Default for HumanitarianSituation {
    fn default() -> Self {
        Self {
            severity_score: 0.0,
            food_insecurity_level: FoodSecurityLevel::Minimal,
            medical_access: MedicalAccessLevel::Normal,
            refugee_count: 0,
            internally_displaced: 0,
        }
    }
}

impl HumanitarianSituation {
    pub fn severity(&self) -> f32 {
        self.severity_score
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[derive(Default)]
pub enum FoodSecurityLevel {
    #[default]
    Minimal,
    Stressed,
    Crisis,
    Emergency,
    Catastrophe,
}


#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[derive(Default)]
pub enum MedicalAccessLevel {
    #[default]
    Normal,
    Limited,
    SeverelyLimited,
    NoAccess,
}


/// Economic impact
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EconomicImpact {
    pub gdp_impact_percent: f32,
    pub trade_disruption: TradeDisruptionLevel,
    pub supply_chain_affected: bool,
    pub infrastructure_cost_estimate: Option<f64>,
}

impl Default for EconomicImpact {
    fn default() -> Self {
        Self {
            gdp_impact_percent: 0.0,
            trade_disruption: TradeDisruptionLevel::None,
            supply_chain_affected: false,
            infrastructure_cost_estimate: None,
        }
    }
}

impl EconomicImpact {
    pub fn impact_score(&self) -> f32 {
        let gdp_score = (self.gdp_impact_percent / 10.0).min(1.0);
        let trade_score = self.trade_disruption.to_score();
        let supply_score = if self.supply_chain_affected { 0.3 } else { 0.0 };

        (gdp_score * 0.5 + trade_score * 0.3 + supply_score).min(1.0)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[derive(Default)]
pub enum TradeDisruptionLevel {
    #[default]
    None,
    Minor,
    Moderate,
    Severe,
    Complete,
}


impl TradeDisruptionLevel {
    pub fn to_score(&self) -> f32 {
        match self {
            TradeDisruptionLevel::None => 0.0,
            TradeDisruptionLevel::Minor => 0.2,
            TradeDisruptionLevel::Moderate => 0.4,
            TradeDisruptionLevel::Severe => 0.7,
            TradeDisruptionLevel::Complete => 1.0,
        }
    }
}

/// Conflict trajectory
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ConflictTrajectory {
    DeEscalating,
    Unchanged,
    Escalating,
}

/// Policy change detection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyChange {
    pub id: Uuid,
    pub country: CountryCode,
    pub policy_type: PolicyType,
    pub title: String,
    pub description: String,
    pub effective_date: DateTime<Utc>,
    pub announcement_date: DateTime<Utc>,
    pub previous_policy: Option<String>,
    pub new_policy: String,
    pub affected_sectors: Vec<String>,
    pub affected_regulations: Vec<String>,
    pub impact_assessment: PolicyImpact,
    pub sources: Vec<String>,
    pub verified: bool,
    pub last_updated: DateTime<Utc>,
}

impl PolicyChange {
    /// Create a new policy change
    pub fn new(
        country: CountryCode,
        policy_type: PolicyType,
        title: String,
        new_policy: String,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            country,
            policy_type,
            title,
            description: String::new(),
            effective_date: now,
            announcement_date: now,
            previous_policy: None,
            new_policy,
            affected_sectors: Vec::new(),
            affected_regulations: Vec::new(),
            impact_assessment: PolicyImpact::default(),
            sources: Vec::new(),
            verified: false,
            last_updated: now,
        }
    }

    /// Calculate impact score
    pub fn impact_score(&self) -> f32 {
        let sector_weight = (self.affected_sectors.len() as f32 / 10.0).min(1.0) * 0.4;
        let type_weight = self.policy_type.weight() * 0.3;
        let assessment_weight = self.impact_assessment.overall_impact * 0.3;

        (sector_weight + type_weight + assessment_weight).min(1.0)
    }
}

/// Policy types
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum PolicyType {
    Trade,
    Investment,
    Taxation,
    Labor,
    Environmental,
    Security,
    Immigration,
    Healthcare,
    Education,
    Technology,
    Financial,
    Energy,
    Agricultural,
    Industrial,
    Other,
}

impl PolicyType {
    pub fn weight(&self) -> f32 {
        match self {
            PolicyType::Trade => 0.8,
            PolicyType::Investment => 0.7,
            PolicyType::Security => 0.9,
            PolicyType::Technology => 0.7,
            PolicyType::Financial => 0.6,
            _ => 0.4,
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            PolicyType::Trade => "Trade Policy",
            PolicyType::Investment => "Investment Policy",
            PolicyType::Taxation => "Taxation Policy",
            PolicyType::Labor => "Labor Policy",
            PolicyType::Environmental => "Environmental Policy",
            PolicyType::Security => "Security Policy",
            PolicyType::Immigration => "Immigration Policy",
            PolicyType::Healthcare => "Healthcare Policy",
            PolicyType::Education => "Education Policy",
            PolicyType::Technology => "Technology Policy",
            PolicyType::Financial => "Financial Policy",
            PolicyType::Energy => "Energy Policy",
            PolicyType::Agricultural => "Agricultural Policy",
            PolicyType::Industrial => "Industrial Policy",
            PolicyType::Other => "Other Policy",
        }
    }
}

/// Policy impact assessment
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyImpact {
    pub overall_impact: f32,
    pub economic_impact: f32,
    pub political_impact: f32,
    pub social_impact: f32,
    pub regional_impact: f32,
    pub timeline: String,
    pub affected_stakeholders: Vec<String>,
}

impl Default for PolicyImpact {
    fn default() -> Self {
        Self {
            overall_impact: 0.5,
            economic_impact: 0.5,
            political_impact: 0.5,
            social_impact: 0.5,
            regional_impact: 0.5,
            timeline: String::new(),
            affected_stakeholders: Vec::new(),
        }
    }
}

/// Infrastructure risk
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InfrastructureRisk {
    pub id: Uuid,
    pub country: CountryCode,
    pub infrastructure_type: InfrastructureType,
    pub name: String,
    pub location: GeoLocation,
    pub risk_score: f32,
    pub vulnerabilities: Vec<InfrastructureVulnerability>,
    pub threats: Vec<InfrastructureThreat>,
    pub mitigation_factors: Vec<String>,
    pub criticality: CriticalityLevel,
    pub operational_status: OperationalStatus,
    pub last_assessment: DateTime<Utc>,
}

impl InfrastructureRisk {
    /// Create a new infrastructure risk
    pub fn new(
        country: CountryCode,
        infrastructure_type: InfrastructureType,
        name: String,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            country: country.clone(),
            infrastructure_type,
            name,
            location: GeoLocation::from_countries(vec![country]),
            risk_score: 0.3,
            vulnerabilities: Vec::new(),
            threats: Vec::new(),
            mitigation_factors: Vec::new(),
            criticality: CriticalityLevel::Medium,
            operational_status: OperationalStatus::Operational,
            last_assessment: Utc::now(),
        }
    }

    /// Calculate risk score
    pub fn calculate_risk(&self) -> f32 {
        let vuln_score: f32 = self.vulnerabilities.iter()
            .map(|v| v.severity)
            .sum::<f32>() / 10.0;

        let threat_score: f32 = self.threats.iter()
            .map(|t| t.probability * t.severity)
            .sum::<f32>() / 10.0;

        let criticality_score = self.criticality.to_score();

        let base = (vuln_score * 0.4 + threat_score * 0.4 + criticality_score * 0.2).min(1.0);

        // Reduce by mitigation factors
        let mitigation_reduction = (self.mitigation_factors.len() as f32 * 0.05).min(0.3);

        (base - mitigation_reduction).max(0.0)
    }
}

/// Infrastructure types
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum InfrastructureType {
    Transportation,
    Energy,
    Communications,
    Water,
    Healthcare,
    Financial,
    Government,
    Military,
    CriticalManufacturing,
    FoodAgriculture,
    Chemical,
    Nuclear,
    Defense,
    Space,
    Dams,
}

impl InfrastructureType {
    pub fn description(&self) -> &'static str {
        match self {
            InfrastructureType::Transportation => "Transportation",
            InfrastructureType::Energy => "Energy",
            InfrastructureType::Communications => "Communications",
            InfrastructureType::Water => "Water Systems",
            InfrastructureType::Healthcare => "Healthcare",
            InfrastructureType::Financial => "Financial Services",
            InfrastructureType::Government => "Government Facilities",
            InfrastructureType::Military => "Military",
            InfrastructureType::CriticalManufacturing => "Critical Manufacturing",
            InfrastructureType::FoodAgriculture => "Food & Agriculture",
            InfrastructureType::Chemical => "Chemical",
            InfrastructureType::Nuclear => "Nuclear",
            InfrastructureType::Defense => "Defense",
            InfrastructureType::Space => "Space",
            InfrastructureType::Dams => "Dams",
        }
    }
}

/// Infrastructure vulnerability
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InfrastructureVulnerability {
    pub vulnerability_type: String,
    pub severity: f32,
    pub description: String,
    pub exploitation_difficulty: ExploitationDifficulty,
}

/// Infrastructure threat
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InfrastructureThreat {
    pub threat_type: String,
    pub probability: f32,
    pub severity: f32,
    pub description: String,
    pub attribution: Option<String>,
}

/// Criticality levels
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum CriticalityLevel {
    Low,
    Medium,
    High,
    VeryHigh,
    Critical,
}

impl CriticalityLevel {
    pub fn to_score(&self) -> f32 {
        match self {
            CriticalityLevel::Low => 0.2,
            CriticalityLevel::Medium => 0.4,
            CriticalityLevel::High => 0.6,
            CriticalityLevel::VeryHigh => 0.8,
            CriticalityLevel::Critical => 1.0,
        }
    }
}

/// Operational status
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum OperationalStatus {
    Operational,
    Degraded,
    PartiallyDown,
    Down,
    UnderMaintenance,
    Unknown,
}

/// Exploitation difficulty
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ExploitationDifficulty {
    Trivial,
    Easy,
    Medium,
    Hard,
    VeryHard,
}

/// Risk assessment request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskAssessmentRequest {
    pub country: CountryCode,
    pub assessment_type: RiskAssessmentType,
    pub time_horizon: TimeHorizon,
    pub include_conflicts: bool,
    pub include_infrastructure: bool,
    pub include_policy: bool,
}

impl Default for RiskAssessmentRequest {
    fn default() -> Self {
        Self {
            country: CountryCode::new("US"),
            assessment_type: RiskAssessmentType::Comprehensive,
            time_horizon: TimeHorizon::OneYear,
            include_conflicts: true,
            include_infrastructure: true,
            include_policy: true,
        }
    }
}

/// Risk assessment types
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum RiskAssessmentType {
    Quick,
    Standard,
    Comprehensive,
    DeepDive,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum TimeHorizon {
    ThreeMonths,
    SixMonths,
    OneYear,
    ThreeYears,
    FiveYears,
}

/// Risk assessment result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskAssessmentResult {
    pub id: Uuid,
    pub country: CountryCode,
    pub stability_score: StabilityScore,
    pub conflict_exposure: Vec<ConflictZone>,
    pub infrastructure_risks: Vec<InfrastructureRisk>,
    pub policy_changes: Vec<PolicyChange>,
    pub overall_risk_score: RiskScore,
    pub recommendations: Vec<String>,
    pub generated_at: DateTime<Utc>,
    pub valid_until: DateTime<Utc>,
}

impl RiskAssessmentResult {
    /// Create from components
    pub fn new(country: CountryCode, stability_score: StabilityScore) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            country,
            stability_score,
            conflict_exposure: Vec::new(),
            infrastructure_risks: Vec::new(),
            policy_changes: Vec::new(),
            overall_risk_score: RiskScore::new(0.5, 0.5, 0.5, 0.5, 0.5),
            recommendations: Vec::new(),
            generated_at: now,
            valid_until: now + chrono::Duration::days(7),
        }
    }

    /// Calculate overall risk
    pub fn calculate_overall(&mut self) {
        let stability_risk = 1.0 - self.stability_score.overall_score;
        let conflict_risk = self.conflict_exposure.iter()
            .map(|c| c.risk_score)
            .fold(0.0f32, |max, s| max.max(s));

        let infra_risk = self.infrastructure_risks.iter()
            .map(|r| r.risk_score)
            .fold(0.0f32, |max, s| max.max(s));

        let policy_risk = self.policy_changes.iter()
            .map(|p| p.impact_score())
            .fold(0.0f32, |max, s| max.max(s));

        self.overall_risk_score = RiskScore::new(
            (stability_risk + conflict_risk + infra_risk + policy_risk) / 4.0,
            infra_risk,
            conflict_risk,
            stability_risk,
            policy_risk,
        );
    }
}

/// Political risk client
pub struct PoliticalRiskClient {
    _http_client: Client,
    _config: GeopoliticalConfig,
    cached_stability_scores: HashMap<String, StabilityScore>,
    active_conflicts: Vec<ConflictZone>,
    recent_policy_changes: Vec<PolicyChange>,
}

impl PoliticalRiskClient {
    /// Create a new political risk client
    pub fn new(http_client: Client, config: GeopoliticalConfig) -> Self {
        Self {
            _http_client: http_client,
            _config: config,
            cached_stability_scores: HashMap::new(),
            active_conflicts: Self::get_default_conflicts(),
            recent_policy_changes: Vec::new(),
        }
    }

    /// Get default conflict zones
    fn get_default_conflicts() -> Vec<ConflictZone> {
        vec![
            {
                let mut conflict = ConflictZone::new(
                    "Ukraine Conflict".to_string(),
                    ConflictType::ArmedConflict,
                    GeoLocation::from_countries(vec![CountryCode::new("UA"), CountryCode::new("RU")]),
                );
                conflict.intensity = ConflictIntensity::High;
                conflict.status = ConflictStatus::Active;
                conflict.trajectory = ConflictTrajectory::Unchanged;
                conflict.add_party(ConflictParty {
                    name: "Ukraine Armed Forces".to_string(),
                    country: CountryCode::new("UA"),
                    party_type: ConflictPartyType::Government,
                    involvement_level: InvolvementLevel::Direct,
                    casualties_estimate: Some(100000),
                });
                conflict.add_party(ConflictParty {
                    name: "Russian Armed Forces".to_string(),
                    country: CountryCode::new("RU"),
                    party_type: ConflictPartyType::ForeignForces,
                    involvement_level: InvolvementLevel::Direct,
                    casualties_estimate: Some(80000),
                });
                conflict.risk_score = 0.85;
                conflict
            },
            {
                let mut conflict = ConflictZone::new(
                    "Middle East Regional Conflict".to_string(),
                    ConflictType::ArmedConflict,
                    GeoLocation::from_countries(vec![CountryCode::new("IL"), CountryCode::new("IR")]),
                );
                conflict.intensity = ConflictIntensity::High;
                conflict.status = ConflictStatus::Escalating;
                conflict.trajectory = ConflictTrajectory::Escalating;
                conflict.risk_score = 0.75;
                conflict
            },
        ]
    }

    /// Get stability score for a country
    pub async fn get_stability_score(&mut self, country: &CountryCode) -> crate::Result<StabilityScore> {
        // Check cache first
        if let Some(score) = self.cached_stability_scores.get(&country.0) {
            // Check if score is still valid
            if score.next_update > Utc::now() {
                return Ok(score.clone());
            }
        }

        // Fetch new score (or use mock data)
        let score = self.fetch_stability_score(country).await?;
        
        // Cache the score
        self.cached_stability_scores.insert(country.0.clone(), score.clone());

        Ok(score)
    }

    /// Fetch stability score (mock implementation)
    async fn fetch_stability_score(&self, country: &CountryCode) -> crate::Result<StabilityScore> {
        // In production, this would fetch from World Bank, IMF, etc.
        let mut score = StabilityScore::new(country.clone());

        // Use country-specific mock data
        match country.0.as_str() {
            "US" => {
                score.overall_score = 0.78;
                score.economic_stability = 0.75;
                score.political_stability = 0.65;
                score.social_cohesion = 0.70;
                score.governance = 0.80;
                score.rule_of_law = 0.85;
                score.corruption_index = 0.20;
                score.conflict_risk = 0.15;
            },
            "DE" | "FR" | "GB" => {
                score.overall_score = 0.82;
                score.economic_stability = 0.85;
                score.political_stability = 0.80;
                score.social_cohesion = 0.75;
                score.governance = 0.88;
                score.rule_of_law = 0.90;
                score.corruption_index = 0.15;
                score.conflict_risk = 0.10;
            },
            "CN" => {
                score.overall_score = 0.65;
                score.economic_stability = 0.80;
                score.political_stability = 0.75;
                score.social_cohesion = 0.55;
                score.governance = 0.70;
                score.rule_of_law = 0.60;
                score.corruption_index = 0.35;
                score.conflict_risk = 0.25;
            },
            "RU" => {
                score.overall_score = 0.40;
                score.economic_stability = 0.35;
                score.political_stability = 0.30;
                score.social_cohesion = 0.45;
                score.governance = 0.35;
                score.rule_of_law = 0.30;
                score.corruption_index = 0.75;
                score.conflict_risk = 0.70;
            },
            "UA" => {
                score.overall_score = 0.20;
                score.economic_stability = 0.15;
                score.political_stability = 0.25;
                score.social_cohesion = 0.30;
                score.governance = 0.20;
                score.rule_of_law = 0.20;
                score.corruption_index = 0.80;
                score.conflict_risk = 0.90;
            },
            "ZA" | "NG" | "EG" => {
                score.overall_score = 0.45;
                score.economic_stability = 0.40;
                score.political_stability = 0.45;
                score.social_cohesion = 0.40;
                score.governance = 0.50;
                score.rule_of_law = 0.45;
                score.corruption_index = 0.60;
                score.conflict_risk = 0.50;
            },
            _ => {
                // Default moderate score
                score.overall_score = 0.55;
                score.economic_stability = 0.55;
                score.political_stability = 0.55;
                score.social_cohesion = 0.55;
                score.governance = 0.55;
                score.rule_of_law = 0.55;
                score.corruption_index = 0.50;
                score.conflict_risk = 0.30;
            }
        }

        score.calculate_overall();
        score.data_sources.push(IntelligenceSource::WorldBank);
        score.data_sources.push(IntelligenceSource::Imf);

        Ok(score)
    }

    /// Get active conflicts
    pub fn get_active_conflicts(&self) -> &[ConflictZone] {
        &self.active_conflicts
    }

    /// Get conflicts affecting a country
    pub fn get_conflicts_by_country(&self, country: &CountryCode) -> Vec<&ConflictZone> {
        self.active_conflicts.iter()
            .filter(|c| c.affects_country(country))
            .collect()
    }

    /// Get conflicts by region
    pub fn get_conflicts_by_region(&self, region: &str) -> Vec<&ConflictZone> {
        self.active_conflicts.iter()
            .filter(|c| c.location.region.as_ref().map(|r| r == region).unwrap_or(false))
            .collect()
    }

    /// Get conflicts by intensity
    pub fn get_conflicts_by_intensity(&self, min_intensity: ConflictIntensity) -> Vec<&ConflictZone> {
        self.active_conflicts.iter()
            .filter(|c| c.intensity >= min_intensity)
            .collect()
    }

    /// Get policy changes
    pub fn get_recent_policy_changes(&self) -> &[PolicyChange] {
        &self.recent_policy_changes
    }

    /// Get policy changes by country
    pub fn get_policy_changes_by_country(&self, country: &CountryCode) -> Vec<&PolicyChange> {
        self.recent_policy_changes.iter()
            .filter(|p| p.country == *country)
            .collect()
    }

    /// Add policy change
    pub fn add_policy_change(&mut self, change: PolicyChange) {
        self.recent_policy_changes.insert(0, change);
        // Keep only last 100 changes
        self.recent_policy_changes.truncate(100);
    }

    /// Perform comprehensive risk assessment
    pub async fn assess_risk(&mut self, request: RiskAssessmentRequest) -> crate::Result<RiskAssessmentResult> {
        let stability_score = self.get_stability_score(&request.country).await?;

        let mut result = RiskAssessmentResult::new(request.country.clone(), stability_score);

        if request.include_conflicts {
            result.conflict_exposure = self.get_conflicts_by_country(&request.country)
                .into_iter()
                .cloned()
                .collect();
        }

        if request.include_infrastructure {
            // Add mock infrastructure risks
            let infra_risk = InfrastructureRisk::new(
                request.country.clone(),
                InfrastructureType::Energy,
                "National Power Grid".to_string(),
            );
            result.infrastructure_risks.push(infra_risk);
        }

        if request.include_policy {
            result.policy_changes = self.get_policy_changes_by_country(&request.country)
                .into_iter()
                .cloned()
                .collect();
        }

        result.calculate_overall();

        // Generate recommendations
        if result.overall_risk_score.overall > 0.7 {
            result.recommendations.push(
                "High risk detected. Consider postponing significant investments.".to_string()
            );
        }
        if !result.conflict_exposure.is_empty() {
            result.recommendations.push(
                "Conflict exposure detected. Review supply chain vulnerabilities.".to_string()
            );
        }

        Ok(result)
    }

    /// Detect policy changes (mock implementation)
    pub async fn detect_policy_changes(&self, country: &CountryCode) -> crate::Result<Vec<PolicyChange>> {
        let mut changes = Vec::new();

        // Mock detection - in production this would analyze official sources
        match country.0.as_str() {
            "US" => {
                changes.push({
                    let mut change = PolicyChange::new(
                        country.clone(),
                        PolicyType::Trade,
                        "New Tariff on Semiconductors".to_string(),
                        "Additional tariffs on semiconductor imports from affected countries".to_string(),
                    );
                    change.description = "New export controls on advanced semiconductors".to_string();
                    change.verified = true;
                    change
                });
            },
            "CN" => {
                changes.push({
                    let mut change = PolicyChange::new(
                        country.clone(),
                        PolicyType::Investment,
                        "New Foreign Investment Rules".to_string(),
                        "Stricter review process for foreign investment in technology sector".to_string(),
                    );
                    change.verified = true;
                    change
                });
            },
            "EU" => {
                changes.push({
                    let mut change = PolicyChange::new(
                        country.clone(),
                        PolicyType::Environmental,
                        "Carbon Border Adjustment Mechanism".to_string(),
                        "Implementation of CBAM for steel, cement, aluminum, fertilizers".to_string(),
                    );
                    change.verified = true;
                    change
                });
            },
            _ => {}
        }

        Ok(changes)
    }

    /// Generate alert for conflict escalation
    pub fn generate_conflict_alert(&self, conflict: &ConflictZone) -> IntelligenceAlert {
        IntelligenceAlert::new(
            crate::models::AlertType::ConflictEscalation,
            match conflict.intensity {
                ConflictIntensity::Extreme | ConflictIntensity::VeryHigh => Severity::Critical,
                ConflictIntensity::High => Severity::High,
                _ => Severity::Medium,
            },
            format!("Conflict Alert: {}", conflict.name),
            format!(
                "{} conflict {} with {} intensity. Trajectory: {:?}",
                conflict.conflict_type.description(),
                conflict.status.description(),
                conflict.intensity.description(),
                conflict.trajectory
            ),
            IntelligenceSource::OpenSource,
            conflict.location.countries.clone(),
        )
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::disallowed_methods)]
    use super::*;

    #[test]
    fn test_stability_score_calculation() {
        let mut score = StabilityScore::new(CountryCode::new("US"));
        score.economic_stability = 0.8;
        score.political_stability = 0.7;
        score.social_cohesion = 0.75;
        score.governance = 0.85;
        score.rule_of_law = 0.88;
        score.corruption_index = 0.2;

        score.calculate_overall();

        assert!(score.overall_score > 0.7);
        assert!(score.is_investment_grade());
    }

    #[test]
    fn test_conflict_zone() {
        let mut conflict = ConflictZone::new(
            "Test Conflict".to_string(),
            ConflictType::ArmedConflict,
            GeoLocation::from_countries(vec![CountryCode::new("AA"), CountryCode::new("BB")]),
        );

        conflict.add_party(ConflictParty {
            name: "Government".to_string(),
            country: CountryCode::new("AA"),
            party_type: ConflictPartyType::Government,
            involvement_level: InvolvementLevel::Direct,
            casualties_estimate: Some(1000),
        });

        assert_eq!(conflict.parties_involved.len(), 1);
        assert!(conflict.affects_country(&CountryCode::new("AA")));
        assert!(!conflict.affects_country(&CountryCode::new("CC")));
    }

    #[test]
    fn test_policy_change_impact() {
        let mut change = PolicyChange::new(
            CountryCode::new("US"),
            PolicyType::Trade,
            "New Tariffs".to_string(),
            "25% tariff on affected goods".to_string(),
        );

        change.affected_sectors = vec![
            "Technology".to_string(),
            "Manufacturing".to_string(),
            "Agriculture".to_string(),
        ];

        let impact = change.impact_score();
        assert!(impact > 0.3);
    }

    #[test]
    fn test_infrastructure_risk() {
        let mut risk = InfrastructureRisk::new(
            CountryCode::new("US"),
            InfrastructureType::Energy,
            "Power Grid".to_string(),
        );

        risk.vulnerabilities.push(InfrastructureVulnerability {
            vulnerability_type: "Cyber".to_string(),
            severity: 0.8,
            description: "Potential cyber attack vector".to_string(),
            exploitation_difficulty: ExploitationDifficulty::Medium,
        });

        risk.threats.push(InfrastructureThreat {
            threat_type: "Cyber Attack".to_string(),
            probability: 0.4,
            severity: 0.7,
            description: "State-sponsored cyber threat".to_string(),
            attribution: Some("APT28".to_string()),
        });

        risk.criticality = CriticalityLevel::Critical;

        let calculated = risk.calculate_risk();
        assert!(calculated > 0.2);
    }

    #[test]
    fn test_conflict_intensity_comparison() {
        assert!(ConflictIntensity::Extreme > ConflictIntensity::VeryHigh);
        assert!(ConflictIntensity::High >= ConflictIntensity::High);
    }

    #[test]
    fn test_damage_level_scores() {
        assert_eq!(DamageLevel::None.to_score(), 0.0);
        assert_eq!(DamageLevel::Severe.to_score(), 0.7);
        assert_eq!(DamageLevel::Total.to_score(), 1.0);
    }

    #[tokio::test]
    async fn test_get_stability_score() {
        let config = GeopoliticalConfig::default();
        let client = Client::new();
        let mut political = PoliticalRiskClient::new(client, config);

        let score = political.get_stability_score(&CountryCode::new("US")).await.unwrap();
        assert!(score.overall_score > 0.5);
    }

    #[tokio::test]
    async fn test_risk_assessment() {
        let config = GeopoliticalConfig::default();
        let client = Client::new();
        let mut political = PoliticalRiskClient::new(client, config);

        let result = political.assess_risk(RiskAssessmentRequest {
            country: CountryCode::new("US"),
            ..Default::default()
        }).await.unwrap();

        assert!(result.stability_score.overall_score > 0.0);
    }

    #[test]
    fn test_policy_types() {
        assert_eq!(PolicyType::Security.weight(), 0.9);
        assert_eq!(PolicyType::Environmental.weight(), 0.4);
    }

    #[test]
    fn test_criticality_levels() {
        assert_eq!(CriticalityLevel::Critical.to_score(), 1.0);
        assert_eq!(CriticalityLevel::Low.to_score(), 0.2);
    }
}
