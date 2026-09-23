use super::*;

type WeeklyMemoRowTuple = (
    Uuid,
    String,
    chrono::NaiveDate,
    chrono::NaiveDate,
    String,
    serde_json::Value,
    serde_json::Value,
    serde_json::Value,
    DateTime<Utc>,
);

fn normalize_memo_window(limit: i64, offset: i64) -> (i64, i64) {
    (clamp_limit(limit), offset.max(0))
}

fn weekly_memo_from_parts(row: WeeklyMemoRowTuple) -> WeeklyMemo {
    let (
        id,
        title,
        week_start,
        week_end,
        executive_summary,
        sections_json,
        metrics_json,
        actions_json,
        generated_at,
    ) = row;
    let sections: Vec<WeeklyMemoSection> =
        serde_json::from_value(sections_json).unwrap_or_default();
    let key_metrics: WeeklyMemoKeyMetrics =
        serde_json::from_value(metrics_json).unwrap_or(WeeklyMemoKeyMetrics {
            warnings_total: 0,
            warnings_critical: 0,
            insights_generated: 0,
            companies_monitored: 0,
            pois_tracked: 0,
        });
    let action_items: Vec<WeeklyMemoActionItem> =
        serde_json::from_value(actions_json).unwrap_or_default();

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
}

impl PgStore {
    pub async fn get_weekly_memo_full(&self) -> Result<Option<WeeklyMemo>> {
        let row: Option<WeeklyMemoRowTuple> = sqlx::query_as(
            r#"SELECT id, title, week_start, week_end, executive_summary, sections, key_metrics, action_items, generated_at
               FROM weekly_memos
               ORDER BY week_start DESC
               LIMIT 1"#,
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(weekly_memo_from_parts))
    }

    pub async fn get_weekly_memo(&self) -> Result<WeeklySummaryStats> {
        self.get_weekly_summary_stats(Utc::now() - chrono::Duration::days(7))
            .await
    }

    pub async fn list_weekly_memos(
        &self,
        limit: i64,
        offset: i64,
    ) -> Result<(Vec<WeeklyMemo>, i64)> {
        let (limit, offset) = normalize_memo_window(limit, offset);
        let (total,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM weekly_memos")
            .fetch_one(&self.pool)
            .await
            .unwrap_or((0,));

        let rows: Vec<WeeklyMemoRowTuple> = sqlx::query_as(
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

        Ok((
            rows.into_iter().map(weekly_memo_from_parts).collect(),
            total,
        ))
    }

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
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::{normalize_memo_window, weekly_memo_from_parts};
    use chrono::{NaiveDate, Utc};
    use serde_json::json;
    use uuid::Uuid;

    #[test]
    fn test_normalize_memo_window_clamps_limit_and_offset() {
        assert_eq!(normalize_memo_window(0, -3), (1, 0));
        assert_eq!(normalize_memo_window(9999, 2), (500, 2));
    }

    #[test]
    fn test_weekly_memo_from_parts_defaults_invalid_json_shapes() {
        let memo = weekly_memo_from_parts((
            Uuid::new_v4(),
            "Weekly Memo".to_string(),
            NaiveDate::from_ymd_opt(2026, 3, 2).unwrap(),
            NaiveDate::from_ymd_opt(2026, 3, 8).unwrap(),
            "Summary".to_string(),
            json!({"bad": true}),
            json!({"bad": true}),
            json!({"bad": true}),
            Utc::now(),
        ));

        assert!(memo.sections.is_empty());
        assert_eq!(memo.key_metrics.warnings_total, 0);
        assert!(memo.action_items.is_empty());
    }
}
