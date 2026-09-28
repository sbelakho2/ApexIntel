//! Alert routing logic — determines which users should receive which alerts.
//!
//! The [`AlertRouter`] resolves an [`AlertEvent`]'s explicit
//! [`AlertAudience`] into an [`AlertRoutingDecision`]:
//!
//! * [`AlertAudience::Broadcast`] — a deliberate system-wide alert; delivered
//!   to every connected user.
//! * [`AlertAudience::Users`] with explicit principals — each candidate is
//!   checked against their `EntityAlertConfig` / `GlobalAlertDefaults` policy.
//! * [`AlertAudience::Users`] with no principals — the entity-subscription
//!   resolver (`user_alert_subscriptions`) determines the candidate set.
//!
//! An empty candidate list is **never** widened into a broadcast: it resolves
//! to [`AlertRoutingDecision::NoRecipients`]. A policy read that fails is
//! **never** treated as "allowed": it resolves to
//! [`AlertRoutingDecision::RetryableFailure`] so the JetStream message can be
//! retried instead of leaking through a disabled/suppressed preference.
//!
//! This module re-exports the shared [`AlertEvent`] type used by both the
//! NATS publisher (worker) and the SSE consumer (API server).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::Arc;
use uuid::Uuid;

pub use apex_core::alert_config::AlertAudience;

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
///
/// Wire-compatible with the worker publisher's format: the full `entity_ids`
/// set (with the legacy singular `entity_id` still accepted on deserialize)
/// and an explicit [`AlertAudience`]. The legacy `user_ids: []` encoding —
/// whose empty value was ambiguously both "everyone" and "nobody" — is
/// accepted only for messages published by older binaries and maps to
/// [`AlertAudience::Broadcast`], matching what those binaries meant by it.
/// New publishes always carry the tagged `audience`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(from = "AlertEventWire")]
pub struct AlertEvent {
    pub id: Uuid,
    pub event_type: AlertEventType,
    pub severity: apex_core::alert_config::AlertSeverity,
    pub title: String,
    pub description: String,
    /// Complete entity set the alert references. Subscriber resolution covers
    /// every entry (union), so a multi-entity warning never notifies only the
    /// first entity's subscribers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entity_ids: Vec<Uuid>,
    pub entity_name: Option<String>,
    /// Who this alert is addressed to. `Users(vec![])` addresses nobody and
    /// only a deliberate `Broadcast` reaches every connected user.
    pub audience: AlertAudience,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

/// Wire form that also accepts the legacy `entity_id` / `user_ids` fields.
#[derive(Deserialize)]
struct AlertEventWire {
    id: Uuid,
    event_type: AlertEventType,
    severity: apex_core::alert_config::AlertSeverity,
    title: String,
    description: String,
    #[serde(default)]
    entity_ids: Vec<Uuid>,
    #[serde(default)]
    entity_id: Option<Uuid>,
    #[serde(default)]
    entity_name: Option<String>,
    #[serde(default)]
    audience: Option<AlertAudience>,
    /// Legacy (pre-audience) recipient encoding. Empty meant broadcast to the
    /// publishing binary; non-empty meant those exact users.
    #[serde(default)]
    user_ids: Option<Vec<Uuid>>,
    #[serde(default)]
    metadata: serde_json::Value,
    created_at: DateTime<Utc>,
}

impl From<AlertEventWire> for AlertEvent {
    fn from(wire: AlertEventWire) -> Self {
        let mut entity_ids = wire.entity_ids;
        if entity_ids.is_empty() {
            if let Some(entity_id) = wire.entity_id {
                entity_ids.push(entity_id);
            }
        }
        let audience = match (wire.audience, wire.user_ids) {
            // New wire format: explicit audience always wins.
            (Some(audience), _) => audience,
            // Legacy format: an empty list was the old publishers' broadcast.
            (None, Some(user_ids)) if user_ids.is_empty() => AlertAudience::Broadcast,
            (None, Some(user_ids)) => AlertAudience::Users(user_ids),
            // Neither field: unresolved, addresses nobody.
            (None, None) => AlertAudience::Users(Vec::new()),
        };
        Self {
            id: wire.id,
            event_type: wire.event_type,
            severity: wire.severity,
            title: wire.title,
            description: wire.description,
            entity_ids,
            entity_name: wire.entity_name,
            audience,
            metadata: wire.metadata,
            created_at: wire.created_at,
        }
    }
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

    /// Primary entity for display and per-entity config lookups, if any.
    pub fn primary_entity_id(&self) -> Option<Uuid> {
        self.entity_ids.first().copied()
    }

    /// A copy addressed to a resolved audience.
    pub fn with_audience(&self, audience: AlertAudience) -> Self {
        let mut routed = self.clone();
        routed.audience = audience;
        routed
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// AlertRouter
// ─────────────────────────────────────────────────────────────────────────────

/// The outcome of resolving an alert's audience.
///
/// A `Vec<Uuid>` cannot express the difference between "everyone", "nobody"
/// and "could not resolve" — which is exactly how suppressed alerts used to be
/// re-broadcast: the router returned an empty list for "suppressed" and the
/// caller read it as "no override, keep the original (broadcast) recipients".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AlertRoutingDecision {
    /// Deliver to exactly these users (never empty).
    Targets(Vec<Uuid>),
    /// A deliberate system-wide broadcast to every connected user.
    Broadcast,
    /// Resolved successfully: nobody should receive this event. The consumer
    /// must ack and drop it — this is an authorization decision, not a
    /// failure, and must never be re-interpreted as a broadcast.
    NoRecipients,
    /// The routing policy could not be read (storage failure). The event must
    /// be retried (NAK), never delivered and never acked as processed.
    RetryableFailure(String),
}

/// Routes alerts to the appropriate users based on configuration.
pub struct AlertRouter {
    db: Arc<apex_store::postgres::PgStore>,
}

impl AlertRouter {
    /// Create a new alert router backed by the database.
    pub fn new(db: Arc<apex_store::postgres::PgStore>) -> Self {
        Self { db }
    }

    /// Resolve the users an alert should be delivered to.
    ///
    /// * Broadcast audience → [`AlertRoutingDecision::Broadcast`].
    /// * Explicit users → filtered by each user's delivery policy.
    /// * Empty users + entity scope → the `user_alert_subscriptions`
    ///   canonical subscriber resolver, then the same policy filter.
    /// * Empty users without entity scope → `NoRecipients`.
    ///
    /// Any policy/subscription storage failure resolves to
    /// [`AlertRoutingDecision::RetryableFailure`]: a database outage fails
    /// closed (retry) instead of leaking suppressed alerts to recipients.
    pub async fn route_alert(&self, alert: &AlertEvent) -> AlertRoutingDecision {
        match &alert.audience {
            AlertAudience::Broadcast => AlertRoutingDecision::Broadcast,
            AlertAudience::Users(user_ids) if !user_ids.is_empty() => {
                self.filter_by_policy(user_ids, alert).await
            }
            AlertAudience::Users(_) => self.resolve_subscribers(alert).await,
        }
    }

    /// Filter an explicit candidate list through each user's delivery policy.
    async fn filter_by_policy(
        &self,
        candidates: &[Uuid],
        alert: &AlertEvent,
    ) -> AlertRoutingDecision {
        let mut targets = Vec::with_capacity(candidates.len());
        for user_id in candidates {
            match self.policy_allows(*user_id, alert).await {
                Ok(true) => targets.push(*user_id),
                Ok(false) => {}
                Err(error) => return AlertRoutingDecision::RetryableFailure(error),
            }
        }
        if targets.is_empty() {
            AlertRoutingDecision::NoRecipients
        } else {
            AlertRoutingDecision::Targets(targets)
        }
    }

    /// Resolve the entity-scoped subscribers for an alert that carries no
    /// explicit recipients.
    async fn resolve_subscribers(&self, alert: &AlertEvent) -> AlertRoutingDecision {
        if alert.entity_ids.is_empty() {
            // No entity and no explicit users: unresolved audience. This is
            // "deliver to nobody", never a broadcast.
            return AlertRoutingDecision::NoRecipients;
        }

        let subscribers = match self
            .db
            .find_subscribed_users_for_entities(
                &alert.entity_ids,
                alert.alert_category(),
                alert.severity,
            )
            .await
        {
            Ok(users) => users,
            Err(error) => {
                return AlertRoutingDecision::RetryableFailure(format!(
                    "subscriber lookup failed for {} entity(ies): {error}",
                    alert.entity_ids.len()
                ));
            }
        };

        if subscribers.is_empty() {
            return AlertRoutingDecision::NoRecipients;
        }

        self.filter_by_policy(&subscribers, alert).await
    }

    /// Check whether a specific user's delivery policy allows this alert.
    ///
    /// Returns `Err` when the policy cannot be read: the caller must treat
    /// that as unresolved (retry), never as "allowed".
    pub async fn policy_allows(&self, _user_id: Uuid, alert: &AlertEvent) -> Result<bool, String> {
        use apex_core::alert_config::AlertChannel;

        // If the alert targets a specific entity, check its config
        if let Some(entity_id) = alert.primary_entity_id() {
            let entity_id_str = entity_id.to_string();
            match self.db.get_entity_alert_config(&entity_id_str).await {
                Ok(Some(cfg)) => {
                    // Check that InApp channel is enabled
                    if !cfg.enabled_channels.contains(&AlertChannel::InApp) {
                        return Ok(false);
                    }
                    // Check if the alert is suppressed
                    if cfg.is_alert_suppressed(alert.alert_category(), alert.severity) {
                        return Ok(false);
                    }
                    return Ok(true);
                }
                Ok(None) => {
                    // No per-entity config — check global defaults
                }
                Err(e) => {
                    return Err(format!(
                        "entity alert config read failed for {entity_id_str}: {e}"
                    ));
                }
            }
        }

        self.check_global_defaults(alert).await
    }

    /// Check the global alert defaults.
    async fn check_global_defaults(&self, alert: &AlertEvent) -> Result<bool, String> {
        use apex_core::alert_config::AlertChannel;

        match self.db.get_global_alert_defaults().await {
            Ok(Some(defaults)) => {
                // Check that InApp channel is enabled globally
                if !defaults.enabled_channels.contains(&AlertChannel::InApp) {
                    return Ok(false);
                }
                // Check severity threshold
                Ok(alert.severity >= defaults.min_severity)
            }
            Ok(None) => {
                // No global config — allow by default if severity >= Medium
                Ok(alert.severity >= apex_core::alert_config::AlertSeverity::Medium)
            }
            Err(e) => Err(format!("global alert defaults read failed: {e}")),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn base_event(audience: AlertAudience) -> AlertEvent {
        AlertEvent {
            id: Uuid::new_v4(),
            event_type: AlertEventType::NewWarning,
            severity: apex_core::alert_config::AlertSeverity::High,
            title: "Test".to_string(),
            description: "Test".to_string(),
            entity_ids: vec![],
            entity_name: None,
            audience,
            metadata: serde_json::json!({}),
            created_at: Utc::now(),
        }
    }

    /// A routing decision never becomes a broadcast from an empty list.
    #[test]
    fn routing_decision_distinguishes_nobody_from_everyone() {
        assert_ne!(
            AlertRoutingDecision::NoRecipients,
            AlertRoutingDecision::Broadcast
        );
        assert_ne!(
            AlertRoutingDecision::Targets(vec![]),
            AlertRoutingDecision::Broadcast
        );
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
        let alert = base_event(AlertAudience::Broadcast);
        assert_eq!(alert.alert_category(), "warning");
    }

    #[test]
    fn alert_event_serde_roundtrip() {
        let entity_id = Uuid::new_v4();
        let user_id = Uuid::new_v4();
        let event = AlertEvent {
            id: Uuid::new_v4(),
            event_type: AlertEventType::CompetitorChange,
            severity: apex_core::alert_config::AlertSeverity::Critical,
            title: "Competitor move".to_string(),
            description: "A competitor changed strategy".to_string(),
            entity_ids: vec![entity_id],
            entity_name: Some("Rival Corp".to_string()),
            audience: AlertAudience::Users(vec![user_id]),
            metadata: serde_json::json!({"change_type": "pivot"}),
            created_at: Utc::now(),
        };

        let json = serde_json::to_string(&event).unwrap();
        let deserialized: AlertEvent = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.id, event.id);
        assert_eq!(deserialized.event_type, event.event_type);
        assert_eq!(deserialized.entity_ids, vec![entity_id]);
        assert_eq!(deserialized.audience, AlertAudience::Users(vec![user_id]));
        assert_eq!(deserialized.metadata["change_type"], "pivot");
    }

    /// The tagged wire format never carries an ambiguous empty `user_ids`.
    #[test]
    fn wire_format_is_tagged_and_explicit() {
        let broadcast = base_event(AlertAudience::Broadcast);
        let json = serde_json::to_value(&broadcast).unwrap();
        assert_eq!(json["audience"]["kind"], "broadcast");

        let empty = base_event(AlertAudience::Users(vec![]));
        let json = serde_json::to_value(&empty).unwrap();
        assert_eq!(json["audience"]["kind"], "users");
        assert_eq!(json["audience"]["user_ids"], serde_json::json!([]));
        // The legacy ambiguous field must not be emitted.
        assert!(json.get("user_ids").is_none());
    }

    /// Legacy messages published by older binaries still decode: an empty
    /// `user_ids` was their deliberate broadcast; a non-empty list was
    /// targeted.
    #[test]
    fn legacy_wire_format_still_decodes() {
        let legacy_broadcast = serde_json::json!({
            "id": Uuid::new_v4(),
            "event_type": "system_alert",
            "severity": "info",
            "title": "System notice",
            "description": "System is running",
            "entity_id": null,
            "entity_name": null,
            "user_ids": [],
            "metadata": {},
            "created_at": Utc::now(),
        });
        let alert: AlertEvent = serde_json::from_value(legacy_broadcast).unwrap();
        assert_eq!(alert.audience, AlertAudience::Broadcast);

        let user_id = Uuid::new_v4();
        let legacy_targeted = serde_json::json!({
            "id": Uuid::new_v4(),
            "event_type": "new_warning",
            "severity": "high",
            "title": "Targeted",
            "description": "For one user",
            "entity_id": Uuid::new_v4(),
            "entity_name": null,
            "user_ids": [user_id],
            "metadata": {},
            "created_at": Utc::now(),
        });
        let alert: AlertEvent = serde_json::from_value(legacy_targeted).unwrap();
        assert_eq!(alert.audience, AlertAudience::Users(vec![user_id]));
        assert_eq!(alert.entity_ids.len(), 1);
    }

    /// A legacy message with neither field addresses nobody — it must not be
    /// upgraded to a broadcast.
    #[test]
    fn legacy_wire_without_recipients_addresses_nobody() {
        let legacy = serde_json::json!({
            "id": Uuid::new_v4(),
            "event_type": "new_insight",
            "severity": "high",
            "title": "No audience",
            "description": "Ambiguous legacy event",
            "metadata": {},
            "created_at": Utc::now(),
        });
        let alert: AlertEvent = serde_json::from_value(legacy).unwrap();
        assert_eq!(alert.audience, AlertAudience::Users(vec![]));
    }
}
