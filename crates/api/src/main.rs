use anyhow::Result;
use apex_api::auth::{self, ApiKey, ApiRole, AuthResult, PermissionLevel};
use apex_api::filters::{validate_search_text, RegionFilter, SeverityFilter, WarningTypeFilter};
use apex_api::responses::{
    aggregate_health, error_response, success_with_meta, ApiError, ApiResponse, ComponentHealth, ErrorCode, HealthResponse,
    HealthStatus, PagedResponse, ResponseMeta,
};
use apex_api::routes;
use apex_api::routes::companies::{
    validate_company_id, CompanyDetail, CompanyEvent, CompanyKeyPerson, CompanyListItem, CompanySite, CompanySortField,
    ListCompaniesQuery,
};
use apex_api::routes::graph::{GraphOverviewWithEdges, GraphEdge, GraphNodeLabel, EdgeTypeCount};
use apex_api::routes::insights::{InsightResponse, ListInsightsQuery};
use apex_api::routes::llm::{
    ExtractEntitiesRequest, ExtractEntitiesResponse, GenerateMemoRequest, GenerateMemoResponse, GenerateRecipeRequest,
    GenerateRecipeResponse, SynthesizePoiRequest, SynthesizePoiResponse,
};
#[cfg(feature = "llm")]
use apex_api::routes::llm::{ExtractedEntity, LlmTask, MemoSection};
use apex_api::routes::persons::{
    validate_person_id, Affiliation, ListPersonsQuery, PersonDetail, PersonEvent, PersonListItem,
    PersonSortField, PriorityVector, RoleHistoryEntry, PeerSummary,
    priority_tier,
};
use apex_api::routes::preferences::{
    NotificationPrefs, PreferencesResponse, UpdatePreferencesRequest, UserPreferences,
};
use apex_api::routes::recipes::{
    sort_recipes, ListRecipesQuery, RecipeListItem, RecipeSortField, RecipeStatus,
};
use apex_api::routes::search::{
    build_facets, highlight_snippet, sort_by_score, tokenize_query, validate_search_query, SearchHit, SearchQuery,
    SearchResponse,
};
use apex_api::routes::security::{
    dns_score, DnsPostureItem, DnsPostureOverview, KevItem, LookalikeDomainItem, SecuritySummary,
};
use apex_api::routes::warnings::{
    validate_acknowledge, validate_warning_id, AcknowledgeRequest, ListWarningsQuery, SortDirection, WarningResponse,
    WarningSortField,
};
use apex_api::routes::ws::{to_ws_event, WarningEvent};
use apex_core::config::AppConfig;
use apex_core::validation::clamp_ratio;
use apex_store::postgres::{
    AdminCrawlStatus, AdminPoiCoverage, AdminRecipePerformance, ArtifactRow, CapabilityRow,
    CertificationRow, CompanyChangeRow, CompanyDossier, CompanyListFilters, CompanyOrderBy,
    CompanyRow, CompetitorChange, DashboardStats, DossierEntryRow, EdgeRow,
    InsightListFilters, InsightRow, LogisticsNodeRow, ObservationRow, PersonChangeRow,
    PersonDossier, PersonEngagement, PersonListFilters, PersonListRow, PersonOrderBy,
    PersonRow, PgStore, ProductFamilyRow, RecipeStatRow, RegulationRow, RoleHistoryRow,
    SiteRow, WarningListFilters, WarningOrderBy, WarningRow, WeeklyMemo,
};
use apex_store::tantivy_index::SearchIndex;
use axum::{
    extract::{Path, Query, State},
    http::{header, Method, StatusCode},
    middleware,
    response::IntoResponse,
    routing::{get, post},
    Extension, Json, Router,
};
use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use lazy_static::lazy_static;
use prometheus::{Encoder, GaugeVec, HistogramOpts, HistogramVec, IntCounterVec, Opts, Registry, TextEncoder};
use serde::{Deserialize, Serialize};
#[cfg(feature = "llm")]
use serde_json::Value as JsonValue;
use std::{collections::{BTreeSet, HashMap}, path::Path as FsPath, sync::Arc, sync::OnceLock, time::Instant};
use redis::AsyncCommands;
use tower_http::{cors::CorsLayer, services::ServeDir, trace::TraceLayer};
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

use apex_api::middleware::session::require_session as require_web_session;
use apex_api::web;

#[cfg(feature = "llm")]
use apex_llm::{validators, LlmClient, LlmProvider, ModelConfig, OpenAiCompatibleClient};

static STARTED_AT: OnceLock<DateTime<Utc>> = OnceLock::new();
const MAX_JSON_DEPTH: usize = 32;

// ─── Prometheus Metrics ─────────────────────────────────────────────────────

lazy_static! {
    static ref METRICS_REGISTRY: Registry = Registry::new();
    
    static ref HTTP_REQUESTS_TOTAL: IntCounterVec = IntCounterVec::new(
        Opts::new("http_requests_total", "Total number of HTTP requests"),
        &["method", "path", "status"]
    ).unwrap();
    
    static ref HTTP_REQUEST_DURATION_SECONDS: HistogramVec = HistogramVec::new(
        HistogramOpts::new("http_request_duration_seconds", "HTTP request duration in seconds")
            .buckets(vec![0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0]),
        &["method", "path"]
    ).unwrap();
    
    static ref WARNINGS_TOTAL: GaugeVec = GaugeVec::new(
        Opts::new("apexintel_warnings_total", "Total warnings by severity"),
        &["severity"]
    ).unwrap();
    
    static ref INSIGHTS_TOTAL: GaugeVec = GaugeVec::new(
        Opts::new("apexintel_insights_total", "Total insights by category"),
        &["category"]
    ).unwrap();
    
    static ref COMPANIES_TRACKED: prometheus::Gauge = prometheus::Gauge::new(
        "apexintel_companies_tracked", "Number of companies tracked"
    ).unwrap();
    
    static ref PERSONS_TRACKED: prometheus::Gauge = prometheus::Gauge::new(
        "apexintel_persons_tracked", "Number of persons of interest tracked"
    ).unwrap();
    
    static ref RECIPES_ACTIVE: prometheus::Gauge = prometheus::Gauge::new(
        "apexintel_recipes_active", "Number of active recipes"
    ).unwrap();
    
    static ref CRAWL_QUEUE_SIZE: prometheus::Gauge = prometheus::Gauge::new(
        "apexintel_crawl_queue_size", "Current crawl queue size"
    ).unwrap();
    
    static ref RECIPE_FIRINGS_TOTAL: IntCounterVec = IntCounterVec::new(
        Opts::new("apexintel_recipe_firings_total", "Total recipe firings"),
        &["recipe_id", "outcome"]
    ).unwrap();
}

fn init_metrics() {
    METRICS_REGISTRY.register(Box::new(HTTP_REQUESTS_TOTAL.clone())).ok();
    METRICS_REGISTRY.register(Box::new(HTTP_REQUEST_DURATION_SECONDS.clone())).ok();
    METRICS_REGISTRY.register(Box::new(WARNINGS_TOTAL.clone())).ok();
    METRICS_REGISTRY.register(Box::new(INSIGHTS_TOTAL.clone())).ok();
    METRICS_REGISTRY.register(Box::new(COMPANIES_TRACKED.clone())).ok();
    METRICS_REGISTRY.register(Box::new(PERSONS_TRACKED.clone())).ok();
    METRICS_REGISTRY.register(Box::new(RECIPES_ACTIVE.clone())).ok();
    METRICS_REGISTRY.register(Box::new(CRAWL_QUEUE_SIZE.clone())).ok();
    METRICS_REGISTRY.register(Box::new(RECIPE_FIRINGS_TOTAL.clone())).ok();
}

#[derive(Clone)]
struct AppState {
    store: Arc<PgStore>,
    search_index: Arc<SearchIndex>,
    /// Read-only API key map, populated once at startup (B281).
    ///
    /// `Arc<HashMap>` is safe for concurrent reads: `Arc` provides shared
    /// ownership and `HashMap` is `Sync` when its key/value types are `Sync`.
    /// Writes are intentionally disallowed after init — runtime key rotation
    /// requires a rolling restart.  If hot-reload is needed in future,
    /// upgrade to `Arc<RwLock<HashMap<String, ApiKey>>>` and install a
    /// SIGHUP handler that calls `write().unwrap().insert(...)` after verifying
    /// the new key format.
    api_keys: Arc<HashMap<String, ApiKey>>,
    /// Redis connection manager for rate limiting (INCR + EXPIRE per key/minute).
    /// None if REDIS_URL is not configured — falls back to no enforcement.
    redis: Option<redis::aio::ConnectionManager>,
    #[cfg(feature = "llm")]
    llm: Option<LlmRuntime>,
}

#[derive(Debug, Clone, Copy)]
struct RateLimitInfo {
    limit_per_min: u32,
    /// Actual remaining requests this minute (computed by Redis INCR).
    /// None if Redis is unavailable — headers will report limit as remaining.
    remaining: Option<u32>,
    /// Unix timestamp of the next minute boundary (for X-RateLimit-Reset).
    reset_at_unix: i64,
}

#[derive(Debug, Clone)]
struct ApiAuthContext {
    key_id: String,
}

#[cfg(feature = "llm")]
#[derive(Clone)]
struct LlmRuntime {
    primary: ModelConfig,
    lightweight: ModelConfig,
}

#[tokio::main]
async fn main() -> Result<()> {
    let log_level = std::env::var("API_LOG_LEVEL").unwrap_or_else(|_| "info".to_string());
    let log_filter = std::env::var("RUST_LOG")
        .unwrap_or_else(|_| format!("{},apex_api={}", log_level, log_level));
    tracing_subscriber::fmt()
        .with_target(false)
        .with_env_filter(EnvFilter::new(log_filter))
        .init();
    STARTED_AT.get_or_init(Utc::now);
    init_metrics();

    let state = build_state().await?;

    // CORS: restrict to allowed origins (env var or default to localhost:3000 for dev)
    let allowed_origin = std::env::var("CORS_ORIGIN")
        .unwrap_or_else(|_| "http://localhost:3000".to_string());
    let cors = CorsLayer::new()
        .allow_origin(allowed_origin.parse::<axum::http::HeaderValue>().unwrap_or_else(|_| {
            "http://localhost:3000".parse().unwrap()
        }))
        .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE])
        .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION]);

    // Public routes (no auth required)
    let public = Router::new()
        .route("/api/health", get(health))
        .route("/api/health/live", get(health_live))
        .route("/api/health/ready", get(health_ready))
        .route("/api/endpoints", get(endpoints))
        .route("/metrics", get(metrics));

    // Protected routes (auth required)
    let protected = Router::new()
        .route("/api/warnings", get(list_warnings).delete(delete_all_warnings))
        .route("/api/warnings/bulk-delete", post(delete_warnings_bulk))
        .route("/api/warnings/:id", get(get_warning_detail).delete(delete_warning))
        .route("/api/warnings/:id/acknowledge", post(acknowledge_warning))
        .route("/api/insights", get(list_insights))
        .route("/api/insights/export", get(export_insights_csv))
        .route("/api/insights/weekly-memo", get(get_weekly_memo))
        .route("/api/insights/:id", get(get_insight_detail))
        .route("/api/insights/:id/analyze", post(analyze_insight))
        .route("/api/insights/:id/bookmark", post(bookmark_insight).delete(unbookmark_insight))
        .route("/api/warnings/:id/analyze", post(analyze_warning))
        .route("/api/memos", get(list_memos))
        .route("/api/companies", get(list_companies))
        .route("/api/companies/export", get(export_companies_csv))
        .route("/api/companies/:id", get(get_company_detail))
        .route("/api/companies/:id/dossier", get(get_company_dossier))
        .route("/api/persons", get(list_persons))
        .route("/api/persons/export", get(export_persons_csv))
        .route("/api/persons/:id", get(get_person_detail))
        .route("/api/persons/:id/dossier", get(get_person_dossier))
        .route("/api/persons/:id/engagement", get(get_person_engagement))
        .route("/api/persons/:id/role-history", get(get_person_role_history))
        .route("/api/persons/:id/changes", get(get_person_changes_api))
        .route("/api/persons/:id/dossier-entries", get(get_person_dossier_entries))
        .route("/api/companies/:id/changes", get(get_company_changes_api))
        .route("/api/companies/:id/dossier-entries", get(get_company_dossier_entries))
        .route("/api/dossier-entries/:id/verify", post(verify_dossier_entry))
        .route("/api/dossier-entries/:id/history", get(get_dossier_entry_history))
        .route("/api/competitors", get(list_competitors))
        .route("/api/competitors/changes", get(list_all_competitor_changes))
        .route("/api/competitors/:id/changes", get(get_competitor_changes))
        .route("/api/search", get(search))
        .route("/api/graph", get(list_graph))
        .route("/api/graph/neighborhood/:id", get(get_graph_neighborhood))
        .route("/api/graph/path/:from/:to", get(get_graph_path))
        .route("/api/recipes", get(list_recipes))
        .route("/api/recipes/staging", get(list_staging_recipes))
        .route("/api/recipes/:id/promote", post(promote_recipe))
        .route("/api/recipes/:id/deprecate", post(deprecate_recipe))
        .route("/api/preferences", get(get_preferences).post(update_preferences))
        .route("/api/security", get(list_security))
        .route("/api/security/dns-posture", get(get_dns_posture))
        .route("/api/security/lookalike-domains", get(get_lookalike_domains))
        .route("/api/security/kev-relevance", get(get_kev_relevance))
        .route("/api/sites", get(list_sites))
        .route("/api/capabilities", get(list_capabilities))
        .route("/api/certifications", get(list_certifications_all))
        .route("/api/observations", get(list_observations))
        .route("/api/product-families", get(list_product_families))
        .route("/api/logistics-nodes", get(list_logistics_nodes))
        .route("/api/regulations", get(list_regulations))
        .route("/api/poi-artifacts", get(list_poi_artifacts))
        .route("/api/dashboard", get(get_dashboard))
        .route("/api/admin/crawl-status", get(get_admin_crawl_status))
        .route("/api/admin/recipe-performance", get(get_admin_recipe_performance))
        .route("/api/admin/poi-coverage", get(get_admin_poi_coverage))
        .route("/api/admin/trigger-scan", post(post_trigger_scan))
        .route("/api/llm/extract-entities", post(llm_extract_entities))
        .route("/api/llm/generate-recipe", post(llm_generate_recipe))
        .route("/api/llm/synthesize-poi", post(llm_synthesize_poi))
        .route("/api/llm/generate-memo", post(llm_generate_memo))
        .route_layer(middleware::from_fn_with_state(state.clone(), require_auth));

    // ── HTML page routes (Askama + HTMX) ────────────────────────────────────
    let html_public = Router::new()
        .route("/login", get(web::auth::login_page).post(web::auth::login_submit))
        .route("/logout", post(web::auth::logout))
        .layer(Extension(state.store.clone()));

    let html_protected = Router::new()
        .route("/", get(web::dashboard::dashboard))
        .route("/warnings", get(web::warnings::list_warnings))
        .route("/warnings/:id", get(web::warnings::get_warning))
        .route("/warnings/:id/acknowledge", post(web::warnings::acknowledge_warning_html))
        .route("/warnings/:id/analyze", post(web::warnings::analyze_warning_html))
        .route("/insights", get(web::insights::list_insights))
        .route("/insights/:id", get(web::insights::get_insight))
        .route("/insights/:id/bookmark", post(web::insights::bookmark_insight_html))
        .route("/insights/:id/analyze", post(web::insights::analyze_insight_html))
        .route("/companies", get(web::companies::list_companies))
        .route("/companies/:id", get(web::companies::get_company))
        .route("/companies/:id/changes", get(web::companies::company_changes_tab))
        .route("/companies/:id/dossier", get(web::companies::company_dossier_tab))
        .route("/persons", get(web::persons::list_persons))
        .route("/persons/:id", get(web::persons::get_person))
        .route("/competitors", get(web::competitors::list_competitors))
        .route("/memos", get(web::memos::list_memos))
        .route("/search", get(web::search::search_page))
        .route("/graph", get(web::graph::graph_page))
        .route("/security", get(web::security::security_page))
        .route("/security/trigger-scan", post(web::security::trigger_scan_html))
        .route("/settings", get(web::settings::settings_page).post(web::settings::save_settings))
        .route("/admin", get(web::admin::admin_page))
        .route("/recipes", get(web::recipes::list_recipes))
        .route("/recipes/new", get(web::recipes::new_recipe))
        .route("/api/warnings/unread-count", get(web::warnings::unread_count))
        .route_layer(middleware::from_fn(require_web_session))
        .layer(Extension(state.store.clone()))
        .layer(Extension(state.search_index.clone()));

    // Resolve path to static directory relative to the working directory.
    // In production the binary runs from /opt/apexintel/bin with static
    // assets at /opt/apexintel/static; in dev, from the crate root.
    let static_dir = if std::path::Path::new("static").is_dir() {
        "static"
    } else if std::path::Path::new("crates/api/static").is_dir() {
        "crates/api/static"
    } else {
        "static"
    };

    let app = Router::new()
        .merge(html_public)
        .merge(html_protected)
        .nest_service("/static", ServeDir::new(static_dir))
        .merge(public)
        .merge(protected)
        .route("/ws/warnings", get(warnings_ws))
        .fallback(get(web::errors::not_found))
        .layer(middleware::from_fn(add_rate_limit_headers))
        .layer(cors)
        .layer(axum::extract::DefaultBodyLimit::max(64 * 1024)) // 64 KiB
        .layer(TraceLayer::new_for_http().make_span_with(|request: &axum::http::Request<_>| {
            tracing::info_span!(
                "http_request",
                method = %request.method(),
                path = %request.uri().path(),
                request_id = %Uuid::new_v4()
            )
        }))
        .with_state(state);

    let host = std::env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
    let port = std::env::var("PORT").unwrap_or_else(|_| "8080".to_string());
    let bind_addr = format!("{}:{}", host, port);
    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
    tracing::info!("ApexIntel API listening on {}", bind_addr);
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
            tracing::info!("shutdown signal received, draining in-flight requests");
        })
        .await?;
    Ok(())
}

// ─── Auth middleware ────────────────────────────────────────────────────────

async fn require_auth(
    State(state): State<AppState>,
    request: axum::extract::Request,
    next: middleware::Next,
) -> axum::response::Response {
    let auth_header = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());

    let token = match auth_header.and_then(auth::extract_bearer_token) {
        Some(t) => t.to_string(),
        None => {
            return auth_error_response(ApiError::unauthorized());
        }
    };

    let auth_result = auth::validate_token(&token, &state.api_keys, Utc::now());
    match &auth_result {
        AuthResult::Valid { key_id, .. } => {
            // Enforce per-key origin restrictions
            if let Some(origin) = request
                .headers()
                .get(header::ORIGIN)
                .and_then(|v| v.to_str().ok())
            {
                let origin_ok = state
                    .api_keys
                    .values()
                    .find(|k| k.key_id == *key_id)
                    .map(|k| auth::check_origin(k, origin))
                    .unwrap_or(false);
                if !origin_ok {
                    return auth_error_response(ApiError::forbidden("Origin not allowed for this API key"));
                }
            }

            // Determine required permission from request method
            let perm = if request.method() == Method::POST
                || request.method() == Method::PUT
                || request.method() == Method::DELETE
            {
                PermissionLevel::Write
            } else {
                PermissionLevel::Read
            };
            if !auth::check_permission(&auth_result, perm) {
                return auth_error_response(ApiError::forbidden("Insufficient permissions"));
            }
            let limit = state
                .api_keys
                .values()
                .find(|k| k.key_id == *key_id)
                .map(|k| k.rate_limit_per_min)
                .unwrap_or(120);

            let now = Utc::now();
            let minute_bucket = now.timestamp() / 60;
            let reset_at_unix = (minute_bucket + 1) * 60;

            // Redis INCR + EXPIRE rate limiting
            let (remaining, over_limit) = if let Some(ref redis_mgr) = state.redis {
                let rl_key = format!("rl:{}:{}", key_id, minute_bucket);
                let mut conn = redis_mgr.clone();
                let result: Result<i64, redis::RedisError> = async {
                    let count: i64 = conn.incr(&rl_key, 1i64).await?;
                    // Set TTL only on first creation to avoid resetting the window
                    if count == 1 {
                        let _: () = conn.expire(&rl_key, 65i64).await?;
                    }
                    Ok(count)
                }.await;

                match result {
                    Ok(count) => {
                        let count = count as u32;
                        if count > limit {
                            (0u32, true)
                        } else {
                            (limit.saturating_sub(count), false)
                        }
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "Redis rate limit check failed — degraded mode");
                        (limit, false) // fail open
                    }
                }
            } else {
                (limit, false) // No Redis configured — no enforcement
            };

            if over_limit {
                let retry_after = reset_at_unix - now.timestamp();
                let api_err = ApiError::new(ErrorCode::RateLimited, "Rate limit exceeded");
                let mut resp = (
                    StatusCode::TOO_MANY_REQUESTS,
                    Json(error_response::<()>(api_err)),
                )
                    .into_response();
                resp.headers_mut().insert(
                    header::HeaderName::from_static("retry-after"),
                    header::HeaderValue::from_str(&retry_after.to_string())
                        .unwrap_or_else(|_| header::HeaderValue::from_static("60")),
                );
                resp.headers_mut().insert(
                    header::HeaderName::from_static("x-ratelimit-limit"),
                    header::HeaderValue::from_str(&limit.to_string())
                        .unwrap_or_else(|_| header::HeaderValue::from_static("120")),
                );
                resp.headers_mut().insert(
                    header::HeaderName::from_static("x-ratelimit-remaining"),
                    header::HeaderValue::from_static("0"),
                );
                resp.headers_mut().insert(
                    header::HeaderName::from_static("x-ratelimit-reset"),
                    header::HeaderValue::from_str(&reset_at_unix.to_string())
                        .unwrap_or_else(|_| header::HeaderValue::from_static("0")),
                );
                return resp;
            }

            let mut request = request;
            request.extensions_mut().insert(RateLimitInfo { limit_per_min: limit, remaining: Some(remaining), reset_at_unix });
            request.extensions_mut().insert(ApiAuthContext { key_id: key_id.clone() });
            next.run(request).await
        }
        AuthResult::Expired { .. } => {
            auth_error_response(ApiError::unauthorized())
        }
        AuthResult::Disabled { .. } => {
            auth_error_response(ApiError::unauthorized())
        }
        AuthResult::InvalidKey | AuthResult::MissingHeader => {
            auth_error_response(ApiError::unauthorized())
        }
    }
}

async fn add_rate_limit_headers(
    request: axum::extract::Request,
    next: middleware::Next,
) -> axum::response::Response {
    let info = request.extensions().get::<RateLimitInfo>().copied();

    let mut response = next.run(request).await;

    if let Some(info) = info {
        let remaining = info.remaining.unwrap_or(info.limit_per_min);
        // X-RateLimit-Limit
        response.headers_mut().insert(
            header::HeaderName::from_static("x-ratelimit-limit"),
            header::HeaderValue::from_str(&info.limit_per_min.to_string())
                .unwrap_or_else(|_| header::HeaderValue::from_static("120")),
        );
        // X-RateLimit-Remaining
        response.headers_mut().insert(
            header::HeaderName::from_static("x-ratelimit-remaining"),
            header::HeaderValue::from_str(&remaining.to_string())
                .unwrap_or_else(|_| header::HeaderValue::from_static("120")),
        );
        // X-RateLimit-Reset
        response.headers_mut().insert(
            header::HeaderName::from_static("x-ratelimit-reset"),
            header::HeaderValue::from_str(&info.reset_at_unix.to_string())
                .unwrap_or_else(|_| header::HeaderValue::from_static("0")),
        );
        // X-RateLimit-Policy
        response.headers_mut().insert(
            header::HeaderName::from_static("x-ratelimit-policy"),
            header::HeaderValue::from_static("requests_per_minute; window=60s"),
        );
    }

    response
}

fn llm_service_unavailable<T: Serialize>(message: &str) -> (StatusCode, Json<ApiResponse<T>>) {
    let api_err = ApiError::new(ErrorCode::ServiceUnavailable, message);
    (
        StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::SERVICE_UNAVAILABLE),
        Json(error_response(api_err)),
    )
}

#[cfg(feature = "llm")]
fn infer_llm_provider(base_url: &str) -> LlmProvider {
    let lower = base_url.to_lowercase();
    if lower.contains("openai.azure.com") {
        LlmProvider::AzureOpenAi
    } else if lower.contains("api.openai.com") {
        LlmProvider::OpenAi
    } else {
        LlmProvider::LlamaCpp
    }
}

#[cfg(feature = "llm")]
fn build_llm_runtime(config: &AppConfig) -> Result<Option<LlmRuntime>> {
    let base_url = match config.llm_base_url.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
        Some(url) => url.to_string(),
        None => return Ok(None),
    };

    let provider = infer_llm_provider(&base_url);
    if matches!(provider, LlmProvider::OpenAi | LlmProvider::AzureOpenAi) && config.llm_api_key.is_none() {
        anyhow::bail!("LLM_API_KEY is required for OpenAI/Azure providers");
    }

    let primary = ModelConfig {
        model_name: config.llm_model.clone(),
        provider: provider.clone(),
        base_url: base_url.clone(),
        api_key: config.llm_api_key.clone(),
        max_tokens: 4096,
        temperature: 0.2,
        timeout_seconds: 300,
    };

    let lightweight = ModelConfig {
        model_name: config.llm_model.clone(),
        provider,
        base_url,
        api_key: config.llm_api_key.clone(),
        max_tokens: 1024,
        temperature: 0.1,
        timeout_seconds: 120,
    };

    let mut issues = primary.validate();
    issues.extend(lightweight.validate());
    if !issues.is_empty() {
        anyhow::bail!("Invalid LLM configuration: {}", issues.join("; "));
    }

    Ok(Some(LlmRuntime { primary, lightweight }))
}

fn auth_error_response(err: ApiError) -> axum::response::Response {
    let status = StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::UNAUTHORIZED);
    (status, Json(error_response::<serde_json::Value>(err))).into_response()
}

async fn health() -> Json<HealthResponse> {
    let started_at = STARTED_AT.get_or_init(Utc::now);
    let uptime_secs = Utc::now()
        .signed_duration_since(*started_at)
        .num_seconds()
        .max(0) as u64;

    let checks = vec![
        ComponentHealth {
            name: "api".to_string(),
            status: HealthStatus::Healthy,
            message: None,
        },
        ComponentHealth {
            name: "routes".to_string(),
            status: HealthStatus::Healthy,
            message: Some(format!("{} endpoints registered", routes::all_endpoints().len())),
        },
    ];

    Json(HealthResponse {
        status: aggregate_health(&checks),
        version: env!("CARGO_PKG_VERSION").to_string(),
        uptime_secs,
        checks,
    })
}

/// Kubernetes liveness probe - returns 200 if process is running.
/// Does NOT check dependencies; only indicates the process is alive.
async fn health_live() -> StatusCode {
    StatusCode::OK
}

/// Kubernetes readiness probe - returns 200 only if the service can handle traffic.
/// Checks database connectivity and search index availability.
async fn health_ready(State(state): State<AppState>) -> (StatusCode, Json<HealthResponse>) {
    let started_at = STARTED_AT.get_or_init(Utc::now);
    let uptime_secs = Utc::now()
        .signed_duration_since(*started_at)
        .num_seconds()
        .max(0) as u64;

    let mut checks = Vec::new();

    // Check database connectivity
    let db_check = match sqlx::query("SELECT 1")
        .fetch_one(&state.store.pool)
        .await
    {
        Ok(_) => ComponentHealth {
            name: "database".to_string(),
            status: HealthStatus::Healthy,
            message: Some("Connection OK".to_string()),
        },
        Err(e) => ComponentHealth {
            name: "database".to_string(),
            status: HealthStatus::Unhealthy,
            message: Some(format!("Connection failed: {}", e)),
        },
    };
    checks.push(db_check);

    // Check search index
    let search_check = ComponentHealth {
        name: "search_index".to_string(),
        status: HealthStatus::Healthy,
        message: Some("Index available".to_string()),
    };
    checks.push(search_check);

    // Check API key configuration
    let api_keys_check = if state.api_keys.is_empty() {
        ComponentHealth {
            name: "api_keys".to_string(),
            status: HealthStatus::Unhealthy,
            message: Some("No API keys configured".to_string()),
        }
    } else {
        ComponentHealth {
            name: "api_keys".to_string(),
            status: HealthStatus::Healthy,
            message: Some(format!("{} keys loaded", state.api_keys.len())),
        }
    };
    checks.push(api_keys_check);

    // API process health
    checks.push(ComponentHealth {
        name: "api".to_string(),
        status: HealthStatus::Healthy,
        message: None,
    });

    let overall = aggregate_health(&checks);
    let status_code = match overall {
        HealthStatus::Healthy => StatusCode::OK,
        HealthStatus::Degraded => StatusCode::OK, // Still accept traffic when degraded
        HealthStatus::Unhealthy => StatusCode::SERVICE_UNAVAILABLE,
    };

    (
        status_code,
        Json(HealthResponse {
            status: overall,
            version: env!("CARGO_PKG_VERSION").to_string(),
            uptime_secs,
            checks,
        }),
    )
}

async fn endpoints() -> Json<Vec<routes::EndpointDef>> {
    Json(routes::all_endpoints())
}

/// Prometheus metrics endpoint - returns metrics in Prometheus text format.
/// Exposes HTTP request metrics, application-specific metrics, and system metrics.
async fn metrics(State(state): State<AppState>) -> impl IntoResponse {
    // Update application metrics from database counts
    if let Ok(stats) = state.store.get_dashboard_stats().await {
        COMPANIES_TRACKED.set(stats.total_companies as f64);
        PERSONS_TRACKED.set(stats.total_persons as f64);
        RECIPES_ACTIVE.set(stats.active_recipes as f64);
        
        // Set total warnings
        WARNINGS_TOTAL.with_label_values(&["total"]).set(stats.total_warnings as f64);
        WARNINGS_TOTAL.with_label_values(&["active"]).set(stats.unacknowledged_warnings as f64);
        
        // Set insights total
        INSIGHTS_TOTAL.with_label_values(&["total"]).set(stats.total_insights as f64);
    }
    
    // Encode all metrics
    let encoder = TextEncoder::new();
    let metric_families = METRICS_REGISTRY.gather();
    let mut buffer = Vec::new();
    if let Err(e) = encoder.encode(&metric_families, &mut buffer) {
        tracing::error!("Failed to encode metrics: {}", e);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/plain")],
            "Failed to encode metrics".to_string(),
        );
    }
    
    let output = String::from_utf8(buffer).unwrap_or_else(|_| "Encoding error".to_string());
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/plain; version=0.0.4; charset=utf-8")],
        output,
    )
}

fn decode_user_preferences(
    user_id: &str,
    record: Option<apex_store::postgres::UserPreferencesRecord>,
) -> (UserPreferences, serde_json::Value) {
    let mut prefs = UserPreferences {
        user_id: user_id.to_string(),
        ..UserPreferences::default()
    };
    let mut raw = serde_json::json!({});

    if let Some(record) = record {
        prefs.theme = record.theme;
        prefs.locale = record.locale;
        prefs.updated_at = record.updated_at;
        raw = if record.preferences.is_object() {
            record.preferences
        } else {
            serde_json::json!({})
        };

        if let Some(value) = raw.get("default_region") {
            prefs.default_region = value.as_str().map(|s| s.to_string());
        }
        if let Some(value) = raw.get("dashboard_layout").and_then(|v| v.as_str()) {
            prefs.dashboard_layout = value.to_string();
        }
        if let Some(value) = raw.get("notifications") {
            if let Ok(parsed) = serde_json::from_value::<NotificationPrefs>(value.clone()) {
                prefs.notifications = parsed;
            }
        }
        if let Some(value) = raw.get("table_columns") {
            if let Ok(parsed) = serde_json::from_value::<HashMap<String, Vec<String>>>(value.clone()) {
                prefs.table_columns = parsed;
            }
        }
        if let Some(value) = raw.get("custom") {
            if let Ok(parsed) = serde_json::from_value::<HashMap<String, serde_json::Value>>(value.clone()) {
                prefs.custom = parsed;
            }
        }
    }

    (prefs, raw)
}

async fn get_preferences(
    State(state): State<AppState>,
    Extension(auth_ctx): Extension<ApiAuthContext>,
) -> (StatusCode, Json<ApiResponse<PreferencesResponse>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let user_id = auth_ctx.key_id;

    let record = match state.store.get_user_preferences_record(&user_id).await {
        Ok(v) => v,
        Err(err) => {
            tracing::error!(request_id = %request_id, "get preferences failed: {err:#}");
            let api_err = ApiError::internal("Failed to fetch preferences");
            return (
                StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(error_response(api_err)),
            );
        }
    };

    let (preferences, _) = decode_user_preferences(&user_id, record);
    let duration_ms = start.elapsed().as_millis() as u64;
    let meta = ResponseMeta::now()
        .with_request_id(request_id)
        .with_duration(duration_ms);
    (
        StatusCode::OK,
        Json(success_with_meta(PreferencesResponse { preferences }, meta)),
    )
}

async fn update_preferences(
    State(state): State<AppState>,
    Extension(auth_ctx): Extension<ApiAuthContext>,
    Json(body): Json<UpdatePreferencesRequest>,
) -> (StatusCode, Json<ApiResponse<PreferencesResponse>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let user_id = auth_ctx.key_id;

    let existing = match state.store.get_user_preferences_record(&user_id).await {
        Ok(v) => v,
        Err(err) => {
            tracing::error!(request_id = %request_id, "fetch existing preferences failed: {err:#}");
            let api_err = ApiError::internal("Failed to load existing preferences");
            return (
                StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(error_response(api_err)),
            );
        }
    };

    let (mut preferences, mut raw) = decode_user_preferences(&user_id, existing);
    preferences.apply_update(body);

    if !raw.is_object() {
        raw = serde_json::json!({});
    }
    if let Some(obj) = raw.as_object_mut() {
        obj.insert("default_region".to_string(), serde_json::to_value(&preferences.default_region).unwrap_or(serde_json::Value::Null));
        obj.insert("dashboard_layout".to_string(), serde_json::to_value(&preferences.dashboard_layout).unwrap_or(serde_json::Value::Null));
        obj.insert("notifications".to_string(), serde_json::to_value(&preferences.notifications).unwrap_or(serde_json::Value::Null));
        obj.insert("table_columns".to_string(), serde_json::to_value(&preferences.table_columns).unwrap_or(serde_json::Value::Null));
        obj.insert("custom".to_string(), serde_json::to_value(&preferences.custom).unwrap_or(serde_json::Value::Null));
    }

    if let Err(err) = state
        .store
        .upsert_user_preferences_record(&user_id, &preferences.theme, &preferences.locale, &raw)
        .await
    {
        tracing::error!(request_id = %request_id, "upsert preferences failed: {err:#}");
        let api_err = ApiError::internal("Failed to save preferences");
        return (
            StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            Json(error_response(api_err)),
        );
    }

    preferences.updated_at = Utc::now();
    let duration_ms = start.elapsed().as_millis() as u64;
    let meta = ResponseMeta::now()
        .with_request_id(request_id)
        .with_duration(duration_ms);
    (
        StatusCode::OK,
        Json(success_with_meta(PreferencesResponse { preferences }, meta)),
    )
}

async fn list_warnings(
    State(state): State<AppState>,
    Query(params): Query<ListWarningsQuery>,
) -> (StatusCode, Json<ApiResponse<PagedResponse<WarningResponse>>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();

    let (page, per_page, _offset) = match validate_pagination(params.page, params.per_page) {
        Ok(p) => p,
        Err(err) => {
            return (
                StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::BAD_REQUEST),
                Json(error_response(err)),
            );
        }
    };
    let date_from = match parse_date_start(&params.date_from) {
        Ok(v) => v,
        Err(msg) => {
            let api_err = ApiError::bad_request(msg);
            return (StatusCode::BAD_REQUEST, Json(error_response(api_err)));
        }
    };
    let date_to = match parse_date_end(&params.date_to) {
        Ok(v) => v,
        Err(msg) => {
            let api_err = ApiError::bad_request(msg);
            return (StatusCode::BAD_REQUEST, Json(error_response(api_err)));
        }
    };
    if let Err(msg) = validate_date_range(&date_from, &date_to) {
        let api_err = ApiError::bad_request(msg);
        return (
            StatusCode::BAD_REQUEST,
            Json(error_response(api_err)),
        );
    }

    let regions = match parse_csv_upper_strict(&params.regions, 32, "regions") {
        Ok(v) => v,
        Err(api_err) => {
            return (
                StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::BAD_REQUEST),
                Json(error_response(api_err)),
            );
        }
    };
    if let Err(api_err) = validate_region_codes(&regions) {
        return (
            StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::BAD_REQUEST),
            Json(error_response(api_err)),
        );
    }
    let severities = match parse_csv_lower_strict(&params.severities, 32, "severities") {
        Ok(v) => v,
        Err(api_err) => {
            return (
                StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::BAD_REQUEST),
                Json(error_response(api_err)),
            );
        }
    };
    if let Err(api_err) = validate_severity_codes(&severities) {
        return (
            StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::BAD_REQUEST),
            Json(error_response(api_err)),
        );
    }
    let warning_types = match parse_csv_lower_strict(&params.warning_types, 32, "warning_types") {
        Ok(v) => v,
        Err(api_err) => {
            return (
                StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::BAD_REQUEST),
                Json(error_response(api_err)),
            );
        }
    };
    if let Err(api_err) = validate_warning_type_codes(&warning_types) {
        return (
            StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::BAD_REQUEST),
            Json(error_response(api_err)),
        );
    }

    let filters = WarningListFilters {
        regions,
        severities,
        warning_types,
        acknowledged: params.acknowledged,
        date_from,
        date_to,
        search: match params.search.as_deref() {
            Some(value) => match validate_search_text(value, 500) {
                Ok(v) => v,
                Err(msg) => {
                    let api_err = ApiError::validation("search", msg);
                    return (
                        StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::BAD_REQUEST),
                        Json(error_response(api_err)),
                    );
                }
            },
            None => None,
        },
    };

    let order_by = params
        .sort_by
        .clone()
        .map(map_warning_sort)
        .unwrap_or(WarningOrderBy::CreatedAt);
    let desc = params.sort_dir.clone().unwrap_or_default() == SortDirection::Desc;

    let total = match tracing::info_span!("db.count_warnings", request_id = %request_id).in_scope(|| {
        state.store.count_warnings(&filters)
    }).await {
        Ok(value) => value.max(0) as u64,
        Err(err) => {
            tracing::error!(request_id = %request_id, "count warnings failed: {err:#}");
            let api_err = ApiError::internal("Failed to count warnings");
            return (
                StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(error_response(api_err)),
            );
        }
    };

    // Re-clamp offset after learning the true total so data matches metadata.
    let clamped_page = clamp_page(page, per_page, total);
    let clamped_offset = ((clamped_page - 1) as i64).saturating_mul(per_page as i64);

    let rows = match tracing::info_span!("db.list_warnings", request_id = %request_id).in_scope(|| {
        state
            .store
            .list_warnings(&filters, Some(order_by), desc, per_page as i64, clamped_offset)
    }).await {
        Ok(value) => value,
        Err(err) => {
            tracing::error!(request_id = %request_id, "list warnings failed: {err:#}");
            let api_err = ApiError::internal("Failed to list warnings");
            return (
                StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(error_response(api_err)),
            );
        }
    };

    let items = rows.into_iter().map(warning_row_to_response).collect();
    let payload = PagedResponse {
        items,
        total,
        page: clamped_page,
        per_page,
    };

    let duration_ms = start.elapsed().as_millis() as u64;
    let meta = ResponseMeta::now()
        .with_request_id(request_id)
        .with_duration(duration_ms);
    log_latency("list_warnings", duration_ms);

    (StatusCode::OK, Json(success_with_meta(payload, meta)))
}

async fn acknowledge_warning(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: axum::body::Bytes,
) -> (StatusCode, Json<ApiResponse<routes::warnings::AcknowledgeResponse>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let id_parsed = match validate_warning_id(&id) {
        Ok(uuid) => uuid,
        Err(msg) => {
            let err = ApiError::bad_request(msg);
            return (
                StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::BAD_REQUEST),
                Json(error_response(err)),
            );
        }
    };

    let body_value: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => {
            let err = ApiError::bad_request("invalid JSON body");
            return (
                StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::BAD_REQUEST),
                Json(error_response(err)),
            );
        }
    };
    if let Err(msg) = validate_json_depth(&body_value, MAX_JSON_DEPTH) {
        let err = ApiError::validation("body", msg);
        return (
            StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::UNPROCESSABLE_ENTITY),
            Json(error_response(err)),
        );
    }
    let body: AcknowledgeRequest = match serde_json::from_value(body_value) {
        Ok(v) => v,
        Err(_) => {
            let err = ApiError::validation("body", "invalid request schema");
            return (
                StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::UNPROCESSABLE_ENTITY),
                Json(error_response(err)),
            );
        }
    };

    if let Err(msg) = validate_acknowledge(&body) {
        let err = ApiError::validation("body", msg);
        return (
            StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::UNPROCESSABLE_ENTITY),
            Json(error_response(err)),
        );
    }

    match tracing::info_span!("db.acknowledge_warning", request_id = %request_id).in_scope(|| {
        state
            .store
            .acknowledge_warning(id_parsed, &body.user_id.trim(), body.note.as_deref())
    }).await {
        Ok(Some(true)) => {
            let resp = routes::warnings::AcknowledgeResponse {
                warning_id: id_parsed.to_string(),
                acknowledged: true,
                acknowledged_by: body.user_id,
                acknowledged_at: Utc::now(),
            };
            let duration_ms = start.elapsed().as_millis() as u64;
            let meta = ResponseMeta::now()
                .with_request_id(request_id)
                .with_duration(duration_ms);
            log_latency("acknowledge_warning", duration_ms);
            (StatusCode::OK, Json(success_with_meta(resp, meta)))
        }
        Ok(Some(false)) => {
            let err = ApiError::new(
                apex_api::responses::ErrorCode::Conflict,
                "Warning is already acknowledged",
            );
            (
                StatusCode::CONFLICT,
                Json(error_response(err)),
            )
        }
        Ok(None) => {
            let err = ApiError::not_found("warning", &id);
            (
                StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::NOT_FOUND),
                Json(error_response(err)),
            )
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "acknowledge warning failed: {err:#}");
            let api_err = ApiError::internal("Failed to acknowledge warning");
            (
                StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(error_response(api_err)),
            )
        }
    }
}

/// Delete a single warning by ID.
async fn delete_warning(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let id_parsed = match validate_warning_id(&id) {
        Ok(uuid) => uuid,
        Err(msg) => {
            let err = ApiError::bad_request(msg);
            return (
                StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::BAD_REQUEST),
                Json(error_response(err)),
            );
        }
    };

    match state.store.delete_warning(id_parsed).await {
        Ok(true) => {
            let duration_ms = start.elapsed().as_millis() as u64;
            let meta = ResponseMeta::now()
                .with_request_id(request_id)
                .with_duration(duration_ms);
            log_latency("delete_warning", duration_ms);
            (StatusCode::OK, Json(success_with_meta(serde_json::json!({
                "deleted": true,
                "warning_id": id
            }), meta)))
        }
        Ok(false) => {
            let err = ApiError::not_found("warning", &id);
            (
                StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::NOT_FOUND),
                Json(error_response(err)),
            )
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "delete warning failed: {err:#}");
            let api_err = ApiError::internal("Failed to delete warning");
            (
                StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(error_response(api_err)),
            )
        }
    }
}

/// Bulk delete warnings by IDs.
#[derive(Debug, Deserialize)]
struct BulkDeleteRequest {
    ids: Vec<String>,
}

async fn delete_warnings_bulk(
    State(state): State<AppState>,
    body: axum::body::Bytes,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();

    let body_value: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => {
            let err = ApiError::bad_request("invalid JSON body");
            return (
                StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::BAD_REQUEST),
                Json(error_response(err)),
            );
        }
    };

    let req: BulkDeleteRequest = match serde_json::from_value(body_value) {
        Ok(v) => v,
        Err(_) => {
            let err = ApiError::validation("body", "expected { ids: string[] }");
            return (
                StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::UNPROCESSABLE_ENTITY),
                Json(error_response(err)),
            );
        }
    };

    if req.ids.is_empty() {
        let err = ApiError::validation("ids", "at least one ID required");
        return (
            StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::UNPROCESSABLE_ENTITY),
            Json(error_response(err)),
        );
    }

    if req.ids.len() > 1000 {
        let err = ApiError::validation("ids", "maximum 1000 IDs per request");
        return (
            StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::UNPROCESSABLE_ENTITY),
            Json(error_response(err)),
        );
    }

    let mut parsed_ids = Vec::with_capacity(req.ids.len());
    for id_str in &req.ids {
        match Uuid::parse_str(id_str) {
            Ok(u) => parsed_ids.push(u),
            Err(_) => {
                let err = ApiError::validation("ids", format!("invalid UUID: {}", id_str));
                return (
                    StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::UNPROCESSABLE_ENTITY),
                    Json(error_response(err)),
                );
            }
        }
    }

    match state.store.delete_warnings(&parsed_ids).await {
        Ok(count) => {
            let duration_ms = start.elapsed().as_millis() as u64;
            let meta = ResponseMeta::now()
                .with_request_id(request_id)
                .with_duration(duration_ms);
            log_latency("delete_warnings_bulk", duration_ms);
            (StatusCode::OK, Json(success_with_meta(serde_json::json!({
                "deleted_count": count,
                "requested_count": req.ids.len()
            }), meta)))
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "bulk delete warnings failed: {err:#}");
            let api_err = ApiError::internal("Failed to delete warnings");
            (
                StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(error_response(api_err)),
            )
        }
    }
}

/// Delete all warnings (dangerous operation).
async fn delete_all_warnings(
    State(state): State<AppState>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();

    match state.store.delete_all_warnings().await {
        Ok(count) => {
            tracing::warn!(request_id = %request_id, "deleted ALL warnings: {} rows", count);
            let duration_ms = start.elapsed().as_millis() as u64;
            let meta = ResponseMeta::now()
                .with_request_id(request_id)
                .with_duration(duration_ms);
            log_latency("delete_all_warnings", duration_ms);
            (StatusCode::OK, Json(success_with_meta(serde_json::json!({
                "deleted_count": count,
                "message": "All warnings deleted"
            }), meta)))
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "delete all warnings failed: {err:#}");
            let api_err = ApiError::internal("Failed to delete all warnings");
            (
                StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(error_response(api_err)),
            )
        }
    }
}

async fn list_insights(
    State(state): State<AppState>,
    Query(params): Query<ListInsightsQuery>,
) -> (StatusCode, Json<ApiResponse<PagedResponse<InsightResponse>>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let (page, per_page, _offset) = match validate_pagination(params.page, params.per_page) {
        Ok(p) => p,
        Err(err) => {
            return (
                StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::BAD_REQUEST),
                Json(error_response(err)),
            );
        }
    };

    let date_from = match parse_date_start(&params.date_from) {
        Ok(v) => v,
        Err(msg) => {
            let api_err = ApiError::bad_request(msg);
            return (StatusCode::BAD_REQUEST, Json(error_response(api_err)));
        }
    };
    let date_to = match parse_date_end(&params.date_to) {
        Ok(v) => v,
        Err(msg) => {
            let api_err = ApiError::bad_request(msg);
            return (StatusCode::BAD_REQUEST, Json(error_response(api_err)));
        }
    };
    if let Err(msg) = validate_date_range(&date_from, &date_to) {
        let api_err = ApiError::bad_request(msg);
        return (
            StatusCode::BAD_REQUEST,
            Json(error_response(api_err)),
        );
    }

    let regions = match parse_csv_upper_strict(&params.regions, 32, "regions") {
        Ok(v) => v,
        Err(api_err) => {
            return (
                StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::BAD_REQUEST),
                Json(error_response(api_err)),
            );
        }
    };
    if let Err(api_err) = validate_region_codes(&regions) {
        return (
            StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::BAD_REQUEST),
            Json(error_response(api_err)),
        );
    }

    let filters = InsightListFilters {
        regions,
        date_from,
        date_to,
        search: match params.search.as_deref() {
            Some(value) => match validate_search_text(value, 500) {
                Ok(v) => v,
                Err(msg) => {
                    let api_err = ApiError::validation("search", msg);
                    return (
                        StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::BAD_REQUEST),
                        Json(error_response(api_err)),
                    );
                }
            },
            None => None,
        },
        insight_types: params.insight_type
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|s| vec![s.to_string()])
            .unwrap_or_default(),
        bookmarked_by: if params.bookmarked.as_deref() == Some("true") {
            Some("default".to_string())
        } else {
            None
        },
    };

    let total = match tracing::info_span!("db.count_insights", request_id = %request_id).in_scope(|| {
        state.store.count_insights(&filters)
    }).await {
        Ok(value) => value.max(0) as u64,
        Err(err) => {
            tracing::error!(request_id = %request_id, "count insights failed: {err:#}");
            let api_err = ApiError::internal("Failed to count insights");
            return (
                StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(error_response(api_err)),
            );
        }
    };

    // Re-clamp offset after learning the true total so data matches metadata.
    let clamped_page = clamp_page(page, per_page, total);
    let clamped_offset = ((clamped_page - 1) as i64).saturating_mul(per_page as i64);

    let rows = match tracing::info_span!("db.list_insights", request_id = %request_id).in_scope(|| {
        state
            .store
            .list_insights(&filters, per_page as i64, clamped_offset)
    }).await {
        Ok(value) => value,
        Err(err) => {
            tracing::error!(request_id = %request_id, "list insights failed: {err:#}");
            let api_err = ApiError::internal("Failed to list insights");
            return (
                StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(error_response(api_err)),
            );
        }
    };

    let mut items: Vec<InsightResponse> = rows.into_iter().map(insight_row_to_response).collect();

    // Annotate items with bookmark status for the current user
    let insight_uuids: Vec<Uuid> = items.iter()
        .filter_map(|i| Uuid::parse_str(&i.id).ok())
        .collect();
    if !insight_uuids.is_empty() {
        if let Ok(bookmarked_ids) = state.store.get_bookmarked_insight_ids("default", &insight_uuids).await {
            let bset: std::collections::HashSet<String> = bookmarked_ids.iter().map(|id| id.to_string()).collect();
            for item in &mut items {
                item.bookmarked = Some(bset.contains(&item.id));
            }
        }
    }

    // Don't re-rank DB-paginated results: the DB already ordered by created_at DESC,
    // and re-sorting by confidence*recency breaks cross-page ordering consistency.

    let payload = PagedResponse {
        items,
        total,
        page: clamped_page,
        per_page,
    };
    let duration_ms = start.elapsed().as_millis() as u64;
    let meta = ResponseMeta::now()
        .with_request_id(request_id)
        .with_duration(duration_ms);
    log_latency("list_insights", duration_ms);

    (StatusCode::OK, Json(success_with_meta(payload, meta)))
}

async fn list_companies(
    State(state): State<AppState>,
    Query(params): Query<ListCompaniesQuery>,
) -> (StatusCode, Json<ApiResponse<PagedResponse<CompanyListItem>>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let (page, per_page, _offset) = match validate_pagination(params.page, params.per_page) {
        Ok(p) => p,
        Err(err) => {
            return (
                StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::BAD_REQUEST),
                Json(error_response(err)),
            );
        }
    };

    let regions = match parse_csv_upper_strict(&params.regions, 32, "regions") {
        Ok(v) => v,
        Err(api_err) => {
            return (
                StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::BAD_REQUEST),
                Json(error_response(api_err)),
            );
        }
    };
    if let Err(api_err) = validate_region_codes(&regions) {
        return (
            StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::BAD_REQUEST),
            Json(error_response(api_err)),
        );
    }

    let filters = CompanyListFilters {
        regions,
        search: match params.search.as_deref() {
            Some(value) => match validate_search_text(value, 500) {
                Ok(v) => v,
                Err(msg) => {
                    let api_err = ApiError::validation("search", msg);
                    return (
                        StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::BAD_REQUEST),
                        Json(error_response(api_err)),
                    );
                }
            },
            None => None,
        },
        is_competitor: params.is_competitor,
    };

    let sort_field = params.sort_by.clone();
    let order_by = sort_field.clone().map(map_company_sort);
    let desc = params
        .sort_dir
        .clone()
        .map(|d| d == SortDirection::Desc)
        .unwrap_or_else(|| {
            matches!(sort_field, Some(CompanySortField::ThreatScore | CompanySortField::UpdatedAt))
        });

    let total = match tracing::info_span!("db.count_companies", request_id = %request_id).in_scope(|| {
        state.store.count_companies(&filters)
    }).await {
        Ok(value) => value.max(0) as u64,
        Err(err) => {
            tracing::error!(request_id = %request_id, "count companies failed: {err:#}");
            let api_err = ApiError::internal("Failed to count companies");
            return (
                StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(error_response(api_err)),
            );
        }
    };

    // Re-clamp offset after learning the true total so data matches metadata.
    let clamped_page = clamp_page(page, per_page, total);
    let clamped_offset = ((clamped_page - 1) as i64).saturating_mul(per_page as i64);

    let rows = match tracing::info_span!("db.list_companies", request_id = %request_id).in_scope(|| {
        state
            .store
            .list_companies(&filters, order_by, desc, per_page as i64, clamped_offset)
    }).await {
        Ok(value) => value,
        Err(err) => {
            tracing::error!(request_id = %request_id, "list companies failed: {err:#}");
            let api_err = ApiError::internal("Failed to list companies");
            return (
                StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(error_response(api_err)),
            );
        }
    };

    let items: Vec<CompanyListItem> = rows.into_iter().map(company_row_to_item).collect();
    let payload = PagedResponse {
        items,
        total,
        page: clamped_page,
        per_page,
    };
    let duration_ms = start.elapsed().as_millis() as u64;
    let meta = ResponseMeta::now()
        .with_request_id(request_id)
        .with_duration(duration_ms);
    log_latency("list_companies", duration_ms);

    (StatusCode::OK, Json(success_with_meta(payload, meta)))
}

async fn list_persons(
    State(state): State<AppState>,
    Query(params): Query<ListPersonsQuery>,
) -> (StatusCode, Json<ApiResponse<PagedResponse<PersonListItem>>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let (page, per_page, _offset) = match validate_pagination(params.page, params.per_page) {
        Ok(p) => p,
        Err(err) => {
            return (
                StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::BAD_REQUEST),
                Json(error_response(err)),
            );
        }
    };

    let regions = match parse_csv_upper_strict(&params.regions, 32, "regions") {
        Ok(v) => v,
        Err(api_err) => {
            return (
                StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::BAD_REQUEST),
                Json(error_response(api_err)),
            );
        }
    };

    let roles = match parse_csv_lower_strict(&params.roles, 32, "roles") {
        Ok(v) => v,
        Err(api_err) => {
            return (
                StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::BAD_REQUEST),
                Json(error_response(api_err)),
            );
        }
    };

    // Tier/priority filter overrides explicit min_priority when present.
    let (tier_min, tier_max): (Option<f64>, Option<f64>) = match params.tier.as_deref() {
        Some("critical") => (Some(0.8), None),
        Some("high")     => (Some(0.6), Some(0.8)),
        Some("medium")   => (Some(0.4), Some(0.6)),
        Some("low")      => (None,      Some(0.4)),
        _                 => (None, None),
    };
    let (priority_min, priority_max): (Option<f64>, Option<f64>) = match params.priority.as_deref() {
        Some("A") | Some("a") => (Some(0.8), None),
        Some("B") | Some("b") => (Some(0.5), Some(0.8)),
        Some("C") | Some("c") => (None, Some(0.5)),
        _ => (None, None),
    };

    let filters = PersonListFilters {
        regions,
        roles,
        search: match params.search.as_deref() {
            Some(value) => match validate_search_text(value, 500) {
                Ok(v) => v,
                Err(msg) => {
                    let api_err = ApiError::validation("search", msg);
                    return (
                        StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::BAD_REQUEST),
                        Json(error_response(api_err)),
                    );
                }
            },
            None => None,
        },
        min_priority: tier_min.or(priority_min).or_else(|| params.min_priority.map(clamp_ratio)),
        max_priority: if tier_max.is_some() { tier_max } else { priority_max },
    };

    let sort_field = params.sort_by.clone();
    let order_by = sort_field.clone().map(map_person_sort);
    let desc = params
        .sort_by
        .clone()
        .map(|v| matches!(v, PersonSortField::Priority | PersonSortField::UpdatedAt))
        .unwrap_or(true);

    let total = match tracing::info_span!("db.count_persons", request_id = %request_id).in_scope(|| {
        state.store.count_persons(&filters)
    }).await {
        Ok(value) => value.max(0) as u64,
        Err(err) => {
            tracing::error!(request_id = %request_id, "count persons failed: {err:#}");
            let api_err = ApiError::internal("Failed to count persons");
            return (
                StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(error_response(api_err)),
            );
        }
    };

    let clamped_page = clamp_page(page, per_page, total);
    let clamped_offset = ((clamped_page - 1) as i64).saturating_mul(per_page as i64);

    let rows = match tracing::info_span!("db.list_persons", request_id = %request_id).in_scope(|| {
        state
            .store
            .list_persons(&filters, order_by, desc, per_page as i64, clamped_offset)
    }).await {
        Ok(value) => value,
        Err(err) => {
            tracing::error!(request_id = %request_id, "list persons failed: {err:#}");
            let api_err = ApiError::internal("Failed to list persons");
            return (
                StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(error_response(api_err)),
            );
        }
    };

    let items: Vec<PersonListItem> = rows.into_iter().map(person_row_to_item).collect();
    let payload = PagedResponse {
        items,
        total,
        page: clamped_page,
        per_page,
    };
    let duration_ms = start.elapsed().as_millis() as u64;
    let meta = ResponseMeta::now()
        .with_request_id(request_id)
        .with_duration(duration_ms);
    log_latency("list_persons", duration_ms);

    (StatusCode::OK, Json(success_with_meta(payload, meta)))
}

async fn get_company_detail(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<CompanyDetail>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();

    let company_id = match validate_company_id(&id) {
        Ok(value) => value,
        Err(msg) => {
            let api_err = ApiError::validation("company_id", msg);
            return (
                StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::BAD_REQUEST),
                Json(error_response(api_err)),
            );
        }
    };

    let row = match tracing::info_span!("db.get_company", request_id = %request_id).in_scope(|| {
        state.store.get_company(company_id)
    }).await {
        Ok(Some(value)) => value,
        Ok(None) => {
            let api_err = ApiError::not_found("company", &id);
            return (
                StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::NOT_FOUND),
                Json(error_response(api_err)),
            );
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "get company failed: {err:#}");
            let api_err = ApiError::internal("Failed to load company");
            return (
                StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(error_response(api_err)),
            );
        }
    };

    let (sites, certifications, persons) = match tokio::try_join!(
        state.store.get_sites_for_company(company_id),
        state.store.get_certifications_for_company(company_id),
        state.store.list_persons_by_org(company_id),
    ) {
        Ok(values) => values,
        Err(err) => {
            tracing::error!(request_id = %request_id, "company detail lookup failed: {err:#}");
            let api_err = ApiError::internal("Failed to load company detail");
            return (
                StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(error_response(api_err)),
            );
        }
    };

    let detail = company_row_to_detail(row, sites, certifications, persons);
    let duration_ms = start.elapsed().as_millis() as u64;
    let meta = ResponseMeta::now()
        .with_request_id(request_id)
        .with_duration(duration_ms);
    log_latency("get_company_detail", duration_ms);

    (StatusCode::OK, Json(success_with_meta(detail, meta)))
}

async fn get_person_detail(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<PersonDetail>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();

    let person_id = match validate_person_id(&id) {
        Ok(value) => value,
        Err(msg) => {
            let api_err = ApiError::validation("person_id", msg);
            return (
                StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::BAD_REQUEST),
                Json(error_response(api_err)),
            );
        }
    };

    let row = match tracing::info_span!("db.get_person", request_id = %request_id).in_scope(|| {
        state.store.get_person(person_id)
    }).await {
        Ok(Some(value)) => value,
        Ok(None) => {
            let api_err = ApiError::not_found("person", &id);
            return (
                StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::NOT_FOUND),
                Json(error_response(api_err)),
            );
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "get person failed: {err:#}");
            let api_err = ApiError::internal("Failed to load person");
            return (
                StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(error_response(api_err)),
            );
        }
    };

    let org_id = row.primary_org_id;
    let org_name = if let Some(oid) = org_id {
        match state.store.get_company(oid).await {
            Ok(Some(company)) => company.name,
            Ok(None) => "Independent".to_string(),
            Err(err) => {
                tracing::error!(request_id = %request_id, "get person org failed: {err:#}");
                "Independent".to_string()
            }
        }
    } else {
        "Independent".to_string()
    };

    let role_family = row.role_family.clone().unwrap_or_else(|| "Unknown".to_string());
    let region = row.region.clone().unwrap_or_default();

    // Fetch artifacts, role history, peers, and related warning/insight counts in parallel
    let entity_ids = vec![person_id];
    let (artifacts, role_history_rows, peer_rows, related_warnings, related_insights) = match tokio::try_join!(
        state.store.get_artifacts_for_person(person_id, 12),
        state.store.get_role_history_for_person(person_id),
        state.store.get_person_peers(person_id, &role_family, &region, 6),
        state.store.get_warnings_by_entity_ids(&entity_ids, 200),
        state.store.get_insights_by_entity_ids(&entity_ids, 200),
    ) {
        Ok(values) => values,
        Err(err) => {
            tracing::error!(request_id = %request_id, "get person detail data failed: {err:#}");
            let api_err = ApiError::internal("Failed to load person detail");
            return (
                StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(error_response(api_err)),
            );
        }
    };

    let detail = person_row_to_detail(
        row,
        org_name,
        org_id,
        artifacts,
        role_history_rows,
        peer_rows,
        related_warnings.len() as i64,
        related_insights.len() as i64,
    );
    let duration_ms = start.elapsed().as_millis() as u64;
    let meta = ResponseMeta::now()
        .with_request_id(request_id)
        .with_duration(duration_ms);
    log_latency("get_person_detail", duration_ms);

    (StatusCode::OK, Json(success_with_meta(detail, meta)))
}

#[cfg(feature = "llm")]
#[derive(serde::Deserialize)]
struct PoiPayload {
    summary: String,
    roles: Vec<String>,
    affiliations: Vec<String>,
    key_facts: Vec<String>,
    risk_indicators: Vec<String>,
}

#[cfg(feature = "llm")]
#[derive(serde::Deserialize)]
struct MemoPayload {
    title: String,
    executive_summary: String,
    sections: Vec<MemoSection>,
    recommendations: Vec<String>,
}

#[cfg(feature = "llm")]
fn parse_entities_value(value: JsonValue) -> Result<Vec<ExtractedEntity>, String> {
    let entities_value = if value.is_array() {
        value
    } else {
        value
            .get("entities")
            .or_else(|| value.get("items"))
            .or_else(|| value.get("data"))
            .cloned()
            .or_else(|| value.as_object().and_then(|map| map.values().find(|v| v.is_array()).cloned()))
            .ok_or_else(|| "Missing 'entities' array in response".to_string())?
    };

    let array = entities_value
        .as_array()
        .ok_or_else(|| "Entities payload is not an array".to_string())?;

    let mut entities = Vec::with_capacity(array.len());
    for (idx, item) in array.iter().enumerate() {
        if let Some(name) = item.as_str() {
            entities.push(ExtractedEntity {
                name: name.to_string(),
                entity_type: "unknown".to_string(),
                confidence: 0.5,
                span_start: None,
                span_end: None,
                canonical: None,
            });
            continue;
        }

        let obj = item
            .as_object()
            .ok_or_else(|| format!("entities[{}] must be an object or string", idx))?;

        let name = obj
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!("entities[{}] missing field 'name'", idx))?;
        let entity_type = obj
            .get("entity_type")
            .or_else(|| obj.get("type"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let confidence = obj
            .get("confidence")
            .and_then(|v| v.as_f64())
            .filter(|v| v.is_finite())
            .unwrap_or(0.5);
        let span_start = obj.get("span_start").and_then(|v| v.as_u64()).map(|v| v as usize);
        let span_end = obj.get("span_end").and_then(|v| v.as_u64()).map(|v| v as usize);
        let canonical = obj.get("canonical").and_then(|v| v.as_str()).map(|v| v.to_string());

        entities.push(ExtractedEntity {
            name: name.to_string(),
            entity_type: entity_type.to_string(),
            confidence,
            span_start,
            span_end,
            canonical,
        });
    }

    Ok(entities)
}

#[cfg(feature = "llm")]
async fn llm_extract_entities(
    State(state): State<AppState>,
    Json(payload): Json<ExtractEntitiesRequest>,
) -> (StatusCode, Json<ApiResponse<ExtractEntitiesResponse>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();

    let issues = payload.validate();
    if !issues.is_empty() {
        let api_err = ApiError::validation("payload", issues.join("; "));
        return (
            StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::UNPROCESSABLE_ENTITY),
            Json(error_response(api_err)),
        );
    }

    let runtime = match state.llm.as_ref() {
        Some(rt) => rt,
        None => return llm_service_unavailable("LLM not configured"),
    };

    let client = OpenAiCompatibleClient::new(runtime.lightweight.clone());
    let system = "You are an OSINT analyst. Extract named entities from the user text.\n\
Return JSON ONLY with field 'entities' as an array of objects: \
{name, entity_type, confidence (0-1), span_start, span_end, canonical}.\n\
Use null for unknown spans. Confidence must be between 0 and 1.\n\
Do not output schema field names as entities. Only real entities from the text.\n\
Example output:\n\
{\"entities\":[{\"name\":\"Starz Electronics\",\"entity_type\":\"company\",\"confidence\":0.9,\"span_start\":0,\"span_end\":17,\"canonical\":\"Starz Electronics\"},{\"name\":\"Tangier\",\"entity_type\":\"location\",\"confidence\":0.8,\"span_start\":33,\"span_end\":40,\"canonical\":\"Tangier\"}]}.";

    let mut user = format!("Text:\n{}\n", payload.text);
    if let Some(doc_type) = &payload.doc_type {
        user.push_str(&format!("Doc type: {}\n", doc_type));
    }
    if let Some(types) = &payload.entity_types {
        user.push_str(&format!("Entity types: {:?}\n", types));
    }
    user.push_str("Return JSON only.");

    let raw = match client.generate_json(system, &user).await {
        Ok(value) => value,
        Err(err) => {
            tracing::error!(request_id = %request_id, "LLM entity extraction failed: {err:#}");
            let api_err = ApiError::internal("LLM entity extraction failed");
            return (
                StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(error_response(api_err)),
            );
        }
    };

    let value = match validators::parse_json_response(&raw) {
        Ok(v) => v,
        Err(_) => {
            // Retry once with stricter JSON guidance to reduce llama.cpp format drift.
            let strict_user = format!("{}\nSTRICT JSON ONLY. No trailing text.", user);
            let retry_raw = match client.generate_json(system, &strict_user).await {
                Ok(v) => v,
                Err(err) => {
                    tracing::error!(request_id = %request_id, "LLM entity extraction retry failed: {err:#}");
                    let api_err = ApiError::internal("LLM entity extraction failed");
                    return (
                        StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                        Json(error_response(api_err)),
                    );
                }
            };
            match validators::parse_json_response(&retry_raw) {
                Ok(v) => v,
                Err(err) => {
                    let api_err = ApiError::validation("llm", err);
                    return (
                        StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::UNPROCESSABLE_ENTITY),
                        Json(error_response(api_err)),
                    );
                }
            }
        }
    };

    let entities = match parse_entities_value(value) {
        Ok(v) => v,
        Err(err) => {
            let api_err = ApiError::validation("entities", err);
            return (
                StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::UNPROCESSABLE_ENTITY),
                Json(error_response(api_err)),
            );
        }
    };

    let duration_ms = start.elapsed().as_millis() as u64;
    let response = ExtractEntitiesResponse {
        entities,
        task: LlmTask::EntityExtraction,
        model_used: client.config().model_name.clone(),
        processing_ms: duration_ms,
    };
    let meta = ResponseMeta::now()
        .with_request_id(request_id)
        .with_duration(duration_ms);

    (StatusCode::OK, Json(success_with_meta(response, meta)))
}

#[cfg(not(feature = "llm"))]
async fn llm_extract_entities(
    State(_state): State<AppState>,
    Json(_payload): Json<ExtractEntitiesRequest>,
) -> (StatusCode, Json<ApiResponse<ExtractEntitiesResponse>>) {
    llm_service_unavailable("LLM feature disabled")
}

#[cfg(feature = "llm")]
async fn llm_generate_recipe(
    State(state): State<AppState>,
    Json(payload): Json<GenerateRecipeRequest>,
) -> (StatusCode, Json<ApiResponse<GenerateRecipeResponse>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();

    let issues = payload.validate();
    if !issues.is_empty() {
        let api_err = ApiError::validation("payload", issues.join("; "));
        return (
            StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::UNPROCESSABLE_ENTITY),
            Json(error_response(api_err)),
        );
    }

    let runtime = match state.llm.as_ref() {
        Some(rt) => rt,
        None => return llm_service_unavailable("LLM not configured"),
    };

    let client = OpenAiCompatibleClient::new(runtime.primary.clone());
    let system = "You are an OSINT analyst generating detection recipes.\n\
Return JSON ONLY with fields: id, signals, narrative_template, action_playbook.\n\
Fields: id is a unique snake_case string, signals is a non-empty array.\n\
Signals must be objects with at least {name, description}.\n\
narrative_template and action_playbook must be non-empty strings.";

    let user = format!(
        "Pattern description: {}\nOutcome: {}\nSignals: {:?}\nExisting IDs: {:?}\nRegions: {:?}\nReturn JSON only.",
        payload.pattern_description,
        payload.outcome,
        payload.signals,
        payload.existing_recipe_ids,
        payload.regions
    );

    let raw = match client.generate_json(system, &user).await {
        Ok(value) => value,
        Err(err) => {
            tracing::error!(request_id = %request_id, "LLM recipe generation failed: {err:#}");
            let api_err = ApiError::internal("LLM recipe generation failed");
            return (
                StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(error_response(api_err)),
            );
        }
    };

    let recipe_json = match validators::parse_json_response(&raw) {
        Ok(v) => v,
        Err(err) => {
            let api_err = ApiError::validation("llm", err);
            return (
                StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::UNPROCESSABLE_ENTITY),
                Json(error_response(api_err)),
            );
        }
    };

    let recipe_issues = validators::validate_recipe_json(&recipe_json);
    if !recipe_issues.is_empty() {
        let api_err = ApiError::validation("recipe_json", recipe_issues.join("; "));
        return (
            StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::UNPROCESSABLE_ENTITY),
            Json(error_response(api_err)),
        );
    }

    let duration_ms = start.elapsed().as_millis() as u64;
    let response = GenerateRecipeResponse {
        recipe_json,
        task: LlmTask::RecipeHypothesis,
        model_used: client.config().model_name.clone(),
        processing_ms: duration_ms,
    };
    let meta = ResponseMeta::now()
        .with_request_id(request_id)
        .with_duration(duration_ms);

    (StatusCode::OK, Json(success_with_meta(response, meta)))
}

#[cfg(not(feature = "llm"))]
async fn llm_generate_recipe(
    State(_state): State<AppState>,
    Json(_payload): Json<GenerateRecipeRequest>,
) -> (StatusCode, Json<ApiResponse<GenerateRecipeResponse>>) {
    llm_service_unavailable("LLM feature disabled")
}

#[cfg(feature = "llm")]
async fn llm_synthesize_poi(
    State(state): State<AppState>,
    Json(payload): Json<SynthesizePoiRequest>,
) -> (StatusCode, Json<ApiResponse<SynthesizePoiResponse>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();

    let issues = payload.validate();
    if !issues.is_empty() {
        let api_err = ApiError::validation("payload", issues.join("; "));
        return (
            StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::UNPROCESSABLE_ENTITY),
            Json(error_response(api_err)),
        );
    }

    let runtime = match state.llm.as_ref() {
        Some(rt) => rt,
        None => return llm_service_unavailable("LLM not configured"),
    };

    let client = OpenAiCompatibleClient::new(runtime.primary.clone());
    let system = "You are an OSINT analyst building POI dossiers.\n\
Return JSON ONLY with fields: summary, roles, affiliations, key_facts, risk_indicators.\n\
All fields required; arrays must be non-empty.";

    let fragments_text = payload
        .fragments
        .iter()
        .enumerate()
        .map(|(i, f)| {
            format!(
                "[{}] source_url={:?} source_type={:?} date={:?}\n{}",
                i + 1,
                f.source_url,
                f.source_type,
                f.date,
                f.text
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    let user = format!(
        "Person: {}\nKnown titles: {:?}\nFragments:\n{}\nReturn JSON only.",
        payload.person_name,
        payload.known_titles,
        fragments_text
    );

    let raw = match client.generate_json(system, &user).await {
        Ok(value) => value,
        Err(err) => {
            tracing::error!(request_id = %request_id, "LLM POI synthesis failed: {err:#}");
            let api_err = ApiError::internal("LLM POI synthesis failed");
            return (
                StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(error_response(api_err)),
            );
        }
    };

    let value = match validators::parse_json_response(&raw) {
        Ok(v) => v,
        Err(err) => {
            let api_err = ApiError::validation("llm", err);
            return (
                StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::UNPROCESSABLE_ENTITY),
                Json(error_response(api_err)),
            );
        }
    };

    let payload_value: PoiPayload = match serde_json::from_value(value) {
        Ok(v) => v,
        Err(err) => {
            let api_err = ApiError::validation("poi", format!("Invalid POI payload: {}", err));
            return (
                StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::UNPROCESSABLE_ENTITY),
                Json(error_response(api_err)),
            );
        }
    };

    if payload_value.summary.trim().is_empty() || payload_value.roles.is_empty() {
        let api_err = ApiError::validation("poi", "summary/roles must not be empty");
        return (
            StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::UNPROCESSABLE_ENTITY),
            Json(error_response(api_err)),
        );
    }

    let duration_ms = start.elapsed().as_millis() as u64;
    let response = SynthesizePoiResponse {
        person_name: payload.person_name,
        summary: payload_value.summary,
        roles: payload_value.roles,
        affiliations: payload_value.affiliations,
        key_facts: payload_value.key_facts,
        risk_indicators: payload_value.risk_indicators,
        source_count: payload.fragments.len(),
        task: LlmTask::PoiSynthesis,
        model_used: client.config().model_name.clone(),
        processing_ms: duration_ms,
    };
    let meta = ResponseMeta::now()
        .with_request_id(request_id)
        .with_duration(duration_ms);

    (StatusCode::OK, Json(success_with_meta(response, meta)))
}

#[cfg(not(feature = "llm"))]
async fn llm_synthesize_poi(
    State(_state): State<AppState>,
    Json(_payload): Json<SynthesizePoiRequest>,
) -> (StatusCode, Json<ApiResponse<SynthesizePoiResponse>>) {
    llm_service_unavailable("LLM feature disabled")
}

#[cfg(feature = "llm")]
async fn llm_generate_memo(
    State(state): State<AppState>,
    Json(payload): Json<GenerateMemoRequest>,
) -> (StatusCode, Json<ApiResponse<GenerateMemoResponse>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();

    let issues = payload.validate();
    if !issues.is_empty() {
        let api_err = ApiError::validation("payload", issues.join("; "));
        return (
            StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::UNPROCESSABLE_ENTITY),
            Json(error_response(api_err)),
        );
    }

    let runtime = match state.llm.as_ref() {
        Some(rt) => rt,
        None => return llm_service_unavailable("LLM not configured"),
    };

    let client = OpenAiCompatibleClient::new(runtime.primary.clone());
    let system = "You are an intelligence analyst writing concise strategic memos.\n\
Return JSON ONLY with fields: title, executive_summary, sections, recommendations.\n\
sections is an array of {heading, content}. recommendations is an array of strings.";

    let context_text = payload
        .context_items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            format!(
                "[{}] {}\n{}\nsource={:?} date={:?}",
                i + 1,
                item.title,
                item.content,
                item.source,
                item.date
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    let user = format!(
        "Topic: {}\nAudience: {:?}\nMax words: {:?}\nContext:\n{}\nReturn JSON only.",
        payload.topic,
        payload.audience,
        payload.max_words,
        context_text
    );

    let raw = match client.generate_json(system, &user).await {
        Ok(value) => value,
        Err(err) => {
            tracing::error!(request_id = %request_id, "LLM memo generation failed: {err:#}");
            let api_err = ApiError::internal("LLM memo generation failed");
            return (
                StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(error_response(api_err)),
            );
        }
    };

    let value = match validators::parse_json_response(&raw) {
        Ok(v) => v,
        Err(err) => {
            let api_err = ApiError::validation("llm", err);
            return (
                StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::UNPROCESSABLE_ENTITY),
                Json(error_response(api_err)),
            );
        }
    };

    let payload_value: MemoPayload = match serde_json::from_value(value) {
        Ok(v) => v,
        Err(err) => {
            let api_err = ApiError::validation("memo", format!("Invalid memo payload: {}", err));
            return (
                StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::UNPROCESSABLE_ENTITY),
                Json(error_response(api_err)),
            );
        }
    };

    if payload_value.title.trim().is_empty()
        || payload_value.executive_summary.trim().is_empty()
        || payload_value.sections.is_empty()
    {
        let api_err = ApiError::validation("memo", "title, executive_summary, sections required");
        return (
            StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::UNPROCESSABLE_ENTITY),
            Json(error_response(api_err)),
        );
    }

    let duration_ms = start.elapsed().as_millis() as u64;
    let response = GenerateMemoResponse {
        title: payload_value.title,
        executive_summary: payload_value.executive_summary,
        sections: payload_value.sections,
        recommendations: payload_value.recommendations,
        task: LlmTask::MemoGeneration,
        model_used: client.config().model_name.clone(),
        processing_ms: duration_ms,
    };
    let meta = ResponseMeta::now()
        .with_request_id(request_id)
        .with_duration(duration_ms);

    (StatusCode::OK, Json(success_with_meta(response, meta)))
}

#[cfg(not(feature = "llm"))]
async fn llm_generate_memo(
    State(_state): State<AppState>,
    Json(_payload): Json<GenerateMemoRequest>,
) -> (StatusCode, Json<ApiResponse<GenerateMemoResponse>>) {
    llm_service_unavailable("LLM feature disabled")
}

async fn search(
    State(state): State<AppState>,
    Query(params): Query<SearchQuery>,
) -> (StatusCode, Json<ApiResponse<SearchResponse>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let (page, per_page, _offset) = match validate_pagination(params.page, params.per_page) {
        Ok(p) => p,
        Err(err) => {
            return (
                StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::BAD_REQUEST),
                Json(error_response(err)),
            );
        }
    };
    let limit = per_page.clamp(1, 50) as usize;
    let query = match validate_search_query(&params.q) {
        Ok(q) => q,
        Err(msg) => {
            let err = ApiError::bad_request(msg);
            return (
                StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::BAD_REQUEST),
                Json(error_response(err)),
            );
        }
    };

    let offset = ((page - 1) as usize).saturating_mul(limit);

    let mut full_query = query.clone();
    if params.entity_types.is_some() {
        let types = match parse_csv_lower_strict(&params.entity_types, 32, "entity_types") {
            Ok(v) => v,
            Err(api_err) => {
                return (
                    StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::BAD_REQUEST),
                    Json(error_response(api_err)),
                );
            }
        };
        if !types.is_empty() {
            let clause = types
                .iter()
                .filter(|t| t.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-'))
                .map(|t| format!("entity_type:{}", t))
                .collect::<Vec<_>>()
                .join(" OR ");
            if !clause.is_empty() {
                full_query = format!("({}) AND ({})", query, clause);
            }
        }
    }
    if params.regions.is_some() {
        let regions = match parse_csv_upper_strict(&params.regions, 32, "regions") {
            Ok(v) => v,
            Err(api_err) => {
                return (
                    StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::BAD_REQUEST),
                    Json(error_response(api_err)),
                );
            }
        };
        if let Err(api_err) = validate_region_codes(&regions) {
            return (
                StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::BAD_REQUEST),
                Json(error_response(api_err)),
            );
        }
        if !regions.is_empty() {
            let clause = regions
                .iter()
                .filter(|r| r.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-'))
                .map(|r| format!("region:{}", r))
                .collect::<Vec<_>>()
                .join(" OR ");
            if !clause.is_empty() {
                full_query = format!("({}) AND ({})", full_query, clause);
            }
        }
    }

    if params.date_from.is_some() || params.date_to.is_some() {
        let from_dt = match parse_date_start(&params.date_from) {
            Ok(v) => v,
            Err(msg) => {
                let api_err = ApiError::bad_request(msg);
                return (StatusCode::BAD_REQUEST, Json(error_response(api_err)));
            }
        };
        let to_dt = match parse_date_end(&params.date_to) {
            Ok(v) => v,
            Err(msg) => {
                let api_err = ApiError::bad_request(msg);
                return (StatusCode::BAD_REQUEST, Json(error_response(api_err)));
            }
        };
        if let Err(msg) = validate_date_range(&from_dt, &to_dt) {
            let api_err = ApiError::bad_request(msg);
            return (
                StatusCode::BAD_REQUEST,
                Json(error_response(api_err)),
            );
        }
        let from = from_dt.map(|dt| dt.timestamp()).unwrap_or(i64::MIN / 2);
        let to = to_dt.map(|dt| dt.timestamp()).unwrap_or(i64::MAX / 2);
        full_query = format!("({}) AND timestamp:[{} TO {}]", full_query, from, to);
    }

    let (results, total_hits) = match state.search_index.search_with_total(&full_query, limit, offset) {
        Ok(r) => r,
        Err(err) => {
            // Tantivy query-parse errors are user errors (bad syntax), not server errors.
            let err_msg = err.to_string();
            let is_parse_err = err_msg.contains("invalid query")
                || err_msg.contains("Syntax Error")
                || err_msg.contains("expected");
            if is_parse_err {
                let api_err = ApiError::bad_request("Invalid search query syntax");
                return (
                    StatusCode::BAD_REQUEST,
                    Json(error_response(api_err)),
                );
            }
            tracing::error!(request_id = %request_id, "search failed: {err:#}");
            let api_err = ApiError::internal("Search service error");
            return (
                StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(error_response(api_err)),
            );
        }
    };

    let tokens = tokenize_query(&query);
    let mut hits: Vec<SearchHit> = results
        .into_iter()
        .map(|r| SearchHit {
            id: r.id,
            entity_type: r.entity_type,
            title: r.title,
            snippet: highlight_snippet(&r.snippet, &tokens, 240),
            score: r.score as f64,
            region: if r.region.is_empty() { None } else { Some(r.region) },
            url: if r.url.is_empty() { None } else { Some(r.url) },
            updated_at: Utc
                .timestamp_opt(r.timestamp, 0)
                .single()
                .unwrap_or_else(|| Utc.timestamp_opt(0, 0).single().unwrap()),
        })
        .collect();

    sort_by_score(&mut hits);
    let facets = build_facets(&hits);

    let resp = SearchResponse {
        query,
        total_hits,
        results: hits,
        facets,
    };

    let duration_ms = start.elapsed().as_millis() as u64;
    let meta = ResponseMeta::now()
        .with_request_id(request_id)
        .with_duration(duration_ms);
    log_latency("search", duration_ms);

    (StatusCode::OK, Json(success_with_meta(resp, meta)))
}

async fn list_graph(
    State(state): State<AppState>,
) -> (StatusCode, Json<ApiResponse<GraphOverviewWithEdges>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();

    let companies_total = match state.store.count_companies(&CompanyListFilters::default()).await {
        Ok(v) => v.max(0) as u64,
        Err(err) => {
            tracing::error!(request_id = %request_id, "count companies failed: {err:#}");
            let api_err = ApiError::internal("Failed to count companies");
            return (
                StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(error_response(api_err)),
            );
        }
    };

    let persons_total = match state.store.count_persons(&PersonListFilters::default()).await {
        Ok(v) => v.max(0) as u64,
        Err(err) => {
            tracing::error!(request_id = %request_id, "count persons failed: {err:#}");
            let api_err = ApiError::internal("Failed to count persons");
            return (
                StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(error_response(api_err)),
            );
        }
    };

    let warnings_total = match state.store.count_warnings(&WarningListFilters::default()).await {
        Ok(v) => v.max(0) as u64,
        Err(err) => {
            tracing::error!(request_id = %request_id, "count warnings failed: {err:#}");
            let api_err = ApiError::internal("Failed to count warnings");
            return (
                StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(error_response(api_err)),
            );
        }
    };

    let insights_total = match state.store.count_insights(&InsightListFilters::default()).await {
        Ok(v) => v.max(0) as u64,
        Err(err) => {
            tracing::error!(request_id = %request_id, "count insights failed: {err:#}");
            let api_err = ApiError::internal("Failed to count insights");
            return (
                StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(error_response(api_err)),
            );
        }
    };

    // Fetch edges for visualization
    let edges_total = match state.store.count_edges().await {
        Ok(v) => v.max(0) as u64,
        Err(err) => {
            tracing::warn!(request_id = %request_id, "count edges failed: {err:#}");
            0
        }
    };

    let edge_rows = match state.store.list_all_edges(200).await {
        Ok(rows) => rows,
        Err(err) => {
            tracing::warn!(request_id = %request_id, "list edges failed: {err:#}");
            Vec::new()
        }
    };

    // Convert EdgeRow to GraphEdge and compute edge type counts
    let edges: Vec<GraphEdge> = edge_rows
        .iter()
        .map(|e| edge_row_to_graph_edge(e))
        .collect();

    let edge_type_counts = compute_edge_type_counts(&edge_rows);

    // Collect all unique node UUIDs referenced in edges.
    // Resolve names directly from companies + persons so the frontend
    // never has to fall back to UUID codes regardless of load order.
    let node_ids: Vec<Uuid> = {
        let mut seen = std::collections::HashSet::new();
        for row in &edge_rows {
            seen.insert(row.source_id);
            seen.insert(row.target_id);
        }
        seen.into_iter().collect()
    };

    let node_labels: Vec<GraphNodeLabel> = if node_ids.is_empty() {
        Vec::new()
    } else {
        #[derive(sqlx::FromRow)]
        struct NodeNameRow {
            id: Uuid,
            label: String,
            node_type: String,
        }
        sqlx::query_as::<_, NodeNameRow>(
            r#"SELECT id, name AS label, 'company' AS node_type FROM companies WHERE id = ANY($1)
               UNION ALL
               SELECT id, name AS label, 'person' AS node_type FROM persons WHERE id = ANY($1)"#
        )
        .bind(&node_ids)
        .fetch_all(&state.store.pool)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|r| GraphNodeLabel { id: r.id.to_string(), label: r.label, node_type: r.node_type })
        .collect()
    };

    let payload = GraphOverviewWithEdges {
        companies_total,
        persons_total,
        warnings_total,
        insights_total,
        edges_total,
        nodes: node_labels,
        edges,
        edge_type_counts,
    };

    let duration_ms = start.elapsed().as_millis() as u64;
    let meta = ResponseMeta::now()
        .with_request_id(request_id)
        .with_duration(duration_ms);
    log_latency("list_graph", duration_ms);

    (StatusCode::OK, Json(success_with_meta(payload, meta)))
}

async fn list_recipes(
    State(state): State<AppState>,
    Query(params): Query<ListRecipesQuery>,
) -> (StatusCode, Json<ApiResponse<PagedResponse<RecipeListItem>>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let (page, per_page, _offset) = match validate_pagination(params.page, params.per_page) {
        Ok(p) => p,
        Err(err) => {
            return (
                StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::BAD_REQUEST),
                Json(error_response(err)),
            );
        }
    };

    // Get recipe statistics from recipes LEFT JOIN warnings
    let recipe_stats = match state.store.get_recipe_stats().await {
        Ok(stats) => {
            tracing::info!(request_id = %request_id, count = stats.len(), "recipe stats loaded");
            stats
        },
        Err(err) => {
            tracing::error!(request_id = %request_id, "get recipe stats failed: {err:#}");
            let api_err = ApiError::internal("Failed to load recipe statistics");
            return (
                StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(error_response(api_err)),
            );
        }
    };

    // Convert to RecipeListItem
    let mut items: Vec<RecipeListItem> = recipe_stats
        .into_iter()
        .map(|stat| recipe_stat_to_list_item(&stat))
        .collect();

    if let Some(status_raw) = params.status.as_deref() {
        let status = match parse_recipe_status(status_raw) {
            Some(status) => status,
            None => {
                let err = ApiError::validation("status", "expected one of: staging|production|deprecated");
                return (
                    StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::UNPROCESSABLE_ENTITY),
                    Json(error_response(err)),
                );
            }
        };
        items.retain(|item| item.status == status);
    }

    if let Some(search) = params.search.as_deref() {
        let search = search.trim().to_lowercase();
        if !search.is_empty() {
            items.retain(|item| {
                item.id.to_lowercase().contains(&search)
                    || item.name.to_lowercase().contains(&search)
                    || item.description.to_lowercase().contains(&search)
            });
        }
    }

    if let Some(min_precision) = params.min_precision {
        let min_precision = clamp_ratio(min_precision);
        items.retain(|item| item.precision >= min_precision);
    }

    if let Some(region) = params.region.as_deref() {
        let region = region.trim().to_lowercase();
        items.retain(|item| {
            item.region
                .as_deref()
                .map(|value| value.eq_ignore_ascii_case(&region))
                .unwrap_or(false)
        });
    }

    let sort_field = params.sort_by.clone().unwrap_or(RecipeSortField::CreatedAt);
    let sort_desc = !matches!(sort_field, RecipeSortField::Name);
    sort_recipes(&mut items, &sort_field, sort_desc);

    let total = items.len() as u64;
    let clamped_page = clamp_page(page, per_page, total);
    let offset = ((clamped_page - 1) as usize).saturating_mul(per_page as usize);
    let paged_items = items
        .into_iter()
        .skip(offset)
        .take(per_page as usize)
        .collect();

    let payload = PagedResponse {
        items: paged_items,
        total,
        page: clamped_page,
        per_page,
    };
    let duration_ms = start.elapsed().as_millis() as u64;
    let meta = ResponseMeta::now()
        .with_request_id(request_id)
        .with_duration(duration_ms);
    log_latency("list_recipes", duration_ms);

    (StatusCode::OK, Json(success_with_meta(payload, meta)))
}

async fn list_security(
    State(state): State<AppState>,
) -> (StatusCode, Json<ApiResponse<SecuritySummary>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();

    let dns_rows = state.store.get_dns_posture_entries(500).await.unwrap_or_default();
    let lookalike_rows = state.store.get_lookalike_domains(5000).await.unwrap_or_default();
    let kev_rows = state.store.get_kev_relevance(200).await.unwrap_or_default();

    let domains_monitored = dns_rows.len() as u64;
    let dns_scores: Vec<f64> = dns_rows.iter().map(|row| {
        let has_spf = row.value.get("has_spf").and_then(|v| v.as_bool()).unwrap_or(false);
        let has_dkim = row.value.get("has_dkim").and_then(|v| v.as_bool()).unwrap_or(false);
        let has_dmarc = row.value.get("has_dmarc").and_then(|v| v.as_bool()).unwrap_or(false);
        dns_score(has_spf, has_dkim, has_dmarc) * 100.0
    }).collect();
    let dns_posture_score = if dns_scores.is_empty() {
        0.0
    } else {
        dns_scores.iter().sum::<f64>() / dns_scores.len() as f64
    };

    let lookalike_domains_detected = lookalike_rows.len() as u64;
    let kev_matches = kev_rows.len() as u64;

    let last_scan_at = dns_rows.iter().chain(lookalike_rows.iter()).chain(kev_rows.iter())
        .map(|row| row.ts_utc)
        .max()
        .map(|ts| ts.to_rfc3339());

    let payload = SecuritySummary {
        dns_posture_score,
        lookalike_domains_detected,
        kev_matches,
        last_scan_at,
        domains_monitored,
    };
    let duration_ms = start.elapsed().as_millis() as u64;
    let meta = ResponseMeta::now()
        .with_request_id(request_id)
        .with_duration(duration_ms);
    log_latency("list_security", duration_ms);

    (StatusCode::OK, Json(success_with_meta(payload, meta)))
}

async fn build_state() -> Result<AppState> {
    dotenvy::dotenv().ok();
    
    // B294: Validate config at startup — fail fast if env vars are misconfigured
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
            "Refusing to start API server with invalid config. Fix the errors above and restart."
        );
    }
    tracing::info!("config validated successfully");

    #[cfg(feature = "llm")]
    let llm = build_llm_runtime(&config)?;

    let store = PgStore::connect(&config.database_url).await?;
    store.run_migrations().await?;

    let index_path = std::env::var("SEARCH_INDEX_PATH").unwrap_or_else(|_| "data/search".to_string());
    let search_index = SearchIndex::open(FsPath::new(&index_path))?;

    // API keys
    let api_keys = load_api_keys();
    tracing::info!("{} API keys loaded", api_keys.len());

    // Optional Redis for rate limiting
    let redis = match std::env::var("REDIS_URL") {
        Ok(url) if !url.trim().is_empty() => {
            match redis::Client::open(url.as_str()) {
                Ok(client) => match client.get_connection_manager().await {
                    Ok(mgr) => {
                        tracing::info!("Redis connected — rate limiting active");
                        Some(mgr)
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "Redis connection manager failed — rate limiting disabled");
                        None
                    }
                },
                Err(e) => {
                    tracing::warn!(error = %e, "Invalid REDIS_URL — rate limiting disabled");
                    None
                }
            }
        }
        _ => {
            tracing::info!("REDIS_URL not set — rate limiting running in no-op mode");
            None
        }
    };

    Ok(AppState {
        store: Arc::new(store),
        search_index: Arc::new(search_index),
        api_keys: Arc::new(api_keys),
        redis,
        #[cfg(feature = "llm")]
        llm,
    })
}

/// Load API keys from environment variables.
/// Format: API_KEY_1=<raw_key>,<display_name>,<role>
/// Example: API_KEY_1=sk-abc123,Admin Key,admin
fn load_api_keys() -> HashMap<String, ApiKey> {
    let mut registry = HashMap::new();
    for i in 1..=50 {
        let env_key = format!("API_KEY_{}", i);
        if let Ok(val) = std::env::var(&env_key) {
            let parts: Vec<&str> = val.splitn(3, ',').collect();
            if parts.len() >= 3 {
                let raw_key = parts[0].trim();
                let name = parts[1].trim();
                let role_str = parts[2].trim();
                let role = ApiRole::from_str(role_str).unwrap_or(ApiRole::Viewer);
                let key_id = format!("key-{}", i);
                let api_key = ApiKey {
                    key_id: key_id.clone(),
                    key_hash: auth::hash_api_key(raw_key),
                    name: name.to_string(),
                    role,
                    created_at: Utc::now(),
                    expires_at: None,
                    enabled: true,
                    rate_limit_per_min: 120,
                    allowed_origins: vec![],
                };
                registry.insert(key_id, api_key);
            } else {
                tracing::warn!("Invalid {} format — expected raw_key,name,role", env_key);
            }
        }
    }
    registry
}

fn pagination(page: Option<u32>, per_page: Option<u32>) -> (u32, u32, i64) {
    let page = page.unwrap_or(1).max(1);
    let per_page = per_page.unwrap_or(25).min(500).max(1);
    let offset = ((page - 1) as i64).saturating_mul(per_page as i64);
    (page, per_page, offset)
}

fn validate_pagination(page: Option<u32>, per_page: Option<u32>) -> Result<(u32, u32, i64), ApiError> {
    let page = page.unwrap_or(1);
    let per_page = per_page.unwrap_or(25);
    if page == 0 {
        return Err(ApiError::validation("page", "page must be >= 1"));
    }
    if per_page == 0 || per_page > 500 {
        return Err(ApiError::validation("per_page", "per_page must be in 1..=500"));
    }
    Ok(pagination(Some(page), Some(per_page)))
}

/// Clamp page to the valid range [1, total_pages] after the total is known.
fn clamp_page(page: u32, per_page: u32, total: u64) -> u32 {
    let total_pages = total
        .checked_add(per_page as u64 - 1)
        .unwrap_or(u64::MAX)
        / per_page.max(1) as u64;
    let total_pages = (total_pages.max(1)).min(u32::MAX as u64) as u32;
    page.clamp(1, total_pages)
}

#[allow(dead_code)]
fn parse_csv_upper(value: &Option<String>) -> Vec<String> {
    value
        .as_ref()
        .map(|raw| {
            raw.split(',')
                .map(|s| s.trim().to_uppercase())
                .filter(|s| !s.is_empty())
                .take(50)
                .collect()
        })
        .unwrap_or_default()
}

#[allow(dead_code)]
fn parse_csv_lower(value: &Option<String>) -> Vec<String> {
    value
        .as_ref()
        .map(|raw| {
            raw.split(',')
                .map(|s| s.trim().to_lowercase())
                .filter(|s| !s.is_empty())
                .take(50)
                .collect()
        })
        .unwrap_or_default()
}

fn parse_csv_upper_strict(
    value: &Option<String>,
    max_len: usize,
    field: &str,
) -> Result<Vec<String>, ApiError> {
    parse_csv_strict(value, max_len, field, true)
}

fn parse_csv_lower_strict(
    value: &Option<String>,
    max_len: usize,
    field: &str,
) -> Result<Vec<String>, ApiError> {
    parse_csv_strict(value, max_len, field, false)
}

fn parse_csv_strict(
    value: &Option<String>,
    max_len: usize,
    field: &str,
    upper: bool,
) -> Result<Vec<String>, ApiError> {
    let Some(raw) = value.as_ref() else { return Ok(Vec::new()); };
    let mut result = Vec::new();
    for token in raw.split(',') {
        let trimmed = token.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.chars().count() > max_len {
            return Err(ApiError::validation(field, format!("{} token too long", field)));
        }
        if !trimmed.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            return Err(ApiError::validation(field, format!("{} contains invalid characters", field)));
        }
        let normalized = if upper {
            trimmed.to_uppercase()
        } else {
            trimmed.to_lowercase()
        };
        result.push(normalized);
        if result.len() >= 50 {
            break;
        }
    }
    Ok(result)
}

fn validate_json_depth(value: &serde_json::Value, max_depth: usize) -> Result<(), String> {
    fn depth(value: &serde_json::Value, current: usize) -> usize {
        match value {
            serde_json::Value::Array(items) => items
                .iter()
                .map(|v| depth(v, current + 1))
                .max()
                .unwrap_or(current + 1),
            serde_json::Value::Object(map) => map
                .values()
                .map(|v| depth(v, current + 1))
                .max()
                .unwrap_or(current + 1),
            _ => current,
        }
    }
    let d = depth(value, 0);
    if d > max_depth {
        Err(format!("JSON payload too deep (max {})", max_depth))
    } else {
        Ok(())
    }
}

fn log_latency(endpoint: &str, duration_ms: u64) {
    let bucket = match duration_ms {
        0..=49 => "lt_50ms",
        50..=199 => "50_200ms",
        200..=499 => "200_500ms",
        500..=999 => "500_1000ms",
        1000..=2999 => "1_3s",
        _ => "gt_3s",
    };
    tracing::info!(endpoint = endpoint, duration_ms = duration_ms, bucket = bucket, "request_latency");
}

/// Safely truncate a UTF-8 string to at most `max_bytes` bytes without splitting multi-byte characters.
fn truncate_utf8_safe(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    // Find the largest char boundary <= max_bytes
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Extract readable text from observation value JSON for LLM context.
/// Avoids dumping raw JSON which confuses the model.
#[allow(dead_code)]
fn format_observation_for_llm(value: &serde_json::Value, provenance: &serde_json::Value) -> String {
    let mut parts = Vec::new();

    // Extract meaningful fields from value
    if let Some(obj) = value.as_object() {
        // Prefer excerpt, summary, text, or title
        if let Some(s) = obj.get("excerpt").and_then(|v| v.as_str()) {
            let clean = s.chars().take(200).collect::<String>();
            parts.push(format!("Excerpt: {}", clean));
        } else if let Some(s) = obj.get("summary").and_then(|v| v.as_str()) {
            let clean = s.chars().take(200).collect::<String>();
            parts.push(format!("Summary: {}", clean));
        } else if let Some(s) = obj.get("text").and_then(|v| v.as_str()) {
            let clean = s.chars().take(200).collect::<String>();
            parts.push(format!("Text: {}", clean));
        } else if let Some(s) = obj.get("title").and_then(|v| v.as_str()) {
            parts.push(format!("Title: {}", s));
        }
        // For web changes, describe the change type
        if let Some(ct) = obj.get("change_type").and_then(|v| v.as_str()) {
            parts.push(format!("Change: {}", ct.replace('_', " ")));
        }
        if let Some(sig) = obj.get("signals_detected").and_then(|v| v.as_i64()) {
            parts.push(format!("{} signals detected", sig));
        }
    } else if let Some(s) = value.as_str() {
        let clean = s.chars().take(200).collect::<String>();
        parts.push(clean);
    }

    // Extract URL from provenance
    if let Some(obj) = provenance.as_object() {
        if let Some(url) = obj.get("url").and_then(|v| v.as_str()) {
            parts.push(format!("URL: {}", url));
        }
    }

    if parts.is_empty() {
        // Fallback: brief JSON excerpt
        let json = serde_json::to_string(value).unwrap_or_default();
        truncate_utf8_safe(&json, 150).to_string()
    } else {
        parts.join(" | ")
    }
}

fn validate_region_codes(regions: &[String]) -> Result<(), ApiError> {
    for region in regions {
        if RegionFilter::from_code(region).is_none() {
            return Err(ApiError::validation("regions", format!("invalid region '{}'", region)));
        }
    }
    Ok(())
}

fn validate_severity_codes(severities: &[String]) -> Result<(), ApiError> {
    for sev in severities {
        if SeverityFilter::from_str_loose(sev).is_none() {
            return Err(ApiError::validation("severities", format!("invalid severity '{}'", sev)));
        }
    }
    Ok(())
}

fn validate_warning_type_codes(warning_types: &[String]) -> Result<(), ApiError> {
    for wt in warning_types {
        if WarningTypeFilter::from_str_loose(wt).is_none() {
            return Err(ApiError::validation("warning_types", format!("invalid warning type '{}'", wt)));
        }
    }
    Ok(())
}

fn parse_date_start(value: &Option<String>) -> Result<Option<DateTime<Utc>>, String> {
    let raw = match value.as_ref() {
        Some(r) => r,
        None => return Ok(None),
    };
    let date = NaiveDate::parse_from_str(raw, "%Y-%m-%d")
        .map_err(|_| format!("invalid date_from '{}': expected YYYY-MM-DD", raw))?;
    let dt = date
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| format!("invalid date_from '{}'", raw))?;
    Ok(Some(Utc.from_utc_datetime(&dt)))
}

fn parse_date_end(value: &Option<String>) -> Result<Option<DateTime<Utc>>, String> {
    let raw = match value.as_ref() {
        Some(r) => r,
        None => return Ok(None),
    };
    let date = NaiveDate::parse_from_str(raw, "%Y-%m-%d")
        .map_err(|_| format!("invalid date_to '{}': expected YYYY-MM-DD", raw))?;
    let dt = date
        .and_hms_opt(23, 59, 59)
        .ok_or_else(|| format!("invalid date_to '{}'", raw))?;
    Ok(Some(Utc.from_utc_datetime(&dt)))
}

/// Validate that date_from <= date_to when both are present.
fn validate_date_range(
    date_from: &Option<DateTime<Utc>>,
    date_to: &Option<DateTime<Utc>>,
) -> Result<(), String> {
    if let (Some(from), Some(to)) = (date_from, date_to) {
        if from > to {
            return Err("date_from must not be after date_to".to_string());
        }
    }
    let now = Utc::now();
    if let Some(from) = date_from {
        if *from > now + chrono::Duration::days(1) {
            return Err("date_from cannot be in the future".to_string());
        }
    }
    if let Some(to) = date_to {
        if *to > now + chrono::Duration::days(1) {
            return Err("date_to cannot be in the future".to_string());
        }
    }
    Ok(())
}

fn warning_row_to_response(row: WarningRow) -> WarningResponse {
    let ts = row.ts_utc;
    WarningResponse {
        id: row.id.to_string(),
        title: row.title,
        description: row.description.unwrap_or_default(),
        severity: row.severity,
        warning_type: row.warning_type,
        region: row.region.unwrap_or_default(),
        source_urls: row.source_urls.unwrap_or_default(),
        entity_ids: row
            .entity_ids
            .unwrap_or_default()
            .into_iter()
            .map(|id| id.to_string())
            .collect(),
        recipe_code: row.recipe_code,
        confidence: clamp_ratio(row.confidence.unwrap_or(0.0)),
        acknowledged: row.acknowledged,
        acknowledged_by: row.acknowledged_by,
        acknowledged_at: row.acknowledged_at,
        acknowledged_note: row.acknowledged_note,
        ts_utc: ts,
        created_at: row.created_at.unwrap_or(ts),
        updated_at: row.updated_at.unwrap_or(ts),
    }
}

fn insight_row_to_response(row: InsightRow) -> InsightResponse {
    InsightResponse {
        id: row.id.to_string(),
        title: row.title,
        summary: row.summary,
        insight_type: row.insight_type.unwrap_or_else(|| "general".to_string()),
        region: row.region.unwrap_or_default(),
        confidence: clamp_ratio(row.confidence.unwrap_or(0.0)),
        evidence_urls: row.evidence_urls.unwrap_or_default(),
        entity_ids: row
            .entity_ids
            .unwrap_or_default()
            .into_iter()
            .map(|id| id.to_string())
            .collect(),
        tags: row.tags.unwrap_or_default(),
        created_at: row.created_at.unwrap_or_else(Utc::now),
        updated_at: row.updated_at.unwrap_or_else(Utc::now),
        bookmarked: None,
    }
}

// ─── CSV Export handlers ─────────────────────────────────────────────────────

fn csv_escape(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

async fn export_companies_csv(
    State(state): State<AppState>,
) -> axum::response::Response {
    let filters = CompanyListFilters::default();
    let rows = state
        .store
        .list_companies(&filters, None, true, 10_000, 0)
        .await
        .unwrap_or_default();

    let mut csv = String::from("id,name,domain,region,country,entity_type,is_competitor,threat_score,capabilities,updated_at\n");
    for row in rows {
        let item = company_row_to_item(row);
        csv.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{}\n",
            csv_escape(&item.id),
            csv_escape(&item.name),
            csv_escape(item.domain.as_deref().unwrap_or("")),
            csv_escape(&item.region),
            csv_escape(&item.country),
            csv_escape(&item.entity_type),
            item.is_competitor,
            item.threat_score.map(|s| format!("{s:.4}")).unwrap_or_default(),
            csv_escape(&item.capabilities.join("|")),
            csv_escape(&item.updated_at.to_rfc3339()),
        ));
    }

    axum::response::Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/csv; charset=utf-8")
        .header(header::CONTENT_DISPOSITION, "attachment; filename=\"companies.csv\"")
        .body(axum::body::Body::from(csv))
        .unwrap()
}

async fn export_persons_csv(
    State(state): State<AppState>,
) -> axum::response::Response {
    let filters = PersonListFilters::default();
    let rows = state
        .store
        .list_persons(&filters, None, true, 10_000, 0)
        .await
        .unwrap_or_default();

    let mut csv = String::from("id,name,role,organization,region,priority_score,engagement_status,updated_at\n");
    for row in rows {
        let item = person_row_to_item(row);
        csv.push_str(&format!(
            "{},{},{},{},{},{},{},{}\n",
            csv_escape(&item.id),
            csv_escape(&item.name),
            csv_escape(&item.role),
            csv_escape(&item.organization),
            csv_escape(&item.region),
            format!("{:.4}", item.priority_score),
            csv_escape(&item.engagement_status),
            csv_escape(&item.updated_at.to_rfc3339()),
        ));
    }

    axum::response::Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/csv; charset=utf-8")
        .header(header::CONTENT_DISPOSITION, "attachment; filename=\"persons.csv\"")
        .body(axum::body::Body::from(csv))
        .unwrap()
}

async fn export_insights_csv(
    State(state): State<AppState>,
) -> axum::response::Response {
    let filters = InsightListFilters::default();
    let rows = state
        .store
        .list_insights(&filters, 10_000, 0)
        .await
        .unwrap_or_default();

    let mut csv = String::from("id,title,insight_type,summary,region,confidence,created_at\n");
    for row in rows {
        csv.push_str(&format!(
            "{},{},{},{},{},{},{}\n",
            csv_escape(&row.id.to_string()),
            csv_escape(&row.title),
            csv_escape(row.insight_type.as_deref().unwrap_or("")),
            csv_escape(&row.summary),
            csv_escape(row.region.as_deref().unwrap_or("")),
            row.confidence.map(|c| format!("{c:.4}")).unwrap_or_default(),
            csv_escape(&row.created_at.unwrap_or_else(Utc::now).to_rfc3339()),
        ));
    }

    axum::response::Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/csv; charset=utf-8")
        .header(header::CONTENT_DISPOSITION, "attachment; filename=\"insights.csv\"")
        .body(axum::body::Body::from(csv))
        .unwrap()
}

// ─────────────────────────────────────────────────────────────────────────────

fn company_row_to_item(row: CompanyRow) -> CompanyListItem {
    let is_competitor = company_is_competitor(&row);

    CompanyListItem {
        id: row.id.to_string(),
        name: row.name,
        domain: row.domain,
        region: row.region.unwrap_or_default(),
        country: row.country_code.unwrap_or_default(),
        entity_type: row.company_type.unwrap_or_else(|| "unknown".to_string()),
        is_competitor,
        threat_score: row.threat_score.map(clamp_ratio),
        capabilities: row.industry_tags.unwrap_or_default(),
        updated_at: row.updated_at.unwrap_or_else(Utc::now),
    }
}

fn person_row_to_item(row: PersonListRow) -> PersonListItem {
    // Use the same compute_priority_vector as the detail endpoint
    // so that list and detail scores are consistent.
    let influence_score = clamp_ratio(row.priority_score);
    let priority_vector = compute_priority_vector(
        Some(row.role_family.as_str()),
        Some(row.role.as_str()),
        &row.organization,
        influence_score,
        row.artifact_count.max(0) as usize,
    );
    let score = priority_vector.composite();
    let influence_score = (score * 100.0).round() as i64;
    let priority = if influence_score >= 80 {
        "A"
    } else if influence_score >= 50 {
        "B"
    } else {
        "C"
    }
    .to_string();
    let mut tags = vec![];
    if !row.role_family.trim().is_empty() {
        tags.push(row.role_family.clone());
    }
    if row.artifact_count > 0 {
        tags.push(format!("{} artifacts", row.artifact_count));
    }
    PersonListItem {
        id: row.id.to_string(),
        name: row.name,
        role: row.role,
        role_family: row.role_family,
        organization: row.organization,
        region: row.region,
        country: row.country_code,
        priority_score: score,
        influence_score,
        priority,
        influence_tier: priority_tier(score).to_string(),
        engagement_status: row.engagement_status,
        tags,
        last_signal: row.updated_at.format("%Y-%m-%d").to_string(),
        updated_at: row.updated_at,
    }
}

fn company_row_to_detail(
    row: CompanyRow,
    sites: Vec<SiteRow>,
    certifications: Vec<CertificationRow>,
    persons: Vec<PersonRow>,
) -> CompanyDetail {
    let is_competitor = company_is_competitor(&row);
    let mut capability_set = BTreeSet::new();
    if let Some(tags) = row.industry_tags.as_ref() {
        for tag in tags {
            capability_set.insert(tag.clone());
        }
    }
    for site in &sites {
        if let Some(caps) = site.capabilities.as_ref() {
            for cap in caps {
                capability_set.insert(cap.clone());
            }
        }
    }

    let mut cert_set = BTreeSet::new();
    for cert in certifications {
        cert_set.insert(cert.standard);
    }

    let city = metadata_string(&row.metadata, "city").or_else(|| {
        sites
            .iter()
            .filter_map(|site| site.city.as_ref())
            .map(|value| value.trim())
            .find(|value| !value.is_empty())
            .map(|value| value.to_string())
    });

    let sites: Vec<CompanySite> = sites
        .into_iter()
        .map(site_row_to_company_site)
        .collect();

    let key_persons: Vec<CompanyKeyPerson> = persons
        .into_iter()
        .map(person_row_to_key_person)
        .collect();

    let recent_events = metadata_company_events(&row.metadata);

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
        threat_score: row.threat_score.map(clamp_ratio),
        overlap_score: row.overlap_score.map(clamp_ratio),
        capabilities: capability_set.into_iter().collect(),
        certifications: cert_set.into_iter().collect(),
        sites,
        key_persons,
        recent_events,
        created_at: row.created_at.unwrap_or_else(Utc::now),
        updated_at: row.updated_at.unwrap_or_else(Utc::now),
    }
}

fn person_row_to_detail(
    row: PersonRow,
    organization: String,
    org_id: Option<Uuid>,
    artifacts: Vec<ArtifactRow>,
    role_history_rows: Vec<RoleHistoryRow>,
    peer_rows: Vec<PersonListRow>,
    warning_count: i64,
    insight_count: i64,
) -> PersonDetail {
    let role = row
        .current_role
        .clone()
        .or(row.role_family.clone())
        .unwrap_or_else(|| "Unknown".to_string());
    let role_family = row.role_family.clone().unwrap_or_else(|| "Unknown".to_string());
    let influence_score = clamp_ratio(row.influence_score.unwrap_or(0.0));
    let artifact_count = artifacts.len();

    // Always compute differentiated priority vector based on POI characteristics
    let priority_vector = compute_priority_vector(
        row.role_family.as_deref(),
        row.current_role.as_deref(),
        &organization,
        influence_score,
        artifact_count,
    );
    let priority_score = priority_vector.composite();
    let influence_score_100 = (priority_score * 100.0).round() as i64;
    let priority = if influence_score_100 >= 80 {
        "A"
    } else if influence_score_100 >= 50 {
        "B"
    } else {
        "C"
    }
    .to_string();

    let mut affiliations = metadata_affiliations(&row.metadata);
    if affiliations.is_empty() && !organization.is_empty() && organization != "Independent" {
        affiliations.push(Affiliation {
            organization: organization.clone(),
            role: role.clone(),
            current: true,
        });
    }

    let timeline: Vec<PersonEvent> = artifacts
        .into_iter()
        .map(|artifact| PersonEvent {
            event_type: artifact.artifact_type,
            description: artifact
                .title
                .or(artifact.content_summary)
                .unwrap_or_else(|| "Artifact".to_string()),
            date: artifact.ts_utc,
            source_url: Some(artifact.url),
        })
        .collect();

    // Extract tags: prefer DB trigger_topics, fallback to metadata
    let tags = if let Some(ref topics) = row.trigger_topics {
        if !topics.is_empty() {
            topics.clone()
        } else {
            metadata_string_vec(&row.metadata, "tags")
        }
    } else {
        metadata_string_vec(&row.metadata, "tags")
    };

    // Use public_email from DB column, fallback to metadata
    let email = row.public_email.clone().or_else(|| metadata_string(&row.metadata, "email"));

    // Build alternative names
    let mut name_alt = Vec::new();
    if let Some(ref ar) = row.name_ar {
        if !ar.is_empty() { name_alt.push(ar.clone()); }
    }
    if let Some(ref fr) = row.name_fr {
        if !fr.is_empty() { name_alt.push(fr.clone()); }
    }

    // Compute data completeness score (0.0 - 1.0)
    let data_completeness = compute_data_completeness(&row, &email, &timeline, &affiliations);

    // Compute engagement readiness score (0.0 - 1.0)
    let engagement_readiness = compute_engagement_readiness(
        &row.decision_style, &row.risk_tolerance, &row.change_appetite,
        &row.communication_style, data_completeness, influence_score,
    );

    // Build role history entries
    let role_history: Vec<RoleHistoryEntry> = role_history_rows.into_iter().map(|rh| {
        let is_current = rh.end_date.is_none();
        RoleHistoryEntry {
            organization: rh.org_name,
            role: rh.title,
            role_family: rh.role_family,
            start_date: rh.start_date.map(|d| d.format("%Y-%m-%d").to_string()),
            end_date: rh.end_date.map(|d| d.format("%Y-%m-%d").to_string()),
            is_current,
            confidence: rh.confidence,
        }
    }).collect();

    // Build peer summaries
    let peers: Vec<PeerSummary> = peer_rows.into_iter().take(6).map(|p| {
        let influence_score = clamp_ratio(p.priority_score);
        let pv = compute_priority_vector(
            Some(p.role_family.as_str()),
            Some(p.role.as_str()),
            &p.organization,
            influence_score,
            p.artifact_count.max(0) as usize,
        );
        let score = pv.composite();
        PeerSummary {
            id: p.id.to_string(),
            name: p.name,
            role: p.role,
            organization: p.organization,
            region: p.region,
            priority_score: score,
            influence_tier: priority_tier(score).to_string(),
        }
    }).collect();

    let engagement_status = metadata_engagement_status(&row.metadata);

    PersonDetail {
        id: row.id.to_string(),
        name: row.name,
        name_alt,
        role,
        role_family,
        organization,
        org_id: org_id.map(|id| id.to_string()),
        region: row.region.unwrap_or_default(),
        country: row.country_code.unwrap_or_default(),
        bio: row.public_bio,
        email,
        phone: metadata_string(&row.metadata, "phone"),
        linkedin: metadata_string(&row.metadata, "linkedin"),
        priority_score,
        influence_score: influence_score_100,
        priority,
        priority_vector,
        influence_tier: priority_tier(priority_score).to_string(),
        engagement_status,
        engagement_readiness: clamp_ratio(engagement_readiness),
        data_completeness: clamp_ratio(data_completeness),
        tags: tags.clone(),
        trigger_topics: row.trigger_topics.unwrap_or_default(),
        decision_style: row.decision_style,
        risk_tolerance: row.risk_tolerance,
        change_appetite: row.change_appetite,
        communication_style: row.communication_style,
        decision_mode: row.decision_mode,
        preferred_proof_type: row.preferred_proof_type,
        pain_index: row.pain_index.map(clamp_ratio),
        change_risk: row.change_risk.map(clamp_ratio),
        role_drift_score: row.role_drift_score.map(clamp_ratio),
        affiliations,
        timeline,
        role_history,
        peers,
        warning_count,
        insight_count,
        created_at: row.created_at.unwrap_or_else(Utc::now),
        updated_at: row.updated_at.unwrap_or_else(Utc::now),
    }
}

/// Compute data completeness as a ratio of populated fields.
fn compute_data_completeness(
    row: &PersonRow,
    email: &Option<String>,
    timeline: &[PersonEvent],
    affiliations: &[Affiliation],
) -> f64 {
    let total = 14.0_f64;
    let mut filled = 0.0_f64;
    if row.public_bio.is_some() { filled += 1.0; }
    if email.is_some() { filled += 1.0; }
    if row.trigger_topics.as_ref().map(|t| !t.is_empty()).unwrap_or(false) { filled += 1.0; }
    if row.decision_style.is_some() { filled += 1.0; }
    if row.risk_tolerance.is_some() { filled += 1.0; }
    if row.change_appetite.is_some() { filled += 1.0; }
    if row.communication_style.is_some() { filled += 1.0; }
    if row.decision_mode.is_some() { filled += 1.0; }
    if row.preferred_proof_type.is_some() { filled += 1.0; }
    if row.pain_index.map(|v| v > 0.0).unwrap_or(false) { filled += 1.0; }
    if row.change_risk.map(|v| v > 0.0).unwrap_or(false) { filled += 1.0; }
    if !timeline.is_empty() { filled += 1.0; }
    if !affiliations.is_empty() { filled += 1.0; }
    if row.region.is_some() { filled += 1.0; }
    filled / total
}

/// Compute engagement readiness based on behavioral and data signals.
fn compute_engagement_readiness(
    _decision_style: &Option<String>,
    risk_tolerance: &Option<String>,
    change_appetite: &Option<String>,
    communication_style: &Option<String>,
    data_completeness: f64,
    influence_score: f64,
) -> f64 {
    let mut score = 0.0_f64;
    // Data availability (40% weight)
    score += data_completeness * 0.40;
    // Risk tolerance signal (20% weight)
    if let Some(rt) = risk_tolerance {
        score += match rt.to_lowercase().as_str() {
            "high" => 0.20,
            "moderate" => 0.14,
            "low" => 0.08,
            _ => 0.10,
        };
    }
    // Change appetite signal (20% weight)
    if let Some(ca) = change_appetite {
        score += match ca.to_lowercase().as_str() {
            "high" => 0.20,
            "moderate" => 0.14,
            "low" => 0.08,
            _ => 0.10,
        };
    }
    // Communication accessibility (10% weight)
    if let Some(cs) = communication_style {
        score += match cs.to_lowercase().as_str() {
            "collaborative" | "solution-focused" => 0.10,
            "narrative-driven" | "stakeholder-oriented" => 0.08,
            "data-first" | "direct & concise" => 0.07,
            "formal & procedural" | "balanced" => 0.05,
            _ => 0.05,
        };
    }
    // Influence adjusts final score (10% weight)
    score += influence_score * 0.10;
    score
}

fn company_is_competitor(row: &CompanyRow) -> bool {
    // First, exclude non-competitor entity types (regardless of metadata flag)
    // Government entities, trade associations, and banks are never competitors.
    let excluded_types = [
        "government",
        "trade_association",
        "bank",
        "regulatory",
        "ngo",
        "international_org",
    ];
    
    if let Some(ref ctype) = row.company_type {
        let ctype_lower = ctype.to_lowercase();
        if excluded_types.iter().any(|&excluded| ctype_lower.contains(excluded)) {
            return false;
        }
    }
    
    // For business types, check the explicit is_competitor flag in metadata
    row.metadata
        .as_ref()
        .and_then(|meta| meta.get("is_competitor"))
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

fn site_row_to_company_site(row: SiteRow) -> CompanySite {
    let location = site_location(&row);
    let site_type = row.site_type.unwrap_or_else(|| "site".to_string());
    CompanySite {
        name: row.name,
        location,
        site_type,
    }
}

fn site_location(row: &SiteRow) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(address) = row.address.as_ref().filter(|v| !v.trim().is_empty()) {
        parts.push(address.to_string());
    }
    if let Some(city) = row.city.as_ref().filter(|v| !v.trim().is_empty()) {
        parts.push(city.to_string());
    }
    if let Some(region) = row.region.as_ref().filter(|v| !v.trim().is_empty()) {
        parts.push(region.to_string());
    }
    if let Some(country) = row.country_code.as_ref().filter(|v| !v.trim().is_empty()) {
        parts.push(country.to_string());
    }
    if parts.is_empty() {
        "Unknown".to_string()
    } else {
        parts.join(", ")
    }
}

fn person_row_to_key_person(row: PersonRow) -> CompanyKeyPerson {
    CompanyKeyPerson {
        person_id: row.id.to_string(),
        name: row.name,
        role: row
            .current_role
            .or(row.role_family)
            .unwrap_or_else(|| "Unknown".to_string()),
    }
}

/// Compute a differentiated priority vector based on POI characteristics.
/// Each dimension uses different signals to produce nuanced scores.
fn compute_priority_vector(
    role_family: Option<&str>,
    current_role: Option<&str>,
    organization: &str,
    influence_score: f64,
    artifact_count: usize,
) -> PriorityVector {
    // Decision Power: Based on role seniority
    let decision_power = compute_decision_power(role_family, current_role);
    
    // Domain Relevance: Based on organization type and role alignment
    let domain_relevance = compute_domain_relevance(role_family, organization, current_role);
    
    // Network Centrality: Based on influence_score with variance
    let network_centrality = compute_network_centrality(influence_score, artifact_count);
    
    // Engagement Potential: Based on data availability and accessibility signals
    let engagement_potential = compute_engagement_potential(artifact_count, organization, current_role);
    
    // Intelligence Value: Composite of decision power and domain relevance
    let intelligence_value = compute_intelligence_value(decision_power, domain_relevance, influence_score);
    
    PriorityVector {
        decision_power: clamp_ratio(decision_power),
        domain_relevance: clamp_ratio(domain_relevance),
        network_centrality: clamp_ratio(network_centrality),
        engagement_potential: clamp_ratio(engagement_potential),
        intelligence_value: clamp_ratio(intelligence_value),
    }
}

/// Compute decision power based on role seniority.
fn compute_decision_power(role_family: Option<&str>, current_role: Option<&str>) -> f64 {
    let role_lower = current_role.map(|r| r.to_lowercase()).unwrap_or_default();
    let family_lower = role_family.map(|r| r.to_lowercase()).unwrap_or_default();
    
    // Check for executive indicators in role title
    let is_ceo = role_lower.contains("ceo") || role_lower.contains("chief executive");
    let is_chairman = role_lower.contains("chairman") || role_lower.contains("chairwoman");
    let is_cxo = role_lower.contains("cfo") || role_lower.contains("cto") || role_lower.contains("coo") 
        || role_lower.contains("chief") || role_lower.contains("c-level");
    let is_minister = role_lower.contains("minister") || role_lower.contains("secretary");
    let is_director_general = role_lower.contains("director general") || role_lower.contains("director-general");
    let is_president = role_lower.contains("president") && !role_lower.contains("vice");
    let is_vp = role_lower.contains("vice president") || role_lower.contains("vp ");
    let is_svp = role_lower.contains("senior vice president") || role_lower.contains("svp");
    let is_evp = role_lower.contains("executive vice president") || role_lower.contains("evp");
    let is_director = role_lower.contains("director") && !role_lower.contains("general");
    let is_head = role_lower.contains("head of") || role_lower.contains("head,");
    let is_manager = role_lower.contains("manager") || role_lower.contains("lead");
    let is_governor = role_lower.contains("governor");
    let is_commissioner = role_lower.contains("commissioner");
    
    // Assign base score by role level
    let base = if is_ceo || is_chairman || is_minister || is_director_general || is_governor || is_commissioner {
        0.95
    } else if is_cxo || is_president {
        0.90
    } else if is_evp || is_svp {
        0.82
    } else if is_vp {
        0.75
    } else if is_director || is_head {
        0.65
    } else if is_manager {
        0.50
    } else {
        // Fall back to role_family
        match family_lower.as_str() {
            "c-suite" | "executive" => 0.88,
            "government" | "eu commission" | "minister" => 0.85,
            "vp" | "vice president" => 0.72,
            "director" | "board" => 0.62,
            "manager" | "senior" => 0.48,
            "analyst" | "specialist" => 0.35,
            "consultant" | "independent" => 0.40,
            _ => 0.45,
        }
    };
    
    // Add small variance based on hash of role string for unique values
    let variance = role_variance(&role_lower) * 0.08;
    base + variance
}

/// Compute domain relevance based on organization type and role.
fn compute_domain_relevance(role_family: Option<&str>, organization: &str, current_role: Option<&str>) -> f64 {
    let org_lower = organization.to_lowercase();
    let role_lower = current_role.map(|r| r.to_lowercase()).unwrap_or_default();
    let family_lower = role_family.map(|r| r.to_lowercase()).unwrap_or_default();
    
    // High relevance for core industry players
    let is_ems = org_lower.contains("flex") || org_lower.contains("jabil") || org_lower.contains("foxconn")
        || org_lower.contains("celestica") || org_lower.contains("sanmina") || org_lower.contains("wistron")
        || org_lower.contains("pegatron") || org_lower.contains("incap") || org_lower.contains("kitron")
        || org_lower.contains("zollner") || org_lower.contains("hanza") || org_lower.contains("ems")
        || org_lower.contains("electronics") || org_lower.contains("asteelflash");
    
    let is_semiconductor = org_lower.contains("stmicro") || org_lower.contains("infineon")
        || org_lower.contains("nxp") || org_lower.contains("intel") || org_lower.contains("amd")
        || org_lower.contains("tsmc") || org_lower.contains("semiconductor");
    
    let is_government = org_lower.contains("government") || org_lower.contains("ministry")
        || org_lower.contains("commission") || family_lower.contains("government");
    
    let is_defense = org_lower.contains("defense") || org_lower.contains("defence")
        || org_lower.contains("military") || org_lower.contains("nato")
        || role_lower.contains("defense") || role_lower.contains("defence");
    
    let is_investment = org_lower.contains("bank") || org_lower.contains("investment")
        || org_lower.contains("bpifrance") || org_lower.contains("ebrd") || org_lower.contains("eib");
    
    let is_trade_assoc = org_lower.contains("cgem") || org_lower.contains("amica")
        || org_lower.contains("utica") || org_lower.contains("conect") || org_lower.contains("association");
    
    let is_independent = org_lower.contains("independent") || org_lower == "independent";
    
    let base = if is_ems {
        0.92
    } else if is_semiconductor {
        0.95
    } else if is_defense {
        0.88
    } else if is_government {
        0.80
    } else if is_investment {
        0.72
    } else if is_trade_assoc {
        0.65
    } else if is_independent {
        0.45
    } else {
        0.60
    };
    
    // Adjust for role relevance
    let role_boost = if role_lower.contains("supply chain") || role_lower.contains("procurement") {
        0.12
    } else if role_lower.contains("manufacturing") || role_lower.contains("operations") {
        0.10
    } else if role_lower.contains("technology") || role_lower.contains("engineering") {
        0.08
    } else if role_lower.contains("sales") || role_lower.contains("business development") {
        0.06
    } else {
        0.0
    };
    
    let variance = role_variance(&org_lower) * 0.06;
    base + role_boost + variance
}

/// Compute network centrality from influence score with variance.
fn compute_network_centrality(influence_score: f64, artifact_count: usize) -> f64 {
    // Start from influence score
    let base = influence_score;
    
    // Boost for having more artifacts (indicates network visibility)
    let artifact_boost = (artifact_count as f64 / 50.0).min(0.15);
    
    // Add variance
    let variance = (influence_score * 100.0) as u64 % 13;
    let variance_factor = (variance as f64 - 6.0) / 100.0;
    
    base + artifact_boost + variance_factor
}

/// Compute engagement potential based on data availability.
fn compute_engagement_potential(artifact_count: usize, organization: &str, current_role: Option<&str>) -> f64 {
    let org_lower = organization.to_lowercase();
    let role_lower = current_role.map(|r| r.to_lowercase()).unwrap_or_default();
    
    // More artifacts = more publicly active = more accessible
    let data_score = match artifact_count {
        0 => 0.25,
        1..=2 => 0.40,
        3..=5 => 0.55,
        6..=10 => 0.68,
        11..=20 => 0.78,
        _ => 0.88,
    };
    
    // Public sector officials are harder to engage directly
    let sector_adj = if org_lower.contains("government") || org_lower.contains("ministry") {
        -0.12
    } else if org_lower.contains("independent") {
        0.08  // consultants are accessible
    } else {
        0.0
    };
    
    // C-suite harder to access than mid-level
    let role_adj = if role_lower.contains("ceo") || role_lower.contains("president") 
        || role_lower.contains("minister") || role_lower.contains("chairman") {
        -0.10
    } else if role_lower.contains("manager") || role_lower.contains("director") {
        0.05
    } else {
        0.0
    };
    
    let variance = role_variance(&format!("{}_{}", org_lower, role_lower)) * 0.07;
    data_score + sector_adj + role_adj + variance
}

/// Compute intelligence value as weighted composite.
fn compute_intelligence_value(decision_power: f64, domain_relevance: f64, influence_score: f64) -> f64 {
    // Weighted combination with slight variance
    let base = decision_power * 0.35 + domain_relevance * 0.35 + influence_score * 0.30;
    let variance = ((decision_power * 1000.0) as u64 % 11) as f64 / 150.0;
    base + variance
}

/// Generate consistent variance from string hash (deterministic).
fn role_variance(s: &str) -> f64 {
    let hash: u64 = s.bytes().fold(0u64, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u64));
    ((hash % 100) as f64 - 50.0) / 100.0  // Returns -0.5 to +0.5
}

fn metadata_string(meta: &Option<serde_json::Value>, key: &str) -> Option<String> {
    meta.as_ref()
        .and_then(|value| value.get(key))
        .and_then(|value| value.as_str())
        .map(|value| value.to_string())
}

fn metadata_string_vec(meta: &Option<serde_json::Value>, key: &str) -> Vec<String> {
    meta.as_ref()
        .and_then(|value| value.get(key))
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(|value| value.to_string()))
                .collect::<Vec<String>>()
        })
        .unwrap_or_default()
}

fn metadata_affiliations(meta: &Option<serde_json::Value>) -> Vec<Affiliation> {
    let Some(items) = meta.as_ref().and_then(|value| value.get("affiliations")).and_then(|value| value.as_array()) else {
        return Vec::new();
    };

    items
        .iter()
        .filter_map(|item| {
            let obj = item.as_object()?;
            let organization = obj.get("organization")?.as_str()?;
            let role = obj.get("role").and_then(|value| value.as_str()).unwrap_or("Unknown");
            let current = obj.get("current").and_then(|value| value.as_bool()).unwrap_or(false);
            Some(Affiliation {
                organization: organization.to_string(),
                role: role.to_string(),
                current,
            })
        })
        .collect()
}

fn metadata_engagement_status(meta: &Option<serde_json::Value>) -> String {
    meta.as_ref()
        .and_then(|value| value.get("engagement_status"))
        .and_then(|value| value.as_str())
        .unwrap_or("untracked")
        .to_string()
}

fn metadata_company_events(meta: &Option<serde_json::Value>) -> Vec<CompanyEvent> {
    let Some(items) = meta
        .as_ref()
        .and_then(|value| value.get("recent_events"))
        .and_then(|value| value.as_array())
    else {
        return Vec::new();
    };

    items
        .iter()
        .filter_map(|item| {
            let obj = item.as_object()?;
            let description = obj.get("description")?.as_str()?.trim().to_string();
            if description.is_empty() {
                return None;
            }

            let event_type = obj
                .get("event_type")
                .and_then(|value| value.as_str())
                .unwrap_or("update")
                .to_string();

            let source_url = obj
                .get("source_url")
                .and_then(|value| value.as_str())
                .map(|value| value.to_string());

            let date = obj
                .get("date")
                .and_then(|value| value.as_str())
                .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
                .map(|value| value.with_timezone(&Utc))
                .unwrap_or_else(Utc::now);

            Some(CompanyEvent {
                event_type,
                description,
                date,
                source_url,
            })
        })
        .collect()
}

fn map_warning_sort(sort: WarningSortField) -> WarningOrderBy {
    match sort {
        WarningSortField::CreatedAt => WarningOrderBy::CreatedAt,
        WarningSortField::Severity => WarningOrderBy::Severity,
        WarningSortField::Type => WarningOrderBy::WarningType,
    }
}

fn map_company_sort(sort: CompanySortField) -> CompanyOrderBy {
    match sort {
        CompanySortField::Name => CompanyOrderBy::Name,
        CompanySortField::Region => CompanyOrderBy::Region,
        CompanySortField::ThreatScore => CompanyOrderBy::ThreatScore,
        CompanySortField::UpdatedAt => CompanyOrderBy::UpdatedAt,
    }
}

fn map_person_sort(sort: PersonSortField) -> PersonOrderBy {
    match sort {
        PersonSortField::Name => PersonOrderBy::Name,
        PersonSortField::Priority => PersonOrderBy::Priority,
        PersonSortField::Region => PersonOrderBy::Region,
        PersonSortField::UpdatedAt => PersonOrderBy::UpdatedAt,
    }
}

// ────────────────────────────────────────────
// Graph & Recipe Mapping
// ────────────────────────────────────────────

fn edge_row_to_graph_edge(row: &EdgeRow) -> GraphEdge {
    GraphEdge {
        source: row.source_id.to_string(),
        target: row.target_id.to_string(),
        edge_type: row.edge_type.clone(),
        weight: row.weight.unwrap_or(1.0),
        label: Some(format!("{} → {}", row.source_type, row.target_type)),
    }
}

fn compute_edge_type_counts(rows: &[EdgeRow]) -> Vec<EdgeTypeCount> {
    let mut counts: HashMap<String, u64> = HashMap::new();
    for row in rows {
        *counts.entry(row.edge_type.clone()).or_insert(0) += 1;
    }
    let mut result: Vec<EdgeTypeCount> = counts
        .into_iter()
        .map(|(edge_type, count)| EdgeTypeCount { edge_type, count })
        .collect();
    result.sort_by(|a, b| b.count.cmp(&a.count));
    result
}

fn recipe_stat_to_list_item(stat: &RecipeStatRow) -> RecipeListItem {
    // Generate a deterministic ID from recipe_code
    let id = format!("recipe-{}", stat.recipe_code.to_lowercase().replace('_', "-"));
    
    // Format name from recipe code (e.g., "TECH_CONVERGENCE" → "Tech Convergence")
    let name = stat.recipe_code
        .split('_')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().chain(chars.flat_map(|c| c.to_lowercase())).collect(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ");

    // Generate a description
    let description = if stat.fired_count > 0 {
        format!(
            "Signal recipe that has fired {} times. {} currently active.",
            stat.fired_count, stat.active_count
        )
    } else {
        "Signal recipe deployed and monitoring. No signals detected yet.".to_string()
    };

    let fired_count = stat.fired_count.max(0) as f64;
    let active_count = stat.active_count.max(0) as f64;
    let inactive_count = (fired_count - active_count).max(0.0);
    let precision = if fired_count > 0.0 {
        (inactive_count / fired_count).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let false_positive_rate = if fired_count > 0.0 {
        (active_count / fired_count).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let activity_signal = (fired_count / 100.0).clamp(0.0, 1.0);
    let recall = (0.7 * precision + 0.3 * activity_signal).clamp(0.0, 1.0);

    let status = if fired_count > 0.0 && active_count > 0.0 {
        RecipeStatus::Production
    } else {
        RecipeStatus::Staging
    };

    RecipeListItem {
        id,
        name,
        description,
        status,
        region: None,
        precision,
        recall,
        false_positive_rate,
        fired_count: stat.fired_count.max(0) as u32,
        last_fired: stat.last_fired,
        created_at: stat.first_fired.unwrap_or_else(Utc::now),
        updated_at: stat.last_fired.unwrap_or_else(Utc::now),
    }
}

fn parse_recipe_status(raw: &str) -> Option<RecipeStatus> {
    match raw.trim().to_lowercase().as_str() {
        "staging" => Some(RecipeStatus::Staging),
        "production" => Some(RecipeStatus::Production),
        "deprecated" => Some(RecipeStatus::Deprecated),
        _ => None,
    }
}

// ─── New entity list endpoints ─────────────────────────────────────────────

#[derive(Debug, serde::Deserialize)]
struct ListSitesQuery {
    page: Option<u32>,
    per_page: Option<u32>,
    company_id: Option<String>,
    region: Option<String>,
}

async fn list_sites(
    State(state): State<AppState>,
    Query(params): Query<ListSitesQuery>,
) -> (StatusCode, Json<ApiResponse<PagedResponse<SiteRow>>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let (page, per_page, _) = match validate_pagination(params.page, params.per_page) {
        Ok(p) => p,
        Err(err) => return (StatusCode::BAD_REQUEST, Json(error_response(err))),
    };
    let company_id = params.company_id.as_deref().and_then(|s| Uuid::parse_str(s).ok());
    let region = params.region.as_deref();
    let total = state.store.count_sites(company_id, region).await.unwrap_or(0).max(0) as u64;
    let clamped_page = clamp_page(page, per_page, total);
    let offset = ((clamped_page - 1) as i64) * (per_page as i64);
    let items = state.store.list_sites(company_id, region, per_page as i64, offset).await.unwrap_or_default();
    let payload = PagedResponse { items, total, page: clamped_page, per_page };
    let dur = start.elapsed().as_millis() as u64;
    log_latency("list_sites", dur);
    (StatusCode::OK, Json(success_with_meta(payload, ResponseMeta::now().with_request_id(request_id).with_duration(dur))))
}

#[derive(Debug, serde::Deserialize)]
struct ListCapabilitiesQuery {
    page: Option<u32>,
    per_page: Option<u32>,
    company_id: Option<String>,
}

async fn list_capabilities(
    State(state): State<AppState>,
    Query(params): Query<ListCapabilitiesQuery>,
) -> (StatusCode, Json<ApiResponse<PagedResponse<CapabilityRow>>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let (page, per_page, _) = match validate_pagination(params.page, params.per_page) {
        Ok(p) => p,
        Err(err) => return (StatusCode::BAD_REQUEST, Json(error_response(err))),
    };
    let company_id = params.company_id.as_deref().and_then(|s| Uuid::parse_str(s).ok());
    let total = state.store.count_capabilities(company_id).await.unwrap_or(0).max(0) as u64;
    let clamped_page = clamp_page(page, per_page, total);
    let offset = ((clamped_page - 1) as i64) * (per_page as i64);
    let items = state.store.list_capabilities(company_id, per_page as i64, offset).await.unwrap_or_default();
    let payload = PagedResponse { items, total, page: clamped_page, per_page };
    let dur = start.elapsed().as_millis() as u64;
    log_latency("list_capabilities", dur);
    (StatusCode::OK, Json(success_with_meta(payload, ResponseMeta::now().with_request_id(request_id).with_duration(dur))))
}

#[derive(Debug, serde::Deserialize)]
struct ListCertificationsQuery {
    page: Option<u32>,
    per_page: Option<u32>,
    company_id: Option<String>,
}

async fn list_certifications_all(
    State(state): State<AppState>,
    Query(params): Query<ListCertificationsQuery>,
) -> (StatusCode, Json<ApiResponse<PagedResponse<CertificationRow>>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let (page, per_page, _) = match validate_pagination(params.page, params.per_page) {
        Ok(p) => p,
        Err(err) => return (StatusCode::BAD_REQUEST, Json(error_response(err))),
    };
    let company_id = params.company_id.as_deref().and_then(|s| Uuid::parse_str(s).ok());
    let total = state.store.count_certifications(company_id).await.unwrap_or(0).max(0) as u64;
    let clamped_page = clamp_page(page, per_page, total);
    let offset = ((clamped_page - 1) as i64) * (per_page as i64);
    let items = state.store.list_certifications(company_id, per_page as i64, offset).await.unwrap_or_default();
    let payload = PagedResponse { items, total, page: clamped_page, per_page };
    let dur = start.elapsed().as_millis() as u64;
    log_latency("list_certifications", dur);
    (StatusCode::OK, Json(success_with_meta(payload, ResponseMeta::now().with_request_id(request_id).with_duration(dur))))
}

#[derive(Debug, serde::Deserialize)]
struct ListObservationsQuery {
    page: Option<u32>,
    per_page: Option<u32>,
    entity_id: Option<String>,
    observation_type: Option<String>,
}

async fn list_observations(
    State(state): State<AppState>,
    Query(params): Query<ListObservationsQuery>,
) -> (StatusCode, Json<ApiResponse<PagedResponse<ObservationRow>>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let (page, per_page, _) = match validate_pagination(params.page, params.per_page) {
        Ok(p) => p,
        Err(err) => return (StatusCode::BAD_REQUEST, Json(error_response(err))),
    };
    let entity_id = params.entity_id.as_deref().and_then(|s| Uuid::parse_str(s).ok());
    let obs_type = params.observation_type.as_deref();
    let total = state.store.count_observations(entity_id, obs_type).await.unwrap_or(0).max(0) as u64;
    let clamped_page = clamp_page(page, per_page, total);
    let offset = ((clamped_page - 1) as i64) * (per_page as i64);
    let items = state.store.list_observations(entity_id, obs_type, per_page as i64, offset).await.unwrap_or_default();
    let payload = PagedResponse { items, total, page: clamped_page, per_page };
    let dur = start.elapsed().as_millis() as u64;
    log_latency("list_observations", dur);
    (StatusCode::OK, Json(success_with_meta(payload, ResponseMeta::now().with_request_id(request_id).with_duration(dur))))
}

#[derive(Debug, serde::Deserialize)]
struct ListProductFamiliesQuery {
    page: Option<u32>,
    per_page: Option<u32>,
    company_id: Option<String>,
}

async fn list_product_families(
    State(state): State<AppState>,
    Query(params): Query<ListProductFamiliesQuery>,
) -> (StatusCode, Json<ApiResponse<PagedResponse<ProductFamilyRow>>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let (page, per_page, _) = match validate_pagination(params.page, params.per_page) {
        Ok(p) => p,
        Err(err) => return (StatusCode::BAD_REQUEST, Json(error_response(err))),
    };
    let company_id = params.company_id.as_deref().and_then(|s| Uuid::parse_str(s).ok());
    let total = state.store.count_product_families(company_id).await.unwrap_or(0).max(0) as u64;
    let clamped_page = clamp_page(page, per_page, total);
    let offset = ((clamped_page - 1) as i64) * (per_page as i64);
    let items = state.store.list_product_families(company_id, per_page as i64, offset).await.unwrap_or_default();
    let payload = PagedResponse { items, total, page: clamped_page, per_page };
    let dur = start.elapsed().as_millis() as u64;
    log_latency("list_product_families", dur);
    (StatusCode::OK, Json(success_with_meta(payload, ResponseMeta::now().with_request_id(request_id).with_duration(dur))))
}

#[derive(Debug, serde::Deserialize)]
struct ListLogisticsNodesQuery {
    page: Option<u32>,
    per_page: Option<u32>,
    country_code: Option<String>,
}

async fn list_logistics_nodes(
    State(state): State<AppState>,
    Query(params): Query<ListLogisticsNodesQuery>,
) -> (StatusCode, Json<ApiResponse<PagedResponse<LogisticsNodeRow>>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let (page, per_page, _) = match validate_pagination(params.page, params.per_page) {
        Ok(p) => p,
        Err(err) => return (StatusCode::BAD_REQUEST, Json(error_response(err))),
    };
    let cc = params.country_code.as_deref();
    let total = state.store.count_logistics_nodes(cc).await.unwrap_or(0).max(0) as u64;
    let clamped_page = clamp_page(page, per_page, total);
    let offset = ((clamped_page - 1) as i64) * (per_page as i64);
    let items = state.store.list_logistics_nodes(cc, per_page as i64, offset).await.unwrap_or_default();
    let payload = PagedResponse { items, total, page: clamped_page, per_page };
    let dur = start.elapsed().as_millis() as u64;
    log_latency("list_logistics_nodes", dur);
    (StatusCode::OK, Json(success_with_meta(payload, ResponseMeta::now().with_request_id(request_id).with_duration(dur))))
}

#[derive(Debug, serde::Deserialize)]
struct ListRegulationsQuery {
    page: Option<u32>,
    per_page: Option<u32>,
    jurisdiction: Option<String>,
}

async fn list_regulations(
    State(state): State<AppState>,
    Query(params): Query<ListRegulationsQuery>,
) -> (StatusCode, Json<ApiResponse<PagedResponse<RegulationRow>>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let (page, per_page, _) = match validate_pagination(params.page, params.per_page) {
        Ok(p) => p,
        Err(err) => return (StatusCode::BAD_REQUEST, Json(error_response(err))),
    };
    let j = params.jurisdiction.as_deref();
    let total = state.store.count_regulations(j).await.unwrap_or(0).max(0) as u64;
    let clamped_page = clamp_page(page, per_page, total);
    let offset = ((clamped_page - 1) as i64) * (per_page as i64);
    let items = state.store.list_regulations(j, per_page as i64, offset).await.unwrap_or_default();
    let payload = PagedResponse { items, total, page: clamped_page, per_page };
    let dur = start.elapsed().as_millis() as u64;
    log_latency("list_regulations", dur);
    (StatusCode::OK, Json(success_with_meta(payload, ResponseMeta::now().with_request_id(request_id).with_duration(dur))))
}

#[derive(Debug, serde::Deserialize)]
struct ListPoiArtifactsQuery {
    page: Option<u32>,
    per_page: Option<u32>,
    person_id: Option<String>,
}

async fn list_poi_artifacts(
    State(state): State<AppState>,
    Query(params): Query<ListPoiArtifactsQuery>,
) -> (StatusCode, Json<ApiResponse<PagedResponse<ArtifactRow>>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let (page, per_page, _) = match validate_pagination(params.page, params.per_page) {
        Ok(p) => p,
        Err(err) => return (StatusCode::BAD_REQUEST, Json(error_response(err))),
    };
    let person_id = params.person_id.as_deref().and_then(|s| Uuid::parse_str(s).ok());
    let total = state.store.count_poi_artifacts(person_id).await.unwrap_or(0).max(0) as u64;
    let clamped_page = clamp_page(page, per_page, total);
    let offset = ((clamped_page - 1) as i64) * (per_page as i64);
    let items = state.store.list_poi_artifacts(person_id, per_page as i64, offset).await.unwrap_or_default();
    let payload = PagedResponse { items, total, page: clamped_page, per_page };
    let dur = start.elapsed().as_millis() as u64;
    log_latency("list_poi_artifacts", dur);
    (StatusCode::OK, Json(success_with_meta(payload, ResponseMeta::now().with_request_id(request_id).with_duration(dur))))
}

async fn get_dashboard(
    State(state): State<AppState>,
) -> (StatusCode, Json<ApiResponse<DashboardStats>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let stats = match state.store.get_dashboard_stats().await {
        Ok(s) => s,
        Err(err) => {
            tracing::error!(request_id = %request_id, "dashboard stats failed: {err:#}");
            let api_err = ApiError::internal("Failed to load dashboard stats");
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(error_response(api_err)));
        }
    };
    let dur = start.elapsed().as_millis() as u64;
    log_latency("get_dashboard", dur);
    (StatusCode::OK, Json(success_with_meta(stats, ResponseMeta::now().with_request_id(request_id).with_duration(dur))))
}

// ─── Warning detail ──────────────────────────────────────

async fn get_warning_detail(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<WarningResponse>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let uid = match Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => return (StatusCode::BAD_REQUEST, Json(error_response(ApiError::bad_request("Invalid UUID")))),
    };
    let warning = match state.store.get_warning(uid).await {
        Ok(Some(w)) => w,
        Ok(None) => return (StatusCode::NOT_FOUND, Json(error_response(ApiError::not_found("Warning", &id)))),
        Err(err) => {
            tracing::error!(request_id = %request_id, "get_warning failed: {err:#}");
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(error_response(ApiError::internal("Failed to load warning"))));
        }
    };
    let response = warning_row_to_response(warning);
    let dur = start.elapsed().as_millis() as u64;
    log_latency("get_warning_detail", dur);
    (StatusCode::OK, Json(success_with_meta(response, ResponseMeta::now().with_request_id(request_id).with_duration(dur))))
}

// ─── Insight detail ──────────────────────────────────────

async fn get_insight_detail(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let uid = match Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => return (StatusCode::BAD_REQUEST, Json(error_response(ApiError::bad_request("Invalid UUID")))),
    };
    let insight = match state.store.get_insight(uid).await {
        Ok(Some(i)) => i,
        Ok(None) => return (StatusCode::NOT_FOUND, Json(error_response(ApiError::not_found("Insight", &id)))),
        Err(err) => {
            tracing::error!(request_id = %request_id, "get_insight failed: {err:#}");
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(error_response(ApiError::internal("Failed to load insight"))));
        }
    };

    // Resolve entity names
    let entity_ids: Vec<Uuid> = insight.entity_ids.clone().unwrap_or_default();
    let company_names = state.store.get_company_names_by_ids(&entity_ids).await.unwrap_or_default();

    // Fetch observations for associated entities (multi-source evidence)
    let mut all_observations = Vec::new();
    for eid in &entity_ids {
        let obs = state.store.get_observations_by_entity(*eid, 20).await.unwrap_or_default();
        all_observations.extend(obs);
    }
    all_observations.sort_by(|a, b| b.ts_utc.cmp(&a.ts_utc));
    all_observations.truncate(50);

    // Fetch related warnings for the same entities
    let related_warnings = state.store.get_warnings_by_entity_ids(&entity_ids, 10).await.unwrap_or_default();

    // Fetch related insights (different from this one)
    let related_insights = state.store.get_related_insights(&entity_ids, uid, 10).await.unwrap_or_default();

    // Serialize observations for response
    let obs_json: Vec<serde_json::Value> = all_observations.iter().map(|o| {
        serde_json::json!({
            "id": o.id,
            "observation_type": o.observation_type,
            "entity_id": o.entity_id,
            "ts_utc": o.ts_utc,
            "value": o.value,
            "provenance": o.provenance,
            "confidence": o.confidence,
        })
    }).collect();

    let warnings_json: Vec<serde_json::Value> = related_warnings.iter().map(|w| {
        serde_json::json!({
            "id": w.id,
            "title": w.title,
            "severity": w.severity,
            "warning_type": w.warning_type,
            "region": w.region,
            "confidence": w.confidence,
            "created_at": w.created_at,
        })
    }).collect();

    let related_json: Vec<serde_json::Value> = related_insights.iter().map(|i| {
        serde_json::json!({
            "id": i.id,
            "title": i.title,
            "insight_type": i.insight_type,
            "region": i.region,
            "confidence": i.confidence,
            "created_at": i.created_at,
        })
    }).collect();

    let entities_json: Vec<serde_json::Value> = company_names.iter().map(|(id, name, region, _)| {
        serde_json::json!({
            "id": id,
            "name": name,
            "region": region,
        })
    }).collect();

    let detail = serde_json::json!({
        "id": insight.id,
        "title": insight.title,
        "summary": insight.summary,
        "insight_type": insight.insight_type,
        "region": insight.region,
        "confidence": insight.confidence,
        "evidence_urls": insight.evidence_urls,
        "entity_ids": insight.entity_ids.clone().unwrap_or_default().into_iter().map(|id| id.to_string()).collect::<Vec<_>>(),
        "tags": insight.tags,
        "created_at": insight.created_at.unwrap_or_else(Utc::now),
        "updated_at": insight.updated_at.unwrap_or_else(Utc::now),
        "entities": entities_json,
        "observations": obs_json,
        "related_warnings": warnings_json,
        "related_insights": related_json,
        "source_count": insight.evidence_urls.as_ref().map(|v| v.len()).unwrap_or(0),
        "observation_count": all_observations.len(),
        "bookmarked": state.store.is_insight_bookmarked(uid, "default").await.unwrap_or(false),
    });

    let dur = start.elapsed().as_millis() as u64;
    log_latency("get_insight_detail", dur);
    (StatusCode::OK, Json(success_with_meta(detail, ResponseMeta::now().with_request_id(request_id).with_duration(dur))))
}

// ─── Insight Bookmarks ──────────────────────────────────────────

async fn bookmark_insight(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let uid = match Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => return (StatusCode::BAD_REQUEST, Json(error_response(ApiError::bad_request("Invalid UUID")))),
    };
    // Verify insight exists
    match state.store.get_insight(uid).await {
        Ok(Some(_)) => {}
        Ok(None) => return (StatusCode::NOT_FOUND, Json(error_response(ApiError::not_found("Insight", &id)))),
        Err(err) => {
            tracing::error!("bookmark lookup failed: {err:#}");
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(error_response(ApiError::internal("Failed to check insight"))));
        }
    }
    match state.store.bookmark_insight(uid, "default", None).await {
        Ok(created) => {
            let status = if created { "created" } else { "already_bookmarked" };
            let meta = ResponseMeta::now();
            (StatusCode::OK, Json(success_with_meta(serde_json::json!({ "status": status, "insight_id": id, "bookmarked": true }), meta)))
        }
        Err(err) => {
            tracing::error!("bookmark_insight failed: {err:#}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(error_response(ApiError::internal("Failed to bookmark insight"))))
        }
    }
}

async fn unbookmark_insight(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let uid = match Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => return (StatusCode::BAD_REQUEST, Json(error_response(ApiError::bad_request("Invalid UUID")))),
    };
    match state.store.unbookmark_insight(uid, "default").await {
        Ok(_removed) => {
            let meta = ResponseMeta::now();
            (StatusCode::OK, Json(success_with_meta(serde_json::json!({ "status": "removed", "insight_id": id, "bookmarked": false }), meta)))
        }
        Err(err) => {
            tracing::error!("unbookmark_insight failed: {err:#}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(error_response(ApiError::internal("Failed to unbookmark insight"))))
        }
    }
}

// ─── LLM Insight Analysis ──────────────────────────────────────

/// Deep LLM-powered analysis for a single insight.
/// This endpoint fetches all associated data (observations, entities, warnings)
/// and asks the LLM to produce a sophisticated multi-source analysis.
#[cfg(feature = "llm")]
async fn analyze_insight(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let uid = match Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => return (StatusCode::BAD_REQUEST, Json(error_response(ApiError::bad_request("Invalid UUID")))),
    };

    let runtime = match state.llm.as_ref() {
        Some(rt) => rt,
        None => return llm_service_unavailable("LLM not configured"),
    };

    // Fetch the insight
    let insight = match state.store.get_insight(uid).await {
        Ok(Some(i)) => i,
        Ok(None) => return (StatusCode::NOT_FOUND, Json(error_response(ApiError::not_found("Insight", &id)))),
        Err(err) => {
            tracing::error!(request_id = %request_id, "analyze_insight: get_insight failed: {err:#}");
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(error_response(ApiError::internal("Failed to load insight"))));
        }
    };

    let entity_ids: Vec<Uuid> = insight.entity_ids.clone().unwrap_or_default();
    let company_names = state.store.get_company_names_by_ids(&entity_ids).await.unwrap_or_default();
    let entity_names: Vec<String> = company_names.iter().map(|(_, name, _, _)| name.clone()).collect();

    // Gather observations (multi-source evidence)
    let mut all_observations = Vec::new();
    for eid in &entity_ids {
        let obs = state.store.get_observations_by_entity(*eid, 30).await.unwrap_or_default();
        all_observations.extend(obs);
    }
    all_observations.sort_by(|a, b| b.ts_utc.cmp(&a.ts_utc));
    all_observations.truncate(40);

    // Deduplicate observations by type to reduce prompt repetition
    {
        let mut seen_types = std::collections::HashSet::new();
        all_observations.retain(|obs| {
            let key = format!("{}:{}", obs.observation_type, obs.value.to_string().chars().take(80).collect::<String>());
            seen_types.insert(key)
        });
    }

    // Gather related warnings
    let related_warnings = state.store.get_warnings_by_entity_ids(&entity_ids, 10).await.unwrap_or_default();

    // Gather related insights
    let related_insights = state.store.get_related_insights(&entity_ids, uid, 5).await.unwrap_or_default();

    // Build COMPACT context — 7B model needs a short prompt for quality output
    let evidence_urls = insight.evidence_urls.clone().unwrap_or_default();
    let source_count = evidence_urls.len();

    let mut context_parts: Vec<String> = Vec::new();

    // Compact observations — max 6, 100 chars each
    if !all_observations.is_empty() {
        context_parts.push(format!("DATA ({} observations):", all_observations.len()));
        for (i, obs) in all_observations.iter().take(6).enumerate() {
            let text = obs.value.get("excerpt")
                .or(obs.value.get("text"))
                .or(obs.value.get("summary"))
                .or(obs.value.get("title"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let truncated: String = text.chars().take(100).collect();
            context_parts.push(format!(
                "[O{}] {} — {}",
                i + 1, obs.observation_type, truncated
            ));
        }
    }

    // Compact warnings — max 3, title only
    for (i, w) in related_warnings.iter().take(3).enumerate() {
        context_parts.push(format!("[W{}] {} ({})", i + 1, w.title, w.severity));
    }

    let context_block = context_parts.join("\n");
    let entity_names_str = if entity_names.is_empty() { "unspecified entities".to_string() } else { entity_names.join(", ") };
    let region = insight.region.as_deref().unwrap_or("Global");
    let insight_type = insight.insight_type.as_deref().unwrap_or("general");

    // ------------------------------------------------------------------
    // Two-phase approach for 7B models:
    //   Phase 1: generate analysis as free text (the model is much better at this)
    //   Phase 2: structure the text into JSON server-side
    // ------------------------------------------------------------------
    let system_prompt = concat!(
        "You are a senior OSINT intelligence analyst specializing in electronics, defense, and supply chains. ",
        "Write a brief analytical report. Be specific — name companies, products, events, and dates. ",
        "Cite data references like [O1], [W1] where relevant."
    );

    // Build a condensed summary from the insight, max 400 chars
    let summary_truncated: String = insight.summary.chars().take(400).collect();

    let user_prompt = format!(
        r#"Write a 4-paragraph intelligence analysis.

SUBJECT: {entities} ({insight_type}, {region})
CONFIDENCE: {confidence:.0}%

BRIEFING: {summary}

{context}

Write EXACTLY 4 paragraphs, each on a new line:
1. SITUATION: What is happening and why it matters (3-4 sentences)
2. ANALYSIS: What the data tells us — correlate observations, identify patterns (3-4 sentences)
3. RISK: What could go wrong, which sectors are affected, timeline (2-3 sentences)
4. ACTION: Specific recommendations and what to monitor (2-3 sentences)"#,
        entities = entity_names_str,
        insight_type = insight_type,
        region = region,
        confidence = insight.confidence.unwrap_or(0.0) * 100.0,
        summary = summary_truncated,
        context = context_block,
    );

    let mut model_config = runtime.primary.clone();
    model_config.temperature = 0.5;
    model_config.max_tokens = 1024;
    model_config.timeout_seconds = 300;
    let client = OpenAiCompatibleClient::new(model_config);

    // Generate free text (not JSON) — 7B models produce much better text than structured JSON
    let raw_analysis = match client.generate_text(&system_prompt, &user_prompt).await {
        Ok(text) => text,
        Err(err) => {
            tracing::error!(request_id = %request_id, "LLM analysis failed: {err:#}");
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(error_response(ApiError::internal("LLM analysis failed"))));
        }
    };

    // Parse paragraphs — try double-newline first, then single-newline
    let paragraphs: Vec<String> = {
        let double_split: Vec<&str> = raw_analysis.split("\n\n")
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();
        if double_split.len() >= 4 {
            double_split.into_iter().map(|s| s.replace('\n', " ")).collect()
        } else {
            raw_analysis.split('\n')
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect()
        }
    };

    // Strip common paragraph labels (handles "1. SITUATION: ..." or "SITUATION: ...")
    fn strip_label(s: &str) -> String {
        let mut result = s.to_string();
        // Strip leading number + dot: "1. " "2. " etc
        if result.len() > 2 && result.as_bytes()[0].is_ascii_digit() && result.as_bytes()[1] == b'.' {
            result = result[2..].trim().to_string();
        }
        // Strip keyword labels
        let labels = ["SITUATION:", "ANALYSIS:", "RISK:", "ACTION:",
                       "THREAT:", "EVIDENCE:", "IMPACT:", "RESPONSE:",
                       "**SITUATION:**", "**ANALYSIS:**", "**RISK:**", "**ACTION:**"];
        for p in labels {
            if result.starts_with(p) {
                result = result[p.len()..].trim().to_string();
                break;
            }
        }
        result
    }

    let exec_summary = strip_label(paragraphs.get(0).map(|s| s.as_str()).unwrap_or("Analysis unavailable."));
    let detailed = strip_label(paragraphs.get(1).map(|s| s.as_str()).unwrap_or(""));
    let risk_assessment_text = strip_label(paragraphs.get(2).map(|s| s.as_str()).unwrap_or(""));
    let recommendations_text = strip_label(paragraphs.get(3).map(|s| s.as_str()).unwrap_or(""));

    // Determine risk level from text keywords
    let risk_lower = risk_assessment_text.to_lowercase();
    let risk_level = if risk_lower.contains("critical") || risk_lower.contains("severe") { "critical" }
        else if risk_lower.contains("high") || risk_lower.contains("significant") { "high" }
        else if risk_lower.contains("low") || risk_lower.contains("minimal") { "low" }
        else { "medium" };

    // Compute server-side metadata
    let source_diversity = if source_count >= 8 { "excellent" }
        else if source_count >= 5 { "good" }
        else if source_count >= 3 { "moderate" }
        else { "limited" };

    let data_sufficiency = if source_count >= 6 && all_observations.len() >= 5 { "strong" }
        else if source_count >= 3 { "adequate" }
        else if source_count >= 1 { "limited" }
        else { "insufficient" };

    let overall_confidence = insight.confidence.unwrap_or(0.0);

    let analysis = serde_json::json!({
        "executive_summary": exec_summary,
        "key_findings": [{
            "finding": detailed.chars().take(200).collect::<String>(),
            "evidence": format!("{} observations, {} sources", all_observations.len(), source_count),
            "confidence": overall_confidence,
            "impact": risk_level,
        }],
        "detailed_analysis": detailed,
        "source_analysis": {
            "total_sources": source_count,
            "observation_signals": all_observations.len(),
            "corroborating_sources": std::cmp::max(1, source_count.saturating_sub(1)),
            "contradicting_signals": 0,
            "source_diversity_assessment": source_diversity,
        },
        "risk_assessment": {
            "overall_risk": risk_level,
            "probability": overall_confidence,
            "time_horizon": "near_term",
            "affected_sectors": entity_names,
            "escalation_potential": risk_assessment_text,
        },
        "correlations": if detailed.len() > 10 { vec![detailed.clone()] } else { vec![] },
        "recommendations": [{
            "action": recommendations_text.chars().take(200).collect::<String>(),
            "priority": if risk_level == "critical" || risk_level == "high" { "high" } else { "medium" },
            "rationale": format!("Based on {} observations from {} sources at {:.0}% confidence",
                all_observations.len(), source_count, overall_confidence * 100.0),
        }],
        "monitoring_indicators": if !recommendations_text.is_empty() {
            vec![recommendations_text.clone()]
        } else {
            vec![format!("Monitor {} for further developments", entity_names_str)]
        },
        "analytical_confidence": {
            "overall": overall_confidence,
            "data_sufficiency": data_sufficiency,
            "key_uncertainties": [],
        },
    });

    let result = serde_json::json!({
        "insight_id": insight.id,
        "insight_title": insight.title,
        "analysis": analysis,
        "context_used": {
            "source_count": source_count,
            "observation_count": all_observations.len(),
            "warning_count": related_warnings.len(),
            "related_insight_count": related_insights.len(),
            "entity_count": entity_ids.len(),
        }
    });

    let dur = start.elapsed().as_millis() as u64;
    log_latency("analyze_insight", dur);
    (StatusCode::OK, Json(success_with_meta(result, ResponseMeta::now().with_request_id(request_id).with_duration(dur))))
}

#[cfg(not(feature = "llm"))]
async fn analyze_insight(
    State(_state): State<AppState>,
    Path(_id): Path<String>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    llm_service_unavailable("LLM feature disabled")
}

// ─── LLM Warning Analysis ──────────────────────────────────────

/// Deep LLM-powered analysis for a single warning.
#[cfg(feature = "llm")]
async fn analyze_warning(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let uid = match Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => return (StatusCode::BAD_REQUEST, Json(error_response(ApiError::bad_request("Invalid UUID")))),
    };

    let runtime = match state.llm.as_ref() {
        Some(rt) => rt,
        None => return llm_service_unavailable("LLM not configured"),
    };

    let warning = match state.store.get_warning(uid).await {
        Ok(Some(w)) => w,
        Ok(None) => return (StatusCode::NOT_FOUND, Json(error_response(ApiError::not_found("Warning", &id)))),
        Err(err) => {
            tracing::error!(request_id = %request_id, "analyze_warning: get_warning failed: {err:#}");
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(error_response(ApiError::internal("Failed to load warning"))));
        }
    };

    let entity_ids: Vec<Uuid> = warning.entity_ids.clone().unwrap_or_default();
    let company_names = state.store.get_company_names_by_ids(&entity_ids).await.unwrap_or_default();
    let entity_names: Vec<String> = company_names.iter().map(|(_, name, _, _)| name.clone()).collect();

    // Gather observations
    let mut all_observations = Vec::new();
    for eid in &entity_ids {
        let obs = state.store.get_observations_by_entity(*eid, 30).await.unwrap_or_default();
        all_observations.extend(obs);
    }
    all_observations.sort_by(|a, b| b.ts_utc.cmp(&a.ts_utc));
    all_observations.truncate(40);

    // Deduplicate observations by type to reduce prompt repetition
    {
        let mut seen_types = std::collections::HashSet::new();
        all_observations.retain(|obs| {
            let key = format!("{}:{}", obs.observation_type, obs.value.to_string().chars().take(80).collect::<String>());
            seen_types.insert(key)
        });
    }

    // Gather related insights
    let related_insights = state.store.get_insights_by_entity_ids(&entity_ids, 10).await.unwrap_or_default();

    let source_urls = warning.source_urls.clone().unwrap_or_default();
    let source_count = source_urls.len();

    let mut context_parts: Vec<String> = Vec::new();

    // Compact observations — max 6, 100 chars each
    if !all_observations.is_empty() {
        context_parts.push(format!("DATA ({} observations):", all_observations.len()));
        for (i, obs) in all_observations.iter().take(6).enumerate() {
            let text = obs.value.get("excerpt")
                .or(obs.value.get("text"))
                .or(obs.value.get("summary"))
                .or(obs.value.get("title"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let truncated: String = text.chars().take(100).collect();
            context_parts.push(format!(
                "[O{}] {} — {}",
                i + 1, obs.observation_type, truncated
            ));
        }
    }

    // Compact related insights — max 3, title only
    for (i, ri) in related_insights.iter().take(3).enumerate() {
        context_parts.push(format!("[I{}] {}", i + 1, ri.title));
    }

    let context_block = context_parts.join("\n");
    let entity_names_str = if entity_names.is_empty() { "unspecified entities".to_string() } else { entity_names.join(", ") };
    let region = warning.region.as_deref().unwrap_or("Global");

    let system_prompt = concat!(
        "You are a senior threat analyst specializing in electronics, defense, and supply chains. ",
        "Write a brief threat assessment. Be specific — name companies, products, events, and dates. ",
        "Cite data references like [O1], [I1] where relevant."
    );

    let desc_truncated: String = warning.description.as_deref().unwrap_or("").chars().take(300).collect();

    let user_prompt = format!(
        r#"Write a 4-paragraph threat assessment.

WARNING: {title} ({severity} severity)
Type: {warning_type} | Region: {region} | Confidence: {confidence:.0}%
Entities: {entities}

DESCRIPTION: {description}

{context}

Write EXACTLY 4 paragraphs, each on a new line:
1. THREAT: What the threat is and who is affected (3-4 sentences)
2. EVIDENCE: What data supports this assessment, citing observations (3-4 sentences)
3. IMPACT: Business, operational, and financial consequences with timeline (2-3 sentences)
4. RESPONSE: Specific mitigation actions and escalation triggers (2-3 sentences)"#,
        title = warning.title,
        warning_type = warning.warning_type,
        severity = warning.severity,
        region = region,
        confidence = warning.confidence.unwrap_or(0.0) * 100.0,
        description = desc_truncated,
        entities = entity_names_str,
        context = context_block,
    );

    let mut model_config = runtime.primary.clone();
    model_config.temperature = 0.5;
    model_config.max_tokens = 1024;
    model_config.timeout_seconds = 300;
    let client = OpenAiCompatibleClient::new(model_config);

    // Generate free text — 7B models produce better prose than structured JSON
    let raw_analysis = match client.generate_text(&system_prompt, &user_prompt).await {
        Ok(text) => text,
        Err(err) => {
            tracing::error!(request_id = %request_id, "LLM warning analysis failed: {err:#}");
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(error_response(ApiError::internal("LLM analysis failed"))));
        }
    };

    // Parse 4-paragraph text into structured fields
    let paragraphs: Vec<&str> = raw_analysis
        .split('\n')
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();

    fn strip_warning_label(s: &str) -> String {
        let patterns = ["THREAT:", "EVIDENCE:", "IMPACT:", "RESPONSE:", "1.", "2.", "3.", "4."];
        let mut result = s.to_string();
        for p in patterns {
            if result.starts_with(p) {
                result = result[p.len()..].trim().to_string();
                break;
            }
        }
        result
    }

    let threat_text = strip_warning_label(paragraphs.get(0).unwrap_or(&"Assessment unavailable."));
    let evidence_text = strip_warning_label(paragraphs.get(1).unwrap_or(&""));
    let impact_text = strip_warning_label(paragraphs.get(2).unwrap_or(&""));
    let response_text = strip_warning_label(paragraphs.get(3).unwrap_or(&""));

    // Compute source metadata server-side
    let source_reliability = if source_count >= 5 { "high" }
        else if source_count >= 2 { "medium" }
        else { "low" };

    let overall_confidence = warning.confidence.unwrap_or(0.0);
    let data_sufficiency = if source_count >= 4 && all_observations.len() >= 3 { "strong" }
        else if source_count >= 2 { "adequate" }
        else { "limited" };

    let analysis = serde_json::json!({
        "threat_assessment": threat_text,
        "severity_justification": evidence_text,
        "key_indicators": [{
            "indicator": evidence_text.chars().take(150).collect::<String>(),
            "evidence": format!("{} observations, {} sources", all_observations.len(), source_count),
            "severity_contribution": warning.severity,
        }],
        "detailed_analysis": format!("{} {}", threat_text, evidence_text),
        "source_analysis": {
            "total_sources": source_count,
            "observation_signals": all_observations.len(),
            "corroborating_sources": std::cmp::max(1, source_count.saturating_sub(1)),
            "source_reliability": source_reliability,
        },
        "impact_assessment": {
            "business_impact": impact_text.chars().take(200).collect::<String>(),
            "operational_impact": impact_text,
            "financial_exposure": if warning.severity == "critical" { "high" } else { "moderate" },
            "timeline": "near_term",
        },
        "response_plan": [{
            "action": response_text.chars().take(200).collect::<String>(),
            "priority": if warning.severity == "critical" { "critical" } else { "high" },
            "owner": "security/risk team",
            "rationale": format!("Based on {} severity warning at {:.0}% confidence",
                warning.severity, overall_confidence * 100.0),
        }],
        "escalation_criteria": if !response_text.is_empty() { vec![response_text.clone()] } else { vec![] },
        "monitoring_indicators": if !impact_text.is_empty() {
            vec![format!("Monitor for: {}", impact_text.chars().take(100).collect::<String>())]
        } else {
            vec![format!("Monitor {} for further developments", entity_names_str)]
        },
        "analytical_confidence": {
            "overall": overall_confidence,
            "data_sufficiency": data_sufficiency,
            "key_assumptions": if paragraphs.len() > 4 {
                paragraphs[4..].iter().map(|s| strip_warning_label(s)).collect::<Vec<_>>()
            } else {
                vec![]
            },
        },
    });

    let result = serde_json::json!({
        "warning_id": warning.id,
        "warning_title": warning.title,
        "analysis": analysis,
        "context_used": {
            "source_count": source_count,
            "observation_count": all_observations.len(),
            "related_insight_count": related_insights.len(),
            "entity_count": entity_ids.len(),
        }
    });

    let dur = start.elapsed().as_millis() as u64;
    log_latency("analyze_warning", dur);
    (StatusCode::OK, Json(success_with_meta(result, ResponseMeta::now().with_request_id(request_id).with_duration(dur))))
}

#[cfg(not(feature = "llm"))]
async fn analyze_warning(
    State(_state): State<AppState>,
    Path(_id): Path<String>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    llm_service_unavailable("LLM feature disabled")
}

// ─── Weekly memo ──────────────────────────────────────

#[derive(Debug, serde::Deserialize)]
struct ListMemosQuery {
    page: Option<u64>,
    per_page: Option<u64>,
}

async fn get_weekly_memo(
    State(state): State<AppState>,
) -> (StatusCode, Json<ApiResponse<WeeklyMemo>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    match state.store.get_weekly_memo_full().await {
        Ok(Some(memo)) => {
            let dur = start.elapsed().as_millis() as u64;
            log_latency("get_weekly_memo", dur);
            (StatusCode::OK, Json(success_with_meta(memo, ResponseMeta::now().with_request_id(request_id).with_duration(dur))))
        }
        Ok(None) => {
            (StatusCode::NOT_FOUND, Json(error_response(ApiError::not_found("WeeklyMemo", "latest"))))
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "weekly memo failed: {err:#}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(error_response(ApiError::internal("Failed to generate weekly memo"))))
        }
    }
}

async fn list_memos(
    State(state): State<AppState>,
    Query(params): Query<ListMemosQuery>,
) -> (StatusCode, Json<ApiResponse<PagedResponse<WeeklyMemo>>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let per_page = params.per_page.unwrap_or(20).min(100) as i64;
    let page = params.page.unwrap_or(1).max(1) as i64;
    let offset = (page - 1) * per_page;

    match state.store.list_weekly_memos(per_page, offset).await {
        Ok((memos, total)) => {
            let dur = start.elapsed().as_millis() as u64;
            log_latency("list_memos", dur);
            let payload = PagedResponse {
                items: memos,
                total: total as u64,
                page: page as u32,
                per_page: per_page as u32,
            };
            (StatusCode::OK, Json(success_with_meta(payload, ResponseMeta::now().with_request_id(request_id).with_duration(dur))))
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "list memos failed: {err:#}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(error_response(ApiError::internal("Failed to list memos"))))
        }
    }
}

// ─── Company dossier ──────────────────────────────────

async fn get_company_dossier(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<CompanyDossier>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let uid = match Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => return (StatusCode::BAD_REQUEST, Json(error_response(ApiError::bad_request("Invalid UUID")))),
    };
    match state.store.get_company_dossier(uid).await {
        Ok(Some(d)) => {
            let dur = start.elapsed().as_millis() as u64;
            log_latency("get_company_dossier", dur);
            (StatusCode::OK, Json(success_with_meta(d, ResponseMeta::now().with_request_id(request_id).with_duration(dur))))
        }
        Ok(None) => (StatusCode::NOT_FOUND, Json(error_response(ApiError::not_found("Company", &id)))),
        Err(err) => {
            tracing::error!(request_id = %request_id, "company dossier failed: {err:#}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(error_response(ApiError::internal("Failed to load company dossier"))))
        }
    }
}

// ─── Person dossier & engagement ──────────────────────

async fn get_person_dossier(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<PersonDossier>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let uid = match Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => return (StatusCode::BAD_REQUEST, Json(error_response(ApiError::bad_request("Invalid UUID")))),
    };
    match state.store.get_person_dossier(uid).await {
        Ok(Some(d)) => {
            let dur = start.elapsed().as_millis() as u64;
            log_latency("get_person_dossier", dur);
            (StatusCode::OK, Json(success_with_meta(d, ResponseMeta::now().with_request_id(request_id).with_duration(dur))))
        }
        Ok(None) => (StatusCode::NOT_FOUND, Json(error_response(ApiError::not_found("Person", &id)))),
        Err(err) => {
            tracing::error!(request_id = %request_id, "person dossier failed: {err:#}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(error_response(ApiError::internal("Failed to load person dossier"))))
        }
    }
}

async fn get_person_engagement(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<PersonEngagement>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let uid = match Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => return (StatusCode::BAD_REQUEST, Json(error_response(ApiError::bad_request("Invalid UUID")))),
    };
    match state.store.get_person_engagement(uid).await {
        Ok(Some(e)) => {
            let dur = start.elapsed().as_millis() as u64;
            log_latency("get_person_engagement", dur);
            (StatusCode::OK, Json(success_with_meta(e, ResponseMeta::now().with_request_id(request_id).with_duration(dur))))
        }
        Ok(None) => (StatusCode::NOT_FOUND, Json(error_response(ApiError::not_found("Person", &id)))),
        Err(err) => {
            tracing::error!(request_id = %request_id, "person engagement failed: {err:#}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(error_response(ApiError::internal("Failed to load engagement"))))
        }
    }
}

// ─── Role History & Dossier Entries & Changes ─────────

async fn get_person_role_history(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<Vec<RoleHistoryRow>>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let uid = match Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => return (StatusCode::BAD_REQUEST, Json(error_response(ApiError::bad_request("Invalid UUID")))),
    };
    match state.store.get_role_history(uid, 100).await {
        Ok(history) => {
            let dur = start.elapsed().as_millis() as u64;
            log_latency("get_person_role_history", dur);
            (StatusCode::OK, Json(success_with_meta(history, ResponseMeta::now().with_request_id(request_id).with_duration(dur))))
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "role history failed: {err:#}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(error_response(ApiError::internal("Failed to load role history"))))
        }
    }
}

async fn get_person_changes_api(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<Vec<PersonChangeRow>>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let uid = match Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => return (StatusCode::BAD_REQUEST, Json(error_response(ApiError::bad_request("Invalid UUID")))),
    };
    match state.store.get_person_changes(uid, 100).await {
        Ok(changes) => {
            let dur = start.elapsed().as_millis() as u64;
            log_latency("get_person_changes", dur);
            (StatusCode::OK, Json(success_with_meta(changes, ResponseMeta::now().with_request_id(request_id).with_duration(dur))))
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "person changes failed: {err:#}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(error_response(ApiError::internal("Failed to load person changes"))))
        }
    }
}

async fn get_person_dossier_entries(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<Vec<DossierEntryRow>>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let uid = match Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => return (StatusCode::BAD_REQUEST, Json(error_response(ApiError::bad_request("Invalid UUID")))),
    };
    match state.store.get_dossier_entries("person", uid, None, 200).await {
        Ok(entries) => {
            let dur = start.elapsed().as_millis() as u64;
            log_latency("get_person_dossier_entries", dur);
            (StatusCode::OK, Json(success_with_meta(entries, ResponseMeta::now().with_request_id(request_id).with_duration(dur))))
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "person dossier entries failed: {err:#}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(error_response(ApiError::internal("Failed to load dossier entries"))))
        }
    }
}

async fn get_company_changes_api(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<Vec<CompanyChangeRow>>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let uid = match Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => return (StatusCode::BAD_REQUEST, Json(error_response(ApiError::bad_request("Invalid UUID")))),
    };
    match state.store.get_company_changes(uid, 100).await {
        Ok(changes) => {
            let dur = start.elapsed().as_millis() as u64;
            log_latency("get_company_changes", dur);
            (StatusCode::OK, Json(success_with_meta(changes, ResponseMeta::now().with_request_id(request_id).with_duration(dur))))
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "company changes failed: {err:#}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(error_response(ApiError::internal("Failed to load company changes"))))
        }
    }
}

async fn get_company_dossier_entries(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<Vec<DossierEntryRow>>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let uid = match Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => return (StatusCode::BAD_REQUEST, Json(error_response(ApiError::bad_request("Invalid UUID")))),
    };
    match state.store.get_dossier_entries("company", uid, None, 200).await {
        Ok(entries) => {
            let dur = start.elapsed().as_millis() as u64;
            log_latency("get_company_dossier_entries", dur);
            (StatusCode::OK, Json(success_with_meta(entries, ResponseMeta::now().with_request_id(request_id).with_duration(dur))))
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "company dossier entries failed: {err:#}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(error_response(ApiError::internal("Failed to load dossier entries"))))
        }
    }
}

async fn verify_dossier_entry(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let uid = match Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => return (StatusCode::BAD_REQUEST, Json(error_response(ApiError::bad_request("Invalid UUID")))),
    };
    match state.store.verify_dossier_entry(uid).await {
        Ok(true) => {
            let dur = start.elapsed().as_millis() as u64;
            log_latency("verify_dossier_entry", dur);
            (StatusCode::OK, Json(success_with_meta(serde_json::json!({"verified": true}), ResponseMeta::now().with_request_id(request_id).with_duration(dur))))
        }
        Ok(false) => (StatusCode::NOT_FOUND, Json(error_response(ApiError::not_found("DossierEntry", &id)))),
        Err(err) => {
            tracing::error!(request_id = %request_id, "verify dossier entry failed: {err:#}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(error_response(ApiError::internal("Failed to verify dossier entry"))))
        }
    }
}

async fn get_dossier_entry_history(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<Vec<DossierEntryRow>>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let uid = match Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => return (StatusCode::BAD_REQUEST, Json(error_response(ApiError::bad_request("Invalid UUID")))),
    };
    match state.store.get_dossier_entry_history(uid).await {
        Ok(entries) => {
            let dur = start.elapsed().as_millis() as u64;
            log_latency("get_dossier_entry_history", dur);
            (StatusCode::OK, Json(success_with_meta(entries, ResponseMeta::now().with_request_id(request_id).with_duration(dur))))
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "dossier entry history failed: {err:#}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(error_response(ApiError::internal("Failed to load dossier entry history"))))
        }
    }
}

// ─── Competitors ──────────────────────────────────────

#[derive(Debug, serde::Deserialize)]
struct ListCompetitorsQuery {
    page: Option<u32>,
    per_page: Option<u32>,
}

async fn list_competitors(
    State(state): State<AppState>,
    Query(params): Query<ListCompetitorsQuery>,
) -> (StatusCode, Json<ApiResponse<PagedResponse<CompanyRow>>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let (page, per_page, _) = match validate_pagination(params.page, params.per_page) {
        Ok(p) => p,
        Err(err) => return (StatusCode::BAD_REQUEST, Json(error_response(err))),
    };
    let total = state.store.count_competitors().await.unwrap_or(0).max(0) as u64;
    let clamped_page = clamp_page(page, per_page, total);
    let offset = ((clamped_page - 1) as i64) * (per_page as i64);
    let items = state.store.list_competitors(per_page as i64, offset).await.unwrap_or_default();
    let payload = PagedResponse { items, total, page: clamped_page, per_page };
    let dur = start.elapsed().as_millis() as u64;
    log_latency("list_competitors", dur);
    (StatusCode::OK, Json(success_with_meta(payload, ResponseMeta::now().with_request_id(request_id).with_duration(dur))))
}

async fn get_competitor_changes(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<Vec<CompanyChangeRow>>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let uid = match Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => return (StatusCode::BAD_REQUEST, Json(error_response(ApiError::bad_request("Invalid UUID")))),
    };
    match state.store.get_competitor_changes(uid, 100).await {
        Ok(changes) => {
            let dur = start.elapsed().as_millis() as u64;
            log_latency("get_competitor_changes", dur);
            (StatusCode::OK, Json(success_with_meta(changes, ResponseMeta::now().with_request_id(request_id).with_duration(dur))))
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "competitor changes failed: {err:#}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(error_response(ApiError::internal("Failed to load competitor changes"))))
        }
    }
}

/// List all competitor changes across all competitors.
#[derive(Debug, serde::Deserialize)]
struct AllCompetitorChangesQuery {
    page: Option<u32>,
    per_page: Option<u32>,
    #[serde(default)]
    #[allow(dead_code)] // Reserved for future "fetch all" mode
    all: bool,
}

async fn list_all_competitor_changes(
    State(state): State<AppState>,
    Query(q): Query<AllCompetitorChangesQuery>,
) -> (StatusCode, Json<ApiResponse<PagedResponse<CompetitorChange>>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let page = q.page.unwrap_or(1).max(1);
    let per_page = q.per_page.unwrap_or(50).clamp(1, 100);

    match state.store.get_all_competitor_changes_paged(page as i64, per_page as i64).await {
        Ok((items, total)) => {
            let dur = start.elapsed().as_millis() as u64;
            log_latency("list_all_competitor_changes", dur);
            let payload = PagedResponse { items, total: total as u64, page, per_page };
            (StatusCode::OK, Json(success_with_meta(payload, ResponseMeta::now().with_request_id(request_id).with_duration(dur))))
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "list all competitor changes failed: {err:#}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(error_response(ApiError::internal("Failed to list competitor changes"))))
        }
    }
}

// ─── Graph neighborhood & path ──────────────────────

async fn get_graph_neighborhood(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<Vec<EdgeRow>>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let uid = match Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => return (StatusCode::BAD_REQUEST, Json(error_response(ApiError::bad_request("Invalid UUID")))),
    };
    match state.store.get_neighborhood(uid, 100).await {
        Ok(edges) => {
            let dur = start.elapsed().as_millis() as u64;
            log_latency("get_graph_neighborhood", dur);
            (StatusCode::OK, Json(success_with_meta(edges, ResponseMeta::now().with_request_id(request_id).with_duration(dur))))
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "graph neighborhood failed: {err:#}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(error_response(ApiError::internal("Failed to load graph neighborhood"))))
        }
    }
}

async fn get_graph_path(
    State(state): State<AppState>,
    Path((from, to)): Path<(String, String)>,
) -> (StatusCode, Json<ApiResponse<Vec<EdgeRow>>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let from_id = match Uuid::parse_str(&from) {
        Ok(u) => u,
        Err(_) => return (StatusCode::BAD_REQUEST, Json(error_response(ApiError::bad_request("Invalid from UUID")))),
    };
    let to_id = match Uuid::parse_str(&to) {
        Ok(u) => u,
        Err(_) => return (StatusCode::BAD_REQUEST, Json(error_response(ApiError::bad_request("Invalid to UUID")))),
    };
    match state.store.get_path_edges(from_id, to_id).await {
        Ok(edges) => {
            let dur = start.elapsed().as_millis() as u64;
            log_latency("get_graph_path", dur);
            (StatusCode::OK, Json(success_with_meta(edges, ResponseMeta::now().with_request_id(request_id).with_duration(dur))))
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "graph path failed: {err:#}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(error_response(ApiError::internal("Failed to load graph path"))))
        }
    }
}

// ─── Recipes staging & promote ──────────────────────

#[derive(Debug, serde::Deserialize)]
struct ListStagingQuery {
    page: Option<u32>,
    per_page: Option<u32>,
}

async fn list_staging_recipes(
    State(state): State<AppState>,
    Query(params): Query<ListStagingQuery>,
) -> (StatusCode, Json<ApiResponse<PagedResponse<RecipeStatRow>>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let (page, per_page, _) = match validate_pagination(params.page, params.per_page) {
        Ok(p) => p,
        Err(err) => return (StatusCode::BAD_REQUEST, Json(error_response(err))),
    };
    let total = state.store.count_staging_recipes().await.unwrap_or(0).max(0) as u64;
    let clamped_page = clamp_page(page, per_page, total);
    let offset = ((clamped_page - 1) as i64) * (per_page as i64);
    let items = state.store.list_staging_recipes(per_page as i64, offset).await.unwrap_or_default();
    let payload = PagedResponse { items, total, page: clamped_page, per_page };
    let dur = start.elapsed().as_millis() as u64;
    log_latency("list_staging_recipes", dur);
    (StatusCode::OK, Json(success_with_meta(payload, ResponseMeta::now().with_request_id(request_id).with_duration(dur))))
}

async fn promote_recipe(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    match state.store.promote_recipe(&id).await {
        Ok(true) => {
            let dur = start.elapsed().as_millis() as u64;
            log_latency("promote_recipe", dur);
            (StatusCode::OK, Json(success_with_meta(
                serde_json::json!({"promoted": true, "recipe_code": id}),
                ResponseMeta::now().with_request_id(request_id).with_duration(dur),
            )))
        }
        Ok(false) => (StatusCode::NOT_FOUND, Json(error_response(ApiError::not_found("Recipe", &id)))),
        Err(err) => {
            tracing::error!(request_id = %request_id, "promote_recipe failed: {err:#}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(error_response(ApiError::internal("Failed to promote recipe"))))
        }
    }
}

async fn deprecate_recipe(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    match state.store.deprecate_recipe(&id).await {
        Ok(true) => {
            let dur = start.elapsed().as_millis() as u64;
            log_latency("deprecate_recipe", dur);
            (StatusCode::OK, Json(success_with_meta(
                serde_json::json!({"deprecated": true, "recipe_code": id}),
                ResponseMeta::now().with_request_id(request_id).with_duration(dur),
            )))
        }
        Ok(false) => (StatusCode::NOT_FOUND, Json(error_response(ApiError::not_found("Recipe", &id)))),
        Err(err) => {
            tracing::error!(request_id = %request_id, "deprecate_recipe failed: {err:#}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(error_response(ApiError::internal("Failed to deprecate recipe"))))
        }
    }
}

// ─── Security sub-endpoints ──────────────────────────

#[derive(Debug, serde::Deserialize)]
struct SecuritySubQuery {
    limit: Option<i64>,
}

async fn get_dns_posture(
    State(state): State<AppState>,
    Query(params): Query<SecuritySubQuery>,
) -> (StatusCode, Json<ApiResponse<DnsPostureOverview>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let limit = params.limit.unwrap_or(50).min(200);
    match state.store.get_dns_posture_entries(limit).await {
        Ok(rows) => {
            let items: Vec<DnsPostureItem> = rows.iter().map(|row| {
                let has_spf = row.value.get("has_spf").and_then(|v| v.as_bool()).unwrap_or(false);
                let has_dkim = row.value.get("has_dkim").and_then(|v| v.as_bool()).unwrap_or(false);
                let has_dmarc = row.value.get("has_dmarc").and_then(|v| v.as_bool()).unwrap_or(false);
                DnsPostureItem {
                    domain: row.value.get("domain").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    company_id: row.entity_id.map(|id| id.to_string()).unwrap_or_default(),
                    company_name: row.value.get("company_name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    has_spf,
                    has_dkim,
                    has_dmarc,
                    dmarc_policy: row.value.get("dmarc_policy").and_then(|v| v.as_str()).map(|s| s.to_string()),
                    posture_score: dns_score(has_spf, has_dkim, has_dmarc) * 100.0,
                    last_checked: row.ts_utc.to_rfc3339(),
                }
            }).collect();
            let overall_score = if items.is_empty() {
                0.0
            } else {
                items.iter().map(|i| i.posture_score).sum::<f64>() / items.len() as f64
            };
            let domains_checked = items.len();
            let overview = DnsPostureOverview { items, overall_score, domains_checked };
            let dur = start.elapsed().as_millis() as u64;
            log_latency("get_dns_posture", dur);
            (StatusCode::OK, Json(success_with_meta(overview, ResponseMeta::now().with_request_id(request_id).with_duration(dur))))
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "dns posture failed: {err:#}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(error_response(ApiError::internal("Failed to load DNS posture"))))
        }
    }
}

async fn get_lookalike_domains(
    State(state): State<AppState>,
    Query(params): Query<SecuritySubQuery>,
) -> (StatusCode, Json<ApiResponse<Vec<LookalikeDomainItem>>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let limit = params.limit.unwrap_or(50).min(200);
    match state.store.get_lookalike_domains(limit).await {
        Ok(rows) => {
            let items: Vec<LookalikeDomainItem> = rows.iter().map(|row| {
                LookalikeDomainItem {
                    id: row.id.to_string(),
                    original_domain: row.value.get("original_domain").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    lookalike_domain: row.value.get("domain").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    distance: row.value.get("distance").and_then(|v| v.as_i64()).unwrap_or(1),
                    threat_type: row.value.get("threat_type").and_then(|v| v.as_str()).unwrap_or("typosquat").to_string(),
                    detected_at: row.ts_utc.to_rfc3339(),
                    active: row.value.get("active").and_then(|v| v.as_bool()).unwrap_or(true),
                    registrar: row.value.get("registrar").and_then(|v| v.as_str()).map(|s| s.to_string()),
                    registration_date: row.value.get("registration_date").and_then(|v| v.as_str()).map(|s| s.to_string()),
                }
            }).collect();
            let dur = start.elapsed().as_millis() as u64;
            log_latency("get_lookalike_domains", dur);
            (StatusCode::OK, Json(success_with_meta(items, ResponseMeta::now().with_request_id(request_id).with_duration(dur))))
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "lookalike domains failed: {err:#}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(error_response(ApiError::internal("Failed to load lookalike domains"))))
        }
    }
}

async fn get_kev_relevance(
    State(state): State<AppState>,
    Query(params): Query<SecuritySubQuery>,
) -> (StatusCode, Json<ApiResponse<Vec<KevItem>>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let limit = params.limit.unwrap_or(50).min(200);
    match state.store.get_kev_relevance(limit).await {
        Ok(rows) => {
            let items: Vec<KevItem> = rows.iter().map(|row| {
                KevItem {
                    cve_id: row.value.get("cve_id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    vendor: row.value.get("vendor").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    product: row.value.get("product").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    vulnerability_name: row.value.get("vulnerability_name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    date_added: row.value.get("date_added").and_then(|v| v.as_str()).unwrap_or("1970-01-01").to_string(),
                    due_date: row.value.get("due_date").and_then(|v| v.as_str()).unwrap_or("1970-01-01").to_string(),
                    relevance_score: row.value.get("relevance_score").and_then(|v| v.as_f64()).unwrap_or(0.5),
                    affected_companies: row.value.get("affected_companies")
                        .and_then(|v| v.as_array())
                        .map(|arr| arr.iter().filter_map(|s| s.as_str().map(|ss| ss.to_string())).collect())
                        .unwrap_or_default(),
                    notes: row.value.get("notes").and_then(|v| v.as_str()).map(|s| s.to_string()),
                }
            }).collect();
            let dur = start.elapsed().as_millis() as u64;
            log_latency("get_kev_relevance", dur);
            (StatusCode::OK, Json(success_with_meta(items, ResponseMeta::now().with_request_id(request_id).with_duration(dur))))
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "kev relevance failed: {err:#}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(error_response(ApiError::internal("Failed to load KEV relevance"))))
        }
    }
}

// ─── Admin endpoints ──────────────────────────────────

async fn get_admin_crawl_status(
    State(state): State<AppState>,
) -> (StatusCode, Json<ApiResponse<AdminCrawlStatus>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    match state.store.get_admin_crawl_status().await {
        Ok(status) => {
            let dur = start.elapsed().as_millis() as u64;
            log_latency("get_admin_crawl_status", dur);
            (StatusCode::OK, Json(success_with_meta(status, ResponseMeta::now().with_request_id(request_id).with_duration(dur))))
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "admin crawl status failed: {err:#}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(error_response(ApiError::internal("Failed to load crawl status"))))
        }
    }
}

async fn get_admin_recipe_performance(
    State(state): State<AppState>,
) -> (StatusCode, Json<ApiResponse<AdminRecipePerformance>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    match state.store.get_admin_recipe_performance().await {
        Ok(perf) => {
            let dur = start.elapsed().as_millis() as u64;
            log_latency("get_admin_recipe_performance", dur);
            (StatusCode::OK, Json(success_with_meta(perf, ResponseMeta::now().with_request_id(request_id).with_duration(dur))))
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "admin recipe performance failed: {err:#}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(error_response(ApiError::internal("Failed to load recipe performance"))))
        }
    }
}

async fn get_admin_poi_coverage(
    State(state): State<AppState>,
) -> (StatusCode, Json<ApiResponse<AdminPoiCoverage>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    match state.store.get_admin_poi_coverage().await {
        Ok(coverage) => {
            let dur = start.elapsed().as_millis() as u64;
            log_latency("get_admin_poi_coverage", dur);
            (StatusCode::OK, Json(success_with_meta(coverage, ResponseMeta::now().with_request_id(request_id).with_duration(dur))))
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "admin poi coverage failed: {err:#}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(error_response(ApiError::internal("Failed to load POI coverage"))))
        }
    }
}

// ─── Admin: trigger manual scan ─────────────────────────────────────────────

#[derive(Debug, serde::Deserialize)]
struct TriggerScanRequest {
    job_kind: String,
}

#[derive(Debug, serde::Serialize)]
struct TriggerScanResponse {
    trigger_id: String,
    job_kind: String,
}

async fn post_trigger_scan(
    State(state): State<AppState>,
    Json(body): Json<TriggerScanRequest>,
) -> (StatusCode, Json<ApiResponse<TriggerScanResponse>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();

    // Validate job_kind is one of the known kinds
    let valid_kinds = [
        "crawl_cycle", "pattern_mining", "hypothesis_generation", "poi_refresh",
        "promotion_board", "recipe_deprecation", "strategy_memo", "feature_drift_check",
        "source_scoring", "cross_domain_mining", "outcome_tracking", "breach_scan",
        "sanctions_screen", "sla_enforcement", "dns_posture_scan", "kev_catalog_fetch",
        "lookalike_domain_scan", "self_improvement_cycle",
    ];
    if !valid_kinds.contains(&body.job_kind.as_str()) {
        let api_err = ApiError::validation("job_kind", format!("Unknown job kind: {}", body.job_kind));
        return (StatusCode::BAD_REQUEST, Json(error_response(api_err)));
    }

    match state.store.queue_job_trigger(&body.job_kind).await {
        Ok(trigger_id) => {
            let dur = start.elapsed().as_millis() as u64;
            log_latency("post_trigger_scan", dur);
            tracing::info!(request_id = %request_id, job_kind = %body.job_kind, trigger_id = %trigger_id, "manual job trigger queued");
            (StatusCode::ACCEPTED, Json(success_with_meta(
                TriggerScanResponse { trigger_id, job_kind: body.job_kind },
                ResponseMeta::now().with_request_id(request_id).with_duration(dur),
            )))
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "queue_job_trigger failed: {err:#}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(error_response(ApiError::internal("Failed to queue job trigger"))))
        }
    }
}

// ─── WebSocket: /ws/warnings ────────────────────────────────────────────────

async fn warnings_ws(
    ws: axum::extract::ws::WebSocketUpgrade,
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> axum::response::Response {
    // Validate token from Sec-WebSocket-Protocol header or Authorization header
    let token = headers
        .get("sec-websocket-protocol")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string())
        .or_else(|| {
            headers
                .get(header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .and_then(auth::extract_bearer_token)
                .map(|s| s.to_string())
        });

    let token = match token {
        Some(t) => t,
        None => {
            return axum::response::Response::builder()
                .status(StatusCode::UNAUTHORIZED)
                .body(axum::body::Body::from("Missing authentication"))
                .unwrap();
        }
    };

    let auth_result = auth::validate_token(&token, &state.api_keys, Utc::now());
    if !matches!(auth_result, AuthResult::Valid { .. }) {
        return axum::response::Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .body(axum::body::Body::from("Invalid or expired token"))
            .unwrap();
    }

    ws.on_upgrade(move |socket| warnings_ws_stream(socket, state))
}

async fn warnings_ws_stream(
    mut socket: axum::extract::ws::WebSocket,
    state: AppState,
) {
    use axum::extract::ws::Message;

    tracing::info!("WebSocket client connected to /ws/warnings");

    // Track most-recent warning timestamp to only send new ones
    let mut last_check = Utc::now();
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));

    loop {
        tokio::select! {
            _ = interval.tick() => {
                // Poll for warnings created since last check
                let filters = WarningListFilters {
                    regions: vec![],
                    severities: vec![],
                    warning_types: vec![],
                    acknowledged: Some(false),
                    date_from: Some(last_check),
                    date_to: None,
                    search: None,
                };
                match state.store.list_warnings(&filters, Some(WarningOrderBy::CreatedAt), true, 10, 0).await {
                    Ok(rows) => {
                        for row in rows {
                            let created = row.created_at.unwrap_or(row.ts_utc);
                            if created > last_check {
                                let event = WarningEvent {
                                    warning_id: row.id.to_string(),
                                    severity: row.severity.clone(),
                                    warning_type: row.warning_type.clone(),
                                    title: row.title.clone(),
                                    region: row.region.clone().unwrap_or_default(),
                                    created_at: created,
                                };
                                let envelope = to_ws_event(event);
                                if let Ok(json) = serde_json::to_string(&envelope) {
                                    if socket.send(Message::Text(json.into())).await.is_err() {
                                        tracing::info!("WebSocket client disconnected (send error)");
                                        return;
                                    }
                                }
                            }
                        }
                        last_check = Utc::now();
                    }
                    Err(e) => {
                        tracing::warn!("WS warning poll error: {e:#}");
                    }
                }
            }
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => {
                        tracing::info!("WebSocket client disconnected");
                        return;
                    }
                    Some(Ok(Message::Ping(data))) => {
                        let _ = socket.send(Message::Pong(data)).await;
                    }
                    Some(Ok(_)) => {
                        // Ignore text/binary messages from client
                    }
                    Some(Err(e)) => {
                        tracing::warn!("WebSocket read error: {e:#}");
                        return;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_warning_row_null_and_empty_lists_map_to_empty_arrays() {
        let now = Utc::now();
        let row_null = WarningRow {
            id: Uuid::new_v4(),
            recipe_code: None,
            warning_type: "demand_spike".to_string(),
            title: "Test warning".to_string(),
            description: None,
            severity: "high".to_string(),
            region: None,
            source_urls: None,
            entity_ids: None,
            confidence: None,
            ts_utc: now,
            acknowledged: false,
            acknowledged_by: None,
            acknowledged_at: None,
            acknowledged_note: None,
            created_at: Some(now),
            updated_at: Some(now),
        };
        let mapped_null = warning_row_to_response(row_null);
        assert!(mapped_null.source_urls.is_empty());
        assert!(mapped_null.entity_ids.is_empty());

        let row_empty = WarningRow {
            id: Uuid::new_v4(),
            recipe_code: None,
            warning_type: "demand_spike".to_string(),
            title: "Test warning".to_string(),
            description: Some("desc".to_string()),
            severity: "high".to_string(),
            region: Some("TN".to_string()),
            source_urls: Some(vec![]),
            entity_ids: Some(vec![]),
            confidence: Some(0.75),
            ts_utc: now,
            acknowledged: false,
            acknowledged_by: None,
            acknowledged_at: None,
            acknowledged_note: None,
            created_at: Some(now),
            updated_at: Some(now),
        };
        let mapped_empty = warning_row_to_response(row_empty);
        assert!(mapped_empty.source_urls.is_empty());
        assert!(mapped_empty.entity_ids.is_empty());
    }

    #[test]
    fn test_insight_row_null_and_empty_lists_map_to_empty_arrays() {
        let now = Utc::now();
        let row_null = InsightRow {
            id: Uuid::new_v4(),
            title: "Test insight".to_string(),
            summary: "Summary".to_string(),
            insight_type: None,
            region: None,
            confidence: None,
            evidence_urls: None,
            entity_ids: None,
            tags: None,
            created_at: Some(now),
            updated_at: Some(now),
        };
        let mapped_null = insight_row_to_response(row_null);
        assert!(mapped_null.evidence_urls.is_empty());
        assert!(mapped_null.entity_ids.is_empty());
        assert!(mapped_null.tags.is_empty());

        let row_empty = InsightRow {
            id: Uuid::new_v4(),
            title: "Test insight".to_string(),
            summary: "Summary".to_string(),
            insight_type: Some("general".to_string()),
            region: Some("TN".to_string()),
            confidence: Some(0.61),
            evidence_urls: Some(vec![]),
            entity_ids: Some(vec![]),
            tags: Some(vec![]),
            created_at: Some(now),
            updated_at: Some(now),
        };
        let mapped_empty = insight_row_to_response(row_empty);
        assert!(mapped_empty.evidence_urls.is_empty());
        assert!(mapped_empty.entity_ids.is_empty());
        assert!(mapped_empty.tags.is_empty());
    }
}
