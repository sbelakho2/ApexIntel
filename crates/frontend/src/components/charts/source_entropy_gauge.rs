use leptos::*;

#[component]
pub fn SourceEntropyGauge(entropy: f64, anomaly: bool) -> impl IntoView {
    let pct = (entropy.clamp(0.0, 1.0) * 100.0).round();
    let fill_class = if anomaly {
        "source-entropy-fill source-entropy-fill-alert"
    } else {
        "source-entropy-fill"
    };

    view! {
        <div class="timeline-item">
            <strong>{format!("Source entropy {:.0}%", pct)}</strong>
            <div class="source-entropy-track">
                <div class=fill_class style=format!("width:{pct}%;")></div>
            </div>
            <span class="muted-copy">
                {if anomaly { "Anomalous diversity shift" } else { "Within expected range" }}
            </span>
        </div>
    }
}