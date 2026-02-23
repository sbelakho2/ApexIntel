use chrono::{DateTime, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::normalizer;

/// Extracted tender/procurement posting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TenderExtract {
    pub title: String,
    pub buyer: Option<String>,
    pub reference_number: Option<String>,
    pub value_estimate: Option<f64>,
    pub currency: Option<String>,
    pub sector: Option<String>,
    pub deadline: Option<String>,
    pub portal: String,
    pub url: String,
    pub description: String,
    pub keywords: Vec<String>,
    pub extracted_at: DateTime<Utc>,
}

/// Extract tender information from page text.
pub fn extract_tender(
    body_text: &str,
    title: &str,
    url: &str,
    portal: &str,
) -> TenderExtract {
    let buyer = extract_buyer(body_text);
    let reference_number = extract_reference(body_text);
    let (value_estimate, currency) = extract_value(body_text);
    let deadline = extract_deadline_text(body_text);
    let sector = detect_sector(body_text);

    let proc_kws = crate::multilingual::procurement_keywords("en");
    let ems_kws = crate::multilingual::ems_keywords("en");
    let all_kws: Vec<&str> = proc_kws.into_iter().chain(ems_kws.into_iter()).collect();
    let keywords = crate::multilingual::contains_keywords(body_text, &all_kws);

    TenderExtract {
        title: normalizer::normalize_whitespace(title),
        buyer,
        reference_number,
        value_estimate,
        currency,
        sector,
        deadline,
        portal: portal.to_string(),
        url: url.to_string(),
        description: normalizer::truncate(body_text, 500),
        keywords,
        extracted_at: Utc::now(),
    }
}

fn extract_buyer(text: &str) -> Option<String> {
    let re = Regex::new(r"(?i)(?:buyer|acheteur|contracting authority|maître d'ouvrage|entidad contratante)[:\s]+([^\n.]+)").ok()?;
    re.captures(text)
        .map(|c| normalizer::normalize_whitespace(c.get(1).unwrap().as_str()))
}

fn extract_reference(text: &str) -> Option<String> {
    let re = Regex::new(r"(?i)(?:ref(?:erence)?|n°|numéro)[:\s]*([A-Z0-9][\w\-/]+)").ok()?;
    re.captures(text)
        .map(|c| c.get(1).unwrap().as_str().to_string())
}

fn extract_value(text: &str) -> (Option<f64>, Option<String>) {
    let re = Regex::new(r"(?i)(?:value|montant|budget|estimated value)[:\s]*([€$£¥]?)\s*([\d,.]+)\s*(EUR|USD|TND|MAD|GBP|CNY|JPY|KRW|ILS)?").ok();
    if let Some(re) = re {
        if let Some(caps) = re.captures(text) {
            let symbol = caps.get(1).map(|m| m.as_str()).unwrap_or("");
            let amount_str = caps.get(2).map(|m| m.as_str()).unwrap_or("");
            let currency_name = caps.get(3).map(|m| m.as_str().to_string());

            let amount = amount_str
                .replace(',', "")
                .parse::<f64>()
                .ok();

            let currency = currency_name.or_else(|| {
                match symbol {
                    "€" => Some("EUR".to_string()),
                    "$" => Some("USD".to_string()),
                    "£" => Some("GBP".to_string()),
                    "¥" => Some("CNY".to_string()),
                    _ => None,
                }
            });

            return (amount, currency);
        }
    }
    (None, None)
}

fn extract_deadline_text(text: &str) -> Option<String> {
    let re = Regex::new(r"(?i)(?:deadline|date limite|fecha límite|closing date)[:\s]+([^\n.]+)").ok()?;
    re.captures(text)
        .map(|c| normalizer::normalize_whitespace(c.get(1).unwrap().as_str()))
}

fn detect_sector(text: &str) -> Option<String> {
    let lower = text.to_lowercase();
    let sectors = [
        ("automotive", "automotive"),
        ("aerospace", "aerospace"),
        ("defense", "defense"),
        ("medical", "medical"),
        ("industrial", "industrial"),
        ("energy", "energy"),
        ("telecom", "telecom"),
        ("electronics", "electronics"),
        ("semiconductor", "semiconductor"),
    ];

    for (keyword, sector) in &sectors {
        if lower.contains(keyword) {
            return Some(sector.to_string());
        }
    }
    None
}

/// Check if a tender is relevant to EMS / electronics manufacturing.
pub fn is_ems_relevant(tender: &TenderExtract) -> bool {
    let ems_kws = crate::multilingual::ems_keywords("en");
    let body = format!("{} {}", tender.title, tender.description).to_lowercase();
    ems_kws.iter().any(|kw| body.contains(&kw.to_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_tender_basic() {
        let body = "Buyer: Ministry of Defense Tunisia. Reference: TN-2024-001. \
                    Value: 500000 TND. Deadline: 2024-03-15. \
                    Supply of SMT assembly line equipment for the electronics sector.";
        let tender = extract_tender(body, "SMT Equipment Supply", "https://tuneps.tn/123", "TUNEPS");

        assert_eq!(tender.title, "SMT Equipment Supply");
        assert!(tender.buyer.is_some());
        assert!(tender.reference_number.is_some());
        assert!(tender.value_estimate.is_some());
        assert_eq!(tender.currency, Some("TND".to_string()));
        assert!(tender.deadline.is_some());
    }

    #[test]
    fn test_extract_buyer() {
        let text = "Buyer: Starz Electronics SARL. Details below.";
        let buyer = extract_buyer(text);
        assert!(buyer.is_some());
        assert!(buyer.unwrap().contains("Starz"));
    }

    #[test]
    fn test_extract_reference() {
        let text = "Reference: TN-2024-EMS-001. Procurement details.";
        let reference = extract_reference(text);
        assert_eq!(reference, Some("TN-2024-EMS-001".to_string()));
    }

    #[test]
    fn test_extract_value_with_currency() {
        let (val, cur) = extract_value("Value: 1000000 EUR");
        assert_eq!(val, Some(1000000.0));
        assert_eq!(cur, Some("EUR".to_string()));
    }

    #[test]
    fn test_extract_value_with_symbol() {
        let (val, cur) = extract_value("Estimated value: €500000");
        assert_eq!(val, Some(500000.0));
        assert_eq!(cur, Some("EUR".to_string()));
    }

    #[test]
    fn test_detect_sector() {
        assert_eq!(detect_sector("automotive electronics assembly"), Some("automotive".to_string()));
        assert_eq!(detect_sector("medical device manufacturing"), Some("medical".to_string()));
        assert_eq!(detect_sector("no sector here"), None);
    }

    #[test]
    fn test_is_ems_relevant() {
        let tender = TenderExtract {
            title: "PCB Assembly Services".to_string(),
            buyer: None,
            reference_number: None,
            value_estimate: None,
            currency: None,
            sector: None,
            deadline: None,
            portal: "test".to_string(),
            url: "https://example.com".to_string(),
            description: "Looking for SMT and box build services".to_string(),
            keywords: vec![],
            extracted_at: Utc::now(),
        };
        assert!(is_ems_relevant(&tender));
    }

    #[test]
    fn test_is_not_ems_relevant() {
        let tender = TenderExtract {
            title: "Office Supplies".to_string(),
            buyer: None,
            reference_number: None,
            value_estimate: None,
            currency: None,
            sector: None,
            deadline: None,
            portal: "test".to_string(),
            url: "https://example.com".to_string(),
            description: "Paper and toner cartridges".to_string(),
            keywords: vec![],
            extracted_at: Utc::now(),
        };
        assert!(!is_ems_relevant(&tender));
    }
}
