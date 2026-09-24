//! Insight handlers — GET /insights (list), GET /insights/:id (detail)
//!
//! Covers: insight list with category/confidence filters, insight detail with
//! evidence, linked entities, and AI analysis section.

use std::sync::Arc;

use askama::Template;
use axum::{
    extract::Form,
    extract::Path,
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse, Redirect},
    Extension,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use url::form_urlencoded::byte_serialize;
use uuid::Uuid;

use super::{is_htmx_request, PageContext};
use crate::middleware::session::WebSession;
use apex_store::postgres::{InsightListFilters, PgStore, WarningListFilters};

// ─── Query params ───────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct InsightsQuery {
    pub page: Option<i64>,
    pub per_page: Option<i64>,
    pub category: Option<String>,
    pub impact: Option<String>,
    pub min_confidence: Option<f64>,
    pub q: Option<String>,
    pub sort: Option<String>,
    pub dir: Option<String>,
    pub bookmarked: Option<bool>,
}

// ─── Template data ──────────────────────────────────────────────────────────

/// One day of insights by type for the trend stacked bar chart.
#[derive(Clone, Debug)]
pub struct InsightTrendDay {
    pub date_label: String,
    pub demand: i64,
    pub competitive: i64,
    pub supply: i64,
    pub security: i64,
    pub macro_s: i64,
    pub regulatory: i64,
    pub predictive: i64,
    pub pricing: i64,
    pub total: i64,
    pub bar_h: i64,
    pub demand_h: i64,
    pub competitive_h: i64,
    pub supply_h: i64,
    pub security_h: i64,
    pub macro_h: i64,
    pub regulatory_h: i64,
    pub predictive_h: i64,
    pub pricing_h: i64,
}

#[derive(Clone, Debug)]
pub struct InsightListItem {
    pub id: String,
    pub title: String,
    pub category: String,
    pub category_label: String,
    pub category_css: String,
    pub confidence: f64,
    pub confidence_pct: i64,
    pub impact_tier: String,
    pub impact_css: String,
    pub company_name: String,
    pub region: String,
    pub created_at: String,
    pub age_label: String,
    pub bookmarked: bool,
    pub tags: Vec<String>,
    pub summary_preview: String,
    pub evidence_count: i64,
    pub source_diversity: String,
}

#[derive(Clone, Debug)]
pub struct InsightEvidence {
    pub index: usize,
    pub source: String,
    pub url: String,
    pub snippet: String,
    pub relevance: i64,
}

#[derive(Clone, Debug)]
pub struct InsightEntity {
    pub kind: String,
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct InsightFilterChip {
    pub label: String,
    pub href: String,
    pub active: bool,
}

#[derive(Clone, Debug)]
pub struct InsightNoteItem {
    pub author: String,
    pub body: String,
    pub created_at: String,
    pub visibility: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct InsightNoteForm {
    pub body: String,
    pub tags: Option<String>,
    pub visibility: Option<String>,
}

fn url_encode_component(input: &str) -> String {
    byte_serialize(input.as_bytes()).collect::<String>()
}

fn build_insights_href(
    category: Option<&str>,
    impact: Option<&str>,
    q: Option<&str>,
    bookmarked: Option<bool>,
    sort: Option<&str>,
    dir: Option<&str>,
) -> String {
    let mut params: Vec<String> = Vec::new();
    for (k, v) in [
        ("category", category),
        ("impact", impact),
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
    if bookmarked.unwrap_or(false) {
        params.push("bookmarked=true".to_string());
    }

    if params.is_empty() {
        "/insights".to_string()
    } else {
        format!("/insights?{}", params.join("&"))
    }
}

fn is_internal_insight_type(insight_type: &str) -> bool {
    let normalized = insight_type.trim().to_ascii_lowercase();
    normalized.starts_with("llm_")
        || normalized == "bias_mitigation"
        || normalized == "hypothesis_ach"
}

fn insight_category_label(raw: &str) -> &'static str {
    match raw {
        "demand_signal" | "demand_procurement" | "demand" => "Demand Signal",
        "supply_risk" | "supply_chain_risk" | "supply_chain" => "Supply Risk",
        "competitive_intel" | "competitive_comparison" | "competitor_market" | "competitor" => {
            "Competitive Intel"
        }
        "security_posture" | "security" => "Security Posture",
        "macro_shift" => "Macro Shift",
        "poi_movement" | "poi" => "POI Movement",
        "predictive_forward" => "Predictive",
        "hypothesis_ach" => "Hypothesis (ACH)",
        "regulatory_policy" => "Regulatory",
        "pricing_market" => "Pricing / Market",
        "geopolitical_analysis" => "Geopolitical",
        "arbitrage_cost_window" => "Arbitrage",
        _ => "Analysis",
    }
}

fn insight_category_css(raw: &str) -> &'static str {
    match raw {
        "demand_signal" | "demand_procurement" | "demand" => {
            "bg-destructive/10 text-destructive border-destructive/30"
        }
        "supply_risk" | "supply_chain_risk" | "supply_chain" => {
            "bg-primary/10 text-primary border-primary/40"
        }
        "competitive_intel" | "competitive_comparison" | "competitor_market" | "competitor" => {
            "bg-secondary border-border"
        }
        "security_posture" | "security" => "bg-secondary/50 border-border text-muted-foreground",
        "predictive_forward" | "hypothesis_ach" => "bg-primary/10 border-primary/40 text-primary",
        "regulatory_policy" | "geopolitical_analysis" => "bg-secondary border-border",
        "pricing_market" | "arbitrage_cost_window" => "bg-primary/10 border-primary/40",
        _ => "bg-secondary border-border",
    }
}

fn impact_tier(confidence: f64) -> (&'static str, &'static str) {
    if confidence >= 0.8 {
        ("Critical", "apex-tier-critical")
    } else if confidence >= 0.7 {
        ("High", "apex-tier-high")
    } else if confidence >= 0.4 {
        ("Medium", "apex-tier-medium")
    } else {
        ("Low", "apex-tier-low")
    }
}

fn relative_age(dt: Option<DateTime<Utc>>) -> String {
    let Some(dt) = dt else { return String::new() };
    let delta = Utc::now() - dt;
    let hours = delta.num_hours();
    if hours < 1 {
        format!("{}m ago", delta.num_minutes().max(1))
    } else if hours < 24 {
        format!("{}h ago", hours)
    } else if hours < 168 {
        format!("{}d ago", delta.num_days())
    } else {
        format!("{}w ago", delta.num_weeks())
    }
}

fn source_diversity_label(evidence_urls: &[String]) -> &'static str {
    let mut domains: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for url in evidence_urls {
        if let Some(domain) = url.split('/').nth(2) {
            domains.insert(domain);
        }
    }
    match domains.len() {
        0 => "None",
        1 => "Single",
        2..=3 => "Moderate",
        _ => "High",
    }
}

/// Map an insight_type string to one of the 8 trend chart buckets.
fn trend_bucket(kind: &str) -> &'static str {
    let k = kind.to_ascii_lowercase();
    if k.contains("demand") || k.contains("procurement") {
        "demand"
    } else if k.contains("competitive") || k.contains("competitor") {
        "competitive"
    } else if k.contains("supply") || k.contains("arbitrage") {
        "supply"
    } else if k.contains("security") {
        "security"
    } else if k.contains("regulat") || k.contains("geopolit") {
        "regulatory"
    } else if k.contains("predict") || k.contains("hypothes") || k.contains("bias") {
        "predictive"
    } else if k.contains("pricing") || k.contains("market") {
        "pricing"
    } else {
        "macro"
    }
}

// ─── Templates ──────────────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "pages/insights.html")]
pub struct InsightsListPage {
    pub current_path: String,
    pub can_admin: bool,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,
    pub status_strip: crate::system_status::StatusStrip,

    pub insights: Vec<InsightListItem>,
    pub total: i64,
    pub page: i64,
    pub per_page: i64,
    pub total_pages: i64,
    pub active_category: String,
    pub active_impact: String,
    pub search_query: String,
    pub sort_field: String,
    pub sort_dir: String,
    pub show_bookmarked_only: bool,
    pub avg_confidence_pct: i64,
    pub insight_type_count: i64,
    pub high_impact_count: i64,
    pub medium_impact_count: i64,
    pub insight_trend: Vec<InsightTrendDay>,
    pub category_filters: Vec<InsightFilterChip>,
    pub impact_filters: Vec<InsightFilterChip>,
    pub bookmarked_filters: Vec<InsightFilterChip>,
    pub active_filters: i64,
    pub reset_href: String,
    pub page_base_href: String,
}

/// HTMX partial — just the results fragment (no base layout).
#[derive(Template)]
#[template(path = "pages/insights/_list.html")]
pub struct InsightsListPartial {
    pub insights: Vec<InsightListItem>,
    pub total: i64,
    pub page: i64,
    pub per_page: i64,
    pub total_pages: i64,
    pub active_category: String,
    pub active_impact: String,
    pub search_query: String,
    pub sort_field: String,
    pub sort_dir: String,
    pub show_bookmarked_only: bool,
    pub avg_confidence_pct: i64,
    pub insight_type_count: i64,
    pub high_impact_count: i64,
    pub medium_impact_count: i64,
    pub insight_trend: Vec<InsightTrendDay>,
    pub category_filters: Vec<InsightFilterChip>,
    pub impact_filters: Vec<InsightFilterChip>,
    pub bookmarked_filters: Vec<InsightFilterChip>,
    pub active_filters: i64,
    pub reset_href: String,
    pub page_base_href: String,
}

#[derive(Template)]
#[template(path = "pages/insight_detail.html")]
pub struct InsightDetailPage {
    pub current_path: String,
    pub can_admin: bool,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,
    pub status_strip: crate::system_status::StatusStrip,

    pub id: String,
    pub title: String,
    pub category: String,
    pub category_label: String,
    pub category_css: String,
    pub confidence: i64,
    pub impact_tier: String,
    pub impact_css: String,
    pub summary: String,
    pub body: String,
    pub company_name: String,
    pub company_id: String,
    pub region: String,
    pub created_at: String,
    pub updated_at: String,
    pub age_label: String,
    pub bookmarked: bool,
    pub tags: Vec<String>,
    pub evidence: Vec<InsightEvidence>,
    pub evidence_count: usize,
    pub source_diversity: String,
    pub entities: Vec<InsightEntity>,
    pub annotations: Vec<InsightNoteItem>,
    pub ai_analysis: Option<String>,
    pub information_gain_bits: Option<String>,
    pub quality_score_pct: Option<i64>,
    pub dissenting_opinions: Vec<DissentingView>,
}

pub struct DissentingView {
    pub severity: String,
    pub category: String,
    pub confidence_pct: i64,
    pub rationale: String,
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

/// GET /insights — paginated insight list.
pub async fn list_insights(
    headers: HeaderMap,
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
    axum::extract::Query(params): axum::extract::Query<InsightsQuery>,
) -> impl IntoResponse {
    let active_category = params.category.clone().unwrap_or_default();
    let active_impact = params.impact.clone().unwrap_or_default();
    let search_query = params.q.clone().unwrap_or_default();
    let sort_field = params.sort.clone().unwrap_or_else(|| "created_at".into());
    let sort_dir = params.dir.clone().unwrap_or_else(|| "desc".into());
    let show_bookmarked = params.bookmarked.unwrap_or(false);

    let category_values = [
        "",
        "demand_signal",
        "supply_risk",
        "competitive_intel",
        "security_posture",
        "macro_shift",
        "regulatory_policy",
        "predictive_forward",
        "pricing_market",
        "geopolitical_analysis",
        "arbitrage_cost_window",
        "poi_movement",
    ];
    let impact_values = ["", "high", "medium", "low"];

    let category_filters = category_values
        .iter()
        .map(|value| InsightFilterChip {
            label: if value.is_empty() {
                "All".to_string()
            } else {
                value.replace('_', " ")
            },
            href: build_insights_href(
                if value.is_empty() { None } else { Some(*value) },
                if active_impact.is_empty() {
                    None
                } else {
                    Some(active_impact.as_str())
                },
                if search_query.is_empty() {
                    None
                } else {
                    Some(search_query.as_str())
                },
                Some(show_bookmarked),
                Some(sort_field.as_str()),
                Some(sort_dir.as_str()),
            ),
            active: active_category == *value,
        })
        .collect::<Vec<_>>();

    let impact_filters = impact_values
        .iter()
        .map(|value| InsightFilterChip {
            label: if value.is_empty() { "All" } else { value }.to_string(),
            href: build_insights_href(
                if active_category.is_empty() {
                    None
                } else {
                    Some(active_category.as_str())
                },
                if value.is_empty() { None } else { Some(*value) },
                if search_query.is_empty() {
                    None
                } else {
                    Some(search_query.as_str())
                },
                Some(show_bookmarked),
                Some(sort_field.as_str()),
                Some(sort_dir.as_str()),
            ),
            active: active_impact == *value,
        })
        .collect::<Vec<_>>();

    let bookmarked_filters = vec![
        InsightFilterChip {
            label: "All".to_string(),
            href: build_insights_href(
                if active_category.is_empty() {
                    None
                } else {
                    Some(active_category.as_str())
                },
                if active_impact.is_empty() {
                    None
                } else {
                    Some(active_impact.as_str())
                },
                if search_query.is_empty() {
                    None
                } else {
                    Some(search_query.as_str())
                },
                Some(false),
                Some(sort_field.as_str()),
                Some(sort_dir.as_str()),
            ),
            active: !show_bookmarked,
        },
        InsightFilterChip {
            label: "Bookmarked".to_string(),
            href: build_insights_href(
                if active_category.is_empty() {
                    None
                } else {
                    Some(active_category.as_str())
                },
                if active_impact.is_empty() {
                    None
                } else {
                    Some(active_impact.as_str())
                },
                if search_query.is_empty() {
                    None
                } else {
                    Some(search_query.as_str())
                },
                Some(true),
                Some(sort_field.as_str()),
                Some(sort_dir.as_str()),
            ),
            active: show_bookmarked,
        },
    ];

    let active_filters = i64::from(!active_category.is_empty())
        + i64::from(!active_impact.is_empty())
        + i64::from(!search_query.is_empty())
        + i64::from(show_bookmarked);
    let reset_href = build_insights_href(
        None,
        None,
        None,
        Some(false),
        Some(sort_field.as_str()),
        Some(sort_dir.as_str()),
    );
    let current_filters_href = build_insights_href(
        if active_category.is_empty() {
            None
        } else {
            Some(active_category.as_str())
        },
        if active_impact.is_empty() {
            None
        } else {
            Some(active_impact.as_str())
        },
        if search_query.is_empty() {
            None
        } else {
            Some(search_query.as_str())
        },
        Some(show_bookmarked),
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

    let filters = InsightListFilters {
        insight_types: if active_category.is_empty() {
            vec![]
        } else {
            vec![active_category.clone()]
        },
        search: if search_query.is_empty() {
            None
        } else {
            Some(search_query.clone())
        },
        bookmarked_by: if show_bookmarked {
            Some(session.username.clone())
        } else {
            None
        },
        exclude_internal: true,
        ..Default::default()
    };

    let all_insight_rows = store
        .list_insights(&filters, 1500, 0)
        .await
        .unwrap_or_else(|e| {
            tracing::error!("Failed to list insights: {e}");
            vec![]
        });

    // Hide internal quality-loop telemetry from user-facing Insights UI.
    let visible_insight_rows: Vec<_> = all_insight_rows
        .into_iter()
        .filter(|row| {
            !row.insight_type
                .as_deref()
                .map(is_internal_insight_type)
                .unwrap_or(false)
        })
        .collect();

    let mut all_insights: Vec<InsightListItem> = visible_insight_rows
        .iter()
        .map(|i| {
            let confidence = i.confidence.unwrap_or(0.0);
            let raw_cat = i.insight_type.clone().unwrap_or_default();
            let (tier, tier_css) = impact_tier(confidence);
            let ev_urls = i.evidence_urls.as_deref().unwrap_or(&[]);
            InsightListItem {
                id: i.id.to_string(),
                title: i.title.clone(),
                category: raw_cat.clone(),
                category_label: insight_category_label(&raw_cat).to_string(),
                category_css: insight_category_css(&raw_cat).to_string(),
                confidence,
                confidence_pct: confidence_to_pct(confidence),
                impact_tier: tier.to_string(),
                impact_css: tier_css.to_string(),
                company_name: String::new(),
                region: i.region.clone().unwrap_or_default(),
                created_at: insight_display_time(i)
                    .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
                    .unwrap_or_default(),
                age_label: relative_age(insight_display_time(i)),
                bookmarked: false,
                tags: i.tags.clone().unwrap_or_default(),
                summary_preview: {
                    let s = i.summary.trim();
                    if s.len() > 160 {
                        format!(
                            "{}…",
                            &s[..s.char_indices().nth(160).map(|(i, _)| i).unwrap_or(s.len())]
                        )
                    } else {
                        s.to_string()
                    }
                },
                evidence_count: ev_urls.len() as i64,
                source_diversity: source_diversity_label(ev_urls).to_string(),
            }
        })
        .collect();

    all_insights.sort_by(|a, b| {
        let category_cmp = a.category == b.category;
        if !category_cmp {
            return std::cmp::Ordering::Equal;
        }
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut diversified = Vec::with_capacity(all_insights.len());
    let mut remaining = all_insights;
    let mut last_category: Option<String> = None;
    let mut consecutive = 0usize;
    while !remaining.is_empty() {
        let selected_idx = remaining
            .iter()
            .position(|item| match last_category.as_deref() {
                Some(previous) if previous == item.category => consecutive < 2,
                _ => true,
            })
            .unwrap_or(0);
        let selected = remaining.remove(selected_idx);
        if last_category.as_deref() == Some(selected.category.as_str()) {
            consecutive += 1;
        } else {
            last_category = Some(selected.category.clone());
            consecutive = 1;
        }
        diversified.push(selected);
    }
    let mut all_insights = diversified;

    if active_impact == "high" {
        all_insights.retain(|i| i.confidence >= 0.7);
    } else if active_impact == "medium" {
        all_insights.retain(|i| i.confidence >= 0.4 && i.confidence < 0.7);
    } else if active_impact == "low" {
        all_insights.retain(|i| i.confidence < 0.4);
    }

    // Apply user's explicit sort preference (overrides diversification order).
    match sort_field.as_str() {
        "created_at" => {
            if sort_dir == "asc" {
                all_insights.sort_by(|a, b| a.created_at.cmp(&b.created_at));
            } else {
                all_insights.sort_by(|a, b| b.created_at.cmp(&a.created_at));
            }
        }
        "confidence" => {
            if sort_dir == "asc" {
                all_insights.sort_by(|a, b| {
                    a.confidence
                        .partial_cmp(&b.confidence)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
            } else {
                all_insights.sort_by(|a, b| {
                    b.confidence
                        .partial_cmp(&a.confidence)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
            }
        }
        _ => {} // keep diversification order for unknown sort fields
    }

    let total = all_insights.len() as i64;
    let total_pages = if total == 0 {
        0
    } else {
        (total + per_page - 1) / per_page
    };

    let avg_confidence_pct = if all_insights.is_empty() {
        0
    } else {
        let avg_confidence =
            all_insights.iter().map(|i| i.confidence).sum::<f64>() / all_insights.len() as f64;
        confidence_to_pct(avg_confidence)
    };
    let insight_type_count = {
        use std::collections::HashSet;
        let mut kinds: HashSet<&str> = HashSet::new();
        for item in &all_insights {
            if !item.category.is_empty() {
                kinds.insert(item.category.as_str());
            }
        }
        kinds.len() as i64
    };
    let high_impact_count = all_insights.iter().filter(|i| i.confidence >= 0.7).count() as i64;
    let medium_impact_count = all_insights
        .iter()
        .filter(|i| i.confidence >= 0.4 && i.confidence < 0.7)
        .count() as i64;

    let start = ((page - 1) * per_page) as usize;
    let end = (start + per_page as usize).min(all_insights.len());
    let insights = if start < all_insights.len() {
        all_insights[start..end].to_vec()
    } else {
        Vec::new()
    };

    // Generate real 30-day insight trend from DB rows.
    let insight_trend: Vec<InsightTrendDay> = {
        use chrono::{Duration, Utc};
        use std::collections::BTreeMap;
        // 8 buckets: demand, competitive, supply, security, macro, regulatory, predictive, pricing
        let mut by_day: BTreeMap<String, [i64; 8]> = BTreeMap::new();
        for i in 0..30 {
            let label = (Utc::now() - Duration::days(29 - i))
                .format("%b %d")
                .to_string();
            by_day.insert(label, [0; 8]);
        }
        for row in &visible_insight_rows {
            let confidence = row.confidence.unwrap_or(0.0);
            if active_impact == "high" && confidence < 0.7 {
                continue;
            }
            if active_impact == "medium" && !(0.4..0.7).contains(&confidence) {
                continue;
            }
            if active_impact == "low" && confidence >= 0.4 {
                continue;
            }

            let Some(event_time) = insight_display_time(row) else {
                continue;
            };
            let date = event_time.format("%b %d").to_string();
            if let Some(buckets) = by_day.get_mut(&date) {
                let kind = row.insight_type.clone().unwrap_or_default();
                match trend_bucket(&kind) {
                    "demand" => buckets[0] += 1,
                    "competitive" => buckets[1] += 1,
                    "supply" => buckets[2] += 1,
                    "security" => buckets[3] += 1,
                    "regulatory" => buckets[5] += 1,
                    "predictive" => buckets[6] += 1,
                    "pricing" => buckets[7] += 1,
                    _ => buckets[4] += 1, // macro
                }
            }
        }
        let raw: Vec<(String, [i64; 8])> = by_day.into_iter().collect();

        let max_total = raw
            .iter()
            .map(|(_, b)| b.iter().sum::<i64>())
            .max()
            .unwrap_or(1)
            .max(1);
        raw.into_iter()
            .map(|(label, b)| {
                let tot: i64 = b.iter().sum();
                let bar_h = tot * 100 / max_total;
                let pct = |v: i64| if tot == 0 { 0 } else { v * 100 / tot };
                InsightTrendDay {
                    date_label: label,
                    demand: b[0],
                    competitive: b[1],
                    supply: b[2],
                    security: b[3],
                    macro_s: b[4],
                    regulatory: b[5],
                    predictive: b[6],
                    pricing: b[7],
                    total: tot,
                    bar_h,
                    demand_h: pct(b[0]),
                    competitive_h: pct(b[1]),
                    supply_h: pct(b[2]),
                    security_h: pct(b[3]),
                    macro_h: pct(b[4]),
                    regulatory_h: pct(b[5]),
                    predictive_h: pct(b[6]),
                    pricing_h: pct(b[7]),
                }
            })
            .collect()
    };

    let unack = store
        .count_warnings(&WarningListFilters {
            acknowledged: Some(false),
            ..Default::default()
        })
        .await
        .unwrap_or(0);
    let ctx = PageContext::from_session(&session, "/insights", unack);

    let tpl = InsightsListPage {
        current_path: ctx.current_path,
        can_admin: ctx.can_admin,
        status_strip: crate::system_status::StatusStrip::current(),
        username: ctx.username,
        warning_count: ctx.warning_count,
        theme: ctx.theme,
        insights,
        total,
        page,
        per_page,
        total_pages,
        active_category,
        active_impact,
        search_query,
        sort_field,
        sort_dir,
        show_bookmarked_only: show_bookmarked,
        avg_confidence_pct,
        insight_type_count,
        high_impact_count,
        medium_impact_count,
        insight_trend,
        category_filters,
        impact_filters,
        bookmarked_filters,
        active_filters,
        reset_href,
        page_base_href,
    };

    if is_htmx_request(&headers) {
        let partial = InsightsListPartial {
            insights: tpl.insights.clone(),
            total: tpl.total,
            page: tpl.page,
            per_page: tpl.per_page,
            total_pages: tpl.total_pages,
            active_category: tpl.active_category.clone(),
            active_impact: tpl.active_impact.clone(),
            search_query: tpl.search_query.clone(),
            sort_field: tpl.sort_field.clone(),
            sort_dir: tpl.sort_dir.clone(),
            show_bookmarked_only: tpl.show_bookmarked_only,
            avg_confidence_pct: tpl.avg_confidence_pct,
            insight_type_count: tpl.insight_type_count,
            high_impact_count: tpl.high_impact_count,
            medium_impact_count: tpl.medium_impact_count,
            insight_trend: tpl.insight_trend.clone(),
            category_filters: tpl.category_filters.clone(),
            impact_filters: tpl.impact_filters.clone(),
            bookmarked_filters: tpl.bookmarked_filters.clone(),
            active_filters: tpl.active_filters,
            reset_href: tpl.reset_href.clone(),
            page_base_href: tpl.page_base_href.clone(),
        };
        super::render_template(&partial)
    } else {
        super::render_template(&tpl)
    }
}

/// GET /insights/:id — single insight detail.
pub async fn get_insight(
    _headers: HeaderMap,
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let unack = store
        .count_warnings(&WarningListFilters {
            acknowledged: Some(false),
            ..Default::default()
        })
        .await
        .unwrap_or(0);
    let ctx = PageContext::from_session(&session, "/insights", unack);

    let uuid = match Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => {
            return super::errors::not_found_with_context(
                &ctx.username,
                "/insights",
                ctx.warning_count,
            );
        }
    };

    let insight = match store.get_insight(uuid).await {
        Ok(Some(i)) => i,
        Ok(None) => {
            return Redirect::to("/insights").into_response();
        }
        Err(e) => {
            tracing::error!("Failed to fetch insight {id}: {e}");
            return super::errors::not_found_with_context(
                &ctx.username,
                "/insights",
                ctx.warning_count,
            );
        }
    };

    // Build evidence from evidence_urls
    let base_confidence_pct = confidence_to_pct(insight.confidence.unwrap_or(0.0));
    let evidence: Vec<InsightEvidence> = insight
        .evidence_urls
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .enumerate()
        .map(|(idx, url)| {
            let relevance = (base_confidence_pct - (idx as i64 * 8)).clamp(35, 100);
            InsightEvidence {
                index: idx + 1,
                source: url.split('/').nth(2).unwrap_or("unknown").to_string(),
                url: url.clone(),
                snippet: format!("Source [{}]", idx + 1),
                relevance,
            }
        })
        .collect();

    let raw_type = insight.insight_type.clone().unwrap_or_default();
    let conf = insight.confidence.unwrap_or(0.0);
    let (tier, tier_css) = impact_tier(conf);
    let assessment_severity = detail_assessment_severity(&insight, conf);
    let ev_urls = insight.evidence_urls.clone().unwrap_or_default();
    let diversity = source_diversity_label(&ev_urls);

    let tpl = InsightDetailPage {
        current_path: ctx.current_path,
        can_admin: ctx.can_admin,
        status_strip: crate::system_status::StatusStrip::current(),
        username: ctx.username,
        warning_count: ctx.warning_count,
        theme: ctx.theme,
        id: insight.id.to_string(),
        title: insight.title.clone(),
        category: raw_type.clone(),
        category_label: insight_category_label(&raw_type).to_string(),
        category_css: insight_category_css(&raw_type).to_string(),
        confidence: confidence_to_pct(conf),
        impact_tier: tier.to_string(),
        impact_css: tier_css.to_string(),
        summary: insight.summary.clone(),
        body: insight.summary.clone(),
        company_name: String::new(),
        company_id: String::new(),
        region: insight.region.clone().unwrap_or_default(),
        created_at: insight
            .created_at
            .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_default(),
        updated_at: insight
            .updated_at
            .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_default(),
        age_label: relative_age(insight.created_at),
        bookmarked: store
            .get_bookmarked_insight_ids(&session.username, &[insight.id])
            .await
            .map(|ids| ids.contains(&insight.id))
            .unwrap_or(false),
        tags: insight.tags.clone().unwrap_or_default(),
        evidence_count: evidence.len(),
        source_diversity: diversity.to_string(),
        evidence,
        entities: vec![],
        annotations: store
            .list_annotations(&session.username, Some("insight"), Some(&id))
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|annotation| InsightNoteItem {
                author: annotation.user_id,
                body: annotation.body,
                created_at: annotation.updated_at.format("%Y-%m-%d %H:%M").to_string(),
                visibility: annotation.visibility,
                tags: annotation.tags,
            })
            .collect(),
        ai_analysis: None,
        information_gain_bits: Some(format!(
            "{:.2}",
            detail_information_gain_bits(conf, &assessment_severity)
        )),
        quality_score_pct: {
            let scores = store
                .get_insight_feedback_scores(&[insight.id])
                .await
                .unwrap_or_default();
            scores.get(&insight.id).map(|s| (s * 100.0).round() as i64)
        },
        dissenting_opinions: insight
            .metadata
            .as_ref()
            .and_then(|m| m.get("dissenting_opinions"))
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| {
                        let severity = item.get("severity")?.as_str()?.to_string();
                        let category = item.get("category")?.as_str()?.to_string();
                        let confidence = item.get("confidence")?.as_f64().unwrap_or(0.0);
                        let rationale = item.get("rationale_summary")?.as_str()?.to_string();
                        Some(DissentingView {
                            severity,
                            category,
                            confidence_pct: (confidence * 100.0).round() as i64,
                            rationale,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default(),
    };

    super::render_template(&tpl)
}

fn confidence_to_pct(value: f64) -> i64 {
    if value <= 1.0 {
        (value * 100.0).round() as i64
    } else {
        value.round() as i64
    }
}

fn detail_information_gain_bits(confidence: f64, severity: &str) -> f64 {
    let prior = [0.70, 0.20, 0.10];
    let target_state = match severity.trim().to_ascii_lowercase().as_str() {
        "critical" | "high" => 2,
        "warning" | "medium" => 1,
        _ => 0,
    };

    let confidence = confidence.clamp(0.0, 1.0);
    let mut posterior = [0.0; 3];
    for (idx, probability) in prior.iter().enumerate() {
        posterior[idx] = probability * (1.0 - confidence);
    }
    posterior[target_state] += confidence;

    let total: f64 = posterior.iter().sum();
    if total > 0.0 {
        for probability in &mut posterior {
            *probability /= total;
        }
    }

    let entropy = |distribution: &[f64; 3]| {
        distribution
            .iter()
            .copied()
            .filter(|probability| *probability > 0.0)
            .map(|probability| -probability * probability.log2())
            .sum::<f64>()
    };

    (entropy(&prior) - entropy(&posterior)).max(0.0)
}

fn detail_assessment_severity(
    insight: &apex_store::postgres::InsightRow,
    confidence: f64,
) -> String {
    insight
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.get("assessment_severity"))
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| impact_tier(confidence).0.to_ascii_lowercase())
}

fn insight_display_time(row: &apex_store::postgres::InsightRow) -> Option<DateTime<Utc>> {
    row.created_at.or(row.updated_at)
}

/// POST /insights/:id/bookmark — toggle bookmark, return updated card fragment.
pub async fn bookmark_insight_html(
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let uuid = match Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Html("Invalid insight ID".to_string()),
            )
                .into_response()
        }
    };

    // Try to bookmark; if already bookmarked, unbookmark instead (toggle)
    let bookmarked = match store.bookmark_insight(uuid, &session.username, None).await {
        Ok(true) => true, // newly bookmarked
        Ok(false) => {
            // Already bookmarked — remove it
            let _ = store.unbookmark_insight(uuid, &session.username).await;
            false
        }
        Err(e) => {
            tracing::error!("Failed to toggle bookmark for insight {id}: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Html("Failed to toggle bookmark".to_string()),
            )
                .into_response();
        }
    };

    if bookmarked {
        let _ = store
            .record_insight_feedback(uuid, &session.username, "bookmarked", None)
            .await;
    }

    let _ = store
        .create_notification(
            &session.username,
            "insight_bookmark",
            if bookmarked {
                "Insight bookmarked"
            } else {
                "Insight bookmark removed"
            },
            &format!(
                "Insight {id} bookmark state changed by {}.",
                session.username
            ),
            Some("insight"),
            Some(&id),
            Some(&format!("/insights/{id}")),
        )
        .await;

    let icon_fill = if bookmarked { "currentColor" } else { "none" };
    let color_class = if bookmarked {
        "text-rams-orange"
    } else {
        "text-muted-foreground"
    };
    let title_text = if bookmarked {
        "Remove bookmark"
    } else {
        "Bookmark"
    };

    Html(format!(
        r#"<button hx-post="/insights/{id}/bookmark" hx-swap="outerHTML"
             class="shrink-0 rounded-sm border border-border p-1.5 hover:bg-muted {color_class}"
             title="{title_text}">
             <svg class="h-3.5 w-3.5" viewBox="0 0 24 24" fill="{icon_fill}" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M19 21l-7-5-7 5V5a2 2 0 0 1 2-2h10a2 2 0 0 1 2 2z"/></svg>
           </button>"#
    )).into_response()
}

/// POST /insights/:id/analyze — trigger AI analysis, return rendered panel.
pub async fn analyze_insight_html(
    _session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let uuid = match Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Html("Invalid insight ID".to_string()),
            )
                .into_response()
        }
    };

    match store.get_insight(uuid).await {
        Ok(Some(i)) => {
            Html(format!(
                r#"<div class="apex-card p-4" id="analysis-panel">
                     <h2 class="mb-2 text-xs font-black uppercase tracking-[0.12em]">AI Analysis</h2>
                     <p class="text-sm leading-relaxed text-muted-foreground">
                       Analysis in progress for "<strong>{}</strong>". This may take a few moments.
                       The system will evaluate confidence levels, cross-reference entities,
                       and generate actionable recommendations.
                     </p>
                     <div class="mt-3 flex items-center gap-2 text-[10px] text-muted-foreground">
                       <span class="h-2 w-2 rounded-full bg-rams-orange animate-pulse"></span>
                       Processing…
                     </div>
                   </div>"#,
                super::escape_html(&i.title)
            )).into_response()
        }
        Ok(None) => (StatusCode::NOT_FOUND, Html("Insight not found".to_string())).into_response(),
        Err(e) => {
            tracing::error!("Failed to fetch insight for analysis {id}: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, Html("Failed to start analysis".to_string())).into_response()
        }
    }
}

pub async fn create_insight_note(
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
    Path(id): Path<String>,
    Form(form): Form<InsightNoteForm>,
) -> impl IntoResponse {
    let body = form.body.trim();
    if body.len() < 3 {
        return Redirect::to(&format!("/insights/{id}")).into_response();
    }

    let tags = parse_tags(form.tags.as_deref());
    let visibility = form.visibility.as_deref().unwrap_or("team");

    match store
        .upsert_annotation(
            None,
            &session.username,
            "insight",
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
                    "Insight note added",
                    body,
                    Some("insight"),
                    Some(&id),
                    Some(&format!("/insights/{id}")),
                )
                .await;
        }
        Err(error) => {
            tracing::error!(insight_id = %id, error = %error, "failed to create insight note");
        }
    }

    Redirect::to(&format!("/insights/{id}")).into_response()
}

/// GET /insights/:id/pdf — download insight as PDF (session-auth, not API-key).
/// The API route at /api/insights/:id/pdf requires API key auth which the
/// browser session cookie can't provide. This web route does the same PDF
/// generation but uses the session middleware.
pub async fn export_insight_pdf_html(
    Extension(store): Extension<Arc<PgStore>>,
    Path(id): Path<String>,
) -> axum::response::Response {
    use axum::http::{header, StatusCode};
    use axum::response::IntoResponse;

    let uid = match Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => {
            return (StatusCode::BAD_REQUEST, "Invalid insight ID").into_response();
        }
    };

    let insight = match store.get_insight(uid).await {
        Ok(Some(row)) => row,
        Ok(None) => return (StatusCode::NOT_FOUND, "Insight not found").into_response(),
        Err(e) => {
            tracing::error!(%e, "pdf export: failed to fetch insight");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
        }
    };

    let report_row = apex_insights::pdf_report::InsightReportRow {
        id: insight.id.to_string(),
        title: insight.title,
        summary: insight.summary,
        insight_type: insight.insight_type.unwrap_or_default(),
        severity: apex_insights::InsightSeverity::Medium,
        confidence: insight.confidence.unwrap_or(0.5),
        region: insight.region,
        evidence: Vec::new(),
        sources: Vec::new(),
        tags: insight.tags.unwrap_or_default(),
        generated_at: None,
    };

    let report = apex_insights::pdf_report::PdfReport::from_insights(
        &format!("Insight: {}", report_row.title),
        &[report_row],
    );

    let pdf_bytes = match crate::pdf_writer::render_report_to_pdf(&report) {
        Ok(bytes) => bytes,
        Err(e) => {
            tracing::error!(%e, "pdf export: generation failed for insight {id}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "PDF generation failed").into_response();
        }
    };

    let filename = format!("insight-{}.pdf", id);
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/pdf".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{}\"", filename),
            ),
        ],
        axum::body::Body::from(pdf_bytes),
    )
        .into_response()
}
