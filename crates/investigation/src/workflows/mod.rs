//! # Investigation Workflows Module
//!
//! Production-ready investigation workflows for OSINT intelligence analysis.
//!
//! This module provides structured, reusable workflows for common investigation
//! scenarios including company intelligence, person deep-dives, supply chain
//! threat assessment, market opportunity detection, and geopolitical risk analysis.
//!
//! ## Workflows
//!
//! ### Company Intelligence Report (`company_intelligence`)
//! Comprehensive company analysis including:
//! - Financial health assessment
//! - Leadership analysis (POI dossier)
//! - Competitive positioning
//! - Supply chain risk
//! - Strategic opportunities
//! - Threat assessment
//!
//! ### Person Deep-Dive (`person_deep_dive`)
//! Detailed person investigation including:
//! - Professional history reconstruction
//! - Network mapping
//! - Risk indicators
//! - Trigger event detection
//! - Engagement strategy
//!
//! ### Supply Chain Threat Assessment (`supply_chain_threat`)
//! Supply chain vulnerability analysis including:
//! - Single-point-of-failure identification
//! - Geopolitical risk factors
//! - Financial stability monitoring
//! - Concentration risk
//! - Alternative supplier discovery
//!
//! ### Market Opportunity Detection (`market_opportunity`)
//! Market analysis for competitive intelligence:
//! - Procurement signal aggregation
//! - Competitor weakness identification
//! - Market timing analysis
//! - Strategic entry points
//!
//! ### Geopolitical Risk Assessment (`geopolitical_risk`)
//! Regional risk analysis including:
//! - Sanctions/regulatory exposure
//! - Political stability indicators
//! - Trade policy impact
//! - Regional risk scoring
//!
//! ## Usage
//!
//! ```rust
//! use apex_investigation::workflows::{
//!     company_intelligence::CompanyIntelligenceWorkflow,
//!     person_deep_dive::PersonDeepDiveWorkflow,
//!     supply_chain_threat::SupplyChainThreatWorkflow,
//!     market_opportunity::MarketOpportunityWorkflow,
//!     geopolitical_risk::GeopoliticalRiskWorkflow,
//! };
//! use apex_investigation::reasoning::EvidenceItem;
//! use chrono::Utc;
//!
//! let signals = vec![EvidenceItem {
//!     id: "test".to_string(),
//!     entity_id: "test".to_string(),
//!     entity_type: "company".to_string(),
//!     evidence_type: "test".to_string(),
//!     description: "Test evidence".to_string(),
//!     source: "test".to_string(),
//!     confidence: 0.8,
//!     timestamp: Utc::now(),
//!     raw_data: serde_json::json!({}),
//! }];
//!
//! // Company Intelligence
//! let mut company_workflow = CompanyIntelligenceWorkflow::new();
//! let company_result = company_workflow.run("Acme Corp", signals.clone());
//!
//! // Person Deep-Dive
//! let mut person_workflow = PersonDeepDiveWorkflow::new();
//! let person_result = person_workflow.run("John Doe", signals.clone());
//!
//! // Supply Chain Threat
//! let mut supply_chain_workflow = SupplyChainThreatWorkflow::new();
//! let supply_chain_result = supply_chain_workflow.run("Supplier A", signals.clone());
//!
//! // Market Opportunity
//! let mut market_workflow = MarketOpportunityWorkflow::new();
//! let market_result = market_workflow.run("Tech Sector", signals.clone());
//!
//! // Geopolitical Risk
//! let mut geo_workflow = GeopoliticalRiskWorkflow::new();
//! let geo_result = geo_workflow.run("North America", signals.clone());
//! ```
//!
//! ## Production Considerations
//!
//! - All workflows include confidence scoring and evidence tracking
//! - Workflow outputs are structured for automated report generation
//! - Each workflow supports incremental updates with new evidence
//! - Built-in bias detection and confidence calibration

pub mod company_intelligence;
pub mod geopolitical_risk;
pub mod market_opportunity;
pub mod person_deep_dive;
pub mod supply_chain_threat;

// Re-export workflow types
pub use company_intelligence::{
    CompanyIntelligenceReport, CompanyIntelligenceWorkflow, CompetitorAnalysis,
    FinancialHealthAssessment, LeadershipAnalysis, StrategicOpportunity, SupplyChainRisk,
    ThreatAssessment, WorkflowResult,
};
pub use geopolitical_risk::{
    GeopoliticalRiskReport, GeopoliticalRiskWorkflow, PoliticalStability, RegionalRiskScore,
    SanctionsExposure, TradePolicyImpact,
};
pub use market_opportunity::{
    CompetitorWeakness, MarketOpportunityReport, MarketOpportunityWorkflow, MarketTiming,
    ProcurementSignal, StrategicEntryPoint,
};
pub use person_deep_dive::{
    EngagementStrategy, NetworkMapping, PersonDeepDiveReport, PersonDeepDiveWorkflow,
    ProfessionalHistory, PsychologicalProfile, RiskIndicatorReport, TriggerEvent,
};
pub use supply_chain_threat::{
    AlternativeSupplier, ConcentrationRisk, FinancialStability, GeographicRisk,
    SinglePointOfFailure, SupplyChainThreatReport, SupplyChainThreatWorkflow,
};
