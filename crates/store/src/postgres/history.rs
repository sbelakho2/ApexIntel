use super::*;

fn normalize_history_limit(limit: i64) -> i64 {
    clamp_limit(limit)
}

impl PgStore {
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

    pub async fn get_role_history(
        &self,
        person_id: Uuid,
        limit: i64,
    ) -> Result<Vec<RoleHistoryRow>> {
        let rows = sqlx::query_as::<_, RoleHistoryRow>(
            "SELECT * FROM role_history WHERE person_id = $1 ORDER BY start_date DESC NULLS FIRST LIMIT $2"
        )
        .bind(person_id)
        .bind(normalize_history_limit(limit))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn close_role_history_entry(
        &self,
        entry_id: Uuid,
        end_date: DateTime<Utc>,
    ) -> Result<bool> {
        let result = sqlx::query(
            "UPDATE role_history SET end_date = $2, updated_at = now() WHERE id = $1 AND end_date IS NULL"
        )
        .bind(entry_id)
        .bind(end_date)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn get_current_role(&self, person_id: Uuid) -> Result<Option<RoleHistoryRow>> {
        let row = sqlx::query_as::<_, RoleHistoryRow>(
            "SELECT * FROM role_history WHERE person_id = $1 AND end_date IS NULL ORDER BY start_date DESC LIMIT 1"
        )
        .bind(person_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

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

    pub async fn get_dossier_entries(
        &self,
        entity_type: &str,
        entity_id: Uuid,
        category: Option<&str>,
        limit: i64,
    ) -> Result<Vec<DossierEntryRow>> {
        let limit = normalize_history_limit(limit);
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

    pub async fn get_dossier_entry_history(&self, entry_id: Uuid) -> Result<Vec<DossierEntryRow>> {
        let rows = sqlx::query_as::<_, DossierEntryRow>(
            r#"WITH RECURSIVE chain AS (
                SELECT * FROM dossier_entries WHERE id = $1
                UNION ALL
                SELECT de.* FROM dossier_entries de
                JOIN chain c ON de.id = c.supersedes_id
            )
            SELECT * FROM chain ORDER BY created_at DESC"#,
        )
        .bind(entry_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn verify_dossier_entry(&self, entry_id: Uuid) -> Result<bool> {
        let result = sqlx::query("UPDATE dossier_entries SET verified = TRUE WHERE id = $1")
            .bind(entry_id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

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

    pub async fn get_company_changes(
        &self,
        company_id: Uuid,
        limit: i64,
    ) -> Result<Vec<CompanyChangeRow>> {
        let rows = sqlx::query_as::<_, CompanyChangeRow>(
            "SELECT * FROM company_changes WHERE company_id = $1 ORDER BY detected_at DESC LIMIT $2"
        )
        .bind(company_id)
        .bind(normalize_history_limit(limit))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

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

    pub async fn get_person_changes(
        &self,
        person_id: Uuid,
        limit: i64,
    ) -> Result<Vec<PersonChangeRow>> {
        let rows = sqlx::query_as::<_, PersonChangeRow>(
            "SELECT * FROM person_changes WHERE person_id = $1 ORDER BY detected_at DESC LIMIT $2",
        )
        .bind(person_id)
        .bind(normalize_history_limit(limit))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn count_role_changes_since(&self, since: DateTime<Utc>) -> Result<i64> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM person_changes WHERE change_type IN ('job_change', 'role_change', 'org_change') AND detected_at >= $1"
        )
        .bind(since)
        .fetch_one(&self.pool)
        .await?;
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_history_limit;
    use crate::postgres::MAX_LIST_LIMIT;

    #[test]
    fn test_normalize_history_limit_clamps_limit() {
        assert_eq!(normalize_history_limit(0), 1);
        assert_eq!(normalize_history_limit(MAX_LIST_LIMIT + 10), MAX_LIST_LIMIT);
    }

    #[test]
    fn test_normalize_history_limit_preserves_valid_values() {
        assert_eq!(normalize_history_limit(30), 30);
    }
}
