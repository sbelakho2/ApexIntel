use super::*;

fn normalize_tag_labels(tags: &[String]) -> Vec<String> {
    let mut normalized = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for tag in tags {
        let trimmed = tag.trim();
        if trimmed.is_empty() {
            continue;
        }
        let canonical = trimmed.to_ascii_lowercase();
        if seen.insert(canonical) {
            normalized.push(trimmed.to_string());
        }
    }
    normalized
}

impl PgStore {
    pub async fn record_audit_event(
        &self,
        actor: &str,
        event_type: &str,
        detail: &Value,
    ) -> Result<()> {
        sqlx::query("INSERT INTO audit_log (event_type, actor, detail) VALUES ($1, $2, $3)")
            .bind(event_type)
            .bind(actor)
            .bind(detail)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn upsert_analyst_user(
        &self,
        id: &str,
        display_name: &str,
        email: Option<&str>,
        role: &str,
        notification_channels: &Value,
        is_active: bool,
    ) -> Result<AnalystUserRecord> {
        let record = sqlx::query_as::<_, AnalystUserRecord>(
            r#"INSERT INTO analyst_users (id, display_name, email, role, notification_channels, is_active)
               VALUES ($1, $2, $3, $4, $5, $6)
               ON CONFLICT (id) DO UPDATE SET
                 display_name = EXCLUDED.display_name,
                 email = EXCLUDED.email,
                 role = EXCLUDED.role,
                 notification_channels = EXCLUDED.notification_channels,
                 is_active = EXCLUDED.is_active,
                 updated_at = NOW()
               RETURNING id, display_name, email, role, notification_channels, is_active, created_at, updated_at"#,
        )
        .bind(id)
        .bind(display_name)
        .bind(email)
        .bind(role)
        .bind(notification_channels)
        .bind(is_active)
        .fetch_one(&self.pool)
        .await?;

        Ok(record)
    }

    pub async fn list_analyst_users(&self) -> Result<Vec<AnalystUserRecord>> {
        Ok(sqlx::query_as::<_, AnalystUserRecord>(
            "SELECT id, display_name, email, role, notification_channels, is_active, created_at, updated_at FROM analyst_users ORDER BY display_name ASC, id ASC",
        )
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn get_analyst_user(&self, id: &str) -> Result<Option<AnalystUserRecord>> {
        Ok(sqlx::query_as::<_, AnalystUserRecord>(
            "SELECT id, display_name, email, role, notification_channels, is_active, created_at, updated_at FROM analyst_users WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn register_api_key_owner(
        &self,
        key_id: &str,
        user_id: &str,
        display_name: &str,
        role: &str,
    ) -> Result<ApiKeyOwnerRecord> {
        let notification_channels = serde_json::json!({});
        let normalized_role = role.trim().to_ascii_lowercase();

        self.upsert_analyst_user(
            user_id,
            display_name,
            None,
            &normalized_role,
            &notification_channels,
            true,
        )
        .await?;

        sqlx::query(
            r#"INSERT INTO analyst_user_roles (user_id, role, granted_by)
               VALUES ($1, $2, $3)
               ON CONFLICT (user_id, role) DO NOTHING"#,
        )
        .bind(user_id)
        .bind(&normalized_role)
        .bind("api_key_sync")
        .execute(&self.pool)
        .await?;

        Ok(sqlx::query_as::<_, ApiKeyOwnerRecord>(
            r#"INSERT INTO api_key_owners (key_id, user_id, role, display_name, last_seen_at)
               VALUES ($1, $2, $3, $4, NOW())
               ON CONFLICT (key_id) DO UPDATE SET
                 user_id = EXCLUDED.user_id,
                 role = EXCLUDED.role,
                 display_name = EXCLUDED.display_name,
                 last_seen_at = NOW(),
                 updated_at = NOW()
               RETURNING key_id, user_id, display_name, role,
                         TRUE AS is_active,
                         '{}'::jsonb AS notification_channels,
                         created_at, updated_at"#,
        )
        .bind(key_id)
        .bind(user_id)
        .bind(&normalized_role)
        .bind(display_name)
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn resolve_api_key_owner(&self, key_id: &str) -> Result<Option<ApiKeyOwnerRecord>> {
        Ok(sqlx::query_as::<_, ApiKeyOwnerRecord>(
            r#"SELECT o.key_id,
                      o.user_id,
                      u.display_name,
                      COALESCE(role_choice.role, u.role, o.role) AS role,
                      u.is_active,
                      u.notification_channels,
                      o.created_at,
                      o.updated_at
               FROM api_key_owners o
               JOIN analyst_users u ON u.id = o.user_id
               LEFT JOIN LATERAL (
                   SELECT role
                   FROM analyst_user_roles
                   WHERE user_id = u.id
                   ORDER BY CASE role
                       WHEN 'admin' THEN 4
                       WHEN 'analyst' THEN 3
                       WHEN 'viewer' THEN 2
                       ELSE 1
                   END DESC,
                   granted_at DESC
                   LIMIT 1
               ) AS role_choice ON TRUE
               WHERE o.key_id = $1"#,
        )
        .bind(key_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn touch_api_key_owner(&self, key_id: &str) -> Result<()> {
        sqlx::query(
            "UPDATE api_key_owners SET last_seen_at = NOW(), updated_at = NOW() WHERE key_id = $1",
        )
        .bind(key_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn replace_tag_assignments(
        &self,
        subject_type: &str,
        subject_id: &str,
        source: &str,
        created_by: Option<&str>,
        tags: &[String],
    ) -> Result<()> {
        let tags = normalize_tag_labels(tags);
        let mut tx = self.pool.begin().await?;

        sqlx::query(
            "DELETE FROM tag_assignments WHERE subject_type = $1 AND subject_id = $2 AND source = $3",
        )
        .bind(subject_type)
        .bind(subject_id)
        .bind(source)
        .execute(&mut *tx)
        .await?;

        for tag in tags {
            let normalized = tag.to_ascii_lowercase();
            let (tag_id,): (Uuid,) = sqlx::query_as(
                r#"INSERT INTO tags (label, normalized_label)
                   VALUES ($1, $2)
                   ON CONFLICT (normalized_label) DO UPDATE SET label = EXCLUDED.label
                   RETURNING id"#,
            )
            .bind(&tag)
            .bind(&normalized)
            .fetch_one(&mut *tx)
            .await?;

            sqlx::query(
                r#"INSERT INTO tag_assignments (tag_id, subject_type, subject_id, source, created_by)
                   VALUES ($1, $2, $3, $4, $5)
                   ON CONFLICT (tag_id, subject_type, subject_id, source) DO NOTHING"#,
            )
            .bind(tag_id)
            .bind(subject_type)
            .bind(subject_id)
            .bind(source)
            .bind(created_by)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    pub async fn upsert_saved_search(
        &self,
        id: Option<Uuid>,
        user_id: &str,
        name: &str,
        query_text: &str,
        filters: &Value,
        default_sort: Option<&str>,
    ) -> Result<SavedSearchRecord> {
        let id = id.unwrap_or_else(Uuid::new_v4);
        Ok(sqlx::query_as::<_, SavedSearchRecord>(
            r#"INSERT INTO saved_searches (id, user_id, name, query_text, filters, default_sort)
               VALUES ($1, $2, $3, $4, $5, $6)
               ON CONFLICT (id) DO UPDATE SET
                 name = EXCLUDED.name,
                 query_text = EXCLUDED.query_text,
                 filters = EXCLUDED.filters,
                 default_sort = EXCLUDED.default_sort,
                 updated_at = NOW()
               RETURNING id, user_id, name, query_text, filters, default_sort, created_at, updated_at"#,
        )
        .bind(id)
        .bind(user_id)
        .bind(name)
        .bind(query_text)
        .bind(filters)
        .bind(default_sort)
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn list_saved_searches(&self, user_id: &str) -> Result<Vec<SavedSearchRecord>> {
        Ok(sqlx::query_as::<_, SavedSearchRecord>(
            "SELECT id, user_id, name, query_text, filters, default_sort, created_at, updated_at FROM saved_searches WHERE user_id = $1 ORDER BY updated_at DESC, id ASC",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn delete_saved_search(&self, user_id: &str, id: Uuid) -> Result<bool> {
        let result = sqlx::query("DELETE FROM saved_searches WHERE id = $1 AND user_id = $2")
            .bind(id)
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn upsert_watchlist(
        &self,
        id: Option<Uuid>,
        user_id: &str,
        name: &str,
        entities: &Value,
        notes: Option<&str>,
    ) -> Result<WatchlistRecord> {
        let id = id.unwrap_or_else(Uuid::new_v4);
        Ok(sqlx::query_as::<_, WatchlistRecord>(
            r#"INSERT INTO watchlists (id, user_id, name, entities, notes)
               VALUES ($1, $2, $3, $4, $5)
               ON CONFLICT (id) DO UPDATE SET
                 name = EXCLUDED.name,
                 entities = EXCLUDED.entities,
                 notes = EXCLUDED.notes,
                 updated_at = NOW()
               RETURNING id, user_id, name, entities, notes, created_at, updated_at"#,
        )
        .bind(id)
        .bind(user_id)
        .bind(name)
        .bind(entities)
        .bind(notes)
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn list_watchlists(&self, user_id: &str) -> Result<Vec<WatchlistRecord>> {
        Ok(sqlx::query_as::<_, WatchlistRecord>(
            "SELECT id, user_id, name, entities, notes, created_at, updated_at FROM watchlists WHERE user_id = $1 ORDER BY updated_at DESC, id ASC",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn delete_watchlist(&self, user_id: &str, id: Uuid) -> Result<bool> {
        let result = sqlx::query("DELETE FROM watchlists WHERE id = $1 AND user_id = $2")
            .bind(id)
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn upsert_annotation(
        &self,
        id: Option<Uuid>,
        user_id: &str,
        entity_type: &str,
        entity_id: &str,
        body: &str,
        tags: &[String],
        visibility: &str,
    ) -> Result<AnnotationRecord> {
        let id = id.unwrap_or_else(Uuid::new_v4);
        let record = sqlx::query_as::<_, AnnotationRecord>(
            r#"INSERT INTO annotations (id, user_id, entity_type, entity_id, body, tags, visibility)
               VALUES ($1, $2, $3, $4, $5, $6, $7)
               ON CONFLICT (id) DO UPDATE SET
                 entity_type = EXCLUDED.entity_type,
                 entity_id = EXCLUDED.entity_id,
                 body = EXCLUDED.body,
                 tags = EXCLUDED.tags,
                 visibility = EXCLUDED.visibility,
                 updated_at = NOW()
               RETURNING id, user_id, entity_type, entity_id, body, tags, visibility, created_at, updated_at"#,
        )
        .bind(id)
        .bind(user_id)
        .bind(entity_type)
        .bind(entity_id)
        .bind(body)
        .bind(tags)
        .bind(visibility)
        .fetch_one(&self.pool)
        .await?;

        self.replace_tag_assignments(
            "annotation",
            &record.id.to_string(),
            "annotation",
            Some(user_id),
            &record.tags,
        )
        .await?;

        Ok(record)
    }

    pub async fn list_annotations(
        &self,
        user_id: &str,
        entity_type: Option<&str>,
        entity_id: Option<&str>,
    ) -> Result<Vec<AnnotationRecord>> {
        let mut qb = QueryBuilder::<Postgres>::new(
            "SELECT id, user_id, entity_type, entity_id, body, tags, visibility, created_at, updated_at FROM annotations WHERE (visibility = 'team' OR user_id = ",
        );
        qb.push_bind(user_id).push(")");
        if let Some(entity_type) = entity_type {
            qb.push(" AND entity_type = ").push_bind(entity_type);
        }
        if let Some(entity_id) = entity_id {
            qb.push(" AND entity_id = ").push_bind(entity_id);
        }
        qb.push(" ORDER BY updated_at DESC, id ASC");

        Ok(qb
            .build_query_as::<AnnotationRecord>()
            .fetch_all(&self.pool)
            .await?)
    }

    pub async fn delete_annotation(&self, user_id: &str, id: Uuid) -> Result<bool> {
        let result = sqlx::query(
            "DELETE FROM annotations WHERE id = $1 AND (user_id = $2 OR visibility = 'team')",
        )
        .bind(id)
        .bind(user_id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn create_notification(
        &self,
        user_id: &str,
        category: &str,
        title: &str,
        body: &str,
        entity_type: Option<&str>,
        entity_id: Option<&str>,
        action_url: Option<&str>,
    ) -> Result<AnalystNotificationRecord> {
        Ok(sqlx::query_as::<_, AnalystNotificationRecord>(
            r#"INSERT INTO analyst_notifications (
                   user_id, category, title, body, entity_type, entity_id, action_url
               )
               VALUES ($1, $2, $3, $4, $5, $6, $7)
               RETURNING id, user_id, category, title, body, entity_type, entity_id,
                         action_url, is_read, read_at, created_at"#,
        )
        .bind(user_id)
        .bind(category)
        .bind(title)
        .bind(body)
        .bind(entity_type)
        .bind(entity_id)
        .bind(action_url)
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn list_notifications(
        &self,
        user_id: &str,
        include_read: bool,
        limit: i64,
    ) -> Result<Vec<AnalystNotificationRecord>> {
        let limit = clamp_limit(limit);
        let mut qb = QueryBuilder::<Postgres>::new(
            "SELECT id, user_id, category, title, body, entity_type, entity_id, action_url, is_read, read_at, created_at FROM analyst_notifications WHERE user_id = ",
        );
        qb.push_bind(user_id);
        if !include_read {
            qb.push(" AND is_read = FALSE");
        }
        qb.push(" ORDER BY created_at DESC, id DESC LIMIT ")
            .push_bind(limit);

        Ok(qb
            .build_query_as::<AnalystNotificationRecord>()
            .fetch_all(&self.pool)
            .await?)
    }

    pub async fn unread_notification_count(&self, user_id: &str) -> Result<i64> {
        let (count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*)::bigint FROM analyst_notifications WHERE user_id = $1 AND is_read = FALSE",
        )
        .bind(user_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(count)
    }

    pub async fn mark_notification_read(&self, user_id: &str, id: Uuid) -> Result<bool> {
        let result = sqlx::query(
            r#"UPDATE analyst_notifications
               SET is_read = TRUE,
                   read_at = COALESCE(read_at, now())
               WHERE id = $1 AND user_id = $2"#,
        )
        .bind(id)
        .bind(user_id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn record_export_history(
        &self,
        user_id: &str,
        export_type: &str,
        format: &str,
        filters: &Value,
        row_count: i64,
        download_name: Option<&str>,
    ) -> Result<ExportHistoryRecord> {
        Ok(sqlx::query_as::<_, ExportHistoryRecord>(
            r#"INSERT INTO export_history (user_id, export_type, format, filters, row_count, download_name)
               VALUES ($1, $2, $3, $4, $5, $6)
               RETURNING id, user_id, export_type, format, filters, row_count, download_name, requested_at"#,
        )
        .bind(user_id)
        .bind(export_type)
        .bind(format)
        .bind(filters)
        .bind(row_count)
        .bind(download_name)
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn list_export_history(&self, user_id: &str) -> Result<Vec<ExportHistoryRecord>> {
        Ok(sqlx::query_as::<_, ExportHistoryRecord>(
            "SELECT id, user_id, export_type, format, filters, row_count, download_name, requested_at FROM export_history WHERE user_id = $1 ORDER BY requested_at DESC, id ASC",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn try_record_sla_reminder(
        &self,
        warning_id: &str,
        reminder_kind: &str,
        delivery_key: &str,
        detail: &Value,
    ) -> Result<bool> {
        let result = sqlx::query(
            r#"INSERT INTO sla_reminder_state (warning_id, reminder_kind, delivery_key, detail)
               VALUES ($1, $2, $3, $4)
               ON CONFLICT (warning_id, reminder_kind) DO NOTHING"#,
        )
        .bind(warning_id)
        .bind(reminder_kind)
        .bind(delivery_key)
        .bind(detail)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn record_notification_delivery_attempt(
        &self,
        delivery_key: &str,
        channel: &str,
        destination: &str,
        payload: &Value,
        status: &str,
        error: Option<&str>,
        next_retry_at: Option<DateTime<Utc>>,
    ) -> Result<()> {
        let attempted_at = Utc::now();
        let delivered_at = if status == "delivered" {
            Some(attempted_at)
        } else {
            None
        };

        sqlx::query(
            r#"INSERT INTO notification_delivery_state (
                   delivery_key, channel, destination, payload, status, attempts,
                   last_attempt_at, next_retry_at, delivered_at, last_error
               )
               VALUES ($1, $2, $3, $4, $5, 1, $6, $7, $8, $9)
               ON CONFLICT (delivery_key) DO UPDATE SET
                   channel = EXCLUDED.channel,
                   destination = EXCLUDED.destination,
                   payload = EXCLUDED.payload,
                   status = EXCLUDED.status,
                   attempts = notification_delivery_state.attempts + 1,
                   last_attempt_at = EXCLUDED.last_attempt_at,
                   next_retry_at = EXCLUDED.next_retry_at,
                   delivered_at = COALESCE(EXCLUDED.delivered_at, notification_delivery_state.delivered_at),
                   last_error = EXCLUDED.last_error,
                   updated_at = NOW()"#,
        )
        .bind(delivery_key)
        .bind(channel)
        .bind(destination)
        .bind(payload)
        .bind(status)
        .bind(attempted_at)
        .bind(next_retry_at)
        .bind(delivered_at)
        .bind(error)
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"INSERT INTO notification_delivery_attempts (delivery_key, attempted_at, status, error)
               VALUES ($1, $2, $3, $4)"#,
        )
        .bind(delivery_key)
        .bind(attempted_at)
        .bind(status)
        .bind(error)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn count_replay_candidates(
        &self,
        from_date: NaiveDate,
        to_date: NaiveDate,
        observation_types: Option<&[String]>,
        entity_ids: Option<&[String]>,
        limit: i64,
    ) -> Result<i64> {
        let mut qb = QueryBuilder::<Postgres>::new(
            "SELECT COUNT(*)::bigint FROM observations WHERE ts_utc >= ",
        );
        qb.push_bind(from_date.and_hms_opt(0, 0, 0).unwrap().and_utc())
            .push(" AND ts_utc <= ")
            .push_bind(to_date.and_hms_opt(23, 59, 59).unwrap().and_utc());
        if let Some(observation_types) = observation_types.filter(|values| !values.is_empty()) {
            qb.push(" AND observation_type = ANY(")
                .push_bind(observation_types)
                .push(")");
        }
        if let Some(entity_ids) = entity_ids.filter(|values| !values.is_empty()) {
            qb.push(" AND COALESCE(entity_id::text, '') = ANY(")
                .push_bind(entity_ids)
                .push(")");
        }
        let count: (i64,) = qb.build_query_as().fetch_one(&self.pool).await?;
        Ok(count.0.min(limit.max(0)))
    }

    pub async fn create_replay_job(
        &self,
        requested_by: &str,
        request: &Value,
    ) -> Result<ReplayJobRecord> {
        Ok(sqlx::query_as::<_, ReplayJobRecord>(
            r#"INSERT INTO replay_jobs (requested_by, status, request, started_at)
               VALUES ($1, 'queued', $2, NOW())
               RETURNING id, requested_by, status, request, total_observations, processed, warnings_generated, errors, started_at, completed_at, created_at, updated_at"#,
        )
        .bind(requested_by)
        .bind(request)
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn update_replay_job(
        &self,
        id: Uuid,
        status: &str,
        total_observations: i64,
        processed: i64,
        warnings_generated: i64,
        errors: i64,
        completed_at: Option<DateTime<Utc>>,
    ) -> Result<ReplayJobRecord> {
        Ok(sqlx::query_as::<_, ReplayJobRecord>(
            r#"UPDATE replay_jobs
               SET status = $2,
                   total_observations = $3,
                   processed = $4,
                   warnings_generated = $5,
                   errors = $6,
                   completed_at = $7,
                   updated_at = NOW()
               WHERE id = $1
               RETURNING id, requested_by, status, request, total_observations, processed, warnings_generated, errors, started_at, completed_at, created_at, updated_at"#,
        )
        .bind(id)
        .bind(status)
        .bind(total_observations)
        .bind(processed)
        .bind(warnings_generated)
        .bind(errors)
        .bind(completed_at)
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn get_replay_job(&self, id: Uuid) -> Result<Option<ReplayJobRecord>> {
        Ok(sqlx::query_as::<_, ReplayJobRecord>(
            "SELECT id, requested_by, status, request, total_observations, processed, warnings_generated, errors, started_at, completed_at, created_at, updated_at FROM replay_jobs WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?)
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_tag_labels;

    #[test]
    fn normalize_tag_labels_dedupes_case_and_trims() {
        let tags = vec![
            " Risk ".to_string(),
            "risk".to_string(),
            "Supply".to_string(),
            "".to_string(),
        ];

        assert_eq!(
            normalize_tag_labels(&tags),
            vec!["Risk".to_string(), "Supply".to_string()]
        );
    }
}
