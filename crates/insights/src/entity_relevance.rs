//! Entity-Relevance Validation Module
//!
//! This module provides semantic validation to ensure insights are actually
//! relevant to the entities they claim to be about. This fixes Issue #1
//! (Entity-Signal Decoupling) and Issue #5 (Generic Templates).
//!
//! The key insight is that an insight about NVIDIA should actually mention
//! NVIDIA-specific topics (GPUs, AI chips, data centers, etc.), not just
//! generic geopolitical news.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Entity profile containing keywords and topics that are relevant to this entity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityProfile {
    /// Entity name (e.g., "NVIDIA", "Foxconn")
    pub entity_name: String,
    /// Industry/vertical keywords specific to this entity
    pub industry_keywords: Vec<String>,
    /// Product/service names specific to this entity
    pub product_keywords: Vec<String>,
    /// Geographic regions where this entity operates
    pub geographic_keywords: Vec<String>,
    /// Competitor names (for competitive intel)
    pub competitor_keywords: Vec<String>,
    /// Topics this entity is known for (AI, chips, supply chain, etc.)
    pub topic_keywords: Vec<String>,
}

impl EntityProfile {
    /// Create a new entity profile
    pub fn new(entity_name: &str) -> Self {
        Self {
            entity_name: entity_name.to_string(),
            industry_keywords: Vec::new(),
            product_keywords: Vec::new(),
            geographic_keywords: Vec::new(),
            competitor_keywords: Vec::new(),
            topic_keywords: Vec::new(),
        }
    }

    /// Add industry keywords
    pub fn with_industry(mut self, keywords: Vec<&str>) -> Self {
        self.industry_keywords = keywords.into_iter().map(String::from).collect();
        self
    }

    /// Add product keywords
    pub fn with_products(mut self, keywords: Vec<&str>) -> Self {
        self.product_keywords = keywords.into_iter().map(String::from).collect();
        self
    }

    /// Add topic keywords
    pub fn with_topics(mut self, keywords: Vec<&str>) -> Self {
        self.topic_keywords = keywords.into_iter().map(String::from).collect();
        self
    }

    /// Add geographic keywords
    pub fn with_geography(mut self, keywords: Vec<&str>) -> Self {
        self.geographic_keywords = keywords.into_iter().map(String::from).collect();
        self
    }

    /// Add competitor keywords
    pub fn with_competitors(mut self, keywords: Vec<&str>) -> Self {
        self.competitor_keywords = keywords.into_iter().map(String::from).collect();
        self
    }

    /// Get all keywords as a single set
    pub fn all_keywords(&self) -> HashSet<String> {
        let mut keywords = HashSet::new();
        for kw in &self.industry_keywords {
            keywords.insert(kw.to_lowercase());
        }
        for kw in &self.product_keywords {
            keywords.insert(kw.to_lowercase());
        }
        for kw in &self.topic_keywords {
            keywords.insert(kw.to_lowercase());
        }
        for kw in &self.geographic_keywords {
            keywords.insert(kw.to_lowercase());
        }
        for kw in &self.competitor_keywords {
            keywords.insert(kw.to_lowercase());
        }
        // Also add entity name itself
        keywords.insert(self.entity_name.to_lowercase());
        keywords
    }
}

/// Result of relevance validation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelevanceValidation {
    /// Overall relevance score (0.0 - 1.0)
    pub relevance_score: f64,
    /// Whether the insight passes the relevance threshold
    pub is_relevant: bool,
    /// Keywords found in the insight
    pub matched_keywords: Vec<String>,
    /// Keywords that were expected but not found
    pub missing_keywords: Vec<String>,
    /// Specificity score - how unique is this to the entity?
    pub specificity_score: f64,
    /// Warning messages
    pub warnings: Vec<String>,
}

/// Relevance validator for insights
pub struct RelevanceValidator {
    /// Minimum relevance score threshold
    min_relevance_threshold: f64,
    /// Minimum specificity score threshold
    min_specificity_threshold: f64,
}

impl Default for RelevanceValidator {
    fn default() -> Self {
        Self {
            min_relevance_threshold: 0.3,
            min_specificity_threshold: 0.2,
        }
    }
}

impl RelevanceValidator {
    /// Create a new validator with custom thresholds
    pub fn new(min_relevance: f64, min_specificity: f64) -> Self {
        Self {
            min_relevance_threshold: min_relevance,
            min_specificity_threshold: min_specificity,
        }
    }

    /// Validate an insight against an entity profile
    pub fn validate(&self, insight_text: &str, profile: &EntityProfile) -> RelevanceValidation {
        let insight_lower = insight_text.to_lowercase();
        let keywords = profile.all_keywords();
        
        let mut matched_keywords = Vec::new();
        let mut missing_keywords = Vec::new();
        let mut warning_messages = Vec::new();

        // Check for entity name mentions
        let entity_name_mentioned = insight_lower.contains(&profile.entity_name.to_lowercase());
        
        // Check keyword matches
        for keyword in &keywords {
            if insight_lower.contains(keyword) {
                matched_keywords.push(keyword.clone());
            } else {
                missing_keywords.push(keyword.clone());
            }
        }

        // Calculate relevance score.  Rich profiles may have 40+ keywords;
        // dividing by the full pool would penalise well-described entities.
        // Cap the denominator so matching a reasonable number of keywords
        // produces a strong signal regardless of vocabulary size.
        let total_keywords = keywords.len();
        let effective_total = (total_keywords as f64).min(10.0);
        let relevance_score = if effective_total > 0.0 {
            (matched_keywords.len() as f64 / effective_total).min(1.0)
        } else {
            0.0
        };

        // Calculate specificity - how many non-generic keywords matched?
        // Generic keywords reduce specificity (e.g., "market", "global", "news")
        let generic_keywords = vec![
            "market", "global", "news", "report", "update", "latest",
            "breaking", "alert", "industry", "sector", "economy", "business"
        ];
        
        let non_generic_matches: Vec<&String> = matched_keywords
            .iter()
            .filter(|k| !generic_keywords.contains(&k.as_str()))
            .collect();
        
        let specificity_score = if !matched_keywords.is_empty() {
            non_generic_matches.len() as f64 / matched_keywords.len() as f64
        } else {
            0.0
        };

        // Check for specific entity-relevant content
        if !entity_name_mentioned {
            warning_messages.push(format!(
                "Entity '{}' not explicitly mentioned in insight",
                profile.entity_name
            ));
        }

        if matched_keywords.is_empty() {
            warning_messages.push("No entity-specific keywords found in insight".to_string());
        }

        if relevance_score < self.min_relevance_threshold {
            warning_messages.push(format!(
                "Relevance score ({:.2}) below threshold ({:.2})",
                relevance_score, self.min_relevance_threshold
            ));
        }

        // Check for generic content that doesn't belong to any specific entity
        let generic_content_indicators = [
            "geopolitical tensions",
            "oil prices",
            "global markets",
            "middle east",
            "escalating",
        ];
        
        let mut generic_count = 0;
        for indicator in generic_content_indicators {
            if insight_lower.contains(indicator) {
                generic_count += 1;
            }
        }
        
        if generic_count >= 3 && matched_keywords.len() <= 2 {
            warning_messages.push("Insight appears to contain mostly generic content".to_string());
        }

        // Determine if relevant
        let is_relevant = relevance_score >= self.min_relevance_threshold
            && specificity_score >= self.min_specificity_threshold
            && entity_name_mentioned;

        RelevanceValidation {
            relevance_score,
            is_relevant,
            matched_keywords,
            missing_keywords: missing_keywords,
            specificity_score,
            warnings: warning_messages,
        }
    }

    /// Validate insight with automatic entity profile generation
    pub fn validate_with_entity_name(&self, insight_text: &str, entity_name: &str) -> RelevanceValidation {
        // Create a basic profile from entity name
        let profile = EntityProfile::new(entity_name);
        self.validate(insight_text, &profile)
    }
}

/// Pre-built profiles for common tech entities
pub mod common_profiles {
    use super::EntityProfile;

    pub fn nvidia_profile() -> EntityProfile {
        EntityProfile::new("NVIDIA")
            .with_industry(vec!["semiconductor", "chip", "GPU", "AI hardware", "data center"])
            .with_products(vec![
                "A100", "H100", "V100", "RTX", "GeForce", "Tesla", "DGX",
                "Jetson", "CUDA", "Tensor Core", "Hopper", "Ada Lovelace"
            ])
            .with_topics(vec![
                "AI", "artificial intelligence", "machine learning", "deep learning",
                "GPU computing", "HPC", "high performance computing", "data center",
                "gaming", "autonomous vehicle", "robotics", "metaverse", "LLM", "GPT"
            ])
            .with_geography(vec!["Santa Clara", "California", "USA", "Taiwan", "China"])
            .with_competitors(vec!["AMD", "Intel", "Qualcomm", "Google", "TPU", "Amazon", "Trainium"])
    }

    pub fn tsmc_profile() -> EntityProfile {
        EntityProfile::new("TSMC")
            .with_industry(vec!["semiconductor", "foundry", "chip manufacturing", "fab"])
            .with_products(vec!["3nm", "5nm", "7nm", "28nm", "wafer", "先进製程"])
            .with_topics(vec!["advanced node", "chip shortage", "fabrication", "wafer", "EUV"])
            .with_geography(vec!["Taiwan", "Arizona", "USA", "Tainan", "Hsinchu"])
            .with_competitors(vec!["Samsung", "Intel", "GlobalFoundries", "SMIC"])
    }

    pub fn foxconn_profile() -> EntityProfile {
        EntityProfile::new("Foxconn")
            .with_industry(vec!["EMS", "electronics manufacturing", "contract manufacturer"])
            .with_products(vec!["iPhone", "smartphone", "assembly", "OEM"])
            .with_topics(vec!["supply chain", "manufacturing", "labor", "factory", "assembly"])
            .with_geography(vec!["Taiwan", "China", "Vietnam", "India", "Wisconsin"])
            .with_competitors(vec!["Flex", "Jabil", "Pegatron", "Wistron"])
    }

    pub fn get_profile_for_entity(entity_name: &str) -> Option<EntityProfile> {
        match entity_name.to_lowercase().as_str() {
            "nvidia" | "nvidia corporation" => Some(nvidia_profile()),
            "tsmc" | "taiwan semiconductor" | "taiwan semiconductor manufacturing company" => Some(tsmc_profile()),
            "foxconn" | "hon hai" | "hon hai precision" => Some(foxconn_profile()),
            _ => None,
        }
    }
}

/// Stale insight detector - checks if similar insights have fired recently
#[derive(Debug, Clone)]
pub struct StalenessDetector {
    /// Maximum number of times an entity-recipe combo can fire before suppression
    max_repetitions: u32,
    /// Days to look back for repetition check
    lookback_days: i64,
}

impl Default for StalenessDetector {
    fn default() -> Self {
        Self {
            max_repetitions: 2,
            lookback_days: 14,
        }
    }
}

impl StalenessDetector {
    /// Check if an entity-recipe combination is becoming stale
    /// Returns (is_stale, repetition_count, suppression_recommended)
    pub fn check_staleness(
        &self,
        _entity_id: &str,
        _recipe_code: &str,
        historical_firings: &[(i64, String)], // (timestamp, insight_summary_hash)
    ) -> (bool, u32, bool) {
        let now = chrono::Utc::now().timestamp();
        let lookback_seconds = self.lookback_days * 86400;
        let cutoff = now - lookback_seconds;

        // Filter to recent firings within the lookback window
        let recent_hashes: Vec<&str> = historical_firings
            .iter()
            .filter(|(ts, _)| *ts >= cutoff)
            .map(|(_, hash)| hash.as_str())
            .collect();

        let repetition_count = recent_hashes.len() as u32;

        // Deduplicate to count unique content
        let unique: HashSet<&str> = recent_hashes.iter().copied().collect();
        let unique_count = unique.len() as u32;

        // Stale if total firings exceed threshold and most are repetitive
        let is_stale = repetition_count >= self.max_repetitions
            && unique_count < repetition_count;
        let suppress = repetition_count > self.max_repetitions
            && unique_count * 2 < repetition_count;

        (is_stale, repetition_count, suppress)
    }

    /// Calculate staleness penalty for confidence score
    pub fn staleness_penalty(&self, repetition_count: u32) -> f64 {
        // Apply penalty: each repetition reduces confidence
        // 0 reps = 1.0 (no penalty), 1 rep = 0.8, 2+ reps = 0.5
        match repetition_count {
            0 => 1.0,
            1 => 0.8,
            _ => 0.5,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nvidia_profile_rejects_generic_content() {
        let validator = RelevanceValidator::default();
        let profile = common_profiles::nvidia_profile();
        
        // This is the BAD insight - generic geopolitical content
        let bad_insight = "Geopolitical tensions in the Middle East are escalating, causing oil prices to surge and impacting global markets. Coverage from multiple sources is being compared for corroboration.";
        
        let result = validator.validate(bad_insight, &profile);
        
        // Should fail relevance check
        assert!(!result.is_relevant, "Generic content should not be relevant to NVIDIA");
        assert!(result.warnings.iter().any(|w| w.contains("generic")), 
            "Should warn about generic content");
    }

    #[test]
    fn test_nvidia_profile_accepts_specific_content() {
        let validator = RelevanceValidator::default();
        let profile = common_profiles::nvidia_profile();
        
        // This is GOOD - NVIDIA-specific content
        let good_insight = "NVIDIA announces new H100 AI chips for data center expansion. The GPU computing demand continues to drive market growth. CUDA ecosystem expands with new AI frameworks.";
        
        let result = validator.validate(good_insight, &profile);
        
        // Should pass relevance check
        assert!(result.is_relevant, "NVIDIA-specific content should be relevant");
        assert!(result.matched_keywords.iter().any(|k| k.contains("nvidia")));
        assert!(result.matched_keywords.iter().any(|k| k.contains("gpu")));
        assert!(result.matched_keywords.iter().any(|k| k.contains("h100")));
    }

    #[test]
    fn test_staleness_detection() {
        let detector = StalenessDetector::default();
        
        let now = chrono::Utc::now().timestamp();
        // Same insight fired 3 times recently
        let history = vec![
            (now - 100, "abc123".to_string()),
            (now - 50, "abc123".to_string()),
            (now - 10, "abc123".to_string()),
        ];
        
        let (is_stale, count, suppress) = detector.check_staleness("nvidia-123", "A001", &history);
        
        assert!(is_stale);
        assert!(count >= 3);
        assert!(suppress);
    }

    #[test]
    fn test_staleness_penalty() {
        let detector = StalenessDetector::default();
        
        assert!((detector.staleness_penalty(0) - 1.0).abs() < 0.01);
        assert!((detector.staleness_penalty(1) - 0.8).abs() < 0.01);
        assert!((detector.staleness_penalty(2) - 0.5).abs() < 0.01);
    }
}

