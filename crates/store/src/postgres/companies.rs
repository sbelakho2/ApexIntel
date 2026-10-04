use super::*;
use apex_core::analysis::{
    compare_temporal_windows, fuse_weak_signals, score_competing_hypotheses, source_group_from_url,
    HypothesisInput, SignalFrame,
};
use apex_core::evidence_quality::{assess_evidence_quality, EvidenceItem, EvidenceStance};

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

/// Lower-case SQL comparison values for a requested region, mirroring the web
/// `canonical_region` mapping. A request for "EU" must match stored "eu",
/// "EU" and "Europe" alike, exactly as the in-memory filter did.
fn company_region_aliases(region: &str) -> Vec<String> {
    let trimmed = region.trim();
    let aliases: &[&str] = match trimmed.to_ascii_lowercase().as_str() {
        "" => return vec![String::new()],
        "tn" | "tunisia" => &["tn", "tunisia"],
        "ma" | "morocco" => &["ma", "morocco"],
        "il" | "israel" => &["il", "israel"],
        "eu" | "europe" => &["eu", "europe"],
        "cn" | "china" => &["cn", "china"],
        "us" | "usa" | "united states" => &["us", "usa", "united states"],
        "global" => &["global"],
        "other" => &["other", ""],
        _ => return vec![trimmed.to_ascii_lowercase()],
    };
    aliases.iter().map(|alias| (*alias).to_string()).collect()
}

/// SQL predicate for one risk tier, banding the truncated 0–100 score exactly
/// like the web `risk_tier` helper (a `None` score is "—", never a tier).
/// Unknown tier values match nothing so an arbitrary query string cannot widen
/// the result set.
fn company_tier_predicate(tier: &str) -> Option<&'static str> {
    Some(match tier {
        "T1" => "risk_score IS NOT NULL AND trunc(risk_score * 100)::bigint >= 85",
        "T2" => "risk_score IS NOT NULL AND trunc(risk_score * 100)::bigint >= 70 AND trunc(risk_score * 100)::bigint < 85",
        "T3" => "risk_score IS NOT NULL AND trunc(risk_score * 100)::bigint >= 55 AND trunc(risk_score * 100)::bigint < 70",
        "T4" => "risk_score IS NOT NULL AND trunc(risk_score * 100)::bigint >= 35 AND trunc(risk_score * 100)::bigint < 55",
        "T5" => "risk_score IS NOT NULL AND trunc(risk_score * 100)::bigint < 35",
        "—" => "risk_score IS NULL",
        _ => return None,
    })
}

/// Push the shared list predicates (search, competitor and the web region/tier
/// filters) onto a `WHERE` builder. Kept in one place so the page rows, the
/// summary aggregate and the region breakdown always describe the same set.
fn push_company_list_predicates<'a>(
    qb: &mut QueryBuilder<'a, Postgres>,
    filters: &'a CompanyListFilters,
    region: Option<&'a str>,
    tier: Option<&'a str>,
) {
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
        has_where = true;
    }

    if let Some(region) = region {
        qb.push(if has_where { " AND " } else { " WHERE " });
        qb.push("lower(btrim(coalesce(region, ''))) = ANY(")
            .push_bind(company_region_aliases(region))
            .push(")");
        has_where = true;
    }

    if let Some(tier) = tier {
        qb.push(if has_where { " AND " } else { " WHERE " });
        match company_tier_predicate(tier) {
            Some(predicate) => qb.push(predicate),
            None => qb.push("FALSE"),
        };
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
    let mut evidence_records: Vec<EvidenceItem> = certifications
        .iter()
        .filter_map(|cert| {
            cert.evidence_url.as_ref().map(|url| {
                // Unknown certification status is an unmeasured relevance, not
                // a neutral 0.55 prior.
                let mut record = EvidenceItem::new_optional(
                    cert.status.as_deref().map(|status| {
                        if status.eq_ignore_ascii_case("active") {
                            0.8
                        } else {
                            0.55
                        }
                    }),
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
                // Missing confidence stays missing: the record still counts,
                // but no synthesized 0.6 weight is attributed to it.
                let mut record =
                    EvidenceItem::new_optional(entry.confidence, EvidenceStance::Supports)
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
                EvidenceItem::new_optional(change.confidence, EvidenceStance::Supports)
                    .with_source_url(url.clone())
                    .with_source_type(change.change_type.clone());
            if let Some(detected_at) = change.detected_at.or(change.created_at) {
                record = record.with_observed_at(detected_at);
            }
            record
        })
    }));
    let evidence_quality = assess_evidence_quality(&evidence_records, &[], now);

    let temporal_delta = company_temporal_delta(recent_changes, dossier_entries);

    let mut signal_frames = Vec::new();
    for change in recent_changes {
        signal_frames.push(SignalFrame {
            id: change.id.to_string(),
            theme: change.change_type.clone(),
            category: change.field_name.clone(),
            region: None,
            entity: Some(change.company_id.to_string()),
            confidence: change.confidence,
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
            confidence: entry.confidence,
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
            heuristic_prior: 0.45,
        },
        HypothesisInput {
            hypothesis: "Compliance or certification stress".to_string(),
            support_score: certification_pressure,
            contradiction_score: capability_signal * 0.30,
            heuristic_prior: 0.35,
        },
        HypothesisInput {
            hypothesis: "Competitive repositioning".to_string(),
            support_score: (change_pressure * 0.65 + correlated_signals.len() as f64 * 0.1)
                .clamp(0.0, 1.0),
            contradiction_score: certification_pressure * 0.20,
            heuristic_prior: 0.40,
        },
    ]);

    let summary = format!(
        "Evidence posture is {} ({:.2}) with {} independent sources. Activity is {} and {} correlated weak-signal cluster(s) remain under watch.",
        evidence_quality.quality_label(),
        evidence_quality.composite_score(),
        evidence_quality.corpus.independent_origin_count,
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

    /// Coverage of priority companies (competitors) by recent observations,
    /// returned as `(total, covered)`. A priority company is covered when at
    /// least one observation linked to it falls inside the window. Feeds the
    /// coverage matrix's priority-company dimension (audit P1-9).
    pub async fn priority_company_coverage(&self, since: DateTime<Utc>) -> Result<(i64, i64)> {
        let row: (i64, i64) = sqlx::query_as(
            r#"SELECT
                   COUNT(*) FILTER (WHERE COALESCE(c.is_competitor, FALSE))::bigint AS total,
                   COUNT(*) FILTER (
                       WHERE COALESCE(c.is_competitor, FALSE)
                         AND EXISTS (
                             SELECT 1 FROM observations o
                             WHERE o.entity_id = c.id AND o.ts_utc >= $1
                         )
                   )::bigint AS covered
               FROM companies c"#,
        )
        .bind(since)
        .fetch_one(&self.pool)
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

    /// Batch-load the fields the recipe applicability gate needs: region,
    /// country code, company type and the real industry tags.
    ///
    /// The context must carry the *actual* metadata: `company_type` is not an
    /// industry, and `country_code` is not the region. A missing row yields no
    /// context (a restricted recipe then does not apply).
    pub async fn get_recipe_entity_contexts(
        &self,
        ids: &[Uuid],
    ) -> Result<Vec<RecipeEntityContextRow>> {
        if ids.is_empty() {
            return Ok(vec![]);
        }
        let rows = sqlx::query_as::<_, RecipeEntityContextRow>(
            "SELECT id, region, country_code, company_type, \
                    COALESCE(industry_tags, ARRAY[]::TEXT[]) AS industry_tags \
             FROM companies WHERE id = ANY($1)",
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
                    is_competitor, metadata, created_at, updated_at
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
                    is_competitor, metadata, created_at, updated_at
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
                    is_competitor, metadata, created_at, updated_at
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

    /// Every company matching `filters` in the requested order. Pages through
    /// `list_companies` (whose order always ends in `id ASC`, so offset pages
    /// are stable) because a single list call clamps to `MAX_LIST_LIMIT`.
    pub async fn list_all_companies_matching(
        &self,
        filters: &CompanyListFilters,
        order_by: Option<CompanyOrderBy>,
        desc: bool,
    ) -> Result<Vec<CompanyRow>> {
        let mut rows = Vec::new();
        loop {
            let offset = rows.len() as i64;
            let page = self
                .list_companies(filters, order_by, desc, MAX_LIST_LIMIT, offset)
                .await?;
            let page_len = page.len() as i64;
            rows.extend(page);
            if page_len < MAX_LIST_LIMIT {
                return Ok(rows);
            }
        }
    }

    /// Every company, read in keyset pages (list calls clamp to
    /// `MAX_LIST_LIMIT`, so "all" must page rather than ask for a big limit).
    pub async fn list_all_companies(&self) -> Result<Vec<CompanyRow>> {
        let mut rows = Vec::new();
        let mut after = None;
        loop {
            let page = self.list_companies_after(after, MAX_LIST_LIMIT).await?;
            let page_len = page.len() as i64;
            after = page.last().map(|row| row.id);
            rows.extend(page);
            if page_len < MAX_LIST_LIMIT {
                return Ok(rows);
            }
        }
    }

    /// Keyset page ordered by `id`: `WHERE id > after_id ORDER BY id ASC`.
    ///
    /// Exports use this instead of `OFFSET` paging: offset over a non-unique
    /// order can duplicate or skip rows when the table changes between pages.
    pub async fn list_companies_after(
        &self,
        after_id: Option<Uuid>,
        limit: i64,
    ) -> Result<Vec<CompanyRow>> {
        let limit = clamp_limit(limit);
        let rows = sqlx::query_as::<_, CompanyRow>(
            "SELECT id, name, legal_name, domain, country_code, region, company_type,
                    industry_tags, employee_estimate, revenue_estimate_usd,
                    risk_score, threat_score, overlap_score, strategic_relevance,
                    is_competitor, metadata, created_at, updated_at
             FROM companies
             WHERE ($1::uuid IS NULL OR id > $1)
             ORDER BY id ASC
             LIMIT $2",
        )
        .bind(after_id)
        .bind(limit)
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

    /// Filtered company page for the web list. Region/tier predicates and
    /// LIMIT/OFFSET are applied in SQL; the whole matched set is never loaded.
    pub async fn list_companies_filtered(
        &self,
        filters: &CompanyListFilters,
        region: Option<&str>,
        tier: Option<&str>,
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
                    is_competitor, metadata, created_at, updated_at
             FROM companies",
        );
        push_company_list_predicates(&mut qb, filters, region, tier);

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

    /// Whole-filtered-set stats for the company list page as
    /// `(total, competitor_count, high_risk_count, avg_risk)`.
    ///
    /// A single filtered aggregate — no `GROUP BY` over the whole table. The
    /// score banding matches the web list exactly (the 0–100 score is
    /// truncated before banding).
    pub async fn summarize_companies_filtered(
        &self,
        filters: &CompanyListFilters,
        region: Option<&str>,
        tier: Option<&str>,
    ) -> Result<(i64, i64, i64, i64)> {
        let mut qb: QueryBuilder<Postgres> = QueryBuilder::new(
            r#"SELECT
                   COUNT(*)::bigint AS total,
                   COUNT(*) FILTER (
                       WHERE metadata -> 'is_competitor' = 'true'::jsonb
                   )::bigint AS competitor_count,
                   COUNT(*) FILTER (
                       WHERE risk_score IS NOT NULL
                         AND trunc(risk_score * 100)::bigint >= 70
                   )::bigint AS high_risk_count,
                   COALESCE(SUM(trunc(risk_score * 100)::bigint) FILTER (
                       WHERE risk_score IS NOT NULL
                         AND trunc(risk_score * 100)::bigint > 0
                   ), 0)::bigint AS risk_sum,
                   COUNT(*) FILTER (
                       WHERE risk_score IS NOT NULL
                         AND trunc(risk_score * 100)::bigint > 0
                   )::bigint AS risk_count
               FROM companies"#,
        );
        push_company_list_predicates(&mut qb, filters, region, tier);

        let (total, competitor_count, high_risk_count, risk_sum, risk_count): (
            i64,
            i64,
            i64,
            i64,
            i64,
        ) = qb.build_query_as().fetch_one(&self.pool).await?;
        let avg_risk = if risk_count == 0 {
            0
        } else {
            risk_sum / risk_count
        };
        Ok((total, competitor_count, high_risk_count, avg_risk))
    }

    /// Counts of matching companies per raw stored region, for the list page's
    /// coverage donut. Grouped over the filtered set only.
    pub async fn count_companies_by_region_filtered(
        &self,
        filters: &CompanyListFilters,
        region: Option<&str>,
        tier: Option<&str>,
    ) -> Result<Vec<(String, i64)>> {
        let mut qb: QueryBuilder<Postgres> =
            QueryBuilder::new("SELECT COALESCE(region, ''), COUNT(*)::bigint FROM companies");
        push_company_list_predicates(&mut qb, filters, region, tier);
        qb.push(" GROUP BY 1 ORDER BY 2 DESC, 1 ASC");

        let rows: Vec<(String, i64)> = qb.build_query_as().fetch_all(&self.pool).await?;
        Ok(rows)
    }

    /// Warning counts for a bounded set of entity ids (the current list page).
    /// Mirrors `get_warnings_by_entity_ids` visibility: soft-deleted warnings
    /// never count.
    pub async fn get_warning_counts_for_entity_ids(
        &self,
        entity_ids: &[Uuid],
    ) -> Result<Vec<(Uuid, i64)>> {
        if entity_ids.is_empty() {
            return Ok(vec![]);
        }
        let rows = sqlx::query_as::<_, (Uuid, i64)>(
            r#"SELECT u.entity_id, COUNT(*)::BIGINT
               FROM warnings w
               CROSS JOIN LATERAL unnest(w.entity_ids) AS u(entity_id)
               WHERE w.deleted_at IS NULL
                 AND w.entity_ids IS NOT NULL
                 AND u.entity_id = ANY($1)
               GROUP BY u.entity_id"#,
        )
        .bind(entity_ids)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Insight counts for a bounded set of entity ids (the current list page),
    /// applying the same visibility rules as `get_insights_by_entity_ids`
    /// (non-empty title, no internal insight types).
    pub async fn get_insight_counts_for_entity_ids(
        &self,
        entity_ids: &[Uuid],
    ) -> Result<Vec<(Uuid, i64)>> {
        if entity_ids.is_empty() {
            return Ok(vec![]);
        }
        let rows = sqlx::query_as::<_, (Uuid, i64)>(
            r#"SELECT u.entity_id, COUNT(*)::BIGINT
               FROM insights i
               CROSS JOIN LATERAL unnest(i.entity_ids) AS u(entity_id)
               WHERE i.entity_ids IS NOT NULL
                 AND u.entity_id = ANY($1)
                 AND btrim(COALESCE(i.title, '')) <> ''
                 AND (i.insight_type IS NULL OR (lower(i.insight_type) NOT LIKE 'llm_%'
                      AND lower(i.insight_type) <> 'bias_mitigation'
                      AND lower(i.insight_type) <> 'hypothesis_ach'))
               GROUP BY u.entity_id"#,
        )
        .bind(entity_ids)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Real number of warnings related to one entity across all time (the
    /// detail page count must not be the length of a 50-row page). Soft-deleted
    /// warnings are excluded, matching the related-warnings list.
    pub async fn count_warnings_for_entity(&self, entity_id: Uuid) -> Result<i64> {
        let row: (i64,) = sqlx::query_as(
            "SELECT COUNT(*)::BIGINT FROM warnings \
             WHERE deleted_at IS NULL AND entity_ids IS NOT NULL AND entity_ids && $1",
        )
        .bind(vec![entity_id])
        .fetch_one(&self.pool)
        .await?;
        Ok(row.0)
    }

    /// Real number of visible insights related to one entity across all time
    /// (the detail page count must not be the length of a 50-row page).
    /// Internal insight types and blank-title rows are excluded, matching
    /// [`Self::get_insights_by_entity_ids`].
    pub async fn count_insights_for_entity(&self, entity_id: Uuid) -> Result<i64> {
        let mut qb: QueryBuilder<Postgres> = QueryBuilder::new(
            "SELECT COUNT(*)::BIGINT FROM insights \
             WHERE entity_ids IS NOT NULL AND entity_ids && ",
        );
        qb.push_bind(vec![entity_id])
            .push(" AND btrim(COALESCE(title, '')) <> '' AND ");
        append_internal_insight_filter_sql_clause(&mut qb, "");
        let row: (i64,) = qb.build_query_as().fetch_one(&self.pool).await?;
        Ok(row.0)
    }

    /// The company's stored narrative/description (`companies.narrative`,
    /// added by migration 045). `None` when unset or unknown; never falls back
    /// to the legal name, which is not a description.
    pub async fn get_company_narrative(&self, id: Uuid) -> Result<Option<String>> {
        let row: Option<(Option<String>,)> =
            sqlx::query_as("SELECT narrative FROM companies WHERE id = $1")
                .bind(id)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.and_then(|(narrative,)| narrative))
    }

    pub async fn get_company_dossier(&self, company_id: Uuid) -> Result<Option<CompanyDossier>> {
        let company = match self.get_company(company_id).await? {
            Some(company) => company,
            None => return Ok(None),
        };
        let sites = self.get_sites_for_company(company_id).await?;
        let capabilities = self.list_capabilities(Some(company_id), 200, 0).await?;
        let certifications = self.get_certifications_for_company(company_id).await?;
        let product_families = self.list_product_families(Some(company_id), 200, 0).await?;
        let edges = self.get_edges_from(company_id, "company").await?;
        let dossier_entries = self
            .get_dossier_entries("company", company_id, None, 200)
            .await?;
        let recent_changes = self.get_company_changes(company_id, 50).await?;
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
    use super::{
        company_region_aliases, company_tier_predicate, normalize_company_domain,
        normalize_company_window,
    };

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

    #[test]
    fn test_company_region_aliases_mirror_canonical_region() {
        assert_eq!(company_region_aliases("EU"), vec!["eu", "europe"]);
        assert_eq!(
            company_region_aliases("usa"),
            vec!["us", "usa", "united states"]
        );
        assert_eq!(company_region_aliases(" Tunisia "), vec!["tn", "tunisia"]);
        assert_eq!(company_region_aliases("Other"), vec!["other", ""]);
        assert_eq!(company_region_aliases("Atlantis"), vec!["atlantis"]);
    }

    #[test]
    fn test_company_tier_predicate_bands_and_rejects_unknown() {
        assert!(company_tier_predicate("T1").unwrap().contains(">= 85"));
        assert!(company_tier_predicate("T5").unwrap().contains("< 35"));
        assert_eq!(company_tier_predicate("—").unwrap(), "risk_score IS NULL");
        assert!(company_tier_predicate("bogus").is_none());
    }
}
