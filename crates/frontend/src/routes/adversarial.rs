use apex_shared::{
    PlacementAlert, QuarantineItem, SourceReliabilityHistory, SourceReliabilityPoint,
    SourceReliabilityTier,
};
use chrono::{Duration, Utc};
use leptos::*;

use crate::components::{
    badges::source_reliability_badge::SourceReliabilityBadge,
    cards::{PageHeader, SurfaceCard},
    charts::source_entropy_gauge::SourceEntropyGauge,
    panels::QuarantineList,
};

fn sample_placements() -> Vec<PlacementAlert> {
    vec![PlacementAlert {
        id: "placement-1".to_string(),
        source_count: 4,
        time_window_hours: 12,
        token_jaccard: 0.83,
        signal_ids: vec![
            "sig-112".to_string(),
            "sig-118".to_string(),
            "sig-129".to_string(),
        ],
        created_at: Utc::now(),
    }]
}

fn sample_quarantine() -> Vec<QuarantineItem> {
    vec![QuarantineItem {
        id: "quarantine-1".to_string(),
        reason: "Source entropy anomaly with duplicated claim language".to_string(),
        release_at: Utc::now() + Duration::hours(19),
        source_domain: "example-trade-press.test".to_string(),
    }]
}

fn sample_history() -> SourceReliabilityHistory {
    SourceReliabilityHistory {
        domain: "example-trade-press.test".to_string(),
        tier: SourceReliabilityTier::TradePress,
        promotion_candidate: true,
        history: vec![
            SourceReliabilityPoint {
                observed_reliability: 0.61,
                effective_reliability: 0.43,
                recorded_at: Utc::now() - Duration::days(14),
            },
            SourceReliabilityPoint {
                observed_reliability: 0.69,
                effective_reliability: 0.51,
                recorded_at: Utc::now() - Duration::days(7),
            },
            SourceReliabilityPoint {
                observed_reliability: 0.78,
                effective_reliability: 0.63,
                recorded_at: Utc::now(),
            },
        ],
    }
}

#[component]
pub fn AdversarialPage() -> impl IntoView {
    let placements = sample_placements();
    let quarantine = sample_quarantine();
    let reliability = sample_history();

    view! {
        <div class="page">
            <PageHeader
                eyebrow="Adversarial Detection"
                title="Robustness Dashboard"
                subtitle="Placement clusters, quarantine windows, and source-tier evolution are rendered from shared adversarial structs inside the WASM frontend."
            />

            <div class="two-up">
                <SurfaceCard title="Coordinated Placements" subtitle="Token-similar claims grouped into reviewable clusters.">
                    <div class="placement-list">
                        <For each=move || placements.clone() key=|alert| alert.id.clone() let:alert>
                            <div class="placement-alert-card">
                                <div class="placement-alert-head">
                                    <p class="placement-alert-title">{format!("{} sources inside {}h", alert.source_count, alert.time_window_hours)}</p>
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
                </SurfaceCard>

                <SurfaceCard title="Quarantine Queue" subtitle="Held items retain explicit release timestamps for analyst review.">
                    <QuarantineList items=quarantine />
                </SurfaceCard>
            </div>

            <SurfaceCard title="Source Reliability" subtitle="Tier badging and promotion-candidate highlighting are already available to any route.">
                <div class="timeline-list">
                    <div class="badge-row">
                        <SourceReliabilityBadge tier=reliability.tier.clone() promotion_candidate=reliability.promotion_candidate />
                        <span class="muted-copy">{format!("{} tracked points", reliability.history.len())}</span>
                    </div>
                    <SourceEntropyGauge entropy=0.78 anomaly=true />
                </div>
            </SurfaceCard>
        </div>
    }
}
