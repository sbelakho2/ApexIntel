use crate::auth::ApiRole;
use crate::responses::ApiError;
use apex_core::identity::UserId;
use axum::http::HeaderMap;
use serde_json::Value;

pub const DELETE_ALL_WARNINGS_CONFIRM_HEADER: &str = "x-apex-confirm-delete";
pub const DELETE_ALL_WARNINGS_REASON_HEADER: &str = "x-apex-delete-reason";
pub const DELETE_ALL_WARNINGS_CONFIRM_VALUE: &str = "warnings";
const DELETE_ALL_WARNINGS_REASON_MAX_LEN: usize = 500;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiAuthContext {
    pub key_id: String,
    /// Canonical `app_users.id` this request acts as.
    pub user_id: UserId,
    pub role: ApiRole,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteAllWarningsAuthorization {
    pub reason: String,
}

pub fn authorize_delete_all_warnings(
    headers: &HeaderMap,
    auth_ctx: &ApiAuthContext,
) -> Result<DeleteAllWarningsAuthorization, ApiError> {
    if !auth_ctx.role.can_admin() {
        return Err(ApiError::forbidden(
            "Admin role required to delete all warnings",
        ));
    }

    let confirmation = headers
        .get(DELETE_ALL_WARNINGS_CONFIRM_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            ApiError::validation(
                DELETE_ALL_WARNINGS_CONFIRM_HEADER,
                format!(
                    "missing confirmation header; expected {}: {}",
                    DELETE_ALL_WARNINGS_CONFIRM_HEADER, DELETE_ALL_WARNINGS_CONFIRM_VALUE
                ),
            )
        })?;

    if confirmation != DELETE_ALL_WARNINGS_CONFIRM_VALUE {
        return Err(ApiError::validation(
            DELETE_ALL_WARNINGS_CONFIRM_HEADER,
            format!(
                "invalid confirmation value; expected '{}'",
                DELETE_ALL_WARNINGS_CONFIRM_VALUE
            ),
        ));
    }

    let reason = headers
        .get(DELETE_ALL_WARNINGS_REASON_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            ApiError::validation(
                DELETE_ALL_WARNINGS_REASON_HEADER,
                "deletion reason is required",
            )
        })?;

    if reason.chars().count() > DELETE_ALL_WARNINGS_REASON_MAX_LEN {
        return Err(ApiError::validation(
            DELETE_ALL_WARNINGS_REASON_HEADER,
            format!(
                "deletion reason must be <= {} characters",
                DELETE_ALL_WARNINGS_REASON_MAX_LEN
            ),
        ));
    }

    Ok(DeleteAllWarningsAuthorization {
        reason: reason.to_string(),
    })
}

pub fn delete_all_warnings_audit_payload(
    auth_ctx: &ApiAuthContext,
    authorization: &DeleteAllWarningsAuthorization,
    deleted_count: u64,
) -> Value {
    Value::Object(serde_json::Map::from_iter([
        (
            "actor_user_id".to_string(),
            Value::String(auth_ctx.user_id.to_string()),
        ),
        (
            "actor_key_id".to_string(),
            Value::String(auth_ctx.key_id.clone()),
        ),
        (
            "actor_role".to_string(),
            Value::String(auth_ctx.role.as_str().to_string()),
        ),
        (
            "reason".to_string(),
            Value::String(authorization.reason.clone()),
        ),
        (
            "deleted_count".to_string(),
            Value::Number(deleted_count.into()),
        ),
        (
            "confirmation".to_string(),
            Value::String(DELETE_ALL_WARNINGS_CONFIRM_VALUE.to_string()),
        ),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::ApiRole;
    use axum::http::HeaderValue;

    fn admin_auth_context() -> ApiAuthContext {
        ApiAuthContext {
            key_id: "admin-key".to_string(),
            user_id: "user-admin".into(),
            role: ApiRole::Admin,
        }
    }

    #[test]
    fn delete_all_warnings_rejects_without_confirmation_header() {
        let headers = HeaderMap::new();
        let err = authorize_delete_all_warnings(&headers, &admin_auth_context())
            .expect_err("missing confirmation should fail");
        assert_eq!(err.http_status(), 422);
        assert!(err.message.contains("missing confirmation header"));
    }

    #[test]
    fn delete_all_warnings_rejects_wrong_confirmation_value() {
        let mut headers = HeaderMap::new();
        headers.insert(
            DELETE_ALL_WARNINGS_CONFIRM_HEADER,
            HeaderValue::from_static("nope"),
        );
        headers.insert(
            DELETE_ALL_WARNINGS_REASON_HEADER,
            HeaderValue::from_static("cleanup"),
        );

        let err = authorize_delete_all_warnings(&headers, &admin_auth_context())
            .expect_err("wrong confirmation should fail");
        assert_eq!(err.http_status(), 422);
        assert!(err.message.contains("invalid confirmation value"));
    }

    #[test]
    fn delete_all_warnings_rejects_without_reason() {
        let mut headers = HeaderMap::new();
        headers.insert(
            DELETE_ALL_WARNINGS_CONFIRM_HEADER,
            HeaderValue::from_static(DELETE_ALL_WARNINGS_CONFIRM_VALUE),
        );

        let err = authorize_delete_all_warnings(&headers, &admin_auth_context())
            .expect_err("missing reason should fail");
        assert_eq!(err.http_status(), 422);
        assert!(err.message.contains("deletion reason is required"));
    }

    #[test]
    fn delete_all_warnings_rejects_readonly_principal() {
        let mut headers = HeaderMap::new();
        headers.insert(
            DELETE_ALL_WARNINGS_CONFIRM_HEADER,
            HeaderValue::from_static(DELETE_ALL_WARNINGS_CONFIRM_VALUE),
        );
        headers.insert(
            DELETE_ALL_WARNINGS_REASON_HEADER,
            HeaderValue::from_static("cleanup"),
        );
        let viewer = ApiAuthContext {
            key_id: "viewer-key".to_string(),
            user_id: "viewer-user".into(),
            role: ApiRole::Viewer,
        };

        let err = authorize_delete_all_warnings(&headers, &viewer)
            .expect_err("viewer should not be allowed");
        assert_eq!(err.http_status(), 403);
        assert!(err.message.contains("Admin role required"));
    }

    #[test]
    fn delete_all_warnings_allows_admin_principal() {
        let mut headers = HeaderMap::new();
        headers.insert(
            DELETE_ALL_WARNINGS_CONFIRM_HEADER,
            HeaderValue::from_static(DELETE_ALL_WARNINGS_CONFIRM_VALUE),
        );
        headers.insert(
            DELETE_ALL_WARNINGS_REASON_HEADER,
            HeaderValue::from_static("cleanup duplicate warnings after backfill"),
        );

        let authorization = authorize_delete_all_warnings(&headers, &admin_auth_context())
            .expect("admin should be allowed");
        assert_eq!(
            authorization.reason,
            "cleanup duplicate warnings after backfill"
        );
    }
}
