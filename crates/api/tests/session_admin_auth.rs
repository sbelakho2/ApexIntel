//! P0 auth contract: browser-session authentication against the `/api/admin/*`
//! surface.
//!
//! This exercises the production path — [`require_api_auth`] with a real
//! [`SessionAuthority`] — rather than the deleted weaker helper that decided
//! admin by username and skipped the database. The authority is the source of
//! the role: an admin row is permitted, an analyst row is forbidden, and a
//! session whose principal no longer resolves is rejected.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::sync::Arc;

use apex_api::auth::{ApiKey, ApiRole};
use apex_api::middleware::auth::require_admin;
use apex_api::middleware::session::{
    create_session_token, require_api_auth, ApiAuthState, SessionAuthority, SessionAuthorityError,
    SessionClaims, WebSession,
};
use apex_core::identity::{UserId, Username};
use async_trait::async_trait;
use axum::body::Body;
use axum::http::{header, HeaderValue, Request, StatusCode};
use axum::middleware;
use axum::routing::{get, post};
use axum::Router;
use hmac::{Hmac, Mac};
use http_body_util::BodyExt;
use sha2::Sha256;
use tower::ServiceExt;
use uuid::Uuid;

const SESSION_SECRET: &str = "session-admin-auth-test-secret";

fn signed_token(username: &str) -> (String, Uuid) {
    let jti = Uuid::new_v4();
    let claims = SessionClaims {
        user_id: UserId::new(username),
        username: Username::new(username),
        role: ApiRole::Analyst,
        issued_at: chrono::Utc::now().timestamp_millis(),
        expires_at: i64::MAX,
        session_version: 1,
        session_id: jti,
    };
    (
        create_session_token(&claims, SESSION_SECRET).expect("session token"),
        jti,
    )
}

/// Independently derive the session-bound CSRF token the way the server does:
/// `HMAC-SHA256(secret, "csrf:" + session_signature)`.
fn derived_csrf(token: &str) -> String {
    let signature = token.split_once('.').expect("token has a signature").1;
    let mut mac = Hmac::<Sha256>::new_from_slice(SESSION_SECRET.as_bytes()).expect("hmac key");
    mac.update(b"csrf:");
    mac.update(signature.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

struct TestAuthority {
    roles: HashMap<String, ApiRole>,
}

#[async_trait]
impl SessionAuthority for TestAuthority {
    async fn authorize(&self, session: &WebSession) -> Result<WebSession, SessionAuthorityError> {
        let role = self
            .roles
            .get(session.user_id.as_str())
            .cloned()
            .ok_or(SessionAuthorityError::UnknownUser)?;
        Ok(WebSession {
            role,
            ..session.clone()
        })
    }
}

fn auth_state() -> ApiAuthState {
    let mut roles = HashMap::new();
    roles.insert("admin".to_string(), ApiRole::Admin);
    roles.insert("analyst-user".to_string(), ApiRole::Analyst);
    ApiAuthState {
        api_keys: Arc::new(HashMap::<String, ApiKey>::new()),
        session_authority: Some(Arc::new(TestAuthority { roles })),
    }
}

fn admin_surface_router() -> Router {
    Router::new()
        .route("/api/admin/probe", get(|| async { "admin-ok" }))
        .route("/api/admin/probe", post(|| async { "admin-post-ok" }))
        .route_layer(middleware::from_fn(require_admin))
        .route_layer(middleware::from_fn_with_state(
            auth_state(),
            require_api_auth,
        ))
}

fn session_cookie_header(username: &str) -> (String, HeaderValue) {
    let (token, _) = signed_token(username);
    let cookie = HeaderValue::from_str(&format!("apex_session={token}")).expect("cookie header");
    (token, cookie)
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

fn set_session_secret() {
    std::env::set_var("SESSION_SECRET", SESSION_SECRET);
}

#[tokio::test]
async fn admin_session_is_permitted_on_admin_api() {
    set_session_secret();
    let (_, cookie) = session_cookie_header("admin");
    let (status, body) = status_of(
        admin_surface_router(),
        request("GET", "/api/admin/probe", Some(cookie)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "admin session should be permitted");
    assert_eq!(body, "admin-ok");
}

#[tokio::test]
async fn analyst_session_is_forbidden_on_admin_api() {
    set_session_secret();
    let (_, cookie) = session_cookie_header("analyst-user");
    let (status, _) = status_of(
        admin_surface_router(),
        request("GET", "/api/admin/probe", Some(cookie)),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "analyst session must not reach the admin surface"
    );
}

#[tokio::test]
async fn session_without_a_canonical_row_is_unauthorized() {
    set_session_secret();
    let (_, cookie) = session_cookie_header("ghost-user");
    let (status, _) = status_of(
        admin_surface_router(),
        request("GET", "/api/admin/probe", Some(cookie)),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "the authority is the source of truth; a signed cookie alone is not enough"
    );
}

#[tokio::test]
async fn missing_session_is_unauthorized_on_admin_api() {
    set_session_secret();
    let (status, _) = status_of(
        admin_surface_router(),
        request("GET", "/api/admin/probe", None),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn invalid_bearer_key_is_not_downgraded_to_the_session() {
    set_session_secret();
    let (_, cookie) = session_cookie_header("admin");
    let mut request = request("GET", "/api/admin/probe", Some(cookie));
    request.headers_mut().insert(
        header::AUTHORIZATION,
        HeaderValue::from_static("Bearer not-a-real-key"),
    );

    let (status, _) = status_of(admin_surface_router(), request).await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "a presented-but-invalid key must fail instead of falling back to the session"
    );
}

#[tokio::test]
async fn admin_session_unsafe_method_requires_the_session_bound_csrf_token() {
    set_session_secret();
    let (token, _) = signed_token("admin");

    // Without any CSRF token, an unsafe method is rejected.
    let (_, cookie) = session_cookie_header("admin");
    let (status, _) = status_of(
        admin_surface_router(),
        request("POST", "/api/admin/probe", Some(cookie)),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // A planted double-submit pair (attacker-chosen cookie + header) is not the
    // derived token and is rejected.
    let planted = HeaderValue::from_str(&format!("apex_session={token}; apex_csrf=attacker-token"))
        .expect("cookie");
    let mut planted_request = request("POST", "/api/admin/probe", Some(planted));
    planted_request
        .headers_mut()
        .insert("x-csrf-token", HeaderValue::from_static("attacker-token"));
    let (status, _) = status_of(admin_surface_router(), planted_request).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // The derived cookie + header pair is accepted.
    let csrf = derived_csrf(&token);
    let cookie =
        HeaderValue::from_str(&format!("apex_session={token}; apex_csrf={csrf}")).expect("cookie");
    let mut request = request("POST", "/api/admin/probe", Some(cookie));
    request
        .headers_mut()
        .insert("x-csrf-token", HeaderValue::from_str(&csrf).expect("csrf"));

    let (status, body) = status_of(admin_surface_router(), request).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "admin-post-ok");
}
