//! Competitors handler — GET /competitors (list)
//!
//! Covers: competitor overview page showing tracked competitor companies,
//! their risk scores, recent changes, and head-to-head comparisons.

use std::sync::Arc;

use askama::Template;
use axum::{extract::Query, http::HeaderMap, response::IntoResponse, Extension};
use serde::Deserialize;
use url::form_urlencoded::byte_serialize;

use super::{is_htmx_request, PageContext};
use crate::middleware::session::WebSession;
use apex_store::postgres::{PgStore, WarningListFilters};

#[derive(Debug, Deserialize)]
pub struct CompetitorsQuery {
    pub threat: Option<String>,
    pub overlap: Option<String>,
}

// ─── Template data ──────────────────────────────────────────────────────────

/// One competitor's data for the comparison bar chart.
#[derive(Clone, Debug)]
pub struct CompetitorCard {
    pub id: String,
    pub name: String,
    pub sector: String,
    pub region: String,
    pub risk_score: i64,
    pub overlap_pct: i64,
    pub warning_count: i64,
    pub insight_count: i64,
    pub recent_change: Option<String>,
    pub change_date: Option<String>,
    // Precomputed SVG chart coordinates
    pub chart_group_x: i64,
    pub chart_threat_y: i64,
    pub chart_threat_h: i64,
    pub chart_overlap_y: i64,
    pub chart_overlap_h: i64,
    pub chart_label_x: i64,
}

#[derive(Clone, Debug)]
pub struct CompetitorChange {
    pub company_name: String,
    pub change_type: String,
    pub description: String,
    pub detected_at: String,
}

#[derive(Clone, Debug)]
pub struct CompetitorFilterChip {
    pub label: String,
    pub href: String,
    pub active: bool,
}

fn url_encode(v: &str) -> String {
    byte_serialize(v.as_bytes()).collect::<String>()
}

fn build_competitors_href(threat: Option<&str>, overlap: Option<&str>) -> String {
    let mut query: Vec<String> = Vec::new();
    if let Some(v) = threat {
        if !v.is_empty() {
            query.push(format!("threat={}", url_encode(v)));
        }
    }
    if let Some(v) = overlap {
        if !v.is_empty() {
            query.push(format!("overlap={}", url_encode(v)));
        }
    }
    if query.is_empty() {
        "/competitors".to_string()
    } else {
        format!("/competitors?{}", query.join("&"))
    }
}

// ─── Template ───────────────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "pages/competitors.html")]
pub struct CompetitorsPage {
    pub current_path: String,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,

    pub competitors: Vec<CompetitorCard>,
    pub total: i64,
    pub recent_changes: Vec<CompetitorChange>,
    pub avg_threat: i64,
    pub high_threat_count: i64,
    pub avg_overlap_pct: i64,
    pub chart_width: i64,
    pub active_threat: String,
    pub active_overlap: String,
    pub threat_filters: Vec<CompetitorFilterChip>,
    pub overlap_filters: Vec<CompetitorFilterChip>,
    pub active_filters: usize,
    pub reset_href: String,
}

/// HTMX partial — just the results fragment.
#[derive(Template)]
#[template(path = "pages/competitors/_list.html")]
pub struct CompetitorsListPartial {
    pub competitors: Vec<CompetitorCard>,
    pub total: i64,
    pub recent_changes: Vec<CompetitorChange>,
    pub avg_threat: i64,
    pub high_threat_count: i64,
    pub avg_overlap_pct: i64,
    pub chart_width: i64,
    pub active_threat: String,
    pub active_overlap: String,
    pub threat_filters: Vec<CompetitorFilterChip>,
    pub overlap_filters: Vec<CompetitorFilterChip>,
    pub active_filters: usize,
    pub reset_href: String,
}

// ─── Handler ────────────────────────────────────────────────────────────────

/// GET /competitors — competitor tracking overview.
pub async fn list_competitors(
    headers: HeaderMap,
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
    Query(params): Query<CompetitorsQuery>,
) -> impl IntoResponse {
    let unack = store
        .count_warnings(&WarningListFilters {
            acknowledged: Some(false),
            ..Default::default()
        })
        .await
        .unwrap_or(0);
    let ctx = PageContext::from_session(&session, "/competitors", unack);

    // Fetch competitor companies
    let competitor_rows = store.list_competitors(100, 0).await.unwrap_or_else(|e| {
        tracing::error!("Failed to list competitors: {e}");
        vec![]
    });
    let total = store
        .count_competitors()
        .await
        .unwrap_or(competitor_rows.len() as i64);

    let n = competitor_rows.len();
    let group_w: i64 = 46;
    let chart_area_h: i64 = 80;
    let chart_width = (n as i64 * group_w).max(46);

    let active_threat = params.threat.unwrap_or_default();
    let active_overlap = params.overlap.unwrap_or_default();

    let threat_values = ["", "high", "medium", "low"];
    let overlap_values = ["", "50%+", "30-50%", "<30%"];

    let threat_filters = threat_values
        .iter()
        .map(|value| CompetitorFilterChip {
            label: if value.is_empty() { "All" } else { value }.to_string(),
            href: build_competitors_href(
                if value.is_empty() { None } else { Some(*value) },
                if active_overlap.is_empty() {
                    None
                } else {
                    Some(active_overlap.as_str())
                },
            ),
            active: active_threat == *value,
        })
        .collect::<Vec<_>>();

    let overlap_filters = overlap_values
        .iter()
        .map(|value| CompetitorFilterChip {
            label: if value.is_empty() { "All" } else { value }.to_string(),
            href: build_competitors_href(
                if active_threat.is_empty() {
                    None
                } else {
                    Some(active_threat.as_str())
                },
                if value.is_empty() { None } else { Some(*value) },
            ),
            active: active_overlap == *value,
        })
        .collect::<Vec<_>>();

    let active_filters = [!active_threat.is_empty(), !active_overlap.is_empty()]
        .into_iter()
        .filter(|v| *v)
        .count();
    let reset_href = "/competitors".to_string();

    let mut competitors: Vec<CompetitorCard> = competitor_rows
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let risk = c
                .risk_score
                .map(|s| (s * 100.0) as i64)
                .unwrap_or(0)
                .clamp(0, 100);
            // overlap proxy (until explicit overlap data is persisted)
            let overlap = (risk as f64 * 0.65).round() as i64;
            let group_x = i as i64 * group_w;
            let threat_h = risk * chart_area_h / 100;
            let overlap_h = overlap * chart_area_h / 100;
            CompetitorCard {
                id: c.id.to_string(),
                name: c.name.clone(),
                sector: c.company_type.clone().unwrap_or_default(),
                region: c.region.clone().unwrap_or_default(),
                risk_score: risk,
                overlap_pct: overlap,
                warning_count: 0,
                insight_count: 0,
                recent_change: None,
                change_date: None,
                chart_group_x: group_x,
                chart_threat_y: chart_area_h - threat_h,
                chart_threat_h: threat_h,
                chart_overlap_y: chart_area_h - overlap_h,
                chart_overlap_h: overlap_h,
                chart_label_x: group_x + group_w / 2,
            }
        })
        .collect();

    if !active_threat.is_empty() {
        competitors.retain(|c| {
            let tier = if c.risk_score >= 70 {
                "high"
            } else if c.risk_score >= 40 {
                "medium"
            } else {
                "low"
            };
            tier == active_threat
        });
    }

    if !active_overlap.is_empty() {
        competitors.retain(|c| {
            let bucket = if c.overlap_pct >= 50 {
                "50%+"
            } else if c.overlap_pct >= 30 {
                "30-50%"
            } else {
                "<30%"
            };
            bucket == active_overlap
        });
    }

    let avg_threat = if competitors.is_empty() {
        0
    } else {
        competitors.iter().map(|c| c.risk_score).sum::<i64>() / competitors.len() as i64
    };
    let high_threat_count = competitors.iter().filter(|c| c.risk_score >= 70).count() as i64;
    let avg_overlap_pct = if competitors.is_empty() {
        0
    } else {
        competitors.iter().map(|c| c.overlap_pct).sum::<i64>() / competitors.len() as i64
    };

    // Fetch recent competitor changes
    let (change_rows, _total_changes) = store
        .get_all_competitor_changes_paged(1, 20)
        .await
        .unwrap_or_else(|e| {
            tracing::error!("Failed to list competitor changes: {e}");
            (vec![], 0)
        });
    let recent_changes: Vec<CompetitorChange> = change_rows
        .iter()
        .map(|ch| CompetitorChange {
            company_name: ch.competitor_name.clone(),
            change_type: ch.change_type.clone(),
            description: ch.description.clone(),
            detected_at: ch.detected_at.clone(),
        })
        .collect();

    let tpl = CompetitorsPage {
        current_path: ctx.current_path,
        username: ctx.username,
        warning_count: ctx.warning_count,
        theme: ctx.theme,
        competitors,
        total,
        recent_changes,
        avg_threat,
        high_threat_count,
        avg_overlap_pct,
        chart_width,
        active_threat,
        active_overlap,
        threat_filters,
        overlap_filters,
        active_filters,
        reset_href,
    };

    if is_htmx_request(&headers) {
        let partial = CompetitorsListPartial {
            competitors: tpl.competitors.clone(),
            total: tpl.total,
            recent_changes: tpl.recent_changes.clone(),
            avg_threat: tpl.avg_threat,
            high_threat_count: tpl.high_threat_count,
            avg_overlap_pct: tpl.avg_overlap_pct,
            chart_width: tpl.chart_width,
            active_threat: tpl.active_threat.clone(),
            active_overlap: tpl.active_overlap.clone(),
            threat_filters: tpl.threat_filters.clone(),
            overlap_filters: tpl.overlap_filters.clone(),
            active_filters: tpl.active_filters,
            reset_href: tpl.reset_href.clone(),
        };
        partial.into_response()
    } else {
        tpl.into_response()
    }
}
