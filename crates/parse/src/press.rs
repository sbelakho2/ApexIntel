use chrono::{DateTime, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::normalizer;

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
pub fn extract_press(
    body_text: &str,
    title: &str,
    url: &str,
) -> PressExtract {
    let source = extract_source(body_text);
    let author = extract_author(body_text);
    let date = extract_article_date(body_text);
    let companies = extract_company_mentions(body_text);
    let topics = classify_topics(body_text, title);
    let lang = crate::multilingual::detect_language(body_text);

    PressExtract {
        headline: normalizer::normalize_whitespace(title),
        source,
        author,
        date,
        summary: extract_lead(body_text),
        companies_mentioned: companies,
        topics,
        url: url.to_string(),
        language: Some(lang),
        extracted_at: Utc::now(),
    }
}

fn extract_source(text: &str) -> Option<String> {
    let re = Regex::new(r"(?i)(?:source|agence|agency|wire)[:\s]+([^\n.;]+)").ok()?;
    re.captures(text)
        .map(|c| normalizer::normalize_whitespace(c.get(1).unwrap().as_str()))
}

fn extract_author(text: &str) -> Option<String> {
    let re = Regex::new(r"(?i)(?:by|author|par|auteur)[:\s]+([A-Z][a-z]+(?:\s[A-Z][a-z]+){1,3})").ok()?;
    re.captures(text)
        .map(|c| normalizer::normalize_whitespace(c.get(1).unwrap().as_str()))
}

fn extract_article_date(text: &str) -> Option<String> {
    let patterns = [
        r"(?:January|February|March|April|May|June|July|August|September|October|November|December)\s+\d{1,2},?\s+\d{4}",
        r"\d{1,2}\s+(?:January|February|March|April|May|June|July|August|September|October|November|December)\s+\d{4}",
        r"\d{4}[-/]\d{2}[-/]\d{2}",
    ];
    for pat in &patterns {
        if let Ok(re) = Regex::new(pat) {
            if let Some(m) = re.find(text) {
                return Some(m.as_str().to_string());
            }
        }
    }
    None
}

fn extract_lead(text: &str) -> String {
    // First paragraph or first 500 chars
    let paragraphs: Vec<&str> = text.split("\n\n")
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

    // Pattern: capitalized words followed by company suffixes
    let suffixes = [
        "Inc", "Corp", "Ltd", "SARL", "SA", "GmbH", "AG", "SAS",
        "LLC", "Co", "Group", "Holdings", "Technologies", "Electronics",
        "Manufacturing", "Services",
    ];
    let suffix_pattern = suffixes.join("|");
    let pattern = format!(
        r"\b([A-Z][a-zA-Z]+(?:\s+[A-Z][a-zA-Z]+){{0,3}})\s+(?:{})\b",
        suffix_pattern
    );

    if let Ok(re) = Regex::new(&pattern) {
        for caps in re.captures_iter(text) {
            let full_match = caps.get(0).unwrap().as_str();
            let name = normalizer::normalize_whitespace(full_match);
            if !seen.contains(&name) {
                seen.insert(name.clone());
                companies.push(name);
            }
        }
    }

    // Also look for known EMS companies
    let known = [
        "Foxconn", "Jabil", "Flex", "Celestica", "Benchmark Electronics",
        "Starz Electronics", "Plexus", "Sanmina", "Venture",
        "Pegatron", "Wistron", "Compal", "Quanta", "USI",
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
        (&["expansion", "new facility", "new plant", "nouvelle usine", "construction"], PressTopicTag::NewPlant),
        (&["acqui", "merger", "rachat", "fusion"], PressTopicTag::Acquisition),
        (&["partnership", "partenariat", "collaboration", "joint venture", "alliance"], PressTopicTag::Partnership),
        (&["contract", "contrat", "award", "win", "won"], PressTopicTag::Contract),
        (&["revenue", "profit", "earnings", "chiffre d'affaires", "quarterly", "annual report"], PressTopicTag::Financial),
        (&["certif", "iso", "iatf", "accredit"], PressTopicTag::Certification),
        (&["launch", "new product", "nouveau produit", "release"], PressTopicTag::Product),
        (&["appoint", "hire", "nomm", "ceo", "cto", "vp", "director"], PressTopicTag::Personnel),
        (&["restructur", "layoff", "fermeture", "closure", "downsize"], PressTopicTag::Restructuring),
        (&["regulat", "compliance", "law", "loi", "directive"], PressTopicTag::Regulatory),
        (&["technolog", "innovat", "r&d", "patent", "brevet"], PressTopicTag::Technology),
        (&["expand", "growth", "croissance", "invest"], PressTopicTag::Expansion),
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
        let press = extract_press(body, "Starz Electronics Opens New Plant", "https://news.com/123");

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
