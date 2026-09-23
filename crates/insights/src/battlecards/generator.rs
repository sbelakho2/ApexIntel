//! Algorithmic synthesis of battlecard sections from entity + deal/pricing data.
//!
//! The primary path is **algorithmic** (rule-based from entity profiles and
//! structured intelligence). The win/loss and pricing sections consume REAL
//! data via [`BattlecardContext`]: closed-deal outcomes from the `closed_deals`
//! table and competitor pricing from `competitor_pricing`.
//!
//! LLM-grounded narrative enrichment is provided separately in
//! [`crate::battlecards::llm_sections`], which wraps this generator and
//! synthesizes positioning / objection-handlers from evidence using the
//! anti-hallucination-gated `apex_llm` client.

use crate::battlecards::kill_shot::{KillShot, KillShotAnalyzer};
use crate::battlecards::objection_handler::{ObjectionHandler, ObjectionHandlerPair};
use crate::battlecards::{ClosedDeal, WinLossAnalyzer};
use crate::battlecards::{
    FeatureCategory, FeatureComparisonData, FeatureMatrixSection, NewsItem, PositioningSection,
    PricingSection, StrengthItem, WeaknessItem, WinLossSection,
};
use crate::entity_relevance::EntityProfile;
use crate::Insight;

use uuid::Uuid;

/// Real data the generator consumes to ground the win/loss and pricing sections.
///
/// Populated by the caller (API handler / worker) from `PgStore` before invoking
/// the generator, so these sections reflect measured outcomes rather than the
/// empty defaults that were returned before.
#[derive(Debug, Clone, Default)]
pub struct BattlecardContext {
    /// Closed deals against this competitor (won + lost) — drives win/loss.
    pub closed_deals: Vec<ClosedDeal>,
    /// Latest competitor pricing observations — drives the pricing section.
    pub pricing: Vec<PricingObservation>,
}

/// A single competitor pricing observation, mirroring `competitor_pricing` rows.
#[derive(Debug, Clone)]
pub struct PricingObservation {
    pub product_category: String,
    pub pricing_model: String,
    pub price_range_low: Option<f64>,
    pub price_range_high: Option<f64>,
    pub currency: String,
    pub average_contract_value: Option<f64>,
    pub discounting_behavior: String,
    pub competitive_position: String,
    pub confidence: f64,
    pub evidence_url: Option<String>,
}

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

    /// Generate the pricing section from REAL competitor-pricing observations.
    ///
    /// When no pricing intelligence has been collected yet, the section honestly
    /// reports that gap rather than inventing numbers. When observations exist,
    /// the lowest/highest ranges and the dominant pricing model / positioning
    /// are aggregated across product categories.
    pub fn generate_pricing(
        &self,
        competitor: &EntityProfile,
        _our_company: &EntityProfile,
        ctx: &BattlecardContext,
    ) -> PricingSection {
        if ctx.pricing.is_empty() {
            return PricingSection {
                pricing_model: "Unknown — no pricing intelligence collected".to_string(),
                price_range_low: 0.0,
                price_range_high: 0.0,
                average_contract_value: None,
                discounting_behavior: "Unknown".to_string(),
                competitive_position: "Unknown".to_string(),
            };
        }

        // Confidence-weighted aggregation across categories (same currency).
        let (low, high) = confidence_weighted_range(&ctx.pricing);
        let avg_acv = ctx
            .pricing
            .iter()
            .filter_map(|p| p.average_contract_value)
            .reduce(|acc, v| acc.max(v));

        // Dominant pricing model + positioning by observation count.
        let models: Vec<&str> = ctx
            .pricing
            .iter()
            .map(|p| p.pricing_model.as_str())
            .collect();
        let positions: Vec<&str> = ctx
            .pricing
            .iter()
            .map(|p| p.competitive_position.as_str())
            .collect();
        let discounts: Vec<&str> = ctx
            .pricing
            .iter()
            .map(|p| p.discounting_behavior.as_str())
            .collect();
        let pricing_model = most_frequent(&models).unwrap_or("mixed").to_string();
        let competitive_position = most_frequent(&positions).unwrap_or("unknown").to_string();
        let discounting_behavior = most_frequent(&discounts).unwrap_or("unknown").to_string();

        let _ = competitor; // entity retained for future capability-gap pricing logic
        PricingSection {
            pricing_model,
            price_range_low: low,
            price_range_high: high,
            average_contract_value: avg_acv,
            discounting_behavior: humanize_discounting(&discounting_behavior),
            competitive_position: humanize_position(&competitive_position),
        }
    }

    /// Build the feature comparison matrix.
    pub fn generate_feature_matrix(
        &self,
        competitor: &EntityProfile,
        our_company: &EntityProfile,
    ) -> FeatureMatrixSection {
        // B337: symmetric support levels. The previous logic derived the
        // competitor's support from the competitor's own keyword lists and
        // could never produce "Not Supported" — so the "Us" advantage arm was
        // dead code and every battlecard summary read "0 advantage us".
        let support_level = |product: &[String], topics: &[String], kw: &str| -> &'static str {
            if product.iter().any(|k| k == kw) {
                "Supported"
            } else if topics.iter().any(|k| k == kw) {
                "Partial"
            } else {
                "Not Supported"
            }
        };

        let mut all_keywords: Vec<&str> = competitor
            .product_keywords
            .iter()
            .chain(competitor.topic_keywords.iter())
            .chain(our_company.product_keywords.iter())
            .chain(our_company.topic_keywords.iter())
            .map(String::as_str)
            .collect();
        all_keywords.sort_unstable();
        all_keywords.dedup();

        let rank = |support: &str| -> u8 {
            match support {
                "Supported" => 2,
                "Partial" => 1,
                _ => 0,
            }
        };

        let mut features = Vec::new();
        for kw in all_keywords {
            let competitor_support =
                support_level(&competitor.product_keywords, &competitor.topic_keywords, kw);
            let our_support = support_level(
                &our_company.product_keywords,
                &our_company.topic_keywords,
                kw,
            );
            let advantage = match (rank(our_support), rank(competitor_support)) {
                (ours, theirs) if ours > theirs => "Us",
                (ours, theirs) if theirs > ours => "Them",
                _ => "Tie",
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
        let our_products: std::collections::HashSet<&str> = our_company
            .product_keywords
            .iter()
            .map(String::as_str)
            .collect();
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

    /// Generate win/loss analysis from REAL closed-deal outcomes.
    ///
    /// Replaces the previous stub that returned `WinLossSection::default()`.
    /// When no deal history exists yet, the section is empty-but-honest (0
    /// deals) rather than fabricated. The `our_company_id` / `competitor_id`
    /// are passed through to the analyzer for provenance only (the math is
    /// driven entirely by the deal records in `ctx`).
    pub fn generate_win_loss(
        &self,
        _competitor: &EntityProfile,
        _our_company: &EntityProfile,
        ctx: &BattlecardContext,
        our_company_id: Uuid,
        competitor_id: Uuid,
    ) -> WinLossSection {
        if ctx.closed_deals.is_empty() {
            return WinLossSection::default();
        }
        let analysis = WinLossAnalyzer::analyze(our_company_id, competitor_id, &ctx.closed_deals);
        WinLossSection {
            win_rate: analysis.win_rate,
            total_deals: analysis.total_deals,
            won: analysis.won,
            lost: analysis.lost,
            total_value_won: analysis.total_value_won,
            total_value_lost: analysis.total_value_lost,
            top_loss_reasons: analysis.top_loss_reasons,
            trends: analysis.trends,
        }
    }
}

// ─── pricing aggregation helpers ───────────────────────────────────────────

/// Confidence-weighted min/max price range across observations.
fn confidence_weighted_range(pricing: &[PricingObservation]) -> (f64, f64) {
    let lows: Vec<(f64, f64)> = pricing
        .iter()
        .filter_map(|p| p.price_range_low.map(|v| (v, p.confidence)))
        .collect();
    let highs: Vec<(f64, f64)> = pricing
        .iter()
        .filter_map(|p| p.price_range_high.map(|v| (v, p.confidence)))
        .collect();
    let low = weighted_min(&lows);
    let high = weighted_max(&highs);
    (low, high)
}

fn weighted_min(values: &[(f64, f64)]) -> f64 {
    // Prefer the lowest value weighted by confidence (favor high-confidence lows).
    values
        .iter()
        .min_by(|a, b| {
            a.0.partial_cmp(&b.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal))
        })
        .map(|(v, _)| *v)
        .unwrap_or(0.0)
}

fn weighted_max(values: &[(f64, f64)]) -> f64 {
    values
        .iter()
        .max_by(|a, b| {
            a.0.partial_cmp(&b.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        })
        .map(|(v, _)| *v)
        .unwrap_or(0.0)
}

fn most_frequent<'a>(items: &[&'a str]) -> Option<&'a str> {
    let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for it in items {
        *counts.entry(it).or_insert(0) += 1;
    }
    counts.into_iter().max_by_key(|(_, c)| *c).map(|(k, _)| k)
}

fn humanize_discounting(raw: &str) -> String {
    match raw {
        "aggressive" => "Aggressive — heavy discounting to win deals".to_string(),
        "moderate" => "Moderate — standard volume discounts".to_string(),
        "conservative" => "Conservative — holds price / limited discounting".to_string(),
        other => other.to_string(),
    }
}

fn humanize_position(raw: &str) -> String {
    match raw {
        "premium" => "Premium — prices above market".to_string(),
        "value" => "Value — market-average pricing".to_string(),
        "low_cost" => "Low-cost — undercuts market".to_string(),
        other => other.to_string(),
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
        competitor
            .topic_keywords
            .first()
            .map(String::as_str)
            .unwrap_or("general")
    )
}

fn derive_value_proposition(competitor: &EntityProfile, our_company: &EntityProfile) -> String {
    let our_products = our_company.product_keywords.join(", ");
    let their_products = competitor.product_keypoints().join(", ");

    if our_products.is_empty() && their_products.is_empty() {
        return "Value proposition analysis requires product data.".to_string();
    }

    format!(
        "We offer {} versus their {}. Our differentiators include {}.",
        if our_products.is_empty() {
            "comparable solutions"
        } else {
            &our_products
        },
        if their_products.is_empty() {
            "similar offerings"
        } else {
            &their_products
        },
        our_company
            .topic_keywords
            .first()
            .cloned()
            .unwrap_or_else(|| "specialized expertise".to_string())
    )
}

fn extract_differentiators(competitor: &EntityProfile, our_company: &EntityProfile) -> Vec<String> {
    let our_set: std::collections::HashSet<&str> = our_company
        .topic_keywords
        .iter()
        .map(String::as_str)
        .collect();
    let their_set: std::collections::HashSet<&str> = competitor
        .topic_keywords
        .iter()
        .map(String::as_str)
        .collect();

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
        self.product_keywords.iter().take(5).cloned().collect()
    }
}
