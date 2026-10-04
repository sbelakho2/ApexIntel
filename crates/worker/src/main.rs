#![cfg_attr(test, allow(dead_code))]
#![allow(clippy::duplicated_attributes)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod config;
mod digest_filtering;
#[cfg(feature = "llm")]
mod entity_admission;
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

use anyhow::{Context, Result};
use apex_core::config::AppConfig;
use apex_core::entities::{Observation, ObservationType};
#[cfg(feature = "llm")]
use apex_core::entities::{Person, PriorityVector};
use apex_core::env::parse_truthy_flag;
use apex_core::profile::DeploymentProfile;
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
use apex_worker::notifications::{SlaEnforcer, SlaWarningRecord};
use apex_worker::scheduler::{
    default_scheduler_from_config, parse_custom_command_argv, validate_custom_command, JobKind,
    JobRun, JobStatus, Scheduler,
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
use job_execution::JobExecutionContext;
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
use std::sync::atomic::{AtomicI64, Ordering};
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

mod alert_evaluator;
mod alert_pipeline;
mod alert_transport;
mod artifacts;
mod bootstrap;
mod continuous_improvement;
mod digest;
mod generation;
mod intelligence_ingress;
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

/// Stable per-process instance id used for job ownership (`worker_job_state`
/// / `worker_job_history` `instance_id`) and the shutdown reconciliation.
///
/// Shares the heartbeat identity resolver so the process that beats, claims
/// jobs and reconciles interrupted rows is always the same instance.
fn process_instance_id() -> String {
    apex_worker::healthcheck::resolve_instance_id()
}

/// Default worker database pool size (audit #65). The previous hard-coded 5
/// starved the pool whenever several jobs ran concurrently, because every job
/// holds a connection for its whole run.
pub(crate) const DEFAULT_WORKER_DB_MAX_CONNECTIONS: u32 = 20;

/// Resolve `WORKER_DB_MAX_CONNECTIONS` (default 20, clamped to 5..=100).
///
/// Clamping is deliberate: a typo (`0`, `100000`) must neither starve the
/// worker's own jobs nor exhaust PostgreSQL's connection slots.
pub(crate) fn resolve_worker_db_max_connections(raw: Option<&str>) -> u32 {
    raw.and_then(|value| value.trim().parse::<u32>().ok())
        .unwrap_or(DEFAULT_WORKER_DB_MAX_CONNECTIONS)
        .clamp(5, 100)
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
    // Build the URL through `url::Url` so credentials are percent-encoded;
    // a password containing `@`, `:`, `/` or `#` previously broke proxy auth
    // or changed the host.
    let mut url = url::Url::parse(&format!("http://{host}:{port}")).ok()?;
    url.set_username(&username).ok()?;
    url.set_password(Some(&password)).ok()?;
    Some(url.to_string())
}

/// Capacity fallback for the proxy rotation pool when `PROXY_POOL_SIZE` is
/// absent or malformed; matches `AppConfig`'s documented default.
pub(crate) const DEFAULT_PROXY_POOL_SIZE: usize = 50;

fn proxy_pool_size_from_env() -> usize {
    std::env::var(apex_core::env::PROXY_POOL_SIZE)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(DEFAULT_PROXY_POOL_SIZE)
        .max(1)
}

fn build_proxy_rotator_from_env() -> Option<ProxyRotator> {
    build_proxy_rotator_with_capacity(
        env_flag(apex_core::env::ENABLE_PROXY_ROTATION),
        proxy_pool_size_from_env(),
    )
}

/// Build the proxy rotator with an explicit pool capacity.
///
/// `None` when rotation is disabled, so the crawl client is constructed
/// without a rotator. When enabled, at most `capacity` free proxies are
/// admitted; overflow entries from `PROXY_LIST` are refused (and counted in
/// the warning) instead of growing the pool past the configured bound.
pub(crate) fn build_proxy_rotator_with_capacity(
    enabled: bool,
    capacity: usize,
) -> Option<ProxyRotator> {
    if !enabled {
        return None;
    }

    let paid_proxy = build_paid_proxy_url_from_env();
    if paid_proxy.is_none() {
        tracing::warn!(
            "proxy rotation enabled, but PROXY_HOST/PROXY_PORT and PROXY_USERNAME|PROXY_USER / PROXY_PASSWORD|PROXY_PASS are incomplete"
        );
    }

    let mut rotator = ProxyRotator::with_capacity(true, paid_proxy, capacity);

    if let Ok(proxy_list) = std::env::var("PROXY_LIST") {
        let parsed = ProxyRotator::parse_proxy_list(&proxy_list);
        let requested = parsed.len();
        if requested > 0 {
            let accepted = rotator.add_proxies(parsed);
            if accepted < requested {
                tracing::warn!(
                    requested,
                    accepted,
                    capacity = rotator.capacity(),
                    refused = requested - accepted,
                    "PROXY_LIST exceeds the configured proxy pool size; extra proxies were refused"
                );
            }
        }
    }

    Some(rotator)
}

/// MinIO endpoint + bucket the worker must use for raw-document storage,
/// derived from the validated [`AppConfig`]. Kept pure so the startup wiring
/// (endpoint AND bucket, not just the default `127.0.0.1:9000`) is unit-tested
/// without a live server.
pub(crate) fn minio_store_target(config: &AppConfig) -> (&str, &str) {
    (config.minio_url.as_str(), config.minio_bucket.as_str())
}

/// Verify the configured MinIO bucket at startup.
///
/// Best-effort by design: raw-document archival degrades when MinIO is down,
/// but the worker's database-backed jobs must still run, so an unreachable
/// endpoint or missing credentials is reported loudly (with the configured
/// endpoint and bucket) rather than aborting startup.
async fn ensure_minio_bucket(config: &AppConfig) {
    let (endpoint, bucket) = minio_store_target(config);
    match apex_store::s3::ObjectStore::from_env_credentials(endpoint, bucket).await {
        Ok(Some(store)) => match store.ensure_bucket().await {
            Ok(()) => tracing::info!(endpoint, bucket, "minio: configured bucket verified"),
            Err(error) => tracing::warn!(
                endpoint,
                bucket,
                error = %error,
                "minio: configured bucket unavailable; raw-document archive will fail until MinIO is reachable"
            ),
        },
        Ok(None) => tracing::warn!(
            endpoint,
            bucket,
            "minio: MINIO_ACCESS_KEY/MINIO_SECRET_KEY not set; raw-document archive disabled"
        ),
        Err(error) => tracing::warn!(
            endpoint,
            bucket,
            error = %error,
            "minio: could not build the object store; raw-document archive disabled"
        ),
    }
}

/// Worker process subcommands. Running the scheduler is the default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkerCommand {
    Run,
    Healthcheck,
}

/// Parse `apex-worker [healthcheck]`. Unknown arguments are rejected instead
/// of silently starting a second scheduler.
pub(crate) fn parse_worker_command(
    args: impl IntoIterator<Item = String>,
) -> Result<WorkerCommand> {
    let mut args = args.into_iter();
    match args.next() {
        None => Ok(WorkerCommand::Run),
        Some(flag) if flag == "healthcheck" => {
            if args.next().is_some() {
                anyhow::bail!("'healthcheck' does not take additional arguments");
            }
            Ok(WorkerCommand::Healthcheck)
        }
        Some(other) => {
            anyhow::bail!("unknown worker subcommand '{other}' (supported: healthcheck)")
        }
    }
}

/// Minimal store surface needed to guarantee a current schema at startup.
/// The trait (rather than `PgStore` directly) lets tests drive the startup
/// gate with a fake store.
#[async_trait::async_trait]
pub(crate) trait SchemaMigrationStore {
    async fn run_migrations(&self) -> Result<()>;
}

#[async_trait::async_trait]
impl SchemaMigrationStore for PgStore {
    async fn run_migrations(&self) -> Result<()> {
        PgStore::run_migrations(self).await
    }
}

/// Fail startup when the database schema is not current.
pub(crate) async fn ensure_database_schema(store: &impl SchemaMigrationStore) -> Result<()> {
    store
        .run_migrations()
        .await
        .context("database schema is not current")
}

/// Tracks the last moment the scheduler made progress (tick start or a job
/// completion).
///
/// The heartbeat task consults this clock: once no progress has been recorded
/// for longer than the declared work budget, heartbeat writes stop so the
/// stale `service_heartbeats` row makes `apex-worker healthcheck` fail instead
/// of reporting a wedged scheduler healthy. Because every in-flight job is
/// aborted at its own enforced timeout, a healthy tick always records progress
/// within one job timeout even when several concurrency waves are running.
#[derive(Debug)]
pub(crate) struct SchedulerProgressClock {
    last_progress_epoch_secs: AtomicI64,
}

impl SchedulerProgressClock {
    pub(crate) fn new() -> Self {
        Self::new_at(Utc::now().timestamp())
    }

    pub(crate) fn new_at(epoch_secs: i64) -> Self {
        Self {
            last_progress_epoch_secs: AtomicI64::new(epoch_secs),
        }
    }

    pub(crate) fn record_progress(&self) {
        self.record_progress_at(Utc::now().timestamp());
    }

    pub(crate) fn record_progress_at(&self, epoch_secs: i64) {
        self.last_progress_epoch_secs
            .store(epoch_secs, Ordering::SeqCst);
    }

    pub(crate) fn is_stalled(&self, now_epoch_secs: i64, budget_secs: i64) -> bool {
        now_epoch_secs - self.last_progress_epoch_secs.load(Ordering::SeqCst) > budget_secs
    }
}

#[tokio::main]
#[allow(clippy::unwrap_used, clippy::expect_used)]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();

    match parse_worker_command(std::env::args().skip(1))? {
        WorkerCommand::Healthcheck => return run_worker_healthcheck().await,
        WorkerCommand::Run => {}
    }

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

    let profile = DeploymentProfile::from_env()?;
    if profile.requires_llm_build() && !apex_worker::BUILD_LLM_ENABLED {
        anyhow::bail!(
            "APEX_PROFILE=full requires the 'llm' feature; rebuild apex-worker with --features llm"
        );
    }
    tracing::info!(
        profile = %profile,
        llm_build = apex_worker::BUILD_LLM_ENABLED,
        "deployment profile resolved"
    );

    // Create database pool for recipe insertion. Every worker connection must
    // carry the same default `service` identity as the API pool: migrations
    // 057/058 FORCE RLS on user-private tables, and an unset
    // `app.current_user_role` matches neither the owner nor the service
    // policy, so worker reads would silently return zero rows and writes would
    // be rejected.
    //
    // Pool size: every in-flight job holds a connection for its whole run, so
    // the old hard-coded 5 starved jobs behind the pool (audit #65).
    let db_max_connections = resolve_worker_db_max_connections(
        std::env::var("WORKER_DB_MAX_CONNECTIONS").ok().as_deref(),
    );
    tracing::info!(db_max_connections, "worker database pool configured");
    let pool = PgPoolOptions::new()
        .max_connections(db_max_connections)
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

    // Wrap pool in a shared PgStore so every job handler can query the DB
    // without creating its own connection pool.  Using from_pool() avoids
    // opening a second connection when the pool was already created above.
    let store = Arc::new(PgStore::from_pool(pool.clone()));
    tracing::info!("store initialized");

    // Migrations normally run at API startup, but the worker can start first
    // (independent systemd units). A worker may never run against an unknown
    // schema: migration failure — or, under APEX_SKIP_MIGRATIONS, a mismatch
    // between the applied and embedded latest migration — aborts startup.
    ensure_database_schema(store.as_ref()).await?;
    tracing::info!("worker startup: database schema is current");

    // Raw-document archive: verify the operator-configured MinIO endpoint and
    // bucket before any crawl can store content through S3. Best-effort — the
    // worker's database jobs must run even when object storage is down — but
    // failures are reported with the configured target instead of being
    // silently ignored.
    ensure_minio_bucket(&config).await;

    // Load seed recipes from config/recipes_seed.yaml and insert them into the
    // database. This MUST run after `ensure_database_schema`: on a fresh
    // database the recipes/observations schema does not exist yet, so seeding
    // before migrations failed and recipes stayed missing until the next
    // restart (audit #63). Recipe seeding is bootstrap work, not service
    // wiring; the per-connection `assume_service_identity` hook above is
    // installed before the first query either way.
    bootstrap::seed_recipes_from_yaml(&pool).await;

    // ─── Shared warning ingress ───────────────────────────────────────────
    // Every warning-producing job submits through this ONE ingress so
    // deterministic warning dedup, semantic triage dedup, alert publication
    // and activity logging happen on every producer path. The triage ingestor
    // is constructed here, never per job.
    //
    // Warning persistence now requires `event_outbox` (migration 061): the
    // warning row and its alert event commit in ONE transaction, so a worker
    // that starts against a database without the table would fail every
    // warning insert. Fail fast instead of silently losing ingestion.
    match store.event_outbox_present().await {
        Ok(true) => {}
        Ok(false) => anyhow::bail!(
            "worker startup: event_outbox is missing; apply migration 061_event_outbox.sql \
             before starting this worker"
        ),
        Err(e) => tracing::warn!(
            error = %e,
            "worker startup: could not verify event_outbox presence; continuing"
        ),
    }
    // The claim/lease publisher and the dead-letter state depend on migration
    // 069 columns; fail fast instead of running an unhardened drain.
    match store.event_outbox_lease_columns_present().await {
        Ok(true) => {}
        Ok(false) => anyhow::bail!(
            "worker startup: event_outbox lease/dead-letter columns are missing; apply \
             migration 076_notification_delivery.sql before starting this worker"
        ),
        Err(e) => tracing::warn!(
            error = %e,
            "worker startup: could not verify event_outbox lease columns; continuing"
        ),
    }

    // One shared rules evaluator for both the fast path and the drain, so rule
    // cooldowns are consistent across the two publication triggers. Loading
    // also publishes the engine state (rule count, config hash, reload result)
    // that readiness probes verify.
    let rules_evaluator = alert_pipeline::load_rules_evaluator_with_state(store.as_ref()).await;
    // Keep a handle for the heartbeat's reload check: it verifies the loaded
    // engine still matches the rules file instead of re-reading the file as if
    // it were the engine.
    let heartbeat_alert_evaluator = rules_evaluator.clone();
    let ingress =
        Arc::new(intelligence_ingress::build(Arc::clone(&store), rules_evaluator.clone()).await);
    tracing::info!("intelligence ingress initialized");

    // ─── Canonical alert outbox publisher ─────────────────────────────────
    // ONE publication path: warnings commit their alert event to
    // `event_outbox` in the same transaction as the warning row, and this
    // drain task redelivers anything the ingress fast path left unpublished
    // (crash, ACK failure, optional NATS outage), waiting for the real
    // publish ACK before stamping `published_at`.
    alert_pipeline::spawn(Arc::clone(&store), rules_evaluator);

    // Create the shared ActivityLogger for recording system events
    // to the activity_feed table across all pipeline stages.
    let _activity_logger = ActivityLogger::new(pool.clone());
    tracing::info!("activity_logger initialized");

    // The validated AppConfig drives the crawl interval, nightly anchor hour,
    // and weekly day; `default_scheduler()` remains the historical literal
    // schedule used by tests and callers without a config handle.
    let mut scheduler_state = default_scheduler_from_config(&config);
    match store.list_worker_job_states().await {
        Ok(states) => {
            runtime::restore_scheduler_state(&mut scheduler_state, &states);
            tracing::info!(states = states.len(), "restored persisted scheduler state");
        }
        Err(error) => {
            tracing::warn!(error = %error, "failed to restore persisted scheduler state");
        }
    }

    // A tick may legitimately run for several concurrency waves of jobs, but
    // every in-flight job is aborted at its enforced timeout, so a healthy
    // scheduler always records progress within one effective job timeout.
    // Longer than that plus slack means the tick is genuinely wedged.
    let scheduler_stall_budget_secs = std::env::var("WORKER_SCHEDULER_STALL_BUDGET_SECS")
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or_else(|| {
            scheduler_state
                .jobs
                .values()
                .map(runtime::effective_job_timeout_secs)
                .max()
                .unwrap_or(7200) as i64
                + 900
        });
    let scheduler_progress = Arc::new(SchedulerProgressClock::new());

    // Stable process identity: job claims, job history rows and the shutdown
    // reconciliation all name this instance, so stopping one replica never
    // touches another replica's live jobs.
    let instance_id = process_instance_id();

    // ─── Liveness heartbeat (migration 049) ───────────────────────────────
    // Health checks read `service_heartbeats.last_seen_at` to distinguish a
    // live worker from one that silently stopped; write every ~30s so
    // staleness is a measurement, not an assumption. The first tick fires
    // immediately, recording a heartbeat at startup. When a scheduler tick
    // exceeds its work budget, heartbeat writes stop so `apex-worker
    // healthcheck` reports the wedged scheduler instead of a healthy process.
    {
        let heartbeat_store = Arc::clone(&store);
        let progress = Arc::clone(&scheduler_progress);
        let instance_id = instance_id.clone();
        tokio::spawn(async move {
            // Same identity the healthcheck subprocess resolves, so a
            // container verifies its own heartbeat row rather than whichever
            // worker replica beat most recently.
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(30));
            loop {
                ticker.tick().await;
                if progress.is_stalled(Utc::now().timestamp(), scheduler_stall_budget_secs) {
                    tracing::error!(
                        scheduler_stall_budget_secs,
                        "scheduler made no progress within its work budget; skipping heartbeat so health checks fail"
                    );
                    continue;
                }
                // Refresh the alert-engine state on the liveness cadence:
                // readiness requires a recent, successful rules reload, and a
                // stale row then means this task stopped refreshing it. The
                // check compares the loaded evaluator's hash with the file, so
                // an edited rules file is reported as a pending restart rather
                // than as loaded rules.
                if let Err(error) = alert_pipeline::refresh_alert_engine_state(
                    &heartbeat_store,
                    heartbeat_alert_evaluator.as_ref(),
                )
                .await
                {
                    tracing::warn!(error = %error, "alert-engine state refresh failed");
                }
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

    let scheduler = Arc::new(TokioMutex::new(scheduler_state));
    let tick_guard = Arc::new(TokioMutex::new(()));
    let trigger_guard = Arc::new(TokioMutex::new(()));
    // #104: detached job tasks report completion through this shared tracker,
    // which is also what the shutdown drain waits on (replacing the old
    // guard-held-across-the-whole-tick scheme).
    let run_tracker = runtime::RunTracker::default();
    let dispatcher = Arc::new(TokioMutex::new(runtime::Dispatcher::from_env(
        run_tracker.clone(),
    )));
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
    // P0 browser crawl: one shared headless renderer for the whole worker
    // lifetime; `Browser`-strategy sources never fall back to plain HTTP.
    let job_context = JobExecutionContext::from_env(Arc::clone(&ingress));
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
                let dispatcher = Arc::clone(&dispatcher);
                let progress = Arc::clone(&scheduler_progress);
                let job_context = job_context.clone();
                let instance_id = instance_id.clone();
                tokio::spawn(async move {
                    // The guard is held for one dispatch pass (drain + spawn),
                    // never across the jobs themselves — ticks no longer skip
                    // when a long job is running (#104).
                    let _guard = tick_guard.lock().await;
                    progress.record_progress();

                    let mut scheduler = scheduler.lock().await;
                    let mut dispatcher = dispatcher.lock().await;
                    runtime::tick_scheduler(
                        &mut scheduler,
                        &store,
                        &job_context,
                        &progress,
                        &instance_id,
                        &mut dispatcher,
                    )
                    .await;
                    progress.record_progress();
                });
            }
            _ = trigger_interval.tick() => {
                let store = Arc::clone(&store);
                let scheduler = Arc::clone(&scheduler);
                let trigger_guard = Arc::clone(&trigger_guard);
                let manual_trigger_semaphore = Arc::clone(&manual_trigger_semaphore);
                let job_context = job_context.clone();
                let instance_id = instance_id.clone();
                let run_tracker = run_tracker.clone();
                tokio::spawn(async move {
                    let Ok(_guard) = trigger_guard.try_lock() else {
                        tracing::debug!("poll_trigger_queue: previous poll still active; skipping tick");
                        return;
                    };

                    runtime::poll_trigger_queue(
                        &store,
                        &scheduler,
                        &manual_trigger_semaphore,
                        manual_max_claims_per_poll,
                        manual_trigger_timeout_secs,
                        &job_context,
                        &instance_id,
                        &run_tracker,
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
    // ─── Graceful drain (audit #64, B324) ─────────────────────────────────
    // The SIGTERM/SIGINT branch above stopped the select loop, so no new tick
    // or trigger poll can start. The guards are held only for one dispatch
    // pass now (#104), so acquiring BOTH proves no tick is mid-dispatch and no
    // trigger poll is claiming; the detached job tasks are then drained via
    // `run_tracker`, which counts every run the dispatcher spawned.
    //
    // The wait is bounded: jobs may legitimately run for hours (their own
    // enforced timeouts), and a container stop must not hang for that long.
    // Anything still in flight past the deadline is aborted and this
    // instance's rows are reconciled to 'interrupted' (migrations 096/098);
    // other replicas' live rows are never touched.
    const SHUTDOWN_DRAIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
    // Let any tick/trigger task spawned just before the signal run first:
    // acquiring an uncontended guard completes without yielding, and without
    // this a not-yet-polled task could take the guard after the drain already
    // released it (its `try_lock` would then succeed against a closing pool).
    tokio::task::yield_now().await;
    let drain_started = std::time::Instant::now();
    let barrier_outcome = tokio::time::timeout(SHUTDOWN_DRAIN_TIMEOUT, async {
        let _tick = tick_guard.lock().await;
        let _trigger = trigger_guard.lock().await;
    })
    .await;
    let remaining = SHUTDOWN_DRAIN_TIMEOUT.saturating_sub(drain_started.elapsed());
    let jobs_drained = if barrier_outcome.is_ok() {
        run_tracker.wait_for_all(remaining).await
    } else {
        false
    };
    if barrier_outcome.is_err() || !jobs_drained {
        tracing::warn!(
            timeout_secs = SHUTDOWN_DRAIN_TIMEOUT.as_secs(),
            in_flight = run_tracker.in_flight(),
            "shutdown drain timed out waiting for in-flight jobs; aborting tasks and marking their rows interrupted"
        );
        run_tracker.abort_all();
        match store.mark_running_jobs_interrupted(&instance_id).await {
            Ok(updated) => tracing::warn!(
                updated,
                "worker shutdown: running job rows marked interrupted"
            ),
            Err(error) => tracing::warn!(
                %error,
                "failed to mark running job rows interrupted; they may remain 'running'"
            ),
        }
    } else {
        // Persist runs that finished after the last tick: with detached
        // dispatch (#104) their completion messages may still be queued.
        {
            let mut scheduler = scheduler.lock().await;
            let mut dispatcher = dispatcher.lock().await;
            dispatcher
                .drain_pending(&mut scheduler, &store, &scheduler_progress, &instance_id)
                .await;
        }
        tracing::info!("shutdown drain complete: no scheduler work in flight");
    }
    pool.close().await;
    tracing::info!("database pool closed");
    Ok(())
}

/// `apex-worker healthcheck`: exit non-zero when the database is unreachable,
/// no worker heartbeat exists, or the newest heartbeat is stale.
async fn run_worker_healthcheck() -> Result<()> {
    let config = AppConfig::from_env()?;
    let health =
        apex_worker::healthcheck::check_worker_heartbeat(config.database_url_value(), Utc::now())
            .await
            .context("worker healthcheck failed")?;

    println!(
        "healthy: worker heartbeat {}s ago (instance {}, version {})",
        health.age_seconds, health.instance_id, health.version
    );
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
        JobStatus::Degraded { .. } => "degraded",
        JobStatus::Failed { .. } => "failed",
        JobStatus::Skipped { .. } => "skipped",
        JobStatus::Running => "running",
        JobStatus::Pending => "pending",
    }
}

#[cfg(test)]
mod db_pool_config_tests {
    use super::{resolve_worker_db_max_connections, DEFAULT_WORKER_DB_MAX_CONNECTIONS};

    #[test]
    fn worker_db_pool_size_defaults_clamps_and_parses() {
        assert_eq!(
            resolve_worker_db_max_connections(None),
            DEFAULT_WORKER_DB_MAX_CONNECTIONS
        );
        assert_eq!(resolve_worker_db_max_connections(Some("")), 20);
        assert_eq!(resolve_worker_db_max_connections(Some("not-a-number")), 20);
        assert_eq!(resolve_worker_db_max_connections(Some(" 80 ")), 80);
        assert_eq!(resolve_worker_db_max_connections(Some("1")), 5);
        assert_eq!(resolve_worker_db_max_connections(Some("1000")), 100);
    }
}

#[cfg(test)]
mod worker_state_restore_tests {
    use super::*;
    use apex_worker::scheduler::default_scheduler;

    /// Audit #64: the shutdown drain records `interrupted` rows; the next
    /// startup restore must understand that status (and the sibling
    /// `running`-after-restart status) instead of discarding it.
    #[test]
    fn interrupted_status_is_mapped_on_startup_restore() {
        let mut scheduler = default_scheduler();
        let kind = scheduler
            .jobs
            .keys()
            .next()
            .cloned()
            .expect("default scheduler has jobs");

        let state = apex_store::postgres::WorkerJobStateRecord {
            job_kind: kind.clone(),
            last_run: None,
            last_status: Some("interrupted".to_string()),
            last_error: Some("worker shut down while the job was running".to_string()),
            last_duration_ms: None,
            consecutive_failures: 0,
            max_consecutive_failures: 3,
            circuit_open: false,
            updated_at: Utc::now(),
        };
        runtime::restore_scheduler_state(&mut scheduler, &[state]);

        match scheduler.jobs[&kind].last_status.as_ref() {
            Some(JobStatus::Skipped { reason }) => assert!(
                reason.contains("interrupted"),
                "interrupted must restore as a skipped-with-reason status, got: {reason}"
            ),
            other => panic!("interrupted must restore as Skipped, got {other:?}"),
        }
    }
}

#[cfg(test)]
mod proxy_url_tests {
    use super::build_paid_proxy_url_from_env;

    const PROXY_KEYS: [&str; 6] = [
        "PROXY_HOST",
        "PROXY_PORT",
        "PROXY_USERNAME",
        "PROXY_USER",
        "PROXY_PASSWORD",
        "PROXY_PASS",
    ];

    fn restore(saved: Vec<(String, Option<String>)>) {
        for (key, value) in saved {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }

    /// Audit #57: credentials must be percent-encoded through `url::Url`, so a
    /// password containing `@`, `:`, `/` or `#` cannot break proxy auth or
    /// silently change the host. The old `format!("http://u:p@host:port")`
    /// interpolated the raw secret into the authority.
    #[test]
    fn proxy_url_percent_encodes_credentials() {
        let saved: Vec<(String, Option<String>)> = PROXY_KEYS
            .iter()
            .map(|key| (key.to_string(), std::env::var(key).ok()))
            .collect();
        for key in PROXY_KEYS {
            std::env::remove_var(key);
        }

        std::env::set_var("PROXY_HOST", "proxy.example.test");
        std::env::set_var("PROXY_PORT", "8080");
        std::env::set_var("PROXY_USERNAME", "user@corp");
        std::env::set_var("PROXY_PASSWORD", "p@ss:w/rd#1");

        let url = build_paid_proxy_url_from_env().expect("complete proxy config");
        assert_eq!(
            url,
            "http://user%40corp:p%40ss%3Aw%2Frd%231@proxy.example.test:8080/"
        );
        // The raw credentials must never appear verbatim in the authority.
        assert!(!url.contains("user@corp"));
        assert!(!url.contains("p@ss:w/rd#1"));
        // The host must be intact and unambiguous.
        let parsed = url::Url::parse(&url).expect("built URL parses");
        assert_eq!(parsed.host_str(), Some("proxy.example.test"));
        assert_eq!(parsed.port(), Some(8080));
        assert_eq!(parsed.to_string(), url);

        // The documented `PROXY_USER` / `PROXY_PASS` fallbacks still work.
        std::env::remove_var("PROXY_USERNAME");
        std::env::remove_var("PROXY_PASSWORD");
        std::env::set_var("PROXY_USER", "fallback-user");
        std::env::set_var("PROXY_PASS", "fallback-pass");
        let url = build_paid_proxy_url_from_env().expect("fallback names accepted");
        assert_eq!(
            url,
            "http://fallback-user:fallback-pass@proxy.example.test:8080/"
        );

        // Incomplete configuration is None, never a half-built URL.
        std::env::remove_var("PROXY_HOST");
        assert!(build_paid_proxy_url_from_env().is_none());

        restore(saved);
    }
}

#[cfg(test)]
mod minio_config_tests {
    use super::minio_store_target;
    use apex_core::config::AppConfig;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// The worker startup path must target the operator-configured endpoint
    /// and bucket — not the localhost default and not a hard-coded bucket.
    #[test]
    fn minio_store_target_uses_configured_endpoint_and_bucket() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|poison| poison.into_inner());
        const KEYS: [&str; 3] = ["DATABASE_URL", "MINIO_URL", "MINIO_BUCKET"];
        let saved: Vec<(&str, Option<String>)> = KEYS
            .iter()
            .map(|key| (*key, std::env::var(key).ok()))
            .collect();

        std::env::set_var("DATABASE_URL", "postgres://test:test@localhost/minio-test");
        std::env::set_var("MINIO_URL", "http://minio.internal:9100");
        std::env::set_var("MINIO_BUCKET", "intel-raw-docs");
        let config = AppConfig::from_env().expect("test config loads");

        assert_eq!(
            minio_store_target(&config),
            ("http://minio.internal:9100", "intel-raw-docs")
        );

        for (key, value) in saved {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }
}

#[cfg(test)]
mod proxy_capacity_tests {
    use super::{build_proxy_rotator_with_capacity, DEFAULT_PROXY_POOL_SIZE};

    #[test]
    fn disabled_rotation_builds_no_rotator() {
        assert!(
            build_proxy_rotator_with_capacity(false, DEFAULT_PROXY_POOL_SIZE).is_none(),
            "rotation disabled must produce no rotator, not a disabled one"
        );
    }

    #[test]
    fn configured_pool_size_bounds_the_rotator() {
        let saved = std::env::var("PROXY_LIST").ok();
        std::env::set_var("PROXY_LIST", "1.1.1.1:1\n2.2.2.2:2\n3.3.3.3:3");

        let rotator = build_proxy_rotator_with_capacity(true, 2).expect("rotation enabled");
        assert_eq!(rotator.capacity(), 2);
        assert_eq!(
            rotator.proxy_count(),
            2,
            "the configured pool size must cap the free pool"
        );

        match saved {
            Some(value) => std::env::set_var("PROXY_LIST", value),
            None => std::env::remove_var("PROXY_LIST"),
        }
    }
}

#[cfg(test)]
mod tests;
