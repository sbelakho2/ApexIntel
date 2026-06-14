//! Feature comparison matrix builder.
//!
//! Extracts features from competitor/company profiles and builds
//! a side-by-side comparison with advantage determination.

use crate::entity_relevance::EntityProfile;

/// Level of support a product provides for a given feature.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum FeatureSupport {
    Supported,
    Partial,
    NotSupported,
    Unknown,
}

/// Which side has the advantage for a given feature.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Advantage {
    Us,
    Them,
    Tie,
    Unknown,
}

/// A single feature comparison row.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FeatureComparison {
    pub feature_name: String,
    pub category: String,
    pub our_support: FeatureSupport,
    pub competitor_support: FeatureSupport,
    pub advantage: Advantage,
    pub evidence_urls: Vec<String>,
}

/// Builds feature comparison matrices from entity profiles.
pub struct FeatureMatrixBuilder;

impl FeatureMatrixBuilder {
    /// Build a feature comparison matrix between a competitor and our company.
    ///
    /// Features are extracted from:
    /// - Product names (`product_keywords`)
    /// - Technology topics (`topic_keywords`)
    /// - Industry capabilities (`industry_keywords`)
    pub fn build(
        competitor: &EntityProfile,
        our_company: &EntityProfile,
    ) -> Vec<FeatureComparison> {
        let mut comparisons = Vec::new();

        // Build a set of unique feature names from both profiles
        let mut feature_names: Vec<String> = competitor
            .product_keywords
            .iter()
            .chain(competitor.topic_keywords.iter())
            .chain(competitor.industry_keywords.iter())
            .cloned()
            .collect();

        // Add our features too, deduplicating
        for f in our_company
            .product_keywords
            .iter()
            .chain(our_company.topic_keywords.iter())
            .chain(our_company.industry_keywords.iter())
        {
            if !feature_names.contains(f) {
                feature_names.push(f.clone());
            }
        }

        // Deduplicate
        feature_names.sort();
        feature_names.dedup();

        let our_set: std::collections::HashSet<&str> = our_company
            .product_keywords
            .iter()
            .chain(our_company.topic_keywords.iter())
            .chain(our_company.industry_keywords.iter())
            .map(String::as_str)
            .collect();

        let their_set: std::collections::HashSet<&str> = competitor
            .product_keywords
            .iter()
            .chain(competitor.topic_keywords.iter())
            .chain(competitor.industry_keywords.iter())
            .map(String::as_str)
            .collect();

        for name in feature_names {
            let our_support = if our_set.contains(name.as_str()) {
                FeatureSupport::Supported
            } else {
                FeatureSupport::NotSupported
            };

            let competitor_support = if their_set.contains(name.as_str()) {
                FeatureSupport::Supported
            } else {
                FeatureSupport::NotSupported
            };

            let advantage = match (&our_support, &competitor_support) {
                (FeatureSupport::Supported, FeatureSupport::NotSupported) => Advantage::Us,
                (FeatureSupport::NotSupported, FeatureSupport::Supported) => Advantage::Them,
                _ => Advantage::Tie,
            };

            let category = if competitor.product_keywords.contains(&name)
                || our_company.product_keywords.contains(&name)
            {
                "Products"
            } else if competitor.topic_keywords.contains(&name)
                || our_company.topic_keywords.contains(&name)
            {
                "Technology"
            } else {
                "Capabilities"
            };

            comparisons.push(FeatureComparison {
                feature_name: name,
                category: category.to_string(),
                our_support,
                competitor_support,
                advantage,
                evidence_urls: Vec::new(),
            });
        }

        comparisons
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity_relevance::{EntityCategory, EntityProfile};

    fn make_profile(name: &str, products: Vec<&str>, topics: Vec<&str>) -> EntityProfile {
        EntityProfile::new(name)
            .with_products(products)
            .with_topics(topics)
    }

    #[test]
    fn test_feature_matrix_builds_comparisons() {
        let competitor = make_profile("RivalCorp", vec!["ProductA", "ProductB"], vec!["AI", "Cloud"]);
        let us = make_profile("OurCorp", vec!["ProductA", "ProductC"], vec!["AI", "Edge"]);

        let comparisons = FeatureMatrixBuilder::build(&competitor, &us);

        assert!(!comparisons.is_empty(), "should produce at least one comparison");

        // ProductA is common -> Tie
        let prod_a = comparisons.iter().find(|c| c.feature_name == "ProductA").unwrap();
        assert_eq!(prod_a.advantage, Advantage::Tie);

        // ProductB is competitor-only -> Them
        let prod_b = comparisons.iter().find(|c| c.feature_name == "ProductB").unwrap();
        assert_eq!(prod_b.advantage, Advantage::Them);

        // ProductC is us-only -> Us
        let prod_c = comparisons.iter().find(|c| c.feature_name == "ProductC").unwrap();
        assert_eq!(prod_c.advantage, Advantage::Us);
    }

    #[test]
    fn test_feature_matrix_empty_profiles() {
        let competitor = EntityProfile::new("Rival");
        let us = EntityProfile::new("Us");

        let comparisons = FeatureMatrixBuilder::build(&competitor, &us);
        assert!(comparisons.is_empty());
    }
}
