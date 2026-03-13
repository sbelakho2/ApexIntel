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
    pub username: String,
    pub warning_count: i64,
    pub theme: String,

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
}

#[derive(Template)]
#[template(path = "pages/company_detail.html")]
pub struct CompanyDetailPage {
    pub current_path: String,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,
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
    pub financials: Vec<CompanyFinancial>,
    pub total_warnings: i64,
    pub total_insights: i64,
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

    let company_rows = store
        .list_companies(&filters, order_by, desc, 2000, 0)
        .await
        .unwrap_or_else(|e| {
            tracing::error!("Failed to list companies: {e}");
            vec![]
        });

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
                warning_count: 0,
                insight_count: 0,
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
        slices.sort_by(|a, b| b.count.cmp(&a.count));
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
        Some(&sort_field),
        Some(&sort_dir_str),
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
        username: ctx.username,
        warning_count: ctx.warning_count,
        theme: ctx.theme,
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

    // Fetch sites for this company
    let site_rows = store.get_sites_for_company(uuid).await.unwrap_or_default();
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
    let person_rows = store.list_persons_by_org(uuid).await.unwrap_or_default();
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
        })
        .collect();

    // Fetch product families
    let product_rows = store
        .list_product_families(Some(uuid), 50, 0)
        .await
        .unwrap_or_default();
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

    let recent_events = store
        .get_company_changes(uuid, 50)
        .await
        .unwrap_or_default()
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

    let dossier_entries = store
        .get_dossier_entries("company", uuid, None, 100)
        .await
        .unwrap_or_default()
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
        username: ctx.username,
        warning_count: ctx.warning_count,
        theme: ctx.theme,
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
        financials: vec![],
        total_warnings: 0,
        total_insights: 0,
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

    let entries_raw = store
        .get_dossier_entries("company", uuid, None, 100)
        .await
        .unwrap_or_default();
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

    let partial = CompanyDossierTabPartial { dossier_entries };
    super::render_template(&partial)
}
