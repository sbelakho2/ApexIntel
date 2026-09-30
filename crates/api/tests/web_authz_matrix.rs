//! Browser authorization matrix (audit P0-1/P0-2).
//!
//! Two complementary layers of evidence:
//!
//! 1. **Guard matrix** — the production middleware (`require_session`,
//!    `require_web_write`, `require_web_admin`) wired exactly like the HTML
//!    surface, exercised for every role × read/write/admin combination.
//! 2. **Real route table** — `apex_api::web::routes::build_web_pages`, the
//!    actual browser router, with every mutating registration parsed out of
//!    `web/routes.rs` and probed: a Viewer session must get 403 on each, an
//!    unauthenticated request must be redirected, so a newly registered
//!    mutation cannot ship without its guard.
//!
//! The database-backed authority contract (disabled rows, stale
//! `session_version`, role downgrade) is covered by
//! `session_authority_integration.rs` and the pure decision tests in
//! `middleware::session`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use apex_api::auth::ApiRole;
use apex_api::middleware::session::{
    create_session_token, require_session, require_web_admin, require_web_write,
    session_cookie_name, SessionAuthority, SessionAuthorityError, SessionClaims, WebSession,
    SESSION_TTL_MS, SESSION_VERSION,
};
use async_trait::async_trait;
use axum::body::Body;
use axum::http::{header, HeaderValue, Request, StatusCode};
use axum::middleware;
use axum::routing::{get, post};
use axum::{Extension, Router};
use chrono::Utc;
use tower::ServiceExt;

const TEST_SECRET: &str = "web-authz-matrix-session-secret";

fn ensure_session_secret() {
    std::env::set_var("SESSION_SECRET", TEST_SECRET);
}

/// Hermetic authority that accepts every signed session. The guard matrix is
/// about authorization (role enforcement); the canonical-row authority is
/// tested separately.
struct AcceptAllAuthority;

#[async_trait]
impl SessionAuthority for AcceptAllAuthority {
    async fn authorize(&self, session: &WebSession) -> Result<WebSession, SessionAuthorityError> {
        Ok(session.clone())
    }
}

/// Authority that rejects every session (disabled user / revoked session).
struct RejectAllAuthority;

#[async_trait]
impl SessionAuthority for RejectAllAuthority {
    async fn authorize(&self, _session: &WebSession) -> Result<WebSession, SessionAuthorityError> {
        Err(SessionAuthorityError::Disabled)
    }
}

fn authority() -> Arc<dyn SessionAuthority> {
    Arc::new(AcceptAllAuthority)
}

fn session_cookie(role: ApiRole, user_id: &str) -> String {
    let now = Utc::now().timestamp_millis();
    let claims = SessionClaims {
        user_id: user_id.into(),
        username: user_id.into(),
        role,
        issued_at: now,
        expires_at: now + SESSION_TTL_MS,
        session_version: SESSION_VERSION,
        session_id: uuid::Uuid::new_v4(),
    };
    let token = create_session_token(&claims, TEST_SECRET).expect("sign session token");
    format!("{}={token}", session_cookie_name())
}

fn legacy_cookie() -> String {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    let payload = serde_json::json!({
        "sub": "legacy",
        "iat": Utc::now().timestamp_millis(),
    });
    let payload_bytes = serde_json::to_vec(&payload).unwrap();
    let payload_b64 = URL_SAFE_NO_PAD.encode(&payload_bytes);
    let mut mac = Hmac::<Sha256>::new_from_slice(TEST_SECRET.as_bytes()).unwrap();
    mac.update(&payload_bytes);
    let sig = hex::encode(mac.finalize().into_bytes());
    format!("apex_session={payload_b64}.{sig}")
}

/// GET request with an optional session cookie.
fn get_request(path: &str, cookie: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder().method("GET").uri(path);
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    builder.body(Body::empty()).expect("request")
}

/// Independently derive the session-bound CSRF token (HMAC over the session
/// signature) the way the server does; a planted static value no longer passes.
fn derived_csrf(session_cookie: &str) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    let token = session_cookie.split_once('=').expect("cookie name=value").1;
    let signature = token.split_once('.').expect("payload.signature").1;
    let mut mac = Hmac::<Sha256>::new_from_slice(TEST_SECRET.as_bytes()).expect("hmac key");
    mac.update(b"csrf:");
    mac.update(signature.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

/// Unsafe request with an optional session cookie. When a session cookie is
/// present, the derived CSRF cookie + header is added so the request clears
/// the `require_session` CSRF check and reaches the role guard under test.
fn post_request(path: &str, cookie: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder().method("POST").uri(path);
    if let Some(cookie) = cookie {
        let csrf = derived_csrf(cookie);
        builder = builder
            .header(header::COOKIE, format!("{cookie}; apex_csrf={csrf}"))
            .header(
                "x-csrf-token",
                HeaderValue::from_str(&csrf).expect("csrf header"),
            );
    }
    builder.body(Body::empty()).expect("request")
}

async fn ok_handler() -> StatusCode {
    StatusCode::OK
}

/// The three-router split exactly as production wires it: reads behind
/// `require_session`, mutations behind `require_web_write` +
/// `require_session`, admin behind `require_web_admin` + `require_session`.
fn matrix_router(authority: Arc<dyn SessionAuthority>) -> Router {
    let web_read_pages = Router::new()
        .route("/page", get(ok_handler))
        .route_layer(middleware::from_fn(require_session));
    let web_write_pages = Router::new()
        .route("/mutation", post(ok_handler))
        .route_layer(middleware::from_fn(require_web_write))
        .route_layer(middleware::from_fn(require_session));
    let admin_pages = Router::new()
        .route("/admin", get(ok_handler))
        .route("/admin/mutation", post(ok_handler))
        .route_layer(middleware::from_fn(require_web_admin))
        .route_layer(middleware::from_fn(require_session));

    web_read_pages
        .merge(web_write_pages)
        .merge(admin_pages)
        .layer(Extension(authority))
}

async fn status(router: Router, request: Request<Body>) -> StatusCode {
    router.oneshot(request).await.expect("request").status()
}

async fn assert_redirects_to_login(router: Router, request: Request<Body>) {
    let response = router.oneshot(request).await.expect("request");
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let location = response
        .headers()
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    assert!(
        location.starts_with("/login"),
        "expected a /login redirect (with optional ?next=), got {location}"
    );
}

#[tokio::test]
async fn viewer_can_read_but_never_writes_or_admins() {
    ensure_session_secret();
    let router = matrix_router(authority());
    let cookie = session_cookie(ApiRole::Viewer, "usr-viewer");

    assert_eq!(
        status(router.clone(), get_request("/page", Some(&cookie))).await,
        StatusCode::OK,
        "Viewer GET must be allowed"
    );
    assert_eq!(
        status(router.clone(), post_request("/mutation", Some(&cookie))).await,
        StatusCode::FORBIDDEN,
        "Viewer ordinary mutation must be refused"
    );
    assert_eq!(
        status(router.clone(), get_request("/admin", Some(&cookie))).await,
        StatusCode::FORBIDDEN,
        "Viewer admin GET must be refused"
    );
    assert_eq!(
        status(router, post_request("/admin/mutation", Some(&cookie))).await,
        StatusCode::FORBIDDEN,
        "Viewer admin mutation must be refused"
    );
}

#[tokio::test]
async fn analyst_can_read_and_write_but_never_admins() {
    ensure_session_secret();
    let router = matrix_router(authority());
    let cookie = session_cookie(ApiRole::Analyst, "usr-analyst");

    assert_eq!(
        status(router.clone(), get_request("/page", Some(&cookie))).await,
        StatusCode::OK
    );
    assert_eq!(
        status(router.clone(), post_request("/mutation", Some(&cookie))).await,
        StatusCode::OK,
        "Analyst mutation must be allowed"
    );
    assert_eq!(
        status(router.clone(), get_request("/admin", Some(&cookie))).await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        status(router, post_request("/admin/mutation", Some(&cookie))).await,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn admin_can_read_write_and_admin() {
    ensure_session_secret();
    let router = matrix_router(authority());
    let cookie = session_cookie(ApiRole::Admin, "usr-admin");

    assert_eq!(
        status(router.clone(), get_request("/page", Some(&cookie))).await,
        StatusCode::OK
    );
    assert_eq!(
        status(router.clone(), post_request("/mutation", Some(&cookie))).await,
        StatusCode::OK
    );
    assert_eq!(
        status(router.clone(), get_request("/admin", Some(&cookie))).await,
        StatusCode::OK
    );
    assert_eq!(
        status(router, post_request("/admin/mutation", Some(&cookie))).await,
        StatusCode::OK
    );
}

#[tokio::test]
async fn service_is_read_only_with_no_browser_mutations() {
    // Policy: Service is a machine role. A browser session acting as Service
    // may read (all roles can read) but every mutation and admin surface is
    // refused because `ApiRole::Service::can_write()`/`can_admin()` are false.
    ensure_session_secret();
    let router = matrix_router(authority());
    let cookie = session_cookie(ApiRole::Service, "usr-service");

    assert_eq!(
        status(router.clone(), get_request("/page", Some(&cookie))).await,
        StatusCode::OK
    );
    assert_eq!(
        status(router.clone(), post_request("/mutation", Some(&cookie))).await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        status(router.clone(), get_request("/admin", Some(&cookie))).await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        status(router, post_request("/admin/mutation", Some(&cookie))).await,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn unauthenticated_requests_are_redirected_for_reads_and_writes() {
    ensure_session_secret();
    let router = matrix_router(authority());

    assert_redirects_to_login(router.clone(), get_request("/page", None)).await;
    assert_redirects_to_login(router.clone(), post_request("/mutation", None)).await;
    assert_redirects_to_login(router, get_request("/admin", None)).await;
}

#[tokio::test]
async fn legacy_cookie_without_principal_claims_is_redirected() {
    ensure_session_secret();
    let router = matrix_router(authority());

    assert_redirects_to_login(router, get_request("/page", Some(&legacy_cookie()))).await;
}

#[tokio::test]
async fn authority_rejection_redirects_even_with_a_valid_signature() {
    ensure_session_secret();
    let router = matrix_router(Arc::new(RejectAllAuthority));
    let cookie = session_cookie(ApiRole::Admin, "usr-disabled");

    assert_redirects_to_login(router, get_request("/page", Some(&cookie))).await;
}

#[tokio::test]
async fn write_guard_without_a_session_redirects_instead_of_allowing() {
    // Ordering safety: even if a mutation were registered without the session
    // layer, the write guard must not run the handler unauthenticated.
    ensure_session_secret();
    let router = Router::new()
        .route("/mutation", post(ok_handler))
        .route_layer(middleware::from_fn(require_web_write));

    assert_redirects_to_login(router, post_request("/mutation", None)).await;
}

// ── Real production route table ─────────────────────────────────────────────

const WEB_ROUTES_RS: &str = include_str!("../src/web/routes.rs");

/// Extract the path literals of every registration whose builder method is
/// `method_name` (e.g. `.post(` / `.admin_post(`). Handles both inline and
/// multi-line registrations.
fn registered_paths(source: &str, method_name: &str) -> Vec<String> {
    let marker = format!(".{method_name}(");
    let mut paths = Vec::new();
    let mut lines = source.lines().peekable();
    while let Some(line) = lines.next() {
        let trimmed = line.trim_start();
        let Some(rest) = trimmed.strip_prefix(&marker) else {
            continue;
        };
        let mut remainder = rest.to_string();
        let start = loop {
            if let Some(start) = remainder.find('"') {
                break start;
            }
            match lines.next() {
                Some(next) => {
                    remainder.push('\n');
                    remainder.push_str(next);
                }
                None => panic!("{marker} without a path literal"),
            }
        };
        let after_quote = &remainder[start + 1..];
        let end = after_quote
            .find('"')
            .unwrap_or_else(|| panic!("{marker} path literal is unterminated"));
        paths.push(after_quote[..end].to_string());
    }
    paths
}

fn concrete_path(path: &str) -> String {
    path.split('/')
        .map(|segment| {
            if segment.starts_with(':') {
                "test-id"
            } else {
                segment
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

#[tokio::test]
async fn every_registered_browser_mutation_requires_a_write_role() {
    ensure_session_secret();

    let mutations = registered_paths(WEB_ROUTES_RS, "post");
    let admin_mutations = registered_paths(WEB_ROUTES_RS, "admin_post");
    let self_mutations = registered_paths(WEB_ROUTES_RS, "self_post");
    assert!(
        mutations.len() >= 20,
        "expected to parse the browser mutation list, found {}",
        mutations.len()
    );
    assert!(
        admin_mutations.len() >= 3,
        "admin replay mutations and trigger-scan must be registered as admin routes, found {}",
        admin_mutations.len()
    );
    assert!(
        !self_mutations.is_empty(),
        "self-service mutations (settings, notifications, saved searches) must be registered"
    );

    let router = apex_api::web::routes::build_web_pages::<()>().layer(Extension(authority()));
    let viewer = session_cookie(ApiRole::Viewer, "usr-viewer");

    for path in mutations.iter().chain(admin_mutations.iter()) {
        let concrete = concrete_path(path);
        // Unauthenticated: the session guard redirects before anything runs.
        let response = router
            .clone()
            .oneshot(post_request(&concrete, None))
            .await
            .expect("request");
        assert_eq!(
            response.status(),
            StatusCode::SEE_OTHER,
            "{concrete} must redirect unauthenticated mutations (registered through the guarded builder)"
        );

        // Viewer: the write/admin guard refuses the mutation.
        let response = router
            .clone()
            .oneshot(post_request(&concrete, Some(&viewer)))
            .await
            .expect("request");
        assert_eq!(
            response.status(),
            StatusCode::FORBIDDEN,
            "{concrete} must refuse a Viewer browser session"
        );
    }

    // Self-service mutations are session-only by design (Viewers manage their
    // own settings/inbox/searches): they still redirect unauthenticated, and a
    // Viewer must not be refused by the guard.
    for path in &self_mutations {
        let concrete = concrete_path(path);
        let response = router
            .clone()
            .oneshot(post_request(&concrete, None))
            .await
            .expect("request");
        assert_eq!(
            response.status(),
            StatusCode::SEE_OTHER,
            "{concrete} must redirect unauthenticated mutations"
        );

        let response = router
            .clone()
            .oneshot(post_request(&concrete, Some(&viewer)))
            .await
            .expect("request");
        assert_ne!(
            response.status(),
            StatusCode::FORBIDDEN,
            "{concrete} is self-service and must not be refused to a Viewer"
        );
    }
}

#[tokio::test]
async fn every_registered_browser_read_page_requires_a_session() {
    ensure_session_secret();

    let reads = registered_paths(WEB_ROUTES_RS, "get");
    let admin_reads = registered_paths(WEB_ROUTES_RS, "admin_get");
    assert!(
        reads.len() >= 40,
        "expected to parse the browser read list, found {}",
        reads.len()
    );

    let router = apex_api::web::routes::build_web_pages::<()>().layer(Extension(authority()));

    for path in reads.iter().chain(admin_reads.iter()) {
        let concrete = concrete_path(path);
        let response = router
            .clone()
            .oneshot(get_request(&concrete, None))
            .await
            .expect("request");
        assert_eq!(
            response.status(),
            StatusCode::SEE_OTHER,
            "{concrete} must redirect an unauthenticated GET"
        );
    }
}
