//! PgEmbeddingStore — pgvector-backed embedding storage and vector search.
//!
//! Provides CRUD operations for the `embeddings` table, cosine-similarity search,
//! and hybrid (BM25 + vector) search via Reciprocal Rank Fusion (RRF).

use anyhow::{Context, Result};
use pgvector::Vector;
use uuid::Uuid;

use crate::postgres::PgStore;

// ─── Row types ─────────────────────────────────────────────────────────────

/// A single embedding row as stored in the database.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct EmbeddingRow {
    pub id: Uuid,
    pub entity_type: String,
    pub entity_id: String,
    pub chunk_index: i32,
    pub source_text: String,
    pub model_name: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// A vector search hit returned by cosine-similarity query.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct VectorSearchHit {
    pub entity_type: String,
    pub entity_id: String,
    pub chunk_index: i32,
    pub source_text: String,
    /// Cosine similarity score (1.0 = identical, 0.0 = orthogonal, negative = opposite)
    pub similarity: f64,
}

/// A combined search hit from hybrid (RRF) search.
#[derive(Debug, Clone)]
pub struct HybridSearchHit {
    pub entity_type: String,
    pub entity_id: String,
    pub title: String,
    pub bm25_score: f64,
    pub vector_score: f64,
    pub combined_score: f64,
}

// ─── PgStore extension methods ────────────────────────────────────────────

impl PgStore {
    /// Upsert an embedding vector for a given entity chunk.
    ///
    /// Uses the unique constraint `(entity_type, entity_id, chunk_index, model_name)`
    /// to perform an upsert: if a row exists with the same key, the embedding,
    /// source_text, and updated_at are overwritten.
    pub async fn upsert_embedding(
        &self,
        entity_type: &str,
        entity_id: &str,
        chunk_index: i32,
        embedding: &[f64],
        source_text: &str,
        model_name: &str,
    ) -> Result<()> {
        let vector = Vector::from(
            embedding.iter().copied().map(|x| x as f32).collect::<Vec<f32>>(),
        );

        sqlx::query(
            r#"
            INSERT INTO embeddings (entity_type, entity_id, chunk_index, embedding, source_text, model_name)
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT (entity_type, entity_id, chunk_index, model_name)
            DO UPDATE SET
                embedding   = EXCLUDED.embedding,
                source_text = EXCLUDED.source_text,
                updated_at  = now()
            "#,
        )
        .bind(entity_type)
        .bind(entity_id)
        .bind(chunk_index)
        .bind(&vector)
        .bind(source_text)
        .bind(model_name)
        .execute(&self.pool)
        .await
        .context("failed to upsert embedding")?;

        Ok(())
    }

    /// Delete all embeddings for a given entity.
    pub async fn delete_entity_embeddings(
        &self,
        entity_type: &str,
        entity_id: &str,
    ) -> Result<u64> {
        let result = sqlx::query(
            "DELETE FROM embeddings WHERE entity_type = $1 AND entity_id = $2",
        )
        .bind(entity_type)
        .bind(entity_id)
        .execute(&self.pool)
        .await
        .context("failed to delete entity embeddings")?;

        Ok(result.rows_affected())
    }

    /// Delete embeddings for a specific model (e.g. before re-indexing with a new model).
    pub async fn delete_embeddings_by_model(&self, model_name: &str) -> Result<u64> {
        let result = sqlx::query("DELETE FROM embeddings WHERE model_name = $1")
            .bind(model_name)
            .execute(&self.pool)
            .await
            .context("failed to delete embeddings by model")?;

        Ok(result.rows_affected())
    }

    /// Perform a cosine-similarity vector search.
    ///
    /// Returns the top-k most similar entities to the query vector.
    pub async fn vector_search(
        &self,
        embedding: &[f64],
        limit: usize,
    ) -> Result<Vec<VectorSearchHit>> {
        let vector = Vector::from(
            embedding.iter().copied().map(|x| x as f32).collect::<Vec<f32>>(),
        );
        let limit = limit.min(100) as i64;

        let rows = sqlx::query_as::<_, VectorSearchHit>(
            r#"
            SELECT
                entity_type,
                entity_id,
                chunk_index,
                source_text,
                1 - (embedding <=> $1) AS similarity
            FROM embeddings
            ORDER BY embedding <=> $1
            LIMIT $2
            "#,
        )
        .bind(&vector)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .context("vector search query failed")?;

        Ok(rows)
    }

    /// Vector search filtered by entity type.
    pub async fn vector_search_by_type(
        &self,
        embedding: &[f64],
        entity_type: &str,
        limit: usize,
    ) -> Result<Vec<VectorSearchHit>> {
        let vector = Vector::from(
            embedding.iter().copied().map(|x| x as f32).collect::<Vec<f32>>(),
        );
        let limit = limit.min(100) as i64;

        let rows = sqlx::query_as::<_, VectorSearchHit>(
            r#"
            SELECT
                entity_type,
                entity_id,
                chunk_index,
                source_text,
                1 - (embedding <=> $1) AS similarity
            FROM embeddings
            WHERE entity_type = $2
            ORDER BY embedding <=> $1
            LIMIT $3
            "#,
        )
        .bind(&vector)
        .bind(entity_type)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .context("vector search by type query failed")?;

        Ok(rows)
    }

    /// Count embeddings for a given entity type (or all if None).
    pub async fn count_embeddings(&self, entity_type: Option<&str>) -> Result<i64> {
        let count: (i64,) = match entity_type {
            Some(ety) => {
                sqlx::query_as(
                    "SELECT COUNT(*) FROM embeddings WHERE entity_type = $1",
                )
                .bind(ety)
                .fetch_one(&self.pool)
                .await
                .context("failed to count embeddings")?
            }
            None => {
                sqlx::query_as("SELECT COUNT(*) FROM embeddings")
                    .fetch_one(&self.pool)
                    .await
                    .context("failed to count embeddings")?
            }
        };
        Ok(count.0)
    }

    /// Get distinct entity types that have embeddings.
    pub async fn embedding_entity_types(&self) -> Result<Vec<String>> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT DISTINCT entity_type FROM embeddings ORDER BY entity_type",
        )
        .fetch_all(&self.pool)
        .await
        .context("failed to list embedding entity types")?;

        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    /// Retrieve all entities of a given type that currently lack embeddings
    /// (for incremental indexing). Returns up to `limit` entity IDs.
    pub async fn entities_missing_embeddings(
        &self,
        entity_type: &str,
        limit: i64,
    ) -> Result<Vec<String>> {
        match entity_type {
            "company" => {
                let rows: Vec<(String,)> = sqlx::query_as(
                    r#"
                    SELECT c.id::text
                    FROM companies c
                    LEFT JOIN embeddings e
                        ON e.entity_type = 'company'
                        AND e.entity_id = c.id::text
                    WHERE e.id IS NULL
                    LIMIT $1
                    "#,
                )
                .bind(limit)
                .fetch_all(&self.pool)
                .await
                .context("failed to query companies missing embeddings")?;
                Ok(rows.into_iter().map(|r| r.0).collect())
            }
            "person" => {
                let rows: Vec<(String,)> = sqlx::query_as(
                    r#"
                    SELECT p.id::text
                    FROM persons p
                    LEFT JOIN embeddings e
                        ON e.entity_type = 'person'
                        AND e.entity_id = p.id::text
                    WHERE e.id IS NULL
                    LIMIT $1
                    "#,
                )
                .bind(limit)
                .fetch_all(&self.pool)
                .await
                .context("failed to query persons missing embeddings")?;
                Ok(rows.into_iter().map(|r| r.0).collect())
            }
            "insight" => {
                let rows: Vec<(String,)> = sqlx::query_as(
                    r#"
                    SELECT i.id::text
                    FROM insights i
                    LEFT JOIN embeddings e
                        ON e.entity_type = 'insight'
                        AND e.entity_id = i.id::text
                    WHERE e.id IS NULL
                    LIMIT $1
                    "#,
                )
                .bind(limit)
                .fetch_all(&self.pool)
                .await
                .context("failed to query insights missing embeddings")?;
                Ok(rows.into_iter().map(|r| r.0).collect())
            }
            "warning" => {
                let rows: Vec<(String,)> = sqlx::query_as(
                    r#"
                    SELECT w.id::text
                    FROM warnings w
                    LEFT JOIN embeddings e
                        ON e.entity_type = 'warning'
                        AND e.entity_id = w.id::text
                    WHERE e.id IS NULL
                    LIMIT $1
                    "#,
                )
                .bind(limit)
                .fetch_all(&self.pool)
                .await
                .context("failed to query warnings missing embeddings")?;
                Ok(rows.into_iter().map(|r| r.0).collect())
            }
            "observation" => {
                let rows: Vec<(String,)> = sqlx::query_as(
                    r#"
                    SELECT o.id::text
                    FROM observations o
                    LEFT JOIN embeddings e
                        ON e.entity_type = 'observation'
                        AND e.entity_id = o.id::text
                    WHERE e.id IS NULL
                    LIMIT $1
                    "#,
                )
                .bind(limit)
                .fetch_all(&self.pool)
                .await
                .context("failed to query observations missing embeddings")?;
                Ok(rows.into_iter().map(|r| r.0).collect())
            }
            other => anyhow::bail!("unsupported entity type for embedding indexing: {other}"),
        }
    }

    /// Get the source text for a given entity (used during indexing to generate embeddings).
    pub async fn entity_source_text(
        &self,
        entity_type: &str,
        entity_id: &str,
    ) -> Result<Option<String>> {
        match entity_type {
            "company" => {
                let row: Option<(String,)> = sqlx::query_as(
                    "SELECT COALESCE(narrative, name) FROM companies WHERE id = $1::uuid",
                )
                .bind(entity_id)
                .fetch_optional(&self.pool)
                .await
                .context("failed to fetch company source text")?;
                Ok(row.map(|r| r.0))
            }
            "person" => {
                let row: Option<(String,)> = sqlx::query_as(
                    "SELECT COALESCE(narrative, full_name) FROM persons WHERE id = $1::uuid",
                )
                .bind(entity_id)
                .fetch_optional(&self.pool)
                .await
                .context("failed to fetch person source text")?;
                Ok(row.map(|r| r.0))
            }
            "insight" => {
                let row: Option<(String,)> = sqlx::query_as(
                    "SELECT COALESCE(description, title) FROM insights WHERE id = $1::uuid",
                )
                .bind(entity_id)
                .fetch_optional(&self.pool)
                .await
                .context("failed to fetch insight source text")?;
                Ok(row.map(|r| r.0))
            }
            "warning" => {
                let row: Option<(String,)> = sqlx::query_as(
                    "SELECT COALESCE(description, title) FROM warnings WHERE id = $1::uuid",
                )
                .bind(entity_id)
                .fetch_optional(&self.pool)
                .await
                .context("failed to fetch warning source text")?;
                Ok(row.map(|r| r.0))
            }
            "observation" => {
                // Observation text lives in the `value` JSONB column under the
                // `content` key (written by the crawl pipeline). There is no
                // top-level `content` column on the observations table.
                let row: Option<(String,)> = sqlx::query_as(
                    "SELECT COALESCE(value->>'content', value->>'body_excerpt', value->>'title', value->>'description', '') FROM observations WHERE id = $1::uuid",
                )
                .bind(entity_id)
                .fetch_optional(&self.pool)
                .await
                .context("failed to fetch observation source text")?;
                Ok(row.map(|r| r.0))
            }
            other => anyhow::bail!("unsupported entity type for source text lookup: {other}"),
        }
    }

    /// Get the embedding vector for a given entity (first chunk, if any).
    pub async fn get_entity_embedding(
        &self,
        entity_type: &str,
        entity_id: &str,
    ) -> Result<Option<Vec<f64>>> {
        let row: Option<(pgvector::Vector,)> = sqlx::query_as(
            r#"
            SELECT embedding
            FROM embeddings
            WHERE entity_type = $1 AND entity_id = $2
            ORDER BY chunk_index
            LIMIT 1
            "#,
        )
        .bind(entity_type)
        .bind(entity_id)
        .fetch_optional(&self.pool)
        .await
        .context("failed to fetch entity embedding")?;

        Ok(row.map(|r| r.0.to_vec().into_iter().map(|x| x as f64).collect()))
    }
}

// ─── Hybrid search: Reciprocal Rank Fusion (RRF) ──────────────────────────

/// Combine BM25 and vector search results using Reciprocal Rank Fusion.
///
/// Both `bm25_hits` and `vector_hits` should already be sorted by descending score.
/// Each hit is identified by `(entity_type, entity_id)`. The combined score is:
///
/// ```text
/// score = 1 / (k + rank_bm25) + 1 / (k + rank_vector)
/// ```
///
/// where `k = 60` (standard RRF constant).
pub fn fuse_ranked_results(
    bm25_hits: &[HybridSearchHit],
    vector_hits: &[VectorSearchHit],
    k: f64,
    top_n: usize,
) -> Vec<HybridSearchHit> {
    use std::collections::HashMap;

    // Map each (entity_type, entity_id) to its BM25 rank (0-based)
    let mut bm25_ranks: HashMap<(String, String), usize> = HashMap::new();
    for (rank, hit) in bm25_hits.iter().enumerate() {
        bm25_ranks.insert((hit.entity_type.clone(), hit.entity_id.clone()), rank);
    }

    // Map each (entity_type, entity_id) to its vector rank (0-based)
    let mut vector_ranks: HashMap<(String, String), usize> = HashMap::new();
    for (rank, hit) in vector_hits.iter().enumerate() {
        vector_ranks.insert((hit.entity_type.clone(), hit.entity_id.clone()), rank);
    }

    // Collect all unique keys
    let mut all_keys: Vec<(String, String)> = Vec::new();
    for hit in bm25_hits {
        let key = (hit.entity_type.clone(), hit.entity_id.clone());
        if !all_keys.contains(&key) {
            all_keys.push(key);
        }
    }
    for hit in vector_hits {
        let key = (hit.entity_type.clone(), hit.entity_id.clone());
        if !all_keys.contains(&key) {
            all_keys.push(key);
        }
    }

    // Compute RRF scores
    let mut scored: Vec<HybridSearchHit> = all_keys
        .into_iter()
        .map(|(entity_type, entity_id)| {
            let bm25_rank = bm25_ranks
                .get(&(entity_type.clone(), entity_id.clone()))
                .copied()
                .unwrap_or(usize::MAX);
            let vector_rank = vector_ranks
                .get(&(entity_type.clone(), entity_id.clone()))
                .copied()
                .unwrap_or(usize::MAX);

            let bm25_score = if bm25_rank == usize::MAX {
                0.0
            } else {
                1.0 / (k + bm25_rank as f64)
            };
            let vector_score = if vector_rank == usize::MAX {
                0.0
            } else {
                1.0 / (k + vector_rank as f64)
            };

            // Find title from BM25 hit if available
            let title = bm25_hits
                .iter()
                .find(|h| h.entity_type == entity_type && h.entity_id == entity_id)
                .map(|h| h.title.clone())
                .unwrap_or_default();

            HybridSearchHit {
                entity_type,
                entity_id,
                title,
                bm25_score,
                vector_score,
                combined_score: bm25_score + vector_score,
            }
        })
        .collect();

    // Sort by combined score descending
    scored.sort_by(|a, b| {
        b.combined_score
            .partial_cmp(&a.combined_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    scored.truncate(top_n);
    scored
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rrf_empty_inputs() {
        let result = fuse_ranked_results(&[], &[], 60.0, 10);
        assert!(result.is_empty());
    }

    #[test]
    fn test_rrf_only_bm25() {
        let bm25 = vec![HybridSearchHit {
            entity_type: "company".to_string(),
            entity_id: "1".to_string(),
            title: "Acme".to_string(),
            bm25_score: 0.5,
            vector_score: 0.0,
            combined_score: 0.0,
        }];
        let result = fuse_ranked_results(&bm25, &[], 60.0, 10);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].entity_id, "1");
        assert!(result[0].combined_score > 0.0);
        assert!(result[0].vector_score == 0.0);
    }

    #[test]
    fn test_rrf_only_vector() {
        let vector = vec![VectorSearchHit {
            entity_type: "company".to_string(),
            entity_id: "2".to_string(),
            chunk_index: 0,
            source_text: "Some text".to_string(),
            similarity: 0.9,
        }];
        let result = fuse_ranked_results(&[], &vector, 60.0, 10);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].entity_id, "2");
        assert!(result[0].bm25_score == 0.0);
        assert!(result[0].vector_score > 0.0);
    }

    #[test]
    fn test_rrf_combines_overlapping() {
        let bm25 = vec![
            HybridSearchHit {
                entity_type: "company".to_string(),
                entity_id: "1".to_string(),
                title: "Acme".to_string(),
                bm25_score: 0.0,
                vector_score: 0.0,
                combined_score: 0.0,
            },
            HybridSearchHit {
                entity_type: "company".to_string(),
                entity_id: "2".to_string(),
                title: "Beta".to_string(),
                bm25_score: 0.0,
                vector_score: 0.0,
                combined_score: 0.0,
            },
        ];
        let vector = vec![
            VectorSearchHit {
                entity_type: "company".to_string(),
                entity_id: "2".to_string(),
                chunk_index: 0,
                source_text: "Beta text".to_string(),
                similarity: 0.9,
            },
            VectorSearchHit {
                entity_type: "company".to_string(),
                entity_id: "3".to_string(),
                chunk_index: 0,
                source_text: "Gamma text".to_string(),
                similarity: 0.8,
            },
        ];
        let result = fuse_ranked_results(&bm25, &vector, 60.0, 10);
        // Should have 3 unique entities: 1, 2, 3
        assert_eq!(result.len(), 3);
        // Entity "2" appears in both, so it should rank highest
        assert_eq!(result[0].entity_id, "2");
        assert!(result[0].combined_score > result[1].combined_score);
    }

    #[test]
    fn test_rrf_top_n_truncation() {
        let bm25 = (0..10)
            .map(|i| HybridSearchHit {
                entity_type: "company".to_string(),
                entity_id: i.to_string(),
                title: format!("Company {i}"),
                bm25_score: 0.0,
                vector_score: 0.0,
                combined_score: 0.0,
            })
            .collect::<Vec<_>>();
        let result = fuse_ranked_results(&bm25, &[], 60.0, 3);
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn test_rrf_k_value_affects_scores() {
        let bm25 = vec![
            HybridSearchHit {
                entity_type: "company".to_string(),
                entity_id: "1".to_string(),
                title: "A".to_string(),
                bm25_score: 0.0,
                vector_score: 0.0,
                combined_score: 0.0,
            },
        ];
        let result_low_k = fuse_ranked_results(&bm25, &[], 1.0, 10);
        let result_high_k = fuse_ranked_results(&bm25, &[], 100.0, 10);
        // Higher k = lower score
        assert!(result_low_k[0].combined_score > result_high_k[0].combined_score);
    }
}
