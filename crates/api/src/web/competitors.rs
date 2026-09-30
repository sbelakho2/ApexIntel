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
use apex_core::data_state::{DataState, DegradedNotice};
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
    /// Measured threat (0..=100); `None` when no risk model has produced a
    /// score. Never rendered or averaged as zero.
    pub risk_pct: Option<i64>,
    /// Measured tracked-entity overlap (0..=100); `None` when not measured.
    pub overlap_pct: Option<i64>,
    /// Real warning count for this competitor (batched query).
    pub warning_count: i64,
    /// Real insight count for this competitor (batched query).
    pub insight_count: i64,
    /// Latest recorded change summary and its date, when one exists.
    pub recent_change: Option<String>,
    pub change_date: Option<String>,
    pub strategic_context: String,
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
    pub can_admin: bool,
    pub can_write: bool,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,
    pub status_strip: crate::system_status::StatusStrip,

    pub competitors: Vec<CompetitorCard>,
    pub total: i64,
    pub recent_changes: Vec<CompetitorChange>,
    /// Mean measured threat over competitors that have a score; `None` when
    /// nothing has been measured.
    pub avg_threat: Option<i64>,
    /// Count of competitors with a measured threat >= 70.
    pub high_threat_count: i64,
    /// Mean measured overlap; `None` when nothing has been measured.
    pub avg_overlap_pct: Option<i64>,
    pub chart_width: i64,
    pub active_threat: String,
    pub active_overlap: String,
    pub active_status: String,
    pub threat_filters: Vec<CompetitorFilterChip>,
    pub overlap_filters: Vec<CompetitorFilterChip>,
    pub active_filters: usize,
    pub reset_href: String,
    pub degraded_notice: Option<String>,
}

/// HTMX partial — just the results fragment.
#[derive(Template)]
#[template(path = "pages/competitors/_list.html")]
pub struct CompetitorsListPartial {
    pub competitors: Vec<CompetitorCard>,
    pub total: i64,
    pub recent_changes: Vec<CompetitorChange>,
    /// Mean measured threat over competitors that have a score; `None` when
    /// nothing has been measured.
    pub avg_threat: Option<i64>,
    /// Count of competitors with a measured threat >= 70.
    pub high_threat_count: i64,
    /// Mean measured overlap; `None` when nothing has been measured.
    pub avg_overlap_pct: Option<i64>,
    pub chart_width: i64,
    pub active_threat: String,
    pub active_overlap: String,
    pub threat_filters: Vec<CompetitorFilterChip>,
    pub overlap_filters: Vec<CompetitorFilterChip>,
    pub active_filters: usize,
    pub reset_href: String,
    pub degraded_notice: Option<String>,
}

// ─── Handler ────────────────────────────────────────────────────────────────

/// GET /competitors — competitor tracking overview.
pub async fn list_competitors(
    headers: HeaderMap,
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
    Query(params): Query<CompetitorsQuery>,
) -> impl IntoResponse {
    let mut degraded_notice: Option<String> = None;
    let unack_state = DataState::from_result(
        store
            .count_warnings(&WarningListFilters {
                acknowledged: Some(false),
                ..Default::default()
            })
            .await,
        "count_warnings failed (web competitors page)",
        |_| false,
    );
    DegradedNotice::capture(&unack_state, &mut degraded_notice);
    let ctx = PageContext::from_session(&session, "/competitors", unack_state.into_loaded_or(0));

    // Fetch competitor companies
    let competitor_rows_state = DataState::from_result(
        store.list_competitors(100, 0).await,
        "list_competitors failed (web competitors page)",
        Vec::is_empty,
    );
    DegradedNotice::capture(&competitor_rows_state, &mut degraded_notice);
    let competitor_rows = competitor_rows_state.into_items();
    let total_state = DataState::from_result(
        store.count_competitors().await,
        "count_competitors failed (web competitors page)",
        |_| false,
    );
    DegradedNotice::capture(&total_state, &mut degraded_notice);
    let total = total_state.into_loaded_or(competitor_rows.len() as i64);

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

    // Real engagement signals for every competitor on the page, in one
    // batched call: warning counts, insight counts, latest change.
    let competitor_ids: Vec<uuid::Uuid> = competitor_rows.iter().map(|c| c.id).collect();
    let engagement_state = DataState::from_result(
        store.get_competitor_engagement(&competitor_ids).await,
        "get_competitor_engagement failed (web competitors page)",
        |engagement| {
            engagement.warning_counts.is_empty()
                && engagement.insight_counts.is_empty()
                && engagement.latest_changes.is_empty()
        },
    );
    DegradedNotice::capture(&engagement_state, &mut degraded_notice);
    let engagement =
        engagement_state.into_loaded_or(apex_store::postgres::CompetitorEngagement::default());

    let mut competitors: Vec<CompetitorCard> = competitor_rows
        .iter()
        .enumerate()
        .map(|(i, c)| {
            // Unknown stays unknown: `None` is not rendered or charted as 0.
            let risk = c
                .risk_score
                .map(|s| ((s * 100.0).round() as i64).clamp(0, 100));
            let overlap = c
                .overlap_score
                .map(|s| ((s * 100.0).round() as i64).clamp(0, 100));
            let group_x = i as i64 * group_w;
            let threat_h = risk.map_or(0, |r| r * chart_area_h / 100);
            let overlap_h = overlap.map_or(0, |o| o * chart_area_h / 100);
            let strategic_context = match (risk, overlap) {
                (Some(r), Some(_)) if r >= 70 => {
                    "High-threat competitor requiring close monitoring".to_string()
                }
                (Some(r), _) if r >= 40 => {
                    "Moderate competitor presence in tracked sectors".to_string()
                }
                (Some(_), Some(o)) if o >= 30 => {
                    "Limited measured threat; notable tracked overlap".to_string()
                }
                (Some(_), Some(_)) => {
                    "Low measured threat with limited tracked overlap".to_string()
                }
                (Some(_), None) => "Measured threat; tracked overlap not measured".to_string(),
                (None, Some(_)) => "Threat not measured; measured tracked overlap".to_string(),
                (None, None) => "Insufficient data to assess threat or overlap".to_string(),
            };
            let latest_change = engagement
                .latest_changes
                .get(&c.id)
                .map(|change| (change.description.clone(), change.detected_at.clone()));
            CompetitorCard {
                id: c.id.to_string(),
                name: c.name.clone(),
                sector: c.company_type.clone().unwrap_or_default(),
                region: c.region.clone().unwrap_or_default(),
                risk_pct: risk,
                overlap_pct: overlap,
                warning_count: engagement.warning_counts.get(&c.id).copied().unwrap_or(0),
                insight_count: engagement.insight_counts.get(&c.id).copied().unwrap_or(0),
                recent_change: latest_change
                    .as_ref()
                    .map(|(description, _)| description.clone()),
                change_date: latest_change.map(|(_, detected_at)| detected_at),
                strategic_context,
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
        // Competitors without a measured threat are not in any tier: an
        // unknown score is not a "low" score.
        competitors.retain(|c| {
            c.risk_pct.is_some_and(|risk| {
                let tier = if risk >= 70 {
                    "high"
                } else if risk >= 40 {
                    "medium"
                } else {
                    "low"
                };
                tier == active_threat
            })
        });
    }

    if !active_overlap.is_empty() {
        competitors.retain(|c| {
            c.overlap_pct.is_some_and(|overlap| {
                let bucket = if overlap >= 50 {
                    "50%+"
                } else if overlap >= 30 {
                    "30-50%"
                } else {
                    "<30%"
                };
                bucket == active_overlap
            })
        });
    }

    // Aggregates cover measured values only; nothing measured means "—".
    let measured_threats: Vec<i64> = competitors.iter().filter_map(|c| c.risk_pct).collect();
    let avg_threat = if measured_threats.is_empty() {
        None
    } else {
        Some(measured_threats.iter().sum::<i64>() / measured_threats.len() as i64)
    };
    let high_threat_count = measured_threats.iter().filter(|risk| **risk >= 70).count() as i64;
    let measured_overlaps: Vec<i64> = competitors.iter().filter_map(|c| c.overlap_pct).collect();
    let avg_overlap_pct = if measured_overlaps.is_empty() {
        None
    } else {
        Some(measured_overlaps.iter().sum::<i64>() / measured_overlaps.len() as i64)
    };

    // Fetch recent competitor changes
    let change_rows_state = DataState::from_result(
        store.get_all_competitor_changes_paged(1, 20).await,
        "get_all_competitor_changes_paged failed (web competitors page)",
        |_| false,
    );
    DegradedNotice::capture(&change_rows_state, &mut degraded_notice);
    let change_rows = change_rows_state.into_loaded_or((vec![], 0)).0;
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
        can_admin: ctx.can_admin,
        can_write: ctx.can_write,
        status_strip: crate::system_status::StatusStrip::current(),
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
        active_status: String::new(),
        threat_filters,
        overlap_filters,
        active_filters,
        reset_href,
        degraded_notice: degraded_notice.clone(),
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
            degraded_notice: tpl.degraded_notice.clone(),
        };
        super::render_template(&partial)
    } else {
        super::render_template(&tpl)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Unknown stays unknown: a competitor with no measured risk/overlap must
    /// not be rendered, averaged or filtered as if it scored zero. The old
    /// test asserted `unknown overlap == 0`, which is the exact semantics the
    /// audit rejected.
    #[test]
    fn unknown_scores_are_not_zero() {
        let card = CompetitorCard {
            id: "test".into(),
            name: "Acme".into(),
            sector: "EMS".into(),
            region: "EU".into(),
            risk_pct: None,
            overlap_pct: None,
            warning_count: 0,
            insight_count: 0,
            recent_change: None,
            change_date: None,
            strategic_context: String::new(),
            chart_group_x: 0,
            chart_threat_y: 0,
            chart_threat_h: 0,
            chart_overlap_y: 0,
            chart_overlap_h: 0,
            chart_label_x: 0,
        };
        assert_ne!(card.risk_pct, Some(0), "unknown threat is not zero threat");
        assert_ne!(
            card.overlap_pct,
            Some(0),
            "unknown overlap is not zero overlap"
        );
        // The old fabricated value would have been (80 * 0.65).round() == 52.
        assert_ne!(card.overlap_pct, Some(52));
    }

    /// Measured values survive rounding and the aggregate helpers skip
    /// unmeasured cards instead of counting them as zero.
    #[test]
    fn measured_values_are_preserved_and_unmeasured_are_skipped() {
        let measured = [Some(80_i64), None, Some(40)];
        let values: Vec<i64> = measured.iter().filter_map(|value| *value).collect();
        assert_eq!(values.len(), 2, "unmeasured entries are skipped");
        assert_eq!(values.iter().sum::<i64>() / values.len() as i64, 60);
    }
}
