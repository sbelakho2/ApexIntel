//! Triage Queue DB operations — PgStore extension methods for the `triage_queue` table.
//!
//! These methods provide direct database access for operations that complement
//! the [`apex_triage::TriageQueue`] higher-level API.  API and web handlers
//! may call these when they need to join triage items with source data or
//! perform batch operations.

use anyhow::{Context, Result};
use uuid::Uuid;

use apex_core::triage::{composite_score, TriageDimensions, TriageThresholds, TriageWeights};

use crate::postgres::PgStore;

/// Default scoring-attempt cap: an item that failed this many LLM scoring
/// passes is no longer claimable by the retry loop.
pub const DEFAULT_MAX_TRIAGE_ATTEMPTS: i32 = 3;

// ─── Row types ────────────────────────────────────────────────────────────

/// A triage queue item with its source data joined (warning / insight).
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TriageItemWithSource {
    pub id: Uuid,
    pub item_type: String,
    pub source_id: String,
    pub title: String,
    pub description: Option<String>,
    pub entity_id: Option<String>,
    pub entity_name: Option<String>,
    pub static_severity: Option<String>,
    pub urgency: Option<f64>,
    pub impact: Option<f64>,
    pub actionability: Option<f64>,
    pub novelty: Option<f64>,
    pub confidence: Option<f64>,
    pub composite_score: Option<f64>,
    pub is_overridden: bool,
    pub override_score: Option<f64>,
    pub status: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub triaged_at: Option<chrono::DateTime<chrono::Utc>>,
    pub acknowledged_at: Option<chrono::DateTime<chrono::Utc>>,
    pub resolved_at: Option<chrono::DateTime<chrono::Utc>>,
    /// JSON-encoded source data (the warning or insight row as a JSON object)
    pub source_data: Option<serde_json::Value>,
}

// ─── PgStore extension methods ────────────────────────────────────────────

impl PgStore {
    /// Fetch a triage item together with its source record (warning or insight).
    ///
    /// The `source_data` field contains the full source row serialised as JSON.
    pub async fn get_triage_item_with_source(
        &self,
        item_id: Uuid,
    ) -> Result<Option<TriageItemWithSource>> {
        let row = sqlx::query_as::<_, TriageItemWithSource>(
            r#"
            SELECT
                q.id,
                q.item_type,
                q.source_id::text,
                q.title,
                q.description,
                q.entity_id,
                q.entity_name,
                q.static_severity,
                q.urgency,
                q.impact,
                q.actionability,
                q.novelty,
                q.confidence,
                q.composite_score,
                q.is_overridden,
                q.override_score,
                q.status::text,
                q.created_at,
                q.triaged_at,
                q.acknowledged_at,
                q.resolved_at,
                CASE q.item_type
                    WHEN 'insight' THEN (SELECT row_to_json(w.*) FROM insights w WHERE w.id = q.source_id::uuid)
                    WHEN 'warning' THEN (SELECT row_to_json(w.*) FROM warnings w WHERE w.id = q.source_id::uuid)
                    ELSE NULL
                END AS source_data
            FROM triage_queue q
            WHERE q.id = $1
            "#,
        )
        .bind(item_id)
        .fetch_optional(&self.pool)
        .await
        .context("failed to fetch triage item with source")?;

        Ok(row)
    }

    /// Count triage items grouped by static severity.
    pub async fn count_triage_by_severity(&self) -> Result<Vec<(String, i64)>> {
        let rows: Vec<(String, i64)> = sqlx::query_as(
            r#"
            SELECT COALESCE(static_severity, 'unknown') AS severity, COUNT(*)::bigint AS cnt
            FROM triage_queue
            GROUP BY severity
            ORDER BY cnt DESC
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .context("failed to count triage items by severity")?;

        Ok(rows)
    }

    /// Search triage queue items by title/description text.
    pub async fn search_triage_items(
        &self,
        query: &str,
        limit: usize,
        offset: u64,
    ) -> Result<Vec<TriageItemWithSource>> {
        let pattern = format!("%{}%", query);
        let limit = limit.min(100) as i64;
        let offset = offset as i64;

        let rows = sqlx::query_as::<_, TriageItemWithSource>(
            r#"
            SELECT
                q.id,
                q.item_type,
                q.source_id::text,
                q.title,
                q.description,
                q.entity_id,
                q.entity_name,
                q.static_severity,
                q.urgency,
                q.impact,
                q.actionability,
                q.novelty,
                q.confidence,
                q.composite_score,
                q.is_overridden,
                q.override_score,
                q.status::text,
                q.created_at,
                q.triaged_at,
                q.acknowledged_at,
                q.resolved_at,
                NULL AS source_data
            FROM triage_queue q
            WHERE q.title ILIKE $1 OR q.description ILIKE $1
            ORDER BY
                CASE WHEN q.is_overridden THEN COALESCE(q.override_score, q.composite_score) ELSE q.composite_score END DESC,
                q.created_at ASC
            LIMIT $2 OFFSET $3
            "#,
        )
        .bind(&pattern)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await
        .context("failed to search triage items")?;

        Ok(rows)
    }

    /// List pending items that have not yet been scored by the LLM
    /// (`composite_score = 0`) and still have scoring attempts left.
    ///
    /// The dimension columns are `NOT NULL DEFAULT 0.0` (migration 032), so
    /// "unscored" is `composite_score = 0` and the lifecycle state is
    /// `pending`; a NULL-dimension predicate would never match a real row.
    pub async fn list_unscored_triage_items(
        &self,
        limit: usize,
        max_attempts: i32,
    ) -> Result<Vec<TriageItemWithSource>> {
        let limit = limit.min(500) as i64;
        let max_attempts = max_attempts.max(1);

        let rows = sqlx::query_as::<_, TriageItemWithSource>(
            r#"
            SELECT
                q.id,
                q.item_type,
                q.source_id::text,
                q.title,
                q.description,
                q.entity_id,
                q.entity_name,
                q.static_severity,
                q.urgency,
                q.impact,
                q.actionability,
                q.novelty,
                q.confidence,
                q.composite_score,
                q.is_overridden,
                q.override_score,
                q.status::text,
                q.created_at,
                q.triaged_at,
                q.acknowledged_at,
                q.resolved_at,
                NULL AS source_data
            FROM triage_queue q
            WHERE q.status = 'pending'
              AND q.composite_score = 0.0
              AND q.triage_attempts < $2
            ORDER BY q.created_at ASC
            LIMIT $1
            "#,
        )
        .bind(limit)
        .bind(max_attempts)
        .fetch_all(&self.pool)
        .await
        .context("failed to list unscored triage items")?;

        Ok(rows)
    }

    /// Claim up to `limit` unscored items for scoring, atomically consuming one
    /// scoring attempt per claimed row.
    ///
    /// The `UPDATE ... FROM (SELECT ... FOR UPDATE SKIP LOCKED)` shape makes
    /// concurrent workers claim disjoint rows, and the `triage_attempts <
    /// max_attempts` predicate removes permanently failing items from the loop
    /// once their budget is spent. Claimed rows are returned with the
    /// post-increment attempt count applied.
    pub async fn claim_unscored_triage_items(
        &self,
        limit: usize,
        max_attempts: i32,
    ) -> Result<Vec<TriageItemWithSource>> {
        let limit = limit.clamp(1, 500) as i64;
        let max_attempts = max_attempts.max(1);

        let rows = sqlx::query_as::<_, TriageItemWithSource>(
            r#"
            WITH claimed AS (
                SELECT id
                FROM triage_queue
                WHERE status = 'pending'
                  AND composite_score = 0.0
                  AND triage_attempts < $2
                ORDER BY created_at ASC
                LIMIT $1
                FOR UPDATE SKIP LOCKED
            )
            UPDATE triage_queue q
               SET triage_attempts = q.triage_attempts + 1,
                   updated_at = NOW()
              FROM claimed c
             WHERE q.id = c.id
            RETURNING
                q.id,
                q.item_type,
                q.source_id::text,
                q.title,
                q.description,
                q.entity_id,
                q.entity_name,
                q.static_severity,
                q.urgency,
                q.impact,
                q.actionability,
                q.novelty,
                q.confidence,
                q.composite_score,
                q.is_overridden,
                q.override_score,
                q.status::text,
                q.created_at,
                q.triaged_at,
                q.acknowledged_at,
                q.resolved_at,
                NULL AS source_data
            "#,
        )
        .bind(limit)
        .bind(max_attempts)
        .fetch_all(&self.pool)
        .await
        .context("failed to claim unscored triage items")?;

        Ok(rows)
    }

    /// Apply LLM scoring dimensions to an item.
    ///
    /// Only overwrites the score dimensions when `dimensions` were actually
    /// supplied; a `None` result (the LLM returned nothing usable) consumes a
    /// scoring attempt and leaves every existing dimension untouched, so a
    /// failed second pass can never zero out a first successful score.
    pub async fn apply_triage_scores(
        &self,
        item_id: Uuid,
        dimensions: Option<&TriageDimensions>,
        weights: &TriageWeights,
    ) -> Result<bool> {
        let Some(dimensions) = dimensions else {
            self.record_triage_attempt_failure(item_id).await?;
            return Ok(false);
        };
        let composite = composite_score(dimensions, weights);

        let result = sqlx::query(
            r#"UPDATE triage_queue
                  SET urgency         = $2,
                      impact          = $3,
                      actionability   = $4,
                      novelty         = $5,
                      confidence      = $6,
                      composite_score = $7,
                      status          = 'triaged',
                      triaged_at      = NOW(),
                      updated_at      = NOW()
                WHERE id = $1
                  AND status IN ('pending', 'triaged')"#,
        )
        .bind(item_id)
        .bind(dimensions.urgency)
        .bind(dimensions.impact)
        .bind(dimensions.actionability)
        .bind(dimensions.novelty)
        .bind(dimensions.confidence)
        .bind(composite)
        .execute(&self.pool)
        .await
        .context("failed to apply triage scores")?;

        Ok(result.rows_affected() > 0)
    }

    /// Consume one scoring attempt for an item whose scoring pass produced no
    /// dimensions. Dimensions are deliberately not touched.
    pub async fn record_triage_attempt_failure(&self, item_id: Uuid) -> Result<()> {
        sqlx::query(
            r#"UPDATE triage_queue
                  SET triage_attempts = triage_attempts + 1,
                      updated_at = NOW()
                WHERE id = $1"#,
        )
        .bind(item_id)
        .execute(&self.pool)
        .await
        .context("failed to record triage attempt")?;
        Ok(())
    }

    /// Reopen a resolved/dismissed item when the underlying signal is seen
    /// again: back to `pending`, occurrence count incremented and
    /// `resolved_at` cleared so it re-enters the queue instead of being
    /// buried as closed.
    pub async fn reopen_triage_item_on_sighting(&self, item_id: Uuid) -> Result<bool> {
        let result = sqlx::query(
            r#"UPDATE triage_queue
                  SET status           = 'pending',
                      resolved_at      = NULL,
                      occurrence_count = occurrence_count + 1,
                      last_seen_at     = NOW(),
                      updated_at       = NOW()
                WHERE id = $1
                  AND status IN ('resolved', 'dismissed')"#,
        )
        .bind(item_id)
        .execute(&self.pool)
        .await
        .context("failed to reopen triage item on new sighting")?;
        Ok(result.rows_affected() > 0)
    }

    /// Count pending items per score band, computed in SQL from the configured
    /// thresholds using the same five bands as
    /// [`apex_core::triage::score_to_band`].
    pub async fn count_triage_bands(
        &self,
        thresholds: &TriageThresholds,
    ) -> Result<Vec<(String, i64)>> {
        let rows: Vec<(String, i64)> = sqlx::query_as(
            r#"
            SELECT
                CASE
                    WHEN composite_score >= $1 THEN 'critical'
                    WHEN composite_score >= $2 THEN 'high'
                    WHEN composite_score >= $3 THEN 'medium'
                    WHEN composite_score >= $4 THEN 'low'
                    ELSE 'info'
                END AS band,
                COUNT(*)::bigint AS cnt
            FROM triage_queue
            WHERE status = 'pending'
            GROUP BY band
            ORDER BY band DESC
            "#,
        )
        .bind(thresholds.critical)
        .bind(thresholds.high)
        .bind(thresholds.medium)
        .bind(thresholds.low)
        .fetch_all(&self.pool)
        .await
        .context("failed to count triage bands")?;
        Ok(rows)
    }

    /// Move a triage item to `target_status`, enforcing the lifecycle
    /// transition table in SQL.
    ///
    /// Valid moves: `pending → triaged | acknowledged | dismissed`,
    /// `triaged → acknowledged | resolved | dismissed`, and
    /// `acknowledged → resolved | dismissed`. Terminal states are only left
    /// through [`Self::reopen_triage_item_on_sighting`]. Returns `None` when
    /// the item does not exist or the transition is not allowed, so callers
    /// can answer 404/409 without a second read.
    pub async fn transition_triage_item(
        &self,
        item_id: Uuid,
        target_status: &str,
    ) -> Result<Option<TriageItemWithSource>> {
        let row = sqlx::query_as::<_, TriageItemWithSource>(
            r#"
            UPDATE triage_queue q
               SET status = $2,
                   triaged_at = CASE WHEN $2 = 'triaged' THEN NOW() ELSE triaged_at END,
                   acknowledged_at = CASE WHEN $2 = 'acknowledged' THEN NOW() ELSE acknowledged_at END,
                   resolved_at = CASE WHEN $2 = 'resolved' THEN NOW() ELSE resolved_at END,
                   updated_at = NOW()
             WHERE q.id = $1
               AND (
                     (q.status = 'pending'      AND $2 IN ('triaged', 'acknowledged', 'dismissed'))
                  OR (q.status = 'triaged'      AND $2 IN ('acknowledged', 'resolved', 'dismissed'))
                  OR (q.status = 'acknowledged' AND $2 IN ('resolved', 'dismissed'))
               )
            RETURNING
                q.id,
                q.item_type,
                q.source_id::text,
                q.title,
                q.description,
                q.entity_id,
                q.entity_name,
                q.static_severity,
                q.urgency,
                q.impact,
                q.actionability,
                q.novelty,
                q.confidence,
                q.composite_score,
                q.is_overridden,
                q.override_score,
                q.status::text,
                q.created_at,
                q.triaged_at,
                q.acknowledged_at,
                q.resolved_at,
                NULL AS source_data
            "#,
        )
        .bind(item_id)
        .bind(target_status)
        .fetch_optional(&self.pool)
        .await
        .context("failed to transition triage item")?;
        Ok(row)
    }

    /// Override an item's composite score, recording who overrode it.
    pub async fn override_triage_score(
        &self,
        item_id: Uuid,
        new_score: f64,
        overridden_by: &str,
    ) -> Result<Option<TriageItemWithSource>> {
        let clamped = new_score.clamp(0.0, 1.0);
        let row = sqlx::query_as::<_, TriageItemWithSource>(
            r#"
            UPDATE triage_queue q
               SET is_overridden = TRUE,
                   override_score = $2,
                   overridden_by = $3,
                   status = 'triaged',
                   triaged_at = COALESCE(triaged_at, NOW()),
                   updated_at = NOW()
             WHERE q.id = $1
            RETURNING
                q.id,
                q.item_type,
                q.source_id::text,
                q.title,
                q.description,
                q.entity_id,
                q.entity_name,
                q.static_severity,
                q.urgency,
                q.impact,
                q.actionability,
                q.novelty,
                q.confidence,
                q.composite_score,
                q.is_overridden,
                q.override_score,
                q.status::text,
                q.created_at,
                q.triaged_at,
                q.acknowledged_at,
                q.resolved_at,
                NULL AS source_data
            "#,
        )
        .bind(item_id)
        .bind(clamped)
        .bind(overridden_by)
        .fetch_optional(&self.pool)
        .await
        .context("failed to override triage score")?;
        Ok(row)
    }
}
