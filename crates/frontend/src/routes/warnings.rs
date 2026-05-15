use leptos::*;

use crate::{
    api,
    components::{
        badges::bayesian_badge::BayesianBadge,
        cards::{PageHeader, SurfaceCard},
        charts::probability_gauge::ProbabilityGauge,
        filters::{FilterBar, FilterChip, Pagination},
    },
};

#[component]
pub fn WarningsPage() -> impl IntoView {
    let (page, set_page) = create_signal(1u32);
    let severity = create_rw_signal(String::new());

    let warnings = create_resource(
        move || (page.get(), severity.get()),
        |(page, severity)| async move {
            api::fetch_warnings(
                page,
                if severity.is_empty() {
                    None
                } else {
                    Some(severity)
                },
            )
            .await
        },
    );

    let severities = ["", "critical", "high", "medium", "low"];

    view! {
        <div class="page">
            <PageHeader
                eyebrow="Operational Feed"
                title="Warnings"
                subtitle="Operational risk signals filtered by severity with calibrated confidence scoring."
            />

            <FilterBar title="Severity">
                <For each=move || severities.into_iter() key=|value| value.to_string() let:value>
                    <FilterChip
                        label=if value.is_empty() { "All".to_string() } else { value.to_uppercase() }
                        active=Signal::derive(move || severity.get() == value)
                        on_click=Callback::new(move |_| {
                            severity.set(value.to_string());
                            set_page.set(1);
                        })
                    />
                </For>
            </FilterBar>

            <Suspense fallback=move || view! { <SurfaceCard title="Warnings" subtitle="Loading live warning feed."><p class="muted-copy">"Loading..."</p></SurfaceCard> }>
                {move || warnings.get().map(|result| match result {
                    Ok(payload) => view! {
                        <SurfaceCard title="Latest Warnings" subtitle="Severity-rated warnings with calibrated probability and Bayesian interpretation.">
                            <div class="warning-list">
                                <For each=move || payload.items.clone() key=|item| item.id.clone() let:item>
                                    {let severity_badge = format!("severity-chip severity-{}", item.severity.to_lowercase()); let severity_text = item.severity.clone(); let calibrated_probability = item.calibrated_probability.unwrap_or(item.confidence); let bayesian_badge = item.bayesian_interpretation.clone().map(|interpretation| view! { <BayesianBadge interpretation=interpretation /> }.into_view()).unwrap_or_else(|| ().into_view()); let evidence_quality_badge = item.evidence_quality_label.clone().map(|label| view! { <span class="source-badge tier-established">{label}</span> }.into_view()).unwrap_or_else(|| ().into_view()); let confidence_interval_copy = item.confidence_interval.clone(); let confidence_interval_view = confidence_interval_copy.as_ref().map(|interval| format!("95% CI {:.0}-{:.0}%", interval.lower * 100.0, interval.upper * 100.0)).map(|text| view! { <span class="muted-copy">{text}</span> }.into_view()).unwrap_or_else(|| ().into_view()); view! {
                                    <article class="warning-item">
                                        <div class="warning-item-head">
                                            <div>
                                                <p class="warning-title">{item.title.clone()}</p>
                                                <p class="warning-meta">{format!("{} · {} · {}", item.warning_type, item.region, item.ts_utc)}</p>
                                            </div>
                                            <span class=severity_badge>{severity_text}</span>
                                        </div>
                                        <div class="metric-row">
                                            <ProbabilityGauge probability=calibrated_probability.clamp(0.0, 1.0) />
                                            <span class="muted-copy">{format!("Confidence {:.0}%", item.confidence * 100.0)}</span>
                                            <Show when=move || item.calibrated_probability.is_some()>
                                                <span class="muted-copy">{format!("Calibrated {:.0}%", calibrated_probability * 100.0)}</span>
                                            </Show>
                                            <span class="muted-copy">{if item.acknowledged { "Acknowledged" } else { "Open" }}</span>
                                        </div>
                                        <div class="badge-row">
                                            {bayesian_badge}
                                            {evidence_quality_badge}
                                            {confidence_interval_view}
                                        </div>
                                        <p class="muted-copy">{item.description.clone()}</p>
                                    </article>
                                    }}
                                </For>
                            </div>
                            <Pagination page=page total=payload.total per_page=payload.per_page set_page=set_page />
                        </SurfaceCard>
                    }
                    .into_view(),
                    Err(message) => view! {
                        <SurfaceCard title="Warnings" subtitle="The API request failed. Try refreshing the page.">
                            <p class="error-copy">{message}</p>
                        </SurfaceCard>
                    }
                    .into_view(),
                })}
            </Suspense>
        </div>
    }
}
