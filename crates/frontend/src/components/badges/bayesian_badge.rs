use apex_shared::BayesianInterpretation;
use leptos::*;

#[component]
pub fn BayesianBadge(interpretation: BayesianInterpretation) -> impl IntoView {
    let class_name = interpretation.css_class();
    let label = interpretation.label();

    view! {
        <span class=format!("bayesian-badge {class_name}")>{label}</span>
    }
}
