use leptos::*;
use leptos_router::A;

use crate::{
    api,
    components::{
        cards::{PageHeader, SurfaceCard},
        filters::{FilterBar, FilterChip, Pagination},
    },
};

#[component]
pub fn PersonsPage() -> impl IntoView {
    let (page, set_page) = create_signal(1u32);
    let priority = create_rw_signal(String::new());

    let persons = create_resource(
        move || (page.get(), priority.get()),
        |(page, priority)| async move {
            api::fetch_persons(
                page,
                if priority.is_empty() {
                    None
                } else {
                    Some(priority)
                },
            )
            .await
        },
    );

    let priorities = ["", "A", "B", "C"];

    view! {
        <div class="page">
            <PageHeader eyebrow="POI Coverage" title="Persons" subtitle="The persons list is now served from the live POI list endpoint with priority filters." />
            <FilterBar title="Priority">
                <For each=move || priorities.into_iter() key=|value| value.to_string() let:value>
                    <FilterChip
                        label=if value.is_empty() { "All".to_string() } else { value.to_string() }
                        active=Signal::derive(move || priority.get() == value)
                        on_click=Callback::new(move |_| { priority.set(value.to_string()); set_page.set(1); })
                    />
                </For>
            </FilterBar>
            <Suspense fallback=move || view! { <SurfaceCard title="Persons" subtitle="Loading people of interest."><p class="muted-copy">"Loading..."</p></SurfaceCard> }>
                {move || persons.get().map(|result| match result {
                    Ok(payload) => view! {
                        <SurfaceCard title="People Of Interest" subtitle="Each record links to a live detail view with timeline data.">
                            <div class="timeline-list">
                                <For each=move || payload.items.clone() key=|item| item.id.clone() let:item>
                                    <article class="timeline-item">
                                        <A class="inline-link" href=format!("/persons/{}", item.id)>{item.name.clone()}</A>
                                        <span class="muted-copy">{format!("{} · {} · {}", item.role, item.organization, item.region)}</span>
                                        <span class="muted-copy">{format!("Priority {} · influence {}", item.priority, item.influence_score)}</span>
                                    </article>
                                </For>
                            </div>
                            <Pagination page=page total=payload.total per_page=payload.per_page set_page=set_page />
                        </SurfaceCard>
                    }.into_view(),
                    Err(message) => view! { <SurfaceCard title="Persons" subtitle="The API request failed. Try refreshing the page."><p class="error-copy">{message}</p></SurfaceCard> }.into_view(),
                })}
            </Suspense>
        </div>
    }
}
