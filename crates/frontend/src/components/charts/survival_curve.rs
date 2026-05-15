use apex_shared::SurvivalPoint;
use leptos::*;

#[component]
pub fn SurvivalCurve(points: Vec<SurvivalPoint>) -> impl IntoView {
    let chart_points = if points.is_empty() {
        vec![SurvivalPoint {
            day: 0,
            survival_probability: 1.0,
        }]
    } else {
        points
    };
    let selected_index = create_rw_signal(0usize);
    let hovered_index = create_rw_signal(None::<usize>);
    let active_index = move || hovered_index.get().unwrap_or(selected_index.get());

    let mut path = String::from("M 30 30");
    let mut last_y = 210.0;
    for point in &chart_points {
        let x = 30.0 + (point.day as f64 * 36.0);
        let y = 230.0 - point.survival_probability.clamp(0.0, 1.0) * 200.0;
        path.push_str(&format!(" L {x:.2} {last_y:.2} L {x:.2} {y:.2}"));
        last_y = y;
    }
    let final_x = 30.0 + (chart_points.last().map(|point| point.day).unwrap_or(0) as f64 * 36.0);
    let area_path = format!("{} L {final_x:.2} 230 L 30 230 Z", path);

    let marker_view = chart_points
        .iter()
        .enumerate()
        .map(|(index, point)| {
            let x = 30.0 + (point.day as f64 * 36.0);
            let y = 230.0 - point.survival_probability.clamp(0.0, 1.0) * 200.0;
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
                    <circle cx=format!("{x:.2}") cy=format!("{y:.2}") r="5" class="chart-series-point" fill="var(--success)" />
                    <title>{format!("Day {} survival {:.0}%", point.day, point.survival_probability * 100.0)}</title>
                </g>
            }
        })
        .collect_view();

    let active_point_view = move || {
        let Some(point) = chart_points.get(active_index()).cloned() else {
            return view! { <p class="muted-copy">"No survival points available."</p> }.into_view();
        };

        view! {
            <div class="graph-side-panel">
                <div class="graph-side-header">
                    <h3 class="graph-side-title">{format!("Day {}", point.day)}</h3>
                    <span class="source-badge tier-official">{format!("{:.0}%", point.survival_probability * 100.0)}</span>
                </div>
                <div class="timeline-list">
                    <div class="timeline-item">
                        <strong>"Survival Probability"</strong>
                        <span class="muted-copy">{format!("{:.1}%", point.survival_probability * 100.0)}</span>
                    </div>
                    <div class="timeline-item">
                        <strong>"Interpretation"</strong>
                        <span>
                            {if point.survival_probability >= 0.85 {
                                "Risk remains low at this point in the window.".to_string()
                            } else if point.survival_probability >= 0.7 {
                                "Probability is decaying and warrants closer monitoring.".to_string()
                            } else {
                                "The event is becoming materially more likely by this day.".to_string()
                            }}
                        </span>
                    </div>
                </div>
            </div>
        }
        .into_view()
    };

    view! {
        <div class="chart-layout">
            <div class="chart-scroll-shell">
                <svg viewBox="0 0 420 260" class="reliability-diagram" aria-label="Kaplan-Meier style survival curve">
                    <rect x="10" y="12" width="400" height="232" rx="14" class="chart-plot-surface" />
                    <line x1="30" y1="180" x2="390" y2="180" class="chart-grid-line" />
                    <line x1="30" y1="130" x2="390" y2="130" class="chart-grid-line" />
                    <line x1="30" y1="80" x2="390" y2="80" class="chart-grid-line" />
                    <line x1="30" y1="230" x2="390" y2="230" class="chart-axis-line" />
                    <line x1="30" y1="30" x2="30" y2="230" class="chart-axis-line" />
                    <path d=area_path class="chart-series-area" />
                    <path d=path class="chart-series-line chart-series-line-green" />
                    {marker_view}
                    <text x="390" y="26" text-anchor="end" class="chart-value-label">"Survival"</text>
                </svg>
            </div>
            {active_point_view}
        </div>
    }
}
