//! Runtime validation gates run before promoting LLM quality changes.
//!
//! The golden-set regression replays the frozen set of reviewed warnings
//! through the current shared quality gate and fails when agreement drops.

use crate::digest_filtering::passes_shared_insight_quality_gate;
use anyhow::Context;
use apex_store::postgres::{HistoricalQualityGateLabel, PgStore, QualityGateGoldenSetExample};
use uuid::Uuid;

#[cfg(feature = "llm")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct QualityGateGoldenSetDisagreement {
    pub(crate) source_id: Uuid,
    pub(crate) content_type: String,
    pub(crate) title: String,
    pub(crate) historical_label: HistoricalQualityGateLabel,
    pub(crate) predicted_label: HistoricalQualityGateLabel,
}

#[cfg(feature = "llm")]
#[derive(Debug, Clone)]
pub(crate) struct QualityGateGoldenSetRegressionResult {
    pub(crate) total_examples: usize,
    pub(crate) accepted_examples: usize,
    pub(crate) rejected_examples: usize,
    pub(crate) agreement: f64,
    pub(crate) disagreements: Vec<QualityGateGoldenSetDisagreement>,
}

#[cfg(feature = "llm")]
pub(crate) fn evaluate_quality_gate_golden_set(
    examples: &[QualityGateGoldenSetExample],
) -> QualityGateGoldenSetRegressionResult {
    let mut disagreements = Vec::new();
    let mut accepted_examples = 0usize;
    let mut rejected_examples = 0usize;

    for example in examples {
        match example.historical_label {
            HistoricalQualityGateLabel::Accepted => accepted_examples += 1,
            HistoricalQualityGateLabel::Rejected => rejected_examples += 1,
        }

        let predicted_label = if passes_shared_insight_quality_gate(
            &example.title,
            &example.body,
            Some(example.content_type.as_str()),
        ) {
            HistoricalQualityGateLabel::Accepted
        } else {
            HistoricalQualityGateLabel::Rejected
        };

        if predicted_label != example.historical_label {
            disagreements.push(QualityGateGoldenSetDisagreement {
                source_id: example.source_id,
                content_type: example.content_type.clone(),
                title: example.title.clone(),
                historical_label: example.historical_label,
                predicted_label,
            });
        }
    }

    let total_examples = examples.len();
    let agreement = if total_examples == 0 {
        0.0
    } else {
        1.0 - (disagreements.len() as f64 / total_examples as f64)
    };

    QualityGateGoldenSetRegressionResult {
        total_examples,
        accepted_examples,
        rejected_examples,
        agreement,
        disagreements,
    }
}

#[cfg(feature = "llm")]
pub(crate) async fn run_quality_gate_golden_set_regression(
    store: &PgStore,
) -> anyhow::Result<QualityGateGoldenSetRegressionResult> {
    let accepted_limit = std::env::var("LLM_GOLDEN_SET_ACCEPTED_LIMIT")
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(50);
    let rejected_limit = std::env::var("LLM_GOLDEN_SET_REJECTED_LIMIT")
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(50);
    let agreement_target = std::env::var("LLM_GOLDEN_SET_MIN_AGREEMENT")
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .unwrap_or(0.95)
        .clamp(0.0, 1.0);

    let export = store
        .export_quality_gate_reviewed_warning_golden_set(accepted_limit, rejected_limit)
        .await
        .context("quality gate golden set export failed")?;
    if export.examples.is_empty() {
        tracing::info!(
            "quality_gate_golden_set: no reviewed warnings yet, skipping regression check"
        );
        return Ok(QualityGateGoldenSetRegressionResult {
            total_examples: 0,
            accepted_examples: 0,
            rejected_examples: 0,
            agreement: 1.0,
            disagreements: Vec::new(),
        });
    }
    let regression = evaluate_quality_gate_golden_set(&export.examples);
    let metrics = serde_json::json!({
        "dataset_id": export.dataset_id,
        "dataset_name": export.dataset_name,
        "dataset_version": export.dataset_version,
        "agreement": regression.agreement,
        "agreement_target": agreement_target,
        "total_examples": regression.total_examples,
        "accepted_examples": regression.accepted_examples,
        "rejected_examples": regression.rejected_examples,
        "disagreement_count": regression.disagreements.len(),
    });
    let artifacts = serde_json::json!({
        "disagreements": regression
            .disagreements
            .iter()
            .take(20)
            .map(|item| serde_json::json!({
                "source_id": item.source_id,
                "content_type": item.content_type,
                "title": item.title,
                "historical_label": item.historical_label.as_str(),
                "predicted_label": item.predicted_label.as_str(),
            }))
            .collect::<Vec<_>>(),
    });
    store
        .record_llm_improvement_run(
            "quality_gate_golden_set_regression",
            &export.dataset_version,
            &metrics,
            &artifacts,
        )
        .await
        .context("quality gate golden set regression persistence failed")?;

    if !regression.disagreements.is_empty() {
        let severity = if regression.agreement + f64::EPSILON < agreement_target {
            "critical"
        } else {
            "high"
        };
        let desc = format!(
            "Quality-gate golden-set disagreement detected. dataset={} agreement={:.1}% target={:.1}% disagreements={}/{}.",
            export.dataset_version,
            regression.agreement * 100.0,
            agreement_target * 100.0,
            regression.disagreements.len(),
            regression.total_examples,
        );
        match store
            .insert_warning(
                "llm_quality_gate_golden_set_review",
                "LLM quality-gate golden set review required",
                Some(&desc),
                severity,
                Some("global"),
                None,
                None,
                None,
                Some((1.0 - regression.agreement).clamp(0.0, 1.0)),
            )
            .await
        {
            Ok(warning_id) => tracing::warn!(
                warning_id = %warning_id,
                dataset_version = %export.dataset_version,
                disagreements = regression.disagreements.len(),
                agreement = regression.agreement,
                "quality_gate_golden_set: review warning inserted"
            ),
            Err(error) => tracing::error!(
                %error,
                dataset_version = %export.dataset_version,
                disagreements = regression.disagreements.len(),
                "quality_gate_golden_set: failed to insert review warning"
            ),
        }
    }

    if regression.agreement + f64::EPSILON < agreement_target {
        anyhow::bail!(
            "quality gate golden set agreement {:.3} below threshold {:.3}",
            regression.agreement,
            agreement_target
        );
    }

    Ok(regression)
}
