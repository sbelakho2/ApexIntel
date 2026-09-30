//! Database-backed session authority contract (audit P0-2).
//!
//! `#[ignore]`d by default; CI runs it against a PostgreSQL service through
//! `scripts/ci/run_pg_integration_suites.sh`. Proves that a signed browser
//! cookie is only the *selection* of a principal and that the canonical
//! `app_users` row is the authority:
//!
//!   * disabled rows are rejected immediately;
//!   * a bumped `session_version` revokes outstanding cookies;
//!   * a role downgrade (Analyst cookie, Viewer row) loses write access on the
//!     very next request and a promotion takes effect just as fast;
//!   * legacy cookies without `uid`/`role`/`sv` (or with `sv = 0`) are
//!     rejected instead of being upgraded to a default role;
//!   * the `/api/*` browser-session fallback resolves the same database role.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::sync::Arc;

use apex_api::auth::ApiRole;
use apex_api::middleware::session::{
    create_session_token, require_admin, require_api_auth, require_session, require_web_admin,
    require_web_write, session_cookie_name, ApiAuthState, SessionAuthority, SessionClaims,
    SESSION_TTL_MS, SESSION_VERSION,
};
use apex_store::postgres::PgStore;
use axum::body::Body;
use axum::http::{header, HeaderValue, Request, StatusCode};
use axum::middleware;
use axum::routing::{get, post};
use axum::{Extension, Router};
use chrono::Utc;
use sqlx::postgres::{PgPool, PgPoolOptions};
use tower::ServiceExt;
use uuid::Uuid;

const TEST_SECRET: &str = "session-authority-integration-secret";

fn database_url() -> String {
    std::env::var("TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("TEST_DATABASE_URL or DATABASE_URL must be set")
}

fn ensure_session_secret() {
    std::env::set_var("SESSION_SECRET", TEST_SECRET);
}

async fn setup() -> PgPool {
    let pool = PgPoolOptions::new()
        .max_connections(3)
        .connect(&database_url())
        .await
        .expect("connect to postgres");
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("run migrations");
    pool
}

async fn insert_user(
    pool: &PgPool,
    id: &str,
    username: &str,
    role: &str,
    enabled: bool,
    session_version: i32,
) {
    sqlx::query(
        "INSERT INTO app_users (id, username, display_name, role, enabled, session_version) \
         VALUES ($1, $2, $2, $3, $4, $5)",
    )
    .bind(id)
    .bind(username)
    .bind(role)
    .bind(enabled)
    .bind(session_version)
    .execute(pool)
    .await
    .expect("insert app_users row");
}

async fn cleanup(pool: &PgPool, ids: &[String]) {
    for id in ids {
        sqlx::query("DELETE FROM app_users WHERE id = $1")
            .bind(id)
            .execute(pool)
            .await
            .expect("cleanup app_users row");
    }
}

fn session_cookie_with_jti(user_id: &str, role: ApiRole, session_version: u32) -> (String, Uuid) {
    let now = Utc::now().timestamp_millis();
    let session_id = Uuid::new_v4();
    let claims = SessionClaims {
        user_id: user_id.into(),
        username: user_id.into(),
        role,
        issued_at: now,
        expires_at: now + SESSION_TTL_MS,
        session_version,
        session_id,
    };
    let token = create_session_token(&claims, TEST_SECRET).expect("sign session token");
    (format!("{}={token}", session_cookie_name()), session_id)
}

fn session_cookie(user_id: &str, role: ApiRole, session_version: u32) -> String {
    session_cookie_with_jti(user_id, role, session_version).0
}

/// Independently derive the session-bound CSRF token the way the server does.
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

fn legacy_cookie(user_id: &str) -> String {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    let payload = serde_json::json!({
        "uid": user_id,
        "sub": user_id,
        "role": "analyst",
        "iat": Utc::now().timestamp_millis(),
        "sv": 0,
    });
    let payload_bytes = serde_json::to_vec(&payload).unwrap();
    let payload_b64 = URL_SAFE_NO_PAD.encode(&payload_bytes);
    let mut mac = Hmac::<Sha256>::new_from_slice(TEST_SECRET.as_bytes()).unwrap();
    mac.update(&payload_bytes);
    let sig = hex::encode(mac.finalize().into_bytes());
    format!("apex_session={payload_b64}.{sig}")
}

fn get_request(path: &str, cookie: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder().method("GET").uri(path);
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    builder.body(Body::empty()).expect("request")
}

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

/// The production browser split with the real `PgStore` authority wired in:
/// reads behind `require_session`, mutations behind `require_web_write`, admin
/// behind `require_web_admin`.
fn authority_router(store: &Arc<PgStore>) -> Router {
    let authority: Arc<dyn SessionAuthority> = store.clone();
    let web_read_pages = Router::new()
        .route("/page", get(ok_handler))
        .route_layer(middleware::from_fn(require_session));
    let web_write_pages = Router::new()
        .route("/mutation", post(ok_handler))
        .route_layer(middleware::from_fn(require_web_write))
        .route_layer(middleware::from_fn(require_session));
    let admin_pages = Router::new()
        .route("/admin", get(ok_handler))
        .route_layer(middleware::from_fn(require_web_admin))
        .route_layer(middleware::from_fn(require_session));

    web_read_pages
        .merge(web_write_pages)
        .merge(admin_pages)
        .layer(Extension(authority))
        .layer(Extension(store.clone()))
}

async fn status(router: Router, request: Request<Body>) -> StatusCode {
    router.oneshot(request).await.expect("request").status()
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn browser_matrix_through_the_database_authority() {
    ensure_session_secret();
    let pool = setup().await;
    let store = Arc::new(PgStore::from_pool(pool.clone()));
    let marker = format!("p0-authz-{}", Uuid::new_v4());

    let viewer = format!("{marker}-viewer");
    let analyst = format!("{marker}-analyst");
    let admin = format!("{marker}-admin");
    let service = format!("{marker}-service");
    for (id, role) in [
        (&viewer, "viewer"),
        (&analyst, "analyst"),
        (&admin, "admin"),
        (&service, "service"),
    ] {
        insert_user(&pool, id, id, role, true, SESSION_VERSION as i32).await;
    }

    let router = authority_router(&store);

    // Viewer: read yes, mutation no, admin no.
    let cookie = session_cookie(&viewer, ApiRole::Viewer, SESSION_VERSION);
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

    // Analyst: read + mutation yes, admin no.
    let cookie = session_cookie(&analyst, ApiRole::Analyst, SESSION_VERSION);
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
        StatusCode::FORBIDDEN
    );

    // Admin: everything yes.
    let cookie = session_cookie(&admin, ApiRole::Admin, SESSION_VERSION);
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

    // Service: read yes, no browser mutations, no admin.
    let cookie = session_cookie(&service, ApiRole::Service, SESSION_VERSION);
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

    cleanup(
        &pool,
        &[
            viewer.clone(),
            analyst.clone(),
            admin.clone(),
            service.clone(),
        ],
    )
    .await;
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn disabled_user_is_rejected_immediately() {
    ensure_session_secret();
    let pool = setup().await;
    let store = Arc::new(PgStore::from_pool(pool.clone()));
    let marker = format!("p0-authz-disabled-{}", Uuid::new_v4());
    let user = format!("{marker}-user");

    insert_user(
        &pool,
        &user,
        &user,
        "analyst",
        false,
        SESSION_VERSION as i32,
    )
    .await;

    let router = authority_router(&store);
    let cookie = session_cookie(&user, ApiRole::Analyst, SESSION_VERSION);
    let response = router
        .oneshot(get_request("/page", Some(&cookie)))
        .await
        .expect("request");
    assert_eq!(
        response.status(),
        StatusCode::SEE_OTHER,
        "a disabled account must not keep a live session"
    );
    let location = response
        .headers()
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    assert!(
        location.starts_with("/login"),
        "a disabled account is redirected to login, got {location}"
    );

    cleanup(&pool, std::slice::from_ref(&user)).await;
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn stale_session_version_is_rejected_immediately() {
    ensure_session_secret();
    let pool = setup().await;
    let store = Arc::new(PgStore::from_pool(pool.clone()));
    let marker = format!("p0-authz-stale-{}", Uuid::new_v4());
    let user = format!("{marker}-user");

    // The cookie carries version 1; the row has been bumped to 2.
    insert_user(
        &pool,
        &user,
        &user,
        "analyst",
        true,
        SESSION_VERSION as i32 + 1,
    )
    .await;

    let router = authority_router(&store);
    let cookie = session_cookie(&user, ApiRole::Analyst, SESSION_VERSION);
    let response = router
        .oneshot(get_request("/page", Some(&cookie)))
        .await
        .expect("request");
    assert_eq!(
        response.status(),
        StatusCode::SEE_OTHER,
        "a cookie whose sv no longer matches the row must be revoked"
    );

    cleanup(&pool, std::slice::from_ref(&user)).await;
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn role_downgrade_loses_write_on_the_next_request() {
    ensure_session_secret();
    let pool = setup().await;
    let store = Arc::new(PgStore::from_pool(pool.clone()));
    let marker = format!("p0-authz-downgrade-{}", Uuid::new_v4());
    let user = format!("{marker}-user");

    // The cookie was minted while the account was an Analyst; the row is now a
    // Viewer (same session version, so the cookie is not stale).
    insert_user(&pool, &user, &user, "viewer", true, SESSION_VERSION as i32).await;

    let router = authority_router(&store);
    let cookie = session_cookie(&user, ApiRole::Analyst, SESSION_VERSION);
    assert_eq!(
        status(router.clone(), get_request("/page", Some(&cookie))).await,
        StatusCode::OK,
        "the session stays valid for reads"
    );
    assert_eq!(
        status(router.clone(), post_request("/mutation", Some(&cookie))).await,
        StatusCode::FORBIDDEN,
        "the database role (Viewer) overrides the signed Analyst role immediately"
    );
    assert_eq!(
        status(router, get_request("/admin", Some(&cookie))).await,
        StatusCode::FORBIDDEN
    );

    cleanup(&pool, std::slice::from_ref(&user)).await;
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn role_promotion_grants_admin_on_the_next_request() {
    ensure_session_secret();
    let pool = setup().await;
    let store = Arc::new(PgStore::from_pool(pool.clone()));
    let marker = format!("p0-authz-promotion-{}", Uuid::new_v4());
    let user = format!("{marker}-user");

    // The cookie says Viewer; the row is now Admin.
    insert_user(&pool, &user, &user, "admin", true, SESSION_VERSION as i32).await;

    let router = authority_router(&store);
    let cookie = session_cookie(&user, ApiRole::Viewer, SESSION_VERSION);
    assert_eq!(
        status(router, get_request("/admin", Some(&cookie))).await,
        StatusCode::OK,
        "the database role (Admin) takes effect without re-login"
    );

    cleanup(&pool, std::slice::from_ref(&user)).await;
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn legacy_cookie_and_unknown_user_are_rejected() {
    ensure_session_secret();
    let pool = setup().await;
    let store = Arc::new(PgStore::from_pool(pool.clone()));
    let marker = format!("p0-authz-legacy-{}", Uuid::new_v4());
    let user = format!("{marker}-user");

    insert_user(&pool, &user, &user, "analyst", true, SESSION_VERSION as i32).await;

    let router = authority_router(&store);

    // sv = 0 legacy payload: rejected before the database is even consulted.
    let response = router
        .clone()
        .oneshot(get_request("/page", Some(&legacy_cookie(&user))))
        .await
        .expect("request");
    assert_eq!(response.status(), StatusCode::SEE_OTHER);

    // A signed cookie for a uid with no app_users row fails closed too.
    let ghost = format!("{marker}-ghost");
    let cookie = session_cookie(&ghost, ApiRole::Admin, SESSION_VERSION);
    let response = router
        .oneshot(get_request("/page", Some(&cookie)))
        .await
        .expect("request");
    assert_eq!(response.status(), StatusCode::SEE_OTHER);

    cleanup(&pool, std::slice::from_ref(&user)).await;
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn api_session_fallback_uses_the_database_role() {
    ensure_session_secret();
    let pool = setup().await;
    let store = Arc::new(PgStore::from_pool(pool.clone()));
    let marker = format!("p0-authz-api-{}", Uuid::new_v4());
    let demoted = format!("{marker}-demoted");
    let promoted = format!("{marker}-promoted");

    insert_user(
        &pool,
        &demoted,
        &demoted,
        "viewer",
        true,
        SESSION_VERSION as i32,
    )
    .await;
    insert_user(
        &pool,
        &promoted,
        &promoted,
        "admin",
        true,
        SESSION_VERSION as i32,
    )
    .await;

    let authority: Arc<dyn SessionAuthority> = store.clone();
    let router = Router::new()
        .route("/api/admin/probe", get(ok_handler))
        .route_layer(middleware::from_fn(require_admin))
        .route_layer(middleware::from_fn_with_state(
            ApiAuthState {
                api_keys: Arc::new(HashMap::new()),
                session_authority: Some(authority),
            },
            require_api_auth,
        ));

    // The cookie still claims Admin, but the row was downgraded to Viewer.
    let cookie = session_cookie(&demoted, ApiRole::Admin, SESSION_VERSION);
    assert_eq!(
        status(
            router.clone(),
            get_request("/api/admin/probe", Some(&cookie))
        )
        .await,
        StatusCode::FORBIDDEN,
        "the API fallback must not honor a signed role that the database revoked"
    );

    // The cookie only says Viewer, but the row was promoted to Admin.
    let cookie = session_cookie(&promoted, ApiRole::Viewer, SESSION_VERSION);
    assert_eq!(
        status(
            router.clone(),
            get_request("/api/admin/probe", Some(&cookie))
        )
        .await,
        StatusCode::OK,
        "the API fallback must honor the canonical database role"
    );

    // A disabled account cannot use the API fallback at all.
    let disabled = format!("{marker}-disabled");
    insert_user(
        &pool,
        &disabled,
        &disabled,
        "admin",
        false,
        SESSION_VERSION as i32,
    )
    .await;
    let cookie = session_cookie(&disabled, ApiRole::Admin, SESSION_VERSION);
    assert_eq!(
        status(router, get_request("/api/admin/probe", Some(&cookie))).await,
        StatusCode::UNAUTHORIZED
    );

    cleanup(
        &pool,
        &[demoted.clone(), promoted.clone(), disabled.clone()],
    )
    .await;
    pool.close().await;
}

/// Logout revocation: the `jti` recorded in `revoked_sessions` invalidates a
/// copied cookie on the very next request, independent of `session_version`.
#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn revoked_session_is_rejected_immediately() {
    ensure_session_secret();
    let pool = setup().await;
    let store = Arc::new(PgStore::from_pool(pool.clone()));
    let user_id = format!("usr-revoke-{}", Uuid::new_v4().simple());
    insert_user(
        &pool,
        &user_id,
        &user_id,
        "analyst",
        true,
        SESSION_VERSION as i32,
    )
    .await;

    let (cookie, jti) = session_cookie_with_jti(&user_id, ApiRole::Analyst, SESSION_VERSION);
    let router = authority_router(&store);

    assert_eq!(
        status(router.clone(), get_request("/page", Some(&cookie))).await,
        StatusCode::OK,
        "the session is valid before revocation"
    );

    // Logout records the session id.
    store
        .revoke_session(jti, Utc::now() + chrono::Duration::hours(1))
        .await
        .expect("record revocation");

    assert_eq!(
        status(router.clone(), get_request("/page", Some(&cookie))).await,
        StatusCode::SEE_OTHER,
        "a revoked session cookie must be rejected on the next request"
    );

    // A different session for the same principal is unaffected.
    let (other_cookie, other_jti) =
        session_cookie_with_jti(&user_id, ApiRole::Analyst, SESSION_VERSION);
    assert_eq!(
        status(router, get_request("/page", Some(&other_cookie))).await,
        StatusCode::OK,
        "revocation is per session id, not per user"
    );

    sqlx::query("DELETE FROM revoked_sessions WHERE jti = $1 OR jti = $2")
        .bind(jti)
        .bind(other_jti)
        .execute(&pool)
        .await
        .expect("cleanup revocations");
    cleanup(&pool, &[user_id]).await;
    pool.close().await;
}
