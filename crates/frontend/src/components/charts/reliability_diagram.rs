use apex_shared::CalibrationCurve;
use leptos::*;

#[component]
pub fn ReliabilityDiagram(curve: CalibrationCurve) -> impl IntoView {
    let points = curve.points.clone();
    let selected_index = create_rw_signal(0usize);
    let hovered_index = create_rw_signal(None::<usize>);

    let active_index = move || hovered_index.get().unwrap_or(selected_index.get());

    let calibration_path = points
        .iter()
        .enumerate()
        .map(|(index, point)| {
            let x = 20.0 + point.predicted_probability.clamp(0.0, 1.0) * 260.0;
            let y = 280.0 - point.observed_frequency.clamp(0.0, 1.0) * 260.0;
            if index == 0 {
                format!("M {x:.2} {y:.2}")
            } else {
                format!("L {x:.2} {y:.2}")
            }
        })
        .collect::<Vec<_>>()
        .join(" ");

    let point_view = points
        .iter()
        .enumerate()
        .map(|(index, point)| {
            let x = 20.0 + point.predicted_probability.clamp(0.0, 1.0) * 260.0;
            let y = 280.0 - point.observed_frequency.clamp(0.0, 1.0) * 260.0;
            let radius = (point.bin_count.max(8) as f64).sqrt() / 2.5;
            view! {
                <g
                    class=move || {
                        if active_index() == index {
                            "chart-point-group chart-point-group-active"
                        } else {
                            "chart-point-group"
                        }
                    }
                    tabindex="0"
                    role="button"
                    on:mouseenter=move |_| hovered_index.set(Some(index))
                    on:mouseleave=move |_| hovered_index.set(None)
                    on:focus=move |_| hovered_index.set(Some(index))
                    on:blur=move |_| hovered_index.set(None)
                    on:click=move |_| selected_index.set(index)
                    on:keydown=move |ev| {
                        let key = ev.key();
                        if key == "Enter" || key == " " {
                            ev.prevent_default();
                            selected_index.set(index);
                        }
                    }
                >
                    <circle cx=format!("{x:.2}") cy=format!("{y:.2}") r=format!("{radius:.2}") class="chart-series-point" fill="var(--accent)" opacity="0.9" />
                    <title>{format!(
                        "Predicted {:.0}% observed {:.0}% with {} samples",
                        point.predicted_probability * 100.0,
                        point.observed_frequency * 100.0,
                        point.bin_count
                    )}</title>
                </g>
            }
        })
        .collect_view();

    let active_point_view = move || {
        let Some(point) = points.get(active_index()).cloned() else {
            return view! { <p class="muted-copy">"No calibration bins available."</p> }
                .into_view();
        };

        view! {
            <div class="graph-side-panel">
                <div class="graph-side-header">
                    <h3 class="graph-side-title">"Selected Bin"</h3>
                    <span class="source-badge tier-trade-press">{format!("{} samples", point.bin_count)}</span>
                </div>
                <div class="timeline-list">
                    <div class="timeline-item">
                        <strong>"Predicted"</strong>
                        <span class="muted-copy">{format!("{:.0}%", point.predicted_probability * 100.0)}</span>
                    </div>
                    <div class="timeline-item">
                        <strong>"Observed"</strong>
                        <span class="muted-copy">{format!("{:.0}%", point.observed_frequency * 100.0)}</span>
                    </div>
                    <div class="timeline-item">
                        <strong>"Calibration Gap"</strong>
                        <span>{format!("{:+.1}%", (point.observed_frequency - point.predicted_probability) * 100.0)}</span>
                    </div>
                </div>
            </div>
        }
        .into_view()
    };

    view! {
        <div class="chart-layout">
            <div class="chart-scroll-shell">
                <svg viewBox="0 0 300 300" class="reliability-diagram" aria-label="Reliability diagram">
                    <rect x="10" y="10" width="280" height="280" rx="14" class="chart-plot-surface" />
                    <line x1="20" y1="280" x2="280" y2="20" class="chart-grid-line" />
                    <line x1="20" y1="215" x2="280" y2="215" class="chart-grid-line" />
                    <line x1="20" y1="150" x2="280" y2="150" class="chart-grid-line" />
                    <line x1="20" y1="85" x2="280" y2="85" class="chart-grid-line" />
                    <line x1="20" y1="280" x2="280" y2="280" class="chart-axis-line" />
                    <line x1="20" y1="20" x2="20" y2="280" class="chart-axis-line" />
                    <path d=calibration_path class="chart-series-line chart-series-line-blue" />
                    {point_view}
                    <text x="280" y="24" text-anchor="end" font-size="11" font-weight="700">
                        {format!("Brier: {:.3}", curve.brier_score)}
                    </text>
                    <text x="280" y="40" text-anchor="end" font-size="11" fill="var(--muted)">
                        {format!("Reliability: {:.2}", curve.reliability)}
                    </text>
                </svg>
            </div>
            {active_point_view}
        </div>
    }
}
