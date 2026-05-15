use leptos::*;

use crate::{
    api,
    components::{
        cards::{PageHeader, SurfaceCard},
        filters::{FilterBar, FilterChip, Pagination},
    },
};

#[component]
pub fn RecipesPage() -> impl IntoView {
    let (page, set_page) = create_signal(1u32);
    let status = create_rw_signal(String::new());

    let recipes = create_resource(
        move || (page.get(), status.get()),
        |(page, status)| async move {
            api::fetch_recipes(
                page,
                if status.is_empty() {
                    None
                } else {
                    Some(status)
                },
            )
            .await
        },
    );

    let statuses = ["", "production", "staging", "deprecated"];

    view! {
        <div class="page">
            <PageHeader eyebrow="Recipe Operations" title="Recipes" subtitle="Recipe list migration now uses the live recipe API with client-side status filters and paging." />
            <FilterBar title="Status">
                <For each=move || statuses.into_iter() key=|value| value.to_string() let:value>
                    <FilterChip label=if value.is_empty() { "All".to_string() } else { value.to_string() } active=Signal::derive(move || status.get() == value) on_click=Callback::new(move |_| { status.set(value.to_string()); set_page.set(1); }) />
                </For>
            </FilterBar>
            <Suspense fallback=move || view! { <SurfaceCard title="Recipes" subtitle="Loading recipe list."><p class="muted-copy">"Loading..."</p></SurfaceCard> }>
                {move || recipes.get().map(|result| match result {
                    Ok(payload) => view! {
                        <SurfaceCard title="Recipe Catalog" subtitle="Precision, recall, and FPR are rendered from the live recipe response.">
                            <div class="timeline-list">
                                <For each=move || payload.items.clone() key=|item| item.id.clone() let:item>
                                    <div class="timeline-item">
                                        <strong>{item.name.clone()}</strong>
                                        <span class="muted-copy">{format!("{} · {:.0}% precision · {:.0}% recall", item.status, item.precision * 100.0, item.recall * 100.0)}</span>
                                        <span>{item.description.clone()}</span>
                                    </div>
                                </For>
                            </div>
                            <Pagination page=page total=payload.total per_page=payload.per_page set_page=set_page />
                        </SurfaceCard>
                    }.into_view(),
                    Err(message) => view! { <SurfaceCard title="Recipes" subtitle="The API request failed. Try refreshing the page."><p class="error-copy">{message}</p></SurfaceCard> }.into_view(),
                })}
            </Suspense>
        </div>
    }
}
