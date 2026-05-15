use apex_shared::{
    BandClass, ConfidenceInterval, GrangerCausalPair, PredictiveAlert, SurvivalPoint,
};
use leptos::*;

use crate::components::{
    cards::{PageHeader, SurfaceCard},
    charts::{causal_graph::CausalGraph, survival_curve::SurvivalCurve},
    panels::PredictiveAlertList,
};

fn sample_pairs() -> Vec<GrangerCausalPair> {
    vec![
        GrangerCausalPair {
            cause_signal: "hiring_velocity".to_string(),
            effect_signal: "capacity_expansion".to_string(),
            optimal_lag_days: 21,
            p_value: 0.008,
            significant_after_fdr: true,
        },
        GrangerCausalPair {
            cause_signal: "customer_mentions".to_string(),
            effect_signal: "program_award".to_string(),
            optimal_lag_days: 28,
            p_value: 0.014,
            significant_after_fdr: true,
        },
    ]
}

fn sample_alerts() -> Vec<PredictiveAlert> {
    vec![PredictiveAlert {
        trigger_signal: "hiring_velocity".to_string(),
        predicted_signal: "capacity_expansion".to_string(),
        expected_within_days: 21,
        historical_precision: 0.72,
        confidence_interval: ConfidenceInterval {
            value: 0.72,
            lower: 0.64,
            upper: 0.79,
            half_width: 0.075,
            band_class: BandClass::Moderate,
        },
    }]
}

fn sample_survival() -> Vec<SurvivalPoint> {
    vec![
        SurvivalPoint {
            day: 0,
            survival_probability: 1.0,
        },
        SurvivalPoint {
            day: 7,
            survival_probability: 0.96,
        },
        SurvivalPoint {
            day: 14,
            survival_probability: 0.91,
        },
        SurvivalPoint {
            day: 21,
            survival_probability: 0.83,
        },
        SurvivalPoint {
            day: 30,
            survival_probability: 0.74,
        },
    ]
}

#[component]
pub fn CausalityPage() -> impl IntoView {
    let pairs = sample_pairs();
    let alerts = sample_alerts();
    let survival = sample_survival();

    view! {
        <div class="page">
            <PageHeader
                eyebrow="Temporal Reasoning"
                title="Causal And Predictive Views"
                subtitle="The WASM frontend now includes directed causal rendering, predictive alert cards, and survival-style timeline charts."
            />

            <div class="two-up">
                <SurfaceCard title="Granger Pairs" subtitle="Directed relationships are rendered with lag and p-value labels.">
                    <CausalGraph pairs=pairs />
                </SurfaceCard>

                <SurfaceCard title="Predictive Alerts" subtitle="Shared confidence intervals are rendered inline with predicted windows.">
                    <PredictiveAlertList alerts=alerts />
                </SurfaceCard>
            </div>

            <SurfaceCard title="Survival Curve" subtitle="Kaplan-Meier style step rendering for churn or disruption timing.">
                <SurvivalCurve points=survival />
            </SurfaceCard>
        </div>
    }
}
