//! Collaboration web handlers — HTML pages for investigation workspaces,
//! priority queue, activity feed, supplier risk, pipeline, evidence, and
//! team assignments.
//!
//! These handlers render server-side pages using Askama + HTMX, following the
//! same pattern as [`crate::web::notifications`] and [`crate::web::triage`].

use std::sync::Arc;

use askama::Template;
use axum::{
    extract::{Path, Query},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Redirect},
    Extension, Form,
};
use serde::Deserialize;
use uuid::Uuid;

use apex_store::postgres::PgStore;

use crate::middleware::session::WebSession;
use crate::routes::collaboration::{fmt_json_value, format_activity_details};
use crate::web::{render_template, PageContext};

// ─── Query parameters ───────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct WorkspaceListQuery {
    pub status: Option<String>,
    pub workspace_type: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateWorkspaceForm {
    pub name: String,
    pub description: Option<String>,
    pub workspace_type: String,
    pub visibility: String,
    pub tags: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct QueueListQuery {
    pub status: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AddToQueueForm {
    pub item_type: String,
    pub item_id: String,
    pub item_title: String,
    pub priority: i32,
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ActivityFeedQuery {
    pub workspace_id: Option<String>,
    pub team_id: Option<String>,
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct AddSupplierRiskForm {
    pub supplier_id: String,
    pub risk_category: String,
    pub risk_score: f64,
    pub risk_factors: Option<String>,
    pub mitigation: Option<String>,
    pub owner_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreatePipelineForm {
    pub title: String,
    pub stage: String,
    pub value_estimate: Option<f64>,
    pub probability: f64,
    pub owner_id: Option<String>,
    pub expected_close: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AddEvidenceForm {
    pub entity_type: String,
    pub entity_id: String,
    pub evidence_type: String,
    pub source_url: String,
    pub source_domain: Option<String>,
    pub source_name: Option<String>,
    pub reliability_score: f64,
    pub excerpt: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateTeamAssignmentForm {
    pub team_id: String,
    pub team_name: String,
    pub entity_type: String,
    pub entity_id: String,
    pub assigned_to: String,
    pub role: String,
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AssignUserForm {
    pub user_id: String,
    pub role: String,
}

#[derive(Debug, Deserialize)]
pub struct ShareWorkspaceForm {
    pub shared_with: String,
    pub share_type: String,
    pub access_level: String,
    pub message: Option<String>,
    pub expires_at: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateStageForm {
    pub stage: String,
}

// ─── Display types ──────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct WorkspaceItem {
    pub id: String,
    pub name: String,
    pub description: String,
    pub workspace_type: String,
    pub owner_id: String,
    pub status: String,
    pub visibility: String,
    pub tags: Vec<String>,
    pub findings: String,
    pub conclusions: String,
    pub created_at: String,
    pub updated_at: String,
    pub closed_at: String,
}

#[derive(Clone, Debug)]
pub struct ActivityItem {
    pub id: String,
    pub actor_name: String,
    pub action_type: String,
    pub entity_type: String,
    pub entity_name: String,
    pub details: String,
    pub created_at: String,
}

#[derive(Clone, Debug)]
pub struct QueueItem {
    pub id: String,
    pub item_type: String,
    pub item_id: String,
    pub item_title: String,
    pub priority: i32,
    pub status: String,
    pub notes: String,
    pub created_at: String,
    pub completed_at: String,
}

#[derive(Clone, Debug)]
pub struct SupplierRiskItem {
    pub id: String,
    pub supplier_id: String,
    pub risk_category: String,
    pub risk_score: f64,
    pub risk_factors: String,
    pub mitigation: String,
    pub owner_id: String,
    pub status: String,
    pub last_reviewed: String,
    pub next_review: String,
    pub created_at: String,
}

#[derive(Clone, Debug)]
pub struct PipelineOpportunityItem {
    pub id: String,
    pub title: String,
    pub stage: String,
    pub value_estimate: String,
    pub probability: f64,
    pub owner_id: String,
    pub expected_close: String,
    pub actual_close: String,
    pub notes: String,
    pub created_at: String,
    pub closed_at: String,
}

#[derive(Clone, Debug)]
pub struct EvidenceItem {
    pub id: String,
    pub entity_type: String,
    pub entity_id: String,
    pub evidence_type: String,
    pub source_url: String,
    pub source_domain: String,
    pub source_name: String,
    pub reliability_score: f64,
    pub excerpt: String,
    pub created_at: String,
}

#[derive(Clone, Debug)]
pub struct TeamAssignmentItem {
    pub id: String,
    pub team_id: String,
    pub team_name: String,
    pub entity_type: String,
    pub entity_id: String,
    pub assigned_by: String,
    pub assigned_to: String,
    pub role: String,
    pub notes: String,
    pub created_at: String,
}

#[derive(Clone, Debug)]
pub struct AssignmentItem {
    pub id: String,
    pub user_id: String,
    pub role: String,
    pub assigned_by: String,
    pub assigned_at: String,
}

#[derive(Clone, Debug)]
pub struct ShareItem {
    pub id: String,
    pub shared_with: String,
    pub share_type: String,
    pub access_level: String,
    pub message: String,
    pub expires_at: String,
    pub created_at: String,
}

// ─── Template structs — Workspaces ──────────────────────────────────────

#[derive(Template)]
#[template(path = "pages/collaboration/workspaces.html")]
pub(crate) struct WorkspacesPage {
    pub current_path: String,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,
    pub workspaces: Vec<WorkspaceItem>,
    pub total: usize,
    pub open_count: usize,
    pub current_status: String,
}

#[derive(Template)]
#[template(path = "pages/collaboration/workspace_detail.html")]
pub(crate) struct WorkspaceDetailPage {
    pub current_path: String,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,
    pub workspace: WorkspaceItem,
    pub assignments: Vec<AssignmentItem>,
    pub shares: Vec<ShareItem>,
    pub activity: Vec<ActivityItem>,
}

// ─── Template structs — Queue, Activity, Risk, Pipeline, Evidence, Teams ─

#[derive(Template)]
#[template(path = "pages/collaboration/queue.html")]
pub(crate) struct QueuePage {
    pub current_path: String,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,
    pub items: Vec<QueueItem>,
    pub total: usize,
    pub pending_count: usize,
    pub current_status: String,
}

#[derive(Template)]
#[template(path = "pages/collaboration/activity.html")]
pub(crate) struct ActivityPage {
    pub current_path: String,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,
    pub items: Vec<ActivityItem>,
}

#[derive(Template)]
#[template(path = "pages/collaboration/supplier_risk.html")]
pub(crate) struct SupplierRiskPage {
    pub current_path: String,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,
    pub entries: Vec<SupplierRiskItem>,
    pub total: usize,
    pub active_count: usize,
}

#[derive(Template)]
#[template(path = "pages/collaboration/pipeline.html")]
pub(crate) struct PipelinePage {
    pub current_path: String,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,
    pub opportunities: Vec<PipelineOpportunityItem>,
    pub total: usize,
}

#[derive(Template)]
#[template(path = "pages/collaboration/evidence.html")]
pub(crate) struct EvidencePage {
    pub current_path: String,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,
    pub items: Vec<EvidenceItem>,
    pub total: usize,
}

#[derive(Template)]
#[template(path = "pages/collaboration/team_assignments.html")]
pub(crate) struct TeamAssignmentsPage {
    pub current_path: String,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,
    pub assignments: Vec<TeamAssignmentItem>,
    pub total: usize,
}

// ─── Helpers ────────────────────────────────────────────────────────────

fn fmt_opt(s: &Option<String>) -> String {
    s.as_deref().unwrap_or("").to_string()
}

fn fmt_opt_date(d: &Option<chrono::DateTime<chrono::Utc>>) -> String {
    match d {
        Some(dt) => dt.format("%Y-%m-%d %H:%M").to_string(),
        None => String::new(),
    }
}

fn fmt_opt_naive_date(d: &Option<chrono::NaiveDate>) -> String {
    match d {
        Some(d) => d.format("%Y-%m-%d").to_string(),
        None => String::new(),
    }
}

fn fmt_opt_f64(v: &Option<f64>) -> String {
    match v {
        Some(val) => format!("{:.2}", val),
        None => String::new(),
    }
}

// ─── Handlers — Workspaces ─────────────────────────────────────────────

/// GET /workspaces — list all investigation workspaces.
pub async fn list_workspaces(
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
    _headers: HeaderMap,
    Query(params): Query<WorkspaceListQuery>,
) -> impl IntoResponse {
    let warning_count = store
        .count_warnings(&apex_store::postgres::WarningListFilters {
            acknowledged: Some(false),
            ..Default::default()
        })
        .await
        .unwrap_or(0);
    let ctx = PageContext::from_session(&session, "/workspaces", warning_count);

    let status_filter = params.status.as_deref();
    let workspaces = store
        .list_investigation_workspaces(100)
        .await
        .unwrap_or_default();

    let filtered: Vec<_> = if let Some(status) = status_filter {
        workspaces
            .into_iter()
            .filter(|w| w.status == status)
            .collect()
    } else {
        workspaces
    };

    let total = filtered.len();
    let open_count = filtered.iter().filter(|w| w.status == "open").count();

    let items: Vec<WorkspaceItem> = filtered
        .into_iter()
        .map(|w| WorkspaceItem {
            id: w.id.to_string(),
            name: w.name,
            description: fmt_opt(&w.description),
            workspace_type: w.workspace_type,
            owner_id: w.owner_id,
            status: w.status,
            visibility: w.visibility,
            tags: w.tags,
            findings: fmt_opt(&w.findings),
            conclusions: fmt_opt(&w.conclusions),
            created_at: w.created_at.format("%Y-%m-%d %H:%M").to_string(),
            updated_at: w.updated_at.format("%Y-%m-%d %H:%M").to_string(),
            closed_at: fmt_opt_date(&w.closed_at),
        })
        .collect();

    let current_status = params.status.unwrap_or_else(|| "all".to_string());

    let page = WorkspacesPage {
        current_path: ctx.current_path,
        username: ctx.username,
        warning_count: ctx.warning_count,
        theme: ctx.theme,
        workspaces: items,
        total,
        open_count,
        current_status,
    };

    render_template(&page)
}

/// GET /workspaces/new — new workspace form page (B308).
///
/// Previously re-rendered the empty workspace list, so the "New Workspace"
/// navigation dead-ended on an empty state and creation was impossible.
#[derive(Template)]
#[template(path = "pages/collaboration/workspace_new.html")]
pub struct WorkspaceNewPage {
    pub current_path: String,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,
}

pub async fn new_workspace_page(
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
) -> impl IntoResponse {
    let warning_count = store
        .count_warnings(&apex_store::postgres::WarningListFilters {
            acknowledged: Some(false),
            ..Default::default()
        })
        .await
        .unwrap_or(0);
    let ctx = PageContext::from_session(&session, "/workspaces/new", warning_count);

    let page = WorkspaceNewPage {
        current_path: ctx.current_path,
        username: ctx.username,
        warning_count: ctx.warning_count,
        theme: ctx.theme,
    };

    render_template(&page)
}

/// POST /workspaces — create a new workspace.
pub async fn create_workspace(
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
    Form(form): Form<CreateWorkspaceForm>,
) -> impl IntoResponse {
    let tags: Vec<String> = form
        .tags
        .as_deref()
        .map(|s| s.split(',').map(|t| t.trim().to_string()).collect())
        .unwrap_or_default();

    let _ = store
        .create_investigation_workspace(
            &form.name,
            form.description.as_deref(),
            &form.workspace_type,
            &session.username,
            None, // team_id
            &form.visibility,
            &tags,
            &serde_json::json!({}),
        )
        .await;

    Redirect::to("/workspaces")
}

/// GET /workspaces/:id — workspace detail page.
pub async fn get_workspace(
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let warning_count = store
        .count_warnings(&apex_store::postgres::WarningListFilters {
            acknowledged: Some(false),
            ..Default::default()
        })
        .await
        .unwrap_or(0);
    let ctx = PageContext::from_session(&session, &format!("/workspaces/{}", id), warning_count);

    let workspace_id = match Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => {
            return (StatusCode::NOT_FOUND, "Invalid workspace ID").into_response();
        }
    };

    let workspace = match store.get_investigation_workspace(workspace_id).await {
        Ok(Some(w)) => w,
        _ => {
            return (StatusCode::NOT_FOUND, "Workspace not found").into_response();
        }
    };

    let assignments = store
        .list_workspace_assignments(workspace_id)
        .await
        .unwrap_or_default();
    let shares = store
        .list_investigation_shares(workspace_id)
        .await
        .unwrap_or_default();
    let activity = store
        .list_activity_feed(Some(workspace_id), None, None, 50)
        .await
        .unwrap_or_default();

    let w_item = WorkspaceItem {
        id: workspace.id.to_string(),
        name: workspace.name,
        description: fmt_opt(&workspace.description),
        workspace_type: workspace.workspace_type,
        owner_id: workspace.owner_id,
        status: workspace.status,
        visibility: workspace.visibility,
        tags: workspace.tags,
        findings: fmt_opt(&workspace.findings),
        conclusions: fmt_opt(&workspace.conclusions),
        created_at: workspace.created_at.format("%Y-%m-%d %H:%M").to_string(),
        updated_at: workspace.updated_at.format("%Y-%m-%d %H:%M").to_string(),
        closed_at: fmt_opt_date(&workspace.closed_at),
    };

    let a_items: Vec<AssignmentItem> = assignments
        .into_iter()
        .map(|a| AssignmentItem {
            id: a.id.to_string(),
            user_id: a.user_id,
            role: a.role,
            assigned_by: a.assigned_by,
            assigned_at: a.assigned_at.format("%Y-%m-%d %H:%M").to_string(),
        })
        .collect();

    let s_items: Vec<ShareItem> = shares
        .into_iter()
        .map(|s| ShareItem {
            id: s.id.to_string(),
            shared_with: s.shared_with,
            share_type: s.share_type,
            access_level: s.access_level,
            message: fmt_opt(&s.message),
            expires_at: fmt_opt_date(&s.expires_at),
            created_at: s.created_at.format("%Y-%m-%d %H:%M").to_string(),
        })
        .collect();

    let act_items: Vec<ActivityItem> = activity
        .into_iter()
        .map(|a| ActivityItem {
            id: a.id.to_string(),
            actor_name: a.actor_name,
            action_type: a.action_type.clone(),
            entity_type: fmt_opt(&a.entity_type),
            entity_name: fmt_opt(&a.entity_name),
            details: format_activity_details(&a.action_type, &a.details),
            created_at: a.created_at.format("%Y-%m-%d %H:%M").to_string(),
        })
        .collect();

    let page = WorkspaceDetailPage {
        current_path: ctx.current_path,
        username: ctx.username,
        warning_count: ctx.warning_count,
        theme: ctx.theme,
        workspace: w_item,
        assignments: a_items,
        shares: s_items,
        activity: act_items,
    };

    render_template(&page)
}

/// POST /workspaces/:id/close — close a workspace.
pub async fn close_workspace(
    _session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    if let Ok(uuid) = Uuid::parse_str(&id) {
        if let Ok(Some(workspace)) = store.get_investigation_workspace(uuid).await {
            let _ = store
                .update_investigation_workspace(
                    uuid,
                    Some(&workspace.name),
                    None, // description — leave as-is
                    Some("closed"),
                    Some(&workspace.tags),
                    Some(&workspace.entity_focus),
                    None, // findings — leave as-is
                    None, // conclusions — leave as-is
                )
                .await;
        }
    }
    Redirect::to(&format!("/workspaces/{}", id))
}

/// POST /workspaces/:id/assign — assign a user to a workspace.
pub async fn assign_user_to_workspace(
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
    Path(id): Path<String>,
    Form(form): Form<AssignUserForm>,
) -> impl IntoResponse {
    if let Ok(uuid) = Uuid::parse_str(&id) {
        let _ = store
            .create_workspace_assignment(uuid, &form.user_id, &form.role, &session.username)
            .await;
    }
    Redirect::to(&format!("/workspaces/{}", id))
}

/// POST /workspaces/:id/shares — share a workspace.
pub async fn share_workspace(
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
    Path(id): Path<String>,
    Form(form): Form<ShareWorkspaceForm>,
) -> impl IntoResponse {
    if let Ok(uuid) = Uuid::parse_str(&id) {
        let expires_at = form.expires_at.and_then(|s| {
            chrono::NaiveDate::parse_from_str(&s, "%Y-%m-%d")
                .ok()
                .and_then(|d| d.and_hms_opt(0, 0, 0))
                .map(|dt| {
                    chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(dt, chrono::Utc)
                })
        });
        let _ = store
            .create_investigation_share(
                uuid,
                &session.username,
                &form.shared_with,
                &form.share_type,
                &form.access_level,
                form.message.as_deref(),
                expires_at,
            )
            .await;
    }
    Redirect::to(&format!("/workspaces/{}", id))
}

// ─── Handlers — Priority Queue ─────────────────────────────────────────

/// GET /queue — priority queue page.
pub async fn list_queue(
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
    _headers: HeaderMap,
    Query(params): Query<QueueListQuery>,
) -> impl IntoResponse {
    let warning_count = store
        .count_warnings(&apex_store::postgres::WarningListFilters {
            acknowledged: Some(false),
            ..Default::default()
        })
        .await
        .unwrap_or(0);
    let ctx = PageContext::from_session(&session, "/queue", warning_count);

    let status_filter = params.status.as_deref();
    let items = store
        .list_priority_queue_items(&session.username, status_filter, 100)
        .await
        .unwrap_or_default();

    let total = items.len();
    let pending_count = items.iter().filter(|i| i.status == "pending").count();

    let q_items: Vec<QueueItem> = items
        .into_iter()
        .map(|i| QueueItem {
            id: i.id.to_string(),
            item_type: i.item_type,
            item_id: i.item_id.to_string(),
            item_title: i.item_title,
            priority: i.priority,
            status: i.status,
            notes: fmt_opt(&i.notes),
            created_at: i.created_at.format("%Y-%m-%d %H:%M").to_string(),
            completed_at: fmt_opt_date(&i.completed_at),
        })
        .collect();

    let current_status = params.status.unwrap_or_else(|| "all".to_string());

    let page = QueuePage {
        current_path: ctx.current_path,
        username: ctx.username,
        warning_count: ctx.warning_count,
        theme: ctx.theme,
        items: q_items,
        total,
        pending_count,
        current_status,
    };

    render_template(&page)
}

/// POST /queue — add item to priority queue.
pub async fn add_to_queue(
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
    Form(form): Form<AddToQueueForm>,
) -> impl IntoResponse {
    let item_id = Uuid::parse_str(&form.item_id).unwrap_or_else(|_| Uuid::new_v4());
    let _ = store
        .create_priority_queue_item(
            &session.username,
            &form.item_type,
            item_id,
            &form.item_title,
            form.priority,
            form.notes.as_deref(),
        )
        .await;
    Redirect::to("/queue")
}

/// POST /queue/:id/complete — mark queue item as completed.
pub async fn complete_queue_item(
    _session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    if let Ok(uuid) = Uuid::parse_str(&id) {
        let _ = store
            .update_priority_queue_item(uuid, None, Some("completed"), None)
            .await;
    }
    Redirect::to("/queue")
}

// ─── Handlers — Activity Feed ──────────────────────────────────────────

/// GET /activity — activity feed page.
pub async fn list_activity(
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
    _headers: HeaderMap,
    Query(params): Query<ActivityFeedQuery>,
) -> impl IntoResponse {
    let warning_count = store
        .count_warnings(&apex_store::postgres::WarningListFilters {
            acknowledged: Some(false),
            ..Default::default()
        })
        .await
        .unwrap_or(0);
    let ctx = PageContext::from_session(&session, "/activity", warning_count);

    let workspace_id = params.workspace_id.and_then(|s| Uuid::parse_str(&s).ok());
    let limit = params.limit.unwrap_or(100);

    let items = store
        .list_activity_feed(workspace_id, params.team_id.as_deref(), None, limit)
        .await
        .unwrap_or_default();

    let a_items: Vec<ActivityItem> = items
        .into_iter()
        .map(|a| ActivityItem {
            id: a.id.to_string(),
            actor_name: a.actor_name,
            action_type: a.action_type.clone(),
            entity_type: fmt_opt(&a.entity_type),
            entity_name: fmt_opt(&a.entity_name),
            details: format_activity_details(&a.action_type, &a.details),
            created_at: a.created_at.format("%Y-%m-%d %H:%M").to_string(),
        })
        .collect();

    let page = ActivityPage {
        current_path: ctx.current_path,
        username: ctx.username,
        warning_count: ctx.warning_count,
        theme: ctx.theme,
        items: a_items,
    };

    render_template(&page)
}

// ─── Handlers — Supplier Risk ──────────────────────────────────────────

/// GET /supplier-risk — supplier risk entries page.
pub async fn list_supplier_risks(
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
    _headers: HeaderMap,
) -> impl IntoResponse {
    let warning_count = store
        .count_warnings(&apex_store::postgres::WarningListFilters {
            acknowledged: Some(false),
            ..Default::default()
        })
        .await
        .unwrap_or(0);
    let ctx = PageContext::from_session(&session, "/supplier-risk", warning_count);

    let entries = store
        .list_supplier_risk_entries(None, 100)
        .await
        .unwrap_or_default();

    let total = entries.len();
    let active_count = entries.iter().filter(|e| e.status == "active").count();

    let s_items: Vec<SupplierRiskItem> = entries
        .into_iter()
        .map(|e| SupplierRiskItem {
            id: e.id.to_string(),
            supplier_id: e.supplier_id,
            risk_category: e.risk_category,
            risk_score: e.risk_score,
            risk_factors: fmt_json_value(&e.risk_factors),
            mitigation: fmt_opt(&e.mitigation),
            owner_id: fmt_opt(&e.owner_id),
            status: e.status,
            last_reviewed: fmt_opt_date(&e.last_reviewed),
            next_review: fmt_opt_date(&e.next_review),
            created_at: e.created_at.format("%Y-%m-%d %H:%M").to_string(),
        })
        .collect();

    let page = SupplierRiskPage {
        current_path: ctx.current_path,
        username: ctx.username,
        warning_count: ctx.warning_count,
        theme: ctx.theme,
        entries: s_items,
        total,
        active_count,
    };

    render_template(&page)
}

/// POST /supplier-risk — add a supplier risk entry.
pub async fn add_supplier_risk(
    _session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
    Form(form): Form<AddSupplierRiskForm>,
) -> impl IntoResponse {
    let risk_factors: serde_json::Value = form
        .risk_factors
        .as_deref()
        .map(|s| serde_json::from_str(s).unwrap_or(serde_json::json!({"summary": s})))
        .unwrap_or(serde_json::json!({}));

    let _ = store
        .create_supplier_risk_entry(
            &form.supplier_id,
            &form.risk_category,
            form.risk_score,
            &risk_factors,
            form.mitigation.as_deref(),
            form.owner_id.as_deref(),
        )
        .await;

    Redirect::to("/supplier-risk")
}

// ─── Handlers — Pipeline Opportunities ─────────────────────────────────

/// GET /pipeline — pipeline opportunities page.
pub async fn list_pipeline(
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
    _headers: HeaderMap,
) -> impl IntoResponse {
    let warning_count = store
        .count_warnings(&apex_store::postgres::WarningListFilters {
            acknowledged: Some(false),
            ..Default::default()
        })
        .await
        .unwrap_or(0);
    let ctx = PageContext::from_session(&session, "/pipeline", warning_count);

    let opportunities = store
        .list_pipeline_opportunities(None, None, 100)
        .await
        .unwrap_or_default();

    let total = opportunities.len();

    let p_items: Vec<PipelineOpportunityItem> = opportunities
        .into_iter()
        .map(|o| PipelineOpportunityItem {
            id: o.id.to_string(),
            title: o.title,
            stage: o.stage,
            value_estimate: fmt_opt_f64(&o.value_estimate),
            probability: o.probability,
            owner_id: fmt_opt(&o.owner_id),
            expected_close: fmt_opt_naive_date(&o.expected_close),
            actual_close: fmt_opt_naive_date(&o.actual_close),
            notes: fmt_opt(&o.notes),
            created_at: o.created_at.format("%Y-%m-%d %H:%M").to_string(),
            closed_at: fmt_opt_date(&o.closed_at),
        })
        .collect();

    let page = PipelinePage {
        current_path: ctx.current_path,
        username: ctx.username,
        warning_count: ctx.warning_count,
        theme: ctx.theme,
        opportunities: p_items,
        total,
    };

    render_template(&page)
}

/// POST /pipeline — create a pipeline opportunity.
pub async fn create_pipeline_opportunity(
    _session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
    Form(form): Form<CreatePipelineForm>,
) -> impl IntoResponse {
    let expected_close = form
        .expected_close
        .and_then(|s| chrono::NaiveDate::parse_from_str(&s, "%Y-%m-%d").ok());

    let _ = store
        .create_pipeline_opportunity(
            None, // opportunity_id
            &form.title,
            &form.stage,
            form.value_estimate,
            form.probability,
            form.owner_id.as_deref(),
            expected_close,
            form.notes.as_deref(),
        )
        .await;

    Redirect::to("/pipeline")
}

/// POST /pipeline/:id/stage — update pipeline stage.
pub async fn update_pipeline_stage(
    _session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
    Path(id): Path<String>,
    Form(form): Form<UpdateStageForm>,
) -> impl IntoResponse {
    if let Ok(uuid) = Uuid::parse_str(&id) {
        let _ = store.update_pipeline_stage(uuid, &form.stage, None).await;
    }
    Redirect::to("/pipeline")
}

// ─── Handlers — Source Evidence ────────────────────────────────────────

/// GET /evidence — source evidence page.
pub async fn list_evidence(
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
    _headers: HeaderMap,
) -> impl IntoResponse {
    let warning_count = store
        .count_warnings(&apex_store::postgres::WarningListFilters {
            acknowledged: Some(false),
            ..Default::default()
        })
        .await
        .unwrap_or(0);
    let ctx = PageContext::from_session(&session, "/evidence", warning_count);

    let items = store
        .list_source_evidence(None, None, None, 100)
        .await
        .unwrap_or_default();

    let total = items.len();

    let e_items: Vec<EvidenceItem> = items
        .into_iter()
        .map(|e| EvidenceItem {
            id: e.id.to_string(),
            entity_type: e.entity_type,
            entity_id: e.entity_id,
            evidence_type: e.evidence_type,
            source_url: e.source_url,
            source_domain: fmt_opt(&e.source_domain),
            source_name: fmt_opt(&e.source_name),
            reliability_score: e.reliability_score,
            excerpt: fmt_opt(&e.excerpt),
            created_at: e.created_at.format("%Y-%m-%d %H:%M").to_string(),
        })
        .collect();

    let page = EvidencePage {
        current_path: ctx.current_path,
        username: ctx.username,
        warning_count: ctx.warning_count,
        theme: ctx.theme,
        items: e_items,
        total,
    };

    render_template(&page)
}

/// POST /evidence — add source evidence.
pub async fn add_evidence(
    _session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
    Form(form): Form<AddEvidenceForm>,
) -> impl IntoResponse {
    let _ = store
        .create_source_evidence(
            &form.entity_type,
            &form.entity_id,
            &form.evidence_type,
            &form.source_url,
            form.source_domain.as_deref(),
            form.source_name.as_deref(),
            form.reliability_score,
            form.excerpt.as_deref(),
            &serde_json::json!({}),
        )
        .await;

    Redirect::to("/evidence")
}

// ─── Handlers — Team Assignments ───────────────────────────────────────

/// GET /team-assignments — team assignments page.
pub async fn list_team_assignments(
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
    _headers: HeaderMap,
) -> impl IntoResponse {
    let warning_count = store
        .count_warnings(&apex_store::postgres::WarningListFilters {
            acknowledged: Some(false),
            ..Default::default()
        })
        .await
        .unwrap_or(0);
    let ctx = PageContext::from_session(&session, "/team-assignments", warning_count);

    let assignments = store
        .list_team_assignments(None, None)
        .await
        .unwrap_or_default();

    let total = assignments.len();

    let t_items: Vec<TeamAssignmentItem> = assignments
        .into_iter()
        .map(|a| TeamAssignmentItem {
            id: a.id.to_string(),
            team_id: a.team_id,
            team_name: a.team_name,
            entity_type: a.entity_type,
            entity_id: a.entity_id,
            assigned_by: a.assigned_by,
            assigned_to: a.assigned_to,
            role: a.role,
            notes: fmt_opt(&a.notes),
            created_at: a.created_at.format("%Y-%m-%d %H:%M").to_string(),
        })
        .collect();

    let page = TeamAssignmentsPage {
        current_path: ctx.current_path,
        username: ctx.username,
        warning_count: ctx.warning_count,
        theme: ctx.theme,
        assignments: t_items,
        total,
    };

    render_template(&page)
}

/// POST /team-assignments — create a team assignment.
pub async fn create_team_assignment(
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
    Form(form): Form<CreateTeamAssignmentForm>,
) -> impl IntoResponse {
    let _ = store
        .create_team_assignment(
            &form.team_id,
            &form.team_name,
            &form.entity_type,
            &form.entity_id,
            &session.username,
            &form.assigned_to,
            &form.role,
            form.notes.as_deref(),
        )
        .await;

    Redirect::to("/team-assignments")
}
