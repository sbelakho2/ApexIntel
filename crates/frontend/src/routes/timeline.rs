use apex_shared::{EventTimeline, TimelineEvent};
use chrono::Utc;
use leptos::*;
use leptos_router::*;

use crate::api::get_json;
use crate::components::{
    cards::{PageHeader, SurfaceCard},
    panels::EntityTimeline,
};

#[derive(serde::Deserialize, Clone, Debug)]
struct TimelineResponse {
    entity_name: String,
    events: Vec<TimelineEventResponse>,
}

#[derive(serde::Deserialize, Clone, Debug)]
struct TimelineEventResponse {
    event_type: String,
    date: String,
    description: String,
}

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

    let timeline = create_resource(entity_id, |eid| async move {
        if eid == "entity-demo" {
            return EventTimeline {
                entity_id: eid,
                entity_name: "No entity selected".to_string(),
                events: vec![],
            };
        }
        let path = format!("/api/entities/{}/timeline", eid);
        match get_json::<TimelineResponse>(&path).await {
            Ok(data) => EventTimeline {
                entity_id: eid,
                entity_name: data.entity_name,
                events: data
                    .events
                    .into_iter()
                    .map(|e| TimelineEvent {
                        event_type: e.event_type,
                        date: chrono::DateTime::parse_from_rfc3339(&e.date)
                            .map(|dt| dt.with_timezone(&Utc))
                            .unwrap_or_else(|_| Utc::now()),
                        description: e.description,
                    })
                    .collect(),
            },
            Err(_) => EventTimeline {
                entity_id: eid,
                entity_name: "Entity data unavailable".to_string(),
                events: vec![],
            },
        }
    });

    view! {
        <div class="page">
            <PageHeader
                eyebrow="Entity Timeline"
                title="Event Timeline"
                subtitle="Chronological entity activity from observed signals, job postings, and intelligence reports."
            />

            <SurfaceCard title="Tracked Events" subtitle="Real-time entity event tracking from monitored sources.">
                <Suspense fallback=move || view! {
                    <div class="empty-state">
                        <div class="apex-spinner"></div>
                        <p class="muted-copy">"Loading timeline data..."</p>
                    </div>
                }>
                    {move || timeline.get().map(|t| view! {
                        <EntityTimeline timeline=t />
                    })}
                </Suspense>
            </SurfaceCard>
        </div>
    }
}
