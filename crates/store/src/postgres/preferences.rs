use super::*;

fn merge_settings_page_preferences(
    existing: Option<Value>,
    prefs: &UserSettingsPrefs,
) -> Result<Value> {
    let mut preferences = existing
        .filter(Value::is_object)
        .unwrap_or_else(|| serde_json::json!({}));
    if let Some(obj) = preferences.as_object_mut() {
        obj.insert("settings_page".to_string(), serde_json::to_value(prefs)?);
    }
    Ok(preferences)
}

async fn fetch_user_settings_prefs(
    conn: &mut sqlx::PgConnection,
    user_id: &str,
) -> Result<Option<UserSettingsPrefs>> {
    let row = sqlx::query("SELECT preferences FROM user_preferences WHERE user_id = $1")
        .bind(user_id)
        .fetch_optional(&mut *conn)
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

async fn fetch_user_preferences_record(
    conn: &mut sqlx::PgConnection,
    user_id: &str,
) -> Result<Option<UserPreferencesRecord>> {
    let row = sqlx::query(
        "SELECT theme, locale, preferences, updated_at FROM user_preferences WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_optional(&mut *conn)
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

async fn upsert_user_preferences_record_on(
    conn: &mut sqlx::PgConnection,
    user_id: &str,
    theme: &str,
    locale: &str,
    preferences: &Value,
) -> Result<()> {
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
    .execute(&mut *conn)
    .await?;

    Ok(())
}

async fn upsert_user_settings_prefs_on(
    conn: &mut sqlx::PgConnection,
    user_id: &str,
    prefs: &UserSettingsPrefs,
) -> Result<()> {
    let row =
        sqlx::query("SELECT preferences, theme, locale FROM user_preferences WHERE user_id = $1")
            .bind(user_id)
            .fetch_optional(&mut *conn)
            .await?;

    let (theme, locale, existing_preferences): (String, String, Option<Value>) =
        if let Some(row) = row {
            let theme: Option<String> = row.try_get("theme")?;
            let locale: Option<String> = row.try_get("locale")?;
            let preferences: Option<Value> = row.try_get("preferences")?;
            (
                theme.unwrap_or_else(|| "system".to_string()),
                locale.unwrap_or_else(|| "en".to_string()),
                preferences,
            )
        } else {
            ("system".to_string(), "en".to_string(), None)
        };

    let preferences = merge_settings_page_preferences(existing_preferences, prefs)?;

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
    .execute(&mut *conn)
    .await?;

    Ok(())
}

impl PgStore {
    pub async fn get_user_settings_prefs(
        &self,
        user_id: &str,
    ) -> Result<Option<UserSettingsPrefs>> {
        let mut conn = self.pool.acquire().await?;
        fetch_user_settings_prefs(&mut conn, user_id).await
    }

    pub async fn get_user_settings_prefs_scoped(
        &self,
        user_id: &str,
        role: &str,
    ) -> Result<Option<UserSettingsPrefs>> {
        let mut tx = self.begin_scoped(user_id, role).await?;
        let prefs = fetch_user_settings_prefs(&mut tx, user_id).await?;
        tx.commit().await?;
        Ok(prefs)
    }

    pub async fn get_user_preferences_record(
        &self,
        user_id: &str,
    ) -> Result<Option<UserPreferencesRecord>> {
        let mut conn = self.pool.acquire().await?;
        fetch_user_preferences_record(&mut conn, user_id).await
    }

    pub async fn get_user_preferences_record_scoped(
        &self,
        user_id: &str,
        role: &str,
    ) -> Result<Option<UserPreferencesRecord>> {
        let mut tx = self.begin_scoped(user_id, role).await?;
        let record = fetch_user_preferences_record(&mut tx, user_id).await?;
        tx.commit().await?;
        Ok(record)
    }

    pub async fn upsert_user_preferences_record(
        &self,
        user_id: &str,
        theme: &str,
        locale: &str,
        preferences: &Value,
    ) -> Result<()> {
        let mut conn = self.pool.acquire().await?;
        upsert_user_preferences_record_on(&mut conn, user_id, theme, locale, preferences).await
    }

    pub async fn upsert_user_preferences_record_scoped(
        &self,
        user_id: &str,
        role: &str,
        theme: &str,
        locale: &str,
        preferences: &Value,
    ) -> Result<()> {
        let mut tx = self.begin_scoped(user_id, role).await?;
        upsert_user_preferences_record_on(&mut tx, user_id, theme, locale, preferences).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn upsert_user_settings_prefs(
        &self,
        user_id: &str,
        prefs: &UserSettingsPrefs,
    ) -> Result<()> {
        let mut conn = self.pool.acquire().await?;
        upsert_user_settings_prefs_on(&mut conn, user_id, prefs).await
    }

    pub async fn upsert_user_settings_prefs_scoped(
        &self,
        user_id: &str,
        role: &str,
        prefs: &UserSettingsPrefs,
    ) -> Result<()> {
        let mut tx = self.begin_scoped(user_id, role).await?;
        upsert_user_settings_prefs_on(&mut tx, user_id, prefs).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn list_user_settings_prefs_for_email_digest(
        &self,
    ) -> Result<Vec<(String, UserSettingsPrefs)>> {
        let rows = sqlx::query(
            "SELECT user_id, preferences->'settings_page' AS settings FROM user_preferences WHERE preferences ? 'settings_page'",
        )
        .fetch_all(&self.pool)
        .await?;

        let mut out = Vec::new();
        for row in rows {
            let user_id: String = row.try_get("user_id")?;
            let settings: Option<Value> = row.try_get("settings")?;
            let Some(settings) = settings else {
                continue;
            };
            let Ok(prefs) = serde_json::from_value::<UserSettingsPrefs>(settings) else {
                continue;
            };
            if prefs.email_digest_enabled {
                out.push((user_id, prefs));
            }
        }
        Ok(out)
    }

    pub async fn mark_email_digest_sent(
        &self,
        user_id: &str,
        sent_at: DateTime<Utc>,
    ) -> Result<()> {
        let sent_at_s = sent_at.to_rfc3339();
        sqlx::query(
            r#"UPDATE user_preferences
               SET preferences = jsonb_set(
                    COALESCE(preferences, '{}'::jsonb),
                    '{settings_page,email_digest_last_sent_at}',
                    to_jsonb($2::text),
                    true
               ),
               updated_at = NOW()
               WHERE user_id = $1"#,
        )
        .bind(user_id)
        .bind(sent_at_s)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn test_merge_settings_page_preferences_preserves_existing_keys() {
        let prefs = UserSettingsPrefs {
            email_digest_enabled: true,
            email_digest_categories: vec!["warnings".to_string()],
            ..UserSettingsPrefs::default()
        };

        let merged =
            merge_settings_page_preferences(Some(serde_json::json!({"theme": "dark"})), &prefs)
                .unwrap();

        assert_eq!(merged["theme"], "dark");
        assert_eq!(merged["settings_page"]["email_digest_enabled"], true);
        assert_eq!(
            merged["settings_page"]["email_digest_categories"],
            serde_json::json!(["warnings"])
        );
    }

    #[test]
    fn test_merge_settings_page_preferences_replaces_non_object_root() {
        let merged = merge_settings_page_preferences(
            Some(serde_json::json!(["stale"])),
            &UserSettingsPrefs::default(),
        )
        .unwrap();

        assert!(merged.is_object());
        assert_eq!(merged["settings_page"]["email_digest_time_cet"], "08:00");
        assert_eq!(merged["settings_page"]["email_digest_weekday"], "Mon");
    }
}
