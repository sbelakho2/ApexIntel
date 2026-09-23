//! Executive Dashboard handler — GET /executive
//!
//! Covers: C-suite-ready single page with top-5 competitors by mention-volume,
//! top-3 risks, trending topics, recent wins/losses, and recommended actions.
//! Reuses existing [`PgStore`] methods for dashboard stats, strategic opportunities,
//! critical threats, and insights.

use std::collections::BTreeMap;
use std::sync::Arc;

use askama::Template;
use axum::{response::IntoResponse, Extension};

use super::PageContext;
use crate::middleware::session::WebSession;
use apex_store::postgres::{InsightListFilters, PgStore};

// ─── Display types ───────────────────────────────────────────────────────────

/// A single stat card on the executive dashboard.
#[derive(Debug, Clone)]
pub struct ExecutiveStatCard {
    pub label: String,
    pub value: String,
    pub icon: String,
    pub accent_class: String,
    pub delta: Option<String>,
    pub direction: String,
}

/// A competitor ranked by mention-volume.
#[derive(Debug, Clone)]
pub struct CompetitorMention {
    pub name: String,
    pub mention_count: i64,
    pub trend_direction: String,
    pub risk_level: String,
    /// Bar width percentage for visualisation (0–100).
    pub bar_pct: i64,
}

/// A top risk item for the executive view.
#[derive(Debug, Clone)]
pub struct TopRisk {
    pub id: String,
    pub title: String,
    pub severity: String,
    pub company: String,
    pub score: f64,
    pub region: String,
    pub score_display: String,
}

/// A trending topic with week-over-week change.
#[derive(Debug, Clone)]
pub struct TrendingTopic {
    pub topic: String,
    pub mention_count: i64,
    pub change_pct: String,
}

/// A recent win or loss from strategic opportunities.
#[derive(Debug, Clone)]
pub struct RecentWinLoss {
    pub title: String,
    pub result_type: String,
    pub company: String,
    pub date: String,
    pub description: String,
}

/// A recommended action derived from high-priority items.
#[derive(Debug, Clone)]
pub struct RecommendedAction {
    pub id: String,
    pub title: String,
    pub priority: String,
    pub owner: String,
    pub due_date: String,
    pub description: String,
}

// ─── Template struct ─────────────────────────────────────────────────────────

/// Executive dashboard page context.
#[derive(Debug, Clone, Template)]
#[template(path = "pages/executive.html")]
pub struct ExecutiveDashboardPage {
    // Base layout fields
    pub current_path: String,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,

    // Executive dashboard data
    pub stat_cards: Vec<ExecutiveStatCard>,
    pub competitors: Vec<CompetitorMention>,
    pub top_risks: Vec<TopRisk>,
    pub trending_topics: Vec<TrendingTopic>,
    pub recent_wins_losses: Vec<RecentWinLoss>,
    pub recommended_actions: Vec<RecommendedAction>,
}

// ─── Handler ─────────────────────────────────────────────────────────────────

/// GET /executive — render the executive dashboard page.
pub async fn executive_dashboard(
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
) -> impl IntoResponse {
    // ── Fetch data sources with graceful degradation ──────────────────────

    let stats_data = store.get_dashboard_stats().await.unwrap_or_else(|e| {
        tracing::error!("Failed to fetch dashboard stats: {e}");
        Default::default()
    });

    let opportunities = store
        .list_strategic_opportunities(false, 20)
        .await
        .unwrap_or_else(|e| {
            tracing::error!("Failed to fetch strategic opportunities: {e}");
            vec![]
        });

    let threats = store
        .list_critical_threats(20)
        .await
        .unwrap_or_else(|e| {
            tracing::error!("Failed to fetch critical threats: {e}");
            vec![]
        });

    // Fetch recent insights for trending topics & competitor mention volume
    let insight_filters = InsightListFilters {
        exclude_internal: true,
        ..Default::default()
    };
    let recent_insights = store
        .list_insights(&insight_filters, 100, 0)
        .await
        .unwrap_or_else(|e| {
            tracing::error!("Failed to fetch insights: {e}");
            vec![]
        });

    // B312: competitor names come from the tracked competitor set, not a
    // hardcoded demo list that only matched one specific deployment.
    let competitor_names: Vec<String> = store
        .list_competitors(50, 0)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|c| c.name)
        .collect();

    let unack_warnings = stats_data.unacknowledged_warnings as i64;
    let ctx = PageContext::from_session(&session, "/executive", unack_warnings);

    // ── Build stat cards ─────────────────────────────────────────────────

    let total_opportunities = opportunities.len();
    let active_threats = threats.len();
    let companies_tracked = stats_data.total_companies;
    let avg_confidence = {
        let confidence_values: Vec<f64> = opportunities
            .iter()
            .map(|o| o.confidence)
            .chain(threats.iter().map(|t| t.confidence))
            .collect();
        if confidence_values.is_empty() {
            0.0
        } else {
            let sum: f64 = confidence_values.iter().sum();
            (sum / confidence_values.len() as f64 * 100.0).round() / 100.0
        }
    };

    let stat_cards = vec![
        ExecutiveStatCard {
            label: "Opportunities".into(),
            value: total_opportunities.to_string(),
            icon: "trending-up".into(),
            accent_class: "metric-rail-green".into(),
            delta: Some(format!("{} open", total_opportunities)),
            direction: "up".into(),
        },
        ExecutiveStatCard {
            label: "Active Threats".into(),
            value: active_threats.to_string(),
            icon: "alert-triangle".into(),
            accent_class: "metric-rail-red".into(),
            delta: Some(format!("{} critical", active_threats)),
            direction: "up".into(),
        },
        ExecutiveStatCard {
            label: "Companies Tracked".into(),
            value: companies_tracked.to_string(),
            icon: "boxes".into(),
            accent_class: "metric-rail-steel".into(),
            delta: None,
            direction: "flat".into(),
        },
        ExecutiveStatCard {
            label: "Avg Confidence".into(),
            value: format!("{:.0}%", avg_confidence * 100.0),
            icon: "bar-chart-3".into(),
            accent_class: "metric-rail-orange".into(),
            delta: None,
            direction: "flat".into(),
        },
    ];

    // ── Build competitor mention-volume ──────────────────────────────────

    // Compute mention-counts from insight entity references (tags/entities)
    let mut mention_counts: BTreeMap<String, i64> = BTreeMap::new();
    for insight in &recent_insights {
        if let Some(ref tags) = insight.tags {
            for tag in tags {
                *mention_counts.entry(tag.clone()).or_default() += 1;
            }
        }
        // Also scan entity_ids for competitor references
        if let Some(ref entity_ids) = insight.entity_ids {
            for _eid in entity_ids {
                // We can't resolve entity names without another query; use tags for now
            }
        }
    }
    // Fallback: if no tag-based mentions, extract from insight titles using
    // the tracked competitor set (B312 — previously a hardcoded demo list).
    for name in &competitor_names {
        if name.trim().is_empty() {
            continue;
        }
        let count = recent_insights
            .iter()
            .filter(|i| {
                i.title.contains(name.as_str())
                    || i.summary.contains(name.as_str())
                    || i.tags.as_ref().is_some_and(|t| t.iter().any(|tag| tag.contains(name.as_str())))
            })
            .count() as i64;
        if count > 0 {
            *mention_counts.entry(name.clone()).or_default() += count;
        }
    }

    let mut sorted_competitors: Vec<(String, i64)> = mention_counts.into_iter().collect();
    sorted_competitors.sort_by(|a, b| b.1.cmp(&a.1));
    sorted_competitors.truncate(5);

    let max_mention = sorted_competitors
        .first()
        .map(|(_, c)| *c)
        .unwrap_or(1)
        .max(1);
    let competitors: Vec<CompetitorMention> = sorted_competitors
        .into_iter()
        .map(|(name, count)| {
            let raw_pct = count * 100 / max_mention;
            let bar_pct = raw_pct.max(5).min(100);
            CompetitorMention {
                name,
                mention_count: count,
                // B313: no fabricated trend — direction is unknown without a
                // comparison window; the template renders a neutral state.
                trend_direction: "flat".into(),
                // Relative-to-top volume bands (share of the noisiest
                // competitor), not absolute mention counts.
                risk_level: if raw_pct >= 67 {
                    "high".into()
                } else if raw_pct >= 34 {
                    "medium".into()
                } else {
                    "low".into()
                },
                bar_pct,
            }
        })
        .collect();

    // ── Build top-3 risks from critical threats ──────────────────────────

    let mut sorted_threats: Vec<_> = threats.into_iter().collect();
    sorted_threats.sort_by(|a, b| {
        b.impact_score
            .partial_cmp(&a.impact_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    sorted_threats.truncate(3);

    let top_risks: Vec<TopRisk> = sorted_threats
        .into_iter()
        .map(|t| TopRisk {
            id: t.id.to_string(),
            title: t.title.clone(),
            severity: t.severity.clone(),
            company: t
                .entity_id
                .clone()
                .unwrap_or_else(|| "Unknown".into()),
            score: t.impact_score,
            region: t.region.clone().unwrap_or_default(),
            score_display: format!("{:.1}", t.impact_score),
        })
        .collect();

    // ── Build trending topics from insights ──────────────────────────────

    let mut topic_counts: BTreeMap<String, i64> = BTreeMap::new();
    for insight in &recent_insights {
        if let Some(ref tags) = insight.tags {
            for tag in tags {
                if !competitor_names.iter().any(|k| tag.contains(k.as_str())) {
                    *topic_counts.entry(tag.clone()).or_default() += 1;
                }
            }
        }
        // Use insight_type as topic signal
        if let Some(ref insight_type) = insight.insight_type {
            if !insight_type.starts_with("llm_") {
                *topic_counts.entry(insight_type.clone()).or_default() += 1;
            }
        }
    }

    let mut sorted_topics: Vec<(String, i64)> = topic_counts.into_iter().collect();
    sorted_topics.sort_by(|a, b| b.1.cmp(&a.1));
    sorted_topics.truncate(5);

    // B313: honest week-over-week change — count this week's mentions against
    // the previous week's from the same loaded window, instead of the
    // fabricated `count × 7.5%` figure that always rendered an upward trend.
    let now = chrono::Utc::now();
    let week_ago = now - chrono::Duration::days(7);
    let two_weeks_ago = now - chrono::Duration::days(14);
    let trending_topics: Vec<TrendingTopic> = sorted_topics
        .into_iter()
        .map(|(topic, count)| {
            let topic_matches =
                |i: &apex_store::postgres::InsightRow, t: &str| -> bool {
                    i.tags.as_ref().is_some_and(|tags| tags.iter().any(|tag| tag == t))
                        || i.insight_type.as_deref() == Some(t)
                };
            let this_week = recent_insights
                .iter()
                .filter(|i| i.created_at.is_some_and(|ts| ts >= week_ago) && topic_matches(i, &topic))
                .count() as i64;
            let prev_week = recent_insights
                .iter()
                .filter(|i| {
                    i.created_at.is_some_and(|ts| ts >= two_weeks_ago && ts < week_ago)
                        && topic_matches(i, &topic)
                })
                .count() as i64;
            let change_pct = if prev_week == 0 {
                if this_week == 0 {
                    "±0%".to_string()
                } else {
                    "new".to_string()
                }
            } else {
                let delta = ((this_week - prev_week) as f64 / prev_week as f64 * 100.0).round() as i64;
                format!("{}{}%", if delta >= 0 { "+" } else { "" }, delta)
            };
            let _ = count;
            TrendingTopic {
                topic,
                mention_count: this_week.max(count),
                change_pct,
            }
        })
        .collect();

    // ── Build recent wins/losses from strategic opportunities ────────────

    let sorted_opps: Vec<_> = {
        let mut v = opportunities.clone();
        v.sort_by(|a, b| b.priority_score.partial_cmp(&a.priority_score).unwrap_or(std::cmp::Ordering::Equal));
        v.truncate(5);
        v
    };

    let recent_wins_losses: Vec<RecentWinLoss> = sorted_opps
        .into_iter()
        .map(|o| {
            let is_win = o.status == "won" || o.status == "closed";
            RecentWinLoss {
                title: o.title.clone(),
                result_type: if is_win { "win".into() } else { "opportunity".into() },
                company: o.entity_id.clone().unwrap_or_default(),
                date: o.created_at.format("%Y-%m-%d").to_string(),
                description: o.description.clone().unwrap_or_default(),
            }
        })
        .collect();

    // ── Build recommended actions from high-priority items ───────────────

    // B313: priority_score is validated to [0,1] on write, so the previous
    // `>= 7.0` / `>= 8.0` cutoffs filtered out every opportunity and the
    // recommended-actions module was permanently empty.
    let high_priority_opps: Vec<_> = opportunities
        .iter()
        .filter(|o| o.priority_score >= 0.7)
        .take(3)
        .collect();

    let recommended_actions: Vec<RecommendedAction> = high_priority_opps
        .into_iter()
        .map(|o| RecommendedAction {
            id: o.id.to_string(),
            title: o.title.clone(),
            priority: if o.priority_score >= 0.8 {
                "critical".into()
            } else {
                "high".into()
            },
            owner: o.owner_id.clone().unwrap_or_else(|| "Unassigned".into()),
            due_date: o
                .due_date
                .map(|d| d.format("%Y-%m-%d").to_string())
                .unwrap_or_else(|| "No deadline".into()),
            description: o.description.clone().unwrap_or_default(),
        })
        .collect();

    let page = ExecutiveDashboardPage {
        current_path: ctx.current_path,
        username: ctx.username,
        warning_count: ctx.warning_count,
        theme: ctx.theme,
        stat_cards,
        competitors,
        top_risks,
        trending_topics,
        recent_wins_losses,
        recommended_actions,
    };

    super::render_template(&page)
}
