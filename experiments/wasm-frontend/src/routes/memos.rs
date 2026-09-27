use leptos::*;

use crate::{
    api,
    components::cards::{PageHeader, SurfaceCard},
};

#[component]
pub fn MemosPage() -> impl IntoView {
    let latest = create_resource(|| (), |_| async { api::fetch_weekly_memo().await });
    let memos = create_resource(|| (), |_| async { api::fetch_memos(1).await });

    view! {
        <div class="page">
            <PageHeader eyebrow="Weekly Synthesis" title="Memos" subtitle="Weekly memo list and latest brief are now loaded from the live memo APIs." />
            <div class="two-up">
                <Suspense fallback=move || view! { <SurfaceCard title="Latest Memo" subtitle="Loading memo brief."><p class="muted-copy">"Loading..."</p></SurfaceCard> }>
                    {move || latest.get().map(|result| match result {
                        Ok(memo) => view! {
                            <SurfaceCard title="Latest Memo" subtitle="Current weekly strategic summary.">
                                <div class="timeline-list">
                                    <div class="timeline-item"><strong>{memo.title.clone()}</strong><span class="muted-copy">{format!("{} to {}", memo.week_start, memo.week_end)}</span></div>
                                    <div class="timeline-item"><strong>"Executive Summary"</strong><span>{memo.executive_summary.clone()}</span></div>
                                    <For each=move || memo.action_items.clone() key=|item| format!("{}-{}", item.owner, item.action) let:item>
                                        <div class="timeline-item"><strong>{item.owner.clone()}</strong><span>{item.action.clone()}</span></div>
                                    </For>
                                </div>
                            </SurfaceCard>
                        }.into_view(),
                        Err(message) => view! { <SurfaceCard title="Latest Memo" subtitle="The API request failed. Try refreshing the page."><p class="error-copy">{message}</p></SurfaceCard> }.into_view(),
                    })}
                </Suspense>
                <Suspense fallback=move || view! { <SurfaceCard title="Memo History" subtitle="Loading memo archive."><p class="muted-copy">"Loading..."</p></SurfaceCard> }>
                    {move || memos.get().map(|result| match result {
                        Ok(payload) => view! {
                            <SurfaceCard title="Memo History" subtitle="Recent memo entries from `/api/memos`.">
                                <div class="timeline-list">
                                    <For each=move || payload.items.clone() key=|item| item.id.clone() let:item>
                                        <div class="timeline-item">
                                            <strong>{item.title.clone()}</strong>
                                            <span class="muted-copy">{item.generated_at.clone()}</span>
                                            <span>{item.executive_summary.clone()}</span>
                                        </div>
                                    </For>
                                </div>
                            </SurfaceCard>
                        }.into_view(),
                        Err(message) => view! { <SurfaceCard title="Memo History" subtitle="The API request failed. Try refreshing the page."><p class="error-copy">{message}</p></SurfaceCard> }.into_view(),
                    })}
                </Suspense>
            </div>
        </div>
    }
}
