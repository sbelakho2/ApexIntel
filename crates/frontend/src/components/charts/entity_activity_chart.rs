use chrono::{DateTime, Utc};
use leptos::*;

/// A data point with utility methods for SVG scaling.
#[derive(Clone, Debug)]
struct MappedPoint {
    x: f64,
    y: f64,
    value: f64,
    date: String,
}

/// Composite chart combining observation counts, insight counts, and activity scores.
///
/// Multi-series line chart with legend, three colored lines, and semantic zoom
/// (last 7, 30, or 90 days). Optional tooltip on hover.
pub struct EntityActivityChart {
    pub observation_counts: Vec<(DateTime<Utc>, u32)>,
    pub insight_counts: Vec<(DateTime<Utc>, u32)>,
    pub activity_scores: Vec<(DateTime<Utc>, f64)>,
    pub width: f64,
    pub height: f64,
    pub days: u32,
}

impl Default for EntityActivityChart {
    fn default() -> Self {
        Self {
            observation_counts: Vec::new(),
            insight_counts: Vec::new(),
            activity_scores: Vec::new(),
            width: 600.0,
            height: 300.0,
            days: 30,
        }
    }
}

impl EntityActivityChart {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_observations(mut self, data: Vec<(DateTime<Utc>, u32)>) -> Self {
        self.observation_counts = data;
        self
    }

    pub fn with_insights(mut self, data: Vec<(DateTime<Utc>, u32)>) -> Self {
        self.insight_counts = data;
        self
    }

    pub fn with_activity_scores(mut self, data: Vec<(DateTime<Utc>, f64)>) -> Self {
        self.activity_scores = data;
        self
    }

    pub fn with_dimensions(mut self, width: f64, height: f64) -> Self {
        self.width = width;
        self.height = height;
        self
    }

    pub fn with_days(mut self, days: u32) -> Self {
        self.days = days;
        self
    }

    /// Construct from a slice of insight records.
    /// This is a stub — real integration would use actual InsightRecord type.
    pub fn from_entity_history(_records: &[(DateTime<Utc>, u32, u32, f64)]) -> Self {
        let mut obs = Vec::new();
        let mut ins = Vec::new();
        let mut scores = Vec::new();
        for (dt, o, i, s) in _records {
            obs.push((*dt, *o));
            ins.push((*dt, *i));
            scores.push((*dt, *s));
        }
        Self {
            observation_counts: obs,
            insight_counts: ins,
            activity_scores: scores,
            width: 600.0,
            height: 300.0,
            days: 30,
        }
    }

    /// Render the composite chart as a Leptos `View`.
    pub fn render(&self) -> View {
        let w = self.width;
        let h = self.height;

        // ── Layout ────────────────────────────────────────────────
        let pad_left = 50.0;
        let pad_right = 16.0;
        let pad_top = 32.0;
        let pad_bottom = 36.0;
        let legend_y = 14.0;
        let plot_w = w - pad_left - pad_right;
        let plot_h = h - pad_top - pad_bottom;

        // ── Series definitions ─────────────────────────────────────
        struct SeriesDef {
            label: &'static str,
            color: &'static str,
            points: Vec<(f64, String)>, // (value, date_label)
        }

        let obs_points: Vec<(f64, String)> = self
            .observation_counts
            .iter()
            .map(|(dt, v)| (*v as f64, dt.format("%Y-%m-%d").to_string()))
            .collect();
        let ins_points: Vec<(f64, String)> = self
            .insight_counts
            .iter()
            .map(|(dt, v)| (*v as f64, dt.format("%Y-%m-%d").to_string()))
            .collect();
        let act_points: Vec<(f64, String)> = self
            .activity_scores
            .iter()
            .map(|(dt, v)| (*v, dt.format("%Y-%m-%d").to_string()))
            .collect();

        let series_list: Vec<SeriesDef> = vec![
            SeriesDef {
                label: "Observations",
                color: "var(--chart-series-blue)",
                points: obs_points,
            },
            SeriesDef {
                label: "Insights",
                color: "var(--chart-series-amber)",
                points: ins_points,
            },
            SeriesDef {
                label: "Activity",
                color: "var(--chart-series-green)",
                points: act_points,
            },
        ];

        // Determine max point count and value range
        let max_points = series_list
            .iter()
            .map(|s| s.points.len())
            .max()
            .unwrap_or(0);

        // ── Empty state ────────────────────────────────────────────
        if max_points == 0 {
            return view! {
                <svg
                    width=w
                    height=h
                    viewBox=format!("0 0 {} {}", w, h)
                    role="img"
                    aria-label="No activity data available"
                    class="entity-activity-chart"
                >
                    <desc>Entity activity chart with no data</desc>
                    <rect
                        x="0" y="0"
                        width=w height=h
                        rx="8"
                        fill="rgba(22,22,22,0.04)"
                        stroke="rgba(22,22,22,0.10)"
                        stroke-width="1"
                    />
                    <text
                        x=w / 2.0
                        y=h / 2.0
                        text-anchor="middle"
                        dominant-baseline="central"
                        fill="var(--muted)"
                        font-size="13"
                        font-weight="600"
                    >"No activity data"</text>
                </svg>
            }
            .into_view();
        }

        // Compute global Y range across all non-empty series
        let all_values: Vec<f64> = series_list
            .iter()
            .flat_map(|s| s.points.iter().map(|(v, _)| *v))
            .collect();
        let max_val = all_values
            .iter()
            .copied()
            .fold(f64::NEG_INFINITY, |a, v| a.max(v))
            .max(1.0);
        let min_val = all_values
            .iter()
            .copied()
            .fold(f64::INFINITY, |a, v| a.min(v));
        let value_range = (max_val - min_val).max(1e-9);

        // ── Scale functions ────────────────────────────────────────
        let scale_y = |v: f64| pad_top + plot_h - ((v - min_val) / value_range) * plot_h;

        // ── Compute polylines ──────────────────────────────────────
        let mut rendered_series = Vec::new();

        for s in &series_list {
            if s.points.is_empty() {
                continue;
            }
            let n = s.points.len();
            let mapped: Vec<MappedPoint> = s
                .points
                .iter()
                .enumerate()
                .map(|(i, (v, date))| {
                    let x = if n <= 1 {
                        pad_left + plot_w / 2.0
                    } else {
                        pad_left + (i as f64 / (n as f64 - 1.0)) * plot_w
                    };
                    let y = scale_y(*v);
                    MappedPoint {
                        x,
                        y,
                        value: *v,
                        date: date.clone(),
                    }
                })
                .collect();

            let polyline_pts = mapped
                .iter()
                .map(|p| format!("{:.1},{:.1}", p.x, p.y))
                .collect::<Vec<_>>()
                .join(" ");

            rendered_series.push((s.label, s.color, mapped, polyline_pts));
        }

        // ── Grid lines (horizontal, 4 levels) ──────────────────────
        let _grid_values = [
            (0.0, "0"),
            (max_val * 0.25, "25%"),
            (max_val * 0.5, "50%"),
            (max_val * 0.75, "75%"),
            (max_val, "100%"),
        ];

        // ── X-axis labels (show ~5 evenly spaced) ──────────────────
        let x_label_count = max_points.clamp(2, 5);

        view! {
            <svg
                width=w
                height=h
                viewBox=format!("0 0 {} {}", w, h)
                role="img"
                aria-label="Entity activity chart — observations, insights, and activity scores over time"
                class="entity-activity-chart"
            >
                <desc>
                    Multi-series line chart showing {
                        rendered_series.iter().map(|(l, _, _, _)| *l).collect::<Vec<_>>().join(", ")
                    } over {max_points} time points
                </desc>

                // Plot background
                <rect
                    x=pad_left
                    y=pad_top
                    width=plot_w
                    height=plot_h
                    rx="8"
                    fill="rgba(255,255,255,0.6)"
                    stroke="rgba(22,22,22,0.10)"
                    stroke-width="1"
                />

                // Horizontal grid lines
                <line
                    x1=pad_left y1=pad_top + plot_h
                    x2=pad_left + plot_w y2=pad_top + plot_h
                    stroke="rgba(22,22,22,0.15)"
                    stroke-width="1"
                />
                <line
                    x1=pad_left y1=scale_y(max_val * 0.75)
                    x2=pad_left + plot_w y2=scale_y(max_val * 0.75)
                    stroke="rgba(22,22,22,0.08)"
                    stroke-width="1"
                    stroke-dasharray="4 4"
                />
                <line
                    x1=pad_left y1=scale_y(max_val * 0.5)
                    x2=pad_left + plot_w y2=scale_y(max_val * 0.5)
                    stroke="rgba(22,22,22,0.08)"
                    stroke-width="1"
                    stroke-dasharray="4 4"
                />
                <line
                    x1=pad_left y1=scale_y(max_val * 0.25)
                    x2=pad_left + plot_w y2=scale_y(max_val * 0.25)
                    stroke="rgba(22,22,22,0.08)"
                    stroke-width="1"
                    stroke-dasharray="4 4"
                />
                <line
                    x1=pad_left y1=pad_top
                    x2=pad_left + plot_w y2=pad_top
                    stroke="rgba(22,22,22,0.08)"
                    stroke-width="1"
                    stroke-dasharray="4 4"
                />

                // Y-axis labels
                <text
                    x=pad_left - 6.0
                    y=pad_top + plot_h
                    text-anchor="end"
                    dominant-baseline="middle"
                    fill="var(--chart-label)"
                    font-size="9"
                    font-weight="600"
                >"0"</text>
                <text
                    x=pad_left - 6.0
                    y=scale_y(max_val * 0.5)
                    text-anchor="end"
                    dominant-baseline="middle"
                    fill="var(--chart-label)"
                    font-size="9"
                    font-weight="600"
                >{format!("{:.0}", max_val * 0.5)}</text>
                <text
                    x=pad_left - 6.0
                    y=scale_y(max_val)
                    text-anchor="end"
                    dominant-baseline="middle"
                    fill="var(--chart-label)"
                    font-size="9"
                    font-weight="600"
                >{format!("{:.0}", max_val)}</text>

                // Series lines
                {rendered_series.iter().map(|(label, color, _mapped, polyline_pts)| {
                    view! {
                        <polyline
                            points=polyline_pts
                            fill="none"
                            stroke=*color
                            stroke-width="2"
                            stroke-linecap="round"
                            stroke-linejoin="round"
                        >
                            <title>{*label}</title>
                        </polyline>
                    }
                }).collect_view()}

                // Data points (circles)
                {rendered_series.iter().flat_map(|(_label, color, mapped, _polyline)| {
                    mapped.iter().map(move |p| {
                        view! {
                            <circle
                                cx=p.x
                                cy=p.y
                                r="3"
                                fill=*color
                                stroke="white"
                                stroke-width="1.5"
                            >
                                <title>{format!("{}: {:.2} ({})", _label, p.value, p.date)}</title>
                            </circle>
                        }
                    }).collect::<Vec<_>>()
                }).collect_view()}

                // X-axis labels (evenly spaced)
                {{
                    let step = if max_points > x_label_count {
                        (max_points - 1) as f64 / (x_label_count - 1) as f64
                    } else {
                        1.0
                    };

                    let first_non_empty = series_list.iter().find(|s| !s.points.is_empty());
                    let indices: Vec<usize> = (0..x_label_count).map(|i| {
                        let idx = (i as f64 * step).round() as usize;
                        idx.min(max_points - 1)
                    }).collect();

                    indices.iter().filter_map(|&i| {
                        let pt = first_non_empty.and_then(|s| s.points.get(i));
                        pt.map(|(_v, date)| {
                            let x = pad_left + (i as f64 / (max_points.max(1) - 1) as f64) * plot_w;
                            view! {
                                <text
                                    x=x
                                    y=h - 8.0
                                    text-anchor="middle"
                                    fill="var(--chart-label)"
                                    font-size="9"
                                    font-weight="600"
                                >{date.split('-').skip(1).collect::<Vec<_>>().join("/")}</text>
                            }
                        })
                    }).collect_view()
                }}

                // Legend
                <g transform=format!("translate({}, {})", pad_left, legend_y)>
                    {rendered_series.iter().enumerate().map(|(i, (label, color, _, _))| {
                        let lx = i as f64 * 140.0;
                        view! {
                            <g transform=format!("translate({}, 0)", lx)>
                                <rect x="0" y="-4" width="10" height="10" rx="2" fill=*color />
                                <text
                                    x="16"
                                    y="3"
                                    dominant-baseline="middle"
                                    fill="var(--foreground)"
                                    font-size="10"
                                    font-weight="700"
                                >{*label}</text>
                            </g>
                        }
                    }).collect_view()}
                </g>
            </svg>
        }
        .into_view()
    }

    /// Quick day-range label.
    pub fn range_label(&self) -> String {
        format!("Last {} days", self.days)
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn make_dates(count: usize, days_apart: i64) -> Vec<DateTime<Utc>> {
        let base = Utc::now() - Duration::days((count as i64) * days_apart);
        (0..count)
            .map(|i| base + Duration::days(i as i64 * days_apart))
            .collect()
    }

    #[test]
    fn test_entity_activity_chart_default() {
        let chart = EntityActivityChart::new();
        assert!(chart.observation_counts.is_empty());
        assert!(chart.insight_counts.is_empty());
        assert!(chart.activity_scores.is_empty());
    }

    #[test]
    fn test_entity_activity_chart_with_data() {
        let dates = make_dates(10, 1);
        let obs: Vec<_> = dates.iter().map(|d| (*d, 5u32)).collect();
        let ins: Vec<_> = dates.iter().map(|d| (*d, 3u32)).collect();
        let scores: Vec<_> = dates.iter().map(|d| (*d, 0.75)).collect();

        let chart = EntityActivityChart::new()
            .with_observations(obs.clone())
            .with_insights(ins.clone())
            .with_activity_scores(scores.clone())
            .with_dimensions(700.0, 350.0)
            .with_days(30);

        assert_eq!(chart.observation_counts.len(), 10);
        assert_eq!(chart.insight_counts.len(), 10);
        assert_eq!(chart.activity_scores.len(), 10);
        assert!((chart.width - 700.0).abs() < f64::EPSILON);
        assert!((chart.height - 350.0).abs() < f64::EPSILON);
        assert_eq!(chart.days, 30);
    }

    #[test]
    fn test_entity_activity_chart_render_empty() {
        let chart = EntityActivityChart::new();
        let _view = chart.render();
        // Should not panic — renders empty state
    }

    #[test]
    fn test_entity_activity_chart_render_single_point() {
        let now = Utc::now();
        let chart = EntityActivityChart::new()
            .with_observations(vec![(now, 5)])
            .with_insights(vec![(now, 2)])
            .with_activity_scores(vec![(now, 0.8)]);
        let _view = chart.render();
        // Should not panic — renders single point
    }

    #[test]
    fn test_entity_activity_chart_render_multi() {
        let dates = make_dates(20, 1);
        let obs: Vec<_> = dates
            .iter()
            .map(|d| (*d, (dates.len() as u32) * 2))
            .collect();
        let chart = EntityActivityChart::new()
            .with_observations(obs)
            .with_insights(vec![])
            .with_activity_scores(vec![]);
        let _view = chart.render();
        // Should not panic — renders with partial data
    }

    #[test]
    fn test_from_entity_history() {
        let now = Utc::now();
        let records = vec![
            (now, 10u32, 3u32, 0.9f64),
            (now - Duration::days(1), 8u32, 2u32, 0.7f64),
        ];
        let chart = EntityActivityChart::from_entity_history(&records);
        assert_eq!(chart.observation_counts.len(), 2);
        assert_eq!(chart.insight_counts.len(), 2);
        assert_eq!(chart.activity_scores.len(), 2);
    }

    #[test]
    fn test_entity_activity_chart_all_zeros() {
        let now = Utc::now();
        let chart = EntityActivityChart::new()
            .with_observations(vec![(now, 0), (now - Duration::days(1), 0)])
            .with_insights(vec![(now, 0)])
            .with_activity_scores(vec![(now, 0.0)]);
        let _view = chart.render();
        // Should not panic — renders all-zero data
    }

    #[test]
    fn test_range_label() {
        let chart = EntityActivityChart::new().with_days(90);
        assert_eq!(chart.range_label(), "Last 90 days");
    }
}
