use std::sync::LazyLock;

use chrono::{DateTime, Utc};
use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};

use crate::normalizer;
use apex_core::validation::normalize_url;

static RE_AWARD_NAME: LazyLock<Regex> = LazyLock::new(|| {
    RegexBuilder::new(r#"(?:award|prize|honor|recognition|distinction)[:\s]+([^\n.;]{5,200})"#)
        .case_insensitive(true)
        .size_limit(1_000_000)
        .dfa_size_limit(1_000_000)
        .build()
        .unwrap_or_else(|error| panic!("invalid award name regex: {error}"))
});

static RE_AWARD_RECIPIENT: LazyLock<Regex> = LazyLock::new(|| {
    RegexBuilder::new(r#"(?:recipient|winner|laureate|awarded to|presented to)[:\s]+(.{5,150}?)(?:\.\s|;\s|\n|$)"#)
        .case_insensitive(true)
        .size_limit(1_000_000)
        .dfa_size_limit(1_000_000)
        .build()
        .unwrap_or_else(|error| panic!("invalid award recipient regex: {error}"))
});

static RE_AWARD_CATEGORY: LazyLock<Regex> = LazyLock::new(|| {
    RegexBuilder::new(r#"(?:category|field|sector|discipline)[:\s]+([^\n.;]{3,100})"#)
        .case_insensitive(true)
        .size_limit(1_000_000)
        .dfa_size_limit(1_000_000)
        .build()
        .unwrap_or_else(|error| panic!("invalid award category regex: {error}"))
});

static RE_AWARD_DATE: LazyLock<Regex> = LazyLock::new(|| {
    RegexBuilder::new(r#"(?:award date|date awarded|presentation date|year)[:\s]+(\d{4}[-/]\d{2}[-/]\d{2}|\d{2}[-/]\d{2}[-/]\d{4}|\d{4})"#)
        .case_insensitive(true)
        .size_limit(1_000_000)
        .dfa_size_limit(1_000_000)
        .build()
        .unwrap_or_else(|error| panic!("invalid award date regex: {error}"))
});

static RE_AWARD_ORGANIZATION: LazyLock<Regex> = LazyLock::new(|| {
    RegexBuilder::new(
        r#"(?:awarding organization|presented by|sponsored by|organized by)[:\s]+([^\n.;]{5,150})"#,
    )
    .case_insensitive(true)
    .size_limit(1_000_000)
    .dfa_size_limit(1_000_000)
    .build()
    .unwrap_or_else(|error| panic!("invalid award organization regex: {error}"))
});

static RE_AWARD_DESCRIPTION: LazyLock<Regex> = LazyLock::new(|| {
    RegexBuilder::new(r#"(?i)(?:description|criteria|purpose)[:\s]+([\s\S]{10,500}?)(?:\n\n|\z)"#)
        .size_limit(1_000_000)
        .dfa_size_limit(1_000_000)
        .build()
        .unwrap_or_else(|error| panic!("invalid award description regex: {error}"))
});

/// Extracted award information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AwardExtract {
    pub award_name: String,
    pub recipient: String,
    pub recipient_type: RecipientType,
    pub category: Option<String>,
    pub award_date: Option<String>,
    pub awarding_organization: Option<String>,
    pub description: String,
    pub url: String,
    pub jurisdiction: String,
    pub keywords: Vec<String>,
    pub extracted_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RecipientType {
    Person,
    Organization,
    Team,
    Product,
    Unknown,
}

/// Extract award info from page body text.
pub fn extract_award(body_text: &str, title: &str, url: &str, jurisdiction: &str) -> AwardExtract {
    let award_name = extract_award_name(body_text, title);
    let recipient = extract_recipient(body_text).unwrap_or_else(|| "Unknown".to_string());
    let recipient_type = classify_recipient_type(&recipient);
    let category = extract_category(body_text);
    let award_date = extract_award_date(body_text);
    let awarding_organization = extract_awarding_organization(body_text);
    let description = extract_award_description(body_text);

    let ems_kws = crate::multilingual::ems_keywords("en");
    let keywords = crate::multilingual::contains_keywords(body_text, &ems_kws);

    let normalized_url = normalize_url(url).unwrap_or_else(|| url.to_string());

    AwardExtract {
        award_name,
        recipient,
        recipient_type,
        category,
        award_date,
        awarding_organization,
        description,
        url: normalized_url,
        jurisdiction: jurisdiction.to_string(),
        keywords,
        extracted_at: Utc::now(),
    }
}

fn extract_award_name(body_text: &str, title: &str) -> String {
    if let Some(caps) = RE_AWARD_NAME.captures(body_text) {
        if let Some(value) = caps.get(1) {
            return normalizer::normalize_whitespace(value.as_str());
        }
    }

    // Try to extract from title if it contains award-like words
    let title_lower = title.to_lowercase();
    if title_lower.contains("award")
        || title_lower.contains("prize")
        || title_lower.contains("honor")
    {
        normalizer::normalize_whitespace(title)
    } else {
        "Unknown Award".to_string()
    }
}

fn extract_recipient(body_text: &str) -> Option<String> {
    RE_AWARD_RECIPIENT.captures(body_text).and_then(|c| {
        c.get(1)
            .map(|m| normalizer::normalize_whitespace(m.as_str()))
    })
}

fn classify_recipient_type(recipient: &str) -> RecipientType {
    let recipient_lower = recipient.to_lowercase();

    // Check for organization indicators
    let org_indicators = [
        "inc",
        "corp",
        "corporation",
        "ltd",
        "limited",
        "llc",
        "company",
        "co",
        "group",
        "association",
        "institute",
        "university",
        "college",
        "lab",
        "laboratory",
        "center",
        "centre",
    ];
    if org_indicators
        .iter()
        .any(|indicator| recipient_lower.contains(indicator))
    {
        return RecipientType::Organization;
    }

    // Check for product indicators
    let product_indicators = [
        "model", "product", "system", "software", "platform", "device",
    ];
    if product_indicators
        .iter()
        .any(|indicator| recipient_lower.contains(indicator))
    {
        return RecipientType::Product;
    }

    // Check for team indicators
    let team_indicators = [
        "team",
        "group",
        "department",
        "division",
        "unit",
        "crew",
        "squad",
    ];
    if team_indicators
        .iter()
        .any(|indicator| recipient_lower.contains(indicator))
    {
        return RecipientType::Team;
    }

    // If it looks like a person name (2-4 capitalized words)
    let words: Vec<&str> = recipient.split_whitespace().collect();
    if words.len() >= 2
        && words.len() <= 4
        && words
            .iter()
            .all(|w| w.chars().next().map(|c| c.is_uppercase()).unwrap_or(false))
    {
        return RecipientType::Person;
    }

    RecipientType::Unknown
}

fn extract_category(body_text: &str) -> Option<String> {
    RE_AWARD_CATEGORY.captures(body_text).and_then(|c| {
        c.get(1)
            .map(|m| normalizer::normalize_whitespace(m.as_str()))
    })
}

fn extract_award_date(body_text: &str) -> Option<String> {
    RE_AWARD_DATE
        .captures(body_text)
        .and_then(|c| {
            c.get(1).map(|m| {
                let raw = m.as_str();
                // If it's just a year, format as YYYY-01-01
                if raw.len() == 4 && raw.chars().all(|c| c.is_ascii_digit()) {
                    format!("{}-01-01", raw)
                } else {
                    normalizer::normalize_whitespace(raw)
                }
            })
        })
        .filter(|raw| normalizer::is_valid_date(raw))
}

fn extract_awarding_organization(body_text: &str) -> Option<String> {
    RE_AWARD_ORGANIZATION.captures(body_text).and_then(|c| {
        c.get(1)
            .map(|m| normalizer::normalize_whitespace(m.as_str()))
    })
}

fn extract_award_description(body_text: &str) -> String {
    if let Some(caps) = RE_AWARD_DESCRIPTION.captures(body_text) {
        if let Some(value) = caps.get(1) {
            return normalizer::normalize_whitespace(value.as_str());
        }
        normalizer::truncate(body_text, 300)
    } else {
        normalizer::truncate(body_text, 300)
    }
}

/// Check if text contains award-related content
pub fn is_award_content(body_text: &str, title: &str) -> bool {
    let combined = format!("{} {}", title, body_text).to_lowercase();
    let award_keywords = [
        "award",
        "prize",
        "honor",
        "recognition",
        "distinction",
        "laureate",
        "winner",
    ];
    award_keywords
        .iter()
        .any(|keyword| combined.contains(keyword))
}

/// Classify award relevance for EMS/electronics manufacturing
pub fn classify_award_relevance(award: &AwardExtract) -> f32 {
    let mut score = 0.0_f32;

    // Check recipient type - organizations and people are most relevant
    match award.recipient_type {
        RecipientType::Organization | RecipientType::Person => score += 0.3,
        RecipientType::Product => score += 0.2,
        _ => score += 0.1,
    }

    // Keyword relevance in award name and description
    let ems_kws = crate::multilingual::ems_keywords("en");
    let combined = format!(
        "{} {} {}",
        award.award_name, award.description, award.recipient
    )
    .to_lowercase();
    for kw in &ems_kws {
        if combined.contains(&kw.to_lowercase()) {
            score += 0.15;
        }
    }

    // Check for specific award names that indicate industry relevance
    let industry_awards = [
        "electronics",
        "manufacturing",
        "supply chain",
        "innovation",
        "technology",
        "semiconductor",
        "engineering",
        "quality",
        "sustainability",
        "export",
    ];
    let award_name_lower = award.award_name.to_lowercase();
    for industry_term in &industry_awards {
        if award_name_lower.contains(industry_term) {
            score += 0.2;
        }
    }

    score.min(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_award_name() {
        let text = "Award: Best Electronics Manufacturer of the Year";
        let name = extract_award_name(text, "");
        assert_eq!(name, "Best Electronics Manufacturer of the Year");
    }

    #[test]
    fn test_extract_recipient() {
        let text = "Winner: John Smith";
        let recipient = extract_recipient(text);
        assert_eq!(recipient, Some("John Smith".to_string()));
    }

    #[test]
    fn test_classify_recipient_type_person() {
        assert_eq!(classify_recipient_type("John Smith"), RecipientType::Person);
        assert_eq!(
            classify_recipient_type("Dr. Jane Doe"),
            RecipientType::Person
        );
    }

    #[test]
    fn test_classify_recipient_type_organization() {
        assert_eq!(
            classify_recipient_type("Intel Corporation"),
            RecipientType::Organization
        );
        assert_eq!(
            classify_recipient_type("Foxconn Ltd."),
            RecipientType::Organization
        );
    }

    #[test]
    fn test_extract_category() {
        let text = "Category: Sustainable Manufacturing";
        let category = extract_category(text);
        assert_eq!(category, Some("Sustainable Manufacturing".to_string()));
    }

    #[test]
    fn test_extract_award_date_year_only() {
        let text = "Year: 2023";
        let date = extract_award_date(text);
        assert_eq!(date, Some("2023-01-01".to_string()));
    }

    #[test]
    fn test_extract_award_date_full() {
        let text = "Award Date: 2023-05-15";
        let date = extract_award_date(text);
        assert_eq!(date, Some("2023-05-15".to_string()));
    }

    #[test]
    fn test_extract_awarding_organization() {
        let text = "Presented by: International Electronics Manufacturing Society";
        let org = extract_awarding_organization(text);
        assert_eq!(
            org,
            Some("International Electronics Manufacturing Society".to_string())
        );
    }

    #[test]
    fn test_is_award_content() {
        let text = "This is about an award for manufacturing excellence.";
        let title = "Industry Recognition";
        assert!(is_award_content(text, title));
    }

    #[test]
    fn test_classify_award_relevance_high() {
        let award = AwardExtract {
            award_name: "Best Electronics Manufacturing Innovation Award".to_string(),
            recipient: "Intel Corporation".to_string(),
            recipient_type: RecipientType::Organization,
            category: Some("Manufacturing Excellence".to_string()),
            award_date: Some("2023-01-01".to_string()),
            awarding_organization: Some("EMS Association".to_string()),
            description: "Award for excellence in electronics manufacturing and semiconductor supply chain innovation".to_string(),
            url: "https://example.com".to_string(),
            jurisdiction: "US".to_string(),
            keywords: vec!["manufacturing".to_string()],
            extracted_at: Utc::now(),
        };
        let score = classify_award_relevance(&award);
        assert!(score > 0.5, "relevance score {score} should be > 0.5");
    }

    #[test]
    fn test_extract_award_full() {
        let body = "Award: EMS Innovation Prize. Winner: Dr. Wei Chen. \
                    Category: Advanced Packaging. Year: 2024. \
                    Presented by: Semiconductor Industry Association. \
                    Description: Recognizes groundbreaking work in 3D IC packaging technology.";
        let award = extract_award(
            body,
            "EMS Innovation Prize Awarded",
            "https://example.com/award",
            "US",
        );

        assert_eq!(award.award_name, "EMS Innovation Prize");
        assert_eq!(award.recipient, "Dr. Wei Chen");
        assert_eq!(award.recipient_type, RecipientType::Person);
        assert_eq!(award.category, Some("Advanced Packaging".to_string()));
        assert_eq!(award.award_date, Some("2024-01-01".to_string()));
        assert_eq!(
            award.awarding_organization,
            Some("Semiconductor Industry Association".to_string())
        );
        assert!(award.description.contains("3D IC packaging"));
    }
}
