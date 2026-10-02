//! Battlecards web UI — list, detail, create/edit/delete, side-by-side
//! comparison and Markdown export, server-rendered via Askama + HTMX.
//!
//! Mutations are registered through `WebPages::post` (write-guarded); the
//! "Regenerate" button posts to the JSON API, which answers HTMX callers with
//! `HX-Refresh` so the page reloads with the regenerated sections.

use std::collections::HashMap;
use std::sync::Arc;

use askama::Template;
use axum::{
    extract::{Path, Query, RawQuery},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Redirect, Response},
    Extension, Form,
};
use serde::Deserialize;
use uuid::Uuid;

use super::{is_htmx_request, render_template, render_template_with_status, PageContext};
use crate::middleware::session::WebSession;
use crate::routes::battlecards::{
    company_names, export_filename, normalize_status, parse_compare_ids,
    render_comparison_markdown, render_markdown, section_lines, section_text, section_value,
    truncate_chars, validate_title, COMPARE_MIN, SECTIONS,
};
use apex_core::data_state::{DataState, DegradedNotice};
use apex_store::postgres::{
    BattlecardCompanyOption, BattlecardRow, BattlecardWriteOutcome, CreateBattlecardOutcome,
    PgStore, WarningListFilters, BATTLECARD_STATUSES,
};

const REQUEST_SCOPE: &str = "web-battlecards";
const LIST_PER_PAGE_DEFAULT: u32 = 50;
const LIST_PER_PAGE_MAX: u32 = 100;
/// Company picker size (the store caps lists at 500).
const COMPANY_OPTION_LIMIT: i64 = 500;
/// Maximum size of one edited section.
const SECTION_INPUT_MAX_BYTES: usize = 64 * 1024;
const COMPARE_CELL_MAX_CHARS: usize = 600;
const TIMESTAMP_FORMAT: &str = "%Y-%m-%d %H:%M UTC";

// ─── Shared view types ──────────────────────────────────────────────────────

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

/// The filterable part of the list page (also the HTMX partial).
#[derive(Clone, Debug)]
pub struct BattlecardListResults {
    pub battlecards: Vec<BattlecardListItem>,
    pub total: i64,
    pub page: i64,
    pub per_page: i64,
    pub active_status: String,
    /// Number of battlecard sections (for "n of total").
    pub section_total: usize,
    /// Set when a backing query failed, so a storage error never renders as
    /// "no battlecards".
    pub degraded_notice: Option<String>,
}

impl BattlecardListResults {
    pub fn has_prev(&self) -> bool {
        self.page > 1
    }

    pub fn has_next(&self) -> bool {
        self.total > self.page * self.per_page
    }

    pub fn can_compare(&self) -> bool {
        self.battlecards.len() >= COMPARE_MIN
    }
}

/// One section on the detail page.
#[derive(Clone, Debug)]
pub struct BattlecardSectionView {
    pub name: String,
    pub label: String,
    pub lines: Vec<String>,
}

/// One `<option>` in a company picker.
#[derive(Clone, Debug)]
pub struct CompanyOptionView {
    pub id: String,
    pub name: String,
    pub selected: bool,
}

/// A company `<select>`: tracked competitors first, then other companies.
#[derive(Clone, Debug, Default)]
pub struct CompanySelect {
    pub competitors: Vec<CompanyOptionView>,
    pub others: Vec<CompanyOptionView>,
}

/// One `<option>` in the status picker.
#[derive(Clone, Debug)]
pub struct StatusOption {
    pub value: String,
    pub label: String,
    pub selected: bool,
}

/// One section textarea in the editor.
#[derive(Clone, Debug)]
pub struct EditorSection {
    pub name: String,
    pub label: String,
    pub content: String,
}

#[derive(Clone, Debug)]
pub struct CompareCard {
    pub id: String,
    pub title: String,
    pub competitor_name: String,
    pub status: String,
}

#[derive(Clone, Debug)]
pub struct CompareCell {
    pub has_data: bool,
    pub value: String,
}

#[derive(Clone, Debug)]
pub struct CompareRow {
    pub label: String,
    pub cells: Vec<CompareCell>,
}

// ─── Templates ──────────────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "pages/battlecards/list.html")]
pub struct BattlecardsListPage {
    pub current_path: String,
    pub can_admin: bool,
    pub can_write: bool,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,
    pub status_strip: crate::system_status::StatusStrip,

    pub results: BattlecardListResults,
    pub count_all: i64,
    pub count_published: i64,
    pub count_draft: i64,
    pub count_archived: i64,
    pub notice: Option<String>,
}

/// HTMX fragment for status-chip and pagination swaps on the list page.
#[derive(Template)]
#[template(path = "pages/battlecards/_list.html")]
pub struct BattlecardsListPartial {
    pub results: BattlecardListResults,
}

#[derive(Template)]
#[template(path = "pages/battlecards/detail.html")]
pub struct BattlecardDetailPage {
    pub current_path: String,
    pub can_admin: bool,
    pub can_write: bool,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,
    pub status_strip: crate::system_status::StatusStrip,

    pub id: String,
    pub title: String,
    pub status: String,
    pub competitor_id: String,
    pub our_company_id: String,
    pub competitor_name: String,
    pub our_company_name: String,
    pub created_at: String,
    pub updated_at: String,
    pub regenerated_at: Option<String>,
    pub sections: Vec<BattlecardSectionView>,
    pub generated_count: usize,
    pub notice: Option<String>,
    pub degraded_notice: Option<String>,
}

#[derive(Template)]
#[template(path = "pages/battlecards/editor.html")]
pub struct BattlecardEditorPage {
    pub current_path: String,
    pub can_admin: bool,
    pub can_write: bool,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,
    pub status_strip: crate::system_status::StatusStrip,

    pub is_new: bool,
    pub battlecard_id: String,
    pub title: String,
    /// Optimistic-concurrency token (`updated_at` in µs) for edits.
    pub version: String,
    pub competitor_name: String,
    pub our_company_name: String,
    pub competitor_select: CompanySelect,
    pub our_company_select: CompanySelect,
    pub statuses: Vec<StatusOption>,
    pub sections: Vec<EditorSection>,
    pub error_notice: Option<String>,
}

#[derive(Template)]
#[template(path = "pages/battlecards/compare.html")]
pub struct BattlecardComparePage {
    pub current_path: String,
    pub can_admin: bool,
    pub can_write: bool,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,
    pub status_strip: crate::system_status::StatusStrip,

    pub battlecards: Vec<CompareCard>,
    pub rows: Vec<CompareRow>,
    pub export_href: String,
    pub error_notice: Option<String>,
}

// ─── Query / form types ─────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct BattlecardsQuery {
    pub status: Option<String>,
    pub page: Option<u32>,
    pub per_page: Option<u32>,
    pub notice: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct BattlecardDetailQuery {
    pub notice: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct NewBattlecardQuery {
    pub competitor_id: Option<String>,
    pub our_company_id: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct CreateBattlecardForm {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub our_company_id: String,
    #[serde(default)]
    pub competitor_id: String,
}

// ─── Helpers ────────────────────────────────────────────────────────────────

/// Load the unread-warning count for the navigation badge.
async fn nav_warning_count(store: &PgStore, session: &WebSession) -> Result<i64, Box<Response>> {
    store
        .count_warnings(&WarningListFilters {
            acknowledged: Some(false),
            ..Default::default()
        })
        .await
        .map_err(|error| {
            tracing::error!("count_warnings failed (web battlecards): {error:#}");
            Box::new(super::errors::internal_error_with_context(
                &session.username,
                0,
                "Failed to load navigation state",
                REQUEST_SCOPE,
            ))
        })
}

fn internal_error(session: &WebSession, warning_count: i64, message: &str) -> Response {
    super::errors::internal_error_with_context(
        &session.username,
        warning_count,
        message,
        REQUEST_SCOPE,
    )
}

/// Parse a path id and load the battlecard, mapping every failure to the
/// matching page (400 malformed id, 404 missing, 500 storage error).
async fn load_battlecard(
    store: &PgStore,
    session: &WebSession,
    raw_id: &str,
    path: &str,
    warning_count: i64,
) -> Result<BattlecardRow, Box<Response>> {
    // A malformed battlecard ID is a validation error: defaulting to the nil
    // UUID would query a fabricated identity that can never exist.
    let id = Uuid::parse_str(raw_id).map_err(|_| {
        Box::new((StatusCode::BAD_REQUEST, "Invalid battlecard ID").into_response())
    })?;
    match store.get_battlecard(id).await {
        Ok(Some(row)) => Ok(row),
        Ok(None) => Err(Box::new(super::errors::not_found_with_context(
            &session.username,
            path,
            warning_count,
        ))),
        Err(error) => {
            tracing::error!("get_battlecard {id} failed (web): {error:#}");
            Err(Box::new(internal_error(
                session,
                warning_count,
                "Failed to load battlecard",
            )))
        }
    }
}

fn status_label(status: &str) -> &'static str {
    match status {
        "published" => "Published",
        "draft" => "Draft",
        "archived" => "Archived",
        _ => "Unknown",
    }
}

fn status_options(selected: &str) -> Vec<StatusOption> {
    BATTLECARD_STATUSES
        .iter()
        .map(|status| StatusOption {
            value: (*status).to_string(),
            label: status_label(status).to_string(),
            selected: *status == selected,
        })
        .collect()
}

fn company_select(options: &[BattlecardCompanyOption], selected: Option<Uuid>) -> CompanySelect {
    let mut select = CompanySelect::default();
    for option in options {
        let view = CompanyOptionView {
            id: option.id.to_string(),
            name: option.name.clone(),
            selected: Some(option.id) == selected,
        };
        if option.is_competitor {
            select.competitors.push(view);
        } else {
            select.others.push(view);
        }
    }
    select
}

fn parse_optional_uuid(raw: Option<&str>) -> Option<Uuid> {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| Uuid::parse_str(value).ok())
}

/// Text shown in a section's editor textarea: prose sections verbatim,
/// structured sections as pretty-printed JSON, empty sections blank.
fn editor_text(value: Option<&serde_json::Value>) -> String {
    match value {
        None => String::new(),
        Some(serde_json::Value::String(text)) => text.clone(),
        Some(other) => serde_json::to_string_pretty(other).unwrap_or_else(|_| other.to_string()),
    }
}

/// Browsers submit textarea line breaks as CRLF.
fn normalize_newlines(text: &str) -> String {
    text.replace("\r\n", "\n")
}

/// Parse one submitted section. Blank clears the section (JSON `null` →
/// SQL `NULL`); input that starts like JSON must be valid JSON; anything else
/// is stored as a prose string.
fn parse_section_input(text: &str) -> Result<serde_json::Value, String> {
    let text = normalize_newlines(text);
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(serde_json::Value::Null);
    }
    if trimmed.len() > SECTION_INPUT_MAX_BYTES {
        return Err(format!(
            "must be at most {} KB",
            SECTION_INPUT_MAX_BYTES / 1024
        ));
    }
    if trimmed.starts_with(['{', '[', '"']) {
        return serde_json::from_str(trimmed).map_err(|error| format!("is not valid JSON ({error})"));
    }
    Ok(serde_json::Value::String(trimmed.to_string()))
}

/// Optimistic-concurrency token for the edit form.
fn version_token(row: &BattlecardRow) -> String {
    row.updated_at.timestamp_micros().to_string()
}

fn parse_version_token(raw: Option<&str>) -> Option<chrono::DateTime<chrono::Utc>> {
    raw.and_then(|value| value.trim().parse::<i64>().ok())
        .and_then(chrono::DateTime::from_timestamp_micros)
}

/// Changed sections in an edit submission, or per-section validation errors.
type SectionChanges = Vec<(&'static str, serde_json::Value)>;

fn collect_section_changes(
    row: &BattlecardRow,
    form: &HashMap<String, String>,
) -> Result<SectionChanges, Vec<String>> {
    let mut changes = Vec::new();
    let mut errors = Vec::new();
    for (name, label) in SECTIONS {
        // A missing field leaves the section untouched.
        let Some(submitted) = form.get(&format!("section_{name}")) else {
            continue;
        };
        let current = editor_text(section_value(row, name));
        if normalize_newlines(submitted).trim() == current.trim() {
            continue;
        }
        match parse_section_input(submitted) {
            Ok(value) => {
                let unchanged = match section_value(row, name) {
                    None => value.is_null(),
                    Some(existing) => *existing == value,
                };
                if !unchanged {
                    changes.push((*name, value));
                }
            }
            Err(message) => errors.push(format!("{label} {message}.")),
        }
    }
    if errors.is_empty() {
        Ok(changes)
    } else {
        Err(errors)
    }
}

fn detail_notice(code: Option<&str>) -> Option<String> {
    match code? {
        "created" => Some(
            "Battlecard created. Use Regenerate to fill its sections from stored data, or Edit to write them."
                .to_string(),
        ),
        "saved" => Some("Changes saved.".to_string()),
        "exists" => Some(
            "A battlecard for this company pair already exists — showing it instead.".to_string(),
        ),
        _ => None,
    }
}

fn list_notice(code: Option<&str>) -> Option<String> {
    match code? {
        "deleted" => Some("Battlecard deleted.".to_string()),
        _ => None,
    }
}

fn markdown_attachment(filename_stem: &str, body: String) -> Response {
    let disposition = format!("attachment; filename=\"{filename_stem}.md\"");
    let disposition = HeaderValue::from_str(&disposition)
        .unwrap_or_else(|_| HeaderValue::from_static("attachment; filename=\"battlecard.md\""));
    (
        StatusCode::OK,
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/markdown; charset=utf-8"),
            ),
            (header::CONTENT_DISPOSITION, disposition),
            (header::CACHE_CONTROL, HeaderValue::from_static("private, no-store")),
        ],
        body,
    )
        .into_response()
}

// ─── List ───────────────────────────────────────────────────────────────────

/// GET /battlecards — list page (HTMX requests get the results fragment).
pub async fn list_battlecards(
    Extension(store): Extension<Arc<PgStore>>,
    Query(query): Query<BattlecardsQuery>,
    Extension(session): Extension<WebSession>,
    headers: HeaderMap,
) -> Response {
    let mut degraded_notice: Option<String> = None;

    let warning_count_state = DataState::from_result(
        store
            .count_warnings(&WarningListFilters {
                acknowledged: Some(false),
                ..Default::default()
            })
            .await,
        "count_warnings failed (web battlecards list)",
        |_| false,
    );
    DegradedNotice::capture(&warning_count_state, &mut degraded_notice);
    let warning_count = warning_count_state.into_loaded_or(0);

    let page_u32 = query.page.unwrap_or(1).max(1);
    let per_page_u32 = query
        .per_page
        .unwrap_or(LIST_PER_PAGE_DEFAULT)
        .clamp(1, LIST_PER_PAGE_MAX);
    // Unknown statuses are ignored (show all) rather than matching nothing.
    let status = query.status.as_deref().and_then(normalize_status);

    let total_state = DataState::from_result(
        store.count_battlecards(status, None).await,
        "count_battlecards failed (web battlecards list)",
        |_| false,
    );
    DegradedNotice::capture(&total_state, &mut degraded_notice);
    let total = total_state.into_loaded_or(0);

    let rows_state = DataState::from_result(
        store
            .list_battlecards(status, None, page_u32, per_page_u32)
            .await,
        "list_battlecards failed (web battlecards list)",
        Vec::is_empty,
    );
    DegradedNotice::capture(&rows_state, &mut degraded_notice);
    let rows = rows_state.into_items();

    let competitor_ids: Vec<Uuid> = rows.iter().map(|row| row.competitor_id).collect();
    let names = match company_names(&store, &competitor_ids).await {
        Ok(names) => names,
        Err(error) => {
            tracing::error!("company_names failed (web battlecards list): {error:#}");
            degraded_notice.get_or_insert_with(|| {
                "Some battlecard data could not be loaded; the list may be incomplete.".to_string()
            });
            HashMap::new()
        }
    };

    let battlecards = rows
        .into_iter()
        .map(|row| BattlecardListItem {
            id: row.id.to_string(),
            section_count: SECTIONS
                .iter()
                .filter(|(name, _)| section_value(&row, name).is_some())
                .count(),
            competitor_name: names.get(&row.competitor_id).cloned().unwrap_or_default(),
            updated_at: row.updated_at.format(TIMESTAMP_FORMAT).to_string(),
            title: row.title,
            status: row.status,
        })
        .collect();

    let results = BattlecardListResults {
        battlecards,
        total,
        page: i64::from(page_u32),
        per_page: i64::from(per_page_u32),
        active_status: status.unwrap_or_default().to_string(),
        section_total: SECTIONS.len(),
        degraded_notice,
    };

    if is_htmx_request(&headers) {
        return render_template(&BattlecardsListPartial { results });
    }

    let mut results = results;
    let by_status = match store.count_battlecards_by_status().await {
        Ok(counts) => counts,
        Err(error) => {
            tracing::error!("count_battlecards_by_status failed (web): {error:#}");
            results.degraded_notice.get_or_insert_with(|| {
                "Some battlecard data could not be loaded; the list may be incomplete.".to_string()
            });
            Vec::new()
        }
    };
    let count_for = |wanted: &str| {
        by_status
            .iter()
            .filter(|(status, _)| status == wanted)
            .map(|(_, count)| *count)
            .sum::<i64>()
    };

    let ctx = PageContext::from_session(&session, "/battlecards", warning_count);
    let page = BattlecardsListPage {
        current_path: ctx.current_path,
        can_admin: ctx.can_admin,
        can_write: ctx.can_write,
        username: ctx.username,
        warning_count: ctx.warning_count,
        theme: ctx.theme,
        status_strip: ctx.status_strip,
        count_all: by_status.iter().map(|(_, count)| *count).sum(),
        count_published: count_for("published"),
        count_draft: count_for("draft"),
        count_archived: count_for("archived"),
        notice: list_notice(query.notice.as_deref()),
        results,
    };
    render_template(&page)
}

// ─── Detail ─────────────────────────────────────────────────────────────────

/// GET /battlecards/:id — detail page
pub async fn get_battlecard(
    Extension(store): Extension<Arc<PgStore>>,
    Path(id): Path<String>,
    Query(query): Query<BattlecardDetailQuery>,
    Extension(session): Extension<WebSession>,
) -> Response {
    let path = format!("/battlecards/{id}");
    let warning_count = match nav_warning_count(&store, &session).await {
        Ok(count) => count,
        Err(response) => return *response,
    };
    let row = match load_battlecard(&store, &session, &id, &path, warning_count).await {
        Ok(row) => row,
        Err(response) => return *response,
    };

    let mut degraded_notice = None;
    let names = match company_names(&store, &[row.competitor_id, row.our_company_id]).await {
        Ok(names) => names,
        Err(error) => {
            tracing::error!("company_names failed (web battlecard {}): {error:#}", row.id);
            degraded_notice = Some("Company names could not be loaded.".to_string());
            HashMap::new()
        }
    };

    let sections: Vec<BattlecardSectionView> = SECTIONS
        .iter()
        .map(|(name, label)| BattlecardSectionView {
            name: (*name).to_string(),
            label: (*label).to_string(),
            lines: section_value(&row, name)
                .map(section_lines)
                .unwrap_or_default(),
        })
        .collect();
    let generated_count = sections.iter().filter(|s| !s.lines.is_empty()).count();

    let ctx = PageContext::from_session(&session, &path, warning_count);
    let page = BattlecardDetailPage {
        current_path: ctx.current_path,
        can_admin: ctx.can_admin,
        can_write: ctx.can_write,
        username: ctx.username,
        warning_count: ctx.warning_count,
        theme: ctx.theme,
        status_strip: ctx.status_strip,
        id: row.id.to_string(),
        competitor_id: row.competitor_id.to_string(),
        our_company_id: row.our_company_id.to_string(),
        competitor_name: names.get(&row.competitor_id).cloned().unwrap_or_default(),
        our_company_name: names.get(&row.our_company_id).cloned().unwrap_or_default(),
        created_at: row.created_at.format(TIMESTAMP_FORMAT).to_string(),
        updated_at: row.updated_at.format(TIMESTAMP_FORMAT).to_string(),
        regenerated_at: row
            .regenerated_at
            .map(|at| at.format(TIMESTAMP_FORMAT).to_string()),
        title: row.title,
        status: row.status,
        sections,
        generated_count,
        notice: detail_notice(query.notice.as_deref()),
        degraded_notice,
    };
    render_template(&page)
}

// ─── Create ─────────────────────────────────────────────────────────────────

struct NewEditorValues {
    title: String,
    competitor_id: Option<Uuid>,
    our_company_id: Option<Uuid>,
}

/// Render the "new battlecard" editor. Prefilled companies that fall outside
/// the picker cap are fetched and added so the selection is never dropped.
async fn render_new_editor(
    store: &PgStore,
    session: &WebSession,
    warning_count: i64,
    mut values: NewEditorValues,
    error_notice: Option<String>,
    status: StatusCode,
) -> Response {
    let mut options = match store
        .list_battlecard_company_options(COMPANY_OPTION_LIMIT)
        .await
    {
        Ok(options) => options,
        Err(error) => {
            tracing::error!("list_battlecard_company_options failed (web): {error:#}");
            return internal_error(session, warning_count, "Failed to load companies");
        }
    };

    let missing: Vec<Uuid> = [values.competitor_id, values.our_company_id]
        .into_iter()
        .flatten()
        .filter(|id| !options.iter().any(|option| option.id == *id))
        .collect();
    if !missing.is_empty() {
        match company_names(store, &missing).await {
            Ok(names) => {
                for id in &missing {
                    if let Some(name) = names.get(id) {
                        options.push(BattlecardCompanyOption {
                            id: *id,
                            name: name.clone(),
                            is_competitor: false,
                        });
                    }
                }
            }
            Err(error) => {
                tracing::error!("company_names failed (web battlecard editor): {error:#}");
                return internal_error(session, warning_count, "Failed to load companies");
            }
        }
        // A prefilled id that matches no company is dropped, not kept as a
        // phantom selection.
        let known = |id: Option<Uuid>| id.filter(|id| options.iter().any(|o| o.id == *id));
        values.competitor_id = known(values.competitor_id);
        values.our_company_id = known(values.our_company_id);
    }

    let ctx = PageContext::from_session(session, "/battlecards/new", warning_count);
    let page = BattlecardEditorPage {
        current_path: ctx.current_path,
        can_admin: ctx.can_admin,
        can_write: ctx.can_write,
        username: ctx.username,
        warning_count: ctx.warning_count,
        theme: ctx.theme,
        status_strip: ctx.status_strip,
        is_new: true,
        battlecard_id: String::new(),
        title: values.title,
        version: String::new(),
        competitor_name: String::new(),
        our_company_name: String::new(),
        competitor_select: company_select(&options, values.competitor_id),
        our_company_select: company_select(&options, values.our_company_id),
        statuses: Vec::new(),
        sections: Vec::new(),
        error_notice,
    };
    render_template_with_status(status, &page)
}

/// GET /battlecards/new — creation form (`?competitor_id=` / `?our_company_id=`
/// prefill the pickers, e.g. from a company page).
pub async fn new_battlecard_page(
    Extension(store): Extension<Arc<PgStore>>,
    Query(query): Query<NewBattlecardQuery>,
    Extension(session): Extension<WebSession>,
) -> Response {
    if !session.can_write() {
        return super::errors::forbidden();
    }
    let warning_count = match nav_warning_count(&store, &session).await {
        Ok(count) => count,
        Err(response) => return *response,
    };
    let values = NewEditorValues {
        title: String::new(),
        competitor_id: parse_optional_uuid(query.competitor_id.as_deref()),
        our_company_id: parse_optional_uuid(query.our_company_id.as_deref()),
    };
    render_new_editor(&store, &session, warning_count, values, None, StatusCode::OK).await
}

/// POST /battlecards — create a draft battlecard.
pub async fn create_battlecard(
    Extension(store): Extension<Arc<PgStore>>,
    Extension(session): Extension<WebSession>,
    Form(form): Form<CreateBattlecardForm>,
) -> Response {
    let warning_count = match nav_warning_count(&store, &session).await {
        Ok(count) => count,
        Err(response) => return *response,
    };
    let values = NewEditorValues {
        title: form.title.trim().to_string(),
        competitor_id: parse_optional_uuid(Some(form.competitor_id.as_str())),
        our_company_id: parse_optional_uuid(Some(form.our_company_id.as_str())),
    };
    let reject = |values: NewEditorValues, message: &str, status: StatusCode| {
        render_new_editor(
            &store,
            &session,
            warning_count,
            values,
            Some(message.to_string()),
            status,
        )
    };

    let (Some(our_company_id), Some(competitor_id)) = (values.our_company_id, values.competitor_id)
    else {
        return reject(
            values,
            "Choose both your company and the competitor.",
            StatusCode::BAD_REQUEST,
        )
        .await;
    };
    if our_company_id == competitor_id {
        return reject(
            values,
            "Your company and the competitor must be different companies.",
            StatusCode::BAD_REQUEST,
        )
        .await;
    }
    let title = match validate_title(&form.title) {
        Ok(title) => title,
        Err(_) => {
            return reject(
                values,
                "Title is required and must be at most 255 characters.",
                StatusCode::BAD_REQUEST,
            )
            .await;
        }
    };

    match store
        .create_battlecard(our_company_id, competitor_id, &title)
        .await
    {
        Ok(CreateBattlecardOutcome::Created(id)) => {
            Redirect::to(&format!("/battlecards/{id}?notice=created")).into_response()
        }
        Ok(CreateBattlecardOutcome::Duplicate(id)) => {
            Redirect::to(&format!("/battlecards/{id}?notice=exists")).into_response()
        }
        Ok(CreateBattlecardOutcome::UnknownCompany) => {
            reject(
                values,
                "One of the selected companies no longer exists.",
                StatusCode::BAD_REQUEST,
            )
            .await
        }
        Err(error) => {
            tracing::error!("create_battlecard failed (web): {error:#}");
            reject(
                values,
                "The battlecard could not be saved. Try again.",
                StatusCode::INTERNAL_SERVER_ERROR,
            )
            .await
        }
    }
}

// ─── Edit ───────────────────────────────────────────────────────────────────

struct EditEditorValues {
    title: String,
    status: String,
    version: String,
    /// Section name → textarea content.
    sections: HashMap<String, String>,
}

impl EditEditorValues {
    fn from_row(row: &BattlecardRow) -> Self {
        Self {
            title: row.title.clone(),
            status: row.status.clone(),
            version: version_token(row),
            sections: SECTIONS
                .iter()
                .map(|(name, _)| ((*name).to_string(), editor_text(section_value(row, name))))
                .collect(),
        }
    }
}

async fn render_edit_editor(
    store: &PgStore,
    session: &WebSession,
    warning_count: i64,
    row: &BattlecardRow,
    values: EditEditorValues,
    error_notice: Option<String>,
    status: StatusCode,
) -> Response {
    let names = match company_names(store, &[row.competitor_id, row.our_company_id]).await {
        Ok(names) => names,
        Err(error) => {
            tracing::error!("company_names failed (web battlecard editor): {error:#}");
            return internal_error(session, warning_count, "Failed to load companies");
        }
    };
    let path = format!("/battlecards/{}/edit", row.id);
    let ctx = PageContext::from_session(session, &path, warning_count);
    let mut section_values = values.sections;
    let page = BattlecardEditorPage {
        current_path: ctx.current_path,
        can_admin: ctx.can_admin,
        can_write: ctx.can_write,
        username: ctx.username,
        warning_count: ctx.warning_count,
        theme: ctx.theme,
        status_strip: ctx.status_strip,
        is_new: false,
        battlecard_id: row.id.to_string(),
        title: values.title,
        version: values.version,
        competitor_name: names
            .get(&row.competitor_id)
            .cloned()
            .unwrap_or_else(|| "Unknown company".to_string()),
        our_company_name: names
            .get(&row.our_company_id)
            .cloned()
            .unwrap_or_else(|| "Unknown company".to_string()),
        competitor_select: CompanySelect::default(),
        our_company_select: CompanySelect::default(),
        statuses: status_options(&values.status),
        sections: SECTIONS
            .iter()
            .map(|(name, label)| EditorSection {
                name: (*name).to_string(),
                label: (*label).to_string(),
                content: section_values.remove(*name).unwrap_or_default(),
            })
            .collect(),
        error_notice,
    };
    render_template_with_status(status, &page)
}

/// GET /battlecards/:id/edit — edit form.
pub async fn edit_battlecard_page(
    Extension(store): Extension<Arc<PgStore>>,
    Path(id): Path<String>,
    Extension(session): Extension<WebSession>,
) -> Response {
    if !session.can_write() {
        return super::errors::forbidden();
    }
    let warning_count = match nav_warning_count(&store, &session).await {
        Ok(count) => count,
        Err(response) => return *response,
    };
    let path = format!("/battlecards/{id}/edit");
    let row = match load_battlecard(&store, &session, &id, &path, warning_count).await {
        Ok(row) => row,
        Err(response) => return *response,
    };
    let values = EditEditorValues::from_row(&row);
    render_edit_editor(
        &store,
        &session,
        warning_count,
        &row,
        values,
        None,
        StatusCode::OK,
    )
    .await
}

/// POST /battlecards/:id — save title, status and edited sections.
pub async fn update_battlecard(
    Extension(store): Extension<Arc<PgStore>>,
    Path(id): Path<String>,
    Extension(session): Extension<WebSession>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let warning_count = match nav_warning_count(&store, &session).await {
        Ok(count) => count,
        Err(response) => return *response,
    };
    let path = format!("/battlecards/{id}");
    let row = match load_battlecard(&store, &session, &id, &path, warning_count).await {
        Ok(row) => row,
        Err(response) => return *response,
    };

    // Re-render with what the user typed so a rejected save loses nothing.
    let submitted = || EditEditorValues {
        title: form.get("title").cloned().unwrap_or_default(),
        status: form
            .get("status")
            .and_then(|raw| normalize_status(raw))
            .unwrap_or(row.status.as_str())
            .to_string(),
        version: form.get("version").cloned().unwrap_or_default(),
        sections: SECTIONS
            .iter()
            .map(|(name, _)| {
                let key = format!("section_{name}");
                let content = form
                    .get(&key)
                    .map(|text| normalize_newlines(text))
                    .unwrap_or_else(|| editor_text(section_value(&row, name)));
                ((*name).to_string(), content)
            })
            .collect(),
    };

    let mut errors = Vec::new();
    let title = match validate_title(form.get("title").map(String::as_str).unwrap_or("")) {
        Ok(title) => Some(title),
        Err(_) => {
            errors.push("Title is required and must be at most 255 characters.".to_string());
            None
        }
    };
    let status = match form.get("status").and_then(|raw| normalize_status(raw)) {
        Some(status) => Some(status),
        None => {
            errors.push("Choose a valid status (draft, published or archived).".to_string());
            None
        }
    };
    let changes = match collect_section_changes(&row, &form) {
        Ok(changes) => changes,
        Err(section_errors) => {
            errors.extend(section_errors);
            Vec::new()
        }
    };
    let (Some(title), Some(status)) = (title, status) else {
        return render_edit_editor(
            &store,
            &session,
            warning_count,
            &row,
            submitted(),
            Some(errors.join(" ")),
            StatusCode::BAD_REQUEST,
        )
        .await;
    };
    if !errors.is_empty() {
        return render_edit_editor(
            &store,
            &session,
            warning_count,
            &row,
            submitted(),
            Some(errors.join(" ")),
            StatusCode::BAD_REQUEST,
        )
        .await;
    }

    let expected_updated_at = parse_version_token(form.get("version").map(String::as_str));
    if expected_updated_at.is_none() {
        return render_edit_editor(
            &store,
            &session,
            warning_count,
            &row,
            submitted(),
            Some("The form is missing its version marker; reload the editor and try again.".to_string()),
            StatusCode::BAD_REQUEST,
        )
        .await;
    }

    match store
        .update_battlecard_details(
            row.id,
            Some(&title),
            Some(status),
            &changes,
            session.user_id.as_str(),
            expected_updated_at,
        )
        .await
    {
        Ok(BattlecardWriteOutcome::Updated) => {
            Redirect::to(&format!("/battlecards/{}?notice=saved", row.id)).into_response()
        }
        Ok(BattlecardWriteOutcome::NotFound) => {
            super::errors::not_found_with_context(&session.username, &path, warning_count)
        }
        Ok(BattlecardWriteOutcome::Conflict) => {
            // Show the latest stored content (with a fresh version marker)
            // so the user can re-apply their edits deliberately.
            let latest = match store.get_battlecard(row.id).await {
                Ok(Some(latest)) => latest,
                Ok(None) => {
                    return super::errors::not_found_with_context(
                        &session.username,
                        &path,
                        warning_count,
                    )
                }
                Err(error) => {
                    tracing::error!("get_battlecard {} failed (web): {error:#}", row.id);
                    return internal_error(&session, warning_count, "Failed to load battlecard");
                }
            };
            let values = EditEditorValues::from_row(&latest);
            render_edit_editor(
                &store,
                &session,
                warning_count,
                &latest,
                values,
                Some(
                    "This battlecard changed after you opened the editor (for example it was regenerated). \
                     The latest version is shown below — re-apply your edits and save again."
                        .to_string(),
                ),
                StatusCode::CONFLICT,
            )
            .await
        }
        Err(error) => {
            tracing::error!("update_battlecard_details {} failed (web): {error:#}", row.id);
            render_edit_editor(
                &store,
                &session,
                warning_count,
                &row,
                submitted(),
                Some("Changes could not be saved. Try again.".to_string()),
                StatusCode::INTERNAL_SERVER_ERROR,
            )
            .await
        }
    }
}

// ─── Delete ─────────────────────────────────────────────────────────────────

/// POST /battlecards/:id/delete
pub async fn delete_battlecard(
    Extension(store): Extension<Arc<PgStore>>,
    Path(id): Path<String>,
    Extension(session): Extension<WebSession>,
) -> Response {
    let Ok(uid) = Uuid::parse_str(&id) else {
        return (StatusCode::BAD_REQUEST, "Invalid battlecard ID").into_response();
    };
    match store.delete_battlecard(uid).await {
        Ok(true) => Redirect::to("/battlecards?notice=deleted").into_response(),
        Ok(false) => super::errors::not_found_with_context(
            &session.username,
            &format!("/battlecards/{id}"),
            0,
        ),
        Err(error) => {
            tracing::error!("delete_battlecard {uid} failed (web): {error:#}");
            internal_error(&session, 0, "Failed to delete battlecard")
        }
    }
}

// ─── Export ─────────────────────────────────────────────────────────────────

/// GET /battlecards/:id/export — Markdown download.
pub async fn export_battlecard(
    Extension(store): Extension<Arc<PgStore>>,
    Path(id): Path<String>,
    Extension(session): Extension<WebSession>,
) -> Response {
    let path = format!("/battlecards/{id}/export");
    let row = match load_battlecard(&store, &session, &id, &path, 0).await {
        Ok(row) => row,
        Err(response) => return *response,
    };
    let names = match company_names(&store, &[row.competitor_id, row.our_company_id]).await {
        Ok(names) => names,
        Err(error) => {
            tracing::error!("company_names failed (web battlecard export): {error:#}");
            return internal_error(&session, 0, "Failed to export battlecard");
        }
    };
    let markdown = render_markdown(
        &row,
        names.get(&row.competitor_id).map(String::as_str),
        names.get(&row.our_company_id).map(String::as_str),
    );
    markdown_attachment(&export_filename(&row.title), markdown)
}

// ─── Compare ────────────────────────────────────────────────────────────────

enum CompareSelection {
    Loaded(Vec<BattlecardRow>, HashMap<Uuid, String>),
    Rejected(StatusCode, String),
    Failed,
}

async fn load_comparison(store: &PgStore, raw_query: Option<&str>) -> CompareSelection {
    let ids = match parse_compare_ids(raw_query) {
        Ok(ids) => ids,
        Err(message) => {
            let mut message = message.to_string();
            if let Some(first) = message.get_mut(0..1) {
                first.make_ascii_uppercase();
            }
            return CompareSelection::Rejected(StatusCode::BAD_REQUEST, format!("{message}."));
        }
    };
    let rows = match store.get_battlecards_by_ids(&ids).await {
        Ok(rows) => rows,
        Err(error) => {
            tracing::error!("get_battlecards_by_ids failed (web compare): {error:#}");
            return CompareSelection::Failed;
        }
    };
    if rows.len() != ids.len() {
        return CompareSelection::Rejected(
            StatusCode::NOT_FOUND,
            "One or more of the selected battlecards no longer exist.".to_string(),
        );
    }
    let competitor_ids: Vec<Uuid> = rows.iter().map(|row| row.competitor_id).collect();
    match company_names(store, &competitor_ids).await {
        Ok(names) => CompareSelection::Loaded(rows, names),
        Err(error) => {
            tracing::error!("company_names failed (web compare): {error:#}");
            CompareSelection::Failed
        }
    }
}

/// GET /battlecards/compare?ids=a,b — side-by-side comparison (2–4 cards).
pub async fn compare_battlecards(
    Extension(store): Extension<Arc<PgStore>>,
    RawQuery(raw_query): RawQuery,
    Extension(session): Extension<WebSession>,
) -> Response {
    let warning_count = match nav_warning_count(&store, &session).await {
        Ok(count) => count,
        Err(response) => return *response,
    };
    let ctx = PageContext::from_session(&session, "/battlecards/compare", warning_count);
    let mut page = BattlecardComparePage {
        current_path: ctx.current_path,
        can_admin: ctx.can_admin,
        can_write: ctx.can_write,
        username: ctx.username,
        warning_count: ctx.warning_count,
        theme: ctx.theme,
        status_strip: ctx.status_strip,
        battlecards: Vec::new(),
        rows: Vec::new(),
        export_href: String::new(),
        error_notice: None,
    };

    let (rows, names) = match load_comparison(&store, raw_query.as_deref()).await {
        CompareSelection::Loaded(rows, names) => (rows, names),
        CompareSelection::Rejected(status, message) => {
            page.error_notice = Some(message);
            return render_template_with_status(status, &page);
        }
        CompareSelection::Failed => {
            return internal_error(&session, warning_count, "Failed to load battlecards")
        }
    };

    page.export_href = format!(
        "/battlecards/compare/export?ids={}",
        rows.iter()
            .map(|row| row.id.to_string())
            .collect::<Vec<_>>()
            .join(",")
    );
    page.rows = SECTIONS
        .iter()
        .map(|(name, label)| CompareRow {
            label: (*label).to_string(),
            cells: rows
                .iter()
                .map(|row| match section_value(row, name) {
                    Some(value) => CompareCell {
                        has_data: true,
                        value: truncate_chars(&section_text(value), COMPARE_CELL_MAX_CHARS),
                    },
                    None => CompareCell {
                        has_data: false,
                        value: String::new(),
                    },
                })
                .collect(),
        })
        .collect();
    page.battlecards = rows
        .into_iter()
        .map(|row| CompareCard {
            id: row.id.to_string(),
            competitor_name: names.get(&row.competitor_id).cloned().unwrap_or_default(),
            title: row.title,
            status: row.status,
        })
        .collect();
    render_template(&page)
}

/// GET /battlecards/compare/export?ids=a,b — comparison as Markdown download.
pub async fn export_comparison(
    Extension(store): Extension<Arc<PgStore>>,
    RawQuery(raw_query): RawQuery,
    Extension(session): Extension<WebSession>,
) -> Response {
    match load_comparison(&store, raw_query.as_deref()).await {
        CompareSelection::Loaded(rows, names) => {
            let labelled: Vec<(&BattlecardRow, String)> = rows
                .iter()
                .map(|row| {
                    let label = match names.get(&row.competitor_id) {
                        Some(competitor) => format!("{} (vs {competitor})", row.title),
                        None => row.title.clone(),
                    };
                    (row, label)
                })
                .collect();
            markdown_attachment(
                "battlecard-comparison",
                render_comparison_markdown(&labelled),
            )
        }
        CompareSelection::Rejected(status, message) => (status, message).into_response(),
        CompareSelection::Failed => internal_error(&session, 0, "Failed to export comparison"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn session(role: crate::auth::ApiRole) -> WebSession {
        WebSession {
            user_id: apex_core::identity::UserId::new("test-user"),
            username: apex_core::identity::Username::new("tester"),
            role,
            session_version: 1,
            principal_id: Uuid::new_v4(),
            session_id: Uuid::new_v4(),
            issued_at: 0,
            expires_at: i64::MAX,
        }
    }

    fn unreachable_store() -> Arc<PgStore> {
        let pool = sqlx::PgPool::connect_lazy("postgres://apex:apex@127.0.0.1:1/apex_unused_test")
            .expect("lazy pool construction never connects");
        Arc::new(PgStore::from_pool(pool))
    }

    fn row() -> BattlecardRow {
        let now = chrono::Utc::now();
        BattlecardRow {
            id: Uuid::new_v4(),
            our_company_id: Uuid::new_v4(),
            competitor_id: Uuid::new_v4(),
            title: "Us vs Them".to_string(),
            status: "draft".to_string(),
            positioning: Some(json!({"summary": "We win on price"})),
            pricing: Some(json!("Per seat")),
            feature_matrix: None,
            strengths: None,
            weaknesses: Some(json!([])),
            objection_handlers: None,
            kill_shots: None,
            recent_news: None,
            win_loss: None,
            created_at: now,
            updated_at: now,
            updated_by: None,
            regenerated_at: None,
        }
    }

    #[tokio::test]
    async fn get_battlecard_rejects_malformed_id() {
        // The nav count fails first against the unreachable store, so probe
        // the id validation through the shared loader directly.
        let response = load_battlecard(
            &unreachable_store(),
            &session(crate::auth::ApiRole::Admin),
            "not-a-uuid",
            "/battlecards/not-a-uuid",
            0,
        )
        .await
        .expect_err("malformed id must be rejected");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn delete_rejects_malformed_id_without_touching_the_store() {
        let response = delete_battlecard(
            Extension(unreachable_store()),
            Path("nope".to_string()),
            Extension(session(crate::auth::ApiRole::Admin)),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn editor_pages_are_forbidden_for_read_only_roles() {
        let viewer = session(crate::auth::ApiRole::Viewer);
        let response = new_battlecard_page(
            Extension(unreachable_store()),
            Query(NewBattlecardQuery::default()),
            Extension(viewer.clone()),
        )
        .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let response = edit_battlecard_page(
            Extension(unreachable_store()),
            Path(Uuid::new_v4().to_string()),
            Extension(viewer),
        )
        .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn compare_export_rejects_bad_selection_before_querying() {
        let response = export_comparison(
            Extension(unreachable_store()),
            RawQuery(Some(format!("ids={}", Uuid::new_v4()))),
            Extension(session(crate::auth::ApiRole::Viewer)),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn section_input_parsing() {
        assert_eq!(parse_section_input("   \r\n").unwrap(), serde_json::Value::Null);
        assert_eq!(
            parse_section_input(" We win on service \r\n").unwrap(),
            json!("We win on service")
        );
        assert_eq!(
            parse_section_input("{\r\n  \"a\": [1, 2]\r\n}").unwrap(),
            json!({"a": [1, 2]})
        );
        assert!(parse_section_input("{not json").is_err());
        assert!(parse_section_input("[1,").is_err());
        let huge = format!("\"{}\"", "x".repeat(SECTION_INPUT_MAX_BYTES + 1));
        assert!(parse_section_input(&huge).is_err());
    }

    #[test]
    fn editor_text_round_trips_without_spurious_changes() {
        let row = row();
        let form: HashMap<String, String> = SECTIONS
            .iter()
            .map(|(name, _)| {
                (
                    format!("section_{name}"),
                    // Browsers send CRLF line breaks.
                    editor_text(section_value(&row, name)).replace('\n', "\r\n"),
                )
            })
            .collect();
        assert_eq!(collect_section_changes(&row, &form).unwrap(), Vec::new());
    }

    #[test]
    fn section_changes_cover_edit_clear_and_errors() {
        let row = row();
        let mut form = HashMap::new();
        form.insert("section_pricing".to_string(), String::new());
        form.insert("section_strengths".to_string(), "[\"Fast\"]".to_string());
        form.insert("section_weaknesses".to_string(), "   ".to_string());
        let changes = collect_section_changes(&row, &form).unwrap();
        assert_eq!(
            changes,
            vec![
                ("pricing", serde_json::Value::Null),
                ("strengths", json!(["Fast"])),
            ]
        );

        form.insert("section_kill_shots".to_string(), "{oops".to_string());
        let errors = collect_section_changes(&row, &form).unwrap_err();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].starts_with("Kill Shots is not valid JSON"));
    }

    #[test]
    fn version_token_round_trips() {
        let row = row();
        let parsed = parse_version_token(Some(&version_token(&row))).unwrap();
        assert_eq!(parsed.timestamp_micros(), row.updated_at.timestamp_micros());
        assert!(parse_version_token(Some("abc")).is_none());
        assert!(parse_version_token(None).is_none());
    }

    #[test]
    fn company_select_groups_competitors_and_marks_selection() {
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let options = vec![
            BattlecardCompanyOption {
                id: a,
                name: "Rival".to_string(),
                is_competitor: true,
            },
            BattlecardCompanyOption {
                id: b,
                name: "Partner".to_string(),
                is_competitor: false,
            },
        ];
        let select = company_select(&options, Some(b));
        assert_eq!(select.competitors.len(), 1);
        assert!(!select.competitors[0].selected);
        assert_eq!(select.others.len(), 1);
        assert!(select.others[0].selected);
    }

    #[test]
    fn status_options_mark_current_status() {
        let options = status_options("published");
        assert_eq!(
            options.iter().map(|o| o.value.as_str()).collect::<Vec<_>>(),
            vec!["draft", "published", "archived"]
        );
        assert!(options.iter().any(|o| o.value == "published" && o.selected));
        assert_eq!(options.iter().filter(|o| o.selected).count(), 1);
    }

    #[test]
    fn markdown_attachment_sets_download_headers() {
        let response = markdown_attachment("battlecard-x", "# x\n".to_string());
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "text/markdown; charset=utf-8"
        );
        assert_eq!(
            response.headers()[header::CONTENT_DISPOSITION],
            "attachment; filename=\"battlecard-x.md\""
        );
    }

    #[test]
    fn notices_map_known_codes_only() {
        assert!(detail_notice(Some("saved")).is_some());
        assert!(detail_notice(Some("<script>")).is_none());
        assert!(list_notice(Some("deleted")).is_some());
        assert!(list_notice(None).is_none());
    }
}
