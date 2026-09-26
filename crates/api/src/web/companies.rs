//! Company handlers — GET /companies (list), GET /companies/:id (detail)
//!
//! Covers: company list with region/sector filters, company detail with
//! key persons, sites, products, risk score, and recent events.

use std::sync::Arc;

use askama::Template;
use axum::{
    extract::Path,
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse},
    Extension,
};
use serde::Deserialize;
use url::form_urlencoded::byte_serialize;
use uuid::Uuid;

use super::{is_htmx_request, PageContext};
use crate::middleware::session::WebSession;
use apex_core::data_state::{DataState, DegradedNotice};
use apex_store::postgres::{CompanyListFilters, CompanyOrderBy, PgStore, WarningListFilters};

// ─── Query params ───────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CompaniesQuery {
    pub page: Option<i64>,
    pub per_page: Option<i64>,
    pub region: Option<String>,
    pub sector: Option<String>,
    pub q: Option<String>,
    pub sort: Option<String>,
    pub dir: Option<String>,
    pub is_competitor: Option<bool>,
}

fn canonical_region(region: &str) -> String {
    let trimmed = region.trim();
    if trimmed.is_empty() {
        return "Other".to_string();
    }
    match trimmed.to_ascii_lowercase().as_str() {
        "tn" | "tunisia" => "Tunisia".to_string(),
        "ma" | "morocco" => "Morocco".to_string(),
        "il" | "israel" => "Israel".to_string(),
        "eu" | "europe" => "Europe".to_string(),
        "cn" | "china" => "China".to_string(),
        "us" | "usa" | "united states" => "United States".to_string(),
        "global" => "Global".to_string(),
        _ => trimmed.to_string(),
    }
}

// ─── Template data ──────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct CompanyListItem {
    pub id: String,
    pub name: String,
    pub sector: String,
    pub region: String,
    pub risk_score: i64,
    pub warning_count: i64,
    pub insight_count: i64,
    pub is_competitor: bool,
    pub updated_at: String,
}

#[derive(Clone, Debug)]
pub struct CompanyKeyPerson {
    pub person_id: String,
    pub name: String,
    pub role: String,
    pub since: String,
    pub background_snippet: String,
}

#[derive(Clone, Debug)]
pub struct CompanySite {
    pub name: String,
    pub location: String,
    pub site_type: String,
}

#[derive(Clone, Debug)]
pub struct CompanyProduct {
    pub name: String,
    pub family: String,
    pub status: String,
}

#[derive(Clone, Debug)]
pub struct CompanyEvent {
    pub kind: String,
    pub description: String,
    pub date: String,
}

#[derive(Clone, Debug)]
pub struct CompanyFinancial {
    pub metric: String,
    pub value: String,
    pub period: String,
}

#[derive(Clone, Debug)]
pub struct CompanyWarning {
    pub id: String,
    pub title: String,
    pub warning_type: String,
    pub severity: String,
    pub confidence_pct: i64,
    pub created_at: String,
    pub age: String,
    pub acknowledged: bool,
}

/// One entry in the entity evidence timeline (collaboration evidence rows and
/// the source URLs attached to the entity's signals, newest first).
#[derive(Clone, Debug)]
pub struct EntityEvidenceItem {
    pub kind: String,
    pub title: String,
    pub source: String,
    pub url: String,
    pub at: String,
    pub detail: String,
}

/// "Why now" summary for the entity workspace: the freshest signal, how many
/// signals are still open, and how much the dossier moved recently.
#[derive(Clone, Debug)]
pub struct EntityWhyNow {
    pub headline: String,
    pub latest_signal_id: String,
    pub latest_signal_title: String,
    pub latest_signal_severity: String,
    pub latest_signal_type: String,
    pub latest_signal_age: String,
    pub open_signals: i64,
    pub recent_changes: i64,
    pub evidence_count: i64,
}

/// Relationship edge rendered on the entity dossier (graph-as-tool).
#[derive(Clone, Debug)]
pub struct EntityRelationship {
    pub edge_type: String,
    pub target_kind: String,
    pub target_id: String,
    pub target_name: String,
    pub confidence_pct: i64,
    pub last_seen: String,
}

/// Open investigation workspace focused on this entity.
#[derive(Clone, Debug)]
pub struct EntityWorkspaceRef {
    pub id: String,
    pub name: String,
    pub status: String,
    pub updated_at: String,
}

/// Pipeline opportunity attached to this entity (sales intelligence loop).
#[derive(Clone, Debug)]
pub struct EntityPipelineItem {
    pub id: String,
    pub title: String,
    pub stage: String,
    pub probability_pct: i64,
    pub value: String,
}

/// Person linked to this entity through a mapped buying centre.
#[derive(Clone, Debug)]
pub struct EntityBuyingCentreMember {
    pub person_id: String,
    pub name: String,
    pub role: String,
    pub influence_pct: i64,
    pub budget_authority: bool,
}

/// Buying centre mapped on this entity (sales intelligence workspace view).
#[derive(Clone, Debug)]
pub struct EntityBuyingCentre {
    pub name: String,
    pub status: String,
    pub members: Vec<EntityBuyingCentreMember>,
}

#[derive(Clone, Debug)]
pub struct CompanyInsight {
    pub id: String,
    pub title: String,
    pub insight_type: String,
    pub summary: String,
    pub confidence_pct: i64,
    pub created_at: String,
}

#[derive(Clone, Debug)]
pub struct RegionSlice {
    pub name: String,
    pub count: i64,
    pub color: String,
    pub dash_array: String,
    pub dash_offset: String,
}

#[derive(Clone, Debug)]
pub struct CompanyQuickLink {
    pub id: String,
    pub name: String,
    pub tier: String,
}

#[derive(Clone, Debug)]
pub struct CompanyFilterChip {
    pub label: String,
    pub href: String,
    pub active: bool,
}

// ─── Templates ──────────────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "pages/companies.html")]
pub struct CompaniesListPage {
    pub current_path: String,
    pub can_admin: bool,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,
    pub status_strip: crate::system_status::StatusStrip,

    pub companies: Vec<CompanyListItem>,
    pub total: i64,
    pub page: i64,
    pub per_page: i64,
    pub total_pages: i64,
    pub active_region: String,
    pub active_sector: String,
    pub search_query: String,
    pub sort_field: String,
    pub sort_dir: String,
    pub competitor_count: i64,
    pub high_risk_count: i64,
    pub regions_count: i64,
    pub avg_risk: i64,
    pub region_slices: Vec<RegionSlice>,
    pub quick_links: Vec<CompanyQuickLink>,
    pub region_filters: Vec<CompanyFilterChip>,
    pub tier_filters: Vec<CompanyFilterChip>,
    pub active_filters: i64,
    pub reset_href: String,
    pub page_base_href: String,
    /// Rendered when the company query failed, instead of "no results".
    pub degraded_notice: Option<String>,
}

/// HTMX partial — just the results fragment.
#[derive(Template)]
#[template(path = "pages/companies/_list.html")]
pub struct CompaniesListPartial {
    pub companies: Vec<CompanyListItem>,
    pub total: i64,
    pub page: i64,
    pub per_page: i64,
    pub total_pages: i64,
    pub active_region: String,
    pub active_sector: String,
    pub search_query: String,
    pub sort_field: String,
    pub sort_dir: String,
    pub competitor_count: i64,
    pub high_risk_count: i64,
    pub regions_count: i64,
    pub avg_risk: i64,
    pub region_slices: Vec<RegionSlice>,
    pub quick_links: Vec<CompanyQuickLink>,
    pub region_filters: Vec<CompanyFilterChip>,
    pub tier_filters: Vec<CompanyFilterChip>,
    pub active_filters: i64,
    pub reset_href: String,
    pub page_base_href: String,
    /// Rendered when the company query failed, instead of "no results".
    pub degraded_notice: Option<String>,
}

fn url_encode_component(input: &str) -> String {
    byte_serialize(input.as_bytes()).collect::<String>()
}

fn build_companies_href(
    region: Option<&str>,
    sector: Option<&str>,
    q: Option<&str>,
    sort: Option<&str>,
    dir: Option<&str>,
) -> String {
    let mut params: Vec<String> = Vec::new();
    for (k, v) in [
        ("region", region),
        ("sector", sector),
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
        "/companies".to_string()
    } else {
        format!("/companies?{}", params.join("&"))
    }
}

/// Company dossier entry for the dossier tab.
#[derive(Clone, Debug)]
pub struct DossierEntry {
    pub field_name: String,
    pub value: String,
    pub source: String,
    pub verified: bool,
}

/// Company changes tab partial template.
#[derive(Template)]
#[template(path = "partials/company_changes_tab.html")]
pub struct CompanyChangesTabPartial {
    pub events: Vec<CompanyEvent>,
}

/// Company dossier tab partial template.
#[derive(Template)]
#[template(path = "partials/company_dossier_tab.html")]
pub struct CompanyDossierTabPartial {
    pub dossier_entries: Vec<DossierEntry>,
    /// Set when the dossier query failed, so a storage error never renders as
    /// "no dossier entries".
    pub degraded_notice: Option<String>,
}

#[derive(Template)]
#[template(path = "pages/company_detail.html")]
pub struct CompanyDetailPage {
    pub current_path: String,
    pub can_admin: bool,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,
    pub status_strip: crate::system_status::StatusStrip,
    pub briefing_mode: bool,

    pub id: String,
    pub name: String,
    pub sector: String,
    pub region: String,
    pub description: String,
    pub website: String,
    pub website_url: String,
    pub risk_score: i64,
    pub is_competitor: bool,
    pub created_at: String,
    pub updated_at: String,
    pub key_persons: Vec<CompanyKeyPerson>,
    pub sites: Vec<CompanySite>,
    pub products: Vec<CompanyProduct>,
    pub recent_events: Vec<CompanyEvent>,
    pub dossier_entries: Vec<DossierEntry>,
    pub warnings: Vec<CompanyWarning>,
    pub insights: Vec<CompanyInsight>,
    pub financials: Vec<CompanyFinancial>,
    pub total_warnings: i64,
    pub total_insights: i64,
    /// High/critical warnings currently open on this entity.
    pub risk_count: i64,
    /// Entity workspace sections (audit #27): why now, evidence timeline,
    /// relationships, open investigations and pipeline status.
    pub why_now: EntityWhyNow,
    pub evidence_timeline: Vec<EntityEvidenceItem>,
    pub relationships: Vec<EntityRelationship>,
    pub open_investigations: Vec<EntityWorkspaceRef>,
    pub pipeline: Vec<EntityPipelineItem>,
    pub buying_centers: Vec<EntityBuyingCentre>,
    /// Rendered when company detail queries failed, instead of "no results".
    pub degraded_notice: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct CompanyDetailQuery {
    pub briefing: Option<bool>,
}

// ─── Handlers ───────────────────────────────────────────────────────────────

/// GET /companies — paginated company list.
pub async fn list_companies(
    headers: HeaderMap,
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
    axum::extract::Query(params): axum::extract::Query<CompaniesQuery>,
) -> impl IntoResponse {
    let active_region = params.region.clone().unwrap_or_default();
    let active_sector = params.sector.clone().unwrap_or_default();
    let search_query = params.q.clone().unwrap_or_default();
    let sort_field = params.sort.clone().unwrap_or_else(|| "name".into());
    let sort_dir_str = params.dir.clone().unwrap_or_else(|| "asc".into());

    let page = params.page.unwrap_or(1).max(1);
    let per_page = params.per_page.unwrap_or(25).clamp(1, 100);
    let offset = (page - 1) * per_page;

    let filters = CompanyListFilters {
        regions: vec![],
        search: if search_query.is_empty() {
            None
        } else {
            Some(search_query.clone())
        },
        is_competitor: params.is_competitor,
    };

    let order_by = match sort_field.as_str() {
        "region" => Some(CompanyOrderBy::Region),
        "threat_score" => Some(CompanyOrderBy::ThreatScore),
        "updated_at" => Some(CompanyOrderBy::UpdatedAt),
        _ => Some(CompanyOrderBy::Name),
    };
    let desc = sort_dir_str != "asc";

    let mut degraded_notice: Option<String> = None;

    let company_rows_state = DataState::from_result(
        store
            .list_companies(&filters, order_by, desc, 2000, 0)
            .await,
        "failed to list companies",
        |rows| rows.is_empty(),
    );
    DegradedNotice::capture(&company_rows_state, &mut degraded_notice);
    let company_rows = company_rows_state.into_items();

    // B315: real per-entity warning/insight counts (two GROUP BY queries)
    // instead of hardcoded zeros on every company row. Failures surface as a
    // degraded marker — never as silently-zeroed counts.
    let warning_counts_state = DataState::from_result(
        store.get_warning_counts_by_entity().await,
        "failed to fetch warning counts by entity",
        |rows| rows.is_empty(),
    );
    DegradedNotice::capture(&warning_counts_state, &mut degraded_notice);
    let warning_counts: std::collections::HashMap<uuid::Uuid, i64> = warning_counts_state
        .into_loaded_or_default()
        .into_iter()
        .collect();
    let insight_counts_state = DataState::from_result(
        store.get_insight_counts_by_entity().await,
        "failed to fetch insight counts by entity",
        |rows| rows.is_empty(),
    );
    DegradedNotice::capture(&insight_counts_state, &mut degraded_notice);
    let insight_counts: std::collections::HashMap<uuid::Uuid, i64> = insight_counts_state
        .into_loaded_or_default()
        .into_iter()
        .collect();

    let mut all_companies: Vec<CompanyListItem> = company_rows
        .iter()
        .map(|c| {
            let is_comp = c
                .metadata
                .as_ref()
                .and_then(|m| m.get("is_competitor"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let risk_score = c.risk_score.map(|s| (s * 100.0) as i64).unwrap_or(0);
            CompanyListItem {
                id: c.id.to_string(),
                name: c.name.clone(),
                sector: c.company_type.clone().unwrap_or_default(),
                region: canonical_region(c.region.as_deref().unwrap_or("")),
                risk_score,
                warning_count: warning_counts.get(&c.id).copied().unwrap_or(0),
                insight_count: insight_counts.get(&c.id).copied().unwrap_or(0),
                is_competitor: is_comp,
                updated_at: c
                    .updated_at
                    .map(|d| d.format("%Y-%m-%d").to_string())
                    .unwrap_or_default(),
            }
        })
        .collect();

    if !active_region.is_empty() {
        let selected = canonical_region(&active_region);
        all_companies.retain(|c| c.region.eq_ignore_ascii_case(&selected));
    }

    if !active_sector.is_empty() {
        all_companies.retain(|c| risk_tier(c.risk_score) == active_sector);
    }

    let total = all_companies.len() as i64;
    let total_pages = if total == 0 {
        0
    } else {
        (total + per_page - 1) / per_page
    };

    let companies: Vec<CompanyListItem> = all_companies
        .iter()
        .skip(offset as usize)
        .take(per_page as usize)
        .cloned()
        .collect();

    let competitor_count = all_companies.iter().filter(|c| c.is_competitor).count() as i64;
    let high_risk_count = all_companies.iter().filter(|c| c.risk_score >= 70).count() as i64;
    let regions_count = {
        use std::collections::HashSet;
        let mut regions: HashSet<&str> = HashSet::new();
        for c in &all_companies {
            if !c.region.is_empty() {
                regions.insert(c.region.as_str());
            }
        }
        regions.len() as i64
    };
    let avg_risk = {
        let scored: Vec<i64> = all_companies
            .iter()
            .map(|c| c.risk_score)
            .filter(|s| *s > 0)
            .collect();
        if scored.is_empty() {
            0
        } else {
            scored.iter().sum::<i64>() / scored.len() as i64
        }
    };

    let region_slices = {
        use std::collections::HashMap;
        let mut counts: HashMap<String, i64> = HashMap::new();
        for c in &all_companies {
            let key = canonical_region(&c.region);
            *counts.entry(key).or_insert(0) += 1;
        }
        let mut slices: Vec<RegionSlice> = counts
            .into_iter()
            .map(|(name, count)| RegionSlice {
                color: String::new(),
                name,
                count,
                dash_array: String::new(),
                dash_offset: String::new(),
            })
            .collect();
        slices.sort_by_key(|a| std::cmp::Reverse(a.count));
        let mut used_colors = std::collections::HashSet::<String>::new();
        for (index, slice) in slices.iter_mut().enumerate() {
            let mut color = region_color(&slice.name, index).to_string();
            if used_colors.contains(&color) {
                if let Some(candidate) = REGION_PALETTE
                    .iter()
                    .find(|candidate| !used_colors.contains(**candidate))
                {
                    color = (*candidate).to_string();
                } else {
                    color = REGION_PALETTE[index % REGION_PALETTE.len()].to_string();
                }
            }
            used_colors.insert(color.clone());
            slice.color = color;
        }
        // Compute SVG donut arc data
        let total_co = all_companies.len() as f64;
        if total_co > 0.0 {
            let r = 65.0f64;
            let circ = 2.0 * std::f64::consts::PI * r;
            let mut cumulative = 0.0f64;
            for slice in &mut slices {
                let frac = slice.count as f64 / total_co;
                slice.dash_array = format!("{:.2} {:.2}", frac * circ, circ);
                slice.dash_offset = format!("{:.2}", circ * 0.25 - cumulative * circ);
                cumulative += frac;
            }
        }
        slices
    };

    let quick_links: Vec<CompanyQuickLink> = all_companies
        .iter()
        .take(5)
        .map(|c| CompanyQuickLink {
            id: c.id.clone(),
            name: c.name.clone(),
            tier: risk_tier(c.risk_score).to_string(),
        })
        .collect();

    let regions = ["Tunisia", "Morocco", "Israel", "EU", "China", "Global"];
    let mut region_filters = vec![CompanyFilterChip {
        label: "All".to_string(),
        href: build_companies_href(
            None,
            Some(&active_sector),
            Some(&search_query),
            Some(&sort_field),
            Some(&sort_dir_str),
        ),
        active: active_region.is_empty(),
    }];
    for region in regions {
        region_filters.push(CompanyFilterChip {
            label: region.to_string(),
            href: build_companies_href(
                Some(region),
                Some(&active_sector),
                Some(&search_query),
                Some(&sort_field),
                Some(&sort_dir_str),
            ),
            active: active_region.eq_ignore_ascii_case(region),
        });
    }

    let tiers = ["T1", "T2", "T3", "T4", "T5"];
    let mut tier_filters: Vec<CompanyFilterChip> = Vec::new();
    for tier in tiers {
        tier_filters.push(CompanyFilterChip {
            label: tier.to_string(),
            href: build_companies_href(
                Some(&active_region),
                Some(tier),
                Some(&search_query),
                Some(&sort_field),
                Some(&sort_dir_str),
            ),
            active: active_sector == tier,
        });
    }

    let active_filters = i64::from(!active_region.is_empty())
        + i64::from(!active_sector.is_empty())
        + i64::from(!search_query.is_empty());
    let reset_href = build_companies_href(None, None, None, Some(&sort_field), Some(&sort_dir_str));
    let current_filters_href = build_companies_href(
        Some(&active_region),
        Some(&active_sector),
        Some(&search_query),
        None, // sort is appended by templates — avoid duplicate params
        None, // dir  is appended by templates — avoid duplicate params
    );
    let page_base_href = if current_filters_href.contains('?') {
        format!("{}&", current_filters_href)
    } else {
        format!("{}?", current_filters_href)
    };

    let unack = store
        .count_warnings(&WarningListFilters {
            acknowledged: Some(false),
            ..Default::default()
        })
        .await
        .unwrap_or(0);
    let ctx = PageContext::from_session(&session, "/companies", unack);

    let tpl = CompaniesListPage {
        current_path: ctx.current_path,
        can_admin: ctx.can_admin,
        status_strip: ctx.status_strip,
        username: ctx.username,
        warning_count: ctx.warning_count,
        theme: ctx.theme,
        degraded_notice: degraded_notice.clone(),
        companies,
        total,
        page,
        per_page,
        total_pages,
        active_region,
        active_sector,
        search_query,
        sort_field,
        sort_dir: sort_dir_str,
        competitor_count,
        high_risk_count,
        regions_count,
        avg_risk,
        region_slices,
        quick_links,
        region_filters,
        tier_filters,
        active_filters,
        reset_href,
        page_base_href,
    };

    if is_htmx_request(&headers) {
        let partial = CompaniesListPartial {
            degraded_notice,
            companies: tpl.companies.clone(),
            total: tpl.total,
            page: tpl.page,
            per_page: tpl.per_page,
            total_pages: tpl.total_pages,
            active_region: tpl.active_region.clone(),
            active_sector: tpl.active_sector.clone(),
            search_query: tpl.search_query.clone(),
            sort_field: tpl.sort_field.clone(),
            sort_dir: tpl.sort_dir.clone(),
            competitor_count: tpl.competitor_count,
            high_risk_count: tpl.high_risk_count,
            regions_count: tpl.regions_count,
            avg_risk: tpl.avg_risk,
            region_slices: tpl.region_slices.clone(),
            quick_links: tpl.quick_links.clone(),
            region_filters: tpl.region_filters.clone(),
            tier_filters: tpl.tier_filters.clone(),
            active_filters: tpl.active_filters,
            reset_href: tpl.reset_href.clone(),
            page_base_href: tpl.page_base_href.clone(),
        };
        super::render_template(&partial)
    } else {
        super::render_template(&tpl)
    }
}

const REGION_PALETTE: [&str; 8] = [
    "#4A90E2", "#2D8C3C", "#FFBE00", "#D62D2D", "#8B5CF6", "#14B8A6", "#F97316", "#06B6D4",
];

fn region_color(region: &str, index: usize) -> &'static str {
    let normalized = region.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "tn" | "tunisia" => "#FFBE00",
        "ma" | "morocco" => "#D62D2D",
        "il" | "israel" => "#4A90E2",
        "eu" | "europe" => "#2D8C3C",
        "cn" | "china" => "#8B5CF6",
        "us" | "usa" | "united states" => "#14B8A6",
        "global" => "#F97316",
        _ => REGION_PALETTE[index % REGION_PALETTE.len()],
    }
}

fn normalize_website_url(website: &str) -> String {
    let trimmed = website.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_string()
    } else {
        format!("https://{}", trimmed)
    }
}

fn risk_tier(score: i64) -> &'static str {
    if score >= 85 {
        "T1"
    } else if score >= 70 {
        "T2"
    } else if score >= 55 {
        "T3"
    } else if score >= 35 {
        "T4"
    } else {
        "T5"
    }
}

/// Short human-readable "how long ago" label for signal/change freshness.
fn humanize_age(ts: chrono::DateTime<chrono::Utc>) -> String {
    let delta = chrono::Utc::now().signed_duration_since(ts);
    let minutes = delta.num_minutes();
    if minutes < 1 {
        "just now".to_string()
    } else if minutes < 60 {
        format!("{minutes}m ago")
    } else if delta.num_hours() < 24 {
        format!("{}h ago", delta.num_hours())
    } else if delta.num_days() < 14 {
        format!("{}d ago", delta.num_days())
    } else if delta.num_days() < 60 {
        format!("{}w ago", delta.num_days() / 7)
    } else {
        format!("{}mo ago", delta.num_days() / 30)
    }
}

fn capitalize(value: &str) -> String {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// True when a workspace's `entity_focus` JSON names this entity, accepting
/// both bare id arrays (`["<uuid>"]`) and object entries (`{"id": "<uuid>"}`).
fn workspace_focuses_entity(focus: &serde_json::Value, entity_id: &str) -> bool {
    fn matches(entry: &serde_json::Value, entity_id: &str) -> bool {
        entry.as_str() == Some(entity_id)
            || entry.get("id").and_then(|v| v.as_str()) == Some(entity_id)
            || entry.get("entity_id").and_then(|v| v.as_str()) == Some(entity_id)
    }
    match focus {
        serde_json::Value::Array(items) => items.iter().any(|item| matches(item, entity_id)),
        serde_json::Value::Object(_) => matches(focus, entity_id),
        _ => false,
    }
}

/// GET /companies/:id — single company detail page.
pub async fn get_company(
    _headers: HeaderMap,
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
    Path(id): Path<String>,
    axum::extract::Query(query): axum::extract::Query<CompanyDetailQuery>,
) -> impl IntoResponse {
    let unack = store
        .count_warnings(&WarningListFilters {
            acknowledged: Some(false),
            ..Default::default()
        })
        .await
        .unwrap_or(0);
    let ctx = PageContext::from_session(&session, "/companies", unack);

    let uuid = match Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => {
            return super::errors::not_found_with_context(
                &ctx.username,
                "/companies",
                ctx.warning_count,
            );
        }
    };

    let company = match store.get_company(uuid).await {
        Ok(Some(c)) => c,
        Ok(None) => {
            return super::errors::not_found_with_context(
                &ctx.username,
                "/companies",
                ctx.warning_count,
            );
        }
        Err(e) => {
            tracing::error!("Failed to fetch company {id}: {e}");
            return super::errors::not_found_with_context(
                &ctx.username,
                "/companies",
                ctx.warning_count,
            );
        }
    };

    let mut degraded_notice: Option<String> = None;

    // Fetch sites for this company
    let sites_state = DataState::from_result(
        store.get_sites_for_company(uuid).await,
        "failed to fetch company sites",
        |rows| rows.is_empty(),
    );
    DegradedNotice::capture(&sites_state, &mut degraded_notice);
    let site_rows = sites_state.into_items();
    let sites: Vec<CompanySite> = site_rows
        .iter()
        .map(|s| CompanySite {
            name: s.name.clone(),
            location: format!(
                "{}, {}",
                s.city.clone().unwrap_or_default(),
                s.country_code.clone().unwrap_or_default()
            ),
            site_type: s.site_type.clone().unwrap_or_default(),
        })
        .collect();

    // Fetch persons for this company
    let persons_state = DataState::from_result(
        store.list_persons_by_org(uuid).await,
        "failed to fetch company persons",
        |rows| rows.is_empty(),
    );
    DegradedNotice::capture(&persons_state, &mut degraded_notice);
    let person_rows = persons_state.into_items();
    let key_persons: Vec<CompanyKeyPerson> = person_rows
        .iter()
        .take(10)
        .map(|p| CompanyKeyPerson {
            person_id: p.id.to_string(),
            name: p.name.clone(),
            role: p.current_role.clone().unwrap_or_default(),
            since: p
                .created_at
                .map(|d| d.format("%Y-%m-%d").to_string())
                .unwrap_or_default(),
            background_snippet: p
                .public_bio
                .as_deref()
                .unwrap_or("")
                .chars()
                .take(120)
                .collect::<String>(),
        })
        .collect();

    // Fetch product families
    let products_state = DataState::from_result(
        store.list_product_families(Some(uuid), 50, 0).await,
        "failed to fetch company product families",
        |rows| rows.is_empty(),
    );
    DegradedNotice::capture(&products_state, &mut degraded_notice);
    let product_rows = products_state.into_items();
    let products: Vec<CompanyProduct> = product_rows
        .iter()
        .map(|pf| CompanyProduct {
            name: pf.name.clone(),
            family: pf
                .tech_tags
                .as_ref()
                .map(|t| t.join(", "))
                .unwrap_or_default(),
            status: "active".into(),
        })
        .collect();

    let changes_state = DataState::from_result(
        store.get_company_changes(uuid, 50).await,
        "failed to fetch company changes",
        |rows| rows.is_empty(),
    );
    DegradedNotice::capture(&changes_state, &mut degraded_notice);
    let change_rows = changes_state.into_items();
    let recent_change_count = change_rows
        .iter()
        .filter(|c| {
            c.detected_at
                .or(c.created_at)
                .map(|ts| chrono::Utc::now().signed_duration_since(ts).num_days() <= 14)
                .unwrap_or(false)
        })
        .count() as i64;
    let recent_events = change_rows
        .into_iter()
        .map(|c| CompanyEvent {
            kind: c.change_type,
            description: c
                .description
                .or(c.new_value)
                .or(c.old_value)
                .unwrap_or_default(),
            date: c
                .detected_at
                .or(c.created_at)
                .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_default(),
        })
        .collect::<Vec<_>>();

    let dossier_state = DataState::from_result(
        store.get_dossier_entries("company", uuid, None, 100).await,
        "failed to fetch company dossier entries",
        |rows| rows.is_empty(),
    );
    DegradedNotice::capture(&dossier_state, &mut degraded_notice);
    let dossier_entries = dossier_state
        .into_items()
        .into_iter()
        .map(|e| DossierEntry {
            field_name: e.title,
            value: e.content,
            source: e
                .source_urls
                .and_then(|urls| urls.first().cloned())
                .unwrap_or_default(),
            verified: e.verified.unwrap_or(false),
        })
        .collect::<Vec<_>>();

    // Related warnings for this company (entity_ids array overlap on company id).
    let warnings_state = DataState::from_result(
        store.get_warnings_by_entity_ids(&[uuid], 50).await,
        "failed to fetch company warnings",
        |rows| rows.is_empty(),
    );
    DegradedNotice::capture(&warnings_state, &mut degraded_notice);
    let warning_rows = warnings_state.into_items();
    let total_warnings = warning_rows.len() as i64;
    let warnings: Vec<CompanyWarning> = warning_rows
        .iter()
        .map(|w| CompanyWarning {
            id: w.id.to_string(),
            title: w.title.clone(),
            warning_type: w.warning_type.clone(),
            severity: w.severity.clone(),
            confidence_pct: w.confidence.map(|c| (c * 100.0) as i64).unwrap_or(0),
            created_at: w.ts_utc.format("%Y-%m-%d").to_string(),
            age: humanize_age(w.ts_utc),
            acknowledged: w.acknowledged,
        })
        .collect();

    // Related insights for this company (entity_ids array overlap on company id).
    let insights_state = DataState::from_result(
        store.get_insights_by_entity_ids(&[uuid], 50).await,
        "failed to fetch company insights",
        |rows| rows.is_empty(),
    );
    DegradedNotice::capture(&insights_state, &mut degraded_notice);
    let insight_rows = insights_state.into_items();
    let total_insights = insight_rows.len() as i64;
    let insights: Vec<CompanyInsight> = insight_rows
        .iter()
        .map(|i| CompanyInsight {
            id: i.id.to_string(),
            title: i.title.clone(),
            insight_type: i.insight_type.clone().unwrap_or_default(),
            summary: i.summary.chars().take(180).collect::<String>(),
            confidence_pct: i.confidence.map(|c| (c * 100.0) as i64).unwrap_or(0),
            created_at: i
                .created_at
                .map(|d| d.format("%Y-%m-%d").to_string())
                .unwrap_or_default(),
        })
        .collect();

    // ── Entity workspace: why now ───────────────────────────────────────
    let open_signals = warning_rows.iter().filter(|w| !w.acknowledged).count() as i64;
    let latest_signal_id = warning_rows
        .first()
        .map(|w| w.id.to_string())
        .unwrap_or_default();
    let latest_signal_title = warning_rows
        .first()
        .map(|w| w.title.clone())
        .unwrap_or_default();
    let latest_signal_severity = warning_rows
        .first()
        .map(|w| w.severity.clone())
        .unwrap_or_default();
    let latest_signal_type = warning_rows
        .first()
        .map(|w| w.warning_type.clone())
        .unwrap_or_default();
    let latest_signal_age = warning_rows
        .first()
        .map(|w| humanize_age(w.ts_utc))
        .unwrap_or_default();
    let headline = if let Some(latest) = warning_rows.first() {
        if latest.acknowledged {
            format!(
                "Latest {} {} signal is reviewed; {} signal(s) still open.",
                latest.severity, latest.warning_type, open_signals
            )
        } else {
            format!(
                "{} {} signal {} — {} signal(s) need review.",
                capitalize(&latest.severity),
                latest.warning_type,
                latest_signal_age,
                open_signals
            )
        }
    } else if recent_change_count > 0 {
        format!(
            "No open signals, but {recent_change_count} dossier change(s) landed in the last 14 days."
        )
    } else {
        "Nothing new on this entity — watch status keeps it covered.".to_string()
    };

    // ── Entity workspace: evidence timeline (collaboration evidence + signal sources)
    let evidence_state = DataState::from_result(
        store
            .list_source_evidence(Some("company"), Some(&id), None, 40)
            .await,
        "failed to fetch entity evidence",
        |rows| rows.is_empty(),
    );
    DegradedNotice::capture(&evidence_state, &mut degraded_notice);
    let mut evidence_timeline: Vec<EntityEvidenceItem> = evidence_state
        .into_items()
        .into_iter()
        .map(|e| EntityEvidenceItem {
            kind: e.evidence_type.clone(),
            title: e
                .source_name
                .clone()
                .or_else(|| e.source_domain.clone())
                .unwrap_or_else(|| "Evidence".to_string()),
            source: e.source_domain.clone().unwrap_or_default(),
            url: e.source_url.clone(),
            at: e.created_at.format("%Y-%m-%d %H:%M").to_string(),
            detail: e.excerpt.clone().unwrap_or_default(),
        })
        .collect();
    for warning in &warning_rows {
        for url in warning.source_urls.as_deref().unwrap_or(&[]) {
            evidence_timeline.push(EntityEvidenceItem {
                kind: format!("signal:{}", warning.warning_type),
                title: warning.title.clone(),
                source: url.split('/').nth(2).unwrap_or("unknown").to_string(),
                url: url.clone(),
                at: warning.ts_utc.format("%Y-%m-%d %H:%M").to_string(),
                detail: warning
                    .description
                    .clone()
                    .unwrap_or_default()
                    .chars()
                    .take(160)
                    .collect(),
            });
        }
    }
    evidence_timeline.sort_by(|a, b| b.at.cmp(&a.at));
    evidence_timeline.truncate(30);
    let evidence_count = evidence_timeline.len() as i64;

    // ── Entity workspace: relationships (graph-as-tool, one hop) ────────
    let edges_state = DataState::from_result(
        store.get_neighborhood(uuid, 24).await,
        "failed to fetch entity relationships",
        Vec::is_empty,
    );
    DegradedNotice::capture(&edges_state, &mut degraded_notice);
    let edge_rows = edges_state.into_items();
    let mut company_peer_ids: Vec<Uuid> = Vec::new();
    let mut person_peer_ids: Vec<Uuid> = Vec::new();
    let mut relationship_seeds: Vec<(String, String, Uuid, i64, String)> = Vec::new();
    for edge in &edge_rows {
        let (other_id, other_type) = if edge.source_id == uuid {
            if edge.target_id == uuid {
                continue;
            }
            (edge.target_id, edge.target_type.as_str())
        } else {
            (edge.source_id, edge.source_type.as_str())
        };
        match other_type {
            "company" => company_peer_ids.push(other_id),
            "person" => person_peer_ids.push(other_id),
            _ => {}
        }
        relationship_seeds.push((
            edge.edge_type.clone(),
            other_type.to_string(),
            other_id,
            edge.confidence
                .map(|c| (c * 100.0).round() as i64)
                .unwrap_or(0),
            edge.last_seen
                .or(edge.first_seen)
                .map(|d| d.format("%Y-%m-%d").to_string())
                .unwrap_or_default(),
        ));
    }
    company_peer_ids.sort();
    company_peer_ids.dedup();
    person_peer_ids.sort();
    person_peer_ids.dedup();
    let company_peer_names: std::collections::HashMap<Uuid, String> = {
        let state = DataState::from_result(
            store.get_company_names_by_ids(&company_peer_ids).await,
            "failed to resolve relationship company names",
            Vec::is_empty,
        );
        DegradedNotice::capture(&state, &mut degraded_notice);
        state
            .into_loaded_or_default()
            .into_iter()
            .map(|(peer_id, name, _, _)| (peer_id, name))
            .collect()
    };
    let person_peer_names: std::collections::HashMap<Uuid, String> = {
        let state = DataState::from_result(
            store.get_person_names_by_ids(&person_peer_ids).await,
            "failed to resolve relationship person names",
            Vec::is_empty,
        );
        DegradedNotice::capture(&state, &mut degraded_notice);
        state
            .into_loaded_or_default()
            .into_iter()
            .map(|(peer_id, name, _)| (peer_id, name))
            .collect()
    };
    let relationships: Vec<EntityRelationship> = relationship_seeds
        .into_iter()
        .map(|(edge_type, kind, peer_id, confidence_pct, last_seen)| {
            let target_name = match kind.as_str() {
                "company" => company_peer_names.get(&peer_id).cloned(),
                "person" => person_peer_names.get(&peer_id).cloned(),
                _ => None,
            }
            .unwrap_or_else(|| peer_id.to_string()[..8].to_string());
            EntityRelationship {
                edge_type,
                target_kind: kind,
                target_id: peer_id.to_string(),
                target_name,
                confidence_pct,
                last_seen,
            }
        })
        .collect();

    // ── Entity workspace: open investigations + pipeline status ─────────
    let workspaces_state = DataState::from_result(
        store.list_investigation_workspaces(100).await,
        "failed to fetch entity investigations",
        Vec::is_empty,
    );
    DegradedNotice::capture(&workspaces_state, &mut degraded_notice);
    let open_investigations: Vec<EntityWorkspaceRef> = workspaces_state
        .into_items()
        .into_iter()
        .filter(|w| {
            w.status != "closed"
                && w.status != "archived"
                && workspace_focuses_entity(&w.entity_focus, &id)
        })
        .map(|w| EntityWorkspaceRef {
            id: w.id.to_string(),
            name: w.name,
            status: w.status,
            updated_at: w.updated_at.format("%Y-%m-%d").to_string(),
        })
        .collect();

    let pipeline_state = DataState::from_result(
        store
            .list_pipeline_opportunities_for_company(uuid, 20)
            .await,
        "failed to fetch entity pipeline",
        Vec::is_empty,
    );
    DegradedNotice::capture(&pipeline_state, &mut degraded_notice);
    let pipeline: Vec<EntityPipelineItem> = pipeline_state
        .into_items()
        .into_iter()
        .map(|p| EntityPipelineItem {
            id: p.id.to_string(),
            title: p.title,
            stage: p.stage,
            probability_pct: (p.probability * 100.0).round() as i64,
            value: p
                .value_estimate
                .map(|v| format!("{v:.0}"))
                .unwrap_or_default(),
        })
        .collect();

    // ── Entity workspace: buying centres (people + decision roles) ──────
    let centers_state = DataState::from_result(
        store.list_buying_centers(uuid).await,
        "failed to fetch entity buying centres",
        Vec::is_empty,
    );
    DegradedNotice::capture(&centers_state, &mut degraded_notice);
    let center_rows = centers_state.into_items();
    type CenterMemberSeed = (Uuid, String, f64, bool);
    let mut center_members: Vec<(String, String, Vec<CenterMemberSeed>)> = Vec::new();
    let mut center_person_ids: Vec<Uuid> = Vec::new();
    for center in &center_rows {
        let members_state = DataState::from_result(
            store.list_buying_center_members(center.id).await,
            "failed to fetch buying centre members",
            Vec::is_empty,
        );
        DegradedNotice::capture(&members_state, &mut degraded_notice);
        let members: Vec<CenterMemberSeed> = members_state
            .into_items()
            .into_iter()
            .map(|m| (m.person_id, m.role, m.influence_score, m.budget_authority))
            .collect();
        for (person_id, _, _, _) in &members {
            center_person_ids.push(*person_id);
        }
        center_members.push((center.name.clone(), center.status.clone(), members));
    }
    center_person_ids.sort();
    center_person_ids.dedup();
    let center_person_names: std::collections::HashMap<Uuid, String> = {
        let state = DataState::from_result(
            store.get_person_names_by_ids(&center_person_ids).await,
            "failed to resolve buying centre person names",
            Vec::is_empty,
        );
        DegradedNotice::capture(&state, &mut degraded_notice);
        state
            .into_loaded_or_default()
            .into_iter()
            .map(|(person_id, name, _)| (person_id, name))
            .collect()
    };
    let buying_centers: Vec<EntityBuyingCentre> = center_members
        .into_iter()
        .map(|(name, status, members)| EntityBuyingCentre {
            name,
            status,
            members: members
                .into_iter()
                .map(
                    |(person_id, role, influence, budget_authority)| EntityBuyingCentreMember {
                        person_id: person_id.to_string(),
                        name: center_person_names
                            .get(&person_id)
                            .cloned()
                            .unwrap_or_else(|| person_id.to_string()[..8].to_string()),
                        role,
                        influence_pct: (influence * 100.0).round() as i64,
                        budget_authority,
                    },
                )
                .collect(),
        })
        .collect();

    let why_now = EntityWhyNow {
        headline,
        latest_signal_id,
        latest_signal_title,
        latest_signal_severity,
        latest_signal_type,
        latest_signal_age,
        open_signals,
        recent_changes: recent_change_count,
        evidence_count,
    };
    let risk_count = warning_rows
        .iter()
        .filter(|w| w.severity == "high" || w.severity == "critical")
        .count() as i64;

    let website = company.domain.clone().unwrap_or_default();
    let website_url = normalize_website_url(&website);

    let is_comp = company
        .metadata
        .as_ref()
        .and_then(|m| m.get("is_competitor"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let tpl = CompanyDetailPage {
        current_path: ctx.current_path,
        can_admin: ctx.can_admin,
        status_strip: ctx.status_strip,
        username: ctx.username,
        warning_count: ctx.warning_count,
        theme: ctx.theme,
        degraded_notice,
        briefing_mode: query.briefing.unwrap_or(false),
        id: company.id.to_string(),
        name: company.name.clone(),
        sector: company.company_type.clone().unwrap_or_default(),
        region: company.region.clone().unwrap_or_default(),
        description: company.legal_name.clone().unwrap_or_default(),
        website,
        website_url,
        risk_score: company.risk_score.map(|s| (s * 100.0) as i64).unwrap_or(0),
        is_competitor: is_comp,
        created_at: company
            .created_at
            .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_default(),
        updated_at: company
            .updated_at
            .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_default(),
        key_persons,
        sites,
        products,
        recent_events,
        dossier_entries,
        warnings,
        insights,
        financials: vec![],
        total_warnings,
        total_insights,
        risk_count,
        why_now,
        evidence_timeline,
        relationships,
        open_investigations,
        pipeline,
        buying_centers,
    };

    super::render_template(&tpl)
}

/// GET /companies/:id/changes — HTMX partial: company changes tab.
pub async fn company_changes_tab(
    Extension(store): Extension<Arc<PgStore>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let uuid = match Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Html("Invalid company ID".to_string()),
            )
                .into_response()
        }
    };

    let change_rows = store
        .get_company_changes(uuid, 50)
        .await
        .unwrap_or_default();
    let events: Vec<CompanyEvent> = change_rows
        .iter()
        .map(|c| CompanyEvent {
            kind: c.change_type.clone(),
            description: c.description.clone().unwrap_or_default(),
            date: c
                .detected_at
                .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_default(),
        })
        .collect();

    let partial = CompanyChangesTabPartial { events };
    super::render_template(&partial)
}

/// GET /companies/:id/dossier — HTMX partial: company dossier tab.
pub async fn company_dossier_tab(
    Extension(store): Extension<Arc<PgStore>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let uuid = match Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Html("Invalid company ID".to_string()),
            )
                .into_response()
        }
    };

    let entries_state = DataState::from_result(
        store.get_dossier_entries("company", uuid, None, 100).await,
        "get_dossier_entries failed (web company dossier tab)",
        Vec::is_empty,
    );
    let mut degraded_notice: Option<String> = None;
    DegradedNotice::capture(&entries_state, &mut degraded_notice);
    let entries_raw = entries_state.into_items();
    let dossier_entries: Vec<DossierEntry> = entries_raw
        .iter()
        .map(|e| DossierEntry {
            field_name: e.title.clone(),
            value: e.content.clone(),
            source: e
                .source_urls
                .as_ref()
                .and_then(|u| u.first())
                .cloned()
                .unwrap_or_default(),
            verified: e.verified.unwrap_or(false),
        })
        .collect();

    let partial = CompanyDossierTabPartial {
        dossier_entries,
        degraded_notice,
    };
    super::render_template(&partial)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn partial(degraded_notice: Option<String>) -> CompaniesListPartial {
        CompaniesListPartial {
            degraded_notice,
            companies: vec![],
            total: 0,
            page: 1,
            per_page: 25,
            total_pages: 0,
            active_region: String::new(),
            active_sector: String::new(),
            search_query: String::new(),
            sort_field: "name".into(),
            sort_dir: "asc".into(),
            competitor_count: 0,
            high_risk_count: 0,
            regions_count: 0,
            avg_risk: 0,
            region_slices: vec![],
            quick_links: vec![],
            region_filters: vec![],
            tier_filters: vec![],
            active_filters: 0,
            reset_href: "/companies".into(),
            page_base_href: "/companies?".into(),
        }
    }

    #[test]
    fn degraded_company_query_renders_degraded_marker_not_no_results() {
        let html = partial(Some(
            "Data unavailable — query failed at 14:03 UTC · incident inc-co123456".into(),
        ))
        .render()
        .expect("companies partial renders");

        assert!(html.contains("incident inc-co123456"));
        assert!(html.contains("data-degraded=\"true\""));
        assert!(!html.contains("No companies found"));
    }

    #[test]
    fn empty_company_result_keeps_empty_state_when_query_succeeded() {
        let html = partial(None).render().expect("companies partial renders");

        assert!(html.contains("No companies found"));
        assert!(!html.contains("data-degraded=\"true\""));
    }

    fn detail_page() -> CompanyDetailPage {
        let company_id = "c0ffee00-0000-4000-8000-000000000001";
        let person_id = "beef0000-0000-4000-8000-000000000001";
        CompanyDetailPage {
            current_path: format!("/companies/{company_id}"),
            can_admin: true,
            username: "admin".into(),
            warning_count: 1,
            theme: "light".into(),
            status_strip: crate::system_status::StatusStrip::unknown(),
            briefing_mode: false,
            id: company_id.into(),
            name: "Northwind Power Systems".into(),
            sector: "manufacturer".into(),
            region: "US".into(),
            description: "Grid-scale storage manufacturer.".into(),
            website: "northwind-power.test".into(),
            website_url: "https://northwind-power.test".into(),
            risk_score: 64,
            is_competitor: false,
            created_at: "2026-01-10 09:00".into(),
            updated_at: "2026-01-16 09:00".into(),
            key_persons: vec![CompanyKeyPerson {
                person_id: person_id.into(),
                name: "Dana Whitfield".into(),
                role: "Chief Executive Officer".into(),
                since: "2026-01-10".into(),
                background_snippet: "Leads the grid-scale expansion.".into(),
            }],
            sites: vec![],
            products: vec![],
            recent_events: vec![CompanyEvent {
                kind: "leadership_change".into(),
                description: "New VP Procurement appointed.".into(),
                date: "2026-01-15 09:00".into(),
            }],
            dossier_entries: vec![],
            warnings: vec![CompanyWarning {
                id: "0ddba110-0000-4000-8000-000000000001".into(),
                title: "Northwind Power expands cell manufacturing capacity".into(),
                warning_type: "capacity_alert".into(),
                severity: "high".into(),
                confidence_pct: 83,
                created_at: "2026-01-15".into(),
                age: "2d ago".into(),
                acknowledged: false,
            }],
            insights: vec![],
            financials: vec![],
            total_warnings: 1,
            total_insights: 0,
            risk_count: 1,
            why_now: EntityWhyNow {
                headline: "High capacity_alert signal 2d ago — 1 signal(s) need review.".into(),
                latest_signal_id: "0ddba110-0000-4000-8000-000000000001".into(),
                latest_signal_title: "Northwind Power expands cell manufacturing capacity".into(),
                latest_signal_severity: "high".into(),
                latest_signal_type: "capacity_alert".into(),
                latest_signal_age: "2d ago".into(),
                open_signals: 1,
                recent_changes: 1,
                evidence_count: 1,
            },
            evidence_timeline: vec![EntityEvidenceItem {
                kind: "signal:capacity_alert".into(),
                title: "Northwind Power expands cell manufacturing capacity".into(),
                source: "example.test".into(),
                url: "https://example.test/reports/northwind-capacity-expansion".into(),
                at: "2026-01-15 08:30".into(),
                detail: "Permit filings indicate expansion.".into(),
            }],
            relationships: vec![EntityRelationship {
                edge_type: "company_person".into(),
                target_kind: "person".into(),
                target_id: person_id.into(),
                target_name: "Dana Whitfield".into(),
                confidence_pct: 90,
                last_seen: "2026-01-16".into(),
            }],
            open_investigations: vec![EntityWorkspaceRef {
                id: "aaaa1111-0000-4000-8000-000000000001".into(),
                name: "Northwind capacity investigation".into(),
                status: "active".into(),
                updated_at: "2026-01-16".into(),
            }],
            pipeline: vec![EntityPipelineItem {
                id: "9e7e0000-0000-4000-8000-000000000001".into(),
                title: "Northwind grid storage capacity opportunity".into(),
                stage: "discovery".into(),
                probability_pct: 35,
                value: "250000".into(),
            }],
            buying_centers: vec![EntityBuyingCentre {
                name: "Northwind Power Systems Buying Center".into(),
                status: "engaged".into(),
                members: vec![EntityBuyingCentreMember {
                    person_id: person_id.into(),
                    name: "Dana Whitfield".into(),
                    role: "decision_maker".into(),
                    influence_pct: 92,
                    budget_authority: true,
                }],
            }],
            degraded_notice: None,
        }
    }

    #[test]
    fn entity_workspace_renders_every_required_section() {
        let html = detail_page().render().expect("company detail renders");

        for needle in [
            "Why now",
            "People &amp; buying centre",
            "Evidence timeline",
            "Risks",
            "Opportunities",
            "Relationships",
            "Watch alerts",
            "Open investigations",
            "Pipeline status",
        ] {
            assert!(html.contains(needle), "entity workspace missing {needle}");
        }
        assert!(html.contains("/persons/beef0000-0000-4000-8000-000000000001"));
        assert!(html.contains("/workspaces/aaaa1111-0000-4000-8000-000000000001"));
        assert!(html.contains("data-action=\"start-investigation\""));
        assert!(
            html.contains("data-claim-source")
                || html.contains("data-evidence-source")
                || html.contains("Evidence timeline")
        );
    }

    #[test]
    fn workspace_focus_matches_id_arrays_and_objects() {
        let company_id = "c0ffee00-0000-4000-8000-000000000001";
        assert!(workspace_focuses_entity(
            &serde_json::json!([company_id]),
            company_id
        ));
        assert!(workspace_focuses_entity(
            &serde_json::json!([{ "id": company_id }]),
            company_id
        ));
        assert!(workspace_focuses_entity(
            &serde_json::json!({ "entity_id": company_id }),
            company_id
        ));
        assert!(!workspace_focuses_entity(
            &serde_json::json!([]),
            company_id
        ));
        assert!(!workspace_focuses_entity(
            &serde_json::json!(["other"]),
            company_id
        ));
    }

    #[test]
    fn humanize_age_reports_recency_buckets() {
        let now = chrono::Utc::now();
        assert_eq!(humanize_age(now), "just now");
        assert_eq!(humanize_age(now - chrono::Duration::minutes(5)), "5m ago");
        assert_eq!(humanize_age(now - chrono::Duration::hours(3)), "3h ago");
        assert_eq!(humanize_age(now - chrono::Duration::days(2)), "2d ago");
    }
}
