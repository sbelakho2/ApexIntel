use leptos::*;

#[component]
pub fn BattlecardsPage() -> impl IntoView {
    let (battlecards, set_battlecards) =
        create_signal::<Vec<crate::api::BattlecardSummary>>(vec![]);
    let (loading, set_loading) = create_signal(true);
    let (error, set_error) = create_signal::<Option<String>>(None);

    create_effect(move |_| {
        spawn_local(async move {
            match crate::api::fetch_battlecards().await {
                Ok(data) => {
                    set_battlecards.set(data);
                    set_loading.set(false);
                }
                Err(e) => {
                    set_error.set(Some(format!("Failed to load battlecards: {}", e)));
                    set_loading.set(false);
                }
            }
        });
    });

    view! {
        <div class="page battlecards-page">
            <div class="page-header">
                <h1 class="page-title">Battlecards</h1>
                <p class="page-subtitle">Competitive intelligence battlecards for key accounts and opportunities</p>
            </div>

            {move || match (loading.get(), error.get()) {
                (true, _) => view! {
                    <div class="loading-state">
                        <div class="spinner" aria-label="Loading battlecards"></div>
                        <p>Loading battlecards...</p>
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
                            spawn_local(async move {
                                match crate::api::fetch_battlecards().await {
                                    Ok(data) => { set_battlecards.set(data); set_loading.set(false); }
                                    Err(e) => { set_error.set(Some(format!("Failed to load: {}", e))); set_loading.set(false); }
                                }
                            });
                        }>Retry</button>
                    </div>
                }.into_view(),
                (false, None) if battlecards.get().is_empty() => view! {
                    <div class="empty-state">
                        <svg class="empty-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" aria-hidden="true">
                            <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/>
                            <polyline points="14 2 14 8 20 8"/>
                            <line x1="16" y1="13" x2="8" y2="13"/>
                            <line x1="16" y1="17" x2="8" y2="17"/>
                        </svg>
                        <h3>No Battlecards Available</h3>
                        <p>Competitive battlecards will appear here as intelligence is gathered on key accounts.</p>
                    </div>
                }.into_view(),
                (false, None) => view! {
                    <div class="battlecards-grid">
                        {battlecards.get().into_iter().map(|card| view! {
                            <article class="card battlecard-card" aria-label=format!("Battlecard for {}", card.account_name)>
                                <div class="card-header">
                                    <h3 class="card-title">{&card.account_name}</h3>
                                    <span class=format!("badge badge-{}", card.threat_level.to_lowercase())>
                                        {&card.threat_level}
                                    </span>
                                </div>
                                <div class="card-body">
                                    <div class="battlecard-stat">
                                        <span class="stat-label">Win Probability</span>
                                        <span class="stat-value">{format!("{:.0}%", card.win_probability * 100.0)}</span>
                                    </div>
                                    <div class="battlecard-stat">
                                        <span class="stat-label">Key Competitors</span>
                                        <span class="stat-value">{card.competitor_count}</span>
                                    </div>
                                    <div class="battlecard-section">
                                        <h4>Key Intel</h4>
                                        <p>{&card.key_intel}</p>
                                    </div>
                                    {card.last_updated.as_ref().map(|date| view! {
                                        <div class="card-meta">Updated: {date.to_string()}</div>
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
