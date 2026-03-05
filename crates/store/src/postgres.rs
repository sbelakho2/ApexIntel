use anyhow::Result;
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::postgres::{PgPool, PgPoolOptions};
use sqlx::Row;
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

fn insight_dedup_key(title: &str, insight_type: Option<&str>, region: Option<&str>) -> String {
    format!(
        "{}|{}|{}",
        title.trim().to_ascii_lowercase(),
        insight_type.unwrap_or_default().trim().to_ascii_lowercase(),
        region.unwrap_or_default().trim().to_ascii_lowercase(),
    )
}

fn normalize_optional_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string())
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
    pub insight_types: Vec<String>,
    /// When Some(user_id), only return bookmarked insights for this user.
    pub bookmarked_by: Option<String>,
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
    pub max_priority: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserSettingsPrefs {
    pub api_key_display: String,
    pub backend_url_display: String,
    pub session_timeout_hours: i64,
    pub default_region: String,
    pub auto_include_neighbors: bool,
    pub daily_crawl_enabled: bool,
    pub crawl_window: String,
    pub slack_enabled: bool,
    pub minimum_severity: String,
    pub auto_cleanup_enabled: bool,
    pub export_format: String,
    pub retention_period: String,
    pub email_digest_enabled: bool,
    pub notification_frequency: String,
    pub critical_only_enabled: bool,
}

#[derive(Debug, Clone)]
pub struct UserPreferencesRecord {
    pub theme: String,
    pub locale: String,
    pub preferences: Value,
    pub updated_at: DateTime<Utc>,
}

impl Default for UserSettingsPrefs {
    fn default() -> Self {
        Self {
            api_key_display: "(not configured)".to_string(),
            backend_url_display: "direct Axum service".to_string(),
            session_timeout_hours: 24,
            default_region: "Global".to_string(),
            auto_include_neighbors: false,
            daily_crawl_enabled: false,
            crawl_window: "00:00-06:00 UTC".to_string(),
            slack_enabled: false,
            minimum_severity: "high".to_string(),
            auto_cleanup_enabled: false,
            export_format: "JSON".to_string(),
            retention_period: "90 days".to_string(),
            email_digest_enabled: false,
            notification_frequency: "Daily".to_string(),
            critical_only_enabled: false,
        }
    }
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

    pub async fn get_user_settings_prefs(&self, user_id: &str) -> Result<Option<UserSettingsPrefs>> {
        self.ensure_user_preferences_table().await?;
        let row = sqlx::query("SELECT preferences FROM user_preferences WHERE user_id = $1")
            .bind(user_id)
            .fetch_optional(&self.pool)
            .await?;

        let Some(row) = row else {
            return Ok(None);
        };

        let preferences: Option<Value> = row.try_get("preferences")?;
        let Some(preferences) = preferences else {
            return Ok(None);
        };

        let Some(settings_value) = preferences.get("settings_page") else {
            return Ok(None);
        };

        Ok(serde_json::from_value::<UserSettingsPrefs>(settings_value.clone()).ok())
    }

    pub async fn get_user_preferences_record(&self, user_id: &str) -> Result<Option<UserPreferencesRecord>> {
        self.ensure_user_preferences_table().await?;
        let row = sqlx::query(
            "SELECT theme, locale, preferences, updated_at FROM user_preferences WHERE user_id = $1",
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;

        let Some(row) = row else {
            return Ok(None);
        };

        let theme: Option<String> = row.try_get("theme")?;
        let locale: Option<String> = row.try_get("locale")?;
        let preferences: Option<Value> = row.try_get("preferences")?;
        let updated_at: Option<DateTime<Utc>> = row.try_get("updated_at")?;

        Ok(Some(UserPreferencesRecord {
            theme: theme.unwrap_or_else(|| "system".to_string()),
            locale: locale.unwrap_or_else(|| "en".to_string()),
            preferences: preferences.unwrap_or_else(|| serde_json::json!({})),
            updated_at: updated_at.unwrap_or_else(Utc::now),
        }))
    }

    pub async fn upsert_user_preferences_record(
        &self,
        user_id: &str,
        theme: &str,
        locale: &str,
        preferences: &Value,
    ) -> Result<()> {
        self.ensure_user_preferences_table().await?;
        sqlx::query(
            r#"INSERT INTO user_preferences (user_id, theme, locale, preferences, updated_at)
               VALUES ($1, $2, $3, $4, NOW())
               ON CONFLICT (user_id) DO UPDATE SET
                 theme = EXCLUDED.theme,
                 locale = EXCLUDED.locale,
                 preferences = EXCLUDED.preferences,
                 updated_at = NOW()"#,
        )
        .bind(user_id)
        .bind(theme)
        .bind(locale)
        .bind(preferences)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn upsert_user_settings_prefs(&self, user_id: &str, prefs: &UserSettingsPrefs) -> Result<()> {
        self.ensure_user_preferences_table().await?;
        let row = sqlx::query("SELECT preferences, theme, locale FROM user_preferences WHERE user_id = $1")
            .bind(user_id)
            .fetch_optional(&self.pool)
            .await?;

        let (theme, locale, mut preferences): (String, String, Value) = if let Some(row) = row {
            let theme: Option<String> = row.try_get("theme")?;
            let locale: Option<String> = row.try_get("locale")?;
            let preferences: Option<Value> = row.try_get("preferences")?;
            (
                theme.unwrap_or_else(|| "system".to_string()),
                locale.unwrap_or_else(|| "en".to_string()),
                preferences.unwrap_or_else(|| serde_json::json!({})),
            )
        } else {
            (
                "system".to_string(),
                "en".to_string(),
                serde_json::json!({}),
            )
        };

        if !preferences.is_object() {
            preferences = serde_json::json!({});
        }
        if let Some(obj) = preferences.as_object_mut() {
            obj.insert("settings_page".to_string(), serde_json::to_value(prefs)?);
        }

        sqlx::query(
            r#"INSERT INTO user_preferences (user_id, theme, locale, preferences, updated_at)
               VALUES ($1, $2, $3, $4, NOW())
               ON CONFLICT (user_id) DO UPDATE SET
                 theme = EXCLUDED.theme,
                 locale = EXCLUDED.locale,
                 preferences = EXCLUDED.preferences,
                 updated_at = NOW()"#,
        )
        .bind(user_id)
        .bind(theme)
        .bind(locale)
        .bind(preferences)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn ensure_user_preferences_table(&self) -> Result<()> {
        sqlx::query(
            r#"CREATE TABLE IF NOT EXISTS user_preferences (
                   user_id TEXT PRIMARY KEY,
                   theme TEXT DEFAULT 'system',
                   locale TEXT DEFAULT 'en',
                   preferences JSONB DEFAULT '{}',
                   updated_at TIMESTAMPTZ DEFAULT now()
               )"#,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
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

    /// Batch-fetch `(id, name, region, company_type)` for a set of company UUIDs.
    pub async fn get_company_names_by_ids(&self, ids: &[Uuid]) -> Result<Vec<(Uuid, String, Option<String>, Option<String>)>> {
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

    /// Batch-fetch `(primary_org_id, name)` for persons linked to given company UUIDs.
    pub async fn get_person_names_by_company_ids(&self, company_ids: &[Uuid]) -> Result<Vec<(Uuid, String)>> {
        if company_ids.is_empty() {
            return Ok(vec![]);
        }
        let rows: Vec<(Uuid, String)> = sqlx::query_as(
            "SELECT primary_org_id, name FROM persons WHERE primary_org_id = ANY($1) ORDER BY influence_score DESC NULLS LAST",
        )
        .bind(company_ids)
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
                // Dedup by normalized warning content and keep the newest row globally.
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
                    acknowledged_by, acknowledged_at, acknowledged_note, created_at, updated_at
                                 FROM dedup"#,
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
        let mut qb: QueryBuilder<Postgres> = if filters.bookmarked_by.is_some() {
            let mut q = QueryBuilder::new(
                "SELECT i.id, i.title, i.summary, i.insight_type, i.region, i.confidence,
                        i.evidence_urls, i.entity_ids, i.tags, i.created_at, i.updated_at
                 FROM insights i
                 INNER JOIN insight_bookmarks bk ON bk.insight_id = i.id AND bk.user_id = ",
            );
            q.push_bind(filters.bookmarked_by.as_deref().unwrap().to_string());
            q
        } else {
            QueryBuilder::new(
                "SELECT id, title, summary, insight_type, region, confidence,
                        evidence_urls, entity_ids, tags, created_at, updated_at
                 FROM insights",
            )
        };
        let col_prefix = if filters.bookmarked_by.is_some() { "i." } else { "" };

        let mut has_where = false;
        if !filters.regions.is_empty() {
            qb.push(if has_where { " AND " } else { " WHERE " });
            qb.push(col_prefix).push("region = ANY(").push_bind(&filters.regions).push(")");
            has_where = true;
        }

        if let Some(date_from) = filters.date_from {
            qb.push(if has_where { " AND " } else { " WHERE " });
            qb.push(col_prefix).push("updated_at >= ").push_bind(date_from);
            has_where = true;
        }

        if let Some(date_to) = filters.date_to {
            qb.push(if has_where { " AND " } else { " WHERE " });
            qb.push(col_prefix).push("updated_at <= ").push_bind(date_to);
            has_where = true;
        }

        if let Some(search) = &filters.search {
            qb.push(if has_where { " AND " } else { " WHERE " });
            let pattern = ilike_pattern(search);
            qb.push("(").push(col_prefix).push("title ILIKE ")
                .push_bind(pattern.clone())
                .push(" OR ").push(col_prefix).push("summary ILIKE ")
                .push_bind(pattern)
                .push(")");
            has_where = true;
        }

        if !filters.insight_types.is_empty() {
            qb.push(if has_where { " AND " } else { " WHERE " });
            qb.push(col_prefix).push("insight_type = ANY(").push_bind(&filters.insight_types).push(")");
            has_where = true;
        }

        let _ = has_where; // suppress unused warning

        // When showing bookmarks, order by bookmark creation (most recent first)
        if filters.bookmarked_by.is_some() {
            qb.push(" ORDER BY bk.created_at DESC, i.id ASC ");
        } else {
            qb.push(" ORDER BY updated_at DESC, id ASC ");
        }
        qb.push(" LIMIT ").push_bind(limit);
        qb.push(" OFFSET ").push_bind(offset);

        let rows = qb.build_query_as::<InsightRow>().fetch_all(&self.pool).await?;

        // Defensive dedup for legacy duplicate rows that differ only by ingestion artifacts.
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
        let mut qb: QueryBuilder<Postgres> = if filters.bookmarked_by.is_some() {
            let mut q = QueryBuilder::new(
                "SELECT COUNT(DISTINCT CONCAT_WS('|', LOWER(TRIM(i.title)), LOWER(COALESCE(i.insight_type, '')), LOWER(COALESCE(i.region, '')))) FROM insights i
                 INNER JOIN insight_bookmarks bk ON bk.insight_id = i.id AND bk.user_id = ",
            );
            q.push_bind(filters.bookmarked_by.as_deref().unwrap().to_string());
            q
        } else {
            QueryBuilder::new(
                "SELECT COUNT(DISTINCT CONCAT_WS('|', LOWER(TRIM(title)), LOWER(COALESCE(insight_type, '')), LOWER(COALESCE(region, '')))) FROM insights"
            )
        };
        let col_prefix = if filters.bookmarked_by.is_some() { "i." } else { "" };
        let mut has_where = false;

        if !filters.regions.is_empty() {
            qb.push(if has_where { " AND " } else { " WHERE " });
            qb.push(col_prefix).push("region = ANY(").push_bind(&filters.regions).push(")");
            has_where = true;
        }

        if let Some(date_from) = filters.date_from {
            qb.push(if has_where { " AND " } else { " WHERE " });
            qb.push(col_prefix).push("updated_at >= ").push_bind(date_from);
            has_where = true;
        }

        if let Some(date_to) = filters.date_to {
            qb.push(if has_where { " AND " } else { " WHERE " });
            qb.push(col_prefix).push("updated_at <= ").push_bind(date_to);
            has_where = true;
        }

        if let Some(search) = &filters.search {
            qb.push(if has_where { " AND " } else { " WHERE " });
            let pattern = ilike_pattern(search);
            qb.push("(").push(col_prefix).push("title ILIKE ")
                .push_bind(pattern.clone())
                .push(" OR ").push(col_prefix).push("summary ILIKE ")
                .push_bind(pattern)
                .push(")");
            has_where = true;
        }

        if !filters.insight_types.is_empty() {
            qb.push(if has_where { " AND " } else { " WHERE " });
            qb.push(col_prefix).push("insight_type = ANY(").push_bind(&filters.insight_types).push(")");
        }

        let row: (i64,) = qb.build_query_as().fetch_one(&self.pool).await?;
        Ok(row.0)
    }

    // ─── Insights (write) ────────────────────────────────────────────────

    /// Insert or upsert an insight into the insights table.
    ///
    /// Uses ON CONFLICT on (md5(title), entity_ids) to deduplicate.
    /// On conflict, keeps the higher confidence and updates summary/tags.
    /// Returns the UUID for the row (new or existing).
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
    ) -> Result<Uuid> {
        let id = Uuid::new_v4();
        let normalized_title = title.trim();
        let normalized_entity_ids: Option<Vec<Uuid>> = entity_ids.map(|mut ids| {
            ids.sort();
            ids.dedup();
            ids
        });
        let row: (Uuid,) = sqlx::query_as(
            r#"INSERT INTO insights
               (id, title, title_hash, summary, insight_type, region, confidence,
                evidence_urls, entity_ids, tags, created_at, updated_at)
               VALUES ($1, $2, md5($2), $3, $4, $5, $6, $7, $8, $9, now(), now())
               ON CONFLICT (title_hash, (COALESCE(entity_ids, ARRAY[]::uuid[])))
               DO UPDATE SET
                 confidence = GREATEST(insights.confidence, EXCLUDED.confidence),
                 summary    = EXCLUDED.summary,
                 tags       = EXCLUDED.tags,
                 updated_at = now()
               RETURNING id"#,
        )
        .bind(id)
        .bind(normalized_title)
        .bind(summary)
        .bind(insight_type)
        .bind(region)
        .bind(confidence)
        .bind(&evidence_urls)
        .bind(&normalized_entity_ids)
        .bind(&tags)
        .fetch_one(&self.pool)
        .await?;
        Ok(row.0)
    }

    // ─── Insight Bookmarks ──────────────────────────────────────────────

    /// Bookmark an insight for a user.  Returns true if newly created, false if
    /// already bookmarked.
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

    /// Remove a bookmark.  Returns true if a row was deleted.
    pub async fn unbookmark_insight(
        &self,
        insight_id: Uuid,
        user_id: &str,
    ) -> Result<bool> {
        let result = sqlx::query(
            "DELETE FROM insight_bookmarks WHERE insight_id = $1 AND user_id = $2",
        )
        .bind(insight_id)
        .bind(user_id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Check if a single insight is bookmarked by a user.
    pub async fn is_insight_bookmarked(
        &self,
        insight_id: Uuid,
        user_id: &str,
    ) -> Result<bool> {
        let row: (bool,) = sqlx::query_as(
            "SELECT EXISTS(SELECT 1 FROM insight_bookmarks WHERE insight_id = $1 AND user_id = $2)",
        )
        .bind(insight_id)
        .bind(user_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(row.0)
    }

    /// Return all bookmarked insight IDs for a user (for bulk lookup).
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
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    /// Get observation type counts per entity over a rolling time window.
    ///
    /// Returns `(entity_id, observation_type, count)` for all entities that
    /// have at least one observation since `since`.  Used by the `RecipeFire`
    /// job to build per-entity `FeatureMap` entries without loading raw JSONB.
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

    /// Get warning type counts per entity for the last N days.
    ///
    /// Returns `(entity_id, warning_type, count)`.  Used by `RecipeFire` to
    /// enrich feature maps with warning-derived signal counts so that recipes
    /// referencing signal types like `CertificationUpdate`, `JobPost`,
    /// `Competitor` can match against warning data.
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

    /// Get CompetitorEvent signal_type and keyword counts per entity.
    ///
    /// Extracts structured fields from the JSONB `value` column of
    /// `CompetitorEvent` observations and returns per-entity counts.
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

    /// Extract rich feature signals from WebChange observation JSONB payloads.
    ///
    /// Returns `(entity_id, source_id, signal_type, count)` tuples for each
    /// entity, giving the feature map builder fine-grained data about what
    /// *kind* of web changes occurred (patent filings, sanctions updates,
    /// tender postings, hiring signals, etc.).
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

    /// Extract WebChange keyword features per entity.
    ///
    /// Returns `(entity_id, keyword, count)` — keywords like "patent",
    /// "sanctions", "acquisition" etc. map directly to recipe observation types.
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

    /// Extract person/POI features per company for recipe evaluation.
    ///
    /// Returns `(company_id, role_family, influence_score, pain_index,
    /// change_risk, person_count)` — one row per role_family per company.
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

    /// Extract certification features per company.
    ///
    /// Returns `(company_id, standard, count)`.
    pub async fn get_certification_features_per_company(
        &self,
    ) -> Result<Vec<(Uuid, String, i64)>> {
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
                let std: String = row.try_get("standard").ok()?;
                let cnt: i64 = row.try_get("cnt").ok()?;
                Some((cid, std, cnt))
            })
            .collect())
    }

    /// Extract capability features per company.
    ///
    /// Returns `(company_id, capability, count)`.
    pub async fn get_capability_features_per_company(
        &self,
    ) -> Result<Vec<(Uuid, String, i64)>> {
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
                let cap: String = row.try_get("capability").ok()?;
                let cnt: i64 = row.try_get("cnt").ok()?;
                Some((cid, cap, cnt))
            })
            .collect())
    }

    /// Extract site features per company.
    ///
    /// Returns `(company_id, country_code, site_type, count)`.
    pub async fn get_site_features_per_company(
        &self,
    ) -> Result<Vec<(Uuid, String, String, i64)>> {
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

    /// Extract graph-edge features per entity.
    ///
    /// Returns `(source_id, edge_type, count)`.
    pub async fn get_graph_edge_features(
        &self,
    ) -> Result<Vec<(Uuid, String, i64)>> {
        let rows = sqlx::query(
            r#"SELECT source_id,
                      edge_type,
                      COUNT(*)::BIGINT AS cnt
               FROM graph_edges
               GROUP BY source_id, edge_type
               ORDER BY source_id"#,
        )
        .fetch_all(&self.pool)
        .await?;

        use sqlx::Row as _;
        Ok(rows
            .into_iter()
            .filter_map(|row| {
                let sid: Uuid = row.try_get("source_id").ok()?;
                let et: String = row.try_get("edge_type").ok()?;
                let cnt: i64 = row.try_get("cnt").ok()?;
                Some((sid, et, cnt))
            })
            .collect())
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
            "SELECT id, name, name_ar, name_fr, primary_org_id, \"current_role\",\n                    role_family, region, country_code, public_bio, public_email,\n                    priority_vector, influence_score, trigger_topics, decision_style,\n                    risk_tolerance, change_appetite, communication_style,\n                    decision_mode, preferred_proof_type, pain_index, change_risk,\n                    role_drift_score,\n                    metadata, created_at, updated_at\n             FROM persons WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// Update confirmed contact details for an existing person.
    /// Only overwrites fields that are provided (not `None`).
    pub async fn update_person_contacts(
        &self,
        person_id: Uuid,
        email: Option<&str>,
        phone: Option<&str>,
        linkedin: Option<&str>,
    ) -> Result<()> {
        // Build a partial-update using COALESCE so we only overwrite NULL fields
        // unless a non-null value is explicitly provided.
        sqlx::query(
            r#"UPDATE persons
               SET public_email = COALESCE($2, public_email),
                   metadata = jsonb_set(
                       jsonb_set(
                           COALESCE(metadata, '{}'::jsonb),
                           '{phone}', to_jsonb($3::TEXT)
                       ),
                       '{linkedin_url}', to_jsonb($4::TEXT)
                   ),
                   updated_at = now()
               WHERE id = $1"#,
        )
        .bind(person_id)
        .bind(email)
        .bind(phone)
        .bind(linkedin)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Return a minimal set of all persons for use as expansion engine seeds.
    /// Returns id, name, current_role, role_family, region, primary_org_id, and
    /// the org's website (joined from companies).
    pub async fn list_expansion_seeds(&self, limit: i64) -> Result<Vec<ExpansionSeedRow>> {
        let rows = sqlx::query_as::<_, ExpansionSeedRow>(
            r#"WITH ranked AS (
                   SELECT p.id,
                          p.name,
                          COALESCE(p."current_role", p.role_family, '') AS current_role,
                          COALESCE(p.role_family, 'Other') AS role_family,
                          COALESCE(p.region, '') AS region,
                          COALESCE(p.country_code, '') AS country_code,
                          p.primary_org_id,
                          COALESCE(c.name, '') AS org_name,
                         c.domain AS org_domain,
                         COALESCE((c.metadata->>'is_competitor')::boolean, false) AS is_competitor,
                          ROW_NUMBER() OVER (
                              PARTITION BY COALESCE(NULLIF(p.region, ''), 'GLOBAL')
                              ORDER BY
                                  CASE
                                      WHEN LOWER(COALESCE(p.role_family, '')) IN (
                                          'government', 'procurement', 'operations', 'engineering',
                                          'quality', 'legal', 'security', 'logistics', 'finance'
                                      ) THEN 0
                                      WHEN LOWER(COALESCE(p.role_family, '')) = 'executive' THEN 2
                                      ELSE 1
                                  END,
                                  COALESCE(p.influence_score, 0) DESC,
                                  p.updated_at DESC NULLS LAST
                          ) AS region_rank
                   FROM persons p
                   LEFT JOIN companies c ON p.primary_org_id = c.id
                   WHERE COALESCE(p.name, '') <> ''
               )
               SELECT id,
                      name,
                      current_role,
                      role_family,
                      region,
                      country_code,
                      primary_org_id,
                      org_name,
                 org_domain,
                 is_competitor
               FROM ranked
               WHERE region_rank <= GREATEST(2, LEAST(8, $1 / 6))
               ORDER BY
                   CASE
                       WHEN LOWER(role_family) IN ('government', 'procurement', 'operations', 'engineering', 'quality', 'legal', 'security', 'logistics', 'finance') THEN 0
                       WHEN LOWER(role_family) = 'executive' THEN 2
                       ELSE 1
                   END,
                   region,
                   name
               LIMIT $1"#,
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn list_persons_by_org(&self, org_id: Uuid) -> Result<Vec<PersonRow>> {
        let rows = sqlx::query_as::<_, PersonRow>(
            "SELECT id, name, name_ar, name_fr, primary_org_id, \"current_role\",
                    role_family, region, country_code, public_bio, public_email,
                    priority_vector, influence_score, trigger_topics, decision_style,
                    risk_tolerance, change_appetite, communication_style,
                    decision_mode, preferred_proof_type, pain_index, change_risk,
                    role_drift_score,
                    metadata, created_at, updated_at
             FROM persons WHERE primary_org_id = $1 ORDER BY name",
        )
        .bind(org_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Update the `influence_score` for a single person.
    /// Used by the PoiRefresh worker job to write back computed scores after
    /// calling `refresh_profile()`. No-op (silently returns Ok) if the person
    /// does not exist.
    pub async fn update_person_influence_score(&self, id: Uuid, score: f64) -> Result<()> {
        sqlx::query(
            r#"UPDATE persons
               SET influence_score = $2,
                   updated_at = now()
               WHERE id = $1"#,
        )
        .bind(id)
        .bind(score.clamp(0.0, 1.0))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Write back LLM-synthesised psychographic/biographical data for a POI.
    /// Only overwrites NULL or short (< 100 chars) existing values so manually
    /// curated entries are preserved. Noop-safe if the person does not exist.
    pub async fn update_person_llm_enrichment(
        &self,
        id: Uuid,
        bio: &str,
        decision_style: Option<&str>,
        communication_style: Option<&str>,
        risk_tolerance: Option<&str>,
        change_appetite: Option<&str>,
        preferred_proof_type: Option<&str>,
        trigger_topics: &[String],
    ) -> Result<()> {
        sqlx::query(
            r#"UPDATE persons SET
                public_bio = CASE
                    WHEN public_bio IS NULL OR length(public_bio) < 100 THEN $2
                    ELSE public_bio
                END,
                decision_style = COALESCE(decision_style, $3),
                communication_style = COALESCE(communication_style, $4),
                risk_tolerance = COALESCE(risk_tolerance, $5),
                change_appetite = COALESCE(change_appetite, $6),
                preferred_proof_type = COALESCE(preferred_proof_type, $7),
                trigger_topics = CASE
                    WHEN trigger_topics IS NULL OR array_length(trigger_topics, 1) IS NULL THEN $8
                    ELSE trigger_topics
                END,
                updated_at = now()
               WHERE id = $1"#,
        )
        .bind(id)
        .bind(bio)
        .bind(decision_style)
        .bind(communication_style)
        .bind(risk_tolerance)
        .bind(change_appetite)
        .bind(preferred_proof_type)
        .bind(trigger_topics)
        .execute(&self.pool)
        .await?;
        Ok(())
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

        if let Some(max_priority) = filters.max_priority {
            if !has_where {
                qb.push(" WHERE ");
                has_where = true;
            } else {
                qb.push(" AND ");
            }
            qb.push("COALESCE(p.influence_score, 0) < ");
            qb.push_bind(max_priority);
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
                    COALESCE(p.country_code, '') AS country_code,
                    COALESCE(p.role_family, 'Unknown') AS role_family,
                    COALESCE(p.influence_score, 0) AS priority_score,
                    COALESCE(p.metadata->>'engagement_status', 'untracked') AS engagement_status,
                    COALESCE(p.updated_at, p.created_at, now()) AS updated_at,
                    (SELECT COUNT(*) FROM poi_artifacts a WHERE a.person_id = p.id) AS artifact_count
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

        if let Some(max_priority) = filters.max_priority {
            if !has_where {
                qb.push(" WHERE ");
                has_where = true;
            } else {
                qb.push(" AND ");
            }
            qb.push("COALESCE(p.influence_score, 0) < ");
            qb.push_bind(max_priority);
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

    /// Get recipe signal statistics from recipes LEFT JOIN warnings
    /// so that recipes with zero warnings still appear in the list.
    pub async fn get_recipe_stats(&self) -> Result<Vec<RecipeStatRow>> {
        let rows = sqlx::query_as::<_, RecipeStatRow>(
            r#"SELECT
                 r.code AS recipe_code,
                 COALESCE(COUNT(w.id), 0) AS fired_count,
                 MAX(w.created_at) AS last_fired,
                 MIN(w.created_at) AS first_fired,
                 COALESCE(COUNT(*) FILTER (WHERE w.id IS NOT NULL AND NOT w.acknowledged), 0) AS active_count
               FROM recipes r
               LEFT JOIN warnings w ON w.recipe_code = r.code
               WHERE r.status IN ('active', 'production', 'staging')
               GROUP BY r.code
               ORDER BY COALESCE(COUNT(w.id), 0) DESC, r.code ASC"#,
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

    pub async fn get_role_history_for_person(
        &self,
        person_id: Uuid,
    ) -> Result<Vec<RoleHistoryRow>> {
        let rows = sqlx::query_as::<_, RoleHistoryRow>(
            "SELECT id, person_id, org_id, org_name, title, role_family,
                    start_date, end_date, source_url, confidence, verified,
                    metadata, created_at, updated_at
             FROM role_history
             WHERE person_id = $1
             ORDER BY start_date DESC NULLS LAST",
        )
        .bind(person_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn get_person_peers(
        &self,
        person_id: Uuid,
        role_family: &str,
        region: &str,
        limit: i64,
    ) -> Result<Vec<PersonListRow>> {
        let limit = clamp_limit(limit);
        let rows = sqlx::query_as::<_, PersonListRow>(
            "SELECT p.id,
                    p.name,
                    COALESCE(p.\"current_role\", p.role_family, 'Unknown') AS role,
                    COALESCE(c.name, 'Independent') AS organization,
                    COALESCE(p.region, '') AS region,
                    COALESCE(p.country_code, '') AS country_code,
                    COALESCE(p.role_family, 'Unknown') AS role_family,
                    COALESCE(p.influence_score, 0) AS priority_score,
                    COALESCE(p.metadata->>'engagement_status', 'untracked') AS engagement_status,
                    COALESCE(p.updated_at, p.created_at, now()) AS updated_at,
                    (SELECT COUNT(*) FROM poi_artifacts a WHERE a.person_id = p.id) AS artifact_count
             FROM persons p
             LEFT JOIN companies c ON p.primary_org_id = c.id
             WHERE p.id != $1
               AND (p.role_family = $2 OR COALESCE(p.region, '') = $3)
             ORDER BY p.influence_score DESC NULLS LAST
             LIMIT $4",
        )
        .bind(person_id)
        .bind(role_family)
        .bind(region)
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
            "SELECT COUNT(*) FROM recipes WHERE created_at >= $1 AND status = 'staging'",
        )
        .bind(_since)
        .fetch_one(&self.pool)
        .await
        .unwrap_or(0);

        // hypotheses_generated = all recipes created in the window (any lifecycle state),
        // since every recipe row represents a hypothesis that was evaluated; recipes_staged
        // is the subset that passed the full staging gate, so hypotheses_generated >= recipes_staged.
        let hypotheses_generated: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM recipes WHERE created_at >= $1",
        )
        .bind(_since)
        .fetch_one(&self.pool)
        .await
        .unwrap_or(0)
        .max(recipes_staged); // ensure invariant: hypotheses >= staged

        Ok(MiningStats {
            candidates_found: candidates_found as u64,
            candidates_passed_gates: candidates_passed as u64,
            hypotheses_generated: hypotheses_generated as u64,
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
            role_changes_detected: self.count_role_changes_since(_since).await.unwrap_or(0) as u64,
            errors: vec![],
        })
    }

    /// Get drift statistics from the feature store.
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
            features_checked: features_checked.max(0) as u64,
            features_drifted: features_drifted.max(0) as u64,
            alerts_raised: features_drifted.max(0) as u64,
            drift_scores,
            errors: vec![],
        })
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
               WHERE status = 'staging'
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
               WHERE status = 'production'
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
            "SELECT COUNT(*) FROM recipes WHERE status = 'production'",
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
        let top_regions = top_regions_rows.into_iter().map(|(region,)| region).collect();

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
        let notable_events = notable_events_rows.into_iter().map(|(title,)| title).collect();

        Ok(WeeklySummaryStats {
            companies_monitored: companies_monitored as u64,
            persons_tracked: persons_tracked as u64,
            warnings_generated: warnings_generated as u64,
            insights_produced: insights_produced as u64,
            recipes_in_production: recipes_in_production as u64,
            top_regions,
            notable_events,
        })
    }

    // ─── List / Count: Sites ────────────────────────────────────────────

    pub async fn list_sites(&self, company_id: Option<Uuid>, region: Option<&str>, limit: i64, offset: i64) -> Result<Vec<SiteRow>> {
        let mut sql = String::from("SELECT * FROM sites WHERE 1=1");
        if company_id.is_some() { sql.push_str(" AND company_id = $3"); }
        if region.is_some() { sql.push_str(if company_id.is_some() { " AND region = $4" } else { " AND region = $3" }); }
        sql.push_str(" ORDER BY name ASC LIMIT $1 OFFSET $2");

        // Use a simple approach - 2 variants
        match (company_id, region) {
            (Some(cid), Some(r)) => {
                Ok(sqlx::query_as::<_, SiteRow>("SELECT * FROM sites WHERE company_id = $3 AND region = $4 ORDER BY name ASC LIMIT $1 OFFSET $2")
                    .bind(limit).bind(offset).bind(cid).bind(r)
                    .fetch_all(&self.pool).await?)
            }
            (Some(cid), None) => {
                Ok(sqlx::query_as::<_, SiteRow>("SELECT * FROM sites WHERE company_id = $3 ORDER BY name ASC LIMIT $1 OFFSET $2")
                    .bind(limit).bind(offset).bind(cid)
                    .fetch_all(&self.pool).await?)
            }
            (None, Some(r)) => {
                Ok(sqlx::query_as::<_, SiteRow>("SELECT * FROM sites WHERE region = $3 ORDER BY name ASC LIMIT $1 OFFSET $2")
                    .bind(limit).bind(offset).bind(r)
                    .fetch_all(&self.pool).await?)
            }
            (None, None) => {
                Ok(sqlx::query_as::<_, SiteRow>("SELECT * FROM sites ORDER BY name ASC LIMIT $1 OFFSET $2")
                    .bind(limit).bind(offset)
                    .fetch_all(&self.pool).await?)
            }
        }
    }

    pub async fn count_sites(&self, company_id: Option<Uuid>, region: Option<&str>) -> Result<i64> {
        let count: i64 = match (company_id, region) {
            (Some(cid), Some(r)) => {
                sqlx::query_scalar("SELECT COUNT(*) FROM sites WHERE company_id = $1 AND region = $2")
                    .bind(cid).bind(r).fetch_one(&self.pool).await?
            }
            (Some(cid), None) => {
                sqlx::query_scalar("SELECT COUNT(*) FROM sites WHERE company_id = $1")
                    .bind(cid).fetch_one(&self.pool).await?
            }
            (None, Some(r)) => {
                sqlx::query_scalar("SELECT COUNT(*) FROM sites WHERE region = $1")
                    .bind(r).fetch_one(&self.pool).await?
            }
            (None, None) => {
                sqlx::query_scalar("SELECT COUNT(*) FROM sites")
                    .fetch_one(&self.pool).await?
            }
        };
        Ok(count)
    }

    // ─── List / Count: Capabilities ──────────────────────────────────────

    pub async fn list_capabilities(&self, company_id: Option<Uuid>, limit: i64, offset: i64) -> Result<Vec<CapabilityRow>> {
        match company_id {
            Some(cid) => {
                Ok(sqlx::query_as::<_, CapabilityRow>("SELECT * FROM capabilities WHERE company_id = $3 ORDER BY capability ASC LIMIT $1 OFFSET $2")
                    .bind(limit).bind(offset).bind(cid)
                    .fetch_all(&self.pool).await?)
            }
            None => {
                Ok(sqlx::query_as::<_, CapabilityRow>("SELECT * FROM capabilities ORDER BY capability ASC LIMIT $1 OFFSET $2")
                    .bind(limit).bind(offset)
                    .fetch_all(&self.pool).await?)
            }
        }
    }

    pub async fn count_capabilities(&self, company_id: Option<Uuid>) -> Result<i64> {
        let count: i64 = match company_id {
            Some(cid) => {
                sqlx::query_scalar("SELECT COUNT(*) FROM capabilities WHERE company_id = $1")
                    .bind(cid).fetch_one(&self.pool).await?
            }
            None => {
                sqlx::query_scalar("SELECT COUNT(*) FROM capabilities")
                    .fetch_one(&self.pool).await?
            }
        };
        Ok(count)
    }

    // ─── List / Count: Certifications ────────────────────────────────────

    pub async fn list_certifications(&self, company_id: Option<Uuid>, limit: i64, offset: i64) -> Result<Vec<CertificationRow>> {
        match company_id {
            Some(cid) => {
                Ok(sqlx::query_as::<_, CertificationRow>("SELECT * FROM certifications WHERE company_id = $3 ORDER BY standard ASC LIMIT $1 OFFSET $2")
                    .bind(limit).bind(offset).bind(cid)
                    .fetch_all(&self.pool).await?)
            }
            None => {
                Ok(sqlx::query_as::<_, CertificationRow>("SELECT * FROM certifications ORDER BY standard ASC LIMIT $1 OFFSET $2")
                    .bind(limit).bind(offset)
                    .fetch_all(&self.pool).await?)
            }
        }
    }

    pub async fn count_certifications(&self, company_id: Option<Uuid>) -> Result<i64> {
        let count: i64 = match company_id {
            Some(cid) => {
                sqlx::query_scalar("SELECT COUNT(*) FROM certifications WHERE company_id = $1")
                    .bind(cid).fetch_one(&self.pool).await?
            }
            None => {
                sqlx::query_scalar("SELECT COUNT(*) FROM certifications")
                    .fetch_one(&self.pool).await?
            }
        };
        Ok(count)
    }

    // ─── List / Count: Observations ──────────────────────────────────────

    pub async fn list_observations(&self, entity_id: Option<Uuid>, obs_type: Option<&str>, limit: i64, offset: i64) -> Result<Vec<ObservationRow>> {
        match (entity_id, obs_type) {
            (Some(eid), Some(ot)) => {
                Ok(sqlx::query_as::<_, ObservationRow>("SELECT * FROM observations WHERE entity_id = $3 AND observation_type = $4 ORDER BY ts_utc DESC LIMIT $1 OFFSET $2")
                    .bind(limit).bind(offset).bind(eid).bind(ot)
                    .fetch_all(&self.pool).await?)
            }
            (Some(eid), None) => {
                Ok(sqlx::query_as::<_, ObservationRow>("SELECT * FROM observations WHERE entity_id = $3 ORDER BY ts_utc DESC LIMIT $1 OFFSET $2")
                    .bind(limit).bind(offset).bind(eid)
                    .fetch_all(&self.pool).await?)
            }
            (None, Some(ot)) => {
                Ok(sqlx::query_as::<_, ObservationRow>("SELECT * FROM observations WHERE observation_type = $3 ORDER BY ts_utc DESC LIMIT $1 OFFSET $2")
                    .bind(limit).bind(offset).bind(ot)
                    .fetch_all(&self.pool).await?)
            }
            (None, None) => {
                Ok(sqlx::query_as::<_, ObservationRow>("SELECT * FROM observations ORDER BY ts_utc DESC LIMIT $1 OFFSET $2")
                    .bind(limit).bind(offset)
                    .fetch_all(&self.pool).await?)
            }
        }
    }

    pub async fn count_observations(&self, entity_id: Option<Uuid>, obs_type: Option<&str>) -> Result<i64> {
        let count: i64 = match (entity_id, obs_type) {
            (Some(eid), Some(ot)) => {
                sqlx::query_scalar("SELECT COUNT(*) FROM observations WHERE entity_id = $1 AND observation_type = $2")
                    .bind(eid).bind(ot).fetch_one(&self.pool).await?
            }
            (Some(eid), None) => {
                sqlx::query_scalar("SELECT COUNT(*) FROM observations WHERE entity_id = $1")
                    .bind(eid).fetch_one(&self.pool).await?
            }
            (None, Some(ot)) => {
                sqlx::query_scalar("SELECT COUNT(*) FROM observations WHERE observation_type = $1")
                    .bind(ot).fetch_one(&self.pool).await?
            }
            (None, None) => {
                sqlx::query_scalar("SELECT COUNT(*) FROM observations")
                    .fetch_one(&self.pool).await?
            }
        };
        Ok(count)
    }

    // ─── List / Count: Product Families ──────────────────────────────────

    pub async fn list_product_families(&self, company_id: Option<Uuid>, limit: i64, offset: i64) -> Result<Vec<ProductFamilyRow>> {
        match company_id {
            Some(cid) => {
                Ok(sqlx::query_as::<_, ProductFamilyRow>("SELECT * FROM product_families WHERE company_id = $3 ORDER BY name ASC LIMIT $1 OFFSET $2")
                    .bind(limit).bind(offset).bind(cid)
                    .fetch_all(&self.pool).await?)
            }
            None => {
                Ok(sqlx::query_as::<_, ProductFamilyRow>("SELECT * FROM product_families ORDER BY name ASC LIMIT $1 OFFSET $2")
                    .bind(limit).bind(offset)
                    .fetch_all(&self.pool).await?)
            }
        }
    }

    pub async fn count_product_families(&self, company_id: Option<Uuid>) -> Result<i64> {
        let count: i64 = match company_id {
            Some(cid) => {
                sqlx::query_scalar("SELECT COUNT(*) FROM product_families WHERE company_id = $1")
                    .bind(cid).fetch_one(&self.pool).await?
            }
            None => {
                sqlx::query_scalar("SELECT COUNT(*) FROM product_families")
                    .fetch_one(&self.pool).await?
            }
        };
        Ok(count)
    }

    // ─── List / Count: Logistics Nodes ───────────────────────────────────

    pub async fn list_logistics_nodes(&self, country_code: Option<&str>, limit: i64, offset: i64) -> Result<Vec<LogisticsNodeRow>> {
        match country_code {
            Some(cc) => {
                Ok(sqlx::query_as::<_, LogisticsNodeRow>("SELECT * FROM logistics_nodes WHERE country_code = $3 ORDER BY name ASC LIMIT $1 OFFSET $2")
                    .bind(limit).bind(offset).bind(cc)
                    .fetch_all(&self.pool).await?)
            }
            None => {
                Ok(sqlx::query_as::<_, LogisticsNodeRow>("SELECT * FROM logistics_nodes ORDER BY name ASC LIMIT $1 OFFSET $2")
                    .bind(limit).bind(offset)
                    .fetch_all(&self.pool).await?)
            }
        }
    }

    pub async fn count_logistics_nodes(&self, country_code: Option<&str>) -> Result<i64> {
        let count: i64 = match country_code {
            Some(cc) => {
                sqlx::query_scalar("SELECT COUNT(*) FROM logistics_nodes WHERE country_code = $1")
                    .bind(cc).fetch_one(&self.pool).await?
            }
            None => {
                sqlx::query_scalar("SELECT COUNT(*) FROM logistics_nodes")
                    .fetch_one(&self.pool).await?
            }
        };
        Ok(count)
    }

    // ─── List / Count: Regulations ───────────────────────────────────────

    pub async fn list_regulations(&self, jurisdiction: Option<&str>, limit: i64, offset: i64) -> Result<Vec<RegulationRow>> {
        match jurisdiction {
            Some(j) => {
                Ok(sqlx::query_as::<_, RegulationRow>("SELECT * FROM regulations WHERE jurisdiction = $3 ORDER BY effective_date DESC NULLS LAST LIMIT $1 OFFSET $2")
                    .bind(limit).bind(offset).bind(j)
                    .fetch_all(&self.pool).await?)
            }
            None => {
                Ok(sqlx::query_as::<_, RegulationRow>("SELECT * FROM regulations ORDER BY effective_date DESC NULLS LAST LIMIT $1 OFFSET $2")
                    .bind(limit).bind(offset)
                    .fetch_all(&self.pool).await?)
            }
        }
    }

    pub async fn count_regulations(&self, jurisdiction: Option<&str>) -> Result<i64> {
        let count: i64 = match jurisdiction {
            Some(j) => {
                sqlx::query_scalar("SELECT COUNT(*) FROM regulations WHERE jurisdiction = $1")
                    .bind(j).fetch_one(&self.pool).await?
            }
            None => {
                sqlx::query_scalar("SELECT COUNT(*) FROM regulations")
                    .fetch_one(&self.pool).await?
            }
        };
        Ok(count)
    }

    // ─── List: POI Artifacts ─────────────────────────────────────────────

    pub async fn list_poi_artifacts(&self, person_id: Option<Uuid>, limit: i64, offset: i64) -> Result<Vec<ArtifactRow>> {
        match person_id {
            Some(pid) => {
                Ok(sqlx::query_as::<_, ArtifactRow>("SELECT * FROM poi_artifacts WHERE person_id = $3 ORDER BY ts_utc DESC LIMIT $1 OFFSET $2")
                    .bind(limit).bind(offset).bind(pid)
                    .fetch_all(&self.pool).await?)
            }
            None => {
                Ok(sqlx::query_as::<_, ArtifactRow>("SELECT * FROM poi_artifacts ORDER BY ts_utc DESC LIMIT $1 OFFSET $2")
                    .bind(limit).bind(offset)
                    .fetch_all(&self.pool).await?)
            }
        }
    }

    pub async fn count_poi_artifacts(&self, person_id: Option<Uuid>) -> Result<i64> {
        let count: i64 = match person_id {
            Some(pid) => {
                sqlx::query_scalar("SELECT COUNT(*) FROM poi_artifacts WHERE person_id = $1")
                    .bind(pid).fetch_one(&self.pool).await?
            }
            None => {
                sqlx::query_scalar("SELECT COUNT(*) FROM poi_artifacts")
                    .fetch_one(&self.pool).await?
            }
        };
        Ok(count)
    }

    // ─── Dashboard Stats ─────────────────────────────────────────────────

    pub async fn get_dashboard_stats(&self) -> Result<DashboardStats> {
        let total_companies: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM companies").fetch_one(&self.pool).await?;
        let total_persons: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM persons").fetch_one(&self.pool).await?;
        let total_warnings: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM warnings").fetch_one(&self.pool).await?;
        let unacknowledged_warnings: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM warnings WHERE acknowledged = false").fetch_one(&self.pool).await?;
        let total_insights: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM insights").fetch_one(&self.pool).await?;
        let active_recipes: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM recipes WHERE status IN ('active', 'production')").fetch_one(&self.pool).await.unwrap_or(0);
        let new_insights_24h: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM insights WHERE created_at > now() - interval '24 hours'").fetch_one(&self.pool).await.unwrap_or(0);
        let new_warnings_24h: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM warnings WHERE created_at > now() - interval '24 hours'").fetch_one(&self.pool).await.unwrap_or(0);

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
            ActivityItem { label: "Companies Tracked".to_string(), count: total_companies as u64, delta: 0 },
            ActivityItem { label: "Persons Tracked".to_string(), count: total_persons as u64, delta: 0 },
            ActivityItem { label: "Active Warnings".to_string(), count: unacknowledged_warnings as u64, delta: new_warnings_24h as i64 },
            ActivityItem { label: "Insights Generated".to_string(), count: total_insights as u64, delta: new_insights_24h as i64 },
        ];

        Ok(DashboardStats {
            total_companies: total_companies as u64,
            total_persons: total_persons as u64,
            total_warnings: total_warnings as u64,
            unacknowledged_warnings: unacknowledged_warnings as u64,
            total_insights: total_insights as u64,
            active_recipes: active_recipes as u64,
            new_insights_24h: new_insights_24h as u64,
            new_warnings_24h: new_warnings_24h as u64,
            top_regions,
            threat_distribution,
            recent_activity,
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

// ─── Weekly Memo Types ───────────────────────────────────────────────────────

/// A section within a weekly memo.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WeeklyMemoSection {
    pub title: String,
    pub content: String,
    pub priority: String,
    pub related_warnings: Vec<String>,
    pub related_insights: Vec<String>,
}

/// Key metrics for a weekly memo.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WeeklyMemoKeyMetrics {
    pub warnings_total: i64,
    pub warnings_critical: i64,
    pub insights_generated: i64,
    pub companies_monitored: i64,
    pub pois_tracked: i64,
}

/// Action item in a weekly memo.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WeeklyMemoActionItem {
    pub text: String,
    pub priority: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
}

/// Full weekly memo structure matching frontend expectations.
#[derive(Debug, Clone, serde::Serialize)]
pub struct WeeklyMemo {
    pub id: Uuid,
    pub title: String,
    pub week_start: String,
    pub week_end: String,
    pub executive_summary: String,
    pub sections: Vec<WeeklyMemoSection>,
    pub key_metrics: WeeklyMemoKeyMetrics,
    pub action_items: Vec<WeeklyMemoActionItem>,
    pub generated_at: String,
}

// ─── Competitor Change Types ─────────────────────────────────────────────────

/// A change event for a competitor.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CompetitorChange {
    pub id: Uuid,
    pub competitor_id: Uuid,
    pub competitor_name: String,
    pub change_type: String,
    pub title: String,
    pub description: String,
    pub detected_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    pub impact_score: f64,
}

/// Competitor with enhanced tracking fields.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CompetitorItem {
    pub id: Uuid,
    pub name: String,
    pub region: String,
    pub entity_type: String,
    pub threat_score: Option<f64>,
    pub capability_overlap: f64,
    pub market_overlap: f64,
    pub last_change_at: Option<String>,
    pub change_count_30d: i64,
    pub capabilities: Vec<String>,
    pub primary_markets: Vec<String>,
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
    pub public_bio: Option<String>,
    pub public_email: Option<String>,
    pub priority_vector: Option<serde_json::Value>,
    pub influence_score: Option<f64>,
    pub trigger_topics: Option<Vec<String>>,
    pub decision_style: Option<String>,
    pub risk_tolerance: Option<String>,
    pub change_appetite: Option<String>,
    pub communication_style: Option<String>,
    pub decision_mode: Option<String>,
    pub preferred_proof_type: Option<String>,
    pub pain_index: Option<f64>,
    pub change_risk: Option<f64>,
    pub role_drift_score: Option<f64>,
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
    pub country_code: String,
    pub role_family: String,
    pub priority_score: f64,
    pub engagement_status: String,
    pub updated_at: DateTime<Utc>,
    pub artifact_count: i64,
}

/// Minimal row used by the POI expansion engine as a seed.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct ExpansionSeedRow {
    pub id: Uuid,
    pub name: String,
    pub current_role: String,
    pub role_family: String,
    pub region: String,
    pub country_code: String,
    pub primary_org_id: Option<Uuid>,
    pub org_name: String,
    pub org_domain: Option<String>,
    pub is_competitor: bool,
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

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct CapabilityRow {
    pub id: Uuid,
    pub company_id: Option<Uuid>,
    pub site_id: Option<Uuid>,
    pub capability: String,
    pub proof_grade: Option<String>,
    pub evidence_urls: Option<Vec<String>>,
    pub first_seen: Option<DateTime<Utc>>,
    pub last_confirmed: Option<DateTime<Utc>>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct ProductFamilyRow {
    pub id: Uuid,
    pub company_id: Option<Uuid>,
    pub name: String,
    pub hs_codes: Option<Vec<String>>,
    pub tech_tags: Option<Vec<String>>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct LogisticsNodeRow {
    pub id: Uuid,
    pub name: String,
    pub node_type: Option<String>,
    pub country_code: Option<String>,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct RegulationRow {
    pub id: Uuid,
    pub name: String,
    pub regulation_type: Option<String>,
    pub jurisdiction: Option<String>,
    pub effective_date: Option<NaiveDate>,
    pub summary: Option<String>,
    pub source_url: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default, serde::Serialize, sqlx::FromRow)]
pub struct RegionCount {
    pub region: String,
    pub count: i64,
}

#[derive(Debug, Clone, Default, serde::Serialize, sqlx::FromRow)]
pub struct SeverityCount {
    pub severity: String,
    pub count: i64,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ActivityItem {
    pub label: String,
    pub count: u64,
    pub delta: i64,
}

/// Dashboard overview statistics.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct DashboardStats {
    pub total_companies: u64,
    pub total_persons: u64,
    pub total_warnings: u64,
    pub unacknowledged_warnings: u64,
    pub total_insights: u64,
    pub active_recipes: u64,
    pub new_insights_24h: u64,
    pub new_warnings_24h: u64,
    pub top_regions: Vec<RegionCount>,
    pub threat_distribution: Vec<SeverityCount>,
    pub recent_activity: Vec<ActivityItem>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct PageFingerprintRow {
    pub id: Uuid,
    pub url: String,
    pub content_hash: String,
    pub ts: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct FeatureRowRow {
    pub entity_id: String,
    pub entity_type: String,
    pub time_bucket: i64,
    pub bucket_size_days: i32,
    pub data: serde_json::Value,
    pub created_at: Option<DateTime<Utc>>,
}

/// Admin overview of crawl pipeline health.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AdminCrawlStatus {
    pub total_fingerprints: i64,
    pub domains_tracked: i64,
    pub latest_crawl_ts: Option<DateTime<Utc>>,
    pub crawl_stats: CrawlStats,
}

/// Admin overview of recipe performance.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AdminRecipePerformance {
    pub total_recipes: i64,
    pub production_count: i64,
    pub staging_count: i64,
    pub deprecated_count: i64,
    pub recipes: Vec<RecipeStatRow>,
}

// ─── New Row Types: Role History, Dossier Entries, Changes ───────────────────

/// A role history entry — one position a POI has held.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct RoleHistoryRow {
    pub id: Uuid,
    pub person_id: Uuid,
    pub org_id: Option<Uuid>,
    pub org_name: String,
    pub title: String,
    pub role_family: Option<String>,
    pub start_date: Option<DateTime<Utc>>,
    pub end_date: Option<DateTime<Utc>>,
    pub source_url: Option<String>,
    pub confidence: Option<f64>,
    pub verified: Option<bool>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

/// A persistent dossier entry — a verified fact attached to a person or company.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct DossierEntryRow {
    pub id: Uuid,
    pub entity_type: String,
    pub entity_id: Uuid,
    pub category: String,
    pub title: String,
    pub content: String,
    pub source_urls: Option<Vec<String>>,
    pub confidence: Option<f64>,
    pub verified: Option<bool>,
    pub supersedes_id: Option<Uuid>,
    pub valid_from: Option<DateTime<Utc>>,
    pub valid_until: Option<DateTime<Utc>>,
    pub author: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: Option<DateTime<Utc>>,
}

/// A changelog entry for a company.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct CompanyChangeRow {
    pub id: Uuid,
    pub company_id: Uuid,
    pub change_type: String,
    pub field_name: Option<String>,
    pub old_value: Option<String>,
    pub new_value: Option<String>,
    pub description: Option<String>,
    pub source_url: Option<String>,
    pub detected_at: Option<DateTime<Utc>>,
    pub confidence: Option<f64>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: Option<DateTime<Utc>>,
}

/// A changelog entry for a person/POI.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct PersonChangeRow {
    pub id: Uuid,
    pub person_id: Uuid,
    pub change_type: String,
    pub field_name: Option<String>,
    pub old_value: Option<String>,
    pub new_value: Option<String>,
    pub description: Option<String>,
    pub source_url: Option<String>,
    pub detected_at: Option<DateTime<Utc>>,
    pub confidence: Option<f64>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: Option<DateTime<Utc>>,
}

/// Admin overview of POI coverage.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AdminPoiCoverage {
    pub total_persons: i64,
    pub with_artifacts: i64,
    pub avg_artifacts_per_person: f64,
    pub total_artifacts: i64,
    pub poi_stats: PoiStats,
}

/// Company dossier: company + sites + capabilities + certifications + edges + dossier_entries + changes
#[derive(Debug, Clone, serde::Serialize)]
pub struct CompanyDossier {
    pub company: CompanyRow,
    pub sites: Vec<SiteRow>,
    pub capabilities: Vec<CapabilityRow>,
    pub certifications: Vec<CertificationRow>,
    pub product_families: Vec<ProductFamilyRow>,
    pub edges: Vec<EdgeRow>,
    pub dossier_entries: Vec<DossierEntryRow>,
    pub recent_changes: Vec<CompanyChangeRow>,
}

/// Person dossier: person + artifacts + observations + edges + role_history + dossier_entries + changes
#[derive(Debug, Clone, serde::Serialize)]
pub struct PersonDossier {
    pub person: PersonRow,
    pub artifacts: Vec<ArtifactRow>,
    pub observations: Vec<ObservationRow>,
    pub edges: Vec<EdgeRow>,
    pub role_history: Vec<RoleHistoryRow>,
    pub dossier_entries: Vec<DossierEntryRow>,
    pub recent_changes: Vec<PersonChangeRow>,
}

/// Engagement guide for a POI
#[derive(Debug, Clone, serde::Serialize)]
pub struct PersonEngagement {
    pub person_id: Uuid,
    pub name: String,
    pub role: Option<String>,
    pub priority_score: f64,
    pub engagement_status: String,
    pub co_appearances: Vec<EdgeRow>,
    pub recent_observations: Vec<ObservationRow>,
}

impl PgStore {
    // ── Single entity lookups ──────────────────────────────────

    pub async fn get_insight(&self, id: Uuid) -> Result<Option<InsightRow>> {
        let row = sqlx::query_as::<_, InsightRow>(
            "SELECT id, title, summary, insight_type, region, confidence,
                    evidence_urls, entity_ids, tags, created_at, updated_at
             FROM insights WHERE id = $1"
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// Fetch related warnings for a set of entity UUIDs.
    pub async fn get_warnings_by_entity_ids(&self, entity_ids: &[Uuid], limit: i64) -> Result<Vec<WarningRow>> {
        if entity_ids.is_empty() {
            return Ok(vec![]);
        }
        let rows = sqlx::query_as::<_, WarningRow>(
            "SELECT * FROM warnings WHERE entity_ids && $1 ORDER BY created_at DESC LIMIT $2"
        )
        .bind(entity_ids)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Fetch related insights for a set of entity UUIDs (excluding a given insight_id).
    pub async fn get_related_insights(&self, entity_ids: &[Uuid], exclude_id: Uuid, limit: i64) -> Result<Vec<InsightRow>> {
        if entity_ids.is_empty() {
            return Ok(vec![]);
        }
        let rows = sqlx::query_as::<_, InsightRow>(
            "SELECT id, title, summary, insight_type, region, confidence,
                    evidence_urls, entity_ids, tags, created_at, updated_at
             FROM insights WHERE entity_ids && $1 AND id != $2 ORDER BY created_at DESC LIMIT $3"
        )
        .bind(entity_ids)
        .bind(exclude_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Fetch insights related to a warning (by entity overlap).
    pub async fn get_insights_by_entity_ids(&self, entity_ids: &[Uuid], limit: i64) -> Result<Vec<InsightRow>> {
        if entity_ids.is_empty() {
            return Ok(vec![]);
        }
        let rows = sqlx::query_as::<_, InsightRow>(
            "SELECT id, title, summary, insight_type, region, confidence,
                    evidence_urls, entity_ids, tags, created_at, updated_at
             FROM insights WHERE entity_ids && $1 ORDER BY created_at DESC LIMIT $2"
        )
        .bind(entity_ids)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn get_warning(&self, id: Uuid) -> Result<Option<WarningRow>> {
        let row = sqlx::query_as::<_, WarningRow>(
            "SELECT * FROM warnings WHERE id = $1"
        )
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

    // ── Company dossier ──────────────────────────────────────

    pub async fn get_company_dossier(&self, company_id: Uuid) -> Result<Option<CompanyDossier>> {
        let company = match self.get_company(company_id).await? {
            Some(c) => c,
            None => return Ok(None),
        };
        let sites = self.get_sites_for_company(company_id).await.unwrap_or_default();
        let capabilities = self.list_capabilities(Some(company_id), 200, 0).await.unwrap_or_default();
        let certifications = self.get_certifications_for_company(company_id).await.unwrap_or_default();
        let product_families = self.list_product_families(Some(company_id), 200, 0).await.unwrap_or_default();
        let edges = self.get_edges_from(company_id, "company").await.unwrap_or_default();
        let dossier_entries = self.get_dossier_entries("company", company_id, None, 200).await.unwrap_or_default();
        let recent_changes = self.get_company_changes(company_id, 50).await.unwrap_or_default();
        Ok(Some(CompanyDossier { company, sites, capabilities, certifications, product_families, edges, dossier_entries, recent_changes }))
    }

    // ── Person dossier ──────────────────────────────────────

    pub async fn get_person_dossier(&self, person_id: Uuid) -> Result<Option<PersonDossier>> {
        let person = match self.get_person(person_id).await? {
            Some(p) => p,
            None => return Ok(None),
        };
        let artifacts = self.get_artifacts_for_person(person_id, 200).await.unwrap_or_default();
        let observations = self.get_observations_by_entity(person_id, 200).await.unwrap_or_default();
        let edges = self.get_edges_from(person_id, "person").await.unwrap_or_default();
        let role_history = self.get_role_history(person_id, 100).await.unwrap_or_default();
        let dossier_entries = self.get_dossier_entries("person", person_id, None, 200).await.unwrap_or_default();
        let recent_changes = self.get_person_changes(person_id, 50).await.unwrap_or_default();
        Ok(Some(PersonDossier { person, artifacts, observations, edges, role_history, dossier_entries, recent_changes }))
    }

    pub async fn get_person_engagement(&self, person_id: Uuid) -> Result<Option<PersonEngagement>> {
        let person = match self.get_person(person_id).await? {
            Some(p) => p,
            None => return Ok(None),
        };
        let co_appearances = self.get_edges_from(person_id, "person").await.unwrap_or_default();
        let recent_observations = sqlx::query_as::<_, ObservationRow>(
            "SELECT * FROM observations WHERE entity_id = $1 ORDER BY ts_utc DESC LIMIT 20"
        )
        .bind(person_id)
        .fetch_all(&self.pool)
        .await
        .unwrap_or_default();
        let influence = person.influence_score.unwrap_or(0.0);
        let engagement_status = person
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.get("engagement_status"))
            .and_then(|value| value.as_str())
            .map(|value| value.to_string())
            .unwrap_or_else(|| "untracked".to_string());
        Ok(Some(PersonEngagement {
            person_id,
            name: person.name.clone(),
            role: person.current_role.clone(),
            priority_score: influence,
            engagement_status,
            co_appearances,
            recent_observations,
        }))
    }

    // ── Competitors ──────────────────────────────────────

    pub async fn list_competitors(&self, limit: i64, offset: i64) -> Result<Vec<CompanyRow>> {
        let rows = sqlx::query_as::<_, CompanyRow>(
            "SELECT * FROM companies WHERE (metadata->>'is_competitor')::boolean = true ORDER BY name ASC LIMIT $1 OFFSET $2"
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn count_competitors(&self) -> Result<i64> {
        let (count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM companies WHERE (metadata->>'is_competitor')::boolean = true"
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(count)
    }

    pub async fn get_competitor_changes(&self, company_id: Uuid, limit: i64) -> Result<Vec<CompanyChangeRow>> {
        let rows = sqlx::query_as::<_, CompanyChangeRow>(
            "SELECT * FROM company_changes WHERE company_id = $1 ORDER BY detected_at DESC LIMIT $2"
        )
        .bind(company_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    // ─── Role History ────────────────────────────────────────────────────

    /// Insert a new role history entry for a POI.
    pub async fn insert_role_history(
        &self,
        person_id: Uuid,
        org_id: Option<Uuid>,
        org_name: &str,
        title: &str,
        role_family: Option<&str>,
        start_date: Option<DateTime<Utc>>,
        end_date: Option<DateTime<Utc>>,
        source_url: Option<&str>,
        confidence: f64,
    ) -> Result<Uuid> {
        let id = Uuid::new_v4();
        sqlx::query(
            r#"INSERT INTO role_history
               (id, person_id, org_id, org_name, title, role_family,
                start_date, end_date, source_url, confidence)
               VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)"#,
        )
        .bind(id)
        .bind(person_id)
        .bind(org_id)
        .bind(org_name)
        .bind(title)
        .bind(role_family)
        .bind(start_date)
        .bind(end_date)
        .bind(source_url)
        .bind(confidence)
        .execute(&self.pool)
        .await?;
        Ok(id)
    }

    /// Get full role history for a POI (chronological, most recent first).
    pub async fn get_role_history(&self, person_id: Uuid, limit: i64) -> Result<Vec<RoleHistoryRow>> {
        let rows = sqlx::query_as::<_, RoleHistoryRow>(
            "SELECT * FROM role_history WHERE person_id = $1 ORDER BY start_date DESC NULLS FIRST LIMIT $2"
        )
        .bind(person_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Close a role history entry (set end_date).
    pub async fn close_role_history_entry(&self, entry_id: Uuid, end_date: DateTime<Utc>) -> Result<bool> {
        let result = sqlx::query(
            "UPDATE role_history SET end_date = $2, updated_at = now() WHERE id = $1 AND end_date IS NULL"
        )
        .bind(entry_id)
        .bind(end_date)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Get the current (open) role for a person.
    pub async fn get_current_role(&self, person_id: Uuid) -> Result<Option<RoleHistoryRow>> {
        let row = sqlx::query_as::<_, RoleHistoryRow>(
            "SELECT * FROM role_history WHERE person_id = $1 AND end_date IS NULL ORDER BY start_date DESC LIMIT 1"
        )
        .bind(person_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    // ─── Dossier Entries ─────────────────────────────────────────────────

    /// Insert a new dossier entry for a person or company.
    pub async fn insert_dossier_entry(
        &self,
        entity_type: &str,
        entity_id: Uuid,
        category: &str,
        title: &str,
        content: &str,
        source_urls: &[String],
        confidence: f64,
        author: &str,
        supersedes_id: Option<Uuid>,
    ) -> Result<Uuid> {
        let id = Uuid::new_v4();
        // If superseding, mark old entry as no longer current
        if let Some(old_id) = supersedes_id {
            sqlx::query(
                "UPDATE dossier_entries SET valid_until = now() WHERE id = $1 AND valid_until IS NULL"
            )
            .bind(old_id)
            .execute(&self.pool)
            .await?;
        }
        sqlx::query(
            r#"INSERT INTO dossier_entries
               (id, entity_type, entity_id, category, title, content,
                source_urls, confidence, author, supersedes_id)
               VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)"#,
        )
        .bind(id)
        .bind(entity_type)
        .bind(entity_id)
        .bind(category)
        .bind(title)
        .bind(content)
        .bind(source_urls)
        .bind(confidence)
        .bind(author)
        .bind(supersedes_id)
        .execute(&self.pool)
        .await?;
        Ok(id)
    }

    /// Get current dossier entries for an entity, optionally filtered by category.
    pub async fn get_dossier_entries(
        &self,
        entity_type: &str,
        entity_id: Uuid,
        category: Option<&str>,
        limit: i64,
    ) -> Result<Vec<DossierEntryRow>> {
        let rows = match category {
            Some(cat) => {
                sqlx::query_as::<_, DossierEntryRow>(
                    "SELECT * FROM dossier_entries WHERE entity_type = $1 AND entity_id = $2 AND category = $3 AND valid_until IS NULL ORDER BY created_at DESC LIMIT $4"
                )
                .bind(entity_type)
                .bind(entity_id)
                .bind(cat)
                .bind(limit)
                .fetch_all(&self.pool)
                .await?
            }
            None => {
                sqlx::query_as::<_, DossierEntryRow>(
                    "SELECT * FROM dossier_entries WHERE entity_type = $1 AND entity_id = $2 AND valid_until IS NULL ORDER BY created_at DESC LIMIT $3"
                )
                .bind(entity_type)
                .bind(entity_id)
                .bind(limit)
                .fetch_all(&self.pool)
                .await?
            }
        };
        Ok(rows)
    }

    /// Get the full history of a dossier entry (all versions).
    pub async fn get_dossier_entry_history(&self, entry_id: Uuid) -> Result<Vec<DossierEntryRow>> {
        // Walk the supersedes chain backward
        let rows = sqlx::query_as::<_, DossierEntryRow>(
            r#"WITH RECURSIVE chain AS (
                SELECT * FROM dossier_entries WHERE id = $1
                UNION ALL
                SELECT de.* FROM dossier_entries de
                JOIN chain c ON de.id = c.supersedes_id
            )
            SELECT * FROM chain ORDER BY created_at DESC"#
        )
        .bind(entry_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Mark a dossier entry as verified by a human analyst.
    pub async fn verify_dossier_entry(&self, entry_id: Uuid) -> Result<bool> {
        let result = sqlx::query(
            "UPDATE dossier_entries SET verified = TRUE WHERE id = $1"
        )
        .bind(entry_id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    // ─── Company Changes ─────────────────────────────────────────────────

    /// Record a change detected in a company's profile.
    pub async fn insert_company_change(
        &self,
        company_id: Uuid,
        change_type: &str,
        field_name: Option<&str>,
        old_value: Option<&str>,
        new_value: Option<&str>,
        description: Option<&str>,
        source_url: Option<&str>,
        confidence: f64,
    ) -> Result<Uuid> {
        let id = Uuid::new_v4();
        sqlx::query(
            r#"INSERT INTO company_changes
               (id, company_id, change_type, field_name, old_value, new_value,
                description, source_url, confidence)
               VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)"#,
        )
        .bind(id)
        .bind(company_id)
        .bind(change_type)
        .bind(field_name)
        .bind(old_value)
        .bind(new_value)
        .bind(description)
        .bind(source_url)
        .bind(confidence)
        .execute(&self.pool)
        .await?;
        Ok(id)
    }

    /// Get changes for a company, ordered by detection time.
    pub async fn get_company_changes(&self, company_id: Uuid, limit: i64) -> Result<Vec<CompanyChangeRow>> {
        let rows = sqlx::query_as::<_, CompanyChangeRow>(
            "SELECT * FROM company_changes WHERE company_id = $1 ORDER BY detected_at DESC LIMIT $2"
        )
        .bind(company_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    // ─── Person Changes ──────────────────────────────────────────────────

    /// Record a change detected in a POI's profile.
    pub async fn insert_person_change(
        &self,
        person_id: Uuid,
        change_type: &str,
        field_name: Option<&str>,
        old_value: Option<&str>,
        new_value: Option<&str>,
        description: Option<&str>,
        source_url: Option<&str>,
        confidence: f64,
    ) -> Result<Uuid> {
        let id = Uuid::new_v4();
        sqlx::query(
            r#"INSERT INTO person_changes
               (id, person_id, change_type, field_name, old_value, new_value,
                description, source_url, confidence)
               VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)"#,
        )
        .bind(id)
        .bind(person_id)
        .bind(change_type)
        .bind(field_name)
        .bind(old_value)
        .bind(new_value)
        .bind(description)
        .bind(source_url)
        .bind(confidence)
        .execute(&self.pool)
        .await?;
        Ok(id)
    }

    /// Get changes for a person/POI, ordered by detection time.
    pub async fn get_person_changes(&self, person_id: Uuid, limit: i64) -> Result<Vec<PersonChangeRow>> {
        let rows = sqlx::query_as::<_, PersonChangeRow>(
            "SELECT * FROM person_changes WHERE person_id = $1 ORDER BY detected_at DESC LIMIT $2"
        )
        .bind(person_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Count role changes detected since a given date (for POI stats).
    pub async fn count_role_changes_since(&self, since: DateTime<Utc>) -> Result<i64> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM person_changes WHERE change_type IN ('job_change', 'role_change', 'org_change') AND detected_at >= $1"
        )
        .bind(since)
        .fetch_one(&self.pool)
        .await?;
        Ok(count)
    }

    // ── Graph operations ──────────────────────────────────

    pub async fn get_neighborhood(&self, node_id: Uuid, limit: u32) -> Result<Vec<EdgeRow>> {
        let rows = sqlx::query_as::<_, EdgeRow>(
            "SELECT * FROM graph_edges WHERE source_id = $1 OR target_id = $1 ORDER BY weight DESC NULLS LAST LIMIT $2"
        )
        .bind(node_id)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn get_path_edges(&self, from_id: Uuid, to_id: Uuid) -> Result<Vec<EdgeRow>> {
        // Simple: return edges involving either node + edges connecting them
        // For a real shortest-path, we'd use a recursive CTE or a graph library
        let rows = sqlx::query_as::<_, EdgeRow>(
            "SELECT DISTINCT * FROM graph_edges WHERE (source_id = $1 OR target_id = $1 OR source_id = $2 OR target_id = $2) ORDER BY weight DESC NULLS LAST LIMIT 100"
        )
        .bind(from_id)
        .bind(to_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    // ── Recipes staging/promote ───────────────────────

    pub async fn list_staging_recipes(&self, limit: i64, offset: i64) -> Result<Vec<RecipeStatRow>> {
        let limit = clamp_limit(limit);
        let offset = offset.max(0);
        let rows = sqlx::query_as::<_, RecipeStatRow>(
            r#"SELECT
                r.code AS recipe_code,
                COALESCE(COUNT(w.id), 0) AS fired_count,
                MAX(w.created_at) AS last_fired,
                MIN(w.created_at) AS first_fired,
                COALESCE(COUNT(*) FILTER (WHERE w.id IS NOT NULL AND NOT w.acknowledged), 0) AS active_count
               FROM recipes r
               LEFT JOIN warnings w ON w.recipe_code = r.code
               WHERE r.status = 'staging'
               GROUP BY r.code
               ORDER BY r.code ASC
               LIMIT $1 OFFSET $2"#
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn count_staging_recipes(&self) -> Result<i64> {
        let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM recipes WHERE status = 'staging'")
            .fetch_one(&self.pool)
            .await?;
        Ok(count)
    }

    pub async fn promote_recipe(&self, recipe_code: &str) -> Result<bool> {
        let result = sqlx::query(
            "UPDATE recipes SET status = 'production', updated_at = now() WHERE code = $1 AND status = 'staging'"
        )
        .bind(recipe_code)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn deprecate_recipe(&self, recipe_code: &str) -> Result<bool> {
        let result = sqlx::query(
            "UPDATE recipes SET status = 'deprecated', updated_at = now() WHERE code = $1 AND status IN ('staging', 'production')"
        )
        .bind(recipe_code)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    // ── Admin endpoints ──────────────────────────────────

    pub async fn get_admin_crawl_status(&self) -> Result<AdminCrawlStatus> {
        let (total_fp,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM page_fingerprints")
            .fetch_one(&self.pool).await.unwrap_or((0,));
        let (domains,): (i64,) = sqlx::query_as("SELECT COUNT(DISTINCT split_part(url, '/', 3)) FROM page_fingerprints")
            .fetch_one(&self.pool).await.unwrap_or((0,));
        let latest: Option<(DateTime<Utc>,)> = sqlx::query_as("SELECT MAX(ts) FROM page_fingerprints")
            .fetch_optional(&self.pool).await.ok().flatten();
        let latest_ts = latest.and_then(|l| Some(l.0));
        let crawl_stats = self.get_crawl_stats(Utc::now() - chrono::Duration::days(7)).await.unwrap_or_default();
        Ok(AdminCrawlStatus { total_fingerprints: total_fp, domains_tracked: domains, latest_crawl_ts: latest_ts, crawl_stats })
    }

    pub async fn get_admin_recipe_performance(&self) -> Result<AdminRecipePerformance> {
        let (total,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM recipes")
            .fetch_one(&self.pool).await.unwrap_or((0,));
        let (prod,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM recipes WHERE status = 'production'")
            .fetch_one(&self.pool).await.unwrap_or((0,));
        let (staging,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM recipes WHERE status = 'staging'")
            .fetch_one(&self.pool).await.unwrap_or((0,));
        let (deprecated,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM recipes WHERE status = 'deprecated'")
            .fetch_one(&self.pool).await.unwrap_or((0,));
        let recipes = self.get_recipe_stats().await.unwrap_or_default();
        Ok(AdminRecipePerformance { total_recipes: total, production_count: prod, staging_count: staging, deprecated_count: deprecated, recipes })
    }

    pub async fn get_admin_poi_coverage(&self) -> Result<AdminPoiCoverage> {
        let (total_persons,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM persons")
            .fetch_one(&self.pool).await.unwrap_or((0,));
        let (with_artifacts,): (i64,) = sqlx::query_as("SELECT COUNT(DISTINCT person_id) FROM poi_artifacts")
            .fetch_one(&self.pool).await.unwrap_or((0,));
        let (total_artifacts,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM poi_artifacts")
            .fetch_one(&self.pool).await.unwrap_or((0,));
        let avg = if total_persons > 0 { total_artifacts as f64 / total_persons as f64 } else { 0.0 };
        let poi_stats = self.get_poi_stats(Utc::now() - chrono::Duration::days(30)).await.unwrap_or_default();
        Ok(AdminPoiCoverage { total_persons, with_artifacts, avg_artifacts_per_person: avg, total_artifacts, poi_stats })
    }

    // ── Security scan write methods ──────────────────────

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
                 updated_at = now()"#
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
                 catalog_fetched_at = EXCLUDED.catalog_fetched_at"#
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
                 active = EXCLUDED.active"#
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

    // ── Worker trigger queue ────────────────────────────

    /// Enqueue a manual job trigger from the API.
    pub async fn queue_job_trigger(&self, job_kind: &str) -> Result<String> {
        let (id,): (Uuid,) = sqlx::query_as(
            "INSERT INTO worker_trigger_queue (job_kind) VALUES ($1) RETURNING id"
        )
        .bind(job_kind)
        .fetch_one(&self.pool)
        .await?;
        Ok(id.to_string())
    }

    /// Claim the oldest unclaimed job trigger (atomic via UPDATE ... RETURNING).
    /// Returns `(trigger_id, job_kind)` or `None` if the queue is empty.
    pub async fn pop_job_trigger(&self) -> Result<Option<(String, String)>> {
        let row: Option<(Uuid, String)> = sqlx::query_as(
            r#"UPDATE worker_trigger_queue
               SET claimed_at = now()
               WHERE id = (
                   SELECT id FROM worker_trigger_queue
                   WHERE claimed_at IS NULL
                   ORDER BY requested_at
                   LIMIT 1
               )
               RETURNING id, job_kind"#
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|(id, kind)| (id.to_string(), kind)))
    }

    /// Mark a trigger as completed (or failed with an error message).
    pub async fn complete_job_trigger(&self, trigger_id: &str, error: Option<&str>) -> Result<()> {
        let uid = Uuid::parse_str(trigger_id)?;
        sqlx::query(
            "UPDATE worker_trigger_queue SET completed_at = now(), error = $2 WHERE id = $1"
        )
        .bind(uid)
        .bind(error)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // ── Weekly memo ──────────────────────────────────

    /// Get the most recent weekly memo from the database.
    pub async fn get_weekly_memo_full(&self) -> Result<Option<WeeklyMemo>> {
        let row: Option<(Uuid, String, chrono::NaiveDate, chrono::NaiveDate, String, serde_json::Value, serde_json::Value, serde_json::Value, DateTime<Utc>)> = sqlx::query_as(
            r#"SELECT id, title, week_start, week_end, executive_summary, sections, key_metrics, action_items, generated_at
               FROM weekly_memos
               ORDER BY week_start DESC
               LIMIT 1"#
        )
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some((id, title, week_start, week_end, executive_summary, sections_json, metrics_json, actions_json, generated_at)) => {
                let sections: Vec<WeeklyMemoSection> = serde_json::from_value(sections_json).unwrap_or_default();
                let key_metrics: WeeklyMemoKeyMetrics = serde_json::from_value(metrics_json).unwrap_or(WeeklyMemoKeyMetrics {
                    warnings_total: 0,
                    warnings_critical: 0,
                    insights_generated: 0,
                    companies_monitored: 0,
                    pois_tracked: 0,
                });
                let action_items: Vec<WeeklyMemoActionItem> = serde_json::from_value(actions_json).unwrap_or_default();

                Ok(Some(WeeklyMemo {
                    id,
                    title,
                    week_start: week_start.to_string(),
                    week_end: week_end.to_string(),
                    executive_summary,
                    sections,
                    key_metrics,
                    action_items,
                    generated_at: generated_at.to_rfc3339(),
                }))
            }
            None => Ok(None),
        }
    }

    /// Legacy method for backward compatibility
    pub async fn get_weekly_memo(&self) -> Result<WeeklySummaryStats> {
        self.get_weekly_summary_stats(Utc::now() - chrono::Duration::days(7)).await
    }

    /// List all weekly memos ordered newest-first with pagination.
    pub async fn list_weekly_memos(&self, limit: i64, offset: i64) -> Result<(Vec<WeeklyMemo>, i64)> {
        let (total,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM weekly_memos")
            .fetch_one(&self.pool)
            .await
            .unwrap_or((0,));

        let rows: Vec<(Uuid, String, chrono::NaiveDate, chrono::NaiveDate, String, serde_json::Value, serde_json::Value, serde_json::Value, DateTime<Utc>)> = sqlx::query_as(
            r#"SELECT id, title, week_start, week_end, executive_summary, sections, key_metrics, action_items, generated_at
               FROM weekly_memos
               ORDER BY week_start DESC
               LIMIT $1 OFFSET $2"#
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await
        .unwrap_or_default();

        let memos: Vec<WeeklyMemo> = rows
            .into_iter()
            .map(|(id, title, week_start, week_end, executive_summary, sections_json, metrics_json, actions_json, generated_at)| {
                let sections: Vec<WeeklyMemoSection> = serde_json::from_value(sections_json).unwrap_or_default();
                let key_metrics: WeeklyMemoKeyMetrics = serde_json::from_value(metrics_json).unwrap_or(WeeklyMemoKeyMetrics {
                    warnings_total: 0,
                    warnings_critical: 0,
                    insights_generated: 0,
                    companies_monitored: 0,
                    pois_tracked: 0,
                });
                let action_items: Vec<WeeklyMemoActionItem> = serde_json::from_value(actions_json).unwrap_or_default();
                WeeklyMemo {
                    id,
                    title,
                    week_start: week_start.to_string(),
                    week_end: week_end.to_string(),
                    executive_summary,
                    sections,
                    key_metrics,
                    action_items,
                    generated_at: generated_at.to_rfc3339(),
                }
            })
            .collect();

        Ok((memos, total))
    }

    /// Upsert a weekly memo for the given week window.
    ///
    /// On conflict (same `week_start`/`week_end`) the existing row is updated in place.
    /// All JSONB fields are passed as raw `serde_json::Value` to avoid coupling to
    /// internal memo struct versions.
    pub async fn upsert_weekly_memo(
        &self,
        title: &str,
        week_start: chrono::NaiveDate,
        week_end: chrono::NaiveDate,
        executive_summary: &str,
        sections_json: serde_json::Value,
        key_metrics_json: serde_json::Value,
        action_items_json: serde_json::Value,
    ) -> Result<Uuid> {
        let id = Uuid::new_v4();
        let returned: (Uuid,) = sqlx::query_as(
            r#"INSERT INTO weekly_memos
               (id, title, week_start, week_end, executive_summary,
                sections, key_metrics, action_items, generated_at, updated_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, now(), now())
               ON CONFLICT (week_start, week_end) DO UPDATE SET
                 title = EXCLUDED.title,
                 executive_summary = EXCLUDED.executive_summary,
                 sections = EXCLUDED.sections,
                 key_metrics = EXCLUDED.key_metrics,
                 action_items = EXCLUDED.action_items,
                 generated_at = now(),
                 updated_at = now()
               RETURNING id"#,
        )
        .bind(id)
        .bind(title)
        .bind(week_start)
        .bind(week_end)
        .bind(executive_summary)
        .bind(&sections_json)
        .bind(&key_metrics_json)
        .bind(&action_items_json)
        .fetch_one(&self.pool)
        .await?;
        Ok(returned.0)
    }

    // ── Competitor changes ──────────────────────────────────

    /// Get all competitor changes with pagination (new format).
    pub async fn get_all_competitor_changes_paged(&self, page: i64, per_page: i64) -> Result<(Vec<CompetitorChange>, i64)> {
        let offset = (page - 1) * per_page;

        let (total,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM competitor_changes")
            .fetch_one(&self.pool)
            .await
            .unwrap_or((0,));

        let rows: Vec<(Uuid, Uuid, String, String, String, String, DateTime<Utc>, Option<String>, f32)> = sqlx::query_as(
            r#"SELECT cc.id, cc.competitor_id, c.name, cc.change_type, cc.title, cc.description, cc.detected_at, cc.source_url, cc.impact_score
               FROM competitor_changes cc
               JOIN companies c ON c.id = cc.competitor_id
               ORDER BY cc.detected_at DESC
               LIMIT $1 OFFSET $2"#
        )
        .bind(per_page)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;

        let changes: Vec<CompetitorChange> = rows.into_iter().map(|(id, competitor_id, competitor_name, change_type, title, description, detected_at, source_url, impact_score)| {
            CompetitorChange {
                id,
                competitor_id,
                competitor_name,
                change_type,
                title,
                description,
                detected_at: detected_at.to_rfc3339(),
                source_url,
                impact_score: impact_score as f64,
            }
        }).collect();

        Ok((changes, total))
    }

    /// Get competitor changes for a specific competitor.
    pub async fn get_competitor_changes_by_id(&self, competitor_id: Uuid, page: i64, per_page: i64) -> Result<(Vec<CompetitorChange>, i64)> {
        let offset = (page - 1) * per_page;

        let (total,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM competitor_changes WHERE competitor_id = $1")
            .bind(competitor_id)
            .fetch_one(&self.pool)
            .await
            .unwrap_or((0,));

        let rows: Vec<(Uuid, Uuid, String, String, String, String, DateTime<Utc>, Option<String>, f32)> = sqlx::query_as(
            r#"SELECT cc.id, cc.competitor_id, c.name, cc.change_type, cc.title, cc.description, cc.detected_at, cc.source_url, cc.impact_score
               FROM competitor_changes cc
               JOIN companies c ON c.id = cc.competitor_id
               WHERE cc.competitor_id = $1
               ORDER BY cc.detected_at DESC
               LIMIT $2 OFFSET $3"#
        )
        .bind(competitor_id)
        .bind(per_page)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;

        let changes: Vec<CompetitorChange> = rows.into_iter().map(|(id, competitor_id, competitor_name, change_type, title, description, detected_at, source_url, impact_score)| {
            CompetitorChange {
                id,
                competitor_id,
                competitor_name,
                change_type,
                title,
                description,
                detected_at: detected_at.to_rfc3339(),
                source_url,
                impact_score: impact_score as f64,
            }
        }).collect();

        Ok((changes, total))
    }

    /// Get competitors with enhanced tracking fields.
    pub async fn get_competitors_enhanced(&self, page: i64, per_page: i64) -> Result<(Vec<CompetitorItem>, i64)> {
        let offset = (page - 1) * per_page;

        let (total,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM companies WHERE is_competitor = true")
            .fetch_one(&self.pool)
            .await
            .unwrap_or((0,));

        let rows: Vec<(Uuid, String, Option<String>, Option<String>, Option<f64>, Option<Vec<String>>)> = sqlx::query_as(
            r#"SELECT id, name, region, company_type, risk_score, industry_tags
               FROM companies
               WHERE is_competitor = true
               ORDER BY risk_score DESC NULLS LAST, name
               LIMIT $1 OFFSET $2"#
        )
        .bind(per_page)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;

        let mut items = Vec::with_capacity(rows.len());
        for (id, name, region, company_type, risk_score, industry_tags) in rows {
            // Get last change and change count
            let change_info: Option<(DateTime<Utc>, i64)> = sqlx::query_as(
                r#"SELECT MAX(detected_at), COUNT(*)
                   FROM competitor_changes
                   WHERE competitor_id = $1 AND detected_at > NOW() - INTERVAL '30 days'"#
            )
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .ok()
            .flatten();

            let (last_change_at, change_count_30d) = change_info.map(|(ts, cnt)| (Some(ts.to_rfc3339()), cnt)).unwrap_or((None, 0));

            items.push(CompetitorItem {
                id,
                name,
                region: region.unwrap_or_else(|| "Unknown".to_string()),
                entity_type: company_type.unwrap_or_else(|| "EMS".to_string()),
                threat_score: risk_score,
                capability_overlap: 0.0, // Placeholder - would need capability analysis
                market_overlap: 0.0,     // Placeholder - would need market analysis
                last_change_at,
                change_count_30d,
                capabilities: industry_tags.unwrap_or_default(),
                primary_markets: vec![],  // Placeholder - would need market data
            });
        }

        Ok((items, total))
    }

    // ── Security sub-endpoints ──────────────────────────

    pub async fn get_dns_posture_entries(&self, limit: i64) -> Result<Vec<ObservationRow>> {
        let rows = sqlx::query_as::<_, ObservationRow>(
            "SELECT * FROM observations WHERE observation_type IN ('dns_posture', 'dmarc_check', 'spf_check', 'dkim_check') ORDER BY ts_utc DESC LIMIT $1"
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn get_lookalike_domains(&self, limit: i64) -> Result<Vec<ObservationRow>> {
        let rows = sqlx::query_as::<_, ObservationRow>(
            "SELECT * FROM observations WHERE observation_type IN ('lookalike_domain', 'typosquat', 'ct_cert_lookalike') ORDER BY ts_utc DESC LIMIT $1"
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn get_kev_relevance(&self, limit: i64) -> Result<Vec<ObservationRow>> {
        let rows = sqlx::query_as::<_, ObservationRow>(
            "SELECT * FROM observations WHERE observation_type IN ('kev_match', 'cve_relevance', 'psirt_advisory') ORDER BY ts_utc DESC LIMIT $1"
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }
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
            public_bio: None,
            public_email: None,
            trigger_topics: None,
            decision_style: None,
            risk_tolerance: None,
            change_appetite: None,
            communication_style: None,
            decision_mode: None,
            preferred_proof_type: None,
            pain_index: None,
            change_risk: None,
            role_drift_score: None,
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
