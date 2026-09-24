use super::*;

/// One process liveness row from `service_heartbeats` (migration 049).
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct ServiceHeartbeatRow {
    pub service: String,
    pub instance_id: String,
    pub version: String,
    pub last_seen_at: DateTime<Utc>,
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
