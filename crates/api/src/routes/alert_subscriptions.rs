//! Request/response types and pure validation for per-user entity alert
//! subscriptions.
//!
//! Endpoints (wired in the binary's `app_router`):
//! - `GET    /api/entities/:id/alert-subscription`
//! - `PUT    /api/entities/:id/alert-subscription`
//! - `DELETE /api/entities/:id/alert-subscription`
//!
//! The subscription identity is always the authenticated principal. Admin
//! principals may additionally target another user with `?user_id=`; every
//! other role, including `Service`, is pinned to its own identity. (The auth
//! middleware additionally rejects all service writes with 403, so a service
//! key can only read its own subscription.)

use crate::responses::ApiError;
use serde::{Deserialize, Serialize};

/// Severities accepted in `min_severity`, lowest first.
pub const ALERT_SEVERITIES: [&str; 5] = ["info", "low", "medium", "high", "critical"];

pub const DEFAULT_MIN_SEVERITY: &str = "medium";

fn default_enabled() -> bool {
    true
}

fn default_min_severity() -> String {
    DEFAULT_MIN_SEVERITY.to_string()
}

/// Body of `PUT /api/entities/:id/alert-subscription`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertSubscriptionRequest {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// `null` (or an empty string) subscribes to every alert category.
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default = "default_min_severity")]
    pub min_severity: String,
    /// Admin/service only: manage another user's subscription instead of the
    /// caller's. Mirrors the query-string override accepted by GET/DELETE.
    #[serde(default)]
    pub user_id: Option<String>,
}

/// Query parameters of `GET`/`DELETE /api/entities/:id/alert-subscription`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AlertSubscriptionQuery {
    /// `None` addresses the "every category" row.
    #[serde(default)]
    pub category: Option<String>,
    /// Admin/service only: another user's subscription.
    #[serde(default)]
    pub user_id: Option<String>,
}

/// Normalise and validate a `min_severity` value.
pub fn validate_min_severity(value: &str) -> Result<String, ApiError> {
    let normalized = value.trim().to_ascii_lowercase();
    if ALERT_SEVERITIES.contains(&normalized.as_str()) {
        Ok(normalized)
    } else {
        Err(ApiError::validation(
            "min_severity",
            format!("must be one of {}", ALERT_SEVERITIES.join(", ")),
        ))
    }
}

/// Normalise and validate an optional alert category. `None` and the empty
/// string both mean "every category".
///
/// `'*'` is rejected: migration 048 uses `COALESCE(lower(category), '*')` as
/// its uniqueness sentinel for the "every category" row, so a literal `'*'`
/// would collide with (and silently rewrite or delete) that wildcard row.
pub fn validate_category(category: Option<&str>) -> Result<Option<String>, ApiError> {
    match category.map(str::trim).filter(|value| !value.is_empty()) {
        None => Ok(None),
        Some(value) if value.chars().count() > 64 => Err(ApiError::validation(
            "category",
            "must be 64 characters or fewer",
        )),
        Some("*") => Err(ApiError::validation(
            "category",
            "'*' is reserved for the all-categories subscription",
        )),
        Some(value) => Ok(Some(value.to_ascii_lowercase())),
    }
}

/// Resolve which user a subscription request acts for.
///
/// Without an override this is always the authenticated principal. Only admin
/// principals may target another user; analyst/viewer/service principals
/// attempting to act for someone else get 403 (never a silent rewrite).
pub fn resolve_subscription_actor(
    auth: &crate::destructive_actions::ApiAuthContext,
    requested_user_id: Option<&str>,
) -> Result<String, ApiError> {
    let requested = match requested_user_id.map(str::trim).filter(|v| !v.is_empty()) {
        None => return Ok(auth.user_id.clone()),
        Some(requested) => requested,
    };

    if requested == auth.user_id {
        return Ok(auth.user_id.clone());
    }

    if auth.role.can_admin() {
        Ok(requested.to_string())
    } else {
        Err(ApiError::forbidden(
            "Only admin principals may manage another user's alert subscription",
        ))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::auth::ApiRole;
    use crate::destructive_actions::ApiAuthContext;

    fn auth(user_id: &str, role: ApiRole) -> ApiAuthContext {
        ApiAuthContext {
            key_id: "test".to_string(),
            user_id: user_id.to_string(),
            role,
        }
    }

    #[test]
    fn actor_defaults_to_authenticated_principal() {
        let ctx = auth("alice", ApiRole::Analyst);
        assert_eq!(
            resolve_subscription_actor(&ctx, None).unwrap(),
            "alice".to_string()
        );
        assert_eq!(
            resolve_subscription_actor(&ctx, Some("alice")).unwrap(),
            "alice".to_string()
        );
    }

    #[test]
    fn actor_override_is_rejected_for_every_non_admin_role() {
        for role in [ApiRole::Analyst, ApiRole::Viewer, ApiRole::Service] {
            let ctx = auth("alice", role);
            let err = resolve_subscription_actor(&ctx, Some("bob")).unwrap_err();
            assert_eq!(err.code, crate::responses::ErrorCode::Forbidden);
        }
    }

    #[test]
    fn actor_override_is_allowed_for_admin() {
        let ctx = auth("ops", ApiRole::Admin);
        assert_eq!(
            resolve_subscription_actor(&ctx, Some("bob")).unwrap(),
            "bob".to_string()
        );
    }

    #[test]
    fn min_severity_is_normalised_and_validated() {
        assert_eq!(validate_min_severity(" HIGH ").unwrap(), "high");
        assert!(validate_min_severity("urgent").is_err());
        assert!(validate_min_severity("").is_err());
    }

    #[test]
    fn category_is_normalised_and_bounded() {
        assert_eq!(validate_category(None).unwrap(), None);
        assert_eq!(validate_category(Some("   ")).unwrap(), None);
        assert_eq!(
            validate_category(Some(" Warning ")).unwrap(),
            Some("warning".to_string())
        );
        assert!(validate_category(Some(&"x".repeat(65))).is_err());
    }

    #[test]
    fn reserved_all_categories_sentinel_is_rejected() {
        // `*` is the COALESCE sentinel for the NULL-category row; allowing it
        // would let a client rewrite or delete the wildcard subscription.
        assert!(validate_category(Some("*")).is_err());
        assert!(validate_category(Some(" * ")).is_err());
    }
}
