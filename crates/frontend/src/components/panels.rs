use apex_shared::{EventTimeline, PredictiveAlert, QuarantineItem};
use leptos::*;

#[component]
pub fn QuarantineList(items: Vec<QuarantineItem>) -> impl IntoView {
    view! {
        <div class="quarantine-list">
            <For each=move || items.clone() key=|item| item.id.clone() let:item>
                <div class="quarantine-item">
                    <div>
                        <strong>{item.source_domain.clone()}</strong>
                        <p class="muted-copy">{item.reason.clone()}</p>
                    </div>
                    <span class="muted-copy">{format!("Release {}", item.release_at.format("%Y-%m-%d %H:%M UTC"))}</span>
                </div>
            </For>
        </div>
    }
}

#[component]
pub fn PredictiveAlertList(alerts: Vec<PredictiveAlert>) -> impl IntoView {
    view! {
        <div class="timeline-list">
            <For each=move || alerts.clone() key=|alert| format!("{}-{}", alert.trigger_signal, alert.predicted_signal) let:alert>
                <div class="timeline-item">
                    <strong>{format!("{} -> {}", alert.trigger_signal, alert.predicted_signal)}</strong>
                    <span class="muted-copy">
                        {format!("Expected within {} days, historical precision {:.0}%", alert.expected_within_days, alert.historical_precision * 100.0)}
                    </span>
                    <span class="muted-copy">
                        {format!("CI [{:.2}, {:.2}]", alert.confidence_interval.lower, alert.confidence_interval.upper)}
                    </span>
                </div>
            </For>
        </div>
    }
}

#[component]
pub fn EntityTimeline(timeline: EventTimeline) -> impl IntoView {
    view! {
        <div class="timeline-list">
            <For each=move || timeline.events.clone() key=|event| format!("{}-{}", event.event_type, event.date) let:event>
                <div class="timeline-item">
                    <strong>{event.event_type.clone()}</strong>
                    <span class="muted-copy">{event.date.format("%Y-%m-%d").to_string()}</span>
                    <span>{event.description.clone()}</span>
                </div>
            </For>
        </div>
    }
}
