#![allow(clippy::disallowed_methods)]

use std::collections::HashMap;

use apex_api::auth::{hash_api_key, ApiKey, ApiRole};
use apex_api::middleware::auth::{
    extract_websocket_token, validate_websocket_token, WebSocketAuthOptions,
};
use axum::http::{header, HeaderMap, HeaderValue};
use chrono::Utc;

fn api_keys() -> HashMap<String, ApiKey> {
    let now = Utc::now();
    HashMap::from([(
        "admin".to_string(),
        ApiKey {
            key_id: "admin-key".to_string(),
            owner_user_id: "user-admin".to_string(),
            key_hash: hash_api_key("admin-secret"),
            name: "Admin".to_string(),
            role: ApiRole::Admin,
            created_at: now,
            expires_at: None,
            enabled: true,
            rate_limit_per_min: 120,
            allowed_origins: vec![],
        },
    )])
}

#[test]
fn warnings_ws_rejects_missing_auth_token() {
    let err = extract_websocket_token(
        &HeaderMap::new(),
        None,
        WebSocketAuthOptions {
            allow_subprotocol_fallback: false,
        },
    )
    .expect_err("missing websocket auth should fail");
    assert_eq!(err.http_status(), 401);
}

#[test]
fn warnings_ws_accepts_supported_auth_transport() {
    let token = extract_websocket_token(
        &HeaderMap::new(),
        Some("query-token"),
        WebSocketAuthOptions {
            allow_subprotocol_fallback: false,
        },
    )
    .expect("query token should pass");
    assert_eq!(token, "query-token");

    let mut headers = HeaderMap::new();
    headers.insert(
        header::AUTHORIZATION,
        HeaderValue::from_static("Bearer admin-secret"),
    );
    let auth_header_token =
        extract_websocket_token(&headers, None, WebSocketAuthOptions::default())
            .expect("authorization header should pass");
    let auth_context = validate_websocket_token(&auth_header_token, &api_keys(), Utc::now())
        .expect("known token should validate");
    assert_eq!(auth_context.key_id, "admin-key");
}

#[test]
fn warnings_ws_rejects_invalid_subprotocol_token_when_compat_disabled() {
    let mut headers = HeaderMap::new();
    headers.insert(
        "sec-websocket-protocol",
        HeaderValue::from_static("legacy-token"),
    );
    let err = extract_websocket_token(
        &headers,
        None,
        WebSocketAuthOptions {
            allow_subprotocol_fallback: false,
        },
    )
    .expect_err("subprotocol fallback should be disabled");
    assert_eq!(err.http_status(), 401);
}
