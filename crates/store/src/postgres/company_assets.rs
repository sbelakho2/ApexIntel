use super::*;

fn normalize_company_assets_window(limit: i64, offset: i64) -> (i64, i64) {
    (clamp_limit(limit), offset.max(0))
}

impl PgStore {
    pub async fn insert_site(&self, s: &Site) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO sites
               (id, company_id, name, address, city, country_code, region,
                lat, lon, site_type, capabilities, certifications,
                employee_estimate, free_zone, metadata, created_at, updated_at)
               VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17)
               ON CONFLICT (id) DO UPDATE SET
                 name = EXCLUDED.name,
                 address = EXCLUDED.address,
                 city = EXCLUDED.city,
                 country_code = EXCLUDED.country_code,
                 region = EXCLUDED.region,
                 lat = EXCLUDED.lat,
                 lon = EXCLUDED.lon,
                 site_type = EXCLUDED.site_type,
                 capabilities = EXCLUDED.capabilities,
                 certifications = EXCLUDED.certifications,
                 employee_estimate = EXCLUDED.employee_estimate,
                 free_zone = EXCLUDED.free_zone,
                 metadata = EXCLUDED.metadata,
                 updated_at = now()"#,
        )
        .bind(s.id)
        .bind(s.company_id)
        .bind(&s.name)
        .bind(&s.address)
        .bind(&s.city)
        .bind(&s.country_code)
        .bind(&s.region)
        .bind(s.lat)
        .bind(s.lon)
        .bind(s.site_type.as_str())
        .bind(&s.capabilities)
        .bind(&s.certifications)
        .bind(s.employee_estimate)
        .bind(&s.free_zone)
        .bind(&s.metadata)
        .bind(s.created_at)
        .bind(s.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_sites_for_company(&self, company_id: Uuid) -> Result<Vec<SiteRow>> {
        let rows = sqlx::query_as::<_, SiteRow>(
            "SELECT id, company_id, name, address, city, country_code, region,
                    lat, lon, site_type, capabilities, certifications,
                    employee_estimate, free_zone, metadata, created_at, updated_at
             FROM sites WHERE company_id = $1 ORDER BY name",
        )
        .bind(company_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn insert_certification(&self, c: &Certification) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO certifications
               (id, company_id, site_id, standard, status, issuing_body,
                valid_from, valid_until, scope, evidence_url, metadata,
                created_at, updated_at)
               VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)
               ON CONFLICT (id) DO UPDATE SET
                 status = EXCLUDED.status,
                 issuing_body = EXCLUDED.issuing_body,
                 valid_from = EXCLUDED.valid_from,
                 valid_until = EXCLUDED.valid_until,
                 scope = EXCLUDED.scope,
                 evidence_url = EXCLUDED.evidence_url,
                 metadata = EXCLUDED.metadata,
                 updated_at = now()"#,
        )
        .bind(c.id)
        .bind(c.company_id)
        .bind(c.site_id)
        .bind(&c.standard)
        .bind(c.status.as_str())
        .bind(&c.issuing_body)
        .bind(c.valid_from)
        .bind(c.valid_until)
        .bind(&c.scope)
        .bind(&c.evidence_url)
        .bind(&c.metadata)
        .bind(c.created_at)
        .bind(c.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_certifications_for_company(
        &self,
        company_id: Uuid,
    ) -> Result<Vec<CertificationRow>> {
        let rows = sqlx::query_as::<_, CertificationRow>(
            "SELECT id, company_id, site_id, standard, status, issuing_body,
                    valid_from, valid_until, scope, evidence_url, metadata,
                    created_at, updated_at
             FROM certifications WHERE company_id = $1 ORDER BY standard",
        )
        .bind(company_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn insert_capability(&self, cap: &Capability) -> Result<()> {
        let evidence_urls = normalize_url_vec(&cap.evidence_urls);
        sqlx::query(
            r#"INSERT INTO capabilities
               (id, company_id, site_id, capability, proof_grade,
                evidence_urls, first_seen, last_confirmed, metadata)
               VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)
               ON CONFLICT (id) DO UPDATE SET
                 capability = EXCLUDED.capability,
                 proof_grade = EXCLUDED.proof_grade,
                 evidence_urls = EXCLUDED.evidence_urls,
                 metadata = EXCLUDED.metadata,
                 last_confirmed = now()"#,
        )
        .bind(cap.id)
        .bind(cap.company_id)
        .bind(cap.site_id)
        .bind(&cap.capability)
        .bind(cap.proof_grade.as_str())
        .bind(&evidence_urls)
        .bind(cap.first_seen)
        .bind(cap.last_confirmed)
        .bind(&cap.metadata)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_sites(
        &self,
        company_id: Option<Uuid>,
        region: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<SiteRow>> {
        let (limit, offset) = normalize_company_assets_window(limit, offset);
        match (company_id, region) {
            (Some(company_id), Some(region)) => {
                Ok(sqlx::query_as::<_, SiteRow>(
                    "SELECT * FROM sites WHERE company_id = $3 AND region = $4 ORDER BY name ASC LIMIT $1 OFFSET $2",
                )
                .bind(limit)
                .bind(offset)
                .bind(company_id)
                .bind(region)
                .fetch_all(&self.pool)
                .await?)
            }
            (Some(company_id), None) => {
                Ok(sqlx::query_as::<_, SiteRow>(
                    "SELECT * FROM sites WHERE company_id = $3 ORDER BY name ASC LIMIT $1 OFFSET $2",
                )
                .bind(limit)
                .bind(offset)
                .bind(company_id)
                .fetch_all(&self.pool)
                .await?)
            }
            (None, Some(region)) => {
                Ok(sqlx::query_as::<_, SiteRow>(
                    "SELECT * FROM sites WHERE region = $3 ORDER BY name ASC LIMIT $1 OFFSET $2",
                )
                .bind(limit)
                .bind(offset)
                .bind(region)
                .fetch_all(&self.pool)
                .await?)
            }
            (None, None) => {
                Ok(sqlx::query_as::<_, SiteRow>(
                    "SELECT * FROM sites ORDER BY name ASC LIMIT $1 OFFSET $2",
                )
                .bind(limit)
                .bind(offset)
                .fetch_all(&self.pool)
                .await?)
            }
        }
    }

    pub async fn count_sites(&self, company_id: Option<Uuid>, region: Option<&str>) -> Result<i64> {
        let count: i64 = match (company_id, region) {
            (Some(company_id), Some(region)) => {
                sqlx::query_scalar(
                    "SELECT COUNT(*) FROM sites WHERE company_id = $1 AND region = $2",
                )
                .bind(company_id)
                .bind(region)
                .fetch_one(&self.pool)
                .await?
            }
            (Some(company_id), None) => {
                sqlx::query_scalar("SELECT COUNT(*) FROM sites WHERE company_id = $1")
                    .bind(company_id)
                    .fetch_one(&self.pool)
                    .await?
            }
            (None, Some(region)) => {
                sqlx::query_scalar("SELECT COUNT(*) FROM sites WHERE region = $1")
                    .bind(region)
                    .fetch_one(&self.pool)
                    .await?
            }
            (None, None) => {
                sqlx::query_scalar("SELECT COUNT(*) FROM sites")
                    .fetch_one(&self.pool)
                    .await?
            }
        };
        Ok(count)
    }

    pub async fn list_capabilities(
        &self,
        company_id: Option<Uuid>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<CapabilityRow>> {
        let (limit, offset) = normalize_company_assets_window(limit, offset);
        match company_id {
            Some(company_id) => {
                Ok(sqlx::query_as::<_, CapabilityRow>(
                    "SELECT * FROM capabilities WHERE company_id = $3 ORDER BY capability ASC LIMIT $1 OFFSET $2",
                )
                .bind(limit)
                .bind(offset)
                .bind(company_id)
                .fetch_all(&self.pool)
                .await?)
            }
            None => {
                Ok(sqlx::query_as::<_, CapabilityRow>(
                    "SELECT * FROM capabilities ORDER BY capability ASC LIMIT $1 OFFSET $2",
                )
                .bind(limit)
                .bind(offset)
                .fetch_all(&self.pool)
                .await?)
            }
        }
    }

    pub async fn count_capabilities(&self, company_id: Option<Uuid>) -> Result<i64> {
        let count: i64 = match company_id {
            Some(company_id) => {
                sqlx::query_scalar("SELECT COUNT(*) FROM capabilities WHERE company_id = $1")
                    .bind(company_id)
                    .fetch_one(&self.pool)
                    .await?
            }
            None => {
                sqlx::query_scalar("SELECT COUNT(*) FROM capabilities")
                    .fetch_one(&self.pool)
                    .await?
            }
        };
        Ok(count)
    }

    pub async fn list_certifications(
        &self,
        company_id: Option<Uuid>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<CertificationRow>> {
        let (limit, offset) = normalize_company_assets_window(limit, offset);
        match company_id {
            Some(company_id) => {
                Ok(sqlx::query_as::<_, CertificationRow>(
                    "SELECT * FROM certifications WHERE company_id = $3 ORDER BY standard ASC LIMIT $1 OFFSET $2",
                )
                .bind(limit)
                .bind(offset)
                .bind(company_id)
                .fetch_all(&self.pool)
                .await?)
            }
            None => {
                Ok(sqlx::query_as::<_, CertificationRow>(
                    "SELECT * FROM certifications ORDER BY standard ASC LIMIT $1 OFFSET $2",
                )
                .bind(limit)
                .bind(offset)
                .fetch_all(&self.pool)
                .await?)
            }
        }
    }

    pub async fn count_certifications(&self, company_id: Option<Uuid>) -> Result<i64> {
        let count: i64 = match company_id {
            Some(company_id) => {
                sqlx::query_scalar("SELECT COUNT(*) FROM certifications WHERE company_id = $1")
                    .bind(company_id)
                    .fetch_one(&self.pool)
                    .await?
            }
            None => {
                sqlx::query_scalar("SELECT COUNT(*) FROM certifications")
                    .fetch_one(&self.pool)
                    .await?
            }
        };
        Ok(count)
    }

    pub async fn list_product_families(
        &self,
        company_id: Option<Uuid>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<ProductFamilyRow>> {
        let (limit, offset) = normalize_company_assets_window(limit, offset);
        match company_id {
            Some(company_id) => {
                Ok(sqlx::query_as::<_, ProductFamilyRow>(
                    "SELECT * FROM product_families WHERE company_id = $3 ORDER BY name ASC LIMIT $1 OFFSET $2",
                )
                .bind(limit)
                .bind(offset)
                .bind(company_id)
                .fetch_all(&self.pool)
                .await?)
            }
            None => {
                Ok(sqlx::query_as::<_, ProductFamilyRow>(
                    "SELECT * FROM product_families ORDER BY name ASC LIMIT $1 OFFSET $2",
                )
                .bind(limit)
                .bind(offset)
                .fetch_all(&self.pool)
                .await?)
            }
        }
    }

    pub async fn count_product_families(&self, company_id: Option<Uuid>) -> Result<i64> {
        let count: i64 = match company_id {
            Some(company_id) => {
                sqlx::query_scalar("SELECT COUNT(*) FROM product_families WHERE company_id = $1")
                    .bind(company_id)
                    .fetch_one(&self.pool)
                    .await?
            }
            None => {
                sqlx::query_scalar("SELECT COUNT(*) FROM product_families")
                    .fetch_one(&self.pool)
                    .await?
            }
        };
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_company_assets_window;

    #[test]
    fn test_normalize_company_assets_window_clamps_limit_and_offset() {
        assert_eq!(normalize_company_assets_window(0, -10), (1, 0));
        assert_eq!(normalize_company_assets_window(9999, 12), (500, 12));
    }

    #[test]
    fn test_normalize_company_assets_window_preserves_valid_values() {
        assert_eq!(normalize_company_assets_window(25, 5), (25, 5));
    }
}
