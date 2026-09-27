use apex_shared::GrangerCausalPair;
use leptos::*;

#[component]
pub fn CausalGraph(pairs: Vec<GrangerCausalPair>) -> impl IntoView {
    let selected_index = create_rw_signal(0usize);
    let hovered_index = create_rw_signal(None::<usize>);

    let active_index = move || hovered_index.get().unwrap_or(selected_index.get());

    let rows = pairs
        .iter()
        .enumerate()
        .map(|(index, pair)| {
            let y = 50.0 + (index as f64 * 90.0);
            let cause = pair.cause_signal.clone();
            let effect = pair.effect_signal.clone();
            let lag_days = pair.optimal_lag_days;
            let p_value = pair.p_value;
            let significant = pair.significant_after_fdr;
            view! {
                <g
                    class=move || {
                        if active_index() == index {
                            "causal-row causal-row-active"
                        } else {
                            "causal-row"
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
                    <circle cx="150" cy=format!("{y:.2}") r="24" fill="var(--accent)" />
                    <circle cx="530" cy=format!("{y:.2}") r="24" fill="var(--primary)" />
                    <line class="causal-row-line" x1="180" y1=format!("{y:.2}") x2="500" y2=format!("{y:.2}") stroke="var(--foreground)" stroke-width="2" marker-end="url(#arrow)" />
                    <text x="150" y=format!("{:.2}", y + 42.0) text-anchor="middle" font-size="11" font-weight="700">{pair.cause_signal.clone()}</text>
                    <text x="530" y=format!("{:.2}", y + 42.0) text-anchor="middle" font-size="11" font-weight="700">{pair.effect_signal.clone()}</text>
                    <text x="340" y=format!("{:.2}", y - 10.0) text-anchor="middle" font-size="10" fill="var(--muted)">
                        {format!("lag {}d · p={:.3}", pair.optimal_lag_days, pair.p_value)}
                    </text>
                    <title>
                        {format!(
                            "{} drives {} after {} days with p-value {:.3}{}",
                            cause,
                            effect,
                            lag_days,
                            p_value,
                            if significant { " and remains significant after FDR" } else { "" }
                        )}
                    </title>
                </g>
            }
        })
        .collect_view();

    let height = 80 + (pairs.len().max(1) as i32 * 90);

    let selected_pair_view = move || {
        let Some(pair) = pairs.get(active_index()).cloned() else {
            return view! { <p class="muted-copy">"Select a relationship to inspect its details."</p> }.into_view();
        };

        view! {
            <div class="graph-side-panel">
                <div class="graph-side-header">
                    <h3 class="graph-side-title">{format!("{} -> {}", pair.cause_signal, pair.effect_signal)}</h3>
                    <span class=if pair.significant_after_fdr { "source-badge tier-official" } else { "source-badge tier-social" }>
                        {if pair.significant_after_fdr { "FDR-kept" } else { "Exploratory" }}
                    </span>
                </div>
                <div class="timeline-list">
                    <div class="timeline-item">
                        <strong>"Optimal Lag"</strong>
                        <span class="muted-copy">{format!("{} days", pair.optimal_lag_days)}</span>
                    </div>
                    <div class="timeline-item">
                        <strong>"P-Value"</strong>
                        <span class="muted-copy">{format!("{:.3}", pair.p_value)}</span>
                    </div>
                    <div class="timeline-item">
                        <strong>"Interpretation"</strong>
                        <span>
                            {if pair.significant_after_fdr {
                                format!(
                                    "Historical data suggests {} tends to precede {} strongly enough to survive multiple-testing correction.",
                                    pair.cause_signal,
                                    pair.effect_signal
                                )
                            } else {
                                format!(
                                    "{} may precede {}, but the current signal should be treated as directional rather than production-grade.",
                                    pair.cause_signal,
                                    pair.effect_signal
                                )
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
                <svg viewBox=format!("0 0 680 {height}") class="community-graph" aria-label="Granger causal graph">
                    <defs>
                        <marker id="arrow" markerWidth="8" markerHeight="8" refX="7" refY="4" orient="auto">
                            <path d="M 0 0 L 8 4 L 0 8 z" fill="var(--foreground)" />
                        </marker>
                    </defs>
                    <rect x="0" y="0" width="680" height=format!("{height}") fill="var(--card)" rx="12" />
                    {rows}
                </svg>
            </div>
            {selected_pair_view}
        </div>
    }
}
