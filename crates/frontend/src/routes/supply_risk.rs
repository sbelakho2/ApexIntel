use leptos::*;

#[component]
pub fn SupplyRiskPage() -> impl IntoView {
    let (risks, set_risks) = create_signal::<Vec<crate::api::SupplyRiskSummary>>(vec![]);
    let (loading, set_loading) = create_signal(true);
    let (error, set_error) = create_signal::<Option<String>>(None);
    let (filter, set_filter) = create_signal(String::from("all"));

    create_effect(move |_| {
        spawn_local(async move {
            match crate::api::fetch_supply_risks().await {
                Ok(data) => {
                    set_risks.set(data);
                    set_loading.set(false);
                }
                Err(e) => {
                    set_error.set(Some(format!("Failed to load supply chain risks: {}", e)));
                    set_loading.set(false);
                }
            }
        });
    });

    let filtered_risks = move || {
        let f = filter.get();
        risks
            .get()
            .into_iter()
            .filter(move |r| f == "all" || r.risk_level.to_lowercase() == f)
            .collect::<Vec<_>>()
    };

    let filter_options: Vec<(&str, &str)> = vec![
        ("all", "All Risks"),
        ("critical", "Critical"),
        ("high", "High"),
        ("medium", "Medium"),
        ("low", "Low"),
    ];

    view! {
        <div class="page supply-risk-page">
            <div class="page-header">
                <h1 class="page-title">Supply Chain Risk</h1>
                <p class="page-subtitle">Monitor supplier dependencies, geopolitical disruptions, and logistics vulnerabilities</p>
            </div>

            <div class="filter-bar" role="toolbar" aria-label="Risk filters">
                {filter_options.iter().map(|(level_val, label)| {
                    let level_val = level_val.to_string();
                    let label = label.to_string();
                    let f = filter;
                    let lv = level_val.clone();
                    view! {
                        <button
                            class=move || {
                                let active = if f.get() == lv { "filter-chip-active" } else { "" };
                                format!("filter-chip {}", active)
                            }
                            on:click={
                                let lv2 = level_val.clone();
                                move |_| set_filter.set(lv2.clone())
                            }
                            aria-pressed={
                                let lv2 = level_val.clone();
                                move || (filter.get() == lv2).to_string()
                            }
                        >
                            {label.clone()}
                        </button>
                    }
                }).collect_view()}
            </div>

            {move || match (loading.get(), error.get()) {
                (true, _) => view! {
                    <div class="loading-state">
                        <div class="spinner" aria-label="Loading supply chain risks"></div>
                        <p>Analyzing supply chain...</p>
                    </div>
                }.into_view(),
                (_, Some(err)) => view! {
                    <div class="error-state" role="alert">
                        <svg class="error-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true">
                            <circle cx="12" cy="12" r="10"/>
                            <line x1="12" y1="8" x2="12" y2="12"/>
                            <line x1="12" y1="16" x2="12.01" y2="16"/>
                        </svg>
                        <p>{err}</p>
                        <button class="btn btn-secondary" on:click=move |_| {
                            set_loading.set(true);
                            set_error.set(None);
                        }>Retry</button>
                    </div>
                }.into_view(),
                (false, None) if filtered_risks().is_empty() => view! {
                    <div class="empty-state">
                        <h3>No Supply Chain Risks Detected</h3>
                        <p>Supply chain risk monitoring is active.</p>
                    </div>
                }.into_view(),
                (false, None) => view! {
                    <div class="risk-table-container">
                        <table class="data-table" role="table">
                            <thead>
                                <tr>
                                    <th scope="col">Supplier</th>
                                    <th scope="col">Level</th>
                                    <th scope="col">Category</th>
                                    <th scope="col">Impact</th>
                                    <th scope="col">Detected</th>
                                </tr>
                            </thead>
                            <tbody>
                                {filtered_risks().into_iter().map(|risk| view! {
                                    <tr>
                                        <td>{risk.name.clone()}</td>
                                        <td><span class=format!("badge badge-{}", risk.risk_level.to_lowercase())>{risk.risk_level.clone()}</span></td>
                                        <td>{risk.category.clone()}</td>
                                        <td>{risk.impact_score}%</td>
                                        <td>{risk.last_detected.clone()}</td>
                                    </tr>
                                }).collect_view()}
                            </tbody>
                        </table>
                    </div>
                }.into_view(),
            }}
        </div>
    }
}
