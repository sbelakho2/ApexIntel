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
pub fn CompaniesPage() -> impl IntoView {
    let (page, set_page) = create_signal(1u32);
    let competitor_only = create_rw_signal(false);

    let companies = create_resource(
        move || (page.get(), competitor_only.get()),
        |(page, competitor_only)| async move { api::fetch_companies(page, competitor_only).await },
    );

    view! {
        <div class="page">
            <PageHeader
                eyebrow="Entity Coverage"
                title="Companies"
                subtitle="Tracked entities with threat scores, capabilities, and source entropy."
            />

            <FilterBar title="Type">
                <FilterChip label="All".to_string() active=Signal::derive(move || !competitor_only.get()) on_click=Callback::new(move |_| { competitor_only.set(false); set_page.set(1); }) />
                <FilterChip label="Competitors".to_string() active=Signal::derive(move || competitor_only.get()) on_click=Callback::new(move |_| { competitor_only.set(true); set_page.set(1); }) />
            </FilterBar>

            <Suspense fallback=move || view! { <SurfaceCard title="Companies" subtitle="Loading companies."><p class="muted-copy">"Loading..."</p></SurfaceCard> }>
                {move || companies.get().map(|result| match result {
                    Ok(payload) => view! {
                        <SurfaceCard title="Tracked Companies" subtitle="Click any company to view its full dossier and intelligence timeline.">
                            <div class="timeline-list">
                                <For each=move || payload.items.clone() key=|item| item.id.clone() let:item>
                                    <article class="timeline-item">
                                        <A class="inline-link" href=format!("/companies/{}", item.id)>{item.name.clone()}</A>
                                        <span class="muted-copy">{format!("{} · {} · {}", item.entity_type, item.region, item.country)}</span>
                                        <span class="muted-copy">{format!("Threat {} · updated {}", item.threat_score.unwrap_or_default(), item.updated_at)}</span>
                                        <Show when=move || item.source_entropy.is_some()>
                                            <span class="muted-copy">{format!("Source entropy {:.0}%", item.source_entropy.unwrap_or_default() * 100.0)}</span>
                                        </Show>
                                        <div class="badge-row">
                                            <For each=move || item.community_badges.clone() key=|badge| badge.clone() let:badge>
                                                <span class="source-badge tier-established">{badge}</span>
                                            </For>
                                        </div>
                                        <div class="code-list">
                                            <For each=move || item.capabilities.clone() key=|cap| cap.clone() let:cap>
                                                <span class="code-pill">{cap}</span>
                                            </For>
                                        </div>
                                    </article>
                                </For>
                            </div>
                            <Pagination page=page total=payload.total per_page=payload.per_page set_page=set_page />
                        </SurfaceCard>
                    }.into_view(),
                    Err(message) => view! { <SurfaceCard title="Companies" subtitle="The API request failed. Try refreshing the page."><p class="error-copy">{message}</p></SurfaceCard> }.into_view(),
                })}
            </Suspense>
        </div>
    }
}
