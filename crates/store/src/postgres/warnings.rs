use super::*;

fn normalize_warning_window(limit: i64, offset: i64) -> (i64, i64) {
    (clamp_limit(limit), offset.max(0))
}

impl PgStore {
    pub async fn list_warnings(
        &self,
        filters: &WarningListFilters,
        order_by: Option<WarningOrderBy>,
        desc: bool,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<WarningRow>> {
        let (limit, offset) = normalize_warning_window(limit, offset);
        let mut qb: QueryBuilder<Postgres> = QueryBuilder::new(
            r#"WITH dedup AS (
                   SELECT * FROM (
                       SELECT w.*,
                           ROW_NUMBER() OVER (
                               PARTITION BY
                                   lower(trim(w.title)),
                                   lower(trim(w.warning_type)),
                                   lower(trim(w.severity)),
                                   coalesce(lower(w.region), ''),
                                   coalesce(lower(trim(w.description)), '')
                               ORDER BY w.updated_at DESC NULLS LAST, w.created_at DESC NULLS LAST, w.ts_utc DESC, w.id DESC
                           ) AS rn
                       FROM warnings w
                   ) ranked
                   WHERE ranked.rn = 1
               )
               SELECT id, recipe_code, warning_type, title, description, severity, region,
                      source_urls, entity_ids, confidence, ts_utc, acknowledged,
                      acknowledged_by, acknowledged_at, acknowledged_note,
                      review_outcome, reviewed_by, reviewed_at, created_at, updated_at
               FROM dedup"#,
        );

        let mut has_where = false;
        if !filters.regions.is_empty() {
            qb.push(if has_where { " AND " } else { " WHERE " });
            qb.push("region = ANY(")
                .push_bind(&filters.regions)
                .push(")");
            has_where = true;
        }

        if !filters.severities.is_empty() {
            qb.push(if has_where { " AND " } else { " WHERE " });
            qb.push("severity = ANY(")
                .push_bind(&filters.severities)
                .push(")");
            has_where = true;
        }

        if !filters.warning_types.is_empty() {
            qb.push(if has_where { " AND " } else { " WHERE " });
            qb.push("warning_type = ANY(")
                .push_bind(&filters.warning_types)
                .push(")");
            has_where = true;
        }

        if let Some(ack) = filters.acknowledged {
            qb.push(if has_where { " AND " } else { " WHERE " });
            qb.push("acknowledged = ").push_bind(ack);
            has_where = true;
        }

        if let Some(date_from) = filters.date_from {
            qb.push(if has_where { " AND " } else { " WHERE " });
            qb.push("ts_utc >= ").push_bind(date_from);
            has_where = true;
        }

        if let Some(date_to) = filters.date_to {
            qb.push(if has_where { " AND " } else { " WHERE " });
            qb.push("ts_utc <= ").push_bind(date_to);
            has_where = true;
        }

        if let Some(search) = &filters.search {
            qb.push(if has_where { " AND " } else { " WHERE " });
            let pattern = ilike_pattern(search);
            qb.push("(title ILIKE ")
                .push_bind(pattern.clone())
                .push(" OR description ILIKE ")
                .push_bind(pattern)
                .push(")");
        }

        let order_by = order_by.unwrap_or(WarningOrderBy::CreatedAt);
        qb.push(" ORDER BY ");
        match order_by {
            WarningOrderBy::CreatedAt => qb.push("ts_utc"),
            WarningOrderBy::WarningType => qb.push("warning_type"),
            WarningOrderBy::Severity => qb.push(
                "CASE severity WHEN 'critical' THEN 4 WHEN 'high' THEN 3 WHEN 'medium' THEN 2 WHEN 'low' THEN 1 ELSE 0 END",
            ),
        };
        qb.push(if desc { " DESC" } else { " ASC" });
        qb.push(", id ASC");
        qb.push(" LIMIT ").push_bind(limit);
        qb.push(" OFFSET ").push_bind(offset);

        let rows = qb
            .build_query_as::<WarningRow>()
            .fetch_all(&self.pool)
            .await?;
        Ok(rows)
    }

    pub async fn count_warnings(&self, filters: &WarningListFilters) -> Result<i64> {
        let mut qb: QueryBuilder<Postgres> = QueryBuilder::new(
            r#"WITH dedup AS (
                   SELECT * FROM (
                       SELECT w.*,
                           ROW_NUMBER() OVER (
                               PARTITION BY
                                   lower(trim(w.title)),
                                   lower(trim(w.warning_type)),
                                   lower(trim(w.severity)),
                                   coalesce(lower(w.region), ''),
                                   coalesce(lower(trim(w.description)), '')
                               ORDER BY w.updated_at DESC NULLS LAST, w.created_at DESC NULLS LAST, w.ts_utc DESC, w.id DESC
                           ) AS rn
                       FROM warnings w
                   ) ranked
                   WHERE ranked.rn = 1
               )
               SELECT COUNT(*) FROM dedup"#,
        );
        let mut has_where = false;

        if !filters.regions.is_empty() {
            qb.push(if has_where { " AND " } else { " WHERE " });
            qb.push("region = ANY(")
                .push_bind(&filters.regions)
                .push(")");
            has_where = true;
        }

        if !filters.severities.is_empty() {
            qb.push(if has_where { " AND " } else { " WHERE " });
            qb.push("severity = ANY(")
                .push_bind(&filters.severities)
                .push(")");
            has_where = true;
        }

        if !filters.warning_types.is_empty() {
            qb.push(if has_where { " AND " } else { " WHERE " });
            qb.push("warning_type = ANY(")
                .push_bind(&filters.warning_types)
                .push(")");
            has_where = true;
        }

        if let Some(ack) = filters.acknowledged {
            qb.push(if has_where { " AND " } else { " WHERE " });
            qb.push("acknowledged = ").push_bind(ack);
            has_where = true;
        }

        if let Some(date_from) = filters.date_from {
            qb.push(if has_where { " AND " } else { " WHERE " });
            qb.push("ts_utc >= ").push_bind(date_from);
            has_where = true;
        }

        if let Some(date_to) = filters.date_to {
            qb.push(if has_where { " AND " } else { " WHERE " });
            qb.push("ts_utc <= ").push_bind(date_to);
            has_where = true;
        }

        if let Some(search) = &filters.search {
            qb.push(if has_where { " AND " } else { " WHERE " });
            let pattern = ilike_pattern(search);
            qb.push("(title ILIKE ")
                .push_bind(pattern.clone())
                .push(" OR description ILIKE ")
                .push_bind(pattern)
                .push(")");
        }

        let row: (i64,) = qb.build_query_as().fetch_one(&self.pool).await?;
        Ok(row.0)
    }

    pub async fn acknowledge_warning(
        &self,
        id: Uuid,
        user_id: &str,
        note: Option<&str>,
        review_outcome: Option<&str>,
    ) -> Result<AcknowledgeWarningResult> {
        let res = sqlx::query(
            r#"UPDATE warnings
               SET acknowledged = TRUE,
                   acknowledged_by = $2,
                   acknowledged_at = now(),
                   acknowledged_note = $3,
                   review_outcome = COALESCE($4, review_outcome),
                   reviewed_by = CASE WHEN $4 IS NULL THEN reviewed_by ELSE $2 END,
                   reviewed_at = CASE WHEN $4 IS NULL THEN reviewed_at ELSE now() END,
                   updated_at = now()
               WHERE id = $1 AND acknowledged = FALSE"#,
        )
        .bind(id)
        .bind(user_id)
        .bind(note)
        .bind(review_outcome)
        .execute(&self.pool)
        .await?;
        if res.rows_affected() == 1 {
            return Ok(AcknowledgeWarningResult::Acknowledged);
        }

        let ack_state = sqlx::query_scalar::<_, Option<bool>>(
            "SELECT acknowledged FROM warnings WHERE id = $1",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await?;

        match ack_state {
            None => Ok(AcknowledgeWarningResult::NotFound),
            Some(false) => Ok(AcknowledgeWarningResult::AlreadyAcknowledged),
            Some(true) => {
                if let Some(review_outcome) = review_outcome {
                    sqlx::query(
                        r#"UPDATE warnings
                           SET review_outcome = $2,
                               reviewed_by = $3,
                               reviewed_at = now(),
                               acknowledged_note = COALESCE($4, acknowledged_note),
                               updated_at = now()
                           WHERE id = $1"#,
                    )
                    .bind(id)
                    .bind(review_outcome)
                    .bind(user_id)
                    .bind(note)
                    .execute(&self.pool)
                    .await?;
                    Ok(AcknowledgeWarningResult::ReviewedExisting)
                } else {
                    Ok(AcknowledgeWarningResult::AlreadyAcknowledged)
                }
            }
        }
    }

    /// Fetch related warnings for a set of entity UUIDs.
    pub async fn get_warnings_by_entity_ids(
        &self,
        entity_ids: &[Uuid],
        limit: i64,
    ) -> Result<Vec<WarningRow>> {
        if entity_ids.is_empty() {
            return Ok(vec![]);
        }
        let (limit, _) = normalize_warning_window(limit, 0);
        let rows = sqlx::query_as::<_, WarningRow>(
            "SELECT * FROM warnings WHERE entity_ids && $1 ORDER BY created_at DESC LIMIT $2",
        )
        .bind(entity_ids)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn get_warning(&self, id: Uuid) -> Result<Option<WarningRow>> {
        let row = sqlx::query_as::<_, WarningRow>("SELECT * FROM warnings WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row)
    }

    /// Insert a new warning row. Returns the inserted row's id.
    pub async fn insert_warning(
        &self,
        warning_type: &str,
        title: &str,
        description: Option<&str>,
        severity: &str,
        region: Option<&str>,
        recipe_code: Option<&str>,
        entity_ids: Option<Vec<Uuid>>,
        source_urls: Option<Vec<String>>,
        confidence: Option<f64>,
    ) -> Result<Uuid> {
        let normalized_title = title.trim().to_string();
        let normalized_description = normalize_optional_text(description);
        let normalized_region = normalize_optional_text(region);
        let normalized_recipe_code = normalize_optional_text(recipe_code);
        let normalized_entity_ids = entity_ids.map(|mut ids| {
            ids.sort();
            ids.dedup();
            ids
        });
        let normalized_source_urls = source_urls.map(|urls| {
            let mut cleaned = normalize_url_vec(&urls);
            cleaned.sort();
            cleaned.dedup();
            cleaned
        });

        if let Some((existing_id,)) = sqlx::query_as::<_, (Uuid,)>(
            r#"SELECT id
                 FROM warnings
                 WHERE lower(trim(title)) = lower(trim($1))
                   AND lower(trim(warning_type)) = lower(trim($2))
                   AND lower(trim(severity)) = lower(trim($3))
                   AND coalesce(lower(region), '') = coalesce(lower($4), '')
                   AND coalesce(lower(trim(description)), '') = coalesce(lower(trim($5)), '')
                 ORDER BY updated_at DESC NULLS LAST, created_at DESC NULLS LAST, ts_utc DESC, id DESC
                 LIMIT 1"#,
        )
        .bind(&normalized_title)
        .bind(warning_type)
        .bind(severity)
        .bind(&normalized_region)
        .bind(&normalized_description)
        .fetch_optional(&self.pool)
        .await?
        {
            sqlx::query(
                r#"UPDATE warnings
                   SET confidence = GREATEST(COALESCE(confidence, 0), COALESCE($2, 0)),
                       source_urls = (
                           SELECT ARRAY(
                               SELECT DISTINCT u
                               FROM unnest(COALESCE(warnings.source_urls, ARRAY[]::TEXT[]) || COALESCE($3, ARRAY[]::TEXT[])) AS u
                               WHERE u IS NOT NULL AND length(trim(u)) > 0
                               ORDER BY u
                           )
                       ),
                       entity_ids = (
                           SELECT ARRAY(
                               SELECT DISTINCT e
                               FROM unnest(COALESCE(warnings.entity_ids, ARRAY[]::UUID[]) || COALESCE($4, ARRAY[]::UUID[])) AS e
                               WHERE e IS NOT NULL
                               ORDER BY e
                           )
                       ),
                       updated_at = now(),
                       ts_utc = GREATEST(ts_utc, now())
                 WHERE id = $1"#,
            )
            .bind(existing_id)
            .bind(confidence)
            .bind(&normalized_source_urls)
            .bind(&normalized_entity_ids)
            .execute(&self.pool)
            .await?;
            return Ok(existing_id);
        }

        let id = Uuid::new_v4();
        sqlx::query(
            r#"INSERT INTO warnings
               (id, warning_type, title, description, severity, region,
                recipe_code, entity_ids, source_urls, confidence,
                ts_utc, acknowledged, created_at, updated_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
                       now(), false, now(), now())"#,
        )
        .bind(id)
        .bind(warning_type)
        .bind(&normalized_title)
        .bind(&normalized_description)
        .bind(severity)
        .bind(&normalized_region)
        .bind(&normalized_recipe_code)
        .bind(&normalized_entity_ids)
        .bind(&normalized_source_urls)
        .bind(confidence)
        .execute(&self.pool)
        .await?;
        Ok(id)
    }

    /// Delete a single warning by ID. Returns true if a row was deleted.
    pub async fn delete_warning(&self, id: Uuid) -> Result<bool> {
        let result = sqlx::query("DELETE FROM warnings WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Delete multiple warnings by IDs. Returns the number of rows deleted.
    pub async fn delete_warnings(&self, ids: &[Uuid]) -> Result<u64> {
        if ids.is_empty() {
            return Ok(0);
        }
        let result = sqlx::query("DELETE FROM warnings WHERE id = ANY($1)")
            .bind(ids)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }

    /// Delete all warnings. Returns the number of rows deleted.
    pub async fn delete_all_warnings(&self) -> Result<u64> {
        let result = sqlx::query("DELETE FROM warnings")
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_warning_window;

    #[test]
    fn test_normalize_warning_window_clamps_limit_and_offset() {
        assert_eq!(normalize_warning_window(0, -10), (1, 0));
        assert_eq!(normalize_warning_window(9999, -1).0, 500);
    }

    #[test]
    fn test_normalize_warning_window_preserves_valid_values() {
        assert_eq!(normalize_warning_window(50, 20), (50, 20));
    }
}
