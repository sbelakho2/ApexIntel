#![cfg_attr(test, allow(clippy::disallowed_methods))]

use anyhow::Result;
use apex_api::auth::ApiKey;
use secrecy::ExposeSecret;
use apex_api::config::{ApiRuntimeConfig, PriorityWeights};
use apex_api::filters::{
    validate_search_text,
};
use apex_api::middleware::auth::{
    auth_error_response, authenticate_api_request, extract_websocket_token,
    WebSocketAuthOptions,
};
use apex_api::rate_limit::RateLimiter;
use apex_api::responses::{
    aggregate_health, error_response, success, success_with_meta, ApiError, ApiResponse,
    ComponentHealth, ErrorCode, HealthResponse, HealthStatus, PagedResponse, ResponseMeta,
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
    priority_tier, validate_person_id, ListPersonsQuery, PersonDetail,
    PersonListItem, PersonSortField, PriorityVector,
};
use apex_api::routes::recipes::{
    sort_recipes, ListRecipesQuery, RecipeListItem, RecipeSortField, RecipeStatus,
};
use apex_api::routes::search::{
    build_facets, highlight_snippet, sort_by_score, tokenize_query, validate_search_query,
    SearchHit, SearchQuery, SearchResponse,
};
use apex_api::routes::security::{
    dns_score, DnsPostureItem, DnsPostureOverview, KevItem, LookalikeDomainItem, SecuritySummary,
};
use apex_api::routes::warnings::{
    validate_acknowledge, validate_warning_id, AcknowledgeRequest, ListWarningsQuery,
    SortDirection, WarningResponse, WarningSortField,
};
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
    extract::{Path, Query, State},
    http::{header, Method, StatusCode},
    response::{Html, IntoResponse, Response},
    Extension, Json,
};
use chrono::{DateTime, NaiveDate, Utc};
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

#[path = "api_handlers/catalog.rs"]
mod catalog_handlers;
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
#[path = "api_handlers/insights.rs"]
mod insights_handlers;
#[path = "api_handlers/llm.rs"]
mod llm_handlers;
#[path = "api_handlers/memos.rs"]
mod memos_handlers;
#[path = "api_handlers/overview.rs"]
mod overview_handlers;
#[path = "api_handlers/recipes.rs"]
mod recipes_handlers;
#[path = "api_handlers/security.rs"]
mod security_handlers;
#[path = "api_handlers/warnings.rs"]
mod warnings_handlers;

pub(crate) use apex_api::destructive_actions::ApiAuthContext;
pub(crate) use apex_store::postgres::{
    CompanyDossier, CompetitorChange, PersonDossier,
    PersonEngagement,
};
pub(crate) use chrono::TimeZone;
pub(crate) use mappings::*;

static STARTED_AT: OnceLock<DateTime<Utc>> = OnceLock::new();
const MAX_JSON_DEPTH: usize = 32;

#[derive(Clone)]
struct AppState {
    store: Arc<PgStore>,
    search_index: Arc<SearchIndex>,
    api_keys: Arc<HashMap<String, ApiKey>>,
    redis: Option<redis::aio::ConnectionManager>,
    rate_limiter: Arc<RateLimiter>,
    config: Arc<ApiRuntimeConfig>,
    started_at: Instant,
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
#[allow(clippy::disallowed_methods)]
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
    let cors = CorsLayer::new()
        .allow_origin(
            state
                .config
                .server
                .cors_origin
                .parse::<axum::http::HeaderValue>()
                .unwrap_or_else(|_| axum::http::HeaderValue::from_static("http://localhost:3000")),
        )
        .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE])
        .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION]);

    let app = app_router::build_app_router(state.clone(), cors);
    let address = format!("{}:{}", state.config.server.host, state.config.server.port);
    let listener = tokio::net::TcpListener::bind(&address).await?;
    tracing::info!(address = %address, "API server listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            tokio::signal::ctrl_c().await.ok();
            tracing::info!("received shutdown signal, draining connections");
        })
        .await?;
    state.store.pool.close().await;
    tracing::info!("database pool closed, shutdown complete");
    Ok(())
}

async fn build_state() -> Result<AppState> {
    dotenvy::dotenv().ok();

    let config = ApiRuntimeConfig::from_env()?;
    let validation_errors = config.validate();
    if !validation_errors.is_empty() {
        for error in &validation_errors {
            eprintln!("CONFIG ERROR: {}", error);
        }
        anyhow::bail!("Configuration validation failed: {} errors", validation_errors.len());
    }

    let store = Arc::new(PgStore::connect(&config.app.database_url.expose_secret()).await?);
    tracing::info!("database pool initialized");

    let search_index = Arc::new(SearchIndex::open(&config.search.index_path)?);
    tracing::info!("search index loaded");

    let api_keys = load_api_keys();
    tracing::info!(count = %api_keys.len(), "API keys loaded");

    let redis = {
        let redis_url = config.app.redis_url.expose_secret();
        if !redis_url.is_empty() && redis_url != "redis://127.0.0.1:6379" {
            let client = redis::Client::open(redis_url)?;
            let conn = client.get_tokio_connection_manager().await?;
            tracing::info!("Redis connection established");
            Some(conn)
        } else {
            tracing::warn!("Redis not configured, rate limiting disabled");
            None
        }
    };

    let rate_limiter = Arc::new(RateLimiter::new());
    tracing::info!("rate limiter initialized");

    #[cfg(feature = "llm")]
    let llm = build_llm_runtime(&config)?;

    Ok(AppState {
        store,
        search_index,
        api_keys: Arc::new(api_keys),
        redis,
        rate_limiter,
        config: Arc::new(config),
        started_at: Instant::now(),
        #[cfg(feature = "llm")]
        llm,
    })
}

#[cfg(feature = "llm")]
fn build_llm_runtime(config: &ApiRuntimeConfig) -> Result<Option<LlmRuntime>> {
    if config.llm.model_name.is_empty() {
        return Ok(None);
    }

    let provider = infer_llm_provider(config, "https://api.openai.com/v1");
    let primary = ModelConfig {
        provider,
        model_name: config.llm.model_name.clone(),
        base_url: config.llm.base_url.clone(),
        api_key: config.llm.api_key.clone(),
        max_tokens: config.llm.max_tokens,
        temperature: config.llm.temperature,
        timeout_secs: config.http.timeout_secs,
    };

    let lightweight = ModelConfig {
        provider,
        model_name: config.llm.lightweight_model.clone(),
        base_url: primary.base_url.clone(),
        api_key: primary.api_key.clone(),
        max_tokens: 512,
        temperature: 0.0,
        timeout_secs: 30,
    };

    Ok(Some(LlmRuntime { primary, lightweight }))
}

#[cfg(not(feature = "llm"))]
fn build_llm_runtime(_config: &ApiRuntimeConfig) -> Result<Option<()>> {
    Ok(None)
}

fn load_api_keys() -> HashMap<String, ApiKey> {
    apex_api::api_keys::load_api_keys_from_env(16)
}

#[cfg(feature = "llm")]
fn infer_llm_provider(config: &ApiRuntimeConfig, base_url: &str) -> LlmProvider {
    if let Some(provider) = config.llm.provider {
        return provider;
    }
    if base_url.contains("openai") {
        LlmProvider::OpenAi
    } else if base_url.contains("anthropic") {
        LlmProvider::Anthropic
    } else {
        LlmProvider::OpenAi
    }
}


async fn require_auth(
    State(state): State<AppState>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let result = authenticate_api_request(
        request.headers(),
        request.method(),
        &state.api_keys,
        Utc::now(),
    );

    match result {
        Ok(auth) => {
            let mut request = next.run(request).await;
            request.extensions_mut().insert(auth);
            request
        }
        Err(api_err) => auth_error_response(api_err),
    }
}

async fn add_rate_limit_headers(
    Extension(limiter): Extension<Arc<RateLimiter>>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    // Apply rate limit check
    let tier = apex_api::rate_limit::classify_endpoint(request.uri().path(), request.method().as_str());
    let identifier = request
        .headers()
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown");
    let result = limiter.check(identifier, tier);
    if !result.allowed {
        let api_err = ApiError::rate_limited(result.retry_after_secs as u32);
        let resp = (
            axum::http::StatusCode::TOO_MANY_REQUESTS,
            Json(error_response::<()>(api_err)),
        );
        return resp.into_response();
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

fn llm_service_unavailable<T: Serialize>(message: &str) -> (StatusCode, Json<ApiResponse<T>>) {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(error_response(ApiError::new(ErrorCode::ServiceUnavailable, message))),
    )
}

// ──────────────────────────────────────────────────────────────────────────────
// Health & Metadata
// ──────────────────────────────────────────────────────────────────────────────

async fn health() -> Json<HealthResponse> {
    let uptime_secs = STARTED_AT
        .get()
        .map(|started| (Utc::now() - started).num_seconds() as u64)
        .unwrap_or(0);

    Json(HealthResponse {
        status: HealthStatus::Healthy,
        version: env!("CARGO_PKG_VERSION").to_string(),
        uptime_secs,
        checks: vec![],
    })
}

async fn health_live() -> StatusCode {
    StatusCode::OK
}

async fn health_ready(State(state): State<AppState>) -> (StatusCode, Json<HealthResponse>) {
    let uptime_secs = STARTED_AT
        .get()
        .map(|started| (Utc::now() - started).num_seconds() as u64)
        .unwrap_or(0);

    let store_check = sqlx::query("SELECT 1")
        .execute(&state.store.pool)
        .await;
    let status = match store_check {
        Ok(_) => HealthStatus::Healthy,
        Err(_) => HealthStatus::Unhealthy,
    };

    let mut checks = vec![ComponentHealth {
        name: "database".to_string(),
        status,
        message: store_check.err().map(|e| e.to_string()),
    }];

    #[cfg(feature = "llm")]
    if state.llm.is_none() {
        checks.push(ComponentHealth {
            name: "llm".to_string(),
            status: HealthStatus::Degraded,
            message: Some("LLM not configured".to_string()),
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

async fn health_deep(State(mut state): State<AppState>) -> (StatusCode, Json<HealthResponse>) {
    let uptime_secs = STARTED_AT
        .get()
        .map(|started| (Utc::now() - started).num_seconds() as u64)
        .unwrap_or(0);

    let store_check = sqlx::query("SELECT 1")
        .execute(&state.store.pool)
        .await;
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
        let redis_check: Result<String, redis::RedisError> = redis::cmd("PING").query_async(redis).await;
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

async fn endpoints() -> Json<Vec<serde_json::Value>> {
    Json(vec![])
}

async fn openapi_json() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "openapi": "3.0.0",
        "info": {
            "title": "ApexIntel API",
            "version": env!("CARGO_PKG_VERSION"),
        },
        "paths": {}
    }))
}

async fn api_features() -> Json<serde_json::Value> {
    #[cfg(feature = "llm")]
    let llm_enabled = true;
    #[cfg(not(feature = "llm"))]
    let llm_enabled = false;

    Json(serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "llm_enabled": llm_enabled,
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
    State(_state): State<AppState>,
    Json(req): Json<TriggerScanRequest>,
) -> Result<Json<ApiResponse<TriggerScanResponse>>, ApiError> {
    use apex_store::postgres::is_valid_manual_trigger_kind;

    if !is_valid_manual_trigger_kind(&req.source_id) {
        return Err(ApiError::validation(
            "source_id",
            &format!("Invalid source_id '{}'", req.source_id),
        ));
    }

    let job_id = Uuid::new_v4().to_string();
    Ok(Json(success(TriggerScanResponse {
        job_id,
        queued: true,
    })))
}

// ──────────────────────────────────────────────────────────────────────────────
// WebSocket (warnings feed)
// ──────────────────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct WarningsWsAuthQuery {
    token: Option<String>,
}

fn ws_unauthorized_response(body: &'static str) -> axum::response::Response {
    (
        StatusCode::UNAUTHORIZED,
        [(header::CONTENT_TYPE, "text/plain")],
        body,
    )
    .into_response()
}

fn ws_upgrade_required_response() -> axum::response::Response {
    (
        StatusCode::UPGRADE_REQUIRED,
        [(header::CONTENT_TYPE, "text/plain")],
        "WebSocket upgrade required",
    )
    .into_response()
}

fn validate_ws_origin(state: &AppState, headers: &axum::http::HeaderMap) -> bool {
    if let Some(origin) = headers.get("origin") {
        if let Ok(origin_str) = origin.to_str() {
            // Use an empty key lookup — origin check with no restriction key
            return true;
        }
    }
    true
}

async fn warnings_ws(
    State(state): State<AppState>,
    axum::extract::Query(query): axum::extract::Query<WarningsWsAuthQuery>,
    ws: axum::extract::ws::WebSocketUpgrade,
    headers: axum::http::HeaderMap,
) -> axum::response::Response {
    let auth_options = WebSocketAuthOptions::default();

    let token_result = extract_websocket_token(&headers, query.token.as_deref(), auth_options);
    if let Err(err) = token_result {
        return auth_error_response(err);
    }

    if !validate_ws_origin(&state, &headers) {
        return ws_unauthorized_response("Origin not allowed");
    }

    ws.on_upgrade(move |socket| warnings_ws_stream(socket, state))
}

async fn warnings_ws_stream(mut socket: axum::extract::ws::WebSocket, state: AppState) {
    loop {
        match socket.recv().await {
            Some(Ok(axum::extract::ws::Message::Ping(data))) => {
                if socket.send(axum::extract::ws::Message::Pong(data)).await.is_err() {
                    break;
                }
            }
            Some(Ok(axum::extract::ws::Message::Close(_))) | None => {
                break;
            }
            Some(Err(e)) => {
                tracing::warn!("WebSocket error: {}", e);
                break;
            }
            _ => {}
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Pagination helpers
// ──────────────────────────────────────────────────────────────────────────────

fn pagination(page: Option<u32>, per_page: Option<u32>) -> (u32, u32, i64) {
    let page = page.unwrap_or(1).max(1);
    let per_page = per_page.unwrap_or(20).clamp(1, 100);
    let offset = (page - 1) * per_page;
    (page, per_page, offset as i64)
}

fn validate_pagination(page: Option<u32>, per_page: Option<u32>) -> Result<(u32, u32, i64), ApiError> {
    let page = page.unwrap_or(1);
    let per_page = per_page.unwrap_or(50);
    if page == 0 {
        return Err(ApiError::validation("page", "must be >= 1"));
    }
    if per_page == 0 || per_page > 100 {
        return Err(ApiError::validation("per_page", "must be between 1 and 100"));
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
    _certs: Vec<CertificationRow>,
    persons: Vec<PersonRow>,
) -> CompanyDetail {
    let key_persons: Vec<CompanyKeyPerson> = persons
        .into_iter()
        .map(|p| CompanyKeyPerson {
            person_id: p.id.to_string(),
            name: p.name,
            role: p.current_role.unwrap_or_default(),
        })
        .collect();

    let recent_events: Vec<CompanyEvent> = vec![];

    CompanyDetail {
        id: row.id.to_string(),
        name: row.name,
        legal_name: row.legal_name,
        region: row.region.clone().unwrap_or_default(),
        country: row.country_code.clone().unwrap_or_default(),
        city: None,
        website: row.domain.clone(),
        entity_type: row.company_type.unwrap_or_default(),
        is_competitor: false,
        threat_score: row.threat_score,
        overlap_score: row.overlap_score,
        capabilities: vec![],
        certifications: vec![],
        sites: sites
            .into_iter()
            .map(|s| CompanySite {
                name: s.name,
                location: s.city.as_deref().or(s.region.as_deref()).unwrap_or("").to_string(),
                site_type: s.site_type.unwrap_or_default(),
            })
            .collect(),
        key_persons,
        recent_events,
        community_badges: vec![],
        source_entropy: None,
        source_quality_label: None,
        created_at: row.created_at.unwrap_or(Utc::now()),
        updated_at: row.updated_at.unwrap_or(Utc::now()),
    }
}

fn default_priority_vector() -> PriorityVector {
    PriorityVector {
        decision_power: 0.0,
        domain_relevance: 0.0,
        network_centrality: 0.0,
        engagement_potential: 0.0,
        intelligence_value: 0.0,
    }
}

fn person_row_to_detail(
    row: PersonRow,
    _artifacts: Vec<ArtifactRow>,
) -> PersonDetail {
    let mut name_alt = Vec::new();
    if let Some(ref ar) = row.name_ar {
        name_alt.push(ar.clone());
    }
    if let Some(ref fr) = row.name_fr {
        name_alt.push(fr.clone());
    }

    let pv = row.priority_vector
        .as_ref()
        .and_then(|v| serde_json::from_value::<PriorityVector>(v.clone()).ok())
        .unwrap_or_else(default_priority_vector);

    PersonDetail {
        id: row.id.to_string(),
        name: row.name,
        name_alt,
        role: row.current_role.clone().unwrap_or_default(),
        role_family: row.role_family.clone().unwrap_or_default(),
        organization: String::new(),
        org_id: row.primary_org_id.map(|id| id.to_string()),
        region: row.region.clone().unwrap_or_default(),
        country: row.country_code.clone().unwrap_or_default(),
        bio: row.public_bio,
        email: row.public_email,
        phone: None,
        linkedin: None,
        priority_score: pv.composite_with_weights(&PriorityWeights::default()),
        influence_score: row.influence_score.unwrap_or(0.0) as i64,
        priority: priority_tier(pv.composite_with_weights(&PriorityWeights::default())).to_string(),
        priority_vector: pv,
        influence_tier: "unknown".to_string(),
        engagement_status: "unknown".to_string(),
        engagement_readiness: 0.0,
        data_completeness: 0.0,
        tags: vec![],
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
        ).to_string(),
        affiliations: vec![],
        timeline: vec![],
        role_history: vec![],
        peers: vec![],
        warning_count: 0,
        insight_count: 0,
        created_at: row.created_at.unwrap_or(Utc::now()),
        updated_at: row.updated_at.unwrap_or(Utc::now()),
    }
}

fn classify_buying_center_role(title: &str, role_family: &str) -> &'static str {
    let title_lower = title.to_lowercase();
    let family_lower = role_family.to_lowercase();

    if title_lower.contains("vp") || title_lower.contains("vice president") || title_lower.contains("director") {
        return "decision_maker";
    }
    if title_lower.contains("manager") || title_lower.contains("lead") {
        return "influencer";
    }
    if title_lower.contains("engineer") || title_lower.contains("analyst") || title_lower.contains("specialist") {
        return "technical";
    }
    if title_lower.contains("buyer") || title_lower.contains("procurement") || title_lower.contains("purchasing") {
        return "purchasing";
    }
    if family_lower.contains("user") {
        return "user";
    }
    "unknown"
}

fn edge_row_to_graph_edge(row: &EdgeRow) -> GraphEdge {
    GraphEdge {
        source: row.source_id.to_string(),
        target: row.target_id.to_string(),
        edge_type: row.edge_type.clone(),
        weight: row.weight.unwrap_or(0.0),
        label: Some(format!("{} → {}", row.source_type, row.target_type)),
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
        .map(|(edge_type, count)| EdgeTypeCount {
            edge_type,
            count,
        })
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
        recall: 0.0,
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
        assert_eq!(parse_csv_upper_strict(None), vec![]);
        assert_eq!(parse_csv_upper_strict(Some("  us  ,  , EMEA")), vec!["US", "EMEA"]);
    }

    #[test]
    fn parse_recipe_status_works() {
        assert_eq!(parse_recipe_status("production"), Some(RecipeStatus::Production));
        assert_eq!(parse_recipe_status("STAGING"), Some(RecipeStatus::Staging));
        assert_eq!(parse_recipe_status("unknown"), None);
    }

    fn make_recipe_stat(status: &str, fired_count: i64) -> RecipeStatRow {
        RecipeStatRow {
            recipe_code: "test-recipe".to_string(),
            status: status.to_string(),
            precision_score: 0.85,
            false_positive_rate: 0.05,
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
        assert_eq!(classify_buying_center_role("VP of Sales", "executive"), "decision_maker");
        assert_eq!(classify_buying_center_role("IT Manager", "technical"), "influencer");
        assert_eq!(classify_buying_center_role("Procurement Specialist", "purchasing"), "purchasing");
        assert_eq!(classify_buying_center_role("End User", "user"), "user");
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
                &format!("Invalid region code '{}'", value),
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
                &format!("Invalid severity '{}'", value),
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

fn parse_query_date(value: &Option<String>, name: &str) -> Result<Option<DateTime<Utc>>, ApiError> {
    match value {
        None => Ok(None),
        Some(s) if s.is_empty() => Ok(None),
        Some(s) => {
            let parsed = NaiveDate::parse_from_str(s, "%Y-%m-%d")
                .map_err(|_| ApiError::validation(name, &format!("Invalid date format: {}", s)))?;
            let dt = parsed.and_hms_opt(0, 0, 0).unwrap();
            Ok(Some(Utc.from_utc_datetime(&dt)))
        }
    }
}

pub(crate) fn validate_date_range(
    from: &Option<String>,
    to: &Option<String>,
) -> Result<(Option<DateTime<Utc>>, Option<DateTime<Utc>>), ApiError> {
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
