use apex_shared::TemporalConsistency;
use leptos::*;

#[component]
pub fn TemporalFlag(consistency: TemporalConsistency) -> impl IntoView {
    if consistency.valid {
        return ().into_view();
    }

    let message = consistency
        .reason
        .unwrap_or_else(|| "Temporal consistency check failed".to_string());

    view! {
        <div class="temporal-warning">
            <span>"Warning"</span>
            <span>{message}</span>
        </div>
    }
    .into_view()
}
