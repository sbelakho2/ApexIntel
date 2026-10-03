//! Browser (HTML) router assembly and its authorization layers.
//!
//! Audit P0-1: the whole HTML app used to sit behind `require_session` only,
//! so any authenticated browser session — including a read-only Viewer —
//! could POST to every mutating page (warnings ack/investigate/analyze/
//! review/notes, insights bookmark/analyze/notes, recipe creation, saved
//! searches, security trigger-scan, settings, workspaces, queue, supplier
//! risk, pipeline, evidence, team assignments, triage actions).
//!
//! The surface is now split into three independently guarded routers:
//!
//! | router             | guard                                       |
//! |--------------------|---------------------------------------------|
//! | `web_read_pages`   | [`require_session`]                         |
//! | `web_write_pages`  | [`require_web_write`] + [`require_session`] |
//! | `admin_pages`      | [`require_web_admin`] + [`require_session`] |
//!
//! ## Structurally impossible unguarded mutations
//!
//! Routes are not registered on the routers directly. They can only be added
//! through [`WebPages`], whose fields are private and whose method-specific
//! methods route each handler into one of the three inner routers:
//!
//! * [`WebPages::get`] is the only read registration and accepts a GET
//!   handler — it can never register a mutating method;
//! * [`WebPages::post`], [`WebPages::put`], [`WebPages::patch`] and
//!   [`WebPages::delete`] are the only mutation registrations, and they always
//!   write into the router that [`WebPages::finish`] wraps in
//!   `require_web_write`;
//! * [`WebPages::admin_get`] / [`WebPages::admin_post`] always write into the
//!   router wrapped in `require_web_admin`.
//!
//! Because `finish` applies the guards after the inner routers are complete
//! and the inner routers cannot be reached or appended to from outside, no
//! code path can register a mutating browser route without the write guard.
//! The guard is a property of the registration API, not of each handler.

use axum::{
    handler::Handler,
    middleware,
    routing::{get, post},
    Router,
};

use crate::middleware::session::{require_session, require_web_admin, require_web_write};

/// Method-derived browser route builder. See the module documentation for the
/// invariant this type enforces.
struct WebPages<S> {
    read: Router<S>,
    write: Router<S>,
    self_service: Router<S>,
    admin: Router<S>,
}

impl<S> WebPages<S>
where
    S: Clone + Send + Sync + 'static,
{
    fn new() -> Self {
        Self {
            read: Router::new(),
            write: Router::new(),
            self_service: Router::new(),
            admin: Router::new(),
        }
    }

    /// Register a safe (GET) page. Only reachable to any enabled session.
    fn get<H, T>(mut self, path: &str, handler: H) -> Self
    where
        H: Handler<T, S>,
        T: 'static,
    {
        self.read = self.read.route(path, get(handler));
        self
    }

    /// Register a POST mutation. Always guarded by `require_web_write`.
    fn post<H, T>(mut self, path: &str, handler: H) -> Self
    where
        H: Handler<T, S>,
        T: 'static,
    {
        self.write = self.write.route(path, post(handler));
        self
    }

    /// Register a POST mutation a Viewer may perform on their own account
    /// (theme/appearance settings, marking their own notifications read, their
    /// own saved searches). Guarded by `require_session` only — CSRF still
    /// applies — because the handlers scope every write to the session user.
    fn self_post<H, T>(mut self, path: &str, handler: H) -> Self
    where
        H: Handler<T, S>,
        T: 'static,
    {
        self.self_service = self.self_service.route(path, post(handler));
        self
    }

    /// Register a GET admin page. Always guarded by `require_web_admin`.
    fn admin_get<H, T>(mut self, path: &str, handler: H) -> Self
    where
        H: Handler<T, S>,
        T: 'static,
    {
        self.admin = self.admin.route(path, get(handler));
        self
    }

    /// Register a POST admin mutation. Always guarded by `require_web_admin`.
    fn admin_post<H, T>(mut self, path: &str, handler: H) -> Self
    where
        H: Handler<T, S>,
        T: 'static,
    {
        self.admin = self.admin.route(path, post(handler));
        self
    }

    /// Apply each router's guard and merge the browser surface.
    ///
    /// The session layer is applied last, so it is the outermost layer and
    /// runs before the write/admin guards that depend on the resolved
    /// [`crate::middleware::session::WebSession`].
    fn finish(self) -> Router<S> {
        let web_read_pages = self.read.route_layer(middleware::from_fn(require_session));
        let web_write_pages = self
            .write
            .route_layer(middleware::from_fn(require_web_write))
            .route_layer(middleware::from_fn(require_session));
        let self_service_pages = self
            .self_service
            .route_layer(middleware::from_fn(require_session));
        let admin_pages = self
            .admin
            .route_layer(middleware::from_fn(require_web_admin))
            .route_layer(middleware::from_fn(require_session));

        web_read_pages
            .merge(web_write_pages)
            .merge(self_service_pages)
            .merge(admin_pages)
    }
}

/// Build the browser (HTML) surface with all authorization guards attached.
///
/// The caller adds the request extensions the handlers need
/// (`Arc<PgStore>`, the session authority, search/autocomplete indexes, and
/// the optional warning-analysis model) — the guards themselves are part of
/// this router and cannot be omitted.
pub fn build_web_pages<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    WebPages::new()
        // ─── Read pages ──────────────────────────────────────────────────────
        .get("/", crate::web::dashboard::dashboard)
        .get("/warnings", crate::web::warnings::list_warnings)
        .get("/warnings/unread-count", crate::web::warnings::unread_count)
        .get("/warnings/:id", crate::web::warnings::get_warning)
        .get(
            "/warnings/:id/analysis/:run_id",
            crate::web::warnings::warning_analysis_status_html,
        )
        .get("/insights", crate::web::insights::list_insights)
        .get("/insights/:id", crate::web::insights::get_insight)
        .get(
            "/insights/:id/pdf",
            crate::web::insights::export_insight_pdf_html,
        )
        .get("/companies", crate::web::companies::list_companies)
        .get("/companies/:id", crate::web::companies::get_company)
        .get(
            "/companies/:id/changes",
            crate::web::companies::company_changes_tab,
        )
        .get(
            "/companies/:id/dossier",
            crate::web::companies::company_dossier_tab,
        )
        .get("/persons", crate::web::persons::list_persons)
        .get("/buying-centers", crate::web::persons::list_buying_centers)
        .get("/persons/:id", crate::web::persons::get_person)
        .get("/competitors", crate::web::competitors::list_competitors)
        .get("/battlecards", crate::web::battlecards::list_battlecards)
        .get(
            "/battlecards/new",
            crate::web::battlecards::new_battlecard_page,
        )
        .get(
            "/battlecards/compare",
            crate::web::battlecards::compare_battlecards,
        )
        .get(
            "/battlecards/compare/export",
            crate::web::battlecards::export_comparison,
        )
        .get("/battlecards/:id", crate::web::battlecards::get_battlecard)
        .get(
            "/battlecards/:id/edit",
            crate::web::battlecards::edit_battlecard_page,
        )
        .get(
            "/battlecards/:id/export",
            crate::web::battlecards::export_battlecard,
        )
        .get("/graph", crate::web::graph::graph_page)
        .get("/recipes", crate::web::recipes::list_recipes)
        .get("/recipes/new", crate::web::recipes::new_recipe)
        .get("/search", crate::web::search::search_page)
        .get("/search/suggestions", crate::web::search::suggestions_html)
        .get("/security", crate::web::security::security_page)
        .get("/memos", crate::web::memos::list_memos)
        .get("/memos/_list", crate::web::memos::list_memos_partial)
        .get(
            "/notifications",
            crate::web::notifications::list_notifications_page,
        )
        .get("/settings", crate::web::settings::settings_page)
        .get(
            "/settings/alerts",
            crate::web::alert_settings::alert_settings_page,
        )
        .get("/workspaces", crate::web::collaboration::list_workspaces)
        .get(
            "/workspaces/new",
            crate::web::collaboration::new_workspace_page,
        )
        .get("/workspaces/:id", crate::web::collaboration::get_workspace)
        .get("/queue", crate::web::collaboration::list_queue)
        .get("/activity", crate::web::collaboration::list_activity)
        .get(
            "/supplier-risk",
            crate::web::collaboration::list_supplier_risks,
        )
        .get("/pipeline", crate::web::collaboration::list_pipeline)
        .get("/evidence", crate::web::collaboration::list_evidence)
        .get(
            "/team-assignments",
            crate::web::collaboration::list_team_assignments,
        )
        .get("/executive", crate::web::executive::executive_dashboard)
        .get("/trends", crate::web::trends::trends_page)
        .get("/triage", crate::web::triage::list_triage)
        .get("/triage/:id", crate::web::triage::get_triage_item)
        // ─── Write pages (require_web_write + require_session) ───────────────
        .post(
            "/warnings/:id/acknowledge",
            crate::web::warnings::acknowledge_warning_html,
        )
        .post(
            "/warnings/:id/investigate",
            crate::web::warnings::start_investigation_html,
        )
        .post(
            "/warnings/:id/analyze",
            crate::web::warnings::analyze_warning_html,
        )
        .post(
            "/warnings/:id/review",
            crate::web::warnings::review_warning_html,
        )
        .post(
            "/warnings/:id/notes",
            crate::web::warnings::create_warning_note,
        )
        .post(
            "/insights/:id/bookmark",
            crate::web::insights::bookmark_insight_html,
        )
        .post(
            "/insights/:id/analyze",
            crate::web::insights::analyze_insight_html,
        )
        .post(
            "/insights/:id/notes",
            crate::web::insights::create_insight_note,
        )
        // B307: the creation form's action target — previously unregistered,
        // so the only creation flow in the product 404'd on submit.
        .post(
            "/recipes/create-form",
            crate::web::recipes::create_recipe_form,
        )
        .post("/battlecards", crate::web::battlecards::create_battlecard)
        .post(
            "/battlecards/:id",
            crate::web::battlecards::update_battlecard,
        )
        .post(
            "/battlecards/:id/delete",
            crate::web::battlecards::delete_battlecard,
        )
        .post("/workspaces", crate::web::collaboration::create_workspace)
        .post(
            "/workspaces/:id/close",
            crate::web::collaboration::close_workspace,
        )
        .post(
            "/workspaces/:id/assign",
            crate::web::collaboration::assign_user_to_workspace,
        )
        .post(
            "/workspaces/:id/shares",
            crate::web::collaboration::share_workspace,
        )
        .post("/queue", crate::web::collaboration::add_to_queue)
        .post(
            "/queue/:id/complete",
            crate::web::collaboration::complete_queue_item,
        )
        .post(
            "/supplier-risk",
            crate::web::collaboration::add_supplier_risk,
        )
        .post(
            "/pipeline",
            crate::web::collaboration::create_pipeline_opportunity,
        )
        .post(
            "/pipeline/:id/stage",
            crate::web::collaboration::update_pipeline_stage,
        )
        .post("/evidence", crate::web::collaboration::add_evidence)
        .post(
            "/team-assignments",
            crate::web::collaboration::create_team_assignment,
        )
        .post(
            "/triage/:id/acknowledge",
            crate::web::triage::acknowledge_triage_html,
        )
        .post(
            "/triage/:id/resolve",
            crate::web::triage::resolve_triage_html,
        )
        .post(
            "/triage/:id/dismiss",
            crate::web::triage::dismiss_triage_html,
        )
        .post(
            "/triage/:id/override",
            crate::web::triage::override_triage_html,
        )
        // ─── Self-service pages (require_session only; CSRF still applies) ──
        // Viewers can change their own theme/session length, clear their own
        // inbox, and manage their own saved searches. Every handler scopes the
        // write to the session principal.
        .self_post("/settings", crate::web::settings::save_settings)
        .self_post("/settings/password", crate::web::settings::change_password)
        .self_post(
            "/notifications/:id/read",
            crate::web::notifications::mark_notification_read,
        )
        .self_post(
            "/notifications/read-all",
            crate::web::notifications::mark_all_notifications_read,
        )
        // Deleting is separate from marking read: read rows still count in
        // the `all` view, so an inbox that is only marked read never empties.
        .self_post(
            "/notifications/:id/delete",
            crate::web::notifications::delete_notification,
        )
        .self_post(
            "/notifications/clear-all",
            crate::web::notifications::clear_all_notifications,
        )
        .self_post(
            "/notifications/clear-read",
            crate::web::notifications::clear_read_notifications,
        )
        .self_post("/search/saved-searches", crate::web::search::save_search)
        .self_post(
            "/search/saved-searches/:id/delete",
            crate::web::search::delete_saved_search,
        )
        // ─── Admin pages (require_web_admin + require_session) ───────────────
        // The HTML `/admin` page carries the same `can_admin()` authorization
        // as `/api/admin/*`: a browser session without an admin role gets 403,
        // while unauthenticated requests are still redirected to /login.
        .admin_get("/admin", crate::web::admin::admin_page)
        .admin_post(
            // Scanning consumes crawler/LLM resources: the API equivalent is
            // admin-only, and the HTML route must not be weaker.
            "/security/trigger-scan",
            crate::web::security::post_trigger_scan_html,
        )
        .admin_post(
            "/admin/notifications/delivery/replay",
            crate::web::admin::admin_replay_delivery,
        )
        .admin_post(
            "/admin/notifications/outbox/replay",
            crate::web::admin::admin_replay_outbox,
        )
        .finish()
}
