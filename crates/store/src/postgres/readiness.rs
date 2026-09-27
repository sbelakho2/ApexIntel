//! Readiness state persisted by the worker and read by API probes.
//!
//! The API cannot inspect the worker's in-memory alert-rule engine, so the
//! worker publishes a singleton snapshot to `alert_engine_state` (migration
//! 069) at startup and refreshes it on its heartbeat cadence. Readiness probes
//! read that snapshot and fail when it is missing, unsuccessful, empty, or
//! stale — a configuration file existing on disk is never accepted as proof
//! that the engine is loaded.

use super::*;

/// One row of `alert_engine_state`.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct AlertEngineStateRow {
    pub id: String,
    pub rules_path: String,
    pub rule_count: i32,
    pub config_hash: Option<String>,
    pub last_reload_success: bool,
    pub last_reload_error: Option<String>,
    pub last_reload_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

/// Snapshot written by the worker after reading and hashing the rules file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlertEngineStateRecord {
    pub rules_path: String,
    pub rule_count: i32,
    pub config_hash: Option<String>,
    pub last_reload_success: bool,
    pub last_reload_error: Option<String>,
}

impl PgStore {
    /// Upsert the singleton alert-engine state row.
    ///
    /// `last_reload_at` only advances on a successful reload; a failure keeps
    /// the previous successful timestamp while still refreshing `updated_at`,
    /// so operators can distinguish "never loaded" from "was loaded, now
    /// failing".
    pub async fn record_alert_engine_state(&self, state: &AlertEngineStateRecord) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO alert_engine_state
                   (id, rules_path, rule_count, config_hash, last_reload_success,
                    last_reload_error, last_reload_at, updated_at)
               VALUES ('default', $1, $2, $3, $4, $5,
                       CASE WHEN $4 THEN NOW() ELSE NULL END, NOW())
               ON CONFLICT (id) DO UPDATE SET
                   rules_path = EXCLUDED.rules_path,
                   rule_count = EXCLUDED.rule_count,
                   config_hash = EXCLUDED.config_hash,
                   last_reload_success = EXCLUDED.last_reload_success,
                   last_reload_error = EXCLUDED.last_reload_error,
                   last_reload_at = CASE
                       WHEN EXCLUDED.last_reload_success THEN NOW()
                       ELSE alert_engine_state.last_reload_at
                   END,
                   updated_at = NOW()"#,
        )
        .bind(&state.rules_path)
        .bind(state.rule_count)
        .bind(state.config_hash.as_deref())
        .bind(state.last_reload_success)
        .bind(state.last_reload_error.as_deref())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Read the singleton alert-engine state row, if the worker ever wrote one.
    pub async fn alert_engine_state(&self) -> Result<Option<AlertEngineStateRow>> {
        let row = sqlx::query_as::<_, AlertEngineStateRow>(
            r#"SELECT id, rules_path, rule_count, config_hash, last_reload_success,
                      last_reload_error, last_reload_at, updated_at
                 FROM alert_engine_state
                WHERE id = 'default'"#,
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }
}
