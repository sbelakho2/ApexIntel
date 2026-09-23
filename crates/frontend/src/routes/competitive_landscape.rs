use leptos::*;

use crate::api;
use crate::components::cards::{PageHeader, SurfaceCard};

#[component]
pub fn CompetitiveLandscapePage() -> impl IntoView {
    let landscape_resource = create_resource(
        || (),
        |_| async { api::fetch_competitive_landscape().await },
    );

    view! {
        <div class="page">
            <PageHeader
                eyebrow="Competitive Analysis"
                title="Competitive Landscape"
                subtitle="Market positioning, competitor capabilities, pricing intelligence, and strategic predictions from the competitive intelligence engine."
            />

            <Suspense fallback=move || view! {
                <SurfaceCard title="Competitive Landscape" subtitle="Loading competitive data...">
                    <p class="muted-copy">"Loading..."</p>
                </SurfaceCard>
            }>
                {move || landscape_resource.get().map(|result| match result {
                    Ok(data) => view! {
                        <div class="competitive-landscape">
                            <SurfaceCard title="Competitive Landscape" subtitle="Data loaded from competitive intelligence engine">
                                <p class="muted-copy">
                                    {format!("{} competitors tracked. Market intelligence is being gathered continuously.", data.get("competitors").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0))}
                                </p>
                            </SurfaceCard>
                        </div>
                    }.into_view(),
                    Err(message) => view! {
                        <SurfaceCard title="Competitive Landscape" subtitle="Failed to load">
                            <p class="error-copy">{message}</p>
                        </SurfaceCard>
                    }.into_view(),
                })}
            </Suspense>
        </div>
    }
}
