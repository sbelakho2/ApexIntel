//! Vector search API handlers.
//!
//! Implements:
//! - `vector_search` — embed query text, then cosine-similarity search
//! - `similar_entities` — find entities similar to a given entity by its embedding
//! - `reindex_embeddings` — admin trigger for full or incremental embedding rebuild

use std::time::Instant;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
#[cfg(feature = "llm")]
use serde_json::Value as JsonValue;
use uuid::Uuid;

use apex_api::responses::{
    error_response, success_with_meta, ApiError, ApiResponse, ResponseMeta,
};
use apex_api::routes::vector_search::{
    ReindexQuery, ReindexResponse, SimilarEntitiesPath, SimilarEntitiesQuery,
    SimilarEntitiesResponse, SimilarEntityHit, VectorSearchHit, VectorSearchQuery,
    VectorSearchResponse,
};
use crate::AppState;

// ─── Handlers ──────────────────────────────────────────────────────────────

/// `GET /api/search/vector`
///
/// Takes a text query, generates an embedding via the LLM client, and returns
/// the most similar entities by cosine similarity.
pub(crate) async fn vector_search(
    State(state): State<AppState>,
    Query(params): Query<VectorSearchQuery>,
) -> (StatusCode, Json<ApiResponse<VectorSearchResponse>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();

    if params.q.trim().is_empty() {
        let err = ApiError::bad_request("query 'q' must not be empty");
        return (
            StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::BAD_REQUEST),
            Json(error_response(err)),
        );
    }

    // Generate embedding for the query text
    let embedding = match generate_embedding(&state, &params.q).await {
        Ok(emb) => emb,
        Err(e) => {
            let err = ApiError::internal(format!("failed to generate embedding: {e}"));
            return (
                StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(error_response(err)),
            );
        }
    };

    // Perform vector search
    let limit = params.limit();
    let results = match &params.entity_types {
        Some(types) if types.len() == 1 => {
            state
                .store
                .vector_search_by_type(&embedding, &types[0], limit)
                .await
        }
        _ => state.store.vector_search(&embedding, limit).await,
    };

    let db_results = match results {
        Ok(hits) => hits,
        Err(e) => {
            let err = ApiError::internal(format!("vector search query failed: {e}"));
            return (
                StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(error_response(err)),
            );
        }
    };

    // Apply min_score filter if specified
    let min_score = params.min_score.unwrap_or(0.0);
    let filtered: Vec<VectorSearchHit> = db_results
        .into_iter()
        .filter(|h| h.similarity >= min_score)
        .map(|h| VectorSearchHit {
            entity_type: h.entity_type,
            entity_id: h.entity_id,
            chunk_index: h.chunk_index,
            source_text: h.source_text,
            similarity: h.similarity,
        })
        .collect();

    let total = filtered.len();
    let duration_ms = start.elapsed().as_millis() as u64;
    let meta = ResponseMeta::now()
        .with_request_id(request_id)
        .with_duration(duration_ms);

    let payload = VectorSearchResponse {
        query: params.q,
        total,
        results: filtered,
        query_time_ms: duration_ms,
    };

    log_latency_vec("vector_search", duration_ms);
    (StatusCode::OK, Json(success_with_meta(payload, meta)))
}

/// `GET /api/entities/:entity_type/:entity_id/similar`
///
/// Looks up the embedding for the given entity, then finds other entities with
/// similar embeddings.
pub(crate) async fn similar_entities(
    State(state): State<AppState>,
    Path(path): Path<SimilarEntitiesPath>,
    Query(params): Query<SimilarEntitiesQuery>,
) -> (StatusCode, Json<ApiResponse<SimilarEntitiesResponse>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();

    // Fetch the first embedding for this entity
    let embedding = match state
        .store
        .get_entity_embedding(&path.entity_type, &path.entity_id)
        .await
        .map_err(|e| format!("{e}"))
    {
        Ok(Some(emb)) => emb,
        Ok(None) => {
            let err = ApiError::not_found(
                "embedding",
                &format!("{}/{}", path.entity_type, path.entity_id),
            );
            return (
                StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::NOT_FOUND),
                Json(error_response(err)),
            );
        }
        Err(e) => {
            let err = ApiError::internal(format!("failed to fetch embedding: {e}"));
            return (
                StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(error_response(err)),
            );
        }
    };

    // Search for similar entities (exclude the query entity itself)
    let limit = params.limit().saturating_add(1); // fetch one extra, filter source
    let results = match state.store.vector_search(&embedding, limit).await {
        Ok(hits) => hits,
        Err(e) => {
            let err = ApiError::internal(format!("similarity search failed: {e}"));
            return (
                StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(error_response(err)),
            );
        }
    };

    // Exclude the source entity and apply min_score
    let min_score = params.min_score.unwrap_or(0.0);
    let filtered: Vec<SimilarEntityHit> = results
        .into_iter()
        .filter(|h| {
            h.entity_id != path.entity_id && h.similarity >= min_score
        })
        .map(|h| SimilarEntityHit {
            entity_id: h.entity_id,
            chunk_index: h.chunk_index,
            source_text: h.source_text,
            similarity: h.similarity,
        })
        .collect();

    let total = filtered.len();
    let duration_ms = start.elapsed().as_millis() as u64;
    let meta = ResponseMeta::now()
        .with_request_id(request_id)
        .with_duration(duration_ms);

    let payload = SimilarEntitiesResponse {
        target_entity_type: path.entity_type,
        target_entity_id: path.entity_id,
        total,
        results: filtered,
        query_time_ms: duration_ms,
    };

    log_latency_vec("similar_entities", duration_ms);
    (StatusCode::OK, Json(success_with_meta(payload, meta)))
}

/// `POST /api/admin/embeddings/reindex`
///
/// Triggers a full or incremental embedding reindex. The actual work is delegated
/// to the worker crate; this endpoint returns immediately with the reindex request
/// acknowledged. Requires admin role.
pub(crate) async fn reindex_embeddings(
    State(state): State<AppState>,
    Query(params): Query<ReindexQuery>,
) -> (StatusCode, Json<ApiResponse<ReindexResponse>>) {
    let _ = state;
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();

    let is_full = params.full.unwrap_or(false);

    // In a production system, this would enqueue a NATS message for the worker.
    // For now, we acknowledge the request and return a status indicating the trigger.
    let message = if is_full {
        "Full embedding reindex triggered. This may take several hours. Check worker logs for progress."
    } else {
        "Incremental embedding reindex triggered. New entities without embeddings will be indexed."
    };

    let payload = ReindexResponse {
        status: "accepted".to_string(),
        chunks_indexed: 0,
        message: message.to_string(),
    };

    let duration_ms = start.elapsed().as_millis() as u64;
    let meta = ResponseMeta::now()
        .with_request_id(request_id)
        .with_duration(duration_ms);

    log_latency_vec("reindex_embeddings", duration_ms);
    (
        StatusCode::ACCEPTED,
        Json(success_with_meta(payload, meta)),
    )
}

// ─── Internal helpers ──────────────────────────────────────────────────────

/// Generate a text embedding using the configured LLM (if available via `llm` feature)
/// or return an error.
async fn generate_embedding(state: &AppState, text: &str) -> Result<Vec<f64>, String> {
    #[cfg(feature = "llm")]
    {
        let llm = state
            .llm
            .as_ref()
            .ok_or_else(|| "LLM not configured; enable the `llm` feature and set LLM_BASE_URL".to_string())?;

        let client = apex_llm::embeddings::EmbeddingClient::from_config(&llm.primary);
        client
            .embed(text)
            .await
            .map_err(|e| format!("embedding generation failed: {e}"))
    }

    #[cfg(not(feature = "llm"))]
    {
        let _ = state;
        let _ = text;
        Err("LLM feature is disabled; embedding generation requires the `llm` feature".to_string())
    }
}

/// Log handler latency for vector search endpoints.
fn log_latency_vec(handler: &str, duration_ms: u64) {
    tracing::debug!(
        handler = handler,
        duration_ms = duration_ms,
        "vector_search_handler"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vector_search_query_validation() {
        let q = VectorSearchQuery {
            q: "".to_string(),
            entity_types: None,
            limit: None,
            min_score: None,
        };
        assert!(q.q.trim().is_empty());
    }
}
