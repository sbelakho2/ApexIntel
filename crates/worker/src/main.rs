#![cfg_attr(test, allow(dead_code))]
#![allow(clippy::duplicated_attributes)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod config;
mod digest_filtering;
#[allow(dead_code)]
mod evidence_scoring;
#[allow(dead_code)]
mod fallback_generation;
#[cfg(feature = "llm")]
mod geo_targeting;
mod job_execution;
#[allow(dead_code)]
mod llm_orchestration;
mod observability;
#[allow(dead_code)]
mod quality_gates;
mod runtime;
#[allow(dead_code)]
mod title_formatting;

#[cfg(feature = "llm")]
pub(crate) use digest_filtering::is_promoted_business_insight_type;
pub(crate) use digest_filtering::passes_shared_insight_quality_gate;
#[cfg(all(feature = "llm", test))]
use digest_filtering::token_jaccard_similarity;
#[cfg(all(feature = "llm", test))]
use digest_filtering::{has_excessive_phrase_repetition, is_readable_and_useful_digest_text};
#[allow(unused_imports)]
pub(crate) use fallback_generation::{
    build_fallback_summary, should_emit_fallback_insight, FallbackSummaryRequest,
};
#[cfg(test)]
use fallback_generation::{count_concrete_signal_details, is_generic_action_hint};
#[allow(unused_imports)]
pub(crate) use title_formatting::build_analytical_title;

use anyhow::Result;
use apex_core::config::AppConfig;
use apex_core::entities::{Observation, ObservationType};
#[cfg(feature = "llm")]
use apex_core::entities::{Person, PriorityVector};
use apex_core::env::parse_truthy_flag;
use apex_core::schemas::Recipe;
#[cfg(any(feature = "llm", test))]
pub(crate) use apex_core::text::truncate_utf8 as truncate_text;
use apex_crawl::breach::BreachMonitor;
#[cfg(feature = "llm")]
use apex_crawl::person_scraper::PersonOsintScraper;
#[cfg(feature = "llm")]
use apex_crawl::poi_expansion::{PoiExpansionEngine, SeedPoi};
use apex_crawl::proxy::ProxyRotator;
use apex_crawl::sanctions::SanctionsList;
use apex_crawl::sanctions::SanctionsScreener;
use apex_crawl::source_scoring::{score_and_rank, ScoringConfig, SourceTelemetry};
use apex_crawl::sources::all_sources;
#[cfg(feature = "llm")]
use apex_crawl::tor_client::TorClient;
#[cfg(feature = "llm")]
use apex_insights::arbitrage::{default_profiles as arbitrage_default_profiles, ArbitrageDetector};
#[cfg(feature = "llm")]
use apex_insights::bias_mitigation::{
    generate_devils_advocate, DevilsAdvocateConfig, EvidenceItem as BiasEvidenceItem, Severity,
};
#[cfg(feature = "llm")]
use apex_insights::comparison::build_comparison_matrix;
#[cfg(feature = "llm")]
use apex_insights::hypothesis::{EntityHypothesisTracker, EvidenceType as HypothesisEvidenceType};
#[cfg(feature = "llm")]
use apex_insights::predictive::predefined_patterns;
#[cfg(feature = "llm")]
use apex_insights::renderer::{Citation, InsightCard};
#[cfg(feature = "llm")]
use apex_insights::weekly_pipeline::{WeeklyPipelineConfig, WeeklyPipelineRunner};
#[cfg(feature = "llm")]
use apex_learning::cross_domain::{mine_signal_combinations, CrossDomainConfig, TypedEvent};
#[cfg(feature = "llm")]
use apex_learning::feedback::{
    rank_observation_types, score_sources, ObsTypeStats, SourceScoringWeights, SourceYield,
};
#[cfg(feature = "llm")]
use apex_llm::{ModelConfig, OpenAiCompatibleClient};
#[cfg(any(feature = "parse", feature = "llm"))]
use apex_parse::html::extract_page;
#[cfg(feature = "llm")]
use apex_poi::model::{
    InfluenceProfile, PoiProfile, PriorityVector as PoiPriorityVector, PsychProfile,
};
use apex_recipes::engine::{FeatureMap, RecipeEngine};
#[cfg(test)]
use apex_store::postgres::{HistoricalQualityGateLabel, QualityGateGoldenSetExample};
use apex_store::postgres::{InsightListFilters, PersonListFilters, PersonOrderBy, PgStore};
use apex_worker::activity_logger::ActivityLogger;
use apex_worker::nightly::{
    process_drift_stage, process_mining_stage, CrawlStageResult, DriftCheckStageResult,
    MiningStageResult, PoiRefreshStageResult,
};
#[cfg(feature = "llm")]
use apex_worker::nightly::{process_hypothesis_generation_stage, HypothesisGenerationStageResult};
use apex_worker::notifications::{NotificationDispatcher, SlaEnforcer, SlaWarningRecord};
use apex_worker::recipe_loader::load_default_seed_recipes;
use apex_worker::scheduler::{
    default_scheduler, validate_custom_command, JobKind, JobRun, JobStatus, Scheduler,
};
use apex_worker::storage::{
    build_memo_inputs, load_production_recipes, load_staged_recipes, StorageContext,
};
use apex_worker::weekly::{
    run_weekly_pipeline, DeprecationPolicy, MemoInputs, ProductionRecipe, PromotionPolicy,
    StagedRecipe,
};
use chrono::Utc;
#[cfg(all(feature = "llm", test))]
use evidence_scoring::noisy_or;
#[cfg(feature = "llm")]
use evidence_scoring::{calculate_relevance, signal_diversity_multiplier};
#[cfg(feature = "llm")]
pub(crate) use quality_gates::normalize_gate_text;
#[cfg(all(feature = "llm", test))]
use quality_gates::{
    contains_causal_link, contains_security_hygiene_marker,
    contains_soft_certification_pressure_marker,
};
#[cfg(all(feature = "llm", test))]
use quality_gates::{
    has_formulaic_commercial_language, has_low_usefulness_public_sector_analysis,
    has_temporal_incoherence, has_unnamed_customer_targeting,
    has_unsupported_certification_commercialization, has_unsupported_certification_escalation,
    has_unsupported_named_target_provenance, has_unsupported_public_sector_commercialization,
    has_unsupported_security_escalation, recommendation_has_action_timing, weighted_phrase_score,
};
use serde::Deserialize;
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
#[cfg(feature = "llm")]
use std::sync::Mutex;
use tokio::sync::{Mutex as TokioMutex, Semaphore};
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

// ────────────────────────────────────────────────────────────────────────────
// File size guard (B293)
// ────────────────────────────────────────────────────────────────────────────

/// Maximum file size for input JSON files (B293).
///
/// If `load_nightly_inputs()` or `load_weekly_inputs()` encounters a file
/// larger than this limit, it aborts immediately **before** allocating any
/// heap memory to read the file.  This prevents a misconfigured or maliciously
/// crafted input file from causing the worker to exhaust available RAM.
///
/// Rationale for 16 MiB:
/// - Production nightly input JSON files are typically 10–50 KiB (compressed stage results).
/// - Weekly input JSON files are typically 100–500 KiB (recipe + memo metadata).
/// - 16 MiB is a 50× safety margin; exceeding it is almost certainly a bug (e.g.,
///   accidentally pointed at a database dump, a raw model checkpoint, a binary blob).
///
/// If legitimate production workloads require larger inputs, increase this value
/// after verifying the memory impact on the worker container.
pub const MAX_INPUT_FILE_BYTES: u64 = 16 * 1024 * 1024; // 16 MiB

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct NightlyInputs {
    crawl: CrawlStageResult,
    mining: MiningStageResult,
    poi: PoiRefreshStageResult,
    drift: DriftCheckStageResult,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct WeeklyInputs {
    staged_recipes: Vec<StagedRecipe>,
    production_recipes: Vec<ProductionRecipe>,
    memo_inputs: MemoInputs,
    promotion_policy: Option<PromotionPolicy>,
    deprecation_policy: Option<DeprecationPolicy>,
}

mod artifacts;
mod bootstrap;
mod continuous_improvement;
mod digest;
mod generation;
mod llm_runtime;
mod poi;
mod prompts;
mod runtime_validation;

// ────────────────────────────────────────────────────────────────────────────
// Root re-exports
//
// Job-execution submodules use `use crate::*;` and the unit tests use
// `use super::*;`; both rely on these names living at the crate root exactly
// as they did before the main.rs split (audit P0 #28).
// ────────────────────────────────────────────────────────────────────────────

#[cfg(feature = "llm")]
pub(crate) use apex_core::entities::{ArtifactType, PoiArtifact};
#[cfg(feature = "llm")]
pub(crate) use apex_crawl::poi_expansion::DiscoveredPoi;
#[cfg(feature = "llm")]
pub(crate) use apex_llm::inference::LlmClient as InferenceLlmClient;
#[cfg(feature = "llm")]
pub(crate) use apex_poi::model::RoleFamily;
#[cfg(not(feature = "llm"))]
pub(crate) use artifacts::extract_domain;
pub(crate) use artifacts::generate_typosquat_variants;
#[cfg(feature = "llm")]
pub(crate) use artifacts::{
    dark_web_to_raw_artifacts, extract_domain, is_high_quality_raw_artifact, raw_to_poi_artifact,
};
#[cfg(feature = "llm")]
pub(crate) use bootstrap::seed_recipe_to_engine_recipe;
#[cfg(feature = "llm")]
pub(crate) use chrono::DateTime;
pub(crate) use continuous_improvement::run_llm_continuous_improvement_cycle;
pub(crate) use digest::run_update_email_digest_job;
#[cfg(not(feature = "llm"))]
pub(crate) use generation::collect_entity_evidence_urls;
#[cfg(feature = "llm")]
pub(crate) use generation::{
    count_numbered_references, extract_facts_from_text, format_sources_footer,
    generate_llm_insight, ranked_source_urls,
};
#[cfg(all(feature = "llm", test))]
use llm_orchestration::{
    quality_gate_blocker, quality_gate_passes_ensemble, quality_gate_requirement,
};
pub(crate) use llm_runtime::build_quality_llm_client;
#[cfg(feature = "llm")]
pub(crate) use poi::{
    classify_role_family, discovery_method_priority, looks_like_buyer_candidate_role,
    looks_like_person_name, resolve_discovered_company_id, validate_person_via_llm,
};
#[cfg(all(feature = "llm", test))]
pub(crate) use poi::{
    company_name_matches_seed, extract_candidate_company_domain, sanitize_validated_org,
    sanitize_validated_role,
};
#[cfg(any(not(feature = "llm"), test))]
pub(crate) use prompts::build_analytical_narrative;
pub(crate) use prompts::{capitalize_first, is_low_quality_narrative, is_public_sector_entity};
#[cfg(not(feature = "llm"))]
pub(crate) use prompts::{clean_rendered_text, resolve_evidence_placeholders};
#[cfg(all(feature = "llm", test))]
pub(crate) use prompts::{
    inferred_supply_chain_role, violates_entity_topic_alignment,
    violates_supply_chain_role_guidance,
};
#[cfg(feature = "llm")]
pub(crate) use prompts::{
    public_sector_procurement_or_program_case, EntityContext, EvidenceSignal,
};
#[cfg(feature = "llm")]
pub(crate) use quality_gates::low_signal_certification_warning_case;
#[cfg(all(feature = "llm", test))]
pub(crate) use runtime_validation::evaluate_quality_gate_golden_set;

fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .map(|v| parse_truthy_flag(&v))
        .unwrap_or(false)
}

fn build_paid_proxy_url_from_env() -> Option<String> {
    let host = std::env::var("PROXY_HOST")
        .ok()
        .filter(|v| !v.trim().is_empty())?;
    let port = std::env::var("PROXY_PORT")
        .ok()
        .filter(|v| !v.trim().is_empty())?;
    let username = std::env::var("PROXY_USERNAME")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| {
            std::env::var("PROXY_USER")
                .ok()
                .filter(|v| !v.trim().is_empty())
        })?;
    let password = std::env::var("PROXY_PASSWORD")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| {
            std::env::var("PROXY_PASS")
                .ok()
                .filter(|v| !v.trim().is_empty())
        })?;
    Some(format!("http://{username}:{password}@{host}:{port}"))
}

fn build_proxy_rotator_from_env() -> Option<ProxyRotator> {
    if !env_flag("ENABLE_PROXY_ROTATION") {
        return None;
    }

    let paid_proxy = build_paid_proxy_url_from_env();
    if paid_proxy.is_none() {
        tracing::warn!(
            "proxy rotation enabled, but PROXY_HOST/PROXY_PORT and PROXY_USERNAME|PROXY_USER / PROXY_PASSWORD|PROXY_PASS are incomplete"
        );
    }

    let mut rotator = ProxyRotator::new(true, paid_proxy);

    if let Ok(proxy_list) = std::env::var("PROXY_LIST") {
        let parsed = ProxyRotator::parse_proxy_list(&proxy_list);
        if !parsed.is_empty() {
            rotator.add_proxies(parsed);
        }
    }

    Some(rotator)
}

#[tokio::main]
#[allow(clippy::unwrap_used, clippy::expect_used)]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    let log_level = std::env::var("WORKER_LOG_LEVEL").unwrap_or_else(|_| "info".to_string());
    let log_filter = std::env::var("RUST_LOG")
        .unwrap_or_else(|_| format!("{},apex_worker={}", log_level, log_level));
    tracing_subscriber::fmt()
        .with_target(false)
        .with_env_filter(EnvFilter::new(log_filter))
        .init();

    // B294: Validate config at startup — fail fast if env vars are misconfigured.
    // AppConfig::from_env() already checks for required fields (DATABASE_URL),
    // and validate() checks numeric ranges, URL formats, etc.
    let config = AppConfig::from_env()?;
    let validation_errors = config.validate();
    if !validation_errors.is_empty() {
        tracing::error!(
            "Configuration validation failed ({} errors):",
            validation_errors.len()
        );
        for err in &validation_errors {
            tracing::error!("  - {}", err);
        }
        anyhow::bail!(
            "Refusing to start worker with invalid config. Fix the errors above and restart."
        );
    }
    tracing::info!("config validated successfully");

    // Create database pool for recipe insertion. Every worker connection must
    // carry the same default `service` identity as the API pool: migrations
    // 057/058 FORCE RLS on user-private tables, and an unset
    // `app.current_user_role` matches neither the owner nor the service
    // policy, so worker reads would silently return zero rows and writes would
    // be rejected.
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(std::time::Duration::from_secs(5))
        .idle_timeout(std::time::Duration::from_secs(600))
        .max_lifetime(std::time::Duration::from_secs(1800))
        .after_connect(|conn, _meta| {
            Box::pin(async move {
                PgStore::assume_service_identity(&mut *conn).await?;
                Ok(())
            })
        })
        .connect(config.database_url_value())
        .await?;
    tracing::info!("connected to database");

    // Load seed recipes from config/recipes_seed.yaml and insert them into the
    // database. Recipe seeding is bootstrap work, not service wiring.
    bootstrap::seed_recipes_from_yaml(&pool).await;

    // Wrap pool in a shared PgStore so every job handler can query the DB
    // without creating its own connection pool.  Using from_pool() avoids
    // opening a second connection when the pool was already created above.
    let store = Arc::new(PgStore::from_pool(pool.clone()));
    tracing::info!("store initialized");

    // Migrations normally run at API startup, but the worker can start first
    // (independent systemd units) and job handlers now write to schema added
    // by late migrations (e.g. triage_queue merge fields, migration 053).
    // Best-effort: a failure (permissions, checksum mismatch with
    // APEX_SKIP_MIGRATIONS semantics) must not take the worker down.
    match store.run_migrations().await {
        Ok(()) => tracing::info!("worker startup: database migrations applied"),
        Err(e) => tracing::warn!(
            error = %e,
            "worker startup: database migrations failed; continuing (some features may be degraded)"
        ),
    }

    // ─── Liveness heartbeat (migration 049) ───────────────────────────────
    // Health checks read `service_heartbeats.last_seen_at` to distinguish a
    // live worker from one that silently stopped; write every ~30s so
    // staleness is a measurement, not an assumption. The first tick fires
    // immediately, recording a heartbeat at startup.
    {
        let heartbeat_store = Arc::clone(&store);
        tokio::spawn(async move {
            let instance_id = std::env::var("APEX_INSTANCE_ID")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| {
                    let host =
                        std::env::var("HOSTNAME").unwrap_or_else(|_| "localhost".to_string());
                    format!("{host}-{}", std::process::id())
                });
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(30));
            loop {
                ticker.tick().await;
                if let Err(error) = heartbeat_store
                    .record_service_heartbeat("worker", &instance_id, env!("CARGO_PKG_VERSION"))
                    .await
                {
                    tracing::warn!(error = %error, "failed to record worker heartbeat");
                }
            }
        });
        tracing::info!("worker heartbeat task started");
    }

    // Create the shared ActivityLogger for recording system events
    // to the activity_feed table across all pipeline stages.
    let _activity_logger = ActivityLogger::new(pool.clone());
    tracing::info!("activity_logger initialized");

    let mut scheduler_state = default_scheduler();
    match store.list_worker_job_states().await {
        Ok(states) => {
            runtime::restore_scheduler_state(&mut scheduler_state, &states);
            tracing::info!(states = states.len(), "restored persisted scheduler state");
        }
        Err(error) => {
            tracing::warn!(error = %error, "failed to restore persisted scheduler state");
        }
    }

    let scheduler = Arc::new(TokioMutex::new(scheduler_state));
    let tick_guard = Arc::new(TokioMutex::new(()));
    let trigger_guard = Arc::new(TokioMutex::new(()));
    let manual_trigger_concurrency = std::env::var("MANUAL_TRIGGER_CONCURRENCY")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(2);
    let manual_max_claims_per_poll = std::env::var("MANUAL_TRIGGER_MAX_CLAIMS_PER_POLL")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(manual_trigger_concurrency);
    let manual_trigger_timeout_secs = std::env::var("MANUAL_TRIGGER_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(2 * 60 * 60);
    let manual_trigger_semaphore = Arc::new(Semaphore::new(manual_trigger_concurrency));
    let jobs_count = scheduler.lock().await.jobs.len();
    tracing::info!(
        jobs = jobs_count,
        manual_trigger_concurrency,
        manual_max_claims_per_poll,
        manual_trigger_timeout_secs,
        "worker started"
    );

    let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
    let mut trigger_interval = tokio::time::interval(std::time::Duration::from_secs(30));
    // B324: SIGTERM is the signal Docker/K8s/systemd send on stop — the
    // previous loop only listened for SIGINT (ctrl_c), so containerized
    // workers were killed outright mid-job with no drain.
    let mut sigterm = {
        use tokio::signal::unix::{signal, SignalKind};
        signal(SignalKind::terminate()).expect("failed to install SIGTERM handler")
    };
    loop {
        tokio::select! {
            _ = interval.tick() => {
                let store = Arc::clone(&store);
                let scheduler = Arc::clone(&scheduler);
                let tick_guard = Arc::clone(&tick_guard);
                tokio::spawn(async move {
                    let Ok(_guard) = tick_guard.try_lock() else {
                        tracing::warn!("tick_scheduler: previous run still active; skipping tick");
                        return;
                    };

                    let mut scheduler = scheduler.lock().await;
                    // Job-level panics are contained inside tick_scheduler
                    // (each job runs in an observed spawn, B325), so the tick
                    // body itself only does bookkeeping.
                    runtime::tick_scheduler(&mut scheduler, &store).await;
                });
            }
            _ = trigger_interval.tick() => {
                let store = Arc::clone(&store);
                let trigger_guard = Arc::clone(&trigger_guard);
                let manual_trigger_semaphore = Arc::clone(&manual_trigger_semaphore);
                tokio::spawn(async move {
                    let Ok(_guard) = trigger_guard.try_lock() else {
                        tracing::debug!("poll_trigger_queue: previous poll still active; skipping tick");
                        return;
                    };

                    runtime::poll_trigger_queue(
                        &store,
                        &manual_trigger_semaphore,
                        manual_max_claims_per_poll,
                        manual_trigger_timeout_secs,
                    )
                    .await;
                });
            }
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("received SIGINT, exiting gracefully");
                break;
            }
            _ = sigterm.recv() => {
                tracing::info!("received SIGTERM, exiting gracefully");
                break;
            }
        }
    }
    pool.close().await;
    tracing::info!("database pool closed");
    Ok(())
}

/// Read a local file to string with a file-size guard (B293).
///
/// **Never use `tokio::fs::read_to_string` directly on user-controlled or
/// environment-controlled paths.**  This function checks the file size before
/// attempting to read, preventing unbounded memory allocation.
///
/// # Errors
/// - Returns `Err` if the file does not exist, is not readable, or exceeds
///   `MAX_INPUT_FILE_BYTES`.
/// - Error messages are safe for logging (do not expose file content).
///
/// # Example
/// ```ignore
/// let content = read_file_with_size_check("runtime/job_input.json").await?;
/// let data: MyStruct = serde_json::from_str(&content)?;
/// ```
async fn read_file_with_size_check(path: &str) -> Result<String> {
    // Fetch file metadata before attempting to read
    let metadata = tokio::fs::metadata(path)
        .await
        .map_err(|err| anyhow::anyhow!("failed to stat file '{}': {}", path, err))?;

    let file_size = metadata.len();
    if file_size > MAX_INPUT_FILE_BYTES {
        anyhow::bail!(
            "file '{}' is {} bytes, which exceeds MAX_INPUT_FILE_BYTES ({}). \
             Refusing to read to prevent memory exhaustion.",
            path,
            file_size,
            MAX_INPUT_FILE_BYTES
        );
    }

    // Size is within limit; proceed with the read
    let content = tokio::fs::read_to_string(path)
        .await
        .map_err(|err| anyhow::anyhow!("failed to read file '{}': {}", path, err))?;

    Ok(content)
}

#[allow(dead_code)] // utility prepared for nightly pipeline consumption
async fn load_nightly_inputs() -> Result<NightlyInputs> {
    let path = std::env::var("NIGHTLY_INPUT_PATH")
        .unwrap_or_else(|_| "runtime/nightly_inputs.json".to_string());
    let content = read_file_with_size_check(&path).await?;
    let payload = serde_json::from_str::<NightlyInputs>(&content)?;
    Ok(payload)
}

#[allow(dead_code)]
async fn load_weekly_inputs() -> Result<WeeklyInputs> {
    let path = std::env::var("WEEKLY_INPUT_PATH")
        .unwrap_or_else(|_| "runtime/weekly_inputs.json".to_string());
    let content = read_file_with_size_check(&path).await?;
    let payload = serde_json::from_str::<WeeklyInputs>(&content)?;
    Ok(payload)
}

fn format_status(run: &JobRun) -> &'static str {
    match run.status {
        JobStatus::Succeeded { .. } => "succeeded",
        JobStatus::Failed { .. } => "failed",
        JobStatus::Skipped { .. } => "skipped",
        JobStatus::Running => "running",
        JobStatus::Pending => "pending",
    }
}

#[cfg(test)]
mod tests;
