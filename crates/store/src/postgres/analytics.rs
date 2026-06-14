use super::*;

fn saturating_count_to_u64(value: i64) -> u64 {
    value.max(0) as u64
}

impl PgStore {
    pub async fn get_daily_observation_counts_per_entity(
        &self,
        since: DateTime<Utc>,
    ) -> Result<Vec<(Uuid, i64, i64)>> {
        let rows = sqlx::query(
            r#"SELECT entity_id,
                      DATE_PART('day', date_trunc('day', ts_utc) - date_trunc('day', $1))::BIGINT AS day_offset,
                      COUNT(*)::BIGINT AS cnt
               FROM observations
               WHERE entity_id IS NOT NULL AND ts_utc >= $1
               GROUP BY entity_id, day_offset
               ORDER BY entity_id, day_offset"#,
        )
        .bind(since)
        .fetch_all(&self.pool)
        .await?;

        use sqlx::Row as _;
        Ok(rows
            .into_iter()
            .filter_map(|row| {
                let entity_id: Uuid = row.try_get("entity_id").ok()?;
                let day_offset: i64 = row.try_get("day_offset").ok()?;
                let cnt: i64 = row.try_get("cnt").ok()?;
                Some((entity_id, day_offset, cnt))
            })
            .collect())
    }

    pub async fn get_daily_warning_counts_per_entity(
        &self,
        since: DateTime<Utc>,
    ) -> Result<Vec<(Uuid, i64, i64)>> {
        let rows = sqlx::query(
            r#"SELECT unnest(entity_ids) AS entity_id,
                      DATE_PART('day', date_trunc('day', COALESCE(created_at, ts_utc)) - date_trunc('day', $1))::BIGINT AS day_offset,
                      COUNT(*)::BIGINT AS cnt
               FROM warnings
               WHERE entity_ids IS NOT NULL
                                 AND deleted_at IS NULL
                 AND array_length(entity_ids, 1) > 0
                 AND COALESCE(created_at, ts_utc) >= $1
               GROUP BY entity_id, day_offset
               ORDER BY entity_id, day_offset"#,
        )
        .bind(since)
        .fetch_all(&self.pool)
        .await?;

        use sqlx::Row as _;
        Ok(rows
            .into_iter()
            .filter_map(|row| {
                let entity_id: Uuid = row.try_get("entity_id").ok()?;
                let day_offset: i64 = row.try_get("day_offset").ok()?;
                let cnt: i64 = row.try_get("cnt").ok()?;
                Some((entity_id, day_offset, cnt))
            })
            .collect())
    }

    pub async fn record_stats_alert_calibration_event(
        &self,
        entity_id: Uuid,
        feature_vector: &Value,
        alert_level: &str,
        predicted_at: DateTime<Utc>,
        expected_by: DateTime<Utc>,
        metadata: &Value,
    ) -> Result<StatsAlertCalibrationEventRecord> {
        Ok(sqlx::query_as::<_, StatsAlertCalibrationEventRecord>(
            r#"INSERT INTO stats_alert_calibration_events (
                   entity_id, feature_vector, alert_level, predicted_at, expected_by, metadata
               )
               VALUES ($1, $2, $3, $4, $5, $6)
               RETURNING id, entity_id, feature_vector, alert_level, predicted_at, expected_by,
                         actual_outcome_within_30d, outcome_source, outcome_reference_id,
                         resolved_at, metadata, created_at"#,
        )
        .bind(entity_id)
        .bind(feature_vector)
        .bind(alert_level)
        .bind(predicted_at)
        .bind(expected_by)
        .bind(metadata)
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn resolve_stats_alert_calibration_events(&self, now: DateTime<Utc>) -> Result<u64> {
        let result = sqlx::query(
            r#"WITH outcome_match AS (
                   SELECT e.id,
                          EXISTS(
                              SELECT 1
                              FROM warnings w
                              WHERE w.entity_ids IS NOT NULL
                                                                AND w.deleted_at IS NULL
                                AND e.entity_id = ANY(w.entity_ids)
                                AND COALESCE(w.created_at, w.ts_utc) >= e.predicted_at
                                AND COALESCE(w.created_at, w.ts_utc) <= e.expected_by
                          ) AS has_outcome,
                          (
                              SELECT w.id
                              FROM warnings w
                              WHERE w.entity_ids IS NOT NULL
                                                                AND w.deleted_at IS NULL
                                AND e.entity_id = ANY(w.entity_ids)
                                AND COALESCE(w.created_at, w.ts_utc) >= e.predicted_at
                                AND COALESCE(w.created_at, w.ts_utc) <= e.expected_by
                              ORDER BY COALESCE(w.created_at, w.ts_utc) ASC, w.id ASC
                              LIMIT 1
                          ) AS warning_id
                   FROM stats_alert_calibration_events e
                   WHERE e.actual_outcome_within_30d IS NULL
                     AND e.expected_by <= $1
               )
               UPDATE stats_alert_calibration_events e
               SET actual_outcome_within_30d = outcome_match.has_outcome,
                   outcome_source = CASE WHEN outcome_match.has_outcome THEN 'warning' ELSE 'none' END,
                   outcome_reference_id = outcome_match.warning_id,
                   resolved_at = $1
               FROM outcome_match
               WHERE e.id = outcome_match.id"#,
        )
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    pub async fn list_resolved_stats_alert_calibration_samples(
        &self,
        since: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<Vec<ResolvedStatsAlertCalibrationSampleRecord>> {
        let limit = clamp_limit(limit);
        match since {
            Some(since) => Ok(
                sqlx::query_as::<_, ResolvedStatsAlertCalibrationSampleRecord>(
                    r#"SELECT id, entity_id, feature_vector, alert_level,
                          actual_outcome_within_30d, resolved_at, metadata
                   FROM stats_alert_calibration_events
                   WHERE actual_outcome_within_30d IS NOT NULL
                     AND resolved_at IS NOT NULL
                     AND resolved_at >= $1
                   ORDER BY resolved_at DESC, id DESC
                   LIMIT $2"#,
                )
                .bind(since)
                .bind(limit)
                .fetch_all(&self.pool)
                .await?,
            ),
            None => Ok(
                sqlx::query_as::<_, ResolvedStatsAlertCalibrationSampleRecord>(
                    r#"SELECT id, entity_id, feature_vector, alert_level,
                          actual_outcome_within_30d, resolved_at, metadata
                   FROM stats_alert_calibration_events
                   WHERE actual_outcome_within_30d IS NOT NULL
                     AND resolved_at IS NOT NULL
                   ORDER BY resolved_at DESC, id DESC
                   LIMIT $1"#,
                )
                .bind(limit)
                .fetch_all(&self.pool)
                .await?,
            ),
        }
    }

    pub async fn aggregate_source_reliability_outcomes(
        &self,
        since: Option<DateTime<Utc>>,
    ) -> Result<Vec<SourceReliabilityAggregateRecord>> {
        let sql = r#"WITH normalized_observations AS (
                   SELECT entity_id,
                          ts_utc,
                          lower(
                              regexp_replace(
                                  split_part(
                                      split_part(
                                          regexp_replace(
                                              COALESCE(
                                                  NULLIF(provenance->>'source_domain', ''),
                                                  NULLIF(provenance->>'domain', ''),
                                                  NULLIF(provenance->>'source_url', ''),
                                                  NULLIF(provenance->>'url', '')
                                              ),
                                              '^https?://',
                                              ''
                                          ),
                                          '/',
                                          1
                                      ),
                                      ':',
                                      1
                                  ),
                                  '^www\\.',
                                  ''
                              )
                          ) AS source_domain
                   FROM observations
                   WHERE entity_id IS NOT NULL
                     AND ($1::timestamptz IS NULL OR ts_utc >= $1)
               )
               SELECT o.source_domain,
                      COUNT(*)::BIGINT AS observation_count,
                      COUNT(*) FILTER (
                          WHERE EXISTS (
                              SELECT 1
                              FROM warnings w
                              WHERE w.entity_ids IS NOT NULL
                                                                AND w.deleted_at IS NULL
                                AND o.entity_id = ANY(w.entity_ids)
                                AND COALESCE(w.created_at, w.ts_utc) >= o.ts_utc
                                AND COALESCE(w.created_at, w.ts_utc) <= o.ts_utc + INTERVAL '30 days'
                          )
                      )::BIGINT AS confirmed_count
               FROM normalized_observations o
               WHERE o.source_domain IS NOT NULL
                 AND o.source_domain <> ''
               GROUP BY o.source_domain
               ORDER BY observation_count DESC, o.source_domain ASC"#;

        Ok(sqlx::query_as::<_, SourceReliabilityAggregateRecord>(sql)
            .bind(since)
            .fetch_all(&self.pool)
            .await?)
    }

    pub async fn upsert_source_reliability_stat(
        &self,
        source_domain: &str,
        tier: &str,
        observation_count: i64,
        confirmed_count: i64,
        observed_reliability: f64,
        effective_reliability: f64,
        promotion_recommended: bool,
        last_refreshed_at: DateTime<Utc>,
    ) -> Result<SourceReliabilityStatRecord> {
        Ok(sqlx::query_as::<_, SourceReliabilityStatRecord>(
            r#"INSERT INTO source_reliability_stats (
                   source_domain,
                   tier,
                   observation_count,
                   confirmed_count,
                   observed_reliability,
                   effective_reliability,
                   promotion_recommended,
                   last_refreshed_at
               )
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
               ON CONFLICT (source_domain) DO UPDATE SET
                   tier = EXCLUDED.tier,
                   observation_count = EXCLUDED.observation_count,
                   confirmed_count = EXCLUDED.confirmed_count,
                   observed_reliability = EXCLUDED.observed_reliability,
                   effective_reliability = EXCLUDED.effective_reliability,
                   promotion_recommended = EXCLUDED.promotion_recommended,
                   last_refreshed_at = EXCLUDED.last_refreshed_at,
                   promotion_alerted_at = CASE
                       WHEN EXCLUDED.promotion_recommended THEN source_reliability_stats.promotion_alerted_at
                       ELSE NULL
                   END,
                   updated_at = NOW()
               RETURNING source_domain, tier, observation_count, confirmed_count,
                         observed_reliability, effective_reliability,
                         promotion_recommended, last_refreshed_at,
                         promotion_alerted_at, created_at, updated_at"#,
        )
        .bind(source_domain)
        .bind(tier)
        .bind(observation_count)
        .bind(confirmed_count)
        .bind(observed_reliability)
        .bind(effective_reliability)
        .bind(promotion_recommended)
        .bind(last_refreshed_at)
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn list_pending_source_reliability_promotions(
        &self,
        limit: i64,
    ) -> Result<Vec<SourceReliabilityStatRecord>> {
        let limit = clamp_limit(limit);
        Ok(sqlx::query_as::<_, SourceReliabilityStatRecord>(
            r#"SELECT source_domain, tier, observation_count, confirmed_count,
                      observed_reliability, effective_reliability,
                      promotion_recommended, last_refreshed_at,
                      promotion_alerted_at, created_at, updated_at
               FROM source_reliability_stats
               WHERE promotion_recommended = TRUE
                 AND promotion_alerted_at IS NULL
               ORDER BY effective_reliability DESC, observation_count DESC, source_domain ASC
               LIMIT $1"#,
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn mark_source_reliability_promotion_alerted(
        &self,
        source_domain: &str,
        alerted_at: DateTime<Utc>,
    ) -> Result<bool> {
        let result = sqlx::query(
            r#"UPDATE source_reliability_stats
               SET promotion_alerted_at = $2,
                   updated_at = NOW()
               WHERE source_domain = $1
                 AND promotion_recommended = TRUE
                 AND promotion_alerted_at IS NULL"#,
        )
        .bind(source_domain)
        .bind(alerted_at)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

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

    /// Extract job-post payload features per entity: role_family, seniority, count.
    pub async fn get_job_post_features_per_entity(
        &self,
        since: DateTime<Utc>,
    ) -> Result<Vec<(Uuid, String, String, i64)>> {
        let rows = sqlx::query(
            r#"SELECT entity_id,
                      COALESCE(value->>'role_family', '') AS rf,
                      COALESCE(value->>'seniority', '') AS sen,
                      COUNT(*)::BIGINT AS cnt
               FROM observations
               WHERE observation_type = 'JobPost'
                 AND entity_id IS NOT NULL
                 AND ts_utc >= $1
               GROUP BY entity_id, rf, sen
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
                let rf: String = row.try_get("rf").ok()?;
                let sen: String = row.try_get("sen").ok()?;
                let cnt: i64 = row.try_get("cnt").ok()?;
                Some((eid, rf, sen, cnt))
            })
            .collect())
    }

    /// Extract commodity/FX observation payload features per entity.
    pub async fn get_commodity_fx_features_per_entity(
        &self,
        since: DateTime<Utc>,
    ) -> Result<Vec<(Uuid, String, String, i64)>> {
        let rows = sqlx::query(
            r#"SELECT entity_id,
                      observation_type AS otype,
                      COALESCE(value->>'commodity', value->>'pair', '') AS item,
                      COUNT(*)::BIGINT AS cnt
               FROM observations
               WHERE observation_type IN ('CommodityPrice', 'FxRate')
                 AND entity_id IS NOT NULL
                 AND ts_utc >= $1
               GROUP BY entity_id, otype, item
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
                let otype: String = row.try_get("otype").ok()?;
                let item: String = row.try_get("item").ok()?;
                let cnt: i64 = row.try_get("cnt").ok()?;
                Some((eid, otype, item, cnt))
            })
            .collect())
    }

    /// Extract POI artifact features per company (articles, appearances, etc.)
    pub async fn get_poi_artifact_features_per_company(
        &self,
        since: DateTime<Utc>,
    ) -> Result<Vec<(Uuid, String, i64)>> {
        let rows = sqlx::query(
            r#"SELECT p.primary_org_id AS company_id,
                      pa.artifact_type,
                      COUNT(*)::BIGINT AS cnt
               FROM poi_artifacts pa
               JOIN persons p ON p.id = pa.person_id
               WHERE p.primary_org_id IS NOT NULL
                 AND pa.ts_utc >= $1
               GROUP BY p.primary_org_id, pa.artifact_type
               ORDER BY p.primary_org_id, cnt DESC"#,
        )
        .bind(since)
        .fetch_all(&self.pool)
        .await?;

        use sqlx::Row as _;
        Ok(rows
            .into_iter()
            .filter_map(|row| {
                let cid: Uuid = row.try_get("company_id").ok()?;
                let at: String = row.try_get("artifact_type").ok()?;
                let cnt: i64 = row.try_get("cnt").ok()?;
                Some((cid, at, cnt))
            })
            .collect())
    }

    /// Extract graph edge details with target names for evidence loading.
    pub async fn get_graph_edge_evidence(
        &self,
        entity_id: Uuid,
    ) -> Result<Vec<(String, String, String, f64, f64)>> {
        let rows = sqlx::query(
            r#"SELECT ge.edge_type,
                      ge.target_type,
                      COALESCE(
                          CASE WHEN ge.target_type = 'company' THEN (SELECT name FROM companies WHERE id = ge.target_id)
                               WHEN ge.target_type = 'person'  THEN (SELECT name FROM persons WHERE id = ge.target_id)
                               ELSE NULL END,
                          ge.target_id::TEXT
                      ) AS target_name,
                      ge.weight,
                      ge.confidence
               FROM graph_edges ge
               WHERE ge.source_id = $1
               ORDER BY ge.weight DESC, ge.confidence DESC
               LIMIT 10"#,
        )
        .bind(entity_id)
        .fetch_all(&self.pool)
        .await?;

        use sqlx::Row as _;
        Ok(rows
            .into_iter()
            .filter_map(|row| {
                let et: String = row.try_get("edge_type").ok()?;
                let tt: String = row.try_get("target_type").ok()?;
                let tn: String = row.try_get("target_name").ok()?;
                let w: f64 = row.try_get("weight").ok()?;
                let c: f64 = row.try_get("confidence").ok()?;
                Some((et, tt, tn, w, c))
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

    /// Return certification features only for certs that represent *dynamic* signals:
    /// recently added/updated (within `since`) or expiring within 90 days.
    /// Static/long-held certifications are excluded to prevent feature inflation.
    pub async fn get_certification_features_per_company(
        &self,
        since: DateTime<Utc>,
    ) -> Result<Vec<(Uuid, String, i64)>> {
        let rows = sqlx::query(
            r#"SELECT company_id,
                      standard,
                      COUNT(*)::BIGINT AS cnt
               FROM certifications
               WHERE company_id IS NOT NULL
                 AND (
                   updated_at >= $1
                   OR created_at >= $1
                   OR (valid_until IS NOT NULL
                       AND valid_until BETWEEN CURRENT_DATE AND CURRENT_DATE + INTERVAL '90 days')
                 )
               GROUP BY company_id, standard
               ORDER BY company_id"#,
        )
        .bind(since)
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

        let warnings_generated: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM warnings WHERE deleted_at IS NULL AND created_at >= $1",
        )
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
                             WHERE deleted_at IS NULL
                                 AND created_at >= $1
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
                             WHERE deleted_at IS NULL
                                 AND created_at >= $1
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
        let total_warnings: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM warnings WHERE deleted_at IS NULL")
                .fetch_one(&self.pool)
                .await?;
        let unacknowledged_warnings: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM warnings WHERE deleted_at IS NULL AND acknowledged = false",
        )
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
            "SELECT COUNT(*) FROM warnings WHERE deleted_at IS NULL AND created_at > now() - interval '24 hours'",
        )
        .fetch_one(&self.pool)
        .await
        .unwrap_or(0);

        let top_regions: Vec<RegionCount> = {
            let mut rows: Vec<RegionCount> = sqlx::query_as(
                "SELECT COALESCE(region, 'Unknown') as region, COUNT(*) as count FROM companies GROUP BY region ORDER BY count DESC"
            )
            .fetch_all(&self.pool)
            .await
            .unwrap_or_default();

            // Collapse regions beyond top 9 into an "Other" bucket so the
            // donut chart accounts for every company.
            if rows.len() > 9 {
                let other_count: i64 = rows[9..].iter().map(|r| r.count).sum();
                rows.truncate(9);
                if other_count > 0 {
                    rows.push(RegionCount {
                        region: "Other".to_string(),
                        count: other_count,
                    });
                }
            }
            rows
        };

        let threat_distribution: Vec<SeverityCount> = sqlx::query_as(
            "SELECT severity, COUNT(*) as count FROM warnings WHERE deleted_at IS NULL GROUP BY severity ORDER BY count DESC"
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

    /// Materialize pattern candidates from recent observations.
    ///
    /// Finds entity+observation_type pairs that appear 3+ times in the window,
    /// inserts them as candidates, and marks high-confidence ones as passed.
    pub async fn materialize_pattern_candidates(&self, since: DateTime<Utc>) -> Result<u64> {
        let result = sqlx::query(
            r#"INSERT INTO pattern_candidates (recipe_code, entity_type, pattern_label, passed_gates, confidence, created_at)
               SELECT
                   'auto_' || o.observation_type,
                   o.observation_type,
                   o.observation_type || ':' || COALESCE(o.entity_id::text, 'global'),
                   CASE WHEN COUNT(*) >= 5 THEN true ELSE false END,
                   LEAST(1.0, COUNT(*)::double precision / 10.0),
                   now()
               FROM observations o
               WHERE o.ts_utc >= $1
               GROUP BY o.observation_type, o.entity_id
               HAVING COUNT(*) >= 3
               ON CONFLICT DO NOTHING"#,
        )
        .bind(since)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    /// Materialize feature rows from entity observation signals.
    ///
    /// Computes signal counts and drift scores per entity for drift detection.
    pub async fn materialize_feature_rows(&self) -> Result<u64> {
        let now_bucket = Utc::now().timestamp() / 86400;
        let result = sqlx::query(
            r#"INSERT INTO feature_rows (entity_id, entity_type, time_bucket, bucket_size_days, data, created_at)
               SELECT
                   COALESCE(o.entity_id::text, 'global'),
                   o.observation_type,
                   $1::bigint,
                   1,
                   jsonb_build_object(
                       'signal_count', COUNT(*),
                       'drift_score', CASE
                           WHEN prev.prev_count IS NULL OR prev.prev_count = 0 THEN 0.0
                           ELSE ABS(COUNT(*)::double precision - prev.prev_count) / GREATEST(prev.prev_count, 1.0)
                       END
                   ),
                   now()
               FROM observations o
               LEFT JOIN LATERAL (
                   SELECT COUNT(*)::double precision AS prev_count
                   FROM observations o2
                   WHERE o2.entity_id = o.entity_id
                     AND o2.observation_type = o.observation_type
                     AND o2.ts_utc >= now() - INTERVAL '48 hours'
                     AND o2.ts_utc < now() - INTERVAL '24 hours'
               ) prev ON true
               WHERE o.ts_utc >= now() - INTERVAL '24 hours'
               GROUP BY o.entity_id, o.observation_type, prev.prev_count
               ON CONFLICT (entity_id, entity_type, time_bucket, bucket_size_days)
               DO UPDATE SET
                   data = EXCLUDED.data,
                   created_at = EXCLUDED.created_at"#,
        )
        .bind(now_bucket)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    /// Load entity-linked observation event streams for pattern mining.
    ///
    /// Returns a map keyed by `observation_type`, where each value is the chronologically
    /// ordered list of `(entity_id, unix_timestamp_secs)` events. Streams with fewer than
    /// `min_events` events are dropped, and the total number of loaded rows is capped at
    /// `max_rows` (oldest first) to bound memory and query time. `observation_type` is the
    /// mining signal taxonomy: the `value` JSONB carries no finer-grained sub-type.
    pub async fn load_observation_event_streams(
        &self,
        since: DateTime<Utc>,
        min_events: usize,
        max_rows: i64,
    ) -> Result<std::collections::HashMap<String, Vec<(String, i64)>>> {
        let rows = sqlx::query_as::<_, (String, String, i64)>(
            r#"SELECT observation_type,
                      entity_id::text,
                      EXTRACT(EPOCH FROM ts_utc)::bigint
               FROM observations
               WHERE ts_utc >= $1
                 AND entity_id IS NOT NULL
                 AND observation_type IS NOT NULL
               ORDER BY ts_utc
               LIMIT $2"#,
        )
        .bind(since)
        .bind(max_rows.max(0))
        .fetch_all(&self.pool)
        .await?;

        let mut streams: std::collections::HashMap<String, Vec<(String, i64)>> =
            std::collections::HashMap::new();
        for (observation_type, entity_id, ts_secs) in rows {
            streams
                .entry(observation_type)
                .or_default()
                .push((entity_id, ts_secs));
        }
        streams.retain(|_, events| events.len() >= min_events);
        Ok(streams)
    }

    /// Persist mined pattern candidates for audit and analytics counters.
    ///
    /// Each row is stamped with `created_at = now()` so it counts within the rolling
    /// analytics window read by [`PgStore::get_mining_stats`]. Inserts run in a single
    /// transaction and the number of inserted rows is returned.
    pub async fn insert_pattern_candidates(
        &self,
        candidates: &[MinedPatternCandidate],
    ) -> Result<u64> {
        if candidates.is_empty() {
            return Ok(0);
        }
        let mut tx = self.pool.begin().await?;
        let mut inserted = 0u64;
        for candidate in candidates {
            let result = sqlx::query(
                r#"INSERT INTO pattern_candidates
                       (recipe_code, entity_type, pattern_label, passed_gates, confidence, created_at)
                   VALUES ($1, $2, $3, $4, $5, now())"#,
            )
            .bind(&candidate.recipe_code)
            .bind(&candidate.entity_type)
            .bind(&candidate.pattern_label)
            .bind(candidate.passed_gates)
            .bind(candidate.confidence)
            .execute(&mut *tx)
            .await?;
            inserted += result.rows_affected();
        }
        tx.commit().await?;
        Ok(inserted)
    }

    /// List all existing recipe codes, used to deduplicate newly staged hypotheses.
    pub async fn list_recipe_codes(&self) -> Result<Vec<String>> {
        let codes = sqlx::query_scalar::<_, String>("SELECT code FROM recipes")
            .fetch_all(&self.pool)
            .await?;
        Ok(codes)
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
