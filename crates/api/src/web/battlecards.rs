//! Battlecards web handler — GET /battlecards (list), GET /battlecards/:id (detail)
//!
//! Server-rendered pages via Askama + HTMX following the pattern in `competitors.rs`.

use std::sync::Arc;

use askama::Template;
use axum::{
    extract::{Path, Query},
    http::HeaderMap,
    response::IntoResponse,
    Extension,
};
use serde::Deserialize;
use uuid::Uuid;

use super::{is_htmx_request, PageContext};
use crate::middleware::session::WebSession;
use crate::routes::battlecards::BattlecardResponse;
use apex_store::postgres::PgStore;

// ─── Query ──────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct BattlecardsQuery {
    pub status: Option<String>,
    pub page: Option<u32>,
    pub per_page: Option<u32>,
}

// ─── Template data types ────────────────────────────────────────────────────

/// Lightweight card shown in the battlecard list.
#[derive(Clone, Debug)]
pub struct BattlecardListItem {
    pub id: String,
    pub title: String,
    pub status: String,
    pub competitor_name: String,
    pub updated_at: String,
    pub section_count: usize,
}

/// One section row in the detail view.
#[derive(Clone, Debug)]
pub struct BattlecardSectionRow {
    pub name: String,
    pub label: String,
    pub has_data: bool,
    pub preview: String,
}

// ─── Templates ──────────────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "pages/battlecards/list.html")]
pub struct BattlecardsListPage {
    pub current_path: String,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,

    pub battlecards: Vec<BattlecardListItem>,
    pub total: i64,
    pub page: i64,
    pub per_page: i64,
    pub active_status: String,
}

#[derive(Template)]
#[template(path = "pages/battlecards/detail.html")]
pub struct BattlecardDetailPage {
    pub current_path: String,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,

    pub battlecard: BattlecardResponse,
    pub sections: Vec<BattlecardSectionRow>,
    pub competitor_name: String,
}

// ─── Handlers ───────────────────────────────────────────────────────────────

/// GET /battlecards — list page
pub async fn list_battlecards(
    Extension(store): Extension<Arc<PgStore>>,
    Query(query): Query<BattlecardsQuery>,
    Extension(session): Extension<WebSession>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let warning_count = store
        .count_warnings(&apex_store::postgres::WarningListFilters {
            acknowledged: Some(false),
            ..Default::default()
        })
        .await
        .unwrap_or(0);

    let _ctx = PageContext::from_session(&session, "/battlecards", warning_count);

    let page_u32 = query.page.unwrap_or(1).max(1);
    let per_page_u32 = query.per_page.unwrap_or(50).clamp(1, 100);
    let active_status = query.status.clone().unwrap_or_default();

    let total = store
        .count_battlecards(query.status.as_deref(), None)
        .await
        .unwrap_or(0) as i64;

    let rows = store
        .list_battlecards(query.status.as_deref(), None, page_u32, per_page_u32)
        .await
        .unwrap_or_default();

    let page = page_u32 as i64;
    let per_page = per_page_u32 as i64;

    // Collect all competitor_ids to batch-fetch company names
    let competitor_ids: Vec<Uuid> = rows.iter().map(|r| r.competitor_id).collect();
    let company_names: std::collections::HashMap<Uuid, String> =
        match store.get_company_names_by_ids(&competitor_ids).await {
            Ok(rows) => rows
                .into_iter()
                .map(|(id, name, _region, _company_type)| (id, name))
                .collect(),
            Err(e) => {
                tracing::warn!("Failed to fetch competitor names for battlecards: {e:#}");
                std::collections::HashMap::new()
            }
        };

    let battlecards: Vec<BattlecardListItem> = rows
        .into_iter()
        .map(|row| {
            let section_count = [
                row.positioning,
                row.pricing,
                row.feature_matrix,
                row.strengths,
                row.weaknesses,
                row.objection_handlers,
                row.kill_shots,
                row.recent_news,
                row.win_loss,
            ]
            .iter()
            .filter(|s| s.is_some())
            .count();

            BattlecardListItem {
                id: row.id.to_string(),
                title: row.title,
                status: row.status,
                competitor_name: company_names
                    .get(&row.competitor_id)
                    .cloned()
                    .unwrap_or_default(),
                updated_at: row.updated_at.format("%Y-%m-%d %H:%M UTC").to_string(),
                section_count,
            }
        })
        .collect();

    let template = BattlecardsListPage {
        current_path: "/battlecards".to_string(),
        username: session.username.clone(),
        warning_count,
        theme: String::new(),
        battlecards,
        total,
        page,
        per_page,
        active_status,
    };

    if is_htmx_request(&headers) {
        // Return partial fragment for HTMX navigation
        super::render_template(&template)
    } else {
        super::render_template(&template)
    }
}

/// GET /battlecards/:id — detail page
pub async fn get_battlecard(
    Extension(store): Extension<Arc<PgStore>>,
    Path(id): Path<String>,
    Extension(session): Extension<WebSession>,
) -> impl IntoResponse {
    let _ctx = PageContext::from_session(&session, &format!("/battlecards/{}", id), 0);

    let uid = Uuid::parse_str(&id).unwrap_or_default();
    let row = store.get_battlecard(uid).await;

    match row {
        Ok(Some(row)) => {
            let response: BattlecardResponse = row.clone().into();
            let sections = vec![
                BattlecardSectionRow {
                    name: "positioning".into(),
                    label: "Positioning".into(),
                    has_data: row.positioning.is_some(),
                    preview: row
                        .positioning
                        .as_ref()
                        .and_then(|v| v.get("summary"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                },
                BattlecardSectionRow {
                    name: "pricing".into(),
                    label: "Pricing".into(),
                    has_data: row.pricing.is_some(),
                    preview: row
                        .pricing
                        .as_ref()
                        .and_then(|v| v.get("model"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                },
                BattlecardSectionRow {
                    name: "feature_matrix".into(),
                    label: "Feature Matrix".into(),
                    has_data: row.feature_matrix.is_some(),
                    preview: String::new(),
                },
                BattlecardSectionRow {
                    name: "strengths".into(),
                    label: "Strengths".into(),
                    has_data: row.strengths.is_some(),
                    preview: String::new(),
                },
                BattlecardSectionRow {
                    name: "weaknesses".into(),
                    label: "Weaknesses".into(),
                    has_data: row.weaknesses.is_some(),
                    preview: String::new(),
                },
                BattlecardSectionRow {
                    name: "objection_handlers".into(),
                    label: "Objection Handlers".into(),
                    has_data: row.objection_handlers.is_some(),
                    preview: String::new(),
                },
                BattlecardSectionRow {
                    name: "kill_shots".into(),
                    label: "Kill Shots".into(),
                    has_data: row.kill_shots.is_some(),
                    preview: String::new(),
                },
                BattlecardSectionRow {
                    name: "recent_news".into(),
                    label: "Recent News".into(),
                    has_data: row.recent_news.is_some(),
                    preview: String::new(),
                },
                BattlecardSectionRow {
                    name: "win_loss".into(),
                    label: "Win/Loss".into(),
                    has_data: row.win_loss.is_some(),
                    preview: String::new(),
                },
            ];

            let template = BattlecardDetailPage {
                current_path: format!("/battlecards/{}", id),
                username: session.username.clone(),
                warning_count: 0,
                theme: String::new(),
                battlecard: response,
                sections,
                competitor_name: String::new(),
            };

            super::render_template(&template)
        }
        Ok(None) => {
            let tpl = super::errors::NotFoundPage {
                current_path: format!("/battlecards/{}", id),
                username: session.username.clone(),
                warning_count: 0,
                theme: String::new(),
                requested_path: format!("/battlecards/{}", id),
            };
            super::render_template_with_status(axum::http::StatusCode::NOT_FOUND, &tpl)
        }
        Err(e) => {
            tracing::error!("Failed to fetch battlecard {id}: {e:#}");
            let tpl = super::errors::InternalErrorPage {
                current_path: format!("/battlecards/{}", id),
                username: session.username.clone(),
                warning_count: 0,
                theme: String::new(),
                error_message: "Failed to load battlecard".to_string(),
                request_id: String::new(),
            };
            super::render_template_with_status(axum::http::StatusCode::INTERNAL_SERVER_ERROR, &tpl)
        }
    }
}
