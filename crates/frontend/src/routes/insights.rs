use leptos::*;

use crate::{
    api,
    components::{cards::{PageHeader, SurfaceCard}, charts::sparkline::Sparkline, filters::{FilterBar, FilterChip, Pagination}},
};

#[component]
pub fn InsightsPage() -> impl IntoView {
    let (page, set_page) = create_signal(1u32);
    let bookmarked = create_rw_signal(false);

    let insights = create_resource(
        move || (page.get(), bookmarked.get()),
        |(page, bookmarked)| async move { api::fetch_insights(page, bookmarked).await },
    );

    view! {
        <div class="page">
            <PageHeader
                eyebrow="Intelligence Feed"
                title="Insights"
                subtitle="The WASM route now reads the paged insights API directly, including bookmark state and tags."
            />

            <FilterBar title="Scope">
                <FilterChip label="All".to_string() active=Signal::derive(move || !bookmarked.get()) on_click=Callback::new(move |_| { bookmarked.set(false); set_page.set(1); }) />
                <FilterChip label="Bookmarked".to_string() active=Signal::derive(move || bookmarked.get()) on_click=Callback::new(move |_| { bookmarked.set(true); set_page.set(1); }) />
            </FilterBar>

            <Suspense fallback=move || view! { <SurfaceCard title="Insights" subtitle="Loading insight feed."><p class="muted-copy">"Loading..."</p></SurfaceCard> }>
                {move || insights.get().map(|result| match result {
                    Ok(payload) => view! {
                        <SurfaceCard title="Latest Insights" subtitle="Confidence, tags, and bookmark state come from the live API response.">
                            <div class="timeline-list">
                                <For each=move || payload.items.clone() key=|item| item.id.clone() let:item>
                                    {let diversity_badge = item.diversity_label.clone().map(|label| view! { <span class="source-badge tier-trade-press">{label}</span> }.into_view()).unwrap_or_else(|| view! { <></> }.into_view()); let causal_badge = item.causal_flag.clone().map(|label| view! { <span class="severity-chip severity-medium">{label}</span> }.into_view()).unwrap_or_else(|| view! { <></> }.into_view()); view! {
                                        <article class="timeline-item">
                                            <strong>{item.title.clone()}</strong>
                                            <span class="muted-copy">{format!("{} · {} · {:.0}% confidence", item.insight_type, item.region, item.confidence * 100.0)}</span>
                                            <span>{item.summary.clone()}</span>
                                            <div class="metric-row">
                                                <Sparkline values=item.information_gain_sparkline.clone() label=format!("Information gain trend for {}", item.title) />
                                                <span class="muted-copy">{format!("IG {:.2} bits", item.information_gain_bits.unwrap_or_default())}</span>
                                                {diversity_badge}
                                                {causal_badge}
                                            </div>
                                            <div class="code-list">
                                                <For each=move || item.tags.clone() key=|tag| tag.clone() let:tag>
                                                    <span class="code-pill">{tag}</span>
                                                </For>
                                                <Show when=move || item.bookmarked.unwrap_or(false)>
                                                    <span class="severity-chip severity-low">"bookmarked"</span>
                                                </Show>
                                            </div>
                                        </article>
                                    }}
                                </For>
                            </div>
                            <Pagination page=page total=payload.total per_page=payload.per_page set_page=set_page />
                        </SurfaceCard>
                    }
                    .into_view(),
                    Err(message) => view! { <SurfaceCard title="Insights" subtitle="The API request failed."><p class="error-copy">{message}</p></SurfaceCard> }.into_view(),
                })}
            </Suspense>
        </div>
    }
}