//!
//! Intelligence Quality Assurance Module
//!
//! Phase 4.2: Implements comprehensive quality framework for OSINT insights:
//! - Source credibility scoring (authority, recency, corroboration)
//! - Output validation (accuracy, consistency, plausibility)
//! - Bias detection (confirmation, availability, geographic, interest conflicts)
//! - Continuous improvement through feedback loops
//!
//! # Key Components
//! - Source Credibility Scorer
//! - Output Validator  
//! - Bias Detector
//! - Quality Improvement Engine

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

// ============================================================================
// 4.2.1 Source Credibility Scoring
// ============================================================================

/// Configuration for source credibility scoring.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredibilityConfig {
    /// Minimum authority tier required (0=Unknown, 1=Social, 2=Commercial, 3=Government)
    pub min_authority_tier: u8,
    /// Maximum age in days before evidence is considered stale
    pub max_age_days: u32,
    /// Minimum number of corroborating sources for high credibility
    pub min_corroboration: usize,
    /// Weight for authority component (0.0-1.0)
    pub authority_weight: f64,
    /// Weight for recency component (0.0-1.0)
    pub recency_weight: f64,
    /// Weight for corroboration component (0.0-1.0)
    pub corroboration_weight: f64,
    /// Weight for completeness component (0.0-1.0)
    pub completeness_weight: f64,
}

impl Default for CredibilityConfig {
    fn default() -> Self {
        Self {
            min_authority_tier: 1, // At least Social tier
            max_age_days: 30,
            min_corroboration: 2,
            authority_weight: 0.35,
            recency_weight: 0.30,
            corroboration_weight: 0.20,
            completeness_weight: 0.15,
        }
    }
}

/// Source credibility score result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredibilityScore {
    /// Overall credibility score (0.0-1.0)
    pub overall: f64,
    /// Authority score component (0.0-1.0)
    pub authority_score: f64,
    /// Recency score component (0.0-1.0)
    pub recency_score: f64,
    /// Corroboration score component (0.0-1.0)
    pub corroboration_score: f64,
    /// Completeness score component (0.0-1.0)
    pub completeness_score: f64,
    /// Number of independent sources
    pub source_count: usize,
    /// Highest authority tier found
    pub highest_authority: u8,
    /// Most recent evidence timestamp
    pub most_recent: Option<DateTime<Utc>>,
    /// Quality flags
    pub flags: Vec<CredibilityFlag>,
}

impl CredibilityScore {
    /// Create a new credibility score with defaults.
    pub fn new() -> Self {
        Self {
            overall: 0.0,
            authority_score: 0.0,
            recency_score: 0.0,
            corroboration_score: 0.0,
            completeness_score: 0.0,
            source_count: 0,
            highest_authority: 0,
            most_recent: None,
            flags: Vec::new(),
        }
    }

    /// Check if credibility meets minimum threshold.
    pub fn is_credible(&self, min_threshold: f64) -> bool {
        self.overall >= min_threshold
    }

    /// Add a quality flag.
    pub fn add_flag(&mut self, flag: CredibilityFlag) {
        self.flags.push(flag);
    }

    /// Get human-readable quality level.
    pub fn quality_level(&self) -> &'static str {
        if self.overall >= 0.8 {
            "Excellent"
        } else if self.overall >= 0.6 {
            "Good"
        } else if self.overall >= 0.4 {
            "Fair"
        } else {
            "Poor"
        }
    }
}

impl Default for CredibilityScore {
    fn default() -> Self {
        Self::new()
    }
}

/// Quality flags for credibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CredibilityFlag {
    /// Only single source (no corroboration)
    SingleSource,
    /// Evidence older than threshold
    StaleEvidence,
    /// Social media only sources
    SocialMediaOnly,
    /// Government sources present
    HasGovernmentSource,
    /// High diversity of sources
    HighSourceDiversity,
    /// Incomplete evidence (missing fields)
    IncompleteEvidence,
    /// Contradictory sources detected
    ContradictorySources,
    /// Anonymous or unknown sources
    UnknownSources,
}

impl CredibilityFlag {
    /// Get severity weight for flag (0.0-1.0).
    pub fn severity_weight(&self) -> f64 {
        // Positive = penalty (bad), Negative = bonus (good)
        match self {
            Self::SingleSource => 0.2,
            Self::StaleEvidence => 0.15,
            Self::SocialMediaOnly => 0.25,
            Self::HasGovernmentSource => -0.15,
            Self::HighSourceDiversity => -0.1,
            Self::IncompleteEvidence => 0.1,
            Self::ContradictorySources => 0.3,
            Self::UnknownSources => 0.15,
        }
    }

    /// Get description.
    pub fn description(&self) -> &'static str {
        match self {
            Self::SingleSource => "Only a single source supports this insight",
            Self::StaleEvidence => "Evidence is older than recommended threshold",
            Self::SocialMediaOnly => "All sources are social media (low authority)",
            Self::HasGovernmentSource => "Government or official sources included",
            Self::HighSourceDiversity => "Multiple independent source types present",
            Self::IncompleteEvidence => "Some evidence records are incomplete",
            Self::ContradictorySources => "Conflicting information from different sources",
            Self::UnknownSources => "Some sources cannot be verified or are anonymous",
        }
    }
}

/// Evidence item for credibility scoring.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoredEvidence {
    /// Evidence ID
    pub id: Uuid,
    /// Source URL
    pub source_url: Option<String>,
    /// Source domain
    pub source_domain: Option<String>,
    /// Authority tier (0-3)
    pub authority_tier: u8,
    /// Observation timestamp
    pub observed_at: DateTime<Utc>,
    /// Evidence text/content
    pub content: String,
    /// Content completeness score (0.0-1.0)
    pub completeness: f64,
}

impl ScoredEvidence {
    /// Create from domain and timestamp.
    pub fn from_domain(domain: &str, observed_at: DateTime<Utc>) -> Self {
        let authority_tier = match domain.to_lowercase().as_str() {
            d if d.contains(".gov") || d.contains(".mil") || d.contains("government") => 3,
            d if d.contains(".com") || d.contains(".org") || d.contains(".net") => 2,
            d if d.contains("twitter") || d.contains("facebook") || d.contains("reddit") => 1,
            _ => 0,
        };

        Self {
            id: Uuid::new_v4(),
            source_url: None,
            source_domain: Some(domain.to_string()),
            authority_tier,
            observed_at,
            content: String::new(),
            completeness: 0.5,
        }
    }
}

/// Source credibility scorer.
pub struct SourceCredibilityScorer {
    config: CredibilityConfig,
}

impl SourceCredibilityScorer {
    /// Create a new scorer with default configuration.
    pub fn new() -> Self {
        Self {
            config: CredibilityConfig::default(),
        }
    }

    /// Create with custom configuration.
    pub fn with_config(config: CredibilityConfig) -> Self {
        Self { config }
    }

    /// Score a collection of evidence.
    pub fn score(&self, evidence: &[ScoredEvidence]) -> CredibilityScore {
        let mut result = CredibilityScore::new();

        if evidence.is_empty() {
            result.add_flag(CredibilityFlag::SingleSource);
            return result;
        }

        result.source_count = evidence.len();

        // Count unique domains for corroboration
        let mut unique_domains: HashSet<&str> = HashSet::new();
        let mut authority_sum = 0;
        let mut most_recent: Option<DateTime<Utc>> = None;

        for ev in evidence {
            // Authority scoring
            authority_sum += ev.authority_tier as i32;
            if ev.authority_tier > result.highest_authority {
                result.highest_authority = ev.authority_tier;
            }

            // Domain diversity
            if let Some(ref domain) = ev.source_domain {
                unique_domains.insert(domain.as_str());
            }

            // Most recent tracking
            if most_recent.is_none_or(|mr| ev.observed_at > mr) {
                most_recent = Some(ev.observed_at);
            }
        }

        result.most_recent = most_recent;

        // Calculate authority score (0-1 from 0-3 tier)
        result.authority_score = (authority_sum as f64 / (evidence.len() as f64 * 3.0))
            .clamp(0.0, 1.0);

        // Calculate recency score
        let now = Utc::now();
        let oldest = evidence.iter()
            .map(|e| e.observed_at)
            .min()
            .unwrap_or(now);
        let age_days = (now - oldest).num_days() as f64;
        
        if age_days <= 1.0 {
            result.recency_score = 1.0;
        } else if age_days <= 7.0 {
            result.recency_score = 0.85;
        } else if age_days <= 30.0 {
            result.recency_score = 0.65 - (age_days - 7.0) * 0.01;
        } else {
            result.recency_score = 0.3 - ((age_days - 30.0) * 0.005).min(0.2);
        }
        result.recency_score = result.recency_score.clamp(0.0, 1.0);

        // Calculate corroboration score
        let domain_count = unique_domains.len();
        result.corroboration_score = if domain_count >= 3 {
            1.0
        } else if domain_count == 2 {
            0.8
        } else if domain_count == 1 && evidence.len() > 1 {
            0.5 // Same domain but multiple sources
        } else {
            0.2 // Single source
        };

        // Calculate completeness score
        let total_completeness: f64 = evidence.iter()
            .map(|e| e.completeness)
            .sum();
        result.completeness_score = total_completeness / evidence.len() as f64;

        // Calculate overall score
        result.overall = 
            result.authority_score * self.config.authority_weight +
            result.recency_score * self.config.recency_weight +
            result.corroboration_score * self.config.corroboration_weight +
            result.completeness_score * self.config.completeness_weight;

        // Apply flags
        self.apply_flags(evidence, &unique_domains, age_days as u32, &mut result);

        // Adjust overall based on flags (positive weights reduce score, negative weights increase it)
        for flag in &result.flags {
            if flag.severity_weight() > 0.0 {
                result.overall -= flag.severity_weight();
            } else {
                result.overall += flag.severity_weight().abs();
            }
        }
        result.overall = result.overall.clamp(0.0, 1.0);

        result
    }

    /// Apply quality flags.
    fn apply_flags(
        &self,
        evidence: &[ScoredEvidence],
        unique_domains: &HashSet<&str>,
        age_days: u32,
        result: &mut CredibilityScore,
    ) {
        // Single source
        if evidence.len() == 1 {
            result.add_flag(CredibilityFlag::SingleSource);
        }

        // Stale evidence
        if age_days > self.config.max_age_days {
            result.add_flag(CredibilityFlag::StaleEvidence);
        }

        // Social media only
        let all_social = evidence.iter().all(|e| e.authority_tier == 1);
        if all_social && !evidence.is_empty() {
            result.add_flag(CredibilityFlag::SocialMediaOnly);
        }

        // Government source
        if result.highest_authority >= 3 {
            result.add_flag(CredibilityFlag::HasGovernmentSource);
        }

        // High diversity
        if unique_domains.len() >= 3 {
            result.add_flag(CredibilityFlag::HighSourceDiversity);
        }

        // Incomplete evidence
        let has_incomplete = evidence.iter().any(|e| e.completeness < 0.5);
        if has_incomplete {
            result.add_flag(CredibilityFlag::IncompleteEvidence);
        }

        // Unknown sources
        let has_unknown = evidence.iter().any(|e| e.authority_tier == 0);
        if has_unknown {
            result.add_flag(CredibilityFlag::UnknownSources);
        }
    }
}

impl Default for SourceCredibilityScorer {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// 4.2.2 Output Validation
// ============================================================================

/// Configuration for output validation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationConfig {
    /// Minimum factual accuracy score (0.0-1.0)
    pub min_accuracy_score: f64,
    /// Minimum internal consistency score (0.0-1.0)
    pub min_consistency_score: f64,
    /// Minimum plausibility score (0.0-1.0)
    pub min_plausibility_score: f64,
    /// Minimum actionability score (0.0-1.0)
    pub min_actionability_score: f64,
    /// Enable plausibility checks
    pub enable_plausibility_check: bool,
    /// Enable consistency check
    pub enable_consistency_check: bool,
}

impl Default for ValidationConfig {
    fn default() -> Self {
        Self {
            min_accuracy_score: 0.6,
            min_consistency_score: 0.7,
            min_plausibility_score: 0.5,
            min_actionability_score: 0.4,
            enable_plausibility_check: true,
            enable_consistency_check: true,
        }
    }
}

/// Validation result for an insight.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationResult {
    /// Overall validation status
    pub status: ValidationStatus,
    /// Factual accuracy score (0.0-1.0)
    pub accuracy_score: f64,
    /// Internal consistency score (0.0-1.0)
    pub consistency_score: f64,
    /// Plausibility score (0.0-1.0)
    pub plausibility_score: f64,
    /// Actionability score (0.0-1.0)
    pub actionability_score: f64,
    /// Overall quality score (0.0-1.0)
    pub quality_score: f64,
    /// Validation errors
    pub errors: Vec<ValidationError>,
    /// Validation warnings
    pub warnings: Vec<ValidationWarning>,
    /// Validated at timestamp
    pub validated_at: DateTime<Utc>,
}

impl ValidationResult {
    /// Create a new validation result.
    pub fn new() -> Self {
        Self {
            status: ValidationStatus::Unvalidated,
            accuracy_score: 0.0,
            consistency_score: 0.0,
            plausibility_score: 0.0,
            actionability_score: 0.0,
            quality_score: 0.0,
            errors: Vec::new(),
            warnings: Vec::new(),
            validated_at: Utc::now(),
        }
    }

    /// Check if validation passed.
    pub fn passed(&self) -> bool {
        self.status == ValidationStatus::Passed
    }

    /// Add a validation error.
    pub fn add_error(&mut self, error: ValidationError) {
        self.errors.push(error);
        self.status = ValidationStatus::Failed;
    }

    /// Add a warning.
    pub fn add_warning(&mut self, warning: ValidationWarning) {
        self.warnings.push(warning);
    }

    /// Calculate overall quality score.
    pub fn calculate_quality_score(&mut self) {
        self.quality_score =
            self.accuracy_score * 0.30 +
            self.consistency_score * 0.25 +
            self.plausibility_score * 0.25 +
            self.actionability_score * 0.20;

        // Add errors for low scores
        if self.accuracy_score < 0.5 {
            let msg = format!("Factual accuracy score ({:.2}) is below threshold", self.accuracy_score);
            self.errors.push(ValidationError::new("Low accuracy score", &msg));
        }
        if self.consistency_score < 0.5 {
            let msg = format!("Internal consistency score ({:.2}) is below threshold", self.consistency_score);
            self.errors.push(ValidationError::new("Low consistency score", &msg));
        }
        if self.plausibility_score < 0.5 {
            let msg = format!("Plausibility score ({:.2}) is below threshold", self.plausibility_score);
            self.errors.push(ValidationError::new("Low plausibility score", &msg));
        }
        if self.actionability_score < 0.3 {
            let msg = format!("Actionability score ({:.2}) is below threshold", self.actionability_score);
            self.errors.push(ValidationError::new("Low actionability score", &msg));
        }

        // Set status based on quality score and errors
        if !self.errors.is_empty() {
            self.status = ValidationStatus::Failed;
        } else if self.quality_score >= 0.8 {
            self.status = ValidationStatus::Passed;
        } else if self.quality_score >= 0.5 {
            self.status = ValidationStatus::PassedWithWarnings;
        } else {
            self.status = ValidationStatus::Failed;
        }
    }
}

impl Default for ValidationResult {
    fn default() -> Self {
        Self::new()
    }
}

/// Validation status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ValidationStatus {
    Unvalidated,
    Passed,
    PassedWithWarnings,
    Failed,
}

impl ValidationStatus {
    /// Get label.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Unvalidated => "Unvalidated",
            Self::Passed => "Passed",
            Self::PassedWithWarnings => "Passed with warnings",
            Self::Failed => "Failed",
        }
    }
}

/// Validation error types.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationError {
    pub code: String,
    pub message: String,
    pub field: Option<String>,
    pub severity: ErrorSeverity,
}

impl ValidationError {
    pub fn new(code: &str, message: &str) -> Self {
        Self {
            code: code.to_string(),
            message: message.to_string(),
            field: None,
            severity: ErrorSeverity::Error,
        }
    }

    pub fn with_field(mut self, field: &str) -> Self {
        self.field = Some(field.to_string());
        self
    }
}

/// Error severity levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorSeverity {
    Critical,
    Error,
    Warning,
}

impl ErrorSeverity {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Critical => "Critical",
            Self::Error => "Error",
            Self::Warning => "Warning",
        }
    }
}

/// Validation warning types.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationWarning {
    pub code: String,
    pub message: String,
    pub suggestion: Option<String>,
}

impl ValidationWarning {
    pub fn new(code: &str, message: &str) -> Self {
        Self {
            code: code.to_string(),
            message: message.to_string(),
            suggestion: None,
        }
    }

    pub fn with_suggestion(mut self, suggestion: &str) -> Self {
        self.suggestion = Some(suggestion.to_string());
        self
    }
}

/// Content to validate.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ValidationContent {
    /// Insight title
    pub title: String,
    /// Insight narrative/description
    pub narrative: String,
    /// Claimed evidence sources
    pub sources: Vec<String>,
    /// Entities mentioned
    pub entities: Vec<String>,
    /// Key claims made
    pub claims: Vec<String>,
    /// Recommendations if any
    pub recommendations: Vec<String>,
}

/// Output validator.
pub struct OutputValidator {
    config: ValidationConfig,
}

impl OutputValidator {
    /// Create a new validator.
    pub fn new() -> Self {
        Self {
            config: ValidationConfig::default(),
        }
    }

    /// Create with custom configuration.
    pub fn with_config(config: ValidationConfig) -> Self {
        Self { config }
    }

    /// Validate content.
    pub fn validate(&self, content: &ValidationContent) -> ValidationResult {
        let mut result = ValidationResult::new();

        // Factual accuracy check
        result.accuracy_score = self.check_accuracy(content);

        // Internal consistency check
        if self.config.enable_consistency_check {
            result.consistency_score = self.check_consistency(content);
        } else {
            result.consistency_score = 1.0;
        }

        // Plausibility check
        if self.config.enable_plausibility_check {
            result.plausibility_score = self.check_plausibility(content);
        } else {
            result.plausibility_score = 1.0;
        }

        // Actionability check
        result.actionability_score = self.check_actionability(content);

        // Calculate overall quality
        result.calculate_quality_score();

        result
    }

    /// Check factual accuracy against sources.
    fn check_accuracy(&self, content: &ValidationContent) -> f64 {
        let mut score: f64 = 1.0;

        // Check if narrative has content
        if content.narrative.is_empty() {
            return 0.0; // ValidationResult will be marked failed via calculate_quality_score
        }

        // Check if sources are provided
        if content.sources.is_empty() {
            score -= 0.3;
        }

        // Check if claims are grounded in narrative
        for claim in &content.claims {
            let claim_lower = claim.to_lowercase();
            let narrative_lower = content.narrative.to_lowercase();
            
            // Check for key claim terms in narrative
            let words: Vec<&str> = claim_lower.split_whitespace().collect();
            let word_count = words.len();
            let found_count = words.iter()
                .filter(|w| narrative_lower.contains(*w))
                .count();
            
            let coverage = if word_count > 0 {
                found_count as f64 / word_count as f64
            } else {
                1.0
            };

            // If claim terms are missing from narrative, reduce score
            if coverage < 0.5 {
                score -= 0.15;
            }
        }

        // Check for unsupported superlatives
        let unsupported = ["always", "never", "definitely", "certainly", "absolutely"];
        for word in &unsupported {
            if content.narrative.to_lowercase().contains(word) && content.sources.is_empty() {
                score -= 0.1;
            }
        }

        score.clamp(0.0, 1.0)
    }

    /// Check internal consistency.
    fn check_consistency(&self, content: &ValidationContent) -> f64 {
        let mut score: f64 = 1.0;

        // Check for contradictory patterns
        let narrative_lower = content.narrative.to_lowercase();

        // Check title-narrative consistency
        let title_lower = content.title.to_lowercase();
        if !title_lower.is_empty() && !narrative_lower.is_empty() {
            // Extract key terms from title
            let title_terms: Vec<&str> = title_lower
                .split_whitespace()
                .filter(|w| w.len() > 4)
                .collect();
            
            // Check if key terms appear in narrative
            let matched = title_terms.iter()
                .filter(|t| narrative_lower.contains(*t))
                .count();
            
            if title_terms.len() > 2 && matched < title_terms.len() / 2 {
                score -= 0.2;
            }
        }

        // Check for self-contradiction
        let contradiction_pairs = [
            ("increase", "decrease"),
            ("growth", "decline"),
            ("growth", "decrease"),
            ("up", "down"),
            ("expand", "contract"),
            ("positive", "negative"),
            ("high", "low"),
        ];

        let mut has_contradiction = false;
        for (pos, neg) in &contradiction_pairs {
            if narrative_lower.contains(pos) && narrative_lower.contains(neg) {
                // Check they're in different sentences (not just mentioning both)
                let sentences: Vec<&str> = narrative_lower.split('.').collect();
                let pos_sentences = sentences.iter().filter(|s| s.contains(pos)).count();
                let neg_sentences = sentences.iter().filter(|s| s.contains(neg)).count();
                
                if pos_sentences > 0 && neg_sentences > 0 {
                    has_contradiction = true;
                    break;
                }
            }
        }

        if has_contradiction {
            score -= 0.3;
        }

        // Check entity consistency
        for entity in &content.entities {
            // Count mentions in narrative
            let mentions = narrative_lower.matches(&entity.to_lowercase()).count();
            if mentions == 0 {
                score -= 0.05; // Minor penalty for named but not discussed entity
            }
        }

        score.clamp(0.0, 1.0)
    }

    /// Check plausibility of claims.
    fn check_plausibility(&self, content: &ValidationContent) -> f64 {
        let mut score: f64 = 1.0;
        let narrative_lower = content.narrative.to_lowercase();

        // Check for always/never patterns (strong claims without evidence)
        let strong_claims = ["always", "never", "definitely", "certainly", "absolutely", "100% certainty", "no exceptions"];
        for claim in strong_claims {
            if narrative_lower.contains(claim) {
                if content.sources.is_empty() {
                    score -= 0.25;
                } else if content.sources.len() < 2 {
                    score -= 0.15;
                }
            }
        }

        // Check for implausible quantifiers without evidence
        let implausible_patterns = [
            ("100%", 0.8), // If claiming 100% without sources
            ("all", 0.9),  // "all" companies, "all" markets
            ("every", 0.9),
            ("none", 0.9),
        ];

        for (pattern, _threshold) in &implausible_patterns {
            if narrative_lower.contains(pattern) {
                if content.sources.is_empty() {
                    score -= 0.15;
                } else if content.sources.len() < 2 {
                    score -= 0.1;
                }
            }
        }

        // Check for reasonable magnitude claims
        let unreasonable = [
            ("increased by 10000%", 0.1),
            ("decreased by 5000%", 0.1),
        ];

        for (pattern, _) in &unreasonable {
            if narrative_lower.contains(pattern) {
                score -= 0.25;
            }
        }

        // Check for vague time references with specific claims
        if narrative_lower.contains("recently") || narrative_lower.contains("currently") {
            // These are acceptable with context
        } else if narrative_lower.contains("yesterday") && content.sources.is_empty() {
            score -= 0.1;
        }

        score.clamp(0.0, 1.0)
    }

    /// Check actionability of insights.
    fn check_actionability(&self, content: &ValidationContent) -> f64 {
        // Recommendations make it actionable
        if !content.recommendations.is_empty() {
            let rec_count = content.recommendations.len() as f64;
            let rec_score = (rec_count.min(5.0) / 5.0 * 0.5) + 0.5; // 0.5-1.0 based on count
            
            // Check recommendation quality
            let has_action_verbs = content.recommendations.iter()
                .any(|r| {
                    let lower = r.to_lowercase();
                    lower.contains("implement") || lower.contains("review") || 
                    lower.contains("contact") || lower.contains("investigate") ||
                    lower.contains("monitor") || lower.contains("assess")
                });
            
            if has_action_verbs {
                return rec_score.max(0.8);
            }
            return rec_score;
        }

        // If no explicit recommendations, check for implicit actionability
        let mut implicit_score: f64 = 0.3;
        
        // Check for action indicators in narrative
        let narrative_lower = content.narrative.to_lowercase();
        let action_indicators = [
            "should", "recommend", "consider", "requires", "needs",
            "action", "next step", "immediately", "urgent"
        ];

        for indicator in &action_indicators {
            if narrative_lower.contains(indicator) {
                implicit_score += 0.1;
            }
        }

        // Check for risk factors (actionable through mitigation)
        if narrative_lower.contains("risk") || narrative_lower.contains("threat") {
            implicit_score += 0.15;
        }

        implicit_score.clamp(0.0, 1.0)
    }
}

impl Default for OutputValidator {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// 4.2.3 Bias Detection
// ============================================================================

/// Configuration for bias detection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BiasConfig {
    /// Minimum confirmation bias threshold
    pub confirmation_bias_threshold: f64,
    /// Minimum availability bias threshold
    pub availability_bias_threshold: f64,
    /// Enable geographic bias detection
    pub detect_geographic_bias: bool,
    /// Enable interest conflict detection
    pub detect_interest_conflicts: bool,
    /// Geographic focus region (if any)
    pub primary_region: Option<String>,
}

impl Default for BiasConfig {
    fn default() -> Self {
        Self {
            confirmation_bias_threshold: 0.7,
            availability_bias_threshold: 0.6,
            detect_geographic_bias: true,
            detect_interest_conflicts: true,
            primary_region: None,
        }
    }
}

/// Bias detection result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BiasReport {
    /// Overall bias risk score (0.0-1.0, higher = more bias)
    pub risk_score: f64,
    /// Confirmation bias indicators
    pub confirmation_bias: BiasIndicator,
    /// Availability bias indicators
    pub availability_bias: BiasIndicator,
    /// Geographic bias indicators
    pub geographic_bias: Option<BiasIndicator>,
    /// Interest conflict indicators
    pub interest_conflict: Option<BiasIndicator>,
    /// Detected bias types
    pub bias_types: Vec<BiasType>,
    /// Recommendations to mitigate bias
    pub recommendations: Vec<String>,
    /// Analyzed at timestamp
    pub analyzed_at: DateTime<Utc>,
}

impl BiasReport {
    /// Create a new bias report.
    pub fn new() -> Self {
        Self {
            risk_score: 0.0,
            confirmation_bias: BiasIndicator::new(BiasType::ConfirmationBias),
            availability_bias: BiasIndicator::new(BiasType::AvailabilityBias),
            geographic_bias: None,
            interest_conflict: None,
            bias_types: Vec::new(),
            recommendations: Vec::new(),
            analyzed_at: Utc::now(),
        }
    }

    /// Add a bias indicator.
    pub fn add_bias(&mut self, indicator: BiasIndicator) {
        match indicator.bias_type {
            BiasType::ConfirmationBias => self.confirmation_bias = indicator,
            BiasType::AvailabilityBias => self.availability_bias = indicator,
            BiasType::GeographicBias => self.geographic_bias = Some(indicator),
            BiasType::InterestConflict => self.interest_conflict = Some(indicator),
        }
    }

    /// Calculate overall risk score.
    pub fn calculate_risk(&mut self) {
        let mut score = 0.0;
        
        score += self.confirmation_bias.severity * 0.35;
        score += self.availability_bias.severity * 0.25;
        
        if let Some(ref geo) = self.geographic_bias {
            score += geo.severity * 0.20;
        }
        
        if let Some(ref conflict) = self.interest_conflict {
            score += conflict.severity * 0.20;
        }

        self.risk_score = score.clamp(0.0, 1.0);

        // Add recommendations based on detected biases
        self.add_recommendations();
    }

    /// Add mitigation recommendations.
    fn add_recommendations(&mut self) {
        if self.confirmation_bias.severity > 0.5 {
            self.recommendations.push(
                "Seek disconfirming evidence to counter confirmation bias".to_string()
            );
        }

        if self.availability_bias.severity > 0.5 {
            self.recommendations.push(
                "Expand source diversity to counter availability heuristic".to_string()
            );
        }

        if let Some(ref geo) = self.geographic_bias {
            if geo.severity > 0.5 {
                self.recommendations.push(
                    "Include sources from multiple geographic regions".to_string()
                );
            }
        }

        if let Some(ref conflict) = self.interest_conflict {
            if conflict.severity > 0.5 {
                self.recommendations.push(
                    "Review for potential conflicts of interest in source selection".to_string()
                );
            }
        }
    }
}

impl Default for BiasReport {
    fn default() -> Self {
        Self::new()
    }
}

/// Bias indicator with details.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BiasIndicator {
    /// Type of bias
    pub bias_type: BiasType,
    /// Severity score (0.0-1.0)
    pub severity: f64,
    /// Evidence supporting this bias indicator
    pub evidence: Vec<String>,
    /// Specific bias instances detected
    pub instances: Vec<BiasInstance>,
}

impl BiasIndicator {
    /// Create a new bias indicator.
    pub fn new(bias_type: BiasType) -> Self {
        Self {
            bias_type,
            severity: 0.0,
            evidence: Vec::new(),
            instances: Vec::new(),
        }
    }

    /// Add a bias instance.
    pub fn add_instance(&mut self, instance: BiasInstance) {
        self.instances.push(instance);
        // Update severity based on instance count
        self.severity = (self.instances.len() as f64 * 0.2).min(1.0);
    }

    /// Add evidence for this bias.
    pub fn add_evidence(&mut self, evidence: &str) {
        self.evidence.push(evidence.to_string());
    }
}

/// Types of cognitive bias.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BiasType {
    /// Confirmation bias - seeking evidence that confirms existing beliefs
    ConfirmationBias,
    /// Availability heuristic - overweighting recent or memorable information
    AvailabilityBias,
    /// Geographic/cultural bias - favoring certain regions or cultures
    GeographicBias,
    /// Interest conflict - analysis influenced by undisclosed interests
    InterestConflict,
}

impl BiasType {
    /// Get human-readable label.
    pub fn label(&self) -> &'static str {
        match self {
            Self::ConfirmationBias => "Confirmation Bias",
            Self::AvailabilityBias => "Availability Bias",
            Self::GeographicBias => "Geographic/Cultural Bias",
            Self::InterestConflict => "Interest Conflict",
        }
    }

    /// Get description.
    pub fn description(&self) -> &'static str {
        match self {
            Self::ConfirmationBias => "Tendency to search for, interpret, and recall information \
                that confirms pre-existing beliefs or values.",
            Self::AvailabilityBias => "Tendency to overestimate the importance of information \
                that is readily available, especially recent or emotionally vivid events.",
            Self::GeographicBias => "Tendency to favor perspectives from certain geographic \
                regions or cultural backgrounds, overlooking alternatives.",
            Self::InterestConflict => "Analysis potentially influenced by undisclosed \
                financial, professional, or personal interests.",
        }
    }
}

/// Specific bias instance detected.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BiasInstance {
    /// Instance description
    pub description: String,
    /// Where detected (title, narrative, sources)
    pub location: String,
    /// Severity of this instance
    pub severity: f64,
}

impl BiasInstance {
    pub fn new(description: &str, location: &str) -> Self {
        Self {
            description: description.to_string(),
            location: location.to_string(),
            severity: 0.5,
        }
    }
}

/// Content to analyze for bias.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BiasContent {
    /// Insight title
    pub title: String,
    /// Insight narrative
    pub narrative: String,
    /// Source domains/URLs
    pub sources: Vec<String>,
    /// Geographic references in content
    pub geographic_references: Vec<String>,
    /// Entities mentioned
    pub entities: Vec<String>,
    /// Publication date if known
    pub published_date: Option<DateTime<Utc>>,
}

/// Bias detector.
pub struct BiasDetector {
    config: BiasConfig,
}

impl BiasDetector {
    /// Create a new bias detector.
    pub fn new() -> Self {
        Self {
            config: BiasConfig::default(),
        }
    }

    /// Create with custom configuration.
    pub fn with_config(config: BiasConfig) -> Self {
        Self { config }
    }

    /// Analyze content for bias.
    pub fn analyze(&self, content: &BiasContent) -> BiasReport {
        let mut report = BiasReport::new();

        // Detection confirmation bias
        self.detect_confirmation_bias(content, &mut report);

        // Detect availability bias
        self.detect_availability_bias(content, &mut report);

        // Detect geographic bias
        if self.config.detect_geographic_bias {
            self.detect_geographic_bias(content, &mut report);
        }

        // Detect interest conflicts
        if self.config.detect_interest_conflicts {
            self.detect_interest_conflicts(content, &mut report);
        }

        // Calculate overall risk
        report.calculate_risk();

        report
    }

    /// Detect confirmation bias indicators.
    fn detect_confirmation_bias(&self, content: &BiasContent, report: &mut BiasReport) {
        let mut indicator = BiasIndicator::new(BiasType::ConfirmationBias);
        let narrative_lower = content.narrative.to_lowercase();

        // Check for selective evidence patterns
        let exclusive_patterns = [
            ("only", "exclusive language"),
            ("just", "exclusive language"),
            ("merely", "exclusive language"),
            ("nothing but", "exclusive language"),
        ];

        for (pattern, desc) in &exclusive_patterns {
            if narrative_lower.contains(pattern) {
                indicator.add_instance(BiasInstance::new(desc, "narrative"));
            }
        }

        // Check for confirmation-seeking language
        let confirm_patterns = [
            ("proves", "claims to prove rather than suggest"),
            ("confirms", "claims to confirm rather than indicate"),
            ("demonstrates", "strong causal language"),
            ("obviously", "assuming shared viewpoint"),
            ("clearly", "assuming clarity without evidence"),
        ];

        for (pattern, desc) in &confirm_patterns {
            if narrative_lower.contains(pattern) {
                indicator.add_instance(BiasInstance::new(desc, "narrative"));
            }
        }

        // Check source diversity for confirmation signals
        let unique_domains: Vec<String> = content.sources.iter()
            .filter_map(|s| {
                // Extract domain from URL
                s.split('/').nth(2).map(|d| d.to_lowercase())
            })
            .collect();
        let unique_domain_count = unique_domains.iter().collect::<std::collections::HashSet<_>>().len();

        // All sources from same domain or perspective suggests confirmation bias
        if unique_domain_count <= 1 && !content.sources.is_empty() {
            indicator.add_instance(BiasInstance::new(
                "Single source domain - may indicate selective sourcing",
                "sources"
            ));
            indicator.add_evidence(&format!(
                "All {} sources from same domain(s): {:?}",
                content.sources.len(),
                unique_domains
            ));
        }

        if !indicator.instances.is_empty() {
            report.add_bias(indicator);
        }
    }

    /// Detect availability bias indicators.
    fn detect_availability_bias(&self, content: &BiasContent, report: &mut BiasReport) {
        let mut indicator = BiasIndicator::new(BiasType::AvailabilityBias);
        let narrative_lower = content.narrative.to_lowercase();

        // Check for recency bias in language
        let recent_patterns = [
            ("recently", "recent temporality without date"),
            ("just announced", "just temporality without date"),
            ("just discovered", "just temporality without date"),
            ("breaking", "breaking news language"),
            ("developing", "developing situation language"),
        ];

        for (pattern, desc) in &recent_patterns {
            if narrative_lower.contains(pattern) {
                indicator.add_instance(BiasInstance::new(desc, "narrative"));
            }
        }

        // Check for emotional language (availability through emotion)
        let emotional_patterns = [
            ("shocking", "emotionally charged language"),
            ("devastating", "emotionally charged language"),
            ("stunning", "emotionally charged language"),
            ("catastrophic", "emotionally charged language"),
            ("terrifying", "emotionally charged language"),
        ];

        for (pattern, desc) in &emotional_patterns {
            if narrative_lower.contains(pattern) {
                indicator.add_instance(BiasInstance::new(desc, "narrative"));
            }
        }

        // Check if published date is very recent (potential recency bias)
        if let Some(ref date) = content.published_date {
            let age_hours = (Utc::now() - *date).num_hours();
            if age_hours < 24 {
                indicator.add_instance(BiasInstance::new(
                    "Analysis based on very recent data (<24h)",
                    "published_date"
                ));
            }
        }

        if !indicator.instances.is_empty() {
            report.add_bias(indicator);
        }
    }

    /// Detect geographic bias.
    fn detect_geographic_bias(&self, content: &BiasContent, report: &mut BiasReport) {
        let mut indicator = BiasIndicator::new(BiasType::GeographicBias);

        // Check geographic diversity in content
        let region_count = content.geographic_references.len();

        if region_count <= 1 && !content.geographic_references.is_empty() {
            indicator.add_instance(BiasInstance::new(
                "Single geographic region referenced",
                "narrative"
            ));
        }

        // Check source geographic distribution
        let source_domains: Vec<&str> = content.sources.iter()
            .filter_map(|s| s.split('/').nth(2))
            .collect();

        // Check if all sources are from similar TLDs
        let tlds: HashSet<&str> = source_domains.iter()
            .filter_map(|d| d.rsplit('.').next())
            .collect();

        if tlds.len() <= 1 && !source_domains.is_empty() {
            indicator.add_instance(BiasInstance::new(
                "Sources concentrated in single geographic TLD",
                "sources"
            ));
            indicator.add_evidence(&format!(
                "Source TLDs: {:?}",
                tlds
            ));
        }

        // Check for primary region bias if configured
        if let Some(ref primary) = self.config.primary_region {
            let primary_refs = content.geographic_references.iter()
                .filter(|r| r.to_lowercase().contains(&primary.to_lowercase()))
                .count();
            
            if primary_refs > content.geographic_references.len() / 2 && region_count > 2 {
                indicator.add_instance(BiasInstance::new(
                    "Heavy focus on primary region despite multiple regions",
                    "narrative"
                ));
            }
        }

        if !indicator.instances.is_empty() {
            report.add_bias(indicator);
        }
    }

    /// Detect interest conflicts.
    fn detect_interest_conflicts(&self, content: &BiasContent, report: &mut BiasReport) {
        let mut indicator = BiasIndicator::new(BiasType::InterestConflict);
        let narrative_lower = content.narrative.to_lowercase();

        // Check for financial interest indicators
        let financial_patterns = [
            ("analyst estimates", "potentially biased analyst viewpoint"),
            ("target price", "financial target language"),
            ("buy rating", "investment recommendation language"),
            ("sell rating", "investment recommendation language"),
            ("outperform", "investment recommendation language"),
            ("underperform", "investment recommendation language"),
        ];

        for (pattern, desc) in &financial_patterns {
            if narrative_lower.contains(pattern) {
                indicator.add_instance(BiasInstance::new(desc, "narrative"));
            }
        }

        // Check for promotional language
        let promo_patterns = [
            ("leading", "promotional language"),
            ("innovative", "promotional language"),
            ("game-changing", "promotional language"),
            ("revolutionary", "promotional language"),
            ("best-in-class", "promotional language"),
        ];

        for (pattern, desc) in &promo_patterns {
            if narrative_lower.contains(pattern) {
                indicator.add_instance(BiasInstance::new(desc, "narrative"));
            }
        }

        // Check source attribution
        let self_promo_sources = ["company.com", "corp.com", "inc.com"];
        for source in &content.sources {
            for promo in &self_promo_sources {
                if source.to_lowercase().contains(promo) {
                    indicator.add_instance(BiasInstance::new(
                        "Source may be entity's own communications",
                        "sources"
                    ));
                    indicator.add_evidence(source);
                }
            }
        }

        if !indicator.instances.is_empty() {
            report.add_bias(indicator);
        }
    }
}

impl Default for BiasDetector {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// 4.2.4 Continuous Improvement
// ============================================================================

/// Configuration for continuous improvement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImprovementConfig {
    /// Minimum feedback count before adjusting
    pub min_feedback_count: u32,
    /// Learning rate for pattern adjustment
    pub learning_rate: f64,
    /// Enable automated recipe refinement
    pub enable_auto_refinement: bool,
    /// Human-in-the-loop threshold
    pub human_review_threshold: f64,
}

impl Default for ImprovementConfig {
    fn default() -> Self {
        Self {
            min_feedback_count: 5,
            learning_rate: 0.1,
            enable_auto_refinement: true,
            human_review_threshold: 0.7, // Review if quality score < 0.7
        }
    }
}

/// Feedback data for improvement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualityFeedback {
    /// Insight ID
    pub insight_id: Uuid,
    /// Feedback type
    pub feedback_type: QualityFeedbackType,
    /// User rating (0.0-1.0)
    pub rating: Option<f64>,
    /// User comments
    pub comments: Option<String>,
    /// Timestamp
    pub timestamp: DateTime<Utc>,
    /// User ID (optional)
    pub user_id: Option<String>,
}

impl QualityFeedback {
    pub fn new(insight_id: Uuid, feedback_type: QualityFeedbackType) -> Self {
        Self {
            insight_id,
            feedback_type,
            rating: None,
            comments: None,
            timestamp: Utc::now(),
            user_id: None,
        }
    }

    pub fn with_rating(mut self, rating: f64) -> Self {
        self.rating = Some(rating.clamp(0.0, 1.0));
        self
    }

    pub fn with_comments(mut self, comments: &str) -> Self {
        self.comments = Some(comments.to_string());
        self
    }
}

/// Quality feedback types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum QualityFeedbackType {
    /// User found insight useful
    Useful,
    /// User found insight not useful
    NotUseful,
    /// User bookmarked insight
    Bookmarked,
    /// User acted on insight
    ActedUpon,
    /// User dismissed insight
    Dismissed,
    /// False positive reported
    FalsePositive,
    /// False negative reported
    FalseNegative,
    /// User provided explicit rating
    Rated,
}

impl QualityFeedbackType {
    /// Convert to numeric score (1.0 = positive, 0.0 = negative).
    pub fn to_score(&self) -> f64 {
        match self {
            Self::Useful | Self::Bookmarked | Self::ActedUpon => 1.0,
            Self::Rated => 0.7, // Will be overridden by actual rating
            Self::NotUseful | Self::Dismissed | Self::FalsePositive => 0.0,
            Self::FalseNegative => 0.5, // Neutral
        }
    }
}

/// Pattern-of-life analysis result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternOfLifeAnalysis {
    /// Analysis period
    pub period_days: u32,
    /// Total insights generated
    pub total_insights: u32,
    /// Average quality score
    pub avg_quality_score: f64,
    /// Quality trend (positive = improving)
    pub quality_trend: f64,
    /// Top performing categories
    pub top_categories: Vec<(String, f64)>,
    /// Bottom performing categories
    pub bottom_categories: Vec<(String, f64)>,
    /// Recommended threshold adjustments
    pub threshold_adjustments: Vec<ThresholdAdjustment>,
    /// Generated at timestamp
    pub generated_at: DateTime<Utc>,
}

impl PatternOfLifeAnalysis {
    /// Create a new analysis.
    pub fn new(period_days: u32) -> Self {
        Self {
            period_days,
            total_insights: 0,
            avg_quality_score: 0.0,
            quality_trend: 0.0,
            top_categories: Vec::new(),
            bottom_categories: Vec::new(),
            threshold_adjustments: Vec::new(),
            generated_at: Utc::now(),
        }
    }
}

/// Recommended threshold adjustment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThresholdAdjustment {
    /// Category or recipe code
    pub target: String,
    /// Current threshold
    pub current: f64,
    /// Recommended threshold
    pub recommended: f64,
    /// Reason for adjustment
    pub reason: String,
}

/// Recipe refinement suggestion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecipeRefinement {
    /// Recipe code to refine
    pub recipe_code: String,
    /// Suggested changes
    pub changes: Vec<String>,
    /// Expected improvement (0.0-1.0)
    pub expected_improvement: f64,
    /// Requires human review
    pub needs_human_review: bool,
    /// Generated at timestamp
    pub generated_at: DateTime<Utc>,
}

impl RecipeRefinement {
    /// Create a new refinement suggestion.
    pub fn new(recipe_code: &str) -> Self {
        Self {
            recipe_code: recipe_code.to_string(),
            changes: Vec::new(),
            expected_improvement: 0.0,
            needs_human_review: false,
            generated_at: Utc::now(),
        }
    }

    /// Add a suggested change.
    pub fn add_change(&mut self, change: &str) {
        self.changes.push(change.to_string());
    }
}

/// Continuous improvement engine.
pub struct QualityImprovementEngine {
    config: ImprovementConfig,
    feedback_history: Vec<QualityFeedback>,
    quality_history: Vec<f64>,
    category_performance: HashMap<String, Vec<f64>>,
}

impl QualityImprovementEngine {
    /// Create a new engine.
    pub fn new() -> Self {
        Self {
            config: ImprovementConfig::default(),
            feedback_history: Vec::new(),
            quality_history: Vec::new(),
            category_performance: HashMap::new(),
        }
    }

    /// Create with custom configuration.
    pub fn with_config(config: ImprovementConfig) -> Self {
        Self {
            config,
            feedback_history: Vec::new(),
            quality_history: Vec::new(),
            category_performance: HashMap::new(),
        }
    }

    /// Record feedback.
    pub fn record_feedback(&mut self, feedback: QualityFeedback) {
        self.feedback_history.push(feedback.clone());

        // Update quality history if rating provided
        if let Some(rating) = feedback.rating {
            self.quality_history.push(rating);
        }

        // Keep history bounded
        if self.feedback_history.len() > 1000 {
            self.feedback_history.remove(0);
        }
        if self.quality_history.len() > 1000 {
            self.quality_history.remove(0);
        }
    }

    /// Record quality for a category.
    pub fn record_category_quality(&mut self, category: &str, quality: f64) {
        let scores = self.category_performance.entry(category.to_string())
            .or_default();
        scores.push(quality);

        // Keep bounded
        if scores.len() > 100 {
            scores.remove(0);
        }
    }

    /// Analyze pattern-of-life for quality.
    pub fn analyze_pattern_of_life(&self, period_days: u32) -> PatternOfLifeAnalysis {
        let mut analysis = PatternOfLifeAnalysis::new(period_days);

        // Calculate average quality
        if !self.quality_history.is_empty() {
            let sum: f64 = self.quality_history.iter().sum();
            analysis.total_insights = self.quality_history.len() as u32;
            analysis.avg_quality_score = sum / self.quality_history.len() as f64;

            // Calculate trend
            if self.quality_history.len() >= 10 {
                let recent: Vec<f64> = self.quality_history.iter().rev().take(5).cloned().collect();
                let older: Vec<f64> = self.quality_history.iter()
                    .rev()
                    .skip(5)
                    .take(5)
                    .cloned()
                    .collect();

                let recent_avg = recent.iter().sum::<f64>() / recent.len() as f64;
                let older_avg = if older.is_empty() {
                    recent_avg
                } else {
                    older.iter().sum::<f64>() / older.len() as f64
                };

                analysis.quality_trend = recent_avg - older_avg;
            }
        }

        // Analyze category performance
        let mut category_avgs: Vec<(String, f64)> = Vec::new();
        for (cat, scores) in &self.category_performance {
            if !scores.is_empty() {
                let avg: f64 = scores.iter().sum::<f64>() / scores.len() as f64;
                category_avgs.push((cat.clone(), avg));
            }
        }

        // Sort for top and bottom
        category_avgs.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        
        analysis.top_categories = category_avgs.iter().take(3).cloned().collect();
        analysis.bottom_categories = category_avgs.iter().rev().take(3).cloned().collect();

        analysis
    }

    /// Generate automated recipe refinements.
    pub fn generate_recipe_refinements(&self) -> Vec<RecipeRefinement> {
        let mut refinements = Vec::new();

        // Analyze category performance for refinement opportunities
        for (cat, scores) in &self.category_performance {
            if scores.len() >= self.config.min_feedback_count as usize {
                let avg: f64 = scores.iter().sum::<f64>() / scores.len() as f64;

                // Low performance - suggest refinement
                if avg < self.config.human_review_threshold {
                    let mut refinement = RecipeRefinement::new(cat);
                    
                    if avg < 0.4 {
                        refinement.expected_improvement = 0.2;
                        refinement.needs_human_review = true;
                        refinement.add_change("Review evidence requirements for this category");
                        refinement.add_change("Consider increasing minimum confidence threshold");
                    } else {
                        refinement.expected_improvement = 0.1;
                        refinement.needs_human_review = false;
                        refinement.add_change("Review source diversity requirements");
                        refinement.add_change("Consider adjusting freshness decay parameters");
                    }

                    refinements.push(refinement);
                }
            }
        }

        // Sort by expected improvement (highest first)
        refinements.sort_by(|a, b| {
            b.expected_improvement.partial_cmp(&a.expected_improvement)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        refinements
    }

    /// Calculate threshold adjustments based on feedback.
    pub fn calculate_threshold_adjustments(&self) -> Vec<ThresholdAdjustment> {
        let mut adjustments = Vec::new();

        // Analyze false positive rates by category
        let mut category_fp_rates: HashMap<String, (u32, u32)> = HashMap::new(); // (FP, total)

        for feedback in &self.feedback_history {
            let entry = category_fp_rates.entry(feedback.insight_id.to_string()).or_insert((0, 0));
            entry.1 += 1;
            
            if feedback.feedback_type == QualityFeedbackType::FalsePositive {
                entry.0 += 1;
            }
        }

        // Generate adjustments based on FP rates
        for (target, (fp, total)) in category_fp_rates {
            if total >= self.config.min_feedback_count {
                let fp_rate = fp as f64 / total as f64;

                if fp_rate > 0.3 {
                    // High FP rate - suggest increasing threshold
                    adjustments.push(ThresholdAdjustment {
                        target: target.clone(),
                        current: 0.1, // Assumed current
                        recommended: (0.1 + fp_rate * 0.1).min(0.5),
                        reason: format!(
                            "False positive rate {:.0}% exceeds threshold. Recommend increasing confidence threshold.",
                            fp_rate * 100.0
                        ),
                    });
                } else if fp_rate < 0.1 && total > 20 {
                    // Very low FP rate - may be able to lower threshold
                    adjustments.push(ThresholdAdjustment {
                        target: target.clone(),
                        current: 0.1,
                        recommended: 0.05,
                        reason: "Very low false positive rate suggests threshold may be too high.".to_string(),
                    });
                }
            }
        }

        adjustments
    }

    /// Check if human review is needed.
    pub fn needs_human_review(&self, quality_score: f64) -> bool {
        quality_score < self.config.human_review_threshold
            || self.feedback_history.iter()
                .filter(|f| f.feedback_type == QualityFeedbackType::FalsePositive)
                .count() >= 3
    }

    /// Get feedback statistics.
    pub fn get_stats(&self) -> QualityStats {
        let total = self.feedback_history.len() as u32;
        let positive = self.feedback_history.iter()
            .filter(|f| matches!(f.feedback_type, 
                QualityFeedbackType::Useful | QualityFeedbackType::Bookmarked | QualityFeedbackType::ActedUpon))
            .count() as f64;
        let negative = self.feedback_history.iter()
            .filter(|f| matches!(f.feedback_type,
                QualityFeedbackType::NotUseful | QualityFeedbackType::FalsePositive | QualityFeedbackType::Dismissed))
            .count() as f64;

        QualityStats {
            total_feedback: total,
            positive_count: positive as u32,
            negative_count: negative as u32,
            positive_rate: if total > 0 { positive / total as f64 } else { 0.5 },
            avg_rating: if self.quality_history.is_empty() {
                0.0
            } else {
                self.quality_history.iter().sum::<f64>() / self.quality_history.len() as f64
            },
            category_count: self.category_performance.len() as u32,
        }
    }
}

impl Default for QualityImprovementEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// Quality statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualityStats {
    pub total_feedback: u32,
    pub positive_count: u32,
    pub negative_count: u32,
    pub positive_rate: f64,
    pub avg_rating: f64,
    pub category_count: u32,
}

// ============================================================================
// Tests
// ============================================================================

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

    // ─── Source Credibility Scorer Tests ───────────────────────────────────

    #[test]
    fn test_credibility_scorer_high_authority() {
        let scorer = SourceCredibilityScorer::new();
        
        let evidence = vec![
            ScoredEvidence::from_domain("sec.gov", Utc::now()),
            ScoredEvidence::from_domain("treasury.gov", Utc::now()),
            ScoredEvidence::from_domain("reuters.com", Utc::now()),
        ];

        let score = scorer.score(&evidence);
        
        assert!(score.overall > 0.7, "High authority sources should score well");
        assert!(score.highest_authority >= 3, "Should detect government source");
        assert!(score.authority_score > 0.8, "Authority score should be high");
    }

    #[test]
    fn test_credibility_scorer_single_source() {
        let scorer = SourceCredibilityScorer::new();
        
        let evidence = vec![
            ScoredEvidence::from_domain("twitter.com", Utc::now()),
        ];

        let score = scorer.score(&evidence);
        
        assert!(score.flags.contains(&CredibilityFlag::SingleSource));
        assert!(score.overall < 0.5, "Single social source should score low");
    }

    #[test]
    fn test_credibility_scorer_stale_evidence() {
        let scorer = SourceCredibilityScorer::new();
        
        let old_date = Utc::now() - chrono::Duration::days(60);
        let evidence = vec![
            ScoredEvidence::from_domain("reuters.com", old_date),
            ScoredEvidence::from_domain("bloomberg.com", old_date),
        ];

        let score = scorer.score(&evidence);
        
        assert!(score.flags.contains(&CredibilityFlag::StaleEvidence));
        assert!(score.recency_score < 0.4, "Stale evidence should have low recency score");
    }

    // ─── Output Validator Tests ─────────────────────────────────────────────

    #[test]
    fn test_validator_passes_clean_content() {
        let validator = OutputValidator::new();
        
        let content = ValidationContent {
            title: "NVIDIA Supply Chain Update".to_string(),
            narrative: "Analysis of recent supply chain developments indicates potential disruption. Multiple sources confirm the trend.".to_string(),
            sources: vec![
                "https://reuters.com/article".to_string(),
                "https://bloomberg.com/article".to_string(),
            ],
            entities: vec!["NVIDIA".to_string()],
            claims: vec!["Supply chain disruption".to_string()],
            recommendations: vec![
                "Monitor supplier status".to_string(),
                "Review alternative sources".to_string(),
            ],
        };

        let result = validator.validate(&content);
        
        assert!(result.accuracy_score > 0.6);
        assert!(result.consistency_score > 0.7);
        assert!(result.passed() || result.status == ValidationStatus::PassedWithWarnings);
    }

    #[test]
    fn test_validator_fails_empty_narrative() {
        let validator = OutputValidator::new();
        
        let content = ValidationContent {
            title: "Test".to_string(),
            narrative: String::new(),
            sources: Vec::new(),
            entities: Vec::new(),
            claims: Vec::new(),
            recommendations: Vec::new(),
        };

        let result = validator.validate(&content);
        
        assert_eq!(result.accuracy_score, 0.0);
        assert!(!result.errors.is_empty());
    }

    #[test]
    fn test_validator_detects_contradiction() {
        let validator = OutputValidator::new();
        
        let content = ValidationContent {
            title: "Market Analysis".to_string(),
            narrative: "The company shows strong growth in Q3. However, revenue has decreased significantly.".to_string(),
            sources: vec!["https://example.com".to_string()],
            entities: vec!["Company".to_string()],
            claims: vec!["Growth and decline".to_string()],
            recommendations: Vec::new(),
        };

        let result = validator.validate(&content);
        
        assert!(result.consistency_score < 0.8, "Should detect contradiction");
    }

    #[test]
    fn test_validator_detects_implausible_claims() {
        let validator = OutputValidator::new();
        
        let content = ValidationContent {
            title: "Prediction".to_string(),
            narrative: "This always happens with 100% certainty and no exceptions.".to_string(),
            sources: Vec::new(),
            entities: vec![],
            claims: vec!["Always happens with certainty".to_string()],
            recommendations: Vec::new(),
        };

        let result = validator.validate(&content);
        
        assert!(result.plausibility_score < 0.8, "Should detect implausibility");
    }

    #[test]
    fn test_validator_actionability() {
        let validator = OutputValidator::new();
        
        // With recommendations
        let with_rec = ValidationContent {
            recommendations: vec![
                "Implement security measures".to_string(),
                "Review access controls".to_string(),
            ],
            ..Default::default()
        };
        let result_with = validator.validate(&with_rec);
        assert!(result_with.actionability_score > 0.5);

        // Without recommendations
        let without_rec = ValidationContent {
            narrative: "General analysis without specific actions.".to_string(),
            ..Default::default()
        };
        let result_without = validator.validate(&without_rec);
        assert!(result_without.actionability_score < result_with.actionability_score);
    }

    // ─── Bias Detector Tests ─────────────────────────────────────────────────

    #[test]
    fn test_bias_detector_confirmation_bias() {
        let detector = BiasDetector::new();
        
        let content = BiasContent {
            title: "Company Analysis".to_string(),
            narrative: "The data clearly proves our theory. This obviously confirms our hypothesis. The facts demonstrate that we were right all along.".to_string(),
            sources: vec!["https://onlyone.com/article".to_string()],
            geographic_references: vec!["USA".to_string()],
            entities: vec!["Company".to_string()],
            published_date: Some(Utc::now()),
        };

        let report = detector.analyze(&content);
        
        assert!(report.confirmation_bias.severity > 0.2, "Should detect confirmation bias");
        assert!(report.risk_score > 0.0);
    }

    #[test]
    fn test_bias_detector_availability_bias() {
        let detector = BiasDetector::new();
        
        let content = BiasContent {
            title: "Breaking News".to_string(),
            narrative: "Recently, shocking developments have just been announced. This is a devastating and terrifying situation.".to_string(),
            sources: Vec::new(),
            geographic_references: vec![],
            entities: vec![],
            published_date: Some(Utc::now() - chrono::Duration::hours(2)),
        };

        let report = detector.analyze(&content);
        
        assert!(report.availability_bias.severity > 0.0, "Should detect availability bias");
    }

    #[test]
    fn test_bias_detector_geographic_bias() {
        let detector = BiasDetector::new();
        
        let content = BiasContent {
            title: "Analysis".to_string(),
            narrative: "Analysis focused on US markets and US regulations.".to_string(),
            sources: vec![
                "https://nytimes.com".to_string(),
                "https://cnn.com".to_string(),
                "https://wsj.com".to_string(),
            ],
            geographic_references: vec!["USA".to_string()],
            entities: vec![],
            published_date: None,
        };

        let report = detector.analyze(&content);
        
        if let Some(ref geo) = report.geographic_bias {
            assert!(geo.severity > 0.0, "Should detect geographic bias");
        }
    }

    #[test]
    fn test_bias_detector_interest_conflict() {
        let detector = BiasDetector::new();
        
        let content = BiasContent {
            title: "Stock Analysis".to_string(),
            narrative: "Analyst estimates target price with buy rating. This leading innovative company has revolutionary technology.".to_string(),
            sources: vec!["https://company.com".to_string()],
            geographic_references: vec![],
            entities: vec!["Company".to_string()],
            published_date: None,
        };

        let report = detector.analyze(&content);
        
        if let Some(ref conflict) = report.interest_conflict {
            assert!(conflict.severity > 0.0, "Should detect interest conflict");
        }
    }

    // ─── Quality Improvement Engine Tests ────────────────────────────────────

    #[test]
    fn test_improvement_engine_feedback_recording() {
        let mut engine = QualityImprovementEngine::new();
        
        engine.record_feedback(
            QualityFeedback::new(Uuid::new_v4(), QualityFeedbackType::Useful)
                .with_rating(0.9)
        );
        
        engine.record_feedback(
            QualityFeedback::new(Uuid::new_v4(), QualityFeedbackType::NotUseful)
                .with_rating(0.3)
        );

        let stats = engine.get_stats();
        
        assert_eq!(stats.total_feedback, 2);
        assert_eq!(stats.positive_count, 1);
        assert_eq!(stats.negative_count, 1);
    }

    #[test]
    fn test_improvement_engine_category_tracking() {
        let mut engine = QualityImprovementEngine::new();
        
        engine.record_category_quality("supply_chain", 0.8);
        engine.record_category_quality("supply_chain", 0.7);
        engine.record_category_quality("supply_chain", 0.6);
        engine.record_category_quality("demand", 0.9);

        let analysis = engine.analyze_pattern_of_life(30);
        
        assert!(analysis.top_categories.iter().any(|(c, _)| c == "demand"));
    }

    #[test]
    fn test_improvement_engine_recipe_refinement() {
        let mut engine = QualityImprovementEngine::new();
        
        // Add low-performing category
        for _ in 0..10 {
            engine.record_category_quality("low_perf", 0.3);
        }
        
        // Add high-performing category
        for _ in 0..5 {
            engine.record_category_quality("high_perf", 0.9);
        }

        let refinements = engine.generate_recipe_refinements();
        
        assert!(!refinements.is_empty());
        let low_refinement = refinements.iter()
            .find(|r| r.recipe_code == "low_perf");
        assert!(low_refinement.is_some());
        assert!(low_refinement.unwrap().needs_human_review);
    }

    #[test]
    fn test_improvement_engine_threshold_adjustments() {
        let mut engine = QualityImprovementEngine::new();

        // Use the same insight_id to accumulate feedback for one target
        let insight_id = Uuid::new_v4();

        // Add false positive feedback for the same insight
        for _ in 0..4 {
            engine.record_feedback(
                QualityFeedback::new(insight_id, QualityFeedbackType::FalsePositive)
            );
        }

        // Add positive feedback for the same insight
        for _ in 0..2 {
            engine.record_feedback(
                QualityFeedback::new(insight_id, QualityFeedbackType::Useful)
            );
        }

        // Total: 6 feedback, 4 FP = 66.7% FP rate (>30%)

        let adjustments = engine.calculate_threshold_adjustments();

        // With >30% FP rate (4/6 = 66.7%), should recommend threshold increase
        assert!(!adjustments.is_empty(), "Should have threshold adjustments with >30% FP rate");
    }
        
    #[test]
    fn test_improvement_engine_needs_human_review() {
        let engine = QualityImprovementEngine::new();
        
        // Low quality score needs review
        assert!(engine.needs_human_review(0.3));
        
        // High quality score doesn't need review
        assert!(!engine.needs_human_review(0.8));
    }

    // ─── Validation Result Tests ─────────────────────────────────────────────

    #[test]
    fn test_validation_result_calculation() {
        let mut result = ValidationResult::new();
        result.accuracy_score = 0.95;
        result.consistency_score = 0.9;
        result.plausibility_score = 0.9;
        result.actionability_score = 0.95;

        result.calculate_quality_score();

        // All scores high, quality > 0.9, no errors
        assert!(result.quality_score > 0.9);
        assert_eq!(result.status, ValidationStatus::Passed);
    }

    #[test]
    fn test_validation_result_with_errors() {
        let mut result = ValidationResult::new();
        result.accuracy_score = 0.2; // Very low - will auto-add error
        result.consistency_score = 0.2;
        result.plausibility_score = 0.2;
        result.actionability_score = 0.2;

        result.calculate_quality_score();

        assert_eq!(result.status, ValidationStatus::Failed);
        assert!(result.errors.len() >= 3); // Multiple auto-added errors
    }

    // ─── Bias Report Tests ───────────────────────────────────────────────────

    #[test]
    fn test_bias_report_risk_calculation() {
        let mut report = BiasReport::new();
        
        report.confirmation_bias.severity = 0.6;
        report.availability_bias.severity = 0.4;
        
        let geo_indicator = BiasIndicator::new(BiasType::GeographicBias);
        report.add_bias(geo_indicator);
        
        report.calculate_risk();
        
        // 0.6*0.35 + 0.4*0.25 + 0*0.20 + 0*0.20 = 0.21 + 0.1 = 0.31
        assert!(report.risk_score > 0.3);
        assert!(!report.recommendations.is_empty());
    }

    // ─── Credibility Score Tests ─────────────────────────────────────────────

    #[test]
    fn test_credibility_score_quality_level() {
        let mut score = CredibilityScore::new();
        
        score.overall = 0.9;
        assert_eq!(score.quality_level(), "Excellent");
        
        score.overall = 0.7;
        assert_eq!(score.quality_level(), "Good");
        
        score.overall = 0.5;
        assert_eq!(score.quality_level(), "Fair");
        
        score.overall = 0.3;
        assert_eq!(score.quality_level(), "Poor");
    }

    #[test]
    fn test_credibility_flags_severity() {
        assert!(CredibilityFlag::SocialMediaOnly.severity_weight() > 0.0);
        assert!(CredibilityFlag::HasGovernmentSource.severity_weight() < 0.0);
        assert!(CredibilityFlag::StaleEvidence.severity_weight() > 0.0);
    }
}
