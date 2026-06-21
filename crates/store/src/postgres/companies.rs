use super::*;
use apex_core::analysis::{
    assess_evidence_quality, compare_temporal_windows, fuse_weak_signals,
    score_competing_hypotheses, source_group_from_url, EvidenceRecord, EvidenceStance,
    HypothesisInput, SignalFrame,
};

fn normalize_company_window(limit: i64, offset: i64) -> (i64, i64) {
    (clamp_limit(limit), offset.max(0))
}

fn normalize_company_domain(domain: &str) -> Option<String> {
    let lowered = domain.trim().to_ascii_lowercase();
    let normalized = lowered.strip_prefix("www.").unwrap_or(&lowered);
    if normalized.is_empty() {
        None
    } else {
        Some(normalized.to_string())
    }
}

fn company_temporal_delta(
    recent_changes: &[CompanyChangeRow],
    dossier_entries: &[DossierEntryRow],
) -> apex_core::analysis::TemporalDelta {
    let mut timestamps: Vec<DateTime<Utc>> = recent_changes
        .iter()
        .filter_map(|change| change.detected_at.or(change.created_at))
        .collect();
    timestamps.extend(dossier_entries.iter().filter_map(|entry| entry.created_at));

    if timestamps.is_empty() {
        return compare_temporal_windows(0.0, 0.0);
    }

    timestamps.sort();
    let midpoint = timestamps[0]
        + chrono::Duration::seconds(
            (timestamps[timestamps.len() - 1] - timestamps[0]).num_seconds() / 2,
        );
    let early = timestamps.iter().filter(|ts| **ts <= midpoint).count() as f64;
    let late = timestamps.iter().filter(|ts| **ts > midpoint).count() as f64;
    compare_temporal_windows(late, early)
}

fn build_company_dossier_analysis(
    capabilities: &[CapabilityRow],
    certifications: &[CertificationRow],
    dossier_entries: &[DossierEntryRow],
    recent_changes: &[CompanyChangeRow],
) -> DossierAnalysis {
    let now = Utc::now();
    let mut evidence_records: Vec<EvidenceRecord> = certifications
        .iter()
        .filter_map(|cert| {
            cert.evidence_url.as_ref().map(|url| {
                let mut record = EvidenceRecord::new(
                    cert.status
                        .as_deref()
                        .map(|status| {
                            if status.eq_ignore_ascii_case("active") {
                                0.8
                            } else {
                                0.55
                            }
                        })
                        .unwrap_or(0.55),
                    EvidenceStance::Supports,
                )
                .with_source_url(url.clone())
                .with_source_type("certification");
                if let Some(updated_at) = cert.updated_at.or(cert.created_at) {
                    record = record.with_observed_at(updated_at);
                }
                record
            })
        })
        .collect();
    evidence_records.extend(dossier_entries.iter().flat_map(|entry| {
        entry
            .source_urls
            .clone()
            .unwrap_or_default()
            .into_iter()
            .map(move |url| {
                let mut record =
                    EvidenceRecord::new(entry.confidence.unwrap_or(0.6), EvidenceStance::Supports)
                        .with_source_url(url)
                        .with_source_type(entry.category.clone());
                if let Some(created_at) = entry.created_at {
                    record = record.with_observed_at(created_at);
                }
                record
            })
    }));
    evidence_records.extend(recent_changes.iter().filter_map(|change| {
        change.source_url.as_ref().map(|url| {
            let mut record =
                EvidenceRecord::new(change.confidence.unwrap_or(0.6), EvidenceStance::Supports)
                    .with_source_url(url.clone())
                    .with_source_type(change.change_type.clone());
            if let Some(detected_at) = change.detected_at.or(change.created_at) {
                record = record.with_observed_at(detected_at);
            }
            record
        })
    }));
    let evidence_quality = assess_evidence_quality(&evidence_records, now);

    let temporal_delta = company_temporal_delta(recent_changes, dossier_entries);

    let mut signal_frames = Vec::new();
    for change in recent_changes {
        signal_frames.push(SignalFrame {
            id: change.id.to_string(),
            theme: change.change_type.clone(),
            category: change.field_name.clone(),
            region: None,
            entity: Some(change.company_id.to_string()),
            confidence: change.confidence.unwrap_or(0.55),
            impact: 0.6,
            source_group: change.source_url.as_deref().and_then(source_group_from_url),
        });
    }
    for entry in dossier_entries {
        signal_frames.push(SignalFrame {
            id: entry.id.to_string(),
            theme: entry.category.clone(),
            category: Some(entry.category.clone()),
            region: None,
            entity: Some(entry.entity_id.to_string()),
            confidence: entry.confidence.unwrap_or(0.55),
            impact: 0.5,
            source_group: entry
                .source_urls
                .as_deref()
                .and_then(|urls| urls.iter().find_map(|url| source_group_from_url(url))),
        });
    }
    let correlated_signals = fuse_weak_signals(&signal_frames);

    let capability_signal = (capabilities.len() as f64 / 10.0).clamp(0.0, 1.0);
    let compliance_signal = certifications
        .iter()
        .filter(|cert| {
            cert.status
                .as_deref()
                .map(|status| !status.eq_ignore_ascii_case("active"))
                .unwrap_or(false)
        })
        .count() as f64;
    let certification_pressure =
        (compliance_signal / certifications.len().max(1) as f64).clamp(0.0, 1.0);
    let change_pressure = (recent_changes.len() as f64 / 8.0).clamp(0.0, 1.0);
    let competing_hypotheses = score_competing_hypotheses(&[
        HypothesisInput {
            hypothesis: "Expansion or capacity buildout".to_string(),
            support_score: (capability_signal + change_pressure * 0.35).clamp(0.0, 1.0),
            contradiction_score: certification_pressure * 0.45,
            prior: 0.45,
        },
        HypothesisInput {
            hypothesis: "Compliance or certification stress".to_string(),
            support_score: certification_pressure,
            contradiction_score: capability_signal * 0.30,
            prior: 0.35,
        },
        HypothesisInput {
            hypothesis: "Competitive repositioning".to_string(),
            support_score: (change_pressure * 0.65 + correlated_signals.len() as f64 * 0.1)
                .clamp(0.0, 1.0),
            contradiction_score: certification_pressure * 0.20,
            prior: 0.40,
        },
    ]);

    let summary = format!(
        "Evidence posture is {} ({:.2}) with {} independent sources. Activity is {} and {} correlated weak-signal cluster(s) remain under watch.",
        evidence_quality.quality_label,
        evidence_quality.overall_score,
        evidence_quality.independent_source_count,
        temporal_delta.label,
        correlated_signals.len(),
    );

    DossierAnalysis {
        evidence_quality,
        temporal_delta,
        correlated_signals,
        competing_hypotheses,
        summary,
    }
}

impl PgStore {
    pub async fn insert_company(&self, c: &Company) -> Result<()> {
        validate_tags(&c.industry_tags)?;
        sqlx::query(
            r#"INSERT INTO companies
               (id, name, legal_name, domain, country_code, region, company_type,
                industry_tags, employee_estimate, revenue_estimate_usd,
                risk_score, threat_score, overlap_score, strategic_relevance,
                is_competitor, metadata, created_at, updated_at)
               VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18)
               ON CONFLICT (id) DO UPDATE SET
                 name = EXCLUDED.name,
                 legal_name = EXCLUDED.legal_name,
                 domain = EXCLUDED.domain,
                 country_code = EXCLUDED.country_code,
                 region = EXCLUDED.region,
                 company_type = EXCLUDED.company_type,
                 industry_tags = EXCLUDED.industry_tags,
                 employee_estimate = EXCLUDED.employee_estimate,
                 revenue_estimate_usd = EXCLUDED.revenue_estimate_usd,
                 risk_score = EXCLUDED.risk_score,
                 threat_score = EXCLUDED.threat_score,
                 overlap_score = EXCLUDED.overlap_score,
                 strategic_relevance = EXCLUDED.strategic_relevance,
                 is_competitor = EXCLUDED.is_competitor,
                 metadata = EXCLUDED.metadata,
                 updated_at = now()"#,
        )
        .bind(c.id)
        .bind(&c.name)
        .bind(&c.legal_name)
        .bind(&c.domain)
        .bind(&c.country_code)
        .bind(&c.region)
        .bind(c.company_type.as_str())
        .bind(&c.industry_tags)
        .bind(c.employee_estimate)
        .bind(c.revenue_estimate_usd)
        .bind(c.risk_score)
        .bind(c.threat_score)
        .bind(c.overlap_score)
        .bind(c.strategic_relevance)
        .bind(c.is_competitor.unwrap_or(false))
        .bind(&c.metadata)
        .bind(c.created_at)
        .bind(c.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_company(&self, id: Uuid) -> Result<Option<CompanyRow>> {
        let row = sqlx::query_as::<_, CompanyRow>(
            "SELECT id, name, legal_name, domain, country_code, region, company_type,
                    industry_tags, employee_estimate, revenue_estimate_usd,
                    risk_score, threat_score, overlap_score, strategic_relevance,
                    is_competitor, metadata, created_at, updated_at
             FROM companies WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn get_company_by_domain(&self, domain: &str) -> Result<Option<CompanyRow>> {
        let Some(normalized) = normalize_company_domain(domain) else {
            return Ok(None);
        };

        let row = sqlx::query_as::<_, CompanyRow>(
            "SELECT id, name, legal_name, domain, country_code, region, company_type,
                    industry_tags, employee_estimate, revenue_estimate_usd,
                    risk_score, threat_score, overlap_score, strategic_relevance,
                    is_competitor, metadata, created_at, updated_at
             FROM companies
             WHERE lower(regexp_replace(coalesce(domain, ''), '^www\\.', '')) = $1
             ORDER BY updated_at DESC
             LIMIT 1",
        )
        .bind(normalized)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn get_company_by_name_ci(&self, name: &str) -> Result<Option<CompanyRow>> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Ok(None);
        }

        let row = sqlx::query_as::<_, CompanyRow>(
            "SELECT id, name, legal_name, domain, country_code, region, company_type,
                    industry_tags, employee_estimate, revenue_estimate_usd,
                    risk_score, threat_score, overlap_score, strategic_relevance,
                    is_competitor, metadata, created_at, updated_at
             FROM companies
             WHERE lower(name) = lower($1) OR lower(coalesce(legal_name, '')) = lower($1)
             ORDER BY updated_at DESC
             LIMIT 1",
        )
        .bind(trimmed)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn get_company_names_by_ids(
        &self,
        ids: &[Uuid],
    ) -> Result<Vec<(Uuid, String, Option<String>, Option<String>)>> {
        if ids.is_empty() {
            return Ok(vec![]);
        }
        let rows: Vec<(Uuid, String, Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT id, name, region, company_type FROM companies WHERE id = ANY($1)",
        )
        .bind(ids)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Batch-load the geographic / classification fields needed to apply
    /// go-to-market targeting during insight ranking.
    ///
    /// Returns, per company id: `country_code` (ISO alpha-2), `region`,
    /// `company_type`, and an `is_competitor` flag. The flag is true when
    /// *either* the canonical `is_competitor` column *or* the
    /// `metadata->>'is_competitor'` attribute is set, so a competitor flagged
    /// through either path is never missed (and therefore never geo-penalised).
    /// The caller uses these to weight demand-side opportunities by sales market
    /// while leaving competitors and upstream suppliers globally monitored.
    pub async fn get_company_geo_by_ids(
        &self,
        ids: &[Uuid],
    ) -> Result<Vec<(Uuid, Option<String>, Option<String>, Option<String>, bool)>> {
        if ids.is_empty() {
            return Ok(vec![]);
        }
        let rows: Vec<(Uuid, Option<String>, Option<String>, Option<String>, bool)> =
            sqlx::query_as(
                "SELECT id, country_code, region, company_type,
                        (COALESCE(is_competitor, false)
                         OR COALESCE((metadata->>'is_competitor')::boolean, false))
                 FROM companies WHERE id = ANY($1)",
            )
            .bind(ids)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows)
    }

    pub async fn list_companies_by_region(&self, region: &str) -> Result<Vec<CompanyRow>> {
        let rows = sqlx::query_as::<_, CompanyRow>(
            "SELECT id, name, legal_name, domain, country_code, region, company_type,
                    industry_tags, employee_estimate, revenue_estimate_usd,
                    risk_score, threat_score, overlap_score, strategic_relevance,
                    metadata, created_at, updated_at
             FROM companies WHERE region = $1 ORDER BY name",
        )
        .bind(region)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn list_companies_by_type(&self, company_type: &str) -> Result<Vec<CompanyRow>> {
        let rows = sqlx::query_as::<_, CompanyRow>(
            "SELECT id, name, legal_name, domain, country_code, region, company_type,
                    industry_tags, employee_estimate, revenue_estimate_usd,
                    risk_score, threat_score, overlap_score, strategic_relevance,
                    metadata, created_at, updated_at
             FROM companies WHERE company_type = $1 ORDER BY name",
        )
        .bind(company_type)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn list_companies(
        &self,
        filters: &CompanyListFilters,
        order_by: Option<CompanyOrderBy>,
        desc: bool,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<CompanyRow>> {
        let (limit, offset) = normalize_company_window(limit, offset);
        let mut qb: QueryBuilder<Postgres> = QueryBuilder::new(
            "SELECT id, name, legal_name, domain, country_code, region, company_type,
                    industry_tags, employee_estimate, revenue_estimate_usd,
                    risk_score, threat_score, overlap_score, strategic_relevance,
                    metadata, created_at, updated_at
             FROM companies",
        );

        let mut has_where = false;
        if !filters.regions.is_empty() {
            qb.push(if has_where { " AND " } else { " WHERE " });
            qb.push("region = ANY(")
                .push_bind(&filters.regions)
                .push(")");
            has_where = true;
        }

        if let Some(search) = &filters.search {
            qb.push(if has_where { " AND " } else { " WHERE " });
            qb.push("(name ILIKE ")
                .push_bind(ilike_pattern(search))
                .push(")");
            has_where = true;
        }

        if let Some(is_competitor) = filters.is_competitor {
            qb.push(if has_where { " AND " } else { " WHERE " });
            qb.push("COALESCE((metadata->>'is_competitor')::boolean, false) = ")
                .push_bind(is_competitor);
        }

        let order_by = order_by.unwrap_or(CompanyOrderBy::UpdatedAt);
        qb.push(" ORDER BY ");
        match order_by {
            CompanyOrderBy::Name => qb.push("name"),
            CompanyOrderBy::Region => qb.push("region"),
            CompanyOrderBy::ThreatScore => qb.push("threat_score"),
            CompanyOrderBy::UpdatedAt => qb.push("updated_at"),
        };
        qb.push(if desc { " DESC" } else { " ASC" });
        qb.push(", id ASC");
        qb.push(" LIMIT ").push_bind(limit);
        qb.push(" OFFSET ").push_bind(offset);

        let rows = qb
            .build_query_as::<CompanyRow>()
            .fetch_all(&self.pool)
            .await?;
        Ok(rows)
    }

    pub async fn count_companies(&self, filters: &CompanyListFilters) -> Result<i64> {
        let mut qb: QueryBuilder<Postgres> = QueryBuilder::new("SELECT COUNT(*) FROM companies");
        let mut has_where = false;

        if !filters.regions.is_empty() {
            qb.push(if has_where { " AND " } else { " WHERE " });
            qb.push("region = ANY(")
                .push_bind(&filters.regions)
                .push(")");
            has_where = true;
        }

        if let Some(search) = &filters.search {
            qb.push(if has_where { " AND " } else { " WHERE " });
            qb.push("(name ILIKE ")
                .push_bind(ilike_pattern(search))
                .push(")");
            has_where = true;
        }

        if let Some(is_competitor) = filters.is_competitor {
            qb.push(if has_where { " AND " } else { " WHERE " });
            qb.push("COALESCE((metadata->>'is_competitor')::boolean, false) = ")
                .push_bind(is_competitor);
        }

        let row: (i64,) = qb.build_query_as().fetch_one(&self.pool).await?;
        Ok(row.0)
    }

    pub async fn get_company_dossier(&self, company_id: Uuid) -> Result<Option<CompanyDossier>> {
        let company = match self.get_company(company_id).await? {
            Some(company) => company,
            None => return Ok(None),
        };
        let sites = self
            .get_sites_for_company(company_id)
            .await
            .unwrap_or_default();
        let capabilities = self
            .list_capabilities(Some(company_id), 200, 0)
            .await
            .unwrap_or_default();
        let certifications = self
            .get_certifications_for_company(company_id)
            .await
            .unwrap_or_default();
        let product_families = self
            .list_product_families(Some(company_id), 200, 0)
            .await
            .unwrap_or_default();
        let edges = self
            .get_edges_from(company_id, "company")
            .await
            .unwrap_or_default();
        let dossier_entries = self
            .get_dossier_entries("company", company_id, None, 200)
            .await
            .unwrap_or_default();
        let recent_changes = self
            .get_company_changes(company_id, 50)
            .await
            .unwrap_or_default();
        let analysis = build_company_dossier_analysis(
            &capabilities,
            &certifications,
            &dossier_entries,
            &recent_changes,
        );
        Ok(Some(CompanyDossier {
            company,
            sites,
            capabilities,
            certifications,
            product_families,
            edges,
            dossier_entries,
            recent_changes,
            analysis,
        }))
    }

    /// Lightweight index of (company_id, lowercase_name) pairs for crawl-time
    /// entity linking.  Includes `legal_name` and `domain` as additional
    /// matching keys so that alternative names and website references also
    /// resolve to the correct entity.
    pub async fn list_entity_name_index(&self) -> Result<Vec<(Uuid, String)>> {
        let rows: Vec<(Uuid, String, Option<String>, Option<String>)> =
            sqlx::query_as("SELECT id, name, legal_name, domain FROM companies")
                .fetch_all(&self.pool)
                .await?;

        let mut index: Vec<(Uuid, String)> = Vec::with_capacity(rows.len() * 2);
        for (id, name, legal_name, domain) in rows {
            let n = name.trim().to_ascii_lowercase();
            if !n.is_empty() {
                index.push((id, n));
            }
            if let Some(ln) = legal_name {
                let ln = ln.trim().to_ascii_lowercase();
                if !ln.is_empty() {
                    index.push((id, ln));
                }
            }
            if let Some(d) = domain {
                let d = d.trim().to_ascii_lowercase();
                let d = d.strip_prefix("www.").unwrap_or(&d);
                if !d.is_empty() {
                    index.push((id, d.to_string()));
                }
            }
        }
        Ok(index)
    }
}

#[cfg(test)]
mod tests {
    use super::{normalize_company_domain, normalize_company_window};

    #[test]
    fn test_normalize_company_window_clamps_limit_and_offset() {
        assert_eq!(normalize_company_window(0, -3), (1, 0));
        assert_eq!(normalize_company_window(9999, 5), (500, 5));
    }

    #[test]
    fn test_normalize_company_domain_trims_and_strips_www() {
        assert_eq!(
            normalize_company_domain("  WWW.Example.com "),
            Some("example.com".to_string())
        );
        assert_eq!(normalize_company_domain("   "), None);
    }
}
