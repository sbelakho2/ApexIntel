//! Route-level tests for the durable login throttle wiring in
//! `login_submit`: evaluate before credential verification, record failures
//! after, and answer 429 without checking the password once locked.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use apex_api::auth::client_fingerprint;
use apex_api::login_throttle::LoginThrottle;
use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::routing::post;
use axum::{Extension, Router};
use http_body_util::BodyExt;
use tower::ServiceExt;

const SECRET: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn login_router(throttle: Arc<LoginThrottle>) -> Router {
    Router::new()
        .route("/login", post(apex_api::web::auth::login_submit))
        .layer(Extension(throttle))
}

fn login_request(password: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/login")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(format!("username=admin&password={password}")))
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
async fn login_submit_enforces_the_throttle_before_verifying_credentials() {
    std::env::set_var("SESSION_SECRET", SECRET);
    let password_hash =
        apex_api::web::auth::hash_password("correct-password").expect("hash password");
    std::env::set_var(
        "WEB_USERS_JSON",
        serde_json::json!([{
            "id": "usr-admin",
            "username": "admin",
            "password_hash": password_hash,
            "role": "admin",
        }])
        .to_string(),
    );

    // The route sees no ConnectInfo/User-Agent in tests, matching this key.
    let attempt_key = format!("admin|{}", client_fingerprint(None, None));
    let now = chrono::Utc::now();

    // ── Locked key: even the correct password is rejected with 429. ────────
    let locked_throttle = Arc::new(LoginThrottle::in_memory());
    for offset in 0..10 {
        locked_throttle
            .record_failure(
                &attempt_key,
                now - chrono::Duration::seconds(1) + chrono::Duration::milliseconds(offset * 10),
            )
            .await;
    }
    let response = login_router(locked_throttle)
        .oneshot(login_request("correct-password"))
        .await
        .expect("login response");
    assert_eq!(
        response.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "a locked key must be rejected before the password is verified"
    );
    assert!(response.headers().get(header::RETRY_AFTER).is_some());
    assert!(body_text(response)
        .await
        .contains("Too many failed sign-in attempts"));

    // ── Failures recorded through the route lock the key on the 10th. ──────
    let failing_throttle = Arc::new(LoginThrottle::in_memory());
    // Nine earlier failures whose progressive backoff has already elapsed.
    for offset in 0..9 {
        failing_throttle
            .record_failure(
                &attempt_key,
                now - chrono::Duration::minutes(2) + chrono::Duration::seconds(offset),
            )
            .await;
    }
    let locked_response = login_router(failing_throttle.clone())
        .oneshot(login_request("wrong-password"))
        .await
        .expect("login response");
    assert_eq!(locked_response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(locked_response.headers().get(header::RETRY_AFTER).is_some());

    // The lock is keyed on the normalised username + fingerprint: a
    // different password (even the right one) stays rejected.
    let still_locked = login_router(failing_throttle)
        .oneshot(login_request("correct-password"))
        .await
        .expect("login response");
    assert_eq!(still_locked.status(), StatusCode::TOO_MANY_REQUESTS);

    std::env::remove_var("WEB_USERS_JSON");
}
