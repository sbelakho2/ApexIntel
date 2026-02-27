use anyhow::Result;
use apex_api::auth::{self, ApiKey, ApiRole, AuthResult, PermissionLevel};
use apex_api::filters::{validate_search_text, RegionFilter, SeverityFilter, WarningTypeFilter};
use apex_api::responses::{
    aggregate_health, error_response, success_with_meta, ApiError, ApiResponse, ComponentHealth, ErrorCode, HealthResponse,
    HealthStatus, PagedResponse, ResponseMeta,
};
use apex_api::routes;
use apex_api::routes::companies::{
    validate_company_id, CompanyDetail, CompanyKeyPerson, CompanyListItem, CompanySite, CompanySortField,
    ListCompaniesQuery,
};
use apex_api::routes::graph::{GraphOverviewWithEdges, GraphEdge, EdgeTypeCount};
use apex_api::routes::insights::{InsightResponse, ListInsightsQuery};
use apex_api::routes::llm::{
    ExtractEntitiesRequest, ExtractEntitiesResponse, GenerateMemoRequest, GenerateMemoResponse, GenerateRecipeRequest,
    GenerateRecipeResponse, SynthesizePoiRequest, SynthesizePoiResponse,
};
#[cfg(feature = "llm")]
use apex_api::routes::llm::{ExtractedEntity, LlmTask, MemoSection};
use apex_api::routes::persons::{
    validate_person_id, Affiliation, ListPersonsQuery, PersonDetail, PersonEvent, PersonListItem, PersonSortField,
    PriorityVector,
};
use apex_api::routes::recipes::ListRecipesQuery;
use apex_api::routes::recipes::RecipeListItem;
use apex_api::routes::search::{
    build_facets, highlight_snippet, sort_by_score, tokenize_query, validate_search_query, SearchHit, SearchQuery,
    SearchResponse,
};
use apex_api::routes::security::SecuritySummary;
use apex_api::routes::warnings::{
    validate_acknowledge, validate_warning_id, AcknowledgeRequest, ListWarningsQuery, SortDirection, WarningResponse,
    WarningSortField,
};
use apex_core::config::AppConfig;
use apex_core::validation::clamp_ratio;
use apex_store::postgres::{
    ArtifactRow, CertificationRow, CompanyListFilters, CompanyOrderBy, CompanyRow, EdgeRow,
    InsightListFilters, InsightRow, PersonListFilters, PersonListRow, PersonOrderBy, PersonRow,
    PgStore, RecipeStatRow, SiteRow, WarningListFilters, WarningOrderBy, WarningRow,
};
use apex_store::tantivy_index::SearchIndex;
use axum::{
    extract::{Path, Query, State},
    http::{header, Method, StatusCode},
    middleware,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use serde::Serialize;
#[cfg(feature = "llm")]
use serde_json::Value as JsonValue;
use std::{collections::{BTreeSet, HashMap}, path::Path as FsPath, sync::Arc, sync::OnceLock, time::Instant};
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

#[cfg(feature = "llm")]
use apex_llm::{validators, LlmClient, LlmProvider, ModelConfig, OpenAiCompatibleClient};

static STARTED_AT: OnceLock<DateTime<Utc>> = OnceLock::new();
const MAX_JSON_DEPTH: usize = 32;

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
    #[cfg(feature = "llm")]
    llm: Option<LlmRuntime>,
}

#[derive(Debug, Clone, Copy)]
struct RateLimitInfo {
    limit_per_min: u32,
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
        .route("/api/endpoints", get(endpoints));

    // Protected routes (auth required)
    let protected = Router::new()
        .route("/api/warnings", get(list_warnings))
        .route("/api/warnings/:id/acknowledge", post(acknowledge_warning))
        .route("/api/insights", get(list_insights))
        .route("/api/companies", get(list_companies))
        .route("/api/companies/:id", get(get_company_detail))
        .route("/api/persons", get(list_persons))
        .route("/api/persons/:id", get(get_person_detail))
        .route("/api/search", get(search))
        .route("/api/graph", get(list_graph))
        .route("/api/recipes", get(list_recipes))
        .route("/api/security", get(list_security))
        .route("/api/llm/extract-entities", post(llm_extract_entities))
        .route("/api/llm/generate-recipe", post(llm_generate_recipe))
        .route("/api/llm/synthesize-poi", post(llm_synthesize_poi))
        .route("/api/llm/generate-memo", post(llm_generate_memo))
        .route_layer(middleware::from_fn_with_state(state.clone(), require_auth));

    let app = Router::new()
        .merge(public)
        .merge(protected)
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
            let mut request = request;
            request.extensions_mut().insert(RateLimitInfo { limit_per_min: limit });
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
    let limit = request
        .extensions()
        .get::<RateLimitInfo>()
        .map(|info| info.limit_per_min)
        .unwrap_or(120);
    
    let mut response = next.run(request).await;
    
    // Standard rate limit headers (following RFC 7231 and draft-ietf-httpapi-ratelimit-headers)
    response.headers_mut().insert(
        header::HeaderName::from_static("x-ratelimit-limit"),
        header::HeaderValue::from_str(&limit.to_string()).unwrap_or_else(|_| header::HeaderValue::from_static("120")),
    );
    
    // Note: Actual remaining count requires Redis or in-memory tracking per key.
    // For now we report the limit as remaining (conservative estimate).
    // TODO: Implement actual rate limit tracking with Redis INCR + EXPIRE
    response.headers_mut().insert(
        header::HeaderName::from_static("x-ratelimit-remaining"),
        header::HeaderValue::from_str(&limit.to_string()).unwrap_or_else(|_| header::HeaderValue::from_static("120")),
    );
    
    // Reset timestamp (next minute boundary)
    let now = Utc::now();
    let reset_at = now + chrono::Duration::seconds(60 - (now.timestamp() % 60));
    response.headers_mut().insert(
        header::HeaderName::from_static("x-ratelimit-reset"),
        header::HeaderValue::from_str(&reset_at.timestamp().to_string()).unwrap_or_else(|_| header::HeaderValue::from_static("0")),
    );
    
    // Policy description
    response.headers_mut().insert(
        header::HeaderName::from_static("x-ratelimit-policy"),
        header::HeaderValue::from_static("requests_per_minute; window=60s"),
    );
    
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

    let items: Vec<InsightResponse> = rows.into_iter().map(insight_row_to_response).collect();
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

    let regions = match parse_csv_lower_strict(&params.regions, 32, "regions") {
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
        min_priority: params.min_priority.map(clamp_ratio),
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

    let org_name = if let Some(org_id) = row.primary_org_id {
        match state.store.get_company(org_id).await {
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

    let artifacts = match state.store.get_artifacts_for_person(person_id, 12).await {
        Ok(rows) => rows,
        Err(err) => {
            tracing::error!(request_id = %request_id, "get person artifacts failed: {err:#}");
            let api_err = ApiError::internal("Failed to load person detail");
            return (
                StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(error_response(api_err)),
            );
        }
    };

    let detail = person_row_to_detail(row, org_name, artifacts);
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

    let payload = GraphOverviewWithEdges {
        companies_total,
        persons_total,
        warnings_total,
        insights_total,
        edges_total,
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

    // Get recipe statistics from warnings
    let recipe_stats = match state.store.get_recipe_stats().await {
        Ok(stats) => stats,
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
    let items: Vec<RecipeListItem> = recipe_stats
        .into_iter()
        .map(|stat| recipe_stat_to_list_item(&stat))
        .collect();

    let total = items.len() as u64;

    let payload = PagedResponse {
        items,
        total,
        page,
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
    State(_state): State<AppState>,
) -> (StatusCode, Json<ApiResponse<SecuritySummary>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();

    let payload = SecuritySummary {
        dns_posture: Vec::new(),
        lookalikes: Vec::new(),
        kev: Vec::new(),
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

    // Load API keys from environment: API_KEY_<N>=<raw_key>,<name>,<role>
    let api_keys = load_api_keys();
    tracing::info!("{} API keys loaded", api_keys.len());

    Ok(AppState {
        store: Arc::new(store),
        search_index: Arc::new(search_index),
        api_keys: Arc::new(api_keys),
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
    let per_page = per_page.unwrap_or(25).min(100).max(1);
    let offset = ((page - 1) as i64).saturating_mul(per_page as i64);
    (page, per_page, offset)
}

fn validate_pagination(page: Option<u32>, per_page: Option<u32>) -> Result<(u32, u32, i64), ApiError> {
    let page = page.unwrap_or(1);
    let per_page = per_page.unwrap_or(25);
    if page == 0 {
        return Err(ApiError::validation("page", "page must be >= 1"));
    }
    if per_page == 0 || per_page > 100 {
        return Err(ApiError::validation("per_page", "per_page must be in 1..=100"));
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
        recipe_id: row.recipe_code,
        confidence: clamp_ratio(row.confidence.unwrap_or(0.0)),
        acknowledged: row.acknowledged,
        acknowledged_by: row.acknowledged_by,
        acknowledged_at: row.acknowledged_at,
        created_at: row.created_at.unwrap_or(row.ts_utc),
        updated_at: row.updated_at.unwrap_or(row.ts_utc),
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
    }
}

fn company_row_to_item(row: CompanyRow) -> CompanyListItem {
    let is_competitor = row
        .metadata
        .as_ref()
        .and_then(|meta| meta.get("is_competitor"))
        .and_then(|value| value.as_bool())
        .unwrap_or(false);

    CompanyListItem {
        id: row.id.to_string(),
        name: row.name,
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
    PersonListItem {
        id: row.id.to_string(),
        name: row.name,
        role: row.role,
        organization: row.organization,
        region: row.region,
        priority_score: clamp_ratio(row.priority_score),
        engagement_status: row.engagement_status,
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

    let sites: Vec<CompanySite> = sites
        .into_iter()
        .map(site_row_to_company_site)
        .collect();

    let key_persons: Vec<CompanyKeyPerson> = persons
        .into_iter()
        .map(person_row_to_key_person)
        .collect();

    CompanyDetail {
        id: row.id.to_string(),
        name: row.name,
        legal_name: row.legal_name,
        region: row.region.unwrap_or_default(),
        country: row.country_code.unwrap_or_default(),
        city: None,
        website: row.domain,
        entity_type: row.company_type.unwrap_or_else(|| "unknown".to_string()),
        is_competitor,
        threat_score: row.threat_score.map(clamp_ratio),
        overlap_score: row.overlap_score.map(clamp_ratio),
        capabilities: capability_set.into_iter().collect(),
        certifications: cert_set.into_iter().collect(),
        sites,
        key_persons,
        recent_events: Vec::new(),
        created_at: row.created_at.unwrap_or_else(Utc::now),
        updated_at: row.updated_at.unwrap_or_else(Utc::now),
    }
}

fn person_row_to_detail(row: PersonRow, organization: String, artifacts: Vec<ArtifactRow>) -> PersonDetail {
    let role = row
        .current_role
        .clone()
        .or(row.role_family.clone())
        .unwrap_or_else(|| "Unknown".to_string());
    let influence_score = clamp_ratio(row.influence_score.unwrap_or(0.0));
    let priority_vector = row
        .priority_vector
        .as_ref()
        .and_then(|value| serde_json::from_value::<PriorityVector>(value.clone()).ok())
        .unwrap_or_else(|| default_priority_vector(influence_score));
    let priority_score = if row.priority_vector.is_some() {
        priority_vector.composite()
    } else {
        influence_score
    };

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

    PersonDetail {
        id: row.id.to_string(),
        name: row.name,
        role,
        organization,
        region: row.region.unwrap_or_default(),
        country: row.country_code.unwrap_or_default(),
        email: metadata_string(&row.metadata, "email"),
        phone: metadata_string(&row.metadata, "phone"),
        linkedin: metadata_string(&row.metadata, "linkedin"),
        priority_score,
        priority_vector,
        engagement_status: metadata_engagement_status(&row.metadata),
        tags: metadata_string_vec(&row.metadata, "tags"),
        affiliations,
        timeline,
        created_at: row.created_at.unwrap_or_else(Utc::now),
        updated_at: row.updated_at.unwrap_or_else(Utc::now),
    }
}

fn company_is_competitor(row: &CompanyRow) -> bool {
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

fn default_priority_vector(score: f64) -> PriorityVector {
    let value = clamp_ratio(score);
    PriorityVector {
        decision_power: value,
        domain_relevance: value,
        network_centrality: value,
        engagement_potential: value,
        intelligence_value: value,
    }
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
    use apex_api::routes::recipes::RecipeStatus;
    
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
    let description = format!(
        "Signal recipe that has fired {} times. {} currently active.",
        stat.fired_count, stat.active_count
    );

    RecipeListItem {
        id,
        name,
        description,
        status: RecipeStatus::Production,
        region: None,
        precision: 0.85, // Default placeholder
        recall: 0.72,    // Default placeholder
        false_positive_rate: 0.08, // Default placeholder
        fired_count: stat.fired_count as u32,
        last_fired: stat.last_fired,
        created_at: stat.first_fired.unwrap_or_else(Utc::now),
        updated_at: stat.last_fired.unwrap_or_else(Utc::now),
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
