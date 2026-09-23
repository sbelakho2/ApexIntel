//! # Geopolitical Risk Assessment Workflow
//!
//! Production-ready workflow for regional risk analysis including:
//! - Sanctions/regulatory exposure
//! - Political stability indicators
//! - Trade policy impact
//! - Regional risk scoring
//!
//! ## Features
//!
//! - **Sanctions/Regulatory Exposure**: OFAC, EU sanctions lists, export controls
//! - **Political Stability Indicators**: Government stability, policy risk, social tension
//! - **Trade Policy Impact**: Tariffs, trade agreements, policy changes
//! - **Regional Risk Scoring**: Multi-factor risk assessment, trend analysis

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::reasoning::EvidenceItem;

/// Complete geopolitical risk assessment report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeopoliticalRiskReport {
    pub report_id: String,
    pub workflow_id: String,
    pub target_region: String,
    pub executive_summary: String,
    pub sanctions_exposure: SanctionsExposure,
    pub political_stability: PoliticalStability,
    pub trade_policy_impact: TradePolicyImpact,
    pub regional_risk_score: RegionalRiskScore,
    pub overall_risk_score: f64,
    pub key_risk_factors: Vec<String>,
    pub critical_concerns: Vec<String>,
    pub recommended_actions: Vec<String>,
    pub monitoring_indicators: Vec<String>,
    pub data_gaps: Vec<String>,
    pub generated_at: DateTime<Utc>,
}

/// Sanctions and regulatory exposure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SanctionsExposure {
    pub direct_sanctions: Vec<SanctionEntry>,
    pub indirect_exposure: Vec<IndirectExposure>,
    pub regulatory_flags: Vec<RegulatoryFlag>,
    pub compliance_risks: Vec<ComplianceRisk>,
    pub overall_exposure_score: f64,
    pub exposure_trend: TrendDirection,
}

/// Sanction entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SanctionEntry {
    pub sanction_type: SanctionType,
    pub issuing_authority: String,
    pub target_name: String,
    pub target_type: TargetType,
    pub listing_date: Option<DateTime<Utc>>,
    pub listing_reason: String,
    pub restriction_types: Vec<RestrictionType>,
    pub confidence: f64,
}

/// Type of sanction
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SanctionType {
    AssetFreeze,
    TravelBan,
    #[serde(rename = "TradeEmbargo")]
    TradeEmbargo,
    SectoralSanction,
    ListBasedSanction,
    ExecutiveOrder,
    ExportControl,
}

impl SanctionType {
    pub fn description(&self) -> &'static str {
        match self {
            SanctionType::AssetFreeze => "Asset Freeze",
            SanctionType::TravelBan => "Travel Ban",
            SanctionType::TradeEmbargo => "Trade Embargo",
            SanctionType::SectoralSanction => "Sectoral Sanction",
            SanctionType::ListBasedSanction => "List-Based Sanction",
            SanctionType::ExecutiveOrder => "Executive Order",
            SanctionType::ExportControl => "Export Control",
        }
    }
}

/// Target type
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TargetType {
    Government,
    GovernmentOfficial,
    StateOwnedEntity,
    PrivateCompany,
    Individual,
    Organization,
}

/// Restriction type
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RestrictionType {
    FinancialTransactions,
    AssetAccess,
    ImportRestrictions,
    ExportRestrictions,
    ServiceRestrictions,
    InvestmentRestrictions,
}

/// Indirect sanctions exposure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndirectExposure {
    pub exposure_type: IndirectExposureType,
    pub connected_entity: String,
    pub connection_description: String,
    pub risk_level: RiskLevel,
    pub recommended_diligence: String,
}

/// Type of indirect exposure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IndirectExposureType {
    OwnershipOverlap,
    SharedManagement,
    SupplierRelationship,
    CustomerRelationship,
    BankingRelationship,
    ServiceProvider,
}

/// Regulatory flag
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegulatoryFlag {
    pub flag_type: RegulatoryFlagType,
    pub description: String,
    pub jurisdiction: String,
    pub severity: RiskSeverity,
    pub regulatory_body: String,
    pub compliance_deadline: Option<DateTime<Utc>>,
}

/// Type of regulatory flag
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RegulatoryFlagType {
    EnhancedDueDiligence,
    ReportingRequirement,
    LicensingRequirement,
    ImportExportLicense,
    DataLocalization,
    LocalContentRequirement,
    AntiMoneyLaundering,
    KnowYourCustomer,
}

/// Compliance risk
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplianceRisk {
    pub risk_category: String,
    pub description: String,
    pub probability: f64,
    pub impact: f64,
    pub risk_score: f64,
    pub mitigation_controls: Vec<String>,
}

/// Political stability indicators
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoliticalStability {
    pub government_stability: GovernmentStability,
    pub policy_risk: PolicyRisk,
    pub social_indicators: SocialIndicators,
    pub conflict_indicators: ConflictIndicators,
    pub overall_stability_score: f64,
    pub stability_trend: TrendDirection,
}

/// Government stability assessment
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GovernmentStability {
    pub regime_type: RegimeType,
    pub tenure_outlook: String,
    pub popular_support: f64,
    pub opposition_strength: f64,
    pub institutional_strength: f64,
    pub stability_score: f64,
}

/// Regime type
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RegimeType {
    Democratic,
    Hybrid,
    Authoritarian,
    Theocratic,
    Unknown,
}

/// Policy risk assessment
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyRisk {
    pub policy_predictability: f64,
    pub regulatory_environment: String,
    pub policy_reversal_risk: f64,
    pub expropriation_risk: f64,
    pub key_policy_risks: Vec<PolicyRiskItem>,
}

/// Policy risk item
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyRiskItem {
    pub policy_area: String,
    pub risk_description: String,
    pub likelihood: f64,
    pub impact: f64,
    pub time_horizon: String,
}

/// Social indicators
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SocialIndicators {
    pub protest_risk: f64,
    pub crime_rate: CrimeRate,
    pub corruption_index: f64,
    pub human_rights_index: f64,
    pub social_tension_level: TensionLevel,
    pub key_social_concerns: Vec<String>,
}

/// Crime rate level
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CrimeRate {
    VeryLow,
    Low,
    Medium,
    High,
    VeryHigh,
}

/// Social tension level
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TensionLevel {
    Minimal,
    Low,
    Moderate,
    High,
    VeryHigh,
}

/// Conflict indicators
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictIndicators {
    pub active_conflicts: Vec<ActiveConflict>,
    pub historical_conflicts: i32,
    pub border_tensions: BorderTension,
    pub terrorism_risk: TerrorismRiskLevel,
    pub overall_conflict_risk: f64,
}

/// Active conflict
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveConflict {
    pub conflict_type: ConflictType,
    pub location: String,
    pub intensity: ConflictIntensity,
    pub start_date: DateTime<Utc>,
    pub parties_involved: Vec<String>,
    pub escalation_risk: f64,
}

/// Conflict type
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConflictType {
    War,
    ArmedConflict,
    PoliticalCrisis,
    CivilUnrest,
    BorderDispute,
}

impl ConflictType {
    pub fn description(&self) -> &'static str {
        match self {
            ConflictType::War => "War",
            ConflictType::ArmedConflict => "Armed Conflict",
            ConflictType::PoliticalCrisis => "Political Crisis",
            ConflictType::CivilUnrest => "Civil Unrest",
            ConflictType::BorderDispute => "Border Dispute",
        }
    }
}

/// Conflict intensity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConflictIntensity {
    Low,
    Medium,
    High,
    Critical,
}

/// Border tension level
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BorderTension {
    Stable,
    Elevated,
    High,
    Conflict,
}

/// Terrorism risk level
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TerrorismRiskLevel {
    Minimal,
    Low,
    Moderate,
    Elevated,
    High,
}

/// Trade policy impact
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradePolicyImpact {
    pub current_trade_agreements: Vec<TradeAgreement>,
    pub tariff_exposure: TariffExposure,
    pub trade_restrictions: Vec<TradeRestriction>,
    pub trade_partner_risks: Vec<TradePartnerRisk>,
    pub supply_chain_considerations: Vec<String>,
    pub overall_trade_impact_score: f64,
    pub trade_outlook: String,
}

/// Trade agreement
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeAgreement {
    pub agreement_name: String,
    pub parties: Vec<String>,
    pub agreement_type: AgreementType,
    pub effective_date: Option<DateTime<Utc>>,
    pub key_provisions: Vec<String>,
    pub benefit_assessment: String,
}

/// Agreement type
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AgreementType {
    Bilateral,
    Multilateral,
    Regional,
    FreeTradeArea,
    CustomsUnion,
    CommonMarket,
}

/// Tariff exposure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TariffExposure {
    pub current_tariff_rates: Vec<TariffRate>,
    pub average_tariff_rate: f64,
    pub tariff_trend: TrendDirection,
    pub duty_suspension_programs: Vec<String>,
    pub trade_agreement_benefits: f64,
    pub exposed_product_categories: Vec<String>,
}

/// Tariff rate
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TariffRate {
    pub product_category: String,
    pub hs_code: String,
    pub tariff_rate: f64,
    pub mfn_rate: f64,
    pub preferential_rate: Option<f64>,
    pub effective_date: DateTime<Utc>,
}

/// Trade restriction
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeRestriction {
    pub restriction_type: TradeRestrictionType,
    pub affected_products: Vec<String>,
    pub issuing_country: String,
    pub reason: String,
    pub effective_date: DateTime<Utc>,
    pub expected_duration: Option<String>,
    pub waiver_options: Vec<String>,
}

/// Type of trade restriction
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TradeRestrictionType {
    ImportQuota,
    ExportQuota,
    ImportBan,
    ExportBan,
    AntiDumpingDuty,
    CountervailingDuty,
    SafeguardMeasure,
    LocalContentRequirement,
}

/// Trade partner risk
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradePartnerRisk {
    pub partner_country: String,
    pub trade_volume: f64,
    pub payment_reliability: f64,
    pub logistics_reliability: f64,
    pub political_relationship: PoliticalRelationship,
    pub overall_risk_score: f64,
}

/// Political relationship with trade partner
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PoliticalRelationship {
    Ally,
    Friendly,
    Neutral,
    Tense,
    Adversarial,
}

/// Regional risk scoring
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegionalRiskScore {
    pub region_name: String,
    pub country_scores: Vec<CountryRiskScore>,
    pub regional_average_score: f64,
    pub risk_distribution: RiskDistribution,
    pub comparative_ranking: i32,
    pub trend: TrendDirection,
}

/// Country-level risk score
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CountryRiskScore {
    pub country_name: String,
    pub overall_score: f64,
    pub component_scores: ComponentScores,
    pub trend: TrendDirection,
    pub outlook: String,
}

/// Component risk scores
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentScores {
    pub political_score: f64,
    pub economic_score: f64,
    pub operational_score: f64,
    pub legal_score: f64,
    pub reputational_score: f64,
}

/// Risk distribution across the region
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskDistribution {
    pub low_risk_countries: i32,
    pub medium_risk_countries: i32,
    pub high_risk_countries: i32,
    pub critical_risk_countries: i32,
    pub distribution_chart: String,
}

/// Trend direction
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TrendDirection {
    Improving,
    Stable,
    Deteriorating,
    Unknown,
}

/// Risk level
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RiskLevel {
    Critical,
    High,
    Medium,
    Low,
}

/// Risk severity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RiskSeverity {
    Critical,
    High,
    Medium,
    Low,
}

/// Geopolitical Risk Workflow
#[derive(Debug, Clone)]
pub struct GeopoliticalRiskWorkflow {
    pub workflow_id: String,
    pub config: WorkflowConfig,
}

/// Workflow configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowConfig {
    pub include_sanctions: bool,
    pub include_political_stability: bool,
    pub include_trade_policy: bool,
    pub include_regional_scoring: bool,
    pub risk_threshold: f64,
}

impl Default for WorkflowConfig {
    fn default() -> Self {
        Self {
            include_sanctions: true,
            include_political_stability: true,
            include_trade_policy: true,
            include_regional_scoring: true,
            risk_threshold: 0.5,
        }
    }
}

impl GeopoliticalRiskWorkflow {
    /// Create a new geopolitical risk workflow
    pub fn new() -> Self {
        Self {
            workflow_id: Uuid::new_v4().to_string(),
            config: WorkflowConfig::default(),
        }
    }

    /// Create with custom configuration
    pub fn with_config(config: WorkflowConfig) -> Self {
        Self {
            workflow_id: Uuid::new_v4().to_string(),
            config,
        }
    }

    /// Run the complete geopolitical risk assessment workflow
    pub fn run(&self, target_region: &str, signals: Vec<EvidenceItem>) -> GeopoliticalRiskReport {
        let mut report = GeopoliticalRiskReport {
            report_id: Uuid::new_v4().to_string(),
            workflow_id: self.workflow_id.clone(),
            target_region: target_region.to_string(),
            executive_summary: String::new(),
            sanctions_exposure: SanctionsExposure {
                direct_sanctions: Vec::new(),
                indirect_exposure: Vec::new(),
                regulatory_flags: Vec::new(),
                compliance_risks: Vec::new(),
                overall_exposure_score: 0.0,
                exposure_trend: TrendDirection::Unknown,
            },
            political_stability: PoliticalStability {
                government_stability: GovernmentStability {
                    regime_type: RegimeType::Democratic,
                    tenure_outlook: String::new(),
                    popular_support: 0.0,
                    opposition_strength: 0.0,
                    institutional_strength: 0.0,
                    stability_score: 0.0,
                },
                policy_risk: PolicyRisk {
                    policy_predictability: 0.0,
                    regulatory_environment: String::new(),
                    policy_reversal_risk: 0.0,
                    expropriation_risk: 0.0,
                    key_policy_risks: Vec::new(),
                },
                social_indicators: SocialIndicators {
                    protest_risk: 0.0,
                    crime_rate: CrimeRate::Medium,
                    corruption_index: 0.0,
                    human_rights_index: 0.0,
                    social_tension_level: TensionLevel::Minimal,
                    key_social_concerns: Vec::new(),
                },
                conflict_indicators: ConflictIndicators {
                    active_conflicts: Vec::new(),
                    historical_conflicts: 0,
                    border_tensions: BorderTension::Stable,
                    terrorism_risk: TerrorismRiskLevel::Minimal,
                    overall_conflict_risk: 0.0,
                },
                overall_stability_score: 0.0,
                stability_trend: TrendDirection::Unknown,
            },
            trade_policy_impact: TradePolicyImpact {
                current_trade_agreements: Vec::new(),
                tariff_exposure: TariffExposure {
                    current_tariff_rates: Vec::new(),
                    average_tariff_rate: 0.0,
                    tariff_trend: TrendDirection::Unknown,
                    duty_suspension_programs: Vec::new(),
                    trade_agreement_benefits: 0.0,
                    exposed_product_categories: Vec::new(),
                },
                trade_restrictions: Vec::new(),
                trade_partner_risks: Vec::new(),
                supply_chain_considerations: Vec::new(),
                overall_trade_impact_score: 0.0,
                trade_outlook: String::new(),
            },
            regional_risk_score: RegionalRiskScore {
                region_name: target_region.to_string(),
                country_scores: Vec::new(),
                regional_average_score: 0.5,
                risk_distribution: RiskDistribution {
                    low_risk_countries: 0,
                    medium_risk_countries: 0,
                    high_risk_countries: 0,
                    critical_risk_countries: 0,
                    distribution_chart: String::new(),
                },
                comparative_ranking: 0,
                trend: TrendDirection::Stable,
            },
            overall_risk_score: 0.0,
            key_risk_factors: Vec::new(),
            critical_concerns: Vec::new(),
            recommended_actions: Vec::new(),
            monitoring_indicators: Vec::new(),
            data_gaps: Vec::new(),
            generated_at: Utc::now(),
        };

        // Run analysis components based on configuration
        if self.config.include_sanctions {
            report.sanctions_exposure = self.analyze_sanctions_exposure(&signals);
        }

        if self.config.include_political_stability {
            report.political_stability = self.analyze_political_stability(&signals);
        }

        if self.config.include_trade_policy {
            report.trade_policy_impact = self.analyze_trade_policy_impact(&signals);
        }

        if self.config.include_regional_scoring {
            report.regional_risk_score = self.calculate_regional_scoring(&signals);
        }

        // Calculate overall risk score
        let mut score_components: Vec<f64> = vec![
            report.sanctions_exposure.overall_exposure_score,
            report.political_stability.overall_stability_score,
            report.trade_policy_impact.overall_trade_impact_score,
            report.regional_risk_score.regional_average_score,
        ];

        // Invert stability score (higher stability = lower risk)
        let stability_component = 1.0 - report.political_stability.overall_stability_score;
        score_components[1] = stability_component;

        report.overall_risk_score = if score_components.is_empty() {
            0.5
        } else {
            score_components.iter().sum::<f64>() / score_components.len() as f64
        };

        // Generate executive summary
        report.executive_summary = self.generate_executive_summary(&report);

        // Extract key risk factors
        report.key_risk_factors = self.extract_key_risk_factors(&report);

        // Identify critical concerns
        report.critical_concerns = self.identify_critical_concerns(&report);

        // Generate recommended actions
        report.recommended_actions = self.generate_recommended_actions(&report);

        // Generate monitoring indicators
        report.monitoring_indicators = self.generate_monitoring_indicators(&report);

        // Identify data gaps
        report.data_gaps = self.identify_data_gaps(&report, &signals);

        report
    }

    /// Analyze sanctions exposure
    fn analyze_sanctions_exposure(&self, signals: &[EvidenceItem]) -> SanctionsExposure {
        let mut direct_sanctions = Vec::new();
        let mut indirect_exposure = Vec::new();
        let mut regulatory_flags = Vec::new();
        let mut compliance_risks = Vec::new();
        let mut exposure_score = 0.0;

        for signal in signals {
            match signal.evidence_type.as_str() {
                "sanction_listing" | "direct_sanction" => {
                    direct_sanctions.push(SanctionEntry {
                        sanction_type: SanctionType::SectoralSanction,
                        issuing_authority: signal.source.clone(),
                        target_name: signal.entity_id.clone(),
                        target_type: TargetType::PrivateCompany,
                        listing_date: Some(signal.timestamp),
                        listing_reason: signal.description.clone(),
                        restriction_types: vec![
                            RestrictionType::FinancialTransactions,
                            RestrictionType::InvestmentRestrictions,
                        ],
                        confidence: signal.confidence,
                    });
                    exposure_score += signal.confidence * 0.3;
                }
                "sanction_risk" | "sanction_exposure" => {
                    indirect_exposure.push(IndirectExposure {
                        exposure_type: IndirectExposureType::OwnershipOverlap,
                        connected_entity: signal.entity_id.clone(),
                        connection_description: signal.description.clone(),
                        risk_level: if signal.confidence > 0.7 {
                            RiskLevel::High
                        } else {
                            RiskLevel::Medium
                        },
                        recommended_diligence: "Enhanced due diligence required".to_string(),
                    });
                    exposure_score += signal.confidence * 0.2;
                }
                "regulatory_flag" | "compliance_issue" => {
                    regulatory_flags.push(RegulatoryFlag {
                        flag_type: RegulatoryFlagType::EnhancedDueDiligence,
                        description: signal.description.clone(),
                        jurisdiction: signal.source.clone(),
                        severity: if signal.confidence > 0.7 {
                            RiskSeverity::High
                        } else {
                            RiskSeverity::Medium
                        },
                        regulatory_body: signal.source.clone(),
                        compliance_deadline: Some(signal.timestamp),
                    });
                    exposure_score += signal.confidence * 0.15;
                }
                "export_control" => {
                    compliance_risks.push(ComplianceRisk {
                        risk_category: "Export Controls".to_string(),
                        description: signal.description.clone(),
                        probability: signal.confidence,
                        impact: 0.7,
                        risk_score: signal.confidence * 0.7,
                        mitigation_controls: vec![
                            "Verify end-user certificates".to_string(),
                            "Implement export screening".to_string(),
                        ],
                    });
                    exposure_score += signal.confidence * 0.2;
                }
                _ => {}
            }
        }

        SanctionsExposure {
            direct_sanctions,
            indirect_exposure,
            regulatory_flags,
            compliance_risks,
            overall_exposure_score: exposure_score.min(1.0),
            exposure_trend: if exposure_score > 0.5 {
                TrendDirection::Deteriorating
            } else {
                TrendDirection::Stable
            },
        }
    }

    /// Analyze political stability — starts from no-data defaults, populates only from signals
    fn analyze_political_stability(&self, signals: &[EvidenceItem]) -> PoliticalStability {
        let mut government_stability = GovernmentStability {
            regime_type: RegimeType::Unknown,
            tenure_outlook: String::new(),
            popular_support: 0.0,
            opposition_strength: 0.0,
            institutional_strength: 0.0,
            stability_score: 0.0,
        };

        let mut policy_risk = PolicyRisk {
            policy_predictability: 0.0,
            regulatory_environment: String::new(),
            policy_reversal_risk: 0.0,
            expropriation_risk: 0.0,
            key_policy_risks: Vec::new(),
        };

        let mut social_indicators = SocialIndicators {
            protest_risk: 0.0,
            crime_rate: CrimeRate::Medium,
            corruption_index: 0.0,
            human_rights_index: 0.0,
            social_tension_level: TensionLevel::Minimal,
            key_social_concerns: Vec::new(),
        };

        let mut conflict_indicators = ConflictIndicators {
            active_conflicts: Vec::new(),
            historical_conflicts: 0,
            border_tensions: BorderTension::Stable,
            terrorism_risk: TerrorismRiskLevel::Minimal,
            overall_conflict_risk: 0.0,
        };

        let mut stability_score = 1.0;

        for signal in signals {
            match signal.evidence_type.as_str() {
                "political_instability" => {
                    stability_score *= 1.0 - signal.confidence * 0.3;
                    government_stability.stability_score = stability_score;
                    policy_risk.policy_predictability *= 1.0 - signal.confidence * 0.2;
                    social_indicators.protest_risk += signal.confidence * 0.3;
                    social_indicators.social_tension_level = TensionLevel::Moderate;
                }
                "government_change" => {
                    government_stability.tenure_outlook = "Transition".to_string();
                    stability_score *= 0.8;
                }
                "policy_change" => {
                    policy_risk.key_policy_risks.push(PolicyRiskItem {
                        policy_area: signal.entity_id.clone(),
                        risk_description: signal.description.clone(),
                        likelihood: signal.confidence,
                        impact: 0.6,
                        time_horizon: "Near-term".to_string(),
                    });
                }
                "expropriation_risk" => {
                    policy_risk.expropriation_risk = signal.confidence;
                }
                "social_unrest" => {
                    social_indicators.protest_risk += signal.confidence * 0.4;
                    social_indicators.social_tension_level = TensionLevel::High;
                    stability_score *= 1.0 - signal.confidence * 0.2;
                }
                "armed_conflict" => {
                    stability_score *= 1.0 - signal.confidence * 0.4;
                    conflict_indicators.active_conflicts.push(ActiveConflict {
                        conflict_type: ConflictType::ArmedConflict,
                        location: signal.entity_id.clone(),
                        intensity: ConflictIntensity::High,
                        start_date: signal.timestamp,
                        parties_involved: vec![signal.description.clone()],
                        escalation_risk: signal.confidence,
                    });
                    conflict_indicators.overall_conflict_risk = signal.confidence;
                }
                "border_tension" => {
                    conflict_indicators.border_tensions = BorderTension::Elevated;
                }
                "terrorism" => {
                    conflict_indicators.terrorism_risk = TerrorismRiskLevel::Elevated;
                }
                "corruption" => {
                    social_indicators.corruption_index = signal.confidence;
                }
                "human_rights" => {
                    social_indicators.human_rights_index = 1.0 - signal.confidence * 0.3;
                }
                _ => {}
            }
        }

        PoliticalStability {
            government_stability,
            policy_risk,
            social_indicators,
            conflict_indicators,
            overall_stability_score: stability_score,
            stability_trend: if stability_score < 0.5 {
                TrendDirection::Deteriorating
            } else {
                TrendDirection::Stable
            },
        }
    }

    /// Analyze trade policy impact — starts from no-data defaults, populates only from signals
    fn analyze_trade_policy_impact(&self, signals: &[EvidenceItem]) -> TradePolicyImpact {
        let mut trade_agreements = Vec::new();
        let mut tariff_exposure = TariffExposure {
            current_tariff_rates: Vec::new(),
            average_tariff_rate: 0.0,
            tariff_trend: TrendDirection::Unknown,
            duty_suspension_programs: Vec::new(),
            trade_agreement_benefits: 0.0,
            exposed_product_categories: Vec::new(),
        };
        let mut trade_restrictions = Vec::new();
        let mut trade_partner_risks = Vec::new();
        let mut supply_chain_considerations = Vec::new();
        let mut trade_impact_score = 0.0;

        for signal in signals {
            match signal.evidence_type.as_str() {
                "tariff_increase" => {
                    tariff_exposure.current_tariff_rates.push(TariffRate {
                        product_category: signal.entity_id.clone(),
                        hs_code: "0000".to_string(),
                        tariff_rate: signal.confidence * 0.25,
                        mfn_rate: 0.0,
                        preferential_rate: None,
                        effective_date: signal.timestamp,
                    });
                    tariff_exposure.average_tariff_rate += signal.confidence * 0.1;
                    tariff_exposure.tariff_trend = TrendDirection::Improving; // higher tariffs = improving for protection
                    trade_impact_score += signal.confidence * 0.2;
                }
                "trade_agreement" => {
                    trade_agreements.push(TradeAgreement {
                        agreement_name: signal.description.clone(),
                        parties: vec![signal.entity_id.clone()],
                        agreement_type: AgreementType::Bilateral,
                        effective_date: Some(signal.timestamp),
                        key_provisions: vec!["Tariff reduction".to_string()],
                        benefit_assessment: "Positive".to_string(),
                    });
                    trade_impact_score -= signal.confidence * 0.1;
                }
                "trade_restriction" | "trade_ban" => {
                    trade_restrictions.push(TradeRestriction {
                        restriction_type: TradeRestrictionType::ImportBan,
                        affected_products: vec![signal.entity_id.clone()],
                        issuing_country: signal.source.clone(),
                        reason: signal.description.clone(),
                        effective_date: signal.timestamp,
                        expected_duration: Some("Ongoing".to_string()),
                        waiver_options: vec!["License application".to_string()],
                    });
                    trade_impact_score += signal.confidence * 0.3;
                }
                "trade_partner_risk" => {
                    trade_partner_risks.push(TradePartnerRisk {
                        partner_country: signal.entity_id.clone(),
                        trade_volume: 0.0,
                        payment_reliability: 1.0 - signal.confidence,
                        logistics_reliability: 1.0 - signal.confidence * 0.5,
                        political_relationship: PoliticalRelationship::Neutral,
                        overall_risk_score: signal.confidence,
                    });
                    trade_impact_score += signal.confidence * 0.15;
                }
                "supply_chain_disruption" => {
                    supply_chain_considerations.push(signal.description.clone());
                    trade_impact_score += signal.confidence * 0.2;
                }
                "trade_war" => {
                    trade_restrictions.push(TradeRestriction {
                        restriction_type: TradeRestrictionType::ImportBan,
                        affected_products: vec!["Multiple categories".to_string()],
                        issuing_country: signal.entity_id.clone(),
                        reason: signal.description.clone(),
                        effective_date: signal.timestamp,
                        expected_duration: None,
                        waiver_options: Vec::new(),
                    });
                    trade_impact_score += signal.confidence * 0.4;
                }
                _ => {}
            }
        }

        TradePolicyImpact {
            current_trade_agreements: trade_agreements,
            tariff_exposure,
            trade_restrictions,
            trade_partner_risks,
            supply_chain_considerations,
            overall_trade_impact_score: trade_impact_score.min(1.0),
            trade_outlook: if trade_impact_score > 0.5 {
                "Challenging".to_string()
            } else {
                "Favorable".to_string()
            },
        }
    }

    /// Calculate regional scoring using a weighted approach.
    ///
    /// ## Weight Rationale
    ///
    /// Each country's score is computed as a weighted average of five risk dimensions:
    ///
    /// | Dimension        | Weight | Rationale                                                                 |
    /// |------------------|--------|---------------------------------------------------------------------------|
    /// | Political        | 0.30   | Political instability has the highest systemic impact on business risk    |
    /// | Economic         | 0.20   | Economic factors (GDP, inflation) directly affect market viability         |
    /// | Operational      | 0.20   | Operational risks (infrastructure, labour) affect day-to-day execution    |
    /// | Legal/Regulatory | 0.15   | Legal environment shapes compliance burden and contract enforceability     |
    /// | Reputational     | 0.15   | Reputational risk influences brand perception and stakeholder trust        |
    ///
    /// The dimension scores are derived from signal confidence data, categorised
    /// by evidence type. Signals that match a specific risk dimension contribute
    /// primarily to that dimension; unmatched signals fall back to the overall
    /// average. The regional aggregate is then the mean of the country-level
    /// weighted scores, which prevents countries with many signals from
    /// disproportionately biasing the result.
    fn calculate_regional_scoring(&self, signals: &[EvidenceItem]) -> RegionalRiskScore {
        let mut country_scores = Vec::new();

        // Extract unique countries from signals
        let mut countries: Vec<String> = signals
            .iter()
            .filter(|s| s.entity_type == "country" || s.entity_type == "region")
            .map(|s| s.entity_id.clone())
            .collect();

        // Deduplicate
        countries.sort();
        countries.dedup();

        // Weights for each risk dimension
        const POLITICAL_WEIGHT: f64 = 0.30;
        const ECONOMIC_WEIGHT: f64 = 0.20;
        const OPERATIONAL_WEIGHT: f64 = 0.20;
        const LEGAL_WEIGHT: f64 = 0.15;
        const REPUTATIONAL_WEIGHT: f64 = 0.15;

        for country in &countries {
            let country_signals: Vec<_> =
                signals.iter().filter(|s| &s.entity_id == country).collect();

            let risk_sum: f64 = country_signals.iter().map(|s| s.confidence).sum();
            let avg_risk = if country_signals.is_empty() {
                0.5
            } else {
                risk_sum / country_signals.len() as f64
            };

            // Derive dimension-specific scores by categorising signals by evidence type
            let mut political_signals = Vec::new();
            let mut economic_signals = Vec::new();
            let mut operational_signals = Vec::new();
            let mut legal_signals = Vec::new();
            let mut reputational_signals = Vec::new();

            for s in &country_signals {
                match s.evidence_type.as_str() {
                    // Political: instability, government change, policy change, expropriation
                    t if t.contains("political")
                        || t.contains("government")
                        || t.contains("expropriation")
                        || t.contains("instability") =>
                    {
                        political_signals.push(s.confidence)
                    }
                    // Economic: tariff, trade, sanctions exposure
                    t if t.contains("tariff") || t.contains("trade") || t.contains("sanction") => {
                        economic_signals.push(s.confidence)
                    }
                    // Operational: supply chain, logistics, disruption
                    t if t.contains("supply_chain")
                        || t.contains("logistics")
                        || t.contains("disruption")
                        || t.contains("operational") =>
                    {
                        operational_signals.push(s.confidence)
                    }
                    // Legal/regulatory: regulatory flag, compliance, export control
                    t if t.contains("regulatory")
                        || t.contains("compliance")
                        || t.contains("export_control")
                        || t.contains("legal") =>
                    {
                        legal_signals.push(s.confidence)
                    }
                    // Reputational: social unrest, corruption, human rights
                    t if t.contains("social")
                        || t.contains("corruption")
                        || t.contains("human_rights")
                        || t.contains("reputational") =>
                    {
                        reputational_signals.push(s.confidence)
                    }
                    _ => {} // unmatched signals contribute to the fallback average only
                }
            }

            let dim_score = |sigs: &[f64]| -> f64 {
                if sigs.is_empty() {
                    avg_risk
                } else {
                    sigs.iter().sum::<f64>() / sigs.len() as f64
                }
            };

            let political_score = dim_score(&political_signals);
            let economic_score = dim_score(&economic_signals);
            let operational_score = dim_score(&operational_signals);
            let legal_score = dim_score(&legal_signals);
            let reputational_score = dim_score(&reputational_signals);

            // Weighted overall score
            let overall_score = political_score * POLITICAL_WEIGHT
                + economic_score * ECONOMIC_WEIGHT
                + operational_score * OPERATIONAL_WEIGHT
                + legal_score * LEGAL_WEIGHT
                + reputational_score * REPUTATIONAL_WEIGHT;

            country_scores.push(CountryRiskScore {
                country_name: country.clone(),
                overall_score,
                component_scores: ComponentScores {
                    political_score,
                    economic_score,
                    operational_score,
                    legal_score,
                    reputational_score,
                },
                trend: TrendDirection::Unknown,
                outlook: if overall_score > 0.6 {
                    "Caution advised".to_string()
                } else {
                    "Standard operations".to_string()
                },
            });
        }

        // Regional score: simple average of country-level weighted scores
        let regional_score = if country_scores.is_empty() {
            0.5
        } else {
            country_scores.iter().map(|c| c.overall_score).sum::<f64>()
                / country_scores.len() as f64
        };

        let distribution = RiskDistribution {
            low_risk_countries: country_scores
                .iter()
                .filter(|c| c.overall_score < 0.3)
                .count() as i32,
            medium_risk_countries: country_scores
                .iter()
                .filter(|c| c.overall_score >= 0.3 && c.overall_score < 0.6)
                .count() as i32,
            high_risk_countries: country_scores
                .iter()
                .filter(|c| c.overall_score >= 0.6 && c.overall_score < 0.8)
                .count() as i32,
            critical_risk_countries: country_scores
                .iter()
                .filter(|c| c.overall_score >= 0.8)
                .count() as i32,
            distribution_chart: format!(
                "Low: {}, Medium: {}, High: {}, Critical: {}",
                country_scores
                    .iter()
                    .filter(|c| c.overall_score < 0.3)
                    .count(),
                country_scores
                    .iter()
                    .filter(|c| c.overall_score >= 0.3 && c.overall_score < 0.6)
                    .count(),
                country_scores
                    .iter()
                    .filter(|c| c.overall_score >= 0.6 && c.overall_score < 0.8)
                    .count(),
                country_scores
                    .iter()
                    .filter(|c| c.overall_score >= 0.8)
                    .count()
            ),
        };

        RegionalRiskScore {
            region_name: "Target Region".to_string(),
            country_scores,
            regional_average_score: regional_score,
            risk_distribution: distribution,
            comparative_ranking: 50,
            trend: TrendDirection::Unknown,
        }
    }

    /// Generate executive summary
    fn generate_executive_summary(&self, report: &GeopoliticalRiskReport) -> String {
        let mut summary = format!(
            "Geopolitical Risk Assessment for {}\n\n",
            report.target_region
        );

        summary.push_str(&format!(
            "Overall Risk Score: {:.0}%\n\n",
            report.overall_risk_score * 100.0
        ));

        summary.push_str(&format!(
            "## Sanctions Exposure\nDirect Sanctions: {}\n",
            report.sanctions_exposure.direct_sanctions.len()
        ));
        summary.push_str(&format!(
            "Indirect Exposure: {}\n",
            report.sanctions_exposure.indirect_exposure.len()
        ));

        summary.push_str(&format!(
            "## Political Stability\nStability Score: {:.0}%\n",
            report.political_stability.overall_stability_score * 100.0
        ));
        summary.push_str(&format!(
            "Trend: {:?}\n",
            report.political_stability.stability_trend
        ));

        summary.push_str(&format!(
            "## Trade Policy Impact\nTrade Impact Score: {:.0}%\n",
            report.trade_policy_impact.overall_trade_impact_score * 100.0
        ));
        summary.push_str(&format!(
            "Trade Outlook: {}\n",
            report.trade_policy_impact.trade_outlook
        ));

        summary.push_str(&format!(
            "## Regional Risk\nRegional Score: {:.0}%\n",
            report.regional_risk_score.regional_average_score * 100.0
        ));

        summary
    }

    /// Extract key risk factors
    fn extract_key_risk_factors(&self, report: &GeopoliticalRiskReport) -> Vec<String> {
        let mut risk_factors = Vec::new();

        if report.sanctions_exposure.overall_exposure_score > 0.5 {
            risk_factors.push("Sanctions exposure presents compliance risk".to_string());
        }

        if report.political_stability.overall_stability_score < 0.5 {
            risk_factors.push("Political instability increases operational risk".to_string());
        }

        if report
            .political_stability
            .conflict_indicators
            .overall_conflict_risk
            > 0.4
        {
            risk_factors.push("Active conflicts affect regional operations".to_string());
        }

        if report.trade_policy_impact.overall_trade_impact_score > 0.5 {
            risk_factors.push("Trade policy changes impact supply chain".to_string());
        }

        if report.trade_policy_impact.trade_restrictions.len() > 3 {
            risk_factors.push("Multiple trade restrictions active in region".to_string());
        }

        if report
            .regional_risk_score
            .risk_distribution
            .critical_risk_countries
            > 0
        {
            risk_factors.push("Critical risk countries identified in region".to_string());
        }

        risk_factors
    }

    /// Identify critical concerns
    fn identify_critical_concerns(&self, report: &GeopoliticalRiskReport) -> Vec<String> {
        let mut concerns = Vec::new();

        for sanction in &report.sanctions_exposure.direct_sanctions {
            if sanction.confidence > 0.8 {
                concerns.push(format!(
                    "Direct sanction: {} ({})",
                    sanction.target_name, sanction.issuing_authority
                ));
            }
        }

        for flag in &report.sanctions_exposure.regulatory_flags {
            if matches!(flag.severity, RiskSeverity::Critical) {
                concerns.push(format!("Critical regulatory flag: {}", flag.description));
            }
        }

        for conflict in &report
            .political_stability
            .conflict_indicators
            .active_conflicts
        {
            if matches!(
                conflict.intensity,
                ConflictIntensity::High | ConflictIntensity::Critical
            ) {
                concerns.push(format!(
                    "Critical conflict: {} in {}",
                    conflict.conflict_type.description(),
                    conflict.location
                ));
            }
        }

        if report
            .political_stability
            .government_stability
            .stability_score
            < 0.3
        {
            concerns.push("Severe government instability detected".to_string());
        }

        if report.trade_policy_impact.overall_trade_impact_score > 0.7 {
            concerns.push("High trade policy risk requires immediate attention".to_string());
        }

        concerns
    }

    /// Generate recommended actions
    fn generate_recommended_actions(&self, report: &GeopoliticalRiskReport) -> Vec<String> {
        let mut actions = Vec::new();

        // Sanctions-related actions
        if !report.sanctions_exposure.direct_sanctions.is_empty() {
            actions.push("Implement comprehensive sanctions screening".to_string());
            actions.push("Establish enhanced due diligence procedures".to_string());
        }

        if !report.sanctions_exposure.compliance_risks.is_empty() {
            actions.push("Review and update compliance controls".to_string());
        }

        // Political stability actions
        if report.political_stability.overall_stability_score < 0.5 {
            actions.push("Develop contingency plans for political instability".to_string());
            actions.push("Consider operational flexibility across borders".to_string());
        }

        // Trade policy actions
        if report.trade_policy_impact.trade_restrictions.len() > 2 {
            actions.push("Diversify supply chain to reduce trade restriction exposure".to_string());
        }

        if report.trade_policy_impact.overall_trade_impact_score > 0.5 {
            actions.push("Monitor trade policy developments closely".to_string());
            actions.push("Evaluate alternative trade routes".to_string());
        }

        // Conflict-related actions
        if !report
            .political_stability
            .conflict_indicators
            .active_conflicts
            .is_empty()
        {
            actions.push("Implement travel security protocols".to_string());
            actions.push("Review business continuity plans".to_string());
        }

        // Regional scoring actions
        if report.regional_risk_score.regional_average_score > 0.5 {
            actions.push("Consider phased entry approach for high-risk areas".to_string());
        }

        actions
    }

    /// Generate monitoring indicators based on actual report content
    fn generate_monitoring_indicators(&self, report: &GeopoliticalRiskReport) -> Vec<String> {
        let mut indicators = Vec::new();

        // Sanctions monitoring — only if there is exposure
        if !report.sanctions_exposure.direct_sanctions.is_empty()
            || !report.sanctions_exposure.indirect_exposure.is_empty()
        {
            indicators.push("Monitor sanctions list updates for listed entities".to_string());
            indicators
                .push("Track regulatory announcements related to identified exposures".to_string());
        }
        if !report.sanctions_exposure.compliance_risks.is_empty() {
            indicators.push("Track compliance incidents and deadlines".to_string());
        }

        // Political monitoring — only if stability data was populated
        if report
            .political_stability
            .government_stability
            .stability_score
            > 0.0
        {
            indicators.push("Monitor government stability indicators".to_string());
        }
        if !report
            .political_stability
            .policy_risk
            .key_policy_risks
            .is_empty()
        {
            indicators
                .push("Track policy change announcements for identified risk areas".to_string());
        }
        if report.political_stability.social_indicators.protest_risk > 0.0 {
            indicators.push("Monitor social media and news for unrest signals".to_string());
        }
        if !report
            .political_stability
            .conflict_indicators
            .active_conflicts
            .is_empty()
        {
            indicators.push("Track conflict escalation indicators".to_string());
        }

        // Trade monitoring — only if there is tariff or trade data
        if !report
            .trade_policy_impact
            .tariff_exposure
            .current_tariff_rates
            .is_empty()
        {
            indicators
                .push("Monitor tariff rate changes for affected product categories".to_string());
        }
        if !report
            .trade_policy_impact
            .current_trade_agreements
            .is_empty()
        {
            indicators.push("Track trade agreement developments".to_string());
        }
        if !report.trade_policy_impact.trade_restrictions.is_empty() {
            indicators.push("Monitor trade restriction announcements".to_string());
        }
        if !report
            .trade_policy_impact
            .supply_chain_considerations
            .is_empty()
        {
            indicators.push("Monitor supply chain disruption early warnings".to_string());
        }

        // Regional monitoring — only if country scores exist
        if !report.regional_risk_score.country_scores.is_empty() {
            indicators.push("Track country risk score changes".to_string());
        }

        // Always add a generic monitoring indicator if none were generated
        if indicators.is_empty() {
            indicators.push(
                "No specific monitoring indicators — insufficient data in report".to_string(),
            );
        }

        indicators
    }

    /// Identify data gaps
    fn identify_data_gaps(
        &self,
        report: &GeopoliticalRiskReport,
        signals: &[EvidenceItem],
    ) -> Vec<String> {
        let mut gaps = Vec::new();

        if report.sanctions_exposure.direct_sanctions.is_empty()
            && report.sanctions_exposure.indirect_exposure.is_empty()
        {
            gaps.push("Limited sanctions screening data".to_string());
        }

        if report
            .political_stability
            .government_stability
            .stability_score
            == 0.0
        {
            gaps.push(
                "No political stability data available - region-specific data needed".to_string(),
            );
        }

        if report
            .trade_policy_impact
            .current_trade_agreements
            .is_empty()
            && report.trade_policy_impact.trade_restrictions.is_empty()
        {
            gaps.push("Limited trade policy data available".to_string());
        }

        if report.regional_risk_score.country_scores.is_empty() {
            gaps.push("Country-level risk data not available".to_string());
        }

        if signals.len() < 5 {
            gaps.push("Limited signal data for comprehensive geopolitical analysis".to_string());
        }

        gaps
    }
}

impl Default for GeopoliticalRiskWorkflow {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn test_workflow_creation() {
        let workflow = GeopoliticalRiskWorkflow::new();
        assert!(!workflow.workflow_id.is_empty());
        assert!(workflow.config.include_sanctions);
    }

    #[test]
    fn test_workflow_with_config() {
        let config = WorkflowConfig {
            include_sanctions: true,
            include_political_stability: false,
            include_trade_policy: true,
            include_regional_scoring: false,
            risk_threshold: 0.6,
        };
        let workflow = GeopoliticalRiskWorkflow::with_config(config);
        assert!(workflow.config.include_sanctions);
        assert!(!workflow.config.include_political_stability);
    }

    #[test]
    fn test_workflow_run() {
        let workflow = GeopoliticalRiskWorkflow::new();
        let signals = vec![
            EvidenceItem {
                id: "sig1".to_string(),
                entity_id: "APAC".to_string(),
                entity_type: "region".to_string(),
                evidence_type: "sanction_exposure".to_string(),
                description: "Sanctions risk identified".to_string(),
                source: "OFAC".to_string(),
                confidence: 0.7,
                timestamp: Utc::now(),
                raw_data: serde_json::json!({}),
            },
            EvidenceItem {
                id: "sig2".to_string(),
                entity_id: "China".to_string(),
                entity_type: "country".to_string(),
                evidence_type: "trade_restriction".to_string(),
                description: "Export restrictions announced".to_string(),
                source: "Ministry of Commerce".to_string(),
                confidence: 0.8,
                timestamp: Utc::now(),
                raw_data: serde_json::json!({}),
            },
        ];

        let report = workflow.run("APAC Region", signals);
        assert_eq!(report.target_region, "APAC Region");
        assert!(report.sanctions_exposure.overall_exposure_score > 0.0);
    }

    #[test]
    fn test_sanctions_exposure_analysis() {
        let workflow = GeopoliticalRiskWorkflow::new();
        let signals = vec![
            EvidenceItem {
                id: "sig1".to_string(),
                entity_id: "Company A".to_string(),
                entity_type: "company".to_string(),
                evidence_type: "sanction_listing".to_string(),
                description: "Added to sanctions list".to_string(),
                source: "OFAC".to_string(),
                confidence: 0.95,
                timestamp: Utc::now(),
                raw_data: serde_json::json!({}),
            },
            EvidenceItem {
                id: "sig2".to_string(),
                entity_id: "Supplier B".to_string(),
                entity_type: "company".to_string(),
                evidence_type: "sanction_risk".to_string(),
                description: "Ownership overlap with sanctioned entity".to_string(),
                source: "Compliance Team".to_string(),
                confidence: 0.8,
                timestamp: Utc::now(),
                raw_data: serde_json::json!({}),
            },
        ];

        let report = workflow.run("Test Region", signals);
        assert!(!report.sanctions_exposure.direct_sanctions.is_empty());
        assert!(!report.sanctions_exposure.indirect_exposure.is_empty());
    }

    #[test]
    fn test_political_stability_analysis() {
        let workflow = GeopoliticalRiskWorkflow::new();
        let signals = vec![
            EvidenceItem {
                id: "sig1".to_string(),
                entity_id: "Country A".to_string(),
                entity_type: "country".to_string(),
                evidence_type: "political_instability".to_string(),
                description: "Government facing challenges".to_string(),
                source: "Political Analysis".to_string(),
                confidence: 0.75,
                timestamp: Utc::now(),
                raw_data: serde_json::json!({}),
            },
            EvidenceItem {
                id: "sig2".to_string(),
                entity_id: "Border Region".to_string(),
                entity_type: "region".to_string(),
                evidence_type: "armed_conflict".to_string(),
                description: "Ongoing armed conflict".to_string(),
                source: "Security Report".to_string(),
                confidence: 0.9,
                timestamp: Utc::now(),
                raw_data: serde_json::json!({}),
            },
        ];

        let report = workflow.run("Test Region", signals);
        assert!(report.political_stability.overall_stability_score < 0.6);
        assert!(!report
            .political_stability
            .conflict_indicators
            .active_conflicts
            .is_empty());
    }

    #[test]
    fn test_trade_policy_impact_analysis() {
        let workflow = GeopoliticalRiskWorkflow::new();
        let signals = vec![
            EvidenceItem {
                id: "sig1".to_string(),
                entity_id: "EU".to_string(),
                entity_type: "region".to_string(),
                evidence_type: "tariff_increase".to_string(),
                description: "New tariffs on electronics".to_string(),
                source: "EU Commission".to_string(),
                confidence: 0.85,
                timestamp: Utc::now(),
                raw_data: serde_json::json!({}),
            },
            EvidenceItem {
                id: "sig2".to_string(),
                entity_id: "USA".to_string(),
                entity_type: "country".to_string(),
                evidence_type: "trade_restriction".to_string(),
                description: "Export controls tightened".to_string(),
                source: "Commerce Dept".to_string(),
                confidence: 0.9,
                timestamp: Utc::now(),
                raw_data: serde_json::json!({}),
            },
        ];

        let report = workflow.run("Test Region", signals);
        assert!(!report
            .trade_policy_impact
            .tariff_exposure
            .current_tariff_rates
            .is_empty());
        assert!(!report.trade_policy_impact.trade_restrictions.is_empty());
    }

    #[test]
    fn test_regional_scoring() {
        let workflow = GeopoliticalRiskWorkflow::new();
        let signals = vec![
            EvidenceItem {
                id: "sig1".to_string(),
                entity_id: "Country A".to_string(),
                entity_type: "country".to_string(),
                evidence_type: "political_instability".to_string(),
                description: "Political risk".to_string(),
                source: "Analysis".to_string(),
                confidence: 0.7,
                timestamp: Utc::now(),
                raw_data: serde_json::json!({}),
            },
            EvidenceItem {
                id: "sig2".to_string(),
                entity_id: "Country B".to_string(),
                entity_type: "country".to_string(),
                evidence_type: "sanction_risk".to_string(),
                description: "Sanction risk".to_string(),
                source: "Analysis".to_string(),
                confidence: 0.6,
                timestamp: Utc::now(),
                raw_data: serde_json::json!({}),
            },
        ];

        let report = workflow.run("Test Region", signals);
        assert!(!report.regional_risk_score.country_scores.is_empty());
        assert!(report.regional_risk_score.regional_average_score > 0.0);
    }

    #[test]
    fn test_overall_risk_calculation() {
        let workflow = GeopoliticalRiskWorkflow::new();
        let signals = vec![
            EvidenceItem {
                id: "sig1".to_string(),
                entity_id: "Region".to_string(),
                entity_type: "region".to_string(),
                evidence_type: "sanction_listing".to_string(),
                description: "High sanction risk".to_string(),
                source: "OFAC".to_string(),
                confidence: 0.85,
                timestamp: Utc::now(),
                raw_data: serde_json::json!({}),
            },
            EvidenceItem {
                id: "sig2".to_string(),
                entity_id: "Country".to_string(),
                entity_type: "country".to_string(),
                evidence_type: "political_instability".to_string(),
                description: "Instability".to_string(),
                source: "Analysis".to_string(),
                confidence: 0.8,
                timestamp: Utc::now(),
                raw_data: serde_json::json!({}),
            },
        ];

        let report = workflow.run("Test Region", signals);
        assert!(report.overall_risk_score > 0.0);
        assert!(report.overall_risk_score <= 1.0);
    }

    #[test]
    fn test_critical_concerns_identification() {
        let workflow = GeopoliticalRiskWorkflow::new();
        let signals = vec![
            EvidenceItem {
                id: "sig1".to_string(),
                entity_id: "Target A".to_string(),
                entity_type: "entity".to_string(),
                evidence_type: "sanction_listing".to_string(),
                description: "Direct sanction".to_string(),
                source: "OFAC".to_string(),
                confidence: 0.95,
                timestamp: Utc::now(),
                raw_data: serde_json::json!({}),
            },
            EvidenceItem {
                id: "sig2".to_string(),
                entity_id: "Region".to_string(),
                entity_type: "region".to_string(),
                evidence_type: "armed_conflict".to_string(),
                description: "Active war".to_string(),
                source: "Security".to_string(),
                confidence: 0.95,
                timestamp: Utc::now(),
                raw_data: serde_json::json!({}),
            },
        ];

        let report = workflow.run("Test Region", signals);
        assert!(!report.critical_concerns.is_empty());
    }

    #[test]
    fn test_monitoring_indicators_generation() {
        let workflow = GeopoliticalRiskWorkflow::new();
        let signals = vec![EvidenceItem {
            id: "sig1".to_string(),
            entity_id: "Region".to_string(),
            entity_type: "region".to_string(),
            evidence_type: "political_instability".to_string(),
            description: "Political risk".to_string(),
            source: "Analysis".to_string(),
            confidence: 0.7,
            timestamp: Utc::now(),
            raw_data: serde_json::json!({}),
        }];

        let report = workflow.run("Test Region", signals);
        // Monitoring indicators are now generated from report content, not static lists.
        // With only a political_instability signal, we expect political monitoring indicators.
        assert!(!report.monitoring_indicators.is_empty());
        assert!(report
            .monitoring_indicators
            .iter()
            .any(|i| i.contains("stability") || i.contains("political")));
    }

    #[test]
    fn test_data_gaps_identification() {
        let workflow = GeopoliticalRiskWorkflow::new();
        let signals = vec![];

        let report = workflow.run("Test Region", signals);
        assert!(!report.data_gaps.is_empty());
    }

    #[test]
    fn test_report_serialization() {
        let workflow = GeopoliticalRiskWorkflow::new();
        let signals = vec![];

        let report = workflow.run("Test Region", signals);
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("Test Region"));
        assert!(json.contains("workflow_id"));
    }
}
