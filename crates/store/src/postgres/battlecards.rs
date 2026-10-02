use super::*;

use chrono::{DateTime, Utc};
use uuid::Uuid;

/// Row type for the battlecards table — mirrors the DB schema exactly.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct BattlecardRow {
    pub id: Uuid,
    pub our_company_id: Uuid,
    pub competitor_id: Uuid,
    pub title: String,
    pub status: String,
    pub positioning: Option<serde_json::Value>,
    pub pricing: Option<serde_json::Value>,
    pub feature_matrix: Option<serde_json::Value>,
    pub strengths: Option<serde_json::Value>,
    pub weaknesses: Option<serde_json::Value>,
    pub objection_handlers: Option<serde_json::Value>,
    pub kill_shots: Option<serde_json::Value>,
    pub recent_news: Option<serde_json::Value>,
    pub win_loss: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub updated_by: Option<String>,
    pub regenerated_at: Option<DateTime<Utc>>,
}

/// The set of valid JSONB section names for battlecards.
const VALID_SECTIONS: &[&str] = &[
    "positioning",
    "pricing",
    "feature_matrix",
    "strengths",
    "weaknesses",
    "objection_handlers",
    "kill_shots",
    "recent_news",
    "win_loss",
];

/// Canonical battlecard lifecycle states (migration 025 CHECK constraint).
pub const BATTLECARD_STATUSES: &[&str] = &["draft", "published", "archived"];

/// Maximum stored title length (migration 025: `VARCHAR(255)`).
pub const BATTLECARD_TITLE_MAX_CHARS: usize = 255;

const BATTLECARD_COLUMNS: &str = "id, our_company_id, competitor_id, title, status, \
     positioning, pricing, feature_matrix, strengths, weaknesses, \
     objection_handlers, kill_shots, recent_news, win_loss, \
     created_at, updated_at, updated_by, regenerated_at";

/// Result of [`PgStore::create_battlecard`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreateBattlecardOutcome {
    /// A new draft battlecard was inserted.
    Created(Uuid),
    /// A battlecard for this (our company, competitor) pair already exists.
    Duplicate(Uuid),
    /// One of the referenced companies does not exist.
    UnknownCompany,
}

/// Result of [`PgStore::update_battlecard_details`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BattlecardWriteOutcome {
    /// The battlecard was updated.
    Updated,
    /// No battlecard with this id exists.
    NotFound,
    /// The battlecard changed after the caller read it (`expected_updated_at`
    /// did not match); nothing was written.
    Conflict,
}

/// A company offered in the battlecard editor's company pickers.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct BattlecardCompanyOption {
    pub id: Uuid,
    pub name: String,
    pub is_competitor: bool,
}

/// Validate that `section` is a legal battlecard section name.
fn validate_section(section: &str) -> Result<()> {
    if !VALID_SECTIONS.contains(&section) {
        return Err(anyhow::anyhow!(
            "invalid section name '{}'; expected one of: {}",
            section,
            VALID_SECTIONS.join(", ")
        ));
    }
    Ok(())
}

fn push_battlecard_filters<'a>(
    qb: &mut sqlx::QueryBuilder<'a, sqlx::Postgres>,
    status: Option<&'a str>,
    competitor_id: Option<Uuid>,
) {
    if let Some(status) = status {
        qb.push(" AND status = ");
        qb.push_bind(status);
    }
    if let Some(competitor_id) = competitor_id {
        qb.push(" AND competitor_id = ");
        qb.push_bind(competitor_id);
    }
}

fn clamp_page_per_page(page: u32, per_page: u32) -> (i64, i64) {
    let limit = per_page.clamp(1, 100) as i64;
    let offset = ((page.max(1) - 1) as i64).saturating_mul(limit);
    (limit, offset)
}

/// A JSON `null` section is stored as SQL `NULL` ("not generated"), never as a
/// JSONB `null` literal.
fn section_param(data: &serde_json::Value) -> Option<&serde_json::Value> {
    (!data.is_null()).then_some(data)
}

/// One transactional battlecard write (details edit or regeneration).
struct BattlecardWrite<'a> {
    title: Option<&'a str>,
    status: Option<&'a str>,
    sections: &'a [(&'a str, serde_json::Value)],
    updated_by: &'a str,
    mark_regenerated: bool,
    expected_updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl PgStore {
    /// List battlecards with optional status and competitor_id filters, paginated.
    pub async fn list_battlecards(
        &self,
        status: Option<&str>,
        competitor_id: Option<Uuid>,
        page: u32,
        per_page: u32,
    ) -> Result<Vec<BattlecardRow>> {
        let (limit, offset) = clamp_page_per_page(page, per_page);

        let mut qb = sqlx::QueryBuilder::<sqlx::Postgres>::new("SELECT ");
        qb.push(BATTLECARD_COLUMNS);
        qb.push(" FROM battlecards WHERE TRUE");
        push_battlecard_filters(&mut qb, status, competitor_id);
        qb.push(" ORDER BY updated_at DESC, id LIMIT ");
        qb.push_bind(limit);
        qb.push(" OFFSET ");
        qb.push_bind(offset);

        let rows = qb
            .build_query_as::<BattlecardRow>()
            .fetch_all(&self.pool)
            .await?;
        Ok(rows)
    }

    /// Count battlecards matching optional filters.
    pub async fn count_battlecards(
        &self,
        status: Option<&str>,
        competitor_id: Option<Uuid>,
    ) -> Result<i64> {
        let mut qb =
            sqlx::QueryBuilder::<sqlx::Postgres>::new("SELECT COUNT(*) FROM battlecards WHERE TRUE");
        push_battlecard_filters(&mut qb, status, competitor_id);
        let (count,): (i64,) = qb.build_query_as().fetch_one(&self.pool).await?;
        Ok(count)
    }

    /// Battlecard totals per lifecycle status (statuses with no rows are
    /// omitted).
    pub async fn count_battlecards_by_status(&self) -> Result<Vec<(String, i64)>> {
        let rows: Vec<(String, i64)> = sqlx::query_as(
            "SELECT status, COUNT(*)::bigint FROM battlecards GROUP BY status ORDER BY status",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Get several battlecards by id, preserving the requested order and
    /// skipping ids that do not exist.
    pub async fn get_battlecards_by_ids(&self, ids: &[Uuid]) -> Result<Vec<BattlecardRow>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        // The ordinality columns must not shadow battlecard column names:
        // BATTLECARD_COLUMNS is unqualified.
        let sql = format!(
            "SELECT {BATTLECARD_COLUMNS} FROM battlecards b \
             JOIN unnest($1::uuid[]) WITH ORDINALITY AS req(requested_id, requested_ord) \
               ON req.requested_id = b.id \
             ORDER BY req.requested_ord"
        );
        let rows = sqlx::query_as::<_, BattlecardRow>(&sql)
            .bind(ids)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows)
    }

    /// Companies offered by the battlecard editor: tracked competitors first,
    /// then every other company, alphabetically, capped at `limit`.
    pub async fn list_battlecard_company_options(
        &self,
        limit: i64,
    ) -> Result<Vec<BattlecardCompanyOption>> {
        let rows = sqlx::query_as::<_, BattlecardCompanyOption>(
            "SELECT id, name, \
                    (COALESCE(is_competitor, FALSE) \
                     OR lower(COALESCE(metadata->>'is_competitor', '')) IN ('true', '1', 'yes')) \
                      AS is_competitor \
             FROM companies \
             ORDER BY 3 DESC, lower(name), id \
             LIMIT $1",
        )
        .bind(clamp_limit(limit))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Get a single battlecard by its primary key.
    pub async fn get_battlecard(&self, id: Uuid) -> Result<Option<BattlecardRow>> {
        let sql = format!("SELECT {BATTLECARD_COLUMNS} FROM battlecards WHERE id = $1");
        let row = sqlx::query_as::<_, BattlecardRow>(&sql)
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row)
    }

    /// Get a battlecard by our_company_id + competitor_id pair.
    pub async fn get_battlecard_by_pair(
        &self,
        our_id: Uuid,
        competitor_id: Uuid,
    ) -> Result<Option<BattlecardRow>> {
        let sql = format!(
            "SELECT {BATTLECARD_COLUMNS} FROM battlecards \
             WHERE our_company_id = $1 AND competitor_id = $2"
        );
        let row = sqlx::query_as::<_, BattlecardRow>(&sql)
            .bind(our_id)
            .bind(competitor_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row)
    }

    /// Create a new battlecard in 'draft' status.
    ///
    /// The (our company, competitor) pair is unique; an existing pair yields
    /// [`CreateBattlecardOutcome::Duplicate`] with the existing id, and a
    /// missing company yields [`CreateBattlecardOutcome::UnknownCompany`]
    /// instead of a raw constraint error. Callers validate the title and that
    /// the two companies differ.
    pub async fn create_battlecard(
        &self,
        our_id: Uuid,
        competitor_id: Uuid,
        title: &str,
    ) -> Result<CreateBattlecardOutcome> {
        let inserted: std::result::Result<Option<(Uuid,)>, sqlx::Error> = sqlx::query_as(
            "INSERT INTO battlecards (our_company_id, competitor_id, title) \
             VALUES ($1, $2, $3) \
             ON CONFLICT (our_company_id, competitor_id) DO NOTHING \
             RETURNING id",
        )
        .bind(our_id)
        .bind(competitor_id)
        .bind(title)
        .fetch_optional(&self.pool)
        .await;

        match inserted {
            Ok(Some((id,))) => Ok(CreateBattlecardOutcome::Created(id)),
            Ok(None) => {
                let existing: Option<(Uuid,)> = sqlx::query_as(
                    "SELECT id FROM battlecards WHERE our_company_id = $1 AND competitor_id = $2",
                )
                .bind(our_id)
                .bind(competitor_id)
                .fetch_optional(&self.pool)
                .await?;
                match existing {
                    Some((id,)) => Ok(CreateBattlecardOutcome::Duplicate(id)),
                    // Conflicting row was deleted between the two statements.
                    None => Err(anyhow::anyhow!(
                        "battlecard pair conflicted but no existing row was found; retry"
                    )),
                }
            }
            Err(sqlx::Error::Database(db)) if db.code().as_deref() == Some("23503") => {
                Ok(CreateBattlecardOutcome::UnknownCompany)
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Update the editable battlecard fields in one transaction: optional
    /// title, optional status and any number of JSONB sections.
    ///
    /// Records the acting principal in `updated_by`. When
    /// `expected_updated_at` is set the write only applies if the stored
    /// `updated_at` still matches (optimistic concurrency), so an editor
    /// cannot silently overwrite a newer regeneration. Invalid statuses or
    /// section names are rejected before any write.
    pub async fn update_battlecard_details(
        &self,
        id: Uuid,
        title: Option<&str>,
        status: Option<&str>,
        sections: &[(&str, serde_json::Value)],
        updated_by: &str,
        expected_updated_at: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<BattlecardWriteOutcome> {
        self.write_battlecard(
            id,
            BattlecardWrite {
                title,
                status,
                sections,
                updated_by,
                mark_regenerated: false,
                expected_updated_at,
            },
        )
        .await
    }

    /// Persist a full regeneration atomically: every generated section plus
    /// `regenerated_at`, or nothing. Returns `false` when the battlecard no
    /// longer exists.
    pub async fn apply_battlecard_regeneration(
        &self,
        id: Uuid,
        sections: &[(&str, serde_json::Value)],
        updated_by: &str,
    ) -> Result<bool> {
        let outcome = self
            .write_battlecard(
                id,
                BattlecardWrite {
                    title: None,
                    status: None,
                    sections,
                    updated_by,
                    mark_regenerated: true,
                    expected_updated_at: None,
                },
            )
            .await?;
        Ok(outcome == BattlecardWriteOutcome::Updated)
    }

    async fn write_battlecard(
        &self,
        id: Uuid,
        write: BattlecardWrite<'_>,
    ) -> Result<BattlecardWriteOutcome> {
        let BattlecardWrite {
            title,
            status,
            sections,
            updated_by,
            mark_regenerated,
            expected_updated_at,
        } = write;
        if let Some(status) = status {
            if !BATTLECARD_STATUSES.contains(&status) {
                return Err(anyhow::anyhow!("invalid status: {}", status));
            }
        }
        if let Some(title) = title {
            if title.trim().is_empty() || title.chars().count() > BATTLECARD_TITLE_MAX_CHARS {
                return Err(anyhow::anyhow!(
                    "title must be 1-{BATTLECARD_TITLE_MAX_CHARS} characters"
                ));
            }
        }
        for (section, _) in sections {
            validate_section(section)?;
        }

        let mut tx = self.pool.begin().await?;
        let result = sqlx::query(
            "UPDATE battlecards \
             SET title = COALESCE($1, title), status = COALESCE($2, status), \
                 updated_at = NOW(), updated_by = $3, \
                 regenerated_at = CASE WHEN $5 THEN NOW() ELSE regenerated_at END \
             WHERE id = $4 AND ($6::timestamptz IS NULL OR updated_at = $6)",
        )
        .bind(title)
        .bind(status)
        .bind(updated_by)
        .bind(id)
        .bind(mark_regenerated)
        .bind(expected_updated_at)
        .execute(&mut *tx)
        .await?;
        if result.rows_affected() == 0 {
            let (exists,): (bool,) =
                sqlx::query_as("SELECT EXISTS (SELECT 1 FROM battlecards WHERE id = $1)")
                    .bind(id)
                    .fetch_one(&mut *tx)
                    .await?;
            tx.rollback().await?;
            return Ok(if exists {
                BattlecardWriteOutcome::Conflict
            } else {
                BattlecardWriteOutcome::NotFound
            });
        }
        for (section, data) in sections {
            // `section` is validated against VALID_SECTIONS above.
            let sql = format!("UPDATE battlecards SET {section} = $1 WHERE id = $2");
            sqlx::query(&sql)
                .bind(section_param(data))
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(BattlecardWriteOutcome::Updated)
    }

    /// Update a single JSONB section by name (PATCH-style).
    ///
    /// Records the acting principal in `updated_by`. Returns `false` when no
    /// battlecard with `id` exists (nothing was updated).
    pub async fn update_battlecard_section(
        &self,
        id: Uuid,
        section: &str,
        data: &serde_json::Value,
        updated_by: &str,
    ) -> Result<bool> {
        validate_section(section)?;

        let sql = format!(
            "UPDATE battlecards SET {} = $1, updated_at = NOW(), updated_by = $2 WHERE id = $3",
            section
        );
        let result = sqlx::query(&sql)
            .bind(section_param(data))
            .bind(updated_by)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Delete a battlecard by id. Returns `false` when no row was deleted.
    pub async fn delete_battlecard(&self, id: Uuid) -> Result<bool> {
        let result = sqlx::query("DELETE FROM battlecards WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_section_accepts_valid() {
        for section in VALID_SECTIONS {
            assert!(
                validate_section(section).is_ok(),
                "expected '{}' to be valid",
                section
            );
        }
    }

    #[test]
    fn test_validate_section_rejects_invalid() {
        let err = validate_section("nonexistent_field").unwrap_err();
        assert!(err.to_string().contains("invalid section name"));
    }

    #[test]
    fn test_validate_section_rejects_empty_string() {
        let err = validate_section("").unwrap_err();
        assert!(err.to_string().contains("invalid section name"));
    }

    #[test]
    fn test_clamp_page_per_page() {
        let (limit, offset) = clamp_page_per_page(0, 0);
        assert_eq!(limit, 1);
        assert_eq!(offset, 0);

        let (limit, offset) = clamp_page_per_page(1, 50);
        assert_eq!(limit, 50);
        assert_eq!(offset, 0);

        let (limit, offset) = clamp_page_per_page(3, 200);
        assert_eq!(limit, 100);
        assert_eq!(offset, 200);
    }
}
