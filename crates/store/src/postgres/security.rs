use super::*;

fn normalize_security_limit(limit: i64) -> i64 {
    clamp_limit(limit)
}

impl PgStore {
    /// Upsert a DNS posture entry for a domain.
    ///
    /// `has_dkim` is only true for a confirmed DKIM key record; the
    /// authoritative observation is `dkim_status` (tri-state), with
    /// `dkim_unknown_reason` recorded when DNS resolution was indeterminate.
    #[allow(clippy::too_many_arguments)]
    pub async fn insert_dns_posture_entry(
        &self,
        company_id: Option<Uuid>,
        domain: &str,
        has_spf: bool,
        has_dkim: bool,
        has_dmarc: bool,
        dmarc_policy: Option<&str>,
        spf_record: Option<&str>,
        posture_score: f64,
        dkim_status: &str,
        dkim_unknown_reason: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO dns_posture_entries
               (company_id, domain, has_spf, has_dkim, has_dmarc, dmarc_policy, spf_record, posture_score, dkim_status, dkim_unknown_reason, checked_at, updated_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, now(), now())
               ON CONFLICT (domain, checked_at) DO UPDATE SET
                 has_spf = EXCLUDED.has_spf,
                 has_dkim = EXCLUDED.has_dkim,
                 has_dmarc = EXCLUDED.has_dmarc,
                 dmarc_policy = EXCLUDED.dmarc_policy,
                 spf_record = EXCLUDED.spf_record,
                 posture_score = EXCLUDED.posture_score,
                 dkim_status = EXCLUDED.dkim_status,
                 dkim_unknown_reason = EXCLUDED.dkim_unknown_reason,
                 updated_at = now()"#,
        )
        .bind(company_id)
        .bind(domain)
        .bind(has_spf)
        .bind(has_dkim)
        .bind(has_dmarc)
        .bind(dmarc_policy)
        .bind(spf_record)
        .bind(posture_score)
        .bind(dkim_status)
        .bind(dkim_unknown_reason)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Upsert a CISA KEV catalog entry.
    pub async fn insert_kev_observation(
        &self,
        cve_id: &str,
        vulnerability_name: &str,
        vendor: Option<&str>,
        product: Option<&str>,
        date_added: Option<NaiveDate>,
        due_date: Option<NaiveDate>,
        notes: Option<&str>,
        relevance_score: f64,
    ) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO kev_observations
               (cve_id, vulnerability_name, vendor, product, date_added, due_date, notes, relevance_score, catalog_fetched_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, now())
               ON CONFLICT (cve_id) DO UPDATE SET
                 vulnerability_name = EXCLUDED.vulnerability_name,
                 vendor = EXCLUDED.vendor,
                 product = EXCLUDED.product,
                 date_added = EXCLUDED.date_added,
                 due_date = EXCLUDED.due_date,
                 notes = EXCLUDED.notes,
                 relevance_score = EXCLUDED.relevance_score,
                 catalog_fetched_at = EXCLUDED.catalog_fetched_at"#,
        )
        .bind(cve_id)
        .bind(vulnerability_name)
        .bind(vendor)
        .bind(product)
        .bind(date_added)
        .bind(due_date)
        .bind(notes)
        .bind(relevance_score)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Upsert a lookalike/typosquat domain detection.
    pub async fn insert_lookalike_domain(
        &self,
        company_id: Option<Uuid>,
        original_domain: &str,
        lookalike_domain: &str,
        threat_type: &str,
        distance: i32,
        active: bool,
    ) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO lookalike_domains
               (company_id, original_domain, lookalike_domain, threat_type, distance, active)
               VALUES ($1, $2, $3, $4, $5, $6)
               ON CONFLICT (original_domain, lookalike_domain) DO UPDATE SET
                 threat_type = EXCLUDED.threat_type,
                 distance = EXCLUDED.distance,
                 active = EXCLUDED.active"#,
        )
        .bind(company_id)
        .bind(original_domain)
        .bind(lookalike_domain)
        .bind(threat_type)
        .bind(distance)
        .bind(active)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_dns_posture_entries(&self, limit: i64) -> Result<Vec<ObservationRow>> {
        // One row per domain: posture is a property of the domain, so repeated
        // checks must not render as duplicate rows. The newest measurement per
        // domain wins.
        let rows = sqlx::query_as::<_, ObservationRow>(
            r#"SELECT id, observation_type, entity_id, entity_type, ts_utc, value, provenance, confidence, created_at
               FROM (
                   SELECT DISTINCT ON (
                              COALESCE(NULLIF(value->>'domain', ''), entity_id::text, id::text)
                          )
                          id, observation_type, entity_id, entity_type, ts_utc, value, provenance, confidence, created_at
                   FROM observations
                   -- Case-insensitive: producers write both the canonical
                   -- 'DnsPosture' entity name (lowercased: dnsposture) and the
                   -- lowercase wire label dns_posture.
                   WHERE lower(observation_type) IN
                         ('dns_posture', 'dnsposture', 'dmarc_check', 'spf_check', 'dkim_check')
                   ORDER BY COALESCE(NULLIF(value->>'domain', ''), entity_id::text, id::text) ASC,
                            ts_utc DESC, created_at DESC NULLS LAST, id DESC
               ) latest_per_domain
               ORDER BY ts_utc DESC
               LIMIT $1"#,
        )
        .bind(normalize_security_limit(limit))
        .fetch_all(&self.pool)
        .await?;

        // If observations table has few or no results, fall back to
        // the dedicated dns_posture_entries table which may have more data.
        if rows.len() < 20 {
            let fallback = self.get_dns_posture_entries_from_table(limit).await?;
            if fallback.len() > rows.len() {
                return Ok(fallback);
            }
        }

        Ok(rows)
    }

    /// Query DNS posture entries directly from the dedicated `dns_posture_entries` table,
    /// converting each row into an ObservationRow-compatible format.
    async fn get_dns_posture_entries_from_table(&self, limit: i64) -> Result<Vec<ObservationRow>> {
        #[derive(Debug, Clone, sqlx::FromRow)]
        struct DnsPostureTableRow {
            id: Uuid,
            company_id: Option<Uuid>,
            domain: String,
            has_spf: bool,
            has_dkim: bool,
            has_dmarc: bool,
            dmarc_policy: Option<String>,
            posture_score: f64,
            checked_at: DateTime<Utc>,
            dkim_status: String,
            dkim_unknown_reason: Option<String>,
        }

        // One row per domain (newest check wins): posture tables accumulate a
        // row per scan, and the readers must not render the same domain once
        // per scan.
        let rows = sqlx::query_as::<_, DnsPostureTableRow>(
            r#"SELECT id, company_id, domain, has_spf, has_dkim, has_dmarc, dmarc_policy,
                      posture_score, checked_at, dkim_status, dkim_unknown_reason
               FROM (
                   SELECT DISTINCT ON (domain)
                          id, company_id, domain, has_spf, has_dkim, has_dmarc, dmarc_policy,
                          posture_score, checked_at, dkim_status, dkim_unknown_reason
                   FROM dns_posture_entries
                   ORDER BY domain ASC, checked_at DESC, id DESC
               ) latest_per_domain
               ORDER BY checked_at DESC, domain ASC
               LIMIT $1"#,
        )
        .bind(normalize_security_limit(limit))
        .fetch_all(&self.pool)
        .await?;

        let obs_rows: Vec<ObservationRow> = rows
            .into_iter()
            .map(|r| {
                // Tri-state: an indeterminate DKIM lookup is "unknown", never
                // "absent". `has_dkim` stays the measured boolean (false when
                // unknown) and the status/reason carry the uncertainty so
                // consumers can exclude unknowns from pass/fail scoring.
                let dkim_measured = !r.dkim_status.eq_ignore_ascii_case("unknown");
                #[allow(clippy::unwrap_used, clippy::expect_used)]
                let value = serde_json::json!({
                    "domain": r.domain,
                    "has_spf": r.has_spf,
                    "has_dkim": r.has_dkim,
                    "dkim_status": r.dkim_status,
                    "dkim_measured": dkim_measured,
                    "dkim_unknown_reason": r.dkim_unknown_reason,
                    "has_dmarc": r.has_dmarc,
                    "dmarc_policy": r.dmarc_policy,
                    "posture_score": r.posture_score,
                });
                #[allow(clippy::unwrap_used, clippy::expect_used)]
                let provenance = serde_json::json!({
                    "source": "dns_posture_entries_table",
                    "content_hash": format!("dns_{}", r.domain),
                });
                ObservationRow {
                    id: r.id,
                    observation_type: "dns_posture".to_string(),
                    entity_id: r.company_id,
                    entity_type: Some("company".to_string()),
                    ts_utc: r.checked_at,
                    value,
                    provenance,
                    confidence: Some(0.95),
                    created_at: Some(r.checked_at),
                }
            })
            .collect();

        Ok(obs_rows)
    }

    pub async fn get_lookalike_domains(&self, limit: i64) -> Result<Vec<ObservationRow>> {
        let rows = sqlx::query_as::<_, ObservationRow>(
            "SELECT id, observation_type, entity_id, entity_type, ts_utc, value, provenance, confidence, created_at
             FROM observations
             WHERE observation_type IN ('lookalike_domain', 'typosquat', 'ct_cert_lookalike')
             ORDER BY ts_utc DESC
             LIMIT $1",
        )
        .bind(normalize_security_limit(limit))
        .fetch_all(&self.pool)
        .await?;

        // If observations table has few or no results, fall back to
        // the dedicated lookalike_domains table which may have more data.
        if rows.len() < 20 {
            let fallback = self.get_lookalike_domains_from_table(limit).await?;
            if fallback.len() > rows.len() {
                return Ok(fallback);
            }
        }

        Ok(rows)
    }

    /// Query lookalike domains directly from the dedicated `lookalike_domains` table,
    /// converting each row into an ObservationRow-compatible format.
    /// This ensures we always have results even if observations are pruned by retention.
    pub async fn get_lookalike_domains_from_table(
        &self,
        limit: i64,
    ) -> Result<Vec<ObservationRow>> {
        #[derive(Debug, Clone, sqlx::FromRow)]
        struct LookalikeDomainRow {
            id: Uuid,
            company_id: Option<Uuid>,
            original_domain: String,
            lookalike_domain: String,
            threat_type: String,
            distance: i32,
            active: bool,
            detected_at: DateTime<Utc>,
        }

        let rows = sqlx::query_as::<_, LookalikeDomainRow>(
            "SELECT id, company_id, original_domain, lookalike_domain, threat_type, distance, active, detected_at
             FROM lookalike_domains
             ORDER BY detected_at DESC
             LIMIT $1",
        )
        .bind(normalize_security_limit(limit))
        .fetch_all(&self.pool)
        .await?;

        let obs_rows: Vec<ObservationRow> = rows
            .into_iter()
            .map(|r| {
                #[allow(clippy::unwrap_used, clippy::expect_used)]
                let value = serde_json::json!({
                    "original_domain": r.original_domain,
                    "domain": r.lookalike_domain,
                    "distance": r.distance,
                    "threat_type": r.threat_type,
                    "active": r.active,
                });
                #[allow(clippy::unwrap_used, clippy::expect_used)]
                let provenance = serde_json::json!({
                    "source": "lookalike_domains_table",
                    "content_hash": format!("la_{}_{}", r.original_domain, r.lookalike_domain),
                });
                ObservationRow {
                    id: r.id,
                    observation_type: "lookalike_domain".to_string(),
                    entity_id: r.company_id,
                    entity_type: Some("company".to_string()),
                    ts_utc: r.detected_at,
                    value,
                    provenance,
                    confidence: Some(0.80),
                    created_at: Some(r.detected_at),
                }
            })
            .collect();

        Ok(obs_rows)
    }

    pub async fn get_kev_relevance(&self, limit: i64) -> Result<Vec<ObservationRow>> {
        let rows = sqlx::query_as::<_, ObservationRow>(
            "SELECT id, observation_type, entity_id, entity_type, ts_utc, value, provenance, confidence, created_at
             FROM observations
             WHERE observation_type IN ('kev_match', 'cve_relevance', 'psirt_advisory')
             ORDER BY ts_utc DESC
             LIMIT $1",
        )
        .bind(normalize_security_limit(limit))
        .fetch_all(&self.pool)
        .await?;

        // If observations table has few or no results, fall back to
        // the dedicated kev_observations table which may have more data.
        if rows.len() < 10 {
            let fallback = self.get_kev_relevance_from_table(limit).await?;
            if fallback.len() > rows.len() {
                return Ok(fallback);
            }
        }

        Ok(rows)
    }

    /// Query KEV entries directly from the dedicated `kev_observations` table,
    /// converting each row into an ObservationRow-compatible format.
    async fn get_kev_relevance_from_table(&self, limit: i64) -> Result<Vec<ObservationRow>> {
        #[derive(Debug, Clone, sqlx::FromRow)]
        struct KevTableRow {
            id: Uuid,
            cve_id: String,
            vulnerability_name: String,
            vendor: Option<String>,
            product: Option<String>,
            date_added: Option<NaiveDate>,
            due_date: Option<NaiveDate>,
            notes: Option<String>,
            relevance_score: f64,
            catalog_fetched_at: DateTime<Utc>,
        }

        let rows = sqlx::query_as::<_, KevTableRow>(
            "SELECT id, cve_id, vulnerability_name, vendor, product, date_added, due_date, notes, relevance_score, catalog_fetched_at
             FROM kev_observations
             ORDER BY relevance_score DESC
             LIMIT $1",
        )
        .bind(normalize_security_limit(limit))
        .fetch_all(&self.pool)
        .await?;

        let obs_rows: Vec<ObservationRow> = rows
            .into_iter()
            .map(|r| {
                let date_added_str = r.date_added.map(|d| d.to_string()).unwrap_or_default();
                let due_date_str = r.due_date.map(|d| d.to_string()).unwrap_or_default();
                #[allow(clippy::unwrap_used, clippy::expect_used)]
                let value = serde_json::json!({
                    "cve_id": r.cve_id,
                    "vulnerability_name": r.vulnerability_name,
                    "name": r.vulnerability_name,
                    "vendor": r.vendor,
                    "product": r.product,
                    "date_added": date_added_str,
                    "due_date": due_date_str,
                    "relevance_score": r.relevance_score,
                    "notes": r.notes,
                });
                // A catalog row is reference data, not a finding about a
                // monitored entity; `catalog_entry` lets readers separate the
                // catalog from observed matches instead of counting catalog
                // size as "CVE findings".
                #[allow(clippy::unwrap_used, clippy::expect_used)]
                let provenance = serde_json::json!({
                    "source": "kev_observations_table",
                    "catalog_entry": true,
                    "content_hash": format!("kev_{}", r.cve_id),
                });
                ObservationRow {
                    id: r.id,
                    observation_type: "kev_match".to_string(),
                    entity_id: None,
                    entity_type: None,
                    ts_utc: r.catalog_fetched_at,
                    value,
                    provenance,
                    confidence: Some(r.relevance_score),
                    created_at: Some(r.catalog_fetched_at),
                }
            })
            .collect();

        Ok(obs_rows)
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_security_limit;
    use crate::postgres::MAX_LIST_LIMIT;

    #[test]
    fn test_normalize_security_limit_clamps_limit_and_offset() {
        assert_eq!(normalize_security_limit(0), 1);
        assert_eq!(
            normalize_security_limit(MAX_LIST_LIMIT + 10),
            MAX_LIST_LIMIT
        );
    }

    #[test]
    fn test_normalize_security_limit_preserves_valid_values() {
        assert_eq!(normalize_security_limit(12), 12);
        assert_eq!(normalize_security_limit(MAX_LIST_LIMIT), MAX_LIST_LIMIT);
    }
}
