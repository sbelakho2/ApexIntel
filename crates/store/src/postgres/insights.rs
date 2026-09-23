use super::*;

fn normalize_insight_window(limit: i64, offset: i64) -> (i64, i64) {
    (clamp_limit(limit), offset.max(0))
}

impl PgStore {
    pub async fn insert_insight(
        &self,
        title: &str,
        summary: &str,
        insight_type: Option<&str>,
        region: Option<&str>,
        confidence: Option<f64>,
        evidence_urls: Option<Vec<String>>,
        entity_ids: Option<Vec<Uuid>>,
        tags: Option<Vec<String>>,
        metadata: Option<serde_json::Value>,
    ) -> Result<Uuid> {
        let id = Uuid::new_v4();
        let normalized_title = title.trim();
        let normalized_entity_ids: Option<Vec<Uuid>> = entity_ids.map(|mut ids| {
            ids.sort();
            ids.dedup();
            ids
        });
        let normalized_summary = summary.trim();
        let recent_story_signature = recent_story_dedup_signature(
            normalized_summary,
            insight_type,
            normalized_entity_ids.as_deref(),
        );
        let row: (Uuid,) = sqlx::query_as(
            r#"WITH existing AS (
                   SELECT i.id
                   FROM insights i
                   WHERE (
                         (i.title_hash = md5($2) OR (i.title_hash IS NULL AND i.title = $2))
                         AND COALESCE(i.entity_ids, ARRAY[]::uuid[]) = COALESCE($8, ARRAY[]::uuid[])
                         AND COALESCE(i.insight_type, '') = COALESCE($4, '')
                         AND COALESCE(i.region, '') = COALESCE($5, '')
                     )
                      OR (
                         $10 IS NOT NULL
                         AND COALESCE(i.entity_ids, ARRAY[]::uuid[]) = COALESCE($8, ARRAY[]::uuid[])
                                 AND COALESCE(i.insight_type, '') = COALESCE($4, '')
                                 AND i.created_at > NOW() - INTERVAL '21 days'
                         AND trim(regexp_replace(regexp_replace(lower(coalesce(i.summary, '')), '[^a-z0-9]+', ' ', 'g'), '\s+', ' ', 'g')) = $10
                     )
                   ORDER BY i.updated_at DESC NULLS LAST, i.created_at DESC NULLS LAST, i.id DESC
                   LIMIT 1
               ),
               updated AS (
                   UPDATE insights i
                   SET confidence = CASE
                           WHEN i.confidence IS NULL THEN $6
                           WHEN $6 IS NULL THEN i.confidence
                           ELSE GREATEST(i.confidence, $6)
                       END,
                       title_hash = md5($2),
                       summary = $3,
                       region = COALESCE($5, i.region),
                       tags = $9,
                       evidence_urls = CASE
                           WHEN $7 IS NULL THEN i.evidence_urls
                           ELSE $7
                       END,
                       updated_at = now()
                   FROM existing e
                   WHERE i.id = e.id
                   RETURNING i.id
               ),
               inserted AS (
                   INSERT INTO insights
                     (id, title, title_hash, summary, insight_type, region, confidence,
                      evidence_urls, entity_ids, tags, metadata, created_at, updated_at)
                   SELECT $1, $2, md5($2), $3, $4, $5, $6, $7, $8, $9, COALESCE($11, '{}'::jsonb), now(), now()
                   WHERE NOT EXISTS (SELECT 1 FROM updated)
                   RETURNING id
               )
               SELECT id FROM updated
               UNION ALL
               SELECT id FROM inserted
               LIMIT 1"#,
        )
        .bind(id)
        .bind(normalized_title)
        .bind(normalized_summary)
        .bind(insight_type)
        .bind(region)
        .bind(confidence)
        .bind(&evidence_urls)
        .bind(&normalized_entity_ids)
        .bind(&tags)
        .bind(&recent_story_signature)
        .bind(&metadata)
        .fetch_one(&self.pool)
        .await?;
        Ok(row.0)
    }

    pub async fn is_insight_bookmarked(&self, insight_id: Uuid, user_id: &str) -> Result<bool> {
        let row: (bool,) = sqlx::query_as(
            "SELECT EXISTS(SELECT 1 FROM insight_bookmarks WHERE insight_id = $1 AND user_id = $2)",
        )
        .bind(insight_id)
        .bind(user_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(row.0)
    }

    pub async fn list_insights(
        &self,
        filters: &InsightListFilters,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<InsightRow>> {
        let (limit, offset) = normalize_insight_window(limit, offset);

        let mut qb: QueryBuilder<Postgres> = if let Some(bookmarked_by) =
            filters.bookmarked_by.as_deref()
        {
            let mut q = QueryBuilder::new(
                "SELECT i.id, i.title, i.summary, i.insight_type, i.region, i.confidence,
                        i.evidence_urls, i.entity_ids, i.tags, i.metadata, i.created_at, i.updated_at
                 FROM insights i
                 INNER JOIN insight_bookmarks bk ON bk.insight_id = i.id AND bk.user_id = ",
            );
            q.push_bind(bookmarked_by.to_string());
            q
        } else {
            QueryBuilder::new(
                "SELECT id, title, summary, insight_type, region, confidence,
                        evidence_urls, entity_ids, tags, metadata, created_at, updated_at
                 FROM insights",
            )
        };
        let col_prefix = if filters.bookmarked_by.is_some() {
            "i."
        } else {
            ""
        };

        let mut has_where = false;
        if !filters.regions.is_empty() {
            qb.push(if has_where { " AND " } else { " WHERE " });
            qb.push(col_prefix)
                .push("region = ANY(")
                .push_bind(&filters.regions)
                .push(")");
            has_where = true;
        }

        if let Some(date_from) = filters.date_from {
            qb.push(if has_where { " AND " } else { " WHERE " });
            qb.push(col_prefix)
                .push("updated_at >= ")
                .push_bind(date_from);
            has_where = true;
        }

        if let Some(date_to) = filters.date_to {
            qb.push(if has_where { " AND " } else { " WHERE " });
            qb.push(col_prefix)
                .push("updated_at <= ")
                .push_bind(date_to);
            has_where = true;
        }

        if let Some(search) = &filters.search {
            qb.push(if has_where { " AND " } else { " WHERE " });
            let pattern = ilike_pattern(search);
            qb.push("(")
                .push(col_prefix)
                .push("title ILIKE ")
                .push_bind(pattern.clone())
                .push(" OR ")
                .push(col_prefix)
                .push("summary ILIKE ")
                .push_bind(pattern)
                .push(")");
            has_where = true;
        }

        if !filters.insight_types.is_empty() {
            qb.push(if has_where { " AND " } else { " WHERE " });
            qb.push(col_prefix)
                .push("insight_type = ANY(")
                .push_bind(&filters.insight_types)
                .push(")");
            has_where = true;
        }

        if filters.exclude_internal {
            qb.push(if has_where { " AND " } else { " WHERE " });
            append_internal_insight_filter_sql_clause(&mut qb, col_prefix);
            has_where = true;
        }

        qb.push(if has_where { " AND " } else { " WHERE " });
        append_legacy_malformed_veracity_sql_clause(&mut qb, col_prefix);

        if filters.bookmarked_by.is_some() {
            qb.push(" ORDER BY bk.created_at DESC, i.id ASC ");
        } else {
            qb.push(" ORDER BY updated_at DESC, id ASC ");
        }
        qb.push(" LIMIT ").push_bind(limit);
        qb.push(" OFFSET ").push_bind(offset);

        let rows = filter_visible_insights(
            qb.build_query_as::<InsightRow>()
                .fetch_all(&self.pool)
                .await?,
        );

        let mut deduped: Vec<InsightRow> = Vec::with_capacity(rows.len());
        let mut seen = std::collections::HashSet::<String>::new();
        for row in rows {
            let key = insight_dedup_key(
                &row.title,
                row.insight_type.as_deref(),
                row.region.as_deref(),
            );
            if seen.insert(key) {
                deduped.push(row);
            }
        }
        Ok(deduped)
    }

    pub async fn count_insights(&self, filters: &InsightListFilters) -> Result<i64> {
        let mut qb: QueryBuilder<Postgres> = if let Some(bookmarked_by) =
            filters.bookmarked_by.as_deref()
        {
            let mut q = QueryBuilder::new(
                "SELECT COUNT(DISTINCT CONCAT_WS('|', LOWER(TRIM(i.title)), LOWER(COALESCE(i.insight_type, '')), LOWER(COALESCE(i.region, '')))) FROM insights i
                 INNER JOIN insight_bookmarks bk ON bk.insight_id = i.id AND bk.user_id = ",
            );
            q.push_bind(bookmarked_by.to_string());
            q
        } else {
            QueryBuilder::new(
                "SELECT COUNT(DISTINCT CONCAT_WS('|', LOWER(TRIM(title)), LOWER(COALESCE(insight_type, '')), LOWER(COALESCE(region, '')))) FROM insights",
            )
        };
        let col_prefix = if filters.bookmarked_by.is_some() {
            "i."
        } else {
            ""
        };
        let mut has_where = false;

        if !filters.regions.is_empty() {
            qb.push(if has_where { " AND " } else { " WHERE " });
            qb.push(col_prefix)
                .push("region = ANY(")
                .push_bind(&filters.regions)
                .push(")");
            has_where = true;
        }

        if let Some(date_from) = filters.date_from {
            qb.push(if has_where { " AND " } else { " WHERE " });
            qb.push(col_prefix)
                .push("updated_at >= ")
                .push_bind(date_from);
            has_where = true;
        }

        if let Some(date_to) = filters.date_to {
            qb.push(if has_where { " AND " } else { " WHERE " });
            qb.push(col_prefix)
                .push("updated_at <= ")
                .push_bind(date_to);
            has_where = true;
        }

        if let Some(search) = &filters.search {
            qb.push(if has_where { " AND " } else { " WHERE " });
            let pattern = ilike_pattern(search);
            qb.push("(")
                .push(col_prefix)
                .push("title ILIKE ")
                .push_bind(pattern.clone())
                .push(" OR ")
                .push(col_prefix)
                .push("summary ILIKE ")
                .push_bind(pattern)
                .push(")");
            has_where = true;
        }

        if !filters.insight_types.is_empty() {
            qb.push(if has_where { " AND " } else { " WHERE " });
            qb.push(col_prefix)
                .push("insight_type = ANY(")
                .push_bind(&filters.insight_types)
                .push(")");
            has_where = true;
        }

        if filters.exclude_internal {
            qb.push(if has_where { " AND " } else { " WHERE " });
            append_internal_insight_filter_sql_clause(&mut qb, col_prefix);
            has_where = true;
        }

        qb.push(if has_where { " AND " } else { " WHERE " });
        append_legacy_malformed_veracity_sql_clause(&mut qb, col_prefix);

        let row: (i64,) = qb.build_query_as().fetch_one(&self.pool).await?;
        Ok(row.0)
    }

    /// Bookmark an insight for a user. Returns true if newly created, false if already bookmarked.
    pub async fn bookmark_insight(
        &self,
        insight_id: Uuid,
        user_id: &str,
        note: Option<&str>,
    ) -> Result<bool> {
        let result = sqlx::query(
            r#"INSERT INTO insight_bookmarks (insight_id, user_id, note)
               VALUES ($1, $2, $3)
               ON CONFLICT (insight_id, user_id) DO NOTHING"#,
        )
        .bind(insight_id)
        .bind(user_id)
        .bind(note)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn record_insight_feedback(
        &self,
        insight_id: Uuid,
        user_id: &str,
        feedback_type: &str,
        notes: Option<&str>,
    ) -> Result<bool> {
        let feedback_type = normalize_insight_feedback_type(feedback_type)
            .ok_or_else(|| anyhow::anyhow!("invalid insight feedback type: {feedback_type}"))?;
        let result = sqlx::query(
            r#"INSERT INTO insight_feedback_events (
                   insight_id,
                   entity_id,
                   recipe_code,
                   feedback_type,
                   user_id,
                   notes
               )
               SELECT
                   i.id,
                   i.entity_ids[1],
                   (
                       SELECT tag
                       FROM unnest(COALESCE(i.tags, ARRAY[]::text[])) AS tag
                       WHERE tag ~ '^[A-Z][0-9]{3,}$'
                       LIMIT 1
                   ),
                   $2,
                   $3,
                   $4
               FROM insights i
               WHERE i.id = $1
               ON CONFLICT (insight_id, user_id, feedback_type)
               DO UPDATE SET
                   notes = COALESCE(EXCLUDED.notes, insight_feedback_events.notes),
                   created_at = now()"#,
        )
        .bind(insight_id)
        .bind(feedback_type)
        .bind(user_id)
        .bind(normalize_optional_text(notes))
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn list_recent_insight_feedback_events(
        &self,
        since: DateTime<Utc>,
    ) -> Result<Vec<InsightFeedbackEventRow>> {
        Ok(sqlx::query_as::<_, InsightFeedbackEventRow>(
            r#"SELECT id, insight_id, entity_id, recipe_code, feedback_type, user_id, notes, created_at
               FROM insight_feedback_events
               WHERE created_at >= $1
               ORDER BY created_at DESC, id DESC"#,
        )
        .bind(since)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn get_insight_feedback_scores(
        &self,
        insight_ids: &[Uuid],
    ) -> Result<std::collections::HashMap<Uuid, f64>> {
        if insight_ids.is_empty() {
            return Ok(std::collections::HashMap::new());
        }

        let rows = sqlx::query(
            r#"SELECT insight_id,
                      CASE
                          WHEN COUNT(*) = 0 THEN 0.5
                          ELSE SUM(
                              CASE
                                  WHEN feedback_type IN ('bookmarked', 'actioned', 'relevant', 'true_positive') THEN 1.0
                                  WHEN feedback_type IN ('dismissed', 'irrelevant', 'false_positive') THEN 0.0
                                  ELSE 0.5
                              END
                          ) / COUNT(*)::double precision
                      END AS quality_score
               FROM insight_feedback_events
               WHERE insight_id = ANY($1)
               GROUP BY insight_id"#,
        )
        .bind(insight_ids)
        .fetch_all(&self.pool)
        .await?;

        use sqlx::Row as _;
        Ok(rows
            .into_iter()
            .filter_map(|row| {
                let insight_id: Uuid = row.try_get("insight_id").ok()?;
                let quality_score: f64 = row.try_get("quality_score").ok()?;
                Some((insight_id, quality_score.clamp(0.0, 1.0)))
            })
            .collect())
    }

    pub async fn record_insight_firing(
        &self,
        insight_id: Uuid,
        entity_id: Uuid,
        recipe_code: &str,
        insight_type: Option<&str>,
        title: &str,
        summary: &str,
        confidence: Option<f64>,
    ) -> Result<()> {
        let insight_hash = dedup_signature_from_texts(&[title, summary]).unwrap_or_else(|| {
            format!(
                "{}|{}",
                title.trim().to_ascii_lowercase(),
                summary.trim().to_ascii_lowercase()
            )
        });
        sqlx::query(
            r#"INSERT INTO insight_firings (
                   insight_id,
                   entity_id,
                   recipe_code,
                   insight_type,
                   title,
                   summary,
                   insight_hash,
                   confidence
               )
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"#,
        )
        .bind(insight_id)
        .bind(entity_id)
        .bind(recipe_code.trim())
        .bind(normalize_optional_text(insight_type))
        .bind(title.trim())
        .bind(summary.trim())
        .bind(insight_hash)
        .bind(confidence)
        .execute(&self.pool)
        .await?;

        // Increment the recipe's fire_count so the precision/recall lifecycle
        // can function. Without this, every recipe stays at fire_count=0
        // forever and the engine's precision prior is frozen at 0.5.
        let _ = sqlx::query(
            r#"UPDATE recipes
                  SET fire_count = COALESCE(fire_count, 0) + 1,
                      last_fired = NOW(),
                      updated_at = NOW()
                WHERE code = $1"#,
        )
        .bind(recipe_code.trim())
        .execute(&self.pool)
        .await;

        Ok(())
    }

    pub async fn list_recent_insight_firings(
        &self,
        since: DateTime<Utc>,
    ) -> Result<Vec<InsightFiringRow>> {
        Ok(sqlx::query_as::<_, InsightFiringRow>(
            r#"SELECT id, insight_id, entity_id, recipe_code, insight_type, title, summary, insight_hash, confidence, created_at
               FROM insight_firings
               WHERE created_at >= $1
               ORDER BY created_at DESC, id DESC"#,
        )
        .bind(since)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn get_recent_insights_for_entities(
        &self,
        entity_ids: &[Uuid],
        since: DateTime<Utc>,
        limit: i64,
    ) -> Result<Vec<InsightRow>> {
        if entity_ids.is_empty() {
            return Ok(vec![]);
        }
        let (limit, _) = normalize_insight_window(limit, 0);
        let rows = sqlx::query_as::<_, InsightRow>(
            r#"SELECT id, title, summary, insight_type, region, confidence,
                      evidence_urls, entity_ids, tags, metadata, created_at, updated_at
               FROM insights
               WHERE entity_ids && $1
                 AND created_at >= $2
               ORDER BY created_at DESC, id DESC
               LIMIT $3"#,
        )
        .bind(entity_ids)
        .bind(since)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(filter_visible_insights(rows))
    }

    /// Remove a bookmark. Returns true if a row was deleted.
    pub async fn unbookmark_insight(&self, insight_id: Uuid, user_id: &str) -> Result<bool> {
        let result =
            sqlx::query("DELETE FROM insight_bookmarks WHERE insight_id = $1 AND user_id = $2")
                .bind(insight_id)
                .bind(user_id)
                .execute(&self.pool)
                .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Return all bookmarked insight IDs for a user.
    pub async fn get_bookmarked_insight_ids(
        &self,
        user_id: &str,
        insight_ids: &[Uuid],
    ) -> Result<Vec<Uuid>> {
        let rows: Vec<(Uuid,)> = sqlx::query_as(
            "SELECT insight_id FROM insight_bookmarks
             WHERE user_id = $1 AND insight_id = ANY($2)",
        )
        .bind(user_id)
        .bind(insight_ids)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|row| row.0).collect())
    }

    /// Insert a deep competitive intelligence insight produced by the DeepInsightGenerator.
    /// Stores the insight with full narrative, risk factors, and recommendations.
    pub async fn insert_deep_insight(
        &self,
        insight_id: Uuid,
        title: &str,
        narrative: &str,
        insight_type: &str,
        company_id: &Uuid,
        company_name: &str,
        risk_factors: &[String],
        recommendations: &[String],
        confidence: f64,
        _created_at: DateTime<Utc>,
    ) -> Result<()> {
        let metadata = serde_json::json!({
            "company_name": company_name,
            "risk_factors": risk_factors,
            "recommendations": recommendations,
            "generated_by": "DeepInsightGenerator",
            "generation_pipeline": "insight_generation_worker",
        });
        let entity_ids: Vec<Uuid> = vec![*company_id];
        let tags: Vec<String> = risk_factors
            .iter()
            .take(5)
            .map(|rf| format!("risk:{}", rf.to_lowercase().replace(' ', "_")))
            .collect();
        sqlx::query(
            r#"INSERT INTO insights
               (id, title, title_hash, summary, insight_type, confidence,
                evidence_urls, entity_ids, tags, metadata, created_at, updated_at)
               VALUES ($1, $2, md5($2), $3, $4, $5, '{}', $6, $7, $8, $9, $9)
               ON CONFLICT (id) DO UPDATE SET
                 title = EXCLUDED.title,
                 title_hash = md5(EXCLUDED.title),
                 summary = EXCLUDED.summary,
                 confidence = GREATEST(insights.confidence, EXCLUDED.confidence),
                 tags = insights.tags || EXCLUDED.tags,
                 metadata = insights.metadata || EXCLUDED.metadata,
                 updated_at = now()"#,
        )
        .bind(insight_id)
        .bind(title.trim())
        .bind(narrative.trim())
        .bind(insight_type)
        .bind(confidence.clamp(0.0, 1.0))
        .bind(&entity_ids)
        .bind(&tags)
        .bind(&metadata)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Link an insight to an entity (company or person) via a graph edge.
    /// Creates an "insight_relates_to" edge from the insight to the entity.
    pub async fn link_insight_to_entity(
        &self,
        insight_id: Uuid,
        entity_id: &Uuid,
        entity_type: &str,
    ) -> Result<()> {
        let edge = GraphEdge {
            id: Uuid::new_v4(),
            source_id: insight_id,
            source_type: "insight".to_string(),
            target_id: *entity_id,
            target_type: entity_type.to_string(),
            edge_type: EdgeType::CompanyCompany,
            weight: 0.85,
            confidence: 0.85,
            evidence_ids: vec![],
            metadata: serde_json::json!({}),
            first_seen: Utc::now(),
            last_seen: Utc::now(),
        };
        self.upsert_edge(&edge).await
    }

    pub async fn get_insight(&self, id: Uuid) -> Result<Option<InsightRow>> {
        let row = sqlx::query_as::<_, InsightRow>(
            "SELECT id, title, summary, insight_type, region, confidence,
                    evidence_urls, entity_ids, tags, metadata, created_at, updated_at
             FROM insights WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.and_then(|row| filter_visible_insights(vec![row]).into_iter().next()))
    }

    /// Fetch related insights for a set of entity UUIDs, excluding the current insight.
    pub async fn get_related_insights(
        &self,
        entity_ids: &[Uuid],
        exclude_id: Uuid,
        limit: i64,
    ) -> Result<Vec<InsightRow>> {
        if entity_ids.is_empty() {
            return Ok(vec![]);
        }
        let (limit, _) = normalize_insight_window(limit, 0);
        let rows = sqlx::query_as::<_, InsightRow>(
            "SELECT id, title, summary, insight_type, region, confidence,
                    evidence_urls, entity_ids, tags, metadata, created_at, updated_at
             FROM insights WHERE entity_ids && $1 AND id != $2 ORDER BY created_at DESC LIMIT $3",
        )
        .bind(entity_ids)
        .bind(exclude_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(filter_visible_insights(rows))
    }

    /// Fetch insights related to a warning by entity overlap.
    pub async fn get_insights_by_entity_ids(
        &self,
        entity_ids: &[Uuid],
        limit: i64,
    ) -> Result<Vec<InsightRow>> {
        if entity_ids.is_empty() {
            return Ok(vec![]);
        }
        let (limit, _) = normalize_insight_window(limit, 0);
        let rows = sqlx::query_as::<_, InsightRow>(
            "SELECT id, title, summary, insight_type, region, confidence,
                    evidence_urls, entity_ids, tags, metadata, created_at, updated_at
             FROM insights WHERE entity_ids && $1 ORDER BY created_at DESC LIMIT $2",
        )
        .bind(entity_ids)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(filter_visible_insights(rows))
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_insight_window;

    #[test]
    fn test_normalize_insight_window_clamps_limit_and_offset() {
        assert_eq!(normalize_insight_window(0, -10), (1, 0));
        assert_eq!(normalize_insight_window(9999, -1), (500, 0));
    }

    #[test]
    fn test_normalize_insight_window_preserves_valid_values() {
        assert_eq!(normalize_insight_window(25, 15), (25, 15));
    }
}
