use std::sync::LazyLock;

use chrono::{DateTime, Utc};
use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};

use crate::normalizer;
use apex_core::validation::normalize_url;

static RE_PATENT_NUMBER: LazyLock<Regex> = LazyLock::new(|| {
    RegexBuilder::new(r"\b((?:US|EP|WO|CN|JP|KR|TN|MA|IL|FR|DE)\s*\d[\d/]+(?:\s*[AB]\d?)?)\b")
        .size_limit(200_000)
        .dfa_size_limit(200_000)
        .build()
        .unwrap()
});

static RE_APPLICANT: LazyLock<Regex> = LazyLock::new(|| {
    RegexBuilder::new(r"(?i)(?:applicant|déposant|assignee|titulaire)[:\s]+([^\n.;]+)")
        .size_limit(200_000)
        .dfa_size_limit(200_000)
        .build()
        .unwrap()
});

static RE_INVENTORS: LazyLock<Regex> = LazyLock::new(|| {
    RegexBuilder::new(r"(?i)(?:inventor|inventeur)[s]?[:\s]+([^\n]+)")
        .size_limit(200_000)
        .dfa_size_limit(200_000)
        .build()
        .unwrap()
});

static RE_IPC_CODES: LazyLock<Regex> = LazyLock::new(|| {
    RegexBuilder::new(r"\b([A-H]\d{2}[A-Z]\s*\d{1,4}/\d{2,4})\b")
        .size_limit(200_000)
        .dfa_size_limit(200_000)
        .build()
        .unwrap()
});

static RE_FILING_DATE: LazyLock<Regex> = LazyLock::new(|| {
    RegexBuilder::new(r"(?i)(?:filing date|date de d\u00e9p\u00f4t|fecha de presentaci\u00f3n)[:\s]+(\d{4}[-/]\d{2}[-/]\d{2}|\d{2}[-/]\d{2}[-/]\d{4})")
        .size_limit(200_000)
        .dfa_size_limit(200_000)
        .build()
        .unwrap()
});

static RE_PUBLICATION_DATE: LazyLock<Regex> = LazyLock::new(|| {
    RegexBuilder::new(r"(?i)(?:publication date|date de publication)[:\s]+(\d{4}[-/]\d{2}[-/]\d{2}|\d{2}[-/]\d{2}[-/]\d{4})")
        .size_limit(200_000)
        .dfa_size_limit(200_000)
        .build()
        .unwrap()
});

static RE_ABSTRACT: LazyLock<Regex> = LazyLock::new(|| {
    RegexBuilder::new(r"(?i)(?:abstract|résumé|abrégé)[:\s]+([\s\S]{10,500}?)(?:\n\n|\z)")
        .size_limit(1_000_000) // larger limit needed for [\s\S] alternation
        .dfa_size_limit(1_000_000)
        .build()
        .unwrap()
});

/// Extracted patent information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatentExtract {
    pub patent_number: String,
    pub title: String,
    pub applicant: String,
    pub inventors: Vec<String>,
    pub filing_date: Option<String>,
    pub publication_date: Option<String>,
    pub ipc_codes: Vec<String>,
    pub abstract_text: String,
    pub url: String,
    pub jurisdiction: String,
    pub keywords: Vec<String>,
    pub extracted_at: DateTime<Utc>,
}

/// Extract patent info from page body text.
pub fn extract_patent(
    body_text: &str,
    title: &str,
    url: &str,
    jurisdiction: &str,
) -> PatentExtract {
    let patent_number = extract_patent_number(body_text)
        .unwrap_or_default();
    let applicant = extract_applicant(body_text)
        .unwrap_or_default();
    let inventors = extract_inventors(body_text);
    let filing_date = RE_FILING_DATE.captures(body_text)
        .map(|c| c.get(1).unwrap().as_str().to_string())
        .filter(|raw| normalizer::is_valid_date(raw));
    let publication_date = RE_PUBLICATION_DATE.captures(body_text)
        .map(|c| c.get(1).unwrap().as_str().to_string())
        .filter(|raw| normalizer::is_valid_date(raw));
    let ipc_codes = extract_ipc_codes(body_text);
    let abstract_text = extract_abstract(body_text);

    let ems_kws = crate::multilingual::ems_keywords("en");
    let keywords = crate::multilingual::contains_keywords(body_text, &ems_kws);

    let normalized_url = normalize_url(url).unwrap_or_else(|| url.to_string());

    PatentExtract {
        patent_number,
        title: normalizer::normalize_whitespace(title),
        applicant,
        inventors,
        filing_date,
        publication_date,
        ipc_codes,
        abstract_text,
        url: normalized_url,
        jurisdiction: jurisdiction.to_string(),
        keywords,
        extracted_at: Utc::now(),
    }
}

fn extract_patent_number(text: &str) -> Option<String> {
    // Patterns: US12345678, EP1234567, WO2024/123456, TN2024001, MA12345
    RE_PATENT_NUMBER.captures(text)
        .map(|c| normalizer::normalize_whitespace(c.get(1).unwrap().as_str()))
}

fn extract_applicant(text: &str) -> Option<String> {
    RE_APPLICANT.captures(text)
        .map(|c| normalizer::normalize_whitespace(c.get(1).unwrap().as_str()))
}

fn extract_inventors(text: &str) -> Vec<String> {
    if let Some(caps) = RE_INVENTORS.captures(text) {
        let list = caps.get(1).unwrap().as_str();
        list.split(&[',', ';'][..])
            .map(|s| normalizer::normalize_whitespace(s))
            .filter(|s| !s.is_empty())
            .collect()
    } else {
        vec![]
    }
}


fn extract_ipc_codes(text: &str) -> Vec<String> {
    // IPC codes like H05K 3/46, B23K 1/00
    RE_IPC_CODES
        .find_iter(text)
        .map(|m| normalizer::normalize_whitespace(m.as_str()))
        .collect()
}

fn extract_abstract(text: &str) -> String {
    if let Some(caps) = RE_ABSTRACT.captures(text) {
        normalizer::normalize_whitespace(caps.get(1).unwrap().as_str())
    } else {
        normalizer::truncate(text, 300)
    }
}

/// Classify patent relevance for EMS/electronics manufacturing.
pub fn classify_patent_relevance(patent: &PatentExtract) -> f32 {
    let mut score = 0.0_f32;

    // IPC code relevance
    for code in &patent.ipc_codes {
        let prefix = &code[..3.min(code.len())];
        match prefix {
            "H05" | "H01" | "H02" | "H03" | "H04" => score += 0.3, // Electrical
            "B23" => score += 0.2, // Machine tools / soldering
            "C25" => score += 0.15, // Electrolytic processes
            "G01" => score += 0.1, // Measuring / testing
            _ => {}
        }
    }

    // Keyword relevance
    let ems_kws = crate::multilingual::ems_keywords("en");
    let combined = format!("{} {} {}", patent.title, patent.abstract_text, patent.applicant).to_lowercase();
    for kw in &ems_kws {
        if combined.contains(&kw.to_lowercase()) {
            score += 0.1;
        }
    }

    score.min(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_patent_number_us() {
        let text = "Patent US12345678B2 filed on 2024-01-15";
        let num = extract_patent_number(text);
        assert!(num.is_some());
        assert!(num.unwrap().starts_with("US"));
    }

    #[test]
    fn test_extract_patent_number_ep() {
        let text = "European patent EP1234567 published";
        let num = extract_patent_number(text);
        assert_eq!(num, Some("EP1234567".to_string()));
    }

    #[test]
    fn test_extract_applicant() {
        let text = "Applicant: Starz Electronics SARL. Filed in Tunisia.";
        let applicant = extract_applicant(text);
        assert_eq!(applicant, Some("Starz Electronics SARL".to_string()));
    }

    #[test]
    fn test_extract_inventors() {
        let text = "Inventors: John Smith, Ahmed Ben Ali, Marie Dupont";
        let inventors = extract_inventors(text);
        assert_eq!(inventors.len(), 3);
        assert_eq!(inventors[0], "John Smith");
    }

    #[test]
    fn test_extract_filing_date() {
        let text = "Filing date: 2024-03-15. Publication date: 2024-09-15.";
        let date = RE_FILING_DATE.captures(text)
            .map(|c| c.get(1).unwrap().as_str().to_string());
        assert_eq!(date, Some("2024-03-15".to_string()));
    }

    #[test]
    fn test_extract_ipc_codes() {
        let text = "IPC: H05K 3/46, B23K 1/00. Classification details.";
        let codes = extract_ipc_codes(text);
        assert_eq!(codes.len(), 2);
        assert!(codes[0].starts_with("H05K"));
    }

    #[test]
    fn test_extract_abstract() {
        let text = "Abstract: A method for surface mount technology assembly using advanced \
                    pick-and-place equipment with vision alignment systems for high-density PCB production.";
        let abstract_text = extract_abstract(text);
        assert!(abstract_text.contains("surface mount"));
    }

    #[test]
    fn test_extract_patent_full() {
        let body = "Patent EP1234567. Applicant: Foxconn Technology Group. \
                    Inventors: Wei Chen, Li Zhang. \
                    Filing date: 2023-06-01. Publication date: 2024-01-15. \
                    IPC: H05K 3/46. \
                    Abstract: Improved reflow soldering process for SMT assembly lines \
                    with reduced thermal stress on BGA components.";
        let patent = extract_patent(body, "Reflow Soldering Process", "https://ep.espacenet.com/123", "EP");

        assert_eq!(patent.patent_number, "EP1234567");
        assert!(patent.applicant.contains("Foxconn"));
        assert_eq!(patent.inventors.len(), 2);
        assert!(!patent.ipc_codes.is_empty());
    }

    #[test]
    fn test_classify_patent_relevance_high() {
        let patent = PatentExtract {
            patent_number: "US12345".to_string(),
            title: "SMT Assembly Method".to_string(),
            applicant: "EMS Corp".to_string(),
            inventors: vec![],
            filing_date: None,
            publication_date: None,
            ipc_codes: vec!["H05K 3/46".to_string()],
            abstract_text: "PCB assembly with reflow soldering".to_string(),
            url: "https://example.com".to_string(),
            jurisdiction: "US".to_string(),
            keywords: vec![],
            extracted_at: Utc::now(),
        };
        let score = classify_patent_relevance(&patent);
        assert!(score > 0.3);
    }

    #[test]
    fn test_classify_patent_relevance_low() {
        let patent = PatentExtract {
            patent_number: "US99999".to_string(),
            title: "Shoe Design".to_string(),
            applicant: "Fashion Inc".to_string(),
            inventors: vec![],
            filing_date: None,
            publication_date: None,
            ipc_codes: vec!["A43B 1/00".to_string()],
            abstract_text: "A new shoe sole design".to_string(),
            url: "https://example.com".to_string(),
            jurisdiction: "US".to_string(),
            keywords: vec![],
            extracted_at: Utc::now(),
        };
        let score = classify_patent_relevance(&patent);
        assert!(score < 0.1);
    }
}
