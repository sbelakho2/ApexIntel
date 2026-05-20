use std::sync::Arc;
use std::time::Duration;

use crate::*;

const WEEKLY_STAGE_TIMEOUT: Duration = Duration::from_secs(45);
const WEEKLY_STAGE_ATTEMPTS: usize = 3;

#[allow(clippy::disallowed_methods)]
pub(super) async fn run_weekly_recipe_job(kind: &JobKind, store: &Arc<PgStore>) -> JobRun {
    let mut run = JobRun::new(kind.clone());
    run.start();
    let ctx = StorageContext {
        store: PgStore::from_pool(store.pool.clone()),
        run_timestamp: Utc::now(),
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
    // Stage: auto-calibrate recipe precision thresholds based on FP rates
    if let Err(e) = super::resilience::run_stage_with_retry(
        "weekly_pipeline.auto_calibrate_thresholds",
        WEEKLY_STAGE_TIMEOUT,
        WEEKLY_STAGE_ATTEMPTS,
        |_| async {
            let adjustments = ctx
                .store
                .auto_calibrate_recipe_thresholds()
                .await?;
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
    if report.overall_success {
        let items = match kind {
            JobKind::PromotionBoard => report
                .promotion_result
                .as_ref()
                .map(|r| r.promoted.len() as u64)
                .unwrap_or(0),
            JobKind::RecipeDeprecation => report
                .deprecation_result
                .as_ref()
                .map(|r| r.deprecated.len() as u64)
                .unwrap_or(0),
            _ => 0,
        };
        run.succeed(items, &report.summary());
    } else {
        run.fail(&report.summary());
    }
    run
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
                    entity_name: row.title.clone(),
                    severity: "medium".to_string(),
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
                                "{} — {} (posterior {:.0}%, support {:.2}, contradiction {:.2})",
                                hypothesis.hypothesis,
                                hypothesis.assessment,
                                hypothesis.posterior * 100.0,
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
                let key_metrics_json = serde_json::json!({
                    "warnings_total": memo.warning_count,
                    "warnings_critical": memo.critical_count,
                    "insights_generated": memo.total_insights,
                    "companies_monitored": 0,
                    "pois_tracked": 0,
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
                    tracing::warn!(error = %e, "strategy_memo: failed to persist memo to DB");
                } else {
                    tracing::info!(
                        week = memo.week_number,
                        year = memo.year,
                        sections = section_count,
                        "strategy_memo: memo persisted to weekly_memos"
                    );
                }
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
        let _ = store;
        match load_weekly_inputs().await {
            Ok(inputs) => {
                let report = run_weekly_pipeline(
                    &inputs.staged_recipes,
                    &inputs.production_recipes,
                    &inputs.memo_inputs,
                    &inputs.promotion_policy.unwrap_or_default(),
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
