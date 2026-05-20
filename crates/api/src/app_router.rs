use crate::*;

use apex_api::middleware::session::require_session;
use axum::{
    middleware,
    routing::{get, post, patch, delete},
    Extension, Router,
};
use tower_http::{cors::CorsLayer, trace::TraceLayer};

pub(crate) fn build_app_router(state: AppState, cors: CorsLayer) -> Router {
    let public = Router::new()
        .route("/api/health", get(health))
        .route("/api/health/live", get(health_live))
        .route("/api/health/ready", get(health_ready))
        .route("/api/health/deep", get(health_deep))
        .route("/api/endpoints", get(endpoints))
        .route("/api/openapi.json", get(openapi_json))
        .route("/api/docs", get(api_docs))
        .route("/api/features", get(api_features))
        .route("/metrics", get(runtime_metrics::metrics))
        .route(
            "/login",
            get(apex_api::web::auth::login_page).post(apex_api::web::auth::login_submit),
        )
        .route("/logout", post(apex_api::web::auth::logout));

    let protected = Router::new()
        // Existing endpoints...
        .route(
            "/api/warnings",
            get(warnings_handlers::list_warnings).delete(warnings_handlers::delete_all_warnings),
        )
        .route(
            "/api/warnings/bulk-delete",
            post(warnings_handlers::delete_warnings_bulk),
        )
        .route(
            "/api/warnings/:id",
            get(details_handlers::get_warning_detail).delete(warnings_handlers::delete_warning),
        )
        .route(
            "/api/warnings/:id/acknowledge",
            post(warnings_handlers::acknowledge_warning),
        )
        .route("/api/insights", get(insights_handlers::list_insights))
        .route(
            "/api/insights/export",
            get(exports_handlers::export_insights_csv),
        )
        .route(
            "/api/insights/weekly-memo",
            get(memos_handlers::get_weekly_memo),
        )
        .route(
            "/api/insights/:id",
            get(details_handlers::get_insight_detail),
        )
        .route(
            "/api/insights/:id/bookmark",
            post(insights_handlers::bookmark_insight).delete(insights_handlers::unbookmark_insight),
        )
        .route(
            "/api/insights/:id/feedback",
            post(insights_handlers::record_insight_feedback),
        )
        .route(
            "/api/insights/:id/analyze",
            post(insights_handlers::analyze_insight),
        )
        .route(
            "/api/warnings/:id/analyze",
            post(insights_handlers::analyze_warning),
        )
        .route("/api/memos", get(memos_handlers::list_memos))
        .route("/api/companies", get(entities_handlers::list_companies))
        .route(
            "/api/companies/export",
            get(exports_handlers::export_companies_csv),
        )
        .route(
            "/api/companies/:id",
            get(entities_handlers::get_company_detail),
        )
        .route(
            "/api/companies/:id/dossier",
            get(dossiers_handlers::get_company_dossier),
        )
        .route("/api/persons", get(entities_handlers::list_persons))
        .route(
            "/api/persons/export",
            get(exports_handlers::export_persons_csv),
        )
        .route(
            "/api/persons/:id",
            get(entities_handlers::get_person_detail),
        )
        .route(
            "/api/persons/:id/dossier",
            get(dossiers_handlers::get_person_dossier),
        )
        .route(
            "/api/persons/:id/engagement",
            get(dossiers_handlers::get_person_engagement),
        )
        .route(
            "/api/persons/:id/role-history",
            get(dossiers_handlers::get_person_role_history),
        )
        .route(
            "/api/persons/:id/changes",
            get(dossiers_handlers::get_person_changes_api),
        )
        .route(
            "/api/persons/:id/dossier-entries",
            get(dossiers_handlers::get_person_dossier_entries),
        )
        .route(
            "/api/companies/:id/changes",
            get(dossiers_handlers::get_company_changes_api),
        )
        .route(
            "/api/companies/:id/dossier-entries",
            get(dossiers_handlers::get_company_dossier_entries),
        )
        .route(
            "/api/dossier-entries/:id/verify",
            post(dossiers_handlers::verify_dossier_entry),
        )
        .route(
            "/api/dossier-entries/:id/history",
            get(dossiers_handlers::get_dossier_entry_history),
        )
        .route(
            "/api/competitors",
            get(competitors_handlers::list_competitors),
        )
        .route(
            "/api/competitors/changes",
            get(competitors_handlers::list_all_competitor_changes),
        )
        .route(
            "/api/competitors/:id/changes",
            get(competitors_handlers::get_competitor_changes),
        )
        .route("/api/search", get(overview_handlers::search))
        .route(
            "/api/search/semantic",
            get(overview_handlers::semantic_search),
        )
        .route("/api/graph", get(overview_handlers::list_graph))
        .route(
            "/api/graph/neighborhood/:id",
            get(graph_handlers::get_graph_neighborhood),
        )
        .route(
            "/api/graph/path/:from/:to",
            get(graph_handlers::get_graph_path),
        )
        .route("/api/recipes", get(overview_handlers::list_recipes))
        .route(
            "/api/recipes/staging",
            get(recipes_handlers::list_staging_recipes),
        )
        .route(
            "/api/recipes/:id/promote",
            post(recipes_handlers::promote_recipe),
        )
        .route(
            "/api/recipes/:id/deprecate",
            post(recipes_handlers::deprecate_recipe),
        )
        .route("/api/security", get(overview_handlers::list_security))
        .route(
            "/api/security/dns-posture",
            get(security_handlers::get_dns_posture),
        )
        .route(
            "/api/security/lookalike-domains",
            get(security_handlers::get_lookalike_domains),
        )
        .route(
            "/api/security/kev-relevance",
            get(security_handlers::get_kev_relevance),
        )
        .route("/api/sites", get(catalog_handlers::list_sites))
        .route(
            "/api/capabilities",
            get(catalog_handlers::list_capabilities),
        )
        .route(
            "/api/certifications",
            get(catalog_handlers::list_certifications_all),
        )
        .route(
            "/api/observations",
            get(catalog_handlers::list_observations),
        )
        .route(
            "/api/product-families",
            get(catalog_handlers::list_product_families),
        )
        .route(
            "/api/logistics-nodes",
            get(catalog_handlers::list_logistics_nodes),
        )
        .route("/api/regulations", get(catalog_handlers::list_regulations))
        .route(
            "/api/poi-artifacts",
            get(catalog_handlers::list_poi_artifacts),
        )
        .route("/api/dashboard", get(catalog_handlers::get_dashboard))
        .route("/api/admin/crawl-status", get(get_admin_crawl_status))
        .route(
            "/api/admin/recipe-performance",
            get(get_admin_recipe_performance),
        )
        .route("/api/admin/poi-coverage", get(get_admin_poi_coverage))
        .route("/api/admin/trigger-scan", post(post_trigger_scan))
        .route(
            "/api/llm/extract-entities",
            post(llm_handlers::llm_extract_entities),
        )
        .route(
            "/api/llm/generate-recipe",
            post(llm_handlers::llm_generate_recipe),
        )
        .route(
            "/api/llm/synthesize-poi",
            post(llm_handlers::llm_synthesize_poi),
        )
        .route(
            "/api/llm/generate-memo",
            post(llm_handlers::llm_generate_memo),
        )
        
        // ─── Phase 4.3: Executive Dashboard ─────────────────────────────
        .route(
            "/api/executive/summary",
            get(collaboration_handlers::get_executive_summary),
        )
        .route(
            "/api/executive/opportunities",
            get(collaboration_handlers::list_opportunities).post(collaboration_handlers::create_opportunity),
        )
        .route(
            "/api/executive/opportunities/:id",
            get(collaboration_handlers::get_opportunity).patch(collaboration_handlers::update_opportunity),
        )
        .route(
            "/api/executive/threats",
            get(collaboration_handlers::list_threats).post(collaboration_handlers::create_threat),
        )
        .route(
            "/api/executive/threats/:id",
            get(collaboration_handlers::get_threat).patch(collaboration_handlers::update_threat),
        )

        // ─── Phase 4.3: Investigation Workspaces ─────────────────────────
        .route(
            "/api/workspaces",
            get(collaboration_handlers::list_workspaces).post(collaboration_handlers::create_workspace),
        )
        .route(
            "/api/workspaces/:id",
            get(collaboration_handlers::get_workspace)
                .patch(collaboration_handlers::update_workspace)
                .delete(collaboration_handlers::delete_workspace),
        )
        .route(
            "/api/workspaces/:id/assignments",
            get(collaboration_handlers::list_workspace_assignments).post(collaboration_handlers::assign_user_to_workspace),
        )
        .route(
            "/api/workspaces/:id/assignments/:user_id",
            delete(collaboration_handlers::remove_user_from_workspace),
        )
        .route(
            "/api/workspaces/:id/shares",
            get(collaboration_handlers::list_workspace_shares).post(collaboration_handlers::share_workspace),
        )

        // ─── Phase 4.3: Activity Feed ─────────────────────────────────────
        .route(
            "/api/activity-feed",
            get(collaboration_handlers::get_activity_feed).post(collaboration_handlers::record_activity),
        )

        // ─── Phase 4.3: Daily Priority Queue ─────────────────────────────
        .route(
            "/api/queue",
            get(collaboration_handlers::list_queue_items).post(collaboration_handlers::add_to_queue),
        )
        .route(
            "/api/queue/:id",
            patch(collaboration_handlers::update_queue_item),
        )

        // ─── Phase 4.3: Supplier Risk ──────────────────────────────────────
        .route(
            "/api/supplier-risk",
            get(collaboration_handlers::list_supplier_risks).post(collaboration_handlers::add_supplier_risk),
        )
        .route(
            "/api/supplier-risk/:id",
            patch(collaboration_handlers::update_supplier_risk),
        )

        // ─── Phase 4.3: Pipeline Opportunities ────────────────────────────
        .route(
            "/api/pipeline",
            get(collaboration_handlers::list_pipeline_opportunities).post(collaboration_handlers::create_pipeline_opportunity),
        )
        .route(
            "/api/pipeline/:id/stage",
            patch(collaboration_handlers::update_pipeline_stage),
        )

        // ─── Phase 4.3: Source Evidence ───────────────────────────────────
        .route(
            "/api/evidence",
            get(collaboration_handlers::get_evidence).post(collaboration_handlers::add_evidence),
        )

        // ─── Phase 4.3: Team Assignments ──────────────────────────────────
        .route(
            "/api/team-assignments",
            get(collaboration_handlers::list_team_assignments).post(collaboration_handlers::create_team_assignment),
        )

        // ─── Phase 4.3: Annotations (TODO: implement collab_routes module) ───
        
        .route_layer(middleware::from_fn_with_state(state.clone(), require_auth));

    let web_pages = Router::new()
        .route("/", get(apex_api::web::dashboard::dashboard))
        .route("/warnings", get(apex_api::web::warnings::list_warnings))
        .route(
            "/warnings/unread-count",
            get(apex_api::web::warnings::unread_count),
        )
        .route("/warnings/:id", get(apex_api::web::warnings::get_warning))
        .route(
            "/warnings/:id/acknowledge",
            post(apex_api::web::warnings::acknowledge_warning_html),
        )
        .route(
            "/warnings/:id/analyze",
            post(apex_api::web::warnings::analyze_warning_html),
        )
        .route(
            "/warnings/:id/review",
            post(apex_api::web::warnings::review_warning_html),
        )
        .route(
            "/warnings/:id/notes",
            post(apex_api::web::warnings::create_warning_note),
        )
        .route("/insights", get(apex_api::web::insights::list_insights))
        .route("/insights/:id", get(apex_api::web::insights::get_insight))
        .route(
            "/insights/:id/bookmark",
            post(apex_api::web::insights::bookmark_insight_html),
        )
        .route(
            "/insights/:id/analyze",
            post(apex_api::web::insights::analyze_insight_html),
        )
        .route(
            "/insights/:id/notes",
            post(apex_api::web::insights::create_insight_note),
        )
        .route("/companies", get(apex_api::web::companies::list_companies))
        .route("/companies/:id", get(apex_api::web::companies::get_company))
        .route(
            "/companies/:id/changes",
            get(apex_api::web::companies::company_changes_tab),
        )
        .route(
            "/companies/:id/dossier",
            get(apex_api::web::companies::company_dossier_tab),
        )
        .route("/persons", get(apex_api::web::persons::list_persons))
        .route("/persons/:id", get(apex_api::web::persons::get_person))
        .route(
            "/competitors",
            get(apex_api::web::competitors::list_competitors),
        )
        .route("/graph", get(apex_api::web::graph::graph_page))
        .route("/recipes", get(apex_api::web::recipes::list_recipes))
        .route("/recipes/new", get(apex_api::web::recipes::new_recipe))
        .route("/search", get(apex_api::web::search::search_page))
        .route("/security", get(apex_api::web::security::security_page))
        .route(
            "/security/trigger-scan",
            post(apex_api::web::security::post_trigger_scan_html),
        )
        .route("/admin", get(apex_api::web::admin::admin_page))
        .route("/memos", get(apex_api::web::memos::list_memos))
        .route(
            "/notifications",
            get(apex_api::web::notifications::list_notifications_page),
        )
        .route(
            "/notifications/:id/read",
            post(apex_api::web::notifications::mark_notification_read),
        )
        .route(
            "/settings",
            get(apex_api::web::settings::settings_page)
                .post(apex_api::web::settings::save_settings),
        )
        .route_layer(middleware::from_fn(require_session))
        .layer(Extension(state.store.clone()))
        .layer(Extension(state.search_index.clone()));

    Router::new()
        .merge(public)
        .merge(protected)
        .merge(web_pages)
        .route("/ws/warnings", get(warnings_ws))
        .layer(middleware::from_fn(add_rate_limit_headers))
        .layer(Extension(state.rate_limiter.clone()))
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
        .with_state(state)
}
