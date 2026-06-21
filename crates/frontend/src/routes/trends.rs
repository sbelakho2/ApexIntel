use leptos::*;
use serde_json::Value;

use crate::api;
use crate::components::cards::{PageHeader, SurfaceCard};

#[component]
pub fn TrendsPage() -> impl IntoView {
    let page = create_rw_signal(1u32);
    let trends_resource = create_resource(move || page.get(), |page| async move {
        api::fetch_trends(page).await
    });

    view! {
        <div class="page">
            <PageHeader
                eyebrow="Historical Trends"
                title="Trends"
                subtitle="Long-term signal evolution, market shifts, and competitive trajectory data from the trends analysis pipeline."
            />

            <Suspense fallback=move || view! {
                <SurfaceCard title="Trends" subtitle="Loading trend data...">
                    <p class="muted-copy">"Loading..."</p>
                </SurfaceCard>
            }>
                {move || trends_resource.get().map(|result| match result {
                    Ok(response) => view! {
                        <SurfaceCard title="Trends" subtitle="Historical trend data">
                            <p class="muted-copy">
                                {format!("{} trend records available. Total tracked: {}.", response.items.len(), response.total)}
                            </p>
                            <div class="pagination-bar">
                                <button
                                    class="btn btn-outline"
                                    disabled=move || page.get() <= 1
                                    on:click=move |_| page.update(|p| *p = p.saturating_sub(1))
                                >
                                    "Previous"
                                </button>
                                <span class="pagination-info">
                                    {format!("Page {}", response.page)}
                                </span>
                                <button
                                    class="btn btn-outline"
                                    disabled=move || response.items.len() < response.per_page as usize
                                    on:click=move |_| page.update(|p| *p += 1)
                                >
                                    "Next"
                                </button>
                            </div>
                        </SurfaceCard>
                    }.into_view(),
                    Err(message) => view! {
                        <SurfaceCard title="Trends" subtitle="Failed to load trend data">
                            <p class="error-copy">{message}</p>
                        </SurfaceCard>
                    }.into_view(),
                })}
            </Suspense>
        </div>
    }
}