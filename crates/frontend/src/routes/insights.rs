use leptos::*;

use crate::{
    api,
    components::{
        cards::{PageHeader, SurfaceCard},
        charts::sparkline::Sparkline,
        filters::{FilterBar, FilterChip, Pagination},
    },
};

#[component]
pub fn InsightsPage() -> impl IntoView {
    let (page, set_page) = create_signal(1u32);
    let bookmarked = create_rw_signal(false);
    let (refresh_nonce, set_refresh_nonce) = create_signal(0u64);
    let (action_error, set_action_error) = create_signal(None::<String>);

    let insights = create_resource(
        move || (page.get(), bookmarked.get(), refresh_nonce.get()),
        |(page, bookmarked, _)| async move { api::fetch_insights(page, bookmarked).await },
    );

    let toggle_bookmark = {
        move |insight_id: String, is_bookmarked: bool| {
            set_action_error.set(None);
            spawn_local(async move {
                match api::set_insight_bookmark(&insight_id, !is_bookmarked).await {
                    Ok(()) => set_refresh_nonce.update(|value| *value += 1),
                    Err(message) => set_action_error.set(Some(message)),
                }
            });
        }
    };

    let submit_feedback = {
        move |insight_id: String, feedback_type: &'static str| {
            set_action_error.set(None);
            spawn_local(async move {
                match api::submit_insight_feedback(&insight_id, feedback_type, None).await {
                    Ok(()) => set_refresh_nonce.update(|value| *value += 1),
                    Err(message) => set_action_error.set(Some(message)),
                }
            });
        }
    };

    view! {
        <div class="page">
            <PageHeader
                eyebrow="Intelligence Feed"
                title="Insights"
                subtitle="Browse, bookmark, and act on intelligence signals across your tracked entities."
            />

            <FilterBar title="Scope">
                <FilterChip label="All".to_string() active=Signal::derive(move || !bookmarked.get()) on_click=Callback::new(move |_| { bookmarked.set(false); set_page.set(1); }) />
                <FilterChip label="Bookmarked".to_string() active=Signal::derive(move || bookmarked.get()) on_click=Callback::new(move |_| { bookmarked.set(true); set_page.set(1); }) />
            </FilterBar>

            <Show when=move || action_error.get().is_some()>
                <SurfaceCard title="Feedback" subtitle="The last insight feedback action failed.">
                    <p class="error-copy">{move || action_error.get().unwrap_or_default()}</p>
                </SurfaceCard>
            </Show>

            <Suspense fallback=move || view! { <SurfaceCard title="Insights" subtitle="Loading insight feed."><p class="muted-copy">"Loading..."</p></SurfaceCard> }>
                {move || insights.get().map(|result| match result {
                    Ok(payload) => view! {
                        <SurfaceCard title="Latest Insights" subtitle="Confidence, tags, and bookmark state come from the live API response.">
                            <div class="timeline-list">
                                <For each=move || payload.items.clone() key=|item| item.id.clone() let:item>
                                    {let diversity_badge = item.diversity_label.clone().map(|label| view! { <span class="source-badge tier-trade-press">{label}</span> }.into_view()).unwrap_or_else(|| ().into_view()); let causal_badge = item.causal_flag.clone().map(|label| view! { <span class="severity-chip severity-medium">{label}</span> }.into_view()).unwrap_or_else(|| ().into_view()); view! {
                                        <article class="timeline-item">
                                            <strong>{item.title.clone()}</strong>
                                            <span class="muted-copy">{format!("{} · {} · {:.0}% confidence", item.insight_type, item.region, item.confidence * 100.0)}</span>
                                            <span>{item.summary.clone()}</span>
                                            <div class="metric-row">
                                                <Sparkline values=item.information_gain_sparkline.clone() label=format!("Information gain trend for {}", item.title) />
                                                <span class="muted-copy">{format!("IG {:.2} bits", item.information_gain_bits.unwrap_or_default())}</span>
                                                <span class="muted-copy">{format!("Quality {:.0}%", item.quality_score.unwrap_or(0.5) * 100.0)}</span>
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
                                            <div class="metric-row">
                                                <button
                                                    type="button"
                                                    class="pagination-button"
                                                    on:click={
                                                        let insight_id = item.id.clone();
                                                        let is_bookmarked = item.bookmarked.unwrap_or(false);
                                                        move |_| toggle_bookmark(insight_id.clone(), is_bookmarked)
                                                    }
                                                >
                                                    {if item.bookmarked.unwrap_or(false) { "Unbookmark" } else { "Bookmark" }}
                                                </button>
                                                <button
                                                    type="button"
                                                    class="pagination-button"
                                                    on:click={
                                                        let insight_id = item.id.clone();
                                                        move |_| submit_feedback(insight_id.clone(), "actioned")
                                                    }
                                                >
                                                    "Actioned"
                                                </button>
                                                <button
                                                    type="button"
                                                    class="pagination-button"
                                                    on:click={
                                                        let insight_id = item.id.clone();
                                                        move |_| submit_feedback(insight_id.clone(), "dismissed")
                                                    }
                                                >
                                                    "Dismiss"
                                                </button>
                                                <button
                                                    type="button"
                                                    class="pagination-button"
                                                    on:click={
                                                        let insight_id = item.id.clone();
                                                        move |_| submit_feedback(insight_id.clone(), "false_positive")
                                                    }
                                                >
                                                    "False Positive"
                                                </button>
                                            </div>
                                        </article>
                                    }}
                                </For>
                            </div>
                            <Pagination page=page total=payload.total per_page=payload.per_page set_page=set_page />
                        </SurfaceCard>
                    }
                    .into_view(),
                    Err(message) => view! { <SurfaceCard title="Insights" subtitle="The API request failed. Try refreshing the page."><p class="error-copy">{message}</p></SurfaceCard> }.into_view(),
                })}
            </Suspense>
        </div>
    }
}
