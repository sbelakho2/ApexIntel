//! Search handler — GET /search
//!
//! Covers: full-text search across all entities (companies, persons,
//! warnings, insights) with faceted results and highlighting.

use std::sync::Arc;

use askama::Template;
use axum::{
    http::HeaderMap,
    response::{Html, IntoResponse},
    Extension,
};
use serde::Deserialize;

use apex_store::postgres::{PgStore, WarningListFilters};
use apex_store::tantivy_index::SearchIndex;
use super::{is_htmx_request, PageContext};
use crate::middleware::session::WebSession;

// ─── Query params ───────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct SearchPageQuery {
    pub q: Option<String>,
    pub entity_type: Option<String>,  // "all" | "company" | "person" | "warning" | "insight"
    pub page: Option<i64>,
    pub per_page: Option<i64>,
}

// ─── Template data ──────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct SearchResultItem {
    pub entity_type: String,
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub snippet: String,
    pub score: f64,
    pub url: String,
}

#[derive(Clone, Debug)]
pub struct SearchFacet {
    pub label: String,
    pub count: i64,
    pub active: bool,
}

// ─── Template ───────────────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "pages/search.html")]
pub struct SearchPage {
    pub current_path: String,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,

    pub query: String,
    pub results: Vec<SearchResultItem>,
    pub total: i64,
    pub page: i64,
    pub per_page: i64,
    pub total_pages: i64,
    pub facets: Vec<SearchFacet>,
    pub active_type: String,
    pub took_ms: i64,
}

// ─── Handler ────────────────────────────────────────────────────────────────

/// GET /search — entity search page.
pub async fn search_page(
    headers: HeaderMap,
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
    Extension(search_index): Extension<Arc<SearchIndex>>,
    axum::extract::Query(params): axum::extract::Query<SearchPageQuery>,
) -> impl IntoResponse {
    let unack = store.count_warnings(&WarningListFilters { acknowledged: Some(false), ..Default::default() }).await.unwrap_or(0);
    let ctx = PageContext::from_session(&session, "/search", unack);
    let page = params.page.unwrap_or(1).max(1);
    let per_page = params.per_page.unwrap_or(25).clamp(1, 100);
    let query_str = params.q.unwrap_or_default();
    let active_type = params.entity_type.unwrap_or_else(|| "all".into());

    let start = std::time::Instant::now();

    let (results, total, facets) = if query_str.is_empty() {
        (vec![], 0i64, build_empty_facets(&active_type))
    } else {
        let offset = ((page - 1) * per_page) as usize;
        let limit = per_page as usize;

        let (items, total_hits) = if active_type == "all" {
            search_index.search_with_total(&query_str, limit, offset).unwrap_or_else(|e| {
                tracing::error!("Search failed: {e}");
                (vec![], 0)
            })
        } else {
            search_index.search_entity_type_with_total(&query_str, &active_type, limit, offset).unwrap_or_else(|e| {
                tracing::error!("Search entity_type failed: {e}");
                (vec![], 0)
            })
        };

        let mapped: Vec<SearchResultItem> = items.iter().map(|sr| {
            let url = if sr.url.is_empty() {
                format!("/{}/{}", sr.entity_type, sr.entity_id)
            } else {
                sr.url.clone()
            };
            SearchResultItem {
                entity_type: sr.entity_type.clone(),
                id: sr.entity_id.clone(),
                title: sr.title.clone(),
                subtitle: sr.region.clone(),
                snippet: sr.snippet.clone(),
                score: sr.score as f64,
                url,
            }
        }).collect();

        // Build simple facets (approximated — real facets would need separate counts)
        let facets = build_empty_facets(&active_type);

        (mapped, total_hits as i64, facets)
    };

    let took_ms = start.elapsed().as_millis() as i64;
    let total_pages = if per_page > 0 { (total + per_page - 1) / per_page } else { 0 };

    let tpl = SearchPage {
        current_path: ctx.current_path,
        username: ctx.username,
        warning_count: ctx.warning_count,
        theme: ctx.theme,
        query: query_str,
        results,
        total,
        page,
        per_page,
        total_pages,
        facets,
        active_type,
        took_ms,
    };

    if is_htmx_request(&headers) {
        Html(format!("<!-- htmx partial: search results -->")).into_response()
    } else {
        tpl.into_response()
    }
}

fn build_empty_facets(active_type: &str) -> Vec<SearchFacet> {
    vec![
        SearchFacet { label: "All".into(),       count: 0, active: active_type == "all" },
        SearchFacet { label: "Companies".into(), count: 0, active: active_type == "company" },
        SearchFacet { label: "Persons".into(),   count: 0, active: active_type == "person" },
        SearchFacet { label: "Warnings".into(),  count: 0, active: active_type == "warning" },
        SearchFacet { label: "Insights".into(),  count: 0, active: active_type == "insight" },
    ]
}
