//! Store methods for per-user alert subscriptions.
//!
//! Backs `user_alert_subscriptions` (migration 048). Each row opts one user
//! into entity-scoped alerts above a severity floor; `category` narrows the
//! subscription to one alert category, and is `NULL` for "every category".

use super::*;

use apex_core::alert_config::{user_principal_id, AlertSeverity};

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

impl PgStore {
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
