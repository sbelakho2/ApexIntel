use leptos::*;
use serde_json::Value;

use crate::api;
use crate::components::cards::{PageHeader, SurfaceCard};

#[component]
pub fn StrategicRadarPage() -> impl IntoView {
    let radar_resource = create_resource(|| (), |_| async { api::fetch_strategic_radar().await });

    view! {
        <div class="page">
            <PageHeader
                eyebrow="Strategic Intelligence"
                title="Strategic Radar"
                subtitle="Forward-looking strategic signals: emerging technologies, market shifts, geopolitical currents, and disruptive threats on the horizon."
            />

            <Suspense fallback=move || view! { <SurfaceCard title="Strategic Radar" subtitle="Loading strategic intelligence..."><p class="muted-copy">"Loading..."</p></SurfaceCard> }>
                {move || radar_resource.get().map(|result| match result {
                    Ok(data) => {
                        let signals = data.get("signals")
                            .and_then(|v| v.as_array())
                            .cloned()
                            .unwrap_or_default();
                        let high_prob_high_impact: Vec<_> = signals.iter().filter(|s| {
                            s.get("high_probability").and_then(|v| v.as_bool()).unwrap_or(false)
                            && s.get("high_impact").and_then(|v| v.as_bool()).unwrap_or(false)
                        }).cloned().collect();
                        let low_prob_high_impact: Vec<_> = signals.iter().filter(|s| {
                            s.get("low_probability").and_then(|v| v.as_bool()).unwrap_or(false)
                            && s.get("high_impact").and_then(|v| v.as_bool()).unwrap_or(false)
                        }).cloned().collect();
                        let high_prob_low_impact: Vec<_> = signals.iter().filter(|s| {
                            s.get("high_probability").and_then(|v| v.as_bool()).unwrap_or(false)
                            && s.get("low_impact").and_then(|v| v.as_bool()).unwrap_or(false)
                        }).cloned().collect();
                        let low_prob_low_impact: Vec<_> = signals.iter().filter(|s| {
                            s.get("low_probability").and_then(|v| v.as_bool()).unwrap_or(false)
                            && s.get("low_impact").and_then(|v| v.as_bool()).unwrap_or(false)
                        }).cloned().collect();

                        view! {
                            <div class="radar-quadrants">
                                <SurfaceCard title="High Impact / High Probability" subtitle="Signals to act on now">
                                    <SignalList signals=high_prob_high_impact />
                                </SurfaceCard>
                                <SurfaceCard title="High Impact / Low Probability" subtitle="Contingency planning needed">
                                    <SignalList signals=low_prob_high_impact />
                                </SurfaceCard>
                                <SurfaceCard title="Low Impact / High Probability" subtitle="Monitor for amplification">
                                    <SignalList signals=high_prob_low_impact />
                                </SurfaceCard>
                                <SurfaceCard title="Low Impact / Low Probability" subtitle="Background monitoring">
                                    <SignalList signals=low_prob_low_impact />
                                </SurfaceCard>
                            </div>
                        }.into_view()
                    },
                    Err(message) => view! { <SurfaceCard title="Strategic Radar" subtitle="Failed to load"><p class="error-copy">{message}</p></SurfaceCard> }.into_view(),
                })}
            </Suspense>
        </div>
    }
}

#[component]
fn SignalList(signals: Vec<Value>) -> impl IntoView {
    if signals.is_empty() {
        return view! { <p class="muted-copy">No signals in this quadrant.</p> }.into_view();
    }
    view! {
        <div class="radar-signal-list">
            <For each=move || signals.clone() key=|s| {
                s.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string()
            } let:signal>
                <div class="radar-signal-card">
                    <div class="signal-head">
                        <p class="signal-title">{signal.get("title").and_then(|v| v.as_str()).unwrap_or("Untitled Signal").to_string()}</p>
                        <span class="severity-chip">{signal.get("category").and_then(|v| v.as_str()).unwrap_or("unknown").to_string()}</span>
                    </div>
                    <p class="muted-copy">{signal.get("description").and_then(|v| v.as_str()).unwrap_or("No description").to_string()}</p>
                    <div class="signal-meta">
                        <span class="code-pill">{signal.get("source").and_then(|v| v.as_str()).unwrap_or("").to_string()}</span>
                        {signal.get("time_horizon").and_then(|v| v.as_str()).map(|h| view! {
                            <span class="muted-copy">{format!("Time horizon: {}", h)}</span>
                        })}
                    </div>
                </div>
            </For>
        </div>
    }.into_view()
}
