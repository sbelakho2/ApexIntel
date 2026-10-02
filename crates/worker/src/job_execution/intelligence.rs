use std::sync::Arc;

use apex_core::measurement::Measurement;
use apex_crawl::source_scoring::{score_and_rank, ScoringConfig, SourceTelemetry};
use apex_crawl::sources::filter_by_tier;
use apex_store::postgres::{NewCrawlMetric, PgStore, RealSourceTelemetry};

use crate::*;

pub(super) async fn run_source_scoring(kind: &JobKind, store: &Arc<PgStore>) -> JobRun {
    let mut run = JobRun::new(kind.clone());
    run.start();

    let sources = all_sources();
    let live_sources = filter_by_tier(&sources, 2);

    if live_sources.is_empty() {
        run.skip("source_scoring: no enabled tier-1/2 sources");
        return run;
    }

    // ── REAL telemetry: aggregate actual per-source yield/freshness/error from
    //    observations + fired warnings, replacing the previously fabricated
    //    constants (error_rate: 0.05, * 0.3 / * 0.1, fake hours_since_last_crawl).
    let window_days = 14;
    let real = match store.compute_source_telemetry(window_days).await {
        Ok(rows) => rows,
        Err(e) => {
            run.fail(&format!(
                "source_scoring: failed to compute source telemetry: {e}"
            ));
            return run;
        }
    };

    // Index real measurements by source slug for O(1) lookup.
    let mut by_slug: std::collections::HashMap<&str, &RealSourceTelemetry> =
        std::collections::HashMap::new();
    for r in &real {
        by_slug.insert(r.source_id.as_str(), r);
    }

    let now = Utc::now();
    let window_start = now - chrono::Duration::days(window_days);

    // Persist real telemetry snapshots (async — done before the sync builder).
    let mut crawl_metric_write_failures: u64 = 0;
    for src in &live_sources {
        let domain = src
            .url
            .split("//")
            .nth(1)
            .and_then(|s| s.split('/').next())
            .unwrap_or(src.url.as_str())
            .to_string();
        if let Some(r) = by_slug.get(src.slug.as_str()) {
            if let Err(error) = store
                .upsert_crawl_metric(&NewCrawlMetric {
                    source_id: r.source_id.clone(),
                    domain: Some(domain),
                    window_start,
                    window_end: now,
                    observations_ingested: r.observations_ingested,
                    observations_in_fires: r.observations_in_fires,
                    observations_in_promotions: r.observations_in_promotions,
                    fetch_attempts: r.fetch_attempts,
                    fetch_errors: r.fetch_errors,
                    median_ingest_latency_secs: r.median_ingest_latency_secs,
                    last_crawl_at: r.last_crawl_at,
                    observation_types_produced: r.observation_types_produced.clone(),
                    metadata: serde_json::json!({}),
                })
                .await
            {
                // Authoritative persistence for source reliability metrics.
                crawl_metric_write_failures += 1;
                tracing::warn!(
                    source = %src.slug,
                    %error,
                    "source_scoring: failed to persist crawl metric snapshot"
                );
            }
        }
    }

    // Build telemetry from measured data (sync). Sources with no observations
    // in the window honestly report zero counts — no fabricated numbers.
    let telemetry: Vec<SourceTelemetry> = live_sources
        .iter()
        .map(|src| {
            let domain = src
                .url
                .split("//")
                .nth(1)
                .and_then(|s| s.split('/').next())
                .unwrap_or(src.url.as_str())
                .to_string();
            let measured = by_slug.get(src.slug.as_str()).copied();
            let ingested = measured.map_or(Measurement::NotMeasured, |r| {
                Measurement::measured(r.observations_ingested as u64)
            });
            let fires = measured.map_or(Measurement::NotMeasured, |r| {
                Measurement::measured(r.observations_in_fires as u64)
            });
            // Real promotion events per source are not recorded; the column is
            // NULL and the scorer sees NotMeasured rather than a proxy.
            let promotions = measured
                .and_then(|r| r.observations_in_promotions)
                .map_or(Measurement::NotMeasured, |value| {
                    Measurement::measured(value.max(0) as u64)
                });
            let obs_types = measured.map_or(Measurement::NotMeasured, |r| {
                Measurement::measured(r.observation_types_produced.clone())
            });
            let last_crawl = measured.and_then(|r| r.last_crawl_at);
            let hours_since_last_crawl = match last_crawl {
                Some(t) => Measurement::measured((now - t).num_minutes().max(0) as f64 / 60.0),
                // No successful crawl for this source in the window is an
                // unmeasured freshness, never an invented "very stale" value.
                None => Measurement::not_measured(),
            };

            // Measured fields come straight from the window's metrics; the
            // configured interval is configuration, not a measurement.
            // `median_ingest_latency_secs` and the fetch error rate are not
            // instrumented at the HTTP layer yet — they are `NotMeasured`, never
            // 0 or a transformed scheduling constant.
            SourceTelemetry {
                source_id: src.slug.clone(),
                domain,
                observations_ingested: ingested,
                observations_in_fires: fires,
                observations_in_promotions: promotions,
                median_ingest_latency_secs: measured
                    .and_then(|r| r.median_ingest_latency_secs)
                    .map_or(Measurement::NotMeasured, Measurement::measured),
                error_rate: Measurement::not_measured(),
                observation_types_produced: obs_types,
                hours_since_last_crawl,
                crawl_interval_hours: src.min_interval_minutes as f64 / 60.0,
            }
        })
        .collect();

    let config = ScoringConfig::default();
    let scored = score_and_rank(&telemetry, &config);
    let top = scored.first();
    let bottom = scored.last();
    tracing::info!(
        sources = scored.len(),
        top_source = top.map(|s| s.source_id.as_str()).unwrap_or("none"),
        top_score = top
            .and_then(|s| s.score.value_copied())
            .map_or("not measured".to_string(), |score| format!("{score:.3}")),
        bottom_source = bottom.map(|s| s.source_id.as_str()).unwrap_or("none"),
        bottom_score = bottom
            .and_then(|s| s.score.value_copied())
            .map_or("not measured".to_string(), |score| format!("{score:.3}")),
        "source_scoring: complete (observation-derived source telemetry)"
    );
    let score_text = |source: Option<&apex_crawl::source_scoring::ScoredSource>| {
        source
            .map(|s| {
                format!(
                    "{} ({})",
                    s.source_id,
                    s.score
                        .value_copied()
                        .map_or("not measured".to_string(), |score| format!("{score:.3}"))
                )
            })
            .unwrap_or_else(|| "none".to_string())
    };
    let summary = format!(
        "source_scoring: ranked {} sources from observation-derived telemetry; top={}, bottom={}; crawl_metric_write_failures={}",
        scored.len(),
        score_text(top),
        score_text(bottom),
        crawl_metric_write_failures,
    );
    if crawl_metric_write_failures > 0 {
        // Scoring completed, but the persisted telemetry snapshots are
        // incomplete: report degraded instead of a clean success.
        run.fail(&summary);
    } else {
        run.succeed(scored.len() as u64, &summary);
    }
    run
}

pub(super) async fn run_cross_domain_mining(kind: &JobKind, store: &Arc<PgStore>) -> JobRun {
    let mut run = JobRun::new(kind.clone());
    run.start();
    #[cfg(feature = "llm")]
    {
        let since = Utc::now() - chrono::Duration::days(30);
        let obs_types = [
            "WebChange",
            "JobPost",
            "SocialPost",
            "CompetitorEvent",
            "lookalike_domain",
            "dns_posture",
            "kev_match",
        ];
        let mut all_events: Vec<TypedEvent> = Vec::new();
        // A failed observation read must not shrink the event set silently and
        // then be reported as "insufficient observations" (a clean skip).
        let mut inputs_failed: usize = 0;
        for obs_type in &obs_types {
            match store.get_observations_by_type(obs_type, since, 500).await {
                Ok(rows) => {
                    for row in rows {
                        all_events.push(TypedEvent {
                            entity_id: row
                                .entity_id
                                .map(|id| id.to_string())
                                .unwrap_or_else(|| row.id.to_string()),
                            obs_type: row.observation_type.clone(),
                            ts_epoch: row.ts_utc.timestamp(),
                        });
                    }
                }
                Err(e) => {
                    inputs_failed += 1;
                    tracing::warn!(
                        obs_type = %obs_type,
                        error = %e,
                        "cross_domain_mining: observation input failed to load"
                    );
                }
            }
        }

        if inputs_failed > 0 {
            run.fail(&format!(
                "cross_domain_mining: {inputs_failed} of {} observation inputs failed to load; \
                 refusing to evaluate on a partial event set",
                obs_types.len()
            ));
            return run;
        }

        if all_events.len() < 10 {
            run.skip(&format!(
                "cross_domain_mining: insufficient observations ({} < 10); re-run after more crawl data accumulates",
                all_events.len()
            ));
            return run;
        }

        let recipe_stats = match store.get_recipe_stats().await {
            Ok(s) => s,
            Err(e) => {
                run.fail(&format!(
                    "cross_domain_mining: failed to load recipe stats: {e}"
                ));
                return run;
            }
        };
        let outcomes: Vec<(String, i64)> = recipe_stats
            .iter()
            .filter(|r| r.fired_count > 0)
            .map(|r| {
                let ts = r.last_fired.map(|t| t.timestamp()).unwrap_or(0);
                (r.recipe_code.clone(), ts)
            })
            .collect();

        let config = CrossDomainConfig::default();
        let combinations =
            mine_signal_combinations(&all_events, &outcomes, "recipe_fire", 7, 30, &config);

        for combo in combinations.iter().take(5) {
            tracing::info!(
                type_a = %combo.type_a,
                type_b = %combo.type_b,
                synergy = combo.synergy_factor,
                stability = combo.stability,
                "cross_domain_mining: synergistic combination"
            );
        }
        run.succeed(
            combinations.len() as u64,
            &format!(
                "cross_domain_mining: {} events → {} synergistic combinations",
                all_events.len(),
                combinations.len()
            ),
        );
    }
    #[cfg(not(feature = "llm"))]
    {
        let _ = store;
        run.skip("cross_domain_mining: requires the `llm` feature (apex-learning/experimental)");
    }
    run
}

pub(super) async fn run_outcome_tracking(kind: &JobKind, store: &Arc<PgStore>) -> JobRun {
    let mut run = JobRun::new(kind.clone());
    run.start();
    #[cfg(feature = "llm")]
    {
        let recipe_stats = match store.get_recipe_stats().await {
            Ok(s) => s,
            Err(e) => {
                run.fail(&format!(
                    "outcome_tracking: failed to load recipe stats: {e}"
                ));
                return run;
            }
        };

        if recipe_stats.is_empty() {
            run.skip("outcome_tracking: no recipe stats available yet");
            return run;
        }

        let source_yields: Vec<SourceYield> = recipe_stats
            .iter()
            .map(|r| SourceYield {
                source_id: r.recipe_code.clone(),
                domain: r.recipe_code.clone(),
                total_observations: r.fired_count as u64,
                observations_in_fired_recipes: r.fired_count as u64,
                observations_in_promoted_recipes: r.active_count as u64,
                observations_in_retired_recipes: 0,
                freshness_hours: r
                    .last_fired
                    .map(|t| (Utc::now() - t).num_hours() as f64)
                    .unwrap_or(720.0),
                diversity_score: 0.5,
            })
            .collect();

        let weights = SourceScoringWeights::default();
        let source_scores = score_sources(&source_yields, &weights);

        let obs_type_names = [
            "web_change",
            "job_post",
            "tender_posted",
            "vuln_notice",
            "person_mention",
            "role_change",
            "procurement_signal",
        ];
        let obs_type_stats: Vec<ObsTypeStats> = obs_type_names
            .iter()
            .map(|&obs_type| {
                let total = recipe_stats
                    .iter()
                    .filter(|r| r.recipe_code.contains(obs_type))
                    .map(|r| r.fired_count as u64)
                    .sum::<u64>();
                let promoted = recipe_stats
                    .iter()
                    .filter(|r| r.recipe_code.contains(obs_type) && r.active_count > 0)
                    .map(|r| r.active_count as u64)
                    .sum::<u64>();
                ObsTypeStats {
                    obs_type: obs_type.to_string(),
                    total_occurrences: total,
                    in_promoted_recipes: promoted,
                    in_staged_recipes: 0,
                    in_retired_recipes: 0,
                }
            })
            .collect();

        let ranked_types = rank_observation_types(&obs_type_stats);

        if let Some(top_source) = source_scores.first() {
            tracing::info!(
                source = %top_source.source_id,
                composite = top_source.composite,
                "outcome_tracking: top source"
            );
        }
        if let Some(top_type) = ranked_types.first() {
            tracing::info!(
                obs_type = %top_type.obs_type,
                net_value = top_type.net_value,
                "outcome_tracking: highest-value observation type"
            );
        }

        run.succeed(
            source_scores.len() as u64,
            &format!(
                "outcome_tracking: {} sources scored, {} obs types ranked; top_type={} (net={:.3})",
                source_scores.len(),
                ranked_types.len(),
                ranked_types
                    .first()
                    .map(|t| t.obs_type.as_str())
                    .unwrap_or("none"),
                ranked_types.first().map(|t| t.net_value).unwrap_or(0.0),
            ),
        );
    }
    #[cfg(not(feature = "llm"))]
    {
        let _ = store;
        run.skip("outcome_tracking: requires the `llm` feature (apex-learning/experimental)");
    }
    run
}

pub(super) async fn run_self_improvement_cycle(
    kind: &JobKind,
    store: &Arc<PgStore>,
    ctx: &super::JobExecutionContext,
) -> JobRun {
    let mut run = JobRun::new(kind.clone());
    run.start();
    tracing::info!("self_improvement_cycle: starting coordinated improvement loop");
    let source_run = Box::pin(super::execute_job(&JobKind::SourceScoring, store, ctx)).await;
    let cross_run = Box::pin(super::execute_job(&JobKind::CrossDomainMining, store, ctx)).await;
    let outcome_run = Box::pin(super::execute_job(&JobKind::OutcomeTracking, store, ctx)).await;

    let base_total =
        source_run.items_processed + cross_run.items_processed + outcome_run.items_processed;
    // Degraded stages count as failures here: a stage that ran with failed
    // inputs or explicitly missing features must not roll up as success.
    let base_failed = [&source_run, &cross_run, &outcome_run]
        .iter()
        .filter(|r| {
            matches!(
                r.status,
                JobStatus::Failed { .. } | JobStatus::Degraded { .. }
            )
        })
        .count();

    #[cfg(feature = "llm")]
    let (total, failed, partial_learning_failures) = {
        let mut total = base_total;
        let mut failed = base_failed;
        let mut partial_learning_failures = false;

        match run_llm_continuous_improvement_cycle(store, &ctx.ingress).await {
            Ok(outcome) => {
                let stage_failures = outcome.failures();
                if !stage_failures.is_empty() {
                    tracing::warn!(
                        stages = ?stage_failures
                            .iter()
                            .map(|failure| failure.stage.as_str())
                            .collect::<Vec<_>>(),
                        "self_improvement_cycle: llm stages reported failures"
                    );
                }
                tracing::info!(
                    eval_pass_rate = outcome.evaluation.value_ref().map(|summary| summary.pass_rate),
                    eval_avg_score = %outcome
                        .evaluation
                        .value_ref()
                        .map(|summary| summary.avg_judge_score.display_fixed(3))
                        .unwrap_or_else(|| "not measured".to_string()),
                    eval_hallucination_rate = outcome
                        .evaluation
                        .value_ref()
                        .map(|summary| summary.hallucination_rate),
                    eval_execution_errors = outcome
                        .evaluation
                        .value_ref()
                        .map(|summary| summary.execution_errors),
                    captures_seeded = outcome
                        .critique
                        .value_ref()
                        .map(|summary| summary.captures_seeded),
                    captures_analysed = outcome
                        .critique
                        .value_ref()
                        .map(|summary| summary.captures_analysed),
                    qualifying_examples = outcome
                        .critique
                        .value_ref()
                        .map(|summary| summary.qualifying_examples),
                    avg_critique = %outcome
                        .critique
                        .value_ref()
                        .map(|summary| summary.avg_critique_score.display_fixed(3))
                        .unwrap_or_else(|| "not measured".to_string()),
                    stage_failures = stage_failures.len(),
                    governance_persistence_failed = outcome.persistence_degraded(),
                    "self_improvement_cycle: llm continuous improvement completed"
                );
                total += outcome
                    .critique
                    .value_ref()
                    .map_or(0, |summary| summary.captures_analysed) as u64;
                // A terminally failed learning stage (evaluation, critique,
                // proposals) fails the cycle. A failed governance/dataset
                // persistence step, or a partial learning stage, degrades it:
                // the cycle produced values, but they are not fully recorded.
                if outcome.has_terminal_learning_failure() {
                    tracing::error!(
                        stages = ?stage_failures
                            .iter()
                            .map(|failure| failure.stage.as_str())
                            .collect::<Vec<_>>(),
                        "self_improvement_cycle: llm learning stages failed"
                    );
                    failed += 1;
                } else if !stage_failures.is_empty() || outcome.persistence_degraded() {
                    tracing::warn!(
                        stages = ?stage_failures
                            .iter()
                            .map(|failure| failure.stage.as_str())
                            .collect::<Vec<_>>(),
                        "self_improvement_cycle: llm cycle degraded (partial stages or persistence failures)"
                    );
                    partial_learning_failures = true;
                }
            }
            Err(e) => {
                tracing::error!(error = %e, "self_improvement_cycle: llm continuous improvement failed");
                failed += 1;
            }
        }
        (total, failed, partial_learning_failures)
    };

    #[cfg(not(feature = "llm"))]
    let (total, failed, partial_learning_failures) = (base_total, base_failed, false);

    if failed > 0 {
        run.fail(&format!(
            "self_improvement_cycle: {failed} sub-jobs/components failed"
        ));
    } else if partial_learning_failures {
        run.degrade(
            total,
            "self_improvement_cycle: quality loops completed with partial learning-stage failures",
        );
    } else {
        run.succeed(
            total,
            &format!(
                "self_improvement_cycle: all jobs and quality loops completed ({total} items)"
            ),
        );
    }
    run
}
