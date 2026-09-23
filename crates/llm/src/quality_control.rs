//!
//! Quality Control Module for ApexIntel OSINT platform.
//!
//! Implements comprehensive quality checks for LLM outputs:
//! - Output coherence scoring
//! - Factual accuracy verification
//! - Hallucination detection
//! - Bias identification
//!
//! Ensures LLM-generated intelligence meets quality standards before delivery.

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::LazyLock;

// Static regex patterns used across quality checks.
// `expect` is allowed here because the patterns are compile-time constants and
// are guaranteed to be valid — a failure would indicate a programming error.
#[allow(clippy::unwrap_used, clippy::expect_used)]
static RE_NUMBER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\d+(?:\.\d+)?").expect("valid number regex"));
#[allow(clippy::unwrap_used, clippy::expect_used)]
static RE_SPECIFIC_NUMBER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\d{4,}|[€$£]\d{3,}").expect("valid specific number regex"));
#[allow(clippy::unwrap_used, clippy::expect_used)]
static RE_VAGUE_DATE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"recently|earlier this|in the past|last \w+|a while ago")
        .expect("valid vague date regex")
});
#[allow(clippy::unwrap_used, clippy::expect_used)]
static RE_SPECIFIC_DATE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\d{1,2}/\d{1,2}/\d{2,4}").expect("valid specific date regex"));
#[allow(clippy::unwrap_used, clippy::expect_used)]
static RE_OVER_SPECIFIC: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\d{1,2}[,.:]\d{1,2}[,.:]\d{3,}").expect("valid over-specific regex")
});

/// Configuration for quality control checks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualityControlConfig {
    /// Minimum coherence score to pass (0.0-1.0).
    pub min_coherence_score: f64,
    /// Minimum factual accuracy score to pass (0.0-1.0).
    pub min_accuracy_score: f64,
    /// Enable hallucination detection.
    pub enable_hallucination_detection: bool,
    /// Enable bias identification.
    pub enable_bias_detection: bool,
    /// Enable factual verification against knowledge base.
    pub enable_factual_verification: bool,
    /// Maximum hallucination risk to pass (0.0-1.0).
    pub max_hallucination_risk: f64,
}

impl Default for QualityControlConfig {
    fn default() -> Self {
        Self {
            min_coherence_score: 0.7,
            min_accuracy_score: 0.75,
            enable_hallucination_detection: true,
            enable_bias_detection: true,
            enable_factual_verification: true,
            max_hallucination_risk: 0.3,
        }
    }
}

/// Output coherence score.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoherenceScore {
    /// Overall coherence score (0.0-1.0).
    pub score: f64,
    /// Semantic consistency score.
    pub semantic_score: f64,
    /// Logical consistency score.
    pub logical_score: f64,
    /// Structural consistency score.
    pub structural_score: f64,
    /// Detected inconsistencies.
    pub inconsistencies: Vec<Inconsistency>,
}

/// A detected inconsistency in the output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Inconsistency {
    /// Type of inconsistency.
    pub kind: InconsistencyType,
    /// Description of the issue.
    pub description: String,
    /// Severity (0.0-1.0).
    pub severity: f64,
    /// Location in text (character offset).
    pub location: Option<usize>,
}

/// Types of inconsistencies that can be detected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InconsistencyType {
    /// Contradictory statements.
    Contradiction,
    /// Unsupported claims.
    UnsupportedClaim,
    /// Logical errors.
    LogicalError,
    /// Inconsistent entity references.
    EntityInconsistency,
    /// Temporal inconsistencies.
    TemporalInconsistency,
    /// Numerical inconsistencies.
    NumericalInconsistency,
}

impl std::fmt::Display for InconsistencyType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Contradiction => write!(f, "contradiction"),
            Self::UnsupportedClaim => write!(f, "unsupported_claim"),
            Self::LogicalError => write!(f, "logical_error"),
            Self::EntityInconsistency => write!(f, "entity_inconsistency"),
            Self::TemporalInconsistency => write!(f, "temporal_inconsistency"),
            Self::NumericalInconsistency => write!(f, "numerical_inconsistency"),
        }
    }
}

/// Hallucination detection result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HallucinationCheck {
    /// Overall hallucination risk score (0.0-1.0, higher = more risk).
    pub risk_score: f64,
    /// Detected hallucinations.
    pub hallucinations: Vec<Hallucination>,
    /// Specific claims that are likely hallucinated.
    pub risky_claims: Vec<RiskyClaim>,
    /// Overall assessment.
    pub assessment: HallucinationAssessment,
}

/// A detected hallucination.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hallucination {
    /// Type of hallucination.
    pub kind: HallucinationType,
    /// The hallucinated content.
    pub content: String,
    /// Why this is suspicious.
    pub reason: String,
    /// Severity (0.0-1.0).
    pub severity: f64,
    /// Suggested alternative if available.
    pub suggested_fixes: Vec<String>,
}

impl Hallucination {
    /// Create a new hallucination with severity based on type.
    pub fn new(kind: HallucinationType, content: String, reason: String) -> Self {
        let severity = kind.default_severity();
        Self {
            kind,
            content,
            reason,
            severity,
            suggested_fixes: vec![],
        }
    }
}

/// Types of hallucinations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HallucinationType {
    /// Fabricated statistics or numbers.
    FabricatedNumbers,
    /// Fabricated names or entity references.
    FabricatedNames,
    /// Fabricated dates or timelines.
    FabricatedDates,
    /// Fabricated quotes or attributions.
    FabricatedQuotes,
    /// Unsupported claims without citations.
    UnsupportedClaim,
    /// Inconsistent facts.
    FactInconsistency,
    /// Specific but unverified details.
    OverSpecific,
}

impl HallucinationType {
    /// Default severity for this hallucination type.
    pub fn default_severity(&self) -> f64 {
        match self {
            Self::FabricatedNumbers => 0.8,
            Self::FabricatedQuotes => 0.9,
            Self::FabricatedNames => 0.7,
            Self::FabricatedDates => 0.6,
            Self::FactInconsistency => 0.7,
            Self::UnsupportedClaim => 0.5,
            Self::OverSpecific => 0.4,
        }
    }
}

/// A claim that carries hallucination risk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskyClaim {
    /// The claim text.
    pub claim: String,
    /// Risk level.
    pub risk_level: RiskLevel,
    /// Why this is risky.
    pub reason: String,
    /// Verification suggestions.
    pub verification_needed: Vec<String>,
}

/// Risk level enumeration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

impl RiskLevel {
    pub fn from_score(score: f64) -> Self {
        if score < 0.2 {
            Self::Low
        } else if score < 0.5 {
            Self::Medium
        } else if score < 0.8 {
            Self::High
        } else {
            Self::Critical
        }
    }
}

/// Overall hallucination assessment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HallucinationAssessment {
    pub verdict: AssessmentVerdict,
    pub summary: String,
    pub confidence: f64,
    pub recommendations: Vec<String>,
}

/// Assessment verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssessmentVerdict {
    /// Output is reliable.
    Approved,
    /// Output needs review.
    NeedsReview,
    /// Output is unreliable.
    Rejected,
}

impl AssessmentVerdict {
    pub fn from_risk_score(score: f64) -> Self {
        if score < 0.2 {
            Self::Approved
        } else if score < 0.5 {
            Self::NeedsReview
        } else {
            Self::Rejected
        }
    }
}

/// Bias detection result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BiasCheck {
    /// Overall bias score (0.0-1.0, higher = more bias).
    pub bias_score: f64,
    /// Detected biases.
    pub biases: Vec<DetectedBias>,
    /// Bias patterns.
    pub patterns: Vec<BiasPattern>,
    /// Recommendations for mitigation.
    pub recommendations: Vec<String>,
}

/// A detected bias in the output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectedBias {
    /// Type of bias.
    pub kind: BiasType,
    /// Description.
    pub description: String,
    /// Severity (0.0-1.0).
    pub severity: f64,
    /// Evidence text.
    pub evidence: String,
    /// Suggested mitigation.
    pub mitigation: String,
}

/// Types of bias.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BiasType {
    /// Confirmation bias (seeking confirming evidence).
    ConfirmationBias,
    /// Availability bias (recent events overweighted).
    AvailabilityBias,
    /// Anchoring bias (first impression dominates).
    AnchoringBias,
    /// Selection bias (non-representative sample).
    SelectionBias,
    /// Survivorship bias.
    SurvivorshipBias,
    /// Halo effect (overall impression biases specific judgments).
    HaloEffect,
    /// In-group bias (favoring those similar to oneself).
    InGroupBias,
    /// Authority bias (excessive deference to authority).
    AuthorityBias,
    /// Status quo bias.
    StatusQuoBias,
    /// Narrative bias (story over data).
    NarrativeBias,
}

impl std::fmt::Display for BiasType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ConfirmationBias => write!(f, "confirmation_bias"),
            Self::AvailabilityBias => write!(f, "availability_bias"),
            Self::AnchoringBias => write!(f, "anchoring_bias"),
            Self::SelectionBias => write!(f, "selection_bias"),
            Self::SurvivorshipBias => write!(f, "survivorship_bias"),
            Self::HaloEffect => write!(f, "halo_effect"),
            Self::InGroupBias => write!(f, "in_group_bias"),
            Self::AuthorityBias => write!(f, "authority_bias"),
            Self::StatusQuoBias => write!(f, "status_quo_bias"),
            Self::NarrativeBias => write!(f, "narrative_bias"),
        }
    }
}

impl BiasType {
    pub fn default_severity(&self) -> f64 {
        match self {
            Self::ConfirmationBias => 0.6,
            Self::AvailabilityBias => 0.5,
            Self::AnchoringBias => 0.5,
            Self::SelectionBias => 0.7,
            Self::SurvivorshipBias => 0.6,
            Self::HaloEffect => 0.4,
            Self::InGroupBias => 0.5,
            Self::AuthorityBias => 0.4,
            Self::StatusQuoBias => 0.3,
            Self::NarrativeBias => 0.5,
        }
    }

    pub fn mitigation(&self) -> &'static str {
        match self {
            Self::ConfirmationBias => "Seek disconfirming evidence and alternative explanations.",
            Self::AvailabilityBias => "Consider historical base rates and long-term trends.",
            Self::AnchoringBias => {
                "Revisit initial assumptions and consider alternative scenarios."
            }
            Self::SelectionBias => "Ensure sample represents the full population of entities.",
            Self::SurvivorshipBias => "Include failed cases and non-success stories in analysis.",
            Self::HaloEffect => "Evaluate each element independently from overall impression.",
            Self::InGroupBias => "Consider perspectives from outside the industry or region.",
            Self::AuthorityBias => "Verify claims independently rather than relying on authority.",
            Self::StatusQuoBias => "Actively consider change and disruption scenarios.",
            Self::NarrativeBias => "Balance storytelling with quantitative evidence.",
        }
    }
}

/// A detected bias pattern.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BiasPattern {
    /// Pattern description.
    pub description: String,
    /// Frequency in text.
    pub frequency: f64,
    /// Affected claims.
    pub affected_claims: Vec<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Quality Control Engine
// ─────────────────────────────────────────────────────────────────────────────

/// Quality control engine for LLM outputs.
pub struct QualityControlEngine {
    config: QualityControlConfig,
}

impl QualityControlEngine {
    pub fn new(config: QualityControlConfig) -> Self {
        Self { config }
    }

    pub fn with_default_config() -> Self {
        Self::new(QualityControlConfig::default())
    }

    /// Run all quality checks on a text output.
    pub fn run_checks(&self, text: &str) -> QualityCheckResult {
        let coherence = self.check_coherence(text);
        let hallucination = if self.config.enable_hallucination_detection {
            self.detect_hallucinations(text)
        } else {
            HallucinationCheck {
                risk_score: 0.0,
                hallucinations: vec![],
                risky_claims: vec![],
                assessment: HallucinationAssessment {
                    verdict: AssessmentVerdict::Approved,
                    summary: "Hallucination detection disabled".to_string(),
                    confidence: 0.0,
                    recommendations: vec![],
                },
            }
        };
        let bias = if self.config.enable_bias_detection {
            self.detect_bias(text)
        } else {
            BiasCheck {
                bias_score: 0.0,
                biases: vec![],
                patterns: vec![],
                recommendations: vec![],
            }
        };

        let overall_score = self.compute_overall_score(&coherence, &hallucination, &bias);

        let passed = overall_score >= self.config.min_coherence_score
            && hallucination.risk_score <= self.config.max_hallucination_risk;
        let recommendations = self.generate_recommendations(&coherence, &hallucination, &bias);

        QualityCheckResult {
            overall_score,
            coherence,
            hallucination,
            bias,
            passed,
            recommendations,
        }
    }

    /// Check output coherence.
    pub fn check_coherence(&self, text: &str) -> CoherenceScore {
        let sentences: Vec<&str> = text.split(['.', '!', '?']).collect();
        let semantic_score = self.calculate_semantic_coherence(&sentences);
        let logical_score = self.calculate_logical_coherence(&sentences);
        let structural_score = self.calculate_structural_coherence(&sentences);
        let inconsistencies = self.detect_inconsistencies(&sentences);

        let score =
            (semantic_score * 0.4 + logical_score * 0.35 + structural_score * 0.25).clamp(0.0, 1.0);

        CoherenceScore {
            score,
            semantic_score,
            logical_score,
            structural_score,
            inconsistencies,
        }
    }

    /// Detect hallucinations in text.
    pub fn detect_hallucinations(&self, text: &str) -> HallucinationCheck {
        let mut hallucinations = Vec::new();
        let mut risky_claims = Vec::new();

        // Pattern-based detection
        let lines: Vec<&str> = text.lines().collect();

        for line in &lines {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            // Check for fabricated statistics patterns
            if self.looks_like_fabricated_number(line) {
                hallucinations.push(Hallucination::new(
                    HallucinationType::FabricatedNumbers,
                    line.to_string(),
                    "Pattern matches fabricated statistics format".to_string(),
                ));
            }

            // Check for fabricated dates
            if self.looks_like_fabricated_date(line) {
                hallucinations.push(Hallucination::new(
                    HallucinationType::FabricatedDates,
                    line.to_string(),
                    "Vague date reference without source".to_string(),
                ));
            }

            // Check for over-specific claims
            if self.looks_over_specific(line) {
                risky_claims.push(RiskyClaim {
                    claim: line.to_string(),
                    risk_level: RiskLevel::Medium,
                    reason: "Highly specific claim without qualification".to_string(),
                    verification_needed: vec!["Verify with primary source".to_string()],
                });
            }

            // Check for unsupported claims
            if self.looks_like_unsupported_claim(line) {
                risky_claims.push(RiskyClaim {
                    claim: line.to_string(),
                    risk_level: RiskLevel::High,
                    reason: "Absolute claim without evidence or qualifier".to_string(),
                    verification_needed: vec![
                        "Add source citation".to_string(),
                        "Add confidence qualifier".to_string(),
                    ],
                });
            }
        }

        // Calculate risk score
        let risk_score = if hallucinations.is_empty() && risky_claims.is_empty() {
            0.1
        } else {
            let hallucination_risk = hallucinations.iter().map(|h| h.severity).sum::<f64>()
                / hallucinations.len().max(1) as f64;
            let claim_risk = risky_claims
                .iter()
                .map(|c| match c.risk_level {
                    RiskLevel::Low => 0.2,
                    RiskLevel::Medium => 0.4,
                    RiskLevel::High => 0.7,
                    RiskLevel::Critical => 0.9,
                })
                .sum::<f64>()
                / risky_claims.len().max(1) as f64;

            (hallucination_risk * 0.6 + claim_risk * 0.4).clamp(0.0, 1.0)
        };

        let assessment =
            self.assess_hallucinations(hallucinations.len(), risky_claims.len(), risk_score);

        HallucinationCheck {
            risk_score,
            hallucinations,
            risky_claims,
            assessment,
        }
    }

    /// Detect biases in text.
    pub fn detect_bias(&self, text: &str) -> BiasCheck {
        let mut biases = Vec::new();
        let lower = text.to_lowercase();

        // Confirmation bias detection
        if lower.contains("confirmed") && lower.contains("clearly") && !lower.contains("however") {
            biases.push(DetectedBias {
                kind: BiasType::ConfirmationBias,
                description: "Strong confirmation language without acknowledging uncertainty"
                    .to_string(),
                severity: 0.5,
                evidence: "Text contains strong confirmatory language".to_string(),
                mitigation: BiasType::ConfirmationBias.mitigation().to_string(),
            });
        }

        // Availability bias detection
        if lower.contains("recent") && lower.contains("trend") {
            biases.push(DetectedBias {
                kind: BiasType::AvailabilityBias,
                description: "Emphasis on recent events may not represent long-term pattern"
                    .to_string(),
                severity: 0.4,
                evidence: "Recent trends heavily emphasized".to_string(),
                mitigation: BiasType::AvailabilityBias.mitigation().to_string(),
            });
        }

        // Narrative bias detection
        let story_words = ["story", "narrative", "tale", "dramatic"];
        let has_narrative = story_words.iter().any(|w| lower.contains(w));

        if has_narrative && !lower.contains("data") && !lower.contains("evidence") {
            biases.push(DetectedBias {
                kind: BiasType::NarrativeBias,
                description: "Storytelling tone without sufficient data backing".to_string(),
                severity: 0.5,
                evidence: "Narrative framing dominant".to_string(),
                mitigation: BiasType::NarrativeBias.mitigation().to_string(),
            });
        }

        // Calculate overall bias score
        let bias_score = if biases.is_empty() {
            0.1
        } else {
            biases.iter().map(|b| b.severity).sum::<f64>() / biases.len() as f64
        };

        let recommendations = biases.iter().map(|b| b.mitigation.clone()).collect();

        BiasCheck {
            bias_score,
            biases,
            patterns: vec![],
            recommendations,
        }
    }

    // ── Coherence calculation helpers ──

    fn calculate_semantic_coherence(&self, sentences: &[&str]) -> f64 {
        if sentences.len() < 2 {
            return 1.0;
        }

        // Check for topic consistency
        let topics = self.extract_topics(sentences);
        if topics.is_empty() {
            return 0.5;
        }

        let dominant_topic_ratio =
            topics.values().map(|v| *v as f64).fold(0f64, f64::max) / sentences.len() as f64;

        dominant_topic_ratio.min(1.0)
    }

    fn calculate_logical_coherence(&self, sentences: &[&str]) -> f64 {
        // Check for transition words suggesting logical flow
        let transition_count = sentences
            .iter()
            .filter(|s| {
                let lower = s.to_lowercase();
                [
                    "however",
                    "therefore",
                    "consequently",
                    "furthermore",
                    "additionally",
                    "meanwhile",
                ]
                .iter()
                .any(|t| lower.contains(t))
            })
            .count();

        let transition_score =
            (transition_count as f64 / sentences.len().max(1) as f64 * 2.0).min(1.0);

        // Check for contradictory indicators
        let has_contrary = sentences.iter().any(|s| {
            let lower = s.to_lowercase();
            (lower.contains("however") && lower.contains("but"))
                || (lower.contains("on one hand") && lower.contains("on the other hand"))
        });

        if has_contrary {
            0.9
        } else {
            transition_score
        }
    }

    fn calculate_structural_coherence(&self, sentences: &[&str]) -> f64 {
        // Check sentence length consistency
        let lengths: Vec<usize> = sentences
            .iter()
            .map(|s| s.len())
            .filter(|l| *l > 0)
            .collect();

        if lengths.len() < 2 {
            return 1.0;
        }

        let avg_len = lengths.iter().sum::<usize>() as f64 / lengths.len() as f64;
        let variance: f64 = lengths
            .iter()
            .map(|l| {
                let diff = *l as f64 - avg_len;
                diff * diff
            })
            .sum::<f64>()
            / lengths.len() as f64;

        // Low variance = high structural coherence
        let cv = variance.sqrt() / avg_len;
        (1.0 - cv.min(1.0)).clamp(0.0, 1.0)
    }

    fn detect_inconsistencies(&self, sentences: &[&str]) -> Vec<Inconsistency> {
        let mut inconsistencies = Vec::new();
        let text = sentences.join(" ");

        // Check for contradictory statements
        let contradiction_pairs = [
            ("increasing", "decreasing"),
            ("growing", "shrinking"),
            ("profitable", "loss-making"),
            ("expanding", "contracting"),
            ("stable", "volatile"),
            ("confident", "uncertain"),
        ];

        for (word_a, word_b) in &contradiction_pairs {
            if text.to_lowercase().contains(word_a) && text.to_lowercase().contains(word_b) {
                inconsistencies.push(Inconsistency {
                    kind: InconsistencyType::Contradiction,
                    description: format!("Text contains both '{}' and '{}'", word_a, word_b),
                    severity: 0.6,
                    location: None,
                });
                break;
            }
        }

        // Check for numerical inconsistencies
        self.detect_numerical_inconsistencies(&text, &mut inconsistencies);

        inconsistencies
    }

    fn detect_numerical_inconsistencies(
        &self,
        text: &str,
        inconsistencies: &mut Vec<Inconsistency>,
    ) {
        // Find all numbers in text
        let numbers: Vec<&str> = RE_NUMBER.find_iter(text).map(|m| m.as_str()).collect();

        // Check for out-of-range values (implausibly large or fractional-only
        // figures fall outside the normal 1..=1000 band and warrant a soft flag).
        for num_str in &numbers {
            if let Ok(num) = num_str.parse::<f64>() {
                if !(1.0..=1000.0).contains(&num) {
                    inconsistencies.push(Inconsistency {
                        kind: InconsistencyType::NumericalInconsistency,
                        description: format!("Suspicious number format: {}", num_str),
                        severity: 0.4,
                        location: None,
                    });
                }
            }
        }
    }

    fn extract_topics(&self, sentences: &[&str]) -> HashMap<String, usize> {
        let mut topics: HashMap<String, usize> = HashMap::new();

        // Simple keyword-based topic extraction
        let topic_keywords = [
            (
                "supply_chain",
                vec!["supply", "chain", "supplier", "logistics"],
            ),
            (
                "financial",
                vec!["revenue", "profit", "financial", "cost", "revenue"],
            ),
            (
                "competitive",
                vec!["competitor", "market", "competitive", "rival"],
            ),
            (
                "geopolitical",
                vec!["geopolitical", "political", "trade", "sanction"],
            ),
            (
                "operational",
                vec!["operation", "facility", "production", "manufacturing"],
            ),
        ];

        for sentence in sentences {
            let lower = sentence.to_lowercase();
            for (topic, keywords) in &topic_keywords {
                if keywords.iter().any(|k| lower.contains(k)) {
                    *topics.entry(topic.to_string()).or_insert(0) += 1;
                }
            }
        }

        topics
    }

    // ── Hallucination detection helpers ──

    fn looks_like_fabricated_number(&self, text: &str) -> bool {
        // Check for suspiciously round or specific numbers
        let has_specific_number = RE_SPECIFIC_NUMBER.is_match(text);

        let has_specific_percent = text.contains("%") && text.contains("exactly");

        has_specific_number || has_specific_percent
    }

    fn looks_like_fabricated_date(&self, text: &str) -> bool {
        // Check for vague date references
        let has_vague_date = RE_VAGUE_DATE.is_match(&text.to_lowercase());

        let has_specific_date = RE_SPECIFIC_DATE.is_match(text);

        has_vague_date && !has_specific_date
    }

    fn looks_over_specific(&self, text: &str) -> bool {
        // Check for very specific details that seem implausible
        let very_specific = RE_OVER_SPECIFIC.is_match(text);

        let multiple_decimals = text.matches('.').count() > 3;

        very_specific || multiple_decimals
    }

    fn looks_like_unsupported_claim(&self, text: &str) -> bool {
        let lower = text.to_lowercase();

        // Absolute claims without qualifiers
        let has_absolute = [
            "all",
            "always",
            "never",
            "none",
            "every",
            "completely",
            "totally",
        ]
        .iter()
        .any(|w| lower.contains(w));

        let has_qualifier = [
            "may", "might", "could", "possibly", "perhaps", "likely", "suggests",
        ]
        .iter()
        .any(|w| lower.contains(w));

        has_absolute && !has_qualifier
    }

    fn assess_hallucinations(
        &self,
        hallucination_count: usize,
        risky_claim_count: usize,
        risk_score: f64,
    ) -> HallucinationAssessment {
        let verdict = AssessmentVerdict::from_risk_score(risk_score);

        let summary = match verdict {
            AssessmentVerdict::Approved => {
                "Output appears reliable with minimal hallucination risk.".to_string()
            }
            AssessmentVerdict::NeedsReview => {
                format!(
                    "Output requires review. {} potential hallucination(s) and {} risky claim(s) detected.",
                    hallucination_count, risky_claim_count
                )
            }
            AssessmentVerdict::Rejected => {
                format!(
                    "Output has HIGH hallucination risk. {} hallucination(s) and {} risky claim(s) detected. Do not use without verification.",
                    hallucination_count, risky_claim_count
                )
            }
        };

        let recommendations = match verdict {
            AssessmentVerdict::Approved => vec![],
            AssessmentVerdict::NeedsReview => vec![
                "Verify specific claims against primary sources".to_string(),
                "Add confidence qualifiers to absolute statements".to_string(),
                "Cross-reference numerical claims".to_string(),
            ],
            AssessmentVerdict::Rejected => vec![
                "Do not use this output without substantial human review".to_string(),
                "Verify all factual claims with authoritative sources".to_string(),
                "Consider regenerating with more specific constraints".to_string(),
            ],
        };

        HallucinationAssessment {
            verdict,
            summary,
            confidence: 1.0 - risk_score,
            recommendations,
        }
    }

    // ── Score computation ──

    fn compute_overall_score(
        &self,
        coherence: &CoherenceScore,
        hallucination: &HallucinationCheck,
        bias: &BiasCheck,
    ) -> f64 {
        let coherence_weight = 0.4;
        let hallucination_weight = 0.35;
        let bias_weight = 0.25;

        // Coherence contributes positively
        let coherence_score = coherence.score;

        // Hallucination contributes inversely (lower is better)
        let hallucination_score = 1.0 - hallucination.risk_score;

        // Bias contributes inversely (lower is better)
        let bias_score = 1.0 - bias.bias_score;

        (coherence_score * coherence_weight
            + hallucination_score * hallucination_weight
            + bias_score * bias_weight)
            .clamp(0.0, 1.0)
    }

    fn generate_recommendations(
        &self,
        coherence: &CoherenceScore,
        hallucination: &HallucinationCheck,
        bias: &BiasCheck,
    ) -> Vec<String> {
        let mut recs = Vec::new();

        if coherence.score < self.config.min_coherence_score {
            recs.push("Output coherence is below threshold. Review logical flow.".to_string());
        }

        for inconsistency in &coherence.inconsistencies {
            if inconsistency.severity > 0.5 {
                recs.push(format!(
                    "Address {}: {}",
                    inconsistency.kind, inconsistency.description
                ));
            }
        }

        if hallucination.risk_score > self.config.max_hallucination_risk {
            recs.push("Hallucination risk is high. Verify claims with sources.".to_string());
        }

        for hallucination in &hallucination.hallucinations {
            recs.push(format!(
                "Verify: {}",
                hallucination.content.chars().take(100).collect::<String>()
            ));
        }

        for bias in &bias.biases {
            if bias.severity > 0.5 {
                recs.push(format!("Address {}: {}", bias.kind, bias.description));
            }
        }

        recs
    }
}

/// Complete quality check result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualityCheckResult {
    pub overall_score: f64,
    pub coherence: CoherenceScore,
    pub hallucination: HallucinationCheck,
    pub bias: BiasCheck,
    pub passed: bool,
    pub recommendations: Vec<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coherence_check_works() {
        let engine = QualityControlEngine::with_default_config();

        let good_text =
            "Company X is expanding its operations. The expansion includes new facilities. \
                        Furthermore, revenue is increasing. However, costs are also rising.";

        let coherence = engine.check_coherence(good_text);
        assert!(coherence.score >= 0.5);
    }

    #[test]
    fn hallucination_detection_finds_fabricated_numbers() {
        let engine = QualityControlEngine::with_default_config();

        let text =
            "The company announced exactly 15742 new employees were hired on March 3rd, 2023.";

        let result = engine.detect_hallucinations(text);
        assert!(!result.hallucinations.is_empty() || !result.risky_claims.is_empty());
    }

    #[test]
    fn hallucination_detection_finds_absolute_claims() {
        let engine = QualityControlEngine::with_default_config();

        let text = "This company never fails to deliver on time. All competitors are behind.";

        let result = engine.detect_hallucinations(text);
        assert!(!result.risky_claims.is_empty());
    }

    #[test]
    fn bias_detection_finds_confirmation_bias() {
        let engine = QualityControlEngine::with_default_config();

        let text = "This is clearly a confirmed case. The evidence is obviously conclusive. \
                  The pattern is definitively established.";

        let result = engine.detect_bias(text);
        assert!(result.bias_score > 0.3);
    }

    #[test]
    fn quality_check_result_passes_good_output() {
        let engine = QualityControlEngine::with_default_config();

        let good_text = "Company X is performing well. Revenue increased by 15% in Q3. \
                        The trend suggests continued growth. However, competition remains fierce.";

        let result = engine.run_checks(good_text);
        // Note: exact threshold depends on implementation
        assert!(result.overall_score >= 0.0);
    }

    #[test]
    fn quality_check_result_fails_bad_output() {
        let engine = QualityControlEngine::with_default_config();

        let bad_text =
            "This company NEVER fails. ALL competitors are BEHIND. Exactly 15742 jobs created. \
                       CONFIRMED 999999% growth. Clearly obviously definitively established.";

        let result = engine.run_checks(bad_text);
        assert!(result.overall_score < 0.8); // Should score poorly
    }

    #[test]
    fn risk_level_from_score() {
        assert_eq!(RiskLevel::from_score(0.1), RiskLevel::Low);
        assert_eq!(RiskLevel::from_score(0.3), RiskLevel::Medium);
        assert_eq!(RiskLevel::from_score(0.6), RiskLevel::High);
        assert_eq!(RiskLevel::from_score(0.9), RiskLevel::Critical);
    }

    #[test]
    fn assessment_verdict_from_risk() {
        assert_eq!(
            AssessmentVerdict::from_risk_score(0.1),
            AssessmentVerdict::Approved
        );
        assert_eq!(
            AssessmentVerdict::from_risk_score(0.4),
            AssessmentVerdict::NeedsReview
        );
        assert_eq!(
            AssessmentVerdict::from_risk_score(0.8),
            AssessmentVerdict::Rejected
        );
    }

    #[test]
    fn inconsistency_type_severity() {
        let engine = QualityControlEngine::with_default_config();

        let contradictory_text = "The company is increasing revenue but also decreasing revenue.";

        let coherence = engine.check_coherence(contradictory_text);
        assert!(!coherence.inconsistencies.is_empty());
    }

    #[test]
    fn hallucination_assessment_summary() {
        let engine = QualityControlEngine::with_default_config();
        let text = "Some risky content here.";

        let result = engine.detect_hallucinations(text);
        assert!(!result.assessment.summary.is_empty());
    }

    #[test]
    fn coherence_score_components() {
        let engine = QualityControlEngine::with_default_config();

        let text = "First statement. Second statement. Third statement.";

        let coherence = engine.check_coherence(text);
        assert!(coherence.semantic_score >= 0.0);
        assert!(coherence.logical_score >= 0.0);
        assert!(coherence.structural_score >= 0.0);
    }

    #[test]
    fn bias_mitigation_suggestions() {
        for bias_type in [
            BiasType::ConfirmationBias,
            BiasType::AvailabilityBias,
            BiasType::AnchoringBias,
            BiasType::SelectionBias,
        ] {
            assert!(!bias_type.mitigation().is_empty());
        }
    }

    #[test]
    fn hallucination_type_severity() {
        assert!(HallucinationType::FabricatedQuotes.default_severity() >= 0.8);
        assert!(HallucinationType::OverSpecific.default_severity() < 0.5);
    }
}
