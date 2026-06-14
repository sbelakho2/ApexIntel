//! Router Integration — connects triage decisions to alert routing and
//! real-time notification systems.
//!
//! This module defines interfaces that the API layer implements to wire
//! triage engine output into [`AlertRouter`] and [`SseManager`]. Keeping
//! these as traits avoids a circular dependency on `apex-api`.

use async_trait::async_trait;
use uuid::Uuid;

use apex_core::triage::TriageItemType;

// ─── Alert Dispatcher Trait ───────────────────────────────────────────────────

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
    async fn dispatch_triage_alert(
        &self,
        item_type: &TriageItemType,
        source_id: &str,
        title: &str,
        description: &str,
        composite_score: f64,
        entity_id: Option<Uuid>,
        entity_name: Option<&str>,
    ) -> Vec<Uuid>;

    /// Send a triage queue update notification.
    async fn notify_queue_update(&self, stats_json: &str);

    /// Send a notification when a specific triage item's status changes.
    async fn notify_status_change(
        &self,
        queue_item_id: Uuid,
        new_status: &str,
        title: &str,
    );
}

/// A no-op dispatcher that silently discards all alerts.
pub struct NoopAlertDispatcher;

#[async_trait]
impl AlertDispatcher for NoopAlertDispatcher {
    async fn dispatch_triage_alert(
        &self,
        _item_type: &TriageItemType,
        _source_id: &str,
        _title: &str,
        _description: &str,
        _composite_score: f64,
        _entity_id: Option<Uuid>,
        _entity_name: Option<&str>,
    ) -> Vec<Uuid> {
        Vec::new()
    }

    async fn notify_queue_update(&self, _stats_json: &str) {}

    async fn notify_status_change(&self, _queue_item_id: Uuid, _new_status: &str, _title: &str) {}
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
    pub async fn dispatch_triage_alert(
        &self,
        item_type: &TriageItemType,
        source_id: &str,
        title: &str,
        description: &str,
        composite_score: f64,
        entity_id: Option<Uuid>,
        entity_name: Option<&str>,
    ) -> Vec<Uuid> {
        self.dispatcher
            .dispatch_triage_alert(
                item_type,
                source_id,
                title,
                description,
                composite_score,
                entity_id,
                entity_name,
            )
            .await
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

/// Map a triage score to an alert severity string.
pub fn triage_score_to_alert_severity(score: f64) -> String {
    if score >= 0.80 {
        "critical".to_string()
    } else if score >= 0.60 {
        "high".to_string()
    } else if score >= 0.40 {
        "medium".to_string()
    } else {
        "low".to_string()
    }
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
        async fn dispatch_triage_alert(
            &self,
            _item_type: &TriageItemType,
            _source_id: &str,
            _title: &str,
            _description: &str,
            _composite_score: f64,
            _entity_id: Option<Uuid>,
            _entity_name: Option<&str>,
        ) -> Vec<Uuid> {
            self.alerted.store(true, Ordering::SeqCst);
            vec![Uuid::nil()]
        }

        async fn notify_queue_update(&self, _stats_json: &str) {}

        async fn notify_status_change(&self, _queue_item_id: Uuid, _new_status: &str, _title: &str) {}
    }

    #[tokio::test]
    async fn test_dispatch_triage_alert_calls_dispatcher() {
        let alerted = Arc::new(AtomicBool::new(false));
        let dispatcher = TestDispatcher {
            alerted: alerted.clone(),
        };
        let integration = RouterIntegration::new(Box::new(dispatcher));

        let user_ids = integration
            .dispatch_triage_alert(
                &TriageItemType::Insight,
                "insight-1",
                "Critical finding",
                "Something important",
                0.95,
                None,
                Some("Acme Corp"),
            )
            .await;

        assert!(alerted.load(Ordering::SeqCst));
        assert_eq!(user_ids.len(), 1);
    }

    #[tokio::test]
    async fn test_noop_dispatcher_returns_empty() {
        let integration = RouterIntegration::new(Box::new(NoopAlertDispatcher));

        let user_ids = integration
            .dispatch_triage_alert(
                &TriageItemType::Warning,
                "warn-1",
                "Test",
                "Testing noop",
                0.5,
                None,
                None,
            )
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
}
