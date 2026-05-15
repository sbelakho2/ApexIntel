//! Multi-hypothesis tracking for competing intelligence assessments.
//!
//! Implements Analysis of Competing Hypotheses (ACH) methodology:
//! - Maintains up to 3 competing hypotheses per entity
//! - Bayesian scoring of hypotheses against incoming evidence
//! - Diagnostic evidence identification to distinguish between hypotheses
//!
//! For each high-priority entity, we maintain competing hypotheses about
//! their strategic direction (expanding, contracting, pivoting) and score
//! them against incoming evidence using Bayesian fusion.

use std::collections::HashMap;
use uuid::Uuid;

/// Hypothesis about an entity's strategic direction.
#[derive(Debug, Clone, PartialEq)]
pub struct Hypothesis {
    /// Unique identifier for this hypothesis
    pub id: String,
    /// Human-readable label (e.g., "Expanding capability")
    pub label: String,
    /// Detailed description
    pub description: String,
    /// Evidence types that support this hypothesis
    pub supporting_evidence: Vec<EvidenceType>,
    /// Prior probability P(H) before evidence
    pub prior: f64,
    /// Posterior probability P(H|E) after evidence
    pub posterior: f64,
    /// Likelihood P(E|H) for computing posterior
    pub likelihood: f64,
}

impl Default for Hypothesis {
    fn default() -> Self {
        Self {
            id: String::new(),
            label: String::new(),
            description: String::new(),
            supporting_evidence: Vec::new(),
            prior: 1.0 / 3.0, // Uniform prior for 3 hypotheses
            posterior: 1.0 / 3.0,
            likelihood: 0.5,
        }
    }
}

/// Types of evidence that can support hypotheses.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum EvidenceType {
    JobPosting,
    PatentFiling,
    CapacityAnnouncement,
    Layoff,
    FacilityClosure,
    ReducedFilings,
    NewCertification,
    LeadershipChange,
    MergerAcquisition,
    PartnershipAnnouncement,
    ProductLaunch,
    FinancialReport,
    RegulatoryFiling,
    SupplyChainChange,
    Other(String),
}

impl EvidenceType {
    /// Parse evidence type from signal type string.
    pub fn from_signal_type(signal_type: &str) -> Self {
        let lower = signal_type.to_lowercase();
        if lower.contains("job") || lower.contains("hiring") {
            EvidenceType::JobPosting
        } else if lower.contains("patent") {
            EvidenceType::PatentFiling
        } else if lower.contains("capacity") || lower.contains("facility") && lower.contains("new")
        {
            EvidenceType::CapacityAnnouncement
        } else if lower.contains("layoff") || lower.contains("reduction") {
            EvidenceType::Layoff
        } else if lower.contains("closure") || lower.contains("shutdown") {
            EvidenceType::FacilityClosure
        } else if lower.contains("certif") {
            EvidenceType::NewCertification
        } else if lower.contains("leader")
            || lower.contains("executive")
            || lower.contains("ceo")
            || lower.contains("cto")
        {
            EvidenceType::LeadershipChange
        } else if lower.contains("merger") || lower.contains("acquisition") {
            EvidenceType::MergerAcquisition
        } else if lower.contains("partner") {
            EvidenceType::PartnershipAnnouncement
        } else if lower.contains("product") || lower.contains("launch") {
            EvidenceType::ProductLaunch
        } else if lower.contains("financial") || lower.contains("earnings") {
            EvidenceType::FinancialReport
        } else if lower.contains("regulatory") || lower.contains("compliance") {
            EvidenceType::RegulatoryFiling
        } else if lower.contains("supply") {
            EvidenceType::SupplyChainChange
        } else {
            EvidenceType::Other(signal_type.to_string())
        }
    }
}

/// Standard hypothesis types for entity strategic direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HypothesisType {
    /// H1: Entity is expanding capability
    Expanding,
    /// H2: Entity is contracting
    Contracting,
    /// H3: Entity is pivoting to new areas
    Pivoting,
}

impl HypothesisType {
    /// Get default supporting evidence types for this hypothesis.
    pub fn supporting_evidence(&self) -> Vec<EvidenceType> {
        match self {
            HypothesisType::Expanding => vec![
                EvidenceType::JobPosting,
                EvidenceType::PatentFiling,
                EvidenceType::CapacityAnnouncement,
                EvidenceType::ProductLaunch,
                EvidenceType::PartnershipAnnouncement,
            ],
            HypothesisType::Contracting => vec![
                EvidenceType::Layoff,
                EvidenceType::FacilityClosure,
                EvidenceType::ReducedFilings,
            ],
            HypothesisType::Pivoting => vec![
                EvidenceType::NewCertification,
                EvidenceType::LeadershipChange,
                EvidenceType::MergerAcquisition,
                EvidenceType::SupplyChainChange,
            ],
        }
    }

    /// Create a default hypothesis for this type.
    pub fn to_hypothesis(&self) -> Hypothesis {
        match self {
            HypothesisType::Expanding => Hypothesis {
                id: "H1_expanding".to_string(),
                label: "Expanding capability".to_string(),
                description: "Entity is expanding capability through hiring, R&D, and capacity investments".to_string(),
                supporting_evidence: self.supporting_evidence(),
                prior: 1.0 / 3.0,
                posterior: 1.0 / 3.0,
                likelihood: 0.5,
            },
            HypothesisType::Contracting => Hypothesis {
                id: "H2_contracting".to_string(),
                label: "Contracting".to_string(),
                description: "Entity is contracting through layoffs, facility closures, and reduced activity".to_string(),
                supporting_evidence: self.supporting_evidence(),
                prior: 1.0 / 3.0,
                posterior: 1.0 / 3.0,
                likelihood: 0.5,
            },
            HypothesisType::Pivoting => Hypothesis {
                id: "H3_pivoting".to_string(),
                label: "Pivoting".to_string(),
                description: "Entity is pivoting to new areas through certifications, leadership changes, and strategic shifts".to_string(),
                supporting_evidence: self.supporting_evidence(),
                prior: 1.0 / 3.0,
                posterior: 1.0 / 3.0,
                likelihood: 0.5,
            },
        }
    }
}

/// Entity hypothesis tracker maintaining competing hypotheses.
#[derive(Debug, Clone)]
pub struct EntityHypothesisTracker {
    /// Entity being tracked
    pub entity_id: Uuid,
    /// Entity name
    pub entity_name: String,
    /// Competing hypotheses (up to 3)
    pub hypotheses: Vec<Hypothesis>,
    /// Observed evidence with counts
    pub evidence_counts: HashMap<EvidenceType, u32>,
    /// Total evidence observations
    pub total_evidence: u32,
    /// Last update timestamp
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl EntityHypothesisTracker {
    /// Create a new tracker with default hypotheses.
    pub fn new(entity_id: Uuid, entity_name: String) -> Self {
        Self {
            entity_id,
            entity_name,
            hypotheses: vec![
                HypothesisType::Expanding.to_hypothesis(),
                HypothesisType::Contracting.to_hypothesis(),
                HypothesisType::Pivoting.to_hypothesis(),
            ],
            evidence_counts: HashMap::new(),
            total_evidence: 0,
            updated_at: chrono::Utc::now(),
        }
    }

    /// Update hypotheses with new evidence using Bayesian fusion.
    ///
    /// P(H_i | evidence) ∝ P(evidence | H_i) · P(H_i)
    ///
    /// The likelihood P(evidence | H_i) is estimated based on whether the
    /// evidence type supports the hypothesis.
    pub fn update_with_evidence(&mut self, evidence_type: EvidenceType) {
        // Track evidence count
        *self
            .evidence_counts
            .entry(evidence_type.clone())
            .or_insert(0) += 1;
        self.total_evidence += 1;

        // Update likelihoods based on evidence support
        let mut unnormalized_posteriors: Vec<f64> = Vec::new();

        for hypothesis in &mut self.hypotheses {
            // Likelihood: higher if evidence supports this hypothesis
            let supports = hypothesis.supporting_evidence.contains(&evidence_type);
            let base_likelihood = if supports { 0.8 } else { 0.3 };

            // Update likelihood with exponential moving average
            hypothesis.likelihood = 0.9 * hypothesis.likelihood + 0.1 * base_likelihood;

            // Compute unnormalized posterior
            let unnormalized = hypothesis.likelihood * hypothesis.prior;
            unnormalized_posteriors.push(unnormalized);
        }

        // Normalize posteriors
        let total: f64 = unnormalized_posteriors.iter().sum();
        if total > 0.0 {
            for (i, hypothesis) in self.hypotheses.iter_mut().enumerate() {
                hypothesis.posterior = unnormalized_posteriors[i] / total;
                // Update prior for next iteration (Bayesian update)
                hypothesis.prior = hypothesis.posterior;
            }
        }

        self.updated_at = chrono::Utc::now();
    }

    /// Get the hypothesis with highest posterior probability.
    pub fn leading_hypothesis(&self) -> Option<&Hypothesis> {
        self.hypotheses.iter().max_by(|a, b| {
            a.posterior
                .partial_cmp(&b.posterior)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    }

    /// Get all hypotheses sorted by posterior probability (descending).
    pub fn ranked_hypotheses(&self) -> Vec<&Hypothesis> {
        let mut sorted: Vec<&Hypothesis> = self.hypotheses.iter().collect();
        sorted.sort_by(|a, b| {
            b.posterior
                .partial_cmp(&a.posterior)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        sorted
    }

    /// Identify diagnostic evidence that would best distinguish between hypotheses.
    ///
    /// For each hypothesis pair (H_i, H_j), find the evidence type that would
    /// most distinguish between them:
    ///
    /// diagnostic_value(obs) = |log(P(obs|H_i)) − log(P(obs|H_j))|
    pub fn identify_diagnostic_evidence(&self) -> Vec<DiagnosticEvidence> {
        let mut diagnostics = Vec::new();

        // Compare each pair of hypotheses
        for i in 0..self.hypotheses.len() {
            for j in (i + 1)..self.hypotheses.len() {
                let h_i = &self.hypotheses[i];
                let h_j = &self.hypotheses[j];

                // Find evidence types that strongly distinguish these hypotheses
                let mut best_evidence: Option<(EvidenceType, f64)> = None;

                // Check all evidence types from both hypotheses
                let mut all_evidence_types: Vec<EvidenceType> = Vec::new();
                all_evidence_types.extend(h_i.supporting_evidence.clone());
                all_evidence_types.extend(h_j.supporting_evidence.clone());
                all_evidence_types.sort_by_key(|e| format!("{:?}", e));
                all_evidence_types.dedup();

                for evidence_type in &all_evidence_types {
                    let p_i: f64 = if h_i.supporting_evidence.contains(evidence_type) {
                        0.8
                    } else {
                        0.2
                    };
                    let p_j: f64 = if h_j.supporting_evidence.contains(evidence_type) {
                        0.8
                    } else {
                        0.2
                    };

                    let diagnostic_value = (p_i.ln() - p_j.ln()).abs();

                    match &best_evidence {
                        None => best_evidence = Some((evidence_type.clone(), diagnostic_value)),
                        Some((_, best_value)) if diagnostic_value > *best_value => {
                            best_evidence = Some((evidence_type.clone(), diagnostic_value));
                        }
                        _ => {}
                    }
                }

                if let Some((evidence_type, value)) = best_evidence {
                    let recommendation = format!(
                        "To distinguish between '{}' and '{}', prioritize collecting {:?} evidence (diagnostic value: {:.2})",
                        h_i.label, h_j.label, evidence_type, value
                    );
                    diagnostics.push(DiagnosticEvidence {
                        evidence_type,
                        hypothesis_a: h_i.id.clone(),
                        hypothesis_b: h_j.id.clone(),
                        diagnostic_value: value,
                        recommendation,
                    });
                }
            }
        }

        // Sort by diagnostic value descending
        diagnostics.sort_by(|a, b| {
            b.diagnostic_value
                .partial_cmp(&a.diagnostic_value)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        diagnostics
    }

    /// Generate a summary of the hypothesis state for analyst review.
    pub fn summary(&self) -> HypothesisSummary {
        let ranked = self.ranked_hypotheses();
        let leading = ranked.first().cloned();
        let diagnostics = self.identify_diagnostic_evidence();

        HypothesisSummary {
            entity_id: self.entity_id,
            entity_name: self.entity_name.clone(),
            leading_hypothesis: leading.map(|h| h.label.clone()),
            leading_posterior: leading.map(|h| h.posterior),
            all_posteriors: ranked
                .iter()
                .map(|h| (h.label.clone(), h.posterior))
                .collect(),
            total_evidence: self.total_evidence,
            top_diagnostic: diagnostics.first().map(|d| d.recommendation.clone()),
            updated_at: self.updated_at,
        }
    }
}

/// Diagnostic evidence recommendation.
#[derive(Debug, Clone)]
pub struct DiagnosticEvidence {
    /// Type of evidence to collect
    pub evidence_type: EvidenceType,
    /// First hypothesis being distinguished
    pub hypothesis_a: String,
    /// Second hypothesis being distinguished
    pub hypothesis_b: String,
    /// Diagnostic value (higher = more distinguishing)
    pub diagnostic_value: f64,
    /// Human-readable recommendation
    pub recommendation: String,
}

/// Summary of hypothesis state for an entity.
#[derive(Debug, Clone)]
pub struct HypothesisSummary {
    pub entity_id: Uuid,
    pub entity_name: String,
    pub leading_hypothesis: Option<String>,
    pub leading_posterior: Option<f64>,
    pub all_posteriors: Vec<(String, f64)>,
    pub total_evidence: u32,
    pub top_diagnostic: Option<String>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl HypothesisSummary {
    /// Check if the entity is potentially anchored (>80% evidence in one direction).
    pub fn is_potentially_anchored(&self) -> bool {
        self.leading_posterior.map(|p| p > 0.80).unwrap_or(false)
    }

    /// Generate anchoring warning if needed.
    pub fn anchoring_warning(&self) -> Option<String> {
        if self.is_potentially_anchored() {
            Some(format!(
                "⚠️ Potentially anchored: {}% of evidence points toward '{}'. Seek disconfirming evidence.",
                (self.leading_posterior.unwrap_or(0.0) * 100.0) as u32,
                self.leading_hypothesis.as_deref().unwrap_or("unknown")
            ))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bayesian_update_increases_posterior() {
        let mut tracker = EntityHypothesisTracker::new(Uuid::new_v4(), "Test Corp".to_string());

        // Initial posteriors should be equal
        assert!((tracker.hypotheses[0].posterior - 1.0 / 3.0).abs() < 0.01);

        // Add expanding evidence
        tracker.update_with_evidence(EvidenceType::JobPosting);
        tracker.update_with_evidence(EvidenceType::PatentFiling);
        tracker.update_with_evidence(EvidenceType::CapacityAnnouncement);

        // Expanding hypothesis should have highest posterior
        let ranked = tracker.ranked_hypotheses();
        assert_eq!(ranked[0].id, "H1_expanding");
        assert!(ranked[0].posterior > ranked[1].posterior);
    }

    #[test]
    fn diagnostic_evidence_identifies_distinguishing_signals() {
        let tracker = EntityHypothesisTracker::new(Uuid::new_v4(), "Test Corp".to_string());

        let diagnostics = tracker.identify_diagnostic_evidence();
        assert!(!diagnostics.is_empty());
        assert!(diagnostics[0].diagnostic_value > 0.0);
    }

    #[test]
    fn anchoring_detection_works() {
        let mut tracker = EntityHypothesisTracker::new(Uuid::new_v4(), "Test Corp".to_string());

        // Add many signals for one hypothesis
        for _ in 0..20 {
            tracker.update_with_evidence(EvidenceType::JobPosting);
        }

        let summary = tracker.summary();
        assert!(summary.is_potentially_anchored());
        assert!(summary.anchoring_warning().is_some());
    }
}
