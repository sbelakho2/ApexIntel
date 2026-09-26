use super::*;

/// One process liveness row from `service_heartbeats` (migration 049).
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct ServiceHeartbeatRow {
    pub service: String,
    pub instance_id: String,
    pub version: String,
    pub last_seen_at: DateTime<Utc>,
}

/// A service heartbeat older than this is considered stale (heartbeats are
/// written every ~30s, so this tolerates three missed writes). Defined once
/// here so the API capability probes, the UI status strip, and the worker
/// container healthcheck agree on what "stale" means.
pub const WORKER_HEARTBEAT_STALE_AFTER_SECS: i64 = 120;

/// Newest heartbeat for one service instance, over a caller-provided
/// connection. Used by the worker healthcheck so a container verifies its own
/// instance's liveness instead of whichever worker beat most recently.
pub async fn latest_service_instance_heartbeat(
    conn: &mut sqlx::postgres::PgConnection,
    service: &str,
    instance_id: &str,
) -> Result<Option<ServiceHeartbeatRow>> {
    let row = sqlx::query_as::<_, ServiceHeartbeatRow>(
        r#"SELECT service, instance_id, version, last_seen_at
           FROM service_heartbeats
           WHERE service = $1 AND instance_id = $2
           ORDER BY last_seen_at DESC
           LIMIT 1"#,
    )
    .bind(service)
    .bind(instance_id)
    .fetch_optional(conn)
    .await?;
    Ok(row)
}

impl PgStore {
    /// Upsert a heartbeat for one process instance. Called every ~30s by the
    /// API and worker so health checks can measure real liveness.
    pub async fn record_service_heartbeat(
        &self,
        service: &str,
        instance_id: &str,
        version: &str,
    ) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO service_heartbeats (service, instance_id, version, last_seen_at)
               VALUES ($1, $2, $3, NOW())
               ON CONFLICT (service, instance_id)
               DO UPDATE SET version = EXCLUDED.version,
                             last_seen_at = NOW(),
                             updated_at = NOW()"#,
        )
        .bind(service)
        .bind(instance_id)
        .bind(version)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Newest heartbeat for a service (`worker`, `api`), if any.
    pub async fn latest_service_heartbeat(
        &self,
        service: &str,
    ) -> Result<Option<ServiceHeartbeatRow>> {
        let row = sqlx::query_as::<_, ServiceHeartbeatRow>(
            r#"SELECT service, instance_id, version, last_seen_at
               FROM service_heartbeats
               WHERE service = $1
               ORDER BY last_seen_at DESC
               LIMIT 1"#,
        )
        .bind(service)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// Timestamp of the most recent observation, used for data-freshness
    /// reporting (`NULL` when the table is genuinely empty).
    pub async fn newest_observation_ts(&self) -> Result<Option<DateTime<Utc>>> {
        let ts =
            sqlx::query_scalar::<_, Option<DateTime<Utc>>>("SELECT MAX(ts_utc) FROM observations")
                .fetch_one(&self.pool)
                .await?;
        Ok(ts)
    }
}
