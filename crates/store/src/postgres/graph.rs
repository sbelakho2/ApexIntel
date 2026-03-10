use super::*;

fn normalize_graph_limit(limit: u32) -> i64 {
    (limit as i64).clamp(1, MAX_LIST_LIMIT)
}

impl PgStore {
    /// Extract graph-edge features per entity.
    pub async fn get_graph_edge_features(&self) -> Result<Vec<(Uuid, String, i64)>> {
        let rows = sqlx::query(
            r#"SELECT source_id,
                      edge_type,
                      COUNT(*)::BIGINT AS cnt
               FROM graph_edges
               GROUP BY source_id, edge_type
               ORDER BY source_id"#,
        )
        .fetch_all(&self.pool)
        .await?;

        use sqlx::Row as _;
        Ok(rows
            .into_iter()
            .filter_map(|row| {
                let sid: Uuid = row.try_get("source_id").ok()?;
                let et: String = row.try_get("edge_type").ok()?;
                let cnt: i64 = row.try_get("cnt").ok()?;
                Some((sid, et, cnt))
            })
            .collect())
    }

    pub async fn upsert_edge(&self, e: &GraphEdge) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO graph_edges
               (id, source_id, source_type, target_id, target_type,
                edge_type, weight, confidence, evidence_ids, metadata,
                first_seen, last_seen)
               VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)
               ON CONFLICT (source_id, source_type, target_id, target_type, edge_type)
               DO UPDATE SET
                 weight = EXCLUDED.weight,
                 confidence = EXCLUDED.confidence,
                 evidence_ids = EXCLUDED.evidence_ids,
                 metadata = EXCLUDED.metadata,
                 last_seen = now()"#,
        )
        .bind(e.id)
        .bind(e.source_id)
        .bind(&e.source_type)
        .bind(e.target_id)
        .bind(&e.target_type)
        .bind(e.edge_type.as_str())
        .bind(e.weight)
        .bind(e.confidence)
        .bind(&e.evidence_ids)
        .bind(&e.metadata)
        .bind(e.first_seen)
        .bind(e.last_seen)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_edges_from(&self, source_id: Uuid, source_type: &str) -> Result<Vec<EdgeRow>> {
        let rows = sqlx::query_as::<_, EdgeRow>(
            "SELECT id, source_id, source_type, target_id, target_type,
                    edge_type, weight, confidence, evidence_ids, metadata,
                    first_seen, last_seen
             FROM graph_edges
             WHERE source_id = $1 AND source_type = $2
             ORDER BY weight DESC",
        )
        .bind(source_id)
        .bind(source_type)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn get_edges_to(&self, target_id: Uuid, target_type: &str) -> Result<Vec<EdgeRow>> {
        let rows = sqlx::query_as::<_, EdgeRow>(
            "SELECT id, source_id, source_type, target_id, target_type,
                    edge_type, weight, confidence, evidence_ids, metadata,
                    first_seen, last_seen
             FROM graph_edges
             WHERE target_id = $1 AND target_type = $2
             ORDER BY weight DESC",
        )
        .bind(target_id)
        .bind(target_type)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// List all graph edges up to the normalized limit.
    pub async fn list_all_edges(&self, limit: u32) -> Result<Vec<EdgeRow>> {
        let rows = sqlx::query_as::<_, EdgeRow>(
            "SELECT id, source_id, source_type, target_id, target_type,
                    edge_type, weight, confidence, evidence_ids, metadata,
                    first_seen, last_seen
             FROM graph_edges
             ORDER BY weight DESC NULLS LAST, last_seen DESC NULLS LAST
             LIMIT $1",
        )
        .bind(normalize_graph_limit(limit))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn count_edges(&self) -> Result<i64> {
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM graph_edges")
            .fetch_one(&self.pool)
            .await?;
        Ok(row.0)
    }

    pub async fn get_neighborhood(&self, node_id: Uuid, limit: u32) -> Result<Vec<EdgeRow>> {
        let rows = sqlx::query_as::<_, EdgeRow>(
            "SELECT id, source_id, source_type, target_id, target_type,
                    edge_type, weight, confidence, evidence_ids, metadata,
                    first_seen, last_seen
             FROM graph_edges
             WHERE source_id = $1 OR target_id = $1
             ORDER BY weight DESC NULLS LAST
             LIMIT $2",
        )
        .bind(node_id)
        .bind(normalize_graph_limit(limit))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn get_path_edges(&self, from_id: Uuid, to_id: Uuid) -> Result<Vec<EdgeRow>> {
        let rows = sqlx::query_as::<_, EdgeRow>(
            "SELECT DISTINCT id, source_id, source_type, target_id, target_type,
                    edge_type, weight, confidence, evidence_ids, metadata,
                    first_seen, last_seen
             FROM graph_edges
             WHERE (source_id = $1 OR target_id = $1 OR source_id = $2 OR target_id = $2)
             ORDER BY weight DESC NULLS LAST
             LIMIT 100",
        )
        .bind(from_id)
        .bind(to_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_graph_limit;
    use crate::postgres::MAX_LIST_LIMIT;

    #[test]
    fn test_normalize_graph_limit_clamps_zero_and_large_values() {
        assert_eq!(normalize_graph_limit(0), 1);
        assert_eq!(
            normalize_graph_limit((MAX_LIST_LIMIT as u32) + 25),
            MAX_LIST_LIMIT
        );
    }

    #[test]
    fn test_normalize_graph_limit_preserves_valid_values() {
        assert_eq!(normalize_graph_limit(25), 25);
        assert_eq!(normalize_graph_limit(MAX_LIST_LIMIT as u32), MAX_LIST_LIMIT);
    }
}
