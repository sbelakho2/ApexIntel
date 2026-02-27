use anyhow::Result;
use chrono::{DateTime, NaiveDate, Utc};
use sqlx::postgres::{PgPool, PgPoolOptions};
use sqlx::{Postgres, QueryBuilder};
use uuid::Uuid;

use apex_core::entities::*;
use apex_core::validation::normalize_url;

/// Escape ILIKE wildcard characters (`%` and `_`) in user input,
/// then wrap with `%…%` for a contains-match pattern.
fn ilike_pattern(raw: &str) -> String {
    let escaped = raw.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
    format!("%{}%", escaped)
}

const MAX_LIST_LIMIT: i64 = 500;

fn clamp_limit(limit: i64) -> i64 {
    limit.clamp(1, MAX_LIST_LIMIT)
}

fn normalize_url_vec(urls: &[String]) -> Vec<String> {
    urls.iter()
        .filter_map(|u| normalize_url(u))
        .collect()
}

fn validate_tags(tags: &[String]) -> Result<()> {
    for tag in tags {
        if tag.chars().count() > 64 {
            return Err(anyhow::anyhow!("tag too long (max 64 chars)"));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Default)]
pub struct WarningListFilters {
    pub regions: Vec<String>,
    pub severities: Vec<String>,
    pub warning_types: Vec<String>,
    pub acknowledged: Option<bool>,
    pub date_from: Option<DateTime<Utc>>,
    pub date_to: Option<DateTime<Utc>>,
    pub search: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct InsightListFilters {
    pub regions: Vec<String>,
    pub date_from: Option<DateTime<Utc>>,
    pub date_to: Option<DateTime<Utc>>,
    pub search: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct CompanyListFilters {
    pub regions: Vec<String>,
    pub search: Option<String>,
    pub is_competitor: Option<bool>,
}

#[derive(Debug, Clone, Default)]
pub struct PersonListFilters {
    pub regions: Vec<String>,
    pub roles: Vec<String>,
    pub search: Option<String>,
    pub min_priority: Option<f64>,
}

#[derive(Debug, Clone, Copy)]
pub enum WarningOrderBy {
    CreatedAt,
    Severity,
    WarningType,
}

#[derive(Debug, Clone, Copy)]
pub enum CompanyOrderBy {
    Name,
    Region,
    ThreatScore,
    UpdatedAt,
}

#[derive(Debug, Clone, Copy)]
pub enum PersonOrderBy {
    Name,
    Priority,
    Region,
    UpdatedAt,
}


/// PostgreSQL connection pool wrapper with all CRUD operations.
#[derive(Clone)]
pub struct PgStore {
    pub pool: PgPool,
}

impl PgStore {
    pub async fn connect(database_url: &str) -> Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(20)
            .acquire_timeout(std::time::Duration::from_secs(5))
            .idle_timeout(std::time::Duration::from_secs(600))
            .max_lifetime(std::time::Duration::from_secs(1800))
            .connect(database_url)
            .await?;
        Ok(Self { pool })
    }

    pub fn from_pool(pool: PgPool) -> Self {
        Self { pool }
    }

    // ─── Companies ───────────────────────────────────────────────────────

    pub async fn insert_company(&self, c: &Company) -> Result<()> {
        validate_tags(&c.industry_tags)?;
        sqlx::query(
            r#"INSERT INTO companies
               (id, name, legal_name, domain, country_code, region, company_type,
                industry_tags, employee_estimate, revenue_estimate_usd,
                risk_score, threat_score, overlap_score, strategic_relevance,
                metadata, created_at, updated_at)
               VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17)
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
                    metadata, created_at, updated_at
             FROM companies WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
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
        let limit = clamp_limit(limit);
        let offset = offset.max(0);
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
            qb.push("region = ANY(").push_bind(&filters.regions).push(")");
            has_where = true;
        }

        if let Some(search) = &filters.search {
            qb.push(if has_where { " AND " } else { " WHERE " });
            qb.push("(name ILIKE ").push_bind(ilike_pattern(search)).push(")");
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
        // Tiebreaker for deterministic pagination when sort column has duplicates
        qb.push(", id ASC");
        qb.push(" LIMIT ").push_bind(limit);
        qb.push(" OFFSET ").push_bind(offset);

        let rows = qb.build_query_as::<CompanyRow>().fetch_all(&self.pool).await?;
        Ok(rows)
    }

    pub async fn count_companies(&self, filters: &CompanyListFilters) -> Result<i64> {
        let mut qb: QueryBuilder<Postgres> = QueryBuilder::new("SELECT COUNT(*) FROM companies");
        let mut has_where = false;

        if !filters.regions.is_empty() {
            qb.push(if has_where { " AND " } else { " WHERE " });
            qb.push("region = ANY(").push_bind(&filters.regions).push(")");
            has_where = true;
        }

        if let Some(search) = &filters.search {
            qb.push(if has_where { " AND " } else { " WHERE " });
            qb.push("(name ILIKE ").push_bind(ilike_pattern(search)).push(")");
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

    // ─── Warnings ───────────────────────────────────────────────────────

    pub async fn list_warnings(
        &self,
        filters: &WarningListFilters,
        order_by: Option<WarningOrderBy>,
        desc: bool,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<WarningRow>> {
        // Query relies on indexes for ts_utc, region, and severity to stay performant.
        let mut qb: QueryBuilder<Postgres> = QueryBuilder::new(
            "SELECT id, recipe_code, warning_type, title, description, severity, region,
                    source_urls, entity_ids, confidence, ts_utc, acknowledged,
                    acknowledged_by, acknowledged_at, acknowledged_note, created_at, updated_at
             FROM warnings",
        );

        let mut has_where = false;
        if !filters.regions.is_empty() {
            qb.push(if has_where { " AND " } else { " WHERE " });
            qb.push("region = ANY(").push_bind(&filters.regions).push(")");
            has_where = true;
        }

        if !filters.severities.is_empty() {
            qb.push(if has_where { " AND " } else { " WHERE " });
            qb.push("severity = ANY(").push_bind(&filters.severities).push(")");
            has_where = true;
        }

        if !filters.warning_types.is_empty() {
            qb.push(if has_where { " AND " } else { " WHERE " });
            qb.push("warning_type = ANY(").push_bind(&filters.warning_types).push(")");
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
            let _ = has_where;
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
        // Tiebreaker for deterministic pagination when sort column has duplicates
        qb.push(", id ASC");
        qb.push(" LIMIT ").push_bind(limit);
        qb.push(" OFFSET ").push_bind(offset);

        let rows = qb.build_query_as::<WarningRow>().fetch_all(&self.pool).await?;
        Ok(rows)
    }

    pub async fn count_warnings(&self, filters: &WarningListFilters) -> Result<i64> {
        let mut qb: QueryBuilder<Postgres> = QueryBuilder::new("SELECT COUNT(*) FROM warnings");
        let mut has_where = false;

        if !filters.regions.is_empty() {
            qb.push(if has_where { " AND " } else { " WHERE " });
            qb.push("region = ANY(").push_bind(&filters.regions).push(")");
            has_where = true;
        }

        if !filters.severities.is_empty() {
            qb.push(if has_where { " AND " } else { " WHERE " });
            qb.push("severity = ANY(").push_bind(&filters.severities).push(")");
            has_where = true;
        }

        if !filters.warning_types.is_empty() {
            qb.push(if has_where { " AND " } else { " WHERE " });
            qb.push("warning_type = ANY(").push_bind(&filters.warning_types).push(")");
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
            let _ = has_where;
        }

        let row: (i64,) = qb.build_query_as().fetch_one(&self.pool).await?;
        Ok(row.0)
    }

    /// Acknowledge a warning. Returns:
    /// - Ok(Some(true))  if the warning was found and newly acknowledged
    /// - Ok(Some(false)) if the warning exists but was already acknowledged
    /// - Ok(None)        if the warning was not found
    pub async fn acknowledge_warning(
        &self,
        id: Uuid,
        user_id: &str,
        note: Option<&str>,
    ) -> Result<Option<bool>> {
        let res = sqlx::query(
            r#"UPDATE warnings
               SET acknowledged = TRUE,
                   acknowledged_by = $2,
                   acknowledged_at = now(),
                   acknowledged_note = $3,
                   updated_at = now()
               WHERE id = $1 AND acknowledged = FALSE"#,
        )
        .bind(id)
        .bind(user_id)
        .bind(note)
        .execute(&self.pool)
        .await?;
        if res.rows_affected() == 1 {
            return Ok(Some(true));
        }
        // Distinguish "not found" from "already acknowledged"
        let exists: (bool,) =
            sqlx::query_as("SELECT EXISTS(SELECT 1 FROM warnings WHERE id = $1)")
                .bind(id)
                .fetch_one(&self.pool)
                .await?;
        if exists.0 {
            Ok(Some(false)) // exists but already acknowledged
        } else {
            Ok(None) // not found
        }
    }

    // ─── Insights ───────────────────────────────────────────────────────

    pub async fn list_insights(
        &self,
        filters: &InsightListFilters,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<InsightRow>> {
        // Query relies on indexes for created_at and region to stay performant.
        let mut qb: QueryBuilder<Postgres> = QueryBuilder::new(
            "SELECT id, title, summary, insight_type, region, confidence,
                    evidence_urls, entity_ids, tags, created_at, updated_at
             FROM insights",
        );

        let mut has_where = false;
        if !filters.regions.is_empty() {
            qb.push(if has_where { " AND " } else { " WHERE " });
            qb.push("region = ANY(").push_bind(&filters.regions).push(")");
            has_where = true;
        }

        if let Some(date_from) = filters.date_from {
            qb.push(if has_where { " AND " } else { " WHERE " });
            qb.push("created_at >= ").push_bind(date_from);
            has_where = true;
        }

        if let Some(date_to) = filters.date_to {
            qb.push(if has_where { " AND " } else { " WHERE " });
            qb.push("created_at <= ").push_bind(date_to);
            has_where = true;
        }

        if let Some(search) = &filters.search {
            qb.push(if has_where { " AND " } else { " WHERE " });
            let pattern = ilike_pattern(search);
            qb.push("(title ILIKE ")
                .push_bind(pattern.clone())
                .push(" OR summary ILIKE ")
                .push_bind(pattern)
                .push(")");
            let _ = has_where;
        }

        // Tiebreaker for deterministic pagination when sort column has duplicates
        qb.push(" ORDER BY created_at DESC, id ASC ");
        qb.push(" LIMIT ").push_bind(limit);
        qb.push(" OFFSET ").push_bind(offset);

        let rows = qb.build_query_as::<InsightRow>().fetch_all(&self.pool).await?;
        Ok(rows)
    }

    pub async fn count_insights(&self, filters: &InsightListFilters) -> Result<i64> {
        let mut qb: QueryBuilder<Postgres> = QueryBuilder::new("SELECT COUNT(*) FROM insights");
        let mut has_where = false;

        if !filters.regions.is_empty() {
            qb.push(if has_where { " AND " } else { " WHERE " });
            qb.push("region = ANY(").push_bind(&filters.regions).push(")");
            has_where = true;
        }

        if let Some(date_from) = filters.date_from {
            qb.push(if has_where { " AND " } else { " WHERE " });
            qb.push("created_at >= ").push_bind(date_from);
            has_where = true;
        }

        if let Some(date_to) = filters.date_to {
            qb.push(if has_where { " AND " } else { " WHERE " });
            qb.push("created_at <= ").push_bind(date_to);
            has_where = true;
        }

        if let Some(search) = &filters.search {
            qb.push(if has_where { " AND " } else { " WHERE " });
            let pattern = ilike_pattern(search);
            qb.push("(title ILIKE ")
                .push_bind(pattern.clone())
                .push(" OR summary ILIKE ")
                .push_bind(pattern)
                .push(")");
            let _ = has_where;
        }

        let row: (i64,) = qb.build_query_as().fetch_one(&self.pool).await?;
        Ok(row.0)
    }

    // ─── Sites ───────────────────────────────────────────────────────────

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

    // ─── Persons (POI) ──────────────────────────────────────────────────

    pub async fn insert_person(&self, p: &Person) -> Result<()> {
        let pv_json = serde_json::to_value(&p.priority_vector)?;
        sqlx::query(
            r#"INSERT INTO persons
               (id, name, name_ar, name_fr, primary_org_id, "current_role",
                role_family, region, country_code, priority_vector,
                influence_score, metadata, created_at, updated_at)
               VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)
               ON CONFLICT (id) DO UPDATE SET
                 name = EXCLUDED.name,
                 name_ar = EXCLUDED.name_ar,
                 name_fr = EXCLUDED.name_fr,
                 primary_org_id = EXCLUDED.primary_org_id,
                 "current_role" = EXCLUDED."current_role",
                 role_family = EXCLUDED.role_family,
                 region = EXCLUDED.region,
                 country_code = EXCLUDED.country_code,
                 priority_vector = EXCLUDED.priority_vector,
                 influence_score = EXCLUDED.influence_score,
                 metadata = EXCLUDED.metadata,
                 updated_at = now()"#,
        )
        .bind(p.id)
        .bind(&p.name)
        .bind(&p.name_ar)
        .bind(&p.name_fr)
        .bind(p.primary_org_id)
        .bind(&p.current_role)
        .bind(p.role_family.as_str())
        .bind(&p.region)
        .bind(&p.country_code)
        .bind(&pv_json)
        .bind(p.influence_score)
        .bind(&p.metadata)
        .bind(p.created_at)
        .bind(p.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_person(&self, id: Uuid) -> Result<Option<PersonRow>> {
        let row = sqlx::query_as::<_, PersonRow>(
            "SELECT id, name, name_ar, name_fr, primary_org_id, \"current_role\",
                    role_family, region, country_code, priority_vector,
                    influence_score, metadata, created_at, updated_at
             FROM persons WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn list_persons_by_org(&self, org_id: Uuid) -> Result<Vec<PersonRow>> {
        let rows = sqlx::query_as::<_, PersonRow>(
            "SELECT id, name, name_ar, name_fr, primary_org_id, \"current_role\",
                    role_family, region, country_code, priority_vector,
                    influence_score, metadata, created_at, updated_at
             FROM persons WHERE primary_org_id = $1 ORDER BY name",
        )
        .bind(org_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn count_persons(&self, filters: &PersonListFilters) -> Result<i64> {
        let mut qb = QueryBuilder::new(
            "SELECT COUNT(*) FROM persons p LEFT JOIN companies c ON p.primary_org_id = c.id",
        );
        let mut has_where = false;

        if !filters.regions.is_empty() {
            let regions: Vec<String> = filters.regions.iter().map(|r| r.to_lowercase()).collect();
            if !has_where {
                qb.push(" WHERE ");
                has_where = true;
            }
            qb.push("LOWER(COALESCE(p.region, '')) = ANY(");
            qb.push_bind(regions);
            qb.push(")");
        }

        if !filters.roles.is_empty() {
            let roles: Vec<String> = filters.roles.iter().map(|r| r.to_lowercase()).collect();
            if !has_where {
                qb.push(" WHERE ");
                has_where = true;
            } else {
                qb.push(" AND ");
            }
            qb.push("LOWER(COALESCE(p.role_family, p.\"current_role\", '')) = ANY(");
            qb.push_bind(roles);
            qb.push(")");
        }

        if let Some(min_priority) = filters.min_priority {
            if !has_where {
                qb.push(" WHERE ");
                has_where = true;
            } else {
                qb.push(" AND ");
            }
            qb.push("COALESCE(p.influence_score, 0) >= ");
            qb.push_bind(min_priority);
        }

        if let Some(search) = &filters.search {
            let pattern = ilike_pattern(search);
            if !has_where {
                qb.push(" WHERE ");
            } else {
                qb.push(" AND ");
            }
            qb.push("(p.name ILIKE ");
            qb.push_bind(pattern.clone());
            qb.push(" OR c.name ILIKE ");
            qb.push_bind(pattern);
            qb.push(")");
        }

        let query = qb.build_query_as::<(i64,)>();
        let (count,) = query.fetch_one(&self.pool).await?;
        Ok(count)
    }

    pub async fn list_persons(
        &self,
        filters: &PersonListFilters,
        order_by: Option<PersonOrderBy>,
        desc: bool,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<PersonListRow>> {
        let limit = clamp_limit(limit);
        let mut qb = QueryBuilder::new(
            "SELECT p.id,
                    p.name,
                    COALESCE(p.\"current_role\", p.role_family, 'Unknown') AS role,
                    COALESCE(c.name, 'Independent') AS organization,
                    COALESCE(p.region, '') AS region,
                    COALESCE(p.influence_score, 0) AS priority_score,
                    COALESCE(p.metadata->>'engagement_status', 'untracked') AS engagement_status,
                    COALESCE(p.updated_at, p.created_at, now()) AS updated_at
             FROM persons p
             LEFT JOIN companies c ON p.primary_org_id = c.id",
        );
        let mut has_where = false;

        if !filters.regions.is_empty() {
            let regions: Vec<String> = filters.regions.iter().map(|r| r.to_lowercase()).collect();
            if !has_where {
                qb.push(" WHERE ");
                has_where = true;
            }
            qb.push("LOWER(COALESCE(p.region, '')) = ANY(");
            qb.push_bind(regions);
            qb.push(")");
        }

        if !filters.roles.is_empty() {
            let roles: Vec<String> = filters.roles.iter().map(|r| r.to_lowercase()).collect();
            if !has_where {
                qb.push(" WHERE ");
                has_where = true;
            } else {
                qb.push(" AND ");
            }
            qb.push("LOWER(COALESCE(p.role_family, p.\"current_role\", '')) = ANY(");
            qb.push_bind(roles);
            qb.push(")");
        }

        if let Some(min_priority) = filters.min_priority {
            if !has_where {
                qb.push(" WHERE ");
                has_where = true;
            } else {
                qb.push(" AND ");
            }
            qb.push("COALESCE(p.influence_score, 0) >= ");
            qb.push_bind(min_priority);
        }

        if let Some(search) = &filters.search {
            let pattern = ilike_pattern(search);
            if !has_where {
                qb.push(" WHERE ");
            } else {
                qb.push(" AND ");
            }
            qb.push("(p.name ILIKE ");
            qb.push_bind(pattern.clone());
            qb.push(" OR c.name ILIKE ");
            qb.push_bind(pattern);
            qb.push(")");
        }

        let order_clause = match order_by.unwrap_or(PersonOrderBy::UpdatedAt) {
            PersonOrderBy::Name => "p.name",
            PersonOrderBy::Priority => "COALESCE(p.influence_score, 0)",
            PersonOrderBy::Region => "COALESCE(p.region, '')",
            PersonOrderBy::UpdatedAt => "COALESCE(p.updated_at, p.created_at)",
        };
        qb.push(" ORDER BY ");
        qb.push(order_clause);
        if desc {
            qb.push(" DESC");
        }
        qb.push(" LIMIT ");
        qb.push_bind(limit);
        qb.push(" OFFSET ");
        qb.push_bind(offset.max(0));

        let query = qb.build_query_as::<PersonListRow>();
        let rows = query.fetch_all(&self.pool).await?;
        Ok(rows)
    }

    // ─── Observations ────────────────────────────────────────────────────

    pub async fn insert_observation(&self, o: &Observation) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO observations
               (id, observation_type, entity_id, entity_type, ts_utc,
                value, provenance, confidence, created_at)
               VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)
               ON CONFLICT (id) DO NOTHING"#,
        )
        .bind(o.id)
        .bind(o.observation_type.as_str())
        .bind(o.entity_id)
        .bind(&o.entity_type)
        .bind(o.ts_utc)
        .bind(&o.value)
        .bind(&o.provenance)
        .bind(o.confidence)
        .bind(o.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_observations_by_entity(
        &self,
        entity_id: Uuid,
        limit: i64,
    ) -> Result<Vec<ObservationRow>> {
        let rows = sqlx::query_as::<_, ObservationRow>(
            "SELECT id, observation_type, entity_id, entity_type, ts_utc,
                    value, provenance, confidence, created_at
             FROM observations
             WHERE entity_id = $1
             ORDER BY ts_utc DESC
             LIMIT $2",
        )
        .bind(entity_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn get_observations_by_type(
        &self,
        obs_type: &str,
        since: DateTime<Utc>,
        limit: i64,
    ) -> Result<Vec<ObservationRow>> {
        let rows = sqlx::query_as::<_, ObservationRow>(
            "SELECT id, observation_type, entity_id, entity_type, ts_utc,
                    value, provenance, confidence, created_at
             FROM observations
             WHERE observation_type = $1 AND ts_utc >= $2
             ORDER BY ts_utc DESC
             LIMIT $3",
        )
        .bind(obs_type)
        .bind(since)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    // ─── Graph Edges ─────────────────────────────────────────────────────

    pub async fn upsert_edge(&self, e: &GraphEdge) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO graph_edges
               (id, source_id, source_type, target_id, target_type,
                edge_type, weight, confidence, evidence_ids, metadata,
                first_seen, last_seen)
               VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)
               ON CONFLICT (source_id, source_type, target_id, target_type, edge_type)
               DO UPDATE SET
                 weight = EXCLUDED.weight,
                 confidence = EXCLUDED.confidence,
                 evidence_ids = EXCLUDED.evidence_ids,
                 metadata = EXCLUDED.metadata,
                 last_seen = now()"#,
        )
        .bind(e.id)
        .bind(e.source_id)
        .bind(&e.source_type)
        .bind(e.target_id)
        .bind(&e.target_type)
        .bind(e.edge_type.as_str())
        .bind(e.weight)
        .bind(e.confidence)
        .bind(&e.evidence_ids)
        .bind(&e.metadata)
        .bind(e.first_seen)
        .bind(e.last_seen)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_edges_from(
        &self,
        source_id: Uuid,
        source_type: &str,
    ) -> Result<Vec<EdgeRow>> {
        let rows = sqlx::query_as::<_, EdgeRow>(
            "SELECT id, source_id, source_type, target_id, target_type,
                    edge_type, weight, confidence, evidence_ids, metadata,
                    first_seen, last_seen
             FROM graph_edges
             WHERE source_id = $1 AND source_type = $2
             ORDER BY weight DESC",
        )
        .bind(source_id)
        .bind(source_type)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn get_edges_to(
        &self,
        target_id: Uuid,
        target_type: &str,
    ) -> Result<Vec<EdgeRow>> {
        let rows = sqlx::query_as::<_, EdgeRow>(
            "SELECT id, source_id, source_type, target_id, target_type,
                    edge_type, weight, confidence, evidence_ids, metadata,
                    first_seen, last_seen
             FROM graph_edges
             WHERE target_id = $1 AND target_type = $2
             ORDER BY weight DESC",
        )
        .bind(target_id)
        .bind(target_type)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// List all graph edges (up to limit)
    pub async fn list_all_edges(&self, limit: u32) -> Result<Vec<EdgeRow>> {
        let rows = sqlx::query_as::<_, EdgeRow>(
            "SELECT id, source_id, source_type, target_id, target_type,
                    edge_type, weight, confidence, evidence_ids, metadata,
                    first_seen, last_seen
             FROM graph_edges
             ORDER BY weight DESC NULLS LAST, last_seen DESC NULLS LAST
             LIMIT $1",
        )
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Count total graph edges
    pub async fn count_edges(&self) -> Result<i64> {
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM graph_edges")
            .fetch_one(&self.pool)
            .await?;
        Ok(row.0)
    }

    /// Get recipe signal statistics from warnings (aggregated by recipe_code)
    pub async fn get_recipe_stats(&self) -> Result<Vec<RecipeStatRow>> {
        let rows = sqlx::query_as::<_, RecipeStatRow>(
            r#"SELECT 
                 recipe_code,
                 COUNT(*) as fired_count,
                 MAX(created_at) as last_fired,
                 MIN(created_at) as first_fired,
                 COUNT(*) FILTER (WHERE NOT acknowledged) as active_count
               FROM warnings
               WHERE recipe_code IS NOT NULL
               GROUP BY recipe_code
               ORDER BY fired_count DESC"#,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    // ─── Certifications ──────────────────────────────────────────────────

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

    // ─── POI Artifacts ───────────────────────────────────────────────────

    pub async fn insert_poi_artifact(&self, a: &PoiArtifact) -> Result<()> {
        let prov_json = serde_json::to_value(&a.provenance)?;
        sqlx::query(
            r#"INSERT INTO poi_artifacts
               (id, person_id, artifact_type, title, content_summary,
                url, source_domain, language, topics, sentiment_score,
                key_phrases, ts_utc, provenance, metadata, created_at)
               VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15)
               ON CONFLICT (id) DO NOTHING"#,
        )
        .bind(a.id)
        .bind(a.person_id)
        .bind(a.artifact_type.as_str())
        .bind(&a.title)
        .bind(&a.content_summary)
        .bind(&a.url)
        .bind(&a.source_domain)
        .bind(&a.language)
        .bind(&a.topics)
        .bind(a.sentiment_score)
        .bind(&a.key_phrases)
        .bind(a.ts_utc)
        .bind(&prov_json)
        .bind(&a.metadata)
        .bind(a.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_artifacts_for_person(
        &self,
        person_id: Uuid,
        limit: i64,
    ) -> Result<Vec<ArtifactRow>> {
        let limit = clamp_limit(limit);
        let rows = sqlx::query_as::<_, ArtifactRow>(
            "SELECT id, person_id, artifact_type, title, content_summary,
                    url, source_domain, language, topics, sentiment_score,
                    key_phrases, ts_utc, provenance, metadata, created_at
             FROM poi_artifacts
             WHERE person_id = $1
             ORDER BY ts_utc DESC
             LIMIT $2",
        )
        .bind(person_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    // ─── Capabilities ────────────────────────────────────────────────────

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

    // ─── Logistics Nodes ─────────────────────────────────────────────────

    pub async fn insert_logistics_node(&self, n: &LogisticsNode) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO logistics_nodes
               (id, name, node_type, country_code, lat, lon, metadata)
               VALUES ($1,$2,$3,$4,$5,$6,$7)
               ON CONFLICT (id) DO NOTHING"#,
        )
        .bind(n.id)
        .bind(&n.name)
        .bind(&n.node_type)
        .bind(&n.country_code)
        .bind(n.lat)
        .bind(n.lon)
        .bind(&n.metadata)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // ─── Worker Pipeline Statistics ──────────────────────────────────────

    /// Get crawl statistics for a given time window.
    /// Used by the worker to build CrawlStageResult from actual data.
    pub async fn get_crawl_stats(&self, _since: DateTime<Utc>) -> Result<CrawlStats> {
        // Query crawl_logs for the time window
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
        .bind(_since)
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(r) => Ok(CrawlStats {
                sources_attempted: r.sources_attempted.unwrap_or(0) as u64,
                sources_succeeded: r.sources_succeeded.unwrap_or(0) as u64,
                sources_failed: r.sources_failed.unwrap_or(0) as u64,
                new_observations: r.new_observations.unwrap_or(0) as u64,
                changed_pages: r.changed_pages.unwrap_or(0) as u64,
                bytes_fetched: r.bytes_fetched.unwrap_or(0) as u64,
                errors: vec![],
            }),
            None => Ok(CrawlStats::default()),
        }
    }

    /// Get mining statistics for a given time window.
    pub async fn get_mining_stats(&self, _since: DateTime<Utc>) -> Result<MiningStats> {
        // Query pattern_candidates and recipes for the time window
        let candidates_found: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM pattern_candidates WHERE created_at >= $1",
        )
        .bind(_since)
        .fetch_one(&self.pool)
        .await
        .unwrap_or(0);

        let candidates_passed: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM pattern_candidates WHERE created_at >= $1 AND passed_gates = true",
        )
        .bind(_since)
        .fetch_one(&self.pool)
        .await
        .unwrap_or(0);

        let recipes_staged: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM recipes WHERE created_at >= $1 AND lifecycle_state = 'staged'",
        )
        .bind(_since)
        .fetch_one(&self.pool)
        .await
        .unwrap_or(0);

        Ok(MiningStats {
            candidates_found: candidates_found as u64,
            candidates_passed_gates: candidates_passed as u64,
            hypotheses_generated: candidates_passed as u64, // Assume 1:1 mapping
            recipes_staged: recipes_staged as u64,
            errors: vec![],
        })
    }

    /// Get POI statistics for a given time window.
    pub async fn get_poi_stats(&self, _since: DateTime<Utc>) -> Result<PoiStats> {
        let profiles_scanned: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM persons WHERE updated_at >= $1",
        )
        .bind(_since)
        .fetch_one(&self.pool)
        .await
        .unwrap_or(0);

        let profiles_updated: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM persons WHERE updated_at >= $1 AND updated_at != created_at",
        )
        .bind(_since)
        .fetch_one(&self.pool)
        .await
        .unwrap_or(0);

        let new_pois: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM persons WHERE created_at >= $1",
        )
        .bind(_since)
        .fetch_one(&self.pool)
        .await
        .unwrap_or(0);

        Ok(PoiStats {
            profiles_scanned: profiles_scanned as u64,
            profiles_updated: profiles_updated as u64,
            new_pois_discovered: new_pois as u64,
            role_changes_detected: 0, // TODO: track role changes
            errors: vec![],
        })
    }

    /// Get drift statistics from the feature store.
    pub async fn get_drift_stats(&self) -> Result<DriftStats> {
        // Placeholder - drift detection is computed at runtime from feature_store
        Ok(DriftStats::default())
    }

    /// Get staged recipes ready for promotion evaluation.
    pub async fn get_staged_recipes_for_promotion(&self) -> Result<Vec<StagedRecipeRow>> {
        let rows = sqlx::query_as::<_, StagedRecipeRow>(
            r#"SELECT 
                id, name, 
                COALESCE(precision_observed, 0.0) as precision_observed,
                COALESCE(recall_observed, 0.0) as recall_observed,
                COALESCE(false_positive_rate, 0.0) as false_positive_rate,
                COALESCE(sample_size, 0) as sample_size,
                EXTRACT(DAY FROM (NOW() - created_at))::INT as days_in_staging,
                created_at
               FROM recipes
               WHERE lifecycle_state = 'staged'
               ORDER BY created_at ASC"#,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Get production recipes for deprecation evaluation.
    pub async fn get_production_recipes_for_deprecation(&self) -> Result<Vec<ProductionRecipeRow>> {
        let rows = sqlx::query_as::<_, ProductionRecipeRow>(
            r#"SELECT 
                id, name,
                COALESCE(precision_observed, 0.0) as precision_current,
                COALESCE(precision_baseline, 0.0) as precision_baseline,
                COALESCE(false_positive_rate, 0.0) as false_positive_rate,
                COALESCE(fpr_baseline, 0.0) as fpr_baseline,
                COALESCE(warnings_generated_last_week, 0) as warnings_generated_last_week,
                last_triggered_at,
                EXTRACT(DAY FROM (NOW() - COALESCE(last_triggered_at, created_at)))::INT as days_inactive,
                created_at
               FROM recipes
               WHERE lifecycle_state = 'production'
               ORDER BY last_triggered_at DESC NULLS LAST"#,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Get weekly summary statistics for memo generation.
    pub async fn get_weekly_summary_stats(&self, since: DateTime<Utc>) -> Result<WeeklySummaryStats> {
        let companies_monitored: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM companies")
            .fetch_one(&self.pool)
            .await
            .unwrap_or(0);

        let persons_tracked: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM persons")
            .fetch_one(&self.pool)
            .await
            .unwrap_or(0);

        let warnings_generated: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM warnings WHERE created_at >= $1",
        )
        .bind(since)
        .fetch_one(&self.pool)
        .await
        .unwrap_or(0);

        let insights_produced: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM insights WHERE created_at >= $1",
        )
        .bind(since)
        .fetch_one(&self.pool)
        .await
        .unwrap_or(0);

        let recipes_in_production: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM recipes WHERE lifecycle_state = 'production'",
        )
        .fetch_one(&self.pool)
        .await
        .unwrap_or(0);

        Ok(WeeklySummaryStats {
            companies_monitored: companies_monitored as u64,
            persons_tracked: persons_tracked as u64,
            warnings_generated: warnings_generated as u64,
            insights_produced: insights_produced as u64,
            recipes_in_production: recipes_in_production as u64,
            top_regions: vec![],
            notable_events: vec![],
        })
    }

    // ─── Schema Migration ────────────────────────────────────────────────

    /// Run the full schema creation. Idempotent via IF NOT EXISTS.
    pub async fn run_migrations(&self) -> Result<()> {
        // Use versioned migrations from ./migrations so schema changes are tracked over time.
        sqlx::migrate!("./migrations").run(&self.pool).await?;
        Ok(())
    }
}

// ─── Row Types (sqlx::FromRow) ──────────────────────────────────────────────

// ─── Worker Pipeline Stats Types ─────────────────────────────────────────────

/// Stats returned by get_crawl_stats for worker pipeline integration.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct CrawlStats {
    pub sources_attempted: u64,
    pub sources_succeeded: u64,
    pub sources_failed: u64,
    pub new_observations: u64,
    pub changed_pages: u64,
    pub bytes_fetched: u64,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct CrawlStatsRow {
    sources_attempted: Option<i64>,
    sources_succeeded: Option<i64>,
    sources_failed: Option<i64>,
    new_observations: Option<i64>,
    changed_pages: Option<i64>,
    bytes_fetched: Option<i64>,
}

/// Stats for the mining pipeline stage.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct MiningStats {
    pub candidates_found: u64,
    pub candidates_passed_gates: u64,
    pub hypotheses_generated: u64,
    pub recipes_staged: u64,
    pub errors: Vec<String>,
}

/// Stats for the POI refresh pipeline stage.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct PoiStats {
    pub profiles_scanned: u64,
    pub profiles_updated: u64,
    pub new_pois_discovered: u64,
    pub role_changes_detected: u64,
    pub errors: Vec<String>,
}

/// Stats for the drift check pipeline stage.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct DriftStats {
    pub features_checked: u64,
    pub features_drifted: u64,
    pub drift_scores: Vec<(String, f64)>,
    pub alerts_raised: u64,
    pub errors: Vec<String>,
}

/// Row type for staged recipe queries.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct StagedRecipeRow {
    pub id: Uuid,
    pub name: String,
    pub precision_observed: f64,
    pub recall_observed: f64,
    pub false_positive_rate: f64,
    pub sample_size: i32,
    pub days_in_staging: i32,
    pub created_at: DateTime<Utc>,
}

/// Row type for production recipe queries.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct ProductionRecipeRow {
    pub id: Uuid,
    pub name: String,
    pub precision_current: f64,
    pub precision_baseline: f64,
    pub false_positive_rate: f64,
    pub fpr_baseline: f64,
    pub warnings_generated_last_week: i64,
    pub last_triggered_at: Option<DateTime<Utc>>,
    pub days_inactive: i32,
    pub created_at: DateTime<Utc>,
}

/// Weekly summary statistics for memo generation.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct WeeklySummaryStats {
    pub companies_monitored: u64,
    pub persons_tracked: u64,
    pub warnings_generated: u64,
    pub insights_produced: u64,
    pub recipes_in_production: u64,
    pub top_regions: Vec<String>,
    pub notable_events: Vec<String>,
}

// ─── Entity Row Types ────────────────────────────────────────────────────────

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct CompanyRow {
    pub id: Uuid,
    pub name: String,
    pub legal_name: Option<String>,
    pub domain: Option<String>,
    pub country_code: Option<String>,
    pub region: Option<String>,
    pub company_type: Option<String>,
    pub industry_tags: Option<Vec<String>>,
    pub employee_estimate: Option<i32>,
    pub revenue_estimate_usd: Option<i64>,
    pub risk_score: Option<f64>,
    pub threat_score: Option<f64>,
    pub overlap_score: Option<f64>,
    pub strategic_relevance: Option<f64>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct SiteRow {
    pub id: Uuid,
    pub company_id: Option<Uuid>,
    pub name: String,
    pub address: Option<String>,
    pub city: Option<String>,
    pub country_code: Option<String>,
    pub region: Option<String>,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub site_type: Option<String>,
    pub capabilities: Option<Vec<String>>,
    pub certifications: Option<Vec<String>>,
    pub employee_estimate: Option<i32>,
    pub free_zone: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct PersonRow {
    pub id: Uuid,
    pub name: String,
    pub name_ar: Option<String>,
    pub name_fr: Option<String>,
    pub primary_org_id: Option<Uuid>,
    pub current_role: Option<String>,
    pub role_family: Option<String>,
    pub region: Option<String>,
    pub country_code: Option<String>,
    pub priority_vector: Option<serde_json::Value>,
    pub influence_score: Option<f64>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct PersonListRow {
    pub id: Uuid,
    pub name: String,
    pub role: String,
    pub organization: String,
    pub region: String,
    pub priority_score: f64,
    pub engagement_status: String,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct WarningRow {
    pub id: Uuid,
    pub recipe_code: Option<String>,
    pub warning_type: String,
    pub title: String,
    pub description: Option<String>,
    pub severity: String,
    pub region: Option<String>,
    pub source_urls: Option<Vec<String>>,
    pub entity_ids: Option<Vec<Uuid>>,
    pub confidence: Option<f64>,
    pub ts_utc: DateTime<Utc>,
    pub acknowledged: bool,
    pub acknowledged_by: Option<String>,
    pub acknowledged_at: Option<DateTime<Utc>>,
    pub acknowledged_note: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct InsightRow {
    pub id: Uuid,
    pub title: String,
    pub summary: String,
    pub insight_type: Option<String>,
    pub region: Option<String>,
    pub confidence: Option<f64>,
    pub evidence_urls: Option<Vec<String>>,
    pub entity_ids: Option<Vec<Uuid>>,
    pub tags: Option<Vec<String>>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct ObservationRow {
    pub id: Uuid,
    pub observation_type: String,
    pub entity_id: Option<Uuid>,
    pub entity_type: Option<String>,
    pub ts_utc: DateTime<Utc>,
    pub value: serde_json::Value,
    pub provenance: serde_json::Value,
    pub confidence: Option<f64>,
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct EdgeRow {
    pub id: Uuid,
    pub source_id: Uuid,
    pub source_type: String,
    pub target_id: Uuid,
    pub target_type: String,
    pub edge_type: String,
    pub weight: Option<f64>,
    pub confidence: Option<f64>,
    pub evidence_ids: Option<Vec<Uuid>>,
    pub metadata: Option<serde_json::Value>,
    pub first_seen: Option<DateTime<Utc>>,
    pub last_seen: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct RecipeStatRow {
    pub recipe_code: String,
    pub fired_count: i64,
    pub last_fired: Option<DateTime<Utc>>,
    pub first_fired: Option<DateTime<Utc>>,
    pub active_count: i64,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct CertificationRow {
    pub id: Uuid,
    pub company_id: Option<Uuid>,
    pub site_id: Option<Uuid>,
    pub standard: String,
    pub status: Option<String>,
    pub issuing_body: Option<String>,
    pub valid_from: Option<NaiveDate>,
    pub valid_until: Option<NaiveDate>,
    pub scope: Option<String>,
    pub evidence_url: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct ArtifactRow {
    pub id: Uuid,
    pub person_id: Option<Uuid>,
    pub artifact_type: String,
    pub title: Option<String>,
    pub content_summary: Option<String>,
    pub url: String,
    pub source_domain: Option<String>,
    pub language: Option<String>,
    pub topics: Option<Vec<String>>,
    pub sentiment_score: Option<f64>,
    pub key_phrases: Option<Vec<String>>,
    pub ts_utc: DateTime<Utc>,
    pub provenance: serde_json::Value,
    pub metadata: Option<serde_json::Value>,
    pub created_at: Option<DateTime<Utc>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// These tests verify query construction and row type structure.
    /// Full integration tests require a running PostgreSQL instance.

    #[test]
    fn test_company_row_fields() {
        // Verify CompanyRow has all expected fields via construction
        let row = CompanyRow {
            id: Uuid::new_v4(),
            name: "Starz Electronics".into(),
            legal_name: Some("Starz Electronics SARL".into()),
            domain: Some("starz-electronics.com".into()),
            country_code: Some("TN".into()),
            region: Some("TN".into()),
            company_type: Some("ems".into()),
            industry_tags: Some(vec!["automotive".into(), "industrial".into()]),
            employee_estimate: Some(500),
            revenue_estimate_usd: Some(50_000_000),
            risk_score: Some(0.3),
            threat_score: Some(0.1),
            overlap_score: Some(0.8),
            strategic_relevance: Some(0.9),
            metadata: Some(serde_json::json!({})),
            created_at: Some(Utc::now()),
            updated_at: Some(Utc::now()),
        };
        assert_eq!(row.name, "Starz Electronics");
        assert_eq!(row.country_code.as_deref(), Some("TN"));

        // Verify serialization
        let json = serde_json::to_value(&row).unwrap();
        assert_eq!(json["name"], "Starz Electronics");
        assert!(json["industry_tags"].is_array());
    }

    #[test]
    fn test_observation_row_fields() {
        let row = ObservationRow {
            id: Uuid::new_v4(),
            observation_type: "JobPost".into(),
            entity_id: Some(Uuid::new_v4()),
            entity_type: Some("company".into()),
            ts_utc: Utc::now(),
            value: serde_json::json!({"role": "SQE", "role_family": "quality"}),
            provenance: serde_json::json!({"url": "https://jobs.example.com"}),
            confidence: Some(0.95),
            created_at: Some(Utc::now()),
        };
        assert_eq!(row.observation_type, "JobPost");
        assert_eq!(row.value["role"], "SQE");
    }

    #[test]
    fn test_edge_row_fields() {
        let row = EdgeRow {
            id: Uuid::new_v4(),
            source_id: Uuid::new_v4(),
            source_type: "company".into(),
            target_id: Uuid::new_v4(),
            target_type: "company".into(),
            edge_type: "competitor".into(),
            weight: Some(0.8),
            confidence: Some(0.9),
            evidence_ids: Some(vec![Uuid::new_v4()]),
            metadata: Some(serde_json::json!({})),
            first_seen: Some(Utc::now()),
            last_seen: Some(Utc::now()),
        };
        assert_eq!(row.edge_type, "competitor");
        assert!(row.weight.unwrap() > 0.5);
    }

    #[test]
    fn test_person_row_fields() {
        let row = PersonRow {
            id: Uuid::new_v4(),
            name: "Ahmed Ben Ali".into(),
            name_ar: Some("أحمد بن علي".into()),
            name_fr: Some("Ahmed Ben Ali".into()),
            primary_org_id: Some(Uuid::new_v4()),
            current_role: Some("VP Procurement".into()),
            role_family: Some("procurement".into()),
            region: Some("TN".into()),
            country_code: Some("TN".into()),
            priority_vector: Some(serde_json::json!({"cost":0.8,"quality":0.6})),
            influence_score: Some(0.7),
            metadata: Some(serde_json::json!({})),
            created_at: Some(Utc::now()),
            updated_at: Some(Utc::now()),
        };
        assert_eq!(row.name, "Ahmed Ben Ali");
        assert_eq!(row.role_family.as_deref(), Some("procurement"));
    }

    #[test]
    fn test_certification_row_fields() {
        let row = CertificationRow {
            id: Uuid::new_v4(),
            company_id: Some(Uuid::new_v4()),
            site_id: None,
            standard: "IATF_16949".into(),
            status: Some("active".into()),
            issuing_body: Some("TUV".into()),
            valid_from: Some(NaiveDate::from_ymd_opt(2023, 1, 1).unwrap()),
            valid_until: Some(NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()),
            scope: Some("Automotive EMS".into()),
            evidence_url: Some("https://example.com/cert".into()),
            metadata: Some(serde_json::json!({})),
            created_at: Some(Utc::now()),
            updated_at: Some(Utc::now()),
        };
        assert_eq!(row.standard, "IATF_16949");
        assert!(row.valid_until.unwrap() > row.valid_from.unwrap());
    }

    #[test]
    fn test_artifact_row_fields() {
        let row = ArtifactRow {
            id: Uuid::new_v4(),
            person_id: Some(Uuid::new_v4()),
            artifact_type: "press_quote".into(),
            title: Some("Industry 4.0 Panel".into()),
            content_summary: Some("Discussed manufacturing automation".into()),
            url: "https://example.com/article".into(),
            source_domain: Some("example.com".into()),
            language: Some("en".into()),
            topics: Some(vec!["automation".into(), "industry_4_0".into()]),
            sentiment_score: Some(0.7),
            key_phrases: Some(vec!["smart factory".into()]),
            ts_utc: Utc::now(),
            provenance: serde_json::json!({"url": "https://example.com/article"}),
            metadata: Some(serde_json::json!({})),
            created_at: Some(Utc::now()),
        };
        assert_eq!(row.artifact_type, "press_quote");
        assert!(row.topics.as_ref().unwrap().contains(&"automation".into()));
    }

    #[test]
    fn test_site_row_fields() {
        let row = SiteRow {
            id: Uuid::new_v4(),
            company_id: Some(Uuid::new_v4()),
            name: "Sousse Plant".into(),
            address: Some("Zone Industrielle".into()),
            city: Some("Sousse".into()),
            country_code: Some("TN".into()),
            region: Some("TN".into()),
            lat: Some(35.8256),
            lon: Some(10.6369),
            site_type: Some("plant".into()),
            capabilities: Some(vec!["SMT".into(), "THT".into(), "AOI".into()]),
            certifications: Some(vec!["ISO_9001".into(), "IATF_16949".into()]),
            employee_estimate: Some(300),
            free_zone: Some("TAC".into()),
            metadata: Some(serde_json::json!({})),
            created_at: Some(Utc::now()),
            updated_at: Some(Utc::now()),
        };
        assert_eq!(row.name, "Sousse Plant");
        assert!(row.lat.unwrap() > 35.0);
        assert!(row.capabilities.as_ref().unwrap().contains(&"SMT".into()));
    }
}
