//! WebSocket route helpers for warning stream payload contracts.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WarningEvent {
    pub warning_id: String,
    pub severity: String,
    pub warning_type: String,
    pub title: String,
    pub region: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsEnvelope<T> {
    pub channel: String,
    pub event: String,
    pub payload: T,
    pub emitted_at: DateTime<Utc>,
}

pub fn warning_channel(region: Option<&str>) -> String {
    match region {
        Some(r) if !r.trim().is_empty() => format!("warnings:{}", r.to_uppercase()),
        _ => "warnings:all".to_string(),
    }
}

pub fn to_ws_event(event: WarningEvent) -> WsEnvelope<WarningEvent> {
    WsEnvelope {
        channel: warning_channel(Some(&event.region)),
        event: "warning.created".to_string(),
        payload: event,
        emitted_at: Utc::now(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_warning_channel() {
        assert_eq!(warning_channel(None), "warnings:all");
        assert_eq!(warning_channel(Some("")), "warnings:all");
        assert_eq!(warning_channel(Some("tn")), "warnings:TN");
    }

    #[test]
    fn test_to_ws_event() {
        let ev = WarningEvent {
            warning_id: "w1".to_string(),
            severity: "high".to_string(),
            warning_type: "security".to_string(),
            title: "New lookalike domain".to_string(),
            region: "TN".to_string(),
            created_at: Utc::now(),
        };
        let ws = to_ws_event(ev);
        assert_eq!(ws.channel, "warnings:TN");
        assert_eq!(ws.event, "warning.created");
    }

    #[test]
    fn test_ws_envelope_serialization() {
        let ws = WsEnvelope {
            channel: "warnings:all".to_string(),
            event: "ping".to_string(),
            payload: "ok".to_string(),
            emitted_at: Utc::now(),
        };
        let json = serde_json::to_string(&ws).unwrap();
        assert!(json.contains("warnings:all"));
    }
}
