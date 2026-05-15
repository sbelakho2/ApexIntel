use apex_shared::{EventTimeline, TimelineEvent};
use chrono::{Duration, Utc};
use leptos::*;
use leptos_router::*;

use crate::components::{
    cards::{PageHeader, SurfaceCard},
    panels::EntityTimeline,
};

#[component]
pub fn TimelinePage() -> impl IntoView {
    let params = use_params_map();
    let entity_id = move || {
        params.with(|map| {
            map.get("entity_id")
                .cloned()
                .unwrap_or_else(|| "entity-demo".to_string())
        })
    };
    let timeline = move || EventTimeline {
        entity_id: entity_id(),
        entity_name: "Benchmark Electronics".to_string(),
        events: vec![
            TimelineEvent {
                event_type: "Hiring spike".to_string(),
                date: Utc::now() - Duration::days(30),
                description: "30-day increase in avionics assembly postings.".to_string(),
            },
            TimelineEvent {
                event_type: "Supplier update".to_string(),
                date: Utc::now() - Duration::days(18),
                description: "New radar subcontractor relationship observed.".to_string(),
            },
            TimelineEvent {
                event_type: "Program award".to_string(),
                date: Utc::now() - Duration::days(4),
                description: "Defense program win inferred from multi-source confirmation."
                    .to_string(),
            },
        ],
    };

    view! {
        <div class="page">
            <PageHeader
                eyebrow="Entity Timeline"
                title="Event Timeline"
                subtitle="Vertical temporal reasoning is now available as a reusable WASM component and can bind to entity-specific routes."
            />

            <SurfaceCard title="Tracked Events" subtitle="Route parameter support is wired through the Leptos router.">
                <EntityTimeline timeline=timeline() />
            </SurfaceCard>
        </div>
    }
}
