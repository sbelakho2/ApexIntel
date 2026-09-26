//! Warning handlers — GET /warnings (list), GET /warnings/:id (detail)
//!
//! Covers: warning list with filters/pagination, warning detail with
//! evidence timeline and acknowledgment status.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use askama::Template;
use axum::{
    extract::Form,
    extract::Path,
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{Html, IntoResponse, Redirect},
    Extension,
};
use serde::Deserialize;
use url::form_urlencoded::byte_serialize;
use uuid::Uuid;

use super::{is_htmx_request, PageContext};
use crate::middleware::session::WebSession;
use apex_core::data_state::{DataState, DegradedNotice};
use apex_store::postgres::{PgStore, WarningListFilters, WarningOrderBy};

// ─── Query params ───────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct WarningsQuery {
    pub page: Option<i64>,
    pub per_page: Option<i64>,
    pub scope: Option<String>,
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
    pub bar_h: i64,      // 0-100 pct of max-day total
    pub critical_h: i64, // 0-100 pct of this day's total
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
    pub evidence_count: i64,
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

#[derive(Clone, Debug)]
pub struct WarningFilterChip {
    pub label: String,
    pub href: String,
    pub active: bool,
}

fn url_encode_component(input: &str) -> String {
    byte_serialize(input.as_bytes()).collect::<String>()
}

#[allow(clippy::too_many_arguments)]
fn build_warnings_href(
    scope: Option<&str>,
    severity: Option<&str>,
    status: Option<&str>,
    warning_type: Option<&str>,
    region: Option<&str>,
    q: Option<&str>,
    sort: Option<&str>,
    dir: Option<&str>,
) -> String {
    let mut params: Vec<String> = Vec::new();
    for (k, v) in [
        ("scope", scope),
        ("severity", severity),
        ("status", status),
        ("warning_type", warning_type),
        ("region", region),
        ("q", q),
        ("sort", sort),
        ("dir", dir),
    ] {
        if let Some(v) = v {
            let v = v.trim();
            if !v.is_empty() {
                params.push(format!("{}={}", k, url_encode_component(v)));
            }
        }
    }
    if params.is_empty() {
        "/warnings".to_string()
    } else {
        format!("/warnings?{}", params.join("&"))
    }
}

/// Related entity link shown on warning detail.
#[derive(Clone, Debug)]
pub struct RelatedEntity {
    pub kind: String, // "company" | "person"
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct AnalystNoteItem {
    pub author: String,
    pub body: String,
    pub created_at: String,
    pub visibility: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct WarningReviewForm {
    pub note: Option<String>,
    pub review_outcome: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct WarningNoteForm {
    pub body: String,
    pub tags: Option<String>,
    pub visibility: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct WarningDetailQuery {
    pub briefing: Option<bool>,
}

// ─── Templates ──────────────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "pages/warnings.html")]
pub struct WarningsListPage {
    // base
    pub current_path: String,
    pub can_admin: bool,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,
    pub status_strip: crate::system_status::StatusStrip,
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
    pub active_scope: String,
    pub severity_filters: Vec<WarningFilterChip>,
    pub status_filters: Vec<WarningFilterChip>,
    pub scope_filters: Vec<WarningFilterChip>,
    pub type_filters: Vec<WarningFilterChip>,
    pub region_filters: Vec<WarningFilterChip>,
    pub active_filters: i64,
    pub reset_href: String,
    pub page_base_href: String,
    /// Set when any backing query failed, so a storage error never renders as
    /// "no warnings".
    pub degraded_notice: Option<String>,
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
    pub active_scope: String,
    pub severity_filters: Vec<WarningFilterChip>,
    pub status_filters: Vec<WarningFilterChip>,
    pub scope_filters: Vec<WarningFilterChip>,
    pub type_filters: Vec<WarningFilterChip>,
    pub region_filters: Vec<WarningFilterChip>,
    pub active_filters: i64,
    pub reset_href: String,
    pub page_base_href: String,
    pub degraded_notice: Option<String>,
}

#[derive(Template)]
#[template(path = "pages/warning_detail.html")]
pub struct WarningDetailPage {
    // base
    pub current_path: String,
    pub can_admin: bool,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,
    pub status_strip: crate::system_status::StatusStrip,
    pub briefing_mode: bool,
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
    pub acknowledged_note: Option<String>,
    pub review_outcome: Option<String>,
    pub evidence: Vec<EvidenceItem>,
    pub related_entities: Vec<RelatedEntity>,
    pub annotations: Vec<AnalystNoteItem>,
    pub ai_analysis: Option<String>,
    /// Set when any backing query failed, so a storage error never renders as
    /// "no evidence" / "no notes".
    pub degraded_notice: Option<String>,
}

fn parse_tags(raw: Option<&str>) -> Vec<String> {
    raw.unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

// ─── Handlers ───────────────────────────────────────────────────────────────

/// GET /warnings — paginated, filterable warning list.
pub async fn list_warnings(
    headers: HeaderMap,
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
    axum::extract::Query(params): axum::extract::Query<WarningsQuery>,
) -> impl IntoResponse {
    let active_scope = match params.scope.as_deref() {
        Some("all") => "all".to_string(),
        _ => "focused".to_string(),
    };
    let active_severity = params.severity.clone().unwrap_or_default();
    let active_type = params.warning_type.clone().unwrap_or_default();
    let active_region = params.region.clone().unwrap_or_default();
    let active_status = params.status.clone().unwrap_or_default();
    let search_query = params.q.clone().unwrap_or_default();
    let sort_field = params.sort.clone().unwrap_or_else(|| "created_at".into());
    let sort_dir_str = params.dir.clone().unwrap_or_else(|| "desc".into());

    let severity_values = ["", "critical", "high", "medium", "low"];
    let status_values = ["", "active", "acknowledged", "resolved"];
    let scope_values = ["focused", "all"];

    let scope_filters = scope_values
        .iter()
        .map(|value| WarningFilterChip {
            label: if *value == "focused" {
                "Focused".to_string()
            } else {
                "All Signals".to_string()
            },
            href: build_warnings_href(
                Some(*value),
                if active_severity.is_empty() {
                    None
                } else {
                    Some(active_severity.as_str())
                },
                if active_status.is_empty() {
                    None
                } else {
                    Some(active_status.as_str())
                },
                if active_type.is_empty() {
                    None
                } else {
                    Some(active_type.as_str())
                },
                if active_region.is_empty() {
                    None
                } else {
                    Some(active_region.as_str())
                },
                if search_query.is_empty() {
                    None
                } else {
                    Some(search_query.as_str())
                },
                Some(sort_field.as_str()),
                Some(sort_dir_str.as_str()),
            ),
            active: active_scope == *value,
        })
        .collect::<Vec<_>>();

    let severity_filters = severity_values
        .iter()
        .map(|value| WarningFilterChip {
            label: if value.is_empty() { "All" } else { value }.to_string(),
            href: build_warnings_href(
                Some(active_scope.as_str()),
                if value.is_empty() { None } else { Some(*value) },
                if active_status.is_empty() {
                    None
                } else {
                    Some(active_status.as_str())
                },
                if active_type.is_empty() {
                    None
                } else {
                    Some(active_type.as_str())
                },
                if active_region.is_empty() {
                    None
                } else {
                    Some(active_region.as_str())
                },
                if search_query.is_empty() {
                    None
                } else {
                    Some(search_query.as_str())
                },
                Some(sort_field.as_str()),
                Some(sort_dir_str.as_str()),
            ),
            active: active_severity == *value,
        })
        .collect::<Vec<_>>();

    let status_filters = status_values
        .iter()
        .map(|value| WarningFilterChip {
            label: if value.is_empty() { "All" } else { value }.to_string(),
            href: build_warnings_href(
                Some(active_scope.as_str()),
                if active_severity.is_empty() {
                    None
                } else {
                    Some(active_severity.as_str())
                },
                if value.is_empty() { None } else { Some(*value) },
                if active_type.is_empty() {
                    None
                } else {
                    Some(active_type.as_str())
                },
                if active_region.is_empty() {
                    None
                } else {
                    Some(active_region.as_str())
                },
                if search_query.is_empty() {
                    None
                } else {
                    Some(search_query.as_str())
                },
                Some(sort_field.as_str()),
                Some(sort_dir_str.as_str()),
            ),
            active: active_status == *value,
        })
        .collect::<Vec<_>>();

    let active_filters = i64::from(!active_severity.is_empty())
        + i64::from(!active_status.is_empty())
        + i64::from(!search_query.is_empty())
        + i64::from(!active_type.is_empty())
        + i64::from(!active_region.is_empty());
    let reset_href = build_warnings_href(
        Some(active_scope.as_str()),
        None,
        None,
        None,
        None,
        None,
        Some(sort_field.as_str()),
        Some(sort_dir_str.as_str()),
    );
    let current_filters_href = build_warnings_href(
        Some(active_scope.as_str()),
        if active_severity.is_empty() {
            None
        } else {
            Some(active_severity.as_str())
        },
        if active_status.is_empty() {
            None
        } else {
            Some(active_status.as_str())
        },
        if active_type.is_empty() {
            None
        } else {
            Some(active_type.as_str())
        },
        if active_region.is_empty() {
            None
        } else {
            Some(active_region.as_str())
        },
        if search_query.is_empty() {
            None
        } else {
            Some(search_query.as_str())
        },
        None, // sort is appended by templates — avoid duplicate params
        None, // dir  is appended by templates — avoid duplicate params
    );
    let page_base_href = if current_filters_href.contains('?') {
        format!("{}&", current_filters_href)
    } else {
        format!("{}?", current_filters_href)
    };

    let page = params.page.unwrap_or(1).max(1);
    let per_page = params.per_page.unwrap_or(25).clamp(1, 100);
    let offset = (page - 1) * per_page;

    let status_ack = match active_status.as_str() {
        "active" => Some(false),
        "acknowledged" | "resolved" => Some(true),
        _ => params.acknowledged,
    };

    let filters = WarningListFilters {
        regions: if active_region.is_empty() {
            vec![]
        } else {
            vec![active_region.clone()]
        },
        severities: if active_severity.is_empty() {
            vec![]
        } else {
            vec![active_severity.clone()]
        },
        warning_types: if active_type.is_empty() {
            vec![]
        } else {
            vec![active_type.clone()]
        },
        acknowledged: status_ack,
        search: if search_query.is_empty() {
            None
        } else {
            Some(search_query.clone())
        },
        exclude_hygiene_signals: active_scope != "all",
        ..Default::default()
    };

    let order_by = match sort_field.as_str() {
        "severity" => Some(WarningOrderBy::Severity),
        "warning_type" => Some(WarningOrderBy::WarningType),
        _ => Some(WarningOrderBy::CreatedAt),
    };
    let desc = sort_dir_str != "asc";

    let mut degraded_notice: Option<String> = None;

    let total_state = DataState::from_result(
        store.count_warnings(&filters).await,
        "count_warnings failed (web warnings list)",
        |_| false,
    );
    DegradedNotice::capture(&total_state, &mut degraded_notice);
    let total = total_state.into_loaded_or(0);
    let total_pages = if total == 0 {
        0
    } else {
        (total + per_page - 1) / per_page
    };

    let warning_rows_state = DataState::from_result(
        store
            .list_warnings(&filters, order_by, desc, per_page, offset)
            .await,
        "list_warnings failed (web warnings list)",
        Vec::is_empty,
    );
    DegradedNotice::capture(&warning_rows_state, &mut degraded_notice);
    let warning_rows = warning_rows_state.into_items();

    let all_warning_rows_state = DataState::from_result(
        store.list_warnings(&filters, order_by, desc, 1500, 0).await,
        "list_warnings (aggregates) failed (web warnings list)",
        Vec::is_empty,
    );
    DegradedNotice::capture(&all_warning_rows_state, &mut degraded_notice);
    let all_warning_rows = all_warning_rows_state.into_items();

    // Resolve entity_ids → company names in one batch query
    let all_entity_ids: Vec<Uuid> = warning_rows
        .iter()
        .flat_map(|w| w.entity_ids.as_deref().unwrap_or_default().iter().copied())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    let company_name_map: HashMap<Uuid, String> = if all_entity_ids.is_empty() {
        HashMap::new()
    } else {
        let company_names_state = DataState::from_result(
            store.get_company_names_by_ids(&all_entity_ids).await,
            "get_company_names_by_ids failed (web warnings list)",
            Vec::is_empty,
        );
        DegradedNotice::capture(&company_names_state, &mut degraded_notice);
        company_names_state
            .into_items()
            .into_iter()
            .map(|(id, name, _, _)| (id, name))
            .collect()
    };

    let warnings: Vec<WarningListItem> = warning_rows
        .iter()
        .map(|w| {
            let company_name = w
                .entity_ids
                .as_deref()
                .unwrap_or_default()
                .iter()
                .filter_map(|eid| company_name_map.get(eid))
                .next()
                .cloned()
                .unwrap_or_default();
            WarningListItem {
                id: w.id.to_string(),
                title: w.title.clone(),
                severity: w.severity.clone(),
                warning_type: w.warning_type.clone(),
                company_name,
                region: w.region.clone().unwrap_or_default(),
                confidence: w.confidence.unwrap_or(0.0),
                confidence_pct: confidence_to_pct(w.confidence.unwrap_or(0.0)),
                created_at: w.ts_utc.format("%Y-%m-%d %H:%M").to_string(),
                acknowledged: w.acknowledged,
                evidence_count: w.source_urls.as_ref().map_or(0, |v| v.len() as i64),
            }
        })
        .collect();

    let critical_count = all_warning_rows
        .iter()
        .filter(|w| w.severity == "critical" && !w.acknowledged)
        .count() as i64;
    let high_count = all_warning_rows
        .iter()
        .filter(|w| w.severity == "high" && !w.acknowledged)
        .count() as i64;
    let medium_count = all_warning_rows
        .iter()
        .filter(|w| w.severity == "medium" && !w.acknowledged)
        .count() as i64;
    let low_count = all_warning_rows
        .iter()
        .filter(|w| w.severity == "low" && !w.acknowledged)
        .count() as i64;

    // Build dynamic type filter chips from actual warning types in result set
    let mut seen_types: Vec<String> = all_warning_rows
        .iter()
        .map(|w| w.warning_type.clone())
        .collect::<std::collections::HashSet<String>>()
        .into_iter()
        .collect();
    seen_types.sort();
    let mut type_filter_values: Vec<&str> = vec![""];
    for t in &seen_types {
        type_filter_values.push(t.as_str());
    }
    let type_filters = type_filter_values
        .iter()
        .map(|value| WarningFilterChip {
            label: if value.is_empty() {
                "All".to_string()
            } else {
                value.replace('_', " ")
            },
            href: build_warnings_href(
                Some(active_scope.as_str()),
                if active_severity.is_empty() {
                    None
                } else {
                    Some(active_severity.as_str())
                },
                if active_status.is_empty() {
                    None
                } else {
                    Some(active_status.as_str())
                },
                if value.is_empty() { None } else { Some(*value) },
                if active_region.is_empty() {
                    None
                } else {
                    Some(active_region.as_str())
                },
                if search_query.is_empty() {
                    None
                } else {
                    Some(search_query.as_str())
                },
                Some(sort_field.as_str()),
                Some(sort_dir_str.as_str()),
            ),
            active: active_type == *value,
        })
        .collect::<Vec<_>>();

    // Build dynamic region filter chips
    let mut seen_regions: Vec<String> = all_warning_rows
        .iter()
        .filter_map(|w| w.region.as_ref())
        .filter(|r: &&String| !r.is_empty())
        .cloned()
        .collect::<std::collections::HashSet<String>>()
        .into_iter()
        .collect();
    seen_regions.sort();
    let mut region_filter_values: Vec<&str> = vec![""];
    for r in &seen_regions {
        region_filter_values.push(r.as_str());
    }
    let region_filters = region_filter_values
        .iter()
        .map(|value| WarningFilterChip {
            label: if value.is_empty() {
                "All".to_string()
            } else {
                value.to_string()
            },
            href: build_warnings_href(
                Some(active_scope.as_str()),
                if active_severity.is_empty() {
                    None
                } else {
                    Some(active_severity.as_str())
                },
                if active_status.is_empty() {
                    None
                } else {
                    Some(active_status.as_str())
                },
                if active_type.is_empty() {
                    None
                } else {
                    Some(active_type.as_str())
                },
                if value.is_empty() { None } else { Some(*value) },
                if search_query.is_empty() {
                    None
                } else {
                    Some(search_query.as_str())
                },
                Some(sort_field.as_str()),
                Some(sort_dir_str.as_str()),
            ),
            active: active_region == *value,
        })
        .collect::<Vec<_>>();

    // Generate real 30-day trend data from warnings in DB.
    let warning_trend: Vec<WarningTrendDay> = {
        use chrono::{Duration, Utc};
        let mut by_day: BTreeMap<String, WarningTrendDay> = BTreeMap::new();
        for i in 0..30 {
            let label = (Utc::now() - Duration::days(29 - i))
                .format("%b %d")
                .to_string();
            by_day.insert(
                label.clone(),
                WarningTrendDay {
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
                },
            );
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
        let max_total = trend
            .iter()
            .map(|d| d.critical + d.high + d.medium + d.low)
            .max()
            .unwrap_or(1)
            .max(1);
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

    let unack_state = DataState::from_result(
        store
            .count_warnings(&WarningListFilters {
                acknowledged: Some(false),
                ..Default::default()
            })
            .await,
        "count_warnings (unacked) failed (web warnings list)",
        |_| false,
    );
    DegradedNotice::capture(&unack_state, &mut degraded_notice);
    let unack = unack_state.into_loaded_or(0);
    let ctx = PageContext::from_session(&session, "/warnings", unack);

    let tpl = WarningsListPage {
        current_path: ctx.current_path,
        can_admin: ctx.can_admin,
        username: ctx.username,
        warning_count: ctx.warning_count,
        theme: ctx.theme,
        status_strip: ctx.status_strip,
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
        active_scope,
        severity_filters,
        status_filters,
        scope_filters,
        type_filters,
        region_filters,
        active_filters,
        reset_href,
        page_base_href,
        degraded_notice: degraded_notice.clone(),
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
            active_scope: tpl.active_scope.clone(),
            severity_filters: tpl.severity_filters.clone(),
            status_filters: tpl.status_filters.clone(),
            scope_filters: tpl.scope_filters.clone(),
            type_filters: tpl.type_filters.clone(),
            region_filters: tpl.region_filters.clone(),
            active_filters: tpl.active_filters,
            reset_href: tpl.reset_href.clone(),
            page_base_href: tpl.page_base_href.clone(),
            degraded_notice,
        };
        super::render_template(&partial)
    } else {
        super::render_template(&tpl)
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
    axum::extract::Query(query): axum::extract::Query<WarningDetailQuery>,
) -> impl IntoResponse {
    let mut degraded_notice: Option<String> = None;
    let unack_state = DataState::from_result(
        store
            .count_warnings(&WarningListFilters {
                acknowledged: Some(false),
                ..Default::default()
            })
            .await,
        "count_warnings (unacked) failed (web warning detail)",
        |_| false,
    );
    DegradedNotice::capture(&unack_state, &mut degraded_notice);
    let unack = unack_state.into_loaded_or(0);
    let ctx = PageContext::from_session(&session, "/warnings", unack);

    let uuid = match Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => {
            return super::errors::not_found_with_context(
                &ctx.username,
                "/warnings",
                ctx.warning_count,
            );
        }
    };

    let warning = match store.get_warning(uuid).await {
        Ok(Some(w)) => w,
        Ok(None) => {
            return super::errors::not_found_with_context(
                &ctx.username,
                "/warnings",
                ctx.warning_count,
            );
        }
        Err(e) => {
            tracing::error!("Failed to fetch warning {id}: {e}");
            return super::errors::not_found_with_context(
                &ctx.username,
                "/warnings",
                ctx.warning_count,
            );
        }
    };

    // Build evidence from source URLs
    let evidence: Vec<EvidenceItem> = warning
        .source_urls
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .enumerate()
        .map(|(i, url)| EvidenceItem {
            id: i.to_string(),
            source: url.split('/').nth(2).unwrap_or("unknown").to_string(),
            url: url.clone(),
            snippet: String::new(),
            found_at: warning.ts_utc.format("%Y-%m-%d %H:%M").to_string(),
        })
        .collect();

    let annotations_state = DataState::from_result(
        store
            .list_annotations_scoped(
                &session.username,
                session.role.as_str(),
                Some("warning"),
                Some(&id),
            )
            .await,
        "list_annotations failed (web warning detail)",
        Vec::is_empty,
    );
    DegradedNotice::capture(&annotations_state, &mut degraded_notice);
    let annotations = annotations_state
        .into_items()
        .into_iter()
        .map(|annotation| AnalystNoteItem {
            author: annotation.user_id,
            body: annotation.body,
            created_at: annotation.updated_at.format("%Y-%m-%d %H:%M").to_string(),
            visibility: annotation.visibility,
            tags: annotation.tags,
        })
        .collect();

    // Resolve entity_ids → company names for detail page
    let entity_ids_slice = warning.entity_ids.as_deref().unwrap_or_default();
    let company_rows = if entity_ids_slice.is_empty() {
        vec![]
    } else {
        let company_rows_state = DataState::from_result(
            store.get_company_names_by_ids(entity_ids_slice).await,
            "get_company_names_by_ids failed (web warning detail)",
            Vec::is_empty,
        );
        DegradedNotice::capture(&company_rows_state, &mut degraded_notice);
        company_rows_state.into_items()
    };
    let primary_company_name = company_rows
        .first()
        .map(|(_, n, _, _)| n.clone())
        .unwrap_or_default();
    let primary_company_id = company_rows
        .first()
        .map(|(id, _, _, _)| id.to_string())
        .unwrap_or_default();
    let related_entities: Vec<RelatedEntity> = company_rows
        .iter()
        .map(|(cid, name, _, _)| RelatedEntity {
            kind: "company".to_string(),
            id: cid.to_string(),
            name: name.clone(),
        })
        .collect();

    let tpl = WarningDetailPage {
        current_path: ctx.current_path,
        can_admin: ctx.can_admin,
        username: ctx.username,
        warning_count: ctx.warning_count,
        theme: ctx.theme,
        status_strip: ctx.status_strip,
        briefing_mode: query.briefing.unwrap_or(false),
        id: warning.id.to_string(),
        title: warning.title.clone(),
        severity: warning.severity.clone(),
        warning_type: warning.warning_type.clone(),
        description: warning.description.clone().unwrap_or_default(),
        company_name: primary_company_name,
        company_id: primary_company_id,
        region: warning.region.clone().unwrap_or_default(),
        confidence: warning.confidence.unwrap_or(0.0),
        created_at: warning
            .created_at
            .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_default(),
        updated_at: warning
            .updated_at
            .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_default(),
        acknowledged: warning.acknowledged,
        acknowledged_by: warning.acknowledged_by.clone(),
        acknowledged_at: warning
            .acknowledged_at
            .map(|d| d.format("%Y-%m-%d %H:%M").to_string()),
        acknowledged_note: warning.acknowledged_note.clone(),
        review_outcome: warning.review_outcome.clone(),
        evidence,
        related_entities,
        annotations,
        ai_analysis: None,
        degraded_notice,
    };

    // For HTMX detail requests, still render the full template since it
    // replaces #main-results via hx-boost.
    super::render_template(&tpl)
}

/// POST /warnings/:id/acknowledge — acknowledge a warning, return updated card HTML.
pub async fn acknowledge_warning_html(
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let uuid = match Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Html("Invalid warning ID".to_string()),
            )
                .into_response()
        }
    };

    let result = store
        .acknowledge_warning(uuid, &session.username, None, None)
        .await;
    match result {
        Ok(apex_store::postgres::AcknowledgeWarningResult::Acknowledged)
        | Ok(apex_store::postgres::AcknowledgeWarningResult::ReviewedExisting) => {
            let _ = store
                .create_notification(
                    &session.username,
                    "warning_review",
                    "Warning acknowledged",
                    &format!("Warning {} was acknowledged by {}.", id, session.username),
                    Some("warning"),
                    Some(&id),
                    Some(&format!("/warnings/{id}")),
                )
                .await;
            let mut headers = HeaderMap::new();
            headers.insert("HX-Trigger", HeaderValue::from_static("warning-acknowledged"));
            (headers, Html(format!(
                r#"<div class="apex-card p-4 border-rams-green/30 bg-rams-green/5">
                     <p class="text-sm font-bold text-rams-green">Warning acknowledged by {}</p>
                     <p class="text-[10px] text-muted-foreground mt-1">The warning has been marked as acknowledged.</p>
                   </div>"#,
                super::escape_html(&session.username)
            ))).into_response()
        }
        Ok(apex_store::postgres::AcknowledgeWarningResult::AlreadyAcknowledged) => {            Html(r#"<div class="apex-card p-4"><p class="text-sm text-muted-foreground">Already acknowledged</p></div>"#.to_string()).into_response()
        }
        Ok(apex_store::postgres::AcknowledgeWarningResult::NotFound) => {
            (StatusCode::NOT_FOUND, Html("Warning not found".to_string())).into_response()
        }
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
    let count = store
        .count_warnings(&WarningListFilters {
            acknowledged: Some(false),
            ..Default::default()
        })
        .await
        .unwrap_or(0);
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
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Html("Invalid warning ID".to_string()),
            )
                .into_response()
        }
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
                       <span class="h-2 w-2 rounded-full bg-rams-orange animate-pulse"></span>
                       Processing…
                     </div>
                   </div>"#,
                super::escape_html(&w.title)
            )).into_response()
        }
        Ok(None) => (StatusCode::NOT_FOUND, Html("Warning not found".to_string())).into_response(),
        Err(e) => {
            tracing::error!("Failed to fetch warning for analysis {id}: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Html("Failed to start analysis".to_string()),
            )
                .into_response()
        }
    }
}

pub async fn review_warning_html(
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
    Path(id): Path<String>,
    Form(form): Form<WarningReviewForm>,
) -> impl IntoResponse {
    let uuid = match Uuid::parse_str(&id) {
        Ok(uuid) => uuid,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Html("Invalid warning ID".to_string()),
            )
                .into_response()
        }
    };

    let note = form
        .note
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let review_outcome = form
        .review_outcome
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    match store
        .acknowledge_warning(uuid, &session.username, note, review_outcome)
        .await
    {
        Ok(apex_store::postgres::AcknowledgeWarningResult::Acknowledged)
        | Ok(apex_store::postgres::AcknowledgeWarningResult::ReviewedExisting) => {
            let detail = match (review_outcome, note) {
                (Some(outcome), Some(note)) => format!("Marked as {outcome} with note: {note}"),
                (Some(outcome), None) => format!("Marked as {outcome}"),
                (None, Some(note)) => format!("Added resolution note: {note}"),
                (None, None) => "Reviewed warning".to_string(),
            };
            let _ = store
                .create_notification(
                    &session.username,
                    "warning_review",
                    "Warning review recorded",
                    &detail,
                    Some("warning"),
                    Some(&id),
                    Some(&format!("/warnings/{id}")),
                )
                .await;
            Html(format!(
                r#"<div class="apex-card p-4 border-primary/30 bg-primary/5">
                     <p class="text-sm font-bold text-primary">Review saved</p>
                     <p class="mt-1 text-[10px] text-muted-foreground">{}</p>
                   </div>"#,
                super::escape_html(&detail)
            ))
            .into_response()
        }
        Ok(apex_store::postgres::AcknowledgeWarningResult::AlreadyAcknowledged) => Html(
            r#"<div class="apex-card p-4"><p class="text-sm text-muted-foreground">Warning already acknowledged; note not changed.</p></div>"#.to_string(),
        )
        .into_response(),
        Ok(apex_store::postgres::AcknowledgeWarningResult::NotFound) => {
            (StatusCode::NOT_FOUND, Html("Warning not found".to_string())).into_response()
        }
        Err(error) => {
            tracing::error!(warning_id = %id, error = %error, "failed to save warning review");
            (StatusCode::INTERNAL_SERVER_ERROR, Html("Failed to save review".to_string())).into_response()
        }
    }
}

pub async fn create_warning_note(
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
    Path(id): Path<String>,
    Form(form): Form<WarningNoteForm>,
) -> impl IntoResponse {
    let body = form.body.trim();
    if body.len() < 3 {
        return Redirect::to(&format!("/warnings/{id}")).into_response();
    }

    let tags = parse_tags(form.tags.as_deref());
    let visibility = form.visibility.as_deref().unwrap_or("team");

    match store
        .upsert_annotation_scoped(
            &session.username,
            session.role.as_str(),
            None,
            "warning",
            &id,
            body,
            &tags,
            visibility,
        )
        .await
    {
        Ok(_) => {
            let _ = store
                .create_notification(
                    &session.username,
                    "annotation",
                    "Warning note added",
                    body,
                    Some("warning"),
                    Some(&id),
                    Some(&format!("/warnings/{id}")),
                )
                .await;
        }
        Err(error) => {
            tracing::error!(warning_id = %id, error = %error, "failed to create warning note");
        }
    }

    axum::response::Redirect::to(&format!("/warnings/{id}")).into_response()
}
