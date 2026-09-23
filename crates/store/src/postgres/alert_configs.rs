//! Store methods for alert threshold configurations.
//!
//! Provides CRUD operations on:
//! - `entity_alert_configs` — per-entity alert overrides (JSONB)
//! - `global_alert_defaults` — singleton row with global defaults (JSONB)

use super::*;

use apex_core::alert_config::{EntityAlertConfig, GlobalAlertDefaults};

impl PgStore {
    // ── Entity-level configs ──────────────────────────────────────────────────

    /// Retrieve the alert config for a single entity, if one exists.
    pub async fn get_entity_alert_config(
        &self,
        entity_id: &str,
    ) -> Result<Option<EntityAlertConfig>> {
        let row: Option<(serde_json::Value,)> =
            sqlx::query_as("SELECT config FROM entity_alert_configs WHERE entity_id = $1")
                .bind(entity_id)
                .fetch_optional(&self.pool)
                .await?;

        match row {
            Some((json,)) => {
                let cfg: EntityAlertConfig = serde_json::from_value(json)?;
                Ok(Some(cfg))
            }
            None => Ok(None),
        }
    }

    /// Return all entity alert configs stored in the database.
    pub async fn list_entity_alert_configs(&self) -> Result<Vec<EntityAlertConfig>> {
        let rows: Vec<(String, serde_json::Value)> =
            sqlx::query_as("SELECT entity_id, config FROM entity_alert_configs ORDER BY entity_id")
                .fetch_all(&self.pool)
                .await?;

        let mut configs = Vec::with_capacity(rows.len());
        for (_, json) in rows {
            match serde_json::from_value(json) {
                Ok(cfg) => configs.push(cfg),
                Err(e) => {
                    tracing::warn!("Skipping malformed entity alert config: {e}");
                }
            }
        }
        Ok(configs)
    }

    /// Insert or update the alert config for an entity.
    pub async fn upsert_entity_alert_config(
        &self,
        entity_id: &str,
        config: &EntityAlertConfig,
    ) -> Result<()> {
        let json = serde_json::to_value(config)?;

        sqlx::query(
            r#"
            INSERT INTO entity_alert_configs (entity_id, config, updated_at)
            VALUES ($1, $2, now())
            ON CONFLICT (entity_id)
            DO UPDATE SET config = $2, updated_at = now()
            "#,
        )
        .bind(entity_id)
        .bind(&json)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Remove the alert config for an entity, restoring global defaults.
    pub async fn delete_entity_alert_config(&self, entity_id: &str) -> Result<bool> {
        let result = sqlx::query("DELETE FROM entity_alert_configs WHERE entity_id = $1")
            .bind(entity_id)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    // ── Global defaults ──────────────────────────────────────────────────────

    /// Retrieve the global alert defaults (the singleton row).
    pub async fn get_global_alert_defaults(&self) -> Result<Option<GlobalAlertDefaults>> {
        let row: Option<(serde_json::Value,)> =
            sqlx::query_as("SELECT config FROM global_alert_defaults WHERE id = 1")
                .fetch_optional(&self.pool)
                .await?;

        match row {
            Some((json,)) => {
                let cfg: GlobalAlertDefaults = serde_json::from_value(json)?;
                Ok(Some(cfg))
            }
            None => Ok(None),
        }
    }

    /// Upsert the global alert defaults singleton row.
    pub async fn upsert_global_alert_defaults(&self, config: &GlobalAlertDefaults) -> Result<()> {
        let json = serde_json::to_value(config)?;

        sqlx::query(
            r#"
            INSERT INTO global_alert_defaults (id, config, updated_at)
            VALUES (1, $1, now())
            ON CONFLICT (id)
            DO UPDATE SET config = $1, updated_at = now()
            "#,
        )
        .bind(&json)
        .execute(&self.pool)
        .await?;

        Ok(())
    }
}
