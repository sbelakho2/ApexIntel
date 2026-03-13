use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SourceReliabilityTier {
    Official,
    Established,
    TradePress,
    Social,
    Unknown,
}

impl Default for SourceReliabilityTier {
    fn default() -> Self {
        Self::Unknown
    }
}

impl SourceReliabilityTier {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Official => "Official",
            Self::Established => "Established",
            Self::TradePress => "Trade Press",
            Self::Social => "Social",
            Self::Unknown => "Unknown",
        }
    }

    pub fn css_class(&self) -> &'static str {
        match self {
            Self::Official => "tier-official",
            Self::Established => "tier-established",
            Self::TradePress => "tier-trade-press",
            Self::Social => "tier-social",
            Self::Unknown => "tier-unknown",
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct PlacementAlert {
    pub id: String,
    pub source_count: u32,
    pub time_window_hours: u32,
    pub token_jaccard: f64,
    pub signal_ids: Vec<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct QuarantineItem {
    pub id: String,
    pub reason: String,
    pub release_at: DateTime<Utc>,
    pub source_domain: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct SourceReliabilityPoint {
    pub observed_reliability: f64,
    pub effective_reliability: f64,
    pub recorded_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct SourceReliabilityHistory {
    pub domain: String,
    pub tier: SourceReliabilityTier,
    pub promotion_candidate: bool,
    pub history: Vec<SourceReliabilityPoint>,
}