use super::*;

fn normalize_artifact_window(limit: i64, offset: i64) -> (i64, i64) {
    (clamp_limit(limit), offset.max(0))
}

impl PgStore {
    pub async fn insert_poi_artifact(&self, a: &PoiArtifact) -> Result<()> {
        let prov_json = serde_json::to_value(&a.provenance)?;
        sqlx::query(
            r#"INSERT INTO poi_artifacts
               (id, person_id, artifact_type, title, content_summary,
                url, source_domain, language, topics, sentiment_score,
                key_phrases, ts_utc, provenance, metadata, created_at)
               VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15)
               ON CONFLICT (id) DO NOTHING"#,
        )
        .bind(a.id)
        .bind(a.person_id)
        .bind(a.artifact_type.as_str())
        .bind(&a.title)
        .bind(&a.content_summary)
        .bind(&a.url)
        .bind(&a.source_domain)
        .bind(&a.language)
        .bind(&a.topics)
        .bind(a.sentiment_score)
        .bind(&a.key_phrases)
        .bind(a.ts_utc)
        .bind(&prov_json)
        .bind(&a.metadata)
        .bind(a.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_artifacts_for_person(
        &self,
        person_id: Uuid,
        limit: i64,
    ) -> Result<Vec<ArtifactRow>> {
        let (limit, _) = normalize_artifact_window(limit, 0);
        let rows = sqlx::query_as::<_, ArtifactRow>(
            "SELECT id, person_id, artifact_type, title, content_summary,
                    url, source_domain, language, topics, sentiment_score,
                    key_phrases, ts_utc, provenance, metadata, created_at
             FROM poi_artifacts
             WHERE person_id = $1
             ORDER BY ts_utc DESC
             LIMIT $2",
        )
        .bind(person_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn list_poi_artifacts(
        &self,
        person_id: Option<Uuid>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<ArtifactRow>> {
        let (limit, offset) = normalize_artifact_window(limit, offset);
        match person_id {
            Some(pid) => Ok(sqlx::query_as::<_, ArtifactRow>(
                "SELECT id, person_id, artifact_type, title, content_summary,
                            url, source_domain, language, topics, sentiment_score,
                            key_phrases, ts_utc, provenance, metadata, created_at
                     FROM poi_artifacts
                     WHERE person_id = $3
                     ORDER BY ts_utc DESC
                     LIMIT $1 OFFSET $2",
            )
            .bind(limit)
            .bind(offset)
            .bind(pid)
            .fetch_all(&self.pool)
            .await?),
            None => Ok(sqlx::query_as::<_, ArtifactRow>(
                "SELECT id, person_id, artifact_type, title, content_summary,
                            url, source_domain, language, topics, sentiment_score,
                            key_phrases, ts_utc, provenance, metadata, created_at
                     FROM poi_artifacts
                     ORDER BY ts_utc DESC
                     LIMIT $1 OFFSET $2",
            )
            .bind(limit)
            .bind(offset)
            .fetch_all(&self.pool)
            .await?),
        }
    }

    pub async fn count_poi_artifacts(&self, person_id: Option<Uuid>) -> Result<i64> {
        let count: i64 = match person_id {
            Some(pid) => {
                sqlx::query_scalar("SELECT COUNT(*) FROM poi_artifacts WHERE person_id = $1")
                    .bind(pid)
                    .fetch_one(&self.pool)
                    .await?
            }
            None => {
                sqlx::query_scalar("SELECT COUNT(*) FROM poi_artifacts")
                    .fetch_one(&self.pool)
                    .await?
            }
        };
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_artifact_window;

    #[test]
    fn test_normalize_artifact_window_clamps_limit_and_offset() {
        assert_eq!(normalize_artifact_window(0, -2), (1, 0));
        assert_eq!(normalize_artifact_window(9999, 8), (500, 8));
    }

    #[test]
    fn test_normalize_artifact_window_preserves_valid_values() {
        assert_eq!(normalize_artifact_window(20, 5), (20, 5));
    }
}
