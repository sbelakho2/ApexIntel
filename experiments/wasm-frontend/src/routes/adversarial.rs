use leptos::*;

use crate::api;
use crate::components::{
    badges::source_reliability_badge::SourceReliabilityBadge,
    cards::{PageHeader, SurfaceCard},
    charts::source_entropy_gauge::SourceEntropyGauge,
    panels::QuarantineList,
};

#[component]
pub fn AdversarialPage() -> impl IntoView {
    let placements_resource = create_resource(
        || (),
        |_| async { api::fetch_adversarial_placements().await },
    );
    let quarantine_resource =
        create_resource(|| (), |_| async { api::fetch_quarantine_items().await });
    let reliability_resource = create_resource(
        || (),
        |_| async { api::fetch_source_reliability_history().await },
    );

    view! {
        <div class="page">
            <PageHeader
                eyebrow="Adversarial Detection"
                title="Robustness Dashboard"
                subtitle="Placement clusters, quarantine windows, and source-tier evolution with live data from the adversarial detection pipeline."
            />

            <div class="two-up">
                <SurfaceCard title="Coordinated Placements" subtitle="Token-similar claims grouped into reviewable clusters.">
                    <Suspense fallback=move || view! { <p class="muted-copy">Loading placements...</p> }>
                        {move || placements_resource.get().map(|result| match result {
                            Ok(placements) => {
                                if placements.is_empty() {
                                    view! { <p class="muted-copy">No coordinated placements detected. The adversarial pipeline is clean.</p> }.into_view()
                                } else {
                                    view! {
                                        <div class="placement-list">
                                            <For each=move || placements.clone() key=|alert| alert.id.clone() let:alert>
                                                <div class="placement-alert-card">
                                                    <div class="placement-alert-head">
                                                        <p class="placement-alert-title">
                                                            {format!("{} sources inside {}h", alert.source_count, alert.time_window_hours)}
                                                        </p>
                                                        <span class="severity-chip severity-high">{"cluster"}</span>
                                                    </div>
                                                    <p class="muted-copy">{format!("Token Jaccard {:.2}", alert.token_jaccard)}</p>
                                                    <div class="code-list">
                                                        <For each=move || alert.signal_ids.clone() key=|signal| signal.clone() let:signal>
                                                            <span class="code-pill">{signal}</span>
                                                        </For>
                                                    </div>
                                                </div>
                                            </For>
                                        </div>
                                    }.into_view()
                                }
                            },
                            Err(err) => view! { <p class="error-copy">Failed to load placements: {err}</p> }.into_view(),
                        })}
                    </Suspense>
                </SurfaceCard>

                <SurfaceCard title="Quarantine Queue" subtitle="Held items retain explicit release timestamps for analyst review.">
                    <Suspense fallback=move || view! { <p class="muted-copy">Loading quarantine queue...</p> }>
                        {move || quarantine_resource.get().map(|result| match result {
                            Ok(items) => {
                                if items.is_empty() {
                                    view! { <p class="muted-copy">No items currently in quarantine.</p> }.into_view()
                                } else {
                                    view! { <QuarantineList items=items /> }.into_view()
                                }
                            },
                            Err(err) => view! { <p class="error-copy">Failed to load quarantine: {err}</p> }.into_view(),
                        })}
                    </Suspense>
                </SurfaceCard>
            </div>

            <SurfaceCard title="Source Reliability" subtitle="Tier badging and promotion-candidate highlighting with live reliability data.">
                <Suspense fallback=move || view! { <p class="muted-copy">Loading source reliability...</p> }>
                    {move || reliability_resource.get().map(|result| match result {
                        Ok(reliability) => {
                            view! {
                                <div class="timeline-list">
                                    <div class="badge-row">
                                        <SourceReliabilityBadge
                                            tier=reliability.tier.clone()
                                            promotion_candidate=reliability.promotion_candidate
                                        />
                                        <span class="muted-copy">{format!("{} tracked points", reliability.history.len())}</span>
                                    </div>
                                    <SourceEntropyGauge
                                        entropy=reliability.history.last().map(|p| p.effective_reliability).unwrap_or(0.0)
                                        anomaly=reliability.history.last().map(|p| p.effective_reliability < 0.5).unwrap_or(false)
                                    />
                                </div>
                            }.into_view()
                        },
                        Err(err) => view! { <p class="error-copy">Failed to load reliability history: {err}</p> }.into_view(),
                    })}
                </Suspense>
            </SurfaceCard>
        </div>
    }
}
