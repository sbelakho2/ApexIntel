//! NATS JetStream publisher for alert events.
//!
//! Provides a [`NatsPublisher`] that connects to NATS, ensures the `alerts`
//! JetStream stream exists, and publishes [`AlertEvent`]s to subjects
//! `alerts.events.{event_type}`.
//!
//! # Stream configuration
//! - Name: `alerts`
//! - Subjects: `alerts.>`
//! - Max age: 7 days
//! - Storage: file
//! - Retention: interest-based (auto-cleanup when consumers acknowledge)

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};
use uuid::Uuid;

// ─────────────────────────────────────────────────────────────────────────────
// Alert event types
// ─────────────────────────────────────────────────────────────────────────────

/// The kind of alert event being published.
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

impl std::fmt::Display for AlertEventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An alert event to be published through NATS JetStream.
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
    /// Build the NATS subject for this event.
    pub fn subject(&self) -> String {
        format!("alerts.events.{}", self.event_type)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// NATS Publisher
// ─────────────────────────────────────────────────────────────────────────────

/// Publishes alert events to NATS JetStream.
///
/// Gracefully degrades when NATS is unavailable: all `publish_alert` calls
/// log a warning and return `Ok(())`.
pub struct NatsPublisher {
    client: Option<async_nats::Client>,
    jetstream: Option<async_nats::jetstream::Context>,
    nats_url: String,
}

impl NatsPublisher {
    /// Create a new disabled publisher (no NATS connection).
    pub fn disabled() -> Self {
        Self {
            client: None,
            jetstream: None,
            nats_url: String::new(),
        }
    }

    /// Connect to NATS and ensure the JetStream stream exists.
    ///
    /// If the connection fails, the publisher will operate in degraded mode
    /// (all publishes become no-ops with warnings).
    pub async fn connect(nats_url: &str) -> Self {
        match Self::try_connect(nats_url).await {
            Ok((client, jetstream)) => {
                info!(nats_url = %nats_url, "NATS JetStream publisher connected");
                Self {
                    client: Some(client),
                    jetstream: Some(jetstream),
                    nats_url: nats_url.to_string(),
                }
            }
            Err(e) => {
                warn!(nats_url = %nats_url, error = %e, "NATS unavailable — alert publishing degraded");
                Self {
                    client: None,
                    jetstream: None,
                    nats_url: nats_url.to_string(),
                }
            }
        }
    }

    async fn try_connect(
        nats_url: &str,
    ) -> Result<(async_nats::Client, async_nats::jetstream::Context)> {
        let client = async_nats::connect(nats_url)
            .await
            .context("failed to connect to NATS")?;
        let jetstream = async_nats::jetstream::new(client.clone());

        // Ensure the stream exists (idempotent)
        Self::ensure_stream(&jetstream).await?;

        Ok((client, jetstream))
    }

    /// Ensure the `alerts` JetStream stream exists, creating it if necessary.
    async fn ensure_stream(jetstream: &async_nats::jetstream::Context) -> Result<()> {
        use async_nats::jetstream::stream::Config;

        let cfg = Config {
            name: "alerts".to_string(),
            subjects: vec!["alerts.>".to_string()],
            max_age: chrono::Duration::days(7)
                .to_std()
                .unwrap_or(std::time::Duration::from_secs(604800)),
            storage: async_nats::jetstream::stream::StorageType::File,
            retention: async_nats::jetstream::stream::RetentionPolicy::Interest,
            ..Config::default()
        };

        match jetstream.get_stream("alerts").await {
            Ok(_) => {
                info!("NATS JetStream stream 'alerts' already exists");
                Ok(())
            }
            Err(_) => {
                jetstream
                    .create_stream(cfg)
                    .await
                    .context("failed to create JetStream stream 'alerts'")?;
                info!("NATS JetStream stream 'alerts' created");
                Ok(())
            }
        }
    }

    /// Publish an alert event to NATS JetStream.
    ///
    /// If NATS is unavailable, logs a warning and returns `Ok(())`
    /// (graceful degradation).
    pub async fn publish_alert(&self, alert: &AlertEvent) -> Result<()> {
        let Some(ref jetstream) = self.jetstream else {
            warn!("NATS publisher not connected — skipping alert publish");
            return Ok(());
        };

        let subject = alert.subject();
        let payload =
            serde_json::to_vec(alert).context("failed to serialize AlertEvent to JSON")?;

        jetstream
            .publish(subject.clone(), payload.into())
            .await
            .context(format!(
                "failed to publish alert to NATS subject '{subject}'"
            ))?;

        info!(
            alert_id = %alert.id,
            event_type = %alert.event_type,
            subject = %subject,
            "Alert event published to NATS JetStream"
        );

        Ok(())
    }

    /// Returns `true` if NATS is connected and operational.
    pub fn is_connected(&self) -> bool {
        self.client.is_some()
    }

    /// Returns the NATS URL this publisher was configured with.
    pub fn nats_url(&self) -> &str {
        &self.nats_url
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alert_event_subject_format() {
        let event = AlertEvent {
            id: Uuid::new_v4(),
            event_type: AlertEventType::NewWarning,
            severity: apex_core::alert_config::AlertSeverity::High,
            title: "Test warning".to_string(),
            description: "A test warning event".to_string(),
            entity_id: Some(Uuid::new_v4()),
            entity_name: Some("Test Corp".to_string()),
            user_ids: vec![],
            metadata: serde_json::json!({}),
            created_at: Utc::now(),
        };
        assert_eq!(event.subject(), "alerts.events.new_warning");
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
    fn disabled_publisher_is_not_connected() {
        let publisher = NatsPublisher::disabled();
        assert!(!publisher.is_connected());
    }

    #[test]
    fn alert_event_serde_roundtrip() {
        let event = AlertEvent {
            id: Uuid::new_v4(),
            event_type: AlertEventType::RecipeMatch,
            severity: apex_core::alert_config::AlertSeverity::Critical,
            title: "Recipe matched".to_string(),
            description: "A new recipe match found".to_string(),
            entity_id: None,
            entity_name: None,
            user_ids: vec![Uuid::new_v4()],
            metadata: serde_json::json!({"score": 0.95}),
            created_at: Utc::now(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: AlertEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.id, event.id);
        assert_eq!(deserialized.event_type, event.event_type);
        assert_eq!(deserialized.user_ids.len(), 1);
    }
}
