use std::sync::LazyLock;

use chrono::{DateTime, Utc};
use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};

use crate::normalizer;
use apex_core::validation::normalize_url;

static RE_SOURCE: LazyLock<Regex> = LazyLock::new(|| {
    RegexBuilder::new(r"(?i)(?:source|agence|agency|wire)[:\s]+([^\n.;]+)")
        .size_limit(200_000)
        .dfa_size_limit(200_000)
        .build()
        .unwrap()
});

static RE_AUTHOR: LazyLock<Regex> = LazyLock::new(|| {
    // Use (?-i:...) around the name capture so that [A-Z]/[a-z] retain
    // case-sensitivity — the outer (?i) is only for the keyword prefix.
    RegexBuilder::new(
        r"(?i)(?:by|author|par|auteur)[:\s]+(?-i:([A-Z][a-z]+(?:\s[A-Z][a-z]+){1,3}))",
    )
    .size_limit(200_000)
    .dfa_size_limit(200_000)
    .build()
    .unwrap()
});

/// Pre-compiled company mention regex — avoids O(n) recompilation per article parse.
static RE_COMPANY_MENTION: LazyLock<Regex> = LazyLock::new(|| {
    let suffix_pattern = [
        "Inc",
        "Corp",
        "Ltd",
        "SARL",
        "SA",
        "GmbH",
        "AG",
        "SAS",
        "LLC",
        "Co",
        "Group",
        "Holdings",
        "Technologies",
        "Electronics",
        "Manufacturing",
        "Services",
    ]
    .join("|");
    let pattern = format!(
        r"\b([A-Z][a-zA-Z]+(?:\s+[A-Z][a-zA-Z]+){{0,3}})\s+(?:{})\b",
        suffix_pattern
    );
    RegexBuilder::new(&pattern)
        .size_limit(200_000)
        .dfa_size_limit(200_000)
        .build()
        .unwrap()
});

static DATE_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r"(?i)(?:January|February|March|April|May|June|July|August|September|October|November|December)\s+\d{1,2},?\s+\d{4}",
        r"(?i)\d{1,2}\s+(?:January|February|March|April|May|June|July|August|September|October|November|December)\s+\d{4}",
        r"\d{4}[-/]\d{2}[-/]\d{2}",
    ]
    .iter()
    .map(|p| {
        RegexBuilder::new(p)
            .size_limit(100_000)
            .dfa_size_limit(100_000)
            .build()
            .unwrap()
    })
    .collect()
});

/// Extracted press release / news article.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PressExtract {
    pub headline: String,
    pub source: Option<String>,
    pub author: Option<String>,
    pub date: Option<String>,
    pub summary: String,
    pub companies_mentioned: Vec<String>,
    pub topics: Vec<PressTopicTag>,
    pub url: String,
    pub language: Option<String>,
    pub extracted_at: DateTime<Utc>,
}

/// Topic tags for press articles.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PressTopicTag {
    Expansion,
    NewPlant,
    Acquisition,
    Partnership,
    Contract,
    Financial,
    Certification,
    Product,
    Personnel,
    Restructuring,
    Regulatory,
    Technology,
    Other,
}

/// Extract a press release / news article from body text.
pub fn extract_press(body_text: &str, title: &str, url: &str) -> PressExtract {
    let normalized_body = normalizer::normalize_whitespace(body_text);
    let source = extract_source(&normalized_body);
    let author = extract_author(&normalized_body);
    let date = extract_article_date(&normalized_body);
    let companies = extract_company_mentions(&normalized_body);
    let topics = classify_topics(&normalized_body, title);
    let mut lang = crate::multilingual::detect_language(&normalized_body);
    if lang.trim().is_empty() {
        lang = "en".to_string();
    }
    let normalized_url = normalize_url(url).unwrap_or_else(|| url.to_string());

    PressExtract {
        headline: normalizer::normalize_whitespace(title),
        source,
        author,
        date,
        summary: extract_lead(&normalized_body),
        companies_mentioned: companies,
        topics,
        url: normalized_url,
        language: Some(lang),
        extracted_at: Utc::now(),
    }
}

fn extract_source(text: &str) -> Option<String> {
    RE_SOURCE
        .captures(text)
        .map(|c| normalizer::normalize_whitespace(c.get(1).unwrap().as_str()))
}

fn extract_author(text: &str) -> Option<String> {
    RE_AUTHOR
        .captures(text)
        .map(|c| normalizer::normalize_whitespace(c.get(1).unwrap().as_str()))
}

fn extract_article_date(text: &str) -> Option<String> {
    for re in DATE_PATTERNS.iter() {
        if let Some(m) = re.find(text) {
            let raw = m.as_str();
            if is_valid_date(raw) {
                return Some(raw.to_string());
            }
        }
    }
    None
}

fn is_valid_date(raw: &str) -> bool {
    let patterns = ["%B %d, %Y", "%B %d %Y", "%d %B %Y", "%Y-%m-%d", "%Y/%m/%d"];
    patterns
        .iter()
        .any(|p| chrono::NaiveDate::parse_from_str(raw, p).is_ok())
}

fn extract_lead(text: &str) -> String {
    // First paragraph or first 500 chars
    let paragraphs: Vec<&str> = text
        .split("\n\n")
        .map(|p| p.trim())
        .filter(|p| p.len() > 20)
        .collect();

    if let Some(first) = paragraphs.first() {
        normalizer::truncate(first, 500)
    } else {
        normalizer::truncate(text, 500)
    }
}

/// Extract company names mentioned in text using capitalization patterns.
pub fn extract_company_mentions(text: &str) -> Vec<String> {
    let mut companies = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for caps in RE_COMPANY_MENTION.captures_iter(text) {
        let full_match = caps.get(0).unwrap().as_str();
        let name = normalizer::normalize_whitespace(full_match);
        if !seen.contains(&name) {
            seen.insert(name.clone());
            companies.push(name);
        }
    }

    // Also look for known EMS companies
    let known = [
        "Foxconn",
        "Jabil",
        "Flex",
        "Celestica",
        "Benchmark Electronics",
        "Starz Electronics",
        "Plexus",
        "Sanmina",
        "Venture",
        "Pegatron",
        "Wistron",
        "Compal",
        "Quanta",
        "USI",
    ];
    for company in &known {
        if text.contains(company) && !seen.contains(*company) {
            seen.insert(company.to_string());
            companies.push(company.to_string());
        }
    }

    companies
}

/// Classify press article topics based on content.
pub fn classify_topics(body: &str, title: &str) -> Vec<PressTopicTag> {
    let combined = format!("{} {}", title, body).to_lowercase();
    let mut topics = Vec::new();

    let rules: &[(&[&str], PressTopicTag)] = &[
        (
            &[
                "expansion",
                "new facility",
                "new plant",
                "nouvelle usine",
                "construction",
            ],
            PressTopicTag::NewPlant,
        ),
        (
            &["acqui", "merger", "rachat", "fusion"],
            PressTopicTag::Acquisition,
        ),
        (
            &[
                "partnership",
                "partenariat",
                "collaboration",
                "joint venture",
                "alliance",
            ],
            PressTopicTag::Partnership,
        ),
        (
            &["contract", "contrat", "award", "win", "won"],
            PressTopicTag::Contract,
        ),
        (
            &[
                "revenue",
                "profit",
                "earnings",
                "chiffre d'affaires",
                "quarterly",
                "annual report",
            ],
            PressTopicTag::Financial,
        ),
        (
            &["certif", "iso", "iatf", "accredit"],
            PressTopicTag::Certification,
        ),
        (
            &["launch", "new product", "nouveau produit", "release"],
            PressTopicTag::Product,
        ),
        (
            &["appoint", "hire", "nomm", "ceo", "cto", "vp", "director"],
            PressTopicTag::Personnel,
        ),
        (
            &["restructur", "layoff", "fermeture", "closure", "downsize"],
            PressTopicTag::Restructuring,
        ),
        (
            &["regulat", "compliance", "law", "loi", "directive"],
            PressTopicTag::Regulatory,
        ),
        (
            &["technolog", "innovat", "r&d", "patent", "brevet"],
            PressTopicTag::Technology,
        ),
        (
            &["expand", "growth", "croissance", "invest"],
            PressTopicTag::Expansion,
        ),
    ];

    for (keywords, tag) in rules {
        if keywords.iter().any(|kw| combined.contains(kw)) {
            topics.push(tag.clone());
        }
    }

    if topics.is_empty() {
        topics.push(PressTopicTag::Other);
    }

    topics
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_press_basic() {
        let body = "By John Smith. January 15, 2025.\n\n\
                    Starz Electronics SARL announced today the opening of a new manufacturing \
                    facility in Sousse, Tunisia. The expansion will create 500 new jobs and \
                    increase SMT assembly capacity by 40%.";
        let press = extract_press(
            body,
            "Starz Electronics Opens New Plant",
            "https://news.com/123",
        );

        assert_eq!(press.headline, "Starz Electronics Opens New Plant");
        assert!(press.author.is_some());
        assert!(press.date.is_some());
        assert!(!press.companies_mentioned.is_empty());
        assert!(press.topics.contains(&PressTopicTag::NewPlant));
    }

    #[test]
    fn test_extract_company_mentions() {
        let text = "Foxconn Technology Group and Jabil Inc announced a partnership. \
                    Starz Electronics will also participate.";
        let companies = extract_company_mentions(text);
        assert!(companies.iter().any(|c| c.contains("Foxconn")));
        assert!(companies.iter().any(|c| c.contains("Jabil")));
        assert!(companies.iter().any(|c| c.contains("Starz")));
    }

    #[test]
    fn test_classify_topics_acquisition() {
        let topics = classify_topics("Company X acquired Company Y for $1B", "Major Acquisition");
        assert!(topics.contains(&PressTopicTag::Acquisition));
    }

    #[test]
    fn test_classify_topics_financial() {
        let topics = classify_topics("Q3 revenue rose 15% to $2.1B", "Quarterly Earnings Report");
        assert!(topics.contains(&PressTopicTag::Financial));
    }

    #[test]
    fn test_classify_topics_multiple() {
        let topics = classify_topics(
            "The company announced a new technology partnership and patent filing",
            "Tech Partnership",
        );
        assert!(topics.contains(&PressTopicTag::Partnership));
        assert!(topics.contains(&PressTopicTag::Technology));
    }

    #[test]
    fn test_classify_topics_fallback() {
        let topics = classify_topics("Nothing relevant here", "Generic headline");
        assert_eq!(topics, vec![PressTopicTag::Other]);
    }

    #[test]
    fn test_extract_article_date() {
        assert!(extract_article_date("January 15, 2025").is_some());
        assert!(extract_article_date("15 March 2025").is_some());
        assert!(extract_article_date("Published: 2025-01-15").is_some());
        assert!(extract_article_date("February 31, 2024").is_none());
    }

    #[test]
    fn test_extract_author() {
        let text = "By Ahmed Ben Ali. Published on Jan 15.";
        let author = extract_author(text);
        assert!(author.is_some());
        assert!(author.unwrap().contains("Ahmed"));
    }

    #[test]
    fn test_extract_lead() {
        let text = "Short intro.\n\n\
                    This is the real first paragraph with enough content to be meaningful \
                    and should be selected as the lead of the article.\n\n\
                    Another paragraph here.";
        let lead = extract_lead(text);
        assert!(lead.contains("real first paragraph"));
    }

    #[test]
    fn test_french_topics() {
        let topics = classify_topics(
            "La société a annoncé la construction d'une nouvelle usine",
            "Expansion industrielle",
        );
        assert!(topics.contains(&PressTopicTag::NewPlant));
    }
}
