//! Cognitive bias mitigation for intelligence analysis.
//!
//! Intelligence analysis is susceptible to anchoring, confirmation bias, and availability
//! bias. This module actively counteracts these biases through:
//! - Devil's advocate counter-narrative generation
//! - Confirmation ratio tracking with anchoring detection
//! - Base rate display for contextualized risk assessment

use std::collections::HashMap;
use uuid::Uuid;

// ============================================================================
// DEVIL'S ADVOCATE MODE
// ============================================================================

/// Configuration for devil's advocate analysis.
#[derive(Debug, Clone)]
pub struct DevilsAdvocateConfig {
    /// Minimum severity to trigger devil's advocate analysis
    pub min_severity: Severity,
    /// Whether to generate full counter-narratives
    pub generate_counter_narrative: bool,
    /// Maximum length of counter-narrative
    pub max_narrative_length: usize,
}

impl Default for DevilsAdvocateConfig {
    fn default() -> Self {
        Self {
            min_severity: Severity::High,
            generate_counter_narrative: true,
            max_narrative_length: 500,
        }
    }
}

/// Severity levels for insights.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    Info,
    Low,
    Medium,
    High,
    Critical,
}

impl Severity {
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "critical" => Self::Critical,
            "high" => Self::High,
            "medium" | "med" => Self::Medium,
            "low" => Self::Low,
            _ => Self::Info,
        }
    }
}

/// An evidence item for analysis.
#[derive(Debug, Clone)]
pub struct EvidenceItem {
    pub id: Uuid,
    pub description: String,
    pub source: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub supports_claim: bool,
    pub strength: f64, // 0.0 - 1.0
}

/// Devil's advocate analysis result.
#[derive(Debug, Clone)]
pub struct DevilsAdvocateResult {
    /// The original claim/conclusion
    pub original_claim: String,
    /// The counter-narrative arguing the opposite
    pub counter_narrative: String,
    /// Evidence points that could support the counter-narrative
    pub counter_evidence: Vec<CounterEvidence>,
    /// Alternative explanations for the evidence
    pub alternative_explanations: Vec<String>,
    /// Questions the analyst should investigate
    pub investigative_questions: Vec<String>,
    /// Overall confidence reduction factor (0.0 - 1.0)
    pub confidence_reduction: f64,
}

/// Evidence point supporting counter-narrative.
#[derive(Debug, Clone)]
pub struct CounterEvidence {
    /// Original evidence item
    pub original_evidence_id: Uuid,
    /// How this evidence could support the opposite conclusion
    pub counter_interpretation: String,
    /// Strength of the counter-interpretation
    pub counter_strength: f64,
}

/// Generate devil's advocate analysis for an insight.
pub fn generate_devils_advocate(
    claim: &str,
    conclusion: &str,
    evidence: &[EvidenceItem],
    severity: Severity,
    config: &DevilsAdvocateConfig,
) -> Option<DevilsAdvocateResult> {
    // Only trigger for high-severity insights
    if severity < config.min_severity {
        return None;
    }

    // Build counter-evidence interpretations
    let counter_evidence: Vec<CounterEvidence> = evidence
        .iter()
        .filter_map(generate_counter_interpretation)
        .collect();

    // Generate alternative explanations based on evidence patterns
    let alternative_explanations = generate_alternative_explanations(claim, evidence);

    // Generate investigative questions
    let investigative_questions = generate_investigative_questions(claim, conclusion, evidence);

    // Build the counter-narrative
    let counter_narrative = if config.generate_counter_narrative {
        build_counter_narrative(claim, &counter_evidence, &alternative_explanations)
    } else {
        String::new()
    };

    // Calculate confidence reduction based on counter-evidence strength
    let confidence_reduction = calculate_confidence_reduction(&counter_evidence);

    Some(DevilsAdvocateResult {
        original_claim: claim.to_string(),
        counter_narrative,
        counter_evidence,
        alternative_explanations,
        investigative_questions,
        confidence_reduction,
    })
}

fn generate_counter_interpretation(evidence: &EvidenceItem) -> Option<CounterEvidence> {
    // Generate plausible alternative interpretation
    let lower_desc = evidence.description.to_lowercase();

    let counter_interpretation = if lower_desc.contains("expand") || lower_desc.contains("growth") {
        format!(
            "This {} could indicate overextension rather than healthy growth, potentially masking underlying financial stress.",
            evidence.description
        )
    } else if lower_desc.contains("contract") || lower_desc.contains("layoff") {
        format!(
            "This {} could represent strategic refocusing rather than distress, potentially preparing for new market entry.",
            evidence.description
        )
    } else if lower_desc.contains("patent") {
        "Patent filing activity may be defensive rather than innovative, possibly indicating patent hoarding rather than genuine R&D progress."
            .to_string()
    } else if lower_desc.contains("certif") {
        "New certification pursuit could indicate compliance pressure rather than voluntary quality improvement."
            .to_string()
    } else if lower_desc.contains("partner") || lower_desc.contains("alliance") {
        "This partnership could signal weakness requiring external support rather than strategic strength."
            .to_string()
    } else if lower_desc.contains("acqui") || lower_desc.contains("merger") {
        "This acquisition activity could represent desperation to find growth rather than strategic positioning."
            .to_string()
    } else {
        // Generic counter-interpretation
        "This evidence could have been selectively presented or may reflect temporary conditions rather than structural changes."
            .to_string()
    };

    // Counter-strength is inversely related to original strength
    let counter_strength = (1.0 - evidence.strength) * 0.7 + 0.1;

    Some(CounterEvidence {
        original_evidence_id: evidence.id,
        counter_interpretation,
        counter_strength: counter_strength.min(0.8),
    })
}

fn generate_alternative_explanations(claim: &str, evidence: &[EvidenceItem]) -> Vec<String> {
    let mut explanations = Vec::new();
    let lower_claim = claim.to_lowercase();

    // Market conditions explanation
    if lower_claim.contains("competitive") || lower_claim.contains("market") {
        explanations.push(
            "Market-wide trends may explain the observed behavior rather than entity-specific strategy.".to_string()
        );
    }

    // Regulatory explanation
    if lower_claim.contains("compliance") || lower_claim.contains("certif") {
        explanations.push(
            "Regulatory pressure may be driving behavior rather than voluntary strategic choice."
                .to_string(),
        );
    }

    // Timing explanation
    if evidence.len() >= 2 {
        explanations.push(
            "Coincidental timing of separate events may create a false impression of causality."
                .to_string(),
        );
    }

    // Source bias explanation
    let unique_sources: std::collections::HashSet<_> = evidence.iter().map(|e| &e.source).collect();
    if unique_sources.len() <= 2 && evidence.len() > 3 {
        explanations.push(
            "Limited source diversity may introduce bias; consider seeking corroboration from independent sources.".to_string()
        );
    }

    // Selection bias explanation
    explanations.push(
        "Available evidence may not represent the full picture; contradicting evidence may exist but not be captured.".to_string()
    );

    explanations
}

fn generate_investigative_questions(
    claim: &str,
    conclusion: &str,
    evidence: &[EvidenceItem],
) -> Vec<String> {
    let mut questions = Vec::new();
    let lower_claim = claim.to_lowercase();
    let lower_conclusion = conclusion.to_lowercase();

    // What would disprove this?
    questions.push(format!(
        "What evidence would definitively disprove '{}'?",
        truncate_str(claim, 80)
    ));

    // Alternative interpretation question
    questions.push("What alternative explanation accounts for all the same evidence?".to_string());

    // Missing evidence question
    questions.push(
        "What evidence do we NOT have that would be present if this conclusion were true?"
            .to_string(),
    );

    // Source reliability question
    if evidence.iter().any(|e| e.strength < 0.5) {
        questions.push(
            "How reliable are the sources? Are there reasons they might be biased or mistaken?"
                .to_string(),
        );
    }

    // Timing question
    if lower_claim.contains("will")
        || lower_claim.contains("predict")
        || lower_conclusion.contains("likely")
    {
        questions
            .push("What is the actual base rate for this type of event occurring?".to_string());
    }

    // Competitive intelligence question
    if lower_claim.contains("competitor") || lower_claim.contains("rival") {
        questions.push(
            "Could this be intentional information planting by the entity or its competitors?"
                .to_string(),
        );
    }

    questions
}

fn build_counter_narrative(
    claim: &str,
    counter_evidence: &[CounterEvidence],
    alternative_explanations: &[String],
) -> String {
    let mut narrative = String::new();

    narrative.push_str(&format!(
        "COUNTER-NARRATIVE: While the evidence suggests {}, an alternative reading is possible.\n\n",
        truncate_str(claim, 100)
    ));

    if !counter_evidence.is_empty() {
        narrative.push_str("Alternative Interpretations:\n");
        for (i, ce) in counter_evidence.iter().take(3).enumerate() {
            narrative.push_str(&format!("{}. {}\n", i + 1, ce.counter_interpretation));
        }
        narrative.push('\n');
    }

    if !alternative_explanations.is_empty() {
        narrative.push_str("Alternative Explanations:\n");
        for (i, exp) in alternative_explanations.iter().take(3).enumerate() {
            narrative.push_str(&format!("{}. {}\n", i + 1, exp));
        }
    }

    narrative
}

fn calculate_confidence_reduction(counter_evidence: &[CounterEvidence]) -> f64 {
    if counter_evidence.is_empty() {
        return 0.0;
    }

    // Average counter-evidence strength determines reduction
    let avg_strength: f64 = counter_evidence
        .iter()
        .map(|ce| ce.counter_strength)
        .sum::<f64>()
        / counter_evidence.len() as f64;

    // Scale to a reasonable reduction range (0.05 - 0.30)
    (avg_strength * 0.3).clamp(0.05, 0.30)
}

fn truncate_str(s: &str, max_len: usize) -> &str {
    if s.len() <= max_len {
        s
    } else {
        &s[..max_len]
    }
}

// ============================================================================
// CONFIRMATION RATIO TRACKING
// ============================================================================

/// Confirmation ratio tracker for detecting anchoring bias.
#[derive(Debug, Clone)]
pub struct ConfirmationTracker {
    /// Entity being tracked
    pub entity_id: Uuid,
    /// Entity name
    pub entity_name: String,
    /// Count of supporting evidence
    pub supporting_count: u32,
    /// Count of contradicting evidence  
    pub contradicting_count: u32,
    /// Confirmation ratio threshold for anchoring warning
    pub anchoring_threshold: f64,
    /// Rolling window of evidence observations
    pub evidence_window: Vec<EvidenceObservation>,
    /// Maximum window size
    pub max_window_size: usize,
}

/// Single evidence observation for tracking.
#[derive(Debug, Clone)]
pub struct EvidenceObservation {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub supports_current_assessment: bool,
    pub evidence_type: String,
}

impl ConfirmationTracker {
    /// Create a new tracker for an entity.
    pub fn new(entity_id: Uuid, entity_name: String) -> Self {
        Self {
            entity_id,
            entity_name,
            supporting_count: 0,
            contradicting_count: 0,
            anchoring_threshold: 0.80, // Warn when >80% evidence supports current view
            evidence_window: Vec::new(),
            max_window_size: 100,
        }
    }

    /// Record a new evidence observation.
    pub fn observe(&mut self, observation: EvidenceObservation) {
        if observation.supports_current_assessment {
            self.supporting_count += 1;
        } else {
            self.contradicting_count += 1;
        }

        self.evidence_window.push(observation);

        // Trim to window size
        while self.evidence_window.len() > self.max_window_size {
            let removed = self.evidence_window.remove(0);
            if removed.supports_current_assessment {
                self.supporting_count = self.supporting_count.saturating_sub(1);
            } else {
                self.contradicting_count = self.contradicting_count.saturating_sub(1);
            }
        }
    }

    /// Calculate the current confirmation ratio.
    pub fn confirmation_ratio(&self) -> f64 {
        let total = self.supporting_count + self.contradicting_count;
        if total == 0 {
            return 0.5; // Neutral when no evidence
        }
        self.supporting_count as f64 / total as f64
    }

    /// Check if the entity is potentially anchored.
    pub fn is_potentially_anchored(&self) -> bool {
        let total = self.supporting_count + self.contradicting_count;
        total >= 5 && self.confirmation_ratio() > self.anchoring_threshold
    }

    /// Generate anchoring warning if applicable.
    pub fn anchoring_warning(&self) -> Option<AnchoringWarning> {
        if !self.is_potentially_anchored() {
            return None;
        }

        let ratio = self.confirmation_ratio();
        let total = self.supporting_count + self.contradicting_count;

        Some(AnchoringWarning {
            entity_id: self.entity_id,
            entity_name: self.entity_name.clone(),
            confirmation_ratio: ratio,
            total_observations: total,
            message: format!(
                "Potentially anchored: {:.0}% of {} observations support current assessment for {}. Actively seek disconfirming evidence.",
                ratio * 100.0,
                total,
                self.entity_name
            ),
            recommended_action: "Prioritize collection of evidence that would contradict the current assessment.".to_string(),
        })
    }

    /// Get diversity of evidence types observed.
    pub fn evidence_type_diversity(&self) -> f64 {
        let mut type_counts: HashMap<&str, u32> = HashMap::new();
        for obs in &self.evidence_window {
            *type_counts.entry(obs.evidence_type.as_str()).or_insert(0) += 1;
        }

        let total = self.evidence_window.len() as f64;
        if total == 0.0 {
            return 0.0;
        }

        // Simpson's diversity index: 1 - Σ(p_i^2)
        let simpson: f64 = type_counts
            .values()
            .map(|&count| {
                let p = count as f64 / total;
                p * p
            })
            .sum();

        1.0 - simpson
    }
}

/// Anchoring warning for an entity.
#[derive(Debug, Clone)]
pub struct AnchoringWarning {
    pub entity_id: Uuid,
    pub entity_name: String,
    pub confirmation_ratio: f64,
    pub total_observations: u32,
    pub message: String,
    pub recommended_action: String,
}

// ============================================================================
// BASE RATE DISPLAY
// ============================================================================

/// Base rate statistics for risk contextualization.
#[derive(Debug, Clone)]
pub struct BaseRateStats {
    /// Type of event (e.g., "certification_loss", "contract_termination")
    pub event_type: String,
    /// Total tracked entities in this category
    pub total_tracked: u32,
    /// Number that experienced this event in the reference period
    pub event_occurrences: u32,
    /// Reference period in days
    pub reference_period_days: u32,
    /// Calculated base rate
    pub base_rate: f64,
    /// 95% confidence interval lower bound
    pub ci_lower: f64,
    /// 95% confidence interval upper bound
    pub ci_upper: f64,
}

impl BaseRateStats {
    /// Create base rate stats from observed data.
    pub fn from_observations(
        event_type: &str,
        total_tracked: u32,
        event_occurrences: u32,
        reference_period_days: u32,
    ) -> Self {
        let base_rate = if total_tracked == 0 {
            0.0
        } else {
            event_occurrences as f64 / total_tracked as f64
        };

        // Wilson score interval for binomial proportion
        let (ci_lower, ci_upper) = wilson_score_interval(
            event_occurrences as f64,
            total_tracked as f64,
            1.96, // 95% CI
        );

        Self {
            event_type: event_type.to_string(),
            total_tracked,
            event_occurrences,
            reference_period_days,
            base_rate,
            ci_lower,
            ci_upper,
        }
    }

    /// Format the base rate for display alongside an insight.
    pub fn context_message(&self, _entity_name: &str) -> String {
        format!(
            "Base rate context: Of {} tracked entities, only {:.1}% (n={}) experienced {} in the past {} days. 95% CI: [{:.1}%, {:.1}%]",
            self.total_tracked,
            self.base_rate * 100.0,
            self.event_occurrences,
            self.event_type.replace('_', " "),
            self.reference_period_days,
            self.ci_lower * 100.0,
            self.ci_upper * 100.0
        )
    }
}

/// Wilson score interval for binomial proportion.
fn wilson_score_interval(successes: f64, total: f64, z: f64) -> (f64, f64) {
    if total == 0.0 {
        return (0.0, 1.0);
    }

    let p_hat = successes / total;
    let n = total;
    let z2 = z * z;

    let denominator = 1.0 + z2 / n;
    let center = (p_hat + z2 / (2.0 * n)) / denominator;
    let margin = z * ((p_hat * (1.0 - p_hat) + z2 / (4.0 * n)) / n).sqrt() / denominator;

    let lower = (center - margin).max(0.0);
    let upper = (center + margin).min(1.0);

    (lower, upper)
}

/// Repository of base rate statistics by event type.
#[derive(Debug, Default)]
pub struct BaseRateRepository {
    stats: HashMap<String, BaseRateStats>,
}

impl BaseRateRepository {
    pub fn new() -> Self {
        Self {
            stats: HashMap::new(),
        }
    }

    /// Add or update base rate stats for an event type.
    pub fn update(&mut self, stats: BaseRateStats) {
        self.stats.insert(stats.event_type.clone(), stats);
    }

    /// Get base rate stats for an event type.
    pub fn get(&self, event_type: &str) -> Option<&BaseRateStats> {
        self.stats.get(event_type)
    }

    /// Get context message for a specific event type.
    pub fn context_for(&self, event_type: &str, entity_name: &str) -> Option<String> {
        self.get(event_type).map(|s| s.context_message(entity_name))
    }

    /// Initialize with common event types and placeholder statistics.
    pub fn with_defaults() -> Self {
        let mut repo = Self::new();

        // Default base rates (should be updated from production data)
        repo.update(BaseRateStats::from_observations(
            "certification_loss",
            500,
            15,
            365,
        ));
        repo.update(BaseRateStats::from_observations(
            "contract_termination",
            500,
            25,
            365,
        ));
        repo.update(BaseRateStats::from_observations(
            "executive_departure",
            500,
            100,
            365,
        ));
        repo.update(BaseRateStats::from_observations(
            "facility_closure",
            500,
            20,
            365,
        ));
        repo.update(BaseRateStats::from_observations("bankruptcy", 500, 5, 365));
        repo.update(BaseRateStats::from_observations(
            "security_breach",
            500,
            35,
            365,
        ));
        repo.update(BaseRateStats::from_observations(
            "sanction_designation",
            500,
            10,
            365,
        ));
        repo.update(BaseRateStats::from_observations(
            "quality_issue",
            500,
            45,
            365,
        ));
        repo.update(BaseRateStats::from_observations(
            "supply_disruption",
            500,
            60,
            365,
        ));
        repo.update(BaseRateStats::from_observations(
            "personnel_strike",
            500,
            8,
            365,
        ));

        repo
    }
}

// ============================================================================
// BIAS MITIGATION SUMMARY
// ============================================================================

/// Comprehensive bias mitigation report for an insight.
#[derive(Debug, Clone)]
pub struct BiasMitigationReport {
    /// Entity being analyzed
    pub entity_id: Uuid,
    /// Entity name
    pub entity_name: String,
    /// Insight ID
    pub insight_id: Uuid,
    /// Devil's advocate result (if applicable)
    pub devils_advocate: Option<DevilsAdvocateResult>,
    /// Anchoring warning (if applicable)
    pub anchoring_warning: Option<AnchoringWarning>,
    /// Base rate context (if applicable)
    pub base_rate_context: Option<String>,
    /// Overall bias risk score (0.0 - 1.0, higher = more bias risk)
    pub bias_risk_score: f64,
    /// Recommended actions for the analyst
    pub recommended_actions: Vec<String>,
}

impl BiasMitigationReport {
    /// Create a bias mitigation report from components.
    pub fn build(
        entity_id: Uuid,
        entity_name: String,
        insight_id: Uuid,
        devils_advocate: Option<DevilsAdvocateResult>,
        anchoring_warning: Option<AnchoringWarning>,
        base_rate_context: Option<String>,
    ) -> Self {
        let mut bias_risk_score = 0.0;
        let mut recommended_actions = Vec::new();

        // Accumulate bias risk from components
        if let Some(ref da) = devils_advocate {
            bias_risk_score += da.confidence_reduction;
            recommended_actions.push(
                "Review the counter-narrative and alternative explanations before accepting the conclusion.".to_string()
            );
            for q in da.investigative_questions.iter().take(2) {
                recommended_actions.push(format!("Investigate: {}", q));
            }
        }

        if anchoring_warning.is_some() {
            bias_risk_score += 0.20;
            recommended_actions
                .push("Actively seek disconfirming evidence for this entity.".to_string());
        }

        if base_rate_context.is_some() {
            recommended_actions
                .push("Consider the base rate when evaluating this risk assessment.".to_string());
        }

        // Cap bias risk score at 1.0
        bias_risk_score = bias_risk_score.min(1.0);

        Self {
            entity_id,
            entity_name,
            insight_id,
            devils_advocate,
            anchoring_warning,
            base_rate_context,
            bias_risk_score,
            recommended_actions,
        }
    }

    /// Check if this report warrants analyst attention.
    pub fn requires_attention(&self) -> bool {
        self.bias_risk_score > 0.30
            || self.anchoring_warning.is_some()
            || self.devils_advocate.is_some()
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::disallowed_methods,
        clippy::field_reassign_with_default,
        clippy::manual_range_contains,
        clippy::needless_borrows_for_generic_args,
        clippy::cloned_ref_to_slice_refs
    )]

    use super::*;

    #[test]
    fn devils_advocate_generates_for_high_severity() {
        let evidence = vec![EvidenceItem {
            id: Uuid::new_v4(),
            description: "Company expanding into new markets".to_string(),
            source: "Trade Press".to_string(),
            timestamp: chrono::Utc::now(),
            supports_claim: true,
            strength: 0.7,
        }];

        let result = generate_devils_advocate(
            "Company is in aggressive growth mode",
            "Likely to capture market share",
            &evidence,
            Severity::High,
            &DevilsAdvocateConfig::default(),
        );

        assert!(result.is_some());
        let da = result.unwrap();
        assert!(!da.counter_narrative.is_empty());
        assert!(!da.investigative_questions.is_empty());
    }

    #[test]
    fn devils_advocate_skips_low_severity() {
        let result = generate_devils_advocate(
            "Company filed a patent",
            "Minor activity",
            &[],
            Severity::Info,
            &DevilsAdvocateConfig::default(),
        );

        assert!(result.is_none());
    }

    #[test]
    fn confirmation_ratio_detects_anchoring() {
        let mut tracker = ConfirmationTracker::new(Uuid::new_v4(), "Test Corp".to_string());

        // Add mostly supporting evidence
        for _ in 0..9 {
            tracker.observe(EvidenceObservation {
                timestamp: chrono::Utc::now(),
                supports_current_assessment: true,
                evidence_type: "patent".to_string(),
            });
        }

        // Add one contradicting
        tracker.observe(EvidenceObservation {
            timestamp: chrono::Utc::now(),
            supports_current_assessment: false,
            evidence_type: "layoff".to_string(),
        });

        assert!(tracker.confirmation_ratio() > 0.80);
        assert!(tracker.is_potentially_anchored());
        assert!(tracker.anchoring_warning().is_some());
    }

    #[test]
    fn base_rate_provides_context() {
        let stats = BaseRateStats::from_observations("certification_loss", 500, 15, 365);

        assert!((stats.base_rate - 0.03).abs() < 0.01);
        let context = stats.context_message("Test Corp");
        assert!(context.contains("3.0%"));
        assert!(context.contains("500"));
    }

    #[test]
    fn wilson_score_interval_is_valid() {
        let (lower, upper) = wilson_score_interval(15.0, 500.0, 1.96);
        assert!(lower > 0.0);
        assert!(upper < 1.0);
        assert!(lower < 0.03);
        assert!(upper > 0.03);
    }

    #[test]
    fn bias_mitigation_report_accumulates_risk() {
        let da = DevilsAdvocateResult {
            original_claim: "Test".to_string(),
            counter_narrative: "Counter".to_string(),
            counter_evidence: vec![],
            alternative_explanations: vec!["Alt".to_string()],
            investigative_questions: vec!["Question?".to_string()],
            confidence_reduction: 0.15,
        };

        let anchoring = AnchoringWarning {
            entity_id: Uuid::new_v4(),
            entity_name: "Test Corp".to_string(),
            confirmation_ratio: 0.85,
            total_observations: 20,
            message: "Warning".to_string(),
            recommended_action: "Action".to_string(),
        };

        let report = BiasMitigationReport::build(
            Uuid::new_v4(),
            "Test Corp".to_string(),
            Uuid::new_v4(),
            Some(da),
            Some(anchoring),
            Some("Base rate context".to_string()),
        );

        assert!(report.bias_risk_score > 0.30);
        assert!(report.requires_attention());
        assert!(!report.recommended_actions.is_empty());
    }
}
