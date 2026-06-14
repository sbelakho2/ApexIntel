//! # Data Models for Threat Intelligence Module
//!
//! Shared data structures and enums used across all threat intelligence components.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ============================================================================
// Shared Types
// ============================================================================

/// Industry sector classification for threat analysis.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum IndustrySector {
    Automotive,
    Aerospace,
    Electronics,
    Semiconductor,
    Healthcare,
    Financial,
    Energy,
    Telecommunications,
    Defense,
    Manufacturing,
    Retail,
    Logistics,
    Government,
    Technology,
    Pharmaceuticals,
    Chemicals,
    FoodBeverage,
    Construction,
    Mining,
    Agriculture,
    Education,
    Media,
    Gaming,
    EnergyStorage,
    Battery,
    Other(String),
}

impl IndustrySector {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Automotive => "automotive",
            Self::Aerospace => "aerospace",
            Self::Electronics => "electronics",
            Self::Semiconductor => "semiconductor",
            Self::Healthcare => "healthcare",
            Self::Financial => "financial",
            Self::Energy => "energy",
            Self::Telecommunications => "telecommunications",
            Self::Defense => "defense",
            Self::Manufacturing => "manufacturing",
            Self::Retail => "retail",
            Self::Logistics => "logistics",
            Self::Government => "government",
            Self::Technology => "technology",
            Self::Pharmaceuticals => "pharmaceuticals",
            Self::Chemicals => "chemicals",
            Self::FoodBeverage => "food_beverage",
            Self::Construction => "construction",
            Self::Mining => "mining",
            Self::Agriculture => "agriculture",
            Self::Education => "education",
            Self::Media => "media",
            Self::Gaming => "gaming",
            Self::EnergyStorage => "energy_storage",
            Self::Battery => "battery",
            Self::Other(s) => s.as_str(),
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "automotive" => Self::Automotive,
            "aerospace" => Self::Aerospace,
            "electronics" => Self::Electronics,
            "semiconductor" => Self::Semiconductor,
            "healthcare" => Self::Healthcare,
            "financial" => Self::Financial,
            "energy" => Self::Energy,
            "telecommunications" | "telecom" => Self::Telecommunications,
            "defense" | "defence" => Self::Defense,
            "manufacturing" => Self::Manufacturing,
            "retail" => Self::Retail,
            "logistics" => Self::Logistics,
            "government" | "gov" => Self::Government,
            "technology" | "tech" => Self::Technology,
            "pharmaceuticals" | "pharma" => Self::Pharmaceuticals,
            "chemicals" => Self::Chemicals,
            "food_beverage" | "food" | "beverage" => Self::FoodBeverage,
            "construction" => Self::Construction,
            "mining" => Self::Mining,
            "agriculture" | "agri" => Self::Agriculture,
            "education" | "edu" => Self::Education,
            "media" => Self::Media,
            "gaming" => Self::Gaming,
            "energy_storage" | "energy storage" | "bess" | "storage" => Self::EnergyStorage,
            "battery" | "batteries" | "battery_manufacturing" => Self::Battery,
            other => Self::Other(other.to_string()),
        }
    }
}

/// Geographic region for risk assessment.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum GeoRegion {
    NorthAmerica,
    SouthAmerica,
    Europe,
    MiddleEast,
    Africa,
    AsiaPacific,
    EastAsia,
    SouthAsia,
    SoutheastAsia,
    CentralAsia,
    Oceania,
    Antarctica,
    Other(String),
}

impl GeoRegion {
    pub fn as_str(&self) -> &str {
        match self {
            Self::NorthAmerica => "north_america",
            Self::SouthAmerica => "south_america",
            Self::Europe => "europe",
            Self::MiddleEast => "middle_east",
            Self::Africa => "africa",
            Self::AsiaPacific => "asia_pacific",
            Self::EastAsia => "east_asia",
            Self::SouthAsia => "south_asia",
            Self::SoutheastAsia => "southeast_asia",
            Self::CentralAsia => "central_asia",
            Self::Oceania => "oceania",
            Self::Antarctica => "antarctica",
            Self::Other(s) => s.as_str(),
        }
    }
}

/// Confidence level for threat intelligence assessments.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ConfidenceLevel {
    High,
    Medium,
    Low,
    Unknown,
}

impl ConfidenceLevel {
    pub fn as_str(&self) -> &str {
        match self {
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
            Self::Unknown => "unknown",
        }
    }

    pub fn from_f64(value: f64) -> Self {
        if value >= 0.8 {
            Self::High
        } else if value >= 0.5 {
            Self::Medium
        } else if value >= 0.2 {
            Self::Low
        } else {
            Self::Unknown
        }
    }
}

/// Severity level for vulnerabilities and threats.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum SeverityLevel {
    Critical,
    High,
    Medium,
    Low,
    Info,
}

impl SeverityLevel {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Critical => "critical",
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
            Self::Info => "info",
        }
    }

    pub fn from_cvss(cvss: f64) -> Self {
        if cvss >= 9.0 {
            Self::Critical
        } else if cvss >= 7.0 {
            Self::High
        } else if cvss >= 4.0 {
            Self::Medium
        } else if cvss >= 0.1 {
            Self::Low
        } else {
            Self::Info
        }
    }
}

/// Risk score with associated metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskScore {
    pub score: f64,
    pub confidence: ConfidenceLevel,
    pub factors: Vec<RiskFactor>,
    pub last_updated: DateTime<Utc>,
}

impl RiskScore {
    pub fn new(score: f64, confidence: ConfidenceLevel) -> Self {
        Self {
            score: score.clamp(0.0, 1.0),
            confidence,
            factors: Vec::new(),
            last_updated: Utc::now(),
        }
    }

    pub fn with_factors(mut self, factors: Vec<RiskFactor>) -> Self {
        self.factors = factors;
        self
    }
}

/// Individual factor contributing to a risk score.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskFactor {
    pub name: String,
    pub contribution: f64,
    pub weight: f64,
    pub description: Option<String>,
}

/// Geographic location for supply chain risk assessment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeoLocation {
    pub country_code: String,
    pub country_name: String,
    pub region: Option<GeoRegion>,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
}

impl GeoLocation {
    pub fn new(country_code: impl Into<String>, country_name: impl Into<String>) -> Self {
        Self {
            country_code: country_code.into().to_uppercase(),
            country_name: country_name.into(),
            region: None,
            lat: None,
            lon: None,
        }
    }
}

/// Time window for threat analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeWindow {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

impl TimeWindow {
    pub fn new(start: DateTime<Utc>, end: DateTime<Utc>) -> Self {
        Self { start, end }
    }

    pub fn days_back(days: u32) -> Self {
        let now = Utc::now();
        let start = now - chrono::Duration::days(days as i64);
        Self { start, end: now }
    }

    pub fn months_back(months: u32) -> Self {
        let now = Utc::now();
        // Use chrono::Months for calendar-month-aware arithmetic instead of
        // approximating with days*30. This correctly handles varying month lengths
        // (28–31 days) and avoids date drift over multi-month windows.
        let start = now - chrono::Months::new(months);
        Self { start, end: now }
    }

    pub fn years_back(years: u32) -> Self {
        let now = Utc::now();
        let start = now - chrono::Duration::days((years as i64) * 365);
        Self { start, end: now }
    }
}

/// Pagination parameters for list queries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaginationParams {
    pub offset: u32,
    pub limit: u32,
}

impl PaginationParams {
    pub fn new(offset: u32, limit: u32) -> Self {
        Self {
            offset,
            limit: limit.min(1000), // Cap at 1000
        }
    }

    pub fn default_page() -> Self {
        Self::new(0, 100)
    }
}

/// Generic paginated response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaginatedResponse<T> {
    pub items: Vec<T>,
    pub total: u64,
    pub offset: u32,
    pub limit: u32,
    pub has_more: bool,
}

impl<T> PaginatedResponse<T> {
    pub fn new(items: Vec<T>, total: u64, offset: u32, limit: u32) -> Self {
        let has_more = (offset + items.len() as u32) < total as u32;
        Self {
            items,
            total,
            offset,
            limit,
            has_more,
        }
    }
}

/// Source attribution for threat intelligence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreatSource {
    pub source_id: String,
    pub source_name: String,
    pub source_type: ThreatSourceType,
    pub reliability_score: f64,
    pub last_fetched: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ThreatSourceType {
    Government,
    IndustryReport,
    OpenSource,
    Commercial,
    Internal,
    Academic,
}

/// Common entity identifier for cross-referencing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityRef {
    pub entity_id: Uuid,
    pub entity_type: String,
    pub name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_industry_sector_parsing() {
        assert_eq!(IndustrySector::from_str("automotive"), IndustrySector::Automotive);
        assert_eq!(IndustrySector::from_str("Automotive"), IndustrySector::Automotive);
        assert_eq!(IndustrySector::from_str("pharma"), IndustrySector::Pharmaceuticals);
        assert_eq!(
            IndustrySector::from_str("custom_sector"),
            IndustrySector::Other("custom_sector".to_string())
        );

        // Battery / BESS pivot taxonomy (aliases collapse to the canonical variant).
        for alias in ["energy_storage", "energy storage", "BESS", "Storage"] {
            assert_eq!(IndustrySector::from_str(alias), IndustrySector::EnergyStorage);
        }
        for alias in ["battery", "Batteries", "battery_manufacturing"] {
            assert_eq!(IndustrySector::from_str(alias), IndustrySector::Battery);
        }

        // as_str round-trips back to a canonical token that re-parses to itself.
        assert_eq!(IndustrySector::EnergyStorage.as_str(), "energy_storage");
        assert_eq!(IndustrySector::Battery.as_str(), "battery");
        assert_eq!(
            IndustrySector::from_str(IndustrySector::EnergyStorage.as_str()),
            IndustrySector::EnergyStorage
        );
        assert_eq!(
            IndustrySector::from_str(IndustrySector::Battery.as_str()),
            IndustrySector::Battery
        );
    }

    #[test]
    fn test_confidence_level_from_f64() {
        assert_eq!(ConfidenceLevel::from_f64(0.9), ConfidenceLevel::High);
        assert_eq!(ConfidenceLevel::from_f64(0.7), ConfidenceLevel::Medium);
        assert_eq!(ConfidenceLevel::from_f64(0.3), ConfidenceLevel::Low);
        assert_eq!(ConfidenceLevel::from_f64(0.0), ConfidenceLevel::Unknown);
    }

    #[test]
    fn test_severity_from_cvss() {
        assert_eq!(SeverityLevel::from_cvss(9.5), SeverityLevel::Critical);
        assert_eq!(SeverityLevel::from_cvss(7.5), SeverityLevel::High);
        assert_eq!(SeverityLevel::from_cvss(5.0), SeverityLevel::Medium);
        assert_eq!(SeverityLevel::from_cvss(2.0), SeverityLevel::Low);
        assert_eq!(SeverityLevel::from_cvss(0.0), SeverityLevel::Info);
    }

    #[test]
    fn test_risk_score_bounds() {
        let rs = RiskScore::new(1.5, ConfidenceLevel::High);
        assert_eq!(rs.score, 1.0);

        let rs = RiskScore::new(-0.5, ConfidenceLevel::High);
        assert_eq!(rs.score, 0.0);
    }

    #[test]
    fn test_time_window_creation() {
        let window = TimeWindow::days_back(30);
        let duration = window.end - window.start;
        assert!(duration.num_days() >= 29 && duration.num_days() <= 31);
    }

    #[test]
    fn test_pagination_defaults() {
        let params = PaginationParams::default_page();
        assert_eq!(params.offset, 0);
        assert_eq!(params.limit, 100);

        let params = PaginationParams::new(0, 2000);
        assert_eq!(params.limit, 1000); // Capped at 1000
    }
}
