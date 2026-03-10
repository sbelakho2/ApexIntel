//! Dashboard handler — GET /
//!
//! Covers: dashboard page with KPI cards, recent warnings, top insights,
//! and activity timeline.

use chrono::{DateTime, Duration, Utc};
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use askama::Template;
use axum::{response::IntoResponse, Extension};

use super::PageContext;
use crate::middleware::session::WebSession;
use apex_store::postgres::{InsightListFilters, PgStore, WarningListFilters};

// ─── Template structs ───────────────────────────────────────────────────────

/// A single stat card on the dashboard.
#[derive(Clone, Debug)]
pub struct StatCard {
    pub label: String,
    pub value: String,
    pub icon: String,
    pub accent: String,
    pub delta: Option<String>,
    pub direction: String, // "up" | "down" | "flat"
}

/// A recent warning shown in the dashboard feed.
#[derive(Clone, Debug)]
pub struct RecentWarning {
    pub id: String,
    pub title: String,
    pub severity: String,
    pub company_name: String,
    pub created_at: String,
}

/// A top insight shown in the dashboard feed.
#[derive(Clone, Debug)]
pub struct TopInsight {
    pub id: String,
    pub title: String,
    pub category: String,
    pub confidence: f64,
    pub confidence_pct: i64,
    pub created_at: String,
}

/// An activity event for the timeline widget.
#[derive(Clone, Debug)]
pub struct ActivityEvent {
    pub kind: String,
    pub description: String,
    pub timestamp: String,
    pub href: String,
}

/// Severity breakdown for the mini-chart.
#[derive(Clone, Debug)]
pub struct SeverityCount {
    pub label: String,
    pub count: i64,
    pub color: String,
}

#[derive(Clone, Debug)]
pub struct WarningTrendDay {
    pub date_label: String,
    pub critical: i64,
    pub high: i64,
    pub medium: i64,
    pub low: i64,
}

#[derive(Clone, Debug)]
pub struct RegionSlice {
    pub name: String,
    pub count: i64,
    pub color: String,
}

#[derive(Clone, Debug)]
pub struct CrawlActivityHour {
    pub hour_label: String,
    pub success: i64,
    pub errors: i64,
}

/// Precomputed donut chart segment for SVG rendering.
#[derive(Clone, Debug)]
pub struct DonutSegment {
    pub label: String,
    pub count: i64,
    pub color: String,
    /// SVG stroke-dasharray value, e.g. "62.8 188.5"
    pub dash_array: String,
    /// SVG stroke-dashoffset value, e.g. "-31.4"
    pub dash_offset: String,
}

/// Compute donut chart segments with SVG stroke-dasharray/offset values.
pub fn compute_donut_segments(counts: &[SeverityCount], radius: f64) -> Vec<DonutSegment> {
    let circumference = 2.0 * std::f64::consts::PI * radius;
    let total: f64 = counts.iter().map(|s| s.count as f64).sum();
    if total == 0.0 {
        return vec![];
    }
    let mut offset = 0.0f64;
    counts
        .iter()
        .map(|s| {
            let pct = s.count as f64 / total;
            let dash = pct * circumference;
            let gap = circumference - dash;
            let seg = DonutSegment {
                label: s.label.clone(),
                count: s.count,
                color: s.color.clone(),
                dash_array: format!("{:.1} {:.1}", dash, gap),
                dash_offset: format!("{:.1}", -offset),
            };
            offset += dash;
            seg
        })
        .collect()
}

#[derive(Template)]
#[template(path = "pages/dashboard.html")]
pub struct DashboardPage {
    // ── base layout fields ──
    pub current_path: String,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,

    // ── dashboard-specific ──
    pub stats: Vec<StatCard>,
    pub recent_warnings: Vec<RecentWarning>,
    pub top_insights: Vec<TopInsight>,
    pub activity: Vec<ActivityEvent>,
    pub severity_breakdown: Vec<SeverityCount>,
    pub donut_segments: Vec<DonutSegment>,
    pub warning_trend: Vec<WarningTrendDay>,
    pub crawl_activity: Vec<CrawlActivityHour>,
    pub region_slices: Vec<RegionSlice>,
    pub companies_tracked: i64,
    pub persons_tracked: i64,
    pub recipes_active: i64,
    pub data_freshness: String,
}

// ─── Handlers ───────────────────────────────────────────────────────────────

/// GET / — render the main dashboard.
pub async fn dashboard(
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
) -> impl IntoResponse {
    // Fetch dashboard stats from DB (graceful degradation on error)
    let stats_data = store.get_dashboard_stats().await.unwrap_or_else(|e| {
        tracing::error!("Failed to fetch dashboard stats: {e}");
        Default::default()
    });

    let unack_warnings = stats_data.unacknowledged_warnings as i64;
    let ctx = PageContext::from_session(&session, "/", unack_warnings);

    // Fetch recent warnings (last 5)
    let warning_filters = WarningListFilters::default();
    let recent_warning_rows = store
        .list_warnings(&warning_filters, None, true, 5, 0)
        .await
        .unwrap_or_else(|e| {
            tracing::error!("Failed to fetch recent warnings: {e}");
            vec![]
        });
    let recent_warnings: Vec<RecentWarning> = recent_warning_rows
        .iter()
        .map(|w| RecentWarning {
            id: w.id.to_string(),
            title: w.title.clone(),
            severity: w.severity.clone(),
            company_name: String::new(),
            created_at: w.ts_utc.format("%Y-%m-%d %H:%M").to_string(),
        })
        .collect();

    // Fetch top insights (last 5)
    let insight_filters = InsightListFilters::default();
    let top_insight_rows = store
        .list_insights(&insight_filters, 5, 0)
        .await
        .unwrap_or_else(|e| {
            tracing::error!("Failed to fetch top insights: {e}");
            vec![]
        });
    let top_insights: Vec<TopInsight> = top_insight_rows
        .iter()
        .map(|i| TopInsight {
            id: i.id.to_string(),
            title: i.title.clone(),
            category: i.insight_type.clone().unwrap_or_default(),
            confidence: i.confidence.unwrap_or(0.0),
            confidence_pct: ((i.confidence.unwrap_or(0.0) * 100.0).round() as i64),
            created_at: i
                .created_at
                .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_default(),
        })
        .collect();

    // Build severity breakdown from dashboard stats
    let severity_breakdown = {
        let mut critical = 0i64;
        let mut high = 0i64;
        let mut medium = 0i64;
        let mut low = 0i64;
        for sc in &stats_data.threat_distribution {
            match sc.severity.as_str() {
                "critical" => critical = sc.count,
                "high" => high = sc.count,
                "medium" => medium = sc.count,
                "low" => low = sc.count,
                _ => {}
            }
        }
        vec![
            SeverityCount {
                label: "Critical".into(),
                count: critical,
                color: "#D62D2D".into(),
            },
            SeverityCount {
                label: "High".into(),
                count: high,
                color: "#F97316".into(),
            },
            SeverityCount {
                label: "Medium".into(),
                count: medium,
                color: "#FFBE00".into(),
            },
            SeverityCount {
                label: "Low".into(),
                count: low,
                color: "#4A90E2".into(),
            },
        ]
    };

    let new_warnings_24h = stats_data.new_warnings_24h;
    let new_insights_24h = stats_data.new_insights_24h;

    let warning_rows = store
        .list_warnings(&WarningListFilters::default(), None, true, 300, 0)
        .await
        .unwrap_or_default();
    let mut daily: BTreeMap<String, WarningTrendDay> = BTreeMap::new();
    for warning in warning_rows {
        let day = warning.ts_utc.format("%m-%d").to_string();
        let entry = daily.entry(day.clone()).or_insert_with(|| WarningTrendDay {
            date_label: day,
            critical: 0,
            high: 0,
            medium: 0,
            low: 0,
        });
        match warning.severity.to_lowercase().as_str() {
            "critical" => entry.critical += 1,
            "high" => entry.high += 1,
            "medium" => entry.medium += 1,
            _ => entry.low += 1,
        }
    }
    let mut warning_trend: Vec<WarningTrendDay> = daily.into_values().collect();
    if warning_trend.len() > 24 {
        warning_trend = warning_trend.split_off(warning_trend.len() - 24);
    }

    let mut used_region_colors: HashSet<String> = HashSet::new();
    let region_slices: Vec<RegionSlice> = stats_data
        .top_regions
        .iter()
        .enumerate()
        .map(|(index, region)| {
            let mut color = region_color(&region.region, index).to_string();
            if used_region_colors.contains(&color) {
                let mut palette_index = index;
                loop {
                    let candidate = palette_color(palette_index).to_string();
                    if !used_region_colors.contains(&candidate) {
                        color = candidate;
                        break;
                    }
                    palette_index += 1;
                }
            }
            used_region_colors.insert(color.clone());
            RegionSlice {
                name: region.region.clone(),
                count: region.count,
                color,
            }
        })
        .collect();

    #[derive(sqlx::FromRow)]
    struct CrawlActivityRow {
        hour_label: String,
        success: i64,
        errors: i64,
    }

    // Use page_fingerprints (pages crawled) + observations (pipeline events) as crawl proxy
    let crawl_activity: Vec<CrawlActivityHour> = sqlx::query_as::<_, CrawlActivityRow>(
        r#"SELECT
               to_char(date_trunc('hour', h.hour), 'HH24:00') AS hour_label,
               COALESCE(pf.cnt, 0)::bigint AS success,
               COALESCE(obs.cnt, 0)::bigint AS errors
           FROM (
               SELECT generate_series(
                   date_trunc('hour', $1::timestamptz),
                   date_trunc('hour', now()),
                   '1 hour'::interval
               ) AS hour
           ) h
           LEFT JOIN (
               SELECT date_trunc('hour', ts) AS hr, COUNT(*) AS cnt
               FROM page_fingerprints
               WHERE ts >= $1
               GROUP BY hr
           ) pf ON pf.hr = h.hour
           LEFT JOIN (
               SELECT date_trunc('hour', ts_utc) AS hr, COUNT(*) AS cnt
               FROM observations
               WHERE ts_utc >= $1
               GROUP BY hr
           ) obs ON obs.hr = h.hour
           ORDER BY h.hour ASC
           LIMIT 24"#,
    )
    .bind(Utc::now() - Duration::hours(24))
    .fetch_all(&store.pool)
    .await
    .unwrap_or_default()
    .into_iter()
    .map(|row| CrawlActivityHour {
        hour_label: row.hour_label,
        success: row.success,
        errors: row.errors,
    })
    .collect();

    // Compute donut chart segments (size=120, stroke=14 → radius=53)
    let donut_segments = compute_donut_segments(&severity_breakdown, 53.0);

    // Build real timestamped activity feed from warnings + insights
    #[derive(sqlx::FromRow)]
    struct ActivityRow {
        event_id: String,
        event_kind: String,
        description: String,
        ts: DateTime<Utc>,
    }

    let activity_rows: Vec<ActivityRow> = sqlx::query_as::<_, ActivityRow>(
         r#"SELECT id::text AS event_id,
                'Warning' AS event_kind,
                  COALESCE(title, 'Untitled') || ' [' || UPPER(severity) || ']' AS description,
                  ts_utc AS ts
           FROM warnings
           UNION ALL
            SELECT id::text AS event_id,
                'Insight' AS event_kind,
                  COALESCE(title, 'Untitled') || ' (' || COALESCE(insight_type, 'general') || ')' AS description,
                  COALESCE(created_at, now()) AS ts
           FROM insights
           ORDER BY ts DESC
           LIMIT 12"#,
    )
    .fetch_all(&store.pool)
    .await
    .unwrap_or_default();

    let activity: Vec<ActivityEvent> = activity_rows
        .into_iter()
        .map(|r| ActivityEvent {
            kind: r.event_kind.clone(),
            description: r.description,
            timestamp: r.ts.format("%m-%d %H:%M").to_string(),
            href: if r.event_kind == "Warning" {
                format!("/warnings/{}", r.event_id)
            } else {
                format!("/insights/{}", r.event_id)
            },
        })
        .collect();

    let page = DashboardPage {
        current_path: ctx.current_path,
        username: ctx.username,
        warning_count: ctx.warning_count,
        theme: ctx.theme,

        stats: vec![
            StatCard {
                label: "Active Warnings".into(),
                value: stats_data.unacknowledged_warnings.to_string(),
                icon: "alert-triangle".into(),
                accent: "var(--rams-orange)".into(),
                delta: if new_warnings_24h > 0 {
                    Some(format!("+{}", new_warnings_24h))
                } else {
                    None
                },
                direction: if new_warnings_24h > 0 {
                    "up".into()
                } else {
                    "flat".into()
                },
            },
            StatCard {
                label: "Insights (7d)".into(),
                value: stats_data.total_insights.to_string(),
                icon: "eye".into(),
                accent: "var(--rams-blue)".into(),
                delta: if new_insights_24h > 0 {
                    Some(format!("+{}", new_insights_24h))
                } else {
                    None
                },
                direction: if new_insights_24h > 0 {
                    "up".into()
                } else {
                    "flat".into()
                },
            },
            StatCard {
                label: "Companies".into(),
                value: stats_data.total_companies.to_string(),
                icon: "boxes".into(),
                accent: "var(--rams-navy)".into(),
                delta: None,
                direction: "flat".into(),
            },
            StatCard {
                label: "Persons of Interest".into(),
                value: stats_data.total_persons.to_string(),
                icon: "users".into(),
                accent: "var(--rams-gold)".into(),
                delta: None,
                direction: "flat".into(),
            },
        ],
        recent_warnings,
        top_insights,
        activity,
        severity_breakdown,
        donut_segments,
        warning_trend,
        crawl_activity,
        region_slices,
        companies_tracked: stats_data.total_companies as i64,
        persons_tracked: stats_data.total_persons as i64,
        recipes_active: stats_data.active_recipes as i64,
        data_freshness: "Live".into(),
    };

    page.into_response()
}

fn region_color(region: &str, index: usize) -> &'static str {
    let normalized = region.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "tunisia" | "tn" => "#FFBE00",
        "morocco" | "ma" => "#D62D2D",
        "israel" | "il" => "#4A90E2",
        "eu" | "europe" | "european union" => "#2D8C3C",
        "china" | "cn" => "#8B5CF6",
        "global" => "#14B8A6",
        "us" | "usa" | "united states" => "#F97316",
        "uk" | "united kingdom" | "gb" => "#06B6D4",
        "france" | "fr" => "#E94F87",
        "germany" | "de" => "#A3E635",
        _ => {
            const PALETTE: [&str; 8] = [
                "#4A90E2", "#2D8C3C", "#FFBE00", "#D62D2D", "#8B5CF6", "#14B8A6", "#F97316",
                "#06B6D4",
            ];
            PALETTE[index % PALETTE.len()]
        }
    }
}

fn palette_color(index: usize) -> &'static str {
    const PALETTE: [&str; 12] = [
        "#4A90E2", "#2D8C3C", "#FFBE00", "#D62D2D", "#8B5CF6", "#14B8A6", "#F97316", "#06B6D4",
        "#E94F87", "#A3E635", "#6366F1", "#10B981",
    ];
    PALETTE[index % PALETTE.len()]
}
