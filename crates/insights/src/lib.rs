#![allow(
    clippy::should_implement_trait,
    clippy::too_many_arguments,
    clippy::type_complexity
)]

pub mod adversarial;
pub mod arbitrage;
pub mod bias_mitigation;
pub mod cep;
pub mod cert_expiry;
pub mod cert_gap;
pub mod company_discovery;
pub mod comparison;
pub mod correlation;
pub mod cross_entity_correlation;
pub mod discovery_pipeline;
pub mod dossier;
pub mod dynamic_poi_discovery;
pub mod entity_relevance;
pub mod entity_verifier;
pub mod evidence_chain;
pub mod gap_analyzer;
pub mod generator_orchestrator;
pub mod hypothesis;
pub mod insight_feedback;
pub mod llm_enricher;
pub mod memo;
pub mod news_digest;
pub mod outcome_tracker;
pub mod predictive;
pub mod renderer;
pub mod shortage_correlation;
pub mod title_diversity;
pub mod weekly_pipeline;

// ── Re-export closed-loop feedback types for convenience ──
pub use insight_feedback::{
    FeedbackController, FeedbackEntry, FeedbackSignal, FeedbackSignalType, InsightRecord,
};
pub use generator_orchestrator::GeneratorOrchestrator;

// ── Re-export company discovery types ──
pub use company_discovery::{CompanyCandidate, DiscoverySource};
pub use discovery_pipeline::{DiscoveryConfig, DiscoveryPipeline};
pub use entity_verifier::{EntityVerifier, VerificationResult};
