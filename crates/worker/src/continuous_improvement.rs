//! Continuous self-improvement cycle for the worker's LLM pipeline.
//!
//! Runs the standard evaluation suite, feeds recent insights and warnings into
//! the self-improvement loop, persists governance artifacts, and gates the
//! cycle on evaluation quality plus the golden-set regression.

use anyhow::Context;
use apex_core::measurement::Measurement;
use apex_core::stage::StructuredFailure;
use apex_llm::evaluation::{standard_eval_suite, EvalRunner};
use apex_llm::self_improvement::{
    ImprovementCycleReport, OutputCapture, SelfImprovementConfig, SelfImprovementLoop, TaskCategory,
};
use apex_store::postgres::{InsightListFilters, PgStore, WarningListFilters};
use chrono::Utc;

use crate::digest_filtering::passes_shared_insight_quality_gate;
use crate::intelligence_ingress::{IntelligenceIngress, NewWarning};
use crate::llm_runtime::build_quality_llm_client;
use crate::observability::WORKER_METRICS;
use crate::runtime_validation::run_quality_gate_golden_set_regression;
#[cfg(feature = "llm")]
#[derive(Debug, Clone)]
pub(crate) struct LlmContinuousImprovementStats {
    pub(crate) eval_pass_rate: f64,
    pub(crate) eval_avg_score: Measurement<f64>,
    pub(crate) eval_hallucination_rate: f64,
    pub(crate) captures_seeded: usize,
    pub(crate) captures_analysed: usize,
    pub(crate) qualifying_examples: usize,
    pub(crate) avg_critique_score: Measurement<f64>,
    /// Structured failures from the learning stages; a non-empty list means at
    /// least one stage failed and the cycle must not roll up as success.
    pub(crate) stage_failures: Vec<StructuredFailure>,
}

#[cfg(feature = "llm")]
pub(crate) async fn run_llm_continuous_improvement_cycle(
    store: &PgStore,
    ingress: &IntelligenceIngress,
) -> anyhow::Result<LlmContinuousImprovementStats> {
    let llm = build_quality_llm_client();

    let min_eval_pass_rate = std::env::var("LLM_SELF_IMPROVEMENT_MIN_PASS_RATE")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(0.75)
        .clamp(0.0, 1.0);
    let min_eval_score = std::env::var("LLM_SELF_IMPROVEMENT_MIN_SCORE")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(0.62)
        .clamp(0.0, 1.0);
    let max_hallucination_rate = std::env::var("LLM_SELF_IMPROVEMENT_MAX_HALLUCINATION_RATE")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(0.30)
        .clamp(0.0, 1.0);

    let eval_runner = EvalRunner::new(llm.clone(), llm.clone());
    let eval_suite = standard_eval_suite();
    let eval_report = eval_runner
        .run(&eval_suite)
        .await
        .context("llm self-improvement: standard eval suite failed")?;

    let eval_pass_rate = eval_report.pass_rate();
    let eval_avg_score = eval_report.avg_judge_score.clone();
    let eval_hallucination_rate = eval_report.estimated_hallucination_rate();

    let failure_ids = eval_report
        .failures()
        .into_iter()
        .map(|(id, _)| id.to_string())
        .collect::<Vec<_>>();
    let failure_preview = if failure_ids.is_empty() {
        "none".to_string()
    } else {
        failure_ids
            .into_iter()
            .take(6)
            .collect::<Vec<_>>()
            .join(", ")
    };

    let eval_summary = format!(
        "suite={} run_id={} pass_rate={:.1}% avg_judge_score={} hallucination_rate={:.1}% total_cases={} failed_cases={} ({})",
        eval_report.suite_name,
        eval_report.run_id,
        eval_pass_rate * 100.0,
        eval_avg_score.display_fixed(3),
        eval_hallucination_rate * 100.0,
        eval_report.total_cases,
        eval_report.failed,
        failure_preview,
    );
    let eval_metrics = serde_json::json!({
        "suite_name": eval_report.suite_name,
        "run_id": eval_report.run_id,
        "pass_rate": eval_pass_rate,
        "avg_judge_score": eval_avg_score.value_copied(),
        "avg_judge_score_state": eval_avg_score.label(),
        "hallucination_rate": eval_hallucination_rate,
        "total_cases": eval_report.total_cases,
        "passed": eval_report.passed,
        "failed": eval_report.failed,
        "failure_preview": failure_preview,
    });
    let eval_artifacts =
        serde_json::to_value(&eval_report).unwrap_or_else(|_| serde_json::json!({}));
    if let Err(error) = store
        .record_llm_improvement_run(
            "standard_eval_suite",
            &eval_report.run_id,
            &eval_metrics,
            &eval_artifacts,
        )
        .await
    {
        tracing::warn!(%error, "self_improvement_cycle: failed to persist eval run artifact");
    }
    if passes_shared_insight_quality_gate(
        "LLM Eval Gate Report",
        &eval_summary,
        Some("llm_eval_report"),
    ) {
        tracing::info!(
            "self_improvement_cycle: llm eval report passed quality gate; storing governance artifact only"
        );
    }

    if !eval_avg_score.is_measured() {
        tracing::warn!(
            state = eval_avg_score.label(),
            "self_improvement_cycle: judge average score not measured; the score threshold is not evaluated"
        );
        WORKER_METRICS.record_self_improvement_not_evaluated();
    }

    // A judge score below threshold is a breach; "not measured" is not a
    // breach (pass_rate and hallucination rate still gate the suite).
    let eval_score_breached = matches!(
        &eval_avg_score,
        Measurement::Measured(value) if *value < min_eval_score
    );
    if eval_pass_rate < min_eval_pass_rate
        || eval_score_breached
        || eval_hallucination_rate > max_hallucination_rate
    {
        tracing::warn!(
            eval_pass_rate,
            min_eval_pass_rate,
            eval_avg_score = %eval_avg_score.display_fixed(3),
            min_eval_score,
            eval_hallucination_rate,
            max_hallucination_rate,
            failed_cases = eval_report.failed,
            "self_improvement_cycle: llm eval quality gate breached"
        );
        let severity = if eval_pass_rate < (min_eval_pass_rate - 0.15)
            || eval_hallucination_rate > (max_hallucination_rate + 0.15)
        {
            "critical"
        } else {
            "high"
        };
        let desc = format!(
            "LLM quality gate breached. pass_rate={:.1}% (min {:.1}%), avg_score={} (min {:.3}), hallucination={:.1}% (max {:.1}%). failed_cases={}.",
            eval_pass_rate * 100.0,
            min_eval_pass_rate * 100.0,
            eval_avg_score.display_fixed(3),
            min_eval_score,
            eval_hallucination_rate * 100.0,
            max_hallucination_rate * 100.0,
            eval_report.failed,
        );
        match ingress
            .submit_warning(
                NewWarning::new(
                    "llm_quality_regression",
                    "LLM quality regression detected",
                    severity,
                )
                .description(&desc)
                .region("global")
                .confidence((1.0 - eval_pass_rate).clamp(0.0, 1.0))
                // Global LLM quality gate: no entity owns it.
                .system_broadcast(),
            )
            .await
        {
            Ok(result) => tracing::warn!(
                warning_id = %result.warning_id(),
                severity,
                "self_improvement_cycle: inserted llm quality regression warning"
            ),
            Err(error) => tracing::error!(
                %error,
                severity,
                "self_improvement_cycle: failed to insert llm quality regression warning"
            ),
        }
    }

    let insights = store
        .list_insights(
            &InsightListFilters {
                date_from: Some(Utc::now() - chrono::Duration::days(14)),
                ..Default::default()
            },
            120,
            0,
        )
        .await
        .context("llm self-improvement: failed to load recent insights")?;
    let warnings = store
        .list_warnings(
            &WarningListFilters {
                date_from: Some(Utc::now() - chrono::Duration::days(14)),
                ..Default::default()
            },
            None,
            true,
            120,
            0,
        )
        .await
        .context("llm self-improvement: failed to load recent warnings")?;

    let mut loop_runner = SelfImprovementLoop::new(
        llm,
        SelfImprovementConfig {
            min_quality_score: std::env::var("LLM_SELF_IMPROVEMENT_MIN_EXAMPLE_QUALITY")
                .ok()
                .and_then(|v| v.parse::<f64>().ok())
                .unwrap_or(0.72)
                .clamp(0.0, 1.0),
            max_history_size: 500,
            critique_batch_size: 30,
            generate_prompt_proposals: true,
            analyse_failures: true,
        },
    );

    let mut captures_seeded = 0usize;
    for row in insights {
        let system_prompt =
            "Generate a strategic OSINT insight with explicit evidence and quantified confidence.";
        let user_prompt = format!(
            "title={} | type={} | region={} | confidence={:.3}",
            row.title,
            row.insight_type.as_deref().unwrap_or("unknown"),
            row.region.as_deref().unwrap_or("global"),
            row.confidence.unwrap_or(0.0),
        );
        let mut capture = OutputCapture::new(
            TaskCategory::InsightGeneration,
            system_prompt,
            user_prompt,
            row.summary,
            0,
        );
        capture.quality_score = row.confidence;
        capture.was_used = true;
        capture.captured_at = row.updated_at.or(row.created_at).unwrap_or_else(Utc::now);
        loop_runner.record(capture);
        captures_seeded += 1;
    }
    for row in warnings {
        let system_prompt =
            "Generate concise, evidence-grounded security or risk warnings without speculation.";
        let user_prompt = format!(
            "warning_type={} | severity={} | region={} | title={}",
            row.warning_type,
            row.severity,
            row.region.as_deref().unwrap_or("global"),
            row.title,
        );
        let mut capture = OutputCapture::new(
            TaskCategory::EvidenceChain,
            system_prompt,
            user_prompt,
            row.description.unwrap_or_default(),
            0,
        );
        capture.quality_score = row.confidence;
        capture.was_used = true;
        capture.captured_at = row.updated_at.or(row.created_at).unwrap_or(row.ts_utc);
        loop_runner.record(capture);
        captures_seeded += 1;
    }

    let cycle_report = loop_runner
        .run_cycle()
        .await
        .context("llm self-improvement: critique cycle failed")?;
    let training_examples = loop_runner.export_training_examples();

    let improvement_summary = format!(
        "cycle_id={} captures_seeded={} analysed={} qualifying_examples={} avg_critique={} critique_status={} prompt_improvements={} prompt_improvements_status={} failure_hypotheses={} failure_hypotheses_status={} training_examples={}",
        cycle_report.cycle_id,
        captures_seeded,
        cycle_report.captures_analysed,
        cycle_report.examples_qualifying,
        cycle_report.avg_critique_score.display_fixed(3),
        cycle_report.critique.status.as_str(),
        cycle_report.prompt_improvements_count(),
        cycle_report.prompt_improvements.status.as_str(),
        cycle_report.failure_hypotheses_count(),
        cycle_report.failure_hypotheses.status.as_str(),
        training_examples.len(),
    );
    if passes_shared_insight_quality_gate(
        "LLM Continuous Improvement Cycle",
        &improvement_summary,
        Some("llm_self_improvement"),
    ) {
        tracing::info!(
            "self_improvement_cycle: continuous improvement summary passed quality gate; storing governance artifact only"
        );
    }

    // Surface every failed learning stage: a failed model call or an
    // unparseable response must never read as "no improvements found".
    for failure in cycle_report.stage_failures() {
        tracing::warn!(
            stage = %failure.stage,
            kind = %failure.kind.as_str(),
            error = %failure.message,
            "self_improvement_cycle: stage failed"
        );
        WORKER_METRICS.record_self_improvement_stage_failure(&failure.stage);
    }
    let stage_failures: Vec<StructuredFailure> =
        cycle_report.stage_failures().into_iter().cloned().collect();

    let jsonl_examples = ImprovementCycleReport::to_jsonl(&training_examples);
    let cycle_metrics = serde_json::json!({
        "cycle_id": cycle_report.cycle_id,
        "captures_seeded": captures_seeded,
        "captures_analysed": cycle_report.captures_analysed,
        "examples_qualifying": cycle_report.examples_qualifying,
        "avg_critique_score": cycle_report.avg_critique_score.value_copied(),
        "avg_critique_score_state": cycle_report.avg_critique_score.label(),
        "critique_status": cycle_report.critique.status.as_str(),
        "critique_failure": cycle_report.critique.failure_ref().map(|f| f.display()),
        "prompt_improvements": cycle_report.prompt_improvements_count(),
        "prompt_improvements_status": cycle_report.prompt_improvements.status.as_str(),
        "prompt_improvements_failure": cycle_report.prompt_improvements.failure_ref().map(|f| f.display()),
        "failure_hypotheses": cycle_report.failure_hypotheses_count(),
        "failure_hypotheses_status": cycle_report.failure_hypotheses.status.as_str(),
        "failure_hypotheses_failure": cycle_report.failure_hypotheses.failure_ref().map(|f| f.display()),
        "stage_failures": stage_failures.iter().map(|f| f.display()).collect::<Vec<_>>(),
        "training_examples": training_examples.len(),
    });
    let cycle_report_value =
        serde_json::to_value(&cycle_report).unwrap_or_else(|_| serde_json::json!({}));
    let cycle_artifacts = serde_json::json!({
        "report": cycle_report_value,
        "training_examples_preview_chars": jsonl_examples.chars().count(),
    });
    if let Err(error) = store
        .record_llm_improvement_run(
            "continuous_self_improvement_cycle",
            &cycle_report.cycle_id,
            &cycle_metrics,
            &cycle_artifacts,
        )
        .await
    {
        tracing::warn!(%error, "self_improvement_cycle: failed to persist continuous improvement run artifact");
    }

    let dataset_version = format!(
        "{}-{}",
        Utc::now().format("%Y%m%dT%H%M%SZ"),
        cycle_report.cycle_id
    );
    let dataset_manifest = serde_json::json!({
        "dataset_name": "llm_self_improvement_examples",
        "dataset_version": dataset_version,
        "source_cycle_id": cycle_report.cycle_id,
        "eval_suite": eval_report.suite_name,
        "eval_run_id": eval_report.run_id,
        "example_count": training_examples.len(),
        "schema": "alpaca_chat_jsonl_v1",
        "tasks": ["insight_generation", "evidence_chain"],
    });
    if let Err(error) = store
        .record_llm_training_dataset(
            "llm_self_improvement_examples",
            &dataset_version,
            "continuous_self_improvement_cycle",
            &cycle_report.cycle_id,
            &dataset_manifest,
            training_examples.len() as i64,
            &jsonl_examples,
        )
        .await
    {
        tracing::warn!(%error, "self_improvement_cycle: failed to persist training dataset artifact");
    }

    if !jsonl_examples.is_empty() {
        tracing::info!(
            training_examples = training_examples.len(),
            jsonl_chars = jsonl_examples.chars().count(),
            dataset_version = %dataset_version,
            "self_improvement_cycle: persisted training examples dataset"
        );
    }

    let min_critique = std::env::var("LLM_SELF_IMPROVEMENT_MIN_CRITIQUE")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(0.60)
        .clamp(0.0, 1.0);
    let critique_breach: Option<f64> = match &cycle_report.avg_critique_score {
        Measurement::Measured(avg) if *avg < min_critique => Some(*avg),
        Measurement::Measured(_) => None,
        // Not evaluated is not a score: never fire the degradation warning for
        // an absent measurement, and never let it pass silently either.
        Measurement::NotMeasured => {
            tracing::warn!(
                captures_analysed = cycle_report.captures_analysed,
                "self_improvement_cycle: critique not evaluated (no captures were critiqued); score gate skipped"
            );
            WORKER_METRICS.record_self_improvement_not_evaluated();
            None
        }
        Measurement::Unavailable(reason) => {
            tracing::warn!(
                reason = %reason.display(),
                captures_analysed = cycle_report.captures_analysed,
                "self_improvement_cycle: critique unavailable; every critique attempt failed"
            );
            WORKER_METRICS.record_self_improvement_stage_failure("critique");
            None
        }
        Measurement::InsufficientEvidence => {
            tracing::warn!(
                captures_analysed = cycle_report.captures_analysed,
                "self_improvement_cycle: insufficient evidence for a critique score; score gate skipped"
            );
            WORKER_METRICS.record_self_improvement_not_evaluated();
            None
        }
    };
    if let Some(avg_critique_score) = critique_breach {
        tracing::warn!(
            avg_critique = avg_critique_score,
            min_critique,
            captures_analysed = cycle_report.captures_analysed,
            qualifying_examples = cycle_report.examples_qualifying,
            "self_improvement_cycle: llm critique quality gate breached"
        );
        let desc = format!(
            "Continuous self-improvement critique score is below threshold: avg_critique={avg_critique_score:.3} < min={min_critique:.3}. analysed={} qualifying_examples={}",
            cycle_report.captures_analysed,
            cycle_report.examples_qualifying,
        );
        match ingress
            .submit_warning(
                NewWarning::new(
                    "llm_self_improvement_degradation",
                    "LLM self-improvement cycle quality below threshold",
                    "high",
                )
                .description(&desc)
                .region("global")
                .confidence((1.0 - avg_critique_score).clamp(0.0, 1.0))
                // Global self-improvement gate: no entity owns it.
                .system_broadcast(),
            )
            .await
        {
            Ok(result) => tracing::warn!(
                warning_id = %result.warning_id(),
                "self_improvement_cycle: inserted llm self-improvement degradation warning"
            ),
            Err(error) => tracing::error!(
                %error,
                "self_improvement_cycle: failed to insert llm self-improvement degradation warning"
            ),
        }
    }

    let golden_set_regression = run_quality_gate_golden_set_regression(store, ingress)
        .await
        .context("llm self-improvement: quality gate golden set regression failed")?;
    tracing::info!(
        agreement = golden_set_regression.agreement,
        total_examples = golden_set_regression.total_examples,
        disagreements = golden_set_regression.disagreements.len(),
        "self_improvement_cycle: quality gate golden set regression completed"
    );

    Ok(LlmContinuousImprovementStats {
        eval_pass_rate,
        eval_avg_score,
        eval_hallucination_rate,
        captures_seeded,
        captures_analysed: cycle_report.captures_analysed,
        qualifying_examples: cycle_report.examples_qualifying,
        avg_critique_score: cycle_report.avg_critique_score,
        stage_failures,
    })
}
