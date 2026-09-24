//! Alert routing logic — determines which users should receive which alerts.
//!
//! The [`AlertRouter`] checks:
//! 1. The event's [`AlertAudience`]: [`AlertAudience::Broadcast`] passes through
//!    untouched; [`AlertAudience::Users`] is filtered by per-user preferences.
//!    An empty `Users` list is resolved against `user_alert_subscriptions`
//!    when the alert targets an entity, so targeted alerts reach real
//!    subscribers instead of everyone (or no one).
//! 2. Per-user notification preferences (`user_preferences.preferences` JSONB,
//!    both the web-settings and JSON-API shapes).
//! 3. Entity subscriptions (`user_alert_subscriptions`).
//! 4. Per-entity overrides (`EntityAlertConfig`) and global defaults
//!    (`GlobalAlertDefaults`).
//!
//! This module re-exports the shared [`AlertEvent`] type used by both the
//! NATS publisher (worker) and the SSE consumer (API server).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, RwLock};
use uuid::Uuid;

use apex_core::alert_config::{AlertAudience, AlertSeverity};

// ─────────────────────────────────────────────────────────────────────────────
// Re-export AlertEvent for use by both worker publisher and API consumer
// ─────────────────────────────────────────────────────────────────────────────

/// The type of alert event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertEventType {
    NewInsight,
    NewWarning,
    RecipeMatch,
    CompetitorChange,
    SupplyChainRisk,
    SystemAlert,
}

impl AlertEventType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NewInsight => "new_insight",
            Self::NewWarning => "new_warning",
            Self::RecipeMatch => "recipe_match",
            Self::CompetitorChange => "competitor_change",
            Self::SupplyChainRisk => "supply_chain_risk",
            Self::SystemAlert => "system_alert",
        }
    }
}

impl fmt::Display for AlertEventType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// An alert event that travels through the pipeline: worker → NATS → API → SSE.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertEvent {
    pub id: Uuid,
    pub event_type: AlertEventType,
    pub severity: apex_core::alert_config::AlertSeverity,
    pub title: String,
    pub description: String,
    pub entity_id: Option<Uuid>,
    pub entity_name: Option<String>,
    /// Who this alert is addressed to. `Users(vec![])` addresses nobody and
    /// only a deliberate `Broadcast` reaches every connected user.
    pub audience: AlertAudience,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

impl AlertEvent {
    /// Map the event type to a string suitable for `EntityAlertConfig` lookups.
    pub fn alert_category(&self) -> &'static str {
        match self.event_type {
            AlertEventType::NewInsight => "insight",
            AlertEventType::NewWarning => "warning",
            AlertEventType::RecipeMatch => "recipe_match",
            AlertEventType::CompetitorChange => "competitor_change",
            AlertEventType::SupplyChainRisk => "supply_chain_risk",
            AlertEventType::SystemAlert => "system_alert",
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// PrincipalDirectory
// ─────────────────────────────────────────────────────────────────────────────

/// In-process directory of authenticated principals with a live real-time
/// connection, keyed by the UUID derived from their user name
/// (`apex_core::alert_config::user_principal_id`).
///
/// `user_preferences` is keyed by user name while alerts address principals by
/// UUID, and UUIDv5 is one-way. The directory bridges the two for connected
/// users so [`AlertRouter::should_notify_user`] can consult their preferences.
#[derive(Debug, Default)]
pub struct PrincipalDirectory {
    usernames: RwLock<HashMap<Uuid, String>>,
}

impl PrincipalDirectory {
    /// Create an empty directory.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record (or refresh) the user name behind a principal ID.
    pub fn record(&self, principal_id: Uuid, username: &str) {
        if let Ok(mut usernames) = self.usernames.write() {
            usernames.insert(principal_id, username.to_string());
        }
    }

    /// Forget a principal once its last connection has closed.
    pub fn forget(&self, principal_id: Uuid) {
        if let Ok(mut usernames) = self.usernames.write() {
            usernames.remove(&principal_id);
        }
    }

    /// Resolve the user name for a principal, if it has connected.
    pub fn username_for(&self, principal_id: Uuid) -> Option<String> {
        self.usernames
            .read()
            .ok()
            .and_then(|usernames| usernames.get(&principal_id).cloned())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// AlertRouter
// ─────────────────────────────────────────────────────────────────────────────

/// Routes alerts to the appropriate users based on configuration.
///
/// Uses the database to resolve subscriptions and preferences and to look up
/// [`EntityAlertConfig`](apex_core::alert_config::EntityAlertConfig) and
/// [`GlobalAlertDefaults`](apex_core::alert_config::GlobalAlertDefaults).
pub struct AlertRouter {
    db: Arc<apex_store::postgres::PgStore>,
    principals: Arc<PrincipalDirectory>,
}

impl AlertRouter {
    /// Create a new alert router backed by the database and the live
    /// principal directory.
    pub fn new(
        db: Arc<apex_store::postgres::PgStore>,
        principals: Arc<PrincipalDirectory>,
    ) -> Self {
        Self { db, principals }
    }

    /// Route an alert to the right audience.
    ///
    /// # Routing logic
    /// - `Broadcast` is a deliberate system-wide alert and passes through.
    /// - `Users([...])` with explicit IDs keeps only the users whose
    ///   preferences and configs allow the alert. If no user remains the
    ///   result addresses nobody.
    /// - `Users([])` resolves the entity's real subscribers from
    ///   `user_alert_subscriptions`; with no entity or no subscribers the
    ///   result is `Users([])` — nobody, never an implicit broadcast.
    ///
    /// Returns an error only when the subscription lookup itself fails, so the
    /// NATS consumer can retry the message instead of dropping it.
    pub async fn route_alert(&self, alert: &AlertEvent) -> anyhow::Result<AlertAudience> {
        match &alert.audience {
            // Deliberate system-wide alerts reach every connected user.
            AlertAudience::Broadcast => Ok(AlertAudience::Broadcast),

            // Explicit addressees: filter by per-user preferences/configs.
            AlertAudience::Users(explicit) if !explicit.is_empty() => {
                let mut targets = Vec::with_capacity(explicit.len());
                for user_id in explicit {
                    if self.should_notify_user(*user_id, alert).await {
                        targets.push(*user_id);
                    }
                }
                Ok(AlertAudience::Users(targets))
            }

            // No explicit addressees: resolve the entity's real subscribers.
            AlertAudience::Users(_) => {
                let subscribers = self.find_subscribed_users(alert).await?;
                let mut targets = Vec::with_capacity(subscribers.len());
                for user_id in subscribers {
                    if self.should_notify_user(user_id, alert).await {
                        targets.push(user_id);
                    }
                }
                Ok(AlertAudience::Users(targets))
            }
        }
    }

    /// Find the real subscribers for an entity-targeted alert.
    ///
    /// Returns principal IDs whose `user_alert_subscriptions` row matches the
    /// alert's entity, category and severity. With no entity, or no matching
    /// rows, returns an empty list (nobody).
    pub async fn find_subscribed_users(&self, alert: &AlertEvent) -> anyhow::Result<Vec<Uuid>> {
        let Some(entity_id) = alert.entity_id else {
            return Ok(Vec::new());
        };

        self.db
            .find_subscribed_users(entity_id, alert.alert_category(), alert.severity)
            .await
    }

    /// Check whether a specific user should receive this alert.
    ///
    /// Consults the user's notification preferences from `user_preferences`
    /// first (when the principal has an active connection), then the entity's
    /// [`EntityAlertConfig`](apex_core::alert_config::EntityAlertConfig), then
    /// the global defaults. Database errors fail open so a transient failure
    /// never silently suppresses an alert.
    pub async fn should_notify_user(&self, user_id: Uuid, alert: &AlertEvent) -> bool {
        if let Some(username) = self.principals.username_for(user_id) {
            match self.db.get_user_preferences_record(&username).await {
                Ok(Some(record)) => {
                    if !user_preferences_allow_alert(&record.preferences, alert) {
                        return false;
                    }
                }
                Ok(None) => {
                    // The user has no preferences row — fall through to the
                    // entity/global configuration.
                }
                Err(e) => {
                    tracing::warn!(
                        user = %username,
                        error = %e,
                        "Failed to fetch user preferences, falling back to alert configs"
                    );
                }
            }
        }

        self.entity_or_global_allows(alert).await
    }

    /// Check the entity alert config (if the alert has an entity and a config
    /// exists) and fall back to the global defaults.
    async fn entity_or_global_allows(&self, alert: &AlertEvent) -> bool {
        use apex_core::alert_config::AlertChannel;

        if let Some(entity_id) = alert.entity_id {
            let entity_id_str = entity_id.to_string();
            match self.db.get_entity_alert_config(&entity_id_str).await {
                Ok(Some(cfg)) => {
                    // Check that InApp channel is enabled
                    if !cfg.enabled_channels.contains(&AlertChannel::InApp) {
                        return false;
                    }
                    // Check if the alert is suppressed
                    return !cfg.is_alert_suppressed(alert.alert_category(), alert.severity);
                }
                Ok(None) => {
                    // No per-entity config — check global defaults
                    return self.check_global_defaults(alert).await;
                }
                Err(e) => {
                    tracing::warn!(
                        entity_id = %entity_id_str,
                        error = %e,
                        "Failed to fetch entity alert config, falling back to defaults"
                    );
                    return self.check_global_defaults(alert).await;
                }
            }
        }

        // No entity — check global defaults
        self.check_global_defaults(alert).await
    }

    /// Check the global alert defaults.
    async fn check_global_defaults(&self, alert: &AlertEvent) -> bool {
        use apex_core::alert_config::AlertChannel;

        match self.db.get_global_alert_defaults().await {
            Ok(Some(defaults)) => {
                // Check that InApp channel is enabled globally
                if !defaults.enabled_channels.contains(&AlertChannel::InApp) {
                    return false;
                }
                // Check severity threshold
                alert.severity >= defaults.min_severity
            }
            Ok(None) => {
                // No global config — allow by default if severity >= Medium
                alert.severity >= AlertSeverity::Medium
            }
            Err(e) => {
                tracing::warn!(error = %e, "Failed to fetch global alert defaults");
                // Allow through on error (fail open)
                true
            }
        }
    }
}

/// Pure per-user preference gate.
///
/// Recognises both preference shapes persisted in
/// `user_preferences.preferences`:
/// - the web settings page (`settings_page.minimum_severity`,
///   `settings_page.critical_only_enabled`), and
/// - the JSON API (`notifications.browser_push`, `notifications.min_severity`).
///
/// A user with no relevant keys allows the alert; the caller still applies the
/// entity/global thresholds.
pub(crate) fn user_preferences_allow_alert(
    preferences: &serde_json::Value,
    alert: &AlertEvent,
) -> bool {
    if let Some(notifications) = preferences.get("notifications") {
        if notifications.get("browser_push").and_then(|v| v.as_bool()) == Some(false) {
            return false;
        }
        if let Some(min) = notifications.get("min_severity").and_then(|v| v.as_str()) {
            if alert.severity < AlertSeverity::from_str(min) {
                return false;
            }
        }
    }

    if let Some(page) = preferences.get("settings_page") {
        if page.get("critical_only_enabled").and_then(|v| v.as_bool()) == Some(true)
            && alert.severity < AlertSeverity::Critical
        {
            return false;
        }
        if let Some(min) = page.get("minimum_severity").and_then(|v| v.as_str()) {
            if alert.severity < AlertSeverity::from_str(min) {
                return false;
            }
        }
    }

    true
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn test_alert(event_type: AlertEventType, severity: AlertSeverity) -> AlertEvent {
        AlertEvent {
            id: Uuid::new_v4(),
            event_type,
            severity,
            title: "Test".to_string(),
            description: "Test".to_string(),
            entity_id: None,
            entity_name: None,
            audience: AlertAudience::Users(vec![]),
            metadata: serde_json::json!({}),
            created_at: Utc::now(),
        }
    }

    #[test]
    fn alert_event_type_as_str() {
        assert_eq!(AlertEventType::NewInsight.as_str(), "new_insight");
        assert_eq!(AlertEventType::NewWarning.as_str(), "new_warning");
        assert_eq!(AlertEventType::RecipeMatch.as_str(), "recipe_match");
        assert_eq!(
            AlertEventType::CompetitorChange.as_str(),
            "competitor_change"
        );
        assert_eq!(
            AlertEventType::SupplyChainRisk.as_str(),
            "supply_chain_risk"
        );
        assert_eq!(AlertEventType::SystemAlert.as_str(), "system_alert");
    }

    #[test]
    fn alert_category_mapping() {
        let alert = test_alert(AlertEventType::NewWarning, AlertSeverity::High);
        assert_eq!(alert.alert_category(), "warning");
    }

    #[test]
    fn alert_event_serde_roundtrip() {
        let event = AlertEvent {
            id: Uuid::new_v4(),
            event_type: AlertEventType::CompetitorChange,
            severity: AlertSeverity::Critical,
            title: "Competitor move".to_string(),
            description: "A competitor changed strategy".to_string(),
            entity_id: Some(Uuid::new_v4()),
            entity_name: Some("Rival Corp".to_string()),
            audience: AlertAudience::Users(vec![Uuid::new_v4()]),
            metadata: serde_json::json!({"change_type": "pivot"}),
            created_at: Utc::now(),
        };

        let json = serde_json::to_string(&event).unwrap();
        let deserialized: AlertEvent = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.id, event.id);
        assert_eq!(deserialized.event_type, event.event_type);
        assert_eq!(deserialized.audience, event.audience);
        assert_eq!(deserialized.metadata["change_type"], "pivot");
    }

    #[test]
    fn broadcast_audience_roundtrips_distinctly_from_empty_users() {
        let mut broadcast = test_alert(AlertEventType::SystemAlert, AlertSeverity::Info);
        broadcast.audience = AlertAudience::Broadcast;

        let json = serde_json::to_value(&broadcast).unwrap();
        assert_eq!(json["audience"], serde_json::json!({"kind": "broadcast"}));

        let empty = test_alert(AlertEventType::SystemAlert, AlertSeverity::Info);
        let empty_json = serde_json::to_value(&empty).unwrap();
        assert_eq!(
            empty_json["audience"],
            serde_json::json!({"kind": "users", "user_ids": []})
        );

        let back: AlertEvent = serde_json::from_value(json).unwrap();
        assert_eq!(back.audience, AlertAudience::Broadcast);
    }

    #[test]
    fn principal_directory_records_and_forgets_usernames() {
        let dir = PrincipalDirectory::new();
        let id = apex_core::alert_config::user_principal_id("alice");

        assert_eq!(dir.username_for(id), None);
        dir.record(id, "alice");
        assert_eq!(dir.username_for(id).as_deref(), Some("alice"));
        dir.forget(id);
        assert_eq!(dir.username_for(id), None);
    }

    #[test]
    fn user_preferences_block_browser_push_disabled() {
        let alert = test_alert(AlertEventType::NewWarning, AlertSeverity::Critical);
        let prefs = serde_json::json!({"notifications": {"browser_push": false}});
        assert!(!user_preferences_allow_alert(&prefs, &alert));
    }

    #[test]
    fn user_preferences_enforce_notification_min_severity() {
        let prefs = serde_json::json!({"notifications": {"min_severity": "high"}});
        assert!(!user_preferences_allow_alert(
            &prefs,
            &test_alert(AlertEventType::NewWarning, AlertSeverity::Medium),
        ));
        assert!(user_preferences_allow_alert(
            &prefs,
            &test_alert(AlertEventType::NewWarning, AlertSeverity::High),
        ));
    }

    #[test]
    fn user_preferences_enforce_settings_page_thresholds() {
        let page = serde_json::json!({
            "settings_page": {"minimum_severity": "medium", "critical_only_enabled": true}
        });
        // critical_only wins over the lower threshold
        assert!(!user_preferences_allow_alert(
            &page,
            &test_alert(AlertEventType::NewWarning, AlertSeverity::High),
        ));
        assert!(user_preferences_allow_alert(
            &page,
            &test_alert(AlertEventType::NewWarning, AlertSeverity::Critical),
        ));
    }

    #[test]
    fn user_preferences_allow_unknown_shape_and_missing_keys() {
        let alert = test_alert(AlertEventType::NewInsight, AlertSeverity::Low);
        assert!(user_preferences_allow_alert(&serde_json::json!({}), &alert));
        assert!(user_preferences_allow_alert(
            &serde_json::json!({"theme": "dark"}),
            &alert
        ));
        assert!(user_preferences_allow_alert(
            &serde_json::json!({"settings_page": {"minimum_severity": "low"}}),
            &alert
        ));
    }
}
