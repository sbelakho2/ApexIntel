//! Memos handler — GET /memos (list)
//!
//! Covers: weekly intelligence memo list with summaries, key findings,
//! and downloadable reports.

use std::sync::Arc;

use askama::Template;
use axum::{extract::Query, response::IntoResponse, Extension};

use super::PageContext;
use crate::middleware::session::WebSession;
use apex_core::data_state::{DataState, DegradedNotice};
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
    pub can_admin: bool,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,
    pub status_strip: crate::system_status::StatusStrip,
    pub briefing_mode: bool,

    pub memos: Vec<MemoListItem>,
    pub total: i64,
    pub degraded_notice: Option<String>,
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
    let mut degraded_notice: Option<String> = None;
    let unack_state = DataState::from_result(
        store
            .count_warnings(&WarningListFilters {
                acknowledged: Some(false),
                ..Default::default()
            })
            .await,
        "count_warnings failed (web memos page)",
        |_| false,
    );
    DegradedNotice::capture(&unack_state, &mut degraded_notice);
    let ctx = PageContext::from_session(&session, "/memos", unack_state.into_loaded_or(0));

    let memos_state = DataState::from_result(
        store.list_weekly_memos(50, 0).await,
        "list_weekly_memos failed (web memos page)",
        |(rows, _)| rows.is_empty(),
    );
    DegradedNotice::capture(&memos_state, &mut degraded_notice);
    let (memo_rows, total) = memos_state.into_loaded_or((vec![], 0));

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
        can_admin: ctx.can_admin,
        status_strip: crate::system_status::StatusStrip::current(),
        username: ctx.username,
        warning_count: ctx.warning_count,
        theme: ctx.theme,
        briefing_mode: query.briefing.unwrap_or(false),
        memos,
        total,
        degraded_notice,
    };

    super::render_template(&tpl)
}

/// HTMX partial: GET /memos/_list — renders just the memo list fragment.
/// Called by the memos page on load via `hx-get="/memos/_list"`.
pub async fn list_memos_partial(
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
) -> impl IntoResponse {
    let mut degraded_notice: Option<String> = None;
    let unack_state = DataState::from_result(
        store
            .count_warnings(&WarningListFilters {
                acknowledged: Some(false),
                ..Default::default()
            })
            .await,
        "count_warnings failed (web memos list partial)",
        |_| false,
    );
    DegradedNotice::capture(&unack_state, &mut degraded_notice);
    let _ctx = PageContext::from_session(&session, "/memos", unack_state.into_loaded_or(0));

    let memos_state = DataState::from_result(
        store.list_weekly_memos(50, 0).await,
        "list_weekly_memos failed (web memos list partial)",
        |(rows, _)| rows.is_empty(),
    );
    DegradedNotice::capture(&memos_state, &mut degraded_notice);
    let (memo_rows, total) = memos_state.into_loaded_or((vec![], 0));

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

    let tpl = MemosListPartial {
        memos,
        total,
        degraded_notice,
    };
    super::render_template(&tpl)
}

/// Template for the HTMX partial fragment.
#[derive(Template)]
#[template(path = "pages/memos/_list.html")]
pub struct MemosListPartial {
    pub memos: Vec<MemoListItem>,
    pub total: i64,
    pub degraded_notice: Option<String>,
}
