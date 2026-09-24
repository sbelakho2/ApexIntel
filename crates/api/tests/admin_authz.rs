//! P0 principal/authz integration coverage.
//!
//! Exercises the production authorization middleware
//! (`require_api_auth` + `require_admin`, `require_session` +
//! `require_web_admin`) over real axum routers, plus the login handler with
//! Argon2id and legacy SHA-256 password hashes.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::sync::Arc;

use apex_api::auth::{hash_api_key, ApiKey, ApiRole};
use apex_api::middleware::session::{
    create_session_token, require_admin, require_api_auth, require_session, require_web_admin,
    session_cookie_name, validate_session, ApiAuthState, SessionClaims, SESSION_TTL_MS,
    SESSION_VERSION,
};
use axum::body::Body;
use axum::http::{header, HeaderMap, HeaderValue, Request, StatusCode};
use axum::middleware;
use axum::routing::{get, post};
use axum::Router;
use chrono::Utc;
use http_body_util::BodyExt;
use tower::ServiceExt;

const TEST_SECRET: &str = "integration-test-session-secret";

fn ensure_session_secret() {
    std::env::set_var("SESSION_SECRET", TEST_SECRET);
}

fn test_api_key(key_id: &str, user_id: &str, raw_key: &str, role: ApiRole) -> ApiKey {
    ApiKey {
        key_id: key_id.to_string(),
        owner_user_id: user_id.to_string(),
        key_hash: hash_api_key(raw_key),
        name: key_id.to_string(),
        role,
        created_at: Utc::now(),
        expires_at: None,
        enabled: true,
        rate_limit_per_min: 120,
        allowed_origins: vec![],
    }
}

fn test_api_keys() -> HashMap<String, ApiKey> {
    HashMap::from([
        (
            "admin".to_string(),
            test_api_key("admin-key", "usr-admin", "admin-secret", ApiRole::Admin),
        ),
        (
            "analyst".to_string(),
            test_api_key(
                "analyst-key",
                "usr-analyst",
                "analyst-secret",
                ApiRole::Analyst,
            ),
        ),
    ])
}

fn session_cookie(role: ApiRole, user_id: &str, username: &str) -> String {
    let now = Utc::now().timestamp_millis();
    let claims = SessionClaims {
        user_id: user_id.to_string(),
        username: username.to_string(),
        role,
        issued_at: now,
        expires_at: now + SESSION_TTL_MS,
        session_version: SESSION_VERSION,
    };
    let token = create_session_token(&claims, TEST_SECRET).expect("sign session token");
    format!("{}={token}", session_cookie_name())
}

fn api_request(path: &str, cookie: Option<&str>, bearer: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder().uri(path);
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    if let Some(bearer) = bearer {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {bearer}"));
    }
    builder.body(Body::empty()).expect("request")
}

async fn ok_handler() -> StatusCode {
    StatusCode::OK
}

/// The `/api/admin/*` surface as wired in production: `require_admin` scoped to
/// the admin routes, `require_api_auth` wrapping everything.
fn admin_api_router() -> Router {
    let admin_routes = Router::new()
        .route("/api/admin/crawl-status", get(ok_handler))
        .route_layer(middleware::from_fn(require_admin));

    Router::new()
        .route("/api/ping", get(ok_handler))
        .merge(admin_routes)
        .route_layer(middleware::from_fn_with_state(
            ApiAuthState {
                api_keys: Arc::new(test_api_keys()),
            },
            require_api_auth,
        ))
}

/// The HTML `/admin` surface as wired in production: `require_web_admin`
/// scoped to the admin page, `require_session` wrapping the web router.
fn admin_page_router() -> Router {
    Router::new()
        .route("/admin", get(ok_handler))
        .route_layer(middleware::from_fn(require_web_admin))
        .route_layer(middleware::from_fn(require_session))
}

fn login_router() -> Router {
    Router::new().route("/login", post(apex_api::web::auth::login_submit))
}

fn login_request(username: &str, password: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/login")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(format!(
            "username={username}&password={password}"
        )))
        .expect("login request")
}

async fn body_text(response: axum::response::Response) -> String {
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    String::from_utf8_lossy(&bytes).to_string()
}

#[tokio::test]
async fn admin_session_can_call_admin_api() {
    ensure_session_secret();
    let response = admin_api_router()
        .oneshot(api_request(
            "/api/admin/crawl-status",
            Some(&session_cookie(ApiRole::Admin, "usr-admin", "admin")),
            None,
        ))
        .await
        .expect("request");
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn analyst_key_gets_403_on_admin_api() {
    ensure_session_secret();
    let response = admin_api_router()
        .oneshot(api_request(
            "/api/admin/crawl-status",
            None,
            Some("analyst-secret"),
        ))
        .await
        .expect("request");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn analyst_session_gets_403_on_admin_api() {
    ensure_session_secret();
    let response = admin_api_router()
        .oneshot(api_request(
            "/api/admin/crawl-status",
            Some(&session_cookie(ApiRole::Analyst, "usr-analyst", "analyst")),
            None,
        ))
        .await
        .expect("request");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn admin_key_can_call_admin_api() {
    ensure_session_secret();
    let response = admin_api_router()
        .oneshot(api_request(
            "/api/admin/crawl-status",
            None,
            Some("admin-secret"),
        ))
        .await
        .expect("request");
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn admin_page_is_200_for_admin_session() {
    ensure_session_secret();
    let response = admin_page_router()
        .oneshot(api_request(
            "/admin",
            Some(&session_cookie(ApiRole::Admin, "usr-admin", "admin")),
            None,
        ))
        .await
        .expect("request");
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn admin_page_is_403_for_analyst_session() {
    ensure_session_secret();
    let response = admin_page_router()
        .oneshot(api_request(
            "/admin",
            Some(&session_cookie(ApiRole::Analyst, "usr-analyst", "analyst")),
            None,
        ))
        .await
        .expect("request");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn admin_page_redirects_to_login_without_session() {
    ensure_session_secret();
    let response = admin_page_router()
        .oneshot(api_request("/admin", None, None))
        .await
        .expect("request");
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        response
            .headers()
            .get(header::LOCATION)
            .and_then(|value| value.to_str().ok()),
        Some("/login")
    );
}

#[tokio::test]
async fn login_accepts_argon2id_and_legacy_sha256_hashes() {
    ensure_session_secret();

    let argon2_hash = apex_api::web::auth::hash_password("s3cret").expect("hash password");
    // Legacy deployments stored the plain SHA-256 hex digest of the password;
    // `hash_api_key` is the same digest.
    let legacy_hash = hash_api_key("legacy-pass");
    let users = serde_json::json!([
        {
            "id": "usr-admin",
            "username": "admin",
            "password_hash": argon2_hash,
            "role": "admin",
        },
        {
            "id": "usr-legacy",
            "username": "legacy",
            "password_hash": legacy_hash,
            "role": "analyst",
        },
    ]);
    std::env::set_var("WEB_USERS_JSON", users.to_string());

    let response = login_router()
        .oneshot(login_request("admin", "s3cret"))
        .await
        .expect("login");
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let set_cookie = response
        .headers()
        .get(header::SET_COOKIE)
        .and_then(|value| value.to_str().ok())
        .expect("session cookie")
        .to_string();
    assert!(set_cookie.starts_with(&format!("{}=", session_cookie_name())));
    assert!(set_cookie.contains("Path=/"));
    assert!(set_cookie.contains("HttpOnly"));
    assert!(set_cookie.contains("SameSite=Lax"));

    let response = login_router()
        .oneshot(login_request("legacy", "legacy-pass"))
        .await
        .expect("legacy login");
    assert_eq!(response.status(), StatusCode::SEE_OTHER);

    let response = login_router()
        .oneshot(login_request("admin", "wrong-password"))
        .await
        .expect("wrong password login");
    assert_eq!(response.status(), StatusCode::OK);
    assert!(body_text(response).await.contains("Invalid credentials"));
}

#[test]
fn session_payload_round_trips_role_admin() {
    ensure_session_secret();
    let cookie = session_cookie(ApiRole::Admin, "usr-admin", "admin");
    let (name, value) = cookie.split_once('=').expect("cookie pair");
    let mut headers = HeaderMap::new();
    headers.insert(
        header::COOKIE,
        HeaderValue::from_str(&format!("{name}={value}")).expect("cookie header"),
    );

    let session = validate_session(&headers, TEST_SECRET).expect("session validates");

    assert_eq!(session.role, ApiRole::Admin);
    assert!(session.can_admin());
    assert_eq!(session.user_id, "usr-admin");
    assert_eq!(session.username, "admin");
    assert_eq!(session.session_version, SESSION_VERSION);
}
