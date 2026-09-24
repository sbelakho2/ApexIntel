//! Dashboard handler — GET /
//!
//! Covers: dashboard page with KPI cards, recent warnings, top insights,
//! and activity timeline.

use chrono::{DateTime, Duration, Utc};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use askama::Template;
use axum::{response::IntoResponse, Extension};
use uuid::Uuid;

use super::PageContext;
use crate::middleware::session::WebSession;
use crate::system_status::StatusStrip;
use apex_core::data_state::{DataState, DegradedNotice};
use apex_store::postgres::{InsightListFilters, PgStore, WarningListFilters};

fn dashboard_reference_now() -> DateTime<Utc> {
    std::env::var("APEX_FIXED_NOW")
        .ok()
        .and_then(|value| DateTime::parse_from_rfc3339(&value).ok())
        .map(|value| value.with_timezone(&Utc))
        .unwrap_or_else(Utc::now)
}

// ─── Template structs ───────────────────────────────────────────────────────

/// A single stat card on the dashboard.
#[derive(Clone, Debug)]
pub struct StatCard {
    pub label: String,
    pub value: String,
    pub icon: String,
    pub accent_class: String,
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
///
/// Backed by the real `activity_feed` table (written by the worker's
/// `ActivityLogger` on every crawl, POI discovery, insight generation, threat
/// detection, and job lifecycle event). Previously this widget UNIONed only
/// `warnings` + `insights`, hiding the bulk of real system activity.
#[derive(Clone, Debug)]
pub struct ActivityEvent {
    /// Human-readable action verb (e.g. "Insight", "POI discovered", "Crawl").
    pub kind: String,
    /// One-line description combining actor + entity + detail.
    pub description: String,
    /// Formatted timestamp (MM-DD HH:MM).
    pub timestamp: String,
    /// Deep-link href to the relevant entity page, if any.
    pub href: String,
    /// Single-glyph icon code for the badge (I/W/P/C/T/J).
    pub icon: &'static str,
    /// Semantic accent class for the badge (insight/warning/poi/crawl/threat/job).
    pub accent: &'static str,
    /// Secondary actor label (e.g. "system", "John Smith").
    pub actor: String,
}

/// Map an `activity_feed.action_type` to a (kind label, icon glyph, accent class).
fn classify_activity_action(action_type: &str) -> (&'static str, &'static str, &'static str) {
    match action_type {
        "insight_generated" => ("Insight", "I", "insight"),
        "poi_discovered" => ("POI discovered", "P", "poi"),
        "crawl_completed" => ("Crawl", "C", "crawl"),
        "company_detected" => ("Company", "C", "crawl"),
        "threat_detected" => ("Threat", "T", "threat"),
        "psych_profile_updated" => ("Profile", "P", "poi"),
        "battlecard_generated" => ("Battlecard", "B", "insight"),
        "memo_generated" => ("Memo", "M", "insight"),
        "recipe_promoted" => ("Recipe", "R", "crawl"),
        "job_completed" => ("Job", "J", "job"),
        "job_failed" => ("Job failed", "J", "threat"),
        "job_skipped" => ("Job skipped", "J", "muted"),
        "create" => ("Created", "+", "insight"),
        "update" => ("Updated", "~", "crawl"),
        "delete" => ("Deleted", "−", "threat"),
        "share" => ("Shared", "↗", "job"),
        "comment" => ("Comment", "…", "muted"),
        "resolve" => ("Resolved", "✓", "poi"),
        "escalate" => ("Escalated", "↑", "threat"),
        _ => ("Activity", "•", "muted"),
    }
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
    pub total: i64,
    pub bar_h: i64,
    pub critical_h: i64,
    pub high_h: i64,
    pub medium_h: i64,
    pub low_h: i64,
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
pub struct CrawlActivityHour {
    pub hour_label: String,
    pub success: i64,
    pub errors: i64,
    pub success_h: i64,
    pub errors_h: i64,
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

// ─── Decisions-first dashboard models (audit P0 #22) ────────────────────────

/// A "what changed since last look" row: a count plus the link that shows it.
#[derive(Clone, Debug)]
pub struct ChangeItem {
    pub label: String,
    pub count: i64,
    pub detail: String,
    pub href: String,
}

/// An item in the "what requires action" priority queue.
#[derive(Clone, Debug)]
pub struct PriorityItem {
    pub id: String,
    /// "warning" | "insight"
    pub kind: String,
    pub title: String,
    pub severity: String,
    pub confidence_pct: Option<i64>,
    pub entity_name: String,
    pub entity_href: String,
    pub reason: String,
    pub age_label: String,
    pub assigned_analyst: String,
    /// Warnings have a one-click acknowledge endpoint; insights link to review.
    pub can_acknowledge: bool,
    pub acknowledge_url: String,
    pub investigate_url: String,
}

/// A newly actionable opportunity: a real account with why-now and next move.
#[derive(Clone, Debug)]
pub struct OpportunityCard {
    pub insight_id: String,
    pub company_id: String,
    pub company_name: String,
    pub region: String,
    pub domain: String,
    pub why_now: String,
    pub evidence_count: i64,
    pub confidence_pct: i64,
    pub next_move: String,
    pub buyer_name: String,
    pub buyer_role: String,
    pub href: String,
}

/// An ingestion coverage / freshness row for "what is stale or broken".
#[derive(Clone, Debug)]
pub struct HealthItem {
    pub label: String,
    pub detail: String,
    /// "ok" | "warn" | "bad"
    pub state: String,
    pub state_label: String,
    pub href: String,
}

fn age_label(dt: DateTime<Utc>) -> String {
    let delta = Utc::now().signed_duration_since(dt);
    let minutes = delta.num_minutes();
    if minutes < 1 {
        "just now".to_string()
    } else if minutes < 60 {
        format!("{minutes}m ago")
    } else if delta.num_hours() < 24 {
        format!("{}h ago", delta.num_hours())
    } else if delta.num_days() < 14 {
        format!("{}d ago", delta.num_days())
    } else {
        format!("{}w ago", delta.num_weeks())
    }
}

fn humanize_token(raw: &str) -> String {
    let cleaned = raw.trim().replace(['_', '-'], " ");
    let mut chars = cleaned.chars();
    match chars.next() {
        Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str()),
        None => "Signal".to_string(),
    }
}

/// Extract the recommended next move from a stored insight summary.
///
/// The worker stores `narrative + "\n\n" + recommendation(+contact block)`.
/// With multiple sections we take the section after the narrative; otherwise
/// we return the summary itself, truncated for card rendering.
fn recommended_next_move(summary: &str) -> String {
    let sections: Vec<&str> = summary
        .split("\n\n")
        .map(str::trim)
        .filter(|section| !section.is_empty())
        .collect();
    let candidate = if sections.len() > 1 {
        sections[sections.len() - 1]
    } else {
        sections.first().copied().unwrap_or("")
    };
    let candidate = candidate.trim();
    if candidate.is_empty() {
        return "Review this insight and assign an owner.".to_string();
    }
    let mut truncated: String = candidate.chars().take(320).collect();
    if candidate.chars().count() > 320 {
        truncated.push('…');
    }
    truncated
}

fn insight_queue_severity(confidence: f64) -> &'static str {
    if confidence >= 0.8 {
        "critical"
    } else if confidence >= 0.7 {
        "high"
    } else if confidence >= 0.4 {
        "medium"
    } else {
        "low"
    }
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
    pub can_admin: bool,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,
    pub status_strip: StatusStrip,

    // ── dashboard-specific ──
    /// Rendered when any repository-backed load failed, so a failed query is
    /// never presented as "no results".
    pub degraded_notice: Option<String>,

    // Decisions-first sections (audit P0 #22), in render order.
    pub changes_since: Vec<ChangeItem>,
    pub priority_queue: Vec<PriorityItem>,
    pub opportunities: Vec<OpportunityCard>,
    pub health_items: Vec<HealthItem>,

    // Vanity/funnel counts (kept below the actionable sections).
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
    pub portfolio_health: String,
    pub portfolio_health_class: String,
}

// ─── Handlers ───────────────────────────────────────────────────────────────

/// GET / — render the main dashboard.
pub async fn dashboard(
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
) -> impl IntoResponse {
    // Fetch dashboard stats from DB. A failed query renders an explicit
    // degraded marker instead of a silently zeroed dashboard.
    let mut degraded_notice: Option<String> = None;
    let stats_state = DataState::from_result(
        store.get_dashboard_stats().await,
        "failed to fetch dashboard stats",
        |_| false,
    );
    DegradedNotice::capture(&stats_state, &mut degraded_notice);
    let stats_data = stats_state.into_loaded_or_default();

    let unack_warnings = stats_data.unacknowledged_warnings as i64;
    let ctx = PageContext::from_session(&session, "/", unack_warnings);
    let status_strip = ctx.status_strip.clone();
    let data_freshness_text = status_strip.data_freshness.clone();

    // Fetch recent warnings (last 5)
    let warning_filters = WarningListFilters::default();
    let recent_warnings_state = DataState::from_result(
        store
            .list_warnings(&warning_filters, None, true, 5, 0)
            .await,
        "failed to fetch recent warnings",
        |rows| rows.is_empty(),
    );
    DegradedNotice::capture(&recent_warnings_state, &mut degraded_notice);
    let recent_warning_rows = recent_warnings_state.into_items();
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
    let insight_filters = InsightListFilters {
        exclude_internal: true,
        ..Default::default()
    };
    let top_insights_state = DataState::from_result(
        store.list_insights(&insight_filters, 5, 0).await,
        "failed to fetch top insights",
        |rows| rows.is_empty(),
    );
    DegradedNotice::capture(&top_insights_state, &mut degraded_notice);
    let top_insight_rows = top_insights_state.into_items();
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

    let warning_trend_state = DataState::from_result(
        store
            .list_warnings(&WarningListFilters::default(), None, true, 300, 0)
            .await,
        "failed to fetch warning trend",
        |rows| rows.is_empty(),
    );
    DegradedNotice::capture(&warning_trend_state, &mut degraded_notice);
    let warning_rows = warning_trend_state.into_items();
    let mut daily: BTreeMap<String, WarningTrendDay> = BTreeMap::new();
    for warning in warning_rows {
        let day = warning.ts_utc.format("%m-%d").to_string();
        let entry = daily.entry(day.clone()).or_insert_with(|| WarningTrendDay {
            date_label: day,
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
    let max_total = warning_trend
        .iter()
        .map(|d| d.critical + d.high + d.medium + d.low)
        .max()
        .unwrap_or(1)
        .max(1);
    for day in &mut warning_trend {
        day.total = day.critical + day.high + day.medium + day.low;
        day.bar_h = day.total * 100 / max_total;
        if day.total > 0 {
            day.critical_h = day.critical * 100 / day.total;
            day.high_h = day.high * 100 / day.total;
            day.medium_h = day.medium * 100 / day.total;
            day.low_h = day.low * 100 / day.total;
        }
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
                dash_array: String::new(),
                dash_offset: String::new(),
            }
        })
        .collect();
    let region_total = region_slices.iter().map(|slice| slice.count).sum::<i64>() as f64;
    let region_radius = 53.0f64;
    let region_circumference = 2.0 * std::f64::consts::PI * region_radius;
    let mut cumulative_region = 0.0f64;
    let region_slices: Vec<RegionSlice> = region_slices
        .into_iter()
        .map(|mut slice| {
            if region_total > 0.0 {
                let fraction = slice.count as f64 / region_total;
                let dash = fraction * region_circumference;
                let gap = region_circumference - dash;
                slice.dash_array = format!("{:.2} {:.2}", dash, gap);
                // Negative cumulative offset positions each segment right
                // after the previous one; the SVG already has -rotate-90 to
                // start at 12-o'clock, so no extra 0.25-turn shift is needed.
                slice.dash_offset = format!("{:.2}", -(cumulative_region * region_circumference));
                cumulative_region += fraction;
            }
            slice
        })
        .collect();

    let reference_now = dashboard_reference_now();
    let crawl_window_start = reference_now - Duration::hours(24);
    let change_window_start = reference_now - Duration::hours(24);
    let opportunity_window_start = reference_now - Duration::days(30);

    // ─── 1. What changed since last look (audit P0 #22) ───────────────────
    // Counts with links. Each load goes through DataState so a failed query
    // renders as degraded with a notice, never as a truthful-looking zero.
    #[derive(sqlx::FromRow)]
    struct ChangeCountsRow {
        new_signals: i64,
        companies_changed: i64,
        new_contacts: i64,
        warnings_requiring_action: i64,
    }

    let change_counts_state = DataState::from_result(
        sqlx::query_as::<_, ChangeCountsRow>(
            r#"SELECT
                 (SELECT COUNT(*)::bigint FROM insights
                   WHERE created_at >= $1 AND COALESCE(confidence, 0) >= 0.7
                     AND COALESCE(insight_type, '') NOT LIKE 'llm_%') AS new_signals,
                 (SELECT COUNT(DISTINCT entity_id)::bigint FROM observations
                   WHERE ts_utc >= $1 AND entity_id IS NOT NULL
                     AND entity_type = 'company') AS companies_changed,
                 (SELECT COUNT(*)::bigint FROM persons
                   WHERE created_at >= $1) AS new_contacts,
                 (SELECT COUNT(*)::bigint FROM warnings
                   WHERE acknowledged = false AND deleted_at IS NULL
                     AND lower(severity) IN ('critical', 'high')) AS warnings_requiring_action"#,
        )
        .bind(change_window_start)
        .fetch_one(&store.pool)
        .await,
        "failed to fetch change counts",
        |_| false,
    );
    DegradedNotice::capture(&change_counts_state, &mut degraded_notice);
    let change_counts = change_counts_state.into_loaded_or(ChangeCountsRow {
        new_signals: 0,
        companies_changed: 0,
        new_contacts: 0,
        warnings_requiring_action: 0,
    });

    let changes_since = vec![
        ChangeItem {
            label: "New high-confidence signals".into(),
            count: change_counts.new_signals,
            detail: "Insights at 70%+ confidence created in the last 24h".into(),
            href: "/insights?min_confidence=0.7".into(),
        },
        ChangeItem {
            label: "Companies changed materially".into(),
            count: change_counts.companies_changed,
            detail: "Companies with new observations in the last 24h".into(),
            href: "/companies".into(),
        },
        ChangeItem {
            label: "New buyer contacts".into(),
            count: change_counts.new_contacts,
            detail: "Persons added to the contact graph in the last 24h".into(),
            href: "/persons".into(),
        },
        ChangeItem {
            label: "Warnings requiring action".into(),
            count: change_counts.warnings_requiring_action,
            detail: "Unacknowledged critical/high warnings right now".into(),
            href: "/warnings?status=active".into(),
        },
    ];

    // ─── 2. What requires action: priority queue ──────────────────────────
    #[derive(sqlx::FromRow)]
    struct WarningQueueRow {
        id: Uuid,
        title: String,
        warning_type: Option<String>,
        severity: String,
        confidence: Option<f64>,
        entity_ids: Option<Vec<Uuid>>,
        ts_utc: DateTime<Utc>,
        acknowledged_by: Option<String>,
    }

    let warning_queue_state = DataState::from_result(
        sqlx::query_as::<_, WarningQueueRow>(
            r#"SELECT id, title, warning_type, severity, confidence,
                      entity_ids, ts_utc, acknowledged_by
               FROM warnings
               WHERE acknowledged = false AND deleted_at IS NULL
                 AND lower(severity) IN ('critical', 'high')
               ORDER BY CASE lower(severity) WHEN 'critical' THEN 0 ELSE 1 END,
                        ts_utc DESC
               LIMIT 8"#,
        )
        .fetch_all(&store.pool)
        .await,
        "failed to fetch priority warnings",
        |rows| rows.is_empty(),
    );
    DegradedNotice::capture(&warning_queue_state, &mut degraded_notice);
    let warning_queue_rows = warning_queue_state.into_items();

    #[derive(sqlx::FromRow)]
    struct InsightQueueRow {
        id: Uuid,
        title: String,
        insight_type: Option<String>,
        confidence: Option<f64>,
        entity_ids: Option<Vec<Uuid>>,
        created_at: Option<DateTime<Utc>>,
    }

    let insight_queue_state = DataState::from_result(
        sqlx::query_as::<_, InsightQueueRow>(
            r#"SELECT id, title, insight_type, confidence, entity_ids, created_at
               FROM insights
               WHERE created_at >= $1
                 AND COALESCE(confidence, 0) >= 0.7
                 AND COALESCE(insight_type, '') NOT LIKE 'llm_%'
               ORDER BY confidence DESC NULLS LAST, created_at DESC
               LIMIT 4"#,
        )
        .bind(change_window_start)
        .fetch_all(&store.pool)
        .await,
        "failed to fetch priority insights",
        |rows| rows.is_empty(),
    );
    DegradedNotice::capture(&insight_queue_state, &mut degraded_notice);
    let insight_queue_rows = insight_queue_state.into_items();

    // Resolve entity names for both queues in one batch query. A failed name
    // lookup must not blank the queue; it falls back to an explicit label.
    let mut queue_entity_ids: Vec<Uuid> = warning_queue_rows
        .iter()
        .flat_map(|w| w.entity_ids.clone().unwrap_or_default())
        .chain(
            insight_queue_rows
                .iter()
                .flat_map(|i| i.entity_ids.clone().unwrap_or_default()),
        )
        .collect();
    queue_entity_ids.sort();
    queue_entity_ids.dedup();
    let queue_company_names: HashMap<Uuid, String> = store
        .get_company_names_by_ids(&queue_entity_ids)
        .await
        .map(|rows| {
            rows.into_iter()
                .map(|(id, name, _, _)| (id, name))
                .collect()
        })
        .unwrap_or_default();

    let mut priority_queue: Vec<PriorityItem> =
        Vec::with_capacity(warning_queue_rows.len() + insight_queue_rows.len());
    for warning in &warning_queue_rows {
        let entity_id = warning
            .entity_ids
            .as_ref()
            .and_then(|ids| ids.first())
            .copied();
        priority_queue.push(PriorityItem {
            id: warning.id.to_string(),
            kind: "warning".into(),
            title: warning.title.clone(),
            severity: warning.severity.to_ascii_lowercase(),
            confidence_pct: warning.confidence.map(|c| (c * 100.0).round() as i64),
            entity_name: entity_id
                .and_then(|id| queue_company_names.get(&id).cloned())
                .unwrap_or_else(|| {
                    if entity_id.is_some() {
                        "Linked entity".into()
                    } else {
                        "No company linked".into()
                    }
                }),
            entity_href: entity_id
                .map(|id| format!("/companies/{id}"))
                .unwrap_or_default(),
            reason: warning
                .warning_type
                .as_deref()
                .map(humanize_token)
                .unwrap_or_else(|| {
                    format!("{} severity warning", humanize_token(&warning.severity))
                }),
            age_label: age_label(warning.ts_utc),
            assigned_analyst: warning
                .acknowledged_by
                .clone()
                .filter(|name| !name.trim().is_empty())
                .unwrap_or_else(|| "Unassigned".into()),
            can_acknowledge: true,
            acknowledge_url: format!("/warnings/{}/acknowledge", warning.id),
            investigate_url: format!("/warnings/{}", warning.id),
        });
    }
    for insight in &insight_queue_rows {
        let confidence = insight.confidence.unwrap_or(0.0);
        let entity_id = insight
            .entity_ids
            .as_ref()
            .and_then(|ids| ids.first())
            .copied();
        priority_queue.push(PriorityItem {
            id: insight.id.to_string(),
            kind: "insight".into(),
            title: insight.title.clone(),
            severity: insight_queue_severity(confidence).into(),
            confidence_pct: Some((confidence * 100.0).round() as i64),
            entity_name: entity_id
                .and_then(|id| queue_company_names.get(&id).cloned())
                .unwrap_or_else(|| {
                    if entity_id.is_some() {
                        "Linked entity".into()
                    } else {
                        "No company linked".into()
                    }
                }),
            entity_href: entity_id
                .map(|id| format!("/companies/{id}"))
                .unwrap_or_default(),
            reason: format!(
                "{} signal at {:.0}% confidence",
                humanize_token(insight.insight_type.as_deref().unwrap_or("analysis")),
                confidence * 100.0
            ),
            age_label: insight
                .created_at
                .map(age_label)
                .unwrap_or_else(|| "unknown age".into()),
            assigned_analyst: "Unassigned".into(),
            can_acknowledge: false,
            acknowledge_url: String::new(),
            investigate_url: format!("/insights/{}", insight.id),
        });
    }

    // ─── 3. Newly actionable opportunities ────────────────────────────────
    #[derive(sqlx::FromRow)]
    struct OpportunityRow {
        insight_id: Uuid,
        title: String,
        summary: Option<String>,
        confidence: Option<f64>,
        evidence_count: i64,
        company_id: Uuid,
        company_name: String,
        company_region: Option<String>,
        company_domain: Option<String>,
    }

    let opportunity_state = DataState::from_result(
        sqlx::query_as::<_, OpportunityRow>(
            r#"SELECT i.id AS insight_id,
                      i.title,
                      i.summary,
                      i.confidence,
                      COALESCE(array_length(i.evidence_urls, 1), 0)::bigint AS evidence_count,
                      c.id AS company_id,
                      c.name AS company_name,
                      c.region AS company_region,
                      c.domain AS company_domain
               FROM insights i
               JOIN companies c ON c.id = i.entity_ids[1]
               WHERE i.created_at >= $1
                 AND COALESCE(i.confidence, 0) >= 0.6
                 AND COALESCE(i.insight_type, '') NOT LIKE 'llm_%'
               ORDER BY i.confidence DESC NULLS LAST, i.created_at DESC
               LIMIT 4"#,
        )
        .bind(opportunity_window_start)
        .fetch_all(&store.pool)
        .await,
        "failed to fetch actionable opportunities",
        |rows| rows.is_empty(),
    );
    DegradedNotice::capture(&opportunity_state, &mut degraded_notice);
    let opportunity_rows = opportunity_state.into_items();

    // Buyer-side contact per account, preferring procurement/supply-chain
    // roles over generic executives. Loaded in one batch query.
    let opportunity_company_ids: Vec<Uuid> =
        opportunity_rows.iter().map(|row| row.company_id).collect();

    #[derive(sqlx::FromRow)]
    struct BuyerRow {
        primary_org_id: Uuid,
        name: String,
        role: Option<String>,
    }

    let buyer_state = DataState::from_result(
        if opportunity_company_ids.is_empty() {
            Ok(Vec::new())
        } else {
            sqlx::query_as::<_, BuyerRow>(
                r#"SELECT primary_org_id, name, "current_role" AS role
                   FROM persons
                   WHERE primary_org_id = ANY($1)
                   ORDER BY name ASC
                   LIMIT 200"#,
            )
            .bind(&opportunity_company_ids)
            .fetch_all(&store.pool)
            .await
        },
        "failed to fetch opportunity buyer contacts",
        |rows| rows.is_empty(),
    );
    DegradedNotice::capture(&buyer_state, &mut degraded_notice);
    let buyer_rows = buyer_state.into_items();

    fn buyer_role_score(role: &str) -> i32 {
        let lower = role.to_ascii_lowercase();
        if lower.contains("procure") || lower.contains("purchas") || lower.contains("sourc") {
            3
        } else if lower.contains("supply") || lower.contains("buyer") {
            2
        } else if lower.contains("director") || lower.contains("head") || lower.contains("vp") {
            1
        } else {
            0
        }
    }

    let mut best_buyers: HashMap<Uuid, (i32, String, String)> = HashMap::new();
    for row in buyer_rows {
        let role = row.role.unwrap_or_default();
        let score = buyer_role_score(&role);
        let entry = best_buyers
            .entry(row.primary_org_id)
            .or_insert_with(|| (score, row.name.clone(), role.clone()));
        if score > entry.0 {
            *entry = (score, row.name, role);
        }
    }

    let opportunities: Vec<OpportunityCard> = opportunity_rows
        .iter()
        .map(|row| {
            let (buyer_name, buyer_role) = best_buyers
                .get(&row.company_id)
                .map(|(_, name, role)| (name.clone(), role.clone()))
                .unwrap_or_else(|| ("No buyer contact recorded".into(), String::new()));
            OpportunityCard {
                insight_id: row.insight_id.to_string(),
                company_id: row.company_id.to_string(),
                company_name: row.company_name.clone(),
                region: row.company_region.clone().unwrap_or_default(),
                domain: row.company_domain.clone().unwrap_or_default(),
                why_now: row.title.clone(),
                evidence_count: row.evidence_count,
                confidence_pct: (row.confidence.unwrap_or(0.0) * 100.0).round() as i64,
                next_move: recommended_next_move(row.summary.as_deref().unwrap_or("")),
                buyer_name,
                buyer_role,
                href: format!("/insights/{}", row.insight_id),
            }
        })
        .collect();

    // ─── 4. What is stale or broken: coverage + freshness ─────────────────
    #[derive(sqlx::FromRow)]
    struct IngestionCoverageRow {
        total: i64,
        ever_succeeded: i64,
        never_attempted: i64,
        circuit_open: i64,
        newest_success: Option<DateTime<Utc>>,
    }

    let ingestion_state = DataState::from_result(
        sqlx::query_as::<_, IngestionCoverageRow>(
            r#"SELECT COUNT(*)::bigint AS total,
                      COUNT(*) FILTER (WHERE last_success_at IS NOT NULL)::bigint AS ever_succeeded,
                      COUNT(*) FILTER (WHERE last_attempt_at IS NULL)::bigint AS never_attempted,
                      COUNT(*) FILTER (WHERE circuit_open_until > NOW())::bigint AS circuit_open,
                      MAX(last_success_at) AS newest_success
               FROM source_runtime_state"#,
        )
        .fetch_one(&store.pool)
        .await,
        "failed to fetch ingestion coverage",
        |_| false,
    );
    DegradedNotice::capture(&ingestion_state, &mut degraded_notice);
    let ingestion = ingestion_state.into_loaded_or(IngestionCoverageRow {
        total: 0,
        ever_succeeded: 0,
        never_attempted: 0,
        circuit_open: 0,
        newest_success: None,
    });

    let stale_services_state = DataState::from_result(
        sqlx::query_scalar::<_, i64>(
            r#"SELECT COUNT(*)::bigint FROM (
                   SELECT service, MAX(last_seen_at) AS last_seen
                   FROM service_heartbeats
                   GROUP BY service
               ) latest
               WHERE latest.last_seen < NOW() - INTERVAL '10 minutes'"#,
        )
        .fetch_one(&store.pool)
        .await,
        "failed to fetch service heartbeats",
        |_| false,
    );
    DegradedNotice::capture(&stale_services_state, &mut degraded_notice);
    let stale_services = stale_services_state.into_loaded_or(0);

    let coverage_detail = if ingestion.total == 0 {
        "No ingestion sources are configured — nothing is being crawled.".to_string()
    } else {
        let newest = ingestion
            .newest_success
            .map(age_label)
            .unwrap_or_else(|| "never".to_string());
        format!(
            "{} sources · {} ever succeeded · {} never attempted · {} circuit open · newest success {}",
            ingestion.total,
            ingestion.ever_succeeded,
            ingestion.never_attempted,
            ingestion.circuit_open,
            newest
        )
    };
    let coverage_stale = ingestion
        .newest_success
        .map(|newest| reference_now.signed_duration_since(newest) > Duration::hours(48))
        .unwrap_or(true);
    let coverage_state = if ingestion.total == 0 || ingestion.circuit_open > 0 {
        "bad"
    } else if ingestion.ever_succeeded == 0 || ingestion.never_attempted > 0 || coverage_stale {
        "warn"
    } else {
        "ok"
    };
    let coverage_state_label = match coverage_state {
        "bad" => "Broken",
        "warn" => "Degraded",
        _ => "Covered",
    };

    let health_items = vec![
        HealthItem {
            label: "Data freshness".into(),
            detail: status_strip.data_freshness.clone(),
            state: if status_strip.data_fresh { "ok" } else { "bad" }.into(),
            state_label: if status_strip.data_fresh {
                "Fresh"
            } else {
                "Stale"
            }
            .into(),
            href: "/settings".into(),
        },
        HealthItem {
            label: "Ingestion coverage".into(),
            detail: coverage_detail,
            state: coverage_state.into(),
            state_label: coverage_state_label.into(),
            href: "/admin".into(),
        },
        HealthItem {
            label: "Service heartbeats".into(),
            detail: format!(
                "{} · {} service(s) last seen more than 10 minutes ago",
                status_strip.system_status, stale_services
            ),
            state: if status_strip.system_ok && stale_services == 0 {
                "ok"
            } else {
                "bad"
            }
            .into(),
            state_label: if status_strip.system_ok && stale_services == 0 {
                "Healthy"
            } else {
                "Stale"
            }
            .into(),
            href: "/admin".into(),
        },
    ];

    #[derive(sqlx::FromRow)]
    struct CrawlActivityRow {
        hour_label: String,
        success: i64,
        errors: i64,
    }

    // Use page_fingerprints (pages crawled) + observations (pipeline events) as crawl proxy
    let crawl_activity_result: Result<Vec<CrawlActivityRow>, sqlx::Error> =
        sqlx::query_as::<_, CrawlActivityRow>(
            r#"SELECT
               to_char(date_trunc('hour', h.hour), 'HH24:00') AS hour_label,
               COALESCE(pf.cnt, 0)::bigint AS success,
               COALESCE(obs.cnt, 0)::bigint AS errors
           FROM (
               SELECT generate_series(
                   date_trunc('hour', $1::timestamptz),
                   date_trunc('hour', $2::timestamptz),
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
        .bind(crawl_window_start)
        .bind(reference_now)
        .fetch_all(&store.pool)
        .await;
    let crawl_activity_state = DataState::from_result(
        crawl_activity_result,
        "failed to fetch crawl activity",
        |rows| rows.is_empty(),
    );
    DegradedNotice::capture(&crawl_activity_state, &mut degraded_notice);
    let crawl_activity_rows = crawl_activity_state.into_items();
    let crawl_max = crawl_activity_rows
        .iter()
        .map(|row| row.success.max(row.errors))
        .max()
        .unwrap_or(1)
        .max(1);
    let crawl_activity: Vec<CrawlActivityHour> = crawl_activity_rows
        .into_iter()
        .map(|row| CrawlActivityHour {
            hour_label: row.hour_label,
            success: row.success,
            errors: row.errors,
            success_h: row.success * 100 / crawl_max,
            errors_h: row.errors * 100 / crawl_max,
        })
        .collect();

    // Compute donut chart segments (size=120, stroke=14 → radius=53)
    let donut_segments = compute_donut_segments(&severity_breakdown, 53.0);

    // Read the REAL activity feed. The worker's `ActivityLogger` writes a row
    // here on every crawl, POI discovery, insight generation, threat detection,
    // battlecard/memo generation, recipe promotion, and job lifecycle event.
    // Previously this widget UNIONed only `warnings` + `insights`, which hid the
    // bulk of genuine system activity and made the feed look empty/stubbed.
    #[derive(sqlx::FromRow)]
    struct ActivityRow {
        action_type: String,
        actor_name: String,
        entity_type: Option<String>,
        entity_id: Option<String>,
        entity_name: Option<String>,
        details: serde_json::Value,
        created_at: DateTime<Utc>,
    }

    let activity_result: Result<Vec<ActivityRow>, sqlx::Error> = sqlx::query_as::<_, ActivityRow>(
        r#"SELECT action_type,
                  actor_name,
                  entity_type,
                  entity_id,
                  entity_name,
                  COALESCE(details, '{}'::jsonb) AS details,
                  created_at
             FROM activity_feed
            ORDER BY created_at DESC
            LIMIT 12"#,
    )
    .fetch_all(&store.pool)
    .await;
    let activity_state =
        DataState::from_result(activity_result, "failed to fetch activity feed", |rows| {
            rows.is_empty()
        });
    DegradedNotice::capture(&activity_state, &mut degraded_notice);
    let activity_rows = activity_state.into_items();

    let activity: Vec<ActivityEvent> = activity_rows
        .into_iter()
        .map(|r| {
            let (kind, icon, accent) = classify_activity_action(&r.action_type);

            // Build a readable description from entity + details.
            let mut parts: Vec<String> = Vec::new();
            if let Some(name) = &r.entity_name {
                if !name.trim().is_empty() {
                    parts.push(name.clone());
                }
            }
            // Surface a short detail note when present (e.g. "generated 3 insights").
            if let Some(note) = r
                .details
                .get("summary")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
            {
                parts.push(note.to_string());
            } else if let Some(obj) = r.details.as_object() {
                // Fallback: take the first string-valued detail field.
                if let Some((_, v)) = obj.iter().find(|(_, v)| v.is_string()) {
                    if let Some(s) = v.as_str() {
                        if !s.trim().is_empty() {
                            parts.push(s.to_string());
                        }
                    }
                }
            }
            let description = if parts.is_empty() {
                kind.to_string()
            } else {
                parts.join(" — ")
            };

            // Build a deep-link when the entity is addressable.
            let href = match (r.entity_type.as_deref(), r.entity_id.as_deref()) {
                (Some("company"), Some(id)) if !id.is_empty() => format!("/companies/{}", id),
                (Some("person"), Some(id)) if !id.is_empty() => format!("/persons/{}", id),
                (Some("insight"), Some(id)) if !id.is_empty() => format!("/insights/{}", id),
                (Some("warning"), Some(id)) if !id.is_empty() => format!("/warnings/{}", id),
                _ => String::new(),
            };

            ActivityEvent {
                kind: kind.to_string(),
                description,
                timestamp: r.created_at.format("%m-%d %H:%M").to_string(),
                href,
                icon,
                accent,
                actor: r.actor_name,
            }
        })
        .collect();

    let page = DashboardPage {
        current_path: ctx.current_path,
        can_admin: ctx.can_admin,
        username: ctx.username,
        warning_count: ctx.warning_count,
        theme: ctx.theme,
        status_strip,
        degraded_notice,

        changes_since,
        priority_queue,
        opportunities,
        health_items,

        stats: vec![
            StatCard {
                label: "Active Warnings".into(),
                value: stats_data.unacknowledged_warnings.to_string(),
                icon: "alert-triangle".into(),
                accent_class: "metric-rail-orange".into(),
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
                accent_class: "metric-rail-blue".into(),
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
                accent_class: "metric-rail-navy".into(),
                delta: None,
                direction: "flat".into(),
            },
            StatCard {
                label: "Persons of Interest".into(),
                value: stats_data.total_persons.to_string(),
                icon: "users".into(),
                accent_class: "metric-rail-gold".into(),
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
        data_freshness: data_freshness_text,
        portfolio_health: {
            let total_entities =
                (stats_data.total_companies + stats_data.total_persons).max(1) as f64;
            let critical_high = stats_data
                .threat_distribution
                .iter()
                .filter(|sc| sc.severity == "critical" || sc.severity == "high")
                .map(|sc| sc.count)
                .sum::<i64>() as f64;
            let ratio = critical_high / total_entities;
            if ratio < 0.1 {
                "Healthy".into()
            } else if ratio < 0.3 {
                "Elevated".into()
            } else {
                "At Risk".into()
            }
        },
        portfolio_health_class: {
            let total_entities =
                (stats_data.total_companies + stats_data.total_persons).max(1) as f64;
            let critical_high = stats_data
                .threat_distribution
                .iter()
                .filter(|sc| sc.severity == "critical" || sc.severity == "high")
                .map(|sc| sc.count)
                .sum::<i64>() as f64;
            let ratio = critical_high / total_entities;
            if ratio < 0.1 {
                "text-rams-green".into()
            } else if ratio < 0.3 {
                "text-rams-orange".into()
            } else {
                "text-rams-red".into()
            }
        },
    };

    super::render_template(&page)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system_status::StatusStrip;

    fn degraded_page(notice: &str) -> DashboardPage {
        DashboardPage {
            current_path: "/".into(),
            can_admin: false,
            username: "analyst".into(),
            warning_count: 0,
            theme: String::new(),
            status_strip: StatusStrip::unknown(),
            degraded_notice: Some(notice.to_string()),
            changes_since: vec![],
            priority_queue: vec![],
            opportunities: vec![],
            health_items: vec![],
            stats: vec![],
            recent_warnings: vec![],
            top_insights: vec![],
            activity: vec![],
            severity_breakdown: vec![],
            donut_segments: vec![],
            warning_trend: vec![],
            crawl_activity: vec![],
            region_slices: vec![],
            companies_tracked: 0,
            persons_tracked: 0,
            recipes_active: 0,
            data_freshness: "Data freshness unknown".into(),
            portfolio_health: "Healthy".into(),
            portfolio_health_class: String::new(),
        }
    }

    #[test]
    fn degraded_query_renders_degraded_marker_not_empty_states() {
        let page =
            degraded_page("Data unavailable — query failed at 14:03 UTC · incident inc-test123");
        let html = page.render().expect("dashboard renders");

        assert!(html.contains("incident inc-test123"));
        assert!(html.contains("data-degraded=\"true\""));
        assert!(!html.contains("No recent warnings"));
        assert!(!html.contains("No recent insights"));
        assert!(!html.contains("No activity yet"));
        assert!(!html.contains("No regional coverage data"));
    }

    #[test]
    fn empty_result_still_renders_empty_states() {
        let mut page = degraded_page("unused");
        page.degraded_notice = None;
        let html = page.render().expect("dashboard renders");

        assert!(!html.contains("incident inc-test123"));
        assert!(html.contains("No recent warnings"));
        assert!(html.contains("No recent insights"));
    }

    #[test]
    fn status_strip_shows_measured_freshness_not_hardcoded_live() {
        let mut page = degraded_page("unused");
        page.degraded_notice = None;
        page.status_strip = StatusStrip::from_parts(
            true,
            Some(chrono::Duration::seconds(15)),
            Some(chrono::Duration::minutes(4)),
        );
        let html = page.render().expect("dashboard renders");

        assert!(html.contains("Data current · newest observation 4m ago"));
        assert!(html.contains("System Online"));
        assert!(!html.contains(">Data Fresh<"));
    }

    #[test]
    fn degraded_dashboard_omits_new_section_empty_states() {
        let page =
            degraded_page("Data unavailable — query failed at 14:03 UTC · incident inc-test123");
        let html = page.render().expect("dashboard renders");

        assert!(!html.contains("Nothing requires action"));
        assert!(!html.contains("No newly actionable opportunities"));
    }

    #[test]
    fn priority_queue_renders_acknowledge_and_investigate_actions() {
        let mut page = degraded_page("unused");
        page.degraded_notice = None;
        page.priority_queue = vec![PriorityItem {
            id: "w-1".into(),
            kind: "warning".into(),
            title: "Supplier insolvency risk".into(),
            severity: "critical".into(),
            confidence_pct: Some(91),
            entity_name: "Acme EMS".into(),
            entity_href: "/companies/acme".into(),
            reason: "Supply risk".into(),
            age_label: "2h ago".into(),
            assigned_analyst: "Unassigned".into(),
            can_acknowledge: true,
            acknowledge_url: "/warnings/w-1/acknowledge".into(),
            investigate_url: "/warnings/w-1".into(),
        }];
        let html = page.render().expect("dashboard renders");

        assert!(html.contains("data-section=\"priority-queue\""));
        assert!(html.contains("hx-post=\"/warnings/w-1/acknowledge\""));
        assert!(html.contains("href=\"/warnings/w-1\""));
        assert!(html.contains("Analyst: Unassigned"));
        assert!(html.contains("91% conf"));
    }

    #[test]
    fn changes_and_health_render_counts_with_links() {
        let mut page = degraded_page("unused");
        page.degraded_notice = None;
        page.changes_since = vec![ChangeItem {
            label: "New high-confidence signals".into(),
            count: 7,
            detail: "Insights at 70%+ confidence created in the last 24h".into(),
            href: "/insights?min_confidence=0.7".into(),
        }];
        page.health_items = vec![HealthItem {
            label: "Ingestion coverage".into(),
            detail: "12 sources · 9 ever succeeded · 3 never attempted".into(),
            state: "warn".into(),
            state_label: "Degraded".into(),
            href: "/admin".into(),
        }];
        let html = page.render().expect("dashboard renders");

        assert!(html.contains("data-section=\"changes\""));
        assert!(html.contains("href=\"/insights?min_confidence=0.7\""));
        assert!(html.contains("New high-confidence signals"));
        assert!(html.contains("data-section=\"health\""));
        assert!(html.contains("3 never attempted"));
        assert!(html.contains("Degraded"));
    }

    #[test]
    fn vanity_counts_render_below_actionable_sections() {
        let mut page = degraded_page("unused");
        page.degraded_notice = None;
        page.stats = vec![StatCard {
            label: "Active Warnings".into(),
            value: "42".into(),
            icon: "alert-triangle".into(),
            accent_class: "metric-rail-orange".into(),
            delta: None,
            direction: "flat".into(),
        }];
        let html = page.render().expect("dashboard renders");

        let priority = html.find("What Requires Action").expect("priority section");
        let changes = html
            .find("What Changed Since Last Look")
            .expect("changes section");
        let opportunities = html
            .find("Newly Actionable Opportunities")
            .expect("opportunities section");
        let health = html
            .find("What Is Stale Or Broken")
            .expect("health section");
        let vanity = html.find("Portfolio Overview").expect("vanity divider");

        assert!(changes < priority);
        assert!(priority < opportunities);
        assert!(opportunities < health);
        assert!(health < vanity);
    }
}
