//! Multi-hypothesis tracking and generation for OSINT investigations.
//!
//! Implements Analysis of Competing Hypotheses (ACH) methodology with:
//! - Anomaly-driven hypothesis creation
//! - Counterfactual reasoning engine
//! - Gap analysis for missing information
//! - Priority-based investigation queue

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Types of evidence that can support hypotheses in OSINT investigations.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum EvidenceType {
    /// Job postings indicating expansion/contraction
    JobPosting,
    /// Patent filings indicating R&D activity
    PatentFiling,
    /// Capacity announcements
    CapacityAnnouncement,
    /// Layoff announcements
    Layoff,
    /// Facility closures
    FacilityClosure,
    /// Reduced regulatory filings
    ReducedFilings,
    /// New certifications obtained
    NewCertification,
    /// Leadership/executive changes
    LeadershipChange,
    /// Merger or acquisition activity
    MergerAcquisition,
    /// Partnership announcements
    PartnershipAnnouncement,
    /// Product launch announcements
    ProductLaunch,
    /// Financial report indicators
    FinancialReport,
    /// Regulatory filing changes
    RegulatoryFiling,
    /// Supply chain relationship changes
    SupplyChainChange,
    /// Website/content changes
    WebChange,
    /// Social media activity
    SocialMediaActivity,
    /// Dark web indicators
    DarkWebIndicator,
    /// Technical OSINT signals (DNS, certs, etc.)
    TechnicalSignal,
    /// Geopolitical risk indicators
    GeopoliticalSignal,
    /// Sanctions/compliance signals
    SanctionsSignal,
    /// Custom evidence type
    Custom(String),
}

impl EvidenceType {
    /// Parse evidence type from signal type string.
    pub fn from_signal_type(signal_type: &str) -> Self {
        let lower = signal_type.to_lowercase();
        if lower.contains("job") || lower.contains("hiring") {
            EvidenceType::JobPosting
        } else if lower.contains("patent") {
            EvidenceType::PatentFiling
        } else if lower.contains("capacity") {
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
        } else if lower.contains("web") || lower.contains("site") {
            EvidenceType::WebChange
        } else if lower.contains("social") || lower.contains("twitter") || lower.contains("linkedin") {
            EvidenceType::SocialMediaActivity
        } else if lower.contains("dark") || lower.contains("tor") {
            EvidenceType::DarkWebIndicator
        } else if lower.contains("dns") || lower.contains("certificate") || lower.contains("whois") {
            EvidenceType::TechnicalSignal
        } else if lower.contains("geopolitical") || lower.contains("political") {
            EvidenceType::GeopoliticalSignal
        } else if lower.contains("sanction") || lower.contains("ofac") {
            EvidenceType::SanctionsSignal
        } else {
            EvidenceType::Custom(signal_type.to_string())
        }
    }
}

/// Standard hypothesis types for entity strategic direction analysis.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum HypothesisType {
    /// H1: Entity is expanding capability
    Expanding,
    /// H2: Entity is contracting
    Contracting,
    /// H3: Entity is pivoting to new areas
    Pivoting,
    /// H4: Entity is under financial stress
    FinancialStress,
    /// H5: Entity is becoming a threat/competitor
    EmergingThreat,
    /// H6: Entity is involved in suspicious activity
    SuspiciousActivity,
    /// H7: Entity has hidden relationships
    HiddenRelationships,
    /// H8: Custom hypothesis
    Custom,
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
            HypothesisType::FinancialStress => vec![
                EvidenceType::Layoff,
                EvidenceType::ReducedFilings,
                EvidenceType::FacilityClosure,
                EvidenceType::LeadershipChange,
                EvidenceType::SanctionsSignal,
            ],
            HypothesisType::EmergingThreat => vec![
                EvidenceType::CapacityAnnouncement,
                EvidenceType::PartnershipAnnouncement,
                EvidenceType::ProductLaunch,
                EvidenceType::PatentFiling,
                EvidenceType::JobPosting,
            ],
            HypothesisType::SuspiciousActivity => vec![
                EvidenceType::DarkWebIndicator,
                EvidenceType::LeadershipChange,
                EvidenceType::SupplyChainChange,
                EvidenceType::GeopoliticalSignal,
            ],
            HypothesisType::HiddenRelationships => vec![
                EvidenceType::SupplyChainChange,
                EvidenceType::PartnershipAnnouncement,
                EvidenceType::MergerAcquisition,
                EvidenceType::TechnicalSignal,
            ],
            HypothesisType::Custom => vec![],
        }
    }

    /// Get the hypothesis label.
    pub fn label(&self) -> String {
        match self {
            HypothesisType::Expanding => "Expanding capability".to_string(),
            HypothesisType::Contracting => "Contracting".to_string(),
            HypothesisType::Pivoting => "Pivoting to new areas".to_string(),
            HypothesisType::FinancialStress => "Under financial stress".to_string(),
            HypothesisType::EmergingThreat => "Emerging as a threat".to_string(),
            HypothesisType::SuspiciousActivity => "Involved in suspicious activity".to_string(),
            HypothesisType::HiddenRelationships => "Has hidden relationships".to_string(),
            HypothesisType::Custom => "Custom hypothesis".to_string(),
        }
    }
}

/// A single hypothesis about an entity's behavior or state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hypothesis {
    /// Unique identifier for this hypothesis
    pub id: String,
    /// Hypothesis type
    pub hypothesis_type: HypothesisType,
    /// Human-readable label
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
    /// Evidence supporting this hypothesis
    pub supporting_signals: Vec<SignalEvidence>,
    /// When this hypothesis was created
    pub created_at: DateTime<Utc>,
    /// Last update timestamp
    pub updated_at: DateTime<Utc>,
}

impl Hypothesis {
    /// Create a default hypothesis for a given type.
    pub fn new(hypothesis_type: HypothesisType, num_competitors: usize) -> Self {
        let prior = 1.0 / (num_competitors as f64);
        let supporting_evidence = hypothesis_type.supporting_evidence();
        let now = Utc::now();

        Self {
            id: Uuid::new_v4().to_string(),
            hypothesis_type,
            label: hypothesis_type.label(),
            description: format!("Hypothesis: {}", hypothesis_type.label()),
            supporting_evidence,
            prior,
            posterior: prior,
            likelihood: 0.5,
            supporting_signals: Vec::new(),
            created_at: now,
            updated_at: now,
        }
    }

    /// Add supporting signal evidence.
    pub fn add_evidence(&mut self, signal: SignalEvidence) {
        self.supporting_signals.push(signal);
        self.updated_at = Utc::now();
    }
}

/// A piece of signal evidence supporting a hypothesis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalEvidence {
    /// Signal identifier
    pub signal_id: String,
    /// Signal type
    pub signal_type: String,
    /// Entity this signal relates to
    pub entity_id: String,
    /// Entity name
    pub entity_name: String,
    /// Confidence in this signal (0-1)
    pub confidence: f64,
    /// Timestamp when signal was observed
    pub observed_at: DateTime<Utc>,
    /// Source of the signal
    pub source: String,
    /// Raw signal data
    pub raw_data: serde_json::Value,
}

/// Investigation priority levels.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum InvestigationPriority {
    /// Critical - immediate investigation required
    Critical = 0,
    /// High priority
    High = 1,
    /// Medium priority
    Medium = 2,
    /// Low priority
    Low = 3,
    /// Informational only
    Informational = 4,
}

impl InvestigationPriority {
    /// Get priority from score (0-1, higher = more urgent).
    pub fn from_score(score: f64) -> Self {
        if score >= 0.9 {
            InvestigationPriority::Critical
        } else if score >= 0.7 {
            InvestigationPriority::High
        } else if score >= 0.5 {
            InvestigationPriority::Medium
        } else if score >= 0.3 {
            InvestigationPriority::Low
        } else {
            InvestigationPriority::Informational
        }
    }
}

/// Counterfactual analysis for what-if reasoning.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CounterfactualAnalysis {
    /// Original hypothesis
    pub original_hypothesis: String,
    /// Counterfactual statement
    pub counterfactual: String,
    /// What would have to be true for this counterfactual
    pub required_conditions: Vec<String>,
    /// Probability of counterfactual being true
    pub probability: f64,
    /// Evidence that would support/disprove this counterfactual
    pub evidence_needed: Vec<String>,
}

/// Gap analysis identifying missing information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GapAnalysis {
    /// Entity being analyzed
    pub entity_id: String,
    /// Gaps in financial information
    pub financial_gaps: Vec<String>,
    /// Gaps in operational information
    pub operational_gaps: Vec<String>,
    /// Gaps in leadership information
    pub leadership_gaps: Vec<String>,
    /// Gaps in supply chain information
    pub supply_chain_gaps: Vec<String>,
    /// Gaps in compliance information
    pub compliance_gaps: Vec<String>,
    /// Overall information completeness score (0-1)
    pub completeness_score: f64,
    /// Priority gaps to fill first
    pub priority_gaps: Vec<String>,
}

/// Hypothesis generator with anomaly-driven creation.
#[derive(Clone)]
pub struct HypothesisGenerator {
    /// Standard hypothesis types to consider
    standard_types: Vec<HypothesisType>,
    /// Minimum evidence threshold for hypothesis activation
    min_evidence_threshold: usize,
    /// Custom hypothesis templates
    #[allow(dead_code)]
    custom_templates: Vec<HypothesisTemplate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HypothesisTemplate {
    pub name: String,
    pub description: String,
    pub required_signals: Vec<String>,
    pub supporting_evidence_types: Vec<EvidenceType>,
    pub confidence_boost: f64,
}

impl Default for HypothesisGenerator {
    fn default() -> Self {
        Self::new()
    }
}

impl HypothesisGenerator {
    /// Create a new hypothesis generator.
    pub fn new() -> Self {
        Self {
            standard_types: vec![
                HypothesisType::Expanding,
                HypothesisType::Contracting,
                HypothesisType::Pivoting,
                HypothesisType::FinancialStress,
                HypothesisType::EmergingThreat,
                HypothesisType::SuspiciousActivity,
                HypothesisType::HiddenRelationships,
            ],
            min_evidence_threshold: 1,
            custom_templates: Vec::new(),
        }
    }

    /// Generate hypotheses based on observed signals.
    pub fn generate_hypotheses(
        &self,
        _entity_id: &str,
        _entity_name: &str,
        signals: &[SignalEvidence],
    ) -> Vec<Hypothesis> {
        let mut hypotheses = Vec::new();

        // Group signals by evidence type
        let mut evidence_counts: HashMap<EvidenceType, Vec<&SignalEvidence>> = HashMap::new();
        for signal in signals {
            let evidence_type = EvidenceType::from_signal_type(&signal.signal_type);
            evidence_counts.entry(evidence_type).or_default().push(signal);
        }

        // Generate standard hypotheses
        for hypothesis_type in &self.standard_types {
            let supporting_types = hypothesis_type.supporting_evidence();
            let supporting_signals: Vec<SignalEvidence> = supporting_types
                .iter()
                .flat_map(|et| {
                    evidence_counts
                        .get(et).cloned()
                        .unwrap_or_default()
                })
                .cloned()
                .collect();

            if supporting_signals.len() >= self.min_evidence_threshold {
                let mut hypothesis = Hypothesis::new(*hypothesis_type, self.standard_types.len());
                hypothesis.description = format!(
                    "{} - Supported by {} signals",
                    hypothesis.label,
                    supporting_signals.len()
                );
                for signal in &supporting_signals {
                    hypothesis.add_evidence(signal.clone());
                }
                hypotheses.push(hypothesis);
            }
        }

        // Sort by posterior probability
        hypotheses.sort_by(|a, b| {
            b.posterior
                .partial_cmp(&a.posterior)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        hypotheses
    }

    /// Update hypotheses with new evidence using Bayesian fusion.
    pub fn update_with_evidence(&self, hypotheses: &mut [Hypothesis], new_evidence: &SignalEvidence) {
        let evidence_type = EvidenceType::from_signal_type(&new_evidence.signal_type);

        // First pass: compute total for normalization
        let totals: Vec<f64> = hypotheses.iter()
            .map(|h| {
                let supports = h.supporting_evidence.contains(&evidence_type);
                let base_likelihood = if supports { 0.8 } else { 0.3 };
                let likelihood = 0.9 * h.likelihood + 0.1 * base_likelihood;
                likelihood * h.prior
            })
            .collect();

        let total: f64 = totals.iter().sum();

        // Second pass: update hypotheses
        for hypothesis in hypotheses.iter_mut() {
            // Likelihood: higher if evidence supports this hypothesis
            let supports = hypothesis.supporting_evidence.contains(&evidence_type);
            let base_likelihood = if supports { 0.8 } else { 0.3 };

            // Update likelihood with exponential moving average
            hypothesis.likelihood = 0.9 * hypothesis.likelihood + 0.1 * base_likelihood;

            // Compute unnormalized posterior
            let unnormalized = hypothesis.likelihood * hypothesis.prior;

            // Normalize
            if total > 0.0 {
                hypothesis.posterior = unnormalized / total;
                hypothesis.prior = hypothesis.posterior;
            }

            // Add evidence if it supports this hypothesis
            if supports {
                hypothesis.add_evidence(new_evidence.clone());
            }

            hypothesis.updated_at = Utc::now();
        }
    }

    /// Generate counterfactual analyses for top hypotheses.
    pub fn generate_counterfactuals(&self, hypothesis: &Hypothesis) -> Vec<CounterfactualAnalysis> {
        let mut counterfactuals = Vec::new();

        // Generate hypothesis-specific counterfactual data
        let (counterfactual_text, required_conditions, evidence_needed) = match hypothesis.hypothesis_type {
            HypothesisType::Expanding => (
                "What if this entity were actually contracting instead?",
                vec![
                    "Locate evidence of facility closures or downsizing".to_string(),
                    "Identify layoff announcements or hiring freezes".to_string(),
                    "Check for reduced R&D or patent filing activity".to_string(),
                    "Verify if expansion signals are misinterpreted seasonal hiring".to_string(),
                ],
                vec![
                    "Sec filings indicating restructuring".to_string(),
                    "Employee count trends on LinkedIn/Glassdoor".to_string(),
                    "Lease termination or sublease listings for facilities".to_string(),
                ],
            ),
            HypothesisType::Contracting => (
                "What if this entity were actually expanding?",
                vec![
                    "Search for undisclosed funding rounds or capital raises".to_string(),
                    "Check for stealth-mode product development".to_string(),
                    "Identify new job postings in growth areas".to_string(),
                    "Verify if contraction is seasonal rather than structural".to_string(),
                ],
                vec![
                    "Crunchbase or PitchBook funding data".to_string(),
                    "Patent filings in new technology areas".to_string(),
                    "Executive hiring announcements on LinkedIn".to_string(),
                ],
            ),
            HypothesisType::Pivoting => (
                "What if this entity were maintaining its current direction?",
                vec![
                    "Analyze whether new initiatives are experimental vs. strategic shifts".to_string(),
                    "Check if leadership changes signal evolution vs. transformation".to_string(),
                    "Compare current capability investments against historical patterns".to_string(),
                    "Assess if pivot signals are defensive reactions to market pressure".to_string(),
                ],
                vec![
                    "Historical strategy documents and investor presentations".to_string(),
                    "Quarterly earnings call transcripts for strategic language".to_string(),
                    "Technology stack analysis over time".to_string(),
                ],
            ),
            HypothesisType::FinancialStress => (
                "What if this entity had no financial issues?",
                vec![
                    "Audit cash flow statements and burn rate projections".to_string(),
                    "Verify if cost-cutting measures are strategic rather than distress-driven".to_string(),
                    "Check for insider selling patterns versus normal trading".to_string(),
                    "Assess debt covenant compliance and refinancing options".to_string(),
                ],
                vec![
                    "Detailed financial statements (10-K, 10-Q)".to_string(),
                    "Credit rating agency reports".to_string(),
                    "Debt maturity schedule and refinancing status".to_string(),
                ],
            ),
            HypothesisType::EmergingThreat => (
                "What if this entity posed no competitive threat?",
                vec![
                    "Compare actual market share gain against claimed capabilities".to_string(),
                    "Assess whether technology differentiators are truly defensible".to_string(),
                    "Check for customer churn or dissatisfaction indicators".to_string(),
                    "Evaluate if threat is overblown by market analysts".to_string(),
                ],
                vec![
                    "Customer reviews and satisfaction surveys".to_string(),
                    "Product comparison benchmarks".to_string(),
                    "Market share data from independent analysts".to_string(),
                ],
            ),
            HypothesisType::SuspiciousActivity => (
                "What if this activity were completely legitimate?",
                vec![
                    "Cross-reference transactions against industry norms".to_string(),
                    "Verify if unusual patterns have benign explanations".to_string(),
                    "Check for regulatory filings that legitimize the activity".to_string(),
                    "Assess whether competitors engage in similar practices".to_string(),
                ],
                vec![
                    "Regulatory filing databases (SEC, FCA, etc.)".to_string(),
                    "Industry association membership records".to_string(),
                    "Independent audit reports".to_string(),
                ],
            ),
            HypothesisType::HiddenRelationships => (
                "What if all relationships were transparent?",
                vec![
                    "Check corporate registry for common directors/officers".to_string(),
                    "Analyze IP assignment chains for undisclosed connections".to_string(),
                    "Verify beneficial ownership through shell company disclosures".to_string(),
                    "Investigate shared service providers or legal representation".to_string(),
                ],
                vec![
                    "Corporate registry filings (OpenCorporates, etc.)".to_string(),
                    "IP assignment records from patent offices".to_string(),
                    "Beneficial ownership registries".to_string(),
                ],
            ),
            HypothesisType::Custom => (
                "What if this hypothesis were false?",
                vec![
                    "Re-examine all evidence sources for methodological bias".to_string(),
                    "Seek independent third-party verification".to_string(),
                    "Consider alternative explanations for observed signals".to_string(),
                ],
                vec![
                    "Additional independent data sources".to_string(),
                    "Expert review of evidence chain".to_string(),
                ],
            ),
        };

        counterfactuals.push(CounterfactualAnalysis {
            original_hypothesis: hypothesis.label.clone(),
            counterfactual: counterfactual_text.to_string(),
            required_conditions,
            probability: 1.0 - hypothesis.posterior,
            evidence_needed,
        });

        counterfactuals
    }

    /// Perform gap analysis for an entity.
    pub fn analyze_gaps(&self, entity_id: &str, existing_data: &EntityDataProfile) -> GapAnalysis {
        let mut financial_gaps = Vec::new();
        let mut operational_gaps = Vec::new();
        let mut leadership_gaps = Vec::new();
        let mut supply_chain_gaps = Vec::new();
        let mut compliance_gaps = Vec::new();

        // Check financial gaps
        if !existing_data.has_financial_reports {
            financial_gaps.push("No recent financial reports available".to_string());
        }
        if !existing_data.has_revenue_data {
            financial_gaps.push("Revenue estimates unavailable".to_string());
        }
        if !existing_data.has_stock_data {
            financial_gaps.push("Stock/valuation data unavailable".to_string());
        }

        // Check operational gaps
        if !existing_data.has_capacity_info {
            operational_gaps.push("No capacity information available".to_string());
        }
        if !existing_data.has_facility_data {
            operational_gaps.push("Facility information incomplete".to_string());
        }

        // Check leadership gaps
        if !existing_data.has_key_personnel {
            leadership_gaps.push("Key personnel not identified".to_string());
        }
        if !existing_data.has_board_info {
            leadership_gaps.push("Board/governance information missing".to_string());
        }

        // Check supply chain gaps
        if !existing_data.has_supplier_data {
            supply_chain_gaps.push("Supplier information unavailable".to_string());
        }
        if !existing_data.has_customer_data {
            supply_chain_gaps.push("Customer/distribution information missing".to_string());
        }

        // Check compliance gaps
        if !existing_data.has_certifications {
            compliance_gaps.push("No certification data available".to_string());
        }
        if !existing_data.has_regulatory_filings {
            compliance_gaps.push("Regulatory filing history incomplete".to_string());
        }

        // Calculate completeness score
        let total_gaps = financial_gaps.len()
            + operational_gaps.len()
            + leadership_gaps.len()
            + supply_chain_gaps.len()
            + compliance_gaps.len();
        let max_gaps = 10;
        let completeness_score = 1.0 - (total_gaps.min(max_gaps) as f64 / max_gaps as f64);

        // Priority gaps are those in critical categories
        let mut priority_gaps = Vec::new();
        priority_gaps.extend(financial_gaps.iter().take(2).cloned());
        priority_gaps.extend(compliance_gaps.iter().take(2).cloned());
        priority_gaps.extend(leadership_gaps.iter().take(1).cloned());

        GapAnalysis {
            entity_id: entity_id.to_string(),
            financial_gaps,
            operational_gaps,
            leadership_gaps,
            supply_chain_gaps,
            compliance_gaps,
            completeness_score,
            priority_gaps,
        }
    }

    /// Calculate investigation priority based on hypothesis scores and gaps.
    pub fn calculate_priority(&self, hypotheses: &[Hypothesis], gaps: &GapAnalysis) -> InvestigationPriority {
        // Factor in the leading hypothesis posterior
        let top_posterior = hypotheses
            .first()
            .map(|h| h.posterior)
            .unwrap_or(0.5);

        // Factor in information completeness
        let information_factor = 1.0 - gaps.completeness_score;

        // Combined urgency score
        let urgency_score = top_posterior * 0.6 + information_factor * 0.4;

        InvestigationPriority::from_score(urgency_score)
    }
}

/// Profile of what data exists for an entity.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[derive(Default)]
pub struct EntityDataProfile {
    pub has_financial_reports: bool,
    pub has_revenue_data: bool,
    pub has_stock_data: bool,
    pub has_capacity_info: bool,
    pub has_facility_data: bool,
    pub has_key_personnel: bool,
    pub has_board_info: bool,
    pub has_supplier_data: bool,
    pub has_customer_data: bool,
    pub has_certifications: bool,
    pub has_regulatory_filings: bool,
}


impl EntityDataProfile {
    /// Build an enriched data profile from signal evidence items.
    ///
    /// Scans signal `evidence_type` and `raw_data` fields to populate
    /// financial, network, identity, sector, geopolitical, and supply chain flags.
    pub fn from_signals(signals: &[super::reasoning::EvidenceItem]) -> Self {
        let mut profile = Self::default();

        let mut known_aliases: Vec<String> = Vec::new();
        let mut known_sectors: Vec<String> = Vec::new();

        for signal in signals {
            let etype = signal.evidence_type.to_lowercase();

            // Financial data indicators
            if etype.contains("financial")
                || etype.contains("revenue")
                || etype.contains("earnings")
                || etype.contains("stock")
                || etype.contains("market_cap")
            {
                profile.has_financial_reports = true;
                profile.has_revenue_data = true;
                profile.has_stock_data = true;
            }

            // Network / technical signal indicators
            if etype.contains("dns")
                || etype.contains("certificate")
                || etype.contains("whois")
                || etype.contains("network")
                || etype.contains("technical")
            {
                profile.has_capacity_info = true;
            }

            // Identity / alias data
            if etype.contains("alias")
                || etype.contains("aka")
                || etype.contains("also_known")
                || etype.contains("identity")
            {
                if let Some(alias) = signal.raw_data.get("alias").and_then(|v| v.as_str()) {
                    known_aliases.push(alias.to_string());
                }
            }

            // Sector data
            if etype.contains("sector")
                || etype.contains("industry")
                || etype.contains("vertical")
            {
                if let Some(sector) = signal.raw_data.get("sector").and_then(|v| v.as_str()) {
                    known_sectors.push(sector.to_string());
                }
            }

            // Geopolitical signals
            if etype.contains("geopolitical")
                || etype.contains("sanction")
                || etype.contains("political")
                || etype.contains("trade")
            {
                profile.has_regulatory_filings = true;
            }

            // Supply chain data
            if etype.contains("supply")
                || etype.contains("supplier")
                || etype.contains("logistics")
                || etype.contains("vendor")
            {
                profile.has_supplier_data = true;
            }

            // Capacity / facility data
            if etype.contains("facility")
                || etype.contains("capacity")
                || etype.contains("plant")
                || etype.contains("manufacturing")
            {
                profile.has_capacity_info = true;
                profile.has_facility_data = true;
            }

            // Personnel data
            if etype.contains("personnel")
                || etype.contains("hiring")
                || etype.contains("executive")
                || etype.contains("leadership")
                || etype.contains("ceo")
                || etype.contains("cto")
            {
                profile.has_key_personnel = true;
            }

            // Board / governance data
            if etype.contains("board")
                || etype.contains("governance")
                || etype.contains("director")
                || etype.contains("chairman")
                || etype.contains("board_member")
            {
                profile.has_board_info = true;
            }

            // Customer / client / distribution data
            if etype.contains("customer")
                || etype.contains("client")
                || etype.contains("distribution")
                || etype.contains("partnership")
                || etype.contains("reseller")
                || etype.contains("channel")
            {
                profile.has_customer_data = true;
            }

            // Certification / compliance
            if etype.contains("certif")
                || etype.contains("iso")
                || etype.contains("compliance")
            {
                profile.has_certifications = true;
            }
        }

        profile
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_evidence_type_from_signal() {
        assert!(matches!(
            EvidenceType::from_signal_type("job_posting"),
            EvidenceType::JobPosting
        ));
        assert!(matches!(
            EvidenceType::from_signal_type("patent_filing"),
            EvidenceType::PatentFiling
        ));
    }

    #[test]
    fn test_hypothesis_generation() {
        let generator = HypothesisGenerator::new();
        let signals = vec![
            SignalEvidence {
                signal_id: "sig1".to_string(),
                signal_type: "job_posting".to_string(),
                entity_id: "ent1".to_string(),
                entity_name: "Test Corp".to_string(),
                confidence: 0.9,
                observed_at: Utc::now(),
                source: "LinkedIn".to_string(),
                raw_data: serde_json::json!({}),
            },
            SignalEvidence {
                signal_id: "sig2".to_string(),
                signal_type: "patent_filing".to_string(),
                entity_id: "ent1".to_string(),
                entity_name: "Test Corp".to_string(),
                confidence: 0.8,
                observed_at: Utc::now(),
                source: "USPTO".to_string(),
                raw_data: serde_json::json!({}),
            },
        ];

        let hypotheses = generator.generate_hypotheses("ent1", "Test Corp", &signals);
        
        // Should generate Expanding hypothesis given job postings and patents
        assert!(!hypotheses.is_empty());
    }

    #[test]
    fn test_priority_calculation() {
        let generator = HypothesisGenerator::new();
        
        let mut hypothesis = Hypothesis::new(HypothesisType::Expanding, 7);
        hypothesis.posterior = 0.8;

        let gaps = GapAnalysis {
            entity_id: "test".to_string(),
            financial_gaps: vec!["Missing".to_string()],
            operational_gaps: vec![],
            leadership_gaps: vec![],
            supply_chain_gaps: vec![],
            compliance_gaps: vec![],
            completeness_score: 0.7,
            priority_gaps: vec![],
        };

        let priority = generator.calculate_priority(&[hypothesis], &gaps);
        // With 0.8 posterior and 0.3 information gap (1 - 0.7), urgency = 0.8*0.6 + 0.3*0.4 = 0.6
        // This should result in Medium priority (>= 0.5 but < 0.7)
        assert!(matches!(priority, InvestigationPriority::Medium | InvestigationPriority::High | InvestigationPriority::Critical));
    }
}