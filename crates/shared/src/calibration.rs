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

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct CalibrationCurve {
    pub points: Vec<CalibrationPoint>,
    pub brier_score: f64,
    pub reliability: f64,
    pub resolution: f64,
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
}
