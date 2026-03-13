use leptos::*;
use leptos_router::use_params_map;

use crate::{api, components::{cards::{PageHeader, SurfaceCard}, charts::source_entropy_gauge::SourceEntropyGauge}};

#[component]
pub fn CompanyDetailPage() -> impl IntoView {
    let params = use_params_map();
    let company_id = move || params.with(|params| params.get("company_id").cloned().unwrap_or_default());

    let detail = create_resource(company_id, |company_id| async move {
        if company_id.is_empty() {
            Err("Missing company id".to_string())
        } else {
            api::fetch_company_detail(&company_id).await
        }
    });

    view! {
        <div class="page">
            <PageHeader eyebrow="Entity Detail" title="Company Detail" subtitle="Company detail is loaded through the live `/api/companies/:id` endpoint." />
            <Suspense fallback=move || view! { <SurfaceCard title="Company" subtitle="Loading company detail."><p class="muted-copy">"Loading..."</p></SurfaceCard> }>
                {move || detail.get().map(|result| match result {
                    Ok(company) => {
                        let community_badges = company.community_badges.clone();
                        let source_entropy = company.source_entropy;
                        let source_quality_label = company.source_quality_label.clone();
                        let community_signals_view = if community_badges.is_empty() {
                            view! { <></> }.into_view()
                        } else {
                            view! {
                                <div class="timeline-item">
                                    <strong>"Community Signals"</strong>
                                    <div class="badge-row">
                                        <For each=move || community_badges.clone() key=|badge| badge.clone() let:badge>
                                            <span class="source-badge tier-established">{badge}</span>
                                        </For>
                                    </div>
                                </div>
                            }
                            .into_view()
                        };
                        let source_entropy_view = if let Some(source_entropy) = source_entropy {
                            view! {
                                <SourceEntropyGauge entropy=source_entropy anomaly=source_entropy < 0.2 />
                            }
                            .into_view()
                        } else {
                            view! { <></> }.into_view()
                        };
                        let source_quality_view = if let Some(source_quality_label) = source_quality_label {
                            view! {
                                <div class="timeline-item"><strong>"Evidence Quality"</strong><span>{source_quality_label}</span></div>
                            }
                            .into_view()
                        } else {
                            view! { <></> }.into_view()
                        };
                        view! {
                        <div class="two-up">
                            <SurfaceCard title="Profile" subtitle="Core company metadata and capabilities.">
                                <div class="timeline-list">
                                    <div class="timeline-item"><strong>{company.name.clone()}</strong><span class="muted-copy">{format!("{} · {}", company.region, company.country)}</span></div>
                                    <div class="timeline-item"><strong>"Website"</strong><span>{company.website.unwrap_or_else(|| "Unavailable".to_string())}</span></div>
                                    <div class="timeline-item"><strong>"Capabilities"</strong><span>{company.capabilities.join(", ")}</span></div>
                                    <div class="timeline-item"><strong>"Certifications"</strong><span>{company.certifications.join(", ")}</span></div>
                                    {community_signals_view}
                                    {source_entropy_view}
                                    {source_quality_view}
                                </div>
                            </SurfaceCard>
                            <SurfaceCard title="Recent Events" subtitle="Recent company events from the detail API.">
                                <div class="timeline-list">
                                    <For each=move || company.recent_events.clone() key=|event| format!("{}-{}", event.event_type, event.date) let:event>
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
                    Err(message) => view! { <SurfaceCard title="Company" subtitle="The API request failed."><p class="error-copy">{message}</p></SurfaceCard> }.into_view(),
                })}
            </Suspense>
        </div>
    }
}