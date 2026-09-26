#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;

use apex_api::auth::{hash_api_key, ApiKey, ApiRole};
use apex_api::middleware::auth::authenticate_api_request;
use axum::http::{header, HeaderMap, HeaderValue, Method};
use chrono::Utc;

fn api_keys() -> HashMap<String, ApiKey> {
    let now = Utc::now();
    HashMap::from([
        (
            "admin".to_string(),
            ApiKey {
                key_id: "admin-key".to_string(),
                owner_user_id: "user-admin".into(),
                key_hash: hash_api_key("admin-secret"),
                name: "Admin".to_string(),
                role: ApiRole::Admin,
                created_at: now,
                expires_at: None,
                enabled: true,
                rate_limit_per_min: 120,
                allowed_origins: vec!["https://allowed.test".to_string()],
            },
        ),
        (
            "viewer".to_string(),
            ApiKey {
                key_id: "viewer-key".to_string(),
                owner_user_id: "user-viewer".into(),
                key_hash: hash_api_key("viewer-secret"),
                name: "Viewer".to_string(),
                role: ApiRole::Viewer,
                created_at: now,
                expires_at: None,
                enabled: true,
                rate_limit_per_min: 60,
                allowed_origins: vec![],
            },
        ),
    ])
}

#[test]
fn auth_middleware_preserves_current_status_codes() {
    let keys = api_keys();
    let missing = authenticate_api_request(&HeaderMap::new(), &Method::GET, &keys, Utc::now())
        .expect_err("missing auth should fail");
    assert_eq!(missing.http_status(), 401);

    let mut viewer_headers = HeaderMap::new();
    viewer_headers.insert(
        header::AUTHORIZATION,
        HeaderValue::from_static("Bearer viewer-secret"),
    );
    let forbidden = authenticate_api_request(&viewer_headers, &Method::POST, &keys, Utc::now())
        .expect_err("viewer write should fail");
    assert_eq!(forbidden.http_status(), 403);
}

#[test]
fn principal_extraction_is_available_to_handlers_after_extraction() {
    let keys = api_keys();
    let mut headers = HeaderMap::new();
    headers.insert(
        header::AUTHORIZATION,
        HeaderValue::from_static("Bearer admin-secret"),
    );
    headers.insert(
        header::ORIGIN,
        HeaderValue::from_static("https://allowed.test"),
    );

    let authenticated = authenticate_api_request(&headers, &Method::GET, &keys, Utc::now())
        .expect("admin auth should pass");
    assert_eq!(authenticated.auth_context.key_id, "admin-key");
    assert_eq!(authenticated.auth_context.user_id, "user-admin");
}
