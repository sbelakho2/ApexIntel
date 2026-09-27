use leptos::*;

use crate::{
    api,
    components::cards::{PageHeader, StatCard, SurfaceCard},
};

#[component]
pub fn SecurityPage() -> impl IntoView {
    let summary = create_resource(|| (), |_| async { api::fetch_security_summary().await });

    view! {
        <div class="page">
            <PageHeader eyebrow="Security Posture" title="Security" subtitle="Security summary cards are now backed by the live `/api/security` endpoint." />
            <Suspense fallback=move || view! { <SurfaceCard title="Security" subtitle="Loading security summary."><p class="muted-copy">"Loading..."</p></SurfaceCard> }>
                {move || summary.get().map(|result| match result {
                    Ok(summary) => view! {
                        <div class="stat-grid">
                            <StatCard label="DNS Posture" value=format!("{:.0}%", summary.dns_posture_score) delta="live summary".to_string()>
                                <div class="stat-mini">"DNS"</div>
                            </StatCard>
                            <StatCard label="Lookalikes" value=summary.lookalike_domains_detected.to_string() delta="detected domains".to_string()>
                                <div class="stat-mini">"Typosquats"</div>
                            </StatCard>
                            <StatCard label="KEV Matches" value=summary.kev_matches.to_string() delta="current relevance".to_string()>
                                <div class="stat-mini">"KEV"</div>
                            </StatCard>
                        </div>
                    }.into_view(),
                    Err(message) => view! { <SurfaceCard title="Security" subtitle="The API request failed. Try refreshing the page."><p class="error-copy">{message}</p></SurfaceCard> }.into_view(),
                })}
            </Suspense>
        </div>
    }
}
