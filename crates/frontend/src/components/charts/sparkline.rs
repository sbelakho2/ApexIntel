use chrono::{DateTime, Utc};
use leptos::*;

/// A minimal time-series sparkline chart.
///
/// Renders an inline SVG with a polyline for the data series and an optional
/// filled area beneath. No axes or labels — just the line in a compact space.
pub struct SparklineChart {
    pub data: Vec<(DateTime<Utc>, f64)>,
    pub width: f64,
    pub height: f64,
    pub color: &'static str,
    pub show_area: bool,
}

impl Default for SparklineChart {
    fn default() -> Self {
        Self {
            data: Vec::new(),
            width: 200.0,
            height: 40.0,
            color: "var(--chart-series-blue)",
            show_area: false,
        }
    }
}

impl SparklineChart {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_data(mut self, data: Vec<(DateTime<Utc>, f64)>) -> Self {
        self.data = data;
        self
    }

    pub fn with_dimensions(mut self, width: f64, height: f64) -> Self {
        self.width = width;
        self.height = height;
        self
    }

    pub fn with_color(mut self, color: &'static str) -> Self {
        self.color = color;
        self
    }

    pub fn with_area(mut self, show_area: bool) -> Self {
        self.show_area = show_area;
        self
    }

    /// Construct from a slice of (DateTime, f64) activity scores.
    pub fn from_activity_scores(scores: &[(DateTime<Utc>, f64)]) -> Self {
        Self {
            data: scores.to_vec(),
            width: 200.0,
            height: 40.0,
            color: "var(--chart-series-amber)",
            show_area: true,
        }
    }

    /// Render the sparkline as a Leptos `View`.
    pub fn render(&self) -> View {
        let data = &self.data;
        let w = self.width;
        let h = self.height;
        let color = self.color;
        let show_area = self.show_area;

        // Handle empty data
        if data.is_empty() {
            return view! {
                <svg
                    width=w
                    height=h
                    viewBox=format!("0 0 {} {}", w, h)
                    role="img"
                    aria-label="No data available"
                    class="sparkline"
                >
                    <desc>Sparkline chart with no data points</desc>
                    <rect
                        x="0" y="0"
                        width=w height=h
                        rx="4"
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
                        font-size="10"
                        font-weight="600"
                    >"No data"</text>
                </svg>
            }
            .into_view();
        }

        // Compute min/max for scaling
        let min_val = data
            .iter()
            .map(|d| d.1)
            .fold(f64::INFINITY, |a, v| a.min(v));
        let max_val = data
            .iter()
            .map(|d| d.1)
            .fold(f64::NEG_INFINITY, |a, v| a.max(v));
        let spread = (max_val - min_val).max(1e-9);

        // Padding inside SVG
        let pad_left = 2.0;
        let pad_right = 2.0;
        let pad_top = 2.0;
        let pad_bottom = 2.0;
        let plot_w = w - pad_left - pad_right;
        let plot_h = h - pad_top - pad_bottom;

        let n = data.len() as f64;
        let points: Vec<(f64, f64)> = data
            .iter()
            .enumerate()
            .map(|(i, (_dt, val))| {
                let x = if n <= 1.0 {
                    w / 2.0
                } else {
                    pad_left + (i as f64 / (n - 1.0)) * plot_w
                };
                let y = pad_top + plot_h - ((val - min_val) / spread) * plot_h;
                (x, y)
            })
            .collect();

        let polyline_pts = points
            .iter()
            .map(|(x, y)| format!("{:.1},{:.1}", x, y))
            .collect::<Vec<_>>()
            .join(" ");

        let last = points.last().copied();

        // Handle single data point — render as a dot
        if data.len() == 1 {
            let (cx, cy) = points[0];
            return view! {
                <svg
                    width=w
                    height=h
                    viewBox=format!("0 0 {} {}", w, h)
                    role="img"
                    aria-label="Sparkline with 1 data point"
                    class="sparkline"
                >
                    <desc>Sparkline chart showing a single data point</desc>
                    <title>Value: {format!("{:.2}", data[0].1)}</title>
                    <circle cx=cx cy=cy r="4" fill=color stroke="white" stroke-width="1.5" />
                    <text
                        x=cx
                        y=cy - 8.0
                        text-anchor="middle"
                        fill="var(--foreground)"
                        font-size="9"
                        font-weight="700"
                    >{format!("{:.1}", data[0].1)}</text>
                </svg>
            }
            .into_view();
        }

        // Area path
        let area_d = if show_area && points.len() >= 2 {
            let first_x = points.first().map(|p| p.0).unwrap_or(0.0);
            let last_x = points.last().map(|p| p.0).unwrap_or(0.0);
            let bottom_y = pad_top + plot_h;
            let mut area = String::new();
            area.push_str(&format!("M {:.1} {:.1}", first_x, bottom_y));
            for (x, y) in &points {
                area.push_str(&format!(" L {:.1} {:.1}", x, y));
            }
            area.push_str(&format!(" L {:.1} {:.1} Z", last_x, bottom_y));
            area
        } else {
            String::new()
        };

        view! {
            <svg
                width=w
                height=h
                viewBox=format!("0 0 {} {}", w, h)
                role="img"
                aria-label="Sparkline chart"
                class="sparkline"
            >
                <desc>Time-series sparkline with {data.len()} data points</desc>
                <title>
                    {format!(
                        "Range: {:.2} – {:.2}, Latest: {:.2}",
                        min_val,
                        max_val,
                        data.last().map(|d| d.1).unwrap_or(0.0)
                    )}
                </title>
                {if show_area && !area_d.is_empty() {
                    view! { <path d=area_d fill=color opacity="0.10" stroke="none" /> }.into_view()
                } else {
                    View::default()
                }}
                <polyline
                    points=polyline_pts
                    fill="none"
                    stroke=color
                    stroke-width="2"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                />
                {last.map(|(lx, ly)| {
                    view! {
                        <circle cx=lx cy=ly r="3" fill=color stroke="white" stroke-width="1.5" />
                    }
                })}
            </svg>
        }
        .into_view()
    }
}

// ─── Existing Leptos component — kept for backward compatibility ─────────────

#[component]
pub fn Sparkline(values: Vec<f64>, #[prop(optional)] label: Option<String>) -> impl IntoView {
    let values = if values.is_empty() { vec![0.0] } else { values };
    let min = values
        .iter()
        .copied()
        .fold(f64::INFINITY, |acc, value| acc.min(value));
    let max = values
        .iter()
        .copied()
        .fold(f64::NEG_INFINITY, |acc, value| acc.max(value));
    let spread = (max - min).max(1e-9);
    let path = values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let x = if values.len() <= 1 {
                44.0
            } else {
                4.0 + index as f64 * (80.0 / (values.len() as f64 - 1.0))
            };
            let y = 26.0 - (((value - min) / spread) * 22.0);
            if index == 0 {
                format!("M {x:.1} {y:.1}")
            } else {
                format!("L {x:.1} {y:.1}")
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    let points = values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let x = if values.len() <= 1 {
                44.0
            } else {
                4.0 + index as f64 * (80.0 / (values.len() as f64 - 1.0))
            };
            let y = 26.0 - (((value - min) / spread) * 22.0);
            (x, y, *value)
        })
        .collect::<Vec<_>>();
    let first = points.first().cloned();
    let last = points.last().cloned();
    let last_value = last.map(|(_, _, value)| value).unwrap_or(0.0);

    view! {
        <svg
            viewBox="0 0 88 30"
            class="sparkline"
            role="img"
            aria-label=label.unwrap_or_else(|| "sparkline".to_string())
        >
            <desc>Sparkline mini chart</desc>
            <rect x="2" y="3" width="84" height="24" rx="8" class="chart-plot-surface" />
            <line x1="4" y1="26" x2="84" y2="26" class="chart-grid-line" />
            <line x1="4" y1="14" x2="84" y2="14" class="chart-grid-line" />
            <path
                d=format!("{} L 84 28 L 4 28 Z", path.clone())
                fill="rgba(74, 144, 226, 0.14)"
                stroke="none"
            />
            <path d=path class="chart-series-line chart-series-line-blue" />
            {first.map(|point| {
                view! { <circle cx=point.0 cy=point.1 r="2.5" fill="var(--accent)" /> }
            })}
            {last.map(|point| {
                view! {
                    <g>
                        <circle
                            cx=point.0
                            cy=point.1
                            r="3.2"
                            class="chart-series-point"
                            fill="var(--foreground)"
                        />
                        <text
                            x=point.0
                            y=format!("{:.1}", point.1 - 5.0)
                            text-anchor="middle"
                            class="chart-value-label"
                        >{format!("{:.1}", last_value)}</text>
                    </g>
                }
            })}
        </svg>
    }
}
