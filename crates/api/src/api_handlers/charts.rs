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
    pub days: i64,
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

// ─── Axis tick generation ────────────────────────────────────────────────────

/// One Y-axis tick: `value` is in data units and `frac` is its position in
/// [0, 1] on the rendered axis (0 = axis minimum, 1 = axis maximum).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct AxisTick {
    pub value: f64,
    pub frac: f64,
}

/// Round a raw tick interval up to the nearest 1/2/5 × 10^n value.
fn nice_step(raw: f64) -> f64 {
    if !raw.is_finite() || raw <= 0.0 {
        return 1.0;
    }
    let magnitude = 10f64.powf(raw.log10().floor());
    let normalized = raw / magnitude;
    let nice = if normalized <= 1.0 {
        1.0
    } else if normalized <= 2.0 {
        2.0
    } else if normalized <= 5.0 {
        5.0
    } else {
        10.0
    };
    nice * magnitude
}

/// Build truthful axis ticks covering `[min, max]`.
///
/// The first and last ticks are exactly `min` and `max`, so the axis is always
/// labelled with the scale actually rendered: a non-zero minimum is never
/// printed as `0`. Interior ticks use 1/2/5 steps when the range allows.
///
/// * non-finite bounds (e.g. no data) → empty
/// * degenerate range (single point) → one centred tick at that value
pub(crate) fn compute_axis_ticks(min: f64, max: f64, max_ticks: usize) -> Vec<AxisTick> {
    if !min.is_finite() || !max.is_finite() || max_ticks == 0 {
        return Vec::new();
    }
    let (min, max) = if min <= max { (min, max) } else { (max, min) };
    if (max - min).abs() <= f64::EPSILON {
        return vec![AxisTick {
            value: min,
            frac: 0.5,
        }];
    }

    let range = max - min;
    let mut ticks = vec![AxisTick {
        value: min,
        frac: 0.0,
    }];
    if max_ticks > 2 {
        let step = nice_step(range / (max_ticks - 1) as f64);
        let mut candidate = (min / step).ceil() * step;
        while candidate < max - step * 1e-9 && ticks.len() < max_ticks - 1 {
            if candidate > min + step * 1e-9 {
                ticks.push(AxisTick {
                    value: candidate,
                    frac: (candidate - min) / range,
                });
            }
            candidate += step;
        }
    }
    ticks.push(AxisTick {
        value: max,
        frac: 1.0,
    });
    ticks
}

fn format_count_tick(value: f64) -> String {
    format!("{value:.0}")
}

fn format_score_tick(value: f64) -> String {
    if (value - value.round()).abs() < 1e-9 {
        format!("{value:.0}")
    } else {
        format!("{value:.1}")
    }
}

/// X coordinate of series index `i` of `n` evenly spaced points.
fn series_x(i: usize, n: usize, pad_left: f64, plot_w: f64) -> f64 {
    if n <= 1 {
        pad_left + plot_w / 2.0
    } else {
        pad_left + (i as f64 / (n - 1) as f64) * plot_w
    }
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
        return r#"<div class="apex-card p-4 text-center"><p class="text-xs font-semibold text-muted-foreground">No data</p></div>"#
            .to_string();
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
            w,
            h,
            w,
            h,
            cx,
            cy,
            color,
            cx,
            cy - 10.0,
            data[0].value
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

    let Some(last_pt) = data.last() else {
        return String::new();
    };
    let last_px = pad + plot_w;
    let last_py = pad + plot_h - ((last_pt.value - min_val) / spread) * plot_h;

    format!(
        r#"<svg width="{}" height="{}" viewBox="0 0 {} {}" role="img" aria-label="Sparkline with {} points" class="w-full"><desc>Trend sparkline</desc><polygon points="{}" fill="{}" opacity="0.10"/><polyline points="{}" fill="none" stroke="{}" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/><circle cx="{:.1}" cy="{:.1}" r="3" fill="{}" stroke="white" stroke-width="1.5"/></svg>"#,
        w, h, w, h, n, area_pts, color, polyline_pts, color, last_px, last_py, color
    )
}

/// Build a multi-series activity chart SVG as an HTML string.
///
/// Observation and insight counts share the left count axis, whose ticks are
/// derived from the rendered data range (a non-zero minimum is labelled with
/// its real value, never a hard-coded `0`). The 0–1 activity score is plotted on
/// its own right axis so it is not flattened by count magnitudes; the
/// normalization is stated in the SVG description.
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
    let pad_right = 44.0;
    let pad_top = 32.0;
    let pad_bottom = 36.0;
    let plot_w = w - pad_left - pad_right;
    let plot_h = h - pad_top - pad_bottom;

    let obs_f64: Vec<f64> = response.observations.iter().map(|v| *v as f64).collect();
    let ins_f64: Vec<f64> = response.insights.iter().map(|v| *v as f64).collect();
    let score_series: &[f64] = &response.scores;

    // Left (count) axis: derived from the rendered data range.
    let count_min = obs_f64
        .iter()
        .chain(ins_f64.iter())
        .copied()
        .fold(f64::INFINITY, f64::min);
    let count_max = obs_f64
        .iter()
        .chain(ins_f64.iter())
        .copied()
        .fold(f64::NEG_INFINITY, f64::max);
    let count_min = if count_min.is_finite() {
        count_min
    } else {
        0.0
    };
    let count_max = if count_max.is_finite() {
        count_max
    } else {
        0.0
    };
    let (count_axis_min, count_axis_max) = if (count_max - count_min).abs() <= f64::EPSILON {
        // Degenerate range (single point / all-equal counts): pad so the axis
        // and its tick labels still describe a real interval.
        (count_min, count_min + 1.0)
    } else {
        (count_min, count_max)
    };
    let count_range = count_axis_max - count_axis_min;
    let count_ticks = compute_axis_ticks(count_axis_min, count_axis_max, 4);

    // Right (score) axis: the producer already normalizes activity to 0–1.
    let score_ticks = compute_axis_ticks(0.0, 1.0, 3);

    let count_y = |v: f64| pad_top + plot_h - ((v - count_axis_min) / count_range) * plot_h;
    let score_y = |v: f64| pad_top + plot_h - v.clamp(0.0, 1.0) * plot_h;

    let mut svg = String::new();
    svg.push_str(&format!(
        r#"<svg width="{}" height="{}" viewBox="0 0 {} {}" role="img" aria-label="Entity activity chart, last {} days: observations, insights and a 0-1 activity score" data-days="{}" data-point-count="{}" class="w-full">"#,
        w, h, w, h, response.days, response.days, n
    ));
    svg.push_str(&format!(
        "<desc>Multi-series activity chart over {} days. Observation and insight counts share the left count axis; the activity score is normalized per window from those same counts (60% observations, 40% insights, clamped to 0-1) and plotted on its own right axis.</desc>",
        response.days
    ));

    // Plot background
    svg.push_str(&format!(
        r#"<rect x="{:.1}" y="{:.1}" width="{:.1}" height="{:.1}" rx="8" fill="rgba(255,255,255,0.6)" stroke="rgba(22,22,22,0.10)" stroke-width="1"/>"#,
        pad_left, pad_top, plot_w, plot_h
    ));

    // Grid lines and left count labels derived from the real scale.
    for tick in &count_ticks {
        let y = pad_top + plot_h * (1.0 - tick.frac);
        let dash = if tick.frac <= 0.0 {
            ""
        } else {
            r#" stroke-dasharray="4 4""#
        };
        svg.push_str(&format!(
            r#"<line x1="{:.1}" y1="{:.1}" x2="{:.1}" y2="{:.1}" stroke="rgba(22,22,22,0.12)" stroke-width="1"{}"/>"#,
            pad_left, y, pad_left + plot_w, y, dash
        ));
        svg.push_str(&format!(
            r#"<text class="apex-tick-counts" data-tick-value="{}" x="{:.1}" y="{:.1}" text-anchor="end" dominant-baseline="middle" font-size="9" font-weight="600" fill="var(--chart-label)">{}</text>"#,
            format_count_tick(tick.value),
            pad_left - 6.0,
            y,
            format_count_tick(tick.value)
        ));
    }

    // Right score-axis labels (0–1).
    for tick in &score_ticks {
        let y = pad_top + plot_h * (1.0 - tick.frac);
        svg.push_str(&format!(
            r#"<text class="apex-tick-score" data-tick-value="{}" x="{:.1}" y="{:.1}" text-anchor="start" dominant-baseline="middle" font-size="9" font-weight="600" fill="var(--chart-label)">{}</text>"#,
            format_score_tick(tick.value),
            pad_left + plot_w + 6.0,
            y,
            format_score_tick(tick.value)
        ));
    }

    // Rendered window label, inside the chart, so the active window is always
    // visible no matter which chip loaded the fragment.
    svg.push_str(&format!(
        r#"<text x="{:.1}" y="14" text-anchor="end" font-size="9" font-weight="700" fill="var(--chart-label)">Last {} days</text>"#,
        pad_left + plot_w + 38.0,
        response.days
    ));

    // Helper to build a polyline on the given axis scale.
    let build_polyline = |values: &[f64],
                          color: &str,
                          class: &str,
                          axis: &str,
                          map: &dyn Fn(f64) -> f64|
     -> String {
        let mut pts = String::new();
        for (i, v) in values.iter().enumerate() {
            let x = series_x(i, n, pad_left, plot_w);
            let y = map(*v);
            if i > 0 {
                pts.push(' ');
            }
            pts.push_str(&format!("{:.1},{:.1}", x, y));
        }
        format!(
            r#"<polyline class="{}" data-axis="{}" points="{}" fill="none" stroke="{}" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>"#,
            class, axis, pts, color
        )
    };

    // Observations + insights share the left count axis.
    svg.push_str(&build_polyline(
        &obs_f64,
        "var(--chart-series-blue)",
        "apex-activity-series apex-activity-observations",
        "counts-left",
        &count_y,
    ));
    svg.push_str(&build_polyline(
        &ins_f64,
        "var(--chart-series-amber)",
        "apex-activity-series apex-activity-insights",
        "counts-left",
        &count_y,
    ));

    // Activity scores live on their own 0–1 right axis.
    svg.push_str(&build_polyline(
        score_series,
        "var(--chart-series-green)",
        "apex-activity-series apex-activity-score",
        "score-0-1-right",
        &score_y,
    ));

    // Data points, each series on its own axis scale.
    struct SeriesRender<'a> {
        values: &'a [f64],
        color: &'a str,
        class: &'a str,
        map: &'a dyn Fn(f64) -> f64,
    }
    let series_points = [
        SeriesRender {
            values: &obs_f64,
            color: "var(--chart-series-blue)",
            class: "apex-activity-observations",
            map: &count_y,
        },
        SeriesRender {
            values: &ins_f64,
            color: "var(--chart-series-amber)",
            class: "apex-activity-insights",
            map: &count_y,
        },
        SeriesRender {
            values: score_series,
            color: "var(--chart-series-green)",
            class: "apex-activity-score",
            map: &score_y,
        },
    ];
    for series in series_points {
        for (i, v) in series.values.iter().enumerate() {
            let x = series_x(i, n, pad_left, plot_w);
            let y = (series.map)(*v);
            svg.push_str(&format!(
                r#"<circle class="{}" cx="{:.1}" cy="{:.1}" r="2.5" fill="{}" stroke="white" stroke-width="1.5"/>"#,
                series.class, x, y, series.color
            ));
        }
    }

    // Legend
    let legend_items = [
        ("var(--chart-series-blue)", "Observations"),
        ("var(--chart-series-amber)", "Insights"),
        ("var(--chart-series-green)", "Activity (right, 0-1)"),
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
        .get_daily_insight_counts_per_entity(since)
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

    Ok(build_activity_series(since, days, &entity_obs, &entity_ins))
}

/// Assemble the per-day activity series for a `days`-long window starting at
/// `since`.
///
/// Rows are `(day_offset, count)` pairs as returned by the store; offsets
/// outside `[0, days)` are ignored, so a wider window really can only add older
/// days — it never re-scales or drops the days a smaller window would show.
fn build_activity_series(
    since: DateTime<Utc>,
    days: i64,
    entity_obs: &[(i64, i64)],
    entity_ins: &[(i64, i64)],
) -> ActivityChartResponse {
    let day_count = days as usize;
    let mut dates = Vec::with_capacity(day_count);
    let mut observations = vec![0u32; day_count];
    let mut insights = vec![0u32; day_count];
    let mut scores = vec![0.0f64; day_count];

    for i in 0..day_count {
        let d = since + Duration::days(i as i64);
        dates.push(d.format("%Y-%m-%d").to_string());
    }

    for (offset, cnt) in entity_obs {
        if let Ok(idx) = usize::try_from(*offset) {
            if idx < day_count {
                observations[idx] = (*cnt).max(0) as u32;
            }
        }
    }

    for (offset, cnt) in entity_ins {
        if let Ok(idx) = usize::try_from(*offset) {
            if idx < day_count {
                insights[idx] = (*cnt).max(0) as u32;
            }
        }
    }

    let max_obs = observations.iter().copied().max().unwrap_or(1).max(1);
    let max_ins = insights.iter().copied().max().unwrap_or(1).max(1);
    for i in 0..day_count {
        let obs_norm = observations[i] as f64 / max_obs as f64;
        let ins_norm = insights[i] as f64 / max_ins as f64;
        scores[i] = (obs_norm * 0.6 + ins_norm * 0.4).clamp(0.0, 1.0);
    }

    ActivityChartResponse {
        days,
        dates,
        observations,
        insights,
        scores,
    }
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
    headers: HeaderMap,
) -> Result<(StatusCode, HeaderMap, String), ApiError> {
    let days = query.days.unwrap_or(30).clamp(1, 365);

    let data = fetch_entity_activity_data(&state.store, id, days).await?;
    let svg = render_activity_chart_svg(&data, 600, 300);

    let mut out_headers = HeaderMap::new();
    let body = if apex_api::web::is_htmx_request(&headers) {
        // HTMX fragment response: update the card's window label out-of-band so
        // the selector's state is always visible, then the chart itself.
        out_headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/html; charset=utf-8"),
        );
        format!(
            r#"<span id="entity-activity-window-{id}" hx-swap-oob="innerHTML">Activity — Last {days} Days</span>{svg}"#
        )
    } else {
        // Direct fetch stays a standalone SVG image.
        out_headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("image/svg+xml"),
        );
        svg
    };
    out_headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=60"),
    );

    Ok((StatusCode::OK, out_headers, body))
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
            bucket_counts[bucket_idx] =
                bucket_counts[bucket_idx].saturating_add((*cnt).max(0) as u64);
        }
    }

    let buckets: Vec<ObservationBucket> = bucket_labels
        .into_iter()
        .zip(bucket_counts)
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

    fn response_with(
        days: i64,
        observations: Vec<u32>,
        insights: Vec<u32>,
        scores: Vec<f64>,
    ) -> ActivityChartResponse {
        let n = observations.len();
        ActivityChartResponse {
            days,
            dates: (0..n).map(|i| format!("2026-01-{:02}", i + 1)).collect(),
            observations,
            insights,
            scores,
        }
    }

    fn polyline_points(svg: &str, class: &str) -> Vec<(f64, f64)> {
        let marker = format!(r#"class="{class}""#);
        let start = svg
            .find(&marker)
            .unwrap_or_else(|| panic!("missing polyline {class}"));
        let seg = &svg[start..];
        let pts_start = seg.find("points=\"").expect("points attr") + "points=\"".len();
        let pts_end = pts_start + seg[pts_start..].find('"').expect("points close");
        seg[pts_start..pts_end]
            .split_whitespace()
            .map(|pair| {
                let (x, y) = pair.split_once(',').expect("x,y pair");
                (x.parse::<f64>().expect("x"), y.parse::<f64>().expect("y"))
            })
            .collect()
    }

    #[test]
    fn test_compute_axis_ticks_non_zero_minimum() {
        let ticks = compute_axis_ticks(5.0, 20.0, 4);
        assert!(!ticks.is_empty());
        assert_eq!(ticks.first().map(|t| t.value), Some(5.0));
        assert_eq!(ticks.last().map(|t| t.value), Some(20.0));
        assert!(ticks.iter().all(|t| t.value >= 5.0 && t.value <= 20.0));
        assert!(ticks.iter().all(|t| t.value.abs() > f64::EPSILON));
        assert!(ticks.windows(2).all(|w| w[0].frac < w[1].frac));
    }

    #[test]
    fn test_compute_axis_ticks_empty_data() {
        assert!(compute_axis_ticks(f64::INFINITY, f64::NEG_INFINITY, 4).is_empty());
        assert!(compute_axis_ticks(f64::NAN, f64::NAN, 4).is_empty());
        assert!(compute_axis_ticks(0.0, 10.0, 0).is_empty());
    }

    #[test]
    fn test_compute_axis_ticks_single_point() {
        let ticks = compute_axis_ticks(7.0, 7.0, 4);
        assert_eq!(ticks.len(), 1);
        assert_eq!(ticks[0].value, 7.0);
        assert_eq!(ticks[0].frac, 0.5);
    }

    #[test]
    fn test_render_activity_chart_svg_empty() {
        let resp = response_with(30, vec![], vec![], vec![]);
        let svg = render_activity_chart_svg(&resp, 600, 300);
        assert!(svg.contains("No activity data"));
        assert!(!svg.contains("NaN"));
    }

    #[test]
    fn test_render_activity_chart_svg_single_point() {
        let resp = response_with(1, vec![5], vec![2], vec![0.8]);
        let svg = render_activity_chart_svg(&resp, 600, 300);
        assert!(svg.contains("<svg"));
        assert!(svg.contains(r#"data-days="1""#));
        assert!(!svg.contains("NaN"));
        assert_eq!(svg.matches("<circle").count(), 3);
    }

    #[test]
    fn test_render_activity_chart_svg_with_data() {
        let resp = response_with(2, vec![5, 10], vec![2, 3], vec![0.5, 0.8]);
        let svg = render_activity_chart_svg(&resp, 600, 300);
        assert!(svg.contains("<svg"));
        assert!(svg.contains("Observations"));
        assert!(svg.contains("Insights"));
        assert!(svg.contains("Activity (right, 0-1)"));
        assert!(svg.contains(r#"data-axis="score-0-1-right""#));
        assert!(svg.contains(r#"data-axis="counts-left""#));
    }

    #[test]
    fn test_activity_score_not_flattened_by_count_magnitude() {
        let resp = response_with(2, vec![1000, 1000], vec![1000, 1000], vec![0.0, 1.0]);
        let svg = render_activity_chart_svg(&resp, 600, 300);

        // Counts that dwarf the score stay on the left axis...
        let counts = polyline_points(&svg, "apex-activity-series apex-activity-observations");
        assert_eq!(counts.len(), 2);
        assert!((counts[0].1 - counts[1].1).abs() < 0.6);

        // ...while the 0–1 score still spans the full plot height instead of
        // collapsing onto the count baseline.
        let score = polyline_points(&svg, "apex-activity-series apex-activity-score");
        assert_eq!(score.len(), 2);
        assert!(
            (score[0].1 - 264.0).abs() < 0.6,
            "score 0.0 y = {}",
            score[0].1
        );
        assert!(
            (score[1].1 - 32.0).abs() < 0.6,
            "score 1.0 y = {}",
            score[1].1
        );
    }

    #[test]
    fn test_count_axis_ticks_never_label_non_zero_minimum_as_zero() {
        let resp = response_with(2, vec![50, 90], vec![40, 60], vec![0.5, 1.0]);
        let svg = render_activity_chart_svg(&resp, 600, 300);
        assert!(svg.contains(r#"class="apex-tick-counts" data-tick-value="40""#));
        assert!(!svg.contains(r#"class="apex-tick-counts" data-tick-value="0""#));
        assert!(svg.contains(r#"class="apex-tick-counts" data-tick-value="90""#));
    }

    #[test]
    fn test_days_window_filters_series() {
        let now = Utc::now();
        let since = now - Duration::days(90);
        let obs = vec![(5_i64, 3_i64), (50_i64, 9_i64)];
        let ins = vec![(5_i64, 1_i64)];

        let week = build_activity_series(since, 7, &obs, &ins);
        assert_eq!(week.days, 7);
        assert_eq!(week.observations.len(), 7);
        assert_eq!(week.observations[5], 3);
        assert_eq!(week.observations.iter().copied().max(), Some(3));
        assert_eq!(week.insights[5], 1);

        let quarter = build_activity_series(since, 90, &obs, &ins);
        assert_eq!(quarter.days, 90);
        assert_eq!(quarter.observations.len(), 90);
        assert_eq!(quarter.observations[5], 3);
        assert_eq!(quarter.observations[50], 9);
        assert_eq!(quarter.insights[5], 1);
    }

    #[test]
    fn test_rendered_series_changes_with_days_window() {
        let now = Utc::now();
        let obs = vec![(5_i64, 3_i64), (50_i64, 9_i64)];
        let week = build_activity_series(now - Duration::days(7), 7, &obs, &[]);
        let quarter = build_activity_series(now - Duration::days(90), 90, &obs, &[]);

        let week_svg = render_activity_chart_svg(&week, 600, 300);
        let quarter_svg = render_activity_chart_svg(&quarter, 600, 300);

        assert!(week_svg.contains(r#"data-days="7""#));
        assert!(quarter_svg.contains(r#"data-days="90""#));
        assert_eq!(week_svg.matches("<circle").count(), 3 * 7);
        assert_eq!(quarter_svg.matches("<circle").count(), 3 * 90);
        assert_ne!(week_svg, quarter_svg);
    }

    #[test]
    fn test_activity_chart_response_serializes() {
        let resp = response_with(1, vec![5], vec![2], vec![0.5]);
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"days\":1"));
        assert!(json.contains("\"observations\""));
        assert!(json.contains("\"insights\""));
    }

    #[test]
    fn test_observation_chart_response_serializes() {
        let resp = ObservationChartResponse {
            buckets: vec![ObservationBucket {
                label: "2026-W01".into(),
                count: 10,
            }],
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"buckets\""));
    }

    #[test]
    fn test_chart_data_point() {
        let pt = ChartDataPoint {
            date: "2026-01-01".into(),
            value: 2.71,
        };
        assert_eq!(pt.date, "2026-01-01");
        assert!((pt.value - 2.71).abs() < f64::EPSILON);
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
