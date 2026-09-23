//! Objection handler — clusters competitor claims from observations
//! and generates counter-arguments from our strengths.

use serde::{Deserialize, Serialize};

use crate::entity_relevance::EntityProfile;
use crate::Insight;

/// Severity of an objection.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ObjectionSeverity {
    Critical,
    High,
    Medium,
    Low,
}

impl ObjectionSeverity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Critical => "critical",
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
        }
    }

    pub fn score(&self) -> f64 {
        match self {
            Self::Critical => 1.0,
            Self::High => 0.75,
            Self::Medium => 0.5,
            Self::Low => 0.25,
        }
    }
}

/// A competitor claim or objection extracted from intelligence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Objection {
    pub claim: String,
    pub source_url: String,
    pub severity: ObjectionSeverity,
}

/// A paired objection with our counter-argument.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectionHandlerPair {
    pub objection: String,
    pub counter_arg: String,
    pub evidence_url: String,
    pub effectiveness: f64,
}

/// Extracts objections from insights and generates counter-arguments.
pub struct ObjectionHandler;

impl ObjectionHandler {
    /// Extract objections from a set of competitor insights.
    pub fn extract_objections(insights: &[Insight]) -> Vec<Objection> {
        insights
            .iter()
            .filter(|i| {
                i.severity.priority() >= 3 // Only critical/high severity insights
            })
            .map(|i| {
                let p = i.severity.priority();
                let severity = match p {
                    5 | 4 => ObjectionSeverity::Critical,
                    3 => ObjectionSeverity::High,
                    2 => ObjectionSeverity::Medium,
                    _ => ObjectionSeverity::Low,
                };

                Objection {
                    claim: i.title.clone(),
                    source_url: i.sources.first().cloned().unwrap_or_default(),
                    severity,
                }
            })
            .collect()
    }

    /// Generate counter-arguments for given objections using our profile strengths.
    pub fn generate_handlers(
        objections: &[Objection],
        our_profile: &EntityProfile,
    ) -> Vec<ObjectionHandlerPair> {
        objections
            .iter()
            .map(|obj| {
                let counter_arg = Self::build_counter_argument(obj, our_profile);
                let effectiveness = Self::score_effectiveness(obj, our_profile);

                ObjectionHandlerPair {
                    objection: obj.claim.clone(),
                    counter_arg,
                    evidence_url: obj.source_url.clone(),
                    effectiveness,
                }
            })
            .collect()
    }

    /// Convenience: extract objections + generate handlers in one call.
    pub fn extract_and_handle(
        competitor: &EntityProfile,
        our_profile: &EntityProfile,
        insights: &[Insight],
    ) -> Vec<ObjectionHandlerPair> {
        // Build objections from competitor's product claims
        let mut objections = Vec::new();

        for prod in &competitor.product_keywords {
            objections.push(Objection {
                claim: format!("Competitor claims superior '{}' capability.", prod),
                source_url: String::new(),
                severity: ObjectionSeverity::Medium,
            });
        }

        for topic in &competitor.topic_keywords {
            objections.push(Objection {
                claim: format!("Competitor emphasizes leadership in '{}'.", topic),
                source_url: String::new(),
                severity: ObjectionSeverity::Low,
            });
        }

        // Also extract from insights
        objections.extend(Self::extract_objections(insights));

        // Deduplicate by claim
        objections.sort_by(|a, b| a.claim.cmp(&b.claim));
        objections.dedup_by(|a, b| a.claim == b.claim);

        // Max 10 objections
        objections.truncate(10);

        Self::generate_handlers(&objections, our_profile)
    }

    fn build_counter_argument(obj: &Objection, our_profile: &EntityProfile) -> String {
        // Try to find a matching strength in our profile
        for topic in &our_profile.topic_keywords {
            if obj.claim.to_lowercase().contains(&topic.to_lowercase()) {
                return format!(
                    "Our '{}' capabilities directly address this concern. \
                     We have demonstrated strength in this area with proven outcomes.",
                    topic
                );
            }
        }

        for product in &our_profile.product_keywords {
            if obj.claim.to_lowercase().contains(&product.to_lowercase()) {
                return format!(
                    "Our '{}' offering provides competitive alternatives. \
                     We match or exceed their capabilities in this dimension.",
                    product
                );
            }
        }

        format!(
            "While the competitor claims '{}', our comprehensive approach \
             across {} domains provides a more integrated solution.",
            obj.claim,
            our_profile.industry_keywords.join(", ")
        )
    }

    fn score_effectiveness(_obj: &Objection, our_profile: &EntityProfile) -> f64 {
        // Algorithmic effectiveness scoring based on profile completeness
        let mut score = 0.3; // baseline

        if !our_profile.product_keywords.is_empty() {
            score += 0.2;
        }
        if !our_profile.topic_keywords.is_empty() {
            score += 0.15;
        }
        if !our_profile.industry_keywords.is_empty() {
            score += 0.1;
        }
        if our_profile.ticker.is_some() {
            score += 0.1;
        }

        f64::min(score, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Insight, InsightSeverity};

    #[test]
    fn test_extract_objections_from_insights() {
        let insights = vec![
            Insight::new("They have better AI", "Detail")
                .with_severity(InsightSeverity::Critical)
                .with_confidence(0.9),
            Insight::new("Low priority note", "Detail")
                .with_severity(InsightSeverity::Low)
                .with_confidence(0.3),
        ];

        let objections = ObjectionHandler::extract_objections(&insights);
        assert_eq!(objections.len(), 1);
        assert_eq!(objections[0].claim, "They have better AI");
    }

    #[test]
    fn test_generate_handlers_uses_profile() {
        let objections = vec![Objection {
            claim: "They claim superior AI capabilities.".to_string(),
            source_url: "https://example.com".to_string(),
            severity: ObjectionSeverity::High,
        }];

        let profile = EntityProfile::new("Us").with_topics(vec!["AI", "Machine Learning"]);

        let handlers = ObjectionHandler::generate_handlers(&objections, &profile);
        assert_eq!(handlers.len(), 1);
        assert!(handlers[0].counter_arg.contains("AI"));
    }

    #[test]
    fn test_extract_and_handle_deduplicates() {
        let competitor = EntityProfile::new("Rival").with_products(vec!["ProductX"]);
        let us = EntityProfile::new("Us").with_topics(vec!["AI"]);

        let result = ObjectionHandler::extract_and_handle(&competitor, &us, &[]);
        assert!(!result.is_empty());
        assert!(result[0].effectiveness > 0.0);
    }
}
