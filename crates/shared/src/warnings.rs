use crate::adversarial::SourceReliabilityTier;
use crate::calibration::{BayesianInterpretation, ConfidenceInterval};
use crate::temporal::TemporalConsistency;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct WarningItem {
    pub id: String,
    pub title: String,
    pub severity: String,
    pub company_name: String,
    pub warning_type: String,
    pub created_at: String,
    pub calibrated_probability: Option<f64>,
    pub bayesian_interpretation: Option<BayesianInterpretation>,
    pub confidence_interval: Option<ConfidenceInterval>,
    pub temporal_consistency: Option<TemporalConsistency>,
    pub information_gain_bits: Option<f64>,
    pub source_reliability_tier: Option<SourceReliabilityTier>,
}
