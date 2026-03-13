use leptos::*;

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
        <svg viewBox="0 0 88 30" class="sparkline" role="img" aria-label=label.unwrap_or_else(|| "sparkline".to_string())>
            <rect x="2" y="3" width="84" height="24" rx="8" class="chart-plot-surface" />
            <line x1="4" y1="26" x2="84" y2="26" class="chart-grid-line" />
            <line x1="4" y1="14" x2="84" y2="14" class="chart-grid-line" />
            <path d=format!("{} L 84 28 L 4 28 Z", path.clone()) fill="rgba(74, 144, 226, 0.14)" stroke="none" />
            <path d=path class="chart-series-line chart-series-line-blue" />
            {first.map(|point| view! { <circle cx=point.0 cy=point.1 r="2.5" fill="var(--accent)" /> })}
            {last.map(|point| view! {
                <g>
                    <circle cx=point.0 cy=point.1 r="3.2" class="chart-series-point" fill="var(--foreground)" />
                    <text x=point.0 y=format!("{:.1}", point.1 - 5.0) text-anchor="middle" class="chart-value-label">{format!("{:.1}", last_value)}</text>
                </g>
            })}
        </svg>
    }
}