use super::*;

fn normalize_logistics_window(limit: i64, offset: i64) -> (i64, i64) {
    (clamp_limit(limit), offset.max(0))
}

impl PgStore {
    pub async fn insert_logistics_node(&self, n: &LogisticsNode) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO logistics_nodes
               (id, name, node_type, country_code, lat, lon, metadata)
               VALUES ($1,$2,$3,$4,$5,$6,$7)
               ON CONFLICT (id) DO NOTHING"#,
        )
        .bind(n.id)
        .bind(&n.name)
        .bind(&n.node_type)
        .bind(&n.country_code)
        .bind(n.lat)
        .bind(n.lon)
        .bind(&n.metadata)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_logistics_nodes(
        &self,
        country_code: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<LogisticsNodeRow>> {
        let (limit, offset) = normalize_logistics_window(limit, offset);
        match country_code {
            Some(cc) => {
                Ok(sqlx::query_as::<_, LogisticsNodeRow>("SELECT * FROM logistics_nodes WHERE country_code = $3 ORDER BY name ASC LIMIT $1 OFFSET $2")
                    .bind(limit).bind(offset).bind(cc)
                    .fetch_all(&self.pool).await?)
            }
            None => {
                Ok(sqlx::query_as::<_, LogisticsNodeRow>("SELECT * FROM logistics_nodes ORDER BY name ASC LIMIT $1 OFFSET $2")
                    .bind(limit).bind(offset)
                    .fetch_all(&self.pool).await?)
            }
        }
    }

    pub async fn count_logistics_nodes(&self, country_code: Option<&str>) -> Result<i64> {
        let count: i64 = match country_code {
            Some(cc) => {
                sqlx::query_scalar("SELECT COUNT(*) FROM logistics_nodes WHERE country_code = $1")
                    .bind(cc)
                    .fetch_one(&self.pool)
                    .await?
            }
            None => {
                sqlx::query_scalar("SELECT COUNT(*) FROM logistics_nodes")
                    .fetch_one(&self.pool)
                    .await?
            }
        };
        Ok(count)
    }

    pub async fn list_regulations(
        &self,
        jurisdiction: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<RegulationRow>> {
        let (limit, offset) = normalize_logistics_window(limit, offset);
        match jurisdiction {
            Some(j) => {
                Ok(sqlx::query_as::<_, RegulationRow>("SELECT * FROM regulations WHERE jurisdiction = $3 ORDER BY effective_date DESC NULLS LAST LIMIT $1 OFFSET $2")
                    .bind(limit).bind(offset).bind(j)
                    .fetch_all(&self.pool).await?)
            }
            None => {
                Ok(sqlx::query_as::<_, RegulationRow>("SELECT * FROM regulations ORDER BY effective_date DESC NULLS LAST LIMIT $1 OFFSET $2")
                    .bind(limit).bind(offset)
                    .fetch_all(&self.pool).await?)
            }
        }
    }

    pub async fn count_regulations(&self, jurisdiction: Option<&str>) -> Result<i64> {
        let count: i64 = match jurisdiction {
            Some(j) => {
                sqlx::query_scalar("SELECT COUNT(*) FROM regulations WHERE jurisdiction = $1")
                    .bind(j)
                    .fetch_one(&self.pool)
                    .await?
            }
            None => {
                sqlx::query_scalar("SELECT COUNT(*) FROM regulations")
                    .fetch_one(&self.pool)
                    .await?
            }
        };
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_logistics_window;

    #[test]
    fn test_normalize_logistics_window_clamps_limit_and_offset() {
        assert_eq!(normalize_logistics_window(0, -1), (1, 0));
        assert_eq!(normalize_logistics_window(9999, 4), (500, 4));
    }

    #[test]
    fn test_normalize_logistics_window_preserves_valid_values() {
        assert_eq!(normalize_logistics_window(50, 12), (50, 12));
    }
}
