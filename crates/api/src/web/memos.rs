//! Memos handler — GET /memos (list)
//!
//! Covers: weekly intelligence memo list with summaries, key findings,
//! and downloadable reports.

use std::sync::Arc;

use askama::Template;
use axum::{extract::Query, response::IntoResponse, Extension};

use super::PageContext;
use crate::middleware::session::WebSession;
use apex_store::postgres::{PgStore, WarningListFilters};

// ─── Template data ──────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct MemoSection {
    pub heading: String,
    pub body: String,
}

#[derive(Clone, Debug)]
pub struct MemoListItem {
    pub id: String,
    pub title: String,
    pub week_start: String,
    pub week_end: String,
    pub summary: String,
    pub key_findings: Vec<String>,
    pub sections: Vec<MemoSection>,
    pub warning_count: i64,
    pub insight_count: i64,
    pub created_at: String,
}

// ─── Template ───────────────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "pages/memos.html")]
pub struct MemosPage {
    pub current_path: String,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,
    pub briefing_mode: bool,

    pub memos: Vec<MemoListItem>,
    pub total: i64,
}

#[derive(Debug, Default, serde::Deserialize)]
pub struct MemoQuery {
    pub briefing: Option<bool>,
}

// ─── Handler ────────────────────────────────────────────────────────────────

/// GET /memos — weekly intelligence memo list.
pub async fn list_memos(
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
    Query(query): Query<MemoQuery>,
) -> impl IntoResponse {
    let unack = store
        .count_warnings(&WarningListFilters {
            acknowledged: Some(false),
            ..Default::default()
        })
        .await
        .unwrap_or(0);
    let ctx = PageContext::from_session(&session, "/memos", unack);

    let (memo_rows, total) = store.list_weekly_memos(50, 0).await.unwrap_or_else(|e| {
        tracing::error!("Failed to list weekly memos: {e}");
        (vec![], 0)
    });

    let memos: Vec<MemoListItem> = memo_rows
        .iter()
        .map(|m| MemoListItem {
            id: m.id.to_string(),
            title: m.title.clone(),
            week_start: m.week_start.clone(),
            week_end: m.week_end.clone(),
            summary: m.executive_summary.clone(),
            key_findings: m.action_items.iter().map(|a| a.text.clone()).collect(),
            sections: m
                .sections
                .iter()
                .map(|s| MemoSection {
                    heading: s.title.clone(),
                    body: s.content.clone(),
                })
                .collect(),
            warning_count: m.key_metrics.warnings_total,
            insight_count: m.key_metrics.insights_generated,
            created_at: m.generated_at.clone(),
        })
        .collect();

    let tpl = MemosPage {
        current_path: ctx.current_path,
        username: ctx.username,
        warning_count: ctx.warning_count,
        theme: ctx.theme,
        briefing_mode: query.briefing.unwrap_or(false),
        memos,
        total,
    };

    tpl.into_response()
}
