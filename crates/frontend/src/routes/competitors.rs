use leptos::*;

use crate::{api, components::cards::{PageHeader, SurfaceCard}};

fn json_string(value: &serde_json::Value, key: &str) -> String {
    value.get(key).and_then(|value| value.as_str()).unwrap_or_default().to_string()
}

#[component]
pub fn CompetitorsPage() -> impl IntoView {
    let competitors = create_resource(|| (), |_| async { api::fetch_competitors(1).await });
    let changes = create_resource(|| (), |_| async { api::fetch_competitor_changes(1).await });

    view! {
        <div class="page">
            <PageHeader eyebrow="Market Tracking" title="Competitors" subtitle="Competitor records and recent change tracking now come from the live competitor APIs." />
            <div class="two-up">
                <Suspense fallback=move || view! { <SurfaceCard title="Competitors" subtitle="Loading competitor list."><p class="muted-copy">"Loading..."</p></SurfaceCard> }>
                    {move || competitors.get().map(|result| match result {
                        Ok(payload) => view! {
                            <SurfaceCard title="Tracked Competitors" subtitle="Current competitor entities from `/api/competitors`.">
                                <div class="timeline-list">
                                    <For each=move || payload.items.clone() key=|item| item.id.clone() let:item>
                                        <div class="timeline-item">
                                            <strong>{item.name.clone()}</strong>
                                            <span class="muted-copy">{format!("{} · threat {:.2}", item.region, item.threat_score.unwrap_or_default())}</span>
                                        </div>
                                    </For>
                                </div>
                            </SurfaceCard>
                        }.into_view(),
                        Err(message) => view! { <SurfaceCard title="Competitors" subtitle="The API request failed."><p class="error-copy">{message}</p></SurfaceCard> }.into_view(),
                    })}
                </Suspense>
                <Suspense fallback=move || view! { <SurfaceCard title="Recent Changes" subtitle="Loading competitor changes."><p class="muted-copy">"Loading..."</p></SurfaceCard> }>
                    {move || changes.get().map(|result| match result {
                        Ok(payload) => view! {
                            <SurfaceCard title="Recent Changes" subtitle="Current change stream from `/api/competitors/changes`.">
                                <div class="timeline-list">
                                    <For each=move || payload.items.clone() key=|item| item.to_string() let:item>
                                        <div class="timeline-item">
                                            <strong>{json_string(&item, "competitor_name")}</strong>
                                            <span class="muted-copy">{json_string(&item, "change_type")}</span>
                                            <span>{json_string(&item, "description")}</span>
                                        </div>
                                    </For>
                                </div>
                            </SurfaceCard>
                        }.into_view(),
                        Err(message) => view! { <SurfaceCard title="Recent Changes" subtitle="The API request failed."><p class="error-copy">{message}</p></SurfaceCard> }.into_view(),
                    })}
                </Suspense>
            </div>
        </div>
    }
}