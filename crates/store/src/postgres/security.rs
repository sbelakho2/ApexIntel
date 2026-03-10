use super::*;

fn normalize_security_limit(limit: i64) -> i64 {
    clamp_limit(limit)
}

impl PgStore {
    /// Upsert a DNS posture entry for a domain.
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
    ) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO dns_posture_entries
               (company_id, domain, has_spf, has_dkim, has_dmarc, dmarc_policy, spf_record, posture_score, checked_at, updated_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, now(), now())
               ON CONFLICT (domain, checked_at) DO UPDATE SET
                 has_spf = EXCLUDED.has_spf,
                 has_dkim = EXCLUDED.has_dkim,
                 has_dmarc = EXCLUDED.has_dmarc,
                 dmarc_policy = EXCLUDED.dmarc_policy,
                 spf_record = EXCLUDED.spf_record,
                 posture_score = EXCLUDED.posture_score,
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
        let rows = sqlx::query_as::<_, ObservationRow>(
            "SELECT id, observation_type, entity_id, entity_type, ts_utc, value, provenance, confidence, created_at
             FROM observations
             WHERE observation_type IN ('dns_posture', 'dmarc_check', 'spf_check', 'dkim_check')
             ORDER BY ts_utc DESC
             LIMIT $1",
        )
        .bind(normalize_security_limit(limit))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
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
        Ok(rows)
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
        Ok(rows)
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
