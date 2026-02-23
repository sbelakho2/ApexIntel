use chrono::{DateTime, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::normalizer;

/// Extracted person of interest from a web page.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonExtract {
    pub name: String,
    pub title: Option<String>,
    pub company: Option<String>,
    pub role_family: Option<String>,
    pub seniority: Option<String>,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub linkedin_url: Option<String>,
    pub bio: String,
    pub url: String,
    pub extracted_at: DateTime<Utc>,
}

/// Extract person information from a page (about pages, team pages, LinkedIn-like).
pub fn extract_person(body_text: &str, url: &str) -> Vec<PersonExtract> {
    let mut persons = Vec::new();
    let mut seen = std::collections::HashSet::new();

    // Try structured patterns first
    let structured = extract_structured_persons(body_text, url);
    for p in structured {
        if !seen.contains(&p.name) {
            seen.insert(p.name.clone());
            persons.push(p);
        }
    }

    // Try name+title pattern
    let named = extract_named_persons(body_text, url);
    for p in named {
        if !seen.contains(&p.name) {
            seen.insert(p.name.clone());
            persons.push(p);
        }
    }

    persons
}

fn extract_structured_persons(text: &str, url: &str) -> Vec<PersonExtract> {
    let mut results = Vec::new();

    // Pattern: "Name, Title at Company" or "Name - Title, Company"
    let patterns = [
        r"([A-Z][a-z]+(?:\s+[A-Z][a-z]+){1,3})\s*,\s*((?:CEO|CTO|COO|CFO|VP|Director|Manager|Head|President|Chairman|Engineer|Founder|Partner)[\w\s]*?)(?:\s+(?:at|of|chez|à)\s+(.+?))?(?:\.|$|\n)",
        r"([A-Z][a-z]+(?:\s+[A-Z][a-z]+){1,3})\s*[-–]\s*((?:CEO|CTO|COO|CFO|VP|Director|Manager|Head|President|Chairman|Engineer|Founder|Partner)[\w\s]*?)(?:\s*,\s*(.+?))?(?:\.|$|\n)",
    ];

    for pat in &patterns {
        if let Ok(re) = Regex::new(pat) {
            for caps in re.captures_iter(text) {
                let name = normalizer::normalize_whitespace(caps.get(1).unwrap().as_str());
                let title = caps.get(2).map(|m| normalizer::normalize_whitespace(m.as_str()));
                let company = caps.get(3).map(|m| normalizer::normalize_whitespace(m.as_str()));

                let seniority = title.as_deref()
                    .map(|t| detect_seniority_from_title(t).to_string());
                let role_family = title.as_deref()
                    .map(|t| detect_role_domain(t).to_string());

                results.push(PersonExtract {
                    name,
                    title,
                    company,
                    role_family,
                    seniority,
                    email: None,
                    phone: None,
                    linkedin_url: None,
                    bio: String::new(),
                    url: url.to_string(),
                    extracted_at: Utc::now(),
                });
            }
        }
    }

    results
}

fn extract_named_persons(text: &str, url: &str) -> Vec<PersonExtract> {
    let mut results = Vec::new();
    let emails = normalizer::extract_emails(text);
    let linkedin_re = Regex::new(r"https?://(?:www\.)?linkedin\.com/in/([\w-]+)")
        .ok();

    // For each email, try to find a name nearby
    for email in &emails {
        let local = email.split('@').next().unwrap_or("");
        // Try to extract name from email local part: john.smith → John Smith
        let parts: Vec<String> = local.split(&['.', '_', '-'][..])
            .filter(|p| p.len() > 1)
            .map(|p| {
                let mut c = p.chars();
                match c.next() {
                    None => String::new(),
                    Some(f) => f.to_uppercase().to_string() + c.as_str(),
                }
            })
            .collect();
        if parts.len() >= 2 {
            let name = parts.join(" ");
            results.push(PersonExtract {
                name,
                title: None,
                company: None,
                role_family: None,
                seniority: None,
                email: Some(email.clone()),
                phone: None,
                linkedin_url: None,
                bio: String::new(),
                url: url.to_string(),
                extracted_at: Utc::now(),
            });
        }
    }

    // Extract LinkedIn URLs
    if let Some(re) = &linkedin_re {
        for caps in re.captures_iter(text) {
            let slug = caps.get(1).unwrap().as_str();
            let linkedin_url = caps.get(0).unwrap().as_str().to_string();
            let parts: Vec<String> = slug.split('-')
                .filter(|p| p.len() > 1 && !p.chars().all(|c| c.is_ascii_digit()))
                .map(|p| {
                    let mut c = p.chars();
                    match c.next() {
                        None => String::new(),
                        Some(f) => f.to_uppercase().to_string() + c.as_str(),
                    }
                })
                .collect();
            if parts.len() >= 2 {
                let name = parts.join(" ");
                results.push(PersonExtract {
                    name,
                    title: None,
                    company: None,
                    role_family: None,
                    seniority: None,
                    email: None,
                    phone: None,
                    linkedin_url: Some(linkedin_url),
                    bio: String::new(),
                    url: url.to_string(),
                    extracted_at: Utc::now(),
                });
            }
        }
    }

    results
}

fn detect_seniority_from_title(title: &str) -> &'static str {
    let lower = title.to_lowercase();
    if lower.contains("director") {
        "Director"
    } else if lower.contains("chief") || lower == "ceo" || lower == "cto"
        || lower == "coo" || lower == "cfo"
        || lower.starts_with("ceo ") || lower.starts_with("cto ")
        || lower.starts_with("coo ") || lower.starts_with("cfo ")
        || lower.contains(" ceo") || lower.contains(" cto")
        || lower.contains(" coo") || lower.contains(" cfo") {
        "C-Level"
    } else if lower.contains("president") || lower.contains("chairman") {
        "Executive"
    } else if lower.contains("vp") || lower.contains("vice president") {
        "VP"
    } else if lower.contains("director") {
        "Director"
    } else if lower.contains("head") || lower.contains("manager") {
        "Manager"
    } else if lower.contains("lead") || lower.contains("senior") {
        "Senior"
    } else if lower.contains("founder") || lower.contains("partner") {
        "Executive"
    } else {
        "Unknown"
    }
}

fn detect_role_domain(title: &str) -> &'static str {
    let lower = title.to_lowercase();
    if lower.contains("engineer") || lower.contains("tech") || lower.contains("r&d") {
        "Engineering"
    } else if lower.contains("sales") || lower.contains("commercial") || lower.contains("business dev") {
        "Sales"
    } else if lower.contains("supply") || lower.contains("procurement") || lower.contains("sourcing") {
        "Supply Chain"
    } else if lower.contains("quality") {
        "Quality"
    } else if lower.contains("finance") || lower.contains("cfo") || lower.contains("controller") {
        "Finance"
    } else if lower.contains("manufactur") || lower.contains("production") || lower.contains("operations") {
        "Operations"
    } else {
        "General Management"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_structured_person() {
        let text = "John Smith, CEO at Starz Electronics. Leading the company since 2020.";
        let persons = extract_structured_persons(text, "https://example.com");
        assert_eq!(persons.len(), 1);
        assert_eq!(persons[0].name, "John Smith");
        assert!(persons[0].title.as_deref().unwrap().contains("CEO"));
        assert!(persons[0].company.as_deref().unwrap().contains("Starz"));
    }

    #[test]
    fn test_extract_person_dash_format() {
        let text = "Marie Dupont - Director of Engineering, Foxconn Technology.";
        let persons = extract_structured_persons(text, "https://example.com");
        assert_eq!(persons.len(), 1);
        assert!(persons[0].title.as_deref().unwrap().contains("Director"));
    }

    #[test]
    fn test_extract_person_from_email() {
        let text = "Contact: john.smith@starz-electronics.tn for more info.";
        let persons = extract_named_persons(text, "https://example.com");
        assert_eq!(persons.len(), 1);
        assert_eq!(persons[0].name, "John Smith");
        assert_eq!(persons[0].email, Some("john.smith@starz-electronics.tn".to_string()));
    }

    #[test]
    fn test_extract_person_from_linkedin() {
        let text = "Visit https://www.linkedin.com/in/ahmed-ben-ali for details.";
        let persons = extract_named_persons(text, "https://example.com");
        assert_eq!(persons.len(), 1);
        assert_eq!(persons[0].name, "Ahmed Ben Ali");
        assert!(persons[0].linkedin_url.is_some());
    }

    #[test]
    fn test_detect_seniority() {
        assert_eq!(detect_seniority_from_title("CEO"), "C-Level");
        assert_eq!(detect_seniority_from_title("VP of Engineering"), "VP");
        assert_eq!(detect_seniority_from_title("Director of Operations"), "Director");
        assert_eq!(detect_seniority_from_title("Head of Quality"), "Manager");
        assert_eq!(detect_seniority_from_title("Founder"), "Executive");
    }

    #[test]
    fn test_detect_role_domain() {
        assert_eq!(detect_role_domain("VP of Engineering"), "Engineering");
        assert_eq!(detect_role_domain("Director of Sales"), "Sales");
        assert_eq!(detect_role_domain("Head of Supply Chain"), "Supply Chain");
        assert_eq!(detect_role_domain("VP Quality"), "Quality");
        assert_eq!(detect_role_domain("Director of Manufacturing"), "Operations");
    }

    #[test]
    fn test_extract_person_full() {
        let text = "Team:\n\
                    Ahmed Ben Ali, CEO at Starz Electronics.\n\
                    Marie Dupont, CTO at Starz Electronics.\n\
                    Contact: info@starz.tn";
        let persons = extract_person(text, "https://starz.tn/team");
        // Should find at least the two structured persons
        assert!(persons.len() >= 2);
    }

    #[test]
    fn test_no_duplicates() {
        let text = "John Smith, CEO at Example Corp. \
                    Contact: john.smith@example.com. \
                    John Smith leads the company.";
        let persons = extract_person(text, "https://example.com");
        let john_count = persons.iter().filter(|p| p.name == "John Smith").count();
        assert_eq!(john_count, 1);
    }
}
