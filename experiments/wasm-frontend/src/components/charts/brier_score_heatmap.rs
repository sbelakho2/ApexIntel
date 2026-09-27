use apex_shared::BrierScoreEntry;
use leptos::*;

fn cell_class(score: f64) -> &'static str {
    if score <= 0.08 {
        "heatmap-cell heatmap-cell-good"
    } else if score <= 0.15 {
        "heatmap-cell heatmap-cell-watch"
    } else {
        "heatmap-cell heatmap-cell-alert"
    }
}

#[component]
pub fn BrierScoreHeatmap(entries: Vec<BrierScoreEntry>) -> impl IntoView {
    view! {
        <div class="heatmap-grid">
            <For each=move || entries.clone() key=|entry| entry.category.clone() let:entry>
                <div class=move || cell_class(entry.score)>
                    <strong>{entry.category.clone()}</strong>
                    <span>{format!("{:.3}", entry.score)}</span>
                </div>
            </For>
        </div>
    }
}
