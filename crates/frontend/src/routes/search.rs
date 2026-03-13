use leptos::*;

use crate::{api, components::cards::{PageHeader, SurfaceCard}};

#[component]
pub fn SearchPage() -> impl IntoView {
    let (query, set_query) = create_signal(String::new());
    let (page, _set_page) = create_signal(1u32);

    let results = create_resource(
        move || (page.get(), query.get()),
        |(page, query)| async move {
            if query.trim().is_empty() {
                Ok(api::SearchResponse::default())
            } else {
                api::fetch_search(page, &query).await
            }
        },
    );

    view! {
        <div class="page">
            <PageHeader eyebrow="Discovery" title="Search" subtitle="The search page now queries the live search API directly from the WASM shell." />
            <SurfaceCard title="Search Query" subtitle="Results update from the current `/api/search` endpoint.">
                <input class="search-input" type="search" placeholder="Search entities, insights, or titles" prop:value=query on:input=move |ev| set_query.set(event_target_value(&ev)) />
            </SurfaceCard>
            <Suspense fallback=move || view! { <SurfaceCard title="Search Results" subtitle="Loading results."><p class="muted-copy">"Loading..."</p></SurfaceCard> }>
                {move || results.get().map(|result| match result {
                    Ok(payload) => view! {
                        <SurfaceCard title="Results" subtitle="Top ranked hits from the live search backend.">
                            <div class="timeline-list">
                                <For each=move || payload.results.clone() key=|item| item.id.clone() let:item>
                                    <article class="timeline-item">
                                        <strong>{item.title.clone()}</strong>
                                        <span class="muted-copy">{format!("{} · {}", item.entity_type, item.region.unwrap_or_else(|| "Global".to_string()))}</span>
                                        <span>{item.snippet.clone()}</span>
                                    </article>
                                </For>
                            </div>
                        </SurfaceCard>
                    }.into_view(),
                    Err(message) => view! { <SurfaceCard title="Search" subtitle="The API request failed."><p class="error-copy">{message}</p></SurfaceCard> }.into_view(),
                })}
            </Suspense>
        </div>
    }
}