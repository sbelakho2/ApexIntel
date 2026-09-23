#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Psychological profiling API handlers.
//!
//! Serves the canonical psych data persisted by `PsychComputeEngine` (written by
//! the worker's `PsychProfileCompute` job) directly from the dedicated tables:
//!
//! - `GET /api/persons/:id/psych`                → latest `psychological_profiles` row
//! - `GET /api/persons/:id/behavioral-patterns`  → recent `behavioral_pattern_events` rows
//! - `GET /api/persons/:id/engagement-profile`   → latest `engagement_profiles` row
//!
//! Note: `/api/persons/:id/engagement` is already taken by the dossier engagement
//! *guide* (`dossiers_handlers::get_person_engagement`), so the psych engagement
//! *profile* is exposed at `/api/persons/:id/engagement-profile` to avoid a route
//! conflict (Axum panics on duplicate routes).

use crate::*;
use apex_api::routes::persons::validate_person_id;
use apex_insights::psych_store;

/// Maximum number of behavioral-pattern events to return per request.
const BEHAVIORAL_PATTERN_LIMIT: i64 = 50;

fn parse_person_uuid(id: &str) -> Result<Uuid, ApiError> {
    validate_person_id(id).map_err(ApiError::bad_request)
}

/// `GET /api/persons/:id/psych` — latest psychological profile snapshot.
pub(crate) async fn get_person_psych(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let uid = match parse_person_uuid(&id) {
        Ok(uid) => uid,
        Err(err) => return (StatusCode::BAD_REQUEST, Json(error_response(err))),
    };

    match psych_store::get_psychological_profile(&state.store.pool, &uid.to_string()).await {
        Ok(Some(profile)) => {
            let duration_ms = start.elapsed().as_millis() as u64;
            log_latency("get_person_psych", duration_ms);
            (
                StatusCode::OK,
                Json(success_with_meta(
                    serde_json::to_value(&profile).unwrap_or_default(),
                    ResponseMeta::now()
                        .with_request_id(request_id)
                        .with_duration(duration_ms),
                )),
            )
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(error_response(ApiError::not_found("Psych profile", &id))),
        ),
        Err(err) => {
            tracing::error!(request_id = %request_id, "get_person_psych failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to load psych profile",
                ))),
            )
        }
    }
}

/// `GET /api/persons/:id/behavioral-patterns` — recent behavioral pattern events.
pub(crate) async fn get_person_behavioral_patterns(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let uid = match parse_person_uuid(&id) {
        Ok(uid) => uid,
        Err(err) => return (StatusCode::BAD_REQUEST, Json(error_response(err))),
    };

    match psych_store::list_recent_behavioral_patterns(
        &state.store.pool,
        &uid.to_string(),
        BEHAVIORAL_PATTERN_LIMIT,
    )
    .await
    {
        Ok(patterns) => {
            let duration_ms = start.elapsed().as_millis() as u64;
            log_latency("get_person_behavioral_patterns", duration_ms);
            (
                StatusCode::OK,
                Json(success_with_meta(
                    serde_json::to_value(&patterns).unwrap_or_default(),
                    ResponseMeta::now()
                        .with_request_id(request_id)
                        .with_duration(duration_ms),
                )),
            )
        }
        Err(err) => {
            tracing::error!(
                request_id = %request_id,
                "get_person_behavioral_patterns failed: {err:#}"
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to load behavioral patterns",
                ))),
            )
        }
    }
}

/// `GET /api/persons/:id/engagement-profile` — latest engagement profile.
pub(crate) async fn get_person_engagement_profile(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let uid = match parse_person_uuid(&id) {
        Ok(uid) => uid,
        Err(err) => return (StatusCode::BAD_REQUEST, Json(error_response(err))),
    };

    match psych_store::get_engagement_profile(&state.store.pool, &uid.to_string()).await {
        Ok(Some(profile)) => {
            let duration_ms = start.elapsed().as_millis() as u64;
            log_latency("get_person_engagement_profile", duration_ms);
            (
                StatusCode::OK,
                Json(success_with_meta(
                    serde_json::to_value(&profile).unwrap_or_default(),
                    ResponseMeta::now()
                        .with_request_id(request_id)
                        .with_duration(duration_ms),
                )),
            )
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(error_response(ApiError::not_found(
                "Engagement profile",
                &id,
            ))),
        ),
        Err(err) => {
            tracing::error!(
                request_id = %request_id,
                "get_person_engagement_profile failed: {err:#}"
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to load engagement profile",
                ))),
            )
        }
    }
}
