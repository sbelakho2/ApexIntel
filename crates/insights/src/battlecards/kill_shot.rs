//! Kill Shot Analyzer — identifies high-confidence competitor weaknesses
//! that can be exploited in competitive deals.

use serde::{Deserialize, Serialize};

use crate::entity_relevance::EntityProfile;

/// A high-confidence weakness / "kill shot" against a competitor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KillShot {
    pub title: String,
    pub description: String,
    pub evidence_url: String,
    pub confidence: f64,
    pub severity: f64,
    pub market_relevance: f64,
    pub priority_score: f64,
}

impl KillShot {
    fn calculate_priority(&self) -> f64 {
        self.confidence * self.severity * self.market_relevance
    }
}

/// Identifies kill shots (high-confidence weaknesses) from competitor data.
pub struct KillShotAnalyzer;

impl KillShotAnalyzer {
    /// Identify kill shots from a full competitive threat assessment.
    pub fn identify(
        competitor_name: &str,
        competitor_topics: &[String],
        competitor_products: &[String],
        competitor_industries: &[String],
        our_topics: &[String],
        our_products: &[String],
    ) -> Vec<KillShot> {
        let mut kill_shots = Vec::new();

        // Check for topics the competitor doesn't cover
        let our_topic_set: std::collections::HashSet<&str> =
            our_topics.iter().map(String::as_str).collect();
        let their_topic_set: std::collections::HashSet<&str> =
            competitor_topics.iter().map(String::as_str).collect();

        for topic in our_topic_set.difference(&their_topic_set) {
            let relevance = if competitor_industries
                .iter()
                .any(|i| i.to_lowercase().contains(&topic.to_lowercase()))
            {
                0.8
            } else {
                0.5
            };

            kill_shots.push(KillShot {
                title: format!("Lack of '{}' capability", topic),
                description: format!(
                    "{} has no demonstrated capability or emphasis on '{}', \
                     which is a key requirement in this market.",
                    competitor_name, topic
                ),
                evidence_url: String::new(),
                confidence: 0.75,
                severity: 0.7,
                market_relevance: relevance,
                priority_score: 0.0, // calculated below
            });
        }

        // Check for product gaps
        let our_product_set: std::collections::HashSet<&str> =
            our_products.iter().map(String::as_str).collect();
        for prod in our_product_set.iter() {
            if !competitor_products.contains(&prod.to_string()) {
                kill_shots.push(KillShot {
                    title: format!("No '{}' offering", prod),
                    description: format!(
                        "{} does not offer '{}', which is a differentiating capability we provide.",
                        competitor_name, prod
                    ),
                    evidence_url: String::new(),
                    confidence: 0.85,
                    severity: 0.6,
                    market_relevance: 0.7,
                    priority_score: 0.0,
                });
            }
        }

        // Calculate priority scores
        for ks in &mut kill_shots {
            ks.priority_score = ks.calculate_priority();
        }

        // Sort by priority descending, take top 5
        kill_shots.sort_by(|a, b| {
            b.priority_score
                .partial_cmp(&a.priority_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        kill_shots.truncate(5);

        kill_shots
    }

    /// Convenience: identify kill shots from EntityProfile objects.
    pub fn identify_from_profiles(
        competitor: &EntityProfile,
        our_company: &EntityProfile,
    ) -> Vec<KillShot> {
        Self::identify(
            &competitor.entity_name,
            &competitor.topic_keywords,
            &competitor.product_keywords,
            &competitor.industry_keywords,
            &our_company.topic_keywords,
            &our_company.product_keywords,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity_relevance::EntityProfile;

    #[test]
    fn test_identify_finds_gaps() {
        let kill_shots = KillShotAnalyzer::identify(
            "RivalCorp",
            &["AI".to_string(), "Cloud".to_string()],
            &["ProductA".to_string()],
            &["Tech".to_string()],
            &["AI".to_string(), "Edge".to_string(), "Security".to_string()],
            &["ProductA".to_string(), "ProductB".to_string()],
        );

        assert!(
            !kill_shots.is_empty(),
            "should identify at least one kill shot"
        );

        // Should identify Edge and Security as gaps
        let edge_shot = kill_shots.iter().find(|k| k.title.contains("Edge"));
        assert!(edge_shot.is_some(), "should find Edge gap");

        let security_shot = kill_shots.iter().find(|k| k.title.contains("Security"));
        assert!(security_shot.is_some(), "should find Security gap");

        // ProductB gap
        let prod_b_shot = kill_shots.iter().find(|k| k.title.contains("ProductB"));
        assert!(prod_b_shot.is_some(), "should find ProductB gap");
    }

    #[test]
    fn test_priority_score_calculation() {
        let kill_shots =
            KillShotAnalyzer::identify("Rival", &[], &[], &[], &["UniqueTech".to_string()], &[]);

        assert!(!kill_shots.is_empty());
        for ks in &kill_shots {
            let expected = ks.confidence * ks.severity * ks.market_relevance;
            assert!(
                (ks.priority_score - expected).abs() < 0.001,
                "priority_score should equal confidence * severity * market_relevance"
            );
        }
    }

    #[test]
    fn test_identify_from_profiles() {
        let competitor = EntityProfile::new("Rival")
            .with_topics(vec!["AI", "Cloud"])
            .with_products(vec!["ProductA"]);

        let us = EntityProfile::new("Us")
            .with_topics(vec!["AI", "Edge", "Security"])
            .with_products(vec!["ProductA", "ProductB"]);

        let kill_shots = KillShotAnalyzer::identify_from_profiles(&competitor, &us);
        assert!(!kill_shots.is_empty());
    }
}
