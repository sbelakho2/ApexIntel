//! Battlecard route definitions — path constants, query types, and request bodies.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ─── Path Constants ────────────────────────────────────────────────────────

pub const BATTLECARDS: &str = "/api/battlecards";
pub const BATTLECARD_DETAIL: &str = "/api/battlecards/:id";
pub const BATTLECARD_REGENERATE: &str = "/api/battlecards/:id/regenerate";
pub const BATTLECARD_EXPORT: &str = "/api/battlecards/:id/export";

// ─── Query Types ───────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ListBattlecardsQuery {
    pub status: Option<String>,
    pub competitor_id: Option<Uuid>,
    pub page: Option<u32>,
    pub per_page: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct CreateBattlecardBody {
    pub our_company_id: Uuid,
    pub competitor_id: Uuid,
    pub title: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateSectionBody {
    pub section: String,
    pub data: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub struct ExportQuery {
    pub format: String, // "markdown" | "slack"
}

// ─── Response Types ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct BattlecardResponse {
    pub id: Uuid,
    pub our_company_id: Uuid,
    pub competitor_id: Uuid,
    pub title: String,
    pub status: String,
    pub sections: std::collections::HashMap<String, Option<serde_json::Value>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub regenerated_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl From<apex_store::postgres::BattlecardRow> for BattlecardResponse {
    fn from(row: apex_store::postgres::BattlecardRow) -> Self {
        let mut sections = std::collections::HashMap::new();
        sections.insert("positioning".to_string(), row.positioning);
        sections.insert("pricing".to_string(), row.pricing);
        sections.insert("feature_matrix".to_string(), row.feature_matrix);
        sections.insert("strengths".to_string(), row.strengths);
        sections.insert("weaknesses".to_string(), row.weaknesses);
        sections.insert("objection_handlers".to_string(), row.objection_handlers);
        sections.insert("kill_shots".to_string(), row.kill_shots);
        sections.insert("recent_news".to_string(), row.recent_news);
        sections.insert("win_loss".to_string(), row.win_loss);

        Self {
            id: row.id,
            our_company_id: row.our_company_id,
            competitor_id: row.competitor_id,
            title: row.title,
            status: row.status,
            sections,
            created_at: row.created_at,
            updated_at: row.updated_at,
            regenerated_at: row.regenerated_at,
        }
    }
}
