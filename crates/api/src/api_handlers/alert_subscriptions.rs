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
//! (`ApiAuthContext.user_id`); admin/service principals may target another user
//! with `?user_id=` or a body `user_id`.

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
pub(crate) async fn get_entity_alert_subscription(
    State(state): State<AppState>,
    Extension(auth): Extension<ApiAuthContext>,
    Path(entity_id): Path<String>,
    Query(query): Query<AlertSubscriptionQuery>,
) -> Result<Json<ApiResponse<AlertSubscriptionListResponse>>, ApiError> {
    let entity_id = parse_entity_id(&entity_id)?;
    let actor = resolve_subscription_actor(&auth, query.user_id.as_deref())?;

    let subscriptions: Vec<UserAlertSubscriptionRecord> = state
        .store
        .list_user_alert_subscriptions(&actor)
        .await
        .map_err(store_err)?
        .into_iter()
        .filter(|record| record.entity_id == entity_id)
        .collect();

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

    // The `user_id` foreign key added in migration 059 requires the principal
    // to exist in `app_users`; login creates it, and this backstop covers API
    // keys whose owner never signed in through the web form.
    state
        .store
        .ensure_app_user(&actor, &actor, auth.role.as_str())
        .await
        .map_err(store_err)?;

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
