//! Data models for the geopolitical intelligence module

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Geographic region enum
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Region {
    NorthAmerica,
    SouthAmerica,
    Europe,
    Africa,
    MiddleEast,
    Asia,
    Oceania,
    CentralAsia,
    SoutheastAsia,
}

impl Region {
    /// Get ISO region code
    pub fn code(&self) -> &'static str {
        match self {
            Region::NorthAmerica => "NA",
            Region::SouthAmerica => "SA",
            Region::Europe => "EU",
            Region::Africa => "AF",
            Region::MiddleEast => "ME",
            Region::Asia => "AS",
            Region::Oceania => "OC",
            Region::CentralAsia => "CA",
            Region::SoutheastAsia => "SEA",
        }
    }
}

/// Country code (ISO 3166-1 alpha-2)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct CountryCode(pub String);

impl CountryCode {
    /// Create a new country code
    pub fn new(code: &str) -> Self {
        Self(code.to_uppercase())
    }

    /// Validate ISO 3166-1 alpha-2 code
    pub fn is_valid(&self) -> bool {
        self.0.len() == 2 && self.0.chars().all(|c| c.is_ascii_alphabetic())
    }
}

impl std::fmt::Display for CountryCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<&str> for CountryCode {
    fn from(s: &str) -> Self {
        CountryCode::new(s)
    }
}

impl From<String> for CountryCode {
    fn from(s: String) -> Self {
        CountryCode::new(&s)
    }
}

/// Severity level for alerts
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Low,
    Medium,
    High,
    Critical,
}

impl Severity {
    /// Get numeric value for comparisons
    pub fn value(&self) -> u8 {
        match self {
            Severity::Low => 1,
            Severity::Medium => 2,
            Severity::High => 3,
            Severity::Critical => 4,
        }
    }

    /// Get alert color for UI
    pub fn color(&self) -> &'static str {
        match self {
            Severity::Low => "#22c55e",
            Severity::Medium => "#eab308",
            Severity::High => "#f97316",
            Severity::Critical => "#ef4444",
        }
    }
}

/// Source of intelligence
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum IntelligenceSource {
    Ofac,
    EuSanctions,
    UnSanctions,
    Wto,
    Imf,
    WorldBank,
    UN,
    RegionalBank,
    Government,
    Commercial,
    OpenSource,
}

impl IntelligenceSource {
    /// Get source name
    pub fn name(&self) -> &'static str {
        match self {
            IntelligenceSource::Ofac => "OFAC",
            IntelligenceSource::EuSanctions => "EU Sanctions",
            IntelligenceSource::UnSanctions => "UN Sanctions",
            IntelligenceSource::Wto => "WTO",
            IntelligenceSource::Imf => "IMF",
            IntelligenceSource::WorldBank => "World Bank",
            IntelligenceSource::UN => "United Nations",
            IntelligenceSource::RegionalBank => "Regional Development Bank",
            IntelligenceSource::Government => "Government",
            IntelligenceSource::Commercial => "Commercial",
            IntelligenceSource::OpenSource => "Open Source",
        }
    }
}

/// Alert type enumeration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum AlertType {
    SanctionsMatch,
    TradeRestriction,
    PoliticalInstability,
    RegulatoryChange,
    TariffChange,
    ConflictEscalation,
    SupplyChainRisk,
    ComplianceDeadline,
}

/// Intelligence alert
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntelligenceAlert {
    pub id: Uuid,
    pub alert_type: AlertType,
    pub severity: Severity,
    pub title: String,
    pub description: String,
    pub source: IntelligenceSource,
    pub countries: Vec<CountryCode>,
    pub affected_entities: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub metadata: serde_json::Value,
    pub is_read: bool,
    pub is_acknowledged: bool,
}

impl IntelligenceAlert {
    /// Create a new alert
    pub fn new(
        alert_type: AlertType,
        severity: Severity,
        title: String,
        description: String,
        source: IntelligenceSource,
        countries: Vec<CountryCode>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            alert_type,
            severity,
            title,
            description,
            source,
            countries,
            affected_entities: Vec::new(),
            created_at: now,
            updated_at: now,
            metadata: serde_json::Value::Null,
            is_read: false,
            is_acknowledged: false,
        }
    }

    /// Add affected entity
    pub fn add_entity(&mut self, entity: String) {
        self.affected_entities.push(entity);
    }

    /// Mark as read
    pub fn mark_read(&mut self) {
        self.is_read = true;
        self.updated_at = Utc::now();
    }

    /// Mark as acknowledged
    pub fn acknowledge(&mut self) {
        self.is_acknowledged = true;
        self.updated_at = Utc::now();
    }
}

/// Confidence level
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Confidence {
    Low,
    Medium,
    High,
    VeryHigh,
}

impl Confidence {
    /// Get percentage range
    pub fn percentage_range(&self) -> (u8, u8) {
        match self {
            Confidence::Low => (0, 40),
            Confidence::Medium => (41, 60),
            Confidence::High => (61, 80),
            Confidence::VeryHigh => (81, 100),
        }
    }

    /// Create from numeric value
    pub fn from_value(value: f32) -> Self {
        match value {
            v if v < 0.4 => Confidence::Low,
            v if v < 0.6 => Confidence::Medium,
            v if v < 0.8 => Confidence::High,
            _ => Confidence::VeryHigh,
        }
    }
}

/// Time series data point
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeSeriesPoint<T> {
    pub timestamp: DateTime<Utc>,
    pub value: T,
}

/// Trend direction
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum TrendDirection {
    Improving,
    Stable,
    Declining,
    Volatile,
}

impl TrendDirection {
    /// Calculate from series of values
    #[allow(clippy::disallowed_methods)]
    pub fn calculate(values: &[f32]) -> Self {
        if values.len() < 2 {
            return TrendDirection::Stable;
        }

        let first = values.first().unwrap();
        let last = values.last().unwrap();
        let diff = last - first;

        // Calculate volatility
        let variance = Self::calculate_variance(values);
        let mean = values.iter().sum::<f32>() / values.len() as f32;
        let volatility = if mean != 0.0 {
            (variance.sqrt() / mean).abs()
        } else {
            0.0
        };

        if volatility > 0.3 {
            TrendDirection::Volatile
        } else if diff > 0.1 {
            TrendDirection::Improving
        } else if diff < -0.1 {
            TrendDirection::Declining
        } else {
            TrendDirection::Stable
        }
    }

    fn calculate_variance(values: &[f32]) -> f32 {
        let mean = values.iter().sum::<f32>() / values.len() as f32;
        values.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / values.len() as f32
    }
}

/// Risk score with components
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskScore {
    pub overall: f32,
    pub economic: f32,
    pub political: f32,
    pub social: f32,
    pub environmental: f32,
    pub confidence: Confidence,
    pub trend: TrendDirection,
}

impl RiskScore {
    /// Create a new risk score
    pub fn new(
        overall: f32,
        economic: f32,
        political: f32,
        social: f32,
        environmental: f32,
    ) -> Self {
        Self {
            overall,
            economic,
            political,
            social,
            environmental,
            confidence: Confidence::from_value(overall),
            trend: TrendDirection::Stable,
        }
    }

    /// Create minimum risk score
    #[allow(dead_code)]
    pub fn minimum() -> Self {
        Self::new(0.0, 0.0, 0.0, 0.0, 0.0)
    }

    /// Create maximum risk score
    #[allow(dead_code)]
    pub fn maximum() -> Self {
        Self::new(1.0, 1.0, 1.0, 1.0, 1.0)
    }

    /// Check if risk is above threshold
    pub fn is_above_threshold(&self, threshold: f32) -> bool {
        self.overall >= threshold
    }

    /// Get risk level description
    pub fn risk_level(&self) -> &'static str {
        match self.overall {
            v if v < 0.2 => "Very Low",
            v if v < 0.4 => "Low",
            v if v < 0.6 => "Medium",
            v if v < 0.8 => "High",
            _ => "Very High",
        }
    }
}

/// Pagination parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaginationParams {
    pub page: u32,
    pub page_size: u32,
    pub total: Option<u64>,
}

impl Default for PaginationParams {
    fn default() -> Self {
        Self {
            page: 1,
            page_size: 20,
            total: None,
        }
    }
}

impl PaginationParams {
    /// Calculate offset
    pub fn offset(&self) -> u64 {
        ((self.page.saturating_sub(1)) * self.page_size) as u64
    }

    /// Calculate total pages
    pub fn total_pages(&self) -> Option<u32> {
        self.total.map(|t| ((t as f64) / (self.page_size as f64)).ceil() as u32)
    }

    /// Check if has next page
    pub fn has_next(&self) -> bool {
        self.total_pages()
            .map(|total| self.page < total)
            .unwrap_or(false)
    }
}

/// Filter parameters for queries
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FilterParams {
    pub countries: Option<Vec<String>>,
    pub regions: Option<Vec<String>>,
    pub severity: Option<Severity>,
    pub date_from: Option<DateTime<Utc>>,
    pub date_to: Option<DateTime<Utc>>,
    pub sources: Option<Vec<IntelligenceSource>>,
    pub keywords: Option<Vec<String>>,
}

/// Pagination response wrapper
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaginatedResponse<T> {
    pub data: Vec<T>,
    pub pagination: PaginationParams,
}

impl<T> PaginatedResponse<T> {
    /// Create a new paginated response
    pub fn new(data: Vec<T>, pagination: PaginationParams) -> Self {
        Self { data, pagination }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_country_code_validation() {
        let code = CountryCode::new("US");
        assert!(code.is_valid());

        let code = CountryCode::new("us");
        assert!(code.is_valid());

        let code = CountryCode::new("USA");
        assert!(!code.is_valid());

        let code = CountryCode::new("U1");
        assert!(!code.is_valid());
    }

    #[test]
    fn test_severity_ordering() {
        assert!(Severity::Critical > Severity::High);
        assert!(Severity::High > Severity::Medium);
        assert!(Severity::Medium > Severity::Low);
    }

    #[test]
    fn test_confidence_from_value() {
        assert_eq!(Confidence::from_value(0.2), Confidence::Low);
        assert_eq!(Confidence::from_value(0.5), Confidence::Medium);
        assert_eq!(Confidence::from_value(0.7), Confidence::High);
        assert_eq!(Confidence::from_value(0.9), Confidence::VeryHigh);
    }

    #[test]
    fn test_risk_score() {
        let score = RiskScore::new(0.5, 0.3, 0.6, 0.4, 0.2);
        assert_eq!(score.risk_level(), "Medium");
        assert!(score.is_above_threshold(0.4));
    }

    #[test]
    fn test_pagination() {
        let params = PaginationParams {
            page: 2,
            page_size: 20,
            total: Some(100),
        };
        assert_eq!(params.offset(), 20);
        assert_eq!(params.total_pages(), Some(5));
        assert!(params.has_next());
    }

    #[test]
    fn test_trend_direction() {
        let values = vec![0.5, 0.55, 0.6, 0.65];
        assert_eq!(TrendDirection::calculate(&values), TrendDirection::Improving);

        let values = vec![0.5, 0.45, 0.4, 0.35];
        assert_eq!(TrendDirection::calculate(&values), TrendDirection::Declining);

        let values = vec![0.5, 0.1, 0.9, 0.2];
        assert_eq!(TrendDirection::calculate(&values), TrendDirection::Volatile);
    }

    #[test]
    fn test_alert_creation() {
        let mut alert = IntelligenceAlert::new(
            AlertType::SanctionsMatch,
            Severity::High,
            "Test Alert".to_string(),
            "Test description".to_string(),
            IntelligenceSource::Ofac,
            vec![CountryCode::new("RU")],
        );

        assert!(!alert.is_read);
        alert.mark_read();
        assert!(alert.is_read);

        alert.add_entity("Entity1".to_string());
        assert_eq!(alert.affected_entities.len(), 1);
    }
}

/// Configuration for the geopolitical intelligence module
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeopoliticalConfig {
    /// API endpoints for sanctions lists
    pub ofac_api_url: Option<String>,
    pub eu_sanctions_url: Option<String>,
    pub un_sanctions_url: Option<String>,
    
    /// Trade data sources
    pub trade_data_url: Option<String>,
    
    /// Political risk settings
    pub conflict_threshold: f32,
    pub stability_update_interval_hours: u32,
    
    /// Regulatory tracking
    pub regulation_sources: Vec<String>,
    
    /// HTTP client settings
    pub http_timeout_secs: u64,
    pub max_retries: u32,
}

impl Default for GeopoliticalConfig {
    fn default() -> Self {
        Self {
            ofac_api_url: None,
            eu_sanctions_url: None,
            un_sanctions_url: None,
            trade_data_url: None,
            conflict_threshold: 0.7,
            stability_update_interval_hours: 24,
            regulation_sources: Vec::new(),
            http_timeout_secs: 30,
            max_retries: 3,
        }
    }
}
