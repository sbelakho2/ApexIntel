use crate::*;

use apex_api::middleware::session::require_session;
use axum::{
    middleware,
    routing::{delete, get, patch, post, put},
    Extension, Router,
};
use tower_http::{cors::CorsLayer, services::ServeDir, trace::TraceLayer};

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
        .route(
            "/login",
            get(apex_api::web::auth::login_page).post(apex_api::web::auth::login_submit),
        )
        .route("/logout", post(apex_api::web::auth::logout))
        // ─── PWA: Service worker (served without auth) ─────────────────
        .route("/sw.js", get(sw_js));

    let protected = Router::new()
        // /metrics exposes platform scale (companies, persons, warnings) and
        // runs DB aggregates per scrape — served behind API auth (B299). Point
        // Prometheus at it with an `Authorization: Bearer <key>` header.
        .route("/metrics", get(runtime_metrics::metrics))
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
            "/api/insights/:id/investigate",
            post(insights_handlers::investigate_insight),
        )
        .route(
            "/api/insights/:id/pdf",
            get(exports_handlers::export_insight_pdf),
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
        .route(
            "/api/companies/:id/dossier/pdf",
            get(exports_handlers::export_company_dossier_pdf),
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
            "/api/persons/:id/dossier/pdf",
            get(exports_handlers::export_person_dossier_pdf),
        )
        .route(
            "/api/persons/:id/engagement",
            get(dossiers_handlers::get_person_engagement),
        )
        // ─── Psychological profiling (canonical psych tables) ───────────────
        .route(
            "/api/persons/:id/psych",
            get(psych_handlers::get_person_psych),
        )
        .route(
            "/api/persons/:id/behavioral-patterns",
            get(psych_handlers::get_person_behavioral_patterns),
        )
        .route(
            "/api/persons/:id/engagement-profile",
            get(psych_handlers::get_person_engagement_profile),
        )
        .route(
            "/api/persons/:id/role-history",
            get(dossiers_handlers::get_person_role_history),
        )
        .route(
            "/api/persons/:id/changes",
            get(dossiers_handlers::get_person_changes_api),
        )
        // ─── Sales activation: contacts + outreach feedback loop ──────────
        .route(
            "/api/persons/:id/contacts",
            get(sales_handlers::list_person_contacts),
        )
        .route(
            "/api/persons/:id/outreach",
            get(sales_handlers::list_person_engagement).post(sales_handlers::record_engagement),
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
        // ─── Sales activation: buying center graph ───────────────────────
        .route(
            "/api/companies/:id/buying-center",
            get(sales_handlers::list_company_buying_center),
        )
        .route(
            "/api/companies/:id/buying-center/members",
            post(sales_handlers::add_buying_member),
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
        // Battlecards
        .route(
            "/api/battlecards",
            get(battlecards_handlers::list_battlecards)
                .post(battlecards_handlers::create_battlecard),
        )
        .route(
            "/api/battlecards/:id",
            get(battlecards_handlers::get_battlecard)
                .delete(battlecards_handlers::delete_battlecard),
        )
        .route(
            "/api/battlecards/:id/section",
            put(battlecards_handlers::update_battlecard_section),
        )
        .route(
            "/api/battlecards/:id/regenerate",
            post(battlecards_handlers::regenerate_battlecard),
        )
        .route(
            "/api/battlecards/:id/export",
            get(battlecards_handlers::export_battlecard),
        )
        .route("/api/search", get(overview_handlers::search))
        .route(
            "/api/search/semantic",
            get(overview_handlers::semantic_search),
        )
        .route("/api/search/suggest", get(overview_handlers::suggest))
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
        // ─── Alert Settings API Routes ───────────────────────────────────
        .route(
            "/api/settings/alerts",
            get(alert_settings_handlers::list_alert_settings),
        )
        .route(
            "/api/settings/alerts/entity/:entity_id",
            get(alert_settings_handlers::get_entity_alert_config)
                .put(alert_settings_handlers::upsert_entity_alert_config)
                .delete(alert_settings_handlers::delete_entity_alert_config),
        )
        .route(
            "/api/settings/alerts/global",
            put(alert_settings_handlers::upsert_global_alert_defaults),
        )
        // B292: admin-only surface. Registered as a separate router so the
        // `require_admin` layer scopes to exactly these routes — a layer in
        // the main chain would also gate every route registered above it.
        .merge(
            Router::new()
                .route("/api/admin/crawl-status", get(get_admin_crawl_status))
                .route(
                    "/api/admin/recipe-performance",
                    get(get_admin_recipe_performance),
                )
                .route("/api/admin/poi-coverage", get(get_admin_poi_coverage))
                .route("/api/admin/trigger-scan", post(post_trigger_scan))
                .route(
                    "/api/admin/search/rebuild-autocomplete",
                    post(post_rebuild_autocomplete),
                )
                .route(
                    "/api/admin/embeddings/reindex",
                    post(vector_search_handlers::reindex_embeddings),
                )
                .route_layer(middleware::from_fn(require_admin)),
        )
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
        // ─── Entity Trend Chart Data Routes ──────────────────────────────
        .route(
            "/api/charts/entity/:id/activity",
            get(charts_handlers::get_entity_activity_chart),
        )
        .route(
            "/api/charts/entity/:id/activity/svg",
            get(charts_handlers::get_entity_activity_chart_svg),
        )
        .route(
            "/api/charts/entity/:id/observations",
            get(charts_handlers::get_entity_observation_chart),
        )
        // ─── Phase 4.3: Executive Dashboard ─────────────────────────────
        .route(
            "/api/executive/summary",
            get(collaboration_handlers::get_executive_summary),
        )
        .route(
            "/api/executive/opportunities",
            get(collaboration_handlers::list_opportunities)
                .post(collaboration_handlers::create_opportunity),
        )
        .route(
            "/api/executive/opportunities/:id",
            get(collaboration_handlers::get_opportunity)
                .patch(collaboration_handlers::update_opportunity),
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
            get(collaboration_handlers::list_workspaces)
                .post(collaboration_handlers::create_workspace),
        )
        .route(
            "/api/workspaces/:id",
            get(collaboration_handlers::get_workspace)
                .patch(collaboration_handlers::update_workspace)
                .delete(collaboration_handlers::delete_workspace),
        )
        .route(
            "/api/workspaces/:id/assignments",
            get(collaboration_handlers::list_workspace_assignments)
                .post(collaboration_handlers::assign_user_to_workspace),
        )
        .route(
            "/api/workspaces/:id/assignments/:user_id",
            delete(collaboration_handlers::remove_user_from_workspace),
        )
        .route(
            "/api/workspaces/:id/shares",
            get(collaboration_handlers::list_workspace_shares)
                .post(collaboration_handlers::share_workspace),
        )
        // ─── Phase 4.3: Activity Feed ─────────────────────────────────────
        .route(
            "/api/activity-feed",
            get(collaboration_handlers::get_activity_feed)
                .post(collaboration_handlers::record_activity),
        )
        // ─── Standalone Activity Feed API ──────────────────────────────────
        .route(
            "/api/activity",
            get(activity_handlers::get_activity_feed)
                .post(activity_handlers::create_activity_event),
        )
        // ─── Supply Chain Risk API ────────────────────────────────────────
        .route(
            "/api/supply-risk",
            get(supply_risk_handlers::get_supply_risks),
        )
        // ─── Threat Intelligence API ──────────────────────────────────────
        .route(
            "/api/threat-intel",
            get(threat_intel_handlers::get_threat_intel),
        )
        // ─── Psychological Profiles API ──────────────────────────────────
        .route(
            "/api/psych-profiles",
            get(psych_profiles_handlers::get_psych_profiles),
        )
        // ─── ICP Sales Targeting API ──────────────────────────────────────
        .route("/api/icp/targets", get(icp_handlers::list_icp_targets))
        .route(
            "/api/icp/companies/:id/score",
            post(icp_handlers::score_company_icp),
        )
        // ─── Phase 4.3: Daily Priority Queue ─────────────────────────────
        .route(
            "/api/queue",
            get(collaboration_handlers::list_queue_items)
                .post(collaboration_handlers::add_to_queue),
        )
        .route(
            "/api/queue/:id",
            patch(collaboration_handlers::update_queue_item),
        )
        // ─── Phase 4.3: Supplier Risk ──────────────────────────────────────
        .route(
            "/api/supplier-risk",
            get(collaboration_handlers::list_supplier_risks)
                .post(collaboration_handlers::add_supplier_risk),
        )
        .route(
            "/api/supplier-risk/:id",
            patch(collaboration_handlers::update_supplier_risk),
        )
        // ─── Phase 4.3: Pipeline Opportunities ────────────────────────────
        .route(
            "/api/pipeline",
            get(collaboration_handlers::list_pipeline_opportunities)
                .post(collaboration_handlers::create_pipeline_opportunity),
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
        // ─── Real-time Events (SSE) ────────────────────────────────────────
        .route("/api/v1/events/stream", get(crate::alert_sse_handler))
        // ─── Phase 4.3: Team Assignments ──────────────────────────────────
        .route(
            "/api/team-assignments",
            get(collaboration_handlers::list_team_assignments)
                .post(collaboration_handlers::create_team_assignment),
        )
        // ─── Vector Search Routes ──────────────────────────────────────────
        .route(
            "/api/search/vector",
            get(vector_search_handlers::vector_search),
        )
        .route(
            "/api/entities/:entity_type/:entity_id/similar",
            get(vector_search_handlers::similar_entities),
        )
        // ─── AI Triage Engine API Routes ────────────────────────────────
        .route("/api/triage", get(triage_handlers::list_triage))
        .route("/api/triage/stats", get(triage_handlers::get_triage_stats))
        .route("/api/triage/bands", get(triage_handlers::get_triage_bands))
        .route("/api/triage/:id", get(triage_handlers::get_triage_item))
        .route(
            "/api/triage/:id/override",
            post(triage_handlers::override_triage_score),
        )
        .route(
            "/api/triage/:id/acknowledge",
            post(triage_handlers::acknowledge_triage_item),
        )
        .route(
            "/api/triage/:id/resolve",
            post(triage_handlers::resolve_triage_item),
        )
        .route(
            "/api/triage/:id/dismiss",
            post(triage_handlers::dismiss_triage_item),
        )
        // ─── Historical Trends API Routes ─────────────────────────────────
        .route("/api/trends", get(trends_handlers::query_trends))
        .route(
            "/api/trends/comparison",
            get(trends_handlers::trend_comparison),
        )
        .route("/api/trends/entities", get(trends_handlers::entity_trends))
        .route_layer(middleware::from_fn_with_state(state.clone(), require_auth))
        // B300: `/api/trends*` handlers extract `Extension<Arc<PgStore>>`, which
        // was previously provided only to the web-page router — every trends
        // request failed with "Missing request extension" (500). Provide the
        // store (and search index) extensions to the JSON API router as well.
        .layer(Extension(state.store.clone()))
        .layer(Extension(state.search_index.clone()));

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
            "/insights/:id/pdf",
            get(apex_api::web::insights::export_insight_pdf_html),
        )
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
        .route(
            "/battlecards",
            get(apex_api::web::battlecards::list_battlecards),
        )
        .route(
            "/battlecards/:id",
            get(apex_api::web::battlecards::get_battlecard),
        )
        .route("/graph", get(apex_api::web::graph::graph_page))
        .route("/recipes", get(apex_api::web::recipes::list_recipes))
        .route("/recipes/new", get(apex_api::web::recipes::new_recipe))
        // B307: the creation form's action target — previously unregistered,
        // so the only creation flow in the product 404'd on submit.
        .route(
            "/recipes/create-form",
            post(apex_api::web::recipes::create_recipe_form),
        )
        .route("/search", get(apex_api::web::search::search_page))
        .route(
            "/search/suggestions",
            get(apex_api::web::search::suggestions_html),
        )
        .route("/security", get(apex_api::web::security::security_page))
        .route(
            "/security/trigger-scan",
            post(apex_api::web::security::post_trigger_scan_html),
        )
        .route("/admin", get(apex_api::web::admin::admin_page))
        .route("/memos", get(apex_api::web::memos::list_memos))
        .route(
            "/memos/_list",
            get(apex_api::web::memos::list_memos_partial),
        )
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
        .route(
            "/settings/alerts",
            get(apex_api::web::alert_settings::alert_settings_page),
        )
        // ─── Collaboration Web Routes ──────────────────────────────────────
        .route(
            "/workspaces",
            get(apex_api::web::collaboration::list_workspaces),
        )
        .route(
            "/workspaces/new",
            get(apex_api::web::collaboration::new_workspace_page),
        )
        .route(
            "/workspaces",
            post(apex_api::web::collaboration::create_workspace),
        )
        .route(
            "/workspaces/:id",
            get(apex_api::web::collaboration::get_workspace),
        )
        .route(
            "/workspaces/:id/close",
            post(apex_api::web::collaboration::close_workspace),
        )
        .route(
            "/workspaces/:id/assign",
            post(apex_api::web::collaboration::assign_user_to_workspace),
        )
        .route(
            "/workspaces/:id/shares",
            post(apex_api::web::collaboration::share_workspace),
        )
        .route("/queue", get(apex_api::web::collaboration::list_queue))
        .route("/queue", post(apex_api::web::collaboration::add_to_queue))
        .route(
            "/queue/:id/complete",
            post(apex_api::web::collaboration::complete_queue_item),
        )
        .route(
            "/activity",
            get(apex_api::web::collaboration::list_activity),
        )
        .route(
            "/supplier-risk",
            get(apex_api::web::collaboration::list_supplier_risks),
        )
        .route(
            "/supplier-risk",
            post(apex_api::web::collaboration::add_supplier_risk),
        )
        .route(
            "/pipeline",
            get(apex_api::web::collaboration::list_pipeline),
        )
        .route(
            "/pipeline",
            post(apex_api::web::collaboration::create_pipeline_opportunity),
        )
        .route(
            "/pipeline/:id/stage",
            post(apex_api::web::collaboration::update_pipeline_stage),
        )
        .route(
            "/evidence",
            get(apex_api::web::collaboration::list_evidence),
        )
        .route(
            "/evidence",
            post(apex_api::web::collaboration::add_evidence),
        )
        .route(
            "/team-assignments",
            get(apex_api::web::collaboration::list_team_assignments),
        )
        .route(
            "/team-assignments",
            post(apex_api::web::collaboration::create_team_assignment),
        )
        // ─── Executive Dashboard Web Route ─────────────────────────────────
        .route(
            "/executive",
            get(apex_api::web::executive::executive_dashboard),
        )
        // ─── Historical Trends Web Route ───────────────────────────────────
        .route("/trends", get(apex_api::web::trends::trends_page))
        // ─── AI Triage Engine Web Routes ─────────────────────────────────
        .route("/triage", get(apex_api::web::triage::list_triage))
        .route("/triage/:id", get(apex_api::web::triage::get_triage_item))
        .route(
            "/triage/:id/acknowledge",
            post(apex_api::web::triage::acknowledge_triage_html),
        )
        .route(
            "/triage/:id/resolve",
            post(apex_api::web::triage::resolve_triage_html),
        )
        .route(
            "/triage/:id/dismiss",
            post(apex_api::web::triage::dismiss_triage_html),
        )
        .route(
            "/triage/:id/override",
            post(apex_api::web::triage::override_triage_html),
        )
        .route_layer(middleware::from_fn(require_session))
        .layer(Extension(state.store.clone()))
        .layer(Extension(state.search_index.clone()))
        .layer(Extension(state.autocomplete_index.clone()));

    Router::new()
        .merge(public)
        .merge(protected)
        .merge(web_pages)
        .route("/ws/warnings", get(warnings_ws))
        // ─── PWA static files (dev mode; nginx serves in production) ───
        .nest_service(
            "/static",
            ServeDir::new(concat!(env!("CARGO_MANIFEST_DIR"), "/static")),
        )
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
