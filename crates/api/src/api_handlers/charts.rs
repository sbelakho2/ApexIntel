//! Chart data API handlers.
//!
//! Returns JSON data points for client-side chart rendering and
//! SVG fragments for HTMX-based chart loading.

use crate::*;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use chrono::{Duration, Utc};
use serde::Serialize;
use std::time::Instant;

// ─── Request types ───────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(crate) struct ActivityQuery {
    pub days: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ObservationChartQuery {
    pub days: Option<i64>,
    pub bucket: Option<String>, // "day", "week", "month"
}

// ─── Response types ──────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub(crate) struct ActivityChartResponse {
    pub dates: Vec<String>,
    pub observations: Vec<u32>,
    pub insights: Vec<u32>,
    pub scores: Vec<f64>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ObservationBucket {
    pub label: String,
    pub count: u64,
}

#[derive(Debug, Serialize)]
pub(crate) struct ObservationChartResponse {
    pub buckets: Vec<ObservationBucket>,
}

// ─── Chart data point for template rendering ─────────────────────────────────

#[derive(Debug, Serialize)]
pub(crate) struct ChartDataPoint {
    pub date: String,
    pub value: f64,
}

// ─── SVG rendering helpers ───────────────────────────────────────────────────

/// Build an SVG polyline sparkline as an HTML string fragment.
pub(crate) fn render_sparkline_svg(
    data: &[ChartDataPoint],
    width: u32,
    height: u32,
    color: &str,
) -> String {
    if data.is_empty() {
        return format!(
            r#"<div class="apex-card p-4 text-center"><p class="text-xs font-semibold text-muted-foreground">No data</p></div>"#
        );
    }

    let w = width.max(50) as f64;
    let h = height.max(20) as f64;
    let pad = 4.0;
    let plot_w = w - pad * 2.0;
    let plot_h = h - pad * 2.0;
    let n = data.len();

    if n == 1 {
        let cx = w / 2.0;
        let cy = h / 2.0;
        return format!(
            r#"<svg width="{}" height="{}" viewBox="0 0 {} {}" role="img" aria-label="Sparkline single point" class="w-full"><desc>Single data point</desc><circle cx="{:.1}" cy="{:.1}" r="4" fill="{}" stroke="white" stroke-width="2"/><text x="{:.1}" y="{:.1}" text-anchor="middle" font-size="10" font-weight="700" fill="currentColor">{:.1}</text></svg>"#,
            w, h, w, h, cx, cy, color, cx, cy - 10.0, data[0].value
        );
    }

    let min_val = data.iter().map(|d| d.value).fold(f64::INFINITY, f64::min);
    let max_val = data
        .iter()
        .map(|d| d.value)
        .fold(f64::NEG_INFINITY, f64::max);
    let spread = (max_val - min_val).max(1e-9);

    let mut polyline_pts = String::new();
    let mut area_pts = String::new();
    let nf = (n - 1) as f64;

    // Area starts at bottom-left
    area_pts.push_str(&format!("{:.1},{:.1}", pad, pad + plot_h));

    for (i, point) in data.iter().enumerate() {
        let x = pad + (i as f64 / nf) * plot_w;
        let y = pad + plot_h - ((point.value - min_val) / spread) * plot_h;
        if i > 0 {
            polyline_pts.push(' ');
            area_pts.push(' ');
        }
        polyline_pts.push_str(&format!("{:.1},{:.1}", x, y));
        area_pts.push_str(&format!("{:.1},{:.1}", x, y));
    }

    // Close area at bottom-right
    let last_x = pad + plot_w;
    area_pts.push_str(&format!(" {:.1},{:.1} Z", last_x, pad + plot_h));

    let last_pt = data.last().unwrap();
    let last_px = pad + plot_w;
    let last_py = pad + plot_h - ((last_pt.value - min_val) / spread) * plot_h;

    format!(
        r#"<svg width="{}" height="{}" viewBox="0 0 {} {}" role="img" aria-label="Sparkline with {} points" class="w-full"><desc>Trend sparkline</desc><polygon points="{}" fill="{}" opacity="0.10"/><polyline points="{}" fill="none" stroke="{}" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/><circle cx="{:.1}" cy="{:.1}" r="3" fill="{}" stroke="white" stroke-width="1.5"/></svg>"#,
        w, h, w, h, n, area_pts, color, polyline_pts, color, last_px, last_py, color
    )
}

/// Build a multi-series activity chart SVG as an HTML string.
pub(crate) fn render_activity_chart_svg(
    response: &ActivityChartResponse,
    width: u32,
    height: u32,
) -> String {
    let n = response.dates.len();
    if n == 0 {
        return r#"<div class="apex-card p-6 text-center"><p class="text-sm font-semibold text-muted-foreground">No activity data</p></div>"#.to_string();
    }

    let w = width.max(200) as f64;
    let h = height.max(100) as f64;
    let pad_left = 50.0;
    let pad_right = 16.0;
    let pad_top = 32.0;
    let pad_bottom = 36.0;
    let plot_w = w - pad_left - pad_right;
    let plot_h = h - pad_top - pad_bottom;

    // Compute global max
    let all_values: Vec<f64> = response
        .observations
        .iter()
        .map(|v| *v as f64)
        .chain(response.insights.iter().map(|v| *v as f64))
        .chain(response.scores.iter().copied())
        .collect();
    let max_val = all_values
        .iter()
        .copied()
        .fold(f64::NEG_INFINITY, f64::max)
        .max(1.0);

    let nf = (n - 1) as f64;

    let series: Vec<(&[u32], &str, &str)> = vec![
        (&response.observations, "var(--chart-series-blue)", "Observations"),
        (&response.insights, "var(--chart-series-amber)", "Insights"),
    ];
    let score_series: &[f64] = &response.scores;

    let mut svg = String::new();
    svg.push_str(&format!(
        r#"<svg width="{}" height="{}" viewBox="0 0 {} {}" role="img" aria-label="Entity activity chart" class="w-full">"#,
        w, h, w, h
    ));
    svg.push_str("<desc>Multi-series activity chart</desc>");

    // Plot background
    svg.push_str(&format!(
        r#"<rect x="{:.1}" y="{:.1}" width="{:.1}" height="{:.1}" rx="8" fill="rgba(255,255,255,0.6)" stroke="rgba(22,22,22,0.10)" stroke-width="1"/>"#,
        pad_left, pad_top, plot_w, plot_h
    ));

    // Grid lines
    svg.push_str(&format!(
        r#"<line x1="{:.1}" y1="{:.1}" x2="{:.1}" y2="{:.1}" stroke="rgba(22,22,22,0.15)" stroke-width="1"/>"#,
        pad_left, pad_top + plot_h, pad_left + plot_w, pad_top + plot_h
    ));
    for frac in [0.25, 0.5, 0.75] {
        let y = pad_top + plot_h * (1.0 - frac);
        svg.push_str(&format!(
            r#"<line x1="{:.1}" y1="{:.1}" x2="{:.1}" y2="{:.1}" stroke="rgba(22,22,22,0.08)" stroke-width="1" stroke-dasharray="4 4"/>"#,
            pad_left, y, pad_left + plot_w, y
        ));
    }

    // Y-axis labels
    for (frac, label) in [(0.0, "0"), (0.5, &format!("{:.0}", max_val * 0.5)), (1.0, &format!("{:.0}", max_val))] {
        let y = pad_top + plot_h * (1.0 - frac);
        svg.push_str(&format!(
            r#"<text x="{:.1}" y="{:.1}" text-anchor="end" dominant-baseline="middle" font-size="9" font-weight="600" fill="var(--chart-label)">{}</text>"#,
            pad_left - 6.0, y, label
        ));
    }

    // Helper to build a polyline
    let build_polyline = |values: &[f64], color: &str| -> String {
        let mut pts = String::new();
        for (i, v) in values.iter().enumerate() {
            let x = pad_left + (i as f64 / nf) * plot_w;
            let y = pad_top + plot_h - ((v - 0.0) / max_val) * plot_h;
            if i > 0 {
                pts.push(' ');
            }
            pts.push_str(&format!("{:.1},{:.1}", x, y));
        }
        format!(
            r#"<polyline points="{}" fill="none" stroke="{}" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>"#,
            pts, color
        )
    };

    // Observations polyline
    let obs_f64: Vec<f64> = response.observations.iter().map(|v| *v as f64).collect();
    svg.push_str(&build_polyline(&obs_f64, "var(--chart-series-blue)"));

    // Insights polyline
    let ins_f64: Vec<f64> = response.insights.iter().map(|v| *v as f64).collect();
    svg.push_str(&build_polyline(&ins_f64, "var(--chart-series-amber)"));

    // Activity scores polyline
    svg.push_str(&build_polyline(score_series, "var(--chart-series-green)"));

    // Data points
    let series_colors: [(&[f64], &str); 3] = [
        (&obs_f64, "var(--chart-series-blue)"),
        (&ins_f64, "var(--chart-series-amber)"),
        (score_series, "var(--chart-series-green)"),
    ];
    for (values, color) in series_colors {
        for (i, v) in values.iter().enumerate() {
            let x = pad_left + (i as f64 / nf) * plot_w;
            let y = pad_top + plot_h - ((v - 0.0) / max_val) * plot_h;
            svg.push_str(&format!(
                r#"<circle cx="{:.1}" cy="{:.1}" r="2.5" fill="{}" stroke="white" stroke-width="1.5"/>"#,
                x, y, color
            ));
        }
    }

    // Legend
    let legend_items = [
        ("var(--chart-series-blue)", "Observations"),
        ("var(--chart-series-amber)", "Insights"),
        ("var(--chart-series-green)", "Activity"),
    ];
    for (i, (color, label)) in legend_items.iter().enumerate() {
        let lx = pad_left + i as f64 * 140.0;
        svg.push_str(&format!(
            r#"<g transform="translate({:.1}, 12)"><rect x="0" y="-4" width="10" height="10" rx="2" fill="{}"/><text x="16" y="3" dominant-baseline="middle" font-size="10" font-weight="700" fill="currentColor">{}</text></g>"#,
            lx, color, label
        ));
    }

    svg.push_str("</svg>");
    svg
}

// ─── Core data fetching logic (shared between JSON and SVG handlers) ──────────

async fn fetch_entity_activity_data(
    store: &PgStore,
    entity_id: Uuid,
    days: i64,
) -> Result<ActivityChartResponse, ApiError> {
    let since = Utc::now() - Duration::days(days);

    let obs_data = store
        .get_daily_observation_counts_per_entity(since)
        .await
        .map_err(|e| {
            tracing::error!("Failed to fetch observation counts: {e:#}");
            ApiError::internal("Failed to fetch chart data")
        })?;

    let entity_obs: Vec<(i64, i64)> = obs_data
        .into_iter()
        .filter(|(eid, _, _)| *eid == entity_id)
        .map(|(_, day_offset, cnt)| (day_offset, cnt))
        .collect();

    let ins_data = store
        .get_daily_warning_counts_per_entity(since)
        .await
        .map_err(|e| {
            tracing::error!("Failed to fetch insight counts: {e:#}");
            ApiError::internal("Failed to fetch chart data")
        })?;

    let entity_ins: Vec<(i64, i64)> = ins_data
        .into_iter()
        .filter(|(eid, _, _)| *eid == entity_id)
        .map(|(_, day_offset, cnt)| (day_offset, cnt))
        .collect();

    let day_count = days as usize;
    let mut dates = Vec::with_capacity(day_count);
    let mut observations = vec![0u32; day_count];
    let mut insights = vec![0u32; day_count];
    let mut scores = vec![0.0f64; day_count];

    for i in 0..day_count {
        let d = since + Duration::days(i as i64);
        dates.push(d.format("%Y-%m-%d").to_string());
    }

    for (offset, cnt) in &entity_obs {
        let idx = *offset as usize;
        if idx < day_count {
            observations[idx] = (*cnt).max(0) as u32;
        }
    }

    for (offset, cnt) in &entity_ins {
        let idx = *offset as usize;
        if idx < day_count {
            insights[idx] = (*cnt).max(0) as u32;
        }
    }

    let max_obs = observations.iter().copied().max().unwrap_or(1).max(1);
    let max_ins = insights.iter().copied().max().unwrap_or(1).max(1);
    for i in 0..day_count {
        let obs_norm = observations[i] as f64 / max_obs as f64;
        let ins_norm = insights[i] as f64 / max_ins as f64;
        scores[i] = (obs_norm * 0.6 + ins_norm * 0.4).clamp(0.0, 1.0);
    }

    Ok(ActivityChartResponse {
        dates,
        observations,
        insights,
        scores,
    })
}

// ─── Handler: GET /api/charts/entity/{id}/activity (returns JSON) ─────────────

pub(crate) async fn get_entity_activity_chart(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
    Query(query): Query<ActivityQuery>,
) -> Result<Json<ActivityChartResponse>, ApiError> {
    let _start = Instant::now();
    let days = query.days.unwrap_or(30).clamp(1, 365);

    let response = fetch_entity_activity_data(&state.store, id, days).await?;

    tracing::info!(
        "entity_activity_chart entity_id={} days={} elapsed={}ms",
        id,
        days,
        _start.elapsed().as_millis()
    );

    Ok(Json(response))
}

// ─── Handler: GET /api/charts/entity/{id}/activity/svg (returns SVG) ─────────

pub(crate) async fn get_entity_activity_chart_svg(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
    Query(query): Query<ActivityQuery>,
) -> Result<(StatusCode, HeaderMap, String), ApiError> {
    let days = query.days.unwrap_or(30).clamp(1, 365);

    let data = fetch_entity_activity_data(&state.store, id, days).await?;
    let svg = render_activity_chart_svg(&data, 600, 300);

    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("image/svg+xml"),
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=60"),
    );

    Ok((StatusCode::OK, headers, svg))
}

// ─── Handler: GET /api/charts/entity/{id}/observations?days=90&bucket=week ────

pub(crate) async fn get_entity_observation_chart(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
    Query(query): Query<ObservationChartQuery>,
) -> Result<Json<ObservationChartResponse>, ApiError> {
    let days = query.days.unwrap_or(90).clamp(1, 365);
    let since = Utc::now() - Duration::days(days);
    let bucket = query.bucket.as_deref().unwrap_or("week");

    let obs_data = state
        .store
        .get_daily_observation_counts_per_entity(since)
        .await
        .map_err(|e| {
            tracing::error!("Failed to fetch observation data: {e:#}");
            ApiError::internal("Failed to fetch chart data")
        })?;

    let entity_obs: Vec<(i64, i64)> = obs_data
        .into_iter()
        .filter(|(eid, _, _)| *eid == id)
        .map(|(_, day_offset, cnt)| (day_offset, cnt))
        .collect();

    let bucket_days: i64 = match bucket {
        "day" => 1,
        "week" => 7,
        "month" => 30,
        _ => 7,
    };

    let num_buckets = ((days as f64) / bucket_days as f64).ceil() as usize;
    let mut bucket_counts = vec![0u64; num_buckets];
    let mut bucket_labels = Vec::with_capacity(num_buckets);

    for i in 0..num_buckets {
        let bucket_start = since + Duration::days((i as i64) * bucket_days);
        let label = match bucket {
            "day" => bucket_start.format("%Y-%m-%d").to_string(),
            "week" => bucket_start.format("%Y-W%V").to_string(),
            "month" => bucket_start.format("%Y-%m").to_string(),
            _ => bucket_start.format("%Y-W%V").to_string(),
        };
        bucket_labels.push(label);
    }

    for (offset, cnt) in &entity_obs {
        let bucket_idx = (*offset / bucket_days) as usize;
        if bucket_idx < num_buckets {
            bucket_counts[bucket_idx] = bucket_counts[bucket_idx].saturating_add((*cnt).max(0) as u64);
        }
    }

    let buckets: Vec<ObservationBucket> = bucket_labels
        .into_iter()
        .zip(bucket_counts.into_iter())
        .map(|(label, count)| ObservationBucket { label, count })
        .collect();

    Ok(Json(ObservationChartResponse { buckets }))
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_sparkline_svg_empty() {
        let svg = render_sparkline_svg(&[], 200, 40, "blue");
        assert!(svg.contains("No data"));
    }

    #[test]
    fn test_render_sparkline_svg_single_point() {
        let data = vec![ChartDataPoint {
            date: "2026-01-01".into(),
            value: 42.0,
        }];
        let svg = render_sparkline_svg(&data, 200, 40, "var(--chart-series-blue)");
        assert!(svg.contains("<svg"));
        assert!(svg.contains("42.0"));
        assert!(svg.contains("circle"));
    }

    #[test]
    fn test_render_sparkline_svg_multi() {
        let data = vec![
            ChartDataPoint {
                date: "2026-01-01".into(),
                value: 10.0,
            },
            ChartDataPoint {
                date: "2026-01-02".into(),
                value: 20.0,
            },
            ChartDataPoint {
                date: "2026-01-03".into(),
                value: 15.0,
            },
        ];
        let svg = render_sparkline_svg(&data, 300, 60, "var(--chart-series-blue)");
        assert!(svg.contains("<polyline"));
        assert!(svg.contains("<polygon"));
    }

    #[test]
    fn test_render_activity_chart_svg_empty() {
        let resp = ActivityChartResponse {
            dates: vec![],
            observations: vec![],
            insights: vec![],
            scores: vec![],
        };
        let svg = render_activity_chart_svg(&resp, 600, 300);
        assert!(svg.contains("No activity data"));
    }

    #[test]
    fn test_render_activity_chart_svg_with_data() {
        let resp = ActivityChartResponse {
            dates: vec!["2026-01-01".into(), "2026-01-02".into()],
            observations: vec![5, 10],
            insights: vec![2, 3],
            scores: vec![0.5, 0.8],
        };
        let svg = render_activity_chart_svg(&resp, 600, 300);
        assert!(svg.contains("<svg"));
        assert!(svg.contains("Observations"));
        assert!(svg.contains("Insights"));
        assert!(svg.contains("Activity"));
    }

    #[test]
    fn test_activity_chart_response_serializes() {
        let resp = ActivityChartResponse {
            dates: vec!["2026-01-01".into()],
            observations: vec![5],
            insights: vec![2],
            scores: vec![0.5],
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"observations\""));
        assert!(json.contains("\"insights\""));
    }

    #[test]
    fn test_observation_chart_response_serializes() {
        let resp = ObservationChartResponse {
            buckets: vec![
                ObservationBucket {
                    label: "2026-W01".into(),
                    count: 10,
                },
            ],
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"buckets\""));
    }

    #[test]
    fn test_chart_data_point() {
        let pt = ChartDataPoint {
            date: "2026-01-01".into(),
            value: 3.14,
        };
        assert_eq!(pt.date, "2026-01-01");
        assert!((pt.value - 3.14).abs() < f64::EPSILON);
    }

    #[test]
    fn test_activity_query_default() {
        let query = ActivityQuery { days: None };
        let days = query.days.unwrap_or(30);
        assert_eq!(days, 30);
    }

    #[test]
    fn test_activity_query_clamp() {
        let days = 500i64.clamp(1, 365);
        assert_eq!(days, 365);
    }
}
