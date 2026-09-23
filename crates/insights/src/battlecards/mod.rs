//! Battlecard Workflow Engine
//!
//! Generates competitive battlecards: structured intelligence summaries comparing
//! "our company" against a specific competitor across multiple dimensions.
//!
//! ## Architecture
//!
//! | Module | Responsibility |
//! |--------|---------------|
//! | `generator` | LLM-driven / algorithmic synthesis of all sections |
//! | `feature_matrix` | Side-by-side feature comparison builder |
//! | `objection_handler` | Objection extraction and counter-argument generation |
//! | `kill_shot` | High-confidence weakness identification |
//! | `win_loss_analyzer` | Win/loss rate analysis against a competitor |
//! | `distribution` | Export to Markdown, Slack Block Kit, or PDF HTML |

pub mod distribution;
pub mod feature_matrix;
pub mod generator;
pub mod kill_shot;
pub mod llm_sections;
pub mod objection_handler;
pub mod win_loss_analyzer;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::entity_relevance::EntityProfile;
use crate::Insight;

// ─── Re-exports ────────────────────────────────────────────────────────────

pub use distribution::BattlecardDistributor;
pub use feature_matrix::{Advantage, FeatureComparison, FeatureMatrixBuilder, FeatureSupport};
pub use generator::{BattlecardContext, BattlecardGenerator, PricingObservation};
pub use kill_shot::{KillShot, KillShotAnalyzer};
pub use objection_handler::{Objection, ObjectionHandler, ObjectionHandlerPair, ObjectionSeverity};
pub use win_loss_analyzer::{
    ClosedDeal, LossReason, WinLossAnalysis, WinLossAnalyzer, WinLossTrend,
};

/// The top-level battlecard engine: orchestrates generation of all sections.
pub struct BattlecardEngine {
    generator: BattlecardGenerator,
    feature_matrix: FeatureMatrixBuilder,
    objection_handler: ObjectionHandler,
    kill_shot: KillShotAnalyzer,
    win_loss: WinLossAnalyzer,
}

impl BattlecardEngine {
    /// Create a new battlecard engine.
    pub fn new() -> Self {
        Self {
            generator: BattlecardGenerator::new(),
            feature_matrix: FeatureMatrixBuilder,
            objection_handler: ObjectionHandler,
            kill_shot: KillShotAnalyzer,
            win_loss: WinLossAnalyzer,
        }
    }

    /// Generate a complete battlecard from entity profiles, recent insights,
    /// and a real-data context (closed deals + competitor pricing).
    pub async fn generate_full_battlecard(
        &self,
        competitor: &EntityProfile,
        our_company: &EntityProfile,
        recent_insights: &[Insight],
        ctx: &BattlecardContext,
        our_company_id: Uuid,
        competitor_id: Uuid,
    ) -> BattlecardData {
        let positioning = self.generator.generate_positioning(competitor, our_company);
        let pricing = self
            .generator
            .generate_pricing(competitor, our_company, ctx);
        let feature_matrix = self
            .generator
            .generate_feature_matrix(competitor, our_company);
        let strengths = self.generator.generate_strengths(competitor, our_company);
        let weaknesses = self.generator.generate_weaknesses(competitor, our_company);
        let objection_handlers =
            self.generator
                .generate_objection_handlers(competitor, our_company, recent_insights);
        let kill_shots = self.generator.generate_kill_shots(competitor, our_company);
        let recent_news = self.generator.generate_recent_news(recent_insights);
        let win_loss = self.generator.generate_win_loss(
            competitor,
            our_company,
            ctx,
            our_company_id,
            competitor_id,
        );

        BattlecardData {
            positioning,
            pricing,
            feature_matrix,
            strengths,
            weaknesses,
            objection_handlers,
            kill_shots,
            recent_news,
            win_loss,
        }
    }

    /// Regenerate a single section by name, optionally using existing data.
    pub fn regenerate_section(
        &self,
        section: &str,
        competitor: &EntityProfile,
        our_company: &EntityProfile,
        recent_insights: &[Insight],
        ctx: &BattlecardContext,
        our_company_id: Uuid,
        competitor_id: Uuid,
        _existing: Option<&serde_json::Value>,
    ) -> Result<serde_json::Value, String> {
        match section {
            "positioning" => {
                let data = self.generator.generate_positioning(competitor, our_company);
                serde_json::to_value(data).map_err(|e| e.to_string())
            }
            "pricing" => {
                let data = self
                    .generator
                    .generate_pricing(competitor, our_company, ctx);
                serde_json::to_value(data).map_err(|e| e.to_string())
            }
            "feature_matrix" => {
                let data = self
                    .generator
                    .generate_feature_matrix(competitor, our_company);
                serde_json::to_value(data).map_err(|e| e.to_string())
            }
            "strengths" => {
                let data = self.generator.generate_strengths(competitor, our_company);
                serde_json::to_value(data).map_err(|e| e.to_string())
            }
            "weaknesses" => {
                let data = self.generator.generate_weaknesses(competitor, our_company);
                serde_json::to_value(data).map_err(|e| e.to_string())
            }
            "objection_handlers" => {
                let data = self.generator.generate_objection_handlers(
                    competitor,
                    our_company,
                    recent_insights,
                );
                serde_json::to_value(data).map_err(|e| e.to_string())
            }
            "kill_shots" => {
                let data = self.generator.generate_kill_shots(competitor, our_company);
                serde_json::to_value(data).map_err(|e| e.to_string())
            }
            "recent_news" => {
                let data = self.generator.generate_recent_news(recent_insights);
                serde_json::to_value(data).map_err(|e| e.to_string())
            }
            "win_loss" => {
                let data = self.generator.generate_win_loss(
                    competitor,
                    our_company,
                    ctx,
                    our_company_id,
                    competitor_id,
                );
                serde_json::to_value(data).map_err(|e| e.to_string())
            }
            _ => Err(format!("unknown section: {}", section)),
        }
    }
}

impl Default for BattlecardEngine {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Data Types ────────────────────────────────────────────────────────────

/// Complete battlecard data, mirroring the JSONB schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BattlecardData {
    pub positioning: PositioningSection,
    pub pricing: PricingSection,
    pub feature_matrix: FeatureMatrixSection,
    pub strengths: Vec<StrengthItem>,
    pub weaknesses: Vec<WeaknessItem>,
    pub objection_handlers: Vec<ObjectionHandlerPair>,
    pub kill_shots: Vec<KillShot>,
    pub recent_news: Vec<NewsItem>,
    pub win_loss: WinLossSection,
}

// ─── Positioning ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PositioningSection {
    pub market_position: String,
    pub value_proposition: String,
    pub differentiators: Vec<String>,
    pub target_segments: Vec<String>,
    pub brand_perception: String,
}

// ─── Pricing ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PricingSection {
    pub pricing_model: String,
    pub price_range_low: f64,
    pub price_range_high: f64,
    pub average_contract_value: Option<f64>,
    pub discounting_behavior: String,
    pub competitive_position: String,
}

impl Default for PricingSection {
    fn default() -> Self {
        Self {
            pricing_model: String::new(),
            price_range_low: 0.0,
            price_range_high: 0.0,
            average_contract_value: None,
            discounting_behavior: String::new(),
            competitive_position: String::new(),
        }
    }
}

// ─── Feature Matrix ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FeatureMatrixSection {
    pub categories: Vec<FeatureCategory>,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureCategory {
    pub category_name: String,
    pub features: Vec<FeatureComparisonData>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureComparisonData {
    pub feature_name: String,
    pub our_support: String,
    pub competitor_support: String,
    pub advantage: String,
}

// ─── Strengths & Weaknesses ───────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrengthItem {
    pub title: String,
    pub description: String,
    pub impact_area: String,
    pub evidence_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WeaknessItem {
    pub title: String,
    pub description: String,
    pub impact_area: String,
    pub severity: f64,
}

// ─── Objections (re-exported from objection_handler) ──────────────────────
// (ObjectionHandlerPair is defined in objection_handler.rs)

// ─── Kill Shots (re-exported from kill_shot) ──────────────────────────────
// (KillShot is defined in kill_shot.rs)

// ─── News ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewsItem {
    pub title: String,
    pub url: String,
    pub published_at: Option<DateTime<Utc>>,
    pub relevance_score: f64,
}

// ─── Win/Loss ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WinLossSection {
    pub win_rate: f64,
    pub total_deals: u32,
    pub won: u32,
    pub lost: u32,
    pub total_value_won: f64,
    pub total_value_lost: f64,
    pub top_loss_reasons: Vec<LossReason>,
    pub trends: Vec<WinLossTrend>,
}

impl Default for WinLossSection {
    fn default() -> Self {
        Self {
            win_rate: 0.0,
            total_deals: 0,
            won: 0,
            lost: 0,
            total_value_won: 0.0,
            total_value_lost: 0.0,
            top_loss_reasons: Vec::new(),
            trends: Vec::new(),
        }
    }
}
