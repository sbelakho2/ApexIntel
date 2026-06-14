//! LLM-driven / algorithmic synthesis of battlecard sections.
//!
//! The primary path is **algorithmic** (rule-based from entity data).
//! LLM enhancement is available as a feature-gated option.

use crate::battlecards::kill_shot::{KillShot, KillShotAnalyzer};
use crate::battlecards::objection_handler::{ObjectionHandler, ObjectionHandlerPair};
use crate::battlecards::{
    FeatureCategory, FeatureComparisonData, FeatureMatrixSection, NewsItem, PositioningSection,
    PricingSection, StrengthItem, WeaknessItem, WinLossSection,
};
use crate::entity_relevance::EntityProfile;
use crate::Insight;

/// Synthesizes structured battlecard sections from competitor intelligence.
pub struct BattlecardGenerator;

impl BattlecardGenerator {
    pub fn new() -> Self {
        Self
    }

    /// Generate the positioning section.
    pub fn generate_positioning(
        &self,
        competitor: &EntityProfile,
        our_company: &EntityProfile,
    ) -> PositioningSection {
        let market_position = self::derive_market_position(competitor);
        let value_proposition = self::derive_value_proposition(competitor, our_company);
        let differentiators = self::extract_differentiators(competitor, our_company);
        let target_segments = competitor
            .geographic_keywords
            .iter()
            .chain(competitor.industry_keywords.iter())
            .take(5)
            .cloned()
            .collect::<Vec<_>>();

        PositioningSection {
            market_position,
            value_proposition,
            differentiators,
            target_segments,
            brand_perception: competitor
                .topic_keywords
                .first()
                .cloned()
                .unwrap_or_else(|| "No data".to_string()),
        }
    }

    /// Generate the pricing section.
    pub fn generate_pricing(
        &self,
        _competitor: &EntityProfile,
        _our_company: &EntityProfile,
    ) -> PricingSection {
        // Algorithmic pricing — in production this would consult a pricing DB
        PricingSection {
            pricing_model: "Unknown — no pricing intelligence collected".to_string(),
            price_range_low: 0.0,
            price_range_high: 0.0,
            average_contract_value: None,
            discounting_behavior: "Unknown".to_string(),
            competitive_position: "Unknown".to_string(),
        }
    }

    /// Build the feature comparison matrix.
    pub fn generate_feature_matrix(
        &self,
        competitor: &EntityProfile,
        our_company: &EntityProfile,
    ) -> FeatureMatrixSection {
        let all_keywords: Vec<&str> = competitor
            .product_keywords
            .iter()
            .chain(competitor.topic_keywords.iter())
            .map(String::as_str)
            .collect();

        let our_keywords: std::collections::HashSet<&str> = our_company
            .product_keywords
            .iter()
            .chain(our_company.topic_keywords.iter())
            .map(String::as_str)
            .collect();

        let mut features = Vec::new();
        for kw in all_keywords {
            let competitor_support = if competitor.product_keywords.contains(&kw.to_string()) {
                "Supported"
            } else {
                "Partial"
            };
            let our_support = if our_keywords.contains(kw) {
                "Supported"
            } else {
                "Not Supported"
            };
            let advantage = match (our_support, competitor_support) {
                ("Supported", "Not Supported") => "Us",
                ("Not Supported", "Supported") => "Them",
                (a, b) if a == b => "Tie",
                _ => "Unknown",
            };
            features.push(FeatureComparisonData {
                feature_name: kw.to_string(),
                our_support: our_support.to_string(),
                competitor_support: competitor_support.to_string(),
                advantage: advantage.to_string(),
            });
        }

        let summary = if features.is_empty() {
            "No feature data available for comparison.".to_string()
        } else {
            let us_count = features.iter().filter(|f| f.advantage == "Us").count();
            let them_count = features.iter().filter(|f| f.advantage == "Them").count();
            let tie_count = features.iter().filter(|f| f.advantage == "Tie").count();
            format!(
                "Compared {} features: {} advantage us, {} advantage them, {} tied.",
                features.len(),
                us_count,
                them_count,
                tie_count
            )
        };

        FeatureMatrixSection {
            categories: vec![FeatureCategory {
                category_name: "Product Capabilities".to_string(),
                features,
            }],
            summary,
        }
    }

    /// Extract strengths from entity profile.
    pub fn generate_strengths(
        &self,
        _competitor: &EntityProfile,
        our_company: &EntityProfile,
    ) -> Vec<StrengthItem> {
        let mut strengths = Vec::new();

        if !our_company.industry_keywords.is_empty() {
            strengths.push(StrengthItem {
                title: "Industry Coverage".to_string(),
                description: format!(
                    "Strong positioning across {} industry segments.",
                    our_company.industry_keywords.len()
                ),
                impact_area: "Market Presence".to_string(),
                evidence_url: String::new(),
            });
        }

        if !our_company.product_keywords.is_empty() {
            strengths.push(StrengthItem {
                title: "Product Portfolio".to_string(),
                description: format!(
                    "Offers {} distinct product capabilities.",
                    our_company.product_keywords.len()
                ),
                impact_area: "Product".to_string(),
                evidence_url: String::new(),
            });
        }

        if !our_company.geographic_keywords.is_empty() {
            strengths.push(StrengthItem {
                title: "Geographic Reach".to_string(),
                description: format!(
                    "Operational presence across {} regions.",
                    our_company.geographic_keywords.len()
                ),
                impact_area: "Operations".to_string(),
                evidence_url: String::new(),
            });
        }

        strengths
    }

    /// Extract weaknesses relative to competitor.
    pub fn generate_weaknesses(
        &self,
        competitor: &EntityProfile,
        our_company: &EntityProfile,
    ) -> Vec<WeaknessItem> {
        let mut weaknesses = Vec::new();

        // Identify areas where competitor has product keywords we don't
        let our_products: std::collections::HashSet<&str> =
            our_company.product_keywords.iter().map(String::as_str).collect();
        for kw in &competitor.product_keywords {
            if !our_products.contains(kw.as_str()) {
                weaknesses.push(WeaknessItem {
                    title: format!("Missing capability: {}", kw),
                    description: format!(
                        "Competitor offers '{}' capability which we do not currently support.",
                        kw
                    ),
                    impact_area: "Product Gap".to_string(),
                    severity: 0.6,
                });
            }
        }

        // Limit to 5 most impactful
        weaknesses.truncate(5);

        if weaknesses.is_empty() {
            weaknesses.push(WeaknessItem {
                title: "Insufficient Data".to_string(),
                description: "Not enough competitive intelligence to identify weaknesses."
                    .to_string(),
                impact_area: "Intelligence Gap".to_string(),
                severity: 0.3,
            });
        }

        weaknesses
    }

    /// Generate objection handlers from competitor claims.
    pub fn generate_objection_handlers(
        &self,
        competitor: &EntityProfile,
        our_company: &EntityProfile,
        insights: &[Insight],
    ) -> Vec<ObjectionHandlerPair> {
        ObjectionHandler::extract_and_handle(competitor, our_company, insights)
    }

    /// Generate kill shots from competitor weaknesses.
    pub fn generate_kill_shots(
        &self,
        competitor: &EntityProfile,
        our_company: &EntityProfile,
    ) -> Vec<KillShot> {
        KillShotAnalyzer::identify_from_profiles(competitor, our_company)
    }

    /// Cluster recent insights into news items.
    pub fn generate_recent_news(&self, insights: &[Insight]) -> Vec<NewsItem> {
        insights
            .iter()
            .take(10)
            .map(|i| NewsItem {
                title: i.title.clone(),
                url: i.sources.first().cloned().unwrap_or_default(),
                published_at: None,
                relevance_score: i.confidence,
            })
            .collect()
    }

    /// Generate win/loss analysis.
    pub fn generate_win_loss(
        &self,
        _competitor: &EntityProfile,
        _our_company: &EntityProfile,
    ) -> WinLossSection {
        // Algorithmic default; real data comes from CRM integration
        WinLossSection::default()
    }
}

impl Default for BattlecardGenerator {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Helper Functions ──────────────────────────────────────────────────────

fn derive_market_position(competitor: &EntityProfile) -> String {
    if competitor.industry_keywords.is_empty() {
        return "Unknown market position — insufficient data.".to_string();
    }
    format!(
        "Competes primarily in {} with focus on {}.",
        competitor.industry_keywords.join(", "),
        competitor.topic_keywords.first().map(String::as_str).unwrap_or("general")
    )
}

fn derive_value_proposition(
    competitor: &EntityProfile,
    our_company: &EntityProfile,
) -> String {
    let our_products = our_company.product_keywords.join(", ");
    let their_products = competitor.product_keypoints().join(", ");

    if our_products.is_empty() && their_products.is_empty() {
        return "Value proposition analysis requires product data.".to_string();
    }

    format!(
        "We offer {} versus their {}. Our differentiators include {}.",
        if our_products.is_empty() { "comparable solutions" } else { &our_products },
        if their_products.is_empty() { "similar offerings" } else { &their_products },
        our_company
            .topic_keywords
            .first()
            .cloned()
            .unwrap_or_else(|| "specialized expertise".to_string())
    )
}

fn extract_differentiators(
    competitor: &EntityProfile,
    our_company: &EntityProfile,
) -> Vec<String> {
    let our_set: std::collections::HashSet<&str> =
        our_company.topic_keywords.iter().map(String::as_str).collect();
    let their_set: std::collections::HashSet<&str> =
        competitor.topic_keywords.iter().map(String::as_str).collect();

    let mut diff = Vec::new();
    for kw in our_set.difference(&their_set) {
        diff.push(format!("Unique strength in {}", kw));
    }
    if diff.is_empty() {
        diff.push("No clear differentiators identified from available data.".to_string());
    }
    diff.truncate(5);
    diff
}

// Extension to get product keywords as a list for display
trait EntityProfileExt {
    fn product_keypoints(&self) -> Vec<String>;
}

impl EntityProfileExt for EntityProfile {
    fn product_keypoints(&self) -> Vec<String> {
        self.product_keywords
            .iter()
            .take(5)
            .cloned()
            .collect()
    }
}
