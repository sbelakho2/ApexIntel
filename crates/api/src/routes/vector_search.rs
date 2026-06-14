//! Vector search endpoints — embedding-based similarity search and hybrid (BM25 + vector) search.
//!
//! Provides:
//! - `GET /api/search/vector` — full-text → embedding → vector cosine search
//! - `GET /api/entities/:type/:id/similar` — find similar entities by embedding
//! - `POST /api/admin/embeddings/reindex` — trigger full embedding reindex

use serde::{Deserialize, Serialize};

// ─── Vector Search (via query text) ────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct VectorSearchQuery {
    /// Free-text query to embed and search by similarity
    pub q: String,
    /// Optional filter: only return results of these entity types
    pub entity_types: Option<Vec<String>>,
    /// Results per page (default 20, max 100)
    pub limit: Option<usize>,
    /// Minimum cosine similarity threshold (0.0–1.0)
    pub min_score: Option<f64>,
}

impl VectorSearchQuery {
    pub fn limit(&self) -> usize {
        self.limit.unwrap_or(20).min(100)
    }
}

/// A single result from a vector search.
#[derive(Debug, Serialize)]
pub struct VectorSearchHit {
    pub entity_type: String,
    pub entity_id: String,
    pub chunk_index: i32,
    pub source_text: String,
    /// Cosine similarity (1.0 = identical)
    pub similarity: f64,
}

#[derive(Debug, Serialize)]
pub struct VectorSearchResponse {
    pub query: String,
    pub total: usize,
    pub results: Vec<VectorSearchHit>,
    pub query_time_ms: u64,
}

// ─── Similar entities (by ID) ──────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct SimilarEntitiesQuery {
    /// Results per page (default 20, max 100)
    pub limit: Option<usize>,
    /// Minimum cosine similarity threshold (0.0–1.0)
    pub min_score: Option<f64>,
}

impl SimilarEntitiesQuery {
    pub fn limit(&self) -> usize {
        self.limit.unwrap_or(20).min(100)
    }
}

#[derive(Debug, Deserialize)]
pub struct SimilarEntitiesPath {
    pub entity_type: String,
    pub entity_id: String,
}

/// A similar entity hit.
#[derive(Debug, Serialize)]
pub struct SimilarEntityHit {
    pub entity_id: String,
    pub chunk_index: i32,
    pub source_text: String,
    /// Cosine similarity (1.0 = identical)
    pub similarity: f64,
}

#[derive(Debug, Serialize)]
pub struct SimilarEntitiesResponse {
    pub target_entity_type: String,
    pub target_entity_id: String,
    pub total: usize,
    pub results: Vec<SimilarEntityHit>,
    pub query_time_ms: u64,
}

// ─── Admin: Reindex ───────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ReindexQuery {
    /// If true, performs a full reindex (truncate + rebuild). Default: false (incremental).
    pub full: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct ReindexResponse {
    pub status: String,
    pub chunks_indexed: u64,
    pub message: String,
}

// ─── Error response types ──────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct VectorSearchError {
    pub error: String,
    pub code: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vector_search_query_defaults() {
        let q = VectorSearchQuery {
            q: "test".to_string(),
            entity_types: None,
            limit: None,
            min_score: None,
        };
        assert_eq!(q.limit(), 20);
    }

    #[test]
    fn test_vector_search_query_clamps_limit() {
        let q = VectorSearchQuery {
            q: "test".to_string(),
            entity_types: None,
            limit: Some(200),
            min_score: None,
        };
        assert_eq!(q.limit(), 100);
    }

    #[test]
    fn test_similar_entities_query_defaults() {
        let q = SimilarEntitiesQuery {
            limit: None,
            min_score: None,
        };
        assert_eq!(q.limit(), 20);
    }

    #[test]
    fn test_similar_entities_query_clamps_limit() {
        let q = SimilarEntitiesQuery {
            limit: Some(200),
            min_score: None,
        };
        assert_eq!(q.limit(), 100);
    }

    #[test]
    fn test_reindex_query_defaults() {
        let q = ReindexQuery { full: None };
        assert_eq!(q.full, None);
    }
}
