//! Triage Queue — manages the priority queue of triaged items.
//!
//! [`TriageQueue`] provides DB-backed operations for enqueueing, dequeueing,
//! prioritizing, and acknowledging items in the triage pipeline.

use std::collections::HashMap;

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use tracing::instrument;
use uuid::Uuid;

use apex_core::triage::{
    composite_score, score_band_color, score_to_band, TriageDimensions, TriageItemType,
    TriageQueueItem, TriageStatus, TriageThresholds, TriageWeights,
};

// ─── Constants ────────────────────────────────────────────────────────────────

const MAX_PEEK: usize = 200;

// ─── TriageQueue ──────────────────────────────────────────────────────────────

/// Manages the triage queue with DB-backed persistence.
///
/// Operations use `sqlx::PgPool` directly for maximum flexibility.
/// This struct is intended to be used via `PgStore` extension methods in the
/// store crate, but is defined here to keep the triage domain logic co-located.
#[derive(Debug, Clone)]
pub struct TriageQueue {
    pool: PgPool,
    weights: TriageWeights,
    thresholds: TriageThresholds,
}

impl TriageQueue {
    /// Create a new [`TriageQueue`] backed by the given pool.
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            weights: TriageWeights::default(),
            thresholds: TriageThresholds::default(),
        }
    }

    /// Create a [`TriageQueue`] with custom weights and thresholds.
    pub fn with_config(pool: PgPool, weights: TriageWeights, thresholds: TriageThresholds) -> Self {
        Self {
            pool,
            weights,
            thresholds,
        }
    }

    // ── Enqueue ────────────────────────────────────────────────────────────────

    /// Enqueue a new item, or update the score of an existing one (by
    /// `(item_type, source_id)` UNIQUE constraint).
    #[instrument(skip(self))]
    pub async fn enqueue(
        &self,
        item_type: TriageItemType,
        source_id: &str,
        title: &str,
        description: &str,
        entity_id: Option<Uuid>,
        entity_name: Option<&str>,
        static_severity: Option<&str>,
        dimensions: Option<&TriageDimensions>,
    ) -> Result<TriageQueueItem> {
        let item_type_str = item_type.as_str();
        let now = Utc::now();

        // Compute score from dimensions, or default to 0.0 for unscored items.
        let (urgency, impact, actionability, novelty, confidence, composite) =
            if let Some(dims) = dimensions {
                let score = composite_score(dims, &self.weights);
                (
                    dims.urgency,
                    dims.impact,
                    dims.actionability,
                    dims.novelty,
                    dims.confidence,
                    score,
                )
            } else {
                (0.0, 0.0, 0.0, 0.0, 0.0, 0.0)
            };

        let row = sqlx::query_as::<_, TriageQueueItemRow>(
            r#"
            INSERT INTO triage_queue
                (item_type, source_id, title, description, entity_id, entity_name,
                 static_severity, urgency, impact, actionability, novelty, confidence,
                 composite_score, status, created_at)
            VALUES
                ($1, $2::uuid, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, 'pending', $14)
            ON CONFLICT (item_type, source_id) DO UPDATE SET
                title             = EXCLUDED.title,
                description       = EXCLUDED.description,
                entity_id         = COALESCE(EXCLUDED.entity_id, triage_queue.entity_id),
                entity_name       = COALESCE(EXCLUDED.entity_name, triage_queue.entity_name),
                static_severity   = (
                    CASE GREATEST(
                        CASE lower(coalesce(triage_queue.static_severity, ''))
                            WHEN 'critical' THEN 3 WHEN 'high' THEN 2
                            WHEN 'medium' THEN 1 WHEN 'low' THEN 0 ELSE -1 END,
                        CASE lower(coalesce(EXCLUDED.static_severity, ''))
                            WHEN 'critical' THEN 3 WHEN 'high' THEN 2
                            WHEN 'medium' THEN 1 WHEN 'low' THEN 0 ELSE -1 END,
                        CASE WHEN triage_queue.occurrence_count + 1 >= $16 THEN 3
                             WHEN triage_queue.occurrence_count + 1 >= $15 THEN 2
                             ELSE -1 END
                    )
                    WHEN 3 THEN 'critical' WHEN 2 THEN 'high'
                    WHEN 1 THEN 'medium' WHEN 0 THEN 'low'
                    ELSE triage_queue.static_severity END
                ),
                occurrence_count  = triage_queue.occurrence_count + 1,
                last_seen_at      = NOW(),
                urgency           = EXCLUDED.urgency,
                impact            = EXCLUDED.impact,
                actionability     = EXCLUDED.actionability,
                novelty           = EXCLUDED.novelty,
                confidence        = EXCLUDED.confidence,
                composite_score   = EXCLUDED.composite_score,
                status            = CASE WHEN triage_queue.status = 'pending' THEN 'pending' ELSE triage_queue.status END,
                updated_at        = $14
            RETURNING
                id, item_type, source_id, title, description, entity_id, entity_name,
                static_severity, urgency, impact, actionability, novelty, confidence,
                composite_score, is_overridden, override_score,
                status::text, created_at, triaged_at, acknowledged_at
            "#,
        )
        .bind(item_type_str)
        .bind(source_id)
        .bind(title)
        .bind(description)
        .bind(entity_id)
        .bind(entity_name)
        .bind(static_severity)
        .bind(urgency)
        .bind(impact)
        .bind(actionability)
        .bind(novelty)
        .bind(confidence)
        .bind(composite)
        .bind(now)
        .bind(crate::semantic_dedup::DEFAULT_ESCALATE_HIGH_AT)
        .bind(crate::semantic_dedup::DEFAULT_ESCALATE_CRITICAL_AT)
        .fetch_one(&self.pool)
        .await?;

        Ok(row.into_item(&self.thresholds))
    }

    // ── Peek / List ────────────────────────────────────────────────────────────

    /// Peek at the highest-priority items that are still pending.
    #[instrument(skip(self))]
    pub async fn peek_top(&self, limit: usize) -> Result<Vec<TriageQueueItem>> {
        let limit = limit.min(MAX_PEEK) as i64;

        let rows = sqlx::query_as::<_, TriageQueueItemRow>(
            r#"
            SELECT id, item_type, source_id, title, description, entity_id, entity_name,
                   static_severity, urgency, impact, actionability, novelty, confidence,
                   composite_score, is_overridden, override_score,
                   status::text, created_at, triaged_at, acknowledged_at
            FROM triage_queue
            WHERE status = 'pending'
            ORDER BY
                CASE WHEN is_overridden THEN COALESCE(override_score, composite_score) ELSE composite_score END DESC,
                created_at ASC
            LIMIT $1
            "#,
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| r.into_item(&self.thresholds))
            .collect())
    }

    /// List items with optional status filter, ordered by priority.
    #[instrument(skip(self))]
    pub async fn list(
        &self,
        status_filter: Option<TriageStatus>,
        limit: usize,
        offset: u64,
    ) -> Result<Vec<TriageQueueItem>> {
        let limit = limit.min(MAX_PEEK) as i64;
        let offset = offset as i64;

        let status_str = status_filter.as_ref().map(|s| s.as_str());

        let rows = sqlx::query_as::<_, TriageQueueItemRow>(
            r#"
            SELECT id, item_type, source_id, title, description, entity_id, entity_name,
                   static_severity, urgency, impact, actionability, novelty, confidence,
                   composite_score, is_overridden, override_score,
                   status::text, created_at, triaged_at, acknowledged_at
            FROM triage_queue
            WHERE ($1::text IS NULL OR status = $1::triage_status)
            ORDER BY
                CASE WHEN is_overridden THEN COALESCE(override_score, composite_score) ELSE composite_score END DESC,
                created_at ASC
            LIMIT $2 OFFSET $3
            "#,
        )
        .bind(status_str)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| r.into_item(&self.thresholds))
            .collect())
    }

    /// Count items, optionally filtered by status.
    pub async fn count(&self, status_filter: Option<TriageStatus>) -> Result<i64> {
        let status_str = status_filter.as_ref().map(|s| s.as_str());

        let row: (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(*) FROM triage_queue
            WHERE ($1::text IS NULL OR status = $1::triage_status)
            "#,
        )
        .bind(status_str)
        .fetch_one(&self.pool)
        .await?;

        Ok(row.0)
    }

    /// Get aggregate statistics for the triage dashboard.
    #[instrument(skip(self))]
    pub async fn stats(&self) -> Result<apex_core::triage::TriageStats> {
        let row: TriageStatsRow = sqlx::query_as(
            r#"
            SELECT
                COUNT(*)::bigint                                            AS total,
                COUNT(*) FILTER (WHERE status = 'pending')::bigint          AS pending,
                COUNT(*) FILTER (WHERE status = 'triaged')::bigint          AS triaged,
                COUNT(*) FILTER (WHERE status = 'acknowledged')::bigint     AS acknowledged,
                COUNT(*) FILTER (WHERE status = 'resolved')::bigint         AS resolved,
                COUNT(*) FILTER (WHERE status = 'dismissed')::bigint        AS dismissed,
                COUNT(*) FILTER (WHERE is_overridden)::bigint               AS overridden_count,
                COALESCE(AVG(composite_score) FILTER (WHERE status = 'pending'), 0.0)::double precision AS avg_score
            FROM triage_queue
            "#,
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(row.into_stats())
    }

    /// Get the count of items per score band (critical, high, medium, low).
    pub async fn band_counts(&self) -> Result<HashMap<String, i64>> {
        let rows: Vec<(String, i64)> = sqlx::query_as(
            r#"
            SELECT
                CASE
                    WHEN composite_score >= 0.80 THEN 'critical'
                    WHEN composite_score >= 0.60 THEN 'high'
                    WHEN composite_score >= 0.40 THEN 'medium'
                    ELSE 'low'
                END AS band,
                COUNT(*)::bigint AS cnt
            FROM triage_queue
            WHERE status = 'pending'
            GROUP BY band
            ORDER BY band DESC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        let mut map: HashMap<String, i64> = HashMap::new();
        for (band, cnt) in rows {
            map.insert(band, cnt);
        }
        Ok(map)
    }

    // ── Score Override ─────────────────────────────────────────────────────────

    /// Override the composite score of a queue item (e.g. from a manual triage
    /// adjustment or a user override).
    #[instrument(skip(self))]
    pub async fn override_score(
        &self,
        id: Uuid,
        new_score: f64,
        overridden_by: &str,
    ) -> Result<TriageQueueItem> {
        let clamped = new_score.clamp(0.0, 1.0);

        let row = sqlx::query_as::<_, TriageQueueItemRow>(
            r#"
            UPDATE triage_queue
            SET is_overridden   = TRUE,
                override_score  = $2,
                status          = 'triaged',
                triaged_at      = NOW()
            WHERE id = $1
            RETURNING
                id, item_type, source_id, title, description, entity_id, entity_name,
                static_severity, urgency, impact, actionability, novelty, confidence,
                composite_score, is_overridden, override_score,
                status::text, created_at, triaged_at, acknowledged_at
            "#,
        )
        .bind(id)
        .bind(clamped)
        .fetch_one(&self.pool)
        .await?;

        Ok(row.into_item(&self.thresholds))
    }

    // ── Status Transitions ─────────────────────────────────────────────────────

    /// Acknowledge a queue item (user has seen it).
    #[instrument(skip(self))]
    pub async fn acknowledge(&self, id: Uuid) -> Result<TriageQueueItem> {
        let row = sqlx::query_as::<_, TriageQueueItemRow>(
            r#"
            UPDATE triage_queue
            SET status          = 'acknowledged',
                acknowledged_at = NOW()
            WHERE id = $1
            RETURNING
                id, item_type, source_id, title, description, entity_id, entity_name,
                static_severity, urgency, impact, actionability, novelty, confidence,
                composite_score, is_overridden, override_score,
                status::text, created_at, triaged_at, acknowledged_at
            "#,
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await?;

        Ok(row.into_item(&self.thresholds))
    }

    /// Resolve a queue item (action taken).
    #[instrument(skip(self))]
    pub async fn resolve(&self, id: Uuid) -> Result<TriageQueueItem> {
        let row = sqlx::query_as::<_, TriageQueueItemRow>(
            r#"
            UPDATE triage_queue
            SET status = 'resolved'
            WHERE id = $1
            RETURNING
                id, item_type, source_id, title, description, entity_id, entity_name,
                static_severity, urgency, impact, actionability, novelty, confidence,
                composite_score, is_overridden, override_score,
                status::text, created_at, triaged_at, acknowledged_at
            "#,
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await?;

        Ok(row.into_item(&self.thresholds))
    }

    /// Dismiss a queue item (false positive / not relevant).
    #[instrument(skip(self))]
    pub async fn dismiss(&self, id: Uuid) -> Result<TriageQueueItem> {
        let row = sqlx::query_as::<_, TriageQueueItemRow>(
            r#"
            UPDATE triage_queue
            SET status = 'dismissed'
            WHERE id = $1
            RETURNING
                id, item_type, source_id, title, description, entity_id, entity_name,
                static_severity, urgency, impact, actionability, novelty, confidence,
                composite_score, is_overridden, override_score,
                status::text, created_at, triaged_at, acknowledged_at
            "#,
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await?;

        Ok(row.into_item(&self.thresholds))
    }

    /// Get a single queue item by ID.
    pub async fn get_by_id(&self, id: Uuid) -> Result<Option<TriageQueueItem>> {
        let row = sqlx::query_as::<_, TriageQueueItemRow>(
            r#"
            SELECT id, item_type, source_id, title, description, entity_id, entity_name,
                   static_severity, urgency, impact, actionability, novelty, confidence,
                   composite_score, is_overridden, override_score,
                   status::text, created_at, triaged_at, acknowledged_at
            FROM triage_queue
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| r.into_item(&self.thresholds)))
    }

    /// Get unscored items (composite_score = 0) that need LLM triage.
    #[instrument(skip(self))]
    pub async fn unscored_items(&self, limit: usize) -> Result<Vec<TriageQueueItem>> {
        let limit = limit.min(MAX_PEEK) as i64;

        let rows = sqlx::query_as::<_, TriageQueueItemRow>(
            r#"
            SELECT id, item_type, source_id, title, description, entity_id, entity_name,
                   static_severity, urgency, impact, actionability, novelty, confidence,
                   composite_score, is_overridden, override_score,
                   status::text, created_at, triaged_at, acknowledged_at
            FROM triage_queue
            WHERE status = 'pending' AND composite_score = 0.0
            ORDER BY created_at ASC
            LIMIT $1
            "#,
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| r.into_item(&self.thresholds))
            .collect())
    }

    /// Batch-update scores for items after LLM processing.
    pub async fn batch_update_scores(&self, scores: Vec<(Uuid, TriageDimensions)>) -> Result<u64> {
        let mut updated = 0u64;
        for (id, dims) in scores {
            let score = composite_score(&dims, &self.weights);
            let result = sqlx::query(
                r#"
                UPDATE triage_queue
                SET urgency        = $2,
                    impact         = $3,
                    actionability  = $4,
                    novelty        = $5,
                    confidence     = $6,
                    composite_score = $7,
                    status         = 'triaged',
                    triaged_at     = NOW()
                WHERE id = $1
                "#,
            )
            .bind(id)
            .bind(dims.urgency)
            .bind(dims.impact)
            .bind(dims.actionability)
            .bind(dims.novelty)
            .bind(dims.confidence)
            .bind(score)
            .execute(&self.pool)
            .await?;

            updated += result.rows_affected();
        }
        Ok(updated)
    }

    /// Purge old resolved/dismissed items beyond the retention period.
    pub async fn purge_old(&self, older_than: DateTime<Utc>) -> Result<u64> {
        let result = sqlx::query(
            r#"
            DELETE FROM triage_queue
            WHERE status IN ('resolved', 'dismissed')
              AND created_at < $1
            "#,
        )
        .bind(older_than)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected())
    }
}

// ─── IngestQueue implementation (semantic dedup ingress) ─────────────────────

const INGEST_SELECT_COLUMNS: &str = r#"
    id, item_type, source_id::text, title, description, entity_id, entity_name,
    static_severity, occurrence_count::bigint, last_seen_at, merged_observation_ids,
    merged_source_urls, created_at
"#;

#[async_trait]
impl crate::semantic_dedup::IngestQueue for TriageQueue {
    async fn find_by_source_id(
        &self,
        item_type: &TriageItemType,
        source_id: &str,
    ) -> Result<Option<crate::semantic_dedup::IngestQueueItem>> {
        let sql = format!(
            "SELECT {INGEST_SELECT_COLUMNS} FROM triage_queue \
             WHERE item_type = $1 AND source_id = $2::uuid"
        );
        let row = sqlx::query_as::<_, IngestQueueRow>(&sql)
            .bind(item_type.as_str())
            .bind(source_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(IngestQueueRow::into_ingest_item))
    }

    async fn find_by_id(&self, id: Uuid) -> Result<Option<crate::semantic_dedup::IngestQueueItem>> {
        let sql = format!("SELECT {INGEST_SELECT_COLUMNS} FROM triage_queue WHERE id = $1");
        let row = sqlx::query_as::<_, IngestQueueRow>(&sql)
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(IngestQueueRow::into_ingest_item))
    }

    async fn find_recent(
        &self,
        item_type: &TriageItemType,
        window: chrono::Duration,
    ) -> Result<Vec<crate::semantic_dedup::IngestQueueItem>> {
        // Only active rows are merge candidates: a new signal must never be
        // absorbed by a resolved/dismissed item and become invisible.
        let sql = format!(
            "SELECT {INGEST_SELECT_COLUMNS} FROM triage_queue \
             WHERE item_type = $1 \
               AND status IN ('pending', 'triaged', 'acknowledged') \
               AND COALESCE(last_seen_at, created_at) >= NOW() - $2::interval \
             ORDER BY COALESCE(last_seen_at, created_at) DESC \
             LIMIT 200"
        );
        let interval = format!("{} seconds", window.num_seconds().max(0));
        let rows = sqlx::query_as::<_, IngestQueueRow>(&sql)
            .bind(item_type.as_str())
            .bind(interval)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows
            .into_iter()
            .map(IngestQueueRow::into_ingest_item)
            .collect())
    }

    async fn insert_submission(
        &self,
        submission: &crate::semantic_dedup::TriageSubmission,
        high_at: i64,
        critical_at: i64,
    ) -> Result<crate::semantic_dedup::IngestQueueItem> {
        let item_type_str = submission.item_type.as_str();
        let now = Utc::now();

        let (urgency, impact, actionability, novelty, confidence, composite) =
            if let Some(dims) = &submission.dimensions {
                let score = composite_score(dims, &self.weights);
                (
                    dims.urgency,
                    dims.impact,
                    dims.actionability,
                    dims.novelty,
                    dims.confidence,
                    score,
                )
            } else {
                (0.0, 0.0, 0.0, 0.0, 0.0, 0.0)
            };

        let sql = format!(
            r#"
            INSERT INTO triage_queue
                (item_type, source_id, title, description, entity_id, entity_name,
                 static_severity, urgency, impact, actionability, novelty, confidence,
                 composite_score, status, created_at, occurrence_count, last_seen_at,
                 merged_observation_ids, merged_source_urls)
            VALUES
                ($1, $2::uuid, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13,
                 'pending', $14, 1, $14, $15, $16)
            ON CONFLICT (item_type, source_id) DO NOTHING
            RETURNING {INGEST_SELECT_COLUMNS}
            "#
        );

        let row = sqlx::query_as::<_, IngestQueueRow>(&sql)
            .bind(item_type_str)
            .bind(&submission.source_id)
            .bind(&submission.title)
            .bind(&submission.description)
            .bind(submission.entity_id)
            .bind(submission.entity_name.as_deref())
            .bind(submission.static_severity.as_deref())
            .bind(urgency)
            .bind(impact)
            .bind(actionability)
            .bind(novelty)
            .bind(confidence)
            .bind(composite)
            .bind(now)
            .bind(&submission.observation_ids)
            .bind(&submission.source_urls)
            .fetch_optional(&self.pool)
            .await?;

        if let Some(row) = row {
            return Ok(row.into_ingest_item());
        }

        // Conflict: a concurrent (or repeated) submission won the insert.
        // Merge into it so the duplicate is counted, not dropped. The merge
        // recomputes severity from the post-increment count.
        let existing = self
            .find_by_source_id(&submission.item_type, &submission.source_id)
            .await?
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "triage queue conflict for {}/{} but no existing row found",
                    item_type_str,
                    submission.source_id
                )
            })?;
        self.merge_submission(existing.id, submission, high_at, critical_at)
            .await
    }

    async fn merge_submission(
        &self,
        target_id: Uuid,
        submission: &crate::semantic_dedup::TriageSubmission,
        high_at: i64,
        critical_at: i64,
    ) -> Result<crate::semantic_dedup::IngestQueueItem> {
        // Severity is escalated from the post-increment occurrence count in a
        // single UPDATE, so concurrent merges cannot persist a level below the
        // one the final count requires.
        let sql = format!(
            r#"
            UPDATE triage_queue
            SET occurrence_count = triage_queue.occurrence_count + 1,
                last_seen_at = NOW(),
                updated_at = NOW(),
                merged_observation_ids = (
                    SELECT COALESCE(array_agg(DISTINCT obs), ARRAY[]::uuid[])
                    FROM unnest(triage_queue.merged_observation_ids || $2::uuid[]) AS obs
                ),
                merged_source_urls = (
                    SELECT COALESCE(array_agg(DISTINCT url), ARRAY[]::text[])
                    FROM unnest(triage_queue.merged_source_urls || $3::text[]) AS url
                ),
                static_severity = (
                    CASE GREATEST(
                        CASE lower(coalesce(triage_queue.static_severity, ''))
                            WHEN 'critical' THEN 3 WHEN 'high' THEN 2
                            WHEN 'medium' THEN 1 WHEN 'low' THEN 0 ELSE -1 END,
                        CASE lower(coalesce($4, ''))
                            WHEN 'critical' THEN 3 WHEN 'high' THEN 2
                            WHEN 'medium' THEN 1 WHEN 'low' THEN 0 ELSE -1 END,
                        CASE WHEN triage_queue.occurrence_count + 1 >= $5 THEN 3
                             WHEN triage_queue.occurrence_count + 1 >= $6 THEN 2
                             ELSE -1 END
                    )
                    WHEN 3 THEN 'critical' WHEN 2 THEN 'high'
                    WHEN 1 THEN 'medium' WHEN 0 THEN 'low'
                    ELSE triage_queue.static_severity END
                )
            WHERE id = $1
            RETURNING {INGEST_SELECT_COLUMNS}
            "#
        );

        let row = sqlx::query_as::<_, IngestQueueRow>(&sql)
            .bind(target_id)
            .bind(&submission.observation_ids)
            .bind(&submission.source_urls)
            .bind(&submission.static_severity)
            .bind(critical_at)
            .bind(high_at)
            .fetch_one(&self.pool)
            .await?;

        Ok(row.into_ingest_item())
    }
}

#[derive(Debug, sqlx::FromRow)]
struct IngestQueueRow {
    id: Uuid,
    item_type: String,
    source_id: String,
    title: String,
    description: String,
    entity_id: Option<Uuid>,
    entity_name: Option<String>,
    static_severity: Option<String>,
    occurrence_count: i64,
    last_seen_at: Option<DateTime<Utc>>,
    merged_observation_ids: Vec<Uuid>,
    merged_source_urls: Vec<String>,
    created_at: DateTime<Utc>,
}

impl IngestQueueRow {
    fn into_ingest_item(self) -> crate::semantic_dedup::IngestQueueItem {
        crate::semantic_dedup::IngestQueueItem {
            id: self.id,
            item_type: TriageItemType::from_str(&self.item_type),
            source_id: self.source_id,
            title: self.title,
            description: self.description,
            entity_id: self.entity_id,
            entity_name: self.entity_name,
            static_severity: self.static_severity,
            occurrence_count: self.occurrence_count,
            last_seen_at: self.last_seen_at,
            merged_observation_ids: self.merged_observation_ids,
            merged_source_urls: self.merged_source_urls,
            created_at: self.created_at,
        }
    }
}

// ─── Trait for mockable queue operations ──────────────────────────────────────

/// Abstract interface for triage queue operations, enabling mock testing.
#[async_trait]
pub trait TriageQueueProvider: Send + Sync {
    async fn enqueue(
        &self,
        item_type: TriageItemType,
        source_id: &str,
        title: &str,
        description: &str,
        entity_id: Option<Uuid>,
        entity_name: Option<&str>,
        static_severity: Option<&str>,
        dimensions: Option<&TriageDimensions>,
    ) -> Result<TriageQueueItem>;

    async fn peek_top(&self, limit: usize) -> Result<Vec<TriageQueueItem>>;

    async fn list(
        &self,
        status_filter: Option<TriageStatus>,
        limit: usize,
        offset: u64,
    ) -> Result<Vec<TriageQueueItem>>;

    async fn count(&self, status_filter: Option<TriageStatus>) -> Result<i64>;

    async fn stats(&self) -> Result<apex_core::triage::TriageStats>;

    async fn get_by_id(&self, id: Uuid) -> Result<Option<TriageQueueItem>>;

    async fn unscored_items(&self, limit: usize) -> Result<Vec<TriageQueueItem>>;

    async fn batch_update_scores(&self, scores: Vec<(Uuid, TriageDimensions)>) -> Result<u64>;

    async fn override_score(
        &self,
        id: Uuid,
        new_score: f64,
        overridden_by: &str,
    ) -> Result<TriageQueueItem>;

    async fn acknowledge(&self, id: Uuid) -> Result<TriageQueueItem>;

    async fn resolve(&self, id: Uuid) -> Result<TriageQueueItem>;

    async fn dismiss(&self, id: Uuid) -> Result<TriageQueueItem>;
}

#[async_trait]
impl TriageQueueProvider for TriageQueue {
    async fn enqueue(
        &self,
        item_type: TriageItemType,
        source_id: &str,
        title: &str,
        description: &str,
        entity_id: Option<Uuid>,
        entity_name: Option<&str>,
        static_severity: Option<&str>,
        dimensions: Option<&TriageDimensions>,
    ) -> Result<TriageQueueItem> {
        self.enqueue(
            item_type,
            source_id,
            title,
            description,
            entity_id,
            entity_name,
            static_severity,
            dimensions,
        )
        .await
    }

    async fn peek_top(&self, limit: usize) -> Result<Vec<TriageQueueItem>> {
        self.peek_top(limit).await
    }

    async fn list(
        &self,
        status_filter: Option<TriageStatus>,
        limit: usize,
        offset: u64,
    ) -> Result<Vec<TriageQueueItem>> {
        self.list(status_filter, limit, offset).await
    }

    async fn count(&self, status_filter: Option<TriageStatus>) -> Result<i64> {
        self.count(status_filter).await
    }

    async fn stats(&self) -> Result<apex_core::triage::TriageStats> {
        self.stats().await
    }

    async fn get_by_id(&self, id: Uuid) -> Result<Option<TriageQueueItem>> {
        self.get_by_id(id).await
    }

    async fn unscored_items(&self, limit: usize) -> Result<Vec<TriageQueueItem>> {
        self.unscored_items(limit).await
    }

    async fn batch_update_scores(&self, scores: Vec<(Uuid, TriageDimensions)>) -> Result<u64> {
        self.batch_update_scores(scores).await
    }

    async fn override_score(
        &self,
        id: Uuid,
        new_score: f64,
        overridden_by: &str,
    ) -> Result<TriageQueueItem> {
        self.override_score(id, new_score, overridden_by).await
    }

    async fn acknowledge(&self, id: Uuid) -> Result<TriageQueueItem> {
        self.acknowledge(id).await
    }

    async fn resolve(&self, id: Uuid) -> Result<TriageQueueItem> {
        self.resolve(id).await
    }

    async fn dismiss(&self, id: Uuid) -> Result<TriageQueueItem> {
        self.dismiss(id).await
    }
}

// ─── Internal Row Types ───────────────────────────────────────────────────────

#[derive(Debug, sqlx::FromRow)]
struct TriageQueueItemRow {
    id: Uuid,
    item_type: String,
    source_id: Uuid,
    title: String,
    description: String,
    entity_id: Option<Uuid>,
    entity_name: Option<String>,
    static_severity: Option<String>,
    urgency: f64,
    impact: f64,
    actionability: f64,
    novelty: f64,
    confidence: f64,
    composite_score: f64,
    is_overridden: bool,
    override_score: Option<f64>,
    status: String,
    created_at: DateTime<Utc>,
    triaged_at: Option<DateTime<Utc>>,
    acknowledged_at: Option<DateTime<Utc>>,
}

impl TriageQueueItemRow {
    fn into_item(self, thresholds: &TriageThresholds) -> TriageQueueItem {
        let effective_score = self.override_score.unwrap_or(self.composite_score);
        let band = score_to_band(effective_score, thresholds);
        let band_color = score_band_color(effective_score, thresholds);
        TriageQueueItem {
            id: self.id,
            item_type: TriageItemType::from_str(&self.item_type),
            source_id: self.source_id.to_string(),
            title: self.title,
            description: self.description,
            entity_id: self.entity_id,
            entity_name: self.entity_name,
            static_severity: self.static_severity,
            dimensions: Some(TriageDimensions {
                urgency: self.urgency,
                impact: self.impact,
                actionability: self.actionability,
                novelty: self.novelty,
                confidence: self.confidence,
            }),
            composite_score: effective_score,
            is_overridden: self.is_overridden,
            override_score: self.override_score,
            status: TriageStatus::from_str(&self.status),
            created_at: self.created_at,
            triaged_at: self.triaged_at,
            acknowledged_at: self.acknowledged_at,
            score_band: band.to_string(),
            score_band_color: band_color.to_string(),
        }
    }
}

#[derive(Debug, sqlx::FromRow)]
struct TriageStatsRow {
    total: i64,
    pending: i64,
    triaged: i64,
    acknowledged: i64,
    resolved: i64,
    dismissed: i64,
    overridden_count: i64,
    avg_score: f64,
}

impl TriageStatsRow {
    fn into_stats(self) -> apex_core::triage::TriageStats {
        let total = self.total as u64;
        let resolved_or_dismissed = (self.resolved + self.dismissed) as u64;
        apex_core::triage::TriageStats {
            total,
            pending: self.pending as u64,
            triaged: self.triaged as u64,
            acknowledged: self.acknowledged as u64,
            resolved: self.resolved as u64,
            dismissed: self.dismissed as u64,
            critical_count: 0,
            high_count: 0,
            medium_count: 0,
            low_count: 0,
            avg_urgency: 0.0,
            avg_impact: 0.0,
            avg_actionability: 0.0,
            avg_novelty: 0.0,
            avg_confidence: 0.0,
            avg_composite: self.avg_score,
            overridden_count: self.overridden_count as u64,
            override_rate: if total > 0 {
                self.overridden_count as f64 / total as f64
            } else {
                0.0
            },
            resolution_rate: if total > 0 {
                resolved_or_dismissed as f64 / total as f64
            } else {
                0.0
            },
        }
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use apex_core::triage::TriageThresholds;

    #[test]
    fn test_triage_item_type_from_str() {
        assert_eq!(TriageItemType::from_str("insight"), TriageItemType::Insight);
        assert_eq!(TriageItemType::from_str("Insight"), TriageItemType::Insight);
        assert_eq!(TriageItemType::from_str("warning"), TriageItemType::Warning);
        assert_eq!(TriageItemType::from_str("Warning"), TriageItemType::Warning);
        assert_eq!(TriageItemType::from_str("alert"), TriageItemType::Alert);
        assert_eq!(TriageItemType::from_str("Alert"), TriageItemType::Alert);
        assert_eq!(TriageItemType::from_str("unknown"), TriageItemType::Insight);
    }

    #[test]
    fn test_triage_status_from_str() {
        assert_eq!(TriageStatus::from_str("pending"), TriageStatus::Pending);
        assert_eq!(TriageStatus::from_str("triaged"), TriageStatus::Triaged);
        assert_eq!(
            TriageStatus::from_str("acknowledged"),
            TriageStatus::Acknowledged
        );
        assert_eq!(TriageStatus::from_str("resolved"), TriageStatus::Resolved);
        assert_eq!(TriageStatus::from_str("dismissed"), TriageStatus::Dismissed);
        assert_eq!(TriageStatus::from_str("unknown"), TriageStatus::Pending);
    }

    #[test]
    fn test_triage_stats_row_into_stats() {
        let row = TriageStatsRow {
            total: 100,
            pending: 40,
            triaged: 20,
            acknowledged: 15,
            resolved: 15,
            dismissed: 10,
            overridden_count: 5,
            avg_score: 0.45,
        };

        let stats = row.into_stats();
        assert_eq!(stats.total, 100);
        assert_eq!(stats.pending, 40);
        assert_eq!(stats.triaged, 20);
        assert_eq!(stats.acknowledged, 15);
        assert_eq!(stats.resolved, 15);
        assert_eq!(stats.dismissed, 10);
        assert_eq!(stats.overridden_count, 5);
        assert!((stats.override_rate - 0.05).abs() < 0.001);
        assert!((stats.resolution_rate - 0.25).abs() < 0.001);
        assert!((stats.avg_composite - 0.45).abs() < 0.001);
    }

    #[test]
    fn test_triage_stats_row_zero_total() {
        let row = TriageStatsRow {
            total: 0,
            pending: 0,
            triaged: 0,
            acknowledged: 0,
            resolved: 0,
            dismissed: 0,
            overridden_count: 0,
            avg_score: 0.0,
        };

        let stats = row.into_stats();
        assert_eq!(stats.total, 0);
        assert!((stats.override_rate - 0.0).abs() < 0.001);
        assert!((stats.resolution_rate - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_queue_item_row_to_item() {
        let thresholds = TriageThresholds::default();
        let row = TriageQueueItemRow {
            id: Uuid::new_v4(),
            item_type: "insight".to_string(),
            source_id: Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap_or_default(),
            title: "Test Insight".to_string(),
            description: "A test triage item".to_string(),
            entity_id: None,
            entity_name: Some("Acme Corp".to_string()),
            static_severity: Some("high".to_string()),
            urgency: 0.8,
            impact: 0.9,
            actionability: 0.7,
            novelty: 0.5,
            confidence: 0.85,
            composite_score: 0.82,
            is_overridden: false,
            override_score: None,
            status: "pending".to_string(),
            created_at: Utc::now(),
            triaged_at: None,
            acknowledged_at: None,
        };

        let item = row.into_item(&thresholds);
        assert_eq!(item.title, "Test Insight");
        assert!(item.dimensions.is_some());
        assert!((item.composite_score - 0.82).abs() < 0.001);
        assert!(matches!(item.status, TriageStatus::Pending));
        assert_eq!(item.score_band, "critical");
    }
}
