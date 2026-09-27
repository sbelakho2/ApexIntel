use leptos::*;
use leptos_router::use_params_map;

use crate::{
    api,
    components::cards::{PageHeader, SurfaceCard},
};

#[component]
pub fn PersonDetailPage() -> impl IntoView {
    let params = use_params_map();
    let person_id =
        move || params.with(|params| params.get("person_id").cloned().unwrap_or_default());

    let detail = create_resource(person_id, |person_id| async move {
        if person_id.is_empty() {
            Err("Missing person id".to_string())
        } else {
            api::fetch_person_detail(&person_id).await
        }
    });

    view! {
        <div class="page">
            <PageHeader eyebrow="POI Detail" title="Person Detail" subtitle="This route consumes the live person detail endpoint and renders the event timeline directly in WASM." />
            <Suspense fallback=move || view! { <SurfaceCard title="Person" subtitle="Loading person detail."><p class="muted-copy">"Loading..."</p></SurfaceCard> }>
                {move || detail.get().map(|result| match result {
                    Ok(person) => {
                        let person_bio = person.bio.clone().unwrap_or_default();
                        let has_bio = !person_bio.is_empty();
                        view! {
                        <div class="two-up">
                            <SurfaceCard title="Profile" subtitle="Live priority and engagement metadata.">
                                <div class="timeline-list">
                                    <div class="timeline-item"><strong>{person.name.clone()}</strong><span class="muted-copy">{format!("{} · {}", person.role, person.organization)}</span></div>
                                    <div class="timeline-item"><strong>"Priority"</strong><span>{format!("{} ({:.0}%)", person.priority, person.priority_score * 100.0)}</span></div>
                                    <div class="timeline-item"><strong>"Coverage"</strong><span>{format!("{} warnings · {} insights", person.warning_count, person.insight_count)}</span></div>
                                    {if has_bio {
                                        view! { <div class="timeline-item"><strong>"Bio"</strong><span>{person_bio.clone()}</span></div> }.into_view()
                                    } else {
                                        View::default()
                                    }}
                                </div>
                            </SurfaceCard>
                            <SurfaceCard title="Timeline" subtitle="The person detail page now includes API-backed event history.">
                                <div class="timeline-list">
                                    <For each=move || person.timeline.clone() key=|event| format!("{}-{}", event.event_type, event.date) let:event>
                                        <div class="timeline-item">
                                            <strong>{event.event_type.clone()}</strong>
                                            <span class="muted-copy">{event.date.clone()}</span>
                                            <span>{event.description.clone()}</span>
                                        </div>
                                    </For>
                                </div>
                            </SurfaceCard>
                        </div>
                    }.into_view()
                    },
                    Err(message) => view! { <SurfaceCard title="Person" subtitle="The API request failed. Try refreshing the page."><p class="error-copy">{message}</p></SurfaceCard> }.into_view(),
                })}
            </Suspense>
        </div>
    }
}
