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
    /// Optional entity/signal the workspace was started from (entity dossier
    /// or warning "Start investigation" action). Stored as `entity_focus` so
    /// the entity page can list its open investigations.
    pub entity_id: Option<String>,
    pub signal_id: Option<String>,
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
    pub can_admin: bool,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,
    pub status_strip: crate::system_status::StatusStrip,
    pub workspaces: Vec<WorkspaceItem>,
    pub total: usize,
    pub open_count: usize,
    pub current_status: String,
}

#[derive(Template)]
#[template(path = "pages/collaboration/workspace_detail.html")]
pub(crate) struct WorkspaceDetailPage {
    pub current_path: String,
    pub can_admin: bool,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,
    pub status_strip: crate::system_status::StatusStrip,
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
    pub can_admin: bool,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,
    pub status_strip: crate::system_status::StatusStrip,
    pub items: Vec<QueueItem>,
    pub total: usize,
    pub pending_count: usize,
    pub current_status: String,
}

#[derive(Template)]
#[template(path = "pages/collaboration/activity.html")]
pub(crate) struct ActivityPage {
    pub current_path: String,
    pub can_admin: bool,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,
    pub status_strip: crate::system_status::StatusStrip,
    pub items: Vec<ActivityItem>,
}

#[derive(Template)]
#[template(path = "pages/collaboration/supplier_risk.html")]
pub(crate) struct SupplierRiskPage {
    pub current_path: String,
    pub can_admin: bool,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,
    pub status_strip: crate::system_status::StatusStrip,
    pub entries: Vec<SupplierRiskItem>,
    pub total: usize,
    pub active_count: usize,
}

#[derive(Template)]
#[template(path = "pages/collaboration/pipeline.html")]
pub(crate) struct PipelinePage {
    pub current_path: String,
    pub can_admin: bool,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,
    pub status_strip: crate::system_status::StatusStrip,
    pub opportunities: Vec<PipelineOpportunityItem>,
    pub total: usize,
}

#[derive(Template)]
#[template(path = "pages/collaboration/evidence.html")]
pub(crate) struct EvidencePage {
    pub current_path: String,
    pub can_admin: bool,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,
    pub status_strip: crate::system_status::StatusStrip,
    pub items: Vec<EvidenceItem>,
    pub total: usize,
}

#[derive(Template)]
#[template(path = "pages/collaboration/team_assignments.html")]
pub(crate) struct TeamAssignmentsPage {
    pub current_path: String,
    pub can_admin: bool,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,
    pub status_strip: crate::system_status::StatusStrip,
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
        // false-success-classification: best-effort — optional/display value default; failure renders empty rather than asserting persistence
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
        can_admin: ctx.can_admin,
        status_strip: crate::system_status::StatusStrip::current(),
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
    pub can_admin: bool,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,
    pub status_strip: crate::system_status::StatusStrip,
    pub prefill_name: String,
    pub prefill_description: String,
    pub prefill_workspace_type: String,
    pub entity_id: String,
    pub signal_id: String,
    pub error_notice: Option<String>,
}

/// Query params used by "Start investigation" entry points to prefill the
/// workspace form from an entity dossier or a signal. `error` carries a
/// failure code back from `create_workspace` so the form can say what
/// happened instead of silently resetting.
#[derive(Debug, Default, Deserialize)]
pub struct WorkspaceNewQuery {
    pub entity_id: Option<String>,
    pub entity_name: Option<String>,
    pub signal_id: Option<String>,
    pub signal_title: Option<String>,
    pub error: Option<String>,
}

/// Canonical name/description for an investigation opened from a signal.
/// Shared by the one-click warning action and the prefilled workspace form so
/// both entry points produce the same workspace.
pub(crate) fn signal_investigation_fields(
    signal_title: &str,
    entity_name: Option<&str>,
) -> (String, String) {
    let title: String = signal_title.chars().take(90).collect();
    let name = format!("Investigate: {title}");
    let entity_suffix = match entity_name.map(str::trim).filter(|name| !name.is_empty()) {
        Some(entity_name) => format!(" on {entity_name}"),
        None => String::new(),
    };
    let description =
        format!("Investigation opened from the signal \"{signal_title}\"{entity_suffix}.");
    (name, description)
}

/// Workspace types the database accepts (migration 002 check constraint).
fn normalized_workspace_type(raw: &str, from_signal: bool) -> &'static str {
    // Signal-originated investigations are always incidents; otherwise honour
    // the submitted type and fall back to `structured` for unknown input.
    if from_signal {
        return "incident";
    }
    match raw.trim() {
        "ad-hoc" => "ad-hoc",
        "incident" => "incident",
        "ongoing" => "ongoing",
        _ => "structured",
    }
}

/// Workspace visibility values the database accepts (migration 002), with a
/// legacy `org` alias mapped to `organization`.
fn normalized_workspace_visibility(raw: &str) -> &'static str {
    match raw.trim() {
        "private" => "private",
        "organization" | "org" => "organization",
        "public" => "public",
        _ => "team",
    }
}

pub async fn new_workspace_page(
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
    Query(query): Query<WorkspaceNewQuery>,
) -> impl IntoResponse {
    let warning_count = store
        .count_warnings(&apex_store::postgres::WarningListFilters {
            acknowledged: Some(false),
            ..Default::default()
        })
        .await
        .unwrap_or(0);
    let ctx = PageContext::from_session(&session, "/workspaces/new", warning_count);

    let entity_id = query.entity_id.clone().unwrap_or_default();
    let signal_id = query.signal_id.clone().unwrap_or_default();
    let signal_title = query.signal_title.clone().unwrap_or_default();
    let entity_name = query.entity_name.clone().unwrap_or_default();
    let (prefill_name, prefill_description) = if !signal_title.is_empty() {
        signal_investigation_fields(
            &signal_title,
            Some(entity_name.as_str()).filter(|name| !name.is_empty()),
        )
    } else if !entity_name.is_empty() {
        (
            format!("{entity_name} investigation"),
            format!("Investigation opened from the {entity_name} entity dossier."),
        )
    } else {
        (String::new(), String::new())
    };
    let prefill_workspace_type = if !signal_id.is_empty() {
        "incident".to_string()
    } else {
        "structured".to_string()
    };
    let error_notice = query.error.as_deref().map(|code| match code {
        "create_failed" => {
            "Workspace could not be created. Check the name and type, then try again.".to_string()
        }
        other => format!("Workspace could not be created ({other}). Try again."),
    });

    let page = WorkspaceNewPage {
        current_path: ctx.current_path,
        can_admin: ctx.can_admin,
        status_strip: crate::system_status::StatusStrip::current(),
        username: ctx.username,
        warning_count: ctx.warning_count,
        theme: ctx.theme,
        prefill_name,
        prefill_description,
        prefill_workspace_type,
        entity_id,
        signal_id,
        error_notice,
    };

    render_template(&page)
}

/// POST /workspaces — create a new workspace.
pub async fn create_workspace(
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
    Form(form): Form<CreateWorkspaceForm>,
) -> impl IntoResponse {
    let from_signal = form
        .signal_id
        .as_deref()
        .map(|signal_id| !signal_id.trim().is_empty())
        // false-success-classification: best-effort — optional boolean default; absence is not a failure
        .unwrap_or(false);

    let mut tags: Vec<String> = form
        .tags
        .as_deref()
        .map(|s| s.split(',').map(|t| t.trim().to_string()).collect())
        .unwrap_or_default();
    if from_signal && !tags.iter().any(|tag| tag == "signal") {
        tags.push("signal".to_string());
    }

    // Entity focus keeps the decision loop closed: investigation started from
    // a signal or dossier stays linked to that entity (audit #27). A malformed
    // entity reference is a validation error — persisting it would create a
    // dangling focus that no dossier can ever resolve.
    let entity_focus = match form
        .entity_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(entity_id) => {
            if Uuid::parse_str(entity_id).is_err() {
                return Redirect::to("/workspaces/new?error=invalid_entity").into_response();
            }
            serde_json::json!([entity_id])
        }
        None => serde_json::json!([]),
    };
    let description = form.description.as_deref().filter(|d| !d.trim().is_empty());
    let description = match (&form.signal_id, description) {
        (Some(signal_id), Some(description)) if !signal_id.trim().is_empty() => {
            Some(format!("{description}\n\nOpened from signal {signal_id}."))
        }
        (Some(signal_id), None) if !signal_id.trim().is_empty() => {
            Some(format!("Opened from signal {signal_id}."))
        }
        (_, description) => description.map(ToOwned::to_owned),
    };

    match store
        .create_investigation_workspace(
            &form.name,
            description.as_deref(),
            normalized_workspace_type(&form.workspace_type, from_signal),
            &session.user_id,
            None, // team_id
            normalized_workspace_visibility(&form.visibility),
            &tags,
            &entity_focus,
        )
        .await
    {
        Ok(workspace) => Redirect::to(&format!("/workspaces/{}", workspace.id)).into_response(),
        Err(error) => {
            tracing::warn!(%error, "create_workspace failed");
            // Surface the failure on the form instead of pretending success,
            // and keep the entity/signal prefill so the retry is one click.
            let mut params: Vec<(&str, &str)> = vec![("error", "create_failed")];
            if let Some(entity_id) = form.entity_id.as_deref().filter(|v| !v.trim().is_empty()) {
                params.push(("entity_id", entity_id));
            }
            if let Some(signal_id) = form.signal_id.as_deref().filter(|v| !v.trim().is_empty()) {
                params.push(("signal_id", signal_id));
            }
            let query = url::form_urlencoded::Serializer::new(String::new())
                .extend_pairs(params)
                .finish();
            Redirect::to(&format!("/workspaces/new?{query}")).into_response()
        }
    }
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
        // false-success-classification: best-effort — optional/display value default; failure renders empty rather than asserting persistence
        .unwrap_or_default();
    let shares = store
        .list_investigation_shares(workspace_id)
        .await
        // false-success-classification: best-effort — optional/display value default; failure renders empty rather than asserting persistence
        .unwrap_or_default();
    let activity = store
        .list_activity_feed(Some(workspace_id), None, None, 50)
        .await
        // false-success-classification: best-effort — optional/display value default; failure renders empty rather than asserting persistence
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
        can_admin: ctx.can_admin,
        status_strip: crate::system_status::StatusStrip::current(),
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
    let uuid = match Uuid::parse_str(&id) {
        Ok(uuid) => uuid,
        Err(_) => {
            return (StatusCode::BAD_REQUEST, "Invalid workspace ID").into_response();
        }
    };

    let workspace = match store.get_investigation_workspace(uuid).await {
        Ok(Some(workspace)) => workspace,
        Ok(None) => {
            return (StatusCode::NOT_FOUND, "Workspace not found").into_response();
        }
        Err(error) => {
            tracing::error!(%error, workspace_id = %id, "close_workspace: load failed");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to close workspace",
            )
                .into_response();
        }
    };

    // Authoritative persistence: the redirect must not claim the workspace
    // was closed when the update failed.
    if let Err(error) = store
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
        .await
    {
        tracing::error!(%error, workspace_id = %id, "close_workspace: write failed");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to close workspace",
        )
            .into_response();
    }

    Redirect::to(&format!("/workspaces/{}", id)).into_response()
}

/// POST /workspaces/:id/assign — assign a user to a workspace.
pub async fn assign_user_to_workspace(
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
    Path(id): Path<String>,
    Form(form): Form<AssignUserForm>,
) -> impl IntoResponse {
    let uuid = match Uuid::parse_str(&id) {
        Ok(uuid) => uuid,
        Err(_) => {
            return (StatusCode::BAD_REQUEST, "Invalid workspace ID").into_response();
        }
    };
    if let Err(error) = store
        .create_workspace_assignment(uuid, &form.user_id, &form.role, &session.user_id)
        .await
    {
        // Authoritative persistence: never redirect as if the assignment
        // was stored when the write failed.
        tracing::error!(%error, workspace_id = %id, "assign_user_to_workspace: write failed");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to assign user to workspace",
        )
            .into_response();
    }
    Redirect::to(&format!("/workspaces/{}", id)).into_response()
}

/// POST /workspaces/:id/shares — share a workspace.
pub async fn share_workspace(
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
    Path(id): Path<String>,
    Form(form): Form<ShareWorkspaceForm>,
) -> impl IntoResponse {
    let uuid = match Uuid::parse_str(&id) {
        Ok(uuid) => uuid,
        Err(_) => {
            return (StatusCode::BAD_REQUEST, "Invalid workspace ID").into_response();
        }
    };
    let expires_at = match form
        .expires_at
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        Some(raw) => match chrono::NaiveDate::parse_from_str(raw, "%Y-%m-%d") {
            Ok(date) => date.and_hms_opt(0, 0, 0).map(|dt| {
                chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(dt, chrono::Utc)
            }),
            Err(_) => {
                return (StatusCode::BAD_REQUEST, "Invalid share expiry date").into_response();
            }
        },
        None => None,
    };
    // Authoritative persistence: a failed share write must not redirect as if
    // the workspace had been shared.
    if let Err(error) = store
        .create_investigation_share(
            uuid,
            &session.user_id,
            &form.shared_with,
            &form.share_type,
            &form.access_level,
            form.message.as_deref(),
            expires_at,
        )
        .await
    {
        tracing::error!(%error, workspace_id = %id, "share_workspace: write failed");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to share workspace",
        )
            .into_response();
    }
    Redirect::to(&format!("/workspaces/{}", id)).into_response()
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
        .list_priority_queue_items(&session.user_id, status_filter, 100)
        .await
        // false-success-classification: best-effort — optional/display value default; failure renders empty rather than asserting persistence
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
        can_admin: ctx.can_admin,
        status_strip: crate::system_status::StatusStrip::current(),
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
    // A malformed item reference is a validation error: substituting a fresh
    // UUID would queue a fabricated identity that no dossier can resolve.
    let item_id = match Uuid::parse_str(form.item_id.trim()) {
        Ok(uuid) => uuid,
        Err(_) => {
            return (StatusCode::BAD_REQUEST, "Invalid queue item ID").into_response();
        }
    };
    if let Err(error) = store
        .create_priority_queue_item(
            &session.user_id,
            &form.item_type,
            item_id,
            &form.item_title,
            form.priority,
            form.notes.as_deref(),
        )
        .await
    {
        tracing::error!(%error, item_type = %form.item_type, "add_to_queue: write failed");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to add item to queue",
        )
            .into_response();
    }
    Redirect::to("/queue").into_response()
}

/// POST /queue/:id/complete — mark queue item as completed.
pub async fn complete_queue_item(
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let uuid = match Uuid::parse_str(&id) {
        Ok(uuid) => uuid,
        Err(_) => {
            return (StatusCode::BAD_REQUEST, "Invalid queue item ID").into_response();
        }
    };
    // Authoritative persistence: a failed completion update must not redirect
    // as if the item had been completed.
    if let Err(error) = store
        .update_priority_queue_item_scoped(
            &session.user_id,
            session.role.as_str(),
            uuid,
            None,
            Some("completed"),
            None,
        )
        .await
    {
        tracing::error!(%error, queue_item_id = %id, "complete_queue_item: write failed");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to complete queue item",
        )
            .into_response();
    }
    Redirect::to("/queue").into_response()
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

    // A malformed workspace filter must not silently widen the feed to every
    // workspace; it is a validation error.
    let workspace_id = match params.workspace_id.as_deref() {
        Some(raw) => match Uuid::parse_str(raw) {
            Ok(uuid) => Some(uuid),
            Err(_) => {
                return (StatusCode::BAD_REQUEST, "Invalid workspace ID filter").into_response();
            }
        },
        None => None,
    };
    let limit = params.limit.unwrap_or(100);

    let items = store
        .list_activity_feed(workspace_id, params.team_id.as_deref(), None, limit)
        .await
        // false-success-classification: best-effort — optional/display value default; failure renders empty rather than asserting persistence
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
        can_admin: ctx.can_admin,
        status_strip: crate::system_status::StatusStrip::current(),
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
        // false-success-classification: best-effort — optional/display value default; failure renders empty rather than asserting persistence
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
        can_admin: ctx.can_admin,
        status_strip: crate::system_status::StatusStrip::current(),
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
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| serde_json::from_str(s).unwrap_or(serde_json::json!({"summary": s})))
        .unwrap_or(serde_json::json!({}));

    // `supplier_risk.supplier_id` is a VARCHAR reference (seeds use ids like
    // `sup-001`), so the contract is a non-empty trimmed id, not a UUID. The
    // trimmed value is what gets persisted, so a padded id can never create a
    // dangling reference.
    let supplier_id = form.supplier_id.trim();
    if supplier_id.is_empty() {
        return (StatusCode::BAD_REQUEST, "Supplier ID is required").into_response();
    }

    if let Err(error) = store
        .create_supplier_risk_entry(
            supplier_id,
            &form.risk_category,
            form.risk_score,
            &risk_factors,
            form.mitigation.as_deref(),
            form.owner_id.as_deref(),
        )
        .await
    {
        tracing::error!(%error, supplier_id = %supplier_id, "add_supplier_risk: write failed");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to save supplier risk entry",
        )
            .into_response();
    }

    Redirect::to("/supplier-risk").into_response()
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
        // false-success-classification: best-effort — optional/display value default; failure renders empty rather than asserting persistence
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
        can_admin: ctx.can_admin,
        status_strip: crate::system_status::StatusStrip::current(),
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
    let expected_close = match form
        .expected_close
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(raw) => match chrono::NaiveDate::parse_from_str(raw, "%Y-%m-%d") {
            Ok(date) => Some(date),
            Err(_) => {
                return (StatusCode::BAD_REQUEST, "Invalid expected close date").into_response();
            }
        },
        None => None,
    };

    // Authoritative persistence: a failed insert must not redirect as if the
    // opportunity had been created.
    if let Err(error) = store
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
        .await
    {
        tracing::error!(%error, title = %form.title, "create_pipeline_opportunity: write failed");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to create pipeline opportunity",
        )
            .into_response();
    }

    Redirect::to("/pipeline").into_response()
}

/// POST /pipeline/:id/stage — update pipeline stage.
pub async fn update_pipeline_stage(
    _session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
    Path(id): Path<String>,
    Form(form): Form<UpdateStageForm>,
) -> impl IntoResponse {
    let uuid = match Uuid::parse_str(&id) {
        Ok(uuid) => uuid,
        Err(_) => {
            return (StatusCode::BAD_REQUEST, "Invalid opportunity ID").into_response();
        }
    };
    if let Err(error) = store.update_pipeline_stage(uuid, &form.stage, None).await {
        // Authoritative persistence: the redirect must not claim the
        // stage change was stored when the write failed.
        tracing::error!(%error, opportunity_id = %id, "update_pipeline_stage: write failed");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to update pipeline stage",
        )
            .into_response();
    }
    Redirect::to("/pipeline").into_response()
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
        // false-success-classification: best-effort — optional/display value default; failure renders empty rather than asserting persistence
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
        can_admin: ctx.can_admin,
        status_strip: crate::system_status::StatusStrip::current(),
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
    // `source_evidence.entity_id` is a VARCHAR reference (seeds use ids like
    // `comp-001`), so the contract is a non-empty trimmed id, not a UUID.
    let entity_id = form.entity_id.trim();
    if entity_id.is_empty() {
        return (StatusCode::BAD_REQUEST, "Evidence entity ID is required").into_response();
    }

    // Authoritative persistence: a failed insert must not redirect as if the
    // evidence had been stored.
    if let Err(error) = store
        .create_source_evidence(
            &form.entity_type,
            entity_id,
            &form.evidence_type,
            &form.source_url,
            form.source_domain.as_deref(),
            form.source_name.as_deref(),
            form.reliability_score,
            form.excerpt.as_deref(),
            &serde_json::json!({}),
        )
        .await
    {
        tracing::error!(%error, entity_type = %form.entity_type, "add_evidence: write failed");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to save source evidence",
        )
            .into_response();
    }

    Redirect::to("/evidence").into_response()
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
        // false-success-classification: best-effort — optional/display value default; failure renders empty rather than asserting persistence
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
        can_admin: ctx.can_admin,
        status_strip: crate::system_status::StatusStrip::current(),
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
    // `team_assignments.entity_id` is a VARCHAR reference, so the contract is
    // a non-empty trimmed id, not a UUID.
    let entity_id = form.entity_id.trim();
    if entity_id.is_empty() {
        return (StatusCode::BAD_REQUEST, "Assignment entity ID is required").into_response();
    }

    // Authoritative persistence: a failed insert must not redirect as if the
    // assignment had been stored.
    if let Err(error) = store
        .create_team_assignment(
            &form.team_id,
            &form.team_name,
            &form.entity_type,
            entity_id,
            &session.user_id,
            &form.assigned_to,
            &form.role,
            form.notes.as_deref(),
        )
        .await
    {
        tracing::error!(%error, team_id = %form.team_id, "create_team_assignment: write failed");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to create team assignment",
        )
            .into_response();
    }

    Redirect::to("/team-assignments").into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signal_investigation_fields_are_shared_and_bounded() {
        let long_title = "x".repeat(200);
        let (name, description) =
            signal_investigation_fields(&long_title, Some("Northwind Power Systems"));
        assert!(name.starts_with("Investigate: "));
        assert_eq!(name.chars().count(), "Investigate: ".len() + 90);
        assert!(description.contains("Northwind Power Systems"));

        let (name_without_entity, description_without_entity) =
            signal_investigation_fields("Capacity alert", None);
        assert_eq!(name_without_entity, "Investigate: Capacity alert");
        assert!(!description_without_entity.contains(" on "));
    }

    #[test]
    fn workspace_types_and_visibility_are_normalized_to_db_values() {
        assert_eq!(
            normalized_workspace_type("investigation", false),
            "structured"
        );
        assert_eq!(normalized_workspace_type("garbage", false), "structured");
        assert_eq!(normalized_workspace_type("incident", false), "incident");
        // Signal-originated investigations are always incidents.
        assert_eq!(normalized_workspace_type("structured", true), "incident");

        assert_eq!(normalized_workspace_visibility("org"), "organization");
        assert_eq!(
            normalized_workspace_visibility("organization"),
            "organization"
        );
        assert_eq!(normalized_workspace_visibility("private"), "private");
        assert_eq!(normalized_workspace_visibility("garbage"), "team");
    }

    // ── Malformed-ID validation and write-failure surfacing ────────────
    //
    // The store below is lazy and points at a closed port: a handler that
    // validates before touching the store must return 400 without connecting,
    // and a handler that reaches the store must surface the failure (500)
    // instead of redirecting as success.

    fn lazy_store() -> Arc<PgStore> {
        let pool = sqlx::PgPool::connect_lazy("postgres://apex:apex@127.0.0.1:1/apex_unused_test")
            .expect("lazy pool construction never connects");
        Arc::new(PgStore::from_pool(pool))
    }

    fn test_session() -> WebSession {
        WebSession {
            user_id: apex_core::identity::UserId::new("test-user"),
            username: apex_core::identity::Username::new("tester"),
            role: crate::auth::ApiRole::Admin,
            session_version: 1,
            principal_id: uuid::Uuid::new_v4(),
            issued_at: 0,
            expires_at: None,
        }
    }

    fn add_to_queue_form(item_id: &str) -> AddToQueueForm {
        AddToQueueForm {
            item_type: "warning".to_string(),
            item_id: item_id.to_string(),
            item_title: "Test item".to_string(),
            priority: 10,
            notes: None,
        }
    }

    #[tokio::test]
    async fn add_to_queue_rejects_malformed_item_id() {
        let response = add_to_queue(
            Extension(test_session()),
            Extension(lazy_store()),
            Form(add_to_queue_form("not-a-uuid")),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn add_to_queue_surfaces_db_write_failure() {
        let response = add_to_queue(
            Extension(test_session()),
            Extension(lazy_store()),
            Form(add_to_queue_form(&uuid::Uuid::new_v4().to_string())),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn close_workspace_rejects_malformed_id() {
        let response = close_workspace(
            Extension(test_session()),
            Extension(lazy_store()),
            Path("not-a-uuid".to_string()),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn complete_queue_item_surfaces_db_write_failure() {
        let response = complete_queue_item(
            Extension(test_session()),
            Extension(lazy_store()),
            Path(uuid::Uuid::new_v4().to_string()),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    fn supplier_risk_form(supplier_id: &str) -> AddSupplierRiskForm {
        AddSupplierRiskForm {
            supplier_id: supplier_id.to_string(),
            risk_category: "financial".to_string(),
            risk_score: 0.5,
            risk_factors: None,
            mitigation: None,
            owner_id: None,
        }
    }

    fn evidence_form(entity_id: &str) -> AddEvidenceForm {
        AddEvidenceForm {
            entity_type: "company".to_string(),
            entity_id: entity_id.to_string(),
            evidence_type: "news".to_string(),
            source_url: "https://example.com".to_string(),
            source_domain: None,
            source_name: None,
            reliability_score: 0.5,
            excerpt: None,
        }
    }

    fn team_assignment_form(entity_id: &str) -> CreateTeamAssignmentForm {
        CreateTeamAssignmentForm {
            team_id: "team-1".to_string(),
            team_name: "Team One".to_string(),
            entity_type: "company".to_string(),
            entity_id: entity_id.to_string(),
            assigned_to: "analyst".to_string(),
            role: "contributor".to_string(),
            notes: None,
        }
    }

    #[tokio::test]
    async fn add_supplier_risk_rejects_empty_supplier_id() {
        let response = add_supplier_risk(
            Extension(test_session()),
            Extension(lazy_store()),
            Form(supplier_risk_form("   ")),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn add_supplier_risk_accepts_text_supplier_ids() {
        // The column and seed data use ids like `sup-001`; validation must be
        // non-empty, and the trimmed value reaches the store (which fails here
        // only because the lazy pool has no database).
        let response = add_supplier_risk(
            Extension(test_session()),
            Extension(lazy_store()),
            Form(supplier_risk_form("  sup-001  ")),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn add_evidence_rejects_empty_entity_id() {
        let response = add_evidence(
            Extension(test_session()),
            Extension(lazy_store()),
            Form(evidence_form("")),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn add_evidence_accepts_text_entity_ids() {
        let response = add_evidence(
            Extension(test_session()),
            Extension(lazy_store()),
            Form(evidence_form("comp-001")),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn create_team_assignment_rejects_empty_entity_id() {
        let response = create_team_assignment(
            Extension(test_session()),
            Extension(lazy_store()),
            Form(team_assignment_form("  ")),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn create_pipeline_opportunity_surfaces_db_write_failure() {
        let response = create_pipeline_opportunity(
            Extension(test_session()),
            Extension(lazy_store()),
            Form(CreatePipelineForm {
                title: "New opportunity".to_string(),
                stage: "discovery".to_string(),
                value_estimate: None,
                probability: 0.5,
                owner_id: None,
                expected_close: None,
                notes: None,
            }),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn create_pipeline_opportunity_rejects_malformed_close_date() {
        let response = create_pipeline_opportunity(
            Extension(test_session()),
            Extension(lazy_store()),
            Form(CreatePipelineForm {
                title: "New opportunity".to_string(),
                stage: "discovery".to_string(),
                value_estimate: None,
                probability: 0.5,
                owner_id: None,
                expected_close: Some("31/12/2026".to_string()),
                notes: None,
            }),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn create_workspace_rejects_malformed_entity_focus() {
        let response = create_workspace(
            Extension(test_session()),
            Extension(lazy_store()),
            Form(CreateWorkspaceForm {
                name: "Investigation".to_string(),
                description: None,
                workspace_type: "structured".to_string(),
                visibility: "team".to_string(),
                tags: None,
                entity_id: Some("not-a-uuid".to_string()),
                signal_id: None,
            }),
        )
        .await
        .into_response();
        assert!(response.status().is_redirection());
    }

    #[tokio::test]
    async fn share_workspace_rejects_malformed_expiry() {
        let response = share_workspace(
            Extension(test_session()),
            Extension(lazy_store()),
            Path(uuid::Uuid::new_v4().to_string()),
            Form(ShareWorkspaceForm {
                shared_with: "analyst".to_string(),
                share_type: "view".to_string(),
                access_level: "read".to_string(),
                message: None,
                expires_at: Some("31/12/2026".to_string()),
            }),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
