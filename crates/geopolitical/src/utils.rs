//! Utility functions for the geopolitical intelligence module


/// HS Code (Harmonized System) utilities
pub mod hs_code {
    use regex::Regex;

    /// Validate HS code format (6 digits, optionally 8 or 10)
    #[allow(clippy::disallowed_methods)]
    pub fn is_valid_hs_code(code: &str) -> bool {
        let clean = code.replace(['-', ' '], "");
        let re = Regex::new(r"^\d{6}(\d{2})?(\d{2})?$").unwrap();
        re.is_match(&clean)
    }

    /// Parse HS code into components
    pub fn parse_hs_code(code: &str) -> Option<HsCodeComponents> {
        let clean = code.replace(['-', ' '], "");
        if clean.len() < 6 {
            return None;
        }

        let chars: Vec<char> = clean.chars().collect();
        if !chars.iter().all(|c| c.is_ascii_digit()) {
            return None;
        }

        Some(HsCodeComponents {
            chapter: chars[0..2].iter().collect(),
            heading: chars[2..4].iter().collect(),
            subheading: chars[4..6].iter().collect(),
            additional_6: if chars.len() >= 8 { Some(chars[6..8].iter().collect()) } else { None },
            additional_2: if chars.len() >= 10 { Some(chars[8..10].iter().collect()) } else { None },
        })
    }

    /// Get HS code chapter description prefix
    pub fn get_chapter_category(chapter: &str) -> Option<&'static str> {
        let chapter_num: u32 = chapter.parse().ok()?;
        
        Some(match chapter_num {
            1..=5 => "Live animals; animal products",
            6..=14 => "Vegetable products",
            15..=24 => "Animal or vegetable fats and oils",
            25..=27 => "Mineral products",
            28..=38 => "Chemicals",
            39..=40 => "Plastics and rubber",
            41..=43 => "Leather and articles thereof",
            44..=49 => "Wood and articles of wood",
            50..=63 => "Textiles and textile articles",
            64..=67 => "Footwear, headgear, etc.",
            68..=70 => "Stone, plaster, cement, etc.",
            71 => "Pearls, precious stones, metals",
            72..=83 => "Base metals and articles thereof",
            84..=85 => "Machinery and electrical equipment",
            86..=89 => "Vehicles, aircraft, vessels",
            90..=92 => "Optical, medical, musical instruments",
            93 => "Arms and ammunition",
            94..=96 => "Miscellaneous manufactured articles",
            97 => "Works of art, antiques",
            _ => return None,
        })
    }

    #[derive(Debug, Clone)]
    pub struct HsCodeComponents {
        pub chapter: String,
        pub heading: String,
        pub subheading: String,
        pub additional_6: Option<String>,
        pub additional_2: Option<String>,
    }
}

/// Hash utilities
pub mod hashing {
    use sha2::{Sha256, Digest};

    /// Generate content hash for deduplication
    pub fn content_hash(content: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(content.as_bytes());
        hex::encode(hasher.finalize())
    }

    /// Generate entity hash for sanctions matching
    pub fn entity_hash(name: &str, country: &str) -> String {
        let normalized = format!(
            "{}|{}",
            name.to_uppercase().trim(),
            country.to_uppercase().trim()
        );
        content_hash(&normalized)
    }
}

/// Country name utilities
pub mod country {
    use std::collections::HashMap;

    /// Common country name mappings
    pub fn standardize_name(name: &str) -> String {
        let name = name.trim().to_uppercase();
        
        let aliases: HashMap<&str, &str> = [
            ("USA", "US"),
            ("UNITED STATES", "US"),
            ("UNITED STATES OF AMERICA", "US"),
            ("UK", "GB"),
            ("UNITED KINGDOM", "GB"),
            ("GREAT BRITAIN", "GB"),
            ("RUSSIA", "RU"),
            ("RUSSIAN FEDERATION", "RU"),
            ("CHINA", "CN"),
            ("PEOPLES REPUBLIC OF CHINA", "CN"),
            ("PRC", "CN"),
            ("GERMANY", "DE"),
            ("FRANCE", "FR"),
            ("JAPAN", "JP"),
            ("CANADA", "CA"),
            ("AUSTRALIA", "AU"),
            ("INDIA", "IN"),
            ("BRAZIL", "BR"),
            ("SOUTH KOREA", "KR"),
            ("REPUBLIC OF KOREA", "KR"),
        ].into_iter().collect();

        aliases.get(name.as_str()).map(|s| s.to_string()).unwrap_or(name)
    }

    /// ISO country code validation
    pub fn is_valid_iso_code(code: &str) -> bool {
        code.len() == 2 && code.chars().all(|c| c.is_ascii_alphabetic())
    }

    /// Get country name from code
    pub fn name_from_code(code: &str) -> Option<&'static str> {
        let countries: HashMap<&str, &str> = [
            ("US", "United States"),
            ("GB", "United Kingdom"),
            ("CN", "China"),
            ("RU", "Russia"),
            ("DE", "Germany"),
            ("FR", "France"),
            ("JP", "Japan"),
            ("CA", "Canada"),
            ("AU", "Australia"),
            ("IN", "India"),
            ("BR", "Brazil"),
            ("KR", "South Korea"),
            ("IT", "Italy"),
            ("ES", "Spain"),
            ("NL", "Netherlands"),
        ].into_iter().collect();
        
        countries.get(code.to_uppercase().as_str()).copied()
    }
}

/// Currency utilities
pub mod currency {
    /// Common currency codes
    pub fn is_valid_currency_code(code: &str) -> bool {
        let valid_currencies = [
            "USD", "EUR", "GBP", "JPY", "CNY", "RUB", "CHF", "CAD", "AUD", "INR",
            "BRL", "KRW", "SGD", "HKD", "NOK", "SEK", "DKK", "NZD", "ZAR", "MXN",
        ];
        valid_currencies.contains(&code.to_uppercase().as_str())
    }

    /// Get currency symbol
    pub fn symbol(code: &str) -> Option<&'static str> {
        let symbols: std::collections::HashMap<&str, &str> = [
            ("USD", "$"),
            ("EUR", "€"),
            ("GBP", "£"),
            ("JPY", "¥"),
            ("CNY", "¥"),
            ("RUB", "₽"),
        ].into_iter().collect();
        symbols.get(code.to_uppercase().as_str()).copied()
    }
}

/// Date utilities
pub mod dates {
    use chrono::NaiveDate;
    use crate::GeopoliticalError;

    /// Parse date from various formats
    pub fn parse_date(date_str: &str) -> std::result::Result<NaiveDate, GeopoliticalError> {
        let formats = [
            "%Y-%m-%d",
            "%d/%m/%Y",
            "%m/%d/%Y",
            "%Y/%m/%d",
            "%B %d, %Y",
            "%d %B %Y",
        ];

        for format in formats {
            if let Ok(date) = NaiveDate::parse_from_str(date_str, format) {
                return Ok(date);
            }
        }

        Err(GeopoliticalError::ParseError(format!(
            "Failed to parse date: {}", date_str
        )))
    }
}

/// Name matching utilities
pub mod name_matching {
    /// Calculate similarity between two names (Jaccard index on character n-grams)
    pub fn name_similarity(name1: &str, name2: &str) -> f32 {
        let n1 = normalize_for_matching(name1);
        let n2 = normalize_for_matching(name2);

        if n1.is_empty() || n2.is_empty() {
            return 0.0;
        }

        let ngrams1 = get_ngrams(&n1, 3);
        let ngrams2 = get_ngrams(&n2, 3);

        let intersection = ngrams1.intersection(&ngrams2).count() as f32;
        let union = ngrams1.union(&ngrams2).count() as f32;

        if union == 0.0 {
            0.0
        } else {
            intersection / union
        }
    }

    fn normalize_for_matching(name: &str) -> String {
        name.to_uppercase()
            .chars()
            .filter(|c| c.is_ascii_alphabetic())
            .collect()
    }

    fn get_ngrams(s: &str, n: usize) -> std::collections::HashSet<String> {
        let chars: Vec<char> = s.chars().collect();
        let mut ngrams = std::collections::HashSet::new();
        
        for i in 0..chars.len().saturating_sub(n - 1) {
            let ngram: String = chars[i..i+n].iter().collect();
            ngrams.insert(ngram);
        }
        
        ngrams
    }
}

/// File format detection
pub mod format {
    /// Detect file format from content or extension
    pub fn detect_format(content: &[u8], filename: Option<&str>) -> Option<FileFormat> {
        // Check BOM
        if content.starts_with(&[0xEF, 0xBB, 0xBF]) {
            return Some(FileFormat::Utf8);
        }
        if content.starts_with(&[0xFF, 0xFE]) {
            return Some(FileFormat::Utf16Le);
        }
        if content.starts_with(&[0xFE, 0xFF]) {
            return Some(FileFormat::Utf16Be);
        }

        // Check for XML
        let start = String::from_utf8_lossy(&content[..content.len().min(100)]);
        if start.trim_start().starts_with("<?xml") || start.trim_start().starts_with("<") {
            return Some(FileFormat::Xml);
        }

        // Check for JSON
        if content[0] == b'{' || content[0] == b'[' {
            return Some(FileFormat::Json);
        }

        // Check from extension
        if let Some(name) = filename {
            if name.ends_with(".xml") {
                return Some(FileFormat::Xml);
            }
            if name.ends_with(".json") {
                return Some(FileFormat::Json);
            }
            if name.ends_with(".csv") {
                return Some(FileFormat::Csv);
            }
            if name.ends_with(".xlsx") || name.ends_with(".xls") {
                return Some(FileFormat::Excel);
            }
        }

        Some(FileFormat::Unknown)
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum FileFormat {
        Json,
        Xml,
        Csv,
        Excel,
        Utf8,
        Utf16Le,
        Utf16Be,
        Unknown,
    }
}

/// Sanctions list update utilities
pub mod sanctions_utils {
    use crate::models::IntelligenceSource;
    use chrono::Utc;
    
    /// Determine which source has most recent data
    pub fn latest_source(sources: &[(IntelligenceSource, chrono::DateTime<Utc>)]) -> Option<IntelligenceSource> {
        sources.iter()
            .max_by_key(|(_, date)| *date)
            .map(|(source, _)| source.clone())
    }
}

/// HTTP utilities
pub mod http {
    /// Build query string from parameters
    pub fn build_query_string(params: &[(&str, &str)]) -> String {
        let encoded: Vec<String> = params.iter()
            .map(|(k, v)| format!("{}={}", urlencoding::encode(k), urlencoding::encode(v)))
            .collect();
        encoded.join("&")
    }
}

/// Score calculation utilities
pub mod scoring {
    /// Calculate weighted risk score
    pub fn weighted_risk(
        economic: f32,
        political: f32,
        social: f32,
        environmental: f32,
        weights: (f32, f32, f32, f32),
    ) -> f32 {
        let total_weight: f32 = weights.0 + weights.1 + weights.2 + weights.3;
        if total_weight == 0.0 {
            return 0.0;
        }

        (economic * weights.0 + political * weights.1 + social * weights.2 + environmental * weights.3) / total_weight
    }

    /// Calculate stability score from multiple factors
    pub fn stability_score(
        governance: f32,
        rule_of_law: f32,
        corruption: f32,
        conflict: f32,
    ) -> f32 {
        let corruption_risk = 1.0 - corruption;
        let conflict_risk = 1.0 - conflict;

        (governance + rule_of_law + corruption_risk + conflict_risk) / 4.0
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::disallowed_methods)]
    use super::*;

    #[test]
    fn test_hs_code_validation() {
        assert!(hs_code::is_valid_hs_code("123456"));
        assert!(hs_code::is_valid_hs_code("12345678"));
        assert!(hs_code::is_valid_hs_code("1234567890"));
        assert!(hs_code::is_valid_hs_code("1234-56"));
        assert!(!hs_code::is_valid_hs_code("12345"));
        assert!(!hs_code::is_valid_hs_code("abcdef"));
    }

    #[test]
    fn test_hs_code_parsing() {
        let components = hs_code::parse_hs_code("847130").unwrap();
        assert_eq!(components.chapter, "84");
        assert_eq!(components.heading, "71");
        assert_eq!(components.subheading, "30");

        let components = hs_code::parse_hs_code("8471300000").unwrap();
        assert_eq!(components.additional_6, Some("00".to_string()));
        assert_eq!(components.additional_2, Some("00".to_string()));
    }

    #[test]
    fn test_name_matching() {
        let similarity = name_matching::name_similarity("JOHN SMITH", "SMITH, JOHN");
        assert!(similarity > 0.5);

        let similarity = name_matching::name_similarity("ACME CORP", "ACME CORPORATION");
        assert!(similarity > 0.4);

        let similarity = name_matching::name_similarity("ABC INC", "XYZ LLC");
        assert!(similarity < 0.3);
    }

    #[test]
    fn test_country_standardization() {
        assert_eq!(country::standardize_name("USA"), "US");
        assert_eq!(country::standardize_name("United Kingdom"), "GB");
        assert_eq!(country::standardize_name("Russia"), "RU");
    }

    #[test]
    fn test_hashing() {
        let hash1 = hashing::entity_hash("Test Entity", "US");
        let hash2 = hashing::entity_hash("test entity", "us");
        assert_eq!(hash1, hash2);

        let hash3 = hashing::entity_hash("Different Entity", "US");
        assert_ne!(hash1, hash3);
    }

    #[test]
    fn test_weighted_risk() {
        let score = scoring::weighted_risk(
            0.8, 0.6, 0.4, 0.2,
            (0.3, 0.3, 0.2, 0.2)
        );
        assert!((score - 0.54).abs() < 0.01);
    }

    #[test]
    fn test_format_detection() {
        let json_content = b"{ \"test\": true }";
        assert_eq!(format::detect_format(json_content, None), Some(format::FileFormat::Json));

        let xml_content = b"<root><item>test</item></root>";
        assert_eq!(format::detect_format(xml_content, None), Some(format::FileFormat::Xml));
    }

    #[test]
    fn test_date_parsing() {
        let date = dates::parse_date("2024-01-15").unwrap();
        assert_eq!(date.to_string(), "2024-01-15");

        let date = dates::parse_date("15/01/2024").unwrap();
        assert_eq!(date.to_string(), "2024-01-15");
    }
}
