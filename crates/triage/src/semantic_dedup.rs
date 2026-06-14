//! Semantic Deduplication — uses embedding similarity to detect near-duplicate
//! items before they enter the triage queue.
//!
//! This module integrates with [`PgEmbeddingStore`] to compare candidate items
//! against recently-triaged items using cosine similarity on vector embeddings.

use anyhow::Result;

use apex_core::triage::TriageItemType;
use apex_llm::embeddings::EmbeddingClient;
use apex_store::postgres::embeddings::VectorSearchHit;

/// Configuration for semantic deduplication.
#[derive(Debug, Clone)]
pub struct DedupConfig {
    /// Similarity threshold above which items are considered duplicates (0.0–1.0).
    pub threshold: f64,
    /// Maximum number of recent items to compare against.
    pub max_candidates: usize,
    /// Entity type string used for embedding lookups.
    pub entity_type: String,
}

impl Default for DedupConfig {
    fn default() -> Self {
        Self {
            threshold: 0.92,
            max_candidates: 20,
            entity_type: "triage_item".to_string(),
        }
    }
}

/// Result of a deduplication check.
#[derive(Debug, Clone)]
pub struct DedupResult {
    /// Whether the item is considered a duplicate.
    pub is_duplicate: bool,
    /// Similarity score of the best match (0.0–1.0).
    pub best_match_score: f64,
    /// ID of the most similar existing item, if any.
    pub best_match_id: Option<String>,
    /// Title of the most similar existing item, if any.
    pub best_match_title: Option<String>,
}

impl DedupResult {
    /// A non-duplicate result.
    pub fn unique() -> Self {
        Self {
            is_duplicate: false,
            best_match_score: 0.0,
            best_match_id: None,
            best_match_title: None,
        }
    }
}

/// Semantic deduplication engine using embedding similarity.
pub struct SemanticDedup {
    embedding_client: Option<EmbeddingClient>,
    config: DedupConfig,
}

impl SemanticDedup {
    /// Create a new [`SemanticDedup`].
    ///
    /// Pass `None` for `embedding_client` to disable dedup (always unique).
    pub fn new(embedding_client: Option<EmbeddingClient>, config: DedupConfig) -> Self {
        Self {
            embedding_client,
            config,
        }
    }

    /// Create a dedup engine with default config and no embedding client
    /// (dedup will be effectively disabled).
    pub fn disabled() -> Self {
        Self {
            embedding_client: None,
            config: DedupConfig::default(),
        }
    }

    /// Check if a candidate item is semantically similar to an existing triaged item.
    ///
    /// This is an async check that calls the embedding API and vector store.
    /// When no embedding client is configured, it returns `unique()` immediately.
    pub async fn check_duplicate(
        &self,
        _item_type: &TriageItemType,
        title: &str,
        description: &str,
    ) -> Result<DedupResult> {
        let client = match &self.embedding_client {
            Some(c) => c,
            None => return Ok(DedupResult::unique()),
        };

        // Build text to embed
        let embed_text = format!("{}: {}", title, description);

        // Generate embedding for the candidate
        let embedding = client.embed(&embed_text).await?;

        if embedding.is_empty() {
            return Ok(DedupResult::unique());
        }

        // For now, return a unique result since we don't have direct access
        // to the triage queue's embedding store from here. The actual comparison
        // would be done at the store layer where PgEmbeddingStore is available.
        //
        // This method is kept for the interface contract; the store layer
        // integration will perform the actual vector search.
        Ok(DedupResult {
            is_duplicate: false,
            best_match_score: 0.0,
            best_match_id: None,
            best_match_title: None,
        })
    }

    /// Compute cosine similarity between two embedding vectors.
    pub fn cosine_similarity(a: &[f64], b: &[f64]) -> f64 {
        if a.len() != b.len() || a.is_empty() {
            return 0.0;
        }

        let dot: f64 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
        let norm_a: f64 = a.iter().map(|x| x * x).sum::<f64>().sqrt();
        let norm_b: f64 = b.iter().map(|x| x * x).sum::<f64>().sqrt();

        if norm_a == 0.0 || norm_b == 0.0 {
            return 0.0;
        }

        (dot / (norm_a * norm_b)).clamp(0.0, 1.0)
    }

    /// Determine if a vector search hit should be considered a duplicate.
    pub fn is_duplicate_hit(hit: &VectorSearchHit, threshold: f64) -> bool {
        hit.similarity >= threshold
    }

    /// Filter a list of search hits to only those above the dedup threshold,
    /// returning the best match.
    pub fn best_match(hits: &[VectorSearchHit], threshold: f64) -> Option<DedupResult> {
        hits.iter()
            .filter(|h| h.similarity >= threshold)
            .max_by(|a, b| a.similarity.partial_cmp(&b.similarity).unwrap_or(std::cmp::Ordering::Equal))
            .map(|hit| DedupResult {
                is_duplicate: true,
                best_match_score: hit.similarity,
                best_match_id: Some(hit.entity_id.clone()),
                best_match_title: Some(hit.entity_type.clone()),
            })
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cosine_similarity_identical() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        let sim = SemanticDedup::cosine_similarity(&a, &b);
        assert!((sim - 1.0).abs() < 1e-9);
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        let sim = SemanticDedup::cosine_similarity(&a, &b);
        assert!((sim - 0.0).abs() < 1e-9);
    }

    #[test]
    fn test_cosine_similarity_partial() {
        let a = vec![1.0, 0.0];
        let b = vec![0.5, 0.5];
        let sim = SemanticDedup::cosine_similarity(&a, &b);
        // dot = 0.5, |a| = 1.0, |b| = sqrt(0.5) ≈ 0.707
        // sim = 0.5 / 0.707 ≈ 0.707
        assert!((sim - std::f64::consts::FRAC_1_SQRT_2).abs() < 0.001);
    }

    #[test]
    fn test_cosine_similarity_empty() {
        let a: Vec<f64> = vec![];
        let b: Vec<f64> = vec![];
        assert!((SemanticDedup::cosine_similarity(&a, &b) - 0.0).abs() < 1e-9);
    }

    #[test]
    fn test_cosine_similarity_zero_vector() {
        let a = vec![0.0, 0.0];
        let b = vec![1.0, 0.0];
        assert!((SemanticDedup::cosine_similarity(&a, &b) - 0.0).abs() < 1e-9);
    }

    #[test]
    fn test_dedup_config_default() {
        let config = DedupConfig::default();
        assert!((config.threshold - 0.92).abs() < 1e-9);
        assert_eq!(config.max_candidates, 20);
    }

    #[tokio::test]
    async fn test_disabled_dedup_returns_unique() {
        let dedup = SemanticDedup::disabled();
        let result = dedup.check_duplicate(
            &TriageItemType::Insight,
            "Test title",
            "Test description",
        ).await;
        assert!(result.is_ok());
        assert!(!result.unwrap().is_duplicate);
    }

    #[test]
    fn test_is_duplicate_hit() {
        let hit = VectorSearchHit {
            entity_id: "abc-123".to_string(),
            entity_type: "triage_item".to_string(),
            chunk_index: 0,
            similarity: 0.95,
            source_text: "Duplicate item content".to_string(),
        };

        assert!(SemanticDedup::is_duplicate_hit(&hit, 0.92));
        assert!(!SemanticDedup::is_duplicate_hit(&hit, 0.96));
    }

    #[test]
    fn test_best_match_returns_highest() {
        let hits = vec![
            VectorSearchHit {
                entity_id: "a".to_string(),
                entity_type: "triage_item".to_string(),
                chunk_index: 0,
                similarity: 0.85,
                source_text: "Low similarity content".to_string(),
            },
            VectorSearchHit {
                entity_id: "b".to_string(),
                entity_type: "triage_item".to_string(),
                chunk_index: 0,
                similarity: 0.97,
                source_text: "High similarity content".to_string(),
            },
            VectorSearchHit {
                entity_id: "c".to_string(),
                entity_type: "triage_item".to_string(),
                chunk_index: 0,
                similarity: 0.93,
                source_text: "Medium similarity content".to_string(),
            },
        ];

        let result = SemanticDedup::best_match(&hits, 0.90);
        assert!(result.is_some());
        let r = result.unwrap();
        assert!(r.is_duplicate);
        assert!((r.best_match_score - 0.97).abs() < 0.001);
        assert_eq!(r.best_match_id.unwrap(), "b");
    }

    #[test]
    fn test_best_match_no_hits_above_threshold() {
        let hits = vec![
            VectorSearchHit {
                entity_id: "a".to_string(),
                entity_type: "triage_item".to_string(),
                chunk_index: 0,
                similarity: 0.80,
                source_text: "Low similarity content".to_string(),
            },
        ];

        let result = SemanticDedup::best_match(&hits, 0.90);
        assert!(result.is_none());
    }

    #[test]
    fn test_best_match_empty() {
        let hits: Vec<VectorSearchHit> = vec![];
        let result = SemanticDedup::best_match(&hits, 0.90);
        assert!(result.is_none());
    }

    #[test]
    fn test_dedup_result_unique() {
        let result = DedupResult::unique();
        assert!(!result.is_duplicate);
        assert!((result.best_match_score - 0.0).abs() < 0.001);
        assert!(result.best_match_id.is_none());
    }
}
