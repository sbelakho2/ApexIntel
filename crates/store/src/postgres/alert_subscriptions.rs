//! Store methods for per-user alert subscriptions.
//!
//! Backs `user_alert_subscriptions` (migration 048). Each row opts one user
//! into entity-scoped alerts above a severity floor; `category` narrows the
//! subscription to one alert category, and is `NULL` for "every category".

use super::*;

use apex_core::alert_config::{user_principal_id, AlertSeverity};

/// One row of `user_alert_subscriptions`.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow, serde::Serialize, serde::Deserialize)]
pub struct UserAlertSubscriptionRecord {
    pub id: Uuid,
    pub user_id: String,
    pub entity_id: Uuid,
    /// `None` means the subscription covers every alert category.
    pub category: Option<String>,
    pub min_severity: String,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Ordinal rank used to compare a subscription's `min_severity` with the
/// severity of an incoming alert. Must match the `CASE` in the query.
fn severity_rank(severity: AlertSeverity) -> i16 {
    match severity {
        AlertSeverity::Info => 0,
        AlertSeverity::Low => 1,
        AlertSeverity::Medium => 2,
        AlertSeverity::High => 3,
        AlertSeverity::Critical => 4,
    }
}

/// Normalise an alert category to the form stored in the table: trimmed and
/// lowercased. The empty string is treated as "every category" (`None`).
pub(crate) fn normalize_alert_category(category: Option<&str>) -> Option<String> {
    category
        .map(|value| value.trim().to_lowercase())
        .filter(|value| !value.is_empty())
}

impl PgStore {
    /// Insert or update the subscription for one `(user, entity, category)`
    /// triple. `category = None` covers every category.
    ///
    /// The `min_severity` input is lowercased before storage so the unique
    /// expression index (`lower(category)` / `COALESCE(category, '*')`) and the
    /// `CHECK` constraint stay consistent with the values the alert router
    /// compares against.
    pub async fn upsert_user_alert_subscription(
        &self,
        user_id: &str,
        entity_id: Uuid,
        category: Option<&str>,
        min_severity: &str,
        enabled: bool,
    ) -> Result<UserAlertSubscriptionRecord> {
        let category = normalize_alert_category(category);
        let record: UserAlertSubscriptionRecord = sqlx::query_as(
            r#"
            INSERT INTO user_alert_subscriptions
                (user_id, entity_id, category, min_severity, enabled, updated_at)
            VALUES ($1, $2, $3, lower($4), $5, now())
            ON CONFLICT (user_id, entity_id, (COALESCE(lower(category), '*')))
            DO UPDATE SET
                category = EXCLUDED.category,
                min_severity = EXCLUDED.min_severity,
                enabled = EXCLUDED.enabled,
                updated_at = now()
            RETURNING id, user_id, entity_id, category, min_severity, enabled,
                      created_at, updated_at
            "#,
        )
        .bind(user_id)
        .bind(entity_id)
        .bind(category.as_deref())
        .bind(min_severity)
        .bind(enabled)
        .fetch_one(&self.pool)
        .await?;

        Ok(record)
    }

    /// Fetch one subscription for a `(user, entity, category)` triple.
    /// `category = None` looks up the "every category" row.
    pub async fn get_user_alert_subscription(
        &self,
        user_id: &str,
        entity_id: Uuid,
        category: Option<&str>,
    ) -> Result<Option<UserAlertSubscriptionRecord>> {
        let category = normalize_alert_category(category);
        let record: Option<UserAlertSubscriptionRecord> = sqlx::query_as(
            r#"
            SELECT id, user_id, entity_id, category, min_severity, enabled,
                   created_at, updated_at
            FROM user_alert_subscriptions
            WHERE user_id = $1
              AND entity_id = $2
              AND COALESCE(lower(category), '*') = COALESCE($3, '*')
            "#,
        )
        .bind(user_id)
        .bind(entity_id)
        .bind(category.as_deref())
        .fetch_optional(&self.pool)
        .await?;

        Ok(record)
    }

    /// Delete one subscription for a `(user, entity, category)` triple.
    /// Returns `true` when a row was removed.
    pub async fn delete_user_alert_subscription(
        &self,
        user_id: &str,
        entity_id: Uuid,
        category: Option<&str>,
    ) -> Result<bool> {
        let category = normalize_alert_category(category);
        let result = sqlx::query(
            r#"
            DELETE FROM user_alert_subscriptions
            WHERE user_id = $1
              AND entity_id = $2
              AND COALESCE(lower(category), '*') = COALESCE($3, '*')
            "#,
        )
        .bind(user_id)
        .bind(entity_id)
        .bind(category.as_deref())
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    /// List every alert subscription owned by one user, newest first.
    pub async fn list_user_alert_subscriptions(
        &self,
        user_id: &str,
    ) -> Result<Vec<UserAlertSubscriptionRecord>> {
        let records: Vec<UserAlertSubscriptionRecord> = sqlx::query_as(
            r#"
            SELECT id, user_id, entity_id, category, min_severity, enabled,
                   created_at, updated_at
            FROM user_alert_subscriptions
            WHERE user_id = $1
            ORDER BY updated_at DESC, entity_id
            "#,
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(records)
    }

    /// Find the users subscribed to a given entity's alerts.
    ///
    /// A subscription matches when it is enabled, its `category` is either
    /// `NULL` (all categories) or equal to the alert's `category`, and its
    /// `min_severity` threshold is at or below the incoming alert severity.
    /// Matching `user_id` values are mapped to the stable principal UUIDs used
    /// on the wire.
    ///
    /// `min_severity` is the severity of the incoming alert, compared against
    /// each subscription's own threshold. No matching rows returns an empty
    /// list — the caller must send to nobody in that case.
    pub async fn find_subscribed_users(
        &self,
        entity_id: Uuid,
        category: &str,
        min_severity: AlertSeverity,
    ) -> Result<Vec<Uuid>> {
        let rows: Vec<(String,)> = sqlx::query_as(
            r#"
            SELECT DISTINCT user_id
            FROM user_alert_subscriptions
            WHERE entity_id = $1
              AND enabled = TRUE
              AND (category IS NULL OR lower(category) = lower($2))
              AND CASE lower(min_severity)
                    WHEN 'info' THEN 0
                    WHEN 'low' THEN 1
                    WHEN 'medium' THEN 2
                    WHEN 'high' THEN 3
                    WHEN 'critical' THEN 4
                    ELSE 2
                  END <= $3
            ORDER BY user_id
            "#,
        )
        .bind(entity_id)
        .bind(category)
        .bind(severity_rank(min_severity))
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|(user_id,)| user_principal_id(&user_id))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn severity_rank_is_monotonic() {
        assert!(severity_rank(AlertSeverity::Info) < severity_rank(AlertSeverity::Low));
        assert!(severity_rank(AlertSeverity::Low) < severity_rank(AlertSeverity::Medium));
        assert!(severity_rank(AlertSeverity::Medium) < severity_rank(AlertSeverity::High));
        assert!(severity_rank(AlertSeverity::High) < severity_rank(AlertSeverity::Critical));
    }
}
