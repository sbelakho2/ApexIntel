//! Triage Queue DB operations — PgStore extension methods for the `triage_queue` table.
//!
//! These methods provide direct database access for operations that complement
//! the [`apex_triage::TriageQueue`] higher-level API.  API and web handlers
//! may call these when they need to join triage items with source data or
//! perform batch operations.

use anyhow::{Context, Result};
use uuid::Uuid;

use crate::postgres::PgStore;

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
    pub async fn get_triage_item_with_source(&self, item_id: Uuid) -> Result<Option<TriageItemWithSource>> {
        let row = sqlx::query_as::<_, TriageItemWithSource>(
            r#"
            SELECT
                q.id,
                q.item_type,
                q.source_id,
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
                q.source_id,
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

    /// List items that have not yet been scored by the LLM (all dimension columns are NULL).
    pub async fn list_unscored_triage_items(&self, limit: usize) -> Result<Vec<TriageItemWithSource>> {
        let limit = limit.min(500) as i64;

        let rows = sqlx::query_as::<_, TriageItemWithSource>(
            r#"
            SELECT
                q.id,
                q.item_type,
                q.source_id,
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
            WHERE q.urgency IS NULL
               OR q.impact IS NULL
               OR q.actionability IS NULL
               OR q.novelty IS NULL
               OR q.confidence IS NULL
            ORDER BY q.created_at ASC
            LIMIT $1
            "#,
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .context("failed to list unscored triage items")?;

        Ok(rows)
    }
}
