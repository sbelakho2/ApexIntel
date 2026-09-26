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

async fn replace_tag_assignments_on(
    conn: &mut sqlx::PgConnection,
    subject_type: &str,
    subject_id: &str,
    source: &str,
    created_by: Option<&str>,
    tags: &[String],
) -> Result<()> {
    let tags = normalize_tag_labels(tags);

    sqlx::query(
        "DELETE FROM tag_assignments WHERE subject_type = $1 AND subject_id = $2 AND source = $3",
    )
    .bind(subject_type)
    .bind(subject_id)
    .bind(source)
    .execute(&mut *conn)
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
        .fetch_one(&mut *conn)
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
        .execute(&mut *conn)
        .await?;
    }

    Ok(())
}

async fn upsert_annotation_on(
    conn: &mut sqlx::PgConnection,
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
           WHERE annotations.user_id = EXCLUDED.user_id
           RETURNING id, user_id, entity_type, entity_id, body, tags, visibility, created_at, updated_at"#,
    )
    .bind(id)
    .bind(user_id)
    .bind(entity_type)
    .bind(entity_id)
    .bind(body)
    .bind(tags)
    .bind(visibility)
    .fetch_one(&mut *conn)
    .await?;

    replace_tag_assignments_on(
        &mut *conn,
        "annotation",
        &record.id.to_string(),
        "annotation",
        Some(user_id),
        &record.tags,
    )
    .await?;

    Ok(record)
}

async fn list_annotations_on(
    conn: &mut sqlx::PgConnection,
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
        .fetch_all(&mut *conn)
        .await?)
}

async fn delete_annotation_on(
    conn: &mut sqlx::PgConnection,
    user_id: &str,
    id: Uuid,
) -> Result<bool> {
    let result = sqlx::query(
        "DELETE FROM annotations WHERE id = $1 AND (user_id = $2 OR visibility = 'team')",
    )
    .bind(id)
    .bind(user_id)
    .execute(&mut *conn)
    .await?;
    Ok(result.rows_affected() > 0)
}

async fn update_priority_queue_item_on(
    conn: &mut sqlx::PgConnection,
    user_id: &str,
    id: Uuid,
    priority: Option<i32>,
    status: Option<&str>,
    notes: Option<Option<&str>>,
) -> Result<Option<PriorityQueueItemRecord>> {
    let current = match sqlx::query_as::<_, PriorityQueueItemRecord>(
        "SELECT id, user_id, queue_date, item_type, item_id, item_title, priority, status, notes, completed_at, created_at, updated_at FROM priority_queue WHERE id = $1 AND user_id = $2",
    )
    .bind(id)
    .bind(user_id)
    .fetch_optional(&mut *conn)
    .await?
    {
        Some(c) => c,
        None => return Ok(None),
    };

    let new_status = status.unwrap_or(&current.status).to_string();
    let completed_at: Option<DateTime<Utc>> = if new_status == "completed" {
        Some(Utc::now())
    } else {
        None
    };

    Ok(sqlx::query_as::<_, PriorityQueueItemRecord>(
        r#"UPDATE priority_queue SET
             priority = $3,
             status = $4,
             notes = $5,
             completed_at = $6,
             updated_at = NOW()
           WHERE id = $1 AND user_id = $2
           RETURNING id, user_id, queue_date, item_type, item_id, item_title, priority, status, notes, completed_at, created_at, updated_at"#,
    )
    .bind(id)
    .bind(user_id)
    .bind(priority.unwrap_or(current.priority))
    .bind(&new_status)
    .bind(notes.unwrap_or(current.notes.as_deref()))
    .bind(completed_at)
    .fetch_optional(&mut *conn)
    .await?)
}

async fn upsert_saved_search_on(
    conn: &mut sqlx::PgConnection,
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
           WHERE saved_searches.user_id = EXCLUDED.user_id
           RETURNING id, user_id, name, query_text, filters, default_sort, created_at, updated_at"#,
    )
    .bind(id)
    .bind(user_id)
    .bind(name)
    .bind(query_text)
    .bind(filters)
    .bind(default_sort)
    .fetch_one(&mut *conn)
    .await?)
}

async fn update_saved_search_on(
    conn: &mut sqlx::PgConnection,
    user_id: &str,
    id: Uuid,
    name: &str,
    query_text: &str,
    filters: &Value,
    default_sort: Option<&str>,
) -> Result<Option<SavedSearchRecord>> {
    Ok(sqlx::query_as::<_, SavedSearchRecord>(
        r#"UPDATE saved_searches
           SET name = $3,
               query_text = $4,
               filters = $5,
               default_sort = $6,
               updated_at = NOW()
           WHERE id = $1 AND user_id = $2
           RETURNING id, user_id, name, query_text, filters, default_sort, created_at, updated_at"#,
    )
    .bind(id)
    .bind(user_id)
    .bind(name)
    .bind(query_text)
    .bind(filters)
    .bind(default_sort)
    .fetch_optional(&mut *conn)
    .await?)
}

async fn list_saved_searches_on(
    conn: &mut sqlx::PgConnection,
    user_id: &str,
) -> Result<Vec<SavedSearchRecord>> {
    Ok(sqlx::query_as::<_, SavedSearchRecord>(
        "SELECT id, user_id, name, query_text, filters, default_sort, created_at, updated_at FROM saved_searches WHERE user_id = $1 ORDER BY updated_at DESC, id ASC",
    )
    .bind(user_id)
    .fetch_all(&mut *conn)
    .await?)
}

async fn delete_saved_search_on(
    conn: &mut sqlx::PgConnection,
    user_id: &str,
    id: Uuid,
) -> Result<bool> {
    let result = sqlx::query("DELETE FROM saved_searches WHERE id = $1 AND user_id = $2")
        .bind(id)
        .bind(user_id)
        .execute(&mut *conn)
        .await?;
    Ok(result.rows_affected() > 0)
}

async fn upsert_watchlist_on(
    conn: &mut sqlx::PgConnection,
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
           WHERE watchlists.user_id = EXCLUDED.user_id
           RETURNING id, user_id, name, entities, notes, created_at, updated_at"#,
    )
    .bind(id)
    .bind(user_id)
    .bind(name)
    .bind(entities)
    .bind(notes)
    .fetch_one(&mut *conn)
    .await?)
}

async fn update_watchlist_on(
    conn: &mut sqlx::PgConnection,
    user_id: &str,
    id: Uuid,
    name: &str,
    entities: &Value,
    notes: Option<&str>,
) -> Result<Option<WatchlistRecord>> {
    Ok(sqlx::query_as::<_, WatchlistRecord>(
        r#"UPDATE watchlists
           SET name = $3,
               entities = $4,
               notes = $5,
               updated_at = NOW()
           WHERE id = $1 AND user_id = $2
           RETURNING id, user_id, name, entities, notes, created_at, updated_at"#,
    )
    .bind(id)
    .bind(user_id)
    .bind(name)
    .bind(entities)
    .bind(notes)
    .fetch_optional(&mut *conn)
    .await?)
}

async fn list_watchlists_on(
    conn: &mut sqlx::PgConnection,
    user_id: &str,
) -> Result<Vec<WatchlistRecord>> {
    Ok(sqlx::query_as::<_, WatchlistRecord>(
        "SELECT id, user_id, name, entities, notes, created_at, updated_at FROM watchlists WHERE user_id = $1 ORDER BY updated_at DESC, id ASC",
    )
    .bind(user_id)
    .fetch_all(&mut *conn)
    .await?)
}

async fn delete_watchlist_on(
    conn: &mut sqlx::PgConnection,
    user_id: &str,
    id: Uuid,
) -> Result<bool> {
    let result = sqlx::query("DELETE FROM watchlists WHERE id = $1 AND user_id = $2")
        .bind(id)
        .bind(user_id)
        .execute(&mut *conn)
        .await?;
    Ok(result.rows_affected() > 0)
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

    pub async fn upsert_saved_search(
        &self,
        id: Option<Uuid>,
        user_id: &str,
        name: &str,
        query_text: &str,
        filters: &Value,
        default_sort: Option<&str>,
    ) -> Result<SavedSearchRecord> {
        let mut conn = self.pool.acquire().await?;
        upsert_saved_search_on(
            &mut conn,
            id,
            user_id,
            name,
            query_text,
            filters,
            default_sort,
        )
        .await
    }

    /// Identity-scoped saved-search write for RLS-forced `saved_searches`.
    pub async fn upsert_saved_search_scoped(
        &self,
        user_id: &str,
        role: &str,
        id: Option<Uuid>,
        name: &str,
        query_text: &str,
        filters: &Value,
        default_sort: Option<&str>,
    ) -> Result<SavedSearchRecord> {
        let mut tx = self.begin_scoped(user_id, role).await?;
        let record = upsert_saved_search_on(
            &mut tx,
            id,
            user_id,
            name,
            query_text,
            filters,
            default_sort,
        )
        .await?;
        tx.commit().await?;
        Ok(record)
    }

    /// Identity-scoped saved-search update; `None` when the id does not belong
    /// to the caller (never a silent overwrite of another user's row).
    pub async fn update_saved_search_scoped(
        &self,
        user_id: &str,
        role: &str,
        id: Uuid,
        name: &str,
        query_text: &str,
        filters: &Value,
        default_sort: Option<&str>,
    ) -> Result<Option<SavedSearchRecord>> {
        let mut tx = self.begin_scoped(user_id, role).await?;
        let record = update_saved_search_on(
            &mut tx,
            user_id,
            id,
            name,
            query_text,
            filters,
            default_sort,
        )
        .await?;
        tx.commit().await?;
        Ok(record)
    }

    pub async fn list_saved_searches(&self, user_id: &str) -> Result<Vec<SavedSearchRecord>> {
        let mut conn = self.pool.acquire().await?;
        list_saved_searches_on(&mut conn, user_id).await
    }

    /// Identity-scoped saved-search read for RLS-forced `saved_searches`.
    pub async fn list_saved_searches_scoped(
        &self,
        user_id: &str,
        role: &str,
    ) -> Result<Vec<SavedSearchRecord>> {
        let mut tx = self.begin_scoped(user_id, role).await?;
        let records = list_saved_searches_on(&mut tx, user_id).await?;
        tx.commit().await?;
        Ok(records)
    }

    pub async fn delete_saved_search(&self, user_id: &str, id: Uuid) -> Result<bool> {
        let mut conn = self.pool.acquire().await?;
        delete_saved_search_on(&mut conn, user_id, id).await
    }

    /// Identity-scoped saved-search delete for RLS-forced `saved_searches`.
    pub async fn delete_saved_search_scoped(
        &self,
        user_id: &str,
        role: &str,
        id: Uuid,
    ) -> Result<bool> {
        let mut tx = self.begin_scoped(user_id, role).await?;
        let deleted = delete_saved_search_on(&mut tx, user_id, id).await?;
        tx.commit().await?;
        Ok(deleted)
    }

    pub async fn upsert_watchlist(
        &self,
        id: Option<Uuid>,
        user_id: &str,
        name: &str,
        entities: &Value,
        notes: Option<&str>,
    ) -> Result<WatchlistRecord> {
        let mut conn = self.pool.acquire().await?;
        upsert_watchlist_on(&mut conn, id, user_id, name, entities, notes).await
    }

    pub async fn upsert_watchlist_scoped(
        &self,
        user_id: &str,
        role: &str,
        id: Option<Uuid>,
        name: &str,
        entities: &Value,
        notes: Option<&str>,
    ) -> Result<WatchlistRecord> {
        let mut tx = self.begin_scoped(user_id, role).await?;
        let record = upsert_watchlist_on(&mut tx, id, user_id, name, entities, notes).await?;
        tx.commit().await?;
        Ok(record)
    }

    pub async fn update_watchlist_scoped(
        &self,
        user_id: &str,
        role: &str,
        id: Uuid,
        name: &str,
        entities: &Value,
        notes: Option<&str>,
    ) -> Result<Option<WatchlistRecord>> {
        let mut tx = self.begin_scoped(user_id, role).await?;
        let record = update_watchlist_on(&mut tx, user_id, id, name, entities, notes).await?;
        tx.commit().await?;
        Ok(record)
    }

    pub async fn list_watchlists(&self, user_id: &str) -> Result<Vec<WatchlistRecord>> {
        let mut conn = self.pool.acquire().await?;
        list_watchlists_on(&mut conn, user_id).await
    }

    pub async fn list_watchlists_scoped(
        &self,
        user_id: &str,
        role: &str,
    ) -> Result<Vec<WatchlistRecord>> {
        let mut tx = self.begin_scoped(user_id, role).await?;
        let records = list_watchlists_on(&mut tx, user_id).await?;
        tx.commit().await?;
        Ok(records)
    }

    pub async fn delete_watchlist(&self, user_id: &str, id: Uuid) -> Result<bool> {
        let mut conn = self.pool.acquire().await?;
        delete_watchlist_on(&mut conn, user_id, id).await
    }

    pub async fn delete_watchlist_scoped(
        &self,
        user_id: &str,
        role: &str,
        id: Uuid,
    ) -> Result<bool> {
        let mut tx = self.begin_scoped(user_id, role).await?;
        let deleted = delete_watchlist_on(&mut tx, user_id, id).await?;
        tx.commit().await?;
        Ok(deleted)
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
        let mut conn = self.pool.acquire().await?;
        upsert_annotation_on(
            &mut conn,
            id,
            user_id,
            entity_type,
            entity_id,
            body,
            tags,
            visibility,
        )
        .await
    }

    /// Identity-scoped annotation write for RLS-forced `annotations`.
    pub async fn upsert_annotation_scoped(
        &self,
        user_id: &str,
        role: &str,
        id: Option<Uuid>,
        entity_type: &str,
        entity_id: &str,
        body: &str,
        tags: &[String],
        visibility: &str,
    ) -> Result<AnnotationRecord> {
        let mut tx = self.begin_scoped(user_id, role).await?;
        let record = upsert_annotation_on(
            &mut tx,
            id,
            user_id,
            entity_type,
            entity_id,
            body,
            tags,
            visibility,
        )
        .await?;
        tx.commit().await?;
        Ok(record)
    }

    pub async fn list_annotations(
        &self,
        user_id: &str,
        entity_type: Option<&str>,
        entity_id: Option<&str>,
    ) -> Result<Vec<AnnotationRecord>> {
        let mut conn = self.pool.acquire().await?;
        list_annotations_on(&mut conn, user_id, entity_type, entity_id).await
    }

    /// Identity-scoped annotation read for RLS-forced `annotations`.
    pub async fn list_annotations_scoped(
        &self,
        user_id: &str,
        role: &str,
        entity_type: Option<&str>,
        entity_id: Option<&str>,
    ) -> Result<Vec<AnnotationRecord>> {
        let mut tx = self.begin_scoped(user_id, role).await?;
        let records = list_annotations_on(&mut tx, user_id, entity_type, entity_id).await?;
        tx.commit().await?;
        Ok(records)
    }

    pub async fn delete_annotation(&self, user_id: &str, id: Uuid) -> Result<bool> {
        let mut conn = self.pool.acquire().await?;
        delete_annotation_on(&mut conn, user_id, id).await
    }

    /// Identity-scoped annotation delete for RLS-forced `annotations`.
    pub async fn delete_annotation_scoped(
        &self,
        user_id: &str,
        role: &str,
        id: Uuid,
    ) -> Result<bool> {
        let mut tx = self.begin_scoped(user_id, role).await?;
        let deleted = delete_annotation_on(&mut tx, user_id, id).await?;
        tx.commit().await?;
        Ok(deleted)
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

    #[allow(clippy::unwrap_used, clippy::expect_used)]
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
        let from_ts = from_date
            .and_hms_opt(0, 0, 0)
            .expect("midnight should be a valid time")
            .and_utc();
        let to_ts = to_date
            .and_hms_opt(23, 59, 59)
            .expect("end-of-day should be a valid time")
            .and_utc();
        qb.push_bind(from_ts)
            .push(" AND ts_utc <= ")
            .push_bind(to_ts);
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

    // ─── Strategic Opportunities ──────────────────────────────────────────────

    pub async fn list_strategic_opportunities(
        &self,
        include_closed: bool,
        limit: i64,
    ) -> Result<Vec<StrategicOpportunityRecord>> {
        let limit = clamp_limit(limit);
        let query = if include_closed {
            "SELECT id, title, description, opportunity_type, priority_score::double precision, confidence::double precision, entity_id, entity_type, region, estimated_value::double precision, recommended_actions, owner_id, status, due_date, metadata, created_at, updated_at FROM strategic_opportunities ORDER BY priority_score DESC, created_at DESC LIMIT $1"
        } else {
            "SELECT id, title, description, opportunity_type, priority_score::double precision, confidence::double precision, entity_id, entity_type, region, estimated_value::double precision, recommended_actions, owner_id, status, due_date, metadata, created_at, updated_at FROM strategic_opportunities WHERE status != 'closed' ORDER BY priority_score DESC, created_at DESC LIMIT $1"
        };
        Ok(sqlx::query_as::<_, StrategicOpportunityRecord>(query)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?)
    }

    pub async fn create_strategic_opportunity(
        &self,
        title: &str,
        description: Option<&str>,
        opportunity_type: &str,
        priority_score: f64,
        confidence: f64,
        entity_id: Option<Uuid>,
        entity_type: Option<&str>,
        region: Option<&str>,
        estimated_value: Option<&str>,
        recommended_actions: &Value,
        owner_id: Option<&str>,
        due_date: Option<DateTime<Utc>>,
    ) -> Result<StrategicOpportunityRecord> {
        Ok(sqlx::query_as::<_, StrategicOpportunityRecord>(
            r#"INSERT INTO strategic_opportunities
                 (title, description, opportunity_type, priority_score, confidence, entity_id, entity_type, region, estimated_value, recommended_actions, owner_id, due_date)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
               RETURNING id, title, description, opportunity_type, priority_score::double precision, confidence::double precision, entity_id, entity_type, region, estimated_value::double precision, recommended_actions, owner_id, status, due_date, metadata, created_at, updated_at"#,
        )
        .bind(title)
        .bind(description)
        .bind(opportunity_type)
        .bind(priority_score)
        .bind(confidence)
        .bind(entity_id)
        .bind(entity_type)
        .bind(region)
        .bind(estimated_value)
        .bind(recommended_actions)
        .bind(owner_id)
        .bind(due_date)
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn get_strategic_opportunity(
        &self,
        id: Uuid,
    ) -> Result<Option<StrategicOpportunityRecord>> {
        Ok(sqlx::query_as::<_, StrategicOpportunityRecord>(
            "SELECT id, title, description, opportunity_type, priority_score::double precision, confidence::double precision, entity_id, entity_type, region, estimated_value::double precision, recommended_actions, owner_id, status, due_date, metadata, created_at, updated_at FROM strategic_opportunities WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn update_strategic_opportunity_status(
        &self,
        id: Uuid,
        status: &str,
    ) -> Result<Option<StrategicOpportunityRecord>> {
        Ok(sqlx::query_as::<_, StrategicOpportunityRecord>(
            r#"UPDATE strategic_opportunities SET status = $2, updated_at = NOW()
               WHERE id = $1
               RETURNING id, title, description, opportunity_type, priority_score::double precision, confidence::double precision, entity_id, entity_type, region, estimated_value::double precision, recommended_actions, owner_id, status, due_date, metadata, created_at, updated_at"#,
        )
        .bind(id)
        .bind(status)
        .fetch_optional(&self.pool)
        .await?)
    }

    // ─── Critical Threats ─────────────────────────────────────────────────────

    pub async fn list_critical_threats(&self, limit: i64) -> Result<Vec<CriticalThreatRecord>> {
        let limit = clamp_limit(limit);
        Ok(sqlx::query_as::<_, CriticalThreatRecord>(
            "SELECT id, title, description, threat_type, severity, impact_score::double precision, confidence::double precision, entity_id, entity_type, region, mitigation_steps, owner_id, status, sla_deadline, resolved_at, metadata, created_at, updated_at FROM critical_threats WHERE status = 'active' ORDER BY impact_score DESC, created_at DESC LIMIT $1",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn create_critical_threat(
        &self,
        title: &str,
        description: Option<&str>,
        threat_type: &str,
        severity: &str,
        impact_score: f64,
        confidence: f64,
        entity_id: Option<Uuid>,
        entity_type: Option<&str>,
        region: Option<&str>,
        mitigation_steps: &Value,
        owner_id: Option<&str>,
        sla_deadline: Option<DateTime<Utc>>,
    ) -> Result<CriticalThreatRecord> {
        Ok(sqlx::query_as::<_, CriticalThreatRecord>(
            r#"INSERT INTO critical_threats
                 (title, description, threat_type, severity, impact_score, confidence, entity_id, entity_type, region, mitigation_steps, owner_id, sla_deadline)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
               RETURNING id, title, description, threat_type, severity, impact_score::double precision, confidence::double precision, entity_id, entity_type, region, mitigation_steps, owner_id, status, sla_deadline, resolved_at, metadata, created_at, updated_at"#,
        )
        .bind(title)
        .bind(description)
        .bind(threat_type)
        .bind(severity)
        .bind(impact_score)
        .bind(confidence)
        .bind(entity_id)
        .bind(entity_type)
        .bind(region)
        .bind(mitigation_steps)
        .bind(owner_id)
        .bind(sla_deadline)
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn get_critical_threat(&self, id: Uuid) -> Result<Option<CriticalThreatRecord>> {
        Ok(sqlx::query_as::<_, CriticalThreatRecord>(
            "SELECT id, title, description, threat_type, severity, impact_score::double precision, confidence::double precision, entity_id, entity_type, region, mitigation_steps, owner_id, status, sla_deadline, resolved_at, metadata, created_at, updated_at FROM critical_threats WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn update_critical_threat_status(
        &self,
        id: Uuid,
        status: &str,
        resolved: bool,
    ) -> Result<Option<CriticalThreatRecord>> {
        let resolved_at = if resolved { "NOW()" } else { "NULL" };
        let sql = format!(
            r#"UPDATE critical_threats SET status = $2, resolved_at = {resolved_at}, updated_at = NOW()
               WHERE id = $1
               RETURNING id, title, description, threat_type, severity, impact_score::double precision, confidence::double precision, entity_id, entity_type, region, mitigation_steps, owner_id, status, sla_deadline, resolved_at, metadata, created_at, updated_at"#
        );
        Ok(sqlx::query_as::<_, CriticalThreatRecord>(&sql)
            .bind(id)
            .bind(status)
            .fetch_optional(&self.pool)
            .await?)
    }

    // ─── Investigation Workspaces ─────────────────────────────────────────────

    pub async fn list_investigation_workspaces(
        &self,
        limit: i64,
    ) -> Result<Vec<InvestigationWorkspaceRecord>> {
        let limit = clamp_limit(limit);
        Ok(sqlx::query_as::<_, InvestigationWorkspaceRecord>(
            "SELECT id, name, description, workspace_type, owner_id, team_id, status, visibility, tags, entity_focus, findings, conclusions, metadata, created_at, updated_at, closed_at FROM investigation_workspaces ORDER BY updated_at DESC LIMIT $1",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Open investigation workspaces focused on one entity. Filters in SQL
    /// (status + `entity_focus` containment) so the entity dossier does not
    /// fetch the whole workspace table, and matching rows beyond a recent
    /// window are not silently dropped. Accepts both bare id arrays
    /// (`["<uuid>"]`) and object entries (`{"id"|"entity_id": "<uuid>"}`).
    pub async fn list_investigation_workspaces_for_entity(
        &self,
        entity_id: &str,
        limit: i64,
    ) -> Result<Vec<InvestigationWorkspaceRecord>> {
        let limit = clamp_limit(limit);
        Ok(sqlx::query_as::<_, InvestigationWorkspaceRecord>(
            r#"
            SELECT id, name, description, workspace_type, owner_id, team_id, status,
                   visibility, tags, entity_focus, findings, conclusions, metadata,
                   created_at, updated_at, closed_at
            FROM investigation_workspaces
            WHERE status NOT IN ('closed', 'archived')
              AND (
                    entity_focus @> jsonb_build_array($1::text)
                 OR entity_focus @> jsonb_build_array(jsonb_build_object('id', $1::text))
                 OR entity_focus @> jsonb_build_array(jsonb_build_object('entity_id', $1::text))
                 OR entity_focus->>'id' = $1
                 OR entity_focus->>'entity_id' = $1
              )
            ORDER BY updated_at DESC
            LIMIT $2
            "#,
        )
        .bind(entity_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn create_investigation_workspace(
        &self,
        name: &str,
        description: Option<&str>,
        workspace_type: &str,
        owner_id: &str,
        team_id: Option<&str>,
        visibility: &str,
        tags: &[String],
        entity_focus: &Value,
    ) -> Result<InvestigationWorkspaceRecord> {
        Ok(sqlx::query_as::<_, InvestigationWorkspaceRecord>(
            r#"INSERT INTO investigation_workspaces
                 (name, description, workspace_type, owner_id, team_id, visibility, tags, entity_focus)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
               RETURNING id, name, description, workspace_type, owner_id, team_id, status, visibility, tags, entity_focus, findings, conclusions, metadata, created_at, updated_at, closed_at"#,
        )
        .bind(name)
        .bind(description)
        .bind(workspace_type)
        .bind(owner_id)
        .bind(team_id)
        .bind(visibility)
        .bind(tags)
        .bind(entity_focus)
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn get_investigation_workspace(
        &self,
        id: Uuid,
    ) -> Result<Option<InvestigationWorkspaceRecord>> {
        Ok(sqlx::query_as::<_, InvestigationWorkspaceRecord>(
            "SELECT id, name, description, workspace_type, owner_id, team_id, status, visibility, tags, entity_focus, findings, conclusions, metadata, created_at, updated_at, closed_at FROM investigation_workspaces WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn update_investigation_workspace(
        &self,
        id: Uuid,
        name: Option<&str>,
        description: Option<Option<&str>>,
        status: Option<&str>,
        tags: Option<&[String]>,
        entity_focus: Option<&Value>,
        findings: Option<Option<&str>>,
        conclusions: Option<Option<&str>>,
    ) -> Result<Option<InvestigationWorkspaceRecord>> {
        let current = match self.get_investigation_workspace(id).await? {
            Some(c) => c,
            None => return Ok(None),
        };
        Ok(sqlx::query_as::<_, InvestigationWorkspaceRecord>(
            r#"UPDATE investigation_workspaces SET
                 name = $2,
                 description = $3,
                 status = $4,
                 tags = $5,
                 entity_focus = $6,
                 findings = $7,
                 conclusions = $8,
                 closed_at = CASE WHEN $4 = 'closed' THEN COALESCE(closed_at, NOW()) ELSE closed_at END,
                 updated_at = NOW()
               WHERE id = $1
               RETURNING id, name, description, workspace_type, owner_id, team_id, status, visibility, tags, entity_focus, findings, conclusions, metadata, created_at, updated_at, closed_at"#,
        )
        .bind(id)
        .bind(name.unwrap_or(&current.name))
        .bind(description.unwrap_or(current.description.as_deref()))
        .bind(status.unwrap_or(&current.status))
        .bind(tags.unwrap_or(&current.tags))
        .bind(entity_focus.unwrap_or(&current.entity_focus))
        .bind(findings.unwrap_or(current.findings.as_deref()))
        .bind(conclusions.unwrap_or(current.conclusions.as_deref()))
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn delete_investigation_workspace(&self, id: Uuid) -> Result<bool> {
        let result = sqlx::query("DELETE FROM investigation_workspaces WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    // ─── Workspace Assignments ────────────────────────────────────────────────

    pub async fn list_workspace_assignments(
        &self,
        workspace_id: Uuid,
    ) -> Result<Vec<WorkspaceAssignmentRecord>> {
        Ok(sqlx::query_as::<_, WorkspaceAssignmentRecord>(
            "SELECT id, workspace_id, user_id, role, assigned_by, assigned_at, updated_at FROM workspace_assignments WHERE workspace_id = $1 ORDER BY assigned_at ASC",
        )
        .bind(workspace_id)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn create_workspace_assignment(
        &self,
        workspace_id: Uuid,
        user_id: &str,
        role: &str,
        assigned_by: &str,
    ) -> Result<WorkspaceAssignmentRecord> {
        Ok(sqlx::query_as::<_, WorkspaceAssignmentRecord>(
            r#"INSERT INTO workspace_assignments (workspace_id, user_id, role, assigned_by)
               VALUES ($1, $2, $3, $4)
               RETURNING id, workspace_id, user_id, role, assigned_by, assigned_at, updated_at"#,
        )
        .bind(workspace_id)
        .bind(user_id)
        .bind(role)
        .bind(assigned_by)
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn remove_workspace_assignment(
        &self,
        workspace_id: Uuid,
        user_id: &str,
    ) -> Result<bool> {
        let result = sqlx::query(
            "DELETE FROM workspace_assignments WHERE workspace_id = $1 AND user_id = $2",
        )
        .bind(workspace_id)
        .bind(user_id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    // ─── Activity Feed ────────────────────────────────────────────────────────

    pub async fn list_activity_feed(
        &self,
        workspace_id: Option<Uuid>,
        team_id: Option<&str>,
        actor_id: Option<&str>,
        limit: i64,
    ) -> Result<Vec<ActivityFeedRecord>> {
        let limit = clamp_limit(limit);
        Ok(sqlx::query_as::<_, ActivityFeedRecord>(
            "SELECT id, actor_id, actor_name, action_type, entity_type, entity_id, entity_name, details, workspace_id, team_id, visibility, created_at FROM activity_feed WHERE ($1::uuid IS NULL OR workspace_id = $1) AND ($2::text IS NULL OR team_id = $2) AND ($3::text IS NULL OR actor_id = $3) ORDER BY created_at DESC LIMIT $4",
        )
        .bind(workspace_id)
        .bind(team_id)
        .bind(actor_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn create_activity_entry(
        &self,
        actor_id: &str,
        actor_name: &str,
        action_type: &str,
        entity_type: Option<&str>,
        entity_id: Option<&str>,
        entity_name: Option<&str>,
        details: &Value,
        workspace_id: Option<Uuid>,
        team_id: Option<&str>,
        visibility: &str,
    ) -> Result<ActivityFeedRecord> {
        Ok(sqlx::query_as::<_, ActivityFeedRecord>(
            r#"INSERT INTO activity_feed
                 (actor_id, actor_name, action_type, entity_type, entity_id, entity_name, details, workspace_id, team_id, visibility)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
               RETURNING id, actor_id, actor_name, action_type, entity_type, entity_id, entity_name, details, workspace_id, team_id, visibility, created_at"#,
        )
        .bind(actor_id)
        .bind(actor_name)
        .bind(action_type)
        .bind(entity_type)
        .bind(entity_id)
        .bind(entity_name)
        .bind(details)
        .bind(workspace_id)
        .bind(team_id)
        .bind(visibility)
        .fetch_one(&self.pool)
        .await?)
    }

    // ─── Investigation Shares ─────────────────────────────────────────────────

    pub async fn list_investigation_shares(
        &self,
        workspace_id: Uuid,
    ) -> Result<Vec<InvestigationShareRecord>> {
        Ok(sqlx::query_as::<_, InvestigationShareRecord>(
            "SELECT id, workspace_id, shared_by, shared_with, share_type, access_level, message, expires_at, created_at FROM investigation_shares WHERE workspace_id = $1 ORDER BY created_at DESC",
        )
        .bind(workspace_id)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn create_investigation_share(
        &self,
        workspace_id: Uuid,
        shared_by: &str,
        shared_with: &str,
        share_type: &str,
        access_level: &str,
        message: Option<&str>,
        expires_at: Option<DateTime<Utc>>,
    ) -> Result<InvestigationShareRecord> {
        Ok(sqlx::query_as::<_, InvestigationShareRecord>(
            r#"INSERT INTO investigation_shares
                 (workspace_id, shared_by, shared_with, share_type, access_level, message, expires_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7)
               RETURNING id, workspace_id, shared_by, shared_with, share_type, access_level, message, expires_at, created_at"#,
        )
        .bind(workspace_id)
        .bind(shared_by)
        .bind(shared_with)
        .bind(share_type)
        .bind(access_level)
        .bind(message)
        .bind(expires_at)
        .fetch_one(&self.pool)
        .await?)
    }

    // ─── Daily Priority Queue ─────────────────────────────────────────────────

    pub async fn list_priority_queue_items(
        &self,
        user_id: &str,
        status: Option<&str>,
        limit: i64,
    ) -> Result<Vec<PriorityQueueItemRecord>> {
        let limit = clamp_limit(limit);
        Ok(sqlx::query_as::<_, PriorityQueueItemRecord>(
            "SELECT id, user_id, queue_date, item_type, item_id, item_title, priority, status, notes, completed_at, created_at, updated_at FROM priority_queue WHERE user_id = $1 AND ($2::text IS NULL OR status = $2) ORDER BY queue_date DESC, priority DESC LIMIT $3",
        )
        .bind(user_id)
        .bind(status)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn create_priority_queue_item(
        &self,
        user_id: &str,
        item_type: &str,
        item_id: Uuid,
        item_title: &str,
        priority: i32,
        notes: Option<&str>,
    ) -> Result<PriorityQueueItemRecord> {
        Ok(sqlx::query_as::<_, PriorityQueueItemRecord>(
            r#"INSERT INTO priority_queue
                 (user_id, queue_date, item_type, item_id, item_title, priority, notes)
               VALUES ($1, CURRENT_DATE, $2, $3, $4, $5, $6)
               ON CONFLICT (user_id, queue_date, item_type, item_id) DO UPDATE SET
                 item_title = EXCLUDED.item_title,
                 priority = EXCLUDED.priority,
                 notes = EXCLUDED.notes,
                 updated_at = NOW()
               RETURNING id, user_id, queue_date, item_type, item_id, item_title, priority, status, notes, completed_at, created_at, updated_at"#,
        )
        .bind(user_id)
        .bind(item_type)
        .bind(item_id)
        .bind(item_title)
        .bind(priority)
        .bind(notes)
        .fetch_one(&self.pool)
        .await?)
    }

    /// Service-path queue update. The `user_id` predicate is mandatory: the
    /// default `service` identity bypasses per-user RLS, so without it any
    /// caller could modify another user's queue item.
    pub async fn update_priority_queue_item(
        &self,
        user_id: &str,
        id: Uuid,
        priority: Option<i32>,
        status: Option<&str>,
        notes: Option<Option<&str>>,
    ) -> Result<Option<PriorityQueueItemRecord>> {
        let mut conn = self.pool.acquire().await?;
        update_priority_queue_item_on(&mut conn, user_id, id, priority, status, notes).await
    }

    /// Identity-scoped queue update for the RLS-forced `priority_queue` table.
    pub async fn update_priority_queue_item_scoped(
        &self,
        user_id: &str,
        role: &str,
        id: Uuid,
        priority: Option<i32>,
        status: Option<&str>,
        notes: Option<Option<&str>>,
    ) -> Result<Option<PriorityQueueItemRecord>> {
        let mut tx = self.begin_scoped(user_id, role).await?;
        let record =
            update_priority_queue_item_on(&mut tx, user_id, id, priority, status, notes).await?;
        tx.commit().await?;
        Ok(record)
    }

    // ─── Supplier Risk Entries ────────────────────────────────────────────────

    pub async fn list_supplier_risk_entries(
        &self,
        status: Option<&str>,
        limit: i64,
    ) -> Result<Vec<SupplierRiskEntryRecord>> {
        let limit = clamp_limit(limit);
        Ok(sqlx::query_as::<_, SupplierRiskEntryRecord>(
            "SELECT id, supplier_id, risk_category, risk_score::double precision, risk_factors, mitigation, owner_id, status, last_reviewed, next_review, created_at, updated_at FROM supplier_risk WHERE ($1::text IS NULL OR status = $1) ORDER BY risk_score DESC, created_at DESC LIMIT $2",
        )
        .bind(status)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn create_supplier_risk_entry(
        &self,
        supplier_id: &str,
        risk_category: &str,
        risk_score: f64,
        risk_factors: &Value,
        mitigation: Option<&str>,
        owner_id: Option<&str>,
    ) -> Result<SupplierRiskEntryRecord> {
        Ok(sqlx::query_as::<_, SupplierRiskEntryRecord>(
            r#"INSERT INTO supplier_risk
                 (supplier_id, risk_category, risk_score, risk_factors, mitigation, owner_id)
               VALUES ($1, $2, $3, $4, $5, $6)
               RETURNING id, supplier_id, risk_category, risk_score::double precision, risk_factors, mitigation, owner_id, status, last_reviewed, next_review, created_at, updated_at"#,
        )
        .bind(supplier_id)
        .bind(risk_category)
        .bind(risk_score)
        .bind(risk_factors)
        .bind(mitigation)
        .bind(owner_id)
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn update_supplier_risk_entry(
        &self,
        id: Uuid,
        risk_score: Option<f64>,
        mitigation: Option<&str>,
        status: Option<&str>,
    ) -> Result<Option<SupplierRiskEntryRecord>> {
        let current = match sqlx::query_as::<_, SupplierRiskEntryRecord>(
            "SELECT id, supplier_id, risk_category, risk_score::double precision, risk_factors, mitigation, owner_id, status, last_reviewed, next_review, created_at, updated_at FROM supplier_risk WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await? {
            Some(c) => c,
            None => return Ok(None),
        };

        Ok(sqlx::query_as::<_, SupplierRiskEntryRecord>(
            r#"UPDATE supplier_risk SET
                 risk_score = $2,
                 mitigation = $3,
                 status = $4,
                 last_reviewed = NOW(),
                 updated_at = NOW()
               WHERE id = $1
               RETURNING id, supplier_id, risk_category, risk_score::double precision, risk_factors, mitigation, owner_id, status, last_reviewed, next_review, created_at, updated_at"#,
        )
        .bind(id)
        .bind(risk_score.unwrap_or(current.risk_score))
        .bind(mitigation)
        .bind(status.unwrap_or(&current.status))
        .fetch_optional(&self.pool)
        .await?)
    }

    // ─── Pipeline Opportunities ───────────────────────────────────────────────

    pub async fn list_pipeline_opportunities(
        &self,
        stage: Option<&str>,
        owner_id: Option<&str>,
        limit: i64,
    ) -> Result<Vec<PipelineOpportunityRecord>> {
        let limit = clamp_limit(limit);
        Ok(sqlx::query_as::<_, PipelineOpportunityRecord>(
            "SELECT id, opportunity_id, title, stage, value_estimate::double precision, probability::double precision, owner_id, expected_close, actual_close, notes, metadata, created_at, updated_at, closed_at FROM pipeline_opportunities WHERE ($1::text IS NULL OR stage = $1) AND ($2::text IS NULL OR owner_id = $2) ORDER BY created_at DESC LIMIT $3",
        )
        .bind(stage)
        .bind(owner_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Pipeline opportunities attached to one company (migration 043 column),
    /// used by the entity dossier "pipeline status" section.
    pub async fn list_pipeline_opportunities_for_company(
        &self,
        company_id: Uuid,
        limit: i64,
    ) -> Result<Vec<PipelineOpportunityRecord>> {
        let limit = clamp_limit(limit);
        Ok(sqlx::query_as::<_, PipelineOpportunityRecord>(
            "SELECT id, opportunity_id, title, stage, value_estimate::double precision, probability::double precision, owner_id, expected_close, actual_close, notes, metadata, created_at, updated_at, closed_at FROM pipeline_opportunities WHERE company_id = $1 ORDER BY created_at DESC LIMIT $2",
        )
        .bind(company_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn create_pipeline_opportunity(
        &self,
        opportunity_id: Option<&str>,
        title: &str,
        stage: &str,
        value_estimate: Option<f64>,
        probability: f64,
        owner_id: Option<&str>,
        expected_close: Option<NaiveDate>,
        notes: Option<&str>,
    ) -> Result<PipelineOpportunityRecord> {
        Ok(sqlx::query_as::<_, PipelineOpportunityRecord>(
            r#"INSERT INTO pipeline_opportunities
                 (opportunity_id, title, stage, value_estimate, probability, owner_id, expected_close, notes)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
               RETURNING id, opportunity_id, title, stage, value_estimate::double precision, probability::double precision, owner_id, expected_close, actual_close, notes, metadata, created_at, updated_at, closed_at"#,
        )
        .bind(opportunity_id)
        .bind(title)
        .bind(stage)
        .bind(value_estimate)
        .bind(probability)
        .bind(owner_id)
        .bind(expected_close)
        .bind(notes)
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn update_pipeline_stage(
        &self,
        id: Uuid,
        stage: &str,
        notes: Option<&str>,
    ) -> Result<Option<PipelineOpportunityRecord>> {
        let closed_at_expr = if stage == "closed_won" || stage == "closed_lost" {
            "COALESCE(closed_at, NOW())"
        } else {
            "closed_at"
        };
        let sql = format!(
            r#"UPDATE pipeline_opportunities SET
                 stage = $2,
                 notes = COALESCE($3, notes),
                 actual_close = CASE WHEN $2 IN ('closed_won','closed_lost') THEN COALESCE(actual_close, CURRENT_DATE) ELSE actual_close END,
                 closed_at = {closed_at_expr},
                 updated_at = NOW()
               WHERE id = $1
               RETURNING id, opportunity_id, title, stage, value_estimate::double precision, probability::double precision, owner_id, expected_close, actual_close, notes, metadata, created_at, updated_at, closed_at"#
        );
        Ok(sqlx::query_as::<_, PipelineOpportunityRecord>(&sql)
            .bind(id)
            .bind(stage)
            .bind(notes)
            .fetch_optional(&self.pool)
            .await?)
    }

    // ─── Source Evidence ──────────────────────────────────────────────────────

    pub async fn list_source_evidence(
        &self,
        entity_type: Option<&str>,
        entity_id: Option<&str>,
        evidence_type: Option<&str>,
        limit: i64,
    ) -> Result<Vec<SourceEvidenceRecord>> {
        let limit = clamp_limit(limit);
        Ok(sqlx::query_as::<_, SourceEvidenceRecord>(
            "SELECT id, entity_type, entity_id, evidence_type, source_url, source_domain, source_name, reliability_score::double precision, content_hash, excerpt, metadata, created_at FROM source_evidence WHERE ($1::text IS NULL OR entity_type = $1) AND ($2::text IS NULL OR entity_id = $2) AND ($3::text IS NULL OR evidence_type = $3) ORDER BY created_at DESC LIMIT $4",
        )
        .bind(entity_type)
        .bind(entity_id)
        .bind(evidence_type)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn create_source_evidence(
        &self,
        entity_type: &str,
        entity_id: &str,
        evidence_type: &str,
        source_url: &str,
        source_domain: Option<&str>,
        source_name: Option<&str>,
        reliability_score: f64,
        excerpt: Option<&str>,
        metadata: &Value,
    ) -> Result<SourceEvidenceRecord> {
        Ok(sqlx::query_as::<_, SourceEvidenceRecord>(
            r#"INSERT INTO source_evidence
                 (entity_type, entity_id, evidence_type, source_url, source_domain, source_name, reliability_score, excerpt, metadata)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
               RETURNING id, entity_type, entity_id, evidence_type, source_url, source_domain, source_name, reliability_score::double precision, content_hash, excerpt, metadata, created_at"#,
        )
        .bind(entity_type)
        .bind(entity_id)
        .bind(evidence_type)
        .bind(source_url)
        .bind(source_domain)
        .bind(source_name)
        .bind(reliability_score)
        .bind(excerpt)
        .bind(metadata)
        .fetch_one(&self.pool)
        .await?)
    }

    // ─── Team Assignments ─────────────────────────────────────────────────────

    pub async fn list_team_assignments(
        &self,
        entity_type: Option<&str>,
        entity_id: Option<&str>,
    ) -> Result<Vec<TeamAssignmentRecord>> {
        Ok(sqlx::query_as::<_, TeamAssignmentRecord>(
            "SELECT id, team_id, team_name, entity_type, entity_id, assigned_by, assigned_to, role, notes, created_at, updated_at FROM team_assignments WHERE ($1::text IS NULL OR entity_type = $1) AND ($2::text IS NULL OR entity_id = $2) ORDER BY created_at DESC",
        )
        .bind(entity_type)
        .bind(entity_id)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn create_team_assignment(
        &self,
        team_id: &str,
        team_name: &str,
        entity_type: &str,
        entity_id: &str,
        assigned_by: &str,
        assigned_to: &str,
        role: &str,
        notes: Option<&str>,
    ) -> Result<TeamAssignmentRecord> {
        Ok(sqlx::query_as::<_, TeamAssignmentRecord>(
            r#"INSERT INTO team_assignments
                 (team_id, team_name, entity_type, entity_id, assigned_by, assigned_to, role, notes)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
               ON CONFLICT (entity_type, entity_id, team_id) DO UPDATE SET
                 team_name = EXCLUDED.team_name,
                 assigned_to = EXCLUDED.assigned_to,
                 role = EXCLUDED.role,
                 notes = EXCLUDED.notes,
                 updated_at = NOW()
               RETURNING id, team_id, team_name, entity_type, entity_id, assigned_by, assigned_to, role, notes, created_at, updated_at"#,
        )
        .bind(team_id)
        .bind(team_name)
        .bind(entity_type)
        .bind(entity_id)
        .bind(assigned_by)
        .bind(assigned_to)
        .bind(role)
        .bind(notes)
        .fetch_one(&self.pool)
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
