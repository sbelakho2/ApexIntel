use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use crate::*;

const WEEKLY_STAGE_TIMEOUT: Duration = Duration::from_secs(45);
const WEEKLY_STAGE_ATTEMPTS: usize = 3;

/// Calibration rewrites recipe activation thresholds from false-positive
/// evidence (#159); it belongs to the promotion board only. Running it from
/// the deprecation audit would mutate production gates as a side effect of a
/// monitoring pass.
pub(super) fn calibration_eligible(kind: &JobKind) -> bool {
    matches!(kind, JobKind::PromotionBoard)
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
pub(super) async fn run_weekly_recipe_job(kind: &JobKind, store: &Arc<PgStore>) -> JobRun {
    let mut run = JobRun::new(kind.clone());
    run.start();
    let ctx = StorageContext {
        store: PgStore::from_pool(store.pool.clone()),
        run_timestamp: Utc::now(),
        activity_logger: ActivityLogger::new(store.pool.clone()),
    };
    if let Err(e) = super::resilience::run_stage_with_retry(
        "weekly_pipeline.persist_metrics",
        WEEKLY_STAGE_TIMEOUT,
        WEEKLY_STAGE_ATTEMPTS,
        |_| async {
            ctx.store
                .record_recipe_weekly_metrics(ctx.run_timestamp)
                .await
        },
    )
    .await
    {
        run.fail(&format!(
            "weekly_pipeline: failed to persist recipe weekly metrics: {e}"
        ));
        return run;
    }
    // Stage: auto-calibrate recipe precision thresholds based on FP rates.
    // #159: only the promotion board calibrates; the deprecation audit must
    // not mutate recipe activation thresholds.
    if calibration_eligible(kind) {
        if let Err(e) = super::resilience::run_stage_with_retry(
            "weekly_pipeline.auto_calibrate_thresholds",
            WEEKLY_STAGE_TIMEOUT,
            WEEKLY_STAGE_ATTEMPTS,
            |_| async {
                let adjustments = ctx.store.auto_calibrate_recipe_thresholds().await?;
                if !adjustments.is_empty() {
                    tracing::info!(
                        "auto-calibrated {} recipe(s) with high FP rate",
                        adjustments.len()
                    );
                }
                Ok::<_, anyhow::Error>(())
            },
        )
        .await
        {
            run.fail(&format!(
                "weekly_pipeline: failed to auto-calibrate recipe thresholds: {e}"
            ));
            return run;
        }
    }
    let (staged_recipes, production_recipes, memo_inputs) =
        match super::resilience::run_stage_with_retry(
            "weekly_pipeline.load_inputs",
            WEEKLY_STAGE_TIMEOUT,
            WEEKLY_STAGE_ATTEMPTS,
            |_| async {
                tokio::try_join!(
                    load_staged_recipes(&ctx),
                    load_production_recipes(&ctx),
                    build_memo_inputs(&ctx),
                )
            },
        )
        .await
        {
            Ok(triple) => triple,
            Err(e) => {
                run.fail(&format!(
                    "weekly_pipeline: failed to load inputs from DB: {e}"
                ));
                return run;
            }
        };
    let report = run_weekly_pipeline(
        &staged_recipes,
        &production_recipes,
        &memo_inputs,
        &Default::default(),
        &Default::default(),
    );
    // #158: the promotion board / deprecation audit used to only compute and
    // log its decisions — the recipe rows were never transitioned, so a recipe
    // that passed the board stayed `staging` forever and a deprecated recipe
    // kept firing. Apply the decision to the recipes table, and only treat the
    // run as successful when every decision was either applied or already
    // satisfied by the database.
    let mut actions = recipe_lifecycle_actions(kind, &report);
    // Constant-testing layer: a promotion is applied only when the measured
    // review telemetry passes the eval gate (significant improvement over the
    // previous week, no critical regression, explicit analyst truth). The gate
    // fails closed: on any telemetry error the promotion is withheld.
    if calibration_eligible(kind) {
        actions = gate_promotions_with_eval(store, &actions).await;
    }
    let applied = match apply_recipe_lifecycle(store, &ctx.activity_logger, &actions).await {
        Ok(applied) => applied,
        Err(error) => {
            run.fail(&error);
            return run;
        }
    };
    if report.overall_success {
        run.succeed(applied, &report.summary());
    } else {
        run.fail(&report.summary());
    }
    run
}

/// Which lifecycle transition a weekly report asks the worker to apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RecipeLifecycleOp {
    Promote,
    Deprecate,
}

/// A single recipe transition derived from the pure weekly report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RecipeLifecycleAction {
    pub(super) recipe_code: String,
    pub(super) op: RecipeLifecycleOp,
}

/// Filter the promotion actions through the measured-telemetry eval gate.
///
/// Each promotion candidate must show a statistically significant improvement
/// in measured precision / false-positive rate over the previous week on real
/// analyst-reviewed warnings, with no critical regression.  The gate fails
/// closed: telemetry errors, missing snapshots, and unmeasured critical
/// metrics withhold the promotion and are recorded as audit events so the
/// decision is reproducible.
async fn gate_promotions_with_eval(
    store: &Arc<PgStore>,
    actions: &[RecipeLifecycleAction],
) -> Vec<RecipeLifecycleAction> {
    use apex_learning::evaluation::{
        evaluate_weekly_promotion, AnalystSignalClass, EvaluationRun, FrozenEvalSetRef,
        LearningMetric, MetricObservation, PromotionDecision, PromotionGateConfig,
    };

    let promote_codes: Vec<String> = actions
        .iter()
        .filter(|action| action.op == RecipeLifecycleOp::Promote)
        .map(|action| action.recipe_code.clone())
        .collect();
    if promote_codes.is_empty() {
        return actions.to_vec();
    }

    let snapshots = match store.list_recipe_weekly_snapshots(&promote_codes, 2).await {
        Ok(snapshots) => snapshots,
        Err(error) => {
            tracing::warn!(
                %error,
                "weekly_pipeline: promotion eval gate telemetry failed; withholding all promotions (fail closed)"
            );
            for code in &promote_codes {
                record_gate_rejection(store, code, "eval_telemetry_unavailable").await;
            }
            return actions
                .iter()
                .filter(|action| action.op == RecipeLifecycleOp::Deprecate)
                .cloned()
                .collect();
        }
    };

    let mut by_recipe: std::collections::HashMap<
        &str,
        Vec<&apex_store::postgres::RecipeWeeklySnapshot>,
    > = std::collections::HashMap::new();
    for snapshot in &snapshots {
        by_recipe
            .entry(snapshot.recipe_code.as_str())
            .or_default()
            .push(snapshot);
    }

    let config = PromotionGateConfig {
        // Aligned with the promotion board's own policy (#160: >= 10 reviewed
        // warnings before promoting or deprecating); stricter than the
        // library default of 30 only in this caller.
        min_sample_size: 10,
        ..PromotionGateConfig::default()
    };

    let measured = [LearningMetric::Precision, LearningMetric::FalsePositiveRate];

    let mut kept: Vec<RecipeLifecycleAction> = Vec::new();
    for action in actions {
        if action.op == RecipeLifecycleOp::Deprecate {
            kept.push(action.clone());
            continue;
        }
        let Some(weeks) = by_recipe.get(action.recipe_code.as_str()) else {
            tracing::warn!(
                recipe_code = %action.recipe_code,
                "weekly_pipeline: promotion withheld — no measured weekly telemetry (fail closed)"
            );
            record_gate_rejection(store, &action.recipe_code, "no_measured_telemetry").await;
            continue;
        };
        let (Some(candidate_week), Some(baseline_week)) = (weeks.first(), weeks.get(1)) else {
            tracing::warn!(
                recipe_code = %action.recipe_code,
                weeks = weeks.len(),
                "weekly_pipeline: promotion withheld — needs two measured weeks (fail closed)"
            );
            record_gate_rejection(store, &action.recipe_code, "insufficient_weeks").await;
            continue;
        };

        let run = |week: &apex_store::postgres::RecipeWeeklySnapshot, label: &str| {
            let reviewed = week.reviewed_warnings.max(0) as u64;
            EvaluationRun {
                run_id: format!("{}-{}", action.recipe_code, label),
                eval_set: FrozenEvalSetRef {
                    id: "weekly_promotion_board_reviews".to_string(),
                    name: "weekly_promotion_board_reviews".to_string(),
                    version: 1,
                    example_count: reviewed,
                    examples_digest: String::new(),
                },
                eval_set_digest: String::new(),
                candidate: apex_learning::evaluation::CandidateRef {
                    kind: apex_learning::evaluation::CandidateKind::Recipe,
                    reference: action.recipe_code.clone(),
                    version: Some(week.week_start.to_string()),
                },
                candidate_artifact_hash: week.week_start.to_string(),
                baseline_artifact_hash: None,
                baseline_artifact_version: None,
                metrics_version: 1,
                observations: vec![
                    MetricObservation {
                        metric: LearningMetric::Precision,
                        value: week.precision_score.clamp(0.0, 1.0),
                        sample_size: reviewed,
                        signal_class: AnalystSignalClass::PositiveConfirmation,
                        is_training_truth: true,
                        is_critical: false,
                    },
                    MetricObservation {
                        metric: LearningMetric::FalsePositiveRate,
                        value: week.false_positive_rate.clamp(0.0, 1.0),
                        sample_size: reviewed,
                        signal_class: AnalystSignalClass::PositiveConfirmation,
                        is_training_truth: true,
                        is_critical: true,
                    },
                ],
            }
        };

        let decision = evaluate_weekly_promotion(
            &run(candidate_week, "candidate"),
            &run(baseline_week, "baseline"),
            &config,
            &measured,
        );
        match decision {
            PromotionDecision::Promote { .. } => {
                tracing::info!(
                    recipe_code = %action.recipe_code,
                    "weekly_pipeline: promotion eval gate passed"
                );
                kept.push(action.clone());
            }
            PromotionDecision::Reject(rejection) => {
                tracing::warn!(
                    recipe_code = %action.recipe_code,
                    ?rejection,
                    "weekly_pipeline: promotion withheld by eval gate"
                );
                record_gate_rejection(store, &action.recipe_code, &format!("{rejection:?}")).await;
            }
        }
    }
    kept
}

async fn record_gate_rejection(store: &Arc<PgStore>, recipe_code: &str, reason: &str) {
    if let Err(error) = store
        .record_audit_event(
            "worker",
            "recipe_promotion_gate_rejected",
            &serde_json::json!({
                "recipe_code": recipe_code,
                "reason": reason,
            }),
        )
        .await
    {
        tracing::warn!(
            recipe_code,
            %error,
            "weekly_pipeline: failed to record promotion-gate audit event"
        );
    }
}

/// Pure decision helper: map a weekly report + job kind to the transitions the
/// worker must persist. `PromotionBoard` applies every promoted recipe;
/// `RecipeDeprecation` applies every deprecated recipe. Other weekly kinds ask
/// for no lifecycle transition.
pub(super) fn recipe_lifecycle_actions(
    kind: &JobKind,
    report: &apex_worker::weekly::WeeklyReport,
) -> Vec<RecipeLifecycleAction> {
    match kind {
        JobKind::PromotionBoard => report
            .promotion_result
            .as_ref()
            .map(|result| {
                result
                    .promoted
                    .iter()
                    .map(|(recipe_code, _reason)| RecipeLifecycleAction {
                        recipe_code: recipe_code.clone(),
                        op: RecipeLifecycleOp::Promote,
                    })
                    .collect()
            })
            .unwrap_or_default(),
        JobKind::RecipeDeprecation => report
            .deprecation_result
            .as_ref()
            .map(|result| {
                result
                    .deprecated
                    .iter()
                    .map(|(recipe_code, _reason)| RecipeLifecycleAction {
                        recipe_code: recipe_code.clone(),
                        op: RecipeLifecycleOp::Deprecate,
                    })
                    .collect()
            })
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// Apply the lifecycle transitions, returning how many rows actually changed.
///
/// Activity-feed logging and the `audit_log` record are written ONLY when the
/// store reports `Ok(true)` (the row really transitioned). `Ok(false)` means
/// the row was already in the target state (or vanished); it is logged at
/// debug level and does not fail the run. Any store error fails the run — a
/// promotion/deprecation that could not be persisted must never be reported as
/// a success.
async fn apply_recipe_lifecycle(
    store: &Arc<PgStore>,
    activity_logger: &ActivityLogger,
    actions: &[RecipeLifecycleAction],
) -> Result<u64, String> {
    let mut applied = 0u64;
    for action in actions {
        match action.op {
            RecipeLifecycleOp::Promote => match store.promote_recipe(&action.recipe_code).await {
                Ok(true) => {
                    activity_logger
                        .log_recipe_promoted(&action.recipe_code, "promotion_board")
                        .await;
                    if let Err(error) = store
                        .record_audit_event(
                            "worker",
                            "recipe_promoted",
                            &serde_json::json!({
                                "recipe_code": action.recipe_code,
                                "source": "weekly_promotion_board",
                            }),
                        )
                        .await
                    {
                        tracing::warn!(
                            recipe_code = %action.recipe_code,
                            %error,
                            "weekly_pipeline: failed to write promotion audit event"
                        );
                    }
                    applied += 1;
                }
                Ok(false) => tracing::debug!(
                    recipe_code = %action.recipe_code,
                    "weekly_pipeline: promotion already applied; no row changed"
                ),
                Err(error) => {
                    return Err(format!(
                        "weekly_pipeline: failed to promote recipe {}: {error}",
                        action.recipe_code
                    ));
                }
            },
            RecipeLifecycleOp::Deprecate => {
                match store.deprecate_recipe(&action.recipe_code).await {
                    Ok(true) => {
                        activity_logger
                            .log_recipe_deprecated(&action.recipe_code, "deprecation_audit")
                            .await;
                        if let Err(error) = store
                            .record_audit_event(
                                "worker",
                                "recipe_deprecated",
                                &serde_json::json!({
                                    "recipe_code": action.recipe_code,
                                    "source": "weekly_deprecation_audit",
                                }),
                            )
                            .await
                        {
                            tracing::warn!(
                                recipe_code = %action.recipe_code,
                                %error,
                                "weekly_pipeline: failed to write deprecation audit event"
                            );
                        }
                        applied += 1;
                    }
                    Ok(false) => tracing::debug!(
                        recipe_code = %action.recipe_code,
                        "weekly_pipeline: deprecation already applied; no row changed"
                    ),
                    Err(error) => {
                        return Err(format!(
                            "weekly_pipeline: failed to deprecate recipe {}: {error}",
                            action.recipe_code
                        ));
                    }
                }
            }
        }
    }
    Ok(applied)
}

/// Insight severity: a stored `metadata.severity` wins only when it is one of
/// the four valid values (case-insensitive); otherwise severity is derived
/// from the measured confidence band:
///
/// | confidence      | severity   |
/// |-----------------|------------|
/// | `>= 0.85`       | `critical` |
/// | `0.70 .. 0.85`  | `high`     |
/// | `0.40 .. 0.70`  | `medium`   |
/// | `< 0.40`        | `low`      |
///
/// Never a hard-coded constant (#161).
pub(super) fn severity_from_metadata_or_confidence(
    metadata: Option<&serde_json::Value>,
    confidence: f64,
) -> String {
    if let Some(stored) = metadata
        .and_then(|value| value.get("severity"))
        .and_then(|value| value.as_str())
    {
        let normalized = stored.trim().to_ascii_lowercase();
        if matches!(normalized.as_str(), "high" | "medium" | "low" | "critical") {
            return normalized;
        }
    }
    if confidence >= 0.85 {
        "critical".to_string()
    } else if confidence >= 0.70 {
        "high".to_string()
    } else if confidence >= 0.40 {
        "medium".to_string()
    } else {
        "low".to_string()
    }
}

/// Resolve an insight's display entity name from its `entity_ids`, preferring
/// companies and then persons. Returns an empty string when nothing resolves —
/// never the insight title and never a fabricated name (#161).
pub(super) fn resolve_entity_name(
    entity_ids: &[Uuid],
    company_names: &HashMap<Uuid, String>,
    person_names: &HashMap<Uuid, String>,
) -> String {
    for id in entity_ids {
        if let Some(name) = company_names.get(id) {
            return name.clone();
        }
    }
    for id in entity_ids {
        if let Some(name) = person_names.get(id) {
            return name.clone();
        }
    }
    String::new()
}

pub(super) async fn run_strategy_memo(kind: &JobKind, store: &Arc<PgStore>) -> JobRun {
    let mut run = JobRun::new(kind.clone());
    run.start();
    #[cfg(feature = "llm")]
    {
        let filters = InsightListFilters {
            regions: vec![],
            date_from: Some(Utc::now() - chrono::Duration::days(7)),
            date_to: None,
            search: None,
            insight_types: vec![],
            exclude_internal: false,
            bookmarked_by: None,
        };
        let insight_rows = match super::resilience::run_stage_with_retry(
            "strategy_memo.load_insights",
            WEEKLY_STAGE_TIMEOUT,
            WEEKLY_STAGE_ATTEMPTS,
            |_| async { store.list_insights(&filters, 200, 0).await },
        )
        .await
        {
            Ok(rows) => rows,
            Err(e) => {
                run.fail(&format!("strategy_memo: failed to load insights: {e}"));
                return run;
            }
        };

        // #161: resolve entity names from the store (companies first, then
        // persons). A failed lookup fails the stage instead of fabricating a
        // name from the insight title.
        let entity_ids: Vec<Uuid> = insight_rows
            .iter()
            .flat_map(|row| row.entity_ids.iter().flatten().copied())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        let company_names: HashMap<Uuid, String> =
            match store.get_company_names_by_ids(&entity_ids).await {
                Ok(rows) => rows
                    .into_iter()
                    .map(|(id, name, _region, _company_type)| (id, name))
                    .collect(),
                Err(e) => {
                    run.fail(&format!(
                        "strategy_memo: failed to resolve company names: {e}"
                    ));
                    return run;
                }
            };
        let person_names: HashMap<Uuid, String> =
            match store.get_person_names_by_ids(&entity_ids).await {
                Ok(rows) => rows
                    .into_iter()
                    .map(|(id, name, _role)| (id, name))
                    .collect(),
                Err(e) => {
                    run.fail(&format!(
                        "strategy_memo: failed to resolve person names: {e}"
                    ));
                    return run;
                }
            };

        let cards: Vec<InsightCard> = insight_rows
            .iter()
            .map(|row| {
                let confidence = row.confidence.unwrap_or(0.5);
                let impact_label = if confidence >= 0.7 {
                    "High"
                } else if confidence >= 0.4 {
                    "Medium"
                } else {
                    "Low"
                };
                let citations: Vec<Citation> = row
                    .evidence_urls
                    .as_deref()
                    .unwrap_or(&[])
                    .iter()
                    .enumerate()
                    .map(|(i, url)| {
                        let domain = url
                            .split("//")
                            .nth(1)
                            .and_then(|s| s.split('/').next())
                            .unwrap_or(url)
                            .to_string();
                        Citation {
                            index: i + 1,
                            source_url: url.clone(),
                            source_domain: domain,
                            observed_at: row.created_at,
                        }
                    })
                    .collect();
                InsightCard {
                    id: row.id,
                    recipe_code: row
                        .insight_type
                        .clone()
                        .unwrap_or_else(|| "general".to_string()),
                    entity_id: row
                        .entity_ids
                        .as_deref()
                        .and_then(|ids| ids.first().copied())
                        .unwrap_or(uuid::Uuid::nil()),
                    entity_name: resolve_entity_name(
                        row.entity_ids.as_deref().unwrap_or(&[]),
                        &company_names,
                        &person_names,
                    ),
                    severity: severity_from_metadata_or_confidence(
                        row.metadata.as_ref(),
                        confidence,
                    ),
                    category: row
                        .insight_type
                        .clone()
                        .unwrap_or_else(|| "general".to_string()),
                    title: row.title.clone(),
                    narrative: row.summary.clone(),
                    actions: row.tags.clone().unwrap_or_default(),
                    citations,
                    confidence,
                    impact: confidence,
                    impact_label: impact_label.to_string(),
                    priority_score: confidence,
                    region: row.region.clone(),
                    rendered_at: row.created_at.unwrap_or_else(Utc::now),
                }
            })
            .collect();

        let card_count = cards.len();
        let config = WeeklyPipelineConfig::default();
        let runner = WeeklyPipelineRunner::headless(config);
        match super::resilience::run_stage_with_retry(
            "strategy_memo.generate_memo",
            Duration::from_secs(120),
            2,
            |_| async { runner.run(cards.clone()).await },
        )
        .await
        {
            Ok(output) => {
                let section_count = output.memo.regional_sections.len();
                let memo = &output.memo;
                let now = Utc::now();
                let week_start = chrono::NaiveDate::from_isoywd_opt(
                    memo.year,
                    memo.week_number,
                    chrono::Weekday::Mon,
                )
                .unwrap_or_else(|| now.date_naive());
                let week_end = week_start + chrono::Duration::days(6);
                let title = format!(
                    "Weekly Intelligence Memo — Week {}/{}",
                    memo.week_number, memo.year
                );
                let mut sections_payload = vec![serde_json::json!({
                    "title": "Momentum and Deltas",
                    "content": format!(
                        "{}\nEarly period: {} | Late period: {} | Direction: {}",
                        memo.temporal_summary.narrative,
                        memo.temporal_summary.early_period_count,
                        memo.temporal_summary.late_period_count,
                        memo.temporal_summary.volume_delta.label,
                    ),
                    "priority": "medium",
                    "related_warnings": [],
                    "related_insights": [],
                })];
                if !memo.fused_signal_clusters.is_empty() {
                    sections_payload.push(serde_json::json!({
                        "title": "Weak-Signal Fusion",
                        "content": memo
                            .fused_signal_clusters
                            .iter()
                            .take(5)
                            .map(|cluster| format!(
                                "{} [{}] — {} signals / {} independent sources / score {:.2}",
                                cluster.theme,
                                cluster.label,
                                cluster.signal_count,
                                cluster.independent_source_count,
                                cluster.combined_score,
                            ))
                            .collect::<Vec<_>>()
                            .join("\n"),
                        "priority": "medium",
                        "related_warnings": [],
                        "related_insights": [],
                    }));
                }
                if !memo.competing_hypotheses.is_empty() {
                    sections_payload.push(serde_json::json!({
                        "title": "Competing Hypotheses",
                        "content": memo
                            .competing_hypotheses
                            .iter()
                            .take(3)
                            .map(|hypothesis| format!(
                                "{} — {} (heuristic score {:.0}%, support {:.2}, contradiction {:.2})",
                                hypothesis.hypothesis,
                                hypothesis.assessment,
                                hypothesis.heuristic_score * 100.0,
                                hypothesis.support_score,
                                hypothesis.contradiction_score,
                            ))
                            .collect::<Vec<_>>()
                            .join("\n"),
                        "priority": "medium",
                        "related_warnings": [],
                        "related_insights": [],
                    }));
                }
                sections_payload.extend(
                    memo.regional_sections
                        .iter()
                        .map(|s| serde_json::json!({
                            "title": format!("{} ({})", s.region_label, s.region),
                            "content": s
                                .top_insights
                                .iter()
                                .map(|i| format!(
                                    "[{}] {} — {} | evidence {} ({:.2})",
                                    i.severity.to_uppercase(),
                                    i.entity_name,
                                    i.title,
                                    i.evidence_quality_label,
                                    i.evidence_quality_score,
                                ))
                                .collect::<Vec<_>>()
                                .join("\n"),
                            "priority": if s.top_insights.iter().any(|insight| insight.severity == "critical") {
                                "high"
                            } else {
                                "medium"
                            },
                            "related_warnings": [],
                            "related_insights": s.top_insights.iter().map(|insight| insight.recipe_code.clone()).collect::<Vec<_>>(),
                        }))
                        .collect::<Vec<_>>()
                );
                let sections_json = serde_json::json!(sections_payload);
                // #161: real store counts for the monitored universe. A failed
                // count fails the stage; a fabricated zero is never persisted.
                let companies_monitored = match store
                    .count_companies(&apex_store::postgres::CompanyListFilters::default())
                    .await
                {
                    Ok(count) => count,
                    Err(e) => {
                        run.fail(&format!(
                            "strategy_memo: failed to count monitored companies: {e}"
                        ));
                        return run;
                    }
                };
                let pois_tracked = match store.count_persons(&PersonListFilters::default()).await {
                    Ok(count) => count,
                    Err(e) => {
                        run.fail(&format!("strategy_memo: failed to count tracked POIs: {e}"));
                        return run;
                    }
                };
                let key_metrics_json = serde_json::json!({
                    "warnings_total": memo.warning_count,
                    "warnings_critical": memo.critical_count,
                    "insights_generated": memo.total_insights,
                    "companies_monitored": companies_monitored,
                    "pois_tracked": pois_tracked,
                    "late_period_count": memo.temporal_summary.late_period_count,
                    "early_period_count": memo.temporal_summary.early_period_count,
                    "fused_signal_clusters": memo.fused_signal_clusters.len(),
                });
                let action_items_json = serde_json::json!(memo
                    .top_actions
                    .iter()
                    .take(10)
                    .map(|a| serde_json::json!({
                        "text": a.action,
                        "priority": a.impact_label.to_ascii_lowercase(),
                        "assignee": serde_json::Value::Null,
                        "rank": a.priority,
                        "entity": a.entity_name,
                        "impact": a.impact_label,
                        "confidence": a.confidence,
                    }))
                    .collect::<Vec<_>>());
                if let Err(e) = store
                    .upsert_weekly_memo(
                        &title,
                        week_start,
                        week_end,
                        &memo.executive_summary,
                        sections_json,
                        key_metrics_json,
                        action_items_json,
                    )
                    .await
                {
                    // #161: a memo that was not persisted is not a successful
                    // stage. Fail the run and do not log memo-generated
                    // activity for a memo that does not exist.
                    run.fail(&format!("strategy_memo: failed to persist memo: {e}"));
                    return run;
                }
                tracing::info!(
                    week = memo.week_number,
                    year = memo.year,
                    sections = section_count,
                    "strategy_memo: memo persisted to weekly_memos"
                );
                // Surface the generated memo in the activity feed.
                let memo_logger =
                    apex_worker::activity_logger::ActivityLogger::new(store.pool.clone());
                memo_logger
                    .log_memo_generated(&title, card_count as u32)
                    .await;
                run.succeed(
                    section_count as u64,
                    &format!(
                        "strategy_memo: pipeline complete — {} cards → {} sections (llm_narrated={})",
                        card_count,
                        section_count,
                        output.llm_narrated,
                    ),
                );
            }
            Err(e) => {
                run.fail(&format!("strategy_memo: pipeline failed: {e}"));
            }
        }
    }
    #[cfg(not(feature = "llm"))]
    {
        // false-success-classification: best-effort — discarded binding only; no fallible call in this statement
        let _ = store;
        match load_weekly_inputs().await {
            Ok(inputs) => {
                let report = run_weekly_pipeline(
                    &inputs.staged_recipes,
                    &inputs.production_recipes,
                    &inputs.memo_inputs,
                    // false-success-classification: best-effort — optional/display value default; failure renders empty rather than asserting persistence
                    &inputs.promotion_policy.unwrap_or_default(),
                    // false-success-classification: best-effort — optional/display value default; failure renders empty rather than asserting persistence
                    &inputs.deprecation_policy.unwrap_or_default(),
                );
                if report.overall_success {
                    let items = report
                        .memo
                        .as_ref()
                        .map(|m| m.sections.len() as u64)
                        .unwrap_or(0);
                    run.succeed(items, &report.summary());
                } else {
                    run.fail(&report.summary());
                }
            }
            Err(err) => {
                run.skip(&format!(
                    "strategy_memo: weekly inputs unavailable: {}",
                    err
                ));
            }
        }
    }
    run
}

pub(super) async fn run_update_email_digest(store: &Arc<PgStore>) -> JobRun {
    let mut run = JobRun::new(JobKind::UpdateEmailDigest);
    run.start();
    match run_update_email_digest_job(store).await {
        Ok((sent, due)) => {
            run.succeed(
                sent,
                &format!(
                    "update_email_digest: users_due={}, digests_sent={}",
                    due, sent
                ),
            );
        }
        Err(e) => run.fail(&format!("update_email_digest failed: {e}")),
    }
    run
}

#[cfg(test)]
mod lifecycle_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::{
        calibration_eligible, recipe_lifecycle_actions, RecipeLifecycleAction, RecipeLifecycleOp,
    };
    use crate::JobKind;
    use apex_worker::weekly::{DeprecationResult, PromotionBoardResult, WeeklyReport};

    fn report_with(
        promoted: Vec<(String, String)>,
        deprecated: Vec<(String, String)>,
    ) -> WeeklyReport {
        let mut report = WeeklyReport::new();
        report.promotion_result = Some(PromotionBoardResult {
            promoted,
            kept: Vec::new(),
            rejected: Vec::new(),
        });
        report.deprecation_result = Some(DeprecationResult {
            deprecated,
            kept: Vec::new(),
        });
        report
    }

    /// #158: every promoted recipe in the report must become a promote action,
    /// so the recipes table is actually transitioned (previously only logged).
    #[test]
    fn promotion_board_maps_every_promoted_recipe() {
        let report = report_with(
            vec![
                ("recipe_a".to_string(), "precision ok".to_string()),
                ("recipe_b".to_string(), "precision ok".to_string()),
            ],
            vec![("stale_recipe".to_string(), "inactive".to_string())],
        );

        let actions = recipe_lifecycle_actions(&JobKind::PromotionBoard, &report);

        assert_eq!(
            actions,
            vec![
                RecipeLifecycleAction {
                    recipe_code: "recipe_a".to_string(),
                    op: RecipeLifecycleOp::Promote,
                },
                RecipeLifecycleAction {
                    recipe_code: "recipe_b".to_string(),
                    op: RecipeLifecycleOp::Promote,
                },
            ]
        );
    }

    /// #158: every deprecated recipe in the report must become a deprecate
    /// action, and promotion decisions must not leak into this job.
    #[test]
    fn deprecation_audit_maps_every_deprecated_recipe() {
        let report = report_with(
            vec![("recipe_a".to_string(), "precision ok".to_string())],
            vec![
                ("stale_recipe".to_string(), "inactive".to_string()),
                ("bad_recipe".to_string(), "high FPR".to_string()),
            ],
        );

        let actions = recipe_lifecycle_actions(&JobKind::RecipeDeprecation, &report);

        assert_eq!(
            actions,
            vec![
                RecipeLifecycleAction {
                    recipe_code: "stale_recipe".to_string(),
                    op: RecipeLifecycleOp::Deprecate,
                },
                RecipeLifecycleAction {
                    recipe_code: "bad_recipe".to_string(),
                    op: RecipeLifecycleOp::Deprecate,
                },
            ]
        );
    }

    /// #158: weekly jobs that are not the promotion board or the deprecation
    /// audit (e.g. strategy memo) must never mutate recipe lifecycle state.
    #[test]
    fn other_weekly_kinds_request_no_lifecycle_transition() {
        let report = report_with(
            vec![("recipe_a".to_string(), "precision ok".to_string())],
            vec![("stale_recipe".to_string(), "inactive".to_string())],
        );

        assert!(recipe_lifecycle_actions(&JobKind::StrategyMemo, &report).is_empty());
    }

    #[test]
    fn missing_results_request_no_lifecycle_transition() {
        let report = WeeklyReport::new();
        assert!(recipe_lifecycle_actions(&JobKind::PromotionBoard, &report).is_empty());
        assert!(recipe_lifecycle_actions(&JobKind::RecipeDeprecation, &report).is_empty());
    }

    /// #159: threshold calibration mutates production recipes and must run
    /// only for the promotion board job; the deprecation audit (and every
    /// other weekly kind) must not calibrate.
    #[test]
    fn calibration_runs_only_for_the_promotion_board() {
        assert!(calibration_eligible(&JobKind::PromotionBoard));
        assert!(!calibration_eligible(&JobKind::RecipeDeprecation));
        assert!(!calibration_eligible(&JobKind::StrategyMemo));
    }
}

#[cfg(test)]
mod memo_helper_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::{resolve_entity_name, severity_from_metadata_or_confidence};
    use std::collections::HashMap;
    use uuid::Uuid;

    fn metadata(severity: &str) -> serde_json::Value {
        serde_json::json!({ "severity": severity })
    }

    /// #161: a valid stored severity wins over the confidence band.
    #[test]
    fn stored_severity_is_used_when_valid() {
        assert_eq!(
            severity_from_metadata_or_confidence(Some(&metadata("critical")), 0.10),
            "critical"
        );
        assert_eq!(
            severity_from_metadata_or_confidence(Some(&metadata("HIGH")), 0.10),
            "high"
        );
        assert_eq!(
            severity_from_metadata_or_confidence(Some(&metadata(" low ")), 0.99),
            "low"
        );
    }

    /// #161: an invalid or absent stored severity falls back to the measured
    /// confidence band, never to a hard-coded constant.
    #[test]
    fn invalid_stored_severity_derives_from_confidence() {
        assert_eq!(
            severity_from_metadata_or_confidence(Some(&metadata("urgent")), 0.90),
            "critical"
        );
        assert_eq!(
            severity_from_metadata_or_confidence(Some(&serde_json::json!({"severity": 7})), 0.75),
            "high"
        );
        assert_eq!(
            severity_from_metadata_or_confidence(Some(&serde_json::json!({})), 0.50),
            "medium"
        );
        assert_eq!(severity_from_metadata_or_confidence(None, 0.10), "low");
    }

    /// #161: documented confidence bands (critical >= .85, high >= .70,
    /// medium >= .40, else low).
    #[test]
    fn severity_confidence_band_boundaries() {
        for (confidence, expected) in [
            (1.0, "critical"),
            (0.85, "critical"),
            (0.8499, "high"),
            (0.70, "high"),
            (0.6999, "medium"),
            (0.40, "medium"),
            (0.3999, "low"),
            (0.0, "low"),
        ] {
            assert_eq!(
                severity_from_metadata_or_confidence(None, confidence),
                expected,
                "confidence {confidence}"
            );
        }
    }

    /// #161: entity names resolve from companies first, then persons, and an
    /// unresolvable id yields an empty string (never the title).
    #[test]
    fn entity_name_prefers_company_then_person_then_empty() {
        let company_id = Uuid::new_v4();
        let person_id = Uuid::new_v4();
        let unknown_id = Uuid::new_v4();
        let companies = HashMap::from([(company_id, "Acme Corp".to_string())]);
        let persons = HashMap::from([(person_id, "Jane Doe".to_string())]);

        assert_eq!(
            resolve_entity_name(&[unknown_id, company_id], &companies, &persons),
            "Acme Corp"
        );
        assert_eq!(
            resolve_entity_name(&[unknown_id, person_id], &companies, &persons),
            "Jane Doe"
        );
        assert_eq!(
            resolve_entity_name(&[person_id, company_id], &companies, &persons),
            "Acme Corp",
            "a company anywhere in entity_ids wins over a person"
        );
        assert_eq!(resolve_entity_name(&[unknown_id], &companies, &persons), "");
        assert_eq!(resolve_entity_name(&[], &companies, &persons), "");
    }
}
