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
pub struct MemoActionItem {
    pub text: String,
    /// Priority exactly as stored on the memo action item ("P1", "high", ...).
    pub priority: String,
    pub assignee: Option<String>,
}

#[derive(Clone, Debug)]
pub struct MemoListItem {
    pub id: String,
    pub title: String,
    pub week_start: String,
    pub week_end: String,
    pub summary: String,
    /// Real action items with their stored priorities — the template used to
    /// hard-code "P1" for every finding (#149).
    pub actions: Vec<MemoActionItem>,
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
    pub can_write: bool,
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

/// Map stored action items to the view model, preserving priority and
/// assignee instead of flattening to text and inventing a priority later.
fn memo_action_items(items: &[apex_store::postgres::WeeklyMemoActionItem]) -> Vec<MemoActionItem> {
    items
        .iter()
        .map(|item| MemoActionItem {
            text: item.text.clone(),
            priority: item.priority.clone(),
            assignee: item.assignee.clone(),
        })
        .collect()
}

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
            actions: memo_action_items(&m.action_items),
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
        can_write: ctx.can_write,
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
            actions: memo_action_items(&m.action_items),
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use apex_store::postgres::WeeklyMemoActionItem;

    #[test]
    fn memo_actions_keep_their_stored_priority() {
        let items = vec![
            WeeklyMemoActionItem {
                text: "Contact supplier".to_string(),
                priority: "P2".to_string(),
                assignee: Some("alice".to_string()),
            },
            WeeklyMemoActionItem {
                text: "Review contract".to_string(),
                priority: "high".to_string(),
                assignee: None,
            },
        ];
        let mapped = memo_action_items(&items);
        assert_eq!(mapped.len(), 2);
        assert_eq!(mapped[0].priority, "P2");
        assert_eq!(mapped[0].assignee.as_deref(), Some("alice"));
        assert_eq!(mapped[1].priority, "high");
        assert!(mapped[1].assignee.is_none());
    }
}

#[cfg(test)]
mod render_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use askama::Template;

    fn page(memos: Vec<MemoListItem>) -> MemosPage {
        MemosPage {
            current_path: "/memos".to_string(),
            can_admin: false,
            can_write: false,
            username: "analyst".to_string(),
            warning_count: 0,
            theme: "dark".to_string(),
            status_strip: crate::system_status::StatusStrip::unknown(),
            briefing_mode: false,
            memos,
            total: 0,
            degraded_notice: None,
        }
    }

    /// #149: the full page must render the list server-side (single load) and
    /// show the current memo's executive summary plus its stored priorities.
    #[test]
    fn memos_page_renders_actions_and_summary_once() {
        let html = page(vec![MemoListItem {
            id: "m1".to_string(),
            title: "Weekly Memo".to_string(),
            week_start: "2026-09-21".to_string(),
            week_end: "2026-09-28".to_string(),
            summary: "Executive summary line one.\nLine two.".to_string(),
            actions: vec![
                MemoActionItem {
                    text: "Call supplier".to_string(),
                    priority: "P2".to_string(),
                    assignee: Some("alice".to_string()),
                },
                MemoActionItem {
                    text: "Review contract".to_string(),
                    priority: "high".to_string(),
                    assignee: None,
                },
            ],
            sections: vec![MemoSection {
                heading: "Market".to_string(),
                body: "Body one.\nBody two.".to_string(),
            }],
            warning_count: 3,
            insight_count: 2,
            created_at: "2026-09-28".to_string(),
        }])
        .render()
        .expect("memos page renders");

        assert!(html.contains("Executive summary line one."));
        assert!(html.contains(">P2<"), "stored priority missing: {html}");
        assert!(html.contains(">high<"));
        assert!(html.contains("Call supplier"));
        assert!(!html.contains(">P1<"), "hard-coded P1 must be gone");
    }
}
