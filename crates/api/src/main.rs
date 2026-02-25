use anyhow::Result;
use apex_api::auth::{self, ApiKey, ApiRole, AuthResult, PermissionLevel};
use apex_api::filters::{validate_search_text, RegionFilter, SeverityFilter, WarningTypeFilter};
use apex_api::responses::{
    aggregate_health, error_response, success_with_meta, ApiError, ApiResponse, ComponentHealth, HealthResponse, HealthStatus,
    PagedResponse, ResponseMeta,
};
use apex_api::routes;
use apex_api::routes::companies::{CompanyListItem, CompanySortField, ListCompaniesQuery};
use apex_api::routes::insights::{InsightResponse, ListInsightsQuery};
use apex_api::routes::search::{
    build_facets, highlight_snippet, sort_by_score, tokenize_query, validate_search_query, SearchHit, SearchQuery,
    SearchResponse,
};
use apex_api::routes::warnings::{
    validate_acknowledge, validate_warning_id, AcknowledgeRequest, ListWarningsQuery, SortDirection, WarningResponse,
    WarningSortField,
};
use apex_core::config::AppConfig;
use apex_core::validation::clamp_ratio;
use apex_store::postgres::{
    CompanyListFilters, CompanyOrderBy, CompanyRow, InsightListFilters, InsightRow, PgStore, WarningListFilters,
    WarningOrderBy, WarningRow,
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
use std::{collections::HashMap, path::Path as FsPath, sync::Arc, sync::OnceLock, time::Instant};
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

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
}

#[derive(Debug, Clone, Copy)]
struct RateLimitInfo {
    limit_per_min: u32,
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
        .route("/api/endpoints", get(endpoints));

    // Protected routes (auth required)
    let protected = Router::new()
        .route("/api/warnings", get(list_warnings))
        .route("/api/warnings/:id/acknowledge", post(acknowledge_warning))
        .route("/api/insights", get(list_insights))
        .route("/api/companies", get(list_companies))
        .route("/api/search", get(search))
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
    mut request: axum::extract::Request,
    next: middleware::Next,
) -> axum::response::Response {
    let limit = request
        .extensions()
        .get::<RateLimitInfo>()
        .map(|info| info.limit_per_min)
        .unwrap_or(120);
    let mut response = next.run(request).await;
    response.headers_mut().insert(
        header::HeaderName::from_static("x-ratelimit-limit"),
        header::HeaderValue::from_str(&limit.to_string()).unwrap_or_else(|_| header::HeaderValue::from_static("120")),
    );
    response.headers_mut().insert(
        header::HeaderName::from_static("x-ratelimit-policy"),
        header::HeaderValue::from_static("burst=60, window=60"),
    );
    response
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
