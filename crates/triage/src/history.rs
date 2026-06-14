//! Triage History — records of triage decisions and manual overrides.
//!
//! [`TriageHistory`] provides DB-backed operations for recording, querying,
//! and analyzing historical triage decisions.

use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use tracing::instrument;
use uuid::Uuid;

use apex_core::triage::{TriageDecision, TriageDecisionType, TriageDimensions};

// ─── TriageHistory ────────────────────────────────────────────────────────────

/// Records and queries triage decision history.
#[derive(Debug, Clone)]
pub struct TriageHistory {
    pool: PgPool,
}

impl TriageHistory {
    /// Create a new [`TriageHistory`] backed by the given pool.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Record an auto-triage decision for a queue item.
    #[instrument(skip(self))]
    pub async fn record_auto_triage(
        &self,
        queue_item_id: Uuid,
        dimensions: &TriageDimensions,
        composite: f64,
    ) -> Result<TriageDecision> {
        let row = sqlx::query_as::<_, TriageDecisionRow>(
            r#"
            INSERT INTO triage_decisions
                (queue_item_id, original_dimensions, original_composite,
                 decision_type, created_at)
            VALUES
                ($1, $2, $3, 'auto_triage', NOW())
            RETURNING
                id, queue_item_id,
                original_dimensions, original_composite,
                override_dimensions, override_composite,
                overridden_by, overridden_at,
                decision_type::text, created_at
            "#,
        )
        .bind(queue_item_id)
        .bind(serde_json::to_value(dimensions)?)
        .bind(composite)
        .fetch_one(&self.pool)
        .await?;

        Ok(row.into_decision())
    }

    /// Record a user override decision.
    #[instrument(skip(self))]
    pub async fn record_override(
        &self,
        queue_item_id: Uuid,
        original_dimensions: &TriageDimensions,
        original_composite: f64,
        override_dimensions: &TriageDimensions,
        override_composite: f64,
        overridden_by: &str,
    ) -> Result<TriageDecision> {
        let row = sqlx::query_as::<_, TriageDecisionRow>(
            r#"
            INSERT INTO triage_decisions
                (queue_item_id, original_dimensions, original_composite,
                 override_dimensions, override_composite,
                 overridden_by, overridden_at,
                 decision_type, created_at)
            VALUES
                ($1, $2, $3, $4, $5, $6, NOW(), 'user_override', NOW())
            RETURNING
                id, queue_item_id,
                original_dimensions, original_composite,
                override_dimensions, override_composite,
                overridden_by, overridden_at,
                decision_type::text, created_at
            "#,
        )
        .bind(queue_item_id)
        .bind(serde_json::to_value(original_dimensions)?)
        .bind(original_composite)
        .bind(serde_json::to_value(override_dimensions)?)
        .bind(override_composite)
        .bind(overridden_by)
        .fetch_one(&self.pool)
        .await?;

        Ok(row.into_decision())
    }

    /// Get the decision history for a specific queue item.
    pub async fn get_history_for_item(&self, queue_item_id: Uuid) -> Result<Vec<TriageDecision>> {
        let rows = sqlx::query_as::<_, TriageDecisionRow>(
            r#"
            SELECT id, queue_item_id,
                   original_dimensions, original_composite,
                   override_dimensions, override_composite,
                   overridden_by, overridden_at,
                   decision_type::text, created_at
            FROM triage_decisions
            WHERE queue_item_id = $1
            ORDER BY created_at DESC
            "#,
        )
        .bind(queue_item_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|r| r.into_decision()).collect())
    }

    /// Get the most recent decisions across the entire system.
    pub async fn get_recent_decisions(&self, limit: usize) -> Result<Vec<TriageDecision>> {
        let limit = limit.min(100) as i64;

        let rows = sqlx::query_as::<_, TriageDecisionRow>(
            r#"
            SELECT id, queue_item_id,
                   original_dimensions, original_composite,
                   override_dimensions, override_composite,
                   overridden_by, overridden_at,
                   decision_type::text, created_at
            FROM triage_decisions
            ORDER BY created_at DESC
            LIMIT $1
            "#,
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|r| r.into_decision()).collect())
    }

    /// Count how many overrides have occurred (optionally within a timeframe).
    pub async fn override_frequency(
        &self,
        since: Option<DateTime<Utc>>,
    ) -> Result<u64> {
        let row: (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(*)::bigint
            FROM triage_decisions
            WHERE decision_type = 'user_override'
              AND ($1::timestamptz IS NULL OR created_at >= $1)
            "#,
        )
        .bind(since)
        .fetch_one(&self.pool)
        .await?;

        Ok(row.0 as u64)
    }

    /// Get the total count of triage decisions.
    pub async fn total_decisions(&self) -> Result<u64> {
        let row: (i64,) = sqlx::query_as(
            r#"SELECT COUNT(*)::bigint FROM triage_decisions"#,
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(row.0 as u64)
    }
}

// ─── Internal Row Types ───────────────────────────────────────────────────────

#[derive(Debug, sqlx::FromRow)]
struct TriageDecisionRow {
    id: Uuid,
    queue_item_id: Uuid,
    original_dimensions: serde_json::Value,
    original_composite: f64,
    override_dimensions: Option<serde_json::Value>,
    override_composite: Option<f64>,
    overridden_by: Option<String>,
    overridden_at: Option<DateTime<Utc>>,
    decision_type: String,
    created_at: DateTime<Utc>,
}

impl TriageDecisionRow {
    fn into_decision(self) -> TriageDecision {
        let original_dims: TriageDimensions =
            serde_json::from_value(self.original_dimensions)
                .unwrap_or(TriageDimensions {
                    urgency: 0.0,
                    impact: 0.0,
                    actionability: 0.0,
                    novelty: 0.0,
                    confidence: 0.0,
                });

        let override_dims = self.override_dimensions.and_then(|v| {
            serde_json::from_value(v).ok()
        });

        TriageDecision {
            id: self.id,
            queue_item_id: self.queue_item_id,
            original_dimensions: original_dims,
            original_composite: self.original_composite,
            override_dimensions: override_dims,
            override_composite: self.override_composite,
            overridden_by: self.overridden_by,
            overridden_at: self.overridden_at,
            decision_type: match self.decision_type.as_str() {
                "user_override" => TriageDecisionType::UserOverride,
                _ => TriageDecisionType::AutoTriage,
            },
            created_at: self.created_at,
        }
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decision_type_from_str() {
        let row = TriageDecisionRow {
            id: Uuid::new_v4(),
            queue_item_id: Uuid::new_v4(),
            original_dimensions: serde_json::json!({
                "urgency": 0.8, "impact": 0.9, "actionability": 0.6,
                "novelty": 0.4, "confidence": 0.7,
            }),
            original_composite: 0.755,
            override_dimensions: None,
            override_composite: None,
            overridden_by: None,
            overridden_at: None,
            decision_type: "auto_triage".to_string(),
            created_at: Utc::now(),
        };

        let decision = row.into_decision();
        assert_eq!(decision.original_composite, 0.755);
        assert!((decision.original_dimensions.urgency - 0.8).abs() < 1e-9);
        assert!(matches!(decision.decision_type, TriageDecisionType::AutoTriage));
        assert!(decision.override_dimensions.is_none());
    }

    #[test]
    fn test_user_override_decision() {
        let row = TriageDecisionRow {
            id: Uuid::new_v4(),
            queue_item_id: Uuid::new_v4(),
            original_dimensions: serde_json::json!({
                "urgency": 0.5, "impact": 0.5, "actionability": 0.5,
                "novelty": 0.5, "confidence": 0.5,
            }),
            original_composite: 0.5,
            override_dimensions: Some(serde_json::json!({
                "urgency": 0.9, "impact": 0.9, "actionability": 0.8,
                "novelty": 0.3, "confidence": 1.0,
            })),
            override_composite: Some(0.88),
            overridden_by: Some("user-abc".to_string()),
            overridden_at: Some(Utc::now()),
            decision_type: "user_override".to_string(),
            created_at: Utc::now(),
        };

        let decision = row.into_decision();
        assert!(matches!(decision.decision_type, TriageDecisionType::UserOverride));
        assert!(decision.override_dimensions.is_some());
        assert!(decision.override_composite.is_some());
        assert_eq!(decision.overridden_by.unwrap(), "user-abc");
    }
}
