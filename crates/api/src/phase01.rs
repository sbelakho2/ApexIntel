use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use axum::extract::{Path, Query, Request, State};
use axum::http::{header, HeaderMap, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Response};
use axum::{Extension, Json, Router};
use chrono::{DateTime, Utc};
use serde_json::Value;
use uuid::Uuid;

use crate::auth::{self, ApiKey, AuthResult, PermissionLevel};
use crate::destructive_actions::{
    authorize_delete_all_warnings, delete_all_warnings_audit_payload, ApiAuthContext,
};
use crate::responses::{
    aggregate_health, error_response, success_with_meta, ApiError, ApiResponse, ComponentHealth,
    HealthResponse, HealthStatus, PagedResponse, ResponseMeta,
};
use crate::routes;
use crate::routes::companies::{CompanyDetail, CompanyEvent, CompanyKeyPerson, CompanySite};
use crate::routes::insights::{InsightResponse, ListInsightsQuery};
use crate::routes::warnings::{
    ListWarningsQuery, SortDirection, WarningResponse, WarningSortField,
};
use apex_store::postgres::{
    CertificationRow, CompanyRow, InsightListFilters, InsightRow, PersonRow, SiteRow,
    WarningListFilters, WarningOrderBy, WarningRow,
};

#[derive(Clone)]
pub struct Phase01State {
    pub store: Arc<dyn Phase01Store>,
    pub api_keys: Arc<HashMap<String, ApiKey>>,
    pub started_at: DateTime<Utc>,
    pub search_ready: bool,
}

#[async_trait]
pub trait Phase01Store: Send + Sync + 'static {
    async fn health_check(&self) -> Result<()>;
    async fn count_warnings(&self, filters: &WarningListFilters) -> Result<i64>;
    async fn list_warnings(
        &self,
        filters: &WarningListFilters,
        order_by: Option<WarningOrderBy>,
        desc: bool,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<WarningRow>>;
    async fn delete_all_warnings(&self) -> Result<u64>;
    async fn record_audit_event(&self, actor: &str, event_type: &str, detail: &Value) -> Result<()>;
    async fn count_insights(&self, filters: &InsightListFilters) -> Result<i64>;
    async fn list_insights(
        &self,
        filters: &InsightListFilters,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<InsightRow>>;
    async fn get_company(&self, id: Uuid) -> Result<Option<CompanyRow>>;
    async fn get_sites_for_company(&self, id: Uuid) -> Result<Vec<SiteRow>>;
    async fn get_certifications_for_company(&self, id: Uuid) -> Result<Vec<CertificationRow>>;
    async fn list_persons_by_org(&self, id: Uuid) -> Result<Vec<PersonRow>>;
}

#[async_trait]
impl Phase01Store for apex_store::postgres::PgStore {
    async fn health_check(&self) -> Result<()> {
        sqlx::query("SELECT 1").fetch_one(&self.pool).await?;
        Ok(())
    }

    async fn count_warnings(&self, filters: &WarningListFilters) -> Result<i64> {
        self.count_warnings(filters).await
    }

    async fn list_warnings(
        &self,
        filters: &WarningListFilters,
        order_by: Option<WarningOrderBy>,
        desc: bool,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<WarningRow>> {
        self.list_warnings(filters, order_by, desc, limit, offset).await
    }

    async fn delete_all_warnings(&self) -> Result<u64> {
        self.delete_all_warnings().await
    }

    async fn record_audit_event(&self, actor: &str, event_type: &str, detail: &Value) -> Result<()> {
        self.record_audit_event(actor, event_type, detail).await
    }

    async fn count_insights(&self, filters: &InsightListFilters) -> Result<i64> {
        self.count_insights(filters).await
    }

    async fn list_insights(
        &self,
        filters: &InsightListFilters,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<InsightRow>> {
        self.list_insights(filters, limit, offset).await
    }

    async fn get_company(&self, id: Uuid) -> Result<Option<CompanyRow>> {
        self.get_company(id).await
    }

    async fn get_sites_for_company(&self, id: Uuid) -> Result<Vec<SiteRow>> {
        self.get_sites_for_company(id).await
    }

    async fn get_certifications_for_company(&self, id: Uuid) -> Result<Vec<CertificationRow>> {
        self.get_certifications_for_company(id).await
    }

    async fn list_persons_by_org(&self, id: Uuid) -> Result<Vec<PersonRow>> {
        self.list_persons_by_org(id).await
    }
}

pub fn build_phase01_router(state: Phase01State) -> Router {
    let protected = Router::<Phase01State>::new()
        .route("/api/warnings", axum::routing::get(list_warnings).delete(delete_all_warnings))
        .route("/api/insights", axum::routing::get(list_insights))
        .route("/api/companies/:id", axum::routing::get(get_company_detail))
        .route_layer(middleware::from_fn_with_state(state.clone(), require_auth));

    Router::<Phase01State>::new()
        .route("/api/health", axum::routing::get(health))
        .route("/api/health/live", axum::routing::get(health_live))
        .route("/api/health/ready", axum::routing::get(health_ready))
        .route("/api/openapi.json", axum::routing::get(openapi_json))
        .route("/api/docs", axum::routing::get(api_docs))
        .route("/api/features", axum::routing::get(api_features))
        .merge(protected)
        .with_state(state)
}

async fn openapi_json() -> Json<Value> {
    Json(routes::openapi_spec())
}

async fn api_features() -> Json<Value> {
    #[derive(serde::Serialize)]
    struct ApiFeatureMatrix {
        llm: bool,
        experimental_llm_tool_calling: bool,
        versioned_api_alias: bool,
        openapi: bool,
    }

    Json(
        serde_json::to_value(ApiFeatureMatrix {
            llm: crate::API_LLM_FEATURE_ENABLED,
            experimental_llm_tool_calling: crate::API_EXPERIMENTAL_LLM_TOOL_CALLING_ENABLED,
            versioned_api_alias: crate::API_VERSIONED_ALIAS_ENABLED,
            openapi: crate::API_OPENAPI_ENABLED,
        })
        .unwrap_or_else(|_| serde_json::json!({"error": "serialization failed"})),
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

async fn require_auth(
    State(state): State<Phase01State>,
    request: Request,
    next: Next,
) -> Response {
    let token = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(auth::extract_bearer_token);

    let Some(token) = token else {
        return auth_error_response(ApiError::unauthorized());
    };

    let auth_result = auth::validate_token(token, &state.api_keys, Utc::now());
    let required = if request.method() == Method::DELETE {
        PermissionLevel::Write
    } else {
        PermissionLevel::Read
    };

    if !auth::check_permission(&auth_result, required) {
        return auth_error_response(ApiError::forbidden("Insufficient permissions"));
    }

    match auth_result {
        AuthResult::Valid {
            key_id,
            owner_user_id,
            role,
        } => {
            let mut request = request;
            request.extensions_mut().insert(ApiAuthContext {
                key_id,
                user_id: owner_user_id,
                role,
            });
            next.run(request).await
        }
        _ => auth_error_response(ApiError::unauthorized()),
    }
}

fn auth_error_response(err: ApiError) -> Response {
    (
        StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::UNAUTHORIZED),
        Json(error_response::<serde_json::Value>(err)),
    )
        .into_response()
}

async fn health(State(state): State<Phase01State>) -> Json<HealthResponse> {
    let uptime_secs = Utc::now()
        .signed_duration_since(state.started_at)
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

async fn health_ready(
    State(state): State<Phase01State>,
) -> (StatusCode, Json<HealthResponse>) {
    let uptime_secs = Utc::now()
        .signed_duration_since(state.started_at)
        .num_seconds()
        .max(0) as u64;

    let checks = vec![
        match state.store.health_check().await {
            Ok(_) => ComponentHealth {
                name: "database".to_string(),
                status: HealthStatus::Healthy,
                message: Some("Connection OK".to_string()),
            },
            Err(err) => ComponentHealth {
                name: "database".to_string(),
                status: HealthStatus::Unhealthy,
                message: Some(format!("Connection failed: {err}")),
            },
        },
        ComponentHealth {
            name: "search_index".to_string(),
            status: if state.search_ready {
                HealthStatus::Healthy
            } else {
                HealthStatus::Degraded
            },
            message: Some(if state.search_ready {
                "Index available".to_string()
            } else {
                "Index unavailable".to_string()
            }),
        },
        ComponentHealth {
            name: "api_keys".to_string(),
            status: if state.api_keys.is_empty() {
                HealthStatus::Unhealthy
            } else {
                HealthStatus::Healthy
            },
            message: Some(if state.api_keys.is_empty() {
                "No API keys configured".to_string()
            } else {
                format!("{} keys loaded", state.api_keys.len())
            }),
        },
    ];

    let status = aggregate_health(&checks);
    let http_status = match status {
        HealthStatus::Healthy | HealthStatus::Degraded => StatusCode::OK,
        HealthStatus::Unhealthy => StatusCode::SERVICE_UNAVAILABLE,
    };

    (
        http_status,
        Json(HealthResponse {
            status,
            version: env!("CARGO_PKG_VERSION").to_string(),
            uptime_secs,
            checks,
        }),
    )
}

async fn list_warnings(
    State(state): State<Phase01State>,
    Extension(auth_ctx): Extension<ApiAuthContext>,
    Query(params): Query<ListWarningsQuery>,
) -> (StatusCode, Json<ApiResponse<PagedResponse<WarningResponse>>>) {
    if params.include_deleted == Some(true) && !auth_ctx.role.can_admin() {
        let api_err = ApiError::forbidden("Admin role required to include deleted warnings");
        return (StatusCode::FORBIDDEN, Json(error_response(api_err)));
    }

    let page = params.page.unwrap_or(1).max(1);
    let per_page = params.per_page.unwrap_or(25).clamp(1, 100);
    let offset = ((page - 1) as i64) * per_page as i64;
    let filters = WarningListFilters {
        regions: split_csv_upper(params.regions.as_deref()),
        severities: split_csv_lower(params.severities.as_deref()),
        warning_types: split_csv_lower(params.warning_types.as_deref()),
        acknowledged: params.acknowledged,
        date_from: None,
        date_to: None,
        search: params.search.filter(|value| !value.trim().is_empty()),
        exclude_hygiene_signals: false,
        include_deleted: params.include_deleted.unwrap_or(false) && auth_ctx.role.can_admin(),
    };

    let mut total = match state.store.count_warnings(&filters).await {
        Ok(value) => value.max(0) as u64,
        Err(err) => {
            let api_err = ApiError::internal(format!("Failed to count warnings: {err}"));
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(error_response(api_err)));
        }
    };

    let order_by = params.sort_by.map(map_warning_sort).or(Some(WarningOrderBy::CreatedAt));
    let desc = params.sort_dir.unwrap_or_default() == SortDirection::Desc;
    let mut resolved_page = page;
    let mut rows = match state
        .store
        .list_warnings(&filters, order_by, desc, per_page as i64, offset)
        .await
    {
        Ok(value) => value,
        Err(err) => {
            let api_err = ApiError::internal(format!("Failed to list warnings: {err}"));
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(error_response(api_err)));
        }
    };

    if rows.is_empty() && total > 0 && page > 1 {
        total = match state.store.count_warnings(&filters).await {
            Ok(value) => value.max(0) as u64,
            Err(err) => {
                let api_err = ApiError::internal(format!("Failed to count warnings: {err}"));
                return (StatusCode::INTERNAL_SERVER_ERROR, Json(error_response(api_err)));
            }
        };
        resolved_page = (total.div_ceil(per_page as u64).max(1) as u32).min(page);

        loop {
            let retry_offset = ((resolved_page - 1) as i64) * per_page as i64;
            rows = match state
                .store
                .list_warnings(&filters, order_by, desc, per_page as i64, retry_offset)
                .await
            {
                Ok(value) => value,
                Err(err) => {
                    let api_err = ApiError::internal(format!("Failed to list warnings: {err}"));
                    return (StatusCode::INTERNAL_SERVER_ERROR, Json(error_response(api_err)));
                }
            };
            if !rows.is_empty() || resolved_page == 1 {
                break;
            }
            resolved_page -= 1;
        }
    }

    (
        StatusCode::OK,
        Json(success_with_meta(
            PagedResponse {
                items: rows.into_iter().map(warning_row_to_response).collect(),
                total,
                page: resolved_page,
                per_page,
            },
            ResponseMeta::now(),
        )),
    )
}

async fn list_insights(
    State(state): State<Phase01State>,
    Query(params): Query<ListInsightsQuery>,
) -> (StatusCode, Json<ApiResponse<PagedResponse<InsightResponse>>>) {
    let page = params.page.unwrap_or(1).max(1);
    let per_page = params.per_page.unwrap_or(25).clamp(1, 100);
    let offset = ((page - 1) as i64) * per_page as i64;
    let filters = InsightListFilters {
        regions: split_csv_upper(params.regions.as_deref()),
        date_from: None,
        date_to: None,
        search: params.search.filter(|value| !value.trim().is_empty()),
        insight_types: params
            .insight_type
            .filter(|value| !value.trim().is_empty())
            .map(|value| vec![value])
            .unwrap_or_default(),
        bookmarked_by: None,
        exclude_internal: false,
    };

    let total = match state.store.count_insights(&filters).await {
        Ok(value) => value.max(0) as u64,
        Err(err) => {
            let api_err = ApiError::internal(format!("Failed to count insights: {err}"));
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(error_response(api_err)));
        }
    };

    let rows = match state.store.list_insights(&filters, per_page as i64, offset).await {
        Ok(value) => value,
        Err(err) => {
            let api_err = ApiError::internal(format!("Failed to list insights: {err}"));
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(error_response(api_err)));
        }
    };

    (
        StatusCode::OK,
        Json(success_with_meta(
            PagedResponse {
                items: rows.into_iter().map(insight_row_to_response).collect(),
                total,
                page,
                per_page,
            },
            ResponseMeta::now(),
        )),
    )
}

async fn get_company_detail(
    State(state): State<Phase01State>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<CompanyDetail>>) {
    let company_id = match routes::companies::validate_company_id(&id) {
        Ok(value) => value,
        Err(msg) => {
            let api_err = ApiError::validation("company_id", msg);
            return (StatusCode::BAD_REQUEST, Json(error_response(api_err)));
        }
    };

    let company = match state.store.get_company(company_id).await {
        Ok(Some(value)) => value,
        Ok(None) => {
            let api_err = ApiError::not_found("company", &id);
            return (StatusCode::NOT_FOUND, Json(error_response(api_err)));
        }
        Err(err) => {
            let api_err = ApiError::internal(format!("Failed to load company: {err}"));
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(error_response(api_err)));
        }
    };

    let sites = state.store.get_sites_for_company(company_id).await.unwrap_or_default();
    let certifications = state
        .store
        .get_certifications_for_company(company_id)
        .await
        .unwrap_or_default();
    let persons = state.store.list_persons_by_org(company_id).await.unwrap_or_default();

    (
        StatusCode::OK,
        Json(success_with_meta(
            company_row_to_detail(company, sites, certifications, persons),
            ResponseMeta::now(),
        )),
    )
}

async fn delete_all_warnings(
    State(state): State<Phase01State>,
    Extension(auth_ctx): Extension<ApiAuthContext>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let authorization = match authorize_delete_all_warnings(&headers, &auth_ctx) {
        Ok(value) => value,
        Err(err) => {
            return (
                StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::BAD_REQUEST),
                Json(error_response(err)),
            );
        }
    };

    let deleted_count = match state.store.delete_all_warnings().await {
        Ok(value) => value,
        Err(err) => {
            let api_err = ApiError::internal(format!("Failed to delete all warnings: {err}"));
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(error_response(api_err)));
        }
    };

    let audit_detail = delete_all_warnings_audit_payload(&auth_ctx, &authorization, deleted_count);
    if let Err(err) = state
        .store
        .record_audit_event(&auth_ctx.user_id, "warnings_deleted_all", &audit_detail)
        .await
    {
        let api_err = ApiError::internal(format!("Failed to persist audit event: {err}"));
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(error_response(api_err)));
    }

    (
        StatusCode::OK,
        Json(success_with_meta(
            Value::Object(serde_json::Map::from_iter([
                (
                    "deleted_count".to_string(),
                    Value::Number(deleted_count.into()),
                ),
                (
                    "message".to_string(),
                    Value::String("All warnings deleted".to_string()),
                ),
            ])),
            ResponseMeta::now(),
        )),
    )
}

fn split_csv_upper(value: Option<&str>) -> Vec<String> {
    split_csv(value, true)
}

fn split_csv_lower(value: Option<&str>) -> Vec<String> {
    split_csv(value, false)
}

fn split_csv(value: Option<&str>, uppercase: bool) -> Vec<String> {
    value
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(|item| {
            if uppercase {
                item.to_uppercase()
            } else {
                item.to_lowercase()
            }
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
        confidence: row.confidence.unwrap_or(0.0).clamp(0.0, 1.0),
        calibrated_probability: None,
        bayesian_interpretation: None,
        confidence_interval: None,
        evidence_quality_label: None,
        information_gain_bits: None,
        acknowledged: row.acknowledged,
        acknowledged_by: row.acknowledged_by,
        acknowledged_at: row.acknowledged_at,
        acknowledged_note: row.acknowledged_note,
        review_outcome: row.review_outcome,
        reviewed_by: None,
        reviewed_at: None,
        deleted_at: row.deleted_at,
        ts_utc: ts,
        created_at: row.created_at.unwrap_or(ts),
        updated_at: row.updated_at.unwrap_or(ts),
    }
}

fn insight_row_to_response(row: InsightRow) -> InsightResponse {
    let now = Utc::now();
    InsightResponse {
        id: row.id.to_string(),
        title: row.title,
        summary: row.summary,
        insight_type: row.insight_type.unwrap_or_else(|| "general".to_string()),
        region: row.region.unwrap_or_default(),
        confidence: row.confidence.unwrap_or(0.0).clamp(0.0, 1.0),
        evidence_urls: row.evidence_urls.unwrap_or_default(),
        entity_ids: row
            .entity_ids
            .unwrap_or_default()
            .into_iter()
            .map(|id| id.to_string())
            .collect(),
        tags: row.tags.unwrap_or_default(),
        information_gain_bits: None,
        information_gain_sparkline: vec![],
        diversity_score: None,
        diversity_label: None,
        causal_flag: None,
        created_at: row.created_at.unwrap_or(now),
        updated_at: row.updated_at.unwrap_or(now),
        bookmarked: None,
    }
}

fn company_row_to_detail(
    row: CompanyRow,
    sites: Vec<SiteRow>,
    certifications: Vec<CertificationRow>,
    persons: Vec<PersonRow>,
) -> CompanyDetail {
    let mut capabilities = BTreeSet::new();
    if let Some(tags) = row.industry_tags.as_ref() {
        for tag in tags {
            capabilities.insert(tag.clone());
        }
    }
    for site in &sites {
        if let Some(site_capabilities) = site.capabilities.as_ref() {
            for capability in site_capabilities {
                capabilities.insert(capability.clone());
            }
        }
    }

    let now = Utc::now();
    CompanyDetail {
        id: row.id.to_string(),
        name: row.name,
        legal_name: row.legal_name,
        region: row.region.unwrap_or_default(),
        country: row.country_code.unwrap_or_default(),
        city: sites.iter().find_map(|site| site.city.clone()),
        website: row.domain,
        entity_type: row.company_type.unwrap_or_else(|| "unknown".to_string()),
        is_competitor: row
            .metadata
            .as_ref()
            .and_then(|meta| meta.get("is_competitor"))
            .and_then(|value| value.as_bool())
            .unwrap_or(false),
        threat_score: row.threat_score,
        overlap_score: row.overlap_score,
        capabilities: capabilities.into_iter().collect(),
        certifications: certifications.into_iter().map(|value| value.standard).collect(),
        sites: sites
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
            .collect(),
        key_persons: persons
            .into_iter()
            .map(|person| CompanyKeyPerson {
                person_id: person.id.to_string(),
                name: person.name,
                role: person
                    .current_role
                    .or(person.role_family)
                    .unwrap_or_else(|| "Unknown".to_string()),
            })
            .collect(),
        recent_events: vec![CompanyEvent {
            event_type: "profile_loaded".to_string(),
            description: "Company detail requested".to_string(),
            date: row.updated_at.unwrap_or(now),
            source_url: None,
        }],
        community_badges: vec![],
        source_entropy: None,
        source_quality_label: None,
        created_at: row.created_at.unwrap_or(now),
        updated_at: row.updated_at.unwrap_or(now),
    }
}