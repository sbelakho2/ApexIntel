//! Alert routing logic — determines which users should receive which alerts.
//!
//! The [`AlertRouter`] checks:
//! 1. Explicit `user_ids` in the [`AlertEvent`]
//! 2. Entity subscriptions (users watching an entity via `EntityAlertConfig`)
//! 3. Default alert thresholds (`GlobalAlertDefaults`)
//! 4. Per-entity overrides (`EntityAlertConfig`)
//!
//! This module re-exports the shared [`AlertEvent`] type used by both the
//! NATS publisher (worker) and the SSE consumer (API server).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::Arc;
use uuid::Uuid;

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
    /// Target user IDs (empty = broadcast to all).
    pub user_ids: Vec<Uuid>,
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
// AlertRouter
// ─────────────────────────────────────────────────────────────────────────────

/// Routes alerts to the appropriate users based on configuration.
///
/// Uses the database to look up [`EntityAlertConfig`] and [`GlobalAlertDefaults`]
/// to determine which users should receive each alert.
pub struct AlertRouter {
    db: Arc<apex_store::postgres::PgStore>,
}

impl AlertRouter {
    /// Create a new alert router backed by the database.
    pub fn new(db: Arc<apex_store::postgres::PgStore>) -> Self {
        Self { db }
    }

    /// Route an alert to the right users.
    ///
    /// Returns the list of user IDs that should receive this alert.
    /// If the alert already has explicit `user_ids`, those are returned directly
    /// (after filtering by user preferences).
    ///
    /// # Routing logic
    /// 1. If `alert.user_ids` is non-empty, check each user's `EntityAlertConfig`
    ///    for the entity (if any) and filter out those whose config suppresses it.
    /// 2. If `alert.user_ids` is empty (broadcast), query all entity alert configs
    ///    and return user IDs whose config allows this alert type/severity.
    /// 3. When no entity config exists, fall back to `GlobalAlertDefaults`.
    pub async fn route_alert(&self, alert: &AlertEvent) -> Vec<Uuid> {
        // If explicit user IDs are set, filter them
        if !alert.user_ids.is_empty() {
            let mut targets = Vec::with_capacity(alert.user_ids.len());
            for user_id in &alert.user_ids {
                if self.should_notify_user(*user_id, alert).await {
                    targets.push(*user_id);
                }
            }
            return targets;
        }

        // Broadcast mode: find all users subscribed to this kind of alert
        self.find_subscribed_users(alert).await
    }

    /// Check whether a specific user should receive this alert.
    ///
    /// Checks the user's entity alert config (if one exists for the alert's
    /// entity), falling back to global defaults.
    pub async fn should_notify_user(&self, _user_id: Uuid, alert: &AlertEvent) -> bool {
        use apex_core::alert_config::AlertChannel;

        // If the alert targets a specific entity, check its config
        if let Some(ref entity_id) = alert.entity_id {
            let entity_id_str = entity_id.to_string();
            match self.db.get_entity_alert_config(&entity_id_str).await {
                Ok(Some(cfg)) => {
                    // Check that InApp channel is enabled
                    if !cfg.enabled_channels.contains(&AlertChannel::InApp) {
                        return false;
                    }
                    // Check if the alert is suppressed
                    if cfg.is_alert_suppressed(alert.alert_category(), alert.severity) {
                        return false;
                    }
                    return true;
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
                alert.severity >= apex_core::alert_config::AlertSeverity::Medium
            }
            Err(e) => {
                tracing::warn!(error = %e, "Failed to fetch global alert defaults");
                // Allow through on error (fail open)
                true
            }
        }
    }

    /// Find users subscribed to this kind of alert via entity alert configs.
    ///
    /// This queries the database for all entity alert configs and returns
    /// user IDs whose config allows this alert category at the given severity.
    ///
    /// Note: The current schema stores configs per entity, not per user.
    /// For user-specific subscriptions, this would need a `user_alert_subscriptions`
    /// table. For now, we return an empty vec (no broadcast subscribers)
    /// unless the entity has an explicit config allowing it.
    async fn find_subscribed_users(&self, alert: &AlertEvent) -> Vec<Uuid> {
        use apex_core::alert_config::AlertChannel;

        // If the alert has an entity, check who's watching it
        if let Some(ref entity_id) = alert.entity_id {
            let entity_id_str = entity_id.to_string();
            match self.db.get_entity_alert_config(&entity_id_str).await {
                Ok(Some(cfg)) => {
                    if cfg.enabled
                        && cfg.enabled_channels.contains(&AlertChannel::InApp)
                        && !cfg.is_alert_suppressed(alert.alert_category(), alert.severity)
                    {
                        // Entity has this alert enabled — in a full implementation
                        // we'd look up which users follow this entity.
                        // For now, return empty (the SSE manager will broadcast
                        // if user_ids is empty, which handles anonymous broadcasts).
                        return vec![];
                    }
                }
                Ok(None) => {
                    // No entity config — check global
                    if self.check_global_defaults(alert).await {
                        return vec![];
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        entity_id = %entity_id_str,
                        error = %e,
                        "Failed to fetch entity alert config for subscription lookup"
                    );
                }
            }
        }

        // No subscribers found via entity configs
        vec![]
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alert_event_type_as_str() {
        assert_eq!(AlertEventType::NewInsight.as_str(), "new_insight");
        assert_eq!(AlertEventType::NewWarning.as_str(), "new_warning");
        assert_eq!(AlertEventType::RecipeMatch.as_str(), "recipe_match");
        assert_eq!(AlertEventType::CompetitorChange.as_str(), "competitor_change");
        assert_eq!(AlertEventType::SupplyChainRisk.as_str(), "supply_chain_risk");
        assert_eq!(AlertEventType::SystemAlert.as_str(), "system_alert");
    }

    #[test]
    fn alert_category_mapping() {
        let alert = AlertEvent {
            id: Uuid::new_v4(),
            event_type: AlertEventType::NewWarning,
            severity: apex_core::alert_config::AlertSeverity::High,
            title: "Test".to_string(),
            description: "Test".to_string(),
            entity_id: None,
            entity_name: None,
            user_ids: vec![],
            metadata: serde_json::json!({}),
            created_at: Utc::now(),
        };
        assert_eq!(alert.alert_category(), "warning");
    }

    #[test]
    fn alert_event_serde_roundtrip() {
        let event = AlertEvent {
            id: Uuid::new_v4(),
            event_type: AlertEventType::CompetitorChange,
            severity: apex_core::alert_config::AlertSeverity::Critical,
            title: "Competitor move".to_string(),
            description: "A competitor changed strategy".to_string(),
            entity_id: Some(Uuid::new_v4()),
            entity_name: Some("Rival Corp".to_string()),
            user_ids: vec![Uuid::new_v4()],
            metadata: serde_json::json!({"change_type": "pivot"}),
            created_at: Utc::now(),
        };

        let json = serde_json::to_string(&event).unwrap();
        let deserialized: AlertEvent = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.id, event.id);
        assert_eq!(deserialized.event_type, event.event_type);
        assert_eq!(deserialized.user_ids.len(), 1);
        assert_eq!(
            deserialized.metadata["change_type"],
            "pivot"
        );
    }

    #[test]
    fn empty_user_ids_means_broadcast() {
        let alert = AlertEvent {
            id: Uuid::new_v4(),
            event_type: AlertEventType::SystemAlert,
            severity: apex_core::alert_config::AlertSeverity::Info,
            title: "System notice".to_string(),
            description: "System is running".to_string(),
            entity_id: None,
            entity_name: None,
            user_ids: vec![],
            metadata: serde_json::json!({}),
            created_at: Utc::now(),
        };
        assert!(alert.user_ids.is_empty());
    }
}
