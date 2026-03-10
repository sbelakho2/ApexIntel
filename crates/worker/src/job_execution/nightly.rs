use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use apex_crawl::client::{CrawlClient, CrawlClientConfig, CrawlRequest};
use apex_crawl::sources::select_sources_for_crawl;

use crate::*;

const NIGHTLY_STAGE_TIMEOUT: Duration = Duration::from_secs(30);
const NIGHTLY_STAGE_ATTEMPTS: usize = 3;

pub(super) async fn run_crawl_cycle(store: &Arc<PgStore>) -> JobRun {
    let mut run = JobRun::new(JobKind::CrawlCycle);
    run.start();
    let sources = all_sources();
    let crawl_limit: usize = std::env::var("CRAWL_MAX_SOURCES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20);

    let always_include_slugs = ["globes_il_tech"];
    let fetch_sources = select_sources_for_crawl(&sources, 2, crawl_limit, &always_include_slugs);

    if fetch_sources.is_empty() {
        run.skip("crawl_cycle: no enabled tier-1/2 sources configured");
        return run;
    }

    tracing::info!(
        selected = fetch_sources.len(),
        limit = crawl_limit,
        forced_sources = always_include_slugs.join(","),
        "crawl_cycle: source selection complete"
    );

    let proxy_rotator =
        build_proxy_rotator_from_env().map(|rotator| Arc::new(tokio::sync::Mutex::new(rotator)));
    if let Some(rotator) = proxy_rotator.as_ref() {
        let proxy_health = rotator.lock().await.health_summary();
        tracing::info!(proxy_health = %proxy_health, "crawl_cycle: proxy rotation enabled");
    }

    let crawl_client = match CrawlClient::new(CrawlClientConfig {
        proxy_rotator: proxy_rotator.clone(),
        ..CrawlClientConfig::default()
    }) {
        Ok(client) => client,
        Err(error) => {
            run.fail(&format!(
                "crawl_cycle: failed to build crawl client: {error}"
            ));
            return run;
        }
    };

    let mut ingested: u64 = 0;
    let mut errors: u64 = 0;
    let mut successful_sources: HashSet<String> = HashSet::new();
    let mut failed_sources: HashSet<String> = HashSet::new();

    for src in &fetch_sources {
        let url = src.rss_url.as_deref().unwrap_or(src.url.as_str());
        let prefers_browser_ua = src.slug == "globes_il_tech";
        let request = CrawlRequest::new(url)
            .source_id(&src.slug)
            .requires_proxy(src.needs_proxy)
            .prefer_browser_user_agent(prefers_browser_ua)
            .override_user_agent(if prefers_browser_ua {
                "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36"
            } else {
                "ApexIntelBot/1.0 (+https://apex-intel.io/bot)"
            });

        match crawl_client.fetch_text(&request).await {
            Ok(response) => {
                let body = response.body;
                #[cfg(any(feature = "parse", feature = "llm"))]
                let obs_value = match extract_page(&body) {
                    Ok(page) => serde_json::json!({
                        "source_id": src.slug,
                        "url": url,
                        "title": page.title,
                        "description": page.description,
                        "body_excerpt": page.body_text.chars().take(1000).collect::<String>(),
                        "language": page.language,
                    }),
                    Err(_) => serde_json::json!({
                        "source_id": src.slug,
                        "url": url,
                    }),
                };
                #[cfg(not(any(feature = "parse", feature = "llm")))]
                let obs_value = serde_json::json!({
                    "source_id": src.slug,
                    "url": url,
                    "body_len": body.len(),
                });

                let obs = Observation::new(
                    ObservationType::WebChange,
                    Utc::now(),
                    obs_value,
                    serde_json::json!({
                        "source": src.slug,
                        "tier": src.tier,
                        "category": format!("{:?}", src.category),
                    }),
                );
                match store.insert_observation(&obs).await {
                    Ok(_) => {
                        ingested += 1;
                        successful_sources.insert(src.slug.clone());
                    }
                    Err(e) => {
                        tracing::warn!(
                            source = %src.slug,
                            error = %e,
                            "crawl_cycle: failed to store observation"
                        );
                        errors += 1;
                        failed_sources.insert(src.slug.clone());
                    }
                }
            }
            Err(error) => {
                tracing::warn!(
                    source = %src.slug,
                    category = %error.category().as_str(),
                    error = %error,
                    "crawl_cycle: fetch error"
                );
                errors += 1;
                failed_sources.insert(src.slug.clone());
            }
        }
    }

    let attempted_sources = fetch_sources.len().max(1);
    let success_ratio = ingested as f64 / attempted_sources as f64;
    let min_success_ratio = std::env::var("CRAWL_MIN_SUCCESS_RATIO")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .map(|v| v.clamp(0.0, 1.0))
        .unwrap_or(0.30);

    let failed_sources_list = {
        let mut v: Vec<_> = failed_sources.iter().cloned().collect();
        v.sort();
        v
    };
    let successful_sources_list = {
        let mut v: Vec<_> = successful_sources.iter().cloned().collect();
        v.sort();
        v
    };

    if ingested == 0 || success_ratio < min_success_ratio {
        let failure_summary = format!(
            "crawl_cycle degraded: ingested={} attempted_sources={} success_ratio={:.2} min_success_ratio={:.2} failed_sources={} successful_sources={}",
            ingested,
            fetch_sources.len(),
            success_ratio,
            min_success_ratio,
            if failed_sources_list.is_empty() { "none".to_string() } else { failed_sources_list.join(",") },
            if successful_sources_list.is_empty() { "none".to_string() } else { successful_sources_list.join(",") },
        );

        let _ = store
            .insert_warning(
                "crawl_health",
                "Crawl reliability degraded",
                Some(&failure_summary),
                "high",
                None,
                Some("crawl_cycle"),
                None,
                None,
                Some((1.0 - success_ratio).clamp(0.0, 1.0)),
            )
            .await;

        run.fail(&format!(
            "crawl_cycle: {}/{} sources attempted; {} observations ingested, {} errors; failed_sources=[{}]",
            fetch_sources.len(),
            sources.iter().filter(|s| s.enabled).count(),
            ingested,
            errors,
            if failed_sources_list.is_empty() { "none".to_string() } else { failed_sources_list.join(",") },
        ));
        return run;
    }

    run.succeed(
        ingested,
        &format!(
            "crawl_cycle: {}/{} sources attempted; {} observations ingested, {} errors; success_ratio={:.2}",
            fetch_sources.len(),
            sources.iter().filter(|s| s.enabled).count(),
            ingested,
            errors,
            success_ratio,
        ),
    );
    run
}

pub(super) async fn run_pattern_mining(kind: &JobKind, store: &Arc<PgStore>) -> JobRun {
    let mut run = JobRun::new(kind.clone());
    run.start();
    let since = Utc::now() - chrono::Duration::hours(24);
    let mining_stats = match super::resilience::run_stage_with_retry(
        "pattern_mining.load_stats",
        NIGHTLY_STAGE_TIMEOUT,
        NIGHTLY_STAGE_ATTEMPTS,
        |_| async { store.get_mining_stats(since).await },
    )
    .await
    {
        Ok(s) => s,
        Err(e) => {
            run.fail(&format!(
                "pattern_mining: failed to load mining stats from DB: {e}"
            ));
            return run;
        }
    };
    let stage = process_mining_stage(&MiningStageResult {
        candidates_found: mining_stats.candidates_found,
        candidates_passed_gates: mining_stats.candidates_passed_gates,
        hypotheses_generated: mining_stats.hypotheses_generated,
        recipes_staged: mining_stats.recipes_staged,
        errors: mining_stats.errors,
    });
    match stage.run.status {
        apex_worker::scheduler::JobStatus::Succeeded { .. } => {
            run.succeed(
                stage.items,
                &format!("mining completed: {}", stage.run.notes),
            );
        }
        apex_worker::scheduler::JobStatus::Failed { .. } => {
            run.fail(&format!("mining failed: {}", stage.run.notes));
        }
        _ => {
            run.skip(&format!("mining stage not terminal: {}", stage.run.notes));
        }
    }
    run
}

pub(super) async fn run_hypothesis_generation(kind: &JobKind, store: &Arc<PgStore>) -> JobRun {
    let mut run = JobRun::new(kind.clone());
    run.start();
    #[cfg(feature = "llm")]
    {
        let since = Utc::now() - chrono::Duration::hours(24);
        let mining_stats = match super::resilience::run_stage_with_retry(
            "hypothesis_generation.load_stats",
            NIGHTLY_STAGE_TIMEOUT,
            NIGHTLY_STAGE_ATTEMPTS,
            |_| async { store.get_mining_stats(since).await },
        )
        .await
        {
            Ok(s) => s,
            Err(e) => {
                run.fail(&format!(
                    "hypothesis_generation: failed to load mining stats from DB: {e}"
                ));
                return run;
            }
        };
        let stage = process_hypothesis_generation_stage(&HypothesisGenerationStageResult {
            candidates_submitted: mining_stats.candidates_passed_gates,
            hypotheses_generated: mining_stats.hypotheses_generated,
            hypotheses_failed: mining_stats
                .candidates_passed_gates
                .saturating_sub(mining_stats.hypotheses_generated),
            recipes_staged: mining_stats.recipes_staged,
            errors: mining_stats.errors,
        });
        match stage.run.status {
            apex_worker::scheduler::JobStatus::Succeeded { .. } => {
                run.succeed(
                    stage.items,
                    &format!("hypothesis gen completed: {}", stage.run.notes),
                );
            }
            apex_worker::scheduler::JobStatus::Failed { .. } => {
                run.fail(&format!("hypothesis gen failed: {}", stage.run.notes));
            }
            _ => {
                run.skip(&format!("hypothesis gen not terminal: {}", stage.run.notes));
            }
        }
    }
    #[cfg(not(feature = "llm"))]
    {
        let _ = store;
        run.skip("hypothesis generation requires the `llm` feature");
    }
    run
}

pub(super) async fn run_feature_drift_check(kind: &JobKind, store: &Arc<PgStore>) -> JobRun {
    let mut run = JobRun::new(kind.clone());
    run.start();
    let drift_stats = match super::resilience::run_stage_with_retry(
        "feature_drift_check.load_stats",
        NIGHTLY_STAGE_TIMEOUT,
        NIGHTLY_STAGE_ATTEMPTS,
        |_| async { store.get_drift_stats().await },
    )
    .await
    {
        Ok(s) => s,
        Err(e) => {
            run.fail(&format!(
                "feature_drift_check: failed to load drift stats from DB: {e}"
            ));
            return run;
        }
    };
    let stage = process_drift_stage(&DriftCheckStageResult {
        features_checked: drift_stats.features_checked,
        features_drifted: drift_stats.features_drifted,
        drift_scores: drift_stats.drift_scores,
        alerts_raised: drift_stats.alerts_raised,
        errors: drift_stats.errors,
    });
    match stage.run.status {
        apex_worker::scheduler::JobStatus::Succeeded { .. } => {
            run.succeed(
                stage.items,
                &format!("drift check completed: {}", stage.run.notes),
            );
        }
        apex_worker::scheduler::JobStatus::Failed { .. } => {
            run.fail(&format!("drift check failed: {}", stage.run.notes));
        }
        _ => {
            run.skip(&format!("drift stage not terminal: {}", stage.run.notes));
        }
    }
    run
}
