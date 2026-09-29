use super::*;

/// Batched engagement numbers for a set of competitors.
#[derive(Debug, Clone, Default)]
pub struct CompetitorEngagement {
    /// Real warning counts per competitor (union over `warnings.entity_ids`).
    pub warning_counts: std::collections::HashMap<Uuid, i64>,
    /// Real insight counts per competitor (`insights.entity_id`).
    pub insight_counts: std::collections::HashMap<Uuid, i64>,
    /// Latest recorded change per competitor.
    pub latest_changes: std::collections::HashMap<Uuid, CompetitorChange>,
}

fn normalize_competitor_window(limit: i64, offset: i64) -> (i64, i64) {
    (clamp_limit(limit), offset.max(0))
}

fn normalize_competitor_page(page: i64, per_page: i64) -> (i64, i64) {
    let limit = clamp_limit(per_page);
    let page = page.max(1);
    let offset = (page - 1).saturating_mul(limit);
    (limit, offset)
}

impl PgStore {
    pub async fn list_competitors(&self, limit: i64, offset: i64) -> Result<Vec<CompanyRow>> {
        let (limit, offset) = normalize_competitor_window(limit, offset);
        let rows = sqlx::query_as::<_, CompanyRow>(
            "SELECT * FROM companies WHERE (metadata->>'is_competitor')::boolean = true ORDER BY name ASC LIMIT $1 OFFSET $2"
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn count_competitors(&self) -> Result<i64> {
        let (count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM companies WHERE (metadata->>'is_competitor')::boolean = true",
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(count)
    }

    pub async fn get_competitor_changes(
        &self,
        company_id: Uuid,
        limit: i64,
    ) -> Result<Vec<CompanyChangeRow>> {
        let rows = sqlx::query_as::<_, CompanyChangeRow>(
            "SELECT * FROM company_changes WHERE company_id = $1 ORDER BY detected_at DESC LIMIT $2"
        )
        .bind(company_id)
        .bind(clamp_limit(limit))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn get_all_competitor_changes_paged(
        &self,
        page: i64,
        per_page: i64,
    ) -> Result<(Vec<CompetitorChange>, i64)> {
        let (limit, offset) = normalize_competitor_page(page, per_page);

        let (total,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM competitor_changes")
            .fetch_one(&self.pool)
            .await?;

        let rows: Vec<(Uuid, Uuid, String, String, String, String, DateTime<Utc>, Option<String>, f64)> = sqlx::query_as(
            r#"SELECT cc.id, cc.competitor_id, c.name, cc.change_type, cc.title, cc.description, cc.detected_at, cc.source_url, cc.impact_score
               FROM competitor_changes cc
               JOIN companies c ON c.id = cc.competitor_id
               ORDER BY cc.detected_at DESC
               LIMIT $1 OFFSET $2"#
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;

        let changes = rows
            .into_iter()
            .map(
                |(
                    id,
                    competitor_id,
                    competitor_name,
                    change_type,
                    title,
                    description,
                    detected_at,
                    source_url,
                    impact_score,
                )| CompetitorChange {
                    id,
                    competitor_id,
                    competitor_name,
                    change_type,
                    title,
                    description,
                    detected_at: detected_at.to_rfc3339(),
                    source_url,
                    impact_score,
                },
            )
            .collect();

        Ok((changes, total))
    }

    /// Batched per-competitor engagement signals for the dashboard: real
    /// warning counts (through `warnings.entity_ids`), real insight counts
    /// (`insights.entity_id`), and each competitor's latest recorded change.
    ///
    /// Three queries total — never N+1. A failed call is an unavailable state;
    /// callers must not render it as zero engagement.
    pub async fn get_competitor_engagement(
        &self,
        competitor_ids: &[Uuid],
    ) -> Result<CompetitorEngagement> {
        if competitor_ids.is_empty() {
            return Ok(CompetitorEngagement::default());
        }

        let warning_rows: Vec<(Uuid, i64)> = sqlx::query_as(
            "SELECT entity_id, COUNT(*)::BIGINT \
             FROM warnings w, \
                  unnest(COALESCE(w.entity_ids, ARRAY[]::UUID[])) AS entity_id \
             WHERE w.deleted_at IS NULL AND entity_id = ANY($1) \
             GROUP BY entity_id",
        )
        .bind(competitor_ids)
        .fetch_all(&self.pool)
        .await?;

        let insight_rows: Vec<(Uuid, i64)> = sqlx::query_as(
            "SELECT entity_id, COUNT(*)::BIGINT FROM insights \
             WHERE entity_id = ANY($1) GROUP BY entity_id",
        )
        .bind(competitor_ids)
        .fetch_all(&self.pool)
        .await?;

        let change_rows: Vec<(
            Uuid,
            Uuid,
            String,
            String,
            String,
            String,
            DateTime<Utc>,
            Option<String>,
            f64,
        )> = sqlx::query_as(
            "SELECT DISTINCT ON (cc.competitor_id) \
                    cc.id, cc.competitor_id, c.name, cc.change_type, cc.title, \
                    cc.description, cc.detected_at, cc.source_url, cc.impact_score \
             FROM competitor_changes cc \
             JOIN companies c ON c.id = cc.competitor_id \
             WHERE cc.competitor_id = ANY($1) \
             ORDER BY cc.competitor_id, cc.detected_at DESC",
        )
        .bind(competitor_ids)
        .fetch_all(&self.pool)
        .await?;

        let mut engagement = CompetitorEngagement {
            warning_counts: warning_rows.into_iter().collect(),
            insight_counts: insight_rows.into_iter().collect(),
            latest_changes: std::collections::HashMap::new(),
        };
        for (
            id,
            competitor_id,
            competitor_name,
            change_type,
            title,
            description,
            detected_at,
            source_url,
            impact_score,
        ) in change_rows
        {
            engagement.latest_changes.insert(
                competitor_id,
                CompetitorChange {
                    id,
                    competitor_id,
                    competitor_name,
                    change_type,
                    title,
                    description,
                    detected_at: detected_at.to_rfc3339(),
                    source_url,
                    impact_score,
                },
            );
        }
        Ok(engagement)
    }

    pub async fn get_competitor_changes_by_id(
        &self,
        competitor_id: Uuid,
        page: i64,
        per_page: i64,
    ) -> Result<(Vec<CompetitorChange>, i64)> {
        let (limit, offset) = normalize_competitor_page(page, per_page);

        let (total,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM competitor_changes WHERE competitor_id = $1")
                .bind(competitor_id)
                .fetch_one(&self.pool)
                .await?;

        let rows: Vec<(Uuid, Uuid, String, String, String, String, DateTime<Utc>, Option<String>, f64)> = sqlx::query_as(
            r#"SELECT cc.id, cc.competitor_id, c.name, cc.change_type, cc.title, cc.description, cc.detected_at, cc.source_url, cc.impact_score
               FROM competitor_changes cc
               JOIN companies c ON c.id = cc.competitor_id
               WHERE cc.competitor_id = $1
               ORDER BY cc.detected_at DESC
               LIMIT $2 OFFSET $3"#
        )
        .bind(competitor_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;

        let changes = rows
            .into_iter()
            .map(
                |(
                    id,
                    competitor_id,
                    competitor_name,
                    change_type,
                    title,
                    description,
                    detected_at,
                    source_url,
                    impact_score,
                )| CompetitorChange {
                    id,
                    competitor_id,
                    competitor_name,
                    change_type,
                    title,
                    description,
                    detected_at: detected_at.to_rfc3339(),
                    source_url,
                    impact_score,
                },
            )
            .collect();

        Ok((changes, total))
    }
}

#[cfg(test)]
mod tests {
    use super::{normalize_competitor_page, normalize_competitor_window};

    #[test]
    fn test_normalize_competitor_window_clamps_limit_and_offset() {
        assert_eq!(normalize_competitor_window(0, -1), (1, 0));
        assert_eq!(normalize_competitor_window(900, 3), (500, 3));
    }

    #[test]
    fn test_normalize_competitor_page_clamps_page_and_size() {
        assert_eq!(normalize_competitor_page(0, 0), (1, 0));
        assert_eq!(normalize_competitor_page(3, 50), (50, 100));
    }
}
