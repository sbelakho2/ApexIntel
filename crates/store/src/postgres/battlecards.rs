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

fn clamp_page_per_page(page: u32, per_page: u32) -> (i64, i64) {
    let limit = (per_page.max(1)).min(100) as i64;
    let offset = ((page.max(1) - 1) as i64).saturating_mul(limit);
    (limit, offset)
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

        let mut sql = String::from(
            "SELECT id, our_company_id, competitor_id, title, status, \
             positioning, pricing, feature_matrix, strengths, weaknesses, \
             objection_handlers, kill_shots, recent_news, win_loss, \
             created_at, updated_at, updated_by, regenerated_at \
             FROM battlecards WHERE 1=1",
        );
        let mut param_idx = 1u32;

        if let Some(_s) = status {
            sql.push_str(&format!(" AND status = ${}", param_idx));
            param_idx += 1;
        }
        if let Some(_cid) = competitor_id {
            sql.push_str(&format!(" AND competitor_id = ${}", param_idx));
            param_idx += 1;
        }

        sql.push_str(&format!(
            " ORDER BY updated_at DESC LIMIT ${} OFFSET ${}",
            param_idx,
            param_idx + 1
        ));

        let mut q = sqlx::query_as::<_, BattlecardRow>(&sql);

        if let Some(s) = status {
            q = q.bind(s);
        }
        if let Some(cid) = competitor_id {
            q = q.bind(cid);
        }

        q = q.bind(limit).bind(offset);

        let rows = q.fetch_all(&self.pool).await?;
        Ok(rows)
    }

    /// Count battlecards matching optional filters.
    pub async fn count_battlecards(
        &self,
        status: Option<&str>,
        competitor_id: Option<Uuid>,
    ) -> Result<i64> {
        let mut sql = String::from("SELECT COUNT(*) FROM battlecards WHERE 1=1");
        let mut param_idx = 1u32;

        if let Some(_s) = status {
            sql.push_str(&format!(" AND status = ${}", param_idx));
            param_idx += 1;
        }
        if let Some(_cid) = competitor_id {
            sql.push_str(&format!(" AND competitor_id = ${}", param_idx));
            param_idx += 1;
        }

        let mut q = sqlx::query_as::<_, (i64,)>(&sql);

        if let Some(s) = status {
            q = q.bind(s);
        }
        if let Some(cid) = competitor_id {
            q = q.bind(cid);
        }

        let (count,): (i64,) = q.fetch_one(&self.pool).await?;
        Ok(count)
    }

    /// Get a single battlecard by its primary key.
    pub async fn get_battlecard(&self, id: Uuid) -> Result<Option<BattlecardRow>> {
        let row = sqlx::query_as::<_, BattlecardRow>(
            "SELECT id, our_company_id, competitor_id, title, status, \
             positioning, pricing, feature_matrix, strengths, weaknesses, \
             objection_handlers, kill_shots, recent_news, win_loss, \
             created_at, updated_at, updated_by, regenerated_at \
             FROM battlecards WHERE id = $1",
        )
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
        let row = sqlx::query_as::<_, BattlecardRow>(
            "SELECT id, our_company_id, competitor_id, title, status, \
             positioning, pricing, feature_matrix, strengths, weaknesses, \
             objection_handlers, kill_shots, recent_news, win_loss, \
             created_at, updated_at, updated_by, regenerated_at \
             FROM battlecards WHERE our_company_id = $1 AND competitor_id = $2",
        )
        .bind(our_id)
        .bind(competitor_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// Create a new battlecard in 'draft' status. Returns the new UUID.
    pub async fn create_battlecard(
        &self,
        our_id: Uuid,
        competitor_id: Uuid,
        title: &str,
    ) -> Result<Uuid> {
        let (id,): (Uuid,) = sqlx::query_as(
            "INSERT INTO battlecards (our_company_id, competitor_id, title) \
             VALUES ($1, $2, $3) RETURNING id",
        )
        .bind(our_id)
        .bind(competitor_id)
        .bind(title)
        .fetch_one(&self.pool)
        .await?;
        Ok(id)
    }

    /// Update a single JSONB section by name (PATCH-style).
    pub async fn update_battlecard_section(
        &self,
        id: Uuid,
        section: &str,
        data: &serde_json::Value,
    ) -> Result<()> {
        validate_section(section)?;

        let sql = format!(
            "UPDATE battlecards SET {} = $1, updated_at = NOW() WHERE id = $2",
            section
        );
        sqlx::query(&sql)
            .bind(data)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Update the battlecard status (draft → published → archived).
    pub async fn update_battlecard_status(&self, id: Uuid, status: &str) -> Result<()> {
        let valid = ["draft", "published", "archived"];
        if !valid.contains(&status) {
            return Err(anyhow::anyhow!("invalid status: {}", status));
        }
        sqlx::query("UPDATE battlecards SET status = $1, updated_at = NOW() WHERE id = $2")
            .bind(status)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Touch updated_at + regenerated_at timestamps.
    pub async fn update_battlecard_timestamp(&self, id: Uuid) -> Result<()> {
        sqlx::query(
            "UPDATE battlecards SET updated_at = NOW(), regenerated_at = NOW() WHERE id = $1",
        )
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Delete a battlecard by id.
    pub async fn delete_battlecard(&self, id: Uuid) -> Result<()> {
        sqlx::query("DELETE FROM battlecards WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
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
