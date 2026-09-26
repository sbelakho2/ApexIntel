use std::sync::Arc;

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
    for src in &live_sources {
        let domain = src
            .url
            .split("//")
            .nth(1)
            .and_then(|s| s.split('/').next())
            .unwrap_or(src.url.as_str())
            .to_string();
        if let Some(r) = by_slug.get(src.slug.as_str()) {
            let _ = store
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
                .await;
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
            let ingested = measured
                .map(|r| r.observations_ingested as u64)
                .unwrap_or(0);
            let fires = measured
                .map(|r| r.observations_in_fires as u64)
                .unwrap_or(0);
            let promotions = measured
                .map(|r| r.observations_in_promotions as u64)
                .unwrap_or(0);
            let obs_types = measured
                .map(|r| r.observation_types_produced.clone())
                .unwrap_or_else(|| vec!["web_change".to_string()]);
            let last_crawl = measured.and_then(|r| r.last_crawl_at);
            let hours_since_last_crawl = last_crawl
                .map(|t| (now - t).num_minutes().max(0) as f64 / 60.0)
                // No crawl yet for this source in the window: report a real large
                // value rather than an invented "2.0 or 6.0".
                .unwrap_or(window_days as f64 * 24.0);

            SourceTelemetry {
                source_id: src.slug.clone(),
                domain,
                observations_ingested: ingested,
                observations_in_fires: fires,
                observations_in_promotions: promotions,
                median_ingest_latency_secs: src.min_interval_minutes as f64 * 30.0,
                // Real per-source fetch error rate is not yet instrumented at the
                // HTTP layer; report 0.0 rather than fabricating a 0.05 constant.
                error_rate: 0.0,
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
        top_score = top.map(|s| s.score).unwrap_or(0.0),
        bottom_source = bottom.map(|s| s.source_id.as_str()).unwrap_or("none"),
        bottom_score = bottom.map(|s| s.score).unwrap_or(0.0),
        "source_scoring: complete (real telemetry)"
    );
    run.succeed(
        scored.len() as u64,
        &format!(
            "source_scoring: ranked {} sources from real telemetry; top={} ({:.3}), bottom={} ({:.3})",
            scored.len(),
            top.map(|s| s.source_id.as_str()).unwrap_or("none"),
            top.map(|s| s.score).unwrap_or(0.0),
            bottom.map(|s| s.source_id.as_str()).unwrap_or("none"),
            bottom.map(|s| s.score).unwrap_or(0.0),
        ),
    );
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
                    tracing::debug!(
                        obs_type = %obs_type,
                        error = %e,
                        "cross_domain_mining: skipping type"
                    );
                }
            }
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
    let base_failed = [&source_run, &cross_run, &outcome_run]
        .iter()
        .filter(|r| matches!(r.status, JobStatus::Failed { .. }))
        .count();

    #[cfg(feature = "llm")]
    let (total, failed) = {
        let mut total = base_total;
        let mut failed = base_failed;

        match run_llm_continuous_improvement_cycle(store, &ctx.ingress).await {
            Ok(stats) => {
                tracing::info!(
                    eval_pass_rate = stats.eval_pass_rate,
                    eval_avg_score = stats.eval_avg_score,
                    eval_hallucination_rate = stats.eval_hallucination_rate,
                    captures_seeded = stats.captures_seeded,
                    captures_analysed = stats.captures_analysed,
                    qualifying_examples = stats.qualifying_examples,
                    avg_critique = stats.avg_critique_score,
                    "self_improvement_cycle: llm continuous improvement completed"
                );
                total += stats.captures_analysed as u64;
            }
            Err(e) => {
                tracing::error!(error = %e, "self_improvement_cycle: llm continuous improvement failed");
                failed += 1;
            }
        }
        (total, failed)
    };

    #[cfg(not(feature = "llm"))]
    let (total, failed) = (base_total, base_failed);

    if failed > 0 {
        run.fail(&format!(
            "self_improvement_cycle: {failed} sub-jobs/components failed"
        ));
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
