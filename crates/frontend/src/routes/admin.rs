use leptos::*;

use crate::{
    api,
    components::cards::{PageHeader, SurfaceCard},
};

fn number_field(value: &serde_json::Value, key: &str) -> String {
    value
        .get(key)
        .map(|value| {
            value
                .as_i64()
                .map(|value| value.to_string())
                .or_else(|| value.as_u64().map(|value| value.to_string()))
                .unwrap_or_else(|| value.to_string())
        })
        .unwrap_or_else(|| "0".to_string())
}

#[component]
pub fn AdminPage() -> impl IntoView {
    let crawl = create_resource(
        || (),
        |_| async { api::fetch_admin_value("/api/admin/crawl-status").await },
    );
    let recipes = create_resource(
        || (),
        |_| async { api::fetch_admin_value("/api/admin/recipe-performance").await },
    );
    let coverage = create_resource(
        || (),
        |_| async { api::fetch_admin_value("/api/admin/poi-coverage").await },
    );

    view! {
        <div class="page">
            <PageHeader eyebrow="Administration" title="Admin" subtitle="Admin overview cards now query the live operational endpoints from the WASM frontend." />
            <div class="three-up">
                <Suspense fallback=move || view! { <SurfaceCard title="Crawl Status" subtitle="Loading crawl metrics."><p class="muted-copy">"Loading..."</p></SurfaceCard> }>
                    {move || crawl.get().map(|result| match result {
                        Ok(value) => view! {
                            <SurfaceCard title="Crawl Status" subtitle="Operational crawl metrics from `/api/admin/crawl-status`.">
                                <div class="timeline-list">
                                    <div class="timeline-item"><strong>"Tracked Domains"</strong><span>{number_field(&value, "domains_tracked")}</span></div>
                                    <div class="timeline-item"><strong>"Total Fingerprints"</strong><span>{number_field(&value, "total_fingerprints")}</span></div>
                                </div>
                            </SurfaceCard>
                        }.into_view(),
                        Err(message) => view! { <SurfaceCard title="Crawl Status" subtitle="The API request failed. Try refreshing the page."><p class="error-copy">{message}</p></SurfaceCard> }.into_view(),
                    })}
                </Suspense>
                <Suspense fallback=move || view! { <SurfaceCard title="Recipe Performance" subtitle="Loading recipe metrics."><p class="muted-copy">"Loading..."</p></SurfaceCard> }>
                    {move || recipes.get().map(|result| match result {
                        Ok(value) => view! {
                            <SurfaceCard title="Recipe Performance" subtitle="Recipe operations metrics from `/api/admin/recipe-performance`.">
                                <div class="timeline-list">
                                    <div class="timeline-item"><strong>"Production"</strong><span>{number_field(&value, "production_count")}</span></div>
                                    <div class="timeline-item"><strong>"Staging"</strong><span>{number_field(&value, "staging_count")}</span></div>
                                    <div class="timeline-item"><strong>"Deprecated"</strong><span>{number_field(&value, "deprecated_count")}</span></div>
                                </div>
                            </SurfaceCard>
                        }.into_view(),
                        Err(message) => view! { <SurfaceCard title="Recipe Performance" subtitle="The API request failed. Try refreshing the page."><p class="error-copy">{message}</p></SurfaceCard> }.into_view(),
                    })}
                </Suspense>
                <Suspense fallback=move || view! { <SurfaceCard title="POI Coverage" subtitle="Loading coverage metrics."><p class="muted-copy">"Loading..."</p></SurfaceCard> }>
                    {move || coverage.get().map(|result| match result {
                        Ok(value) => view! {
                            <SurfaceCard title="POI Coverage" subtitle="Coverage metrics from `/api/admin/poi-coverage`.">
                                <div class="timeline-list">
                                    <div class="timeline-item"><strong>"Total POIs"</strong><span>{number_field(&value, "total_pois")}</span></div>
                                    <div class="timeline-item"><strong>"Average Priority"</strong><span>{number_field(&value, "avg_priority")}</span></div>
                                </div>
                            </SurfaceCard>
                        }.into_view(),
                        Err(message) => view! { <SurfaceCard title="POI Coverage" subtitle="The API request failed. Try refreshing the page."><p class="error-copy">{message}</p></SurfaceCard> }.into_view(),
                    })}
                </Suspense>
            </div>
        </div>
    }
}
