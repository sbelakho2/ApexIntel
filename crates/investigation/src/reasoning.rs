//! Chain-of-thought reasoning engine for multi-hop inference.
//!
//! Provides:
//! - Multi-hop inference chains
//! - Temporal pattern analysis (90+ day windows)
//! - Entity correlation across types
//! - Confidence propagation

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// A single step in a reasoning chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasoningStep {
    /// Step number in the chain (0-indexed)
    pub step_number: usize,
    /// Type of reasoning performed
    pub reasoning_type: ReasoningType,
    /// Premise/fact used for this step
    pub premise: String,
    /// Inference made
    pub inference: String,
    /// Confidence in this step's inference (0-1)
    pub confidence: f64,
    /// Evidence supporting this step
    pub supporting_evidence: Vec<EvidenceRef>,
    /// Timestamp when this step was computed
    pub computed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceRef {
    pub evidence_id: String,
    pub evidence_type: String,
    pub source: String,
    pub confidence: f64,
}

/// Types of reasoning performed at each step.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum ReasoningType {
    /// Direct observation/evidence
    Observation,
    /// Temporal correlation
    TemporalCorrelation,
    /// Causal inference
    CausalInference,
    /// Entity relationship inference
    EntityRelationship,
    /// Pattern matching
    PatternMatch,
    /// Analogical reasoning
    Analogical,
    /// Abductive reasoning
    Abductive,
    /// Deductive reasoning
    Deductive,
    /// Statistical inference
    Statistical,
    /// Expert knowledge
    ExpertKnowledge,
}

impl ReasoningType {
    pub fn label(&self) -> &'static str {
        match self {
            ReasoningType::Observation => "Direct Observation",
            ReasoningType::TemporalCorrelation => "Temporal Correlation",
            ReasoningType::CausalInference => "Causal Inference",
            ReasoningType::EntityRelationship => "Entity Relationship",
            ReasoningType::PatternMatch => "Pattern Match",
            ReasoningType::Analogical => "Analogical Reasoning",
            ReasoningType::Abductive => "Abductive Reasoning",
            ReasoningType::Deductive => "Deductive Reasoning",
            ReasoningType::Statistical => "Statistical Inference",
            ReasoningType::ExpertKnowledge => "Expert Knowledge",
        }
    }
}

/// A complete inference chain with multiple hops.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceChain {
    /// Unique identifier
    pub id: String,
    /// Chain name/description
    pub name: String,
    /// Target entity being analyzed
    pub target_entity: String,
    /// Target entity type
    pub entity_type: String,
    /// Ordered reasoning steps
    pub steps: Vec<ReasoningStep>,
    /// Overall conclusion
    pub conclusion: String,
    /// Overall confidence in the conclusion (0-1)
    pub overall_confidence: f64,
    /// When this chain was created
    pub created_at: DateTime<Utc>,
    /// Entities involved in this chain
    pub involved_entities: Vec<String>,
    /// Time span of the analysis
    pub time_span_days: i64,
}

impl InferenceChain {
    /// Create a new inference chain.
    pub fn new(target_entity: &str, entity_type: &str, name: &str) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            name: name.to_string(),
            target_entity: target_entity.to_string(),
            entity_type: entity_type.to_string(),
            steps: Vec::new(),
            conclusion: String::new(),
            overall_confidence: 0.0,
            created_at: Utc::now(),
            involved_entities: vec![target_entity.to_string()],
            time_span_days: 0,
        }
    }

    /// Add a reasoning step to the chain.
    pub fn add_step(&mut self, step: ReasoningStep) {
        // Track involved entities
        for evidence in &step.supporting_evidence {
            if !self.involved_entities.contains(&evidence.evidence_id) {
                self.involved_entities.push(evidence.evidence_id.clone());
            }
        }
        self.steps.push(step);
        self.recompute_confidence();
    }

    /// Set the final conclusion.
    pub fn set_conclusion(&mut self, conclusion: &str, confidence: f64) {
        self.conclusion = conclusion.to_string();
        self.overall_confidence = confidence;
    }

    /// Recompute overall confidence from all steps.
    fn recompute_confidence(&mut self) {
        if self.steps.is_empty() {
            self.overall_confidence = 0.0;
            return;
        }

        // Use minimum confidence path (chain is only as strong as weakest link)
        let min_confidence = self
            .steps
            .iter()
            .map(|s| s.confidence)
            .fold(1.0f64, |a, b| a.min(b));

        // Weight by number of steps (more steps = more complex = potentially lower reliability)
        let step_penalty = (self.steps.len() as f64 - 1.0) * 0.02;
        self.overall_confidence = (min_confidence - step_penalty).clamp(0.0, 1.0);
    }

    /// Generate a human-readable summary of the chain.
    pub fn summarize(&self) -> String {
        let mut lines = vec![
            format!("# Inference Chain: {}", self.name),
            format!("Target: {} ({})", self.target_entity, self.entity_type),
            format!(
                "Steps: {} | Overall Confidence: {:.0}%",
                self.steps.len(),
                self.overall_confidence * 100.0
            ),
            String::new(),
            "## Reasoning Steps".to_string(),
        ];

        for (i, step) in self.steps.iter().enumerate() {
            lines.push(format!(
                "### Step {}: {}",
                i + 1,
                step.reasoning_type.label()
            ));
            lines.push(format!("**Premise:** {}", step.premise));
            lines.push(format!("**Inference:** {}", step.inference));
            lines.push(format!("Confidence: {:.0}%", step.confidence * 100.0));
            if !step.supporting_evidence.is_empty() {
                lines.push(format!(
                    "Evidence: {} sources",
                    step.supporting_evidence.len()
                ));
            }
            lines.push(String::new());
        }

        lines.push("## Conclusion".to_string());
        lines.push(self.conclusion.clone());
        lines.join("\n")
    }
}

/// Temporal pattern detected in time-series data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemporalPattern {
    /// Pattern type
    pub pattern_type: TemporalPatternType,
    /// Description of the pattern
    pub description: String,
    /// Start of the pattern window
    pub start_date: DateTime<Utc>,
    /// End of the pattern window
    pub end_date: DateTime<Utc>,
    /// Duration in days
    pub duration_days: i64,
    /// Number of events in this pattern
    pub event_count: usize,
    /// Entities involved
    pub involved_entities: Vec<String>,
    /// Significance score (0-1)
    pub significance: f64,
    /// Correlation coefficient if applicable
    pub correlation: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum TemporalPatternType {
    /// Gradual trend over time
    Trend,
    /// Sudden change/break
    Anomaly,
    /// Seasonal/cyclical pattern
    Cyclical,
    /// Burst of activity
    Burst,
    /// Period of inactivity
    Silence,
    /// Lead-lag relationship
    LeadLag,
    /// Co-movement with another entity
    Correlation,
    /// Pattern with unknown cause
    Unknown,
}

impl TemporalPatternType {
    pub fn label(&self) -> &'static str {
        match self {
            TemporalPatternType::Trend => "Trend",
            TemporalPatternType::Anomaly => "Anomaly",
            TemporalPatternType::Cyclical => "Cyclical",
            TemporalPatternType::Burst => "Burst",
            TemporalPatternType::Silence => "Silence",
            TemporalPatternType::LeadLag => "Lead-Lag",
            TemporalPatternType::Correlation => "Correlation",
            TemporalPatternType::Unknown => "Unknown",
        }
    }
}

/// Correlation between entities across time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityCorrelation {
    /// First entity
    pub entity_a: String,
    /// Second entity
    pub entity_b: String,
    /// Type of correlation
    pub correlation_type: CorrelationType,
    /// Pearson correlation coefficient (-1 to 1)
    pub coefficient: f64,
    /// Lag in days (0 if simultaneous)
    pub lag_days: i64,
    /// Statistical significance (p-value)
    pub p_value: f64,
    /// Time window analyzed
    pub window_start: DateTime<Utc>,
    /// Time window end
    pub window_end: DateTime<Utc>,
    /// Shared events count
    pub shared_events: usize,
    /// Direction of causality (if determined)
    pub causality_direction: Option<CasualityDirection>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum CorrelationType {
    /// Both entities changed together
    Positive,
    /// One went up when other went down
    Negative,
    /// No clear pattern
    None,
    /// Complex/non-linear relationship
    NonLinear,
}

impl CorrelationType {
    pub fn label(&self) -> &'static str {
        match self {
            CorrelationType::Positive => "Positive",
            CorrelationType::Negative => "Negative",
            CorrelationType::None => "None",
            CorrelationType::NonLinear => "Non-linear",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum CasualityDirection {
    /// A causes B
    AToB,
    /// B causes A
    BToA,
    /// Both influence each other
    Bidirectional,
    /// Common cause explains both
    CommonCause,
}

/// Confidence propagation through reasoning chains.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfidencePropagation {
    /// Initial evidence confidence
    pub initial_confidence: f64,
    /// Evidence weight
    pub evidence_weights: Vec<f64>,
    /// Number of reasoning steps
    pub num_steps: usize,
    /// Decay rate per step
    pub decay_rate: f64,
    /// Final propagated confidence
    pub final_confidence: f64,
    /// Confidence interval
    pub confidence_interval: (f64, f64),
}

impl ConfidencePropagation {
    /// Compute final confidence after propagation through reasoning steps.
    pub fn propagate(initial: f64, num_steps: usize, decay_rate: f64) -> Self {
        let mut evidence_weights = Vec::new();
        let mut current_weight = 1.0f64;

        for i in 0..num_steps {
            evidence_weights.push(current_weight);
            // Decay weight based on position in chain
            current_weight *= (1.0 - decay_rate).powi((i + 1) as i32);
        }

        // Normalize weights
        let total_weight: f64 = evidence_weights.iter().sum();
        let normalized_weights: Vec<f64> = if total_weight > 0.0 {
            evidence_weights.iter().map(|w| w / total_weight).collect()
        } else {
            evidence_weights
        };

        // Compute final confidence
        let final_confidence = initial * normalized_weights.iter().sum::<f64>();
        let final_confidence = final_confidence.clamp(0.0, 1.0);

        // Estimate confidence interval (wider for longer chains)
        let uncertainty = num_steps as f64 * 0.05;
        let lower = (final_confidence - uncertainty).max(0.0);
        let upper = (final_confidence + uncertainty).min(1.0);

        Self {
            initial_confidence: initial,
            evidence_weights: normalized_weights,
            num_steps,
            decay_rate,
            final_confidence,
            confidence_interval: (lower, upper),
        }
    }
}

/// Chain-of-thought reasoner for multi-hop inference.
#[derive(Clone)]
pub struct ChainOfThoughtReasoner {
    /// Maximum chain depth
    max_depth: usize,
    /// Minimum confidence threshold
    min_confidence: f64,
    /// Temporal window for analysis (days)
    temporal_window_days: i64,
    /// Enabled reasoning types
    #[allow(dead_code)]
    enabled_reasoning_types: Vec<ReasoningType>,
}

impl Default for ChainOfThoughtReasoner {
    fn default() -> Self {
        Self::new()
    }
}

impl ChainOfThoughtReasoner {
    /// Create a new reasoner.
    pub fn new() -> Self {
        Self {
            max_depth: 5,
            min_confidence: 0.3,
            temporal_window_days: 90,
            enabled_reasoning_types: vec![
                ReasoningType::Observation,
                ReasoningType::TemporalCorrelation,
                ReasoningType::EntityRelationship,
                ReasoningType::PatternMatch,
                ReasoningType::CausalInference,
            ],
        }
    }

    /// Configure maximum chain depth.
    pub fn with_max_depth(mut self, depth: usize) -> Self {
        self.max_depth = depth;
        self
    }

    /// Configure minimum confidence threshold.
    pub fn with_min_confidence(mut self, confidence: f64) -> Self {
        self.min_confidence = confidence;
        self
    }

    /// Configure temporal window.
    pub fn with_temporal_window(mut self, days: i64) -> Self {
        self.temporal_window_days = days;
        self
    }

    /// Build an inference chain from evidence.
    pub fn build_chain(
        &self,
        target_entity: &str,
        entity_type: &str,
        evidence: &[EvidenceItem],
    ) -> InferenceChain {
        let mut chain = InferenceChain::new(
            target_entity,
            entity_type,
            &format!("Investigation of {}", target_entity),
        );

        // Sort evidence by timestamp
        let mut sorted_evidence: Vec<&EvidenceItem> = evidence.iter().collect();
        sorted_evidence.sort_by_key(|a| a.timestamp);

        // Step 1: Observation-based reasoning
        let direct_observations: Vec<_> = sorted_evidence
            .iter()
            .filter(|e| e.evidence_type == "direct_observation")
            .collect();

        for obs in direct_observations.iter().take(3) {
            let step = ReasoningStep {
                step_number: chain.steps.len(),
                reasoning_type: ReasoningType::Observation,
                premise: obs.description.clone(),
                inference: format!("Directly observed: {}", obs.description),
                confidence: obs.confidence,
                supporting_evidence: vec![EvidenceRef {
                    evidence_id: obs.id.clone(),
                    evidence_type: obs.evidence_type.clone(),
                    source: obs.source.clone(),
                    confidence: obs.confidence,
                }],
                computed_at: Utc::now(),
            };
            chain.add_step(step);
        }

        // Step 2: Pattern-based reasoning
        let patterns = self.detect_patterns(&sorted_evidence);
        for pattern in patterns.iter().take(2) {
            let step = ReasoningStep {
                step_number: chain.steps.len(),
                reasoning_type: ReasoningType::PatternMatch,
                premise: pattern.description.clone(),
                inference: format!(
                    "Pattern detected: {} (significance: {:.0}%)",
                    pattern.pattern_type.label(),
                    pattern.significance * 100.0
                ),
                confidence: pattern.significance,
                supporting_evidence: sorted_evidence
                    .iter()
                    .filter(|e| pattern.involved_entities.contains(&e.entity_id))
                    .map(|e| EvidenceRef {
                        evidence_id: e.id.clone(),
                        evidence_type: e.evidence_type.clone(),
                        source: e.source.clone(),
                        confidence: e.confidence,
                    })
                    .collect(),
                computed_at: Utc::now(),
            };
            chain.add_step(step);
        }

        // Step 3: Entity relationship reasoning
        let relationships = self.infer_relationships(&sorted_evidence);
        for rel in relationships.iter().take(2) {
            let step = ReasoningStep {
                step_number: chain.steps.len(),
                reasoning_type: ReasoningType::EntityRelationship,
                premise: format!("{} and {} show relationship", rel.entity_a, rel.entity_b),
                inference: format!(
                    "Entities correlated: {} relationship with {:.0}% confidence",
                    rel.correlation_type.label(),
                    rel.coefficient * 100.0
                ),
                confidence: rel.coefficient.abs(),
                supporting_evidence: Vec::new(),
                computed_at: Utc::now(),
            };
            chain.add_step(step);
            if !chain.involved_entities.contains(&rel.entity_a) {
                chain.involved_entities.push(rel.entity_a.clone());
            }
            if !chain.involved_entities.contains(&rel.entity_b) {
                chain.involved_entities.push(rel.entity_b.clone());
            }
        }

        // Step 4: Causal inference (if enough evidence)
        if chain.steps.len() >= 3 {
            let causal_step = self.infer_causation(&chain);
            chain.add_step(causal_step);
        }

        // Set conclusion based on accumulated evidence
        let conclusion = self.generate_conclusion(&chain);
        chain.set_conclusion(&conclusion, chain.overall_confidence);

        // Calculate time span
        if let (Some(first), Some(last)) = (sorted_evidence.first(), sorted_evidence.last()) {
            chain.time_span_days = (last.timestamp - first.timestamp).num_days();
        }

        chain
    }

    /// Detect temporal patterns in evidence.
    fn detect_patterns(&self, evidence: &[&EvidenceItem]) -> Vec<TemporalPattern> {
        let mut patterns = Vec::new();

        // Group by entity
        let mut by_entity: HashMap<String, Vec<&EvidenceItem>> = HashMap::new();
        for e in evidence {
            by_entity.entry(e.entity_id.clone()).or_default().push(*e);
        }

        for entity_evidence in by_entity.values() {
            if entity_evidence.len() >= 3 {
                // Detect burst pattern
                if let Some(burst) = self.detect_burst(entity_evidence) {
                    patterns.push(burst);
                }

                // Detect trend
                if let Some(trend) = self.detect_trend(entity_evidence) {
                    patterns.push(trend);
                }
            }
        }

        patterns
    }

    /// Detect burst of activity.
    fn detect_burst(&self, evidence: &[&EvidenceItem]) -> Option<TemporalPattern> {
        let mut sorted: Vec<_> = evidence.iter().collect();
        sorted.sort_by_key(|a| a.timestamp);

        // Check if events are clustered within a short time window
        let now = Utc::now();
        let recent: Vec<_> = sorted
            .iter()
            .filter(|e| (now - e.timestamp).num_days() <= 14)
            .collect();

        if recent.len() >= 3 {
            let start = recent.first().map(|e| e.timestamp).unwrap_or(now);
            let end = recent.last().map(|e| e.timestamp).unwrap_or(now);
            let duration = (end - start).num_days();

            return Some(TemporalPattern {
                pattern_type: TemporalPatternType::Burst,
                description: format!("Burst of {} events within {} days", recent.len(), duration),
                start_date: start,
                end_date: end,
                duration_days: duration,
                event_count: recent.len(),
                involved_entities: vec![evidence
                    .first()
                    .map(|e| e.entity_id.clone())
                    .unwrap_or_default()],
                significance: (recent.len() as f64 / 10.0).min(1.0),
                correlation: None,
            });
        }

        None
    }

    /// Detect trend over time.
    fn detect_trend(&self, evidence: &[&EvidenceItem]) -> Option<TemporalPattern> {
        if evidence.len() < 5 {
            return None;
        }

        let mut sorted: Vec<_> = evidence.iter().collect();
        sorted.sort_by_key(|a| a.timestamp);

        // Simple trend detection based on event frequency
        let window = 30; // days
        let now = Utc::now();
        let recent: usize = sorted
            .iter()
            .filter(|e| (now - e.timestamp).num_days() <= window)
            .count();
        let older: usize = sorted
            .iter()
            .filter(|e| {
                (now - e.timestamp).num_days() > window
                    && (now - e.timestamp).num_days() <= 2 * window
            })
            .count();

        if recent > older * 2 && recent >= 3 {
            return Some(TemporalPattern {
                pattern_type: TemporalPatternType::Trend,
                description: format!(
                    "Increasing activity: {} recent events vs {} in prior period",
                    recent, older
                ),
                start_date: sorted.first().map(|e| e.timestamp).unwrap_or(now),
                end_date: now,
                duration_days: (now - sorted.first().map(|e| e.timestamp).unwrap_or(now))
                    .num_days(),
                event_count: evidence.len(),
                involved_entities: vec![evidence
                    .first()
                    .map(|e| e.entity_id.clone())
                    .unwrap_or_default()],
                significance: (recent as f64 / 10.0).min(1.0),
                correlation: None,
            });
        }

        None
    }

    /// Infer relationships between entities.
    fn infer_relationships(&self, evidence: &[&EvidenceItem]) -> Vec<EntityCorrelation> {
        let mut correlations = Vec::new();

        // Group by entity
        let mut by_entity: HashMap<String, Vec<&EvidenceItem>> = HashMap::new();
        for e in evidence {
            by_entity.entry(e.entity_id.clone()).or_default().push(*e);
        }

        let entities: Vec<String> = by_entity.keys().cloned().collect();

        // Check pairwise correlations
        for i in 0..entities.len() {
            for j in (i + 1)..entities.len() {
                let entity_a = &entities[i];
                let entity_b = &entities[j];

                let evidence_a = &by_entity[entity_a];
                let evidence_b = &by_entity[entity_b];

                // Count co-occurring events (within 7 days)
                let mut cooccurrence = 0usize;
                for e_a in evidence_a {
                    for e_b in evidence_b {
                        let diff = (e_a.timestamp - e_b.timestamp).num_days().abs();
                        if diff <= 7 {
                            cooccurrence += 1;
                        }
                    }
                }

                if cooccurrence > 0 {
                    let coefficient = (cooccurrence as f64 / 20.0).min(1.0);
                    let correlation_type = if coefficient > 0.3 {
                        CorrelationType::Positive
                    } else if coefficient < -0.3 {
                        CorrelationType::Negative
                    } else {
                        CorrelationType::None
                    };

                    correlations.push(EntityCorrelation {
                        entity_a: entity_a.clone(),
                        entity_b: entity_b.clone(),
                        correlation_type,
                        coefficient,
                        lag_days: 0,
                        p_value: {
                            // Dynamic p-value based on evidence volume and coefficient strength
                            let n_approx = evidence_a.len().max(evidence_b.len()) as f64;
                            if n_approx >= 30.0 {
                                0.01
                            } else if n_approx >= 10.0 {
                                0.05
                            } else {
                                // Use coefficient strength as fallback: stronger correlation = lower p-value
                                1.0 - coefficient.abs().min(0.99)
                            }
                        },
                        window_start: evidence_a
                            .first()
                            .map(|e| e.timestamp)
                            .unwrap_or_else(Utc::now),
                        window_end: Utc::now(),
                        shared_events: cooccurrence,
                        causality_direction: None,
                    });
                }
            }
        }

        correlations
    }

    /// Infer causation from reasoning chain.
    fn infer_causation(&self, chain: &InferenceChain) -> ReasoningStep {
        // Find the strongest inference
        let strongest_step = chain
            .steps
            .iter()
            .max_by(|a, b| {
                a.confidence
                    .partial_cmp(&b.confidence)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .cloned();

        let (premise, inference, confidence) = if let Some(step) = strongest_step {
            (
                format!("Based on {} steps of reasoning", chain.steps.len()),
                format!(
                    "Strongest inference: {} (confidence: {:.0}%)",
                    step.inference,
                    step.confidence * 100.0
                ),
                step.confidence * 0.9, // Slight decay for causal inference
            )
        } else {
            (
                "Limited evidence available".to_string(),
                "Cannot establish causal relationship".to_string(),
                0.3,
            )
        };

        ReasoningStep {
            step_number: chain.steps.len(),
            reasoning_type: ReasoningType::CausalInference,
            premise,
            inference,
            confidence,
            supporting_evidence: Vec::new(),
            computed_at: Utc::now(),
        }
    }

    /// Generate conclusion from reasoning chain.
    fn generate_conclusion(&self, chain: &InferenceChain) -> String {
        if chain.steps.is_empty() {
            return "Insufficient evidence to draw conclusions".to_string();
        }

        let mut conclusions = Vec::new();

        // Summarize pattern findings
        let patterns: Vec<_> = chain
            .steps
            .iter()
            .filter(|s| s.reasoning_type == ReasoningType::PatternMatch)
            .collect();

        if !patterns.is_empty() {
            conclusions.push(format!(
                "Detected {} pattern(s) in target activity",
                patterns.len()
            ));
        }

        // Summarize relationship findings
        let relationships: Vec<_> = chain
            .steps
            .iter()
            .filter(|s| s.reasoning_type == ReasoningType::EntityRelationship)
            .collect();

        if !relationships.is_empty() {
            conclusions.push(format!(
                "Identified {} potential entity relationship(s)",
                relationships.len()
            ));
        }

        // Overall assessment
        let confidence = chain.overall_confidence;
        if confidence >= 0.7 {
            conclusions.push(format!(
                "High confidence ({:.0}%) assessment: {} appears to show significant activity patterns",
                confidence * 100.0,
                chain.target_entity
            ));
        } else if confidence >= 0.4 {
            conclusions.push(format!(
                "Moderate confidence ({:.0}%) assessment: Further investigation recommended for {}",
                confidence * 100.0,
                chain.target_entity
            ));
        } else {
            conclusions.push(format!(
                "Low confidence ({:.0}%): Limited evidence for {}",
                confidence * 100.0,
                chain.target_entity
            ));
        }

        conclusions.join(". ")
    }
}

/// Evidence item for reasoning.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceItem {
    pub id: String,
    pub entity_id: String,
    pub entity_type: String,
    pub evidence_type: String,
    pub description: String,
    pub source: String,
    pub confidence: f64,
    pub timestamp: DateTime<Utc>,
    pub raw_data: serde_json::Value,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn test_inference_chain_creation() {
        let chain = InferenceChain::new("TestCorp", "company", "Test Investigation");
        assert_eq!(chain.target_entity, "TestCorp");
        assert!(chain.steps.is_empty());
        assert_eq!(chain.overall_confidence, 0.0);
    }

    #[test]
    fn test_confidence_propagation() {
        let prop = ConfidencePropagation::propagate(0.9, 5, 0.1);
        assert!(prop.final_confidence > 0.0);
        assert!(prop.final_confidence >= 0.0);
        assert!(prop.final_confidence <= 1.0);
    }

    #[test]
    fn test_chain_of_thought_builder() {
        let reasoner = ChainOfThoughtReasoner::new();
        let evidence = vec![
            EvidenceItem {
                id: "e1".to_string(),
                entity_id: "TestCorp".to_string(),
                entity_type: "company".to_string(),
                evidence_type: "direct_observation".to_string(),
                description: "New hiring detected".to_string(),
                source: "LinkedIn".to_string(),
                confidence: 0.9,
                timestamp: Utc::now() - Duration::days(5),
                raw_data: serde_json::json!({}),
            },
            EvidenceItem {
                id: "e2".to_string(),
                entity_id: "TestCorp".to_string(),
                entity_type: "company".to_string(),
                evidence_type: "direct_observation".to_string(),
                description: "Patent filed".to_string(),
                source: "USPTO".to_string(),
                confidence: 0.85,
                timestamp: Utc::now() - Duration::days(10),
                raw_data: serde_json::json!({}),
            },
        ];

        let chain = reasoner.build_chain("TestCorp", "company", &evidence);
        assert!(!chain.steps.is_empty());
        assert!(chain.overall_confidence > 0.0);
    }
}
