use super::*;

/// A pending entity-review row (migration 058).
///
/// Written by [`PgStore::enqueue_entity_review`]; the worker adapter builds
/// these from the insights `EntityReviewEntry` admission type.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EntityReviewRow {
    pub candidate_id: Uuid,
    pub candidate_name: String,
    pub normalized_name: String,
    pub confidence: f64,
    pub outcome: String,
    pub review_reasons: Vec<String>,
    pub evidence: serde_json::Value,
    pub metadata: serde_json::Value,
    pub source: String,
}

impl PgStore {
    /// Enqueue an entity-review row, returning its id.
    ///
    /// Idempotent per candidate: if a pending row already exists for
    /// `candidate_id` it is returned instead of inserting a duplicate (the
    /// partial unique index in migration 058 enforces this under races).
    pub async fn enqueue_entity_review(&self, entry: &EntityReviewRow) -> Result<Uuid> {
        let inserted = sqlx::query_as::<_, (Uuid,)>(
            r#"
            INSERT INTO entity_review_queue
                (candidate_id, candidate_name, normalized_name, confidence, outcome,
                 review_reasons, evidence, metadata, source, status)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'pending')
            ON CONFLICT DO NOTHING
            RETURNING id
            "#,
        )
        .bind(entry.candidate_id)
        .bind(&entry.candidate_name)
        .bind(&entry.normalized_name)
        .bind(entry.confidence)
        .bind(&entry.outcome)
        .bind(&entry.review_reasons)
        .bind(&entry.evidence)
        .bind(&entry.metadata)
        .bind(&entry.source)
        .fetch_optional(&self.pool)
        .await?;

        if let Some((id,)) = inserted {
            return Ok(id);
        }

        let existing = sqlx::query_as::<_, (Uuid,)>(
            "SELECT id FROM entity_review_queue
             WHERE candidate_id = $1 AND status = 'pending'
             ORDER BY created_at DESC
             LIMIT 1",
        )
        .bind(entry.candidate_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(existing.0)
    }
}
