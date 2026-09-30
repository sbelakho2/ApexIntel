#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use anyhow::{Context, Result};
use apex_api::config::ApiRuntimeConfig;
use apex_api::filters::validate_search_text;
use apex_api::middleware::auth::{auth_error_response, authenticate_api_request};
use apex_api::rate_limit::RateLimiter;
use apex_api::responses::{
    aggregate_health, error_response, success, success_with_meta, ApiError, ApiResponse,
    ComponentHealth, ErrorCode, HealthResponse, HealthStatus, PagedResponse, ResponseMeta,
};
use apex_api::routes::capabilities::{
    probe_capabilities, readiness_http_status, Capabilities, ProbeContext, ProductSurface,
    ReadinessReport, SchemaLineageReport, SurfaceHealth,
};
use apex_api::routes::companies::{
    validate_company_id, CompanyDetail, CompanyEvent, CompanyKeyPerson, CompanyListItem,
    CompanySite, CompanySortField, ListCompaniesQuery,
};
use apex_api::routes::graph::{EdgeTypeCount, GraphEdge, GraphNodeLabel, GraphOverviewWithEdges};
use apex_api::routes::insights::{InsightResponse, ListInsightsQuery};
use apex_api::routes::llm::{
    ExtractEntitiesRequest, ExtractEntitiesResponse, GenerateMemoRequest, GenerateMemoResponse,
    GenerateRecipeRequest, GenerateRecipeResponse, SynthesizePoiRequest, SynthesizePoiResponse,
};
#[cfg(feature = "llm")]
use apex_api::routes::llm::{ExtractedEntity, LlmTask, MemoSection};
use apex_api::routes::persons::{
    validate_person_id, ListPersonsQuery, PersonDetail, PersonEvent, PersonListItem,
    PersonSortField, StoredPriorityVector,
};
use apex_api::routes::probes::{
    BrowserProbeState, EmbeddingGenerator, LlmProbeTarget, ReadinessPolicy,
};
use apex_api::routes::recipes::{
    sort_recipes, ListRecipesQuery, RecipeListItem, RecipeSortField, RecipeStatus,
};
use apex_api::routes::search::{
    build_facets, highlight_snippet, sort_by_score, tokenize_query, validate_search_query,
    SearchHit, SearchQuery, SearchResponse, SuggestItem, SuggestQuery, SuggestResponse,
};
use apex_api::routes::security::{
    dns_score, DnsPostureItem, DnsPostureOverview, KevItem, LookalikeDomainItem, SecuritySummary,
};
use apex_api::routes::warnings::{
    validate_acknowledge, validate_warning_id, AcknowledgeRequest, ListWarningsQuery,
    SortDirection, WarningResponse, WarningSortField,
};
use apex_core::alert_config::principal_uuid_from_user_id;
use apex_core::profile::DeploymentProfile;
use apex_core::validation::clamp_ratio;
use apex_store::postgres::{
    AdminCrawlStatus, AdminPoiCoverage, AdminRecipePerformance, ArtifactRow, CapabilityRow,
    CertificationRow, CompanyChangeRow, CompanyListFilters, CompanyRow, DashboardStats,
    DossierEntryRow, EdgeRow, InsightListFilters, InsightRow, LogisticsNodeRow, ObservationRow,
    PersonChangeRow, PersonListFilters, PersonListRow, PersonOrderBy, PersonRow, PgStore,
    ProductFamilyRow, RecipeStatRow, RegulationRow, RoleHistoryRow, SiteRow, WarningListFilters,
    WarningOrderBy, WarningRow, WeeklyMemo,
};
use apex_store::tantivy_index::SearchIndex;
use axum::{
    extract::{ConnectInfo, Path, Query, State},
    http::{header, Method, StatusCode},
    response::{Html, IntoResponse, Response},
    Extension, Json,
};
use chrono::{DateTime, NaiveDate, Utc};
use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};
#[cfg(feature = "llm")]
use serde_json::Value as JsonValue;
use std::{
    collections::HashMap,
    sync::{Arc, OnceLock},
    time::Instant,
};
use tower_http::cors::CorsLayer;
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

#[cfg(feature = "llm")]
use apex_llm::{validators, LlmClient, LlmProvider, ModelConfig, OpenAiCompatibleClient};

mod app_router;
mod mappings;
mod runtime_metrics;

#[path = "api_handlers/activity.rs"]
mod activity_handlers;
#[path = "api_handlers/alert_settings.rs"]
mod alert_settings_handlers;
#[path = "api_handlers/alert_subscriptions.rs"]
mod alert_subscription_handlers;
#[path = "api_handlers/battlecards.rs"]
mod battlecards_handlers;
#[path = "api_handlers/catalog.rs"]
mod catalog_handlers;
#[path = "api_handlers/charts.rs"]
mod charts_handlers;
#[path = "api_handlers/collaboration.rs"]
mod collaboration_handlers;
#[path = "api_handlers/competitors.rs"]
mod competitors_handlers;
#[path = "api_handlers/details.rs"]
mod details_handlers;
#[path = "api_handlers/dossiers.rs"]
mod dossiers_handlers;
#[path = "api_handlers/entities.rs"]
mod entities_handlers;
#[path = "api_handlers/exports.rs"]
mod exports_handlers;
#[path = "api_handlers/graph.rs"]
mod graph_handlers;
#[path = "api_handlers/icp.rs"]
mod icp_handlers;
#[path = "api_handlers/insights.rs"]
mod insights_handlers;
#[path = "api_handlers/llm.rs"]
mod llm_handlers;
#[path = "api_handlers/memos.rs"]
mod memos_handlers;
#[path = "api_handlers/overview.rs"]
mod overview_handlers;
#[path = "api_handlers/psych.rs"]
mod psych_handlers;
#[path = "api_handlers/psych_profiles.rs"]
mod psych_profiles_handlers;
#[path = "api_handlers/recipes.rs"]
mod recipes_handlers;
#[path = "api_handlers/sales.rs"]
mod sales_handlers;
#[path = "api_handlers/security.rs"]
mod security_handlers;
#[path = "api_handlers/supply_risk.rs"]
mod supply_risk_handlers;
#[path = "api_handlers/threat_intel.rs"]
mod threat_intel_handlers;
#[path = "api_handlers/trends.rs"]
mod trends_handlers;
#[path = "api_handlers/triage.rs"]
mod triage_handlers;
#[path = "api_handlers/vector_search.rs"]
mod vector_search_handlers;
#[path = "api_handlers/warnings.rs"]
mod warnings_handlers;
pub(crate) use apex_api::destructive_actions::ApiAuthContext;
pub(crate) use apex_core::data_state::DataState;
pub(crate) use apex_store::autocomplete::AutocompleteIndex;
pub(crate) use apex_store::postgres::{
    CompanyDossier, CompetitorChange, PersonDossier, PersonEngagement, SchemaLineage,
};
pub(crate) use chrono::TimeZone;
pub(crate) use mappings::*;

static STARTED_AT: OnceLock<DateTime<Utc>> = OnceLock::new();
const MAX_JSON_DEPTH: usize = 32;

#[derive(Clone)]
struct AppState {
    store: Arc<PgStore>,
    search_index: Arc<SearchIndex>,
    autocomplete_index: Arc<std::sync::RwLock<AutocompleteIndex>>,
    /// Hot-reloadable API key registry (env or file backed).
    api_keys: Arc<apex_api::api_keys::ApiKeyManager>,
    redis: Option<redis::aio::ConnectionManager>,
    rate_limiter: Arc<RateLimiter>,
    /// Durable login throttle (Redis when configured, otherwise PostgreSQL).
    login_throttle: Arc<apex_api::login_throttle::LoginThrottle>,
    config: Arc<ApiRuntimeConfig>,
    profile: DeploymentProfile,
    /// Configurable intelligence profile for LLM analysis prompts
    /// (`APEX_INTELLIGENCE_PROFILE` / `APEX_INTELLIGENCE_PROFILE_PATH`).
    intelligence_profile: apex_core::intelligence_profile::IntelligenceProfile,
    /// Configurable readiness policy (thresholds + probe budgets).
    policy: Arc<ReadinessPolicy>,
    /// Real browser render self-test capability, or why it is unavailable.
    browser_probe: BrowserProbeState,
    /// Resolved LLM endpoint/model for the capability probe.
    llm_probe_target: Option<LlmProbeTarget>,
    /// Embedding generator for the round-trip canary.
    embedding_generator: Option<Arc<dyn EmbeddingGenerator>>,
    /// SSE manager for real-time alert streaming.
    sse_manager: Option<Arc<apex_api::sse::SseManager>>,
    /// Last measured capability report, refreshed by the status heartbeat
    /// every 30s. Public probes read this cache instead of re-running the
    /// expensive probe set (LLM call, embedding round trip, browser render,
    /// NATS connect) on every anonymous request.
    capabilities_cache: Arc<std::sync::RwLock<Option<Capabilities>>>,
    /// Cancellation token for graceful shutdown (SIGTERM / Ctrl-C).
    shutdown: tokio_util::sync::CancellationToken,
    #[cfg(feature = "llm")]
    llm: Option<LlmRuntime>,
}

#[cfg(feature = "llm")]
#[derive(Clone)]
struct LlmRuntime {
    primary: ModelConfig,
    lightweight: ModelConfig,
}

#[tokio::main]
#[allow(clippy::unwrap_used, clippy::expect_used)]
async fn main() -> Result<()> {
    let log_level = std::env::var("API_LOG_LEVEL").unwrap_or_else(|_| "info".to_string());
    let log_filter = std::env::var("RUST_LOG")
        .unwrap_or_else(|_| format!("{},apex_api={}", log_level, log_level));
    tracing_subscriber::fmt()
        .with_target(false)
        .with_env_filter(EnvFilter::new(log_filter))
        .init();

    STARTED_AT.get_or_init(Utc::now);
    runtime_metrics::init_metrics();

    let state = build_state().await?;
    // Measure liveness/freshness from startup: an API heartbeat row now, then
    // refreshed every ~30s together with the UI status snapshot.
    if let Err(error) = state
        .store
        .record_service_heartbeat("api", &process_instance_id(), env!("CARGO_PKG_VERSION"))
        .await
    {
        tracing::warn!(error = %error, "failed to record startup API heartbeat");
    }
    // Prime the capability cache so no public request ever triggers the
    // expensive probe set itself.
    let initial_capabilities =
        probe_capabilities(&probe_context(&state, Some(&nats_url_from_env()), None)).await;
    if let Ok(mut cache) = state.capabilities_cache.write() {
        *cache = Some(initial_capabilities);
    }
    start_status_heartbeat(state.clone());
    // Hot-reload API keys and provision any new principals.
    apex_api::api_keys::spawn_api_key_reloader_with_provisioning(
        state.api_keys.clone(),
        std::time::Duration::from_secs(state.config.api_keys.reload_interval_secs),
        Some(state.store.clone()),
    );

    let cors = CorsLayer::new()
        .allow_origin(
            state
                .config
                .server
                .cors_origin
                .parse::<axum::http::HeaderValue>()
                .unwrap_or_else(|_| axum::http::HeaderValue::from_static("http://localhost:3000")),
        )
        // PATCH is used by 13 registered routes (executive opportunities/threats,
        // queue, supplier-risk, pipeline) — omitting it broke every cross-origin
        // PATCH preflight (B297).
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
        ])
        .allow_headers([
            header::CONTENT_TYPE,
            header::AUTHORIZATION,
            header::HeaderName::from_static("x-csrf-token"),
        ]);

    let app = app_router::build_app_router(state.clone(), cors);
    let address = format!("{}:{}", state.config.server.host, state.config.server.port);
    let listener = tokio::net::TcpListener::bind(&address).await?;
    tracing::info!(address = %address, "API server listening");
    axum::serve(
        listener,
        // ConnectInfo powers the rate limiter's per-client identity (B298).
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal(state.shutdown.clone()))
    .await?;
    state.store.pool.close().await;
    tracing::info!("database pool closed, shutdown complete");
    Ok(())
}

/// Resolve on Ctrl-C or SIGTERM (docker stop / Kubernetes pod termination),
/// then cancel the token so SSE streams end and the server can drain.
async fn shutdown_signal(shutdown: tokio_util::sync::CancellationToken) {
    let ctrl_c = async {
        tokio::signal::ctrl_c().await.ok();
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(error) => {
                tracing::warn!(%error, "failed to install SIGTERM handler");
                std::future::pending::<()>().await;
            }
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    tracing::info!("received shutdown signal, draining connections");
    shutdown.cancel();
}

/// Documented placeholder / CI secrets that must never sign real sessions.
const KNOWN_PLACEHOLDER_SESSION_SECRETS: &[&str] = &[
    "<openssl rand -hex 32>",
    "<GENERATE_64_CHAR_HEX_SECRET>",
    "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
];

/// Reject placeholder, too-short, or low-entropy session secrets at startup.
///
/// A non-empty documented placeholder otherwise signs cookies that pass the
/// database authority check, so anyone could forge an admin session.
/// `APEX_ALLOW_TEST_SESSION_SECRET=1` is the explicit test/CI override.
fn validate_session_secret(secret: &str) -> Result<()> {
    let secret = secret.trim();
    anyhow::ensure!(
        secret.len() >= 32,
        "SESSION_SECRET must be at least 32 bytes (generate one with `openssl rand -hex 32`)"
    );
    let test_override = std::env::var("APEX_ALLOW_TEST_SESSION_SECRET").as_deref() == Ok("1");
    anyhow::ensure!(
        test_override
            || (!secret.contains('<') && !KNOWN_PLACEHOLDER_SESSION_SECRETS.contains(&secret)),
        "SESSION_SECRET is a documented placeholder/test value; \
         generate a real secret with `openssl rand -hex 32`"
    );
    let distinct: std::collections::HashSet<char> = secret.chars().collect();
    anyhow::ensure!(
        distinct.len() >= 8,
        "SESSION_SECRET has too little entropy (fewer than 8 distinct characters)"
    );
    Ok(())
}

async fn build_state() -> Result<AppState> {
    dotenvy::dotenv().ok();

    // Fail startup on a placeholder/short secret: serving with one would let
    // anyone forge an admin session.
    validate_session_secret(&std::env::var("SESSION_SECRET").unwrap_or_default())?;

    let config = ApiRuntimeConfig::from_env()?;
    let validation_errors = config.validate();
    if !validation_errors.is_empty() {
        for error in &validation_errors {
            eprintln!("CONFIG ERROR: {}", error);
        }
        anyhow::bail!(
            "Configuration validation failed: {} errors",
            validation_errors.len()
        );
    }

    let profile = DeploymentProfile::from_env().context("invalid APEX_PROFILE")?;
    if profile.requires_llm_build() && !apex_api::BUILD_LLM_ENABLED {
        anyhow::bail!(
            "APEX_PROFILE=full requires the 'llm' feature; rebuild apex-api with --features llm"
        );
    }
    tracing::info!(
        profile = %profile,
        llm_build = apex_api::BUILD_LLM_ENABLED,
        "deployment profile resolved"
    );

    // Domain framing for LLM analysis is deployment configuration (audit
    // P1-2): APEX_INTELLIGENCE_PROFILE / APEX_INTELLIGENCE_PROFILE_PATH, with
    // the embedded profile set as the documented default. Invalid
    // configuration fails startup rather than falling back to an empty prompt.
    let intelligence_profile = apex_core::intelligence_profile::IntelligenceProfileSet::from_env()
        .context("invalid intelligence profile configuration")?;
    tracing::info!(
        intelligence_profile = %intelligence_profile.name,
        "intelligence profile resolved"
    );

    let store = Arc::new(PgStore::connect(config.app.database_url.expose_secret()).await?);
    tracing::info!("database pool initialized");

    // Run database migrations (idempotent via IF NOT EXISTS)
    store.run_migrations().await?;
    tracing::info!("database migrations applied");

    let search_index = Arc::new(SearchIndex::open(&config.search.index_path)?);
    tracing::info!("search index loaded");

    // ─── Autocomplete index ───────────────────────────────────────────────
    let autocomplete_path = config.search.index_path.join("autocomplete.fst");
    let autocomplete_index = if autocomplete_path.exists() {
        match AutocompleteIndex::load(&autocomplete_path) {
            Ok(idx) => {
                tracing::info!(path = %autocomplete_path.display(), entries = %idx.len(), "autocomplete index loaded from disk");
                idx
            }
            Err(err) => {
                tracing::warn!(path = %autocomplete_path.display(), error = %err, "failed to load autocomplete index, building empty");
                AutocompleteIndex::new()
            }
        }
    } else {
        tracing::warn!(path = %autocomplete_path.display(), "autocomplete index not found, building empty");
        AutocompleteIndex::new()
    };
    let autocomplete_index = Arc::new(std::sync::RwLock::new(autocomplete_index));
    // ──────────────────────────────────────────────────────────────────────

    // The manager honors API_KEYS_FILE, API_KEYS_RELOAD_INTERVAL_SECS and
    // API_KEYS_ENV_SLOTS, hot-reloads file changes, and feeds the per-key
    // rate limit; the previous `load_api_keys_from_env(50)` ignored all of
    // that and made key revocation a restart.
    let api_keys = Arc::new(apex_api::api_keys::ApiKeyManager::new(&config.api_keys)?);
    tracing::info!(count = %api_keys.key_count(), "API keys loaded");
    apex_api::api_keys::ensure_api_key_principals(&store, &api_keys.snapshot()).await;

    // Environment credentials are bootstrap-only: they seed `app_users` rows
    // that do not yet carry a password hash. Once a row has credentials, the
    // database record is authoritative and the environment is ignored.
    let bootstrapped = apex_api::web::auth::bootstrap_app_users_from_env(&store).await;
    if bootstrapped > 0 {
        tracing::info!(count = %bootstrapped, "bootstrapped app_users identities from environment");
    }

    let redis = {
        let redis_url = config.app.redis_url.expose_secret();
        // An explicitly configured localhost Redis is still configured: the
        // old default-address comparison silently disabled it.
        if !redis_url.trim().is_empty() {
            let client = redis::Client::open(redis_url)?;
            let conn = client.get_connection_manager().await?;
            tracing::info!("Redis connection established");
            Some(conn)
        } else {
            tracing::warn!("Redis not configured, rate limiting disabled");
            None
        }
    };

    let rate_limiter = Arc::new(RateLimiter::new());
    tracing::info!("rate limiter initialized");

    // Durable login throttle: Redis when REDIS_URL is configured, otherwise
    // the canonical PostgreSQL store, so lockouts survive restarts and are
    // shared by every replica.
    let login_throttle = Arc::new(apex_api::login_throttle::LoginThrottle::new(
        Some(store.clone()),
        redis.clone(),
    ));
    tracing::info!(
        backend = login_throttle.backend().as_str(),
        "login throttle initialized"
    );

    // ─── SSE / Real-time alerts ─────────────────────────────────────────
    let nats_url =
        std::env::var("NATS_URL").unwrap_or_else(|_| "nats://127.0.0.1:4222".to_string());
    let sse_manager = if !nats_url.is_empty() {
        let manager = Arc::new(apex_api::sse::SseManager::new());
        let alert_router = Arc::new(apex_api::alert_router::AlertRouter::new(store.clone()));

        // Start NATS consumer in background
        let nats_enabled = std::env::var("NATS_SSE_ENABLED")
            .ok()
            .map(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
            .unwrap_or(true);

        if nats_enabled {
            let mgr = manager.clone();
            let router = alert_router.clone();
            let url = nats_url.clone();
            tokio::spawn(async move {
                mgr.start_nats_consumer(&url, router).await;
            });
            tracing::info!(nats_url = %nats_url, "SSE real-time alerts enabled with NATS consumer");
        } else {
            tracing::info!(
                "SSE manager initialized (NATS consumer disabled by NATS_SSE_ENABLED=false)"
            );
        }

        Some(manager)
    } else {
        tracing::warn!("NATS_URL not set — SSE real-time alerts disabled");
        None
    };
    // ─────────────────────────────────────────────────────────────────────

    #[cfg(feature = "llm")]
    let llm = build_llm_runtime(&config)?;

    // ─── Readiness truth wiring ───────────────────────────────────────────
    // Probes measure real capability: the policy carries configurable
    // thresholds, the browser state can run a real render self-test, and the
    // LLM/embedding probes target the configured endpoint/model. A malformed
    // readiness/security threshold is a startup configuration error naming the
    // variable and value — never a silent fallback.
    let policy = Arc::new(
        ReadinessPolicy::from_env()
            .map_err(|errors| anyhow::anyhow!("invalid readiness configuration: {errors}"))?,
    );
    tracing::info!(
        search_index_max_lag_secs = policy.search_index_max_lag_secs,
        crawl_freshness_max_age_secs = policy.crawl_freshness_max_age_secs,
        coverage_families = policy.coverage.families.len(),
        priority_company_coverage_min_pct = policy.coverage.min_priority_company_coverage_pct,
        critical_jobs = ?policy.critical_jobs,
        "readiness policy resolved"
    );

    let browser_probe = BrowserProbeState::from_env();
    if let BrowserProbeState::Unavailable(reason) = &browser_probe {
        tracing::warn!(reason = %reason, "headless browser enabled but renderer unavailable");
    }

    #[cfg(feature = "llm")]
    let llm_probe_target = llm.as_ref().map(|runtime| LlmProbeTarget {
        base_url: runtime.lightweight.base_url.clone(),
        chat_endpoint: runtime.lightweight.chat_endpoint(),
        model: runtime.lightweight.model_name.clone(),
        timeout_secs: policy.llm_probe_timeout_secs,
    });
    #[cfg(not(feature = "llm"))]
    let llm_probe_target: Option<LlmProbeTarget> = None;

    #[cfg(feature = "llm")]
    let embedding_generator: Option<Arc<dyn EmbeddingGenerator>> = llm.as_ref().map(|runtime| {
        Arc::new(apex_llm::embeddings::EmbeddingClient::from_config(
            &runtime.lightweight,
        )) as Arc<dyn EmbeddingGenerator>
    });
    #[cfg(not(feature = "llm"))]
    let embedding_generator: Option<Arc<dyn EmbeddingGenerator>> = None;
    // ──────────────────────────────────────────────────────────────────────

    Ok(AppState {
        store,
        search_index,
        autocomplete_index,
        api_keys,
        redis,
        rate_limiter,
        login_throttle,
        config: Arc::new(config),
        profile,
        intelligence_profile,
        policy,
        browser_probe,
        llm_probe_target,
        embedding_generator,
        sse_manager,
        capabilities_cache: Arc::new(std::sync::RwLock::new(None)),
        shutdown: tokio_util::sync::CancellationToken::new(),
        #[cfg(feature = "llm")]
        llm,
    })
}

#[cfg(feature = "llm")]
fn build_llm_runtime(config: &ApiRuntimeConfig) -> Result<Option<LlmRuntime>> {
    // Model name / endpoint / key now live on AppConfig (config.app); the
    // LlmRuntimeConfig (config.llm) only carries token & timeout budgets plus
    // provider override / validation policy.
    let model_name = config.llm_model_name();
    if model_name.is_empty() {
        return Ok(None);
    }

    let base_url = config
        .app
        .llm_base_url
        .clone()
        .unwrap_or_else(|| "http://localhost:8080".to_string());
    let provider = infer_llm_provider(config, &base_url);
    let api_key = config
        .app
        .llm_api_key
        .as_ref()
        .map(|secret| apex_llm::ApiKeySecret::from(secret.expose_secret()));

    let primary = ModelConfig {
        model_name: model_name.to_string(),
        provider: provider.clone(),
        base_url: base_url.clone(),
        api_key: api_key.clone(),
        max_tokens: config.llm.primary_max_tokens,
        temperature: 0.2,
        timeout_seconds: config.llm.primary_timeout_secs,
    };

    let lightweight = ModelConfig {
        model_name: model_name.to_string(),
        provider,
        base_url,
        api_key,
        max_tokens: config.llm.lightweight_max_tokens,
        temperature: 0.0,
        timeout_seconds: config.llm.lightweight_timeout_secs,
    };

    Ok(Some(LlmRuntime {
        primary,
        lightweight,
    }))
}

#[cfg(not(feature = "llm"))]
#[allow(dead_code)]
fn build_llm_runtime(_config: &ApiRuntimeConfig) -> Result<Option<()>> {
    Ok(None)
}

#[cfg(feature = "llm")]
fn infer_llm_provider(config: &ApiRuntimeConfig, base_url: &str) -> LlmProvider {
    use apex_api::config::LlmProviderChoice;

    // Explicit override wins over URL inference.
    if let Some(choice) = config.llm.provider_override {
        return match choice {
            LlmProviderChoice::LlamaCpp => LlmProvider::LlamaCpp,
            LlmProviderChoice::OpenAi => LlmProvider::OpenAi,
            LlmProviderChoice::AzureOpenAi => LlmProvider::AzureOpenAi,
        };
    }

    if base_url.contains("azure") {
        LlmProvider::AzureOpenAi
    } else if base_url.contains("openai") {
        LlmProvider::OpenAi
    } else {
        // Local llama-server / OpenAI-compatible endpoint is the platform default.
        LlmProvider::LlamaCpp
    }
}

async fn require_auth(
    State(state): State<AppState>,
    mut request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    // Snapshot once per request: a concurrent hot reload keeps serving the
    // previous consistent registry instead of tearing.
    let api_keys = state.api_keys.snapshot();
    let result =
        authenticate_api_request(request.headers(), request.method(), &api_keys, Utc::now());

    match result {
        Ok(auth) => {
            // Enforce the key's own per-minute limit (the registry carried
            // `rate_limit_per_min` but nothing applied it).
            let limit = auth.rate_limit_per_min.max(1);
            let decision = state.rate_limiter.check_with_limit(
                &auth.auth_context.key_id,
                "api-key",
                limit,
                std::time::Duration::from_secs(60),
            );
            if !decision.allowed {
                let retry_after = decision.retry_after_secs.max(1) as u32;
                let mut response = auth_error_response(ApiError::rate_limited(retry_after));
                if let Ok(value) = axum::http::HeaderValue::from_str(&retry_after.to_string()) {
                    response
                        .headers_mut()
                        .insert(axum::http::header::RETRY_AFTER, value);
                }
                return response;
            }
            request.extensions_mut().insert(auth.auth_context);
            next.run(request).await
        }
        Err(api_err) => {
            // B291: browser-session fallback. The web UI links and scripts call
            // `/api/*` endpoints (charts, CSV/PDF exports, graph expansion,
            // alert settings, battlecard regeneration) with the ambient
            // `apex_session` cookie rather than a Bearer key. Accept a valid web
            // session as its canonical `app_users` role — but only when no
            // Authorization header was presented (an invalid key must fail, not
            // silently downgrade) and, for unsafe methods, only when the
            // double-submit CSRF check passes.
            if let Some(ctx) =
                session_fallback_context(request.headers(), request.method(), &state).await
            {
                request.extensions_mut().insert(ctx);
                return next.run(request).await;
            }
            auth_error_response(api_err)
        }
    }
}

/// Build an `ApiAuthContext` from the browser session cookie.
/// Returns `None` when a Bearer key was presented (handled above), the session
/// is missing/expired, or an unsafe method fails the CSRF check.
///
/// The context carries the role resolved against the canonical `app_users`
/// row (via `session_api_context`), so a role change or disabled account
/// takes effect on API calls immediately and an admin web session keeps admin
/// capabilities on `/api/admin/*` instead of being downgraded.
async fn session_fallback_context(
    headers: &axum::http::HeaderMap,
    method: &axum::http::Method,
    state: &AppState,
) -> Option<ApiAuthContext> {
    apex_api::middleware::session::session_api_context(headers, method, state.store.as_ref()).await
}

async fn add_rate_limit_headers(
    Extension(limiter): Extension<Arc<RateLimiter>>,
    ConnectInfo(connect_info): ConnectInfo<std::net::SocketAddr>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let path = request.uri().path();
    // Static assets and the service worker are not API budget: charging every
    // page load's CSS/JS against 120/min 429s offices behind one NAT.
    if path.starts_with("/static/") || path == "/sw.js" {
        return next.run(request).await;
    }

    let tier = apex_api::rate_limit::classify_endpoint(path, request.method().as_str());
    // B298: identify clients by the trusted address. `X-Forwarded-For` is
    // honored only under API_TRUST_PROXY=1, and then only its rightmost
    // (proxy-appended) entry — the leftmost entry is client-controlled and
    // trivially rotated to bypass limits.
    // IPv6 clients are bucketed by /64 so a subscriber cannot rotate through
    // its prefix to get fresh buckets (see `rate_limit_identity`).
    let identifier =
        apex_api::middleware::client_ip::client_ip(request.headers(), Some(connect_info.ip()))
            .map(apex_api::middleware::client_ip::rate_limit_identity)
            .unwrap_or_else(|| {
                apex_api::middleware::client_ip::rate_limit_identity(connect_info.ip())
            });
    let result = limiter.check(&identifier, tier);
    if !result.allowed {
        let retry_after = result.retry_after_secs.max(1) as u32;
        let api_err = ApiError::rate_limited(retry_after);
        let mut resp = (
            axum::http::StatusCode::TOO_MANY_REQUESTS,
            Json(error_response::<()>(api_err)),
        )
            .into_response();
        if let Ok(value) = axum::http::HeaderValue::from_str(&retry_after.to_string()) {
            resp.headers_mut()
                .insert(axum::http::header::RETRY_AFTER, value);
        }
        return resp;
    }
    let mut response = next.run(request).await;
    response.headers_mut().insert(
        axum::http::header::HeaderName::from_static("x-ratelimit-limit"),
        axum::http::HeaderValue::from(result.limit),
    );
    response.headers_mut().insert(
        axum::http::header::HeaderName::from_static("x-ratelimit-remaining"),
        axum::http::HeaderValue::from(result.remaining),
    );
    response
}

/// Build an explicit API error for a failed repository load so the client
/// never receives a failure shaped like an empty result set.
fn degraded_api_error<T>(context: &str, state: &DataState<T>) -> ApiError {
    match state.error_id() {
        Some(error_id) => ApiError::internal(format!(
            "{context} — data unavailable · incident {error_id}"
        )),
        None => ApiError::internal(context),
    }
}

fn llm_service_unavailable<T: Serialize>(message: &str) -> (StatusCode, Json<ApiResponse<T>>) {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(error_response(ApiError::new(
            ErrorCode::ServiceUnavailable,
            message,
        ))),
    )
}

// ──────────────────────────────────────────────────────────────────────────────
// Health & Metadata
// ──────────────────────────────────────────────────────────────────────────────

fn nats_url_from_env() -> String {
    std::env::var("NATS_URL").unwrap_or_else(|_| "nats://127.0.0.1:4222".to_string())
}

/// Stable per-process instance id for `service_heartbeats`.
fn process_instance_id() -> String {
    std::env::var("APEX_INSTANCE_ID")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            let host = std::env::var("HOSTNAME").unwrap_or_else(|_| "localhost".to_string());
            format!("{host}-{}", std::process::id())
        })
}

/// The heartbeat-measured capability report. Public probes read this cache so
/// an anonymous client cannot drive the LLM call, embedding round trip,
/// browser render, or NATS connect per request. Only a process whose cache was
/// never primed (tests) pays for a live probe.
async fn capabilities_snapshot(state: &AppState) -> Capabilities {
    if let Some(cached) = state
        .capabilities_cache
        .read()
        .ok()
        .and_then(|cache| cache.clone())
    {
        return cached;
    }
    probe_capabilities(&probe_context(state, Some(&nats_url_from_env()), None)).await
}

/// Resolve the capability probe context from process state. `nats_url` is
/// `None` when the profile does not require NATS, so optional probes stay
/// side-effect free.
fn probe_context<'a>(
    state: &'a AppState,
    nats_url: Option<&'a str>,
    schema_lineage: Option<&'a SchemaLineage>,
) -> ProbeContext<'a> {
    ProbeContext {
        pool: &state.store.pool,
        search_index: &state.search_index,
        nats_url,
        policy: state.policy.as_ref(),
        llm: state.llm_probe_target.as_ref(),
        browser: &state.browser_probe,
        embedding_generator: state.embedding_generator.as_deref(),
        schema_lineage,
    }
}

/// Record API heartbeats and refresh the UI status snapshot every ~30s.
/// The first tick runs immediately, so startup liveness is measured too.
fn start_status_heartbeat(state: AppState) {
    let instance_id = process_instance_id();
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(30));
        loop {
            ticker.tick().await;

            // NATS is optional under `core`, so skip the live connect probe there and
            // only measure it when the profile requires NATS.
            let nats_url = nats_url_from_env();
            let nats_probe = if state.profile.requires_capability("nats") {
                Some(nats_url.as_str())
            } else {
                None
            };
            let capabilities = probe_capabilities(&probe_context(&state, nats_probe, None)).await;
            apex_api::system_status::StatusStrip::publish(capabilities.status_strip());
            // The same measured report powers the server-rendered capability
            // badges (admin page), so the UI never claims a state the probes
            // did not measure, and public probes read this cached copy instead
            // of re-running the expensive probe set per request.
            apex_api::system_status::publish_capabilities(&capabilities);
            if let Ok(mut cache) = state.capabilities_cache.write() {
                *cache = Some(capabilities.clone());
            }

            if let Err(error) = state
                .store
                .record_service_heartbeat("api", &instance_id, env!("CARGO_PKG_VERSION"))
                .await
            {
                tracing::warn!(error = %error, "failed to record API heartbeat");
            }

            // Bounded maintenance: logout revocations are only kept until the
            // revoked session's own expiry.
            match state.store.purge_expired_revoked_sessions(Utc::now()).await {
                Ok(purged) if purged > 0 => {
                    tracing::debug!(purged, "purged expired revoked sessions");
                }
                Ok(_) => {}
                Err(error) => {
                    tracing::warn!(error = %error, "failed to purge revoked sessions");
                }
            }

            // Housekeeping moved off the polling GET: a warning-analysis run
            // abandoned by a crashed process must resolve to an explicit
            // failure, but the UPDATE belongs on the heartbeat, not per poll.
            // (The warning-analysis pipeline only exists with the `llm` feature.)
            #[cfg(feature = "llm")]
            {
                let stale_seconds = state
                    .llm
                    .as_ref()
                    .map(|runtime| {
                        apex_api::warning_analysis::stale_run_seconds(
                            runtime.primary.timeout_seconds,
                        )
                    })
                    .unwrap_or(apex_api::warning_analysis::DEFAULT_STALE_RUN_SECONDS);
                apex_api::warning_analysis::expire_stale_runs(&state.store, stale_seconds).await;
            }
        }
    });
}

/// `/api/health` — overall health with capability checks derived from real
/// probes (database, LLM endpoint, embeddings canary, NATS, browser render
/// self-test, search index lag, worker heartbeat, data freshness, source
/// coverage, alert-engine state, outbox backlog, scheduled-job freshness).
async fn health(State(state): State<AppState>) -> (StatusCode, Json<HealthResponse>) {
    let uptime_secs = STARTED_AT
        .get()
        .map(|started| (Utc::now() - started).num_seconds() as u64)
        .unwrap_or(0);

    let capabilities = capabilities_snapshot(&state).await;
    let checks = capabilities.health_checks();
    let overall = aggregate_health(&checks);

    (
        readiness_http_status(&overall),
        Json(HealthResponse {
            status: overall,
            version: env!("CARGO_PKG_VERSION").to_string(),
            uptime_secs,
            checks,
        }),
    )
}

/// `/api/health/capabilities` — per-capability probe report for the UI,
/// dashboards, and operational tooling.
async fn health_capabilities(State(state): State<AppState>) -> Json<Capabilities> {
    Json(capabilities_snapshot(&state).await)
}

async fn health_live() -> StatusCode {
    StatusCode::OK
}

/// `/api/health/ready` — profile-aware readiness probe. Under `full` this is
/// the composed product readiness: process + data + intelligence + delivery
/// surfaces, each proving its required capabilities. Answers 503 when any
/// required capability is not `ok`, and publishes the required set and
/// configurable thresholds as policy. `/api/health` remains the detailed
/// matrix.
async fn health_ready(State(state): State<AppState>) -> (StatusCode, Json<ReadinessReport>) {
    let uptime_secs = STARTED_AT
        .get()
        .map(|started| (Utc::now() - started).num_seconds() as u64)
        .unwrap_or(0);

    // Probe only the capabilities this profile requires; optional capabilities
    // are not measured, keeping the frequently polled probe cheap. The schema
    // lineage is measured once and reused by both the `schema` capability and
    // the readiness report, so the two always describe the same snapshot.
    let schema_lineage = state
        .store
        .schema_lineage()
        .await
        .map_err(|error| error.to_string());
    // The capability snapshot comes from the heartbeat cache; only the cheap
    // lineage query is measured per request to keep the report self-consistent.
    let capabilities = capabilities_snapshot(&state).await;
    let checks = capabilities.readiness_checks(state.profile);
    let surfaces = capabilities.surface_reports(state.profile);
    let overall = aggregate_health(&checks);

    (
        readiness_http_status(&overall),
        Json(ReadinessReport {
            status: overall,
            version: env!("CARGO_PKG_VERSION").to_string(),
            uptime_secs,
            profile: state.profile.to_string(),
            required_capabilities: state
                .profile
                .required_capabilities()
                .iter()
                .map(|name| (*name).to_string())
                .collect(),
            thresholds: state.policy.as_ref().clone(),
            schema_lineage: SchemaLineageReport::from_result(schema_lineage),
            surfaces,
            checks,
        }),
    )
}

async fn surface_health(
    state: &AppState,
    surface: ProductSurface,
) -> (StatusCode, Json<SurfaceHealth>) {
    let capabilities = capabilities_snapshot(state).await;
    let report =
        SurfaceHealth::from_checks(surface, capabilities.surface_checks(surface, state.profile));
    (report.http_status(), Json(report))
}

/// `/process/ready` — process surface: database, worker heartbeat, and
/// critical scheduled-job freshness for the active profile.
async fn process_ready(State(state): State<AppState>) -> (StatusCode, Json<SurfaceHealth>) {
    surface_health(&state, ProductSurface::Process).await
}

/// `/data/healthy` — data surface: crawl freshness, operational source
/// coverage, search-index lag, and real browser rendering.
async fn data_healthy(State(state): State<AppState>) -> (StatusCode, Json<SurfaceHealth>) {
    surface_health(&state, ProductSurface::Data).await
}

/// `/intelligence/healthy` — intelligence surface: LLM endpoint, embedding
/// round trip, and alert-rule engine state.
async fn intelligence_healthy(State(state): State<AppState>) -> (StatusCode, Json<SurfaceHealth>) {
    surface_health(&state, ProductSurface::Intelligence).await
}

/// `/delivery/healthy` — delivery surface: NATS connectivity and outbox
/// publisher backlog.
async fn delivery_healthy(State(state): State<AppState>) -> (StatusCode, Json<SurfaceHealth>) {
    surface_health(&state, ProductSurface::Delivery).await
}

/// `/process/live` — process liveness; answers 200 whenever the process is
/// serving requests.
async fn process_live(State(state): State<AppState>) -> Json<serde_json::Value> {
    let uptime_secs = STARTED_AT
        .get()
        .map(|started| (Utc::now() - started).num_seconds().max(0) as u64)
        .unwrap_or(0);
    Json(serde_json::json!({
        "status": "ok",
        "profile": state.profile.to_string(),
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_secs": uptime_secs,
    }))
}

async fn health_deep(State(mut state): State<AppState>) -> (StatusCode, Json<HealthResponse>) {
    let uptime_secs = STARTED_AT
        .get()
        .map(|started| (Utc::now() - started).num_seconds() as u64)
        .unwrap_or(0);

    let store_check = sqlx::query("SELECT 1").execute(&state.store.pool).await;
    let db_status = match &store_check {
        Ok(_) => HealthStatus::Healthy,
        Err(e) => {
            tracing::warn!("database health check failed: {}", e);
            HealthStatus::Unhealthy
        }
    };

    let mut checks = vec![ComponentHealth {
        name: "database".to_string(),
        status: db_status,
        message: store_check.as_ref().err().map(|e| e.to_string()),
    }];

    if let Some(ref mut redis) = state.redis {
        let redis_check: Result<String, redis::RedisError> =
            redis::cmd("PING").query_async(redis).await;
        let redis_status = match &redis_check {
            Ok(_) => HealthStatus::Healthy,
            Err(e) => {
                tracing::warn!("redis health check failed: {}", e);
                HealthStatus::Degraded
            }
        };
        checks.push(ComponentHealth {
            name: "redis".to_string(),
            status: redis_status,
            message: redis_check.as_ref().err().map(|e| e.to_string()),
        });
    } else {
        checks.push(ComponentHealth {
            name: "redis".to_string(),
            status: HealthStatus::Degraded,
            message: Some("Redis not configured".to_string()),
        });
    }

    let overall = aggregate_health(&checks);
    (
        match overall {
            HealthStatus::Unhealthy => StatusCode::SERVICE_UNAVAILABLE,
            HealthStatus::Degraded => StatusCode::OK,
            HealthStatus::Healthy => StatusCode::OK,
        },
        Json(HealthResponse {
            status: overall,
            version: env!("CARGO_PKG_VERSION").to_string(),
            uptime_secs,
            checks,
        }),
    )
}

async fn endpoints() -> Json<Vec<apex_api::routes::EndpointDef>> {
    Json(apex_api::routes::all_endpoints())
}

/// `/api/version` — running deployment provenance: the git SHA, CI pipeline,
/// artifact digest, and deploy time recorded by `scripts/ops/record_deployment.sh`
/// at deploy time. Unconfigured values are reported as `"unknown"` rather than
/// guessed.
async fn api_version() -> Json<apex_api::provenance::DeploymentProvenance> {
    Json(apex_api::provenance::DeploymentProvenance::from_env(
        "apex-api",
    ))
}

/// Serve the OpenAPI 3.1 document built from the endpoint catalogue, so route
/// metadata, the catalogue and the served contract can never drift apart.
async fn openapi_json() -> Json<serde_json::Value> {
    Json(apex_api::routes::openapi_spec())
}

#[allow(dead_code)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
async fn api_features(State(state): State<AppState>) -> Json<serde_json::Value> {
    // Report the semantic dedup backend the worker actually constructed. A
    // missing row means no worker has recorded a backend; a failed read is an
    // explicit degraded state. Neither is reported as an assumed-good
    // pgvector state.
    let semantic_dedup = match state.store.get_semantic_dedup_state().await {
        Ok(Some(recorded)) => serde_json::json!({
            "status": recorded.status.as_str(),
            "backend": recorded.backend.as_str(),
            "detail": recorded.detail,
            "updated_at": recorded.updated_at,
        }),
        Ok(None) => serde_json::json!({
            "status": "degraded",
            "backend": "memory",
            "detail": "worker has not recorded a dedup backend yet",
        }),
        Err(error) => {
            tracing::error!(%error, "api_features: failed to read semantic dedup state");
            serde_json::json!({
                "status": "degraded",
                "backend": "memory",
                "detail": format!("semantic dedup state unavailable: {error}"),
            })
        }
    };

    Json(serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "llm_enabled": apex_api::BUILD_LLM_ENABLED,
        "semantic_dedup": semantic_dedup,
        // Backend actually serving the durable login throttle. `memory` means
        // no durable store was configured (tests/dev only).
        "login_throttle_backend": state.login_throttle.backend().as_str(),
        "login_throttle_durable": state.login_throttle.is_durable(),
        "features": [
            "warnings", "insights", "companies", "persons", "search",
            "graph", "recipes", "security", "admin", "weekly_memo",
            "dossiers", "competitors", "observations", "collaboration"
        ]
    }))
}

async fn api_docs() -> Html<String> {
    Html(format!(
        r#"<!DOCTYPE html>
<html>
<head><title>ApexIntel API</title></head>
<body>
<h1>ApexIntel API v{}</h1>
<p>See <a href="/api/openapi.json">OpenAPI spec</a></p>
</body>
</html>"#,
        env!("CARGO_PKG_VERSION")
    ))
}

/// Serve the PWA service worker at `/sw.js` (unauthenticated).
async fn sw_js() -> impl axum::response::IntoResponse {
    (
        [(
            axum::http::header::CONTENT_TYPE,
            "application/javascript; charset=utf-8",
        )],
        include_str!("../static/sw.js"),
    )
}

// ──────────────────────────────────────────────────────────────────────────────
// Admin Handlers
// ──────────────────────────────────────────────────────────────────────────────

async fn get_admin_crawl_status(
    State(state): State<AppState>,
) -> Result<Json<ApiResponse<AdminCrawlStatus>>, ApiError> {
    let status = state
        .store
        .get_admin_crawl_status()
        .await
        .map_err(|e| ApiError::internal(format!("Failed to get crawl status: {}", e)))?;
    Ok(Json(success(status)))
}

async fn get_admin_recipe_performance(
    State(state): State<AppState>,
) -> Result<Json<ApiResponse<AdminRecipePerformance>>, ApiError> {
    let perf = state
        .store
        .get_admin_recipe_performance()
        .await
        .map_err(|e| ApiError::internal(format!("Failed to get recipe performance: {}", e)))?;
    Ok(Json(success(perf)))
}

async fn get_admin_poi_coverage(
    State(state): State<AppState>,
) -> Result<Json<ApiResponse<AdminPoiCoverage>>, ApiError> {
    let coverage = state
        .store
        .get_admin_poi_coverage()
        .await
        .map_err(|e| ApiError::internal(format!("Failed to get POI coverage: {}", e)))?;
    Ok(Json(success(coverage)))
}

#[derive(Debug, Deserialize)]
struct TriggerScanRequest {
    source_id: String,
    #[cfg(feature = "llm")]
    force: Option<bool>,
}

#[derive(Debug, Serialize)]
struct TriggerScanResponse {
    job_id: String,
    queued: bool,
}

async fn post_trigger_scan(
    State(state): State<AppState>,
    Json(req): Json<TriggerScanRequest>,
) -> Result<Json<ApiResponse<TriggerScanResponse>>, ApiError> {
    use apex_store::postgres::is_valid_manual_trigger_kind;

    if !is_valid_manual_trigger_kind(&req.source_id) {
        return Err(ApiError::validation(
            "source_id",
            format!("Invalid source_id '{}'", req.source_id),
        ));
    }

    let job_id = state
        .store
        .queue_job_trigger(&req.source_id)
        .await
        .map_err(|e| ApiError::internal(format!("Failed to queue job trigger: {e}")))?;

    Ok(Json(success(TriggerScanResponse {
        job_id,
        queued: true,
    })))
}

async fn post_rebuild_autocomplete(
    State(state): State<AppState>,
) -> Result<Json<ApiResponse<serde_json::Value>>, ApiError> {
    let start = std::time::Instant::now();

    let path = state.config.search.index_path.join("autocomplete.fst");
    let store = &*state.store;

    let new_index = apex_store::autocomplete::build_from_database(store)
        .await
        .map_err(|e| ApiError::internal(format!("Failed to rebuild autocomplete index: {e}")))?;

    new_index
        .save(&path)
        .map_err(|e| ApiError::internal(format!("Failed to save autocomplete index: {e}")))?;

    let entry_count = new_index.len();
    // B305: recover from lock poisoning instead of panicking — one poisoned
    // write lock previously 500'd every subsequent suggest/rebuild request.
    *state
        .autocomplete_index
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = new_index;

    let duration_ms = start.elapsed().as_millis() as u64;
    tracing::info!(entries = %entry_count, duration_ms = %duration_ms, "autocomplete index rebuilt");

    let payload = serde_json::json!({
        "entries": entry_count,
        "duration_ms": duration_ms,
        "path": path.to_string_lossy().to_string(),
    });
    Ok(Json(success(payload)))
}

/// SSE endpoint handler for `/api/v1/events/stream`.
///
/// Registers the authenticated user for real-time event streaming.
/// Requires the SSE manager to be configured.
async fn alert_sse_handler(
    State(state): State<AppState>,
    Extension(auth): Extension<ApiAuthContext>,
    headers: axum::http::HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> axum::response::Response {
    let Some(ref sse_manager) = state.sse_manager else {
        return (
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "SSE not configured",
        )
            .into_response();
    };

    // B402: the connection is keyed by the authenticated principal (API key
    // owner or web-session user), not a random per-connection UUID, so alerts
    // addressed to real users can actually reach them. The cleanup guard
    // unregisters the sender when the stream is dropped so disconnected
    // clients no longer leak subscriber slots.
    let user_id = auth.user_id;
    let principal_id = principal_uuid_from_user_id(&user_id);

    // Replay/resync cursor: browsers send `Last-Event-ID` on native reconnect;
    // a query parameter covers clients whose reconnect is a fresh fetch.
    let last_event_id = headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
        .or_else(|| query.get("last_event_id").cloned())
        .or_else(|| query.get("lastEventId").cloned());
    let (tx, rx) = sse_manager
        .register_with_last_event_id(principal_id, last_event_id.as_deref())
        .await;

    let stream = apex_api::sse::SseManager::build_sse_stream_with_cleanup(
        rx,
        sse_manager.clone(),
        principal_id,
        tx,
        state.shutdown.clone(),
    );
    stream.into_response()
}

// ──────────────────────────────────────────────────────────────────────────────
// Pagination helpers
// ──────────────────────────────────────────────────────────────────────────────

#[allow(dead_code)]
fn pagination(page: Option<u32>, per_page: Option<u32>) -> (u32, u32, i64) {
    let page = page.unwrap_or(1).max(1);
    let per_page = per_page.unwrap_or(20).clamp(1, 100);
    let offset = (page - 1) * per_page;
    (page, per_page, offset as i64)
}

fn validate_pagination(
    page: Option<u32>,
    per_page: Option<u32>,
) -> Result<(u32, u32, i64), ApiError> {
    let page = page.unwrap_or(1);
    let per_page = per_page.unwrap_or(50);
    if page == 0 {
        return Err(ApiError::validation("page", "must be >= 1"));
    }
    if per_page == 0 || per_page > 100 {
        return Err(ApiError::validation(
            "per_page",
            "must be between 1 and 100",
        ));
    }
    let offset = ((page - 1) as i64) * (per_page as i64);
    Ok((page, per_page, offset))
}

fn clamp_page(page: u32, per_page: u32, total: u64) -> u32 {
    if total == 0 {
        return 1;
    }
    let max_page = ((total as f64 - 1.0) / (per_page as f64)).floor() as u32 + 1;
    page.min(max_page)
}

fn parse_csv_upper_strict(value: Option<&str>) -> Vec<String> {
    value
        .map(|v| {
            v.split(',')
                .filter_map(|s| {
                    let trimmed = s.trim().to_uppercase();
                    if trimmed.is_empty() {
                        None
                    } else {
                        Some(trimmed)
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

fn parse_csv_lower_strict(value: Option<&str>) -> Vec<String> {
    value
        .map(|v| {
            v.split(',')
                .filter_map(|s| {
                    let trimmed = s.trim().to_lowercase();
                    if trimmed.is_empty() {
                        None
                    } else {
                        Some(trimmed)
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

#[allow(dead_code)]
fn parse_csv_strict(value: Option<&str>) -> Vec<String> {
    value
        .map(|v| {
            v.split(',')
                .filter_map(|s| {
                    let trimmed = s.trim().to_string();
                    if trimmed.is_empty() {
                        None
                    } else {
                        Some(trimmed)
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

fn validate_json_depth(value: &serde_json::Value, max_depth: usize) -> Result<(), String> {
    fn depth(value: &serde_json::Value, current: usize) -> usize {
        match value {
            serde_json::Value::Object(map) => map
                .values()
                .map(|v| depth(v, current + 1))
                .max()
                .unwrap_or(current),
            serde_json::Value::Array(arr) => arr
                .iter()
                .map(|v| depth(v, current + 1))
                .max()
                .unwrap_or(current),
            _ => current,
        }
    }
    if depth(value, 0) > max_depth {
        Err(format!(
            "JSON depth {} exceeds maximum {}",
            depth(value, 0),
            max_depth
        ))
    } else {
        Ok(())
    }
}

fn log_latency(endpoint: &str, duration_ms: u64) {
    if duration_ms > 500 {
        tracing::warn!(endpoint, duration_ms, "slow request");
    } else {
        tracing::debug!(endpoint, duration_ms, "request completed");
    }
}

fn company_row_to_detail(
    row: CompanyRow,
    sites: Vec<SiteRow>,
    certs: Vec<CertificationRow>,
    persons: Vec<PersonRow>,
) -> CompanyDetail {
    let key_persons: Vec<CompanyKeyPerson> = persons
        .iter()
        .map(|p| CompanyKeyPerson {
            person_id: p.id.to_string(),
            name: p.name.clone(),
            role: p.current_role.clone().unwrap_or_else(|| {
                p.role_family
                    .clone()
                    .unwrap_or_else(|| "Unknown".to_string())
            }),
        })
        .collect();

    // Extract capabilities from industry_tags and site capabilities
    let mut capabilities: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    if let Some(ref tags) = row.industry_tags {
        for tag in tags {
            capabilities.insert(tag.clone());
        }
    }
    for site in &sites {
        if let Some(ref site_caps) = site.capabilities {
            for cap in site_caps {
                capabilities.insert(cap.clone());
            }
        }
    }

    // Extract certifications from the certs parameter
    let certifications: Vec<String> = certs.iter().map(|c| c.standard.clone()).collect();

    // Determine if competitor from the canonical column, with metadata fallback
    let is_competitor = row.is_competitor.unwrap_or_else(|| {
        row.metadata
            .as_ref()
            .and_then(|meta| meta.get("is_competitor"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    });

    // Extract city from sites
    let city = sites.iter().find_map(|s| s.city.clone());

    let now = Utc::now();

    // Build a baseline recent event from the company's own metadata
    let region_text = row.region.clone().unwrap_or_default();
    let type_text = row
        .company_type
        .clone()
        .unwrap_or_else(|| "Unknown type".to_string());
    let domain_clone = row.domain.clone();

    let recent_events = vec![CompanyEvent {
        event_type: "profile_loaded".to_string(),
        description: format!(
            "{} | {} | {} | Risk: {:.1}",
            region_text,
            type_text,
            row.employee_estimate
                .map(|e| format!("~{} employees", e))
                .unwrap_or_default(),
            row.risk_score.unwrap_or(0.0),
        ),
        date: row.updated_at.unwrap_or(now),
        source_url: domain_clone,
    }];

    // Extract community_badges and source_quality metrics from metadata JSON
    let metadata_ref = row.metadata.clone();
    let community_badges: Vec<String> = metadata_ref
        .as_ref()
        .and_then(|meta| meta.get("community_badges"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let source_entropy: Option<f64> = metadata_ref
        .as_ref()
        .and_then(|meta| meta.get("source_entropy"))
        .and_then(|v| v.as_f64());

    let source_quality_label: Option<String> = metadata_ref
        .as_ref()
        .and_then(|meta| meta.get("source_quality_label"))
        .and_then(|v| v.as_str().map(String::from));

    CompanyDetail {
        id: row.id.to_string(),
        name: row.name,
        legal_name: row.legal_name,
        region: row.region.unwrap_or_default(),
        country: row.country_code.unwrap_or_default(),
        city,
        website: row.domain,
        entity_type: row.company_type.unwrap_or_else(|| "unknown".to_string()),
        is_competitor,
        threat_score: row.threat_score,
        overlap_score: row.overlap_score,
        capabilities: capabilities.into_iter().collect(),
        certifications,
        sites: sites
            .into_iter()
            .map(|s| CompanySite {
                name: s.name,
                location: [s.address, s.city, s.region, s.country_code]
                    .into_iter()
                    .flatten()
                    .filter(|v| !v.trim().is_empty())
                    .collect::<Vec<_>>()
                    .join(", "),
                site_type: s.site_type.unwrap_or_else(|| "site".to_string()),
            })
            .collect(),
        key_persons,
        recent_events,
        community_badges,
        source_entropy,
        source_quality_label,
        // Unknown row time stays null rather than a fabricated "now".
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

fn person_row_to_detail(
    row: PersonRow,
    artifacts: Vec<ArtifactRow>,
    organization: Option<String>,
) -> PersonDetail {
    let mut name_alt = Vec::new();
    if let Some(ref ar) = row.name_ar {
        name_alt.push(ar.clone());
    }
    if let Some(ref fr) = row.name_fr {
        name_alt.push(fr.clone());
    }

    // Canonical intelligence view (audit P2-18): priority from the stored
    // vector, influence measured, completeness from the actual profile fields.
    let priority_vector = row
        .priority_vector
        .as_ref()
        .and_then(|v| serde_json::from_value::<StoredPriorityVector>(v.clone()).ok());
    let view = apex_api::person_intelligence::PersonIntelligenceView::from_measurements(
        row.priority_vector.as_ref(),
        row.influence_score,
        row.metadata
            .as_ref()
            .and_then(|meta| meta.get("engagement_status"))
            .and_then(serde_json::Value::as_str),
        None,
        &[
            row.public_bio.is_some(),
            row.public_email.is_some(),
            row.primary_org_id.is_some(),
            !row.region.as_deref().unwrap_or("").is_empty(),
            row.current_role.is_some(),
            row.influence_score.is_some(),
            row.priority_vector.is_some(),
            !row.trigger_topics.as_deref().unwrap_or(&[]).is_empty(),
        ],
    );
    let priority_score = view.priority_score;
    // The API contract for `priority` is the A/B/C band (same as the list
    // model); the verbal tier is exposed separately.
    let priority = view.priority_band.clone();
    let priority_tier = view.priority_tier_label.clone();
    let influence_score = view.influence_score;
    let influence_tier = view.influence_tier_label;

    // Evidence-backed contacts and timeline from artifacts (previously loaded
    // and discarded). Nothing is inferred beyond what an artifact states.
    let linkedin = artifacts
        .iter()
        .find(|artifact| artifact.url.contains("linkedin.com"))
        .map(|artifact| artifact.url.clone());
    let phone = artifacts.iter().find_map(extract_phone_from_artifact);
    let timeline: Vec<PersonEvent> = artifacts
        .iter()
        .map(|artifact| PersonEvent {
            event_type: artifact.artifact_type.clone(),
            description: artifact
                .title
                .clone()
                .or_else(|| artifact.content_summary.clone())
                .unwrap_or_else(|| artifact.url.clone()),
            date: artifact.ts_utc,
            source_url: Some(artifact.url.clone()),
        })
        .collect();
    let mut tags: Vec<String> = Vec::new();
    for artifact in &artifacts {
        if let Some(topics) = &artifact.topics {
            tags.extend(topics.iter().cloned());
        }
        if let Some(phrases) = &artifact.key_phrases {
            tags.extend(phrases.iter().cloned());
        }
    }
    tags.sort();
    tags.dedup();

    PersonDetail {
        id: row.id.to_string(),
        name: row.name,
        name_alt,
        role: row.current_role.clone().unwrap_or_default(),
        role_family: row.role_family.clone().unwrap_or_default(),
        organization,
        org_id: row.primary_org_id.map(|id| id.to_string()),
        region: row.region.clone().unwrap_or_default(),
        country: row.country_code.clone().unwrap_or_default(),
        bio: row.public_bio,
        email: row.public_email,
        phone,
        linkedin,
        priority_score,
        influence_score,
        priority,
        priority_tier,
        priority_vector,
        influence_tier,
        // Engagement readiness is not computed by this endpoint yet: unknown,
        // not zero.
        engagement_status: view.engagement_status,
        engagement_readiness: view.engagement_readiness,
        data_completeness: view.data_completeness,
        tags,
        trigger_topics: row.trigger_topics.unwrap_or_default(),
        decision_style: row.decision_style,
        risk_tolerance: row.risk_tolerance,
        change_appetite: row.change_appetite,
        communication_style: row.communication_style,
        decision_mode: row.decision_mode,
        preferred_proof_type: row.preferred_proof_type,
        pain_index: row.pain_index,
        change_risk: row.change_risk,
        role_drift_score: row.role_drift_score,
        buying_center_role: classify_buying_center_role(
            row.current_role.as_deref().unwrap_or(""),
            row.role_family.as_deref().unwrap_or(""),
        )
        .to_string(),
        affiliations: Vec::new(),
        timeline,
        role_history: Vec::new(),
        peers: Vec::new(),
        warning_count: 0,
        insight_count: 0,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

/// Extract a phone number only when an artifact's text states one.
fn extract_phone_from_artifact(artifact: &ArtifactRow) -> Option<String> {
    let haystack = artifact
        .content_summary
        .as_deref()
        .unwrap_or(artifact.title.as_deref().unwrap_or(""));
    let mut current = String::new();
    let mut candidates: Vec<String> = Vec::new();
    let mut consider = |candidate: &str| {
        let candidate = candidate.trim();
        let digits = candidate.chars().filter(char::is_ascii_digit).count();
        // 7+ digits also matches dates like "2024-03-15"; a phone number has
        // 9–15 digits and is not the dddd-dd-dd shape.
        let parts: Vec<&str> = candidate.split('-').collect();
        let is_date = parts.len() == 3 && parts.iter().map(|part| part.len()).eq([4, 2, 2]);
        if (9..=15).contains(&digits) && !is_date {
            candidates.push(candidate.to_string());
        }
    };
    for ch in haystack.chars() {
        if ch.is_ascii_digit() || ch == '+' || ch == '-' || ch == ' ' {
            current.push(ch);
        } else {
            consider(&current);
            current.clear();
        }
    }
    consider(&current);
    candidates.into_iter().next()
}

/// Classify a person's buying-center role from whole tokens in the title.
/// Substring matching misclassified every C-suite title as "unknown" and
/// matched fragments like "vp" inside unrelated words.
fn classify_buying_center_role(title: &str, role_family: &str) -> &'static str {
    let tokens: Vec<String> = title
        .to_lowercase()
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_string)
        .collect();
    let has_token = |candidates: &[&str]| {
        tokens
            .iter()
            .any(|token| candidates.contains(&token.as_str()))
    };

    if has_token(&[
        "vp",
        "svp",
        "evp",
        "ceo",
        "cto",
        "cfo",
        "coo",
        "cio",
        "ciso",
        "cmo",
        "chro",
        "chief",
        "president",
        "founder",
        "owner",
        "director",
        "head",
    ]) {
        return "decision_maker";
    }
    if has_token(&["manager", "lead", "leader"]) {
        return "influencer";
    }
    if has_token(&["buyer", "procurement", "purchasing", "sourcing"]) {
        return "purchasing";
    }
    if has_token(&[
        "engineer",
        "analyst",
        "specialist",
        "technician",
        "architect",
    ]) {
        return "technical";
    }
    if role_family.to_lowercase().contains("user") {
        return "user";
    }
    "unknown"
}

fn edge_row_to_graph_edge(row: &EdgeRow) -> GraphEdge {
    let weight = row.weight.unwrap_or(1.0);
    GraphEdge {
        source: row.source_id.to_string(),
        target: row.target_id.to_string(),
        edge_type: row.edge_type.clone(),
        weight,
        label: Some(format!("{} → {}", row.source_type, row.target_type)),
        confidence: Some(row.confidence.unwrap_or(weight).clamp(0.0, 1.0)),
        first_seen: row.first_seen.map(|ts| ts.to_rfc3339()),
        last_confirmed: row.last_seen.map(|ts| ts.to_rfc3339()),
        evidence_count: Some(
            row.evidence_ids
                .as_ref()
                .map(|ids| ids.len() as i64)
                .unwrap_or(0),
        ),
        source_name: apex_api::routes::graph::edge_source_name(row.metadata.as_ref()),
    }
}

fn compute_edge_type_counts(rows: &[EdgeRow]) -> Vec<EdgeTypeCount> {
    use std::collections::HashMap;
    let mut counts: HashMap<String, u64> = HashMap::new();
    for row in rows {
        *counts.entry(row.edge_type.clone()).or_insert(0) += 1;
    }
    counts
        .into_iter()
        .map(|(edge_type, count)| EdgeTypeCount { edge_type, count })
        .collect()
}

fn recipe_stat_to_list_item(stat: &RecipeStatRow) -> RecipeListItem {
    RecipeListItem {
        id: stat.recipe_code.clone(),
        name: stat.recipe_code.clone(),
        description: String::new(),
        status: parse_recipe_status(&stat.status).unwrap_or(RecipeStatus::Staging),
        region: None,
        precision: stat.precision_score,
        promotion_evidence_score: 0.0,
        false_positive_rate: stat.false_positive_rate,
        fired_count: stat.fired_count as u32,
        last_fired: stat.last_fired,
        created_at: stat.first_fired.unwrap_or(Utc::now()),
        updated_at: stat.last_fired.unwrap_or(Utc::now()),
    }
}

fn parse_recipe_status(raw: &str) -> Option<RecipeStatus> {
    match raw.to_lowercase().as_str() {
        "production" => Some(RecipeStatus::Production),
        "staging" => Some(RecipeStatus::Staging),
        "deprecated" => Some(RecipeStatus::Deprecated),
        _ => None,
    }
}

pub(crate) fn map_warning_sort(sort: WarningSortField) -> WarningOrderBy {
    match sort {
        WarningSortField::CreatedAt => WarningOrderBy::CreatedAt,
        WarningSortField::Severity => WarningOrderBy::Severity,
        WarningSortField::Type => WarningOrderBy::WarningType,
    }
}

pub(crate) fn map_company_sort(sort: CompanySortField) -> apex_store::postgres::CompanyOrderBy {
    match sort {
        CompanySortField::Name => apex_store::postgres::CompanyOrderBy::Name,
        CompanySortField::ThreatScore => apex_store::postgres::CompanyOrderBy::ThreatScore,
        CompanySortField::UpdatedAt => apex_store::postgres::CompanyOrderBy::UpdatedAt,
        CompanySortField::Region => apex_store::postgres::CompanyOrderBy::Region,
    }
}

pub(crate) fn map_person_sort(sort: PersonSortField) -> PersonOrderBy {
    match sort {
        PersonSortField::Name => PersonOrderBy::Name,
        PersonSortField::Priority => PersonOrderBy::Priority,
        PersonSortField::Region => PersonOrderBy::Region,
        PersonSortField::UpdatedAt => PersonOrderBy::UpdatedAt,
    }
}

pub(crate) fn validate_region_codes(values: &[String]) -> Result<(), ApiError> {
    for value in values {
        if value.len() != 2 {
            return Err(ApiError::validation(
                "region",
                format!("Invalid region code '{}'", value),
            ));
        }
    }
    Ok(())
}

pub(crate) fn validate_severity_codes(values: &[String]) -> Result<(), ApiError> {
    let valid = ["low", "medium", "high", "critical"];
    for value in values {
        if !valid.contains(&value.to_lowercase().as_str()) {
            return Err(ApiError::validation(
                "severity",
                format!("Invalid severity '{}'", value),
            ));
        }
    }
    Ok(())
}

pub(crate) fn validate_warning_type_codes(values: &[String]) -> Result<(), ApiError> {
    for value in values {
        if value.is_empty() {
            return Err(ApiError::validation("warning_type", "cannot be empty"));
        }
    }
    Ok(())
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
pub(crate) fn parse_date_start(value: &Option<String>) -> Result<Option<DateTime<Utc>>, String> {
    match value {
        None => Ok(None),
        Some(s) if s.is_empty() => Ok(None),
        Some(s) => {
            let parsed = NaiveDate::parse_from_str(s, "%Y-%m-%d")
                .map_err(|_| format!("Invalid date format: {}", s))?;
            let dt = parsed.and_hms_opt(0, 0, 0).unwrap();
            Ok(Some(Utc.from_utc_datetime(&dt)))
        }
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
fn parse_query_date(value: &Option<String>, name: &str) -> Result<Option<DateTime<Utc>>, ApiError> {
    match value {
        None => Ok(None),
        Some(s) if s.is_empty() => Ok(None),
        Some(s) => {
            let parsed = NaiveDate::parse_from_str(s, "%Y-%m-%d")
                .map_err(|_| ApiError::validation(name, format!("Invalid date format: {}", s)))?;
            let dt = parsed.and_hms_opt(0, 0, 0).unwrap();
            Ok(Some(Utc.from_utc_datetime(&dt)))
        }
    }
}

#[allow(dead_code)]
type DateRangeResult = (Option<DateTime<Utc>>, Option<DateTime<Utc>>);

#[allow(dead_code)]
pub(crate) fn validate_date_range(
    from: &Option<String>,
    to: &Option<String>,
) -> Result<DateRangeResult, ApiError> {
    let from_dt = parse_query_date(from, "date_from")?;
    let to_dt = parse_query_date(to, "date_to")?;
    if let (Some(from), Some(to)) = (from_dt, to_dt) {
        if from > to {
            return Err(ApiError::validation(
                "date_range",
                "date_from must be before date_to",
            ));
        }
    }
    Ok((from_dt, to_dt))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pagination_defaults() {
        let (page, per_page, offset) = pagination(None, None);
        assert_eq!(page, 1);
        assert_eq!(per_page, 20);
        assert_eq!(offset, 0);
    }

    #[test]
    fn pagination_custom() {
        let (page, per_page, offset) = pagination(Some(3), Some(50));
        assert_eq!(page, 3);
        assert_eq!(per_page, 50);
        assert_eq!(offset, 100);
    }

    #[test]
    fn clamp_page_works() {
        assert_eq!(clamp_page(1, 20, 100), 1);
        assert_eq!(clamp_page(10, 20, 100), 5);
        assert_eq!(clamp_page(100, 20, 0), 1);
    }

    #[test]
    fn parse_csv_upper_strict_works() {
        assert_eq!(parse_csv_upper_strict(Some("us,EMEA")), vec!["US", "EMEA"]);
        assert_eq!(parse_csv_upper_strict(None), Vec::<String>::new());
        assert_eq!(
            parse_csv_upper_strict(Some("  us  ,  , EMEA")),
            vec!["US", "EMEA"]
        );
    }

    #[test]
    fn parse_recipe_status_works() {
        assert_eq!(
            parse_recipe_status("production"),
            Some(RecipeStatus::Production)
        );
        assert_eq!(parse_recipe_status("STAGING"), Some(RecipeStatus::Staging));
        assert_eq!(parse_recipe_status("unknown"), None);
    }

    fn make_recipe_stat(status: &str, fired_count: i64) -> RecipeStatRow {
        RecipeStatRow {
            recipe_code: "test-recipe".to_string(),
            status: status.to_string(),
            precision_score: Some(0.85),
            avg_model_confidence: Some(0.8),
            false_positive_rate: Some(0.05),
            fired_count,
            last_fired: None,
            first_fired: None,
            active_count: 0,
        }
    }

    #[test]
    fn recipe_status_reflects_explicit_store_status() {
        let stat = make_recipe_stat("production", 100);
        let item = recipe_stat_to_list_item(&stat);
        assert_eq!(item.status, RecipeStatus::Production);
    }

    #[test]
    fn recipe_status_does_not_change_when_warning_count_fluctuates() {
        let stat1 = make_recipe_stat("production", 50);
        let stat2 = make_recipe_stat("production", 500);
        let item1 = recipe_stat_to_list_item(&stat1);
        let item2 = recipe_stat_to_list_item(&stat2);
        assert_eq!(item1.status, item2.status);
    }

    #[test]
    fn classify_buying_center_role_works() {
        assert_eq!(
            classify_buying_center_role("VP of Sales", "executive"),
            "decision_maker"
        );
        assert_eq!(
            classify_buying_center_role("IT Manager", "technical"),
            "influencer"
        );
        assert_eq!(
            classify_buying_center_role("Procurement Specialist", "purchasing"),
            "purchasing"
        );
        assert_eq!(classify_buying_center_role("End User", "user"), "user");

        // C-suite must not classify as unknown.
        for title in [
            "Chief Executive Officer",
            "CTO",
            "CFO",
            "President",
            "Founder",
            "Owner",
        ] {
            assert_eq!(
                classify_buying_center_role(title, "executive"),
                "decision_maker",
                "{title} is a decision maker"
            );
        }

        // Whole-token matching: "lead" as a word is an influencer role, and a
        // title fragment must not trigger "vp".
        assert_eq!(
            classify_buying_center_role("Lead Developer", "technical"),
            "influencer"
        );
    }

    #[test]
    fn phone_extraction_does_not_surface_dates() {
        let artifact = |summary: &str| ArtifactRow {
            id: Uuid::new_v4(),
            person_id: None,
            artifact_type: "note".to_string(),
            title: None,
            content_summary: Some(summary.to_string()),
            url: "https://example.com".to_string(),
            source_domain: None,
            language: None,
            topics: None,
            sentiment_score: None,
            key_phrases: None,
            ts_utc: Utc::now(),
            provenance: serde_json::json!({}),
            metadata: None,
            created_at: None,
        };
        assert_eq!(
            extract_phone_from_artifact(&artifact("Report dated 2024-03-15.")),
            None
        );
        assert_eq!(
            extract_phone_from_artifact(&artifact("Call +212 555 123 456 today")),
            Some("+212 555 123 456".to_string())
        );
    }
}
