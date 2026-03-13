#![cfg_attr(test, allow(clippy::disallowed_methods))]

use anyhow::Result;
use apex_api::auth::{self, ApiKey, ApiRole};
use apex_api::config::{ApiRuntimeConfig, PriorityWeights};
use apex_api::destructive_actions::ApiAuthContext;
use apex_api::filters::{
    parse_date, validate_search_text, RegionFilter, SeverityFilter, WarningTypeFilter,
};
use apex_api::middleware::auth::{
    auth_error_response, authenticate_api_request, extract_websocket_token,
    validate_websocket_token, WebSocketAuthOptions,
};
use apex_api::middleware::rate_limit::{
    append_rate_limit_headers, enforce_rate_limit, rate_limited_response, RateLimitDecision,
    RateLimitInfo,
};
use apex_api::middleware::session::require_session;
use apex_api::rate_limit::RateLimiter;
use apex_api::responses::{
    aggregate_health, error_response, success, success_with_meta, ApiError, ApiResponse,
    ComponentHealth, ErrorCode, HealthResponse, HealthStatus, PagedResponse, ResponseMeta,
};
use apex_api::routes;
use apex_api::routes::companies::{
    validate_company_id, CompanyDetail, CompanyKeyPerson, CompanyListItem,
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
    priority_tier, validate_person_id, Affiliation, ListPersonsQuery, PersonDetail, PersonEvent,
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
    dns_score, DnsPostureItem, DnsPostureOverview, KevItem, LookalikeDomainItem,
    SecuritySummary,
};
use apex_api::routes::warnings::{
    validate_acknowledge, validate_warning_id, AcknowledgeRequest, ListWarningsQuery,
    SortDirection, WarningResponse, WarningSortField,
};
use apex_api::routes::ws::{to_ws_event, WarningEvent};
use apex_core::validation::clamp_ratio;
use apex_store::postgres::{
    AdminCrawlStatus, AdminPoiCoverage, AdminRecipePerformance, ArtifactRow, CapabilityRow,
    CertificationRow, CompanyChangeRow, CompanyListFilters, CompanyRow, DashboardStats,
    DossierEntryRow, EdgeRow, InsightListFilters, InsightRow, LogisticsNodeRow, ObservationRow,
    PersonChangeRow, PersonListFilters, PersonListRow, PersonOrderBy, PersonRow, PgStore,
    ProductFamilyRow, RecipeStatRow, RegulationRow, RoleHistoryRow, SiteRow,
    WarningListFilters, WarningOrderBy, WarningRow, WeeklyMemo,
};
use apex_store::tantivy_index::SearchIndex;
use axum::{
    extract::{Path, Query, State},
    http::{header, Method, StatusCode},
    middleware,
    response::{Html, IntoResponse},
    Extension, Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
#[cfg(feature = "llm")]
use serde_json::Value as JsonValue;
use std::{
    collections::{BTreeSet, HashMap},
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

pub(crate) use mappings::*;
pub(crate) use chrono::TimeZone;
pub(crate) use apex_store::postgres::{
    CompanyDossier, CompetitorChange, PersonDossier, PersonEngagement,
};

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
    axum::serve(listener, app).await?;
    Ok(())
}

async fn build_state() -> Result<AppState> {
    dotenvy::dotenv().ok();

    let config = ApiRuntimeConfig::from_env()?;
    let validation_errors = config.validate();
    if !validation_errors.is_empty() {
        for error in &validation_errors {
            tracing::error!(%error, "configuration validation failed");
        }
        anyhow::bail!("Refusing to start API server with invalid configuration");
    }

    #[cfg(feature = "llm")]
    let llm = build_llm_runtime(&config)?;

    let store = PgStore::connect(config.app.database_url_value()).await?;
    store.run_migrations().await?;

    let search_index = SearchIndex::open(config.search.index_path.as_path())?;
    let api_keys = load_api_keys();
    for api_key in api_keys.values() {
        store
            .register_api_key_owner(
                &api_key.key_id,
                &api_key.owner_user_id,
                &api_key.name,
                api_key.role.as_str(),
            )
            .await?;
    }

    let redis = match config.app.redis_url_value().trim() {
        "" => None,
        url => match redis::Client::open(url) {
            Ok(client) => match client.get_connection_manager().await {
                Ok(manager) => Some(manager),
                Err(error) => {
                    tracing::warn!(error = %error, "redis connection manager unavailable");
                    None
                }
            },
            Err(error) => {
                tracing::warn!(error = %error, "invalid redis url");
                None
            }
        },
    };

    Ok(AppState {
        store: Arc::new(store),
        search_index: Arc::new(search_index),
        api_keys: Arc::new(api_keys),
        redis,
        rate_limiter: Arc::new(RateLimiter::new()),
        config: Arc::new(config),
        started_at: Instant::now(),
        #[cfg(feature = "llm")]
        llm,
    })
}

fn load_api_keys() -> HashMap<String, ApiKey> {
    let mut registry = HashMap::new();
    for index in 1..=50 {
        let env_key = format!("API_KEY_{}", index);
        let Ok(value) = std::env::var(&env_key) else {
            continue;
        };
        let parts: Vec<&str> = value.splitn(4, ',').collect();
        if parts.len() < 3 {
            tracing::warn!(env_key = %env_key, "invalid api key format");
            continue;
        }
        let raw_key = parts[0].trim();
        let name = parts[1].trim();
        let role = parts[2].trim().parse::<ApiRole>().unwrap_or(ApiRole::Viewer);
        let key_id = format!("key-{}", index);
        let owner_user_id = parts
            .get(3)
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string())
            .unwrap_or_else(|| format!("usr-{}", key_id));
        registry.insert(
            key_id.clone(),
            ApiKey {
                key_id,
                owner_user_id,
                key_hash: auth::hash_api_key(raw_key),
                name: name.to_string(),
                role,
                created_at: Utc::now(),
                expires_at: None,
                enabled: true,
                rate_limit_per_min: 120,
                allowed_origins: vec![],
            },
        );
    }
    registry
}

#[cfg(feature = "llm")]
fn infer_llm_provider(config: &ApiRuntimeConfig, base_url: &str) -> LlmProvider {
    match config.llm.provider_override {
        Some(apex_api::config::LlmProviderChoice::LlamaCpp) => LlmProvider::LlamaCpp,
        Some(apex_api::config::LlmProviderChoice::OpenAi) => LlmProvider::OpenAi,
        Some(apex_api::config::LlmProviderChoice::AzureOpenAi) => LlmProvider::AzureOpenAi,
        None => {
            let lower = base_url.to_ascii_lowercase();
            if lower.contains("openai.azure.com") {
                LlmProvider::AzureOpenAi
            } else if lower.contains("api.openai.com") {
                LlmProvider::OpenAi
            } else {
                LlmProvider::LlamaCpp
            }
        }
    }
}

#[cfg(feature = "llm")]
fn build_llm_runtime(config: &ApiRuntimeConfig) -> Result<Option<LlmRuntime>> {
    let Some(base_url) = config
        .app
        .llm_base_url
        .as_ref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };

    let provider = infer_llm_provider(config, base_url);
    let api_key = config.app.llm_api_key_value().map(|value| value.to_string());

    let primary = ModelConfig {
        model_name: config.llm_model_name().to_string(),
        provider: provider.clone(),
        base_url: base_url.to_string(),
        api_key: api_key.clone(),
        max_tokens: config.llm.primary_max_tokens,
        temperature: 0.2,
        timeout_seconds: config.llm.primary_timeout_secs,
    };

    let lightweight = ModelConfig {
        model_name: config.llm_model_name().to_string(),
        provider,
        base_url: base_url.to_string(),
        api_key,
        max_tokens: config.llm.lightweight_max_tokens,
        temperature: 0.1,
        timeout_seconds: config.llm.lightweight_timeout_secs,
    };

    Ok(Some(LlmRuntime { primary, lightweight }))
}

async fn require_auth(
    State(state): State<AppState>,
    mut request: axum::extract::Request,
    next: middleware::Next,
) -> axum::response::Response {
    let now = Utc::now();
    let method = request.method().clone();
    let path = request.uri().path().to_string();
    let authenticated = match authenticate_api_request(
        request.headers(),
        &method,
        state.api_keys.as_ref(),
        now,
    ) {
        Ok(authenticated) => authenticated,
        Err(error) => return auth_error_response(error),
    };

    match enforce_rate_limit(
        state.redis.clone(),
        state.rate_limiter.as_ref(),
        &authenticated.auth_context.key_id,
        authenticated.rate_limit_per_min,
        &path,
        &method,
        now,
    )
    .await
    {
        RateLimitDecision::Allowed(info) => {
            request.extensions_mut().insert(authenticated.auth_context);
            request.extensions_mut().insert(info);
            next.run(request).await
        }
        RateLimitDecision::Limited {
            info,
            retry_after_secs,
        } => rate_limited_response(retry_after_secs, info),
    }
}

async fn add_rate_limit_headers(
    request: axum::extract::Request,
    next: middleware::Next,
) -> axum::response::Response {
    let info = request.extensions().get::<RateLimitInfo>().copied();
    let mut response = next.run(request).await;
    if let Some(info) = info {
        append_rate_limit_headers(response.headers_mut(), &info);
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

async fn health_live() -> StatusCode {
    StatusCode::OK
}

async fn health_ready(State(state): State<AppState>) -> (StatusCode, Json<HealthResponse>) {
    let started_at = STARTED_AT.get_or_init(Utc::now);
    let uptime_secs = Utc::now()
        .signed_duration_since(*started_at)
        .num_seconds()
        .max(0) as u64;

    let database = match sqlx::query("SELECT 1").fetch_one(&state.store.pool).await {
        Ok(_) => ComponentHealth {
            name: "database".to_string(),
            status: HealthStatus::Healthy,
            message: Some("Connection OK".to_string()),
        },
        Err(error) => ComponentHealth {
            name: "database".to_string(),
            status: HealthStatus::Unhealthy,
            message: Some(format!("Connection failed: {}", error)),
        },
    };

    let search = ComponentHealth {
        name: "search_index".to_string(),
        status: HealthStatus::Healthy,
        message: Some("Index available".to_string()),
    };

    let api_keys = if state.api_keys.is_empty() {
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

    let checks = vec![
        database,
        search,
        api_keys,
        ComponentHealth {
            name: "api".to_string(),
            status: HealthStatus::Healthy,
            message: None,
        },
    ];
    let overall = aggregate_health(&checks);
    let status_code = match overall {
        HealthStatus::Healthy | HealthStatus::Degraded => StatusCode::OK,
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

async fn health_deep(
    State(state): State<AppState>,
) -> (StatusCode, Json<apex_api::routes::health::DeepHealthCheck>) {
    let llm_base_url = state
        .config
        .app
        .llm_base_url
        .clone()
        .unwrap_or_else(|| "http://127.0.0.1:11434".to_string());
    let response = apex_api::routes::health::deep_health_check(
        &state.store.pool,
        state.config.app.redis_url_value(),
        &state.config.app.nats_url,
        &state.config.app.minio_url,
        &llm_base_url,
        state.started_at,
    )
    .await;

    let status = match response.status.as_str() {
        "healthy" | "degraded" => StatusCode::OK,
        _ => StatusCode::SERVICE_UNAVAILABLE,
    };
    (status, Json(response))
}

async fn endpoints() -> Json<Vec<routes::EndpointDef>> {
    Json(routes::all_endpoints())
}

async fn openapi_json() -> Json<serde_json::Value> {
    Json(routes::openapi_spec())
}

async fn api_features() -> Json<serde_json::Value> {
    #[derive(serde::Serialize)]
    struct ApiFeatureMatrix {
        llm: bool,
        experimental_llm_tool_calling: bool,
        versioned_api_alias: bool,
        openapi: bool,
    }

    Json(
        serde_json::to_value(ApiFeatureMatrix {
            llm: apex_api::API_LLM_FEATURE_ENABLED,
            experimental_llm_tool_calling: apex_api::API_EXPERIMENTAL_LLM_TOOL_CALLING_ENABLED,
            versioned_api_alias: apex_api::API_VERSIONED_ALIAS_ENABLED,
            openapi: apex_api::API_OPENAPI_ENABLED,
        })
        .unwrap_or_else(|err| panic!("failed to serialize api feature matrix: {err}")),
    )
}

async fn api_docs() -> Html<String> {
    Html(format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>ApexIntel API Docs</title></head><body><main><h1>ApexIntel API</h1><p>Machine-readable spec: <a href=\"{}\">{}</a></p><p>Feature matrix: <a href=\"{}\">{}</a></p></main></body></html>",
        routes::paths::OPENAPI_JSON,
        routes::paths::OPENAPI_JSON,
        routes::paths::FEATURES,
        routes::paths::FEATURES,
    ))
}

async fn get_admin_crawl_status(
    State(state): State<AppState>,
) -> (StatusCode, Json<ApiResponse<AdminCrawlStatus>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    match state.store.get_admin_crawl_status().await {
        Ok(status) => {
            let dur = start.elapsed().as_millis() as u64;
            log_latency("get_admin_crawl_status", dur);
            (
                StatusCode::OK,
                Json(success_with_meta(
                    status,
                    ResponseMeta::now()
                        .with_request_id(request_id)
                        .with_duration(dur),
                )),
            )
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "admin crawl status failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal("Failed to load crawl status"))),
            )
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
            (
                StatusCode::OK,
                Json(success_with_meta(
                    perf,
                    ResponseMeta::now()
                        .with_request_id(request_id)
                        .with_duration(dur),
                )),
            )
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "admin recipe performance failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to load recipe performance",
                ))),
            )
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
            (
                StatusCode::OK,
                Json(success_with_meta(
                    coverage,
                    ResponseMeta::now()
                        .with_request_id(request_id)
                        .with_duration(dur),
                )),
            )
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "admin poi coverage failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal("Failed to load POI coverage"))),
            )
        }
    }
}

#[derive(Debug, Deserialize)]
struct TriggerScanRequest {
    job_kind: String,
}

#[derive(Debug, Serialize)]
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
    let valid_kinds = [
        "crawl_cycle",
        "pattern_mining",
        "hypothesis_generation",
        "poi_refresh",
        "promotion_board",
        "recipe_deprecation",
        "strategy_memo",
        "feature_drift_check",
        "source_scoring",
        "cross_domain_mining",
        "outcome_tracking",
        "breach_scan",
        "sanctions_screen",
        "sla_enforcement",
        "dns_posture_scan",
        "kev_catalog_fetch",
        "lookalike_domain_scan",
        "self_improvement_cycle",
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
            (
                StatusCode::ACCEPTED,
                Json(success_with_meta(
                    TriggerScanResponse {
                        trigger_id,
                        job_kind: body.job_kind,
                    },
                    ResponseMeta::now()
                        .with_request_id(request_id)
                        .with_duration(dur),
                )),
            )
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "queue_job_trigger failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal("Failed to queue job trigger"))),
            )
        }
    }
}

#[derive(Debug, Deserialize)]
struct WarningsWsAuthQuery {
    access_token: Option<String>,
}

async fn warnings_ws(
    ws: axum::extract::ws::WebSocketUpgrade,
    State(state): State<AppState>,
    Query(query): Query<WarningsWsAuthQuery>,
    headers: axum::http::HeaderMap,
) -> axum::response::Response {
    let token = match extract_websocket_token(
        &headers,
        query.access_token.as_deref(),
        WebSocketAuthOptions::default(),
    ) {
        Ok(token) => token,
        Err(_) => {
            return axum::response::Response::builder()
                .status(StatusCode::UNAUTHORIZED)
                .body(axum::body::Body::from("Missing authentication"))
                .unwrap_or_else(|err| panic!("failed to build unauthorized websocket response: {err}"));
        }
    };

    if validate_websocket_token(&token, state.api_keys.as_ref(), Utc::now()).is_err() {
        return axum::response::Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .body(axum::body::Body::from("Invalid or expired token"))
            .unwrap_or_else(|err| panic!("failed to build websocket auth failure response: {err}"));
    }

    ws.on_upgrade(move |socket| warnings_ws_stream(socket, state))
}

async fn warnings_ws_stream(mut socket: axum::extract::ws::WebSocket, state: AppState) {
    use axum::extract::ws::Message;

    tracing::info!("WebSocket client connected to /ws/warnings");
    let mut last_check = Utc::now();
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));

    loop {
        tokio::select! {
            _ = interval.tick() => {
                let filters = WarningListFilters {
                    regions: vec![],
                    severities: vec![],
                    warning_types: vec![],
                    acknowledged: Some(false),
                    date_from: Some(last_check),
                    date_to: None,
                    search: None,
                    exclude_hygiene_signals: false,
                    include_deleted: false,
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
                                    if socket.send(Message::Text(json)).await.is_err() {
                                        tracing::info!("WebSocket client disconnected (send error)");
                                        return;
                                    }
                                }
                            }
                        }
                        last_check = Utc::now();
                    }
                    Err(error) => {
                        tracing::warn!("WS warning poll error: {error:#}");
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
                    Some(Ok(_)) => {}
                    Some(Err(error)) => {
                        tracing::warn!("WebSocket read error: {error:#}");
                        return;
                    }
                }
            }
        }
    }
}

fn pagination(page: Option<u32>, per_page: Option<u32>) -> (u32, u32, i64) {
    let page = page.unwrap_or(1).max(1);
    let per_page = per_page.unwrap_or(25).clamp(1, 500);
    let offset = ((page - 1) as i64).saturating_mul(per_page as i64);
    (page, per_page, offset)
}

fn validate_pagination(
    page: Option<u32>,
    per_page: Option<u32>,
) -> Result<(u32, u32, i64), ApiError> {
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

fn clamp_page(page: u32, per_page: u32, total: u64) -> u32 {
    let total_pages = total
        .saturating_add(per_page as u64 - 1)
        / per_page.max(1) as u64;
    let total_pages = total_pages.max(1).min(u32::MAX as u64) as u32;
    page.clamp(1, total_pages)
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
    let Some(raw) = value.as_ref() else {
        return Ok(Vec::new());
    };

    let mut result = Vec::new();
    for token in raw.split(',') {
        let trimmed = token.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.chars().count() > max_len {
            return Err(ApiError::validation(field, format!("{} token too long", field)));
        }
        if !trimmed
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
        {
            return Err(ApiError::validation(
                field,
                format!("{} contains invalid characters", field),
            ));
        }
        result.push(if upper {
            trimmed.to_ascii_uppercase()
        } else {
            trimmed.to_ascii_lowercase()
        });
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
                .map(|item| depth(item, current + 1))
                .max()
                .unwrap_or(current + 1),
            serde_json::Value::Object(map) => map
                .values()
                .map(|item| depth(item, current + 1))
                .max()
                .unwrap_or(current + 1),
            _ => current,
        }
    }

    let actual = depth(value, 0);
    if actual > max_depth {
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
    tracing::info!(endpoint, duration_ms, bucket, "request_latency");
}

fn company_row_to_detail(
    row: CompanyRow,
    sites: Vec<SiteRow>,
    certifications: Vec<CertificationRow>,
    persons: Vec<PersonRow>,
) -> CompanyDetail {
    let mut capabilities = BTreeSet::new();
    for capability in row.industry_tags.clone().unwrap_or_default() {
        capabilities.insert(capability);
    }
    for site in &sites {
        for capability in site.capabilities.clone().unwrap_or_default() {
            capabilities.insert(capability);
        }
    }

    let certifications = certifications
        .into_iter()
        .map(|row| row.standard)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();

    let site_items = sites
        .into_iter()
        .map(|site| CompanySite {
            name: site.name,
            location: [site.address, site.city, site.region, site.country_code]
                .into_iter()
                .flatten()
                .filter(|value| !value.trim().is_empty())
                .collect::<Vec<_>>()
                .join(", "),
            site_type: site.site_type.unwrap_or_else(|| "site".to_string()),
        })
        .collect::<Vec<_>>();

    let key_persons = persons
        .into_iter()
        .map(|person| CompanyKeyPerson {
            person_id: person.id.to_string(),
            name: person.name,
            role: person
                .current_role
                .or(person.role_family)
                .unwrap_or_else(|| "Unknown".to_string()),
        })
        .collect();

    let city = row
        .metadata
        .as_ref()
        .and_then(|value| value.get("city"))
        .and_then(|value| value.as_str())
        .map(|value| value.to_string());
    let is_competitor = row
        .metadata
        .as_ref()
        .and_then(|value| value.get("is_competitor"))
        .and_then(|value| value.as_bool())
        .unwrap_or(false);

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
        capabilities: capabilities.into_iter().collect(),
        certifications,
        sites: site_items,
        key_persons,
        recent_events: Vec::new(),
        community_badges: Vec::new(),
        source_entropy: None,
        source_quality_label: None,
        created_at: row.created_at.unwrap_or_else(Utc::now),
        updated_at: row.updated_at.unwrap_or_else(Utc::now),
    }
}

fn person_row_to_detail(
    row: PersonRow,
    organization: String,
    artifacts: Vec<ArtifactRow>,
    weights: &PriorityWeights,
) -> PersonDetail {
    let role = row
        .current_role
        .clone()
        .or(row.role_family.clone())
        .unwrap_or_else(|| "Unknown".to_string());
    let role_family = row
        .role_family
        .clone()
        .unwrap_or_else(|| "Unknown".to_string());
    let influence_seed = clamp_ratio(row.influence_score.unwrap_or(0.0));
    let topic_density = row
        .trigger_topics
        .as_ref()
        .map(|topics| clamp_ratio(topics.len() as f64 / 5.0))
        .unwrap_or(0.0);
    let artifact_density = clamp_ratio(artifacts.len() as f64 / 8.0);
    let decision_power = if role_family.eq_ignore_ascii_case("executive") {
        0.9
    } else if role_family.eq_ignore_ascii_case("technology")
        || role_family.eq_ignore_ascii_case("operations")
    {
        0.7
    } else {
        0.5
    };
    let engagement_potential = if row.public_email.is_some() { 0.85 } else { 0.45 };
    let priority_vector = PriorityVector {
        decision_power,
        domain_relevance: topic_density.max(artifact_density),
        network_centrality: influence_seed,
        engagement_potential,
        intelligence_value: clamp_ratio((topic_density + artifact_density + influence_seed) / 3.0),
    };
    let priority_score = priority_vector.composite_with_weights(weights);
    let influence_score = (priority_score * 100.0).round() as i64;
    let priority = if influence_score >= 80 {
        "A"
    } else if influence_score >= 50 {
        "B"
    } else {
        "C"
    }
    .to_string();

    let timeline = artifacts
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
        .collect::<Vec<_>>();

    let data_completeness = {
        let mut count = 0.0;
        if row.public_bio.is_some() {
            count += 1.0;
        }
        if row.public_email.is_some() {
            count += 1.0;
        }
        if !row.trigger_topics.clone().unwrap_or_default().is_empty() {
            count += 1.0;
        }
        if !timeline.is_empty() {
            count += 1.0;
        }
        if row.region.is_some() {
            count += 1.0;
        }
        if row.country_code.is_some() {
            count += 1.0;
        }
        clamp_ratio(count / 6.0)
    };

    let engagement_readiness = clamp_ratio(
        (if row.public_email.is_some() { 0.4 } else { 0.15 })
            + (if !timeline.is_empty() { 0.25 } else { 0.0 })
            + (priority_score * 0.35),
    );

    let affiliations = if organization.is_empty() || organization == "Independent" {
        Vec::new()
    } else {
        vec![Affiliation {
            organization: organization.clone(),
            role: role.clone(),
            current: true,
        }]
    };

    let phone = row
        .metadata
        .as_ref()
        .and_then(|value| value.get("phone"))
        .and_then(|value| value.as_str())
        .map(|value| value.to_string());
    let linkedin = row
        .metadata
        .as_ref()
        .and_then(|value| value.get("linkedin"))
        .and_then(|value| value.as_str())
        .map(|value| value.to_string());
    let engagement_status = row
        .metadata
        .as_ref()
        .and_then(|value| value.get("engagement_status"))
        .and_then(|value| value.as_str())
        .unwrap_or("unknown")
        .to_string();

    let mut name_alt = Vec::new();
    if let Some(value) = row.name_ar.clone().filter(|value| !value.is_empty()) {
        name_alt.push(value);
    }
    if let Some(value) = row.name_fr.clone().filter(|value| !value.is_empty()) {
        name_alt.push(value);
    }

    PersonDetail {
        id: row.id.to_string(),
        name: row.name,
        name_alt,
        role,
        role_family,
        organization,
        org_id: row.primary_org_id.map(|id| id.to_string()),
        region: row.region.unwrap_or_default(),
        country: row.country_code.unwrap_or_default(),
        bio: row.public_bio,
        email: row.public_email,
        phone,
        linkedin,
        priority_score,
        influence_score,
        priority,
        priority_vector,
        influence_tier: priority_tier(priority_score).to_string(),
        engagement_status,
        engagement_readiness,
        data_completeness,
        tags: row.trigger_topics.clone().unwrap_or_default(),
        trigger_topics: row.trigger_topics.unwrap_or_default(),
        decision_style: row.decision_style,
        risk_tolerance: row.risk_tolerance,
        change_appetite: row.change_appetite,
        communication_style: row.communication_style,
        decision_mode: None,
        preferred_proof_type: None,
        pain_index: None,
        change_risk: None,
        role_drift_score: None,
        affiliations,
        timeline,
        role_history: Vec::new(),
        peers: Vec::new(),
        warning_count: 0,
        insight_count: 0,
        created_at: row.created_at.unwrap_or_else(Utc::now),
        updated_at: row.updated_at.unwrap_or_else(Utc::now),
    }
}

fn edge_row_to_graph_edge(row: &EdgeRow) -> GraphEdge {
    GraphEdge {
        source: row.source_id.to_string(),
        target: row.target_id.to_string(),
        edge_type: row.edge_type.clone(),
        weight: clamp_ratio(row.weight.or(row.confidence).unwrap_or(0.5)),
        label: None,
    }
}

fn compute_edge_type_counts(rows: &[EdgeRow]) -> Vec<EdgeTypeCount> {
    let mut counts = HashMap::<String, u64>::new();
    for row in rows {
        *counts.entry(row.edge_type.clone()).or_insert(0) += 1;
    }
    let mut values = counts
        .into_iter()
        .map(|(edge_type, count)| EdgeTypeCount { edge_type, count })
        .collect::<Vec<_>>();
    values.sort_by(|left, right| right.count.cmp(&left.count));
    values
}

fn recipe_stat_to_list_item(stat: &RecipeStatRow) -> RecipeListItem {
    let status = parse_recipe_status(&stat.status).unwrap_or(RecipeStatus::Staging);
    let created_at = stat.first_fired.unwrap_or_else(Utc::now);
    let updated_at = stat.last_fired.unwrap_or(created_at);
    RecipeListItem {
        id: stat.recipe_code.clone(),
        name: stat.recipe_code.clone(),
        description: format!("Recipe {}", stat.recipe_code),
        status,
        region: None,
        precision: clamp_ratio(stat.precision_score),
        recall: clamp_ratio(1.0 - stat.false_positive_rate),
        false_positive_rate: clamp_ratio(stat.false_positive_rate),
        fired_count: stat.fired_count.max(0) as u32,
        last_fired: stat.last_fired,
        created_at,
        updated_at,
    }
}

fn parse_recipe_status(raw: &str) -> Option<RecipeStatus> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "staging" => Some(RecipeStatus::Staging),
        "production" | "active" => Some(RecipeStatus::Production),
        "deprecated" => Some(RecipeStatus::Deprecated),
        _ => None,
    }
}

#[allow(clippy::items_after_test_module)]
#[cfg(test)]
mod tests {
    use super::{parse_recipe_status, recipe_stat_to_list_item};
    use apex_api::routes::recipes::RecipeStatus;
    use apex_store::postgres::RecipeStatRow;
    use chrono::Utc;

    fn make_recipe_stat(status: &str, fired_count: i64) -> RecipeStatRow {
        let now = Utc::now();
        RecipeStatRow {
            recipe_code: format!("recipe-{}", fired_count),
            status: status.to_string(),
            precision_score: 0.82,
            false_positive_rate: 0.04,
            fired_count,
            last_fired: Some(now),
            first_fired: Some(now),
            active_count: fired_count.max(0),
        }
    }

    #[test]
    fn recipe_status_reflects_explicit_store_status() {
        let active_recipe = recipe_stat_to_list_item(&make_recipe_stat("active", 0));
        let deprecated_recipe = recipe_stat_to_list_item(&make_recipe_stat("deprecated", 12));

        assert_eq!(active_recipe.status, RecipeStatus::Production);
        assert_eq!(deprecated_recipe.status, RecipeStatus::Deprecated);
        assert_eq!(parse_recipe_status("active"), Some(RecipeStatus::Production));
    }

    #[test]
    fn recipe_status_does_not_change_when_warning_count_fluctuates() {
        let quiet = recipe_stat_to_list_item(&make_recipe_stat("production", 0));
        let noisy = recipe_stat_to_list_item(&make_recipe_stat("production", 27));

        assert_eq!(quiet.status, RecipeStatus::Production);
        assert_eq!(noisy.status, RecipeStatus::Production);
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
        CompanySortField::Region => apex_store::postgres::CompanyOrderBy::Region,
        CompanySortField::ThreatScore => apex_store::postgres::CompanyOrderBy::ThreatScore,
        CompanySortField::UpdatedAt => apex_store::postgres::CompanyOrderBy::UpdatedAt,
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
        if RegionFilter::from_code(value).is_none() {
            return Err(ApiError::validation(
                "regions",
                format!("invalid region code: {}", value),
            ));
        }
    }
    Ok(())
}

pub(crate) fn validate_severity_codes(values: &[String]) -> Result<(), ApiError> {
    for value in values {
        if SeverityFilter::from_str_loose(value).is_none() {
            return Err(ApiError::validation(
                "severities",
                format!("invalid severity: {}", value),
            ));
        }
    }
    Ok(())
}

pub(crate) fn validate_warning_type_codes(values: &[String]) -> Result<(), ApiError> {
    for value in values {
        if WarningTypeFilter::from_str_loose(value).is_none() {
            return Err(ApiError::validation(
                "warning_types",
                format!("invalid warning type: {}", value),
            ));
        }
    }
    Ok(())
}

pub(crate) fn parse_date_start(value: &Option<String>) -> Result<Option<DateTime<Utc>>, String> {
    parse_query_date(value, false)
}

pub(crate) fn parse_date_end(value: &Option<String>) -> Result<Option<DateTime<Utc>>, String> {
    parse_query_date(value, true)
}

fn parse_query_date(
    value: &Option<String>,
    end_of_day: bool,
) -> Result<Option<DateTime<Utc>>, String> {
    let Some(raw) = value.as_ref().map(|value| value.trim()).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };

    let date = parse_date(raw).ok_or_else(|| format!("invalid date: {}", raw))?;
    let naive = if end_of_day {
        date.and_hms_opt(23, 59, 59)
    } else {
        date.and_hms_opt(0, 0, 0)
    }
    .ok_or_else(|| format!("invalid date: {}", raw))?;
    Ok(Some(DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc)))
}

pub(crate) fn validate_date_range(
    from: &Option<DateTime<Utc>>,
    to: &Option<DateTime<Utc>>,
) -> Result<(), String> {
    if let (Some(from), Some(to)) = (from.as_ref(), to.as_ref()) {
        if from > to {
            return Err("date_from must be before or equal to date_to".to_string());
        }
    }
    Ok(())
}
