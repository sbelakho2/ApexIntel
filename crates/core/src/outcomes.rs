use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Outcome events represent OSINT-derived "labels" that serve as supervised
/// signals for the continuous learning loop. Even without Starz internal data,
/// these outcomes are defined purely from public sources.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum OutcomeEvent {
    RfQPosted,
    CertificationUpdate,
    PlantExpansionSignal,
    DistressSignal,
    SecurityImpersonationDetected,
    SupplierBreachDisclosed,
    PortShock,
    CompetitorCapabilityShift,
    RoleChange,
    AllocationWave,
    RegulatoryShift,
    MnAEvent,
    PriceWarSignal,
    QualityEscapeEvent,
    ContractAward,
}

impl OutcomeEvent {
    pub fn as_str(&self) -> &str {
        match self {
            Self::RfQPosted => "RfQPosted",
            Self::CertificationUpdate => "CertificationUpdate",
            Self::PlantExpansionSignal => "PlantExpansionSignal",
            Self::DistressSignal => "DistressSignal",
            Self::SecurityImpersonationDetected => "SecurityImpersonationDetected",
            Self::SupplierBreachDisclosed => "SupplierBreachDisclosed",
            Self::PortShock => "PortShock",
            Self::CompetitorCapabilityShift => "CompetitorCapabilityShift",
            Self::RoleChange => "RoleChange",
            Self::AllocationWave => "AllocationWave",
            Self::RegulatoryShift => "RegulatoryShift",
            Self::MnAEvent => "MnAEvent",
            Self::PriceWarSignal => "PriceWarSignal",
            Self::QualityEscapeEvent => "QualityEscapeEvent",
            Self::ContractAward => "ContractAward",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "RfQPosted" => Some(Self::RfQPosted),
            "CertificationUpdate" => Some(Self::CertificationUpdate),
            "PlantExpansionSignal" => Some(Self::PlantExpansionSignal),
            "DistressSignal" => Some(Self::DistressSignal),
            "SecurityImpersonationDetected" => Some(Self::SecurityImpersonationDetected),
            "SupplierBreachDisclosed" => Some(Self::SupplierBreachDisclosed),
            "PortShock" => Some(Self::PortShock),
            "CompetitorCapabilityShift" => Some(Self::CompetitorCapabilityShift),
            "RoleChange" => Some(Self::RoleChange),
            "AllocationWave" => Some(Self::AllocationWave),
            "RegulatoryShift" => Some(Self::RegulatoryShift),
            "MnAEvent" => Some(Self::MnAEvent),
            "PriceWarSignal" => Some(Self::PriceWarSignal),
            "QualityEscapeEvent" => Some(Self::QualityEscapeEvent),
            "ContractAward" => Some(Self::ContractAward),
            _ => None,
        }
    }

    /// All possible variants, useful for iteration.
    pub fn all() -> Vec<Self> {
        vec![
            Self::RfQPosted,
            Self::CertificationUpdate,
            Self::PlantExpansionSignal,
            Self::DistressSignal,
            Self::SecurityImpersonationDetected,
            Self::SupplierBreachDisclosed,
            Self::PortShock,
            Self::CompetitorCapabilityShift,
            Self::RoleChange,
            Self::AllocationWave,
            Self::RegulatoryShift,
            Self::MnAEvent,
            Self::PriceWarSignal,
            Self::QualityEscapeEvent,
            Self::ContractAward,
        ]
    }
}

/// A recorded outcome event instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutcomeRecord {
    pub id: Uuid,
    pub event: OutcomeEvent,
    pub entity_id: Uuid,
    pub entity_type: String,
    pub ts_utc: DateTime<Utc>,
    pub details: serde_json::Value,
    pub source_observation_ids: Vec<Uuid>,
    pub created_at: DateTime<Utc>,
}

impl OutcomeRecord {
    pub fn new(event: OutcomeEvent, entity_id: Uuid, entity_type: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            event,
            entity_id,
            entity_type: entity_type.into(),
            ts_utc: now,
            details: serde_json::json!({}),
            source_observation_ids: Vec::new(),
            created_at: now,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_outcome_event_roundtrip() {
        for event in OutcomeEvent::all() {
            let s = event.as_str();
            let back = OutcomeEvent::from_str(s).unwrap();
            assert_eq!(event, back);
        }
    }

    #[test]
    fn test_outcome_event_all_count() {
        assert_eq!(OutcomeEvent::all().len(), 15);
    }

    #[test]
    fn test_outcome_event_unknown() {
        assert!(OutcomeEvent::from_str("UnknownEvent").is_none());
    }

    #[test]
    fn test_outcome_record_new() {
        let entity_id = Uuid::new_v4();
        let record = OutcomeRecord::new(OutcomeEvent::PortShock, entity_id, "logistics_node");
        assert_eq!(record.event, OutcomeEvent::PortShock);
        assert_eq!(record.entity_id, entity_id);
        assert_eq!(record.entity_type, "logistics_node");
        assert!(record.source_observation_ids.is_empty());
    }

    #[test]
    fn test_outcome_record_serialize() {
        let record = OutcomeRecord::new(OutcomeEvent::ContractAward, Uuid::new_v4(), "company");
        let json = serde_json::to_string(&record).unwrap();
        let record2: OutcomeRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(record.id, record2.id);
        assert_eq!(record.event, record2.event);
    }
}
