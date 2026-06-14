//! Investigation management and orchestration.
//!
//! Provides:
//! - Investigation lifecycle management
//! - Investigation workflow engine
//! - Investigation result tracking

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{
    hypothesis::{GapAnalysis, Hypothesis, HypothesisGenerator, InvestigationPriority},
    narratives::{NarrativeSynthesizer, SynthesisContext},
    reasoning::{ChainOfThoughtReasoner, EvidenceItem},
    threats::ThreatModeling,
};

/// Investigation types supported by the framework.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum InvestigationType {
    /// Company intelligence deep-dive
    CompanyDeepDive,
    /// Person investigation
    PersonInvestigation,
    /// Supply chain analysis
    SupplyChainAnalysis,
    /// Threat assessment
    ThreatAssessment,
    /// Geopolitical risk assessment
    GeopoliticalRisk,
    /// Market opportunity detection
    MarketOpportunity,
    /// General investigation
    General,
}

impl InvestigationType {
    pub fn label(&self) -> &'static str {
        match self {
            InvestigationType::CompanyDeepDive => "Company Deep-Dive",
            InvestigationType::PersonInvestigation => "Person Investigation",
            InvestigationType::SupplyChainAnalysis => "Supply Chain Analysis",
            InvestigationType::ThreatAssessment => "Threat Assessment",
            InvestigationType::GeopoliticalRisk => "Geopolitical Risk",
            InvestigationType::MarketOpportunity => "Market Opportunity",
            InvestigationType::General => "General Investigation",
        }
    }
}

/// Investigation status.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum InvestigationStatus {
    /// Created, not yet started
    Created,
    /// In progress
    InProgress,
    /// Completed
    Completed,
    /// Suspended
    Suspended,
    /// Cancelled
    Cancelled,
    /// Failed
    Failed,
}

impl InvestigationStatus {
    pub fn label(&self) -> &'static str {
        match self {
            InvestigationStatus::Created => "Created",
            InvestigationStatus::InProgress => "In Progress",
            InvestigationStatus::Completed => "Completed",
            InvestigationStatus::Suspended => "Suspended",
            InvestigationStatus::Cancelled => "Cancelled",
            InvestigationStatus::Failed => "Failed",
        }
    }
}

/// Configuration for investigation engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvestigationConfig {
    /// Investigation type
    pub investigation_type: InvestigationType,
    /// Maximum reasoning depth
    pub max_reasoning_depth: usize,
    /// Minimum confidence threshold
    pub min_confidence_threshold: f64,
    /// Enable timeline reconstruction
    pub enable_timeline: bool,
    /// Enable attribution analysis
    pub enable_attribution: bool,
    /// Enable threat modeling
    pub enable_threat_modeling: bool,
    /// Maximum signals to process
    pub max_signals: usize,
    /// Temporal window in days
    pub temporal_window_days: i64,
}

impl Default for InvestigationConfig {
    fn default() -> Self {
        Self {
            investigation_type: InvestigationType::General,
            max_reasoning_depth: 5,
            min_confidence_threshold: 0.3,
            enable_timeline: true,
            enable_attribution: true,
            enable_threat_modeling: true,
            max_signals: 1000,
            temporal_window_days: 90,
        }
    }
}

/// An active investigation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Investigation {
    /// Investigation unique ID
    pub id: String,
    /// Investigation title
    pub title: String,
    /// Investigation type
    pub investigation_type: InvestigationType,
    /// Current status
    pub status: InvestigationStatus,
    /// Priority level
    pub priority: InvestigationPriority,
    /// Lead analyst (if assigned)
    pub lead_analyst: Option<String>,
    /// Target entity ID
    pub target_entity_id: Option<String>,
    /// Target entity name
    pub target_entity_name: String,
    /// Target entity type
    pub target_entity_type: String,
    /// Hypotheses generated
    pub hypotheses: Vec<Hypothesis>,
    /// Gaps identified
    pub gaps: Option<GapAnalysis>,
    /// Number of signals analyzed
    pub signals_analyzed: usize,
    /// Overall confidence score
    pub confidence_score: f64,
    /// Configuration used
    pub config: InvestigationConfig,
    /// Created timestamp
    pub created_at: DateTime<Utc>,
    /// Last updated timestamp
    pub updated_at: DateTime<Utc>,
    /// Completed timestamp (if applicable)
    pub completed_at: Option<DateTime<Utc>>,
    /// Error message (if failed)
    pub error_message: Option<String>,
    /// Investigation metadata
    pub metadata: serde_json::Value,
}

impl Investigation {
    /// Create a new investigation.
    pub fn new(
        title: &str,
        investigation_type: InvestigationType,
        target_entity_name: &str,
        target_entity_type: &str,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4().to_string(),
            title: title.to_string(),
            investigation_type,
            status: InvestigationStatus::Created,
            priority: InvestigationPriority::Medium,
            lead_analyst: None,
            target_entity_id: None,
            target_entity_name: target_entity_name.to_string(),
            target_entity_type: target_entity_type.to_string(),
            hypotheses: Vec::new(),
            gaps: None,
            signals_analyzed: 0,
            confidence_score: 0.0,
            config: InvestigationConfig::default(),
            created_at: now,
            updated_at: now,
            completed_at: None,
            error_message: None,
            metadata: serde_json::json!({}),
        }
    }

    /// Mark investigation as started.
    pub fn start(&mut self) {
        self.status = InvestigationStatus::InProgress;
        self.updated_at = Utc::now();
    }

    /// Mark investigation as completed.
    pub fn complete(&mut self, confidence_score: f64) {
        self.status = InvestigationStatus::Completed;
        self.confidence_score = confidence_score;
        self.completed_at = Some(Utc::now());
        self.updated_at = Utc::now();
    }

    /// Mark investigation as failed.
    pub fn fail(&mut self, error: &str) {
        self.status = InvestigationStatus::Failed;
        self.error_message = Some(error.to_string());
        self.updated_at = Utc::now();
    }

    /// Update investigation with new findings.
    pub fn update(&mut self, confidence_score: f64) {
        self.confidence_score = confidence_score;
        self.updated_at = Utc::now();
    }

    /// Check if investigation is terminal.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self.status,
            InvestigationStatus::Completed
                | InvestigationStatus::Cancelled
                | InvestigationStatus::Failed
        )
    }
}

/// Investigation result containing all analysis outputs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvestigationResult {
    /// Investigation ID
    pub investigation_id: String,
    /// Generated hypotheses
    pub hypotheses: Vec<Hypothesis>,
    /// Gap analysis
    pub gaps: Option<GapAnalysis>,
    /// Reasoning chains
    pub reasoning_chains: Vec<super::reasoning::InferenceChain>,
    /// Narrative report
    pub report: Option<super::narratives::InvestigativeReport>,
    /// Threat model (if applicable)
    pub threat_model: Option<super::threats::SupplyChainThreatModel>,
    /// Overall confidence
    pub overall_confidence: f64,
    /// Processing time in milliseconds
    pub processing_time_ms: u64,
    /// Signals processed
    pub signals_processed: usize,
}

/// Investigation engine orchestrating all analysis components.
#[derive(Clone)]
pub struct InvestigationEngine {
    hypothesis_generator: HypothesisGenerator,
    reasoner: ChainOfThoughtReasoner,
    threat_modeler: ThreatModeling,
    synthesizer: NarrativeSynthesizer,
}

impl Default for InvestigationEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl InvestigationEngine {
    /// Create a new investigation engine.
    pub fn new() -> Self {
        Self {
            hypothesis_generator: HypothesisGenerator::new(),
            reasoner: ChainOfThoughtReasoner::new(),
            threat_modeler: ThreatModeling::new(),
            synthesizer: NarrativeSynthesizer::new(),
        }
    }

    /// Create and start a new investigation.
    pub fn create_investigation(
        &self,
        title: &str,
        investigation_type: InvestigationType,
        target_entity_name: &str,
        target_entity_type: &str,
    ) -> Investigation {
        Investigation::new(
            title,
            investigation_type,
            target_entity_name,
            target_entity_type,
        )
    }

    /// Run a complete investigation.
    pub fn run_investigation(
        &self,
        investigation: &mut Investigation,
        signals: Vec<EvidenceItem>,
    ) -> InvestigationResult {
        let start_time = std::time::Instant::now();

        // Mark as in progress
        investigation.start();

        // Limit signals
        let signals: Vec<_> = signals
            .into_iter()
            .take(investigation.config.max_signals)
            .collect();
        investigation.signals_analyzed = signals.len();

        // Convert signals to hypothesis evidence
        let evidence = signals
            .iter()
            .map(|s| super::hypothesis::SignalEvidence {
                signal_id: s.id.clone(),
                signal_type: s.evidence_type.clone(),
                entity_id: s.entity_id.clone(),
                entity_name: s.entity_type.clone(),
                confidence: s.confidence,
                observed_at: s.timestamp,
                source: s.source.clone(),
                raw_data: s.raw_data.clone(),
            })
            .collect::<Vec<_>>();

        // Step 1: Generate hypotheses
        let hypotheses = self
            .hypothesis_generator
            .generate_hypotheses(
                &investigation.target_entity_name,
                &investigation.target_entity_name,
                &evidence,
            );

        // Step 2: Analyze gaps (build profile from signals)
        let entity_profile = super::hypothesis::EntityDataProfile::from_signals(&signals);
        let gaps = self
            .hypothesis_generator
            .analyze_gaps(&investigation.target_entity_name, &entity_profile);

        // Step 3: Build reasoning chains
        let evidence_refs: Vec<_> = signals
            .iter()
            .map(|e| super::reasoning::EvidenceItem {
                id: e.id.clone(),
                entity_id: e.entity_id.clone(),
                entity_type: e.entity_type.clone(),
                evidence_type: e.evidence_type.clone(),
                description: e.description.clone(),
                source: e.source.clone(),
                confidence: e.confidence,
                timestamp: e.timestamp,
                raw_data: e.raw_data.clone(),
            })
            .collect();

        let reasoner = self
            .reasoner
            .clone()
            .with_max_depth(investigation.config.max_reasoning_depth)
            .with_min_confidence(investigation.config.min_confidence_threshold)
            .with_temporal_window(investigation.config.temporal_window_days);

        let reasoning_chains = vec![reasoner.build_chain(
            &investigation.target_entity_name,
            &investigation.target_entity_type,
            &evidence_refs,
        )];

        // Step 4: Generate narrative report
        let synthesis_context = SynthesisContext {
            report_type: match investigation.investigation_type {
                InvestigationType::CompanyDeepDive => super::narratives::ReportType::CompanyIntelligence,
                InvestigationType::PersonInvestigation => super::narratives::ReportType::PersonDeepDive,
                InvestigationType::SupplyChainAnalysis => {
                    super::narratives::ReportType::SupplyChainThreat
                }
                InvestigationType::ThreatAssessment => {
                    super::narratives::ReportType::CompetitiveIntel
                }
                InvestigationType::GeopoliticalRisk => {
                    super::narratives::ReportType::GeopoliticalRisk
                }
                InvestigationType::MarketOpportunity => {
                    super::narratives::ReportType::MarketOpportunity
                }
                InvestigationType::General => super::narratives::ReportType::General,
            },
            title: investigation.title.clone(),
            entities: vec![investigation.target_entity_name.clone()],
            signals: evidence
                .iter()
                .map(|e| super::narratives::SignalInfo {
                    signal_type: e.signal_type.clone(),
                    description: format!("{} signal for {}", e.signal_type, e.entity_name),
                    confidence: e.confidence,
                    source: e.source.clone(),
                })
                .collect(),
            findings: hypotheses
                .iter()
                .map(|h| h.description.clone())
                .collect(),
            primary_conclusion: reasoning_chains
                .first()
                .map(|c| c.conclusion.clone()),
            risk_level: None,
            threat_summary: None,
            relationships: vec![],
            events: vec![],
            sources: evidence.iter().map(|e| e.source.clone()).collect(),
        };

        let synthesizer = self
            .synthesizer
            .clone()
            .with_timeline(investigation.config.enable_timeline)
            .with_attribution(investigation.config.enable_attribution);

        let report = synthesizer.synthesize(&synthesis_context);

        // Calculate overall confidence
        let overall_confidence = if !reasoning_chains.is_empty() {
            let chain_confidence = reasoning_chains
                .iter()
                .map(|c| c.overall_confidence)
                .sum::<f64>()
                / reasoning_chains.len() as f64;
            let hypothesis_confidence = if !hypotheses.is_empty() {
                hypotheses.iter().map(|h| h.posterior).sum::<f64>() / hypotheses.len() as f64
            } else {
                0.0
            };
            (chain_confidence + hypothesis_confidence) / 2.0
        } else {
            0.0
        };

        // Update investigation
        investigation.hypotheses = hypotheses.clone();
        investigation.gaps = Some(gaps.clone());
        investigation.complete(overall_confidence);

        let processing_time = start_time.elapsed().as_millis() as u64;

        // Step 5: Generate supply chain threat model
        let threat_model = if investigation.config.enable_threat_modeling {
            Some(self.threat_modeler.model_supply_chain(
                &investigation.target_entity_name,
                &[],
            ))
        } else {
            None
        };

        InvestigationResult {
            investigation_id: investigation.id.clone(),
            hypotheses,
            gaps: Some(gaps),
            reasoning_chains,
            report: Some(report),
            threat_model,
            overall_confidence,
            processing_time_ms: processing_time,
            signals_processed: investigation.signals_analyzed,
        }
    }

    /// Run investigation asynchronously, offloading the sync pipeline via `spawn_blocking`.
    ///
    /// Clones the investigation so the sync pipeline can run on a dedicated blocking thread,
    /// preventing stalls on the async runtime. After completion, the original investigation
    /// is updated with the results from the clone.
    pub async fn run_investigation_async(
        &self,
        investigation: &mut Investigation,
        signals: Vec<EvidenceItem>,
    ) -> InvestigationResult {
        let this = self.clone();
        let mut inv_clone = investigation.clone();

        // Return the mutated clone alongside the result so we can copy fields back
        let result: Result<(InvestigationResult, Investigation), _> =
            tokio::task::spawn_blocking(move || {
                let res = this.run_investigation(&mut inv_clone, signals);
                (res, inv_clone)
            })
            .await;

        match result {
            Ok((res, inv_after)) => {
                // Copy back the mutated investigation state
                investigation.hypotheses = inv_after.hypotheses;
                investigation.gaps = inv_after.gaps;
                investigation.signals_analyzed = inv_after.signals_analyzed;
                investigation.confidence_score = inv_after.confidence_score;
                investigation.status = inv_after.status;
                investigation.updated_at = inv_after.updated_at;
                investigation.completed_at = inv_after.completed_at;
                investigation.error_message = inv_after.error_message;
                res
            }
            Err(join_err) => {
                eprintln!("Investigation panicked: {}", join_err);
                InvestigationResult {
                    investigation_id: investigation.id.clone(),
                    hypotheses: vec![],
                    gaps: None,
                    reasoning_chains: vec![],
                    report: None,
                    threat_model: None,
                    overall_confidence: 0.0,
                    processing_time_ms: 0,
                    signals_processed: 0,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::disallowed_methods)]
    use super::*;
    use chrono::Duration;

    #[test]
    fn test_investigation_creation() {
        let investigation = Investigation::new(
            "Test Investigation",
            InvestigationType::CompanyDeepDive,
            "TestCorp",
            "company",
        );

        assert_eq!(investigation.title, "Test Investigation");
        assert_eq!(investigation.status, InvestigationStatus::Created);
        assert!(investigation.hypotheses.is_empty());
    }

    #[test]
    fn test_investigation_lifecycle() {
        let mut investigation = Investigation::new(
            "Test",
            InvestigationType::General,
            "TestCorp",
            "company",
        );

        investigation.start();
        assert_eq!(investigation.status, InvestigationStatus::InProgress);

        investigation.complete(0.85);
        assert_eq!(investigation.status, InvestigationStatus::Completed);
        assert_eq!(investigation.confidence_score, 0.85);
        assert!(investigation.completed_at.is_some());
    }

    #[test]
    fn test_investigation_engine() {
        let engine = InvestigationEngine::new();
        let mut investigation = engine.create_investigation(
            "Test Investigation",
            InvestigationType::CompanyDeepDive,
            "TestCorp",
            "company",
        );

        // Create sample signals
        let signals = vec![
            EvidenceItem {
                id: "sig1".to_string(),
                entity_id: "TestCorp".to_string(),
                entity_type: "company".to_string(),
                evidence_type: "job_posting".to_string(),
                description: "New hiring detected".to_string(),
                source: "LinkedIn".to_string(),
                confidence: 0.9,
                timestamp: Utc::now() - Duration::days(5),
                raw_data: serde_json::json!({}),
            },
            EvidenceItem {
                id: "sig2".to_string(),
                entity_id: "TestCorp".to_string(),
                entity_type: "company".to_string(),
                evidence_type: "patent_filing".to_string(),
                description: "Patent filed".to_string(),
                source: "USPTO".to_string(),
                confidence: 0.85,
                timestamp: Utc::now() - Duration::days(10),
                raw_data: serde_json::json!({}),
            },
        ];

        let result = engine.run_investigation(&mut investigation, signals);

        assert_eq!(result.investigation_id, investigation.id);
        assert!(result.signals_processed == 2);
    }

    #[test]
    fn test_investigation_result() {
        let engine = InvestigationEngine::new();
        let mut investigation = engine.create_investigation(
            "Empty Test",
            InvestigationType::General,
            "EmptyCorp",
            "company",
        );

        let result = engine.run_investigation(&mut investigation, vec![]);

        assert!(result.investigation_id == investigation.id);
        // With no signals, confidence should be low but result should still be valid
        assert!(result.overall_confidence >= 0.0);
    }
}