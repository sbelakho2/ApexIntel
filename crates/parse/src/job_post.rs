use std::sync::LazyLock;

use chrono::{DateTime, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};

use apex_core::entities::RoleFamily;
use crate::normalizer;
use apex_core::validation::normalize_url;

static RE_JOB_LOCATION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:location|lieu|ville|city|based in)[:\s]+([A-Za-z\u{00c0}-\u{00ff}\s,]+)").unwrap()
});

static RE_SALARY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:salary|compensation|r\u{00e9}mun\u{00e9}ration)[:\s]*([^\n.]+)").unwrap()
});

/// og:site_name — both attribute orderings.
static RE_OG_SITE_NAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)(?:property=["']og:site_name["'][^>]*content=["']([^"']+)["']|content=["']([^"']+)["'][^>]*property=["']og:site_name["'])"#).unwrap()
});

/// JSON-LD "name" inside an Organization/JobPosting node.
static RE_JSONLD_ORG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)["'](?:@type)["']\s*:\s*["'](?:Organization|JobPosting|EmployerAggregateRating)["'][^}]*?["']name["']\s*:\s*["']([^"']{2,80})["']"#).unwrap()
});

/// Extracted job posting data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobPosting {
    pub title: String,
    pub company_name: Option<String>,
    pub location: Option<String>,
    pub role_family: RoleFamily,
    pub seniority: Seniority,
    pub salary_range: Option<String>,
    pub keywords: Vec<String>,
    pub source_url: String,
    pub extracted_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Seniority {
    Junior,
    Mid,
    Senior,
    Lead,
    Director,
    VP,
    CLevel,
    Unknown,
}

impl Seniority {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Junior => "junior",
            Self::Mid => "mid",
            Self::Senior => "senior",
            Self::Lead => "lead",
            Self::Director => "director",
            Self::VP => "vp",
            Self::CLevel => "c_level",
            Self::Unknown => "unknown",
        }
    }
}

/// Classify a job title into role family.
pub fn classify_role_family(title: &str) -> RoleFamily {
    let lower = title.to_lowercase();

    // Procurement
    if lower.contains("procurement")
        || lower.contains("buyer")
        || lower.contains("sourcing")
        || lower.contains("purchasing")
        || lower.contains("supply chain")
        || lower.contains("achats")
        || lower.contains("approvisionnement")
    {
        return RoleFamily::Procurement;
    }

    // Quality
    if lower.contains("quality")
        || lower.contains("sqe")
        || lower.contains("qms")
        || lower.contains("inspection")
        || lower.contains("audit")
        || lower.contains("qualité")
    {
        return RoleFamily::Quality;
    }

    // Engineering
    if lower.contains("engineer")
        || lower.contains("design")
        || lower.contains("r&d")
        || lower.contains("npi")
        || lower.contains("process engineer")
        || lower.contains("ingénieur")
    {
        return RoleFamily::Engineering;
    }

    // Operations
    if lower.contains("operations")
        || lower.contains("manufacturing")
        || lower.contains("production")
        || lower.contains("plant manager")
        || lower.contains("fabrication")
    {
        return RoleFamily::Operations;
    }

    // Security
    if lower.contains("security")
        || lower.contains("cyber")
        || lower.contains("infosec")
        || lower.contains("sécurité")
    {
        return RoleFamily::Security;
    }

    // Executive
    if lower.contains("ceo")
        || lower.contains("cfo")
        || lower.contains("cto")
        || lower.contains("coo")
        || lower.contains("president")
        || lower.contains("directeur général")
    {
        return RoleFamily::Executive;
    }

    // Government
    if lower.contains("government")
        || lower.contains("regulatory")
        || lower.contains("compliance officer")
        || lower.contains("réglementaire")
    {
        return RoleFamily::Government;
    }

    // Logistics
    if lower.contains("logistics")
        || lower.contains("warehouse")
        || lower.contains("shipping")
        || lower.contains("transport")
        || lower.contains("logistique")
    {
        return RoleFamily::Logistics;
    }

    // Finance
    if lower.contains("finance")
        || lower.contains("accounting")
        || lower.contains("controller")
        || lower.contains("financ")
    {
        return RoleFamily::Finance;
    }

    RoleFamily::Other("Unknown".to_string())
}

/// Detect seniority from title.
pub fn detect_seniority(title: &str) -> Seniority {
    let lower = title.to_lowercase();

    // C-level must be checked before Director: a title like "Director and CTO"
    // should classify as CLevel (higher seniority).
    if lower.contains("chief") || lower == "ceo" || lower == "cfo"
        || lower == "cto" || lower == "coo" || lower == "cpo"
        || lower.starts_with("ceo ") || lower.starts_with("cfo ")
        || lower.starts_with("cto ") || lower.starts_with("coo ")
        || lower.starts_with("cpo ")
        || lower.contains(" ceo") || lower.contains(" cfo")
        || lower.contains(" cto") || lower.contains(" coo")
    {
        return Seniority::CLevel;
    }
    if lower.contains("director") || lower.contains("directeur") {
        return Seniority::Director;
    }
    if lower.contains("vice president") || lower.contains("vp ") || lower == "vp"
        || lower.starts_with("vp ") || lower.starts_with("vp-") || lower.starts_with("vp,")
        || lower.contains(" vp,") || lower.ends_with(" vp")
    {
        return Seniority::VP;
    }
    if lower.contains("lead") || lower.contains("principal") || lower.contains("head of") {
        return Seniority::Lead;
    }
    if lower.contains("senior") || lower.contains("sr.") || lower.contains("sr ") {
        return Seniority::Senior;
    }
    if lower.contains("junior") || lower.contains("jr.") || lower.contains("jr ")
        || lower.contains("intern") || lower.contains("trainee")
    {
        return Seniority::Junior;
    }

    Seniority::Mid
}

/// Extract a job posting from HTML page content.
pub fn extract_job_posting(
    body_text: &str,
    title: &str,
    source_url: &str,
) -> JobPosting {
    let normalized_body = normalizer::normalize_whitespace(body_text);
    let role_family = classify_role_family(title);
    let seniority = detect_seniority(title);

    // Extract location patterns
    let location = extract_location(&normalized_body);
    let salary_range = extract_salary(&normalized_body);

    // Keyword extraction
    let ems_kws = crate::multilingual::ems_keywords("en");
    let cert_kws = crate::multilingual::certification_keywords("en");
    let all_kws: Vec<&str> = ems_kws.into_iter().chain(cert_kws.into_iter()).collect();
    let keywords = crate::multilingual::contains_keywords(&normalized_body, &all_kws);
    let normalized_url = normalize_url(source_url).unwrap_or_else(|| source_url.to_string());

    JobPosting {
        title: normalizer::normalize_whitespace(title),
        company_name: extract_company_name(&normalized_body, source_url),
        location,
        role_family,
        seniority,
        salary_range,
        keywords,
        source_url: normalized_url,
        extracted_at: Utc::now(),
    }
}

fn extract_location(text: &str) -> Option<String> {
    RE_JOB_LOCATION.captures(text)
        .map(|c| normalizer::normalize_whitespace(c.get(1).unwrap().as_str()))
}

/// Attempt to extract company name from HTML metadata, JSON-LD, or the URL domain.
fn extract_company_name(body_text: &str, source_url: &str) -> Option<String> {
    // 1. Try og:site_name meta tag.
    if let Some(caps) = RE_OG_SITE_NAME.captures(body_text) {
        let name = caps.get(1).or_else(|| caps.get(2))
            .map(|m| m.as_str().trim().to_string())
            .filter(|s| !s.is_empty());
        if name.is_some() {
            return name;
        }
    }

    // 2. Try JSON-LD Organization/JobPosting "name".
    if let Some(caps) = RE_JSONLD_ORG.captures(body_text) {
        let name = caps.get(1)
            .map(|m| m.as_str().trim().to_string())
            .filter(|s| !s.is_empty());
        if name.is_some() {
            return name;
        }
    }

    // 3. Fall back to second-level domain of the URL, title-cased.
    extract_domain_company(source_url)
}

/// Extract and title-case a company hint from the URL's second-level domain.
/// Strips common TLDs, suffixes (inc, corp, ltd, llc), and hyphens.
fn extract_domain_company(url: &str) -> Option<String> {
    // Find the host part between :// and the next /
    let after_scheme = url.split("://").nth(1)?;
    let host = after_scheme.split('/').next()?;
    // Remove port and leading www.
    let host = host.split(':').next().unwrap_or(host);
    let host = host.strip_prefix("www.").unwrap_or(host);

    // Extract the SLD (part before last dot).
    let sld = host.rsplit_once('.').map(|(prefix, _)| prefix).unwrap_or(host);
    // If there's still a dot (e.g. jobs.company), take the last component.
    let sld = sld.rsplit('.').next().unwrap_or(sld);

    let words: Vec<String> = sld.split(&['-', '_'][..])
        .filter(|w| !matches!(*w, "jobs" | "careers" | "hr" | "talent"))
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            }
        })
        .collect();

    if words.is_empty() { None } else { Some(words.join(" ")) }
}

fn extract_salary(text: &str) -> Option<String> {
    RE_SALARY.captures(text)
        .map(|c| normalizer::normalize_whitespace(c.get(1).unwrap().as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_procurement() {
        assert_eq!(classify_role_family("Senior Procurement Manager"), RoleFamily::Procurement);
        assert_eq!(classify_role_family("Global Sourcing Specialist"), RoleFamily::Procurement);
        assert_eq!(classify_role_family("Responsable Achats"), RoleFamily::Procurement);
    }

    #[test]
    fn test_classify_quality() {
        assert_eq!(classify_role_family("Supplier Quality Engineer"), RoleFamily::Quality);
        assert_eq!(classify_role_family("SQE Manager"), RoleFamily::Quality);
        assert_eq!(classify_role_family("Quality Audit Lead"), RoleFamily::Quality);
    }

    #[test]
    fn test_classify_engineering() {
        assert_eq!(classify_role_family("Process Engineer"), RoleFamily::Engineering);
        assert_eq!(classify_role_family("R&D Manager"), RoleFamily::Engineering);
        assert_eq!(classify_role_family("NPI Engineer"), RoleFamily::Engineering);
    }

    #[test]
    fn test_classify_operations() {
        assert_eq!(classify_role_family("Plant Manager"), RoleFamily::Operations);
        assert_eq!(classify_role_family("Manufacturing Supervisor"), RoleFamily::Operations);
    }

    #[test]
    fn test_classify_executive() {
        assert_eq!(classify_role_family("CEO"), RoleFamily::Executive);
        assert_eq!(classify_role_family("CFO and President"), RoleFamily::Executive);
    }

    #[test]
    fn test_classify_other() {
        assert_eq!(classify_role_family("Marketing Specialist"), RoleFamily::Other("Unknown".to_string()));
    }

    #[test]
    fn test_detect_seniority_clevel() {
        assert_eq!(detect_seniority("Chief Technology Officer"), Seniority::CLevel);
        assert_eq!(detect_seniority("CEO"), Seniority::CLevel);
    }

    #[test]
    fn test_detect_seniority_senior() {
        assert_eq!(detect_seniority("Senior Engineer"), Seniority::Senior);
        assert_eq!(detect_seniority("Sr. Quality Manager"), Seniority::Senior);
    }

    #[test]
    fn test_detect_seniority_director() {
        assert_eq!(detect_seniority("Director of Operations"), Seniority::Director);
    }

    #[test]
    fn test_detect_seniority_mid() {
        assert_eq!(detect_seniority("Quality Engineer"), Seniority::Mid);
    }

    #[test]
    fn test_extract_job_posting() {
        let body = "Location: Sousse, Tunisia. We are looking for a Senior Quality Engineer with IATF 16949 and SMT experience.";
        let posting = extract_job_posting(body, "Senior Quality Engineer", "https://jobs.example.com/123");

        assert_eq!(posting.role_family, RoleFamily::Quality);
        assert_eq!(posting.seniority, Seniority::Senior);
        assert!(posting.location.is_some());
        assert!(posting.keywords.iter().any(|k| k == "SMT"));
    }

    #[test]
    fn test_extract_location() {
        let text = "Location: Tunis, Tunisia. Great opportunity.";
        let loc = extract_location(text);
        assert!(loc.is_some());
        assert!(loc.unwrap().contains("Tunis"));
    }

    #[test]
    fn test_extract_salary() {
        let text = "Salary: 50,000 - 70,000 TND per year.";
        let sal = extract_salary(text);
        assert!(sal.is_some());
    }
}
