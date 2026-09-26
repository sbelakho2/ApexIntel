//! API handlers for per-user entity alert subscriptions.
//!
//! Endpoints:
//! - `GET    /api/entities/:id/alert-subscription` — the caller's subscriptions for an entity
//! - `PUT    /api/entities/:id/alert-subscription` — opt in (severity + category floor)
//! - `DELETE /api/entities/:id/alert-subscription` — opt out
//!
//! `user_alert_subscriptions` (migration 048) is what the alert router resolves
//! addressee-less alerts against; without these routes the table had no product
//! management path. Identity always comes from the authenticated principal
//! (`ApiAuthContext.user_id`); admin principals may target another user with
//! `?user_id=` or a body `user_id`. Service keys act as their own owner
//! identity — the auth middleware rejects every service write with 403.

use crate::*;

use apex_api::routes::alert_subscriptions::{
    resolve_subscription_actor, validate_category, validate_min_severity, AlertSubscriptionQuery,
    AlertSubscriptionRequest,
};
use apex_store::postgres::UserAlertSubscriptionRecord;

#[derive(Debug, Serialize)]
pub(crate) struct AlertSubscriptionListResponse {
    /// True when at least one enabled subscription covers this entity.
    pub watching: bool,
    pub subscriptions: Vec<UserAlertSubscriptionRecord>,
}

#[derive(Debug, Serialize)]
pub(crate) struct AlertSubscriptionResponse {
    pub subscription: UserAlertSubscriptionRecord,
}

fn parse_entity_id(raw: &str) -> Result<Uuid, ApiError> {
    Uuid::parse_str(raw).map_err(|_| ApiError::validation("id", "must be a UUID"))
}

fn store_err(err: impl std::fmt::Display) -> ApiError {
    ApiError::internal(format!("Alert subscription store error: {err}"))
}

/// GET /api/entities/:id/alert-subscription
///
/// Without `?category=`, returns every subscription the caller has for the
/// entity; with it, only that natural-key row (the store lookup uses the
/// `(user_id, entity_id, category)` prefix instead of scanning the user's set).
pub(crate) async fn get_entity_alert_subscription(
    State(state): State<AppState>,
    Extension(auth): Extension<ApiAuthContext>,
    Path(entity_id): Path<String>,
    Query(query): Query<AlertSubscriptionQuery>,
) -> Result<Json<ApiResponse<AlertSubscriptionListResponse>>, ApiError> {
    let entity_id = parse_entity_id(&entity_id)?;
    let actor = resolve_subscription_actor(&auth, query.user_id.as_deref())?;

    let subscriptions: Vec<UserAlertSubscriptionRecord> = match query.category.as_deref() {
        Some(category) => {
            let category = validate_category(Some(category))?;
            state
                .store
                .get_user_alert_subscription(&actor, entity_id, category.as_deref())
                .await
                .map_err(store_err)?
                .into_iter()
                .collect()
        }
        None => state
            .store
            .list_user_alert_subscriptions_for_entity(&actor, entity_id)
            .await
            .map_err(store_err)?,
    };

    let watching = subscriptions.iter().any(|record| record.enabled);
    Ok(Json(success(AlertSubscriptionListResponse {
        watching,
        subscriptions,
    })))
}

/// PUT /api/entities/:id/alert-subscription
pub(crate) async fn upsert_entity_alert_subscription(
    State(state): State<AppState>,
    Extension(auth): Extension<ApiAuthContext>,
    Path(entity_id): Path<String>,
    Query(query): Query<AlertSubscriptionQuery>,
    Json(body): Json<AlertSubscriptionRequest>,
) -> Result<Json<ApiResponse<AlertSubscriptionResponse>>, ApiError> {
    let entity_id = parse_entity_id(&entity_id)?;
    let requested_user = body.user_id.as_deref().or(query.user_id.as_deref());
    let actor = resolve_subscription_actor(&auth, requested_user)?;
    let category = validate_category(body.category.as_deref())?;
    let min_severity = validate_min_severity(&body.min_severity)?;

    // Migration 059's `user_id` foreign key requires the principal to exist in
    // `app_users`. Login and API-key startup provision their own identities;
    // this backstop covers a principal with no row yet (first login is enough
    // everywhere else). For an admin/service override the target's identity
    // attributes are unknown here, so a missing target is reported instead of
    // inventing a row with the caller's role.
    if state
        .store
        .get_app_user(&actor)
        .await
        .map_err(store_err)?
        .is_none()
    {
        if actor == auth.user_id.as_str() {
            state
                .store
                .ensure_app_user_exists(&actor, &actor, auth.role.as_str())
                .await
                .map_err(store_err)?;
        } else {
            return Err(ApiError::not_found("app user", &actor));
        }
    }

    let subscription = state
        .store
        .upsert_user_alert_subscription(
            &actor,
            entity_id,
            category.as_deref(),
            &min_severity,
            body.enabled,
        )
        .await
        .map_err(store_err)?;

    Ok(Json(success(AlertSubscriptionResponse { subscription })))
}

/// DELETE /api/entities/:id/alert-subscription
pub(crate) async fn delete_entity_alert_subscription(
    State(state): State<AppState>,
    Extension(auth): Extension<ApiAuthContext>,
    Path(entity_id): Path<String>,
    Query(query): Query<AlertSubscriptionQuery>,
) -> Result<Json<ApiResponse<serde_json::Value>>, ApiError> {
    let entity_id = parse_entity_id(&entity_id)?;
    let actor = resolve_subscription_actor(&auth, query.user_id.as_deref())?;
    let category = validate_category(query.category.as_deref())?;

    let deleted = state
        .store
        .delete_user_alert_subscription(&actor, entity_id, category.as_deref())
        .await
        .map_err(store_err)?;

    if !deleted {
        return Err(ApiError::not_found(
            "alert subscription",
            &format!("{actor}:{entity_id}"),
        ));
    }

    Ok(Json(success(serde_json::json!({ "deleted": true }))))
}
