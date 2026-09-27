use leptos::*;

#[component]
pub fn ThreatIntelPage() -> impl IntoView {
    let (threats, set_threats) = create_signal::<Vec<crate::api::ThreatIntelSummary>>(vec![]);
    let (loading, set_loading) = create_signal(true);
    let (error, set_error) = create_signal::<Option<String>>(None);

    create_effect(move |_| {
        spawn_local(async move {
            match crate::api::fetch_threat_intel().await {
                Ok(data) => {
                    set_threats.set(data);
                    set_loading.set(false);
                }
                Err(e) => {
                    set_error.set(Some(format!("Failed to load threat intelligence: {}", e)));
                    set_loading.set(false);
                }
            }
        });
    });

    view! {
        <div class="page threat-intel-page">
            <div class="page-header">
                <h1 class="page-title">Threat Intelligence</h1>
                <p class="page-subtitle">Cyber, physical, and geopolitical threat monitoring for your supply chain and operations</p>
            </div>

            {move || match (loading.get(), error.get()) {
                (true, _) => view! {
                    <div class="loading-state">
                        <div class="spinner" aria-label="Loading threat intelligence"></div>
                        <p>Loading threat intelligence...</p>
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
                (false, None) if threats.get().is_empty() => view! {
                    <div class="empty-state">
                        <svg class="empty-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" aria-hidden="true">
                            <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"/>
                        </svg>
                        <h3>Threat Intelligence Monitoring Active</h3>
                        <p>No active threats detected. The system continuously monitors for cyber, physical, and geopolitical threats.</p>
                    </div>
                }.into_view(),
                (false, None) => view! {
                    <div class="threat-grid">
                        {threats.get().into_iter().map(|threat| view! {
                            <article
                                class=format!("card threat-card threat-card-{}", threat.severity.to_lowercase())
                                aria-label=format!("Threat: {}", threat.title)
                            >
                                <div class="card-header">
                                    <div class="threat-header-left">
                                        <span class=format!("badge badge-severity-{}", threat.severity.to_lowercase())>
                                            {&threat.severity}
                                        </span>
                                        <span class="badge badge-category">{&threat.category}</span>
                                    </div>
                                    <span class="threat-date">{&threat.detected_at}</span>
                                </div>
                                <div class="card-body">
                                    <h3 class="threat-title">{&threat.title}</h3>
                                    <p class="threat-description">{&threat.description}</p>
                                    <div class="threat-details">
                                        <div class="threat-detail">
                                            <span class="detail-label">Source</span>
                                            <span class="detail-value">{&threat.source}</span>
                                        </div>
                                        <div class="threat-detail">
                                            <span class="detail-label">Confidence</span>
                                            <span class="detail-value">{format!("{:.0}%", threat.confidence * 100.0)}</span>
                                        </div>
                                        <div class="threat-detail">
                                            <span class="detail-label">Affected Entities</span>
                                            <span class="detail-value">{threat.affected_entity_count}</span>
                                        </div>
                                    </div>
                                    {threat.mitre_tactic.as_ref().map(|tactic| view! {
                                        <div class="threat-mitre">
                                            <span class="mitre-label">MITRE ATT&CK</span>
                                            <span class="mitre-value">{tactic}</span>
                                        </div>
                                    })}
                                </div>
                            </article>
                        }).collect_view()}
                    </div>
                }.into_view(),
            }}
        </div>
    }
}
