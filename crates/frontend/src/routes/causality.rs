use apex_shared::{BandClass, ConfidenceInterval, GrangerCausalPair, PredictiveAlert, SurvivalPoint};
use leptos::*;

use crate::api;
use crate::components::{
    cards::{PageHeader, SurfaceCard},
    charts::{causal_graph::CausalGraph, survival_curve::SurvivalCurve},
    panels::PredictiveAlertList,
};

#[component]
pub fn CausalityPage() -> impl IntoView {
    let pairs_resource =
        create_resource(|| (), |_| async { api::fetch_causal_pairs().await });
    let alerts_resource =
        create_resource(|| (), |_| async { api::fetch_predictive_alerts().await });
    let survival_resource =
        create_resource(|| (), |_| async { api::fetch_survival_points().await });

    view! {
        <div class="page">
            <PageHeader
                eyebrow="Temporal Reasoning"
                title="Causal And Predictive Views"
                subtitle="Directed causal rendering, predictive alert cards, and survival-style timeline charts with live data from the temporal analysis pipeline."
            />

            <div class="two-up">
                <SurfaceCard title="Granger Pairs" subtitle="Directed relationships with lag and p-value labels from live Granger causality analysis.">
                    <Suspense fallback=move || view! { <p class="muted-copy">Loading causal pairs...</p> }>
                        {move || pairs_resource.get().map(|result| match result {
                            Ok(pairs) => {
                                if pairs.is_empty() {
                                    view! { <p class="muted-copy">No significant Granger-causal relationships detected yet. More temporal data needed for causality testing.</p> }.into_view()
                                } else {
                                    view! { <CausalGraph pairs=pairs /> }.into_view()
                                }
                            },
                            Err(err) => view! { <p class="error-copy">Failed to load causal pairs: {err}</p> }.into_view(),
                        })}
                    </Suspense>
                </SurfaceCard>

                <SurfaceCard title="Predictive Alerts" subtitle="Shared confidence intervals rendered inline with predicted windows from live data.">
                    <Suspense fallback=move || view! { <p class="muted-copy">Loading predictive alerts...</p> }>
                        {move || alerts_resource.get().map(|result| match result {
                            Ok(alerts) => {
                                if alerts.is_empty() {
                                    view! { <p class="muted-copy">No predictive alerts triggered. The system is monitoring for leading-indicator patterns.</p> }.into_view()
                                } else {
                                    view! { <PredictiveAlertList alerts=alerts /> }.into_view()
                                }
                            },
                            Err(err) => view! { <p class="error-copy">Failed to load alerts: {err}</p> }.into_view(),
                        })}
                    </Suspense>
                </SurfaceCard>
            </div>

            <SurfaceCard title="Survival Curve" subtitle="Kaplan-Meier style step rendering for churn or disruption timing from live survival analysis.">
                <Suspense fallback=move || view! { <p class="muted-copy">Loading survival data...</p> }>
                    {move || survival_resource.get().map(|result| match result {
                        Ok(points) => {
                            if points.is_empty() {
                                view! { <p class="muted-copy">Survival data is being accumulated. Check back after the system has processed more temporal records.</p> }.into_view()
                            } else {
                                view! { <SurvivalCurve points=points /> }.into_view()
                            }
                        },
                        Err(err) => view! { <p class="error-copy">Failed to load survival data: {err}</p> }.into_view(),
                    })}
                </Suspense>
            </SurfaceCard>
        </div>
    }
}