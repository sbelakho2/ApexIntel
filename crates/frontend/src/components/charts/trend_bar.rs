use leptos::*;

/// A single bar series in a trend bar chart.
pub struct BarSeries {
    pub label: String,
    pub values: Vec<f64>,
    pub color: &'static str,
}

impl BarSeries {
    pub fn new(label: impl Into<String>, values: Vec<f64>, color: &'static str) -> Self {
        Self {
            label: label.into(),
            values,
            color,
        }
    }
}

/// A rendered bar rectangle: `(x, y, width, height, fill color, value)`.
type BarRect = (f64, f64, f64, f64, &'static str, f64);

/// A single x-axis group of rendered bars.
type BarGroup = Vec<BarRect>;

/// Simple bar chart for weekly/monthly counts.
///
/// Renders SVG rect elements for each bar with optional Y-axis labels
/// on the left and X-axis labels below. Minimal horizontal grid lines.
pub struct TrendBarChart {
    pub bars: Vec<BarSeries>,
    pub width: f64,
    pub height: f64,
    pub show_labels: bool,
}

impl Default for TrendBarChart {
    fn default() -> Self {
        Self {
            bars: Vec::new(),
            width: 400.0,
            height: 200.0,
            show_labels: true,
        }
    }
}

impl TrendBarChart {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_bars(mut self, bars: Vec<BarSeries>) -> Self {
        self.bars = bars;
        self
    }

    pub fn with_dimensions(mut self, width: f64, height: f64) -> Self {
        self.width = width;
        self.height = height;
        self
    }

    pub fn with_labels(mut self, show_labels: bool) -> Self {
        self.show_labels = show_labels;
        self
    }

    pub fn add_series(mut self, series: BarSeries) -> Self {
        self.bars.push(series);
        self
    }

    /// Render the bar chart as a Leptos `View`.
    pub fn render(&self) -> View {
        let w = self.width;
        let h = self.height;
        let show_labels = self.show_labels;

        // ── Layout constants ─────────────────────────────────────────
        let pad_left = if show_labels { 40.0 } else { 8.0 };
        let pad_right = 8.0;
        let pad_top = 8.0;
        let pad_bottom = if show_labels { 32.0 } else { 8.0 };
        let plot_w = w - pad_left - pad_right;
        let plot_h = h - pad_top - pad_bottom;

        let n_series = self.bars.len();
        let n_groups = self.bars.first().map(|b| b.values.len()).unwrap_or(0);

        // ── Handle empty data ────────────────────────────────────────
        if n_groups == 0 || n_series == 0 {
            return view! {
                <svg
                    width=w
                    height=h
                    viewBox=format!("0 0 {} {}", w, h)
                    role="img"
                    aria-label="No data available"
                    class="trend-bar-chart"
                >
                    <desc>Bar chart with no data</desc>
                    <rect
                        x="0" y="0"
                        width=w height=h
                        rx="6"
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
                        font-size="12"
                        font-weight="600"
                    >"No data"</text>
                </svg>
            }
            .into_view();
        }

        // ── Compute value range for scaling ──────────────────────────
        let all_values: Vec<f64> = self
            .bars
            .iter()
            .flat_map(|b| b.values.iter().copied())
            .collect();
        let max_val = all_values
            .iter()
            .copied()
            .fold(f64::NEG_INFINITY, |a, v| a.max(v))
            .max(1.0);

        // ── Grid lines (horizontal, 3 levels) ────────────────────────
        let _grid_lines = [
            (0.0, "0"),
            (max_val * 0.5, format!("{:.0}", max_val * 0.5).leak()),
            (max_val, format!("{:.0}", max_val).leak()),
        ];

        let series_count = n_series;
        let group_width = plot_w / n_groups as f64;
        let bar_gap = 2.0;
        let bar_group_width = group_width - 4.0;
        let bar_width =
            (bar_group_width - bar_gap * (series_count as f64 - 1.0)) / series_count as f64;
        let bar_width = bar_width.max(2.0);

        // ── Single data point special case ──────────────────────────
        if n_groups == 1 {
            let group_x = pad_left + 0.0;
            let mut rects = Vec::new();
            for (si, series) in self.bars.iter().enumerate() {
                let val = series.values.first().copied().unwrap_or(0.0);
                let bar_h = (val / max_val) * plot_h;
                let x = group_x + si as f64 * (bar_width + bar_gap);
                let y = pad_top + plot_h - bar_h;
                rects.push((
                    x,
                    y,
                    bar_width,
                    bar_h,
                    series.color,
                    val,
                    series.label.clone(),
                ));
            }

            return view! {
                <svg
                    width=w
                    height=h
                    viewBox=format!("0 0 {} {}", w, h)
                    role="img"
                    aria-label="Bar chart with 1 data group"
                    class="trend-bar-chart"
                >
                    <desc>Bar chart showing a single data group</desc>
                    <rect
                        x=pad_left
                        y=pad_top
                        width=plot_w
                        height=plot_h
                        rx="6"
                        fill="rgba(255,255,255,0.6)"
                        stroke="rgba(22,22,22,0.10)"
                        stroke-width="1"
                    />
                    // Grid lines
                    <line
                        x1=pad_left y1=pad_top + plot_h
                        x2=pad_left + plot_w y2=pad_top + plot_h
                        stroke="rgba(22,22,22,0.15)"
                        stroke-width="1"
                    />
                    <line
                        x1=pad_left y1=pad_top + plot_h * 0.5
                        x2=pad_left + plot_w y2=pad_top + plot_h * 0.5
                        stroke="rgba(22,22,22,0.08)"
                        stroke-width="1"
                        stroke-dasharray="4 4"
                    />
                    {rects.iter().map(|(x, y, bw, bh, color, val, _label)| {
                        view! {
                            <rect
                                x=*x y=*y
                                width=*bw height=*bh
                                fill=*color
                                rx="2"
                            >
                                <title>{format!("Value: {:.1}", val)}</title>
                            </rect>
                        }
                    }).collect_view()}
                    {if show_labels {
                        rects.first().map(|(_, _, _, _, _, _val, label)| {
                            view! {
                                <text
                                    x=pad_left + plot_w / 2.0
                                    y=h - 6.0
                                    text-anchor="middle"
                                    fill="var(--chart-label)"
                                    font-size="10"
                                    font-weight="600"
                                >{label.clone()}</text>
                            }
                        })
                    } else {
                        None
                    }}
                </svg>
            }
            .into_view();
        }

        // ── Multi-group bar chart ─────────────────────────────────────
        let mut groups: Vec<BarGroup> = Vec::new();

        for gi in 0..n_groups {
            let group_x = pad_left + gi as f64 * group_width + 2.0;
            let mut bars_in_group = Vec::new();
            for (si, series) in self.bars.iter().enumerate() {
                let val = series.values.get(gi).copied().unwrap_or(0.0);
                let bar_h = (val / max_val) * plot_h;
                let x = group_x + si as f64 * (bar_width + bar_gap);
                let y = pad_top + plot_h - bar_h;
                bars_in_group.push((x, y, bar_width, bar_h, series.color, val));
            }
            groups.push(bars_in_group);
        }

        view! {
            <svg
                width=w
                height=h
                viewBox=format!("0 0 {} {}", w, h)
                role="img"
                aria-label="Trend bar chart"
                class="trend-bar-chart"
            >
                <desc>Bar chart with {n_groups} groups and {n_series} series</desc>
                // Plot background
                <rect
                    x=pad_left
                    y=pad_top
                    width=plot_w
                    height=plot_h
                    rx="6"
                    fill="rgba(255,255,255,0.6)"
                    stroke="rgba(22,22,22,0.10)"
                    stroke-width="1"
                />
                // Grid lines
                <line
                    x1=pad_left y1=pad_top + plot_h
                    x2=pad_left + plot_w y2=pad_top + plot_h
                    stroke="rgba(22,22,22,0.15)"
                    stroke-width="1"
                />
                <line
                    x1=pad_left y1=pad_top + plot_h * 0.5
                    x2=pad_left + plot_w y2=pad_top + plot_h * 0.5
                    stroke="rgba(22,22,22,0.08)"
                    stroke-width="1"
                    stroke-dasharray="4 4"
                />
                // Y-axis labels
                {if show_labels {
                    view! {
                        <>
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
                                y=pad_top + plot_h * 0.5
                                text-anchor="end"
                                dominant-baseline="middle"
                                fill="var(--chart-label)"
                                font-size="9"
                                font-weight="600"
                            >{format!("{:.0}", max_val * 0.5)}</text>
                            <text
                                x=pad_left - 6.0
                                y=pad_top + 2.0
                                text-anchor="end"
                                dominant-baseline="hanging"
                                fill="var(--chart-label)"
                                font-size="9"
                                font-weight="600"
                            >{format!("{:.0}", max_val)}</text>
                        </>
                    }.into_view()
                } else {
                    View::default()
                }}
                // Bars
                {groups.iter().flat_map(|bars_in_group| {
                    bars_in_group.iter().map(move |(x, y, bw, bh, color, val)| {
                        view! {
                            <rect
                                x=*x y=*y
                                width=*bw height=*bh
                                fill=*color
                                rx="2"
                            >
                                <title>{format!("Value: {:.1}", val)}</title>
                            </rect>
                        }
                    }).collect::<Vec<_>>()
                }).collect_view()}
                // X-axis labels
                {if show_labels {
                    let empty: Vec<f64> = Vec::new();
                    let first_series = self.bars.first().map(|b| &b.values).unwrap_or(&empty);
                    let _labels: Vec<&str> = first_series.iter().map(|_| {
                        self.bars.first().map(|b| b.label.as_str()).unwrap_or("")
                    }).collect();
                    // Use generic x-group labels if we don't have meaningful labels per bar
                    // Each group gets the label from the first series
                    let group_indices: Vec<usize> = (0..n_groups).collect();
                    group_indices.iter().map(|&gi| {
                        let cx = pad_left + gi as f64 * group_width + group_width / 2.0;
                        let cy = h - 8.0;
                        view! {
                            <text
                                x=cx
                                y=cy
                                text-anchor="end"
                                transform=format!("rotate(-45 {} {})", cx, cy)
                                fill="var(--chart-label)"
                                font-size="9"
                                font-weight="600"
                            >{gi + 1}</text>
                        }
                    }).collect_view()
                } else {
                    View::default()
                }}
            </svg>
        }
        .into_view()
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trend_bar_chart_default() {
        let chart = TrendBarChart::new();
        assert!(chart.bars.is_empty());
        assert!((chart.width - 400.0).abs() < f64::EPSILON);
        assert!((chart.height - 200.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_trend_bar_chart_with_bars() {
        let series = BarSeries::new(
            "Observations",
            vec![10.0, 25.0, 15.0, 30.0],
            "var(--chart-series-blue)",
        );
        let chart = TrendBarChart::new()
            .with_bars(vec![series])
            .with_dimensions(500.0, 250.0);
        assert_eq!(chart.bars.len(), 1);
        assert!((chart.width - 500.0).abs() < f64::EPSILON);
        assert!((chart.height - 250.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_trend_bar_chart_render_empty() {
        let chart = TrendBarChart::new();
        let _view = chart.render();
        // Should not panic — renders empty state
    }

    #[test]
    fn test_trend_bar_chart_render_single_point() {
        let series = BarSeries::new("Test", vec![42.0], "var(--chart-series-blue)");
        let chart = TrendBarChart::new().with_bars(vec![series]);
        let _view = chart.render();
        // Should not panic — renders single bar
    }

    #[test]
    fn test_trend_bar_chart_render_multi_series() {
        let s1 = BarSeries::new("A", vec![10.0, 20.0, 30.0], "var(--chart-series-blue)");
        let s2 = BarSeries::new("B", vec![5.0, 15.0, 25.0], "var(--chart-series-green)");
        let chart = TrendBarChart::new().with_bars(vec![s1, s2]);
        let _view = chart.render();
        // Should not panic — renders grouped bars
    }

    #[test]
    fn test_bar_series_new() {
        let series = BarSeries::new("Test", vec![1.0, 2.0, 3.0], "red");
        assert_eq!(series.label, "Test");
        assert_eq!(series.values, vec![1.0, 2.0, 3.0]);
        assert_eq!(series.color, "red");
    }
}
