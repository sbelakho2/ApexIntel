//! # Apex Investigation Framework
//!
//! Deep investigation framework for OSINT intelligence analysis.
//!
//! This crate provides comprehensive tools for conducting deep-dive investigations
//! into entities (companies, people, organizations) using multi-source OSINT data.
//!
//! ## Core Components
//!
//! ### Investigation Engine (`investigations`)
 //! The main orchestration layer that manages the complete investigation lifecycle:
//! - Creates and manages investigations
//! - Orchestrates all analysis components
//! - Returns comprehensive investigation results
//!
//! ### Hypothesis Generation (`hypothesis`)
//! Multi-hypothesis tracking using Analysis of Competing Hypotheses (ACH):
//! - Generates competing hypotheses about entity behavior
//! - Bayesian evidence fusion
//! - Gap analysis for missing information
//! - Counterfactual reasoning
//!
//! ### Chain-of-Thought Reasoning (`reasoning`)
//! Multi-hop inference engine:
//! - Builds reasoning chains from evidence
//! - Temporal pattern detection
//! - Entity correlation analysis
//! - Confidence propagation through reasoning steps
//!
//! ### Threat Modeling (`threats`)
//! Comprehensive threat assessment:
//! - Attack surface assessment
//! - Supply chain threat modeling
//! - Scenario simulation
//! - Risk vector analysis
//!
//! ### Narrative Synthesis (`narratives`)
//! Automated report generation:
//! - Executive summaries
//! - Timeline reconstruction
//! - Actor attribution analysis
//! - Actionable recommendations
//!
//! ### Investigation Workflows (`workflows`)
//! Production-ready investigation workflows for common scenarios:
//! - Company Intelligence Report (`company_intelligence`)
//! - Person Deep-Dive Investigation (`person_deep_dive`)
//! - Supply Chain Threat Assessment (`supply_chain_threat`)
//! - Market Opportunity Detection (`market_opportunity`)
//! - Geopolitical Risk Assessment (`geopolitical_risk`)
//!
//! ## Usage
//!
//! ```rust
//! use apex_investigation::{
//!     investigations::{InvestigationEngine, InvestigationType},
//!     reasoning::EvidenceItem,
//! };
//! use chrono::{Duration, Utc};
//!
//! // Create investigation engine
//! let engine = InvestigationEngine::new();
//!
//! // Create a new investigation
//! let mut investigation = engine.create_investigation(
//!     "TestCorp Analysis",
//!     InvestigationType::CompanyDeepDive,
//!     "TestCorp Inc",
//!     "company",
//! );
//!
//! // Create sample evidence
//! let signals = vec![EvidenceItem {
//!     id: "sig1".to_string(),
//!     entity_id: "TestCorp".to_string(),
//!     entity_type: "company".to_string(),
//!     evidence_type: "job_posting".to_string(),
//!     description: "Expansion hiring detected".to_string(),
//!     source: "LinkedIn".to_string(),
//!     confidence: 0.9,
//!     timestamp: Utc::now() - Duration::days(5),
//!     raw_data: serde_json::json!({}),
//! }];
//!
//! // Run the investigation
//! let result = engine.run_investigation(&mut investigation, signals);
//!
//! // Access results
//! println!("Confidence: {:.0}%", result.overall_confidence * 100.0);
//! if let Some(report) = result.report {
//!     println!("Report: {}", report.executive_summary);
//! }
//! ```
//!
//! ### Workflow Usage
//!
//! ```rust
//! use apex_investigation::workflows::{
//!     CompanyIntelligenceWorkflow,
//!     PersonDeepDiveWorkflow,
//!     SupplyChainThreatWorkflow,
//!     MarketOpportunityWorkflow,
//!     GeopoliticalRiskWorkflow,
//! };
//! use chrono::Utc;
//!
//! // Company Intelligence
//! let mut company_workflow = CompanyIntelligenceWorkflow::new();
//! let company_signals = vec![];
//! let company_report = company_workflow.run("TargetCompany", company_signals);
//! println!("Company Risk Score: {:.0}%", company_report.overall_confidence * 100.0);
//!
//! // Person Deep-Dive
//! let mut person_workflow = PersonDeepDiveWorkflow::new();
//! let person_signals = vec![];
//! let person_report = person_workflow.run("John Doe", person_signals);
//! println!("Person Risk Score: {:.0}%", person_report.overall_confidence * 100.0);
//!
//! // Supply Chain Threat
//! let mut supply_chain_workflow = SupplyChainThreatWorkflow::new();
//! let supply_signals = vec![];
//! let supply_report = supply_chain_workflow.run("TargetCorp", supply_signals);
//! println!("Supply Chain Risk: {:.0}%", supply_report.overall_risk_score * 100.0);
//!
//! // Market Opportunity
//! let mut market_workflow = MarketOpportunityWorkflow::new();
//! let market_signals = vec![];
//! let market_report = market_workflow.run("Technology Market", market_signals);
//! println!("Market Opportunity: {:.0}%", market_report.overall_opportunity_score * 100.0);
//!
//! // Geopolitical Risk
//! let mut geo_workflow = GeopoliticalRiskWorkflow::new();
//! let geo_signals = vec![];
//! let geo_report = geo_workflow.run("APAC Region", geo_signals);
//! println!("Geopolitical Risk: {:.0}%", geo_report.overall_risk_score * 100.0);
//! ```
//!
//! ## Architecture
//!
//! The framework follows a pipeline architecture:
//!
//! ```text
//! Evidence/Signals
//!      |
//!      v
//! +----------------+
//! | Hypothesis Gen | --> Competing Hypotheses
//! +----------------+
//!      |
//!      v
//! +----------------+
//! | Gap Analysis   | --> Missing Information
//! +----------------+
//!      |
//!      v
//! +----------------+
//! | Reasoning Chain| --> Multi-hop Inferences
//! +----------------+
//!      |
//!      v
//! +----------------+
//! | Threat Modeling| --> Risk Assessment
//! +----------------+
//!      |
//!      v
//! +----------------+
//! | Narrative Synth| --> Final Report
//! +----------------+
//! ```
//!
//! ## Features
//!
//! - **Multi-source evidence fusion**: Combines signals from diverse OSINT sources
//! - **Bayesian hypothesis testing**: Rigorous probabilistic reasoning
//! - **Temporal analysis**: 90+ day pattern detection
//! - **Entity correlation**: Cross-entity relationship inference
//! - **Automated reporting**: Markdown and structured output formats
//! - **Configurable depth**: Adjustable reasoning depth and confidence thresholds
//! - **Production-ready workflows**: Pre-built investigation workflows for common scenarios

pub mod hypothesis;
pub mod investigations;
pub mod narratives;
pub mod reasoning;
pub mod threats;
pub mod workflows;

// Re-export commonly used types
pub use hypothesis::{
    CounterfactualAnalysis, EvidenceType, GapAnalysis, Hypothesis, HypothesisGenerator,
    HypothesisType, InvestigationPriority, SignalEvidence,
};
pub use investigations::{
    Investigation, InvestigationConfig, InvestigationEngine, InvestigationResult,
    InvestigationStatus, InvestigationType,
};
pub use narratives::{
    ActorAttribution, InvestigativeReport, NarrativeSynthesizer, Recommendation,
    RecommendationPriority, ReportSection, ReportType, SectionType, TimelineEvent,
    TimelineReconstruction,
};
pub use reasoning::{
    ChainOfThoughtReasoner, EntityCorrelation, EvidenceItem, InferenceChain, ReasoningStep,
    ReasoningType, TemporalPattern, TemporalPatternType,
};
pub use threats::{
    AdversaryCapability, AttackSurfaceAssessment, DisruptionScenario, ExposureType,
    RiskVector, ScenarioSimulation, ScenarioType, SupplyChainThreatModel, ThreatActor,
    ThreatActorType, ThreatModeling,
};

// Re-export workflow types
pub use workflows::{
    // Company Intelligence
    CompanyIntelligenceReport, CompanyIntelligenceWorkflow, CompetitorAnalysis,
    FinancialHealthAssessment, LeadershipAnalysis, StrategicOpportunity, SupplyChainRisk,
    ThreatAssessment, WorkflowResult,
    // Person Deep-Dive
    EngagementStrategy, NetworkMapping, PersonDeepDiveReport, PersonDeepDiveWorkflow,
    ProfessionalHistory, PsychologicalProfile, RiskIndicatorReport, TriggerEvent,
    // Supply Chain Threat
    AlternativeSupplier, ConcentrationRisk, FinancialStability, GeographicRisk,
    SinglePointOfFailure, SupplyChainThreatReport, SupplyChainThreatWorkflow,
    // Market Opportunity
    CompetitorWeakness, MarketOpportunityReport, MarketOpportunityWorkflow,
    MarketTiming, ProcurementSignal, StrategicEntryPoint,
    // Geopolitical Risk
    GeopoliticalRiskReport, GeopoliticalRiskWorkflow, PoliticalStability,
    RegionalRiskScore, SanctionsExposure, TradePolicyImpact,
};
