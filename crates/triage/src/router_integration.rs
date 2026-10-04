//! Router Integration — connects triage decisions to alert routing and
//! real-time notification systems.
//!
//! This module defines interfaces that the API layer implements to wire
//! triage engine output into [`AlertRouter`] and [`SseManager`]. Keeping
//! these as traits avoids a circular dependency on `apex-api`.

use async_trait::async_trait;
use uuid::Uuid;

use apex_core::triage::{TriageItemType, TriageThresholds};

// ─── Alert Dispatcher Trait ───────────────────────────────────────────────────

/// Payload for [`AlertDispatcher::dispatch_triage_alert`].
///
/// Bundles the alert fields so the dispatch API stays within the clippy
/// argument-count limit.
#[derive(Debug)]
pub struct TriageAlertRequest<'a> {
    pub item_type: &'a TriageItemType,
    pub source_id: &'a str,
    pub title: &'a str,
    pub description: &'a str,
    pub composite_score: f64,
    pub entity_id: Option<Uuid>,
    pub entity_name: Option<&'a str>,
}

/// A dispatch target for triage-generated alerts.
///
/// The API crate provides a real implementation that routes through
/// [`AlertRouter`](apex_api::alert_router::AlertRouter) and
/// [`SseManager`](apex_api::sse::SseManager).
#[async_trait]
pub trait AlertDispatcher: Send + Sync {
    /// Dispatch a high-priority triage item to the alert system.
    ///
    /// Returns the list of user IDs who were notified.
    async fn dispatch_triage_alert(&self, request: TriageAlertRequest<'_>) -> Vec<Uuid>;

    /// Send a triage queue update notification.
    async fn notify_queue_update(&self, stats_json: &str);

    /// Send a notification when a specific triage item's status changes.
    async fn notify_status_change(&self, queue_item_id: Uuid, new_status: &str, title: &str);
}

/// A fallback dispatcher that logs alerts and writes them to the activity_feed table.
///
/// Used when the real [`AlertRouter`] / [`SseManager`] are not available
/// (e.g. during development or in a Worker-only deployment without SSE).
pub struct LoggingAlertDispatcher {
    db_pool: Option<sqlx::PgPool>,
    thresholds: TriageThresholds,
}

/// Truncate a description to at most `max_bytes` bytes without splitting a
/// UTF-8 character (descriptions come from crawled/LLM text).
fn truncate_on_char_boundary(value: &str, max_bytes: usize) -> &str {
    if value.len() <= max_bytes {
        return value;
    }
    let mut end = max_bytes;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

impl LoggingAlertDispatcher {
    /// Create a dispatcher that logs to console only, with default thresholds.
    pub fn console_only() -> Self {
        Self {
            db_pool: None,
            thresholds: TriageThresholds::default(),
        }
    }

    /// Create a dispatcher that also writes alert events to `activity_feed`,
    /// with default thresholds.
    pub fn with_db(pool: sqlx::PgPool) -> Self {
        Self {
            db_pool: Some(pool),
            thresholds: TriageThresholds::default(),
        }
    }

    /// Create a database-backed dispatcher that maps severities with the given
    /// configured thresholds instead of the defaults.
    pub fn with_db_and_thresholds(pool: sqlx::PgPool, thresholds: TriageThresholds) -> Self {
        Self {
            db_pool: Some(pool),
            thresholds,
        }
    }
}

/// Insert one triage activity-feed row, returning the SQL error.
///
/// The caller decides how to surface the failure; discarding it is what made a
/// failed activity write invisible. `action_type` is bound explicitly so a
/// failure can be attributed to the event that was lost.
async fn insert_activity_feed(
    pool: &sqlx::PgPool,
    action_type: &str,
    entity_type: &str,
    entity_id: &str,
    entity_name: &str,
    details: &serde_json::Value,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO activity_feed (actor_id, actor_name, action_type, entity_type, entity_id, entity_name, details, created_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, NOW())",
    )
    .bind(Uuid::nil())
    .bind("ApexIntel Triage Engine")
    .bind(action_type)
    .bind(entity_type)
    .bind(entity_id)
    .bind(entity_name)
    .bind(details)
    .execute(pool)
    .await
    .map(|_| ())
}

#[async_trait]
impl AlertDispatcher for LoggingAlertDispatcher {
    async fn dispatch_triage_alert(&self, request: TriageAlertRequest<'_>) -> Vec<Uuid> {
        let TriageAlertRequest {
            item_type,
            source_id,
            title,
            description,
            composite_score,
            entity_id: _,
            entity_name,
        } = request;

        let severity = triage_score_to_alert_severity_with(composite_score, &self.thresholds);

        let item_type_label = format!("{:?}", item_type);
        tracing::warn!(
            target = "triage::alert",
            %item_type_label, %source_id, %title, composite_score, %severity,
            entity_name = entity_name.unwrap_or("none"),
            "Triage alert dispatched"
        );

        if let Some(ref pool) = self.db_pool {
            let details = serde_json::json!({
                "item_type": item_type_label,
                "source_id": source_id,
                "composite_score": composite_score,
                "severity": severity,
                "entity_name": entity_name,
                "description": truncate_on_char_boundary(description, 500),
            });
            if let Err(error) = insert_activity_feed(
                pool,
                "triage_alert",
                "triage_item",
                source_id,
                entity_name.unwrap_or("unknown"),
                &details,
            )
            .await
            {
                tracing::error!(
                    action_type = "triage_alert",
                    error = %error,
                    "failed to insert triage activity feed event"
                );
            }
        }
        Vec::new() // No SSE subscriptions to return from the fallback
    }

    async fn notify_queue_update(&self, stats_json: &str) {
        tracing::info!(
            target = "triage::queue",
            stats = %stats_json,
            "Triage queue stats updated"
        );
        if let Some(ref pool) = self.db_pool {
            let details = serde_json::json!({ "stats": stats_json });
            if let Err(error) = insert_activity_feed(
                pool,
                "triage_queue_update",
                "triage_queue",
                &Uuid::nil().to_string(),
                "system",
                &details,
            )
            .await
            {
                tracing::error!(
                    action_type = "triage_queue_update",
                    error = %error,
                    "failed to insert triage activity feed event"
                );
            }
        }
    }

    async fn notify_status_change(&self, queue_item_id: Uuid, new_status: &str, title: &str) {
        tracing::info!(
            target = "triage::status",
            %queue_item_id, %new_status, %title,
            "Triage item status changed"
        );
        if let Some(ref pool) = self.db_pool {
            let details = serde_json::json!({ "new_status": new_status, "title": title });
            if let Err(error) = insert_activity_feed(
                pool,
                "triage_status_change",
                "triage_item",
                &queue_item_id.to_string(),
                title,
                &details,
            )
            .await
            {
                tracing::error!(
                    action_type = "triage_status_change",
                    error = %error,
                    "failed to insert triage activity feed event"
                );
            }
        }
    }
}

// ─── RouterIntegration ────────────────────────────────────────────────────────

/// Routes triage decisions to external alerting and notification systems.
///
/// Uses the [`AlertDispatcher`] trait so the API layer can inject the real
/// [`AlertRouter`] / [`SseManager`] implementations without circular deps.
pub struct RouterIntegration {
    dispatcher: Box<dyn AlertDispatcher>,
}

impl RouterIntegration {
    /// Create a new [`RouterIntegration`] with the given dispatcher.
    pub fn new(dispatcher: Box<dyn AlertDispatcher>) -> Self {
        Self { dispatcher }
    }

    /// Dispatch a high-priority triage item to the alert system.
    ///
    /// Returns the list of user IDs who were notified.
    pub async fn dispatch_triage_alert(&self, request: TriageAlertRequest<'_>) -> Vec<Uuid> {
        self.dispatcher.dispatch_triage_alert(request).await
    }

    /// Send a general triage queue update via the dispatcher.
    pub async fn notify_queue_update(&self, stats: serde_json::Value) {
        self.dispatcher
            .notify_queue_update(&stats.to_string())
            .await;
    }

    /// Send a notification when a specific triage item's status changes.
    pub async fn notify_status_change(&self, queue_item_id: Uuid, new_status: &str, title: &str) {
        self.dispatcher
            .notify_status_change(queue_item_id, new_status, title)
            .await;
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// Map a triage score to an alert severity string using configured thresholds.
///
/// Delegates to [`apex_core::triage::score_to_band`] — the same configured
/// mapping the triage queue uses — so alert severity can never drift from the
/// queue's band labels. The queue's `info` band is folded to `low`: alert
/// severities have no `info` level and existing callers/tests relied on the
/// old floor of `low`.
pub fn triage_score_to_alert_severity_with(score: f64, thresholds: &TriageThresholds) -> String {
    match apex_core::triage::score_to_band(score, thresholds) {
        "info" => "low".to_string(),
        band => band.to_string(),
    }
}

/// Map a triage score to an alert severity string with default thresholds.
///
/// Backwards-compatible helper for existing callers; use
/// [`triage_score_to_alert_severity_with`] when configured thresholds are
/// available.
pub fn triage_score_to_alert_severity(score: f64) -> String {
    triage_score_to_alert_severity_with(score, &TriageThresholds::default())
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    struct TestDispatcher {
        alerted: Arc<AtomicBool>,
    }

    #[async_trait]
    impl AlertDispatcher for TestDispatcher {
        async fn dispatch_triage_alert(&self, _request: TriageAlertRequest<'_>) -> Vec<Uuid> {
            self.alerted.store(true, Ordering::SeqCst);
            vec![Uuid::nil()]
        }

        async fn notify_queue_update(&self, _stats_json: &str) {}

        async fn notify_status_change(
            &self,
            _queue_item_id: Uuid,
            _new_status: &str,
            _title: &str,
        ) {
        }
    }

    #[tokio::test]
    async fn test_dispatch_triage_alert_calls_dispatcher() {
        let alerted = Arc::new(AtomicBool::new(false));
        let dispatcher = TestDispatcher {
            alerted: alerted.clone(),
        };
        let integration = RouterIntegration::new(Box::new(dispatcher));

        let user_ids = integration
            .dispatch_triage_alert(TriageAlertRequest {
                item_type: &TriageItemType::Insight,
                source_id: "insight-1",
                title: "Critical finding",
                description: "Something important",
                composite_score: 0.95,
                entity_id: None,
                entity_name: Some("Acme Corp"),
            })
            .await;

        assert!(alerted.load(Ordering::SeqCst));
        assert_eq!(user_ids.len(), 1);
    }

    #[tokio::test]
    async fn test_noop_dispatcher_returns_empty() {
        let integration = RouterIntegration::new(Box::new(LoggingAlertDispatcher::console_only()));

        let user_ids = integration
            .dispatch_triage_alert(TriageAlertRequest {
                item_type: &TriageItemType::Warning,
                source_id: "warn-1",
                title: "Test",
                description: "Testing noop",
                composite_score: 0.5,
                entity_id: None,
                entity_name: None,
            })
            .await;

        assert!(user_ids.is_empty());
    }

    #[test]
    fn test_triage_score_to_alert_severity() {
        assert_eq!(triage_score_to_alert_severity(0.90), "critical");
        assert_eq!(triage_score_to_alert_severity(0.70), "high");
        assert_eq!(triage_score_to_alert_severity(0.50), "medium");
        assert_eq!(triage_score_to_alert_severity(0.30), "low");
        assert_eq!(triage_score_to_alert_severity(0.10), "low");
    }

    #[test]
    fn test_custom_thresholds_change_mapped_severity_at_boundary() {
        // Regression: severity was hard-coded a second time in the dispatcher,
        // so raising `critical` to 0.90 still mapped 0.85 to "critical".
        let custom = TriageThresholds {
            critical: 0.90,
            high: 0.60,
            medium: 0.40,
            low: 0.20,
        };
        assert_eq!(
            triage_score_to_alert_severity_with(0.85, &custom),
            "high",
            "0.85 is below a configured critical threshold of 0.90"
        );
        assert_eq!(
            triage_score_to_alert_severity_with(0.90, &custom),
            "critical"
        );
        // The default helper keeps the historical default mapping.
        assert_eq!(triage_score_to_alert_severity(0.85), "critical");

        // Raising `medium` to 0.55 moves 0.50 from "medium" to "low".
        let raised_medium = TriageThresholds {
            medium: 0.55,
            ..TriageThresholds::default()
        };
        assert_eq!(
            triage_score_to_alert_severity_with(0.50, &raised_medium),
            "low"
        );
        assert_eq!(
            triage_score_to_alert_severity_with(0.55, &raised_medium),
            "medium"
        );
        assert_eq!(triage_score_to_alert_severity(0.50), "medium");
    }

    #[test]
    fn test_no_activity_feed_write_discards_its_error() {
        // Regression: all three inserts used `let _ = sqlx::query(...)`, which
        // made a failed activity write invisible.
        let source = include_str!("router_integration.rs");
        let production = source.split("#[cfg(test)]").next().unwrap_or(source);
        assert!(
            !production.contains("let _ = sqlx::query"),
            "activity-feed writes must not discard their SQL error"
        );
        let observable_calls = production
            .matches("if let Err(error) = insert_activity_feed(")
            .count();
        assert!(
            observable_calls >= 3,
            "all three activity-feed inserts must route through the error-observable helper, found {observable_calls}"
        );
    }

    #[tokio::test]
    async fn test_activity_insert_failure_is_returned_not_discarded() {
        // Regression: the three activity-feed inserts used `let _ =`, so a
        // failed write was invisible. The insert helper must surface the SQL
        // error to its caller instead.
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(std::time::Duration::from_millis(500))
            .connect_lazy("postgres://apex:apex@127.0.0.1:1/apex_unreachable")
            .expect("lazy pool construction must not require a live database");

        let result = insert_activity_feed(
            &pool,
            "triage_status_change",
            "triage_item",
            &Uuid::new_v4().to_string(),
            "Unreachable database",
            &serde_json::json!({ "new_status": "resolved" }),
        )
        .await;

        assert!(
            result.is_err(),
            "a failed activity-feed write must return a SQL error to its caller"
        );
    }

    #[derive(Clone, Default)]
    struct RecordingDispatcher {
        queue_updates: Arc<std::sync::Mutex<Vec<String>>>,
        status_changes: Arc<std::sync::Mutex<Vec<(Uuid, String, String)>>>,
    }

    fn lock_or_recover<T>(mutex: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
        match mutex.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    #[async_trait]
    impl AlertDispatcher for RecordingDispatcher {
        async fn dispatch_triage_alert(&self, _request: TriageAlertRequest<'_>) -> Vec<Uuid> {
            Vec::new()
        }

        async fn notify_queue_update(&self, stats_json: &str) {
            lock_or_recover(&self.queue_updates).push(stats_json.to_string());
        }

        async fn notify_status_change(&self, queue_item_id: Uuid, new_status: &str, title: &str) {
            lock_or_recover(&self.status_changes).push((
                queue_item_id,
                new_status.to_string(),
                title.to_string(),
            ));
        }
    }

    #[tokio::test]
    async fn test_notify_queue_update_reaches_dispatcher() {
        let dispatcher = RecordingDispatcher::default();
        let queue_updates = dispatcher.queue_updates.clone();
        let integration = RouterIntegration::new(Box::new(dispatcher));

        integration
            .notify_queue_update(serde_json::json!({ "total": 4 }))
            .await;

        let recorded = lock_or_recover(&queue_updates).clone();
        assert_eq!(recorded.len(), 1);
        assert!(recorded[0].contains("\"total\":4"));
    }

    #[tokio::test]
    async fn test_notify_status_change_reaches_dispatcher() {
        let dispatcher = RecordingDispatcher::default();
        let status_changes = dispatcher.status_changes.clone();
        let integration = RouterIntegration::new(Box::new(dispatcher));

        let item_id = Uuid::new_v4();
        integration
            .notify_status_change(item_id, "acknowledged", "Suspicious shipment")
            .await;

        assert_eq!(
            lock_or_recover(&status_changes).clone(),
            vec![(
                item_id,
                "acknowledged".to_string(),
                "Suspicious shipment".to_string()
            )]
        );
    }

    #[tokio::test]
    async fn test_dispatch_alert_multibyte_description_does_not_panic() {
        let integration = RouterIntegration::new(Box::new(LoggingAlertDispatcher::console_only()));
        // 499 ASCII bytes followed by a 2-byte character straddling byte 500.
        let description = format!("{}é tail", "a".repeat(499));
        let user_ids = integration
            .dispatch_triage_alert(TriageAlertRequest {
                item_type: &TriageItemType::Warning,
                source_id: "warn-multibyte",
                title: "Multibyte",
                description: &description,
                composite_score: 0.9,
                entity_id: None,
                entity_name: None,
            })
            .await;
        assert!(user_ids.is_empty());
    }

    #[test]
    fn test_truncate_on_char_boundary_never_splits_multibyte() {
        let value = format!("{}é tail", "a".repeat(499));
        // Byte 500 falls inside the 2-byte 'é'; truncation must back off to 499.
        let truncated = truncate_on_char_boundary(&value, 500);
        assert_eq!(truncated, "a".repeat(499));

        // Short values are returned unchanged.
        assert_eq!(truncate_on_char_boundary("short", 500), "short");
        // A boundary capped by a leading multibyte char degrades to empty.
        assert_eq!(truncate_on_char_boundary("é", 1), "");
    }
}
