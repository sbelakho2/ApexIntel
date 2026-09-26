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
            return Err(ApiError::forbidden("Origin not allowed for this API key"));
        }
    }

    let permission = if matches!(
        *method,
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    ) {
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
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer viewer-secret"),
        );
        let forbidden = authenticate_api_request(&headers, &Method::POST, &keys, Utc::now())
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
            .expect("admin auth should succeed");
        assert_eq!(authenticated.auth_context.key_id, "admin-key");
        assert_eq!(authenticated.auth_context.user_id, "user-admin");
        assert_eq!(authenticated.rate_limit_per_min, 120);
    }
}
