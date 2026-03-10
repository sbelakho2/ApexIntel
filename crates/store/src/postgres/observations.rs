use super::*;

fn normalize_observation_window(limit: i64, offset: i64) -> (i64, i64) {
    (clamp_limit(limit), offset.max(0))
}

impl PgStore {
    pub async fn insert_observation(&self, o: &Observation) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO observations
               (id, observation_type, entity_id, entity_type, ts_utc,
                value, provenance, confidence, created_at)
               VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)
               ON CONFLICT (id) DO NOTHING"#,
        )
        .bind(o.id)
        .bind(o.observation_type.as_str())
        .bind(o.entity_id)
        .bind(&o.entity_type)
        .bind(o.ts_utc)
        .bind(&o.value)
        .bind(&o.provenance)
        .bind(o.confidence)
        .bind(o.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_observations_by_entity(
        &self,
        entity_id: Uuid,
        limit: i64,
    ) -> Result<Vec<ObservationRow>> {
        let (limit, _) = normalize_observation_window(limit, 0);
        let rows = sqlx::query_as::<_, ObservationRow>(
            "SELECT id, observation_type, entity_id, entity_type, ts_utc,
                    value, provenance, confidence, created_at
             FROM observations
             WHERE entity_id = $1
             ORDER BY ts_utc DESC
             LIMIT $2",
        )
        .bind(entity_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn get_observations_by_type(
        &self,
        obs_type: &str,
        since: DateTime<Utc>,
        limit: i64,
    ) -> Result<Vec<ObservationRow>> {
        let (limit, _) = normalize_observation_window(limit, 0);
        let rows = sqlx::query_as::<_, ObservationRow>(
            "SELECT id, observation_type, entity_id, entity_type, ts_utc,
                    value, provenance, confidence, created_at
             FROM observations
             WHERE observation_type = $1 AND ts_utc >= $2
             ORDER BY ts_utc DESC
             LIMIT $3",
        )
        .bind(obs_type)
        .bind(since)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn list_observations(
        &self,
        entity_id: Option<Uuid>,
        obs_type: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<ObservationRow>> {
        let (limit, offset) = normalize_observation_window(limit, offset);
        match (entity_id, obs_type) {
            (Some(entity_id), Some(obs_type)) => {
                Ok(sqlx::query_as::<_, ObservationRow>(
                    "SELECT * FROM observations WHERE entity_id = $3 AND observation_type = $4 ORDER BY ts_utc DESC LIMIT $1 OFFSET $2",
                )
                .bind(limit)
                .bind(offset)
                .bind(entity_id)
                .bind(obs_type)
                .fetch_all(&self.pool)
                .await?)
            }
            (Some(entity_id), None) => {
                Ok(sqlx::query_as::<_, ObservationRow>(
                    "SELECT * FROM observations WHERE entity_id = $3 ORDER BY ts_utc DESC LIMIT $1 OFFSET $2",
                )
                .bind(limit)
                .bind(offset)
                .bind(entity_id)
                .fetch_all(&self.pool)
                .await?)
            }
            (None, Some(obs_type)) => {
                Ok(sqlx::query_as::<_, ObservationRow>(
                    "SELECT * FROM observations WHERE observation_type = $3 ORDER BY ts_utc DESC LIMIT $1 OFFSET $2",
                )
                .bind(limit)
                .bind(offset)
                .bind(obs_type)
                .fetch_all(&self.pool)
                .await?)
            }
            (None, None) => {
                Ok(sqlx::query_as::<_, ObservationRow>(
                    "SELECT * FROM observations ORDER BY ts_utc DESC LIMIT $1 OFFSET $2",
                )
                .bind(limit)
                .bind(offset)
                .fetch_all(&self.pool)
                .await?)
            }
        }
    }

    pub async fn count_observations(
        &self,
        entity_id: Option<Uuid>,
        obs_type: Option<&str>,
    ) -> Result<i64> {
        let count: i64 = match (entity_id, obs_type) {
            (Some(entity_id), Some(obs_type)) => sqlx::query_scalar(
                "SELECT COUNT(*) FROM observations WHERE entity_id = $1 AND observation_type = $2",
            )
            .bind(entity_id)
            .bind(obs_type)
            .fetch_one(&self.pool)
            .await?,
            (Some(entity_id), None) => {
                sqlx::query_scalar("SELECT COUNT(*) FROM observations WHERE entity_id = $1")
                    .bind(entity_id)
                    .fetch_one(&self.pool)
                    .await?
            }
            (None, Some(obs_type)) => {
                sqlx::query_scalar("SELECT COUNT(*) FROM observations WHERE observation_type = $1")
                    .bind(obs_type)
                    .fetch_one(&self.pool)
                    .await?
            }
            (None, None) => {
                sqlx::query_scalar("SELECT COUNT(*) FROM observations")
                    .fetch_one(&self.pool)
                    .await?
            }
        };
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_observation_window;

    #[test]
    fn test_normalize_observation_window_clamps_limit_and_offset() {
        assert_eq!(normalize_observation_window(0, -2), (1, 0));
        assert_eq!(normalize_observation_window(9999, 9), (500, 9));
    }

    #[test]
    fn test_normalize_observation_window_preserves_valid_values() {
        assert_eq!(normalize_observation_window(30, 6), (30, 6));
    }
}
