#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::*;

fn parse_detail_uuid(id: &str) -> Result<Uuid, ApiError> {
    Uuid::parse_str(id).map_err(|_| ApiError::bad_request("Invalid UUID"))
}

pub(crate) async fn get_warning_detail(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<WarningResponse>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let warning_id = match parse_detail_uuid(&id) {
        Ok(uuid) => uuid,
        Err(err) => return (StatusCode::BAD_REQUEST, Json(error_response(err))),
    };
    let warning = match state.store.get_warning(warning_id).await {
        Ok(Some(warning)) => warning,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(error_response(ApiError::not_found("Warning", &id))),
            );
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "get_warning failed: {err:#}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal("Failed to load warning"))),
            );
        }
    };
    let response = warning_row_to_response(warning);
    let duration_ms = start.elapsed().as_millis() as u64;
    log_latency("get_warning_detail", duration_ms);
    (
        StatusCode::OK,
        Json(success_with_meta(
            response,
            ResponseMeta::now()
                .with_request_id(request_id)
                .with_duration(duration_ms),
        )),
    )
}

pub(crate) async fn get_insight_detail(
    State(state): State<AppState>,
    Extension(auth_ctx): Extension<ApiAuthContext>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let insight_id = match parse_detail_uuid(&id) {
        Ok(uuid) => uuid,
        Err(err) => return (StatusCode::BAD_REQUEST, Json(error_response(err))),
    };
    let insight = match state.store.get_insight(insight_id).await {
        Ok(Some(insight)) => insight,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(error_response(ApiError::not_found("Insight", &id))),
            );
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "get_insight failed: {err:#}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal("Failed to load insight"))),
            );
        }
    };

    let entity_ids: Vec<Uuid> = insight.entity_ids.clone().unwrap_or_default();
    let company_names = state
        .store
        .get_company_names_by_ids(&entity_ids)
        .await
        .unwrap_or_default();

    let mut all_observations = Vec::new();
    for entity_id in &entity_ids {
        let observations = state
            .store
            .get_observations_by_entity(*entity_id, 20)
            .await
            .unwrap_or_default();
        all_observations.extend(observations);
    }
    all_observations.sort_by_key(|o| std::cmp::Reverse(o.ts_utc));
    all_observations.truncate(50);

    let related_warnings = state
        .store
        .get_warnings_by_entity_ids(&entity_ids, 10)
        .await
        .unwrap_or_default();
    let related_insights = state
        .store
        .get_related_insights(&entity_ids, insight_id, 10)
        .await
        .unwrap_or_default();

    let observations_json: Vec<serde_json::Value> = all_observations
        .iter()
        .map(|observation| {
            serde_json::json!({
                "id": observation.id,
                "observation_type": observation.observation_type,
                "entity_id": observation.entity_id,
                "ts_utc": observation.ts_utc,
                "value": observation.value,
                "provenance": observation.provenance,
                "confidence": observation.confidence,
            })
        })
        .collect();

    let warnings_json: Vec<serde_json::Value> = related_warnings
        .iter()
        .map(|warning| {
            serde_json::json!({
                "id": warning.id,
                "title": warning.title,
                "severity": warning.severity,
                "warning_type": warning.warning_type,
                "region": warning.region,
                "confidence": warning.confidence,
                "created_at": warning.created_at,
            })
        })
        .collect();

    let related_json: Vec<serde_json::Value> = related_insights
        .iter()
        .map(|related| {
            serde_json::json!({
                "id": related.id,
                "title": related.title,
                "insight_type": related.insight_type,
                "region": related.region,
                "confidence": related.confidence,
                "created_at": related.created_at,
            })
        })
        .collect();

    let entities_json: Vec<serde_json::Value> = company_names
        .iter()
        .map(|(entity_id, name, region, _)| {
            serde_json::json!({
                "id": entity_id,
                "name": name,
                "region": region,
            })
        })
        .collect();

    let quality_score = state
        .store
        .get_insight_feedback_scores(&[insight_id])
        .await
        .ok()
        .and_then(|scores| scores.get(&insight_id).copied())
        .unwrap_or(0.5);

    let detail = serde_json::json!({
        "id": insight.id,
        "title": insight.title,
        "summary": insight.summary,
        "insight_type": insight.insight_type,
        "region": insight.region,
        "confidence": insight.confidence,
        "evidence_urls": insight.evidence_urls,
        "entity_ids": insight.entity_ids.clone().unwrap_or_default().into_iter().map(|entity_id| entity_id.to_string()).collect::<Vec<_>>(),
        "tags": insight.tags,
        "created_at": insight.created_at.unwrap_or_else(Utc::now),
        "updated_at": insight.updated_at.unwrap_or_else(Utc::now),
        "entities": entities_json,
        "observations": observations_json,
        "related_warnings": warnings_json,
        "related_insights": related_json,
        "source_count": insight.evidence_urls.as_ref().map(|urls| urls.len()).unwrap_or(0),
        "observation_count": all_observations.len(),
        "bookmarked": state.store.is_insight_bookmarked(insight_id, &auth_ctx.user_id).await.unwrap_or(false),
        "quality_score": quality_score,
    });

    let duration_ms = start.elapsed().as_millis() as u64;
    log_latency("get_insight_detail", duration_ms);
    (
        StatusCode::OK,
        Json(success_with_meta(
            detail,
            ResponseMeta::now()
                .with_request_id(request_id)
                .with_duration(duration_ms),
        )),
    )
}

#[cfg(test)]
mod tests {
    use super::parse_detail_uuid;
    use uuid::Uuid;

    #[test]
    fn test_parse_detail_uuid_accepts_valid_uuid() {
        let id = Uuid::new_v4().to_string();

        let parsed = parse_detail_uuid(&id).expect("valid uuid should parse");

        assert_eq!(parsed.to_string(), id);
    }

    #[test]
    fn test_parse_detail_uuid_rejects_invalid_uuid() {
        let err = parse_detail_uuid("bad-id").expect_err("invalid uuid should fail");

        assert_eq!(err.http_status(), 400);
        assert_eq!(err.message, "Invalid UUID");
    }
}
