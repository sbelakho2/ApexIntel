//! # Geopolitical Intelligence Module for ApexIntel
//!
//! This module provides comprehensive geopolitical intelligence capabilities including:
//!
//! ## 3.3.1 Sanctions Monitoring
//! - OFAC SDN (Specially Designated Nationals) list integration
//! - EU sanction list tracking
//! - UN sanction list monitoring
//! - Automated compliance alerts
//!
//! ## 3.3.2 Trade Intelligence
//! - Import/export data analysis with HS codes
//! - Tariff impact assessment
//! - Trade agreement monitoring
//! - Supply chain rerouting signals
//!
//! ## 3.3.3 Political Risk Assessment
//! - Country stability scoring
//! - Policy change detection
//! - Conflict zone monitoring
//! - Infrastructure risk analysis
//!
//! ## 3.3.4 Regulatory Intelligence
//! - Industry regulation tracking
//! - Environmental compliance monitoring
//! - Labor law change detection
//! - Tax policy updates

pub mod sanctions;
pub mod trade;
pub mod political_risk;
pub mod regulatory;

pub mod error;
pub mod models;
pub mod utils;

// Re-export commonly used types
pub use sanctions::{
    SanctionsClient, SanctionsMonitor, SanctionEntity, SanctionList, 
    SanctionEntityType, SanctionProgram, ScreeningRequest, ScreeningResult, 
    ComplianceStatus as SanctionsComplianceStatus, ComplianceAlert, SanctionsMatch, SanctionsUpdateSummary
};
pub use trade::{
    TradeIntelligenceClient, HsCode, TariffInfo, TradeFlow, TradeRoute,
    SupplyChainSignal, SupplyChainSignalType, TradeAgreement, TradeAgreementType,
    TradeAgreementStatus, TradeRestriction, TradeRestrictionType, TradeAnalysis,
    TradeAnalysisType
};
pub use political_risk::{
    PoliticalRiskClient, StabilityScore, ConflictZone, ConflictType, 
    ConflictIntensity, ConflictStatus, ConflictParty, ConflictTrajectory,
    PolicyChange, PolicyType, PolicyImpact, InfrastructureRisk, 
    InfrastructureType, RiskAssessmentRequest, 
    RiskAssessmentResult, RiskAssessmentType, TimeHorizon
};
pub use regulatory::{
    RegulatoryClient, Regulation, RegulationType, RegulationStatus,
    ComplianceRequirement, ComplianceStatus, RequirementType, ReportingFrequency,
    Penalty, PenaltyType, Exemption, TaxPolicy, TaxType, RateType,
    EnvironmentalCompliance, LaborLawChange, LaborLawType, ComplianceReport,
    UpcomingDeadline, ComplianceRisk, RegulatoryUpdate, UpdateType
};
pub use error::{GeopoliticalError, Result};

