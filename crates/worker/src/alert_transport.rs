//! The ONE alert transport seam.
//!
//! `NatsPublisher::publish_alert` is reachable from production code only
//! through this module. CI enforces the boundary with
//! `scripts/ci/check_alert_publish.sh`: a `publish_alert(` call anywhere else in
//! `crates/` fails the build.
//!
//! Producer paths must never publish directly. Domain alerts are committed to
//! `event_outbox` in the same transaction as their source row and published by
//! the canonical outbox drain ([`crate::alert_pipeline`]), which passes the
//! stable outbox row id as the JetStream `Nats-Msg-Id` so the broker can
//! deduplicate an at-least-once republish inside its duplicate window.

use anyhow::Result;

use apex_worker::nats_stream::{AlertEvent, NatsPublisher};

/// Thin transport wrapper around [`NatsPublisher`] used by the outbox
/// publisher. Cloneable so fast-path and drain can share one connection.
#[derive(Clone)]
pub struct AlertTransport {
    publisher: NatsPublisher,
}

impl AlertTransport {
    pub fn new(publisher: NatsPublisher) -> Self {
        Self { publisher }
    }

    /// A transport that never delivers (no NATS configured).
    pub fn disabled() -> Self {
        Self::new(NatsPublisher::disabled())
    }

    /// A disabled transport that reports unavailable NATS as an error.
    pub fn disabled_with_requirement(required: bool) -> Self {
        Self::new(NatsPublisher::disabled_with_requirement(required))
    }

    /// Whether the underlying publisher is connected.
    pub fn is_connected(&self) -> bool {
        self.publisher.is_connected()
    }

    /// Whether an unavailable NATS is a hard failure for this deployment.
    pub fn required(&self) -> bool {
        self.publisher.required()
    }

    /// Publish an alert event and await the JetStream ACK.
    ///
    /// `msg_id` is the stable transport identity (the outbox row id); it is sent
    /// as `Nats-Msg-Id`.
    pub async fn publish_event(&self, event: &AlertEvent, msg_id: &str) -> Result<()> {
        self.publisher
            .publish_alert_with_msg_id(event, Some(msg_id))
            .await
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use apex_worker::nats_stream::{AlertEventType, JetStreamTransport, PendingPublishAck};
    use async_trait::async_trait;
    use futures::future::BoxFuture;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct TransportLog {
        msg_ids: Mutex<Vec<Option<String>>>,
    }

    struct OkAck;

    impl PendingPublishAck for OkAck {
        fn wait_for_ack(self: Box<Self>) -> BoxFuture<'static, Result<()>> {
            Box::pin(async move { Ok(()) })
        }
    }

    struct RecordingTransport {
        log: Arc<TransportLog>,
    }

    #[async_trait]
    impl JetStreamTransport for RecordingTransport {
        async fn publish(
            &self,
            _subject: String,
            msg_id: Option<String>,
            _payload: Vec<u8>,
        ) -> Result<Box<dyn PendingPublishAck>> {
            self.log.msg_ids.lock().unwrap().push(msg_id);
            Ok(Box::new(OkAck))
        }
    }

    fn sample_event() -> AlertEvent {
        AlertEvent {
            id: uuid::Uuid::new_v4(),
            event_type: AlertEventType::NewWarning,
            severity: apex_core::alert_config::AlertSeverity::High,
            title: "t".to_string(),
            description: "d".to_string(),
            entity_id: None,
            entity_name: None,
            audience: apex_core::alert_config::AlertAudience::Users(vec![]),
            metadata: serde_json::json!({}),
            created_at: chrono::Utc::now(),
        }
    }

    #[tokio::test]
    async fn transport_passes_the_stable_msg_id() {
        let log = Arc::new(TransportLog::default());
        let publisher = NatsPublisher::with_transport(
            Arc::new(RecordingTransport {
                log: Arc::clone(&log),
            }),
            false,
        );
        let transport = AlertTransport::new(publisher);

        transport
            .publish_event(&sample_event(), "outbox-row-1")
            .await
            .unwrap();

        assert_eq!(
            log.msg_ids.lock().unwrap().as_slice(),
            &[Some("outbox-row-1".to_string())]
        );
    }
}
