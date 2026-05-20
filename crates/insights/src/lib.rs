//!
//! Intelligence Insights Generation for ApexIntel OSINT platform.
//!
//! Generates actionable intelligence insights from collected data:
//! - Pattern detection and anomaly identification
//! - Trend analysis and forecasting
//! - Risk summarization and alerts
//! - Relationship mapping insights
//!
//! Part of Phase 2.1: LLM Integration for ApexIntel OSINT platform.
//! Phase 4.1: Deep Insight Generation Quality
//! Phase 4.2: Intelligence Quality Assurance

pub mod analysis;
pub mod patterns;
pub mod risk_summarizer;
pub mod trend_analyzer;

// Phase 4.1: Deep Insight Generation
pub mod deep_insight;

// Phase 4.2: Quality Assurance
pub mod quality_assurance;

// Discovery & Entity modules
pub mod company_discovery;
pub mod entity_relevance;
pub mod entity_verifier;
pub mod title_diversity;

// Intelligence pipeline modules
pub mod hypothesis;
pub mod predictive;
pub mod renderer;
pub mod memo;
pub mod weekly_pipeline;
pub mod arbitrage;
pub mod bias_mitigation;
pub mod comparison;

// Supporting modules
pub mod adversarial;
pub mod cep;
pub mod cert_expiry;
pub mod cert_gap;
pub mod correlation;
pub mod cross_entity_correlation;
pub mod cross_entity_intelligence;
pub mod discovery_pipeline;
pub mod dossier;
pub mod dynamic_poi_discovery;
pub mod evidence_chain;
pub mod gap_analyzer;
pub mod generator_orchestrator;
pub mod insight_feedback;
pub mod llm_enricher;
pub mod news_digest;
pub mod outcome_tracker;
pub mod shortage_correlation;

use serde::{Deserialize, Serialize};

/// Insight severity levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InsightSeverity {
    Critical,
    High,
    Medium,
    Low,
    Info,
}

impl InsightSeverity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Critical => "critical",
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
            Self::Info => "info",
        }
    }

    pub fn priority(&self) -> u8 {
        match self {
            Self::Critical => 5,
            Self::High => 4,
            Self::Medium => 3,
            Self::Low => 2,
            Self::Info => 1,
        }
    }
}

/// Base insight structure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Insight {
    /// Unique insight ID.
    pub id: String,
    /// Insight title.
    pub title: String,
    /// Detailed description.
    pub description: String,
    /// Severity level.
    pub severity: InsightSeverity,
    /// Confidence in insight (0.0-1.0).
    pub confidence: f64,
    /// Entities involved.
    pub entities: Vec<String>,
    /// Related sources.
    pub sources: Vec<String>,
    /// Tags for categorization.
    pub tags: Vec<String>,
}

impl Insight {
    pub fn new(title: &str, description: &str) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            title: title.to_string(),
            description: description.to_string(),
            severity: InsightSeverity::Medium,
            confidence: 0.5,
            entities: vec![],
            sources: vec![],
            tags: vec![],
        }
    }

    pub fn with_severity(mut self, severity: InsightSeverity) -> Self {
        self.severity = severity;
        self
    }

    pub fn with_confidence(mut self, confidence: f64) -> Self {
        self.confidence = confidence.clamp(0.0, 1.0);
        self
    }

    pub fn with_entities(mut self, entities: Vec<String>) -> Self {
        self.entities = entities;
        self
    }

    pub fn with_sources(mut self, sources: Vec<String>) -> Self {
        self.sources = sources;
        self
    }

    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }
}

/// Insight generation result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InsightResult {
    pub insights: Vec<Insight>,
    pub summary: String,
    pub generation_time_ms: u64,
    pub model_used: String,
}

impl InsightResult {
    pub fn sorted_by_severity(&self) -> Vec<&Insight> {
        let mut insights: Vec<&Insight> = self.insights.iter().collect();
        insights.sort_by(|a, b| {
            b.severity
                .priority()
                .cmp(&a.severity.priority())
                .then_with(|| b.confidence.partial_cmp(&a.confidence).unwrap())
        });
        insights
    }

    pub fn critical_insights(&self) -> Vec<&Insight> {
        self.insights
            .iter()
            .filter(|i| i.severity == InsightSeverity::Critical)
            .collect()
    }

    pub fn high_confidence_insights(&self) -> Vec<&Insight> {
        self.insights
            .iter()
            .filter(|i| i.confidence >= 0.8)
            .collect()
    }
}

// Re-export key types for convenience
pub use analysis::{IntelligenceAnalyzer, IntelligenceAnalyzerConfig, AnalysisInput};
pub use patterns::{PatternDetector, PatternDetectorConfig, PatternType, PatternData};
pub use risk_summarizer::{RiskSummarizer, RiskSummarizerConfig, RiskSummary};
pub use trend_analyzer::{TrendAnalyzer, TrendAnalyzerConfig};

// Phase 4.1: Deep Insight exports
pub use deep_insight::{
    AuthorityTier, DeepEvidence, DeepInsight, DeepInsightConfig, DeepInsightGenerator,
    FreshnessTier, InsightSeverityLevel, RiskCategory, RiskFactor,
    ActionPriority, ActionRecommendation, ActionRisk,
};

// Phase 4.2: Quality Assurance exports
pub use quality_assurance::{
    BiasConfig, BiasContent, BiasDetector, BiasIndicator, BiasInstance, BiasReport, BiasType,
    CredibilityConfig, CredibilityFlag, CredibilityScore, ErrorSeverity,
    ImprovementConfig, OutputValidator, PatternOfLifeAnalysis, QualityFeedback,
    QualityFeedbackType, QualityStats, RecipeRefinement, ScoredEvidence,
    ThresholdAdjustment, ValidationConfig, ValidationContent, ValidationError,
    ValidationResult, ValidationStatus, ValidationWarning,
    SourceCredibilityScorer, QualityImprovementEngine,
};
