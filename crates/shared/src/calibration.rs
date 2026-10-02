use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BandClass {
    Narrow,
    #[default]
    Moderate,
    Wide,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct CalibrationPoint {
    pub predicted_probability: f64,
    pub observed_frequency: f64,
    pub bin_count: u32,
}

/// A reliability/calibration curve built from resolved alert samples.
///
/// The metric fields are `Option` because a curve with no samples has no
/// measured Brier score, reliability or resolution. `Default` is therefore
/// "no samples measured", not "everything scored zero".
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct CalibrationCurve {
    pub points: Vec<CalibrationPoint>,
    /// Number of resolved samples the curve was computed from.
    #[serde(default)]
    pub samples: usize,
    /// Mean Brier score over the samples; `None` when `samples == 0`.
    pub brier_score: Option<f64>,
    /// Reliability component (weighted squared bin gap); `None` when
    /// `samples == 0`.
    pub reliability: Option<f64>,
    /// Resolution component (weighted squared bin/base-rate gap); `None` when
    /// `samples == 0`.
    pub resolution: Option<f64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ConfidenceInterval {
    pub value: f64,
    pub lower: f64,
    pub upper: f64,
    pub half_width: f64,
    pub band_class: BandClass,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BayesianInterpretation {
    Decisive,
    VeryStrong,
    Strong,
    Substantial,
    Barely,
    Against,
}

impl BayesianInterpretation {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Decisive => "Decisive",
            Self::VeryStrong => "Very Strong",
            Self::Strong => "Strong",
            Self::Substantial => "Substantial",
            Self::Barely => "Barely",
            Self::Against => "Against",
        }
    }

    pub fn css_class(&self) -> &'static str {
        match self {
            Self::Decisive => "badge-decisive",
            Self::VeryStrong => "badge-very-strong",
            Self::Strong => "badge-strong",
            Self::Substantial => "badge-substantial",
            Self::Barely => "badge-barely",
            Self::Against => "badge-against",
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct BrierScoreEntry {
    pub category: String,
    pub score: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpretation_mappings_are_stable() {
        assert_eq!(BayesianInterpretation::VeryStrong.label(), "Very Strong");
        assert_eq!(BayesianInterpretation::Against.css_class(), "badge-against");
    }

    #[test]
    fn default_curve_reports_no_samples_instead_of_zero_metrics() {
        let curve = CalibrationCurve::default();
        assert_eq!(curve.samples, 0);
        assert!(curve.points.is_empty());
        assert_eq!(curve.brier_score, None);
        assert_eq!(curve.reliability, None);
        assert_eq!(curve.resolution, None);
    }

    #[test]
    fn default_curve_serializes_nulls_not_zeros() {
        let json = serde_json::to_value(CalibrationCurve::default()).expect("serialize");
        assert_eq!(json["samples"], 0);
        assert!(json["brier_score"].is_null());
        assert!(json["reliability"].is_null());
        assert!(json["resolution"].is_null());
        // The points array stays present for consumers that always iterate it.
        assert_eq!(json["points"], serde_json::json!([]));
    }

    #[test]
    fn measured_curve_serializes_values() {
        let curve = CalibrationCurve {
            points: vec![CalibrationPoint {
                predicted_probability: 0.6,
                observed_frequency: 0.5,
                bin_count: 3,
            }],
            samples: 3,
            brier_score: Some(0.25),
            reliability: Some(0.01),
            resolution: Some(0.02),
        };
        let json = serde_json::to_value(&curve).expect("serialize");
        assert_eq!(json["samples"], 3);
        assert_eq!(json["brier_score"], 0.25);
        assert_eq!(json["reliability"], 0.01);
        assert_eq!(json["resolution"], 0.02);
    }
}
