//! Continuous self-improvement cycle for the worker's LLM pipeline.
//!
//! Runs the standard evaluation suite, feeds recent insights and warnings into
//! the self-improvement loop, persists governance artifacts, and gates the
//! cycle on evaluation quality plus the golden-set regression.
//!
//! Every learning stage and every persistence step reports a [`StageResult`]:
//! a failed stage carries the structured reason instead of looking like an
//! empty success, an un-serializable report is an error rather than `{}`, and
//! governance/dataset persistence failures degrade the job instead of being
//! warn-and-continue.

use anyhow::Context;
use apex_core::measurement::Measurement;
use apex_core::stage::{FailureKind, StageResult, StageStatus, StructuredFailure};
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

/// Measured summary of the evaluation stage.
#[cfg(feature = "llm")]
#[derive(Debug, Clone)]
pub(crate) struct EvaluationStageSummary {
    pub(crate) suite_name: String,
    pub(crate) run_id: String,
    pub(crate) pass_rate: f64,
    pub(crate) avg_judge_score: Measurement<f64>,
    pub(crate) hallucination_rate: f64,
    pub(crate) total_cases: usize,
    pub(crate) failed_cases: usize,
    pub(crate) execution_errors: usize,
}

/// Thresholds of the LLM evaluation quality gate.
#[cfg(feature = "llm")]
#[derive(Debug, Clone, Copy)]
pub(crate) struct EvalGateThresholds {
    pub(crate) min_pass_rate: f64,
    pub(crate) min_score: f64,
    pub(crate) max_hallucination_rate: f64,
}

/// Decision of the LLM evaluation quality gate.
#[cfg(feature = "llm")]
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum EvalGateDecision {
    /// No case produced model output (model unreachable): quality was not
    /// measured, so there is no quality verdict and no regression warning.
    NotEvaluated,
    Pass,
    Breach {
        severity: &'static str,
    },
}

/// Gate the graded cases of an eval run. Cases that could not execute are
/// already excluded from `pass_rate` / `hallucination_rate` by the report.
#[cfg(feature = "llm")]
pub(crate) fn eval_gate_decision(
    evaluated_cases: usize,
    pass_rate: f64,
    avg_judge_score: &Measurement<f64>,
    hallucination_rate: f64,
    thresholds: EvalGateThresholds,
) -> EvalGateDecision {
    if evaluated_cases == 0 {
        return EvalGateDecision::NotEvaluated;
    }
    // A judge score below threshold is a breach; "not measured" is not a
    // breach (pass_rate and hallucination rate still gate the suite).
    let score_breached = matches!(
        avg_judge_score,
        Measurement::Measured(value) if *value < thresholds.min_score
    );
    if pass_rate < thresholds.min_pass_rate
        || score_breached
        || hallucination_rate > thresholds.max_hallucination_rate
    {
        let severity = if pass_rate < (thresholds.min_pass_rate - 0.15)
            || hallucination_rate > (thresholds.max_hallucination_rate + 0.15)
        {
            "critical"
        } else {
            "high"
        };
        EvalGateDecision::Breach { severity }
    } else {
        EvalGateDecision::Pass
    }
}

/// Measured summary of the critique cycle.
#[cfg(feature = "llm")]
#[derive(Debug, Clone)]
pub(crate) struct CritiqueStageSummary {
    pub(crate) cycle_id: String,
    pub(crate) captures_seeded: usize,
    pub(crate) captures_analysed: usize,
    pub(crate) qualifying_examples: usize,
    pub(crate) avg_critique_score: Measurement<f64>,
}

/// Summary of the proposal stages (prompt improvements + failure hypotheses).
#[cfg(feature = "llm")]
#[derive(Debug, Clone)]
pub(crate) struct ProposalsStageSummary {
    pub(crate) prompt_improvements: usize,
    pub(crate) failure_hypotheses: usize,
}

/// Governance persistence receipt.
#[cfg(feature = "llm")]
#[derive(Debug, Clone)]
pub(crate) struct GovernancePersistenceSummary {
    pub(crate) runs_persisted: usize,
}

/// Training-dataset persistence receipt.
#[cfg(feature = "llm")]
#[derive(Debug, Clone)]
pub(crate) struct DatasetPersistenceSummary {
    pub(crate) dataset_version: String,
    pub(crate) example_count: usize,
}

/// Structured outcome of one continuous-improvement cycle.
///
/// Invariants carried by the caller:
/// * a terminal failure in `evaluation`/`critique`/`proposals` fails the job;
/// * a failure in `governance_persistence`/`dataset_persistence` degrades it.
#[cfg(feature = "llm")]
#[derive(Debug, Clone)]
pub(crate) struct ContinuousImprovementOutcome {
    /// Golden-set agreement below target on a meaningful sample.
    pub(crate) golden_set_below_target: bool,
    pub(crate) evaluation: StageResult<EvaluationStageSummary>,
    pub(crate) critique: StageResult<CritiqueStageSummary>,
    pub(crate) proposals: StageResult<ProposalsStageSummary>,
    pub(crate) governance_persistence: StageResult<GovernancePersistenceSummary>,
    pub(crate) dataset_persistence: StageResult<DatasetPersistenceSummary>,
}

#[cfg(feature = "llm")]
impl ContinuousImprovementOutcome {
    /// Every structured failure attached to any stage.
    pub(crate) fn failures(&self) -> Vec<StructuredFailure> {
        let mut failures = Vec::new();
        for failure in [
            self.evaluation.failure_ref(),
            self.critique.failure_ref(),
            self.proposals.failure_ref(),
            self.governance_persistence.failure_ref(),
            self.dataset_persistence.failure_ref(),
        ]
        .into_iter()
        .flatten()
        {
            failures.push(failure.clone());
        }
        failures
    }

    /// Whether a learning stage terminally failed (its value is unknown).
    pub(crate) fn has_terminal_learning_failure(&self) -> bool {
        self.evaluation.is_failed() || self.critique.is_failed() || self.proposals.is_failed()
    }

    /// Whether governance or dataset persistence failed.
    pub(crate) fn persistence_degraded(&self) -> bool {
        self.governance_persistence.has_failure() || self.dataset_persistence.has_failure()
    }

    /// Golden-set agreement below target on a meaningful sample: the cycle
    /// degrades and the promotion is withheld (governance outcome), without
    /// failing the whole run.
    pub(crate) fn quality_gate_degraded(&self) -> bool {
        self.golden_set_below_target
    }
}

#[cfg(feature = "llm")]
fn cycle_stage_from_status<T, V>(
    stage: &StageResult<T>,
    stage_name: &str,
    value: V,
) -> StageResult<V> {
    match stage.status {
        StageStatus::Failed => {
            StageResult::failed(stage.failure_ref().cloned().unwrap_or_else(|| {
                StructuredFailure::new(
                    stage_name,
                    FailureKind::Internal,
                    "stage failed without a recorded failure",
                )
            }))
        }
        StageStatus::Partial => StageResult::partial(
            value,
            stage.failure_ref().cloned().unwrap_or_else(|| {
                StructuredFailure::new(
                    stage_name,
                    FailureKind::Internal,
                    "stage was partial without a recorded failure",
                )
            }),
        ),
        StageStatus::Success | StageStatus::Empty => StageResult::success(value),
    }
}

#[cfg(feature = "llm")]
pub(crate) async fn run_llm_continuous_improvement_cycle(
    store: &PgStore,
    ingress: &IntelligenceIngress,
) -> anyhow::Result<ContinuousImprovementOutcome> {
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

    // ── 1. Evaluation stage ────────────────────────────────────────────
    // Failures from either governance run below accumulate here; a persistence
    // failure is never warn-and-continue.
    let mut governance_failures: Vec<StructuredFailure> = Vec::new();
    let mut governance_runs_persisted = 0usize;

    // #92: the evaluation suite drives model calls; hold one process-wide
    // LLM permit for the run so the self-improvement job cannot push the
    // endpoint past the global concurrency cap.
    let evaluation_result = {
        let _llm_slot = apex_worker::llm_concurrency::acquire_llm_slot().await;
        eval_runner.run(&eval_suite).await
    };
    let evaluation = match evaluation_result {
        Ok(eval_report) => {
            let eval_pass_rate = eval_report.pass_rate();
            let eval_avg_score = eval_report.avg_judge_score.clone();
            let eval_hallucination_rate = eval_report.estimated_hallucination_rate();
            let evaluated_cases = eval_report.evaluated_cases();
            let execution_errors = eval_report.execution_errors;

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
                "suite={} run_id={} pass_rate={:.1}% avg_judge_score={} hallucination_rate={:.1}% total_cases={} evaluated_cases={} execution_errors={} failed_cases={} ({})",
                eval_report.suite_name,
                eval_report.run_id,
                eval_pass_rate * 100.0,
                eval_avg_score.display_fixed(3),
                eval_hallucination_rate * 100.0,
                eval_report.total_cases,
                evaluated_cases,
                execution_errors,
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
                "evaluated_cases": evaluated_cases,
                "execution_errors": execution_errors,
                "passed": eval_report.passed,
                "failed": eval_report.failed,
                "failure_preview": failure_preview,
            });

            if passes_shared_insight_quality_gate(
                "LLM Eval Gate Report",
                &eval_summary,
                Some("llm_eval_report"),
            ) {
                tracing::info!(
                    "self_improvement_cycle: llm eval report passed quality gate; storing governance artifact only"
                );
            }

            if !eval_avg_score.is_measured() && evaluated_cases > 0 {
                tracing::warn!(
                    state = eval_avg_score.label(),
                    "self_improvement_cycle: judge average score not measured; the score threshold is not evaluated"
                );
                WORKER_METRICS.record_self_improvement_not_evaluated();
            }

            let gate = eval_gate_decision(
                evaluated_cases,
                eval_pass_rate,
                &eval_avg_score,
                eval_hallucination_rate,
                EvalGateThresholds {
                    min_pass_rate: min_eval_pass_rate,
                    min_score: min_eval_score,
                    max_hallucination_rate,
                },
            );
            if gate == EvalGateDecision::NotEvaluated {
                tracing::warn!(
                    total_cases = eval_report.total_cases,
                    execution_errors,
                    "self_improvement_cycle: no eval case executed (model unavailable); quality gate not evaluated"
                );
                WORKER_METRICS.record_self_improvement_not_evaluated();
            }
            if let EvalGateDecision::Breach { severity } = gate {
                tracing::warn!(
                    eval_pass_rate,
                    min_eval_pass_rate,
                    eval_avg_score = %eval_avg_score.display_fixed(3),
                    min_eval_score,
                    eval_hallucination_rate,
                    max_hallucination_rate,
                    failed_cases = eval_report.failed,
                    execution_errors,
                    "self_improvement_cycle: llm eval quality gate breached"
                );
                let desc = format!(
                    "LLM quality gate breached. pass_rate={:.1}% (min {:.1}%), avg_score={} (min {:.3}), hallucination={:.1}% (max {:.1}%). failed_cases={} of {} graded ({} not executed).",
                    eval_pass_rate * 100.0,
                    min_eval_pass_rate * 100.0,
                    eval_avg_score.display_fixed(3),
                    min_eval_score,
                    eval_hallucination_rate * 100.0,
                    max_hallucination_rate * 100.0,
                    eval_report.failed,
                    evaluated_cases,
                    execution_errors,
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

            // Persist the eval run. An un-serializable report is a governance
            // failure, never an empty `{}` artifact.
            match serde_json::to_value(&eval_report) {
                Ok(eval_artifacts) => {
                    match store
                        .record_llm_improvement_run(
                            "standard_eval_suite",
                            &eval_report.run_id,
                            &eval_metrics,
                            &eval_artifacts,
                        )
                        .await
                    {
                        Ok(_) => governance_runs_persisted += 1,
                        Err(error) => governance_failures.push(StructuredFailure::new(
                            "governance_persistence",
                            FailureKind::Storage,
                            format!("failed to persist eval run artifact: {error}"),
                        )),
                    }
                }
                Err(error) => governance_failures.push(StructuredFailure::new(
                    "governance_persistence",
                    FailureKind::InvalidResponse,
                    format!("failed to serialize eval report artifact: {error}"),
                )),
            }

            let summary = EvaluationStageSummary {
                suite_name: eval_report.suite_name,
                run_id: eval_report.run_id,
                pass_rate: eval_pass_rate,
                avg_judge_score: eval_avg_score,
                hallucination_rate: eval_hallucination_rate,
                total_cases: eval_report.total_cases,
                failed_cases: eval_report.failed,
                execution_errors,
            };
            if gate == EvalGateDecision::NotEvaluated {
                StageResult::failed(StructuredFailure::new(
                    "evaluation",
                    FailureKind::ModelCallFailed,
                    format!(
                        "no eval case executed: {execution_errors} of {} cases hit execution errors (model unavailable)",
                        summary.total_cases
                    ),
                ))
            } else if execution_errors > 0 {
                StageResult::partial(
                    summary,
                    StructuredFailure::new(
                        "evaluation",
                        FailureKind::ModelCallFailed,
                        format!(
                            "{execution_errors} eval case(s) hit execution errors; gate evaluated on {evaluated_cases} graded case(s)"
                        ),
                    ),
                )
            } else {
                StageResult::success(summary)
            }
        }
        Err(error) => {
            tracing::error!(
                %error,
                "self_improvement_cycle: standard eval suite failed"
            );
            StageResult::failed(StructuredFailure::new(
                "evaluation",
                FailureKind::ModelCallFailed,
                format!("standard eval suite failed: {error}"),
            ))
        }
    };

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
        // A missing confidence is textual "unknown" — embedding 0.000 would
        // feed the self-improvement loop a confidence the row never had.
        let confidence_text = row
            .confidence
            .map_or_else(|| "unknown".to_string(), |value| format!("{value:.3}"));
        let user_prompt = format!(
            "title={} | type={} | region={} | confidence={}",
            row.title,
            row.insight_type.as_deref().unwrap_or("unknown"),
            row.region.as_deref().unwrap_or("global"),
            confidence_text,
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
        // Unknown capture time stays unknown: the record keeps the epoch only
        // as an explicit "no timestamp" marker rather than looking current.
        capture.captured_at = row
            .updated_at
            .or(row.created_at)
            .unwrap_or(chrono::DateTime::<Utc>::UNIX_EPOCH);
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

    // ── 2. Critique stage ──────────────────────────────────────────────
    let training_examples = loop_runner.export_training_examples();
    // Serialize the examples once; both the governance artifact preview and
    // the dataset persistence reuse the same JSONL.
    let jsonl_examples = ImprovementCycleReport::to_jsonl(&training_examples);
    // #92: the critique cycle is the second model-call stage; it takes the
    // same process-wide permit as the evaluation stage.
    let cycle_result = {
        let _llm_slot = apex_worker::llm_concurrency::acquire_llm_slot().await;
        loop_runner.run_cycle().await
    };
    let cycle_report: Option<ImprovementCycleReport> = match cycle_result {
        Ok(report) => Some(report),
        Err(error) => {
            tracing::error!(%error, "self_improvement_cycle: critique cycle failed");
            None
        }
    };

    let (critique, proposals) = match &cycle_report {
        Some(cycle_report) => {
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
            let failed_stages: Vec<String> = cycle_report
                .failed_stages()
                .into_iter()
                .map(|failure| failure.stage.clone())
                .collect();
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
                "failed_stages": &failed_stages,
                "training_examples": training_examples.len(),
            });

            // Persist the cycle report. An un-serializable report is a
            // governance failure; `{}` must never be stored in its place.
            match serde_json::to_value(cycle_report) {
                Ok(report_value) => {
                    let cycle_artifacts = serde_json::json!({
                        "report": report_value,
                        "training_examples_preview_chars": jsonl_examples.chars().count(),
                    });
                    match store
                        .record_llm_improvement_run(
                            "continuous_self_improvement_cycle",
                            &cycle_report.cycle_id,
                            &cycle_metrics,
                            &cycle_artifacts,
                        )
                        .await
                    {
                        Ok(_) => governance_runs_persisted += 1,
                        Err(error) => governance_failures.push(StructuredFailure::new(
                            "governance_persistence",
                            FailureKind::Storage,
                            format!(
                                "failed to persist continuous improvement run artifact: {error}"
                            ),
                        )),
                    }
                }
                Err(error) => governance_failures.push(StructuredFailure::new(
                    "governance_persistence",
                    FailureKind::InvalidResponse,
                    format!("failed to serialize cycle report artifact: {error}"),
                )),
            }

            let critique_value = CritiqueStageSummary {
                cycle_id: cycle_report.cycle_id.clone(),
                captures_seeded,
                captures_analysed: cycle_report.captures_analysed,
                qualifying_examples: cycle_report.examples_qualifying,
                avg_critique_score: cycle_report.avg_critique_score.clone(),
            };
            let critique =
                cycle_stage_from_status(&cycle_report.critique, "critique", critique_value);

            let proposals_value = ProposalsStageSummary {
                prompt_improvements: cycle_report.prompt_improvements_count(),
                failure_hypotheses: cycle_report.failure_hypotheses_count(),
            };
            let mut proposal_failures: Vec<StructuredFailure> = Vec::new();
            if let Some(failure) = cycle_report.prompt_improvements.failure_ref() {
                proposal_failures.push(failure.clone());
            }
            if let Some(failure) = cycle_report.failure_hypotheses.failure_ref() {
                proposal_failures.push(failure.clone());
            }
            let proposals = if cycle_report.prompt_improvements.is_failed()
                || cycle_report.failure_hypotheses.is_failed()
            {
                StageResult::failed(proposal_failures.into_iter().next().unwrap_or_else(|| {
                    StructuredFailure::new(
                        "proposals",
                        FailureKind::Internal,
                        "proposal stage failed without a recorded failure",
                    )
                }))
            } else if let Some(failure) = proposal_failures.into_iter().next() {
                StageResult::partial(proposals_value, failure)
            } else {
                StageResult::success(proposals_value)
            };

            (critique, proposals)
        }
        None => (
            StageResult::failed(StructuredFailure::new(
                "critique",
                FailureKind::ModelCallFailed,
                "self-improvement critique cycle failed",
            )),
            StageResult::failed(StructuredFailure::new(
                "proposals",
                FailureKind::MissingField,
                "proposal stages skipped: critique cycle failed",
            )),
        ),
    };

    // ── 3. Critique quality gate ───────────────────────────────────────
    if let Some(avg_critique_score) =
        critique
            .value_ref()
            .and_then(|summary| match &summary.avg_critique_score {
                Measurement::Measured(avg) => Some(*avg),
                _ => None,
            })
    {
        let min_critique = std::env::var("LLM_SELF_IMPROVEMENT_MIN_CRITIQUE")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(0.60)
            .clamp(0.0, 1.0);
        if avg_critique_score < min_critique {
            tracing::warn!(
                avg_critique = avg_critique_score,
                min_critique,
                captures_analysed = critique
                    .value_ref()
                    .map(|summary| summary.captures_analysed)
                    .unwrap_or(0),
                qualifying_examples = critique
                    .value_ref()
                    .map(|summary| summary.qualifying_examples)
                    .unwrap_or(0),
                "self_improvement_cycle: llm critique quality gate breached"
            );
            let desc = format!(
                "Continuous self-improvement critique score is below threshold: avg_critique={avg_critique_score:.3} < min={min_critique:.3}. analysed={} qualifying_examples={}",
                critique
                    .value_ref()
                    .map(|summary| summary.captures_analysed)
                    .unwrap_or(0),
                critique
                    .value_ref()
                    .map(|summary| summary.qualifying_examples)
                    .unwrap_or(0),
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
    } else if let Some(summary) = critique.value_ref() {
        match &summary.avg_critique_score {
            Measurement::NotMeasured => {
                tracing::warn!(
                    captures_analysed = summary.captures_analysed,
                    "self_improvement_cycle: critique not evaluated (no captures were critiqued); score gate skipped"
                );
                WORKER_METRICS.record_self_improvement_not_evaluated();
            }
            Measurement::Unavailable(reason) => {
                tracing::warn!(
                    reason = %reason.display(),
                    captures_analysed = summary.captures_analysed,
                    "self_improvement_cycle: critique unavailable; every critique attempt failed"
                );
            }
            Measurement::InsufficientEvidence => {
                tracing::warn!(
                    captures_analysed = summary.captures_analysed,
                    "self_improvement_cycle: insufficient evidence for a critique score; score gate skipped"
                );
                WORKER_METRICS.record_self_improvement_not_evaluated();
            }
            Measurement::Measured(_) => {}
        }
    }

    // ── 4. Dataset persistence ─────────────────────────────────────────
    let dataset_persistence = match &cycle_report {
        Some(cycle_report) => {
            let dataset_version = format!(
                "{}-{}",
                Utc::now().format("%Y%m%dT%H%M%SZ"),
                cycle_report.cycle_id
            );
            let dataset_manifest = serde_json::json!({
                "dataset_name": "llm_self_improvement_examples",
                "dataset_version": dataset_version,
                "source_cycle_id": cycle_report.cycle_id,
                "eval_suite": evaluation.value_ref().map(|summary| summary.suite_name.clone()),
                "eval_run_id": evaluation.value_ref().map(|summary| summary.run_id.clone()),
                "example_count": training_examples.len(),
                "schema": "alpaca_chat_jsonl_v1",
                "tasks": ["insight_generation", "evidence_chain"],
            });
            match store
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
                Ok(_) => {
                    if !jsonl_examples.is_empty() {
                        tracing::info!(
                            training_examples = training_examples.len(),
                            jsonl_chars = jsonl_examples.chars().count(),
                            dataset_version = %dataset_version,
                            "self_improvement_cycle: persisted training examples dataset"
                        );
                    }
                    StageResult::success(DatasetPersistenceSummary {
                        dataset_version,
                        example_count: training_examples.len(),
                    })
                }
                Err(error) => StageResult::failed(StructuredFailure::new(
                    "dataset_persistence",
                    FailureKind::Storage,
                    format!("failed to persist training dataset artifact: {error}"),
                )),
            }
        }
        None => StageResult::failed(StructuredFailure::new(
            "dataset_persistence",
            FailureKind::MissingField,
            "training dataset persistence skipped: critique cycle failed",
        )),
    };

    let governance_persistence = if governance_failures.is_empty() {
        StageResult::success(GovernancePersistenceSummary {
            runs_persisted: governance_runs_persisted,
        })
    } else {
        StageResult::failed(governance_failures.remove(0))
    };

    // Record the persistence stages that have no learning-stage metric loop of
    // their own (the cycle's learning stages are recorded where they occur).
    if evaluation.has_failure() {
        WORKER_METRICS.record_self_improvement_stage_failure("evaluation");
    }
    if governance_persistence.has_failure() {
        WORKER_METRICS.record_self_improvement_stage_failure("governance_persistence");
    }
    if dataset_persistence.has_failure() {
        WORKER_METRICS.record_self_improvement_stage_failure("dataset_persistence");
    }

    // ── 5. Golden-set regression gate ──────────────────────────────────
    // A regression is governance state (degrade + withhold promotion), not a
    // cycle failure; persistence/export errors still fail loudly.
    let golden_set_regression = run_quality_gate_golden_set_regression(store, ingress)
        .await
        .context("llm self-improvement: quality gate golden set regression failed")?;
    tracing::info!(
        agreement = golden_set_regression.agreement,
        total_examples = golden_set_regression.total_examples,
        disagreements = golden_set_regression.disagreements.len(),
        below_target = golden_set_regression.below_target,
        "self_improvement_cycle: quality gate golden set regression completed"
    );
    let golden_set_below_target = golden_set_regression.below_target;

    Ok(ContinuousImprovementOutcome {
        golden_set_below_target,
        evaluation,
        critique,
        proposals,
        governance_persistence,
        dataset_persistence,
    })
}

#[cfg(all(test, feature = "llm"))]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn evaluation_summary() -> EvaluationStageSummary {
        EvaluationStageSummary {
            suite_name: "standard_eval_suite".to_string(),
            run_id: "run-1".to_string(),
            pass_rate: 0.9,
            avg_judge_score: Measurement::Measured(0.8),
            hallucination_rate: 0.1,
            total_cases: 10,
            failed_cases: 1,
            execution_errors: 0,
        }
    }

    const THRESHOLDS: EvalGateThresholds = EvalGateThresholds {
        min_pass_rate: 0.75,
        min_score: 0.62,
        max_hallucination_rate: 0.30,
    };

    /// Regression: an unreachable LLM produced a critical "LLM quality
    /// regression detected" broadcast. With zero graded cases the gate must
    /// report NotEvaluated, never a breach.
    #[test]
    fn eval_gate_with_no_graded_case_is_not_evaluated() {
        assert_eq!(
            eval_gate_decision(0, 0.0, &Measurement::not_measured(), 0.0, THRESHOLDS),
            EvalGateDecision::NotEvaluated
        );
        assert_eq!(
            eval_gate_decision(0, 0.0, &Measurement::not_measured(), 1.0, THRESHOLDS),
            EvalGateDecision::NotEvaluated
        );
    }

    #[test]
    fn eval_gate_passes_healthy_run_with_unmeasured_judge() {
        assert_eq!(
            eval_gate_decision(4, 1.0, &Measurement::not_measured(), 0.0, THRESHOLDS),
            EvalGateDecision::Pass
        );
    }

    #[test]
    fn eval_gate_breach_severity() {
        assert_eq!(
            eval_gate_decision(4, 0.70, &Measurement::Measured(0.9), 0.0, THRESHOLDS),
            EvalGateDecision::Breach { severity: "high" }
        );
        assert_eq!(
            eval_gate_decision(4, 0.25, &Measurement::Measured(0.9), 0.0, THRESHOLDS),
            EvalGateDecision::Breach {
                severity: "critical"
            }
        );
        assert_eq!(
            eval_gate_decision(4, 1.0, &Measurement::Measured(0.5), 0.0, THRESHOLDS),
            EvalGateDecision::Breach { severity: "high" }
        );
        assert_eq!(
            eval_gate_decision(4, 1.0, &Measurement::Measured(0.9), 0.5, THRESHOLDS),
            EvalGateDecision::Breach {
                severity: "critical"
            }
        );
    }

    fn critique_summary() -> CritiqueStageSummary {
        CritiqueStageSummary {
            cycle_id: "cycle-1".to_string(),
            captures_seeded: 4,
            captures_analysed: 4,
            qualifying_examples: 2,
            avg_critique_score: Measurement::Measured(0.7),
        }
    }

    fn outcome_with(
        governance_persistence: StageResult<GovernancePersistenceSummary>,
        dataset_persistence: StageResult<DatasetPersistenceSummary>,
    ) -> ContinuousImprovementOutcome {
        ContinuousImprovementOutcome {
            golden_set_below_target: false,
            evaluation: StageResult::success(evaluation_summary()),
            critique: StageResult::success(critique_summary()),
            proposals: StageResult::success(ProposalsStageSummary {
                prompt_improvements: 1,
                failure_hypotheses: 1,
            }),
            governance_persistence,
            dataset_persistence,
        }
    }

    #[test]
    fn governance_persistence_failure_degrades_instead_of_failing() {
        let outcome = outcome_with(
            StageResult::failed(StructuredFailure::new(
                "governance_persistence",
                FailureKind::Storage,
                "insert failed",
            )),
            StageResult::success(DatasetPersistenceSummary {
                dataset_version: "v1".to_string(),
                example_count: 2,
            }),
        );

        assert!(!outcome.has_terminal_learning_failure());
        assert!(outcome.persistence_degraded());
        let failures = outcome.failures();
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].stage, "governance_persistence");
    }

    #[test]
    fn dataset_persistence_failure_degrades_instead_of_failing() {
        let outcome = outcome_with(
            StageResult::success(GovernancePersistenceSummary { runs_persisted: 2 }),
            StageResult::failed(StructuredFailure::new(
                "dataset_persistence",
                FailureKind::Storage,
                "insert failed",
            )),
        );

        assert!(!outcome.has_terminal_learning_failure());
        assert!(outcome.persistence_degraded());
    }

    #[test]
    fn critique_failure_is_terminal() {
        let mut outcome = outcome_with(
            StageResult::success(GovernancePersistenceSummary { runs_persisted: 2 }),
            StageResult::success(DatasetPersistenceSummary {
                dataset_version: "v1".to_string(),
                example_count: 2,
            }),
        );
        outcome.critique = StageResult::failed(StructuredFailure::new(
            "critique",
            FailureKind::ModelCallFailed,
            "timeout",
        ));

        assert!(outcome.has_terminal_learning_failure());
        assert!(!outcome.persistence_degraded());
    }

    #[test]
    fn stage_failures_are_retained_with_their_reason() {
        let outcome = outcome_with(
            StageResult::failed(StructuredFailure::new(
                "governance_persistence",
                FailureKind::InvalidResponse,
                "cycle report failed to serialize",
            )),
            StageResult::failed(StructuredFailure::new(
                "dataset_persistence",
                FailureKind::MissingField,
                "cycle report unavailable",
            )),
        );

        let failures = outcome.failures();
        assert_eq!(failures.len(), 2);
        assert!(failures
            .iter()
            .any(|failure| failure.kind == FailureKind::InvalidResponse));
        assert!(outcome.persistence_degraded());
    }
}
