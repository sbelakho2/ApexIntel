use super::*;

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
            .await
            .unwrap_or((0,));

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
                .await
                .unwrap_or((0,));

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

    pub async fn get_competitors_enhanced(
        &self,
        page: i64,
        per_page: i64,
    ) -> Result<(Vec<CompetitorItem>, i64)> {
        let (limit, offset) = normalize_competitor_page(page, per_page);

        let (total,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM companies WHERE is_competitor = true")
                .fetch_one(&self.pool)
                .await
                .unwrap_or((0,));

        let rows: Vec<(
            Uuid,
            String,
            Option<String>,
            Option<String>,
            Option<f64>,
            Option<Vec<String>>,
        )> = sqlx::query_as(
            r#"SELECT id, name, region, company_type, risk_score, industry_tags
               FROM companies
               WHERE is_competitor = true
               ORDER BY risk_score DESC NULLS LAST, name
               LIMIT $1 OFFSET $2"#,
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;

        let mut items = Vec::with_capacity(rows.len());
        for (id, name, region, company_type, risk_score, industry_tags) in rows {
            let change_info: Option<(DateTime<Utc>, i64)> = sqlx::query_as(
                r#"SELECT MAX(detected_at), COUNT(*)
                   FROM competitor_changes
                   WHERE competitor_id = $1 AND detected_at > NOW() - INTERVAL '30 days'"#,
            )
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .ok()
            .flatten();

            let (last_change_at, change_count_30d) = change_info
                .map(|(ts, cnt)| (Some(ts.to_rfc3339()), cnt))
                .unwrap_or((None, 0));

            items.push(CompetitorItem {
                id,
                name,
                region: region.unwrap_or_else(|| "Unknown".to_string()),
                entity_type: company_type.unwrap_or_else(|| "EMS".to_string()),
                threat_score: risk_score,
                // Overlap fields are not yet populated from real data.
                // Return None/0.0 to signal "unknown" rather than fabricating values.
                capability_overlap: 0.0,
                market_overlap: 0.0,
                last_change_at,
                change_count_30d,
                capabilities: industry_tags.unwrap_or_default(),
                primary_markets: vec![],
            });
        }

        Ok((items, total))
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
