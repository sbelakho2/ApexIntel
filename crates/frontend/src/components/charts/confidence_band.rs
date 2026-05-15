use apex_shared::ConfidenceInterval;
use leptos::*;

#[derive(Clone, Debug, PartialEq)]
pub struct ConfidenceBandPoint {
    pub label: String,
    pub interval: ConfidenceInterval,
}

#[component]
pub fn ConfidenceBand(points: Vec<ConfidenceBandPoint>) -> impl IntoView {
    let chart_points = points
        .iter()
        .enumerate()
        .map(|(index, point)| {
            let x = if points.len() > 1 {
                28.0 + index as f64 * (244.0 / (points.len() as f64 - 1.0))
            } else {
                150.0
            };
            let value_y = 130.0 - point.interval.value * 100.0;
            let lower_y = 130.0 - point.interval.lower * 100.0;
            let upper_y = 130.0 - point.interval.upper * 100.0;
            (point.label.clone(), x, value_y, lower_y, upper_y)
        })
        .collect::<Vec<_>>();

    let path = chart_points
        .iter()
        .enumerate()
        .map(|(index, (_, x, y, _, _))| {
            if index == 0 {
                format!("M {:.1} {:.1}", x, y)
            } else {
                format!("L {:.1} {:.1}", x, y)
            }
        })
        .collect::<Vec<_>>()
        .join(" ");

    let band_path = if chart_points.is_empty() {
        String::new()
    } else {
        let upper = chart_points
            .iter()
            .enumerate()
            .map(|(index, (_, x, _, _, upper_y))| {
                if index == 0 {
                    format!("M {:.1} {:.1}", x, upper_y)
                } else {
                    format!("L {:.1} {:.1}", x, upper_y)
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
        let lower = chart_points
            .iter()
            .rev()
            .map(|(_, x, _, lower_y, _)| format!("L {:.1} {:.1}", x, lower_y))
            .collect::<Vec<_>>()
            .join(" ");
        format!("{} {} Z", upper, lower)
    };

    let final_value = chart_points.last().map(|(_, _, value_y, _, _)| *value_y);

    view! {
        <svg viewBox="0 0 300 160" class="confidence-band-chart" role="img" aria-label="Confidence interval band chart">
            <rect x="20" y="20" width="252" height="112" rx="12" class="chart-plot-surface" />
            <line x1="28" y1="130" x2="272" y2="130" class="chart-axis-line" />
            <line x1="28" y1="30" x2="28" y2="130" class="chart-axis-line" />
            <line x1="28" y1="80" x2="272" y2="80" class="chart-grid-line" />
            <line x1="28" y1="55" x2="272" y2="55" class="chart-grid-line" />
            <path d=band_path class="chart-series-band" />
            <path d=path class="chart-series-line chart-series-line-blue" />
            <For
                each=move || chart_points.clone()
                key=|(label, _, _, _, _)| label.clone()
                let:point
            >
                <line x1=point.1 y1=point.3 x2=point.1 y2=point.4 stroke="var(--chart-series-amber)" stroke-width="4" stroke-linecap="round" />
                <circle cx=point.1 cy=point.2 r="4" class="chart-series-point" fill="var(--primary)" />
                <text x=point.1 y="150" text-anchor="middle" class="chart-axis-label">{point.0}</text>
            </For>
            {final_value.map(|y| view! { <text x="272" y=format!("{:.1}", y - 8.0) text-anchor="end" class="chart-value-label">"Latest"</text> })}
        </svg>
    }
}
