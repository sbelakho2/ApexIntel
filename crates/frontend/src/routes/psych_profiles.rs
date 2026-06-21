use leptos::*;

#[component]
pub fn PsychProfilesPage() -> impl IntoView {
    let (profiles, set_profiles) = create_signal::<Vec<crate::api::PsychProfileSummary>>(vec![]);
    let (loading, set_loading) = create_signal(true);
    let (error, set_error) = create_signal::<Option<String>>(None);
    let (selected_id, set_selected_id) = create_signal::<Option<String>>(None);

    create_effect(move |_| {
        spawn_local(async move {
            match crate::api::fetch_psych_profiles().await {
                Ok(data) => {
                    set_profiles.set(data);
                    set_loading.set(false);
                }
                Err(e) => {
                    set_error.set(Some(format!("Failed to load psychological profiles: {}", e)));
                    set_loading.set(false);
                }
            }
        });
    });

    let selected_profile = move || {
        let id = selected_id.get();
        profiles.get().into_iter().find(|p| Some(p.person_id.clone()) == id)
    };

    view! {
        <div class="page psych-profiles-page">
            <div class="page-header">
                <h1 class="page-title">Psychological Profiles</h1>
                <p class="page-subtitle">Decision-maker personality profiles, influence maps, and communication strategies</p>
            </div>

            {move || match (loading.get(), error.get()) {
                (true, _) => view! {
                    <div class="loading-state">
                        <div class="spinner" aria-label="Loading psychological profiles"></div>
                        <p>Loading psychological profiles...</p>
                    </div>
                }.into_view(),
                (_, Some(err)) => view! {
                    <div class="error-state" role="alert">
                        <p>{err}</p>
                        <button class="btn btn-secondary" on:click=move |_| {
                            set_loading.set(true);
                            set_error.set(None);
                        }>Retry</button>
                    </div>
                }.into_view(),
                (false, None) if profiles.get().is_empty() => view! {
                    <div class="empty-state">
                        <h3>No Psychological Profiles</h3>
                        <p>Psychological profiles are generated from OSINT data on key decision-makers.</p>
                    </div>
                }.into_view(),
                (false, None) => view! {
                    <div class="profiles-layout">
                        <div class="profiles-list">
                            {profiles.get().into_iter().map(|profile| {
                                let pid = profile.person_id.clone();
                                let pid_for_click = pid.clone();
                                let sid = selected_id.clone();
                                view! {
                                    <button
                                        class=move || format!("profile-list-item {}", if sid.get().as_ref() == Some(&pid) { "profile-list-item-active" } else { "" })
                                        on:click=move |_| set_selected_id.set(Some(pid_for_click.clone()))
                                    >
                                        <div class="profile-list-name">{profile.person_name.clone()}</div>
                                        <div class="profile-list-role">{profile.current_role.clone()}</div>
                                        <div class="profile-list-company">{profile.company_name.clone()}</div>
                                    </button>
                                }
                            }).collect_view()}
                        </div>
                        <div class="profile-detail">
                            {move || match selected_profile() {
                                None => view! { <p>Select a profile to view psychological details</p> }.into_view(),
                                Some(profile) => view! {
                                    <div class="profile-detail-content">
                                        <h2>{profile.person_name.clone()}</h2>
                                        <p>{profile.current_role.clone()} @ {profile.company_name.clone()}</p>
                                        <div class="profile-section">
                                            <h3>Decision Style</h3>
                                            <span class="badge">{profile.decision_style.clone()}</span>
                                        </div>
                                        <div class="profile-section">
                                            <h3>Influence Role</h3>
                                            <span class="badge">{profile.influence_role.clone()}</span>
                                        </div>
                                        <div class="profile-section">
                                            <h3>Communication Style</h3>
                                            <p>{profile.communication_style.clone()}</p>
                                        </div>
                                        <div class="profile-section">
                                            <h3>Recommended Approach</h3>
                                            <p>{profile.recommended_approach.clone()}</p>
                                        </div>
                                    </div>
                                }.into_view(),
                            }}
                        </div>
                    </div>
                }.into_view(),
            }}
        </div>
    }
}