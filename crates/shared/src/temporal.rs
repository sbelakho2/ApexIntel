use crate::calibration::ConfidenceInterval;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct TimelineEvent {
    pub event_type: String,
    pub date: DateTime<Utc>,
    pub description: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct EventTimeline {
    pub entity_id: String,
    pub entity_name: String,
    pub events: Vec<TimelineEvent>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct GrangerCausalPair {
    pub cause_signal: String,
    pub effect_signal: String,
    pub optimal_lag_days: u32,
    pub p_value: f64,
    pub significant_after_fdr: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct PredictiveAlert {
    pub trigger_signal: String,
    pub predicted_signal: String,
    pub expected_within_days: u32,
    pub historical_precision: f64,
    pub confidence_interval: ConfidenceInterval,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct TemporalConsistency {
    pub valid: bool,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct SurvivalPoint {
    pub day: u32,
    pub survival_probability: f64,
}