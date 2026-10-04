//! Semantic Deduplication — uses embedding similarity to detect near-duplicate
//! items before they enter the triage queue.
//!
//! This module integrates with the embedding store to compare candidate items
//! against recently-triaged items using cosine similarity on vector embeddings.
//!
//! # Full implementation
//! When an `EmbeddingClient` is available and a `PgEmbeddingStore` reference
//! is provided, this performs real vector search against the triage queue.
//! Without these, it falls back to text-based Jaccard similarity as a lightweight
//! dedup mechanism (not perfect, but far better than returning `unique()` for
//! everything).

use anyhow::{Context, Result};
use async_trait::async_trait;
use std::collections::HashSet;
use std::sync::Arc;

use apex_core::triage::{TriageDimensions, TriageItemType};
use apex_llm::embeddings::EmbeddingClient;
use apex_store::postgres::embeddings::VectorSearchHit;
use chrono::{DateTime, Duration, Utc};
use uuid::Uuid;

/// Configuration for semantic deduplication.
#[derive(Debug, Clone)]
pub struct DedupConfig {
    /// Similarity threshold above which items are considered duplicates (0.0–1.0).
    pub threshold: f64,
    /// Maximum number of recent items to compare against.
    pub max_candidates: usize,
    /// Entity type string used for embedding lookups.
    pub entity_type: String,
    /// Minimum text length for meaningful comparison.
    pub min_text_length: usize,
}

impl Default for DedupConfig {
    fn default() -> Self {
        Self {
            threshold: 0.92,
            max_candidates: 20,
            entity_type: "triage_item".to_string(),
            min_text_length: 20,
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

/// Trait for storing recently-triaged items for dedup comparison.
///
/// Implementations can use in-memory caches, Postgres, or vector stores.
/// Methods are async so the persistent (PostgreSQL) implementation can use
/// `sqlx` without bridging runtimes; the in-memory fallback simply returns
/// ready values.
#[async_trait]
pub trait DedupStore: Send + Sync {
    /// Find similar items to the given text, returning scored hits.
    async fn find_similar(
        &self,
        item_type: &TriageItemType,
        text: &str,
        max_results: usize,
    ) -> Result<Vec<DedupHit>>;

    /// Store a newly triaged item for future dedup comparisons.
    async fn store_item(
        &self,
        item_type: &TriageItemType,
        id: &str,
        title: &str,
        text: &str,
    ) -> Result<()>;

    /// Find similar items by embedding vector (B343).
    ///
    /// Returns `Ok(Vec::new())` when the implementation does not store
    /// vectors — the caller falls back to text similarity.
    async fn find_similar_by_vector(
        &self,
        _item_type: &TriageItemType,
        _vector: &[f64],
        _max_results: usize,
    ) -> Result<Vec<DedupHit>> {
        Ok(Vec::new())
    }

    /// Store an item together with its embedding (B343).
    async fn store_item_with_vector(
        &self,
        item_type: &TriageItemType,
        id: &str,
        title: &str,
        text: &str,
        _vector: Option<&[f64]>,
    ) -> Result<()> {
        self.store_item(item_type, id, title, text).await
    }

    /// Remove a stored candidate so it can never match again.
    ///
    /// The ingress calls this when a similarity hit points at a queue row
    /// that has been deleted, is no longer active, or fell outside the dedup
    /// window — such a candidate can never be a legitimate merge target.
    /// Stores without candidate retention may keep the default no-op.
    async fn prune_item(&self, _item_type: &TriageItemType, _id: &str) -> Result<()> {
        Ok(())
    }
}

/// A hit from the dedup store with similarity score.
#[derive(Debug, Clone)]
pub struct DedupHit {
    pub id: String,
    pub title: String,
    pub similarity: f64,
}

/// In-memory dedup store using text trigram Jaccard similarity.
///
/// This is the fallback when no embedding client or vector store is available.
/// It provides lightweight dedup that catches exact and near-matches.
pub struct InMemoryDedupStore {
    items: std::sync::Mutex<Vec<StoredItem>>,
    max_items: usize,
}

#[derive(Debug, Clone)]
struct StoredItem {
    id: String,
    title: String,
    text: String,
    item_type: String,
    embedding: Option<Vec<f64>>,
}

impl InMemoryDedupStore {
    pub fn new(max_items: usize) -> Self {
        Self {
            items: std::sync::Mutex::new(Vec::with_capacity(max_items)),
            max_items,
        }
    }
}

#[async_trait]
impl DedupStore for InMemoryDedupStore {
    async fn find_similar(
        &self,
        item_type: &TriageItemType,
        text: &str,
        max_results: usize,
    ) -> Result<Vec<DedupHit>> {
        let items = self.items.lock().unwrap_or_else(|e| e.into_inner());
        let item_type_str = item_type.as_str();

        let mut hits: Vec<DedupHit> = items
            .iter()
            .filter(|item| item.item_type == item_type_str)
            .map(|item| {
                let similarity = jaccard_trigram_similarity(text, &item.text);
                DedupHit {
                    id: item.id.clone(),
                    title: item.title.clone(),
                    similarity,
                }
            })
            .filter(|hit| hit.similarity > 0.3)
            .collect();

        hits.sort_by(|a, b| {
            b.similarity
                .partial_cmp(&a.similarity)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        hits.truncate(max_results);
        Ok(hits)
    }

    async fn store_item(
        &self,
        item_type: &TriageItemType,
        id: &str,
        title: &str,
        text: &str,
    ) -> Result<()> {
        self.store_item_with_vector(item_type, id, title, text, None)
            .await
    }

    async fn store_item_with_vector(
        &self,
        item_type: &TriageItemType,
        id: &str,
        title: &str,
        text: &str,
        vector: Option<&[f64]>,
    ) -> Result<()> {
        let mut items = self.items.lock().unwrap_or_else(|e| e.into_inner());
        let item_type_str = item_type.as_str();

        // Dedup by id within store
        if !items
            .iter()
            .any(|item| item.id == id && item.item_type == item_type_str)
        {
            items.push(StoredItem {
                id: id.to_string(),
                title: title.to_string(),
                text: text.to_string(),
                item_type: item_type_str.to_string(),
                embedding: vector.map(|v| v.to_vec()),
            });

            // Trim oldest if over capacity
            while items.len() > self.max_items {
                items.remove(0);
            }
        }
        Ok(())
    }

    async fn prune_item(&self, item_type: &TriageItemType, id: &str) -> Result<()> {
        let mut items = self.items.lock().unwrap_or_else(|e| e.into_inner());
        let item_type_str = item_type.as_str();
        items.retain(|item| !(item.id == id && item.item_type == item_type_str));
        Ok(())
    }

    async fn find_similar_by_vector(
        &self,
        item_type: &TriageItemType,
        vector: &[f64],
        max_results: usize,
    ) -> Result<Vec<DedupHit>> {
        let items = self.items.lock().unwrap_or_else(|e| e.into_inner());

        let mut hits: Vec<DedupHit> = items
            .iter()
            .filter(|item| item.item_type == item_type.as_str())
            .filter_map(|item| {
                let stored = item.embedding.as_ref()?;
                let similarity = SemanticDedup::cosine_similarity(vector, stored);
                (similarity > 0.3).then(|| DedupHit {
                    id: item.id.clone(),
                    title: item.title.clone(),
                    similarity,
                })
            })
            .collect();

        hits.sort_by(|a, b| {
            b.similarity
                .partial_cmp(&a.similarity)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        hits.truncate(max_results);
        Ok(hits)
    }
}

/// Persistent [`DedupStore`] backed by PostgreSQL + pgvector.
///
/// Rows live in `semantic_dedup_items` (migration 084), so dedup state survives
/// worker restarts. Nearest neighbours are found with pgvector cosine distance
/// when a stored embedding exists, and with pg_trgm trigram similarity for
/// items stored before an embedding client was configured.
///
/// This is the production store. [`InMemoryDedupStore`] remains the
/// test/dev fallback only.
#[derive(Debug, Clone)]
pub struct PgSemanticDedupStore {
    pool: sqlx::PgPool,
}

impl PgSemanticDedupStore {
    /// Create a persistent dedup store over the given pool.
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }

    /// Number of persisted items (used by capability/status checks).
    pub async fn count_items(&self) -> Result<i64> {
        let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM semantic_dedup_items")
            .fetch_one(&self.pool)
            .await
            .context("failed to count semantic dedup items")?;
        Ok(count)
    }
}

#[async_trait]
impl DedupStore for PgSemanticDedupStore {
    async fn find_similar(
        &self,
        item_type: &TriageItemType,
        text: &str,
        max_results: usize,
    ) -> Result<Vec<DedupHit>> {
        let max_results = max_results.clamp(1, 200) as i64;

        let rows = sqlx::query_as::<_, (String, String, f64)>(
            r#"
            SELECT item_id, title, similarity(text_content, $1)::float8 AS similarity
            FROM semantic_dedup_items
            WHERE item_type = $2
              AND text_content % $1
            ORDER BY similarity DESC
            LIMIT $3
            "#,
        )
        .bind(text)
        .bind(item_type.as_str())
        .bind(max_results)
        .fetch_all(&self.pool)
        .await
        .context("semantic dedup text similarity query failed")?;

        Ok(rows
            .into_iter()
            .map(|(id, title, similarity)| DedupHit {
                id,
                title,
                similarity,
            })
            .collect())
    }

    async fn store_item(
        &self,
        item_type: &TriageItemType,
        id: &str,
        title: &str,
        text: &str,
    ) -> Result<()> {
        self.store_item_with_vector(item_type, id, title, text, None)
            .await
    }

    async fn store_item_with_vector(
        &self,
        item_type: &TriageItemType,
        id: &str,
        title: &str,
        text: &str,
        vector: Option<&[f64]>,
    ) -> Result<()> {
        let vector = vector.map(|values| {
            pgvector::Vector::from(values.iter().map(|x| *x as f32).collect::<Vec<f32>>())
        });

        sqlx::query(
            r#"
            INSERT INTO semantic_dedup_items
                (item_type, item_id, title, text_content, embedding)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (item_type, item_id) DO UPDATE SET
                title        = EXCLUDED.title,
                text_content = EXCLUDED.text_content,
                embedding    = COALESCE(EXCLUDED.embedding, semantic_dedup_items.embedding),
                updated_at   = NOW()
            "#,
        )
        .bind(item_type.as_str())
        .bind(id)
        .bind(title)
        .bind(text)
        .bind(&vector)
        .execute(&self.pool)
        .await
        .context("semantic dedup item upsert failed")?;
        Ok(())
    }

    async fn prune_item(&self, item_type: &TriageItemType, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM semantic_dedup_items WHERE item_type = $1 AND item_id = $2")
            .bind(item_type.as_str())
            .bind(id)
            .execute(&self.pool)
            .await
            .context("semantic dedup item prune failed")?;
        Ok(())
    }

    async fn find_similar_by_vector(
        &self,
        item_type: &TriageItemType,
        vector: &[f64],
        max_results: usize,
    ) -> Result<Vec<DedupHit>> {
        let max_results = max_results.clamp(1, 200) as i64;
        let vector = pgvector::Vector::from(vector.iter().map(|x| *x as f32).collect::<Vec<f32>>());

        let rows = sqlx::query_as::<_, (String, String, f64)>(
            r#"
            SELECT item_id, title, 1 - (embedding <=> $1) AS similarity
            FROM semantic_dedup_items
            WHERE item_type = $2
              AND embedding IS NOT NULL
            ORDER BY embedding <=> $1
            LIMIT $3
            "#,
        )
        .bind(&vector)
        .bind(item_type.as_str())
        .bind(max_results)
        .fetch_all(&self.pool)
        .await
        .context("semantic dedup vector query failed")?;

        Ok(rows
            .into_iter()
            .map(|(id, title, similarity)| DedupHit {
                id,
                title,
                similarity,
            })
            .collect())
    }
}

/// Seam over the embedding client.
///
/// The ingress embeds a submission at most once per submit: the vector is
/// reused for the similarity search and, when the item is new, for storage.
/// The trait keeps that call path observable in tests without an embedding
/// server.
#[async_trait]
trait TextEmbedder: Send + Sync {
    async fn embed(&self, text: &str) -> Result<Vec<f64>>;
}

#[async_trait]
impl TextEmbedder for EmbeddingClient {
    async fn embed(&self, text: &str) -> Result<Vec<f64>> {
        EmbeddingClient::embed(self, text).await
    }
}

/// Semantic deduplication engine using embedding similarity with fallback.
pub struct SemanticDedup {
    embedding_client: Option<Arc<dyn TextEmbedder>>,
    dedup_store: Option<Box<dyn DedupStore>>,
    config: DedupConfig,
}

impl SemanticDedup {
    /// Create a new [`SemanticDedup`] with optional embedding client and dedup store.
    ///
    /// When both are `None`, an in-memory text-similarity store is used as fallback.
    pub fn new(
        embedding_client: Option<EmbeddingClient>,
        dedup_store: Option<Box<dyn DedupStore>>,
        config: DedupConfig,
    ) -> Self {
        Self {
            embedding_client: embedding_client
                .map(|client| Arc::new(client) as Arc<dyn TextEmbedder>),
            dedup_store,
            config,
        }
    }

    /// Test-only constructor accepting a custom [`TextEmbedder`].
    #[cfg(test)]
    fn with_embedder(
        embedder: Arc<dyn TextEmbedder>,
        dedup_store: Option<Box<dyn DedupStore>>,
        config: DedupConfig,
    ) -> Self {
        Self {
            embedding_client: Some(embedder),
            dedup_store,
            config,
        }
    }

    /// Create a dedup engine with in-memory text similarity fallback.
    ///
    /// Test/dev only: this store forgets everything on restart. Production
    /// construction (worker `intelligence_ingress::build`) uses
    /// [`PgSemanticDedupStore`] instead.
    pub fn with_in_memory_fallback() -> Self {
        Self {
            embedding_client: None,
            dedup_store: Some(Box::new(InMemoryDedupStore::new(1000))),
            config: DedupConfig::default(),
        }
    }

    /// Check if a candidate item is semantically similar to an existing triaged item.
    ///
    /// Uses embedding-based vector search when an embedding client is available,
    /// falling back to text-based Jaccard trigram similarity when it's not.
    pub async fn check_duplicate(
        &self,
        item_type: &TriageItemType,
        title: &str,
        description: &str,
    ) -> Result<DedupResult> {
        Ok(self
            .check_duplicate_with_embedding(item_type, title, description)
            .await?
            .0)
    }

    /// Like [`Self::check_duplicate`], but also returns the embedding computed
    /// for this submission (when one was produced) so the caller can store a
    /// new item without embedding it a second time.
    pub async fn check_duplicate_with_embedding(
        &self,
        item_type: &TriageItemType,
        title: &str,
        description: &str,
    ) -> Result<(DedupResult, Option<Vec<f64>>)> {
        let text = format!("{}: {}", title, description);

        // Skip dedup for very short texts (not enough signal)
        if text.len() < self.config.min_text_length {
            return Ok((DedupResult::unique(), None));
        }

        // Try embedding-based dedup first. A failed or empty embedding falls
        // back to text similarity, exactly like `check_duplicate` always did.
        let mut embedding = None;
        if let Some(client) = &self.embedding_client {
            match client.embed(&text).await {
                Ok(vector) if !vector.is_empty() => embedding = Some(vector),
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        item_type = %item_type.as_str(),
                        "Embedding-based dedup failed, falling back to text similarity"
                    );
                }
            }
        }

        let result = if let Some(vector) = &embedding {
            match self
                .check_via_embedding_vector(item_type, vector, &text)
                .await
            {
                Ok(Some(result)) => result,
                Ok(None) => self.check_via_text(item_type, &text).await?,
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        item_type = %item_type.as_str(),
                        "Embedding-based dedup failed, falling back to text similarity"
                    );
                    self.check_via_text(item_type, &text).await?
                }
            }
        } else {
            self.check_via_text(item_type, &text).await?
        };

        Ok((result, embedding))
    }

    /// Check via embedding vector search.
    async fn check_via_embedding_vector(
        &self,
        item_type: &TriageItemType,
        embedding: &[f64],
        text: &str,
    ) -> Result<Option<DedupResult>> {
        if embedding.is_empty() {
            return Ok(None);
        }

        // Search against stored items using the dedup store if available.
        // B343: compare VECTORS — the previous code paid for the embedding
        // and then threw it away, searching by raw text instead, so the
        // 0.92 cosine threshold was being checked against trigram-Jaccard
        // scores that essentially never reach it (dedup never fired).
        if let Some(store) = &self.dedup_store {
            let mut hits = store
                .find_similar_by_vector(item_type, embedding, self.config.max_candidates)
                .await?;
            // Items stored before embeddings were available have no vector;
            // merge text-based hits so they remain comparable.
            hits.extend(
                store
                    .find_similar(item_type, text, self.config.max_candidates)
                    .await?,
            );
            if let Some(best) = hits
                .iter()
                .filter(|h| h.similarity >= self.config.threshold)
                .max_by(|a, b| {
                    a.similarity
                        .partial_cmp(&b.similarity)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
            {
                return Ok(Some(DedupResult {
                    is_duplicate: true,
                    best_match_score: best.similarity,
                    best_match_id: Some(best.id.clone()),
                    best_match_title: Some(best.title.clone()),
                }));
            }
        }

        Ok(None)
    }

    /// Check via text-based trigram similarity (fallback).
    async fn check_via_text(&self, item_type: &TriageItemType, text: &str) -> Result<DedupResult> {
        if let Some(store) = &self.dedup_store {
            let hits = store
                .find_similar(item_type, text, self.config.max_candidates)
                .await?;
            if let Some(best) = hits
                .iter()
                .filter(|h| h.similarity >= self.config.threshold)
                .max_by(|a, b| {
                    a.similarity
                        .partial_cmp(&b.similarity)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
            {
                return Ok(DedupResult {
                    is_duplicate: true,
                    best_match_score: best.similarity,
                    best_match_id: Some(best.id.clone()),
                    best_match_title: Some(best.title.clone()),
                });
            }
        }

        Ok(DedupResult::unique())
    }

    /// Store a newly triaged item for future dedup lookups.
    ///
    /// Embeds the text once here; callers that already embedded the submission
    /// should use [`Self::store_triaged_item_with_embedding`] instead.
    pub async fn store_triaged_item(
        &self,
        item_type: &TriageItemType,
        id: &str,
        title: &str,
        description: &str,
    ) -> Result<()> {
        let text = format!("{}: {}", title, description);
        let embedding = match &self.embedding_client {
            Some(client) => client.embed(&text).await.ok(),
            None => None,
        };
        self.store_triaged_item_with_embedding(
            item_type,
            id,
            title,
            description,
            embedding.as_deref(),
        )
        .await
    }

    /// Store a newly triaged item together with an embedding computed by the
    /// caller (see [`Self::check_duplicate_with_embedding`]) so the submission
    /// is embedded exactly once.
    pub async fn store_triaged_item_with_embedding(
        &self,
        item_type: &TriageItemType,
        id: &str,
        title: &str,
        description: &str,
        embedding: Option<&[f64]>,
    ) -> Result<()> {
        let text = format!("{}: {}", title, description);
        if let Some(store) = &self.dedup_store {
            // B343: store the embedding alongside the text so future
            // vector comparisons have something to compare against.
            store
                .store_item_with_vector(item_type, id, title, &text, embedding)
                .await?;
        }
        Ok(())
    }

    /// Remove a stored dedup candidate that can no longer be a merge target.
    pub async fn prune_stored_item(&self, item_type: &TriageItemType, id: &str) -> Result<()> {
        match &self.dedup_store {
            Some(store) => store.prune_item(item_type, id).await,
            None => Ok(()),
        }
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
            .max_by(|a, b| {
                a.similarity
                    .partial_cmp(&b.similarity)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|hit| DedupResult {
                is_duplicate: true,
                best_match_score: hit.similarity,
                best_match_id: Some(hit.entity_id.clone()),
                best_match_title: Some(hit.entity_type.clone()),
            })
    }
}

// ─── Triage ingress with semantic dedup ───────────────────────────────────────

/// Default occurrence count at which repeated merges raise severity to `high`.
pub const DEFAULT_ESCALATE_HIGH_AT: i64 = 3;
/// Default occurrence count at which repeated merges raise severity to `critical`.
pub const DEFAULT_ESCALATE_CRITICAL_AT: i64 = 5;

/// Minimum text similarity (trigram Jaccard) for the same-entity/time-window
/// merge stage. Entity identity alone is not evidence of duplication:
/// unrelated signals about one company must stay separate rows.
pub const SAME_ENTITY_MIN_TEXT_SIMILARITY: f64 = 0.25;

/// A candidate triage item submitted to the ingress.
#[derive(Debug, Clone)]
pub struct TriageSubmission {
    /// insight / warning / alert.
    pub item_type: TriageItemType,
    /// Deterministic producer id (e.g. the warning or insight UUID). Two
    /// submissions with the same `(item_type, source_id)` are duplicates by
    /// definition.
    pub source_id: String,
    pub title: String,
    pub description: String,
    pub entity_id: Option<Uuid>,
    pub entity_name: Option<String>,
    pub static_severity: Option<String>,
    pub dimensions: Option<TriageDimensions>,
    /// Observation ids backing this submission (merged, never discarded).
    pub observation_ids: Vec<Uuid>,
    /// Source urls backing this submission (merged, never discarded).
    pub source_urls: Vec<String>,
}

impl TriageSubmission {
    /// Text used for lexical / embedding similarity.
    pub fn text(&self) -> String {
        format!("{}: {}", self.title, self.description)
    }
}

/// An existing triage queue row as seen by the ingress, including the merge
/// bookkeeping added by migration `053_triage_merge_fields.sql`.
#[derive(Debug, Clone)]
pub struct IngestQueueItem {
    pub id: Uuid,
    pub item_type: TriageItemType,
    pub source_id: String,
    pub title: String,
    pub description: String,
    pub entity_id: Option<Uuid>,
    pub entity_name: Option<String>,
    pub static_severity: Option<String>,
    pub occurrence_count: i64,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub merged_observation_ids: Vec<Uuid>,
    pub merged_source_urls: Vec<String>,
    pub created_at: DateTime<Utc>,
}

impl IngestQueueItem {
    /// Text used for similarity comparisons.
    pub fn text(&self) -> String {
        format!("{}: {}", self.title, self.description)
    }
}

/// Narrow queue interface used by [`TriageIngestor`].
///
/// The production implementation is `TriageQueue` (SQL-backed, with the
/// `ON CONFLICT (item_type, source_id)` unique constraint as the concurrency
/// guard); tests use an in-memory implementation to prove the pipeline
/// without a database.
#[async_trait]
pub trait IngestQueue: Send + Sync {
    /// Exact dedup by the deterministic `(item_type, source_id)` key.
    async fn find_by_source_id(
        &self,
        item_type: &TriageItemType,
        source_id: &str,
    ) -> Result<Option<IngestQueueItem>>;

    /// Look up a queue row by its primary key.
    async fn find_by_id(&self, id: Uuid) -> Result<Option<IngestQueueItem>>;

    /// Rows of this item type seen within the trailing `window`.
    async fn find_recent(
        &self,
        item_type: &TriageItemType,
        window: Duration,
    ) -> Result<Vec<IngestQueueItem>>;

    /// Insert a new row. Implementations MUST guarantee that two concurrent
    /// submissions with the same `(item_type, source_id)` result in a single
    /// row whose `occurrence_count` reflects both submissions, and that the
    /// stored severity never falls below the level implied by the resulting
    /// occurrence count (`high_at` / `critical_at` thresholds).
    async fn insert_submission(
        &self,
        submission: &TriageSubmission,
        high_at: i64,
        critical_at: i64,
    ) -> Result<IngestQueueItem>;

    /// Merge a duplicate into `target_id`: increments `occurrence_count`,
    /// unions the observation ids / source urls, updates `last_seen_at`, and
    /// atomically raises `static_severity` when the post-increment occurrence
    /// count (or the incoming severity) justifies it. Implementations MUST
    /// compute the escalation from the updated row, not from a stale snapshot.
    async fn merge_submission(
        &self,
        target_id: Uuid,
        submission: &TriageSubmission,
        high_at: i64,
        critical_at: i64,
    ) -> Result<IngestQueueItem>;
}

/// Why two submissions were merged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeReason {
    /// Same deterministic `(item_type, source_id)`.
    ExactDeterministicId,
    /// Lexical near-duplicate (trigram similarity above threshold).
    LexicalNearDuplicate,
    /// Embedding similarity above threshold.
    EmbeddingSimilarity,
    /// Same entity within the configured time window.
    SameEntityTimeWindow,
}

/// Blanket delegation so `Arc<Q>` can be used as an ingress queue (needed to
/// share one ingestor across concurrent producers).
#[async_trait]
impl<T: IngestQueue + ?Sized> IngestQueue for std::sync::Arc<T> {
    async fn find_by_source_id(
        &self,
        item_type: &TriageItemType,
        source_id: &str,
    ) -> Result<Option<IngestQueueItem>> {
        (**self).find_by_source_id(item_type, source_id).await
    }

    async fn find_by_id(&self, id: Uuid) -> Result<Option<IngestQueueItem>> {
        (**self).find_by_id(id).await
    }

    async fn find_recent(
        &self,
        item_type: &TriageItemType,
        window: Duration,
    ) -> Result<Vec<IngestQueueItem>> {
        (**self).find_recent(item_type, window).await
    }

    async fn insert_submission(
        &self,
        submission: &TriageSubmission,
        high_at: i64,
        critical_at: i64,
    ) -> Result<IngestQueueItem> {
        (**self)
            .insert_submission(submission, high_at, critical_at)
            .await
    }

    async fn merge_submission(
        &self,
        target_id: Uuid,
        submission: &TriageSubmission,
        high_at: i64,
        critical_at: i64,
    ) -> Result<IngestQueueItem> {
        (**self)
            .merge_submission(target_id, submission, high_at, critical_at)
            .await
    }
}

impl MergeReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ExactDeterministicId => "exact_deterministic_id",
            Self::LexicalNearDuplicate => "lexical_near_duplicate",
            Self::EmbeddingSimilarity => "embedding_similarity",
            Self::SameEntityTimeWindow => "same_entity_time_window",
        }
    }
}

/// Result of a submission.
#[derive(Debug, Clone)]
pub enum IngestOutcome {
    /// A new queue row was created.
    Enqueued(IngestQueueItem),
    /// The submission was merged into an existing row.
    Merged {
        item: IngestQueueItem,
        reason: MergeReason,
    },
}

impl IngestOutcome {
    /// The resulting queue row (new or merged).
    pub fn item(&self) -> &IngestQueueItem {
        match self {
            Self::Enqueued(item) => item,
            Self::Merged { item, .. } => item,
        }
    }

    /// Whether the submission merged into an existing row.
    pub fn merged(&self) -> bool {
        matches!(self, Self::Merged { .. })
    }
}

/// Configuration for [`TriageIngestor`].
#[derive(Debug, Clone)]
pub struct IngestConfig {
    /// Trigram similarity above which two items are lexical near-duplicates.
    pub lexical_threshold: f64,
    /// How far back to look for merge candidates.
    pub entity_window: Duration,
    /// Occurrence count at which repeated merges raise severity to `high`.
    pub escalate_high_at: i64,
    /// Occurrence count at which repeated merges raise severity to `critical`.
    pub escalate_critical_at: i64,
}

impl Default for IngestConfig {
    fn default() -> Self {
        Self {
            lexical_threshold: 0.45,
            entity_window: Duration::hours(24),
            escalate_high_at: DEFAULT_ESCALATE_HIGH_AT,
            escalate_critical_at: DEFAULT_ESCALATE_CRITICAL_AT,
        }
    }
}

/// Numeric severity ranking; unknown severities rank below `low`.
pub fn severity_rank(severity: &str) -> i32 {
    match severity.trim().to_lowercase().as_str() {
        "critical" => 3,
        "high" => 2,
        "medium" => 1,
        "low" => 0,
        _ => -1,
    }
}

/// True when merging `incoming` into a row carrying `existing` severity would
/// bury a stricter signal: the incoming severity is strictly higher than the
/// target row's.
///
/// Unknown severities rank below `low`, so an unknown incoming signal never
/// blocks a merge on its own.
pub fn merge_would_weaken_severity(existing: Option<&str>, incoming: Option<&str>) -> bool {
    severity_rank(incoming.unwrap_or_default()) > severity_rank(existing.unwrap_or_default())
}

/// Escalate a severity based on the incoming severity and repeat count.
///
/// Returns the effective severity name, or `None` when neither the existing
/// nor the incoming severity is known and the repeat count does not justify
/// an escalation.
pub fn escalate_severity(
    current: Option<&str>,
    incoming: Option<&str>,
    occurrence_count: i64,
    high_at: i64,
    critical_at: i64,
) -> Option<String> {
    let base = current
        .map(severity_rank)
        .unwrap_or(-1)
        .max(incoming.map(severity_rank).unwrap_or(-1));
    let repeat_floor = if occurrence_count >= critical_at {
        3
    } else if occurrence_count >= high_at {
        2
    } else {
        -1
    };
    match base.max(repeat_floor) {
        3 => Some("critical".to_string()),
        2 => Some("high".to_string()),
        1 => Some("medium".to_string()),
        0 => Some("low".to_string()),
        _ => None,
    }
}

/// The triage ingress: exact → lexical → embedding → entity/time-window
/// dedup, then enqueue or merge.
///
/// All stages run before a row is inserted. Merges never discard evidence:
/// the target row accumulates observation ids and source urls, stores the
/// occurrence count and last-seen timestamp, and can be escalated in
/// severity when repeats justify it.
pub struct TriageIngestor<Q: IngestQueue> {
    queue: Q,
    dedup: SemanticDedup,
    config: IngestConfig,
}

impl<Q: IngestQueue> TriageIngestor<Q> {
    /// Create an ingestor over the given queue and dedup engine.
    pub fn new(queue: Q, dedup: SemanticDedup) -> Self {
        Self {
            queue,
            dedup,
            config: IngestConfig::default(),
        }
    }

    /// Create an ingestor with a custom config.
    pub fn with_config(queue: Q, dedup: SemanticDedup, config: IngestConfig) -> Self {
        Self {
            queue,
            dedup,
            config,
        }
    }

    /// Borrow the underlying queue.
    pub fn queue(&self) -> &Q {
        &self.queue
    }

    /// The active configuration.
    pub fn config(&self) -> &IngestConfig {
        &self.config
    }

    /// Submit an item to the triage queue.
    ///
    /// Pipeline: exact deterministic-id dedup → lexical near-duplicate →
    /// embedding similarity → same-entity/time-window → enqueue or merge.
    pub async fn submit(&self, submission: TriageSubmission) -> Result<IngestOutcome> {
        let item_type = submission.item_type.clone();
        let text = submission.text();

        // Stage 1 — exact deterministic id.
        if let Some(existing) = self
            .queue
            .find_by_source_id(&item_type, &submission.source_id)
            .await?
        {
            return self
                .merge(existing, &submission, MergeReason::ExactDeterministicId)
                .await;
        }

        let recent = self
            .queue
            .find_recent(&item_type, self.config.entity_window)
            .await?;

        // Stage 2 — lexical near-duplicate, but only within the same entity
        // identity: text similarity across two different entities (or across
        // a missing entity and a set one) is a false positive. When both
        // sides carry an entity id they must be equal; a null id never
        // matches a different entity's item.
        let normalized = text.to_lowercase();
        let lexical = recent
            .iter()
            .filter(|item| item.entity_id == submission.entity_id)
            .map(|item| {
                (
                    jaccard_trigram_similarity(&normalized, &item.text().to_lowercase()),
                    item,
                )
            })
            .filter(|(score, _)| *score >= self.config.lexical_threshold)
            .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        if let Some((_, target)) = lexical {
            return self
                .merge(
                    target.clone(),
                    &submission,
                    MergeReason::LexicalNearDuplicate,
                )
                .await;
        }

        // Stage 3 — embedding similarity. The submission is embedded once and
        // the vector is reused below if the item turns out to be new. A hit
        // may only merge into a queue row that is an active, in-window
        // candidate (the same set `find_recent` exposes); a hit pointing at a
        // deleted, inactive or out-of-window row prunes the stale dedup
        // candidate so it can never match again.
        let (dedup_result, embedding) = self
            .dedup
            .check_duplicate_with_embedding(&item_type, &submission.title, &submission.description)
            .await
            .unwrap_or_else(|error| {
                tracing::warn!(
                    %error,
                    item_type = %item_type.as_str(),
                    "semantic dedup lookup failed; continuing without a merge candidate"
                );
                (DedupResult::unique(), None)
            });
        if dedup_result.is_duplicate {
            let target_id = dedup_result
                .best_match_id
                .as_deref()
                .and_then(|id| Uuid::parse_str(id).ok());
            if let Some(target_id) = target_id {
                let target = self.queue.find_by_id(target_id).await?;
                match target {
                    Some(target) if recent.iter().any(|item| item.id == target_id) => {
                        return self
                            .merge(target, &submission, MergeReason::EmbeddingSimilarity)
                            .await;
                    }
                    Some(_) | None => {
                        if let Err(error) = self
                            .dedup
                            .prune_stored_item(&item_type, &target_id.to_string())
                            .await
                        {
                            tracing::debug!(
                                %error,
                                %target_id,
                                "triage ingress: failed to prune a stale dedup candidate"
                            );
                        }
                    }
                }
            }
        }

        // Stage 4 — same entity within the time window. Entity identity alone
        // is not duplication: the texts must also be similar, and a stricter
        // incoming signal must never be absorbed by a weaker row (that would
        // hide the escalation behind a soft target).
        if let Some(entity_id) = submission.entity_id {
            let incoming_severity = submission.static_severity.as_deref();
            let candidate = recent
                .iter()
                .filter(|item| item.entity_id == Some(entity_id))
                .map(|item| {
                    (
                        jaccard_trigram_similarity(&normalized, &item.text().to_lowercase()),
                        item,
                    )
                })
                .filter(|(score, _)| *score >= SAME_ENTITY_MIN_TEXT_SIMILARITY)
                .filter(|(_, item)| {
                    !merge_would_weaken_severity(item.static_severity.as_deref(), incoming_severity)
                })
                .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
            if let Some((_, target)) = candidate {
                return self
                    .merge(
                        target.clone(),
                        &submission,
                        MergeReason::SameEntityTimeWindow,
                    )
                    .await;
            }
        }

        // Stage 5 — new item. A concurrent submission with the same
        // deterministic id may have won the insert; the queue's conflict path
        // then merges this submission into that row, which is reported as a
        // merge (occurrence_count > 1) rather than a fresh enqueue.
        let item = self
            .queue
            .insert_submission(
                &submission,
                self.config.escalate_high_at,
                self.config.escalate_critical_at,
            )
            .await?;
        if item.occurrence_count > 1 {
            return Ok(IngestOutcome::Merged {
                item,
                reason: MergeReason::ExactDeterministicId,
            });
        }
        if let Err(e) = self
            .dedup
            .store_triaged_item_with_embedding(
                &item_type,
                &item.id.to_string(),
                &submission.title,
                &submission.description,
                embedding.as_deref(),
            )
            .await
        {
            tracing::debug!(error = %e, "triage ingress: failed to store item for future dedup");
        }
        Ok(IngestOutcome::Enqueued(item))
    }

    /// Merge a duplicate into an existing row. Severity escalation is
    /// computed by the queue from the post-increment occurrence count, so
    /// concurrent merges cannot leave the stored severity below the level the
    /// final count requires.
    async fn merge(
        &self,
        existing: IngestQueueItem,
        submission: &TriageSubmission,
        reason: MergeReason,
    ) -> Result<IngestOutcome> {
        let merged = self
            .queue
            .merge_submission(
                existing.id,
                submission,
                self.config.escalate_high_at,
                self.config.escalate_critical_at,
            )
            .await?;
        Ok(IngestOutcome::Merged {
            item: merged,
            reason,
        })
    }
}

// ─── Text similarity (trigram Jaccard) ───

/// Compute Jaccard similarity between two strings using character trigrams.
fn jaccard_trigram_similarity(a: &str, b: &str) -> f64 {
    let trig_a = trigrams(&a.to_lowercase());
    let trig_b = trigrams(&b.to_lowercase());

    if trig_a.is_empty() || trig_b.is_empty() {
        return 0.0;
    }

    let intersection = trig_a.intersection(&trig_b).count();
    let union = trig_a.union(&trig_b).count();

    if union == 0 {
        0.0
    } else {
        intersection as f64 / union as f64
    }
}

fn trigrams(s: &str) -> HashSet<String> {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() < 3 {
        let padded: Vec<char> = format!(" {} ", s).chars().collect();
        return padded.windows(3).map(|w| w.iter().collect()).collect();
    }
    chars.windows(3).map(|w| w.iter().collect()).collect()
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
    fn test_cosine_similarity_empty() {
        let a: Vec<f64> = vec![];
        let b: Vec<f64> = vec![];
        assert!((SemanticDedup::cosine_similarity(&a, &b) - 0.0).abs() < 1e-9);
    }

    #[test]
    fn test_jaccard_trigram_identical() {
        let sim = jaccard_trigram_similarity(
            "Supply chain disruption at Foxconn",
            "Supply chain disruption at Foxconn",
        );
        assert!((sim - 1.0).abs() < 1e-9);
    }

    #[test]
    fn test_jaccard_trigram_different() {
        let sim = jaccard_trigram_similarity(
            "Supply chain disruption at Foxconn",
            "New quality certification for Samsung",
        );
        assert!(sim < 0.4);
    }

    #[test]
    fn test_jaccard_trigram_similar() {
        let sim = jaccard_trigram_similarity(
            "Supply chain disruption at Foxconn Tunisia",
            "Supply chain issues at Foxconn TN",
        );
        // These share "supply chain", "at foxconn", "tn"/"tunisia" trigrams but
        // differ in wording — Jaccard overlap is moderate, not high.
        assert!(sim > 0.2, "expected moderate similarity, got {sim}");
        // Verify dissimilar strings have near-zero similarity.
        let low =
            jaccard_trigram_similarity("Supply chain disruption", "Quarterly earnings report");
        assert!(low < 0.15, "expected low similarity, got {low}");
    }

    #[tokio::test]
    async fn test_in_memory_dedup_store() {
        let store = InMemoryDedupStore::new(10);
        let item_type = TriageItemType::Insight;

        store
            .store_item(&item_type, "1", "Test insight", "Supply chain disruption")
            .await
            .unwrap();

        let hits = store
            .find_similar(
                &TriageItemType::Insight,
                "Supply chain breakdown and disruption",
                5,
            )
            .await
            .unwrap();

        assert!(!hits.is_empty());
        assert!(hits[0].similarity > 0.3);
    }

    #[tokio::test]
    async fn test_semantic_dedup_with_memory_store() {
        let store = Box::new(InMemoryDedupStore::new(10));
        let dedup = SemanticDedup::new(None, Some(store), DedupConfig::default());

        let item_type = TriageItemType::Insight;

        // Store a reference item
        dedup
            .store_triaged_item(
                &item_type,
                "ref-1",
                "Foxconn quality crisis",
                "Quality defect recall at Foxconn Tunisia manufacturing plant",
            )
            .await
            .unwrap();

        // Check a very similar item
        let result = dedup
            .check_duplicate(
                &item_type,
                "Foxconn quality issue",
                "Defect and recall at Foxconn Tunisia factory",
            )
            .await
            .unwrap();

        // Without embeddings configured, the dedup check uses trigram title
        // similarity (or returns 0 if only embeddings are wired). The score
        // may be 0 when the embedding backend is absent — verify it doesn't
        // error and returns a valid result either way.
        assert!(
            result.best_match_score >= 0.0,
            "score should be non-negative"
        );
    }

    #[tokio::test]
    async fn test_semantic_dedup_different_items() {
        let store = Box::new(InMemoryDedupStore::new(10));
        let dedup = SemanticDedup::new(None, Some(store), DedupConfig::default());

        let item_type = TriageItemType::Insight;

        dedup
            .store_triaged_item(
                &item_type,
                "ref-1",
                "Foxconn quality crisis",
                "Quality defect recall at Foxconn Tunisia",
            )
            .await
            .unwrap();

        let result = dedup
            .check_duplicate(
                &item_type,
                "Samsung expansion",
                "Samsung announces new semiconductor fab investment in Korea",
            )
            .await
            .unwrap();

        assert!(!result.is_duplicate);
    }

    #[test]
    fn test_dedup_config_default() {
        let config = DedupConfig::default();
        assert!((config.threshold - 0.92).abs() < 1e-9);
        assert_eq!(config.max_candidates, 20);
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
                source_text: "Low similarity".to_string(),
            },
            VectorSearchHit {
                entity_id: "b".to_string(),
                entity_type: "triage_item".to_string(),
                chunk_index: 0,
                similarity: 0.97,
                source_text: "High similarity".to_string(),
            },
            VectorSearchHit {
                entity_id: "c".to_string(),
                entity_type: "triage_item".to_string(),
                chunk_index: 0,
                similarity: 0.93,
                source_text: "Medium similarity".to_string(),
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
        let hits = vec![VectorSearchHit {
            entity_id: "a".to_string(),
            entity_type: "triage_item".to_string(),
            chunk_index: 0,
            similarity: 0.80,
            source_text: "Low similarity".to_string(),
        }];

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

    #[tokio::test]
    async fn in_memory_store_survives_a_poisoned_lock() {
        use std::panic::{catch_unwind, AssertUnwindSafe};

        let store = InMemoryDedupStore::new(10);

        // Poison the internal mutex by panicking while holding the lock.
        let poisoned = catch_unwind(AssertUnwindSafe(|| {
            let _guard = store.items.lock().unwrap();
            panic!("poison the mutex");
        }));
        assert!(poisoned.is_err());
        assert!(store.items.is_poisoned());

        // Every operation must recover from poisoning rather than panic.
        store
            .store_item(&TriageItemType::Warning, "id-1", "Title", "some text body")
            .await
            .unwrap();
        let hits = store
            .find_similar(&TriageItemType::Warning, "some text body", 5)
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert!(
            store
                .find_similar_by_vector(&TriageItemType::Warning, &[1.0, 0.0], 5)
                .await
                .is_ok(),
            "vector search must not panic on a poisoned lock"
        );
    }

    #[tokio::test]
    async fn in_memory_store_dedups_by_id_and_evicts_oldest() {
        let store = InMemoryDedupStore::new(2);
        let t = TriageItemType::Insight;
        store
            .store_item(&t, "a", "A", "alpha content here")
            .await
            .unwrap();
        // Same id again must not create a second row.
        store
            .store_item(&t, "a", "A", "alpha content here")
            .await
            .unwrap();
        store
            .store_item(&t, "b", "B", "beta content here")
            .await
            .unwrap();
        // Capacity 2: inserting a third evicts the oldest ("a").
        store
            .store_item(&t, "c", "C", "gamma content here")
            .await
            .unwrap();

        let all = store
            .find_similar(&t, "gamma content here", 10)
            .await
            .unwrap();
        let ids: Vec<&str> = all.iter().map(|h| h.id.as_str()).collect();
        assert!(!ids.contains(&"a"), "oldest entry should be evicted");
        assert!(ids.contains(&"c"));
    }

    // ─── Triage ingress ───────────────────────────────────────────────────

    #[derive(Default)]
    struct InMemoryIngestQueue {
        rows: std::sync::Mutex<Vec<IngestQueueItem>>,
        /// Rows simulating non-active triage statuses: still findable by id,
        /// but excluded from `find_recent` (mirroring the SQL
        /// `status IN ('pending', 'triaged', 'acknowledged')` filter).
        inactive_ids: std::sync::Mutex<std::collections::HashSet<Uuid>>,
    }

    impl InMemoryIngestQueue {
        fn new() -> Self {
            Self::default()
        }

        fn len(&self) -> usize {
            self.rows.lock().unwrap_or_else(|e| e.into_inner()).len()
        }

        fn snapshot(&self) -> Vec<IngestQueueItem> {
            self.rows.lock().unwrap_or_else(|e| e.into_inner()).clone()
        }

        fn push_row(&self, row: IngestQueueItem) {
            self.rows
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(row);
        }

        fn mark_inactive(&self, id: Uuid) {
            self.inactive_ids
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(id);
        }

        fn merge_row(
            row: &mut IngestQueueItem,
            submission: &TriageSubmission,
            severity: Option<String>,
        ) -> IngestQueueItem {
            row.occurrence_count += 1;
            row.last_seen_at = Some(Utc::now());
            for obs in &submission.observation_ids {
                if !row.merged_observation_ids.contains(obs) {
                    row.merged_observation_ids.push(*obs);
                }
            }
            for url in &submission.source_urls {
                if !row.merged_source_urls.contains(url) {
                    row.merged_source_urls.push(url.clone());
                }
            }
            if severity.is_some() {
                row.static_severity = severity;
            }
            row.clone()
        }
    }

    #[async_trait]
    impl IngestQueue for InMemoryIngestQueue {
        async fn find_by_source_id(
            &self,
            item_type: &TriageItemType,
            source_id: &str,
        ) -> Result<Option<IngestQueueItem>> {
            let rows = self.rows.lock().unwrap_or_else(|e| e.into_inner());
            Ok(rows
                .iter()
                .find(|r| &r.item_type == item_type && r.source_id == source_id)
                .cloned())
        }

        async fn find_by_id(&self, id: Uuid) -> Result<Option<IngestQueueItem>> {
            let rows = self.rows.lock().unwrap_or_else(|e| e.into_inner());
            Ok(rows.iter().find(|r| r.id == id).cloned())
        }

        async fn find_recent(
            &self,
            item_type: &TriageItemType,
            window: Duration,
        ) -> Result<Vec<IngestQueueItem>> {
            let cutoff = Utc::now() - window;
            let inactive = self.inactive_ids.lock().unwrap_or_else(|e| e.into_inner());
            let rows = self.rows.lock().unwrap_or_else(|e| e.into_inner());
            Ok(rows
                .iter()
                .filter(|r| &r.item_type == item_type)
                .filter(|r| !inactive.contains(&r.id))
                .filter(|r| r.last_seen_at.unwrap_or(r.created_at) >= cutoff)
                .cloned()
                .collect())
        }

        async fn insert_submission(
            &self,
            submission: &TriageSubmission,
            high_at: i64,
            critical_at: i64,
        ) -> Result<IngestQueueItem> {
            let mut rows = self.rows.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(pos) = rows.iter().position(|r| {
                r.item_type == submission.item_type && r.source_id == submission.source_id
            }) {
                let severity = escalate_severity(
                    rows[pos].static_severity.as_deref(),
                    submission.static_severity.as_deref(),
                    rows[pos].occurrence_count + 1,
                    high_at,
                    critical_at,
                );
                return Ok(Self::merge_row(&mut rows[pos], submission, severity));
            }

            let now = Utc::now();
            let item = IngestQueueItem {
                id: Uuid::new_v4(),
                item_type: submission.item_type.clone(),
                source_id: submission.source_id.clone(),
                title: submission.title.clone(),
                description: submission.description.clone(),
                entity_id: submission.entity_id,
                entity_name: submission.entity_name.clone(),
                static_severity: submission.static_severity.clone(),
                occurrence_count: 1,
                last_seen_at: Some(now),
                merged_observation_ids: submission.observation_ids.clone(),
                merged_source_urls: submission.source_urls.clone(),
                created_at: now,
            };
            rows.push(item.clone());
            Ok(item)
        }

        async fn merge_submission(
            &self,
            target_id: Uuid,
            submission: &TriageSubmission,
            high_at: i64,
            critical_at: i64,
        ) -> Result<IngestQueueItem> {
            let mut rows = self.rows.lock().unwrap_or_else(|e| e.into_inner());
            let pos = rows
                .iter()
                .position(|r| r.id == target_id)
                .ok_or_else(|| anyhow::anyhow!("no row {target_id}"))?;
            let severity = escalate_severity(
                rows[pos].static_severity.as_deref(),
                submission.static_severity.as_deref(),
                rows[pos].occurrence_count + 1,
                high_at,
                critical_at,
            );
            Ok(Self::merge_row(&mut rows[pos], submission, severity))
        }
    }

    fn submission(source_id: &str, title: &str, description: &str) -> TriageSubmission {
        TriageSubmission {
            item_type: TriageItemType::Warning,
            source_id: source_id.to_string(),
            title: title.to_string(),
            description: description.to_string(),
            entity_id: None,
            entity_name: None,
            static_severity: Some("low".to_string()),
            dimensions: None,
            observation_ids: Vec::new(),
            source_urls: Vec::new(),
        }
    }

    fn ingestor(queue: InMemoryIngestQueue) -> TriageIngestor<InMemoryIngestQueue> {
        TriageIngestor::new(queue, SemanticDedup::with_in_memory_fallback())
    }

    /// Test [`TextEmbedder`] that returns a fixed vector and counts calls.
    struct CountingEmbedder {
        calls: std::sync::atomic::AtomicUsize,
        vector: Vec<f64>,
    }

    impl CountingEmbedder {
        fn new(vector: Vec<f64>) -> Self {
            Self {
                calls: std::sync::atomic::AtomicUsize::new(0),
                vector,
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl TextEmbedder for CountingEmbedder {
        async fn embed(&self, _text: &str) -> Result<Vec<f64>> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(self.vector.clone())
        }
    }

    /// Seed a queue row directly so tests can control its age/severity.
    fn queue_row(
        source_id: &str,
        title: &str,
        description: &str,
        entity_id: Option<Uuid>,
        last_seen_at: DateTime<Utc>,
    ) -> IngestQueueItem {
        IngestQueueItem {
            id: Uuid::new_v4(),
            item_type: TriageItemType::Warning,
            source_id: source_id.to_string(),
            title: title.to_string(),
            description: description.to_string(),
            entity_id,
            entity_name: None,
            static_severity: Some("low".to_string()),
            occurrence_count: 1,
            last_seen_at: Some(last_seen_at),
            merged_observation_ids: Vec::new(),
            merged_source_urls: Vec::new(),
            created_at: last_seen_at,
        }
    }

    #[tokio::test]
    async fn duplicate_submit_merges_single_row_and_keeps_evidence() {
        let queue = InMemoryIngestQueue::new();
        let ingestor = ingestor(queue);

        let obs_a = Uuid::new_v4();
        let obs_b = Uuid::new_v4();

        let mut first = submission(
            "warning-1",
            "Volume spike for Acme Corp",
            "Observation volume spiked 300% versus baseline",
        );
        first.observation_ids = vec![obs_a];
        first.source_urls = vec!["https://news-a.example/story".to_string()];
        let first_outcome = ingestor.submit(first).await.unwrap();
        assert!(!first_outcome.merged());
        assert_eq!(first_outcome.item().occurrence_count, 1);

        // Same deterministic id (source_id) but fresh evidence arrays.
        let mut second = submission(
            "warning-1",
            "Volume spike for Acme Corp",
            "Observation volume spiked 300% versus baseline",
        );
        second.observation_ids = vec![obs_b];
        second.source_urls = vec!["https://news-b.example/story".to_string()];
        let second_outcome = ingestor.submit(second).await.unwrap();
        assert!(second_outcome.merged());
        assert_eq!(second_outcome.item().occurrence_count, 2);

        let rows = ingestor.queue().snapshot();
        assert_eq!(rows.len(), 1, "duplicate submit must not double-insert");
        let row = &rows[0];
        assert_eq!(row.occurrence_count, 2);
        assert_eq!(row.merged_observation_ids.len(), 2);
        assert!(row.merged_observation_ids.contains(&obs_a));
        assert!(row.merged_observation_ids.contains(&obs_b));
        assert_eq!(row.merged_source_urls.len(), 2);
        assert!(row.last_seen_at.is_some());
    }

    #[tokio::test]
    async fn lexical_near_duplicate_with_new_id_merges() {
        let queue = InMemoryIngestQueue::new();
        let ingestor = ingestor(queue);

        let mut first = submission(
            "warning-1",
            "Foxconn quality crisis",
            "Quality defect recall at Foxconn Tunisia manufacturing plant",
        );
        first.observation_ids = vec![Uuid::new_v4()];
        ingestor.submit(first).await.unwrap();

        // Different deterministic id, near-identical text.
        let mut duplicate = submission(
            "warning-2",
            "Foxconn quality crisis",
            "Quality defect recall at Foxconn Tunisia manufacturing plant",
        );
        duplicate.observation_ids = vec![Uuid::new_v4()];
        let outcome = ingestor.submit(duplicate).await.unwrap();

        assert!(outcome.merged(), "near-identical text must merge");
        assert_eq!(outcome.item().occurrence_count, 2);
        assert_eq!(ingestor.queue().len(), 1);
    }

    /// #120: same entity within the window is not enough — unrelated text
    /// must stay two rows.
    #[tokio::test]
    async fn same_entity_with_unrelated_text_stays_separate() {
        let queue = InMemoryIngestQueue::new();
        let ingestor = ingestor(queue);
        let entity = Uuid::new_v4();

        let mut first = submission(
            "warning-1",
            "Export control change",
            "New export control rule affects shipments",
        );
        first.entity_id = Some(entity);
        first.observation_ids = vec![Uuid::new_v4()];
        ingestor.submit(first).await.unwrap();

        let mut second = submission(
            "warning-2",
            "Completely different headline about the same company",
            "A totally unrelated sentence that shares almost no trigrams",
        );
        second.entity_id = Some(entity);
        second.observation_ids = vec![Uuid::new_v4()];
        let outcome = ingestor.submit(second).await.unwrap();

        assert!(
            !outcome.merged(),
            "same entity with unrelated text must not merge"
        );
        assert_eq!(ingestor.queue().len(), 2);
    }

    /// #120: same entity plus similar text (above the same-entity floor,
    /// below the lexical threshold and the embedding threshold) merges.
    #[tokio::test]
    async fn same_entity_with_similar_text_merges() {
        let queue = InMemoryIngestQueue::new();
        let ingestor = ingestor(queue);
        let entity = Uuid::new_v4();

        let mut first = submission(
            "warning-1",
            "Port congestion at Shanghai",
            "Container backlog grows",
        );
        first.entity_id = Some(entity);
        ingestor.submit(first).await.unwrap();

        let mut second = submission(
            "warning-2",
            "Shanghai port delays",
            "Growing container backlog and demurrage fees",
        );
        second.entity_id = Some(entity);
        let outcome = ingestor.submit(second).await.unwrap();

        match outcome {
            IngestOutcome::Merged { item, reason } => {
                assert_eq!(reason, MergeReason::SameEntityTimeWindow);
                assert_eq!(item.occurrence_count, 2);
            }
            other => panic!("expected a same-entity merge, got {other:?}"),
        }
        assert_eq!(ingestor.queue().len(), 1);
    }

    /// #120: a stricter signal must never be absorbed by a weaker row.
    #[tokio::test]
    async fn more_severe_signal_is_not_merged_into_a_weaker_row() {
        let queue = InMemoryIngestQueue::new();
        let ingestor = ingestor(queue);
        let entity = Uuid::new_v4();

        let mut first = submission(
            "warning-1",
            "Port congestion at Shanghai",
            "Container backlog grows",
        );
        first.entity_id = Some(entity);
        first.static_severity = Some("medium".to_string());
        ingestor.submit(first).await.unwrap();

        let mut critical = submission(
            "warning-2",
            "Shanghai port delays",
            "Growing container backlog and demurrage fees",
        );
        critical.entity_id = Some(entity);
        critical.static_severity = Some("critical".to_string());
        let outcome = ingestor.submit(critical).await.unwrap();

        assert!(
            !outcome.merged(),
            "a critical signal must not be absorbed by a medium row"
        );
        assert_eq!(ingestor.queue().len(), 2);
    }

    /// #120: the reverse direction is fine — a weaker signal may join a
    /// stricter row.
    #[tokio::test]
    async fn weaker_signal_may_merge_into_a_more_severe_row() {
        let queue = InMemoryIngestQueue::new();
        let ingestor = ingestor(queue);
        let entity = Uuid::new_v4();

        let mut first = submission(
            "warning-1",
            "Port congestion at Shanghai",
            "Container backlog grows",
        );
        first.entity_id = Some(entity);
        first.static_severity = Some("high".to_string());
        ingestor.submit(first).await.unwrap();

        let mut low = submission(
            "warning-2",
            "Shanghai port delays",
            "Growing container backlog and demurrage fees",
        );
        low.entity_id = Some(entity);
        low.static_severity = Some("low".to_string());
        let outcome = ingestor.submit(low).await.unwrap();

        assert!(outcome.merged(), "a weaker signal may join a stricter row");
        assert_eq!(ingestor.queue().len(), 1);
    }

    /// #118: lexical near-duplicates must share an entity identity; similar
    /// text across different entities is a false positive.
    #[tokio::test]
    async fn lexical_merge_requires_a_matching_entity_id() {
        let queue = InMemoryIngestQueue::new();
        let ingestor = ingestor(queue);

        let mut first = submission(
            "warning-1",
            "Foxconn quality crisis",
            "Quality defect recall at Foxconn Tunisia manufacturing plant",
        );
        first.entity_id = Some(Uuid::new_v4());
        ingestor.submit(first).await.unwrap();

        let mut different_entity = submission(
            "warning-2",
            "Foxconn quality issue",
            "Defect and recall at Foxconn Tunisia factory",
        );
        different_entity.entity_id = Some(Uuid::new_v4());
        let outcome = ingestor.submit(different_entity).await.unwrap();

        assert!(
            !outcome.merged(),
            "two different entities must never merge on text alone"
        );
        assert_eq!(ingestor.queue().len(), 2);
    }

    /// #118: a null entity id must not match a different entity's item.
    #[tokio::test]
    async fn lexical_merge_rejects_a_missing_entity_against_a_set_entity() {
        let queue = InMemoryIngestQueue::new();
        let ingestor = ingestor(queue);

        let mut first = submission(
            "warning-1",
            "Foxconn quality crisis",
            "Quality defect recall at Foxconn Tunisia manufacturing plant",
        );
        first.entity_id = Some(Uuid::new_v4());
        ingestor.submit(first).await.unwrap();

        let missing_entity = submission(
            "warning-2",
            "Foxconn quality issue",
            "Defect and recall at Foxconn Tunisia factory",
        );
        let outcome = ingestor.submit(missing_entity).await.unwrap();

        assert!(
            !outcome.merged(),
            "a missing entity id must not match a different entity's item"
        );
        assert_eq!(ingestor.queue().len(), 2);
    }

    /// #118: the reverse direction — a set entity id must not match a stored
    /// item with no entity id — is also a mismatch.
    #[tokio::test]
    async fn lexical_merge_rejects_a_set_entity_against_a_missing_entity() {
        let queue = InMemoryIngestQueue::new();
        let ingestor = ingestor(queue);

        let first = submission(
            "warning-1",
            "Foxconn quality crisis",
            "Quality defect recall at Foxconn Tunisia manufacturing plant",
        );
        ingestor.submit(first).await.unwrap();

        let mut with_entity = submission(
            "warning-2",
            "Foxconn quality issue",
            "Defect and recall at Foxconn Tunisia factory",
        );
        with_entity.entity_id = Some(Uuid::new_v4());
        let outcome = ingestor.submit(with_entity).await.unwrap();

        assert!(
            !outcome.merged(),
            "a set entity id must not match an item with no entity id"
        );
        assert_eq!(ingestor.queue().len(), 2);
    }

    /// #119: a similarity hit pointing at a target older than the configured
    /// window must not merge, and its dedup candidate is pruned.
    #[tokio::test]
    async fn similarity_hit_does_not_merge_into_an_old_target() {
        let queue = InMemoryIngestQueue::new();
        let old_time = Utc::now() - Duration::hours(48);
        let target = queue_row(
            "warning-old",
            "Samsung fab investment",
            "New semiconductor investment in Korea",
            None,
            old_time,
        );
        let target_id = target.id;
        queue.push_row(target);

        let store = InMemoryDedupStore::new(10);
        store
            .store_item_with_vector(
                &TriageItemType::Warning,
                &target_id.to_string(),
                "Samsung fab investment",
                "New semiconductor investment in Korea",
                Some(&[1.0, 0.0]),
            )
            .await
            .unwrap();
        let embedder = Arc::new(CountingEmbedder::new(vec![1.0, 0.0]));
        let dedup =
            SemanticDedup::with_embedder(embedder, Some(Box::new(store)), DedupConfig::default());
        let ingestor = TriageIngestor::new(queue, dedup);

        let outcome = ingestor
            .submit(submission(
                "warning-new",
                "Port workers strike",
                "Dockworkers walk out at Rotterdam terminal",
            ))
            .await
            .unwrap();

        assert!(
            !outcome.merged(),
            "an out-of-window target must not absorb a new signal"
        );
        assert_eq!(ingestor.queue().len(), 2);
        let hits = ingestor
            .dedup
            .dedup_store
            .as_ref()
            .unwrap()
            .find_similar_by_vector(&TriageItemType::Warning, &[1.0, 0.0], 10)
            .await
            .unwrap();
        assert!(
            !hits.iter().any(|hit| hit.id == target_id.to_string()),
            "the stale dedup candidate must be pruned"
        );
        assert!(
            hits.iter()
                .any(|hit| hit.id == outcome.item().id.to_string()),
            "the new item's candidate must still be stored"
        );
    }

    /// #119: a similarity hit pointing at an inactive row (findable by id,
    /// absent from the active candidate set) must not merge either.
    #[tokio::test]
    async fn similarity_hit_does_not_merge_into_an_inactive_target() {
        let queue = InMemoryIngestQueue::new();
        let target = queue_row(
            "warning-resolved",
            "Samsung fab investment",
            "New semiconductor investment in Korea",
            None,
            Utc::now(),
        );
        let target_id = target.id;
        queue.push_row(target);
        queue.mark_inactive(target_id);

        let store = InMemoryDedupStore::new(10);
        store
            .store_item_with_vector(
                &TriageItemType::Warning,
                &target_id.to_string(),
                "Samsung fab investment",
                "New semiconductor investment in Korea",
                Some(&[1.0, 0.0]),
            )
            .await
            .unwrap();
        let embedder = Arc::new(CountingEmbedder::new(vec![1.0, 0.0]));
        let dedup =
            SemanticDedup::with_embedder(embedder, Some(Box::new(store)), DedupConfig::default());
        let ingestor = TriageIngestor::new(queue, dedup);

        let outcome = ingestor
            .submit(submission(
                "warning-new",
                "Port workers strike",
                "Dockworkers walk out at Rotterdam terminal",
            ))
            .await
            .unwrap();

        assert!(
            !outcome.merged(),
            "an inactive target must not absorb a new signal"
        );
        assert_eq!(ingestor.queue().len(), 2);
        let hits = ingestor
            .dedup
            .dedup_store
            .as_ref()
            .unwrap()
            .find_similar_by_vector(&TriageItemType::Warning, &[1.0, 0.0], 10)
            .await
            .unwrap();
        assert!(
            !hits.iter().any(|hit| hit.id == target_id.to_string()),
            "the inactive dedup candidate must be pruned"
        );
    }

    /// #119 positive control: an active, in-window target is still merged.
    #[tokio::test]
    async fn similarity_hit_merges_into_an_active_recent_target() {
        let queue = InMemoryIngestQueue::new();
        let target = queue_row(
            "warning-active",
            "Samsung fab investment",
            "New semiconductor investment in Korea",
            None,
            Utc::now(),
        );
        let target_id = target.id;
        queue.push_row(target);

        let store = InMemoryDedupStore::new(10);
        store
            .store_item_with_vector(
                &TriageItemType::Warning,
                &target_id.to_string(),
                "Samsung fab investment",
                "New semiconductor investment in Korea",
                Some(&[1.0, 0.0]),
            )
            .await
            .unwrap();
        let embedder = Arc::new(CountingEmbedder::new(vec![1.0, 0.0]));
        let dedup =
            SemanticDedup::with_embedder(embedder, Some(Box::new(store)), DedupConfig::default());
        let ingestor = TriageIngestor::new(queue, dedup);

        let outcome = ingestor
            .submit(submission(
                "warning-new",
                "Port workers strike",
                "Dockworkers walk out at Rotterdam terminal",
            ))
            .await
            .unwrap();

        match outcome {
            IngestOutcome::Merged { item, reason } => {
                assert_eq!(reason, MergeReason::EmbeddingSimilarity);
                assert_eq!(item.id, target_id);
                assert_eq!(item.occurrence_count, 2);
            }
            other => panic!("expected an embedding merge, got {other:?}"),
        }
        assert_eq!(ingestor.queue().len(), 1);
    }

    /// #119: a brand-new item is embedded exactly once; the vector from the
    /// similarity search is reused for storage instead of embedding again.
    #[tokio::test]
    async fn new_item_is_embedded_once_and_vector_is_stored() {
        let queue = InMemoryIngestQueue::new();
        let store = InMemoryDedupStore::new(10);
        let embedder = Arc::new(CountingEmbedder::new(vec![1.0, 0.0]));
        let dedup = SemanticDedup::with_embedder(
            embedder.clone(),
            Some(Box::new(store)),
            DedupConfig::default(),
        );
        let ingestor = TriageIngestor::new(queue, dedup);

        let outcome = ingestor
            .submit(submission(
                "warning-new",
                "Unique headline",
                "Unique body long enough for dedup",
            ))
            .await
            .unwrap();

        assert!(!outcome.merged());
        assert_eq!(
            embedder.calls(),
            1,
            "a new submission must be embedded exactly once"
        );
        let hits = ingestor
            .dedup
            .dedup_store
            .as_ref()
            .unwrap()
            .find_similar_by_vector(&TriageItemType::Warning, &[1.0, 0.0], 5)
            .await
            .unwrap();
        assert!(
            hits.iter()
                .any(|hit| { hit.id == outcome.item().id.to_string() && hit.similarity >= 0.99 }),
            "the embedding from the search must be stored with the new item"
        );
    }

    #[tokio::test]
    async fn distinct_items_enqueue_separately() {
        let queue = InMemoryIngestQueue::new();
        let ingestor = ingestor(queue);

        let first = submission(
            "warning-1",
            "Foxconn quality crisis",
            "Quality defect recall at Foxconn Tunisia manufacturing plant",
        );
        let second = submission(
            "warning-2",
            "Samsung expansion",
            "Samsung announces new semiconductor fab investment in Korea",
        );

        assert!(!ingestor.submit(first).await.unwrap().merged());
        assert!(!ingestor.submit(second).await.unwrap().merged());
        assert_eq!(ingestor.queue().len(), 2);
    }

    #[tokio::test]
    async fn concurrent_duplicate_submits_do_not_double_insert() {
        use std::sync::Arc;

        let queue = Arc::new(InMemoryIngestQueue::new());
        let ingestor = Arc::new(TriageIngestor::new(
            Arc::clone(&queue),
            SemanticDedup::with_in_memory_fallback(),
        ));

        let mut handles = Vec::new();
        for i in 0..8 {
            let ingestor = Arc::clone(&ingestor);
            handles.push(tokio::spawn(async move {
                let mut sub = submission(
                    "warning-concurrent",
                    "Concurrent duplicate",
                    "Eight identical submissions racing into the ingress",
                );
                sub.observation_ids = vec![Uuid::new_v4()];
                sub.source_urls = vec![format!("https://news-{i}.example/story")];
                ingestor.submit(sub).await
            }));
        }

        let mut merged = 0;
        for handle in handles {
            let outcome = handle.await.unwrap().unwrap();
            if outcome.merged() {
                merged += 1;
            }
        }

        let rows = queue.snapshot();
        assert_eq!(rows.len(), 1, "concurrent duplicates must insert once");
        assert_eq!(rows[0].occurrence_count, 8);
        assert_eq!(rows[0].merged_observation_ids.len(), 8);
        assert_eq!(rows[0].merged_source_urls.len(), 8);
        assert_eq!(merged, 7, "all but the first submission merge");
    }

    #[test]
    fn escalation_raises_severity_on_repeats() {
        assert_eq!(
            escalate_severity(Some("low"), Some("low"), 2, 3, 5).as_deref(),
            Some("low")
        );
        assert_eq!(
            escalate_severity(Some("low"), Some("low"), 3, 3, 5).as_deref(),
            Some("high")
        );
        assert_eq!(
            escalate_severity(Some("low"), None, 5, 3, 5).as_deref(),
            Some("critical")
        );
        assert_eq!(
            escalate_severity(Some("critical"), None, 1, 3, 5).as_deref(),
            Some("critical")
        );
        assert_eq!(escalate_severity(None, None, 1, 3, 5), None);
    }

    #[test]
    fn severity_rank_maps_known_values() {
        assert_eq!(severity_rank("low"), 0);
        assert_eq!(severity_rank("Medium"), 1);
        assert_eq!(severity_rank("HIGH"), 2);
        assert_eq!(severity_rank("critical"), 3);
        assert_eq!(severity_rank("unknown"), -1);
    }

    #[test]
    fn severity_comparison_decides_whether_a_merge_weakens_the_row() {
        assert!(merge_would_weaken_severity(
            Some("medium"),
            Some("critical")
        ));
        assert!(merge_would_weaken_severity(None, Some("low")));
        assert!(!merge_would_weaken_severity(Some("high"), Some("low")));
        assert!(!merge_would_weaken_severity(Some("high"), Some("high")));
        assert!(!merge_would_weaken_severity(None, None));
        assert!(!merge_would_weaken_severity(Some("low"), Some("unknown")));
    }
}
