use leptos::*;

use crate::api;
use crate::components::{
    cards::{PageHeader, StatCard, SurfaceCard},
    charts::probability_gauge::ProbabilityGauge,
};

fn render_warning_preview(items: Vec<api::WarningRecord>) -> View {
    let preview_items = items.into_iter().take(3).collect::<Vec<_>>();

    view! {
        <SurfaceCard title="Recent Warnings" subtitle="The landing route now incorporates the live warning feed.">
            <div class="warning-list">
                <For each=move || preview_items.clone() key=|item| item.id.clone() let:item>
                    <article class="warning-item">
                        <div class="warning-item-head">
                            <div>
                                <p class="warning-title">{item.title.clone()}</p>
                                <p class="warning-meta">{format!("{} · {}", item.warning_type, item.region)}</p>
                            </div>
                            <span class=format!("severity-chip severity-{}", item.severity.to_lowercase())>{item.severity.clone()}</span>
                        </div>
                        <div class="metric-row">
                            <ProbabilityGauge probability=item.confidence.clamp(0.0, 1.0) />
                            <span class="muted-copy">{format!("Confidence {:.0}%", item.confidence * 100.0)}</span>
                        </div>
                    </article>
                </For>
            </div>
        </SurfaceCard>
    }
}

#[component]
pub fn OverviewPage() -> impl IntoView {
    let dashboard = create_resource(|| (), |_| async { api::fetch_dashboard().await });
    let warnings = create_resource(|| (), |_| async { api::fetch_warnings(1, None).await });

    view! {
        <div class="page">
            <PageHeader
                eyebrow="Operational Overview"
                title="Analytical Overview"
                subtitle="Overview statistics and recent warnings now load from live dashboard and warning endpoints instead of static demo data."
            />

            <Suspense fallback=move || view! { <SurfaceCard title="Overview" subtitle="Loading dashboard summary."><p class="muted-copy">"Loading..."</p></SurfaceCard> }>
                {move || dashboard.get().map(|result| match result {
                    Ok(stats) => view! {
                        <div class="stat-grid">
                            <StatCard label="Warnings" value=stats.total_warnings.to_string() delta=format!("{} unacknowledged", stats.unacknowledged_warnings)>
                                <ProbabilityGauge probability=(stats.unacknowledged_warnings as f64 / stats.total_warnings.max(1) as f64).clamp(0.0, 1.0) />
                            </StatCard>
                            <StatCard label="Insights" value=stats.total_insights.to_string() delta=format!("{} new in 24h", stats.new_insights_24h)>
                                <div class="stat-mini">"Insights"</div>
                            </StatCard>
                            <StatCard label="Companies" value=stats.total_companies.to_string() delta=format!("{} persons tracked", stats.total_persons)>
                                <div class="stat-mini">"Coverage"</div>
                            </StatCard>
                            <StatCard label="Recipes" value=stats.active_recipes.to_string() delta=format!("{} warnings in 24h", stats.new_warnings_24h)>
                                <div class="stat-mini">"Rules"</div>
                            </StatCard>
                        </div>
                    }.into_view(),
                    Err(message) => view! { <SurfaceCard title="Overview" subtitle="The dashboard request failed."><p class="error-copy">{message}</p></SurfaceCard> }.into_view(),
                })}
            </Suspense>

            <Suspense fallback=move || view! { <SurfaceCard title="Recent Warnings" subtitle="Loading live warning feed."><p class="muted-copy">"Loading..."</p></SurfaceCard> }>
                {move || warnings.get().map(|result| match result {
                    Ok(payload) => render_warning_preview(payload.items),
                    Err(message) => view! { <SurfaceCard title="Recent Warnings" subtitle="The warning request failed."><p class="error-copy">{message}</p></SurfaceCard> }.into_view(),
                })}
            </Suspense>
        </div>
    }
}