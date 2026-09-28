//! P0 auth contract: browser-session authentication against the `/api/admin/*`
//! surface.
//!
//! The admin web session (the configured `APEX_ADMIN_USERNAME` principal) must
//! be permitted on admin APIs, while an analyst-level session must receive 403.
//!
//! The test wires the same production middleware functions the binary uses —
//! [`authenticate_request_or_session`] (Bearer key, then session fallback) and
//! [`require_admin`] — and drives them through a real Axum router. When the
//! P0 auth change landed, sessions for the configured admin username began
//! mapping to [`ApiRole::Admin`]; every other session stays [`ApiRole::Analyst`].

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;

use apex_api::auth::ApiRole;
use apex_api::middleware::auth::{
    auth_error_response, authenticate_request_or_session, require_admin, session_api_auth_context,
};
use apex_api::middleware::session::{create_session_token, SessionClaims};
use apex_core::identity::{UserId, Username};
use axum::body::Body;
use axum::http::{header, HeaderValue, Method, Request, StatusCode};
use axum::middleware;
use axum::routing::{get, post};
use axum::Router;
use http_body_util::BodyExt;
use tower::ServiceExt;

const SESSION_SECRET: &str = "session-admin-auth-test-secret";

fn signed_token(username: &str) -> String {
    let claims = SessionClaims {
        user_id: UserId::new(username),
        username: Username::new(username),
        role: ApiRole::Analyst,
        issued_at: chrono::Utc::now().timestamp(),
        expires_at: i64::MAX,
        session_version: 1,
    };
    create_session_token(&claims, SESSION_SECRET).expect("session token")
}
const ADMIN_USERNAME: &str = "admin";

/// Test stand-in for the binary's `require_auth` middleware: accepts a Bearer
/// key (none configured here) or the session cookie, using the exact library
/// function the binary calls.
async fn session_or_key_auth(
    mut request: Request<Body>,
    next: middleware::Next,
) -> axum::response::Response {
    let api_keys: HashMap<String, apex_api::auth::ApiKey> = HashMap::new();
    match authenticate_request_or_session(
        request.headers(),
        request.method(),
        &api_keys,
        SESSION_SECRET,
        ADMIN_USERNAME,
        chrono::Utc::now(),
    ) {
        Ok(context) => {
            request.extensions_mut().insert(context);
            next.run(request).await
        }
        Err(error) => auth_error_response(error),
    }
}

fn admin_surface_router() -> Router {
    Router::new()
        .route("/api/admin/probe", get(|| async { "admin-ok" }))
        .route("/api/admin/probe", post(|| async { "admin-post-ok" }))
        .route_layer(middleware::from_fn(require_admin))
        .route_layer(middleware::from_fn(session_or_key_auth))
}

fn session_cookie(username: &str) -> HeaderValue {
    let token = signed_token(username);
    HeaderValue::from_str(&format!("apex_session={token}")).expect("cookie header")
}

fn request(method: &str, path: &str, cookie: Option<HeaderValue>) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(path);
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    builder.body(Body::empty()).expect("request")
}

async fn status_of(app: Router, request: Request<Body>) -> (StatusCode, String) {
    let response = app.oneshot(request).await.expect("response");
    let status = response.status();
    let body = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    (status, String::from_utf8_lossy(&body).to_string())
}

#[test]
fn session_role_mapping_is_admin_for_configured_admin_and_analyst_otherwise() {
    let admin_headers = {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(header::COOKIE, session_cookie("admin"));
        headers
    };
    let admin =
        session_api_auth_context(&admin_headers, &Method::GET, SESSION_SECRET, ADMIN_USERNAME)
            .expect("admin session context");
    assert_eq!(admin.role, ApiRole::Admin);
    assert_eq!(admin.user_id, "admin");

    let analyst_headers = {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(header::COOKIE, session_cookie("analyst-user"));
        headers
    };
    let analyst = session_api_auth_context(
        &analyst_headers,
        &Method::GET,
        SESSION_SECRET,
        ADMIN_USERNAME,
    )
    .expect("analyst session context");
    assert_eq!(analyst.role, ApiRole::Analyst);
}

#[tokio::test]
async fn admin_session_is_permitted_on_admin_api() {
    let (status, body) = status_of(
        admin_surface_router(),
        request("GET", "/api/admin/probe", Some(session_cookie("admin"))),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "admin session should be permitted");
    assert_eq!(body, "admin-ok");
}

#[tokio::test]
async fn analyst_session_is_forbidden_on_admin_api() {
    let (status, _) = status_of(
        admin_surface_router(),
        request(
            "GET",
            "/api/admin/probe",
            Some(session_cookie("analyst-user")),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "analyst session must not reach the admin surface"
    );
}

#[tokio::test]
async fn missing_session_is_unauthorized_on_admin_api() {
    let (status, _) = status_of(
        admin_surface_router(),
        request("GET", "/api/admin/probe", None),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn invalid_bearer_key_is_not_downgraded_to_the_session() {
    let token = signed_token("admin");
    let mut request = request("GET", "/api/admin/probe", None);
    request.headers_mut().insert(
        header::AUTHORIZATION,
        HeaderValue::from_static("Bearer not-a-real-key"),
    );
    request.headers_mut().insert(
        header::COOKIE,
        HeaderValue::from_str(&format!("apex_session={token}")).expect("cookie"),
    );

    let (status, _) = status_of(admin_surface_router(), request).await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "a presented-but-invalid key must fail instead of falling back to the session"
    );
}

#[tokio::test]
async fn admin_session_unsafe_method_requires_csrf() {
    // Without the double-submit token, an unsafe method is rejected.
    let (status, _) = status_of(
        admin_surface_router(),
        request("POST", "/api/admin/probe", Some(session_cookie("admin"))),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // With a matching cookie + header pair, the admin session is permitted.
    let token = signed_token("admin");
    let mut request = request("POST", "/api/admin/probe", None);
    request.headers_mut().insert(
        header::COOKIE,
        HeaderValue::from_str(&format!("apex_session={token}; apex_csrf=csrf-token"))
            .expect("cookie"),
    );
    request
        .headers_mut()
        .insert("x-csrf-token", HeaderValue::from_static("csrf-token"));

    let (status, body) = status_of(admin_surface_router(), request).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "admin-post-ok");
}
