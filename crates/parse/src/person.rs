use std::sync::LazyLock;

use chrono::{DateTime, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::normalizer;
use apex_core::validation::normalize_url;

static PERSON_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r"([A-Z][a-z\u{00e0}-\u{00ff}]+(?:[\s-]+(?:[a-z\u{00e0}-\u{00ff}]{1,4}\s+)*[A-Z][a-z\u{00e0}-\u{00ff}]+){1,4})\s*,\s*((?:CEO|CTO|COO|CFO|VP|Director|Manager|Head|President|Chairman|Engineer|Founder|Partner)[\w\s]*?)(?:\s+(?:at|of|chez|\u{00e0})\s+(.+?))?(?:\.|$|\n)",
        r"([A-Z][a-z\u{00e0}-\u{00ff}]+(?:[\s-]+(?:[a-z\u{00e0}-\u{00ff}]{1,4}\s+)*[A-Z][a-z\u{00e0}-\u{00ff}]+){1,4})\s*[-\u{2013}]\s*((?:CEO|CTO|COO|CFO|VP|Director|Manager|Head|President|Chairman|Engineer|Founder|Partner)[\w\s]*?)(?:\s*,\s*(.+?))?(?:\.|$|\n)",
    ]
    .iter()
    .map(|p| Regex::new(p).unwrap())
    .collect()
});

static RE_LINKEDIN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"https?://(?:www\.)?linkedin\.com/in/([\w-]+)").unwrap()
});

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
    let normalized_body = normalizer::normalize_whitespace(body_text);
    let normalized_url = normalize_url(url).unwrap_or_else(|| url.to_string());
    let mut persons = Vec::new();
    let mut seen = std::collections::HashSet::new();

    // Try structured patterns first
    let structured = extract_structured_persons(&normalized_body, &normalized_url);
    for p in structured {
        if !seen.contains(&p.name) {
            seen.insert(p.name.clone());
            persons.push(p);
        }
    }

    // Try name+title pattern
    let named = extract_named_persons(&normalized_body, &normalized_url);
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
    for re in PERSON_PATTERNS.iter() {
        for caps in re.captures_iter(text) {
            let name = normalizer::normalize_whitespace(caps.get(1).unwrap().as_str());
            let title = caps.get(2).map(|m| normalizer::normalize_whitespace(m.as_str()));
            let mut company = caps.get(3).map(|m| normalizer::normalize_whitespace(m.as_str()));

            let seniority = title.as_deref()
                .map(|t| detect_seniority_from_title(t).to_string());
            let role_family = title.as_deref()
                .map(|t| detect_role_domain(t).to_string());

            // For government roles, extract department/ministry from title if no company found
            if company.is_none() || company.as_deref() == Some("") {
                if let Some(ref t) = title {
                    if detect_role_domain(t) == "Government" || detect_role_domain(t) == "Military" {
                        company = extract_government_affiliation(t);
                    }
                }
            }

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

    results
}

fn extract_named_persons(text: &str, url: &str) -> Vec<PersonExtract> {
    let mut results = Vec::new();
    let emails = normalizer::extract_emails(text);

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
    for caps in RE_LINKEDIN.captures_iter(text) {
        let slug = caps.get(1).unwrap().as_str();
        let raw_url = caps.get(0).unwrap().as_str().to_string();
        let linkedin_url = normalize_url(&raw_url).unwrap_or(raw_url);
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

    results
}

fn detect_seniority_from_title(title: &str) -> &'static str {
    let lower = title.to_lowercase();
    if lower.contains("chief") || lower == "ceo" || lower == "cto"
        || lower == "coo" || lower == "cfo"
        || lower.starts_with("ceo ") || lower.starts_with("cto ")
        || lower.starts_with("coo ") || lower.starts_with("cfo ")
        || lower.contains(" ceo") || lower.contains(" cto")
        || lower.contains(" coo") || lower.contains(" cfo") {
        "C-Level"
    } else if lower.contains("president") || lower.contains("chairman") {
        "Executive"
    } else if lower.contains("vice president")
        || lower == "vp"
        || lower.contains(" vp ")
        || lower.contains(" vp,")
        || lower.starts_with("vp ")
        || lower.starts_with("vp-")
        || lower.ends_with(" vp")
    {
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
    
    // ═══════════════════════════════════════════════════════════════════════════
    // GOVERNMENT / PUBLIC SECTOR DETECTION (must be first to avoid false matches)
    // ═══════════════════════════════════════════════════════════════════════════
    if lower.contains("minister") || lower.contains("ministère") || lower.contains("وزير")
        || lower.contains("secretary of state") || lower.contains("secrétaire d'état")
        || lower.contains("governor") || lower.contains("gouverneur") || lower.contains("والي")
        || lower.contains("ambassador") || lower.contains("ambassadeur") || lower.contains("سفير")
        || lower.contains("ministry of") || lower.contains("ministère de")
        || lower.contains("department of defense") || lower.contains("department of commerce")
        || lower.contains("director general") && (lower.contains("ministry") || lower.contains("defence") || lower.contains("defense"))
        || lower.contains("president of the") && (lower.contains("council") || lower.contains("region") || lower.contains("assembly"))
        || lower.contains("chairman of the") && lower.contains("commission")
        || lower.contains("delegate") && (lower.contains("national") || lower.contains("defense"))
        || lower.contains("federal") || lower.contains("congressional")
        || lower.contains("parliament") || lower.contains("senate")
        || lower.contains("public sector") || lower.contains("civil service")
    {
        return "Government";
    }
    
    // Military / Defense
    if lower.contains("general") && (lower.contains("army") || lower.contains("forces") || lower.contains("military") || lower.contains("command"))
        || lower.contains("admiral") || lower.contains("colonel") || lower.contains("brigadier")
        || lower.contains("defense attaché") || lower.contains("military attaché")
        || lower.contains("chief of staff") && lower.contains("armed")
    {
        return "Military";
    }
    
    // Regulatory / Compliance
    if lower.contains("regulator") || lower.contains("compliance officer")
        || lower.contains("inspector general") || lower.contains("audit")
        || lower.contains("customs") || lower.contains("export control")
    {
        return "Regulatory";
    }
    
    // ═══════════════════════════════════════════════════════════════════════════
    // PRIVATE SECTOR ROLES
    // ═══════════════════════════════════════════════════════════════════════════
    if lower.contains("engineer") || lower.contains("tech") || lower.contains("r&d")
        || lower.contains("developer") || lower.contains("architect") 
    {
        "Engineering"
    } else if lower.contains("sales") || lower.contains("commercial") || lower.contains("business dev")
        || lower.contains("account") || lower.contains("customer success")
    {
        "Sales"
    } else if lower.contains("supply") || lower.contains("procurement") || lower.contains("sourcing")
        || lower.contains("purchasing") || lower.contains("buyer")
    {
        "Supply Chain"
    } else if lower.contains("quality") || lower.contains("qa ") || lower.contains("qc ")
        || lower.contains("test") || lower.contains("assurance")
    {
        "Quality"
    } else if lower.contains("finance") || lower.contains("cfo") || lower.contains("controller")
        || lower.contains("treasurer") || lower.contains("accounting") || lower.contains("investor")
    {
        "Finance"
    } else if lower.contains("manufactur") || lower.contains("production") || lower.contains("operations")
        || lower.contains("plant") || lower.contains("factory") || lower.contains("site")
    {
        "Operations"
    } else if lower.contains("human resource") || lower.contains("hr ") || lower.starts_with("hr")
        || lower.contains("talent") || lower.contains("people")
    {
        "Human Resources"
    } else if lower.contains("legal") || lower.contains("counsel") || lower.contains("attorney")
        || lower.contains("compliance") && !lower.contains("export")
    {
        "Legal"
    } else if lower.contains("marketing") || lower.contains("brand") || lower.contains("communication")
        || lower.contains("pr ") || lower.contains("public relations")
    {
        "Marketing"
    } else if lower.contains("research") || lower.contains("scientist") || lower.contains("phd")
        || lower.contains("professor") || lower.contains("academic")
    {
        "Research"
    } else if lower.contains("security") || lower.contains("cyber") || lower.contains("ciso")
        || lower.contains("information security")
    {
        "Security"
    } else if lower.contains("strategy") || lower.contains("business planning") || lower.contains("m&a")
        || lower.contains("transformation") || lower.contains("corporate development")
    {
        "Strategy"
    } else {
        "General Management"
    }
}

/// Extract government/ministry affiliation from a role title
pub fn extract_government_affiliation(title: &str) -> Option<String> {
    let lower = title.to_lowercase();
    
    // Pattern: "Minister of X" → "Ministry of X"
    if lower.contains("minister of ") {
        if let Some(idx) = lower.find("minister of ") {
            let rest = title.get(idx + 12..)?;
            if let Some(end) = rest.find(|c: char| c == ',' || c == '.' || c == '\n') {
                return Some(format!("Ministry of {}", rest.get(..end).unwrap_or(rest).trim()));
            } else {
                return Some(format!("Ministry of {}", rest.trim()));
            }
        }
    }
    
    // Pattern: "Ministry of X" directly mentioned
    if lower.contains("ministry of ") {
        if let Some(idx) = lower.find("ministry of ") {
            let rest = title.get(idx..)?;
            if let Some(body) = rest.get(12..) {
                if let Some(end) = body.find(|c: char| c == ',' || c == '.' || c == '\n') {
                    return Some(rest.get(..12 + end).unwrap_or(rest).trim().to_string());
                }
            } else {
                return Some(rest.trim().to_string());
            }
            return Some(rest.trim().to_string());
        }
    }
    
    // Pattern: "Governor of X" → "Government of X (Regional)"
    if lower.contains("governor of ") || lower.contains("gouverneur de ") {
        if let Some((idx, offset)) = lower
            .find("governor of ")
            .map(|idx| (idx, 12))
            .or_else(|| lower.find("gouverneur de ").map(|idx| (idx, 14)))
        {
            let rest = title.get(idx + offset..)?;
            if let Some(end) = rest.find(|c: char| c == ',' || c == '.' || c == '\n') {
                return Some(format!("Regional Government of {}", rest.get(..end).unwrap_or(rest).trim()));
            } else {
                return Some(format!("Regional Government of {}", rest.trim()));
            }
        }
    }
    
    // Pattern: "President, X Regional Council" → "X Regional Council"
    if lower.contains("regional council") || lower.contains("conseil régional") {
        if let Some(idx) = lower.find("regional council").or_else(|| lower.find("conseil régional")) {
            // Look backwards for the region name
            let before = title.get(..idx).unwrap_or(title);
            if let Some(comma) = before.rfind(',') {
                return Some(format!("{} Regional Council", before[comma + 1..].trim()));
            }
        }
    }
    
    // Pattern: "Department of X"
    if lower.contains("department of ") {
        if let Some(idx) = lower.find("department of ") {
            let rest = title.get(idx..)?;
            if let Some(body) = rest.get(14..) {
                if let Some(end) = body.find(|c: char| c == ',' || c == '.' || c == '\n') {
                    return Some(rest.get(..14 + end).unwrap_or(rest).trim().to_string());
                }
            } else {
                return Some(rest.trim().to_string());
            }
            return Some(rest.trim().to_string());
        }
    }
    
    None
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
        assert_eq!(detect_seniority_from_title("Director and CTO"), "C-Level");
        assert_eq!(detect_seniority_from_title("Head of Quality"), "Manager");
        assert_eq!(detect_seniority_from_title("Founder"), "Executive");
        assert_eq!(detect_seniority_from_title("Senior VP"), "VP"); // Regression: VP at end of title
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
