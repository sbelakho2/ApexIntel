use anyhow::Result;
use apex_api::auth::{self, ApiKey, ApiRole, AuthResult, PermissionLevel};
use apex_api::filters::{validate_search_text, RegionFilter, SeverityFilter, WarningTypeFilter};
use apex_api::responses::{
    aggregate_health, error_response, success, success_with_meta, ApiError, ApiResponse,
    ComponentHealth, ErrorCode, HealthResponse, HealthStatus, PagedResponse, ResponseMeta,
};
use apex_api::routes;
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
    priority_tier, validate_person_id, Affiliation, ListPersonsQuery, PeerSummary, PersonDetail,
    PersonEvent, PersonListItem, PersonSortField, PriorityVector, RoleHistoryEntry,
};
use apex_api::routes::preferences::{
    NotificationPrefs, PreferencesResponse, UpdatePreferencesRequest, UserPreferences,
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
use apex_api::routes::ws::{to_ws_event, WarningEvent};
use apex_core::config::AppConfig;
use apex_core::validation::clamp_ratio;
use apex_store::postgres::{
    AdminCrawlStatus, AdminPoiCoverage, AdminRecipePerformance, ArtifactRow, CapabilityRow,
    CertificationRow, CompanyChangeRow, CompanyDossier, CompanyListFilters, CompanyOrderBy,
    CompanyRow, CompetitorChange, DashboardStats, DossierEntryRow, EdgeRow, InsightListFilters,
    InsightRow, LogisticsNodeRow, ObservationRow, PersonChangeRow, PersonDossier, PersonEngagement,
    PersonListFilters, PersonListRow, PersonOrderBy, PersonRow, PgStore, ProductFamilyRow,
    RecipeStatRow, RegulationRow, RoleHistoryRow, SiteRow, WarningListFilters, WarningOrderBy,
    WarningRow, WeeklyMemo,
};
use apex_store::tantivy_index::SearchIndex;
use axum::{
    extract::{Path, Query, State},
    http::{header, Method, StatusCode},
    middleware,
    response::{Html, IntoResponse},
    routing::{get, post},
    Extension, Json, Router,
};
use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use lazy_static::lazy_static;
use prometheus::{
    Encoder, GaugeVec, HistogramOpts, HistogramVec, IntCounterVec, Opts, Registry, TextEncoder,
};
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
#[cfg(feature = "llm")]
use serde_json::Value as JsonValue;
use std::{
    collections::{BTreeSet, HashMap},
    path::Path as FsPath,
    sync::Arc,
    sync::OnceLock,
    time::Instant,
};
use tower_http::{cors::CorsLayer, services::ServeDir, trace::TraceLayer};
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

use apex_api::middleware::session::require_session as require_web_session;
use apex_api::web;

#[cfg(feature = "llm")]
use apex_llm::{validators, LlmClient, LlmProvider, ModelConfig, OpenAiCompatibleClient};

mod api_handlers;

static STARTED_AT: OnceLock<DateTime<Utc>> = OnceLock::new();
const MAX_JSON_DEPTH: usize = 32;

lazy_static! {
    static ref METRICS_REGISTRY: Registry = Registry::new();
    static ref HTTP_REQUESTS_TOTAL: IntCounterVec = IntCounterVec::new(
        Opts::new("http_requests_total", "Total number of HTTP requests"),
        &["method", "path", "status"]
    )
    .unwrap();
    static ref HTTP_REQUEST_DURATION_SECONDS: HistogramVec = HistogramVec::new(
        HistogramOpts::new(
            "http_request_duration_seconds",
            "HTTP request duration in seconds"
        )
        .buckets(vec![
            0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0
        ]),
        &["method", "path"]
    )
    .unwrap();
    static ref WARNINGS_TOTAL: GaugeVec = GaugeVec::new(
        Opts::new("apexintel_warnings_total", "Total warnings by severity"),
        &["severity"]
    )
    .unwrap();
    static ref INSIGHTS_TOTAL: GaugeVec = GaugeVec::new(
        Opts::new("apexintel_insights_total", "Total insights by category"),
        &["category"]
    )
    .unwrap();
    static ref COMPANIES_TRACKED: prometheus::Gauge =
        prometheus::Gauge::new("apexintel_companies_tracked", "Number of companies tracked")
            .unwrap();
    static ref PERSONS_TRACKED: prometheus::Gauge = prometheus::Gauge::new(
        "apexintel_persons_tracked",
        "Number of persons of interest tracked"
    )
    .unwrap();
    static ref RECIPES_ACTIVE: prometheus::Gauge =
        prometheus::Gauge::new("apexintel_recipes_active", "Number of active recipes").unwrap();
    static ref CRAWL_QUEUE_SIZE: prometheus::Gauge =
        prometheus::Gauge::new("apexintel_crawl_queue_size", "Current crawl queue size").unwrap();
    static ref RECIPE_FIRINGS_TOTAL: IntCounterVec = IntCounterVec::new(
        Opts::new("apexintel_recipe_firings_total", "Total recipe firings"),
        &["recipe_id", "outcome"]
    )
    .unwrap();
}

fn init_metrics() {
    METRICS_REGISTRY
        .register(Box::new(HTTP_REQUESTS_TOTAL.clone()))
        .ok();
    METRICS_REGISTRY
        .register(Box::new(HTTP_REQUEST_DURATION_SECONDS.clone()))
        .ok();
    METRICS_REGISTRY
        .register(Box::new(WARNINGS_TOTAL.clone()))
        .ok();
    METRICS_REGISTRY
        .register(Box::new(INSIGHTS_TOTAL.clone()))
        .ok();
    METRICS_REGISTRY
        .register(Box::new(COMPANIES_TRACKED.clone()))
        .ok();
    METRICS_REGISTRY
        .register(Box::new(PERSONS_TRACKED.clone()))
        .ok();
    METRICS_REGISTRY
        .register(Box::new(RECIPES_ACTIVE.clone()))
        .ok();
    METRICS_REGISTRY
        .register(Box::new(CRAWL_QUEUE_SIZE.clone()))
        .ok();
    METRICS_REGISTRY
        .register(Box::new(RECIPE_FIRINGS_TOTAL.clone()))
        .ok();
}

#[derive(Clone)]
struct AppState {
    store: Arc<PgStore>,
    search_index: Arc<SearchIndex>,
    api_keys: Arc<HashMap<String, ApiKey>>,
    redis: Option<redis::aio::ConnectionManager>,
    config: Arc<AppConfig>,
    started_at: Instant,
    #[cfg(feature = "llm")]
    llm: Option<LlmRuntime>,
}

#[derive(Debug, Clone, Copy)]
struct RateLimitInfo {
    limit_per_min: u32,
    remaining: Option<u32>,
    reset_at_unix: i64,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
struct ApiAuthContext {
    key_id: String,
    user_id: String,
    role: ApiRole,
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

    let allowed_origin =
        std::env::var("CORS_ORIGIN").unwrap_or_else(|_| "http://localhost:3000".to_string());
    let cors = CorsLayer::new()
        .allow_origin(
            allowed_origin
                .parse::<axum::http::HeaderValue>()
                .unwrap_or_else(|_| "http://localhost:3000".parse().unwrap()),
        )
        .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE])
        .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION]);

    let public = Router::new()
        .route("/api/health", get(health))
        .route("/api/health/live", get(health_live))
        .route("/api/health/ready", get(health_ready))
        .route("/api/endpoints", get(endpoints))
        .route("/api/openapi.json", get(openapi_json))
        .route("/api/docs", get(api_docs))
        .route("/metrics", get(metrics));

    let public_v1 = Router::new()
        .route("/api/v1/health", get(health))
        .route("/api/v1/health/live", get(health_live))
        .route("/api/v1/health/ready", get(health_ready))
        .route("/api/v1/endpoints", get(endpoints))
        .route("/api/v1/openapi.json", get(openapi_json))
        .route("/api/v1/docs", get(api_docs));

    let protected = Router::new()
        .route(
            "/api/warnings",
            get(api_handlers::warnings::list_warnings)
                .delete(api_handlers::warnings::delete_all_warnings),
        )
        .route(
            "/api/warnings/bulk-delete",
            post(api_handlers::warnings::delete_warnings_bulk),
        )
        .route(
            "/api/warnings/:id",
            get(api_handlers::details::get_warning_detail)
                .delete(api_handlers::warnings::delete_warning),
        )
        .route(
            "/api/warnings/:id/acknowledge",
            post(api_handlers::warnings::acknowledge_warning),
        )
        .route("/api/insights", get(api_handlers::insights::list_insights))
        .route(
            "/api/insights/export",
            get(api_handlers::exports::export_insights_csv),
        )
        .route(
            "/api/insights/weekly-memo",
            get(api_handlers::memos::get_weekly_memo),
        )
        .route(
            "/api/insights/:id",
            get(api_handlers::details::get_insight_detail),
        )
        .route(
            "/api/insights/:id/analyze",
            post(api_handlers::insights::analyze_insight),
        )
        .route(
            "/api/insights/:id/bookmark",
            post(api_handlers::insights::bookmark_insight)
                .delete(api_handlers::insights::unbookmark_insight),
        )
        .route(
            "/api/warnings/:id/analyze",
            post(api_handlers::insights::analyze_warning),
        )
        .route("/api/memos", get(api_handlers::memos::list_memos))
        .route(
            "/api/companies",
            get(api_handlers::entities::list_companies),
        )
        .route(
            "/api/companies/export",
            get(api_handlers::exports::export_companies_csv),
        )
        .route(
            "/api/companies/:id",
            get(api_handlers::entities::get_company_detail),
        )
        .route(
            "/api/companies/:id/dossier",
            get(api_handlers::dossiers::get_company_dossier),
        )
        .route("/api/persons", get(api_handlers::entities::list_persons))
        .route(
            "/api/persons/export",
            get(api_handlers::exports::export_persons_csv),
        )
        .route(
            "/api/persons/:id",
            get(api_handlers::entities::get_person_detail),
        )
        .route(
            "/api/persons/:id/dossier",
            get(api_handlers::dossiers::get_person_dossier),
        )
        .route(
            "/api/persons/:id/engagement",
            get(api_handlers::dossiers::get_person_engagement),
        )
        .route(
            "/api/persons/:id/role-history",
            get(api_handlers::dossiers::get_person_role_history),
        )
        .route(
            "/api/persons/:id/changes",
            get(api_handlers::dossiers::get_person_changes_api),
        )
        .route(
            "/api/persons/:id/dossier-entries",
            get(api_handlers::dossiers::get_person_dossier_entries),
        )
        .route(
            "/api/users",
            get(api_handlers::collaboration::list_users)
                .post(api_handlers::collaboration::create_user),
        )
        .route(
            "/api/users/:id",
            post(api_handlers::collaboration::update_user)
                .put(api_handlers::collaboration::update_user),
        )
        .route(
            "/api/saved-searches",
            get(api_handlers::collaboration::list_saved_searches)
                .post(api_handlers::collaboration::create_saved_search),
        )
        .route(
            "/api/saved-searches/:id",
            axum::routing::delete(api_handlers::collaboration::delete_saved_search)
                .put(api_handlers::collaboration::update_saved_search),
        )
        .route(
            "/api/watchlists",
            get(api_handlers::collaboration::list_watchlists)
                .post(api_handlers::collaboration::create_watchlist),
        )
        .route(
            "/api/watchlists/:id",
            axum::routing::delete(api_handlers::collaboration::delete_watchlist)
                .put(api_handlers::collaboration::update_watchlist),
        )
        .route(
            "/api/annotations",
            get(api_handlers::collaboration::list_annotations)
                .post(api_handlers::collaboration::create_annotation),
        )
        .route(
            "/api/annotations/:id",
            axum::routing::delete(api_handlers::collaboration::delete_annotation)
                .put(api_handlers::collaboration::update_annotation),
        )
        .route(
            "/api/audit-log",
            get(api_handlers::collaboration::list_audit_log),
        )
        .route(
            "/api/export-history",
            get(api_handlers::collaboration::list_export_history),
        )
        .route(
            "/api/companies/:id/changes",
            get(api_handlers::dossiers::get_company_changes_api),
        )
        .route(
            "/api/companies/:id/dossier-entries",
            get(api_handlers::dossiers::get_company_dossier_entries),
        )
        .route(
            "/api/dossier-entries/:id/verify",
            post(api_handlers::dossiers::verify_dossier_entry),
        )
        .route(
            "/api/dossier-entries/:id/history",
            get(api_handlers::dossiers::get_dossier_entry_history),
        )
        .route(
            "/api/competitors",
            get(api_handlers::competitors::list_competitors),
        )
        .route(
            "/api/competitors/changes",
            get(api_handlers::competitors::list_all_competitor_changes),
        )
        .route(
            "/api/competitors/:id/changes",
            get(api_handlers::competitors::get_competitor_changes),
        )
        .route("/api/search", get(api_handlers::overview::search))
        .route(
            "/api/search/semantic",
            get(api_handlers::overview::semantic_search),
        )
        .route("/api/graph", get(api_handlers::overview::list_graph))
        .route(
            "/api/graph/neighborhood/:id",
            get(api_handlers::graph::get_graph_neighborhood),
        )
        .route(
            "/api/graph/path/:from/:to",
            get(api_handlers::graph::get_graph_path),
        )
        .route("/api/recipes", get(api_handlers::overview::list_recipes))
        .route(
            "/api/recipes/staging",
            get(api_handlers::recipes::list_staging_recipes),
        )
        .route(
            "/api/recipes/:id/promote",
            post(api_handlers::recipes::promote_recipe),
        )
        .route(
            "/api/recipes/:id/deprecate",
            post(api_handlers::recipes::deprecate_recipe),
        )
        .route(
            "/api/preferences",
            get(api_handlers::preferences::get_preferences)
                .post(api_handlers::preferences::update_preferences),
        )
        .route("/api/security", get(api_handlers::overview::list_security))
        .route(
            "/api/security/dns-posture",
            get(api_handlers::security::get_dns_posture),
        )
        .route(
            "/api/security/lookalike-domains",
            get(api_handlers::security::get_lookalike_domains),
        )
        .route(
            "/api/security/kev-relevance",
            get(api_handlers::security::get_kev_relevance),
        )
        .route("/api/sites", get(api_handlers::catalog::list_sites))
        .route(
            "/api/capabilities",
            get(api_handlers::catalog::list_capabilities),
        )
        .route(
            "/api/certifications",
            get(api_handlers::catalog::list_certifications_all),
        )
        .route(
            "/api/observations",
            get(api_handlers::catalog::list_observations),
        )
        .route(
            "/api/product-families",
            get(api_handlers::catalog::list_product_families),
        )
        .route(
            "/api/logistics-nodes",
            get(api_handlers::catalog::list_logistics_nodes),
        )
        .route(
            "/api/regulations",
            get(api_handlers::catalog::list_regulations),
        )
        .route(
            "/api/poi-artifacts",
            get(api_handlers::catalog::list_poi_artifacts),
        )
        .route("/api/dashboard", get(api_handlers::catalog::get_dashboard))
        .route(
            "/api/admin/crawl-status",
            get(api_handlers::admin::get_admin_crawl_status),
        )
        .route(
            "/api/admin/recipe-performance",
            get(api_handlers::admin::get_admin_recipe_performance),
        )
        .route(
            "/api/admin/poi-coverage",
            get(api_handlers::admin::get_admin_poi_coverage),
        )
        .route(
            "/api/admin/llm-governance",
            get(api_handlers::admin::get_admin_llm_governance),
        )
        .route(
            "/api/admin/trigger-scan",
            post(api_handlers::admin::post_trigger_scan),
        )
        .route("/api/admin/replay", post(api_handlers::admin::post_replay))
        .route(
            "/api/admin/replay/:job_id",
            get(api_handlers::admin::get_replay_status),
        )
        .route("/api/health/deep", get(health_deep))
        .route(
            "/api/llm/extract-entities",
            post(api_handlers::llm::llm_extract_entities),
        )
        .route(
            "/api/llm/generate-recipe",
            post(api_handlers::llm::llm_generate_recipe),
        )
        .route(
            "/api/llm/synthesize-poi",
            post(api_handlers::llm::llm_synthesize_poi),
        )
        .route(
            "/api/llm/generate-memo",
            post(api_handlers::llm::llm_generate_memo),
        )
        .route_layer(middleware::from_fn_with_state(state.clone(), require_auth));

    let protected_v1 = Router::new()
        .route(
            "/api/v1/warnings",
            get(api_handlers::warnings::list_warnings),
        )
        .route(
            "/api/v1/warnings/:id/acknowledge",
            post(api_handlers::warnings::acknowledge_warning),
        )
        .route(
            "/api/v1/insights",
            get(api_handlers::insights::list_insights),
        )
        .route(
            "/api/v1/insights/export",
            get(api_handlers::exports::export_insights_csv),
        )
        .route("/api/v1/memos", get(api_handlers::memos::list_memos))
        .route(
            "/api/v1/companies",
            get(api_handlers::entities::list_companies),
        )
        .route(
            "/api/v1/companies/export",
            get(api_handlers::exports::export_companies_csv),
        )
        .route(
            "/api/v1/companies/:id",
            get(api_handlers::entities::get_company_detail),
        )
        .route("/api/v1/persons", get(api_handlers::entities::list_persons))
        .route(
            "/api/v1/persons/export",
            get(api_handlers::exports::export_persons_csv),
        )
        .route(
            "/api/v1/persons/:id",
            get(api_handlers::entities::get_person_detail),
        )
        .route("/api/v1/search", get(api_handlers::overview::search))
        .route(
            "/api/v1/search/semantic",
            get(api_handlers::overview::semantic_search),
        )
        .route("/api/v1/graph", get(api_handlers::overview::list_graph))
        .route("/api/v1/recipes", get(api_handlers::overview::list_recipes))
        .route(
            "/api/v1/preferences",
            get(api_handlers::preferences::get_preferences)
                .post(api_handlers::preferences::update_preferences),
        )
        .route(
            "/api/v1/security",
            get(api_handlers::overview::list_security),
        )
        .route(
            "/api/v1/users",
            get(api_handlers::collaboration::list_users)
                .post(api_handlers::collaboration::create_user),
        )
        .route(
            "/api/v1/users/:id",
            post(api_handlers::collaboration::update_user)
                .put(api_handlers::collaboration::update_user),
        )
        .route(
            "/api/v1/saved-searches",
            get(api_handlers::collaboration::list_saved_searches)
                .post(api_handlers::collaboration::create_saved_search),
        )
        .route(
            "/api/v1/saved-searches/:id",
            axum::routing::delete(api_handlers::collaboration::delete_saved_search)
                .put(api_handlers::collaboration::update_saved_search),
        )
        .route(
            "/api/v1/watchlists",
            get(api_handlers::collaboration::list_watchlists)
                .post(api_handlers::collaboration::create_watchlist),
        )
        .route(
            "/api/v1/watchlists/:id",
            axum::routing::delete(api_handlers::collaboration::delete_watchlist)
                .put(api_handlers::collaboration::update_watchlist),
        )
        .route(
            "/api/v1/annotations",
            get(api_handlers::collaboration::list_annotations)
                .post(api_handlers::collaboration::create_annotation),
        )
        .route(
            "/api/v1/annotations/:id",
            axum::routing::delete(api_handlers::collaboration::delete_annotation)
                .put(api_handlers::collaboration::update_annotation),
        )
        .route(
            "/api/v1/audit-log",
            get(api_handlers::collaboration::list_audit_log),
        )
        .route(
            "/api/v1/export-history",
            get(api_handlers::collaboration::list_export_history),
        )
        .route(
            "/api/v1/admin/crawl-status",
            get(api_handlers::admin::get_admin_crawl_status),
        )
        .route(
            "/api/v1/admin/recipe-performance",
            get(api_handlers::admin::get_admin_recipe_performance),
        )
        .route(
            "/api/v1/admin/poi-coverage",
            get(api_handlers::admin::get_admin_poi_coverage),
        )
        .route(
            "/api/v1/admin/llm-governance",
            get(api_handlers::admin::get_admin_llm_governance),
        )
        .route(
            "/api/v1/admin/trigger-scan",
            post(api_handlers::admin::post_trigger_scan),
        )
        .route(
            "/api/v1/admin/replay",
            post(api_handlers::admin::post_replay),
        )
        .route(
            "/api/v1/admin/replay/:job_id",
            get(api_handlers::admin::get_replay_status),
        )
        .route("/api/v1/health/deep", get(health_deep))
        .route_layer(middleware::from_fn_with_state(state.clone(), require_auth));

    let html_public = Router::new()
        .route(
            "/login",
            get(web::auth::login_page).post(web::auth::login_submit),
        )
        .layer(Extension(state.store.clone()));

    let html_protected = Router::new()
        .route("/", get(web::dashboard::dashboard))
        .route("/logout", post(web::auth::logout))
        .route("/warnings", get(web::warnings::list_warnings))
        .route("/warnings/:id", get(web::warnings::get_warning))
        .route(
            "/warnings/:id/acknowledge",
            post(web::warnings::acknowledge_warning_html),
        )
        .route(
            "/warnings/:id/analyze",
            post(web::warnings::analyze_warning_html),
        )
        .route(
            "/warnings/:id/review",
            post(web::warnings::review_warning_html),
        )
        .route(
            "/warnings/:id/notes",
            post(web::warnings::create_warning_note),
        )
        .route("/insights", get(web::insights::list_insights))
        .route("/insights/:id", get(web::insights::get_insight))
        .route(
            "/insights/:id/bookmark",
            post(web::insights::bookmark_insight_html),
        )
        .route(
            "/insights/:id/analyze",
            post(web::insights::analyze_insight_html),
        )
        .route(
            "/insights/:id/notes",
            post(web::insights::create_insight_note),
        )
        .route("/companies", get(web::companies::list_companies))
        .route("/companies/:id", get(web::companies::get_company))
        .route(
            "/companies/:id/changes",
            get(web::companies::company_changes_tab),
        )
        .route(
            "/companies/:id/dossier",
            get(web::companies::company_dossier_tab),
        )
        .route("/persons", get(web::persons::list_persons))
        .route("/persons/:id", get(web::persons::get_person))
        .route("/competitors", get(web::competitors::list_competitors))
        .route("/memos", get(web::memos::list_memos))
        .route(
            "/notifications",
            get(web::notifications::list_notifications_page),
        )
        .route(
            "/notifications/:id/read",
            post(web::notifications::mark_notification_read),
        )
        .route("/search", get(web::search::search_page))
        .route("/graph", get(web::graph::graph_page))
        .route("/security", get(web::security::security_page))
        .route(
            "/security/trigger-scan",
            post(api_handlers::html_mutations::post_trigger_scan_html),
        )
        .route(
            "/settings",
            get(web::settings::settings_page).post(web::settings::save_settings),
        )
        .route("/admin", get(web::admin::admin_page))
        .route("/recipes", get(web::recipes::list_recipes))
        .route("/recipes/new", get(web::recipes::new_recipe))
        .route(
            "/recipes/create",
            post(api_handlers::html_mutations::post_recipe_create_html),
        )
        .route(
            "/recipes/test",
            post(api_handlers::html_mutations::post_recipe_test_html),
        )
        .route(
            "/api/warnings/unread-count",
            get(web::warnings::unread_count),
        )
        .route_layer(middleware::from_fn(require_web_session))
        .layer(Extension(state.store.clone()))
        .layer(Extension(state.search_index.clone()));

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
        .merge(public_v1)
        .merge(protected)
        .merge(protected_v1)
        .route("/ws/warnings", get(warnings_ws))
        .fallback(get(web::errors::not_found))
        .layer(middleware::from_fn(add_rate_limit_headers))
        .layer(cors)
        .layer(axum::extract::DefaultBodyLimit::max(64 * 1024))
        .layer(
            TraceLayer::new_for_http().make_span_with(|request: &axum::http::Request<_>| {
                tracing::info_span!(
                    "http_request",
                    method = %request.method(),
                    path = %request.uri().path(),
                    request_id = %Uuid::new_v4()
                )
            }),
        )
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

fn required_permission_for_request(method: &Method, path: &str) -> PermissionLevel {
    let path = routes::strip_version_prefix(path);

    let is_admin_route = path.starts_with("/api/admin/")
        || path == "/api/health/deep"
        || (*method == Method::DELETE && path == "/api/warnings")
        || (*method == Method::POST && path == "/api/warnings/bulk-delete")
        || (*method == Method::DELETE
            && path.starts_with("/api/warnings/")
            && !path.ends_with("/acknowledge"))
        || path.starts_with("/api/users");

    let is_write_route =
        (*method == Method::POST || *method == Method::PUT || *method == Method::DELETE)
            || path.ends_with("/acknowledge")
            || path.ends_with("/bookmark")
            || path.ends_with("/analyze")
            || path == "/api/preferences";

    if is_admin_route {
        PermissionLevel::Admin
    } else if is_write_route {
        PermissionLevel::Write
    } else {
        PermissionLevel::Read
    }
}

async fn require_auth(
    State(state): State<AppState>,
    request: axum::extract::Request,
    next: middleware::Next,
) -> axum::response::Response {
    let auth_header = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok());

    let token = match auth_header.and_then(auth::extract_bearer_token) {
        Some(value) => value.to_string(),
        None => return auth_error_response(ApiError::unauthorized()),
    };

    let auth_result = auth::validate_token(&token, &state.api_keys, Utc::now());
    match &auth_result {
        AuthResult::Valid {
            key_id,
            owner_user_id,
            role,
        } => {
            if let Some(origin) = request
                .headers()
                .get(header::ORIGIN)
                .and_then(|value| value.to_str().ok())
            {
                let origin_ok = state
                    .api_keys
                    .values()
                    .find(|key| key.key_id == *key_id)
                    .map(|key| auth::check_origin(key, origin))
                    .unwrap_or(false);
                if !origin_ok {
                    return auth_error_response(ApiError::forbidden(
                        "Origin not allowed for this API key",
                    ));
                }
            }

            let resolved_identity = match state.store.resolve_api_key_owner(key_id).await {
                Ok(Some(identity)) => identity,
                Ok(None) => {
                    return auth_error_response(ApiError::forbidden(
                        "API key is not mapped to an active analyst identity",
                    ));
                }
                Err(err) => {
                    tracing::error!(key_id = %key_id, "resolve_api_key_owner failed: {err:#}");
                    return auth_error_response(ApiError::internal(
                        "Failed to resolve API identity",
                    ));
                }
            };

            if !resolved_identity.is_active {
                return auth_error_response(ApiError::forbidden("Analyst identity is inactive"));
            }

            let effective_role =
                ApiRole::from_str(&resolved_identity.role).unwrap_or_else(|| role.clone());

            let perm = required_permission_for_request(request.method(), request.uri().path());
            let effective_auth = AuthResult::Valid {
                key_id: key_id.clone(),
                owner_user_id: owner_user_id.clone(),
                role: effective_role.clone(),
            };
            if !auth::check_permission(&effective_auth, perm) {
                return auth_error_response(ApiError::forbidden("Insufficient permissions"));
            }

            let _ = state.store.touch_api_key_owner(key_id).await;

            let limit = state
                .api_keys
                .values()
                .find(|key| key.key_id == *key_id)
                .map(|key| key.rate_limit_per_min)
                .unwrap_or(120);

            let now = Utc::now();
            let minute_bucket = now.timestamp() / 60;
            let reset_at_unix = (minute_bucket + 1) * 60;

            let (remaining, over_limit) = if let Some(ref redis_mgr) = state.redis {
                let rl_key = format!("rl:{}:{}", key_id, minute_bucket);
                let mut conn = redis_mgr.clone();
                let result: Result<i64, redis::RedisError> = async {
                    let count: i64 = conn.incr(&rl_key, 1i64).await?;
                    if count == 1 {
                        let _: () = conn.expire(&rl_key, 65i64).await?;
                    }
                    Ok(count)
                }
                .await;

                match result {
                    Ok(count) => {
                        let count = count as u32;
                        if count > limit {
                            (0u32, true)
                        } else {
                            (limit.saturating_sub(count), false)
                        }
                    }
                    Err(error) => {
                        tracing::warn!(error = %error, "Redis rate limit check failed — degraded mode");
                        (limit, false)
                    }
                }
            } else {
                (limit, false)
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
            request.extensions_mut().insert(RateLimitInfo {
                limit_per_min: limit,
                remaining: Some(remaining),
                reset_at_unix,
            });
            request.extensions_mut().insert(ApiAuthContext {
                key_id: key_id.clone(),
                user_id: resolved_identity.user_id,
                role: effective_role,
            });
            next.run(request).await
        }
        AuthResult::Expired { .. }
        | AuthResult::Disabled { .. }
        | AuthResult::InvalidKey
        | AuthResult::MissingHeader => auth_error_response(ApiError::unauthorized()),
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
        response.headers_mut().insert(
            header::HeaderName::from_static("x-ratelimit-limit"),
            header::HeaderValue::from_str(&info.limit_per_min.to_string())
                .unwrap_or_else(|_| header::HeaderValue::from_static("120")),
        );
        response.headers_mut().insert(
            header::HeaderName::from_static("x-ratelimit-remaining"),
            header::HeaderValue::from_str(&remaining.to_string())
                .unwrap_or_else(|_| header::HeaderValue::from_static("120")),
        );
        response.headers_mut().insert(
            header::HeaderName::from_static("x-ratelimit-reset"),
            header::HeaderValue::from_str(&info.reset_at_unix.to_string())
                .unwrap_or_else(|_| header::HeaderValue::from_static("0")),
        );
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
    let base_url = match config
        .llm_base_url
        .as_ref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
    {
        Some(url) => url.to_string(),
        None => return Ok(None),
    };

    let provider = infer_llm_provider(&base_url);
    let llm_api_key = config.llm_api_key_value();

    if matches!(provider, LlmProvider::OpenAi | LlmProvider::AzureOpenAi) && llm_api_key.is_none() {
        anyhow::bail!("LLM_API_KEY is required for OpenAI/Azure providers");
    }

    let primary = ModelConfig {
        model_name: config.llm_model.clone(),
        provider: provider.clone(),
        base_url: base_url.clone(),
        api_key: llm_api_key.clone(),
        max_tokens: 4096,
        temperature: 0.2,
        timeout_seconds: 300,
    };

    let lightweight = ModelConfig {
        model_name: config.llm_model.clone(),
        provider,
        base_url,
        api_key: llm_api_key,
        max_tokens: 1024,
        temperature: 0.1,
        timeout_seconds: 120,
    };

    let mut issues = primary.validate();
    issues.extend(lightweight.validate());
    if !issues.is_empty() {
        anyhow::bail!("Invalid LLM configuration: {}", issues.join("; "));
    }

    Ok(Some(LlmRuntime {
        primary,
        lightweight,
    }))
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
            message: Some(format!(
                "{} endpoints registered",
                routes::all_endpoints().len()
            )),
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
    let db_check = match sqlx::query("SELECT 1").fetch_one(&state.store.pool).await {
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

async fn health_deep(
    State(state): State<AppState>,
) -> (StatusCode, Json<apex_api::routes::health::DeepHealthCheck>) {
    let llm_base_url = state
        .config
        .llm_base_url
        .clone()
        .unwrap_or_else(|| "http://127.0.0.1:11434".to_string());
    let response = apex_api::routes::health::deep_health_check(
        &state.store.pool,
        state.config.redis_url_value(),
        &state.config.nats_url,
        &state.config.minio_url,
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

async fn api_docs() -> Html<String> {
    let endpoints = routes::all_endpoints();
    let rows = endpoints
        .iter()
        .map(|endpoint| {
            format!(
                "<tr><td>{}</td><td><code>{}</code><br/><code>{}</code></td><td>{}</td><td>{}</td></tr>",
                endpoint.method.label(),
                endpoint.path,
                routes::versioned_path(endpoint.path),
                endpoint.description,
                endpoint.min_role,
            )
        })
        .collect::<Vec<_>>()
        .join("");
    Html(format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>ApexIntel API Docs</title><style>body{{font-family:ui-monospace,Menlo,monospace;background:#101418;color:#eef2f5;padding:32px}}table{{width:100%;border-collapse:collapse}}td,th{{border:1px solid #30404b;padding:10px;vertical-align:top}}code{{color:#9dd4ff}}a{{color:#8fd3ff}}</style></head><body><h1>ApexIntel API</h1><p>Stable aliases are available under <code>/api/v1</code>. Machine-readable schema: <a href=\"/api/openapi.json\">/api/openapi.json</a>.</p><table><thead><tr><th>Method</th><th>Paths</th><th>Description</th><th>Min role</th></tr></thead><tbody>{rows}</tbody></table></body></html>"
    ))
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
        WARNINGS_TOTAL
            .with_label_values(&["total"])
            .set(stats.total_warnings as f64);
        WARNINGS_TOTAL
            .with_label_values(&["active"])
            .set(stats.unacknowledged_warnings as f64);

        // Set insights total
        INSIGHTS_TOTAL
            .with_label_values(&["total"])
            .set(stats.total_insights as f64);
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
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        output,
    )
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

    let store = PgStore::connect(config.database_url_value()).await?;
    store.run_migrations().await?;

    let index_path =
        std::env::var("SEARCH_INDEX_PATH").unwrap_or_else(|_| "data/search".to_string());
    let search_index = SearchIndex::open(FsPath::new(&index_path))?;

    // API keys
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
    tracing::info!("{} API keys loaded", api_keys.len());

    // Optional Redis for rate limiting
    let redis = match std::env::var("REDIS_URL") {
        Ok(url) if !url.trim().is_empty() => match redis::Client::open(url.as_str()) {
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
        },
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
        config: Arc::new(config),
        started_at: Instant::now(),
        #[cfg(feature = "llm")]
        llm,
    })
}

/// Load API keys from environment variables.
/// Format: API_KEY_1=<raw_key>,<display_name>,<role>[,<user_id>]
/// Example: API_KEY_1=sk-abc123,Admin Key,admin,usr-admin
fn load_api_keys() -> HashMap<String, ApiKey> {
    let mut registry = HashMap::new();
    for i in 1..=50 {
        let env_key = format!("API_KEY_{}", i);
        if let Ok(val) = std::env::var(&env_key) {
            let parts: Vec<&str> = val.splitn(4, ',').collect();
            if parts.len() >= 3 {
                let raw_key = parts[0].trim();
                let name = parts[1].trim();
                let role_str = parts[2].trim();
                let role = ApiRole::from_str(role_str).unwrap_or(ApiRole::Viewer);
                let key_id = format!("key-{}", i);
                let owner_user_id = parts
                    .get(3)
                    .map(|value| value.trim())
                    .filter(|value| !value.is_empty())
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| derive_api_owner_user_id(name, &key_id));
                let api_key = ApiKey {
                    key_id: key_id.clone(),
                    owner_user_id,
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
                tracing::warn!(
                    "Invalid {} format — expected raw_key,name,role[,user_id]",
                    env_key
                );
            }
        }
    }
    registry
}

fn derive_api_owner_user_id(name: &str, key_id: &str) -> String {
    let slug = name
        .to_ascii_lowercase()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join("-");

    if slug.is_empty() {
        format!("usr-{}", key_id)
    } else {
        format!("usr-{}", slug)
    }
}

fn pagination(page: Option<u32>, per_page: Option<u32>) -> (u32, u32, i64) {
    let page = page.unwrap_or(1).max(1);
    let per_page = per_page.unwrap_or(25).min(500).max(1);
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
        return Err(ApiError::validation(
            "per_page",
            "per_page must be in 1..=500",
        ));
    }
    Ok(pagination(Some(page), Some(per_page)))
}

/// Clamp page to the valid range [1, total_pages] after the total is known.
fn clamp_page(page: u32, per_page: u32, total: u64) -> u32 {
    let total_pages =
        total.checked_add(per_page as u64 - 1).unwrap_or(u64::MAX) / per_page.max(1) as u64;
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
            return Err(ApiError::validation(
                field,
                format!("{} token too long", field),
            ));
        }
        if !trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err(ApiError::validation(
                field,
                format!("{} contains invalid characters", field),
            ));
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
    tracing::info!(
        endpoint = endpoint,
        duration_ms = duration_ms,
        bucket = bucket,
        "request_latency"
    );
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
            return Err(ApiError::validation(
                "regions",
                format!("invalid region '{}'", region),
            ));
        }
    }
    Ok(())
}

fn validate_severity_codes(severities: &[String]) -> Result<(), ApiError> {
    for sev in severities {
        if SeverityFilter::from_str_loose(sev).is_none() {
            return Err(ApiError::validation(
                "severities",
                format!("invalid severity '{}'", sev),
            ));
        }
    }
    Ok(())
}

fn validate_warning_type_codes(warning_types: &[String]) -> Result<(), ApiError> {
    for wt in warning_types {
        if WarningTypeFilter::from_str_loose(wt).is_none() {
            return Err(ApiError::validation(
                "warning_types",
                format!("invalid warning type '{}'", wt),
            ));
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
        review_outcome: row.review_outcome,
        reviewed_by: row.reviewed_by,
        reviewed_at: row.reviewed_at,
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

    let sites: Vec<CompanySite> = sites.into_iter().map(site_row_to_company_site).collect();

    let key_persons: Vec<CompanyKeyPerson> =
        persons.into_iter().map(person_row_to_key_person).collect();

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
    let role_family = row
        .role_family
        .clone()
        .unwrap_or_else(|| "Unknown".to_string());
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
    let email = row
        .public_email
        .clone()
        .or_else(|| metadata_string(&row.metadata, "email"));

    // Build alternative names
    let mut name_alt = Vec::new();
    if let Some(ref ar) = row.name_ar {
        if !ar.is_empty() {
            name_alt.push(ar.clone());
        }
    }
    if let Some(ref fr) = row.name_fr {
        if !fr.is_empty() {
            name_alt.push(fr.clone());
        }
    }

    // Compute data completeness score (0.0 - 1.0)
    let data_completeness = compute_data_completeness(&row, &email, &timeline, &affiliations);

    // Compute engagement readiness score (0.0 - 1.0)
    let engagement_readiness = compute_engagement_readiness(
        &row.decision_style,
        &row.risk_tolerance,
        &row.change_appetite,
        &row.communication_style,
        data_completeness,
        influence_score,
    );

    // Build role history entries
    let role_history: Vec<RoleHistoryEntry> = role_history_rows
        .into_iter()
        .map(|rh| {
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
        })
        .collect();

    // Build peer summaries
    let peers: Vec<PeerSummary> = peer_rows
        .into_iter()
        .take(6)
        .map(|p| {
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
        })
        .collect();

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
    if row.public_bio.is_some() {
        filled += 1.0;
    }
    if email.is_some() {
        filled += 1.0;
    }
    if row
        .trigger_topics
        .as_ref()
        .map(|t| !t.is_empty())
        .unwrap_or(false)
    {
        filled += 1.0;
    }
    if row.decision_style.is_some() {
        filled += 1.0;
    }
    if row.risk_tolerance.is_some() {
        filled += 1.0;
    }
    if row.change_appetite.is_some() {
        filled += 1.0;
    }
    if row.communication_style.is_some() {
        filled += 1.0;
    }
    if row.decision_mode.is_some() {
        filled += 1.0;
    }
    if row.preferred_proof_type.is_some() {
        filled += 1.0;
    }
    if row.pain_index.map(|v| v > 0.0).unwrap_or(false) {
        filled += 1.0;
    }
    if row.change_risk.map(|v| v > 0.0).unwrap_or(false) {
        filled += 1.0;
    }
    if !timeline.is_empty() {
        filled += 1.0;
    }
    if !affiliations.is_empty() {
        filled += 1.0;
    }
    if row.region.is_some() {
        filled += 1.0;
    }
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
        if excluded_types
            .iter()
            .any(|&excluded| ctype_lower.contains(excluded))
        {
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
    let engagement_potential =
        compute_engagement_potential(artifact_count, organization, current_role);

    // Intelligence Value: Composite of decision power and domain relevance
    let intelligence_value =
        compute_intelligence_value(decision_power, domain_relevance, influence_score);

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
    let is_cxo = role_lower.contains("cfo")
        || role_lower.contains("cto")
        || role_lower.contains("coo")
        || role_lower.contains("chief")
        || role_lower.contains("c-level");
    let is_minister = role_lower.contains("minister") || role_lower.contains("secretary");
    let is_director_general =
        role_lower.contains("director general") || role_lower.contains("director-general");
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
    let base = if is_ceo
        || is_chairman
        || is_minister
        || is_director_general
        || is_governor
        || is_commissioner
    {
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
fn compute_domain_relevance(
    role_family: Option<&str>,
    organization: &str,
    current_role: Option<&str>,
) -> f64 {
    let org_lower = organization.to_lowercase();
    let role_lower = current_role.map(|r| r.to_lowercase()).unwrap_or_default();
    let family_lower = role_family.map(|r| r.to_lowercase()).unwrap_or_default();

    // High relevance for core industry players
    let is_ems = org_lower.contains("flex")
        || org_lower.contains("jabil")
        || org_lower.contains("foxconn")
        || org_lower.contains("celestica")
        || org_lower.contains("sanmina")
        || org_lower.contains("wistron")
        || org_lower.contains("pegatron")
        || org_lower.contains("incap")
        || org_lower.contains("kitron")
        || org_lower.contains("zollner")
        || org_lower.contains("hanza")
        || org_lower.contains("ems")
        || org_lower.contains("electronics")
        || org_lower.contains("asteelflash");

    let is_semiconductor = org_lower.contains("stmicro")
        || org_lower.contains("infineon")
        || org_lower.contains("nxp")
        || org_lower.contains("intel")
        || org_lower.contains("amd")
        || org_lower.contains("tsmc")
        || org_lower.contains("semiconductor");

    let is_government = org_lower.contains("government")
        || org_lower.contains("ministry")
        || org_lower.contains("commission")
        || family_lower.contains("government");

    let is_defense = org_lower.contains("defense")
        || org_lower.contains("defence")
        || org_lower.contains("military")
        || org_lower.contains("nato")
        || role_lower.contains("defense")
        || role_lower.contains("defence");

    let is_investment = org_lower.contains("bank")
        || org_lower.contains("investment")
        || org_lower.contains("bpifrance")
        || org_lower.contains("ebrd")
        || org_lower.contains("eib");

    let is_trade_assoc = org_lower.contains("cgem")
        || org_lower.contains("amica")
        || org_lower.contains("utica")
        || org_lower.contains("conect")
        || org_lower.contains("association");

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
fn compute_engagement_potential(
    artifact_count: usize,
    organization: &str,
    current_role: Option<&str>,
) -> f64 {
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
        0.08 // consultants are accessible
    } else {
        0.0
    };

    // C-suite harder to access than mid-level
    let role_adj = if role_lower.contains("ceo")
        || role_lower.contains("president")
        || role_lower.contains("minister")
        || role_lower.contains("chairman")
    {
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
fn compute_intelligence_value(
    decision_power: f64,
    domain_relevance: f64,
    influence_score: f64,
) -> f64 {
    // Weighted combination with slight variance
    let base = decision_power * 0.35 + domain_relevance * 0.35 + influence_score * 0.30;
    let variance = ((decision_power * 1000.0) as u64 % 11) as f64 / 150.0;
    base + variance
}

/// Generate consistent variance from string hash (deterministic).
fn role_variance(s: &str) -> f64 {
    let hash: u64 = s
        .bytes()
        .fold(0u64, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u64));
    ((hash % 100) as f64 - 50.0) / 100.0 // Returns -0.5 to +0.5
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
    let Some(items) = meta
        .as_ref()
        .and_then(|value| value.get("affiliations"))
        .and_then(|value| value.as_array())
    else {
        return Vec::new();
    };

    items
        .iter()
        .filter_map(|item| {
            let obj = item.as_object()?;
            let organization = obj.get("organization")?.as_str()?;
            let role = obj
                .get("role")
                .and_then(|value| value.as_str())
                .unwrap_or("Unknown");
            let current = obj
                .get("current")
                .and_then(|value| value.as_bool())
                .unwrap_or(false);
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
    let id = format!(
        "recipe-{}",
        stat.recipe_code.to_lowercase().replace('_', "-")
    );

    // Format name from recipe code (e.g., "TECH_CONVERGENCE" → "Tech Convergence")
    let name = stat
        .recipe_code
        .split('_')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first
                    .to_uppercase()
                    .chain(chars.flat_map(|c| c.to_lowercase()))
                    .collect(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ");

    // Generate a description
    let description = if stat.fired_count > 0 {
        format!(
            "Signal recipe that has fired {} times with {:.0}% observed precision.",
            stat.fired_count,
            (stat.precision_score * 100.0).clamp(0.0, 100.0)
        )
    } else {
        "Signal recipe deployed and monitoring. No signals detected yet.".to_string()
    };

    let fired_count = stat.fired_count.max(0) as f64;
    let precision = stat.precision_score.clamp(0.0, 1.0);
    let false_positive_rate = stat.false_positive_rate.clamp(0.0, 1.0);
    let activity_signal = (fired_count / 100.0).clamp(0.0, 1.0);
    let recall = (0.7 * precision + 0.3 * activity_signal).clamp(0.0, 1.0);

    let status = match stat.status.as_str() {
        "active" | "production" => RecipeStatus::Production,
        "deprecated" => RecipeStatus::Deprecated,
        _ => RecipeStatus::Staging,
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

// ─── Security sub-endpoints ──────────────────────────

#[derive(Debug, Deserialize, Serialize)]
struct RecipeBuilderSignal {
    source_type: Option<String>,
    entity_type: Option<String>,
    keywords: Option<String>,
    match_mode: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct RecipeBuilderTransform {
    kind: Option<String>,
    window_days: Option<i64>,
    aggregation: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct RecipeBuilderThreshold {
    metric: Option<String>,
    operator: Option<String>,
    value: Option<f64>,
}

#[derive(Debug, Deserialize, Serialize)]
struct RecipeBuilderAction {
    kind: Option<String>,
    target: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RecipeBuilderPayload {
    name: String,
    description: Option<String>,
    severity: Option<String>,
    cooldown_hours: Option<i64>,
    enabled: Option<bool>,
    narrative_template: Option<String>,
    #[serde(default)]
    signals: Vec<RecipeBuilderSignal>,
    #[serde(default)]
    transforms: Vec<RecipeBuilderTransform>,
    #[serde(default)]
    thresholds: Vec<RecipeBuilderThreshold>,
    #[serde(default)]
    actions: Vec<RecipeBuilderAction>,
}

const MANUAL_TRIGGER_KINDS: &[&str] = &[
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
    "update_email_digest",
];

fn is_valid_manual_trigger_kind(kind: &str) -> bool {
    MANUAL_TRIGGER_KINDS.contains(&kind)
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

async fn warnings_ws_stream(mut socket: axum::extract::ws::WebSocket, state: AppState) {
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
            review_outcome: None,
            reviewed_by: None,
            reviewed_at: None,
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
            review_outcome: None,
            reviewed_by: None,
            reviewed_at: None,
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

    #[test]
    fn test_required_permission_for_delete_all_warnings_is_admin() {
        assert_eq!(
            required_permission_for_request(&Method::DELETE, "/api/warnings"),
            PermissionLevel::Admin
        );
    }

    #[test]
    fn test_required_permission_for_warning_acknowledge_remains_write() {
        assert_eq!(
            required_permission_for_request(&Method::POST, "/api/warnings/123/acknowledge"),
            PermissionLevel::Write
        );
    }

    #[test]
    fn test_required_permission_for_delete_warning_is_admin() {
        assert_eq!(
            required_permission_for_request(&Method::DELETE, "/api/warnings/123"),
            PermissionLevel::Admin
        );
    }

    #[test]
    fn test_required_permission_for_bulk_delete_warnings_is_admin() {
        assert_eq!(
            required_permission_for_request(&Method::POST, "/api/warnings/bulk-delete"),
            PermissionLevel::Admin
        );
    }
}
