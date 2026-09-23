//! Web (HTML) handler for the Historical Trends page.
//!
//! Renders the trends page with metric selector, bucket selector,
//! date range picker, inline chart, summary stat cards,
//! and month-over-month / year-over-year comparison cards.

use std::sync::Arc;

use askama::Template;
use axum::{extract::Query, response::IntoResponse, Extension};
use chrono::{Datelike, NaiveDate, Utc};
use serde::Deserialize;

use super::PageContext;
use crate::middleware::session::WebSession;
use apex_store::postgres::trends::{
    BucketType, TrendComparisonQuery, TrendDataPoint, TrendQuery, TrendSummary,
};
use apex_store::postgres::PgStore;

// ─────────────────────────────────────────────────────────────────────────────
// Query parameters
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Default)]
pub struct TrendsPageParams {
    pub metric: Option<String>,
    pub bucket: Option<String>,
    pub range: Option<String>, // "30d", "90d", "1y", "5y", "custom"
    pub from: Option<String>,
    pub to: Option<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Template struct
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "pages/trends.html")]
pub struct TrendsPage {
    // ── base layout fields ──
    pub current_path: String,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,

    // ── filter state ──
    pub selected_metric: String,
    pub selected_bucket: String,
    pub selected_range: String,
    pub from_date: String,
    pub to_date: String,

    // ── trend data ──
    pub data_points: Vec<TrendDataPoint>,
    pub max_val: i64,
    pub max_val_half: i64,
    pub chart_w: i64,
    pub chart_h: i64,
    pub pad: i64,
    pub plot_w: i64,
    pub plot_h: i64,
    pub bar_count: i64,
    pub bar_data: Vec<BarRenderData>,
    pub summary: TrendSummary,

    // ── comparison data ──
    pub mom_comparison: Option<TrendComparisonView>,
    pub yoy_comparison: Option<TrendComparisonView>,

    // ── available metrics ──
    pub available_metrics: Vec<MetricOption>,

    // ── entity breakdown ──
    pub entity_data: Vec<EntityTrendRow>,
}

/// Display-friendly metric option.
#[derive(Debug, Clone)]
pub struct MetricOption {
    pub value: String,
    pub label: String,
    pub selected: bool,
}

/// Display-friendly comparison view.
#[derive(Debug, Clone)]
pub struct TrendComparisonView {
    pub label: String,
    pub current_total: String,
    pub previous_total: String,
    pub change_pct: String,
    pub change_abs: String,
    pub direction: String, // "up", "down", "flat"
    pub direction_class: String, // "text-rams-green", "text-rams-red", "text-muted-foreground"
    pub icon: String, // "trending-up", "trending-down", "minus"
}

/// A single row in the entity breakdown table.
#[derive(Debug, Clone)]
pub struct EntityTrendRow {
    pub entity_type: String,
    pub entity_id: String,
    pub metric_name: String,
    pub total: i64,
}

/// Pre-computed bar position for the SVG chart.
/// Askama 0.12 cannot compute `as` casts, `.max()`, or `.min()`;
/// all SVG geometry is pre-computed in Rust.
#[derive(Debug, Clone)]
pub struct BarRenderData {
    pub x: i64,
    pub y: i64,
    pub bar_w: i64,
    pub bar_h: i64,
    pub date: String,
    pub value: i64,
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

fn format_number(n: i64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

fn compute_range_dates(range: &str) -> (NaiveDate, NaiveDate) {
    let today = Utc::now().date_naive();
    match range {
        "30d" => (today - chrono::Duration::days(30), today),
        "90d" => (today - chrono::Duration::days(90), today),
        "1y" => (today - chrono::Duration::days(365), today),
        "5y" => (today - chrono::Duration::days(365 * 5), today),
        _ => (today - chrono::Duration::days(90), today),
    }
}

fn available_metrics(selected: &str) -> Vec<MetricOption> {
    let metrics = vec![
        ("warnings", "Warnings"),
        ("insights", "Insights"),
        ("observations", "Observations"),
        ("companies_tracked", "Companies"),
        ("persons_tracked", "Persons"),
        ("active_recipes", "Active Recipes"),
        ("new_warnings", "New Warnings"),
        ("unacknowledged_warnings", "Unacknowledged"),
    ];
    metrics
        .into_iter()
        .map(|(value, label)| MetricOption {
            value: value.to_string(),
            label: label.to_string(),
            selected: value == selected,
        })
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// Handler: GET /trends
// ─────────────────────────────────────────────────────────────────────────────

/// Render the historical trends page.
pub async fn trends_page(
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
    Query(params): Query<TrendsPageParams>,
) -> impl IntoResponse {
    let ctx = PageContext::from_session(&session, "/trends", 0);

    let selected_metric = params.metric.unwrap_or_else(|| "warnings".to_string());
    let selected_bucket = params.bucket.unwrap_or_else(|| "monthly".to_string());
    let selected_range = params.range.unwrap_or_else(|| "90d".to_string());

    let (from_date, to_date) = if selected_range == "custom" {
        let from = params
            .from
            .as_deref()
            .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
            .unwrap_or_else(|| Utc::now().date_naive() - chrono::Duration::days(90));
        let to = params
            .to
            .as_deref()
            .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
            .unwrap_or_else(|| Utc::now().date_naive());
        (from, to)
    } else {
        compute_range_dates(&selected_range)
    };

    // Validate bucket type
    let bucket_type = if BucketType::from_str(&selected_bucket).is_some() {
        selected_bucket.clone()
    } else {
        "monthly".to_string()
    };

    // Query trend data
    let trend_query = TrendQuery {
        bucket_type: bucket_type.clone(),
        metric_name: selected_metric.clone(),
        from_date: Some(from_date),
        to_date: Some(to_date),
        entity_type: None,
        entity_id: None,
        limit: Some(500),
    };

    let data_points = store
        .query_trends(&trend_query)
        .await
        .unwrap_or_else(|e| {
            tracing::error!("Failed to query trends: {e}");
            vec![]
        });

    // Pre-compute bar chart geometry (Askama 0.12 cannot do `as` casts, .max(), .min())
    let bar_count = data_points.len() as i64;
    let chart_w = (bar_count * 60).max(400).min(1200);
    let chart_h: i64 = 240;
    let pad: i64 = 40;
    let plot_w = chart_w - pad * 2;
    let plot_h = chart_h - pad * 2;
    let max_val = data_points
        .iter()
        .map(|p| p.value)
        .max()
        .unwrap_or(1)
        .max(1);
    let max_val_half = max_val / 2;

    let bar_data: Vec<BarRenderData> = data_points
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let bar_w = (plot_w / bar_count).max(4) - 2;
            let bar_h = if max_val > 0 {
                (p.value as f64 / max_val as f64 * plot_h as f64) as i64
            } else {
                0
            };
            let x = pad + (i as i64 * plot_w / bar_count);
            let y = pad + plot_h - bar_h;
            BarRenderData {
                x,
                y,
                bar_w,
                bar_h,
                date: p.date.to_string(),
                value: p.value,
            }
        })
        .collect();

    let summary = store
        .get_trend_summary(&trend_query)
        .await
        .unwrap_or_else(|e| {
            tracing::error!("Failed to get trend summary: {e}");
            TrendSummary {
                total: 0,
                average: 0.0,
                min: 0,
                max: 0,
                data_points: 0,
            }
        });

    // Compute MoM comparison (current month vs previous month)
    let today = to_date;
    let current_month_start = today.with_day(1).unwrap();
    let previous_month_end = current_month_start - chrono::Duration::days(1);
    let previous_month_start = previous_month_end.with_day(1).unwrap();
    let mom_days = (today - current_month_start).num_days().max(1);

    let mom_query = TrendComparisonQuery {
        metric_name: selected_metric.clone(),
        current_period_start: current_month_start,
        previous_period_start: previous_month_start,
        period_duration_days: mom_days,
        entity_type: None,
        entity_id: None,
    };

    let mom_comparison = store
        .get_trend_comparison(&mom_query)
        .await
        .ok()
        .map(|c| TrendComparisonView {
            label: "Month-over-Month".to_string(),
            current_total: format_number(c.current_total),
            previous_total: format_number(c.previous_total),
            change_pct: format!("{:.1}%", c.percent_change),
            change_abs: format_number(c.absolute_change.abs()),
            direction: c.direction.clone(),
            direction_class: match c.direction.as_str() {
                "up" => "text-rams-green".to_string(),
                "down" => "text-rams-red".to_string(),
                _ => "text-muted-foreground".to_string(),
            },
            icon: match c.direction.as_str() {
                "up" => "trending-up".to_string(),
                "down" => "trending-down".to_string(),
                _ => "minus".to_string(),
            },
        });

    // Compute YoY comparison (current year vs previous year)
    let current_year_start = today.with_month(1).and_then(|d| d.with_day(1)).unwrap();
    let previous_year_start = current_year_start.with_year(today.year() - 1).unwrap();
    let yoy_days = (today - current_year_start).num_days().max(1);

    let yoy_query = TrendComparisonQuery {
        metric_name: selected_metric.clone(),
        current_period_start: current_year_start,
        previous_period_start: previous_year_start,
        period_duration_days: yoy_days,
        entity_type: None,
        entity_id: None,
    };

    let yoy_comparison = store
        .get_trend_comparison(&yoy_query)
        .await
        .ok()
        .map(|c| TrendComparisonView {
            label: "Year-over-Year".to_string(),
            current_total: format_number(c.current_total),
            previous_total: format_number(c.previous_total),
            change_pct: format!("{:.1}%", c.percent_change),
            change_abs: format_number(c.absolute_change.abs()),
            direction: c.direction.clone(),
            direction_class: match c.direction.as_str() {
                "up" => "text-rams-green".to_string(),
                "down" => "text-rams-red".to_string(),
                _ => "text-muted-foreground".to_string(),
            },
            icon: match c.direction.as_str() {
                "up" => "trending-up".to_string(),
                "down" => "trending-down".to_string(),
                _ => "minus".to_string(),
            },
        });

    let metrics = available_metrics(&selected_metric);

    // Entity breakdown — top entities by metric
    let entity_data = fetch_entity_breakdown(&store, &selected_metric, &bucket_type, from_date, to_date).await;

    let page = TrendsPage {
        current_path: "/trends".to_string(),
        max_val,
        max_val_half,
        chart_w,
        chart_h,
        pad,
        plot_w,
        plot_h,
        bar_count,
        bar_data,
        username: ctx.username,
        warning_count: ctx.warning_count,
        theme: ctx.theme,
        selected_metric,
        selected_bucket: bucket_type,
        selected_range,
        from_date: from_date.to_string(),
        to_date: to_date.to_string(),
        data_points,
        summary,
        mom_comparison,
        yoy_comparison,
        available_metrics: metrics,
        entity_data,
    };

    super::render_template(&page)
}

/// Fetch entity-level breakdown data for the trends page.
async fn fetch_entity_breakdown(
    store: &PgStore,
    metric: &str,
    bucket_type: &str,
    from_date: NaiveDate,
    to_date: NaiveDate,
) -> Vec<EntityTrendRow> {
    // Only fetch entity breakdown for observation/warning metrics
    if metric != "observations" && metric != "warnings" {
        return vec![];
    }

    let entity_type = if metric == "observations" {
        "company"
    } else {
        "company"
    };

    let query = TrendQuery {
        bucket_type: bucket_type.to_string(),
        metric_name: metric.to_string(),
        from_date: Some(from_date),
        to_date: Some(to_date),
        entity_type: Some(entity_type.to_string()),
        entity_id: None,
        limit: Some(20),
    };

    // For entity breakdown, we need to query all entities.
    // We use the entity-level rollup data to get top entities by total.
    // Since we can't easily do GROUP BY across multiple rows in a single query_trends call,
    // we'll use a direct SQL query for the entity breakdown.
    let rows = match sqlx::query_as::<_, (String, String, i64)>(
        r#"SELECT
               COALESCE(entity_type, 'unknown'),
               COALESCE(entity_id, 'unknown'),
               SUM(metric_value)::BIGINT AS total
           FROM trend_rollups
           WHERE bucket_type = $1
             AND metric_name = $2
             AND entity_type IS NOT NULL
             AND entity_id IS NOT NULL
             AND bucket_date >= $3
             AND bucket_date <= $4
           GROUP BY entity_type, entity_id
           ORDER BY total DESC
           LIMIT 10"#,
    )
    .bind(&query.bucket_type)
    .bind(&query.metric_name)
    .bind(query.from_date)
    .bind(query.to_date)
    .fetch_all(&store.pool)
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("Failed to fetch entity breakdown: {e}");
            return vec![];
        }
    };

    rows.into_iter()
        .map(|(entity_type, entity_id, total)| EntityTrendRow {
            entity_type,
            entity_id,
            metric_name: metric.to_string(),
            total,
        })
        .collect()
}
