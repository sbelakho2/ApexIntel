use apex_shared::SourceReliabilityTier;
use leptos::*;

#[component]
pub fn SourceReliabilityBadge(
    tier: SourceReliabilityTier,
    #[prop(optional)] promotion_candidate: Option<bool>,
) -> impl IntoView {
    let promotion_class = if promotion_candidate.unwrap_or(false) {
        " promotion-candidate"
    } else {
        ""
    };

    view! {
        <span class=format!("source-badge {}{}", tier.css_class(), promotion_class)>
            {tier.label()}
        </span>
    }
}
