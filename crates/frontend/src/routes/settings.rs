use leptos::*;

use crate::{
    api,
    components::cards::{PageHeader, SurfaceCard},
};

#[component]
pub fn SettingsPage() -> impl IntoView {
    let preferences = create_resource(|| (), |_| async { api::fetch_preferences().await });

    view! {
        <div class="page">
            <PageHeader eyebrow="Preferences" title="Settings" subtitle="The WASM settings view is now grounded in the persisted preferences API." />
            <Suspense fallback=move || view! { <SurfaceCard title="Preferences" subtitle="Loading preferences."><p class="muted-copy">"Loading..."</p></SurfaceCard> }>
                {move || preferences.get().map(|result| match result {
                    Ok(response) => {
                        let preferences = response.preferences;
                        view! {
                            <SurfaceCard title="User Preferences" subtitle="Stored settings from `/api/preferences`.">
                                <div class="timeline-list">
                                    <div class="timeline-item"><strong>"Theme"</strong><span>{preferences.theme}</span></div>
                                    <div class="timeline-item"><strong>"Locale"</strong><span>{preferences.locale}</span></div>
                                    <div class="timeline-item"><strong>"Default Region"</strong><span>{preferences.default_region.unwrap_or_else(|| "Unset".to_string())}</span></div>
                                    <div class="timeline-item"><strong>"Dashboard Layout"</strong><span>{preferences.dashboard_layout}</span></div>
                                    <div class="timeline-item"><strong>"Notifications"</strong><span>{format!("email={} slack={} browser={} min={}", preferences.notifications.email_enabled, preferences.notifications.slack_enabled, preferences.notifications.browser_push, preferences.notifications.min_severity)}</span></div>
                                </div>
                            </SurfaceCard>
                        }.into_view()
                    }
                    Err(message) => view! { <SurfaceCard title="Preferences" subtitle="The API request failed. Try refreshing the page."><p class="error-copy">{message}</p></SurfaceCard> }.into_view(),
                })}
            </Suspense>
        </div>
    }
}
