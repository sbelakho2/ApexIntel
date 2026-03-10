use std::sync::LazyLock;

use chrono::{DateTime, Utc};
use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};

use crate::normalizer;
use apex_core::validation::normalize_url;

static RE_PRICE: LazyLock<Regex> = LazyLock::new(|| {
    RegexBuilder::new(r"(?i)(?:[$€£¥])\s*([\d,.]+)|(?:([\d,.]+)\s*(?:USD|EUR|GBP|CNY|JPY))")
        .size_limit(200_000)
        .dfa_size_limit(200_000)
        .build()
        .unwrap()
});

static RE_DATE: LazyLock<Regex> = LazyLock::new(|| {
    // B103: Anchored with \b word boundaries to avoid partial matches
    RegexBuilder::new(r"\b\d{4}[-/]\d{2}[-/]\d{2}\b")
        .size_limit(200_000)
        .dfa_size_limit(200_000)
        .build()
        .unwrap()
});

static RE_TABULAR: LazyLock<Regex> = LazyLock::new(|| {
    // B103: Anchored with \b to prevent partial commodity name matches
    RegexBuilder::new(
        r"(?i)\b(copper|gold|silver|tin|palladium|aluminum|steel|solder|silicon|neon|fr[-]?4)\b[\s|]+\$?([\d,.]+)\s*(USD|EUR|GBP|CNY|JPY)?\s*(?:/\s*(\w+))?",
    )
    .size_limit(200_000)
    .dfa_size_limit(200_000)
    .build()
    .unwrap()
});

/// A commodity price observation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommodityPrice {
    pub commodity: String,
    pub price: f64,
    pub currency: String,
    pub unit: String,
    pub date: Option<String>,
    pub source: String,
    pub url: String,
    pub extracted_at: DateTime<Utc>,
}

/// Known EMS-relevant commodities.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommodityType {
    Copper,
    Gold,
    Silver,
    Tin,
    Palladium,
    Aluminum,
    Steel,
    Epoxy,
    Solder,
    Silicon,
    Neon,
    FR4,
    Other(String),
}

impl CommodityType {
    pub fn from_name(name: &str) -> Self {
        let lower = name.to_lowercase();
        if lower.contains("copper") || lower == "cu" {
            return CommodityType::Copper;
        }
        if lower.contains("gold") || lower == "au" {
            return CommodityType::Gold;
        }
        if lower.contains("silver") || lower == "ag" {
            return CommodityType::Silver;
        }
        if lower.contains("tin") || lower == "sn" {
            return CommodityType::Tin;
        }
        if lower.contains("palladium") || lower == "pd" {
            return CommodityType::Palladium;
        }
        if lower.contains("alumi") || lower == "al" {
            return CommodityType::Aluminum;
        }
        if lower.contains("steel") {
            return CommodityType::Steel;
        }
        if lower.contains("epoxy") {
            return CommodityType::Epoxy;
        }
        if lower.contains("solder") {
            return CommodityType::Solder;
        }
        if lower.contains("silicon") || lower == "si" {
            return CommodityType::Silicon;
        }
        if lower.contains("neon") || lower == "ne" {
            return CommodityType::Neon;
        }
        if lower.contains("fr4") || lower.contains("fr-4") {
            return CommodityType::FR4;
        }
        CommodityType::Other(name.to_string())
    }

    pub fn display_name(&self) -> &str {
        match self {
            CommodityType::Copper => "Copper",
            CommodityType::Gold => "Gold",
            CommodityType::Silver => "Silver",
            CommodityType::Tin => "Tin",
            CommodityType::Palladium => "Palladium",
            CommodityType::Aluminum => "Aluminum",
            CommodityType::Steel => "Steel",
            CommodityType::Epoxy => "Epoxy",
            CommodityType::Solder => "Solder",
            CommodityType::Silicon => "Silicon",
            CommodityType::Neon => "Neon",
            CommodityType::FR4 => "FR-4",
            CommodityType::Other(s) => s.as_str(),
        }
    }
}

/// Extract commodity prices from page text (price feeds, commodity trackers).
pub fn extract_commodity_prices(body_text: &str, source: &str, url: &str) -> Vec<CommodityPrice> {
    let normalized_url = normalize_url(url).unwrap_or_else(|| url.to_string());
    let mut prices = Vec::new();
    let mut seen = std::collections::HashSet::new();

    // Pattern: "Commodity: $X.XX /unit" or "Commodity ... X.XX USD/unit"
    let commodities = [
        "copper",
        "gold",
        "silver",
        "tin",
        "palladium",
        "aluminum",
        "steel",
        "epoxy",
        "solder",
        "silicon",
        "neon",
        "fr4",
        "fr-4",
    ];

    for commodity in &commodities {
        if let Some(price_info) = find_price_for_commodity(body_text, commodity) {
            let key = format!("{}:{}", commodity, price_info.0);
            if !seen.contains(&key) {
                seen.insert(key);
                prices.push(CommodityPrice {
                    commodity: commodity.to_string(),
                    price: price_info.0,
                    currency: price_info.1,
                    unit: price_info.2,
                    date: extract_price_date(body_text),
                    source: source.to_string(),
                    url: normalized_url.clone(),
                    extracted_at: Utc::now(),
                });
            }
        }
    }

    // Also try generic tabular extraction
    let tabular = extract_tabular_prices(body_text, source, &normalized_url);
    for p in tabular {
        let key = format!("{}:{}", p.commodity, p.price);
        if !seen.contains(&key) {
            seen.insert(key);
            prices.push(p);
        }
    }

    prices
}

fn find_price_for_commodity(text: &str, commodity: &str) -> Option<(f64, String, String)> {
    // Look for the commodity name followed by a price within 200 chars
    let lower = text.to_lowercase();
    // Use word-boundary search to avoid matching "tin" inside "heating",
    // "testing", etc.  Build a small regex per commodity (they are static
    // strings so the set is tiny and only compiled once per call).
    let boundary_re = regex::Regex::new(&format!(r"\b{}\b", regex::escape(commodity))).ok()?;
    let m = boundary_re.find(&lower)?;
    let idx = m.start();

    // Work entirely in the lowered string to avoid byte-offset mismatch
    // between lowercase and original text (multi-byte characters can change length).
    let window_end = lower.ceil_char_boundary((idx + 200).min(lower.len()));
    let window = &lower[idx..window_end];

    // Extract price with currency
    let caps = RE_PRICE.captures(window)?;

    let price_str = caps
        .get(1)
        .or(caps.get(2))
        .map(|m| m.as_str())
        .unwrap_or("0");

    let price = normalizer::parse_number(price_str)?;

    // Detect currency (window is lowercased, so check lowercase codes too).
    // Check explicit currency codes before the ambiguous Yen symbol.
    let currency = if window.contains('$') || window.contains("usd") {
        "USD".to_string()
    } else if window.contains('€') || window.contains("eur") {
        "EUR".to_string()
    } else if window.contains('£') || window.contains("gbp") {
        "GBP".to_string()
    } else if window.contains("jpy") {
        "JPY".to_string()
    } else if window.contains('¥') || window.contains("cny") {
        "CNY".to_string()
    } else {
        "USD".to_string()
    };

    // Detect unit
    let unit = detect_unit(window);

    Some((price, currency, unit))
}

fn detect_unit(text: &str) -> String {
    let lower = text.to_lowercase();
    if lower.contains("/lb") || lower.contains("per lb") || lower.contains("per pound") {
        "lb".to_string()
    } else if lower.contains("/kg") || lower.contains("per kg") {
        "kg".to_string()
    } else if lower.contains("/oz") || lower.contains("per ounce") || lower.contains("per troy oz")
    {
        "oz".to_string()
    } else if lower.contains("/mt")
        || lower.contains("per metric ton")
        || lower.contains("per tonne")
    {
        "mt".to_string()
    } else if lower.contains("/ton") {
        "ton".to_string()
    } else if lower.contains("/sheet") || lower.contains("per sheet") {
        "sheet".to_string()
    } else {
        "unit".to_string()
    }
}

fn extract_price_date(text: &str) -> Option<String> {
    RE_DATE
        .find(text)
        .map(|m| m.as_str().to_string())
        .filter(|raw| normalizer::is_valid_date(raw))
}

fn extract_tabular_prices(text: &str, source: &str, url: &str) -> Vec<CommodityPrice> {
    let mut prices = Vec::new();

    // Pattern for table rows: "Name | Price | Currency | Unit"
    for caps in RE_TABULAR.captures_iter(text) {
        let commodity = caps.get(1).unwrap().as_str().to_lowercase();
        let price_str = caps.get(2).unwrap().as_str();
        let currency = caps
            .get(3)
            .map(|m| m.as_str().to_uppercase())
            .unwrap_or_else(|| "USD".to_string());
        let unit = caps
            .get(4)
            .map(|m| m.as_str().to_string())
            .unwrap_or_else(|| "unit".to_string());

        if let Some(price) = normalizer::parse_number(price_str) {
            prices.push(CommodityPrice {
                commodity,
                price,
                currency,
                unit,
                date: extract_price_date(text),
                source: source.to_string(),
                url: url.to_string(),
                extracted_at: Utc::now(),
            });
        }
    }

    prices
}

/// Compute price change percentage between two observations.
pub fn price_change_pct(old_price: f64, new_price: f64) -> f64 {
    if old_price == 0.0 {
        return 0.0;
    }
    ((new_price - old_price) / old_price) * 100.0
}

/// Check if a commodity is critical for PCB/EMS manufacturing.
pub fn is_critical_ems_commodity(commodity_type: &CommodityType) -> bool {
    matches!(
        commodity_type,
        CommodityType::Copper
            | CommodityType::Tin
            | CommodityType::Gold
            | CommodityType::Silver
            | CommodityType::Palladium
            | CommodityType::Solder
            | CommodityType::FR4
            | CommodityType::Epoxy
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_commodity_type_from_name() {
        assert_eq!(CommodityType::from_name("copper"), CommodityType::Copper);
        assert_eq!(CommodityType::from_name("Gold"), CommodityType::Gold);
        assert_eq!(CommodityType::from_name("FR-4"), CommodityType::FR4);
        assert_eq!(CommodityType::from_name("Cu"), CommodityType::Copper);
        assert_eq!(
            CommodityType::from_name("unknown metal"),
            CommodityType::Other("unknown metal".to_string())
        );
    }

    #[test]
    fn test_commodity_display_names() {
        assert_eq!(CommodityType::Copper.display_name(), "Copper");
        assert_eq!(CommodityType::FR4.display_name(), "FR-4");
        assert_eq!(
            CommodityType::Other("Zinc".to_string()).display_name(),
            "Zinc"
        );
    }

    #[test]
    fn test_extract_commodity_prices() {
        let text =
            "Market update 2025-01-15: copper $4.25/lb, gold $2025.50/oz, tin $28500 USD/mt.";
        let prices = extract_commodity_prices(text, "MetalPrices.com", "https://example.com");

        assert!(prices.len() >= 2);
        let copper = prices.iter().find(|p| p.commodity == "copper");
        assert!(copper.is_some());
        let cu = copper.unwrap();
        assert!(cu.price > 0.0);
        assert_eq!(cu.currency, "USD");
    }

    #[test]
    fn test_find_price_for_commodity() {
        let text = "Copper spot price: $4.25 per lb today.";
        let result = find_price_for_commodity(text, "copper");
        assert!(result.is_some());
        let (price, currency, unit) = result.unwrap();
        assert!((price - 4.25).abs() < 0.01);
        assert_eq!(currency, "USD");
        assert_eq!(unit, "lb");
    }

    #[test]
    fn test_detect_unit() {
        assert_eq!(detect_unit("$4.25/lb"), "lb");
        assert_eq!(detect_unit("€50/kg"), "kg");
        assert_eq!(detect_unit("$2000 per troy oz"), "oz");
        assert_eq!(detect_unit("$28000/mt"), "mt");
    }

    #[test]
    fn test_price_change_pct() {
        assert!((price_change_pct(100.0, 110.0) - 10.0).abs() < 0.001);
        assert!((price_change_pct(100.0, 90.0) - (-10.0)).abs() < 0.001);
        assert!((price_change_pct(0.0, 100.0)).abs() < 0.001);
    }

    #[test]
    fn test_is_critical_ems_commodity() {
        assert!(is_critical_ems_commodity(&CommodityType::Copper));
        assert!(is_critical_ems_commodity(&CommodityType::Tin));
        assert!(is_critical_ems_commodity(&CommodityType::Gold));
        assert!(is_critical_ems_commodity(&CommodityType::FR4));
        assert!(is_critical_ems_commodity(&CommodityType::Solder));
        assert!(!is_critical_ems_commodity(&CommodityType::Aluminum));
        assert!(!is_critical_ems_commodity(&CommodityType::Steel));
    }

    #[test]
    fn test_tabular_extraction() {
        let text = "Commodity prices:\n\
                    copper 4.25 USD /lb\n\
                    tin 28500 USD /mt\n\
                    gold 2025.50 USD /oz";
        let prices = extract_tabular_prices(text, "test", "https://example.com");
        assert!(prices.len() >= 2);
    }

    #[test]
    fn test_price_date_extraction() {
        let text = "Updated: 2025-01-15. Copper: $4.25/lb";
        let date = extract_price_date(text);
        assert_eq!(date, Some("2025-01-15".to_string()));
    }

    #[test]
    fn test_euro_price() {
        let text = "copper price: €3.95 per kg";
        let result = find_price_for_commodity(text, "copper");
        assert!(result.is_some());
        let (price, currency, _) = result.unwrap();
        assert!((price - 3.95).abs() < 0.01);
        assert_eq!(currency, "EUR");
    }

    #[test]
    fn test_usd_code_price() {
        // Regression: "100 USD" style must work even though text is lowercased internally
        let text = "copper price: 4.25 USD per lb";
        let result = find_price_for_commodity(text, "copper");
        assert!(result.is_some());
        let (price, currency, _) = result.unwrap();
        assert!((price - 4.25).abs() < 0.01);
        assert_eq!(currency, "USD");
    }

    #[test]
    fn test_yen_symbol_defaults_to_cny() {
        // Bare Yen symbol without explicit currency code defaults to CNY
        let text = "gold price: ¥185000 per oz";
        let result = find_price_for_commodity(text, "gold");
        assert!(result.is_some());
        let (_, currency, _) = result.unwrap();
        assert_eq!(currency, "CNY");
    }

    #[test]
    fn test_explicit_jpy_text() {
        // Explicit "JPY" in the text must be classified as JPY, not CNY
        let text = "gold price: 185000 JPY per oz";
        let result = find_price_for_commodity(text, "gold");
        assert!(result.is_some());
        let (price, currency, _) = result.unwrap();
        assert!((price - 185000.0).abs() < 1.0);
        assert_eq!(currency, "JPY");
    }
}
