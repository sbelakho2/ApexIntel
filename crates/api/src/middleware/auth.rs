use std::collections::HashMap;

use axum::http::{header, HeaderMap, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};

use crate::auth::{self, ApiKey, AuthResult, PermissionLevel};
use crate::destructive_actions::ApiAuthContext;
use crate::responses::{error_response, ApiError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticatedRequest {
    pub auth_context: ApiAuthContext,
    pub rate_limit_per_min: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WebSocketAuthOptions {
    pub allow_subprotocol_fallback: bool,
}

impl Default for WebSocketAuthOptions {
    fn default() -> Self {
        Self {
            allow_subprotocol_fallback: websocket_subprotocol_compat_enabled(),
        }
    }
}

pub fn authenticate_api_request(
    headers: &HeaderMap,
    method: &Method,
    api_keys: &HashMap<String, ApiKey>,
    now: DateTime<Utc>,
) -> Result<AuthenticatedRequest, ApiError> {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(auth::extract_bearer_token)
        .ok_or_else(ApiError::unauthorized)?;

    let auth_result = auth::validate_token(token, api_keys, now);
    let (key_id, owner_user_id, role) = match &auth_result {
        AuthResult::Valid {
            key_id,
            owner_user_id,
            role,
        } => (key_id.clone(), owner_user_id.clone(), role.clone()),
        _ => return Err(ApiError::unauthorized()),
    };

    if let Some(origin) = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
    {
        let origin_ok = api_keys
            .values()
            .find(|key| key.key_id == key_id)
            .map(|key| auth::check_origin(key, origin))
            .unwrap_or(false);
        if !origin_ok {
            return Err(ApiError::forbidden(
                "Origin not allowed for this API key",
            ));
        }
    }

    let permission = if matches!(*method, Method::POST | Method::PUT | Method::PATCH | Method::DELETE)
    {
        PermissionLevel::Write
    } else {
        PermissionLevel::Read
    };

    if !auth::check_permission(&auth_result, permission) {
        return Err(ApiError::forbidden("Insufficient permissions"));
    }

    let rate_limit_per_min = api_keys
        .values()
        .find(|key| key.key_id == key_id)
        .map(|key| key.rate_limit_per_min)
        .unwrap_or(120);

    Ok(AuthenticatedRequest {
        auth_context: ApiAuthContext {
            key_id,
            user_id: owner_user_id,
            role,
        },
        rate_limit_per_min,
    })
}

pub fn extract_websocket_token(
    headers: &HeaderMap,
    query_token: Option<&str>,
    options: WebSocketAuthOptions,
) -> Result<String, ApiError> {
    if let Some(token) = query_token.map(str::trim).filter(|value| !value.is_empty()) {
        return Ok(token.to_string());
    }

    if let Some(token) = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(auth::extract_bearer_token)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Ok(token.to_string());
    }

    if options.allow_subprotocol_fallback {
        if let Some(token) = headers
            .get("sec-websocket-protocol")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(',').next())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Ok(token.to_string());
        }
    }

    Err(ApiError::unauthorized())
}

pub fn validate_websocket_token(
    token: &str,
    api_keys: &HashMap<String, ApiKey>,
    now: DateTime<Utc>,
) -> Result<ApiAuthContext, ApiError> {
    match auth::validate_token(token, api_keys, now) {
        AuthResult::Valid {
            key_id,
            owner_user_id,
            role,
        } => Ok(ApiAuthContext {
            key_id,
            user_id: owner_user_id,
            role,
        }),
        _ => Err(ApiError::unauthorized()),
    }
}

pub fn websocket_subprotocol_compat_enabled() -> bool {
    std::env::var("APEX_WS_SUBPROTOCOL_AUTH_COMPAT")
        .ok()
        .map(|value| matches!(value.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
}

pub fn auth_error_response(err: ApiError) -> Response {
    let status = StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::UNAUTHORIZED);
    (status, axum::Json(error_response::<serde_json::Value>(err))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{hash_api_key, ApiRole};
    use axum::http::HeaderValue;

    fn api_keys() -> HashMap<String, ApiKey> {
        let now = Utc::now();
        HashMap::from([
            (
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
                    allowed_origins: vec!["https://allowed.test".to_string()],
                },
            ),
            (
                "viewer".to_string(),
                ApiKey {
                    key_id: "viewer-key".to_string(),
                    owner_user_id: "user-viewer".to_string(),
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

        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, HeaderValue::from_static("Bearer viewer-secret"));
        let forbidden = authenticate_api_request(&headers, &Method::POST, &keys, Utc::now())
            .expect_err("viewer write should fail");
        assert_eq!(forbidden.http_status(), 403);
    }

    #[test]
    fn principal_extraction_is_available_to_handlers_after_extraction() {
        let keys = api_keys();
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, HeaderValue::from_static("Bearer admin-secret"));
        headers.insert(header::ORIGIN, HeaderValue::from_static("https://allowed.test"));

        let authenticated = authenticate_api_request(&headers, &Method::GET, &keys, Utc::now())
            .expect("admin auth should succeed");
        assert_eq!(authenticated.auth_context.key_id, "admin-key");
        assert_eq!(authenticated.auth_context.user_id, "user-admin");
        assert_eq!(authenticated.rate_limit_per_min, 120);
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
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, HeaderValue::from_static("Bearer admin-secret"));
        let token = extract_websocket_token(&headers, None, WebSocketAuthOptions::default())
            .expect("authorization header should succeed");
        assert_eq!(token, "admin-secret");

        let query_token = extract_websocket_token(
            &HeaderMap::new(),
            Some("query-token"),
            WebSocketAuthOptions {
                allow_subprotocol_fallback: false,
            },
        )
        .expect("query token should succeed");
        assert_eq!(query_token, "query-token");
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

    #[test]
    fn warnings_ws_validates_supported_auth_transport() {
        let keys = api_keys();
        let auth_context = validate_websocket_token("admin-secret", &keys, Utc::now())
            .expect("known token should validate");
        assert_eq!(auth_context.key_id, "admin-key");
    }
}