use super::*;

fn saturating_count_to_u64(value: i64) -> u64 {
    value.max(0) as u64
}

impl PgStore {
    pub async fn get_obs_type_counts_per_entity(
        &self,
        since: DateTime<Utc>,
    ) -> Result<Vec<(Uuid, String, i64)>> {
        let rows = sqlx::query(
            r#"SELECT entity_id, observation_type, COUNT(*)::BIGINT AS cnt
               FROM observations
               WHERE entity_id IS NOT NULL AND ts_utc >= $1
               GROUP BY entity_id, observation_type
               ORDER BY entity_id, cnt DESC"#,
        )
        .bind(since)
        .fetch_all(&self.pool)
        .await?;

        use sqlx::Row as _;
        let result = rows
            .into_iter()
            .filter_map(|row| {
                let entity_id: Uuid = row.try_get("entity_id").ok()?;
                let obs_type: String = row.try_get("observation_type").ok()?;
                let cnt: i64 = row.try_get("cnt").ok()?;
                Some((entity_id, obs_type, cnt))
            })
            .collect();

        Ok(result)
    }

    pub async fn get_warning_type_counts_per_entity(
        &self,
        since: DateTime<Utc>,
    ) -> Result<Vec<(Uuid, String, i64)>> {
        let rows = sqlx::query(
            r#"SELECT unnest(entity_ids) AS eid, warning_type, COUNT(*)::BIGINT AS cnt
               FROM warnings
               WHERE entity_ids IS NOT NULL
                 AND array_length(entity_ids, 1) > 0
                 AND created_at >= $1
               GROUP BY eid, warning_type
               ORDER BY eid, cnt DESC"#,
        )
        .bind(since)
        .fetch_all(&self.pool)
        .await?;

        use sqlx::Row as _;
        let result = rows
            .into_iter()
            .filter_map(|row| {
                let eid: Uuid = row.try_get("eid").ok()?;
                let wt: String = row.try_get("warning_type").ok()?;
                let cnt: i64 = row.try_get("cnt").ok()?;
                Some((eid, wt, cnt))
            })
            .collect();

        Ok(result)
    }

    pub async fn get_competitor_event_features(
        &self,
        since: DateTime<Utc>,
    ) -> Result<Vec<(Uuid, String, String, i64)>> {
        let rows = sqlx::query(
            r#"SELECT entity_id,
                      COALESCE(value->>'signal_type', '') AS sig,
                      COALESCE(value->>'keyword', '') AS kw,
                      COUNT(*)::BIGINT AS cnt
               FROM observations
               WHERE observation_type = 'CompetitorEvent'
                 AND entity_id IS NOT NULL
                 AND ts_utc >= $1
               GROUP BY entity_id, sig, kw
               ORDER BY entity_id, cnt DESC"#,
        )
        .bind(since)
        .fetch_all(&self.pool)
        .await?;

        use sqlx::Row as _;
        let result = rows
            .into_iter()
            .filter_map(|row| {
                let eid: Uuid = row.try_get("entity_id").ok()?;
                let sig: String = row.try_get("sig").ok()?;
                let kw: String = row.try_get("kw").ok()?;
                let cnt: i64 = row.try_get("cnt").ok()?;
                Some((eid, sig, kw, cnt))
            })
            .collect();

        Ok(result)
    }

    pub async fn get_webchange_jsonb_features(
        &self,
        since: DateTime<Utc>,
    ) -> Result<Vec<(Uuid, String, String, i64)>> {
        let rows = sqlx::query(
            r#"SELECT entity_id,
                      COALESCE(value->>'source_id', '') AS src,
                      COALESCE(value->>'signal_type', '') AS sig,
                      COUNT(*)::BIGINT AS cnt
               FROM observations
               WHERE observation_type = 'WebChange'
                 AND entity_id IS NOT NULL
                 AND ts_utc >= $1
               GROUP BY entity_id, src, sig
               ORDER BY entity_id, cnt DESC"#,
        )
        .bind(since)
        .fetch_all(&self.pool)
        .await?;

        use sqlx::Row as _;
        let result = rows
            .into_iter()
            .filter_map(|row| {
                let eid: Uuid = row.try_get("entity_id").ok()?;
                let src: String = row.try_get("src").ok()?;
                let sig: String = row.try_get("sig").ok()?;
                let cnt: i64 = row.try_get("cnt").ok()?;
                Some((eid, src, sig, cnt))
            })
            .collect();

        Ok(result)
    }

    pub async fn get_webchange_keyword_features(
        &self,
        since: DateTime<Utc>,
    ) -> Result<Vec<(Uuid, String, i64)>> {
        let rows = sqlx::query(
            r#"SELECT entity_id,
                      COALESCE(value->>'keyword', '') AS kw,
                      COUNT(*)::BIGINT AS cnt
               FROM observations
               WHERE observation_type = 'WebChange'
                 AND entity_id IS NOT NULL
                 AND value->>'keyword' IS NOT NULL
                 AND ts_utc >= $1
               GROUP BY entity_id, kw
               ORDER BY entity_id, cnt DESC"#,
        )
        .bind(since)
        .fetch_all(&self.pool)
        .await?;

        use sqlx::Row as _;
        Ok(rows
            .into_iter()
            .filter_map(|row| {
                let eid: Uuid = row.try_get("entity_id").ok()?;
                let kw: String = row.try_get("kw").ok()?;
                let cnt: i64 = row.try_get("cnt").ok()?;
                Some((eid, kw, cnt))
            })
            .collect())
    }

    pub async fn get_person_features_per_company(
        &self,
    ) -> Result<Vec<(Uuid, String, f64, f64, f64, i64)>> {
        let rows = sqlx::query(
            r#"SELECT primary_org_id AS company_id,
                      COALESCE(role_family, 'Unknown') AS rf,
                      COALESCE(AVG(influence_score), 0) AS avg_inf,
                      COALESCE(AVG(pain_index), 0) AS avg_pain,
                      COALESCE(AVG(change_risk), 0) AS avg_cr,
                      COUNT(*)::BIGINT AS cnt
               FROM persons
               WHERE primary_org_id IS NOT NULL
               GROUP BY primary_org_id, role_family
               ORDER BY primary_org_id"#,
        )
        .fetch_all(&self.pool)
        .await?;

        use sqlx::Row as _;
        Ok(rows
            .into_iter()
            .filter_map(|row| {
                let cid: Uuid = row.try_get("company_id").ok()?;
                let rf: String = row.try_get("rf").ok()?;
                let inf: f64 = row.try_get("avg_inf").ok()?;
                let pain: f64 = row.try_get("avg_pain").ok()?;
                let cr: f64 = row.try_get("avg_cr").ok()?;
                let cnt: i64 = row.try_get("cnt").ok()?;
                Some((cid, rf, inf, pain, cr, cnt))
            })
            .collect())
    }

    pub async fn get_certification_features_per_company(&self) -> Result<Vec<(Uuid, String, i64)>> {
        let rows = sqlx::query(
            r#"SELECT company_id,
                      standard,
                      COUNT(*)::BIGINT AS cnt
               FROM certifications
               WHERE company_id IS NOT NULL
               GROUP BY company_id, standard
               ORDER BY company_id"#,
        )
        .fetch_all(&self.pool)
        .await?;

        use sqlx::Row as _;
        Ok(rows
            .into_iter()
            .filter_map(|row| {
                let cid: Uuid = row.try_get("company_id").ok()?;
                let standard: String = row.try_get("standard").ok()?;
                let cnt: i64 = row.try_get("cnt").ok()?;
                Some((cid, standard, cnt))
            })
            .collect())
    }

    pub async fn get_capability_features_per_company(&self) -> Result<Vec<(Uuid, String, i64)>> {
        let rows = sqlx::query(
            r#"SELECT company_id,
                      capability,
                      COUNT(*)::BIGINT AS cnt
               FROM capabilities
               WHERE company_id IS NOT NULL
               GROUP BY company_id, capability
               ORDER BY company_id"#,
        )
        .fetch_all(&self.pool)
        .await?;

        use sqlx::Row as _;
        Ok(rows
            .into_iter()
            .filter_map(|row| {
                let cid: Uuid = row.try_get("company_id").ok()?;
                let capability: String = row.try_get("capability").ok()?;
                let cnt: i64 = row.try_get("cnt").ok()?;
                Some((cid, capability, cnt))
            })
            .collect())
    }

    pub async fn get_site_features_per_company(&self) -> Result<Vec<(Uuid, String, String, i64)>> {
        let rows = sqlx::query(
            r#"SELECT company_id,
                      COALESCE(country_code, '') AS cc,
                      COALESCE(site_type, '') AS st,
                      COUNT(*)::BIGINT AS cnt
               FROM sites
               WHERE company_id IS NOT NULL
               GROUP BY company_id, country_code, site_type
               ORDER BY company_id"#,
        )
        .fetch_all(&self.pool)
        .await?;

        use sqlx::Row as _;
        Ok(rows
            .into_iter()
            .filter_map(|row| {
                let cid: Uuid = row.try_get("company_id").ok()?;
                let cc: String = row.try_get("cc").ok()?;
                let st: String = row.try_get("st").ok()?;
                let cnt: i64 = row.try_get("cnt").ok()?;
                Some((cid, cc, st, cnt))
            })
            .collect())
    }

    pub async fn get_crawl_stats(&self, since: DateTime<Utc>) -> Result<CrawlStats> {
        let row = sqlx::query_as::<_, CrawlStatsRow>(
            r#"SELECT 
                COALESCE(COUNT(*), 0) as sources_attempted,
                COALESCE(SUM(CASE WHEN status = 'success' THEN 1 ELSE 0 END), 0) as sources_succeeded,
                COALESCE(SUM(CASE WHEN status = 'failed' THEN 1 ELSE 0 END), 0) as sources_failed,
                COALESCE(SUM(new_observations), 0) as new_observations,
                COALESCE(SUM(changed_pages), 0) as changed_pages,
                COALESCE(SUM(bytes_fetched), 0) as bytes_fetched
               FROM crawl_logs
               WHERE created_at >= $1"#,
        )
        .bind(since)
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(r) => Ok(CrawlStats {
                sources_attempted: saturating_count_to_u64(r.sources_attempted.unwrap_or(0)),
                sources_succeeded: saturating_count_to_u64(r.sources_succeeded.unwrap_or(0)),
                sources_failed: saturating_count_to_u64(r.sources_failed.unwrap_or(0)),
                new_observations: saturating_count_to_u64(r.new_observations.unwrap_or(0)),
                changed_pages: saturating_count_to_u64(r.changed_pages.unwrap_or(0)),
                bytes_fetched: saturating_count_to_u64(r.bytes_fetched.unwrap_or(0)),
                errors: vec![],
            }),
            None => Ok(CrawlStats::default()),
        }
    }

    pub async fn get_mining_stats(&self, since: DateTime<Utc>) -> Result<MiningStats> {
        let candidates_found: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM pattern_candidates WHERE created_at >= $1")
                .bind(since)
                .fetch_one(&self.pool)
                .await
                .unwrap_or(0);

        let candidates_passed: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM pattern_candidates WHERE created_at >= $1 AND passed_gates = true",
        )
        .bind(since)
        .fetch_one(&self.pool)
        .await
        .unwrap_or(0);

        let recipes_staged: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM recipes WHERE created_at >= $1 AND status = 'staging'",
        )
        .bind(since)
        .fetch_one(&self.pool)
        .await
        .unwrap_or(0);

        let hypotheses_generated: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM recipes WHERE created_at >= $1")
                .bind(since)
                .fetch_one(&self.pool)
                .await
                .unwrap_or(0)
                .max(recipes_staged);

        Ok(MiningStats {
            candidates_found: saturating_count_to_u64(candidates_found),
            candidates_passed_gates: saturating_count_to_u64(candidates_passed),
            hypotheses_generated: saturating_count_to_u64(hypotheses_generated),
            recipes_staged: saturating_count_to_u64(recipes_staged),
            errors: vec![],
        })
    }

    pub async fn get_poi_stats(&self, since: DateTime<Utc>) -> Result<PoiStats> {
        let profiles_scanned: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM persons WHERE updated_at >= $1")
                .bind(since)
                .fetch_one(&self.pool)
                .await
                .unwrap_or(0);

        let profiles_updated: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM persons WHERE updated_at >= $1 AND updated_at != created_at",
        )
        .bind(since)
        .fetch_one(&self.pool)
        .await
        .unwrap_or(0);

        let new_pois: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM persons WHERE created_at >= $1")
                .bind(since)
                .fetch_one(&self.pool)
                .await
                .unwrap_or(0);

        Ok(PoiStats {
            profiles_scanned: saturating_count_to_u64(profiles_scanned),
            profiles_updated: saturating_count_to_u64(profiles_updated),
            new_pois_discovered: saturating_count_to_u64(new_pois),
            role_changes_detected: saturating_count_to_u64(
                self.count_role_changes_since(since).await.unwrap_or(0),
            ),
            errors: vec![],
        })
    }

    pub async fn get_drift_stats(&self) -> Result<DriftStats> {
        let features_checked: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM feature_rows")
            .fetch_one(&self.pool)
            .await
            .unwrap_or(0);

        let features_drifted: i64 = sqlx::query_scalar(
            r#"SELECT COUNT(*)
               FROM feature_rows
               WHERE (data ? 'drift_score')
                 AND (data->>'drift_score') ~ '^-?[0-9]+(\.[0-9]+)?$'
                 AND (data->>'drift_score')::double precision >= 0.20"#,
        )
        .fetch_one(&self.pool)
        .await
        .unwrap_or(0);

        let drift_scores = sqlx::query_as::<_, (String, f64)>(
            r#"SELECT
                    entity_type || ':' || entity_id AS feature_key,
                    (data->>'drift_score')::double precision AS drift_score
               FROM feature_rows
               WHERE (data ? 'drift_score')
                 AND (data->>'drift_score') ~ '^-?[0-9]+(\.[0-9]+)?$'
               ORDER BY drift_score DESC
               LIMIT 20"#,
        )
        .fetch_all(&self.pool)
        .await
        .unwrap_or_default();

        Ok(DriftStats {
            features_checked: saturating_count_to_u64(features_checked),
            features_drifted: saturating_count_to_u64(features_drifted),
            alerts_raised: saturating_count_to_u64(features_drifted),
            drift_scores,
            errors: vec![],
        })
    }

    pub async fn get_weekly_summary_stats(
        &self,
        since: DateTime<Utc>,
    ) -> Result<WeeklySummaryStats> {
        let companies_monitored: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM companies")
            .fetch_one(&self.pool)
            .await
            .unwrap_or(0);

        let persons_tracked: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM persons")
            .fetch_one(&self.pool)
            .await
            .unwrap_or(0);

        let warnings_generated: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM warnings WHERE created_at >= $1")
                .bind(since)
                .fetch_one(&self.pool)
                .await
                .unwrap_or(0);

        let insights_produced: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM insights WHERE created_at >= $1")
                .bind(since)
                .fetch_one(&self.pool)
                .await
                .unwrap_or(0);

        let recipes_in_production: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM recipes WHERE status IN ('active', 'production')",
        )
        .fetch_one(&self.pool)
        .await
        .unwrap_or(0);

        let top_regions_rows = sqlx::query_as::<_, (String,)>(
            r#"SELECT region
               FROM warnings
               WHERE created_at >= $1
                 AND region IS NOT NULL
                 AND region <> ''
               GROUP BY region
               ORDER BY COUNT(*) DESC
               LIMIT 5"#,
        )
        .bind(since)
        .fetch_all(&self.pool)
        .await
        .unwrap_or_default();
        let top_regions = top_regions_rows
            .into_iter()
            .map(|(region,)| region)
            .collect();

        let notable_events_rows = sqlx::query_as::<_, (String,)>(
            r#"SELECT title
               FROM warnings
               WHERE created_at >= $1
               ORDER BY confidence DESC NULLS LAST, created_at DESC
               LIMIT 5"#,
        )
        .bind(since)
        .fetch_all(&self.pool)
        .await
        .unwrap_or_default();
        let notable_events = notable_events_rows
            .into_iter()
            .map(|(title,)| title)
            .collect();

        Ok(WeeklySummaryStats {
            companies_monitored: saturating_count_to_u64(companies_monitored),
            persons_tracked: saturating_count_to_u64(persons_tracked),
            warnings_generated: saturating_count_to_u64(warnings_generated),
            insights_produced: saturating_count_to_u64(insights_produced),
            recipes_in_production: saturating_count_to_u64(recipes_in_production),
            top_regions,
            notable_events,
        })
    }

    pub async fn get_dashboard_stats(&self) -> Result<DashboardStats> {
        let total_companies: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM companies")
            .fetch_one(&self.pool)
            .await?;
        let total_persons: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM persons")
            .fetch_one(&self.pool)
            .await?;
        let total_warnings: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM warnings")
            .fetch_one(&self.pool)
            .await?;
        let unacknowledged_warnings: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM warnings WHERE acknowledged = false")
                .fetch_one(&self.pool)
                .await?;
        let total_insights: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM insights")
            .fetch_one(&self.pool)
            .await?;
        let active_recipes: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM recipes WHERE status IN ('active', 'production')",
        )
        .fetch_one(&self.pool)
        .await
        .unwrap_or(0);
        let new_insights_24h: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM insights WHERE created_at > now() - interval '24 hours'",
        )
        .fetch_one(&self.pool)
        .await
        .unwrap_or(0);
        let new_warnings_24h: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM warnings WHERE created_at > now() - interval '24 hours'",
        )
        .fetch_one(&self.pool)
        .await
        .unwrap_or(0);

        let top_regions: Vec<RegionCount> = sqlx::query_as(
            "SELECT COALESCE(region, 'Unknown') as region, COUNT(*) as count FROM companies GROUP BY region ORDER BY count DESC LIMIT 10"
        )
        .fetch_all(&self.pool)
        .await
        .unwrap_or_default();

        let threat_distribution: Vec<SeverityCount> = sqlx::query_as(
            "SELECT severity, COUNT(*) as count FROM warnings GROUP BY severity ORDER BY count DESC"
        )
        .fetch_all(&self.pool)
        .await
        .unwrap_or_default();

        let recent_activity = vec![
            ActivityItem {
                label: "Companies Tracked".to_string(),
                count: saturating_count_to_u64(total_companies),
                delta: 0,
            },
            ActivityItem {
                label: "Persons Tracked".to_string(),
                count: saturating_count_to_u64(total_persons),
                delta: 0,
            },
            ActivityItem {
                label: "Active Warnings".to_string(),
                count: saturating_count_to_u64(unacknowledged_warnings),
                delta: new_warnings_24h as i64,
            },
            ActivityItem {
                label: "Insights Generated".to_string(),
                count: saturating_count_to_u64(total_insights),
                delta: new_insights_24h as i64,
            },
        ];

        Ok(DashboardStats {
            total_companies: saturating_count_to_u64(total_companies),
            total_persons: saturating_count_to_u64(total_persons),
            total_warnings: saturating_count_to_u64(total_warnings),
            unacknowledged_warnings: saturating_count_to_u64(unacknowledged_warnings),
            total_insights: saturating_count_to_u64(total_insights),
            active_recipes: saturating_count_to_u64(active_recipes),
            new_insights_24h: saturating_count_to_u64(new_insights_24h),
            new_warnings_24h: saturating_count_to_u64(new_warnings_24h),
            top_regions,
            threat_distribution,
            recent_activity,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::saturating_count_to_u64;

    #[test]
    fn test_saturating_count_to_u64_clamps_negative_counts() {
        assert_eq!(saturating_count_to_u64(-5), 0);
    }

    #[test]
    fn test_saturating_count_to_u64_preserves_positive_counts() {
        assert_eq!(saturating_count_to_u64(42), 42);
    }
}
