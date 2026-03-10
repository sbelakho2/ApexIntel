//! Filters — query parameter parsing and validation for API endpoints.
//!
//! Pure filter/predicate types covering region, entity type, severity,
//! date ranges, search text, and domain-specific filter dimensions.

use chrono::{NaiveDate, Utc};
use serde::{Deserialize, Serialize};

// ────────────────────────────────────────────
// Enums
// ────────────────────────────────────────────

/// Region codes used across the platform.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum RegionFilter {
    TN, // Tunisia
    MA, // Morocco
    IL, // Israel
    CN, // China
    EA, // East Africa
    EU, // Europe
    US, // United States
}

impl RegionFilter {
    pub fn from_code(code: &str) -> Option<Self> {
        match code.to_uppercase().as_str() {
            "TN" => Some(Self::TN),
            "MA" => Some(Self::MA),
            "IL" => Some(Self::IL),
            "CN" => Some(Self::CN),
            "EA" => Some(Self::EA),
            "EU" => Some(Self::EU),
            "US" => Some(Self::US),
            _ => None,
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            Self::TN => "TN",
            Self::MA => "MA",
            Self::IL => "IL",
            Self::CN => "CN",
            Self::EA => "EA",
            Self::EU => "EU",
            Self::US => "US",
        }
    }

    pub fn all() -> Vec<Self> {
        vec![
            Self::TN,
            Self::MA,
            Self::IL,
            Self::CN,
            Self::EA,
            Self::EU,
            Self::US,
        ]
    }
}

/// Warning severity levels.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum SeverityFilter {
    Low,
    Medium,
    High,
    Critical,
}

impl SeverityFilter {
    pub fn from_str_loose(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "low" | "l" => Some(Self::Low),
            "medium" | "med" | "m" => Some(Self::Medium),
            "high" | "h" => Some(Self::High),
            "critical" | "crit" | "c" => Some(Self::Critical),
            _ => None,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Critical => "critical",
        }
    }

    pub fn numeric(&self) -> u8 {
        match self {
            Self::Low => 1,
            Self::Medium => 2,
            Self::High => 3,
            Self::Critical => 4,
        }
    }
}

/// Entity type classification for filtering search/list endpoints.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum EntityTypeFilter {
    Company,
    Person,
    Product,
    Facility,
    Event,
    Document,
}

impl EntityTypeFilter {
    pub fn from_str_loose(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "company" | "companies" | "org" => Some(Self::Company),
            "person" | "persons" | "people" | "poi" => Some(Self::Person),
            "product" | "products" | "component" => Some(Self::Product),
            "facility" | "site" | "factory" => Some(Self::Facility),
            "event" | "events" => Some(Self::Event),
            "document" | "doc" | "docs" => Some(Self::Document),
            _ => None,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Company => "company",
            Self::Person => "person",
            Self::Product => "product",
            Self::Facility => "facility",
            Self::Event => "event",
            Self::Document => "document",
        }
    }
}

/// Recipe status for filtering recipes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum RecipeStatusFilter {
    Staging,
    Production,
    Deprecated,
}

impl RecipeStatusFilter {
    pub fn from_str_loose(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "staging" | "staged" => Some(Self::Staging),
            "production" | "prod" | "active" => Some(Self::Production),
            "deprecated" | "dep" | "retired" => Some(Self::Deprecated),
            _ => None,
        }
    }
}

/// Warning type classification.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum WarningTypeFilter {
    SupplyChain,
    Competitive,
    Security,
    Regulatory,
    Personnel,
    Market,
}

impl WarningTypeFilter {
    pub fn from_str_loose(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "supply_chain" | "supply-chain" | "supplychain" | "supply" => Some(Self::SupplyChain),
            "competitive" | "competition" | "competitor" => Some(Self::Competitive),
            "security" | "cyber" | "sec" => Some(Self::Security),
            "regulatory" | "regulation" | "reg" | "compliance" => Some(Self::Regulatory),
            "personnel" | "hr" | "people" | "talent" => Some(Self::Personnel),
            "market" | "mkt" | "commodity" => Some(Self::Market),
            _ => None,
        }
    }
}

// ────────────────────────────────────────────
// Date range
// ────────────────────────────────────────────

/// A date range (inclusive) for filtering.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DateRange {
    pub from: Option<NaiveDate>,
    pub to: Option<NaiveDate>,
}

impl DateRange {
    pub fn new(from: Option<NaiveDate>, to: Option<NaiveDate>) -> Self {
        Self { from, to }
    }

    /// Check if a date falls within the range.
    pub fn contains(&self, date: &NaiveDate) -> bool {
        if let Some(from) = &self.from {
            if date < from {
                return false;
            }
        }
        if let Some(to) = &self.to {
            if date > to {
                return false;
            }
        }
        true
    }

    /// Validate that from <= to if both are present.
    pub fn is_valid(&self) -> bool {
        match (&self.from, &self.to) {
            (Some(f), Some(t)) => f <= t,
            _ => true,
        }
    }

    /// Last N days range.
    pub fn last_n_days(n: u32) -> Self {
        let today = Utc::now().date_naive();
        let from = today - chrono::Duration::days(n as i64);
        Self {
            from: Some(from),
            to: Some(today),
        }
    }
}

// ────────────────────────────────────────────
// Composite filters
// ────────────────────────────────────────────

/// Combined filter for the warnings list endpoint.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WarningFilters {
    pub regions: Vec<RegionFilter>,
    pub severities: Vec<SeverityFilter>,
    pub warning_types: Vec<WarningTypeFilter>,
    pub date_range: Option<DateRange>,
    pub search_text: Option<String>,
    pub acknowledged: Option<bool>,
}

/// Combined filter for the companies list endpoint.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompanyFilters {
    pub regions: Vec<RegionFilter>,
    pub search_text: Option<String>,
    pub is_competitor: Option<bool>,
}

/// Combined filter for the persons/POI list endpoint.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonFilters {
    pub regions: Vec<RegionFilter>,
    pub search_text: Option<String>,
    pub min_priority: Option<f64>,
    pub roles: Vec<String>,
}

/// Combined filter for the recipes list endpoint.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecipeFilters {
    pub status: Option<RecipeStatusFilter>,
    pub search_text: Option<String>,
    pub min_precision: Option<f64>,
    pub region: Option<RegionFilter>,
}

/// Combined filter for the insights endpoint.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InsightFilters {
    pub regions: Vec<RegionFilter>,
    pub date_range: Option<DateRange>,
    pub search_text: Option<String>,
}

/// Global search filter.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchFilter {
    pub query: String,
    pub entity_types: Vec<EntityTypeFilter>,
    pub regions: Vec<RegionFilter>,
    pub date_range: Option<DateRange>,
}

// ────────────────────────────────────────────
// Parser helpers
// ────────────────────────────────────────────

/// Parse a comma-separated region string like "TN,MA,EU".
pub fn parse_regions(input: &str) -> Vec<RegionFilter> {
    input
        .split(',')
        .filter_map(|s| RegionFilter::from_code(s.trim()))
        .collect()
}

/// Parse a comma-separated severity string like "high,critical".
pub fn parse_severities(input: &str) -> Vec<SeverityFilter> {
    input
        .split(',')
        .filter_map(|s| SeverityFilter::from_str_loose(s.trim()))
        .collect()
}

/// Parse a comma-separated entity type string.
pub fn parse_entity_types(input: &str) -> Vec<EntityTypeFilter> {
    input
        .split(',')
        .filter_map(|s| EntityTypeFilter::from_str_loose(s.trim()))
        .collect()
}

/// Parse a date string in YYYY-MM-DD format.
pub fn parse_date(input: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(input.trim(), "%Y-%m-%d").ok()
}

/// Parse "YYYY-MM-DD..YYYY-MM-DD" into a DateRange.
pub fn parse_date_range(input: &str) -> Option<DateRange> {
    let parts: Vec<&str> = input.split("..").collect();
    match parts.len() {
        1 => {
            let d = parse_date(parts[0])?;
            Some(DateRange::new(Some(d), Some(d)))
        }
        2 => {
            let from = if parts[0].is_empty() {
                None
            } else {
                Some(parse_date(parts[0])?)
            };
            let to = if parts[1].is_empty() {
                None
            } else {
                Some(parse_date(parts[1])?)
            };
            let range = DateRange::new(from, to);
            if range.is_valid() {
                Some(range)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Sanitize search text: trim, collapse whitespace, limit length.
pub fn sanitize_search_text(input: &str, max_len: usize) -> Option<String> {
    let trimmed: String = input.split_whitespace().collect::<Vec<&str>>().join(" ");
    if trimmed.is_empty() {
        None
    } else if trimmed.len() > max_len {
        // Use char-safe truncation to avoid panicking on multi-byte UTF-8.
        Some(trimmed.chars().take(max_len).collect())
    } else {
        Some(trimmed)
    }
}

/// Validate search text: enforce max length and reject empty input.
pub fn validate_search_text(input: &str, max_len: usize) -> Result<Option<String>, String> {
    let trimmed: String = input.split_whitespace().collect::<Vec<&str>>().join(" ");
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.chars().count() > max_len {
        return Err(format!("search text too long (max {} chars)", max_len));
    }
    Ok(Some(trimmed))
}

/// Check if a minimum value filter is in a valid range.
pub fn validate_min_value(val: Option<f64>, min: f64, max: f64) -> Option<f64> {
    val.and_then(|v| if v >= min && v <= max { Some(v) } else { None })
}

// ────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── RegionFilter ──

    #[test]
    fn test_region_from_code() {
        assert_eq!(RegionFilter::from_code("TN"), Some(RegionFilter::TN));
        assert_eq!(RegionFilter::from_code("tn"), Some(RegionFilter::TN));
        assert_eq!(RegionFilter::from_code("ma"), Some(RegionFilter::MA));
        assert_eq!(RegionFilter::from_code("XX"), None);
    }

    #[test]
    fn test_region_code_roundtrip() {
        for region in RegionFilter::all() {
            let code = region.code();
            let back = RegionFilter::from_code(code).unwrap();
            assert_eq!(back, region);
        }
    }

    #[test]
    fn test_region_all() {
        assert_eq!(RegionFilter::all().len(), 7);
    }

    // ── SeverityFilter ──

    #[test]
    fn test_severity_from_str() {
        assert_eq!(
            SeverityFilter::from_str_loose("high"),
            Some(SeverityFilter::High)
        );
        assert_eq!(
            SeverityFilter::from_str_loose("crit"),
            Some(SeverityFilter::Critical)
        );
        assert_eq!(
            SeverityFilter::from_str_loose("med"),
            Some(SeverityFilter::Medium)
        );
        assert_eq!(SeverityFilter::from_str_loose("unknown"), None);
    }

    #[test]
    fn test_severity_ordering() {
        assert!(SeverityFilter::Low < SeverityFilter::Medium);
        assert!(SeverityFilter::Medium < SeverityFilter::High);
        assert!(SeverityFilter::High < SeverityFilter::Critical);
    }

    #[test]
    fn test_severity_numeric() {
        assert_eq!(SeverityFilter::Low.numeric(), 1);
        assert_eq!(SeverityFilter::Critical.numeric(), 4);
    }

    // ── EntityTypeFilter ──

    #[test]
    fn test_entity_type_from_str() {
        assert_eq!(
            EntityTypeFilter::from_str_loose("company"),
            Some(EntityTypeFilter::Company)
        );
        assert_eq!(
            EntityTypeFilter::from_str_loose("poi"),
            Some(EntityTypeFilter::Person)
        );
        assert_eq!(
            EntityTypeFilter::from_str_loose("factory"),
            Some(EntityTypeFilter::Facility)
        );
        assert_eq!(EntityTypeFilter::from_str_loose("xyz"), None);
    }

    // ── RecipeStatusFilter ──

    #[test]
    fn test_recipe_status_from_str() {
        assert_eq!(
            RecipeStatusFilter::from_str_loose("staging"),
            Some(RecipeStatusFilter::Staging)
        );
        assert_eq!(
            RecipeStatusFilter::from_str_loose("prod"),
            Some(RecipeStatusFilter::Production)
        );
        assert_eq!(
            RecipeStatusFilter::from_str_loose("deprecated"),
            Some(RecipeStatusFilter::Deprecated)
        );
    }

    // ── WarningTypeFilter ──

    #[test]
    fn test_warning_type_from_str() {
        assert_eq!(
            WarningTypeFilter::from_str_loose("supply_chain"),
            Some(WarningTypeFilter::SupplyChain)
        );
        assert_eq!(
            WarningTypeFilter::from_str_loose("cyber"),
            Some(WarningTypeFilter::Security)
        );
        assert_eq!(
            WarningTypeFilter::from_str_loose("talent"),
            Some(WarningTypeFilter::Personnel)
        );
    }

    // ── DateRange ──

    #[test]
    fn test_date_range_contains() {
        let range = DateRange::new(
            Some(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap()),
            Some(NaiveDate::from_ymd_opt(2024, 12, 31).unwrap()),
        );
        assert!(range.contains(&NaiveDate::from_ymd_opt(2024, 6, 15).unwrap()));
        assert!(range.contains(&NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())); // inclusive
        assert!(range.contains(&NaiveDate::from_ymd_opt(2024, 12, 31).unwrap())); // inclusive
        assert!(!range.contains(&NaiveDate::from_ymd_opt(2023, 12, 31).unwrap()));
        assert!(!range.contains(&NaiveDate::from_ymd_opt(2025, 1, 1).unwrap()));
    }

    #[test]
    fn test_date_range_open_ended() {
        let open_from = DateRange::new(None, Some(NaiveDate::from_ymd_opt(2024, 6, 1).unwrap()));
        assert!(open_from.contains(&NaiveDate::from_ymd_opt(2020, 1, 1).unwrap()));
        assert!(!open_from.contains(&NaiveDate::from_ymd_opt(2024, 7, 1).unwrap()));

        let open_to = DateRange::new(Some(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap()), None);
        assert!(open_to.contains(&NaiveDate::from_ymd_opt(2030, 1, 1).unwrap()));
        assert!(!open_to.contains(&NaiveDate::from_ymd_opt(2023, 12, 31).unwrap()));
    }

    #[test]
    fn test_date_range_is_valid() {
        let valid = DateRange::new(
            Some(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap()),
            Some(NaiveDate::from_ymd_opt(2024, 12, 31).unwrap()),
        );
        assert!(valid.is_valid());

        let invalid = DateRange::new(
            Some(NaiveDate::from_ymd_opt(2024, 12, 31).unwrap()),
            Some(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap()),
        );
        assert!(!invalid.is_valid());

        let open = DateRange::new(None, None);
        assert!(open.is_valid());
    }

    #[test]
    fn test_last_n_days() {
        let range = DateRange::last_n_days(7);
        assert!(range.from.is_some());
        assert!(range.to.is_some());
        let diff = range.to.unwrap() - range.from.unwrap();
        assert_eq!(diff.num_days(), 7);
    }

    // ── Parsers ──

    #[test]
    fn test_parse_regions() {
        let regions = parse_regions("TN, MA, EU");
        assert_eq!(regions.len(), 3);
        assert_eq!(regions[0], RegionFilter::TN);
        assert_eq!(regions[1], RegionFilter::MA);
        assert_eq!(regions[2], RegionFilter::EU);
    }

    #[test]
    fn test_parse_regions_with_invalid() {
        let regions = parse_regions("TN,XX,MA");
        assert_eq!(regions.len(), 2); // XX skipped
    }

    #[test]
    fn test_parse_severities() {
        let sevs = parse_severities("high,critical");
        assert_eq!(sevs.len(), 2);
        assert_eq!(sevs[0], SeverityFilter::High);
        assert_eq!(sevs[1], SeverityFilter::Critical);
    }

    #[test]
    fn test_parse_entity_types() {
        let types = parse_entity_types("company,person,product");
        assert_eq!(types.len(), 3);
    }

    #[test]
    fn test_parse_date() {
        let d = parse_date("2024-06-15");
        assert_eq!(d, Some(NaiveDate::from_ymd_opt(2024, 6, 15).unwrap()));
        assert_eq!(parse_date("not-a-date"), None);
    }

    #[test]
    fn test_parse_date_range_full() {
        let range = parse_date_range("2024-01-01..2024-12-31").unwrap();
        assert_eq!(
            range.from,
            Some(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
        );
        assert_eq!(
            range.to,
            Some(NaiveDate::from_ymd_opt(2024, 12, 31).unwrap())
        );
    }

    #[test]
    fn test_parse_date_range_open_end() {
        let range = parse_date_range("2024-01-01..").unwrap();
        assert!(range.from.is_some());
        assert!(range.to.is_none());
    }

    #[test]
    fn test_parse_date_range_single_date() {
        let range = parse_date_range("2024-06-15").unwrap();
        assert_eq!(range.from, range.to);
    }

    #[test]
    fn test_parse_date_range_invalid() {
        // from > to
        let result = parse_date_range("2024-12-31..2024-01-01");
        assert!(result.is_none());
    }

    // ── Sanitize ──

    #[test]
    fn test_sanitize_search_text() {
        assert_eq!(
            sanitize_search_text("  hello   world  ", 100),
            Some("hello world".to_string())
        );
        assert_eq!(sanitize_search_text("   ", 100), None);
        assert_eq!(sanitize_search_text("abcdef", 3), Some("abc".to_string()));
    }

    #[test]
    fn test_validate_search_text() {
        // "hello world" is 11 chars; max_len must be >= 11 for it to pass
        assert_eq!(
            validate_search_text("  hello world ", 11).unwrap(),
            Some("hello world".to_string())
        );
        // Shorter input with smaller max
        assert_eq!(
            validate_search_text("  hello  ", 10).unwrap(),
            Some("hello".to_string())
        );
        assert_eq!(validate_search_text("   ", 10).unwrap(), None);
        assert!(validate_search_text("x".repeat(11).as_str(), 10).is_err());
    }

    #[test]
    fn test_validate_min_value() {
        assert_eq!(validate_min_value(Some(0.5), 0.0, 1.0), Some(0.5));
        assert_eq!(validate_min_value(Some(1.5), 0.0, 1.0), None);
        assert_eq!(validate_min_value(None, 0.0, 1.0), None);
    }

    // ── Composite filters ──

    #[test]
    fn test_warning_filters_default() {
        let f = WarningFilters::default();
        assert!(f.regions.is_empty());
        assert!(f.severities.is_empty());
        assert!(f.search_text.is_none());
        assert!(f.acknowledged.is_none());
    }

    #[test]
    fn test_warning_filters_serialization() {
        let f = WarningFilters {
            regions: vec![RegionFilter::TN, RegionFilter::MA],
            severities: vec![SeverityFilter::High],
            warning_types: vec![WarningTypeFilter::Security],
            date_range: None,
            search_text: Some("starz".to_string()),
            acknowledged: Some(false),
        };
        let json = serde_json::to_string(&f).unwrap();
        let back: WarningFilters = serde_json::from_str(&json).unwrap();
        assert_eq!(back.regions.len(), 2);
        assert_eq!(back.search_text, Some("starz".to_string()));
    }

    #[test]
    fn test_company_filters_serialization() {
        let f = CompanyFilters {
            regions: vec![RegionFilter::CN],
            search_text: Some("foxconn".to_string()),
            is_competitor: Some(true),
        };
        let json = serde_json::to_string(&f).unwrap();
        assert!(json.contains("foxconn"));
    }

    #[test]
    fn test_search_filter_serialization() {
        let f = SearchFilter {
            query: "pcb assembly".to_string(),
            entity_types: vec![EntityTypeFilter::Company, EntityTypeFilter::Product],
            regions: vec![RegionFilter::TN],
            date_range: None,
        };
        let json = serde_json::to_string(&f).unwrap();
        assert!(json.contains("pcb assembly"));
    }
}
