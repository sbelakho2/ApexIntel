//! Deep Insight Generation Module
//!
//! Phase 4.1: Transforms templates into investigative narratives with detailed
//! evidence compilation, risk factor analysis, and actionable recommendations.
//!
//! # Key Capabilities
//! - Narrative transformation from templates
//! - Evidence chain compilation and confidence scoring
//! - Risk factor identification and scoring
//! - Actionable recommendation generation
//! - Contextual intelligence presentation

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

// ============================================================================
// Narrative Transformation
// ============================================================================

/// Configuration for deep insight generation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeepInsightConfig {
    /// Minimum evidence count for high confidence insights
    pub min_evidence_count: usize,
    /// Minimum corroboration sources required
    pub min_corroboration_sources: usize,
    /// Maximum narrative length in characters
    pub max_narrative_length: usize,
    /// Include risk factors in output
    pub include_risk_factors: bool,
    /// Include action recommendations
    pub include_recommendations: bool,
    /// Confidence threshold for actionable insights
    pub action_confidence_threshold: f64,
}

impl Default for DeepInsightConfig {
    fn default() -> Self {
        Self {
            min_evidence_count: 2,
            min_corroboration_sources: 1,
            max_narrative_length: 5000,
            include_risk_factors: true,
            include_recommendations: true,
            action_confidence_threshold: 0.6,
        }
    }
}

/// Evidence item with full provenance tracking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeepEvidence {
    /// Unique evidence ID
    pub id: Uuid,
    /// Evidence description
    pub description: String,
    /// Source URL or identifier
    pub source: Option<String>,
    /// Source domain for categorization
    pub source_domain: Option<String>,
    /// Source authority tier (government=3, commercial=2, social=1)
    pub authority_tier: AuthorityTier,
    /// When this evidence was observed
    pub observed_at: DateTime<Utc>,
    /// Evidence freshness tier
    pub freshness_tier: FreshnessTier,
    /// Confidence in this evidence (0.0-1.0)
    pub confidence: f64,
    /// Whether this corroborates other evidence
    pub corroborates: Vec<Uuid>,
    /// Entity this evidence pertains to
    pub entity: Option<String>,
}

impl DeepEvidence {
    /// Create a new deep evidence item.
    pub fn new(description: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            description: description.into(),
            source: None,
            source_domain: None,
            authority_tier: AuthorityTier::Unknown,
            freshness_tier: FreshnessTier::Unknown,
            confidence: 0.5,
            observed_at: Utc::now(),
            corroborates: Vec::new(),
            entity: None,
        }
    }

    /// Set the source URL.
    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    /// Set the source domain.
    pub fn with_domain(mut self, domain: impl Into<String>) -> Self {
        self.source_domain = Some(domain.into());
        self
    }

    /// Set the authority tier.
    pub fn with_authority(mut self, tier: AuthorityTier) -> Self {
        self.authority_tier = tier;
        self
    }

    /// Set the observation time.
    pub fn with_observed_at(mut self, time: DateTime<Utc>) -> Self {
        self.observed_at = time;
        self.freshness_tier = FreshnessTier::from_datetime(time, Utc::now());
        self
    }

    /// Set the confidence.
    pub fn with_confidence(mut self, confidence: f64) -> Self {
        self.confidence = confidence.clamp(0.0, 1.0);
        self
    }

    /// Add corroborating evidence ID.
    pub fn add_corroboration(&mut self, evidence_id: Uuid) {
        self.corroborates.push(evidence_id);
    }

    /// Set the entity.
    pub fn with_entity(mut self, entity: impl Into<String>) -> Self {
        self.entity = Some(entity.into());
        self
    }
}

/// Source authority tier classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum AuthorityTier {
    /// Government or official sources (highest authority)
    Government = 3,
    /// Commercial or professional sources
    Commercial = 2,
    /// Social media or user-generated content (lowest authority)
    Social = 1,
    /// Unknown or unclassified source
    Unknown = 0,
}

impl AuthorityTier {
    /// Determine tier from source domain patterns.
    pub fn from_domain(domain: &str) -> Self {
        let lower = domain.to_lowercase();
        
        // Government domains
        let gov_patterns = [
            ".gov", ".mil", ".gov.", "government", 
            "federal", "court", " senate.", " congress",
        ];
        for pattern in &gov_patterns {
            if lower.contains(pattern) {
                return Self::Government;
            }
        }

        // Social media patterns
        let social_patterns = [
            "twitter", "x.com", "facebook", "instagram",
            "tiktok", "reddit", "youtube", "telegram",
            "discord", "mastodon", "threads",
        ];
        for pattern in &social_patterns {
            if lower.contains(pattern) {
                return Self::Social;
            }
        }

        // Commercial and professional domains
        let commercial_patterns = [
            ".com", ".org", ".net", ".edu", 
            "bloomberg", "reuters", "wsj", "ft.com",
            "marketwatch", "seekingalpha", "linkedin",
            "crunchbase", "pitchbook", "govt",
        ];
        for pattern in &commercial_patterns {
            if lower.contains(pattern) {
                return Self::Commercial;
            }
        }

        Self::Unknown
    }

    /// Get the authority weight for scoring.
    pub fn weight(&self) -> f64 {
        match self {
            Self::Government => 1.0,
            Self::Commercial => 0.7,
            Self::Social => 0.4,
            Self::Unknown => 0.3,
        }
    }
}

/// Evidence freshness classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum FreshnessTier {
    /// Real-time or within 1 hour
    RealTime = 4,
    /// Within 24 hours
    Within24Hours = 3,
    /// Within 7 days
    Within7Days = 2,
    /// Within 30 days
    Within30Days = 1,
    /// Older than 30 days or unknown
    Unknown = 0,
}

impl FreshnessTier {
    /// Determine tier from observation time.
    pub fn from_datetime(observed: DateTime<Utc>, now: DateTime<Utc>) -> Self {
        let age_hours = (now - observed).num_hours();

        if age_hours <= 1 {
            Self::RealTime
        } else if age_hours <= 24 {
            Self::Within24Hours
        } else if age_hours <= 168 { // 7 days
            Self::Within7Days
        } else if age_hours <= 720 { // 30 days
            Self::Within30Days
        } else {
            Self::Unknown
        }
    }

    /// Get the freshness weight for scoring.
    pub fn weight(&self) -> f64 {
        match self {
            Self::RealTime => 1.0,
            Self::Within24Hours => 0.85,
            Self::Within7Days => 0.65,
            Self::Within30Days => 0.4,
            Self::Unknown => 0.2,
        }
    }

    /// Get human-readable label.
    pub fn label(&self) -> &'static str {
        match self {
            Self::RealTime => "real-time",
            Self::Within24Hours => "24h",
            Self::Within7Days => "7d",
            Self::Within30Days => "30d",
            Self::Unknown => "unknown",
        }
    }
}

// ============================================================================
// Risk Factor Analysis
// ============================================================================

/// A risk factor identified from evidence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskFactor {
    /// Unique risk factor ID
    pub id: Uuid,
    /// Risk category
    pub category: RiskCategory,
    /// Risk description
    pub description: String,
    /// Risk severity (0.0-1.0)
    pub severity: f64,
    /// Evidence supporting this risk
    pub supporting_evidence: Vec<Uuid>,
    /// Recommendations to mitigate this risk
    pub mitigations: Vec<String>,
}

impl RiskFactor {
    /// Create a new risk factor.
    pub fn new(category: RiskCategory, description: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            category,
            description: description.into(),
            severity: 0.5,
            supporting_evidence: Vec::new(),
            mitigations: Vec::new(),
        }
    }

    /// Set the severity (0.0-1.0).
    pub fn with_severity(mut self, severity: f64) -> Self {
        self.severity = severity.clamp(0.0, 1.0);
        self
    }

    /// Add supporting evidence ID.
    pub fn add_evidence(&mut self, evidence_id: Uuid) {
        self.supporting_evidence.push(evidence_id);
    }

    /// Add a mitigation recommendation.
    pub fn add_mitigation(&mut self, mitigation: impl Into<String>) {
        self.mitigations.push(mitigation.into());
    }
}

/// Risk categories for classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RiskCategory {
    /// Financial or economic risk
    Financial,
    /// Supply chain or operational risk
    SupplyChain,
    /// Regulatory or compliance risk
    Regulatory,
    /// Security or cyber risk
    Security,
    /// Reputational or brand risk
    Reputational,
    /// Competitive or market risk
    Competitive,
    /// Geopolitical or political risk
    Geopolitical,
    /// Environmental or ESG risk
    Environmental,
    /// Legal or litigation risk
    Legal,
    /// Strategic or business model risk
    Strategic,
}

impl RiskCategory {
    /// Get human-readable label.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Financial => "Financial",
            Self::SupplyChain => "Supply Chain",
            Self::Regulatory => "Regulatory",
            Self::Security => "Security",
            Self::Reputational => "Reputational",
            Self::Competitive => "Competitive",
            Self::Geopolitical => "Geopolitical",
            Self::Environmental => "Environmental",
            Self::Legal => "Legal",
            Self::Strategic => "Strategic",
        }
    }

    /// Get default severity for category.
    pub fn default_severity(&self) -> f64 {
        match self {
            Self::Financial => 0.7,
            Self::Security => 0.8,
            Self::Regulatory => 0.6,
            Self::SupplyChain => 0.5,
            _ => 0.4,
        }
    }
}

// ============================================================================
// Actionable Recommendations
// ============================================================================

/// An actionable recommendation derived from evidence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionRecommendation {
    /// Unique recommendation ID
    pub id: Uuid,
    /// Recommendation title
    pub title: String,
    /// Detailed action description
    pub description: String,
    /// Priority level
    pub priority: ActionPriority,
    /// Expected impact (0.0-1.0)
    pub impact: f64,
    /// Estimated time to implement (in hours)
    pub estimated_hours: Option<u32>,
    /// Risk level of taking action
    pub risk_level: ActionRisk,
    /// Stakeholders to involve
    pub stakeholders: Vec<String>,
    /// Success metrics
    pub success_metrics: Vec<String>,
}

impl ActionRecommendation {
    /// Create a new action recommendation.
    pub fn new(title: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            title: title.into(),
            description: description.into(),
            priority: ActionPriority::Medium,
            impact: 0.5,
            estimated_hours: None,
            risk_level: ActionRisk::Low,
            stakeholders: Vec::new(),
            success_metrics: Vec::new(),
        }
    }

    /// Set the priority.
    pub fn with_priority(mut self, priority: ActionPriority) -> Self {
        self.priority = priority;
        self
    }

    /// Set the expected impact.
    pub fn with_impact(mut self, impact: f64) -> Self {
        self.impact = impact.clamp(0.0, 1.0);
        self
    }

    /// Add a stakeholder.
    pub fn add_stakeholder(mut self, stakeholder: impl Into<String>) -> Self {
        self.stakeholders.push(stakeholder.into());
        self
    }

    /// Add a success metric.
    pub fn add_metric(mut self, metric: impl Into<String>) -> Self {
        self.success_metrics.push(metric.into());
        self
    }
}

/// Priority levels for actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ActionPriority {
    Critical = 4,
    High = 3,
    Medium = 2,
    Low = 1,
}

impl ActionPriority {
    /// Get human-readable label.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Critical => "Critical",
            Self::High => "High",
            Self::Medium => "Medium",
            Self::Low => "Low",
        }
    }
}

/// Risk level of taking an action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActionRisk {
    Low,
    Medium,
    High,
}

impl ActionRisk {
    /// Get human-readable label.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Low => "Low",
            Self::Medium => "Medium",
            Self::High => "High",
        }
    }
}

// ============================================================================
// Deep Insight Output
// ============================================================================

/// A fully developed deep insight with narrative, evidence, risk, and actions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeepInsight {
    /// Unique insight ID
    pub id: Uuid,
    /// Insight title
    pub title: String,
    /// Full investigative narrative
    pub narrative: String,
    /// Executive summary
    pub summary: String,
    /// Evidence compilation
    pub evidence: Vec<DeepEvidence>,
    /// Confidence score (0.0-1.0)
    pub confidence: f64,
    /// Overall impact score (0.0-1.0)
    pub impact: f64,
    /// Severity level
    pub severity: InsightSeverityLevel,
    /// Risk factors identified
    pub risk_factors: Vec<RiskFactor>,
    /// Actionable recommendations
    pub recommendations: Vec<ActionRecommendation>,
    /// Source credibility score (0.0-1.0)
    pub source_credibility: f64,
    /// Corroboration score (number of independent sources)
    pub corroboration_count: usize,
    /// When this insight was generated
    pub generated_at: DateTime<Utc>,
    /// Entities this insight relates to
    pub entities: Vec<String>,
    /// Tags for categorization
    pub tags: Vec<String>,
}

impl DeepInsight {
    /// Create a new deep insight.
    pub fn new(title: impl Into<String>, narrative: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            title: title.into(),
            narrative: narrative.into(),
            summary: String::new(),
            evidence: Vec::new(),
            confidence: 0.5,
            impact: 0.5,
            severity: InsightSeverityLevel::Medium,
            risk_factors: Vec::new(),
            recommendations: Vec::new(),
            source_credibility: 0.5,
            corroboration_count: 0,
            generated_at: Utc::now(),
            entities: Vec::new(),
            tags: Vec::new(),
        }
    }

    /// Generate the executive summary from narrative.
    pub fn generate_summary(&mut self) {
        let max_summary_len = 300;
        let narrative = self.narrative.trim();
        
        // Try to extract first sentence or paragraph
        let summary = if let Some(pos) = narrative.find('.') {
            let candidate = &narrative[..=pos];
            if candidate.len() <= max_summary_len {
                candidate.to_string()
            } else {
                truncate_string(narrative, max_summary_len)
            }
        } else {
            truncate_string(narrative, max_summary_len)
        };
        
        self.summary = summary;
    }

    /// Calculate overall confidence from evidence.
    pub fn calculate_confidence(&mut self, config: &DeepInsightConfig) {
        if self.evidence.is_empty() {
            self.confidence = 0.3;
            return;
        }

        // Count corroborating sources
        let mut unique_domains: HashSet<&str> = HashSet::new();
        for e in &self.evidence {
            if let Some(domain) = e.source_domain.as_ref() {
                unique_domains.insert(domain.as_str());
            }
        }
        self.corroboration_count = unique_domains.len();

        // Calculate weighted confidence
        let mut total_weight = 0.0;
        let mut weighted_sum = 0.0;

        for evidence in &self.evidence {
            let weight = evidence.authority_tier.weight() * evidence.freshness_tier.weight();
            weighted_sum += evidence.confidence * weight;
            total_weight += weight;
        }

        let base_confidence = if total_weight > 0.0 {
            weighted_sum / total_weight
        } else {
            0.5
        };

        // Apply corroboration bonus
        let corroboration_bonus = (self.corroboration_count as f64 * 0.05).min(0.2);
        
        // Apply evidence count bonus
        let evidence_bonus = if self.evidence.len() >= config.min_evidence_count {
            0.1
        } else {
            0.0
        };

        self.confidence = (base_confidence + corroboration_bonus + evidence_bonus).clamp(0.0, 1.0);
        
        // Calculate source credibility
        let authority_sum: f64 = self.evidence.iter()
            .map(|e| e.authority_tier.weight())
            .sum();
        self.source_credibility = if !self.evidence.is_empty() {
            authority_sum / self.evidence.len() as f64
        } else {
            0.3
        };
    }

    /// Determine severity based on confidence and impact.
    pub fn determine_severity(&mut self) {
        let combined_score = self.confidence * 0.6 + self.impact * 0.4;
        
        self.severity = if combined_score >= 0.8 {
            InsightSeverityLevel::Critical
        } else if combined_score >= 0.6 {
            InsightSeverityLevel::High
        } else if combined_score >= 0.4 {
            InsightSeverityLevel::Medium
        } else {
            InsightSeverityLevel::Low
        };
    }

    /// Add a risk factor.
    pub fn add_risk_factor(&mut self, risk: RiskFactor) {
        self.risk_factors.push(risk);
    }

    /// Add an action recommendation.
    pub fn add_recommendation(&mut self, recommendation: ActionRecommendation) {
        self.recommendations.push(recommendation);
    }

    /// Add an entity tag.
    pub fn add_entity(&mut self, entity: impl Into<String>) {
        let entity_str = entity.into();
        let entity_lower = entity_str.to_lowercase();
        // Case-insensitive deduplication
        if !self.entities.iter().any(|e| e.to_lowercase() == entity_lower) {
            self.entities.push(entity_str);
        }
    }

    /// Add a tag.
    pub fn add_tag(&mut self, tag: impl Into<String>) {
        let tag_str = tag.into();
        let tag_lower = tag_str.to_lowercase();
        // Case-insensitive deduplication
        if !self.tags.iter().any(|t| t.to_lowercase() == tag_lower) {
            self.tags.push(tag_str);
        }
    }
}

/// Insight severity levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum InsightSeverityLevel {
    Critical,
    High,
    Medium,
    Low,
    Info,
}

impl InsightSeverityLevel {
    /// Get human-readable label.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Critical => "Critical",
            Self::High => "High",
            Self::Medium => "Medium",
            Self::Low => "Low",
            Self::Info => "Info",
        }
    }

    /// Get numeric priority (higher = more urgent).
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

// ============================================================================
// Deep Insight Generator
// ============================================================================

/// Generator for deep insights from evidence.
pub struct DeepInsightGenerator {
    config: DeepInsightConfig,
}

impl DeepInsightGenerator {
    /// Create a new generator with default configuration.
    pub fn new() -> Self {
        Self {
            config: DeepInsightConfig::default(),
        }
    }

    /// Create a new generator with custom configuration.
    pub fn with_config(config: DeepInsightConfig) -> Self {
        Self { config }
    }

    /// Generate a deep insight from evidence.
    pub fn generate(&self, title: &str, evidence: &[DeepEvidence]) -> DeepInsight {
        let mut insight = DeepInsight::new(title, "");
        
        // Add evidence
        insight.evidence.extend(evidence.iter().cloned());

        // Calculate confidence
        insight.calculate_confidence(&self.config);
        
        // Determine severity
        insight.determine_severity();

        // Add entities from evidence
        for e in evidence {
            if let Some(ref entity) = e.entity {
                insight.add_entity(entity.clone());
            }
        }

        // Generate narrative
        insight.narrative = self.build_narrative(&insight);
        
        // Generate summary
        insight.generate_summary();

        // Identify risk factors
        if self.config.include_risk_factors {
            self.identify_risk_factors(&mut insight);
        }

        // Generate recommendations
        if self.config.include_recommendations {
            self.generate_recommendations(&mut insight);
        }

        insight
    }

    /// Build investigative narrative from evidence.
    fn build_narrative(&self, insight: &DeepInsight) -> String {
        let mut narrative = String::new();

        // Opening context
        narrative.push_str("## Intelligence Assessment\n\n");
        
        // Evidence summary
        narrative.push_str(&format!(
            "Based on {} pieces of evidence from {} independent source{}:\n\n",
            insight.evidence.len(),
            insight.corroboration_count,
            if insight.corroboration_count == 1 { "" } else { "s" }
        ));

        // Evidence compilation with citations
        narrative.push_str("### Evidence Compilation\n\n");
        for (i, ev) in insight.evidence.iter().enumerate() {
            let freshness_label = ev.freshness_tier.label();
            let authority_label = match ev.authority_tier {
                AuthorityTier::Government => "Official",
                AuthorityTier::Commercial => "Commercial",
                AuthorityTier::Social => "Social",
                AuthorityTier::Unknown => "Unknown",
            };
            
            narrative.push_str(&format!(
                "[{}] {} [{}] {}\n",
                i + 1,
                ev.description,
                freshness_label,
                authority_label
            ));
            
            if let Some(ref source) = ev.source {
                narrative.push_str(&format!("    Source: {}\n", source));
            }
            narrative.push('\n');
        }

        // Analysis section
        narrative.push_str("### Analysis\n\n");
        
        // Confidence assessment
        let confidence_pct = (insight.confidence * 100.0).round() as i32;
        let credibility_pct = (insight.source_credibility * 100.0).round() as i32;
        
        narrative.push_str(&format!(
            "Overall confidence: {}% (source credibility: {}%)\n\n",
            confidence_pct, credibility_pct
        ));

        // Risk factors if present
        if !insight.risk_factors.is_empty() {
            narrative.push_str("### Risk Assessment\n\n");
            for risk in &insight.risk_factors {
                narrative.push_str(&format!(
                    "- **[{}]** {} (Severity: {:.0}%)\n",
                    risk.category.label(),
                    risk.description,
                    risk.severity * 100.0
                ));
            }
            narrative.push('\n');
        }

        // Truncate if needed
        if narrative.len() > self.config.max_narrative_length {
            truncate_string(&narrative, self.config.max_narrative_length)
        } else {
            narrative
        }
    }

    /// Identify risk factors from evidence.
    fn identify_risk_factors(&self, insight: &mut DeepInsight) {
        // Pre-collect evidence data to avoid borrow conflicts
        let evidence_data: Vec<(Uuid, String)> = insight.evidence.iter()
            .map(|ev| (ev.id, ev.description.clone()))
            .collect();

        // Build risk patterns
        let mut risk_patterns: HashMap<RiskCategory, Vec<String>> = HashMap::new();

        for (_, desc) in &evidence_data {
            let desc_lower = desc.to_lowercase();

            // Financial risk patterns
            if desc_lower.contains("debt") || desc_lower.contains("bankruptcy")
                || desc_lower.contains("delinquency") || desc_lower.contains("default") {
                risk_patterns.entry(RiskCategory::Financial)
                    .or_default()
                    .push(desc.clone());
            }

            // Supply chain risk patterns
            if desc_lower.contains("shortage") || desc_lower.contains("disruption")
                || desc_lower.contains("delay") || desc_lower.contains("supplier") {
                risk_patterns.entry(RiskCategory::SupplyChain)
                    .or_default()
                    .push(desc.clone());
            }

            // Regulatory risk patterns
            if desc_lower.contains("sanction") || desc_lower.contains("violation")
                || desc_lower.contains("penalty") || desc_lower.contains("investigation") {
                risk_patterns.entry(RiskCategory::Regulatory)
                    .or_default()
                    .push(desc.clone());
            }

            // Security risk patterns
            if desc_lower.contains("breach") || desc_lower.contains("cyber")
                || desc_lower.contains("attack") || desc_lower.contains("vulnerability") {
                risk_patterns.entry(RiskCategory::Security)
                    .or_default()
                    .push(desc.clone());
            }

            // Reputational risk patterns
            if desc_lower.contains("scandal") || desc_lower.contains("fraud")
                || desc_lower.contains("lawsuit") || desc_lower.contains("controversy") {
                risk_patterns.entry(RiskCategory::Reputational)
                    .or_default()
                    .push(desc.clone());
            }
        }

        // Create risk factors - collect all risk factors first, then add them
        let mut risk_factors_to_add = Vec::new();
        
        for (category, descriptions) in risk_patterns {
            if !descriptions.is_empty() {
                let mut risk = RiskFactor::new(
                    category,
                    format!("{} risk identified from {} indicator(s)",
                        category.label(), descriptions.len())
                );
                risk = risk.with_severity(category.default_severity());

                // Find matching evidence IDs
                for (id, desc) in &evidence_data {
                    if descriptions.iter().any(|d| d == desc) {
                        risk.add_evidence(*id);
                    }
                }
                
                risk_factors_to_add.push(risk);
            }
        }

        // Add all risk factors at once
        for risk in risk_factors_to_add {
            insight.add_risk_factor(risk);
        }
    }

    /// Generate actionable recommendations.
    fn generate_recommendations(&self, insight: &mut DeepInsight) {
        // Only generate recommendations if confidence meets threshold
        if insight.confidence < self.config.action_confidence_threshold {
            return;
        }

        // Generate based on severity
        match insight.severity {
            InsightSeverityLevel::Critical | InsightSeverityLevel::High => {
                // Add immediate action recommendations
                insight.add_recommendation(
                    ActionRecommendation::new(
                        "Immediate Investigation Required",
                        format!(
                            "High confidence ({:.0}%) and {} corroborating sources indicate this \
                            insight warrants immediate investigation and stakeholder notification.",
                            insight.confidence * 100.0,
                            insight.corroboration_count
                        )
                    )
                    .with_priority(ActionPriority::Critical)
                    .with_impact(0.9)
                    .add_stakeholder("Intelligence Team")
                    .add_stakeholder("Executive Leadership")
                );

                // Risk-specific recommendations
                let mut risk_recommendations = Vec::new();
                for risk in &insight.risk_factors {
                    let rec = ActionRecommendation::new(
                        format!("Mitigate {} Risk", risk.category.label()),
                        format!(
                            "Address {} risk with {} supporting evidence indicators.",
                            risk.category.label(),
                            risk.supporting_evidence.len()
                        )
                    )
                    .with_priority(ActionPriority::High)
                    .with_impact(risk.severity)
                    .add_metric("Risk factor resolved or mitigated");
                    risk_recommendations.push(rec);
                }
                for rec in risk_recommendations {
                    insight.add_recommendation(rec);
                }
            }
            _ => {
                // Lower priority recommendations
                insight.add_recommendation(
                    ActionRecommendation::new(
                        "Monitor Situation",
                        format!(
                            "Continue monitoring this development. Confidence level is {:.0}% \
                            with {} source(s) providing corroboration.",
                            insight.confidence * 100.0,
                            insight.corroboration_count
                        )
                    )
                    .with_priority(ActionPriority::Medium)
                    .with_impact(0.5)
                );
            }
        }
    }

    /// Get configuration reference.
    pub fn config(&self) -> &DeepInsightConfig {
        &self.config
    }
}

impl Default for DeepInsightGenerator {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Utility Functions
// ============================================================================

fn truncate_string(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        return s.to_string();
    }
    
    // Find a good break point (space or punctuation)
    let truncated = &s[..max_len];
    if let Some(last_space) = truncated.rfind(|c: char| c.is_whitespace() || c == '.' || c == ',') {
        format!("{}...", &truncated[..last_space])
    } else {
        format!("{}...", truncated)
    }
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

    fn create_test_evidence(domain: &str, authority: AuthorityTier, confidence: f64) -> DeepEvidence {
        let now = Utc::now();
        DeepEvidence::new(format!("Test evidence from {}", domain))
            .with_domain(domain)
            .with_authority(authority)
            .with_observed_at(now)
            .with_confidence(confidence)
    }

    #[test]
    fn test_authority_tier_from_domain() {
        assert_eq!(AuthorityTier::from_domain("irs.gov"), AuthorityTier::Government);
        assert_eq!(AuthorityTier::from_domain("sec.gov"), AuthorityTier::Government);
        assert_eq!(AuthorityTier::from_domain("treasury.gov"), AuthorityTier::Government);
        assert_eq!(AuthorityTier::from_domain("reuters.com"), AuthorityTier::Commercial);
        assert_eq!(AuthorityTier::from_domain("bloomberg.com"), AuthorityTier::Commercial);
        assert_eq!(AuthorityTier::from_domain("twitter.com"), AuthorityTier::Social);
        assert_eq!(AuthorityTier::from_domain("unknown.xyz"), AuthorityTier::Unknown);
    }

    #[test]
    fn test_authority_tier_weights() {
        assert!((AuthorityTier::Government.weight() - 1.0).abs() < 0.001);
        assert!((AuthorityTier::Commercial.weight() - 0.7).abs() < 0.001);
        assert!((AuthorityTier::Social.weight() - 0.4).abs() < 0.001);
        assert!((AuthorityTier::Unknown.weight() - 0.3).abs() < 0.001);
    }

    #[test]
    fn test_freshness_tier_from_datetime() {
        let now = Utc::now();
        
        // Real-time (within 1 hour)
        let recent = now - chrono::Duration::minutes(30);
        assert_eq!(FreshnessTier::from_datetime(recent, now), FreshnessTier::RealTime);
        
        // Within 24 hours
        let yesterday = now - chrono::Duration::hours(20);
        assert_eq!(FreshnessTier::from_datetime(yesterday, now), FreshnessTier::Within24Hours);
        
        // Within 7 days
        let week_ago = now - chrono::Duration::days(5);
        assert_eq!(FreshnessTier::from_datetime(week_ago, now), FreshnessTier::Within7Days);
        
        // Within 30 days
        let month_ago = now - chrono::Duration::days(20);
        assert_eq!(FreshnessTier::from_datetime(month_ago, now), FreshnessTier::Within30Days);
        
        // Unknown (old)
        let old = now - chrono::Duration::days(60);
        assert_eq!(FreshnessTier::from_datetime(old, now), FreshnessTier::Unknown);
    }

    #[test]
    fn test_deep_insight_generation() {
        let generator = DeepInsightGenerator::new();
        
        let evidence = vec![
            create_test_evidence("reuters.com", AuthorityTier::Commercial, 0.85),
            create_test_evidence("sec.gov", AuthorityTier::Government, 0.9),
            create_test_evidence("bloomberg.com", AuthorityTier::Commercial, 0.80),
        ];
        
        let insight = generator.generate("Test Insight", &evidence);
        
        assert!(insight.narrative.contains("3 pieces of evidence"));
        assert!(insight.narrative.contains("3 independent source"));
        assert_eq!(insight.corroboration_count, 3);
        assert!(insight.confidence > 0.7);
    }

    #[test]
    fn test_deep_insight_calculates_confidence() {
        let mut insight = DeepInsight::new("Test", "Test narrative");
        
        insight.evidence.push(
            DeepEvidence::new("High confidence evidence")
                .with_authority(AuthorityTier::Government)
                .with_observed_at(Utc::now())
                .with_confidence(0.9)
        );
        
        insight.evidence.push(
            DeepEvidence::new("Another evidence")
                .with_domain("reuters.com")
                .with_authority(AuthorityTier::Commercial)
                .with_observed_at(Utc::now())
                .with_confidence(0.8)
        );
        
        insight.calculate_confidence(&DeepInsightConfig::default());
        
        assert!(insight.confidence > 0.7);
        assert_eq!(insight.corroboration_count, 1); // Only reuters.com (gov is unknown domain)
    }

    #[test]
    fn test_deep_insight_severity_determination() {
        let mut insight = DeepInsight::new("Test", "Test");
        insight.confidence = 0.85;
        insight.impact = 0.8;
        insight.determine_severity();
        assert_eq!(insight.severity, InsightSeverityLevel::Critical);
        
        let mut insight2 = DeepInsight::new("Test", "Test");
        insight2.confidence = 0.6;
        insight2.impact = 0.6;
        insight2.determine_severity();
        assert_eq!(insight2.severity, InsightSeverityLevel::High);
        
        let mut insight3 = DeepInsight::new("Test", "Test");
        insight3.confidence = 0.4;
        insight3.impact = 0.4;
        insight3.determine_severity();
        assert_eq!(insight3.severity, InsightSeverityLevel::Medium);
    }

    #[test]
    fn test_risk_factor_identification() {
        let mut insight = DeepInsight::new("Financial Risk Test", "Test");
        
        insight.evidence.push(
            DeepEvidence::new("Company has significant debt load")
                .with_confidence(0.8)
                .with_entity("Test Corp")
        );
        
        insight.evidence.push(
            DeepEvidence::new("Supply chain disruption reported")
                .with_confidence(0.75)
                .with_entity("Test Corp")
        );
        
        // Add risk factors manually
        insight.add_risk_factor(
            RiskFactor::new(RiskCategory::Financial, "High debt load detected")
                .with_severity(0.7)
        );
        
        assert_eq!(insight.risk_factors.len(), 1);
        assert_eq!(insight.risk_factors[0].category, RiskCategory::Financial);
    }

    #[test]
    fn test_action_recommendation_generation() {
        let mut insight = DeepInsight::new("Test", "Test");
        insight.confidence = 0.8;
        insight.severity = InsightSeverityLevel::High;
        insight.corroboration_count = 3;
        
        insight.add_recommendation(
            ActionRecommendation::new(
                "Immediate Action Required",
                "Based on high confidence insight"
            )
            .with_priority(ActionPriority::Critical)
            .with_impact(0.9)
        );
        
        assert_eq!(insight.recommendations.len(), 1);
        assert_eq!(insight.recommendations[0].priority, ActionPriority::Critical);
    }

    #[test]
    fn test_summary_generation() {
        let mut insight = DeepInsight::new(
            "Test",
            "This is a longer narrative that contains multiple sentences. \
            The first sentence provides key context about the subject matter."
        );
        
        insight.generate_summary();
        
        // Should contain the first sentence
        assert!(insight.summary.contains("multiple sentences"));
    }

    #[test]
    fn test_entity_deduplication() {
        let mut insight = DeepInsight::new("Test", "Test");
        insight.add_entity("NVIDIA");
        insight.add_entity("Nvidia");
        insight.add_entity("nvidia");
        
        // Should only have one entity (deduplicated)
        assert_eq!(insight.entities.len(), 1);
    }

    #[test]
    fn test_deep_evidence_builder() {
        let evidence = DeepEvidence::new("Test evidence")
            .with_source("https://example.com/article")
            .with_domain("example.com")
            .with_authority(AuthorityTier::Commercial)
            .with_observed_at(Utc::now())
            .with_confidence(0.85)
            .with_entity("Test Corp");
        
        assert_eq!(evidence.description, "Test evidence");
        assert_eq!(evidence.source_domain, Some("example.com".to_string()));
        assert_eq!(evidence.authority_tier, AuthorityTier::Commercial);
        assert_eq!(evidence.confidence, 0.85);
    }

    #[test]
    fn test_generator_config() {
        let config = DeepInsightConfig {
            min_evidence_count: 3,
            min_corroboration_sources: 2,
            max_narrative_length: 10000,
            include_risk_factors: true,
            include_recommendations: true,
            action_confidence_threshold: 0.7,
        };
        
        let generator = DeepInsightGenerator::with_config(config);
        assert_eq!(generator.config().min_evidence_count, 3);
        assert_eq!(generator.config().action_confidence_threshold, 0.7);
    }

    #[test]
    fn test_truncate_string() {
        let long_text = "This is a very long text that needs to be truncated because it's too long for the display area.";
        let truncated = truncate_string(long_text, 50);
        assert!(truncated.len() <= 53); // 50 + "..."
        assert!(truncated.ends_with("..."));
    }
}
