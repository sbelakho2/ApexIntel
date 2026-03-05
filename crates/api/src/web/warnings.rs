//! Warning handlers — GET /warnings (list), GET /warnings/:id (detail)
//!
//! Covers: warning list with filters/pagination, warning detail with
//! evidence timeline and acknowledgment status.

use std::sync::Arc;
use std::collections::BTreeMap;

use askama::Template;
use axum::{
    extract::Path,
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse},
    Extension,
};
use serde::Deserialize;
use uuid::Uuid;

use apex_store::postgres::{PgStore, WarningListFilters, WarningOrderBy};
use super::{is_htmx_request, PageContext};
use crate::middleware::session::WebSession;

// ─── Query params ───────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct WarningsQuery {
    pub page: Option<i64>,
    pub per_page: Option<i64>,
    pub severity: Option<String>,
    pub warning_type: Option<String>,
    pub region: Option<String>,
    pub status: Option<String>,
    pub q: Option<String>,
    pub sort: Option<String>,
    pub dir: Option<String>,
    pub acknowledged: Option<bool>,
}

// ─── Template data ──────────────────────────────────────────────────────────

/// One day's worth of warning counts for the trend chart.
#[derive(Clone, Debug)]
pub struct WarningTrendDay {
    pub date_label: String,
    pub critical: i64,
    pub high: i64,
    pub medium: i64,
    pub low: i64,
    pub total: i64,
    // precomputed for div-based chart
    pub bar_h: i64,        // 0-100 pct of max-day total
    pub critical_h: i64,   // 0-100 pct of this day's total
    pub high_h: i64,
    pub medium_h: i64,
    pub low_h: i64,
}

/// Row shown in the warnings list table.
#[derive(Clone, Debug)]
pub struct WarningListItem {
    pub id: String,
    pub title: String,
    pub severity: String,
    pub warning_type: String,
    pub company_name: String,
    pub region: String,
    pub confidence: f64,
    pub confidence_pct: i64,
    pub created_at: String,
    pub acknowledged: bool,
}

/// Evidence item shown on warning detail.
#[derive(Clone, Debug)]
pub struct EvidenceItem {
    pub id: String,
    pub source: String,
    pub url: String,
    pub snippet: String,
    pub found_at: String,
}

/// Related entity link shown on warning detail.
#[derive(Clone, Debug)]
pub struct RelatedEntity {
    pub kind: String,   // "company" | "person"
    pub id: String,
    pub name: String,
}

// ─── Templates ──────────────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "pages/warnings.html")]
pub struct WarningsListPage {
    // base
    pub current_path: String,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,
    // list-specific
    pub warnings: Vec<WarningListItem>,
    pub total: i64,
    pub page: i64,
    pub per_page: i64,
    pub total_pages: i64,
    pub active_severity: String,
    pub active_type: String,
    pub active_region: String,
    pub search_query: String,
    pub sort_field: String,
    pub sort_dir: String,
    pub critical_count: i64,
    pub high_count: i64,
    pub medium_count: i64,
    pub low_count: i64,
    pub warning_trend: Vec<WarningTrendDay>,
    pub active_status: String,
}

/// HTMX partial — just the results fragment (no base layout).
#[derive(Template)]
#[template(path = "pages/warnings/_list.html")]
pub struct WarningsListPartial {
    pub warnings: Vec<WarningListItem>,
    pub total: i64,
    pub page: i64,
    pub per_page: i64,
    pub total_pages: i64,
    pub active_severity: String,
    pub active_type: String,
    pub active_region: String,
    pub search_query: String,
    pub sort_field: String,
    pub sort_dir: String,
    pub critical_count: i64,
    pub high_count: i64,
    pub medium_count: i64,
    pub low_count: i64,
    pub warning_trend: Vec<WarningTrendDay>,
    pub active_status: String,
}

#[derive(Template)]
#[template(path = "pages/warning_detail.html")]
pub struct WarningDetailPage {
    // base
    pub current_path: String,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,
    // detail-specific
    pub id: String,
    pub title: String,
    pub severity: String,
    pub warning_type: String,
    pub description: String,
    pub company_name: String,
    pub company_id: String,
    pub region: String,
    pub confidence: f64,
    pub created_at: String,
    pub updated_at: String,
    pub acknowledged: bool,
    pub acknowledged_by: Option<String>,
    pub acknowledged_at: Option<String>,
    pub evidence: Vec<EvidenceItem>,
    pub related_entities: Vec<RelatedEntity>,
    pub ai_analysis: Option<String>,
}

// ─── Handlers ───────────────────────────────────────────────────────────────

/// GET /warnings — paginated, filterable warning list.
pub async fn list_warnings(
    headers: HeaderMap,
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
    axum::extract::Query(params): axum::extract::Query<WarningsQuery>,
) -> impl IntoResponse {
    let active_severity = params.severity.clone().unwrap_or_default();
    let active_type = params.warning_type.clone().unwrap_or_default();
    let active_region = params.region.clone().unwrap_or_default();
    let active_status = params.status.clone().unwrap_or_default();
    let search_query = params.q.clone().unwrap_or_default();
    let sort_field = params.sort.clone().unwrap_or_else(|| "created_at".into());
    let sort_dir_str = params.dir.clone().unwrap_or_else(|| "desc".into());

    let page = params.page.unwrap_or(1).max(1);
    let per_page = params.per_page.unwrap_or(25).clamp(1, 100);
    let offset = (page - 1) * per_page;

    let status_ack = match active_status.as_str() {
        "active" => Some(false),
        "acknowledged" | "resolved" => Some(true),
        _ => params.acknowledged,
    };

    let filters = WarningListFilters {
        regions: if active_region.is_empty() { vec![] } else { vec![active_region.clone()] },
        severities: if active_severity.is_empty() { vec![] } else { vec![active_severity.clone()] },
        warning_types: if active_type.is_empty() { vec![] } else { vec![active_type.clone()] },
        acknowledged: status_ack,
        search: if search_query.is_empty() { None } else { Some(search_query.clone()) },
        ..Default::default()
    };

    let order_by = match sort_field.as_str() {
        "severity" => Some(WarningOrderBy::Severity),
        "warning_type" => Some(WarningOrderBy::WarningType),
        _ => Some(WarningOrderBy::CreatedAt),
    };
    let desc = sort_dir_str != "asc";

    let total = store.count_warnings(&filters).await.unwrap_or_else(|e| {
        tracing::error!("Failed to count warnings: {e}");
        0
    });
    let total_pages = if total == 0 { 0 } else { (total + per_page - 1) / per_page };

    let warning_rows = store.list_warnings(&filters, order_by, desc, per_page, offset).await.unwrap_or_else(|e| {
        tracing::error!("Failed to list warnings: {e}");
        vec![]
    });

    let all_warning_rows = store.list_warnings(&filters, order_by, desc, 1500, 0).await.unwrap_or_else(|e| {
        tracing::error!("Failed to list warning aggregates: {e}");
        vec![]
    });

    let warnings: Vec<WarningListItem> = warning_rows.iter().map(|w| {
        WarningListItem {
            id: w.id.to_string(),
            title: w.title.clone(),
            severity: w.severity.clone(),
            warning_type: w.warning_type.clone(),
            company_name: String::new(),
            region: w.region.clone().unwrap_or_default(),
            confidence: w.confidence.unwrap_or(0.0),
            confidence_pct: confidence_to_pct(w.confidence.unwrap_or(0.0)),
            created_at: w.ts_utc.format("%Y-%m-%d %H:%M").to_string(),
            acknowledged: w.acknowledged,
        }
    }).collect();

    let critical_count = all_warning_rows.iter().filter(|w| w.severity == "critical" && !w.acknowledged).count() as i64;
    let high_count = all_warning_rows.iter().filter(|w| w.severity == "high" && !w.acknowledged).count() as i64;
    let medium_count = all_warning_rows.iter().filter(|w| w.severity == "medium" && !w.acknowledged).count() as i64;
    let low_count = all_warning_rows.iter().filter(|w| w.severity == "low" && !w.acknowledged).count() as i64;

    // Generate real 30-day trend data from warnings in DB.
    let warning_trend: Vec<WarningTrendDay> = {
        use chrono::{Duration, Utc};
        let mut by_day: BTreeMap<String, WarningTrendDay> = BTreeMap::new();
        for i in 0..30 {
            let label = (Utc::now() - Duration::days(29 - i)).format("%b %d").to_string();
            by_day.insert(label.clone(), WarningTrendDay {
                date_label: label,
                critical: 0,
                high: 0,
                medium: 0,
                low: 0,
                total: 0,
                bar_h: 0,
                critical_h: 0,
                high_h: 0,
                medium_h: 0,
                low_h: 0,
            });
        }
        for w in &all_warning_rows {
            let key = w.ts_utc.format("%b %d").to_string();
            if let Some(day) = by_day.get_mut(&key) {
                match w.severity.as_str() {
                    "critical" => day.critical += 1,
                    "high" => day.high += 1,
                    "medium" => day.medium += 1,
                    _ => day.low += 1,
                }
            }
        }
        let mut trend: Vec<WarningTrendDay> = by_day.into_values().collect();
        let max_total = trend.iter().map(|d| d.critical + d.high + d.medium + d.low).max().unwrap_or(1).max(1);
        for day in &mut trend {
            day.total = day.critical + day.high + day.medium + day.low;
            day.bar_h = day.total * 100 / max_total;
            if day.total > 0 {
                day.critical_h = day.critical * 100 / day.total;
                day.high_h = day.high * 100 / day.total;
                day.medium_h = day.medium * 100 / day.total;
                day.low_h = day.low * 100 / day.total;
            }
        }
        trend
    };

    let unack = store.count_warnings(&WarningListFilters { acknowledged: Some(false), ..Default::default() }).await.unwrap_or(0);
    let ctx = PageContext::from_session(&session, "/warnings", unack);

    let tpl = WarningsListPage {
        current_path: ctx.current_path,
        username: ctx.username,
        warning_count: ctx.warning_count,
        theme: ctx.theme,
        warnings,
        total,
        page,
        per_page,
        total_pages,
        active_severity,
        active_type,
        active_region,
        search_query,
        sort_field,
        sort_dir: sort_dir_str,
        critical_count,
        high_count,
        medium_count,
        low_count,
        warning_trend,
        active_status,
    };

    if is_htmx_request(&headers) {
        let partial = WarningsListPartial {
            warnings: tpl.warnings.clone(),
            total: tpl.total,
            page: tpl.page,
            per_page: tpl.per_page,
            total_pages: tpl.total_pages,
            active_severity: tpl.active_severity.clone(),
            active_type: tpl.active_type.clone(),
            active_region: tpl.active_region.clone(),
            search_query: tpl.search_query.clone(),
            sort_field: tpl.sort_field.clone(),
            sort_dir: tpl.sort_dir.clone(),
            critical_count: tpl.critical_count,
            high_count: tpl.high_count,
            medium_count: tpl.medium_count,
            low_count: tpl.low_count,
            warning_trend: tpl.warning_trend.clone(),
            active_status: tpl.active_status.clone(),
        };
        partial.into_response()
    } else {
        tpl.into_response()
    }
}

fn confidence_to_pct(value: f64) -> i64 {
    if value <= 1.0 {
        (value * 100.0).round() as i64
    } else {
        value.round() as i64
    }
}

/// GET /warnings/:id — single warning detail page.
pub async fn get_warning(
    _headers: HeaderMap,
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let unack = store.count_warnings(&WarningListFilters { acknowledged: Some(false), ..Default::default() }).await.unwrap_or(0);
    let ctx = PageContext::from_session(&session, "/warnings", unack);

    let uuid = match Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => {
            return super::errors::not_found_with_context(&ctx.username, "/warnings", ctx.warning_count);
        }
    };

    let warning = match store.get_warning(uuid).await {
        Ok(Some(w)) => w,
        Ok(None) => {
            return super::errors::not_found_with_context(&ctx.username, "/warnings", ctx.warning_count);
        }
        Err(e) => {
            tracing::error!("Failed to fetch warning {id}: {e}");
            return super::errors::not_found_with_context(&ctx.username, "/warnings", ctx.warning_count);
        }
    };

    // Build evidence from source URLs
    let evidence: Vec<EvidenceItem> = warning.source_urls.as_deref().unwrap_or(&[]).iter().enumerate().map(|(i, url)| {
        EvidenceItem {
            id: i.to_string(),
            source: url.split('/').nth(2).unwrap_or("unknown").to_string(),
            url: url.clone(),
            snippet: String::new(),
            found_at: warning.ts_utc.format("%Y-%m-%d %H:%M").to_string(),
        }
    }).collect();

    let tpl = WarningDetailPage {
        current_path: ctx.current_path,
        username: ctx.username,
        warning_count: ctx.warning_count,
        theme: ctx.theme,
        id: warning.id.to_string(),
        title: warning.title.clone(),
        severity: warning.severity.clone(),
        warning_type: warning.warning_type.clone(),
        description: warning.description.clone().unwrap_or_default(),
        company_name: String::new(),
        company_id: String::new(),
        region: warning.region.clone().unwrap_or_default(),
        confidence: warning.confidence.unwrap_or(0.0),
        created_at: warning.created_at.map(|d| d.format("%Y-%m-%d %H:%M").to_string()).unwrap_or_default(),
        updated_at: warning.updated_at.map(|d| d.format("%Y-%m-%d %H:%M").to_string()).unwrap_or_default(),
        acknowledged: warning.acknowledged,
        acknowledged_by: warning.acknowledged_by.clone(),
        acknowledged_at: warning.acknowledged_at.map(|d| d.format("%Y-%m-%d %H:%M").to_string()),
        evidence,
        related_entities: vec![],
        ai_analysis: None,
    };

    // For HTMX detail requests, still render the full template since it
    // replaces #main-results via hx-boost.
    tpl.into_response()
}

/// POST /warnings/:id/acknowledge — acknowledge a warning, return updated card HTML.
pub async fn acknowledge_warning_html(
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let uuid = match Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => return (StatusCode::BAD_REQUEST, Html("Invalid warning ID".to_string())).into_response(),
    };

    let result = store.acknowledge_warning(uuid, &session.username, None).await;
    match result {
        Ok(Some(true)) => {
            Html(format!(
                r#"<div class="apex-card p-4 border-green-500/30 bg-green-500/5">
                     <p class="text-sm font-bold text-green-600">Warning acknowledged by {}</p>
                     <p class="text-[10px] text-muted-foreground mt-1">The warning has been marked as acknowledged.</p>
                   </div>"#,
                session.username
            )).into_response()
        }
        Ok(Some(false)) => {
            Html(r#"<div class="apex-card p-4"><p class="text-sm text-muted-foreground">Already acknowledged</p></div>"#.to_string()).into_response()
        }
        Ok(None) => (StatusCode::NOT_FOUND, Html("Warning not found".to_string())).into_response(),
        Err(e) => {
            tracing::error!("Failed to acknowledge warning {id}: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, Html("Failed to acknowledge warning".to_string())).into_response()
        }
    }
}

/// GET /api/warnings/unread-count — returns the count of unacknowledged warnings as plain text.
pub async fn unread_count(
    _session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
) -> impl IntoResponse {
    let count = store.count_warnings(&WarningListFilters {
        acknowledged: Some(false),
        ..Default::default()
    }).await.unwrap_or(0);
    Html(format!("{}", count))
}
/// POST /warnings/:id/analyze — trigger AI analysis, return rendered panel.
pub async fn analyze_warning_html(
    _session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let uuid = match Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => return (StatusCode::BAD_REQUEST, Html("Invalid warning ID".to_string())).into_response(),
    };

    // Check the warning exists
    match store.get_warning(uuid).await {
        Ok(Some(w)) => {
            // Return a placeholder analysis panel — actual LLM analysis
            // is triggered via the API endpoint POST /api/warnings/:id/analyze
            Html(format!(
                r#"<div class="apex-card p-4" id="analysis-panel">
                     <h2 class="mb-2 text-xs font-black uppercase tracking-[0.12em]">AI Analysis</h2>
                     <p class="text-sm leading-relaxed text-muted-foreground">
                       Analysis in progress for "<strong>{}</strong>". This may take a few moments.
                       The analysis will evaluate the threat severity, assess affected entities,
                       and recommend response actions based on available intelligence.
                     </p>
                     <div class="mt-3 flex items-center gap-2 text-[10px] text-muted-foreground">
                       <span class="h-2 w-2 rounded-full bg-yellow-500 animate-pulse"></span>
                       Processing…
                     </div>
                   </div>"#,
                w.title
            )).into_response()
        }
        Ok(None) => (StatusCode::NOT_FOUND, Html("Warning not found".to_string())).into_response(),
        Err(e) => {
            tracing::error!("Failed to fetch warning for analysis {id}: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, Html("Failed to start analysis".to_string())).into_response()
        }
    }
}
