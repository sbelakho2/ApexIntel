use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

use regex::{Regex, RegexBuilder};

use crate::normalizer;
use apex_core::validation::normalize_url;

/// Extracted member from a business/industry directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectoryMemberExtract {
    /// Name of the company/organization
    pub name: String,
    /// Member category (e.g., "Gold Member", "Platinum Sponsor", "Corporate Member")
    pub category: Option<String>,
    /// Geographic location if mentioned
    pub location: Option<String>,
    /// Industry/sector if mentioned
    pub industry: Option<String>,
    /// Website URL if provided
    pub website: Option<String>,
    /// Description/role in directory
    pub description: String,
}

/// Extracted directory (business directory, chamber of commerce, industry association, etc.).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectoryExtract {
    /// Directory name (e.g., "IPC Member Directory", "Chamber of Commerce Business List")
    pub directory_name: String,
    /// Directory type (e.g., "chamber_of_commerce", "industry_association", "trade_directory")
    pub directory_type: String,
    /// Geographic focus of the directory
    pub geographic_focus: Option<String>,
    /// List of member organizations
    pub members: Vec<DirectoryMemberExtract>,
    /// Source URL where directory was found
    pub url: String,
    /// When this extraction was performed
    pub extracted_at: chrono::DateTime<chrono::Utc>,
}

/// Patterns for member entries in directories
static RE_MEMBER_ENTRY: LazyLock<Regex> = LazyLock::new(|| {
    RegexBuilder::new(r"(?m)^\s*(?:[•\-*]|\d+\.)\s+([^\n]+?)(?:\s+-\s+([^\n]+?))?(?:\s+\((?:location|based in|headquartered in):\s*([^)]+)\))?(?:\s+\[([^\]]+)\])?$")
        .size_limit(200_000)
        .dfa_size_limit(200_000)
        .build()
        .unwrap_or_else(|error| panic!("invalid directory member-entry regex: {error}"))
});

/// Patterns for member category detection
static RE_MEMBER_CATEGORY: LazyLock<Regex> = LazyLock::new(|| {
    RegexBuilder::new(r"(?i)(?:gold|platinum|silver|bronze|corporate|sustaining|executive|founding)\s+(?:member|sponsor|partner)")
        .size_limit(100_000)
        .dfa_size_limit(100_000)
        .build()
        .unwrap_or_else(|error| panic!("invalid directory member-category regex: {error}"))
});

/// Patterns for website extraction
static RE_WEBSITE: LazyLock<Regex> = LazyLock::new(|| {
    RegexBuilder::new(r"(?i)(?:website|site|web):?\s*(https?://[^\s]+|www\.[^\s]+)")
        .size_limit(100_000)
        .dfa_size_limit(100_000)
        .build()
        .unwrap_or_else(|error| panic!("invalid directory website regex: {error}"))
});

/// Extract directory information from page text.
pub fn extract_directory(body_text: &str, title: &str, url: &str) -> DirectoryExtract {
    let normalized_body = normalizer::normalize_whitespace(body_text);
    let normalized_url = normalize_url(url).unwrap_or_else(|| url.to_string());

    // Determine directory type based on content
    let directory_type = if title.to_lowercase().contains("chamber of commerce") {
        "chamber_of_commerce".to_string()
    } else if title.to_lowercase().contains("industry association")
        || title.to_lowercase().contains("trade association")
    {
        "industry_association".to_string()
    } else if title.to_lowercase().contains("member directory") {
        "member_directory".to_string()
    } else if title.to_lowercase().contains("business directory") {
        "business_directory".to_string()
    } else {
        "trade_directory".to_string()
    };

    // Extract geographic focus from title or body
    let geographic_focus = extract_geographic_focus(title, &normalized_body);

    // Extract members
    let members = extract_members(body_text);

    DirectoryExtract {
        directory_name: normalizer::normalize_whitespace(title),
        directory_type,
        geographic_focus,
        members,
        url: normalized_url,
        extracted_at: chrono::Utc::now(),
    }
}

/// Extract geographic focus from directory content
fn extract_geographic_focus(title: &str, body: &str) -> Option<String> {
    // Common geographic patterns
    let patterns = [
        r"(?:based in|headquartered in|located in)\s+([A-Z][a-zA-Z\s,]+)",
        r"(?:serving|covering)\s+([A-Z][a-zA-Z\s,]+)",
        r"(?:chamber of commerce of|industry association of)\s+([A-Z][a-zA-Z\s,]+)",
    ];

    let text = format!("{} {}", title, body);

    for pattern in &patterns {
        if let Ok(re) = RegexBuilder::new(pattern)
            .size_limit(100_000)
            .dfa_size_limit(100_000)
            .build()
        {
            if let Some(caps) = re.captures(&text) {
                if let Some(matched) = caps.get(1) {
                    let location = normalizer::normalize_whitespace(matched.as_str());
                    if !location.is_empty() {
                        return Some(location);
                    }
                }
            }
        }
    }

    None
}

/// Extract member organizations from directory text
fn extract_members(text: &str) -> Vec<DirectoryMemberExtract> {
    let mut members = Vec::new();

    // Try structured list patterns first
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.len() < 3 {
            continue;
        }

        // Check if line looks like a member entry
        if RE_MEMBER_ENTRY.is_match(trimmed) {
            if let Some(caps) = RE_MEMBER_ENTRY.captures(trimmed) {
                let name = caps
                    .get(1)
                    .map(|m| normalizer::normalize_whitespace(m.as_str()))
                    .unwrap_or_default();

                if name.is_empty() {
                    continue;
                }

                let description = caps
                    .get(2)
                    .map(|m| normalizer::normalize_whitespace(m.as_str()))
                    .unwrap_or_default();

                let location = caps
                    .get(3)
                    .map(|m| normalizer::normalize_whitespace(m.as_str()));

                let category_raw = caps.get(4).map(|m| m.as_str().to_string());

                // Extract category from description or category field
                let category = category_raw.or_else(|| {
                    if RE_MEMBER_CATEGORY.is_match(&description) {
                        RE_MEMBER_CATEGORY
                            .find(&description)
                            .map(|m| m.as_str().to_string())
                    } else {
                        None
                    }
                });

                // Extract industry hints from description
                let industry = extract_industry_hint(&description);

                // Extract website from description
                let website = RE_WEBSITE
                    .captures(&description)
                    .and_then(|caps| caps.get(1))
                    .map(|m| {
                        let url = m.as_str().to_string();
                        if url.starts_with("www.") {
                            format!("https://{}", url)
                        } else {
                            url
                        }
                    });

                members.push(DirectoryMemberExtract {
                    name,
                    category,
                    location,
                    industry,
                    website,
                    description: if description.is_empty() {
                        "Directory member".to_string()
                    } else {
                        description
                    },
                });
            }
        }
    }

    // If no structured entries found, try to extract company names from paragraphs
    if members.is_empty() {
        extract_companies_from_text(text, &mut members);
    }

    members
}

/// Extract company names from free text using heuristics
fn extract_companies_from_text(text: &str, members: &mut Vec<DirectoryMemberExtract>) {
    // Simple heuristic: lines with capitalized multi-word phrases that look like company names
    let lines: Vec<&str> = text.lines().collect();

    for line in lines {
        let trimmed = line.trim();
        if trimmed.len() < 4 || trimmed.len() > 120 {
            continue;
        }

        // Skip lines that are clearly not company names
        if trimmed.starts_with("•") || trimmed.starts_with("-") || trimmed.starts_with("*") {
            continue;
        }

        // Check if line looks like a company name (capitalized words, no sentence-ending punctuation)
        let words: Vec<&str> = trimmed.split_whitespace().collect();
        if !words.is_empty() && words.len() <= 5 {
            let first_word = words[0];
            if first_word
                .chars()
                .next()
                .map(|c| c.is_uppercase())
                .unwrap_or(false)
            {
                // Check if most words are capitalized (company name pattern)
                let capitalized_count = words
                    .iter()
                    .filter(|w| w.chars().next().map(|c| c.is_uppercase()).unwrap_or(false))
                    .count();

                if capitalized_count as f32 / words.len() as f32 > 0.7 {
                    let name = trimmed.to_string();
                    // Skip obvious non-company names
                    if !is_likely_company_name(&name) {
                        continue;
                    }

                    members.push(DirectoryMemberExtract {
                        name,
                        category: None,
                        location: None,
                        industry: None,
                        website: None,
                        description: "Extracted from directory text".to_string(),
                    });
                }
            }
        }
    }
}

/// Industry hint extraction from description
fn extract_industry_hint(description: &str) -> Option<String> {
    let industries = [
        ("electronics", "Electronics"),
        ("semiconductor", "Semiconductor"),
        ("manufacturing", "Manufacturing"),
        ("engineering", "Engineering"),
        ("technology", "Technology"),
        ("defense", "Defense"),
        ("aerospace", "Aerospace"),
        ("automotive", "Automotive"),
        ("medical", "Medical"),
        ("pharmaceutical", "Pharmaceutical"),
        ("energy", "Energy"),
        ("telecommunications", "Telecommunications"),
        ("software", "Software"),
        ("consulting", "Consulting"),
        ("logistics", "Logistics"),
        ("construction", "Construction"),
    ];

    let lower_desc = description.to_lowercase();
    for (keyword, industry) in industries.iter() {
        if lower_desc.contains(keyword) {
            return Some(industry.to_string());
        }
    }

    None
}

/// Check if a string looks like a company name (not a person, not generic)
fn is_likely_company_name(name: &str) -> bool {
    let lower = name.to_lowercase();

    // Skip common non-company patterns
    let skip_patterns = [
        "home",
        "about",
        "contact",
        "login",
        "sign up",
        "register",
        "search",
        "privacy policy",
        "terms of service",
        "cookie policy",
        "sitemap",
        "back to top",
        "all rights reserved",
        "copyright",
    ];

    if skip_patterns.iter().any(|p| lower.contains(p)) {
        return false;
    }

    // Company names often have indicators — check these first before person heuristic
    let company_indicators = [
        "inc",
        "corp",
        "corporation",
        "ltd",
        "limited",
        "llc",
        "gmbh",
        "co",
        "company",
        "group",
        "holdings",
        "technologies",
        "solutions",
        "systems",
        "international",
        "global",
        "industries",
        "enterprises",
    ];

    if company_indicators
        .iter()
        .any(|indicator| lower.ends_with(indicator) || lower.contains(&format!(" {}", indicator)))
    {
        return true;
    }

    // Skip person-like names (usually 2-3 words all capitalized, common first/last names)
    let words: Vec<&str> = name.split_whitespace().collect();
    if words.len() == 2 || words.len() == 3 {
        // Check if it looks like "John Smith" or "John A. Smith"
        let looks_like_person = words.iter().all(|w| {
            w.chars().next().map(|c| c.is_uppercase()).unwrap_or(false)
                && w.chars().all(|c| c.is_alphabetic() || c == '.')
        });

        if looks_like_person {
            return false;
        }
    }

    // Default to true for now - let other filters handle it
    true
}

/// Check if text appears to contain directory content
pub fn is_directory_content(title: &str, body_text: &str) -> bool {
    let normalized_title = title.to_lowercase();
    let normalized_body = body_text.to_lowercase();

    // Title patterns
    let title_patterns = [
        "member directory",
        "business directory",
        "chamber of commerce",
        "industry association",
        "trade association",
        "corporate members",
        "partner list",
        "sponsor directory",
    ];

    if title_patterns.iter().any(|p| normalized_title.contains(p)) {
        return true;
    }

    // Body content patterns
    let body_patterns = [
        "list of members",
        "our members include",
        "corporate membership",
        "gold member",
        "platinum sponsor",
        "chamber members",
        "association members",
    ];

    if body_patterns.iter().any(|p| normalized_body.contains(p)) {
        return true;
    }

    // Check for structured list patterns
    let lines: Vec<&str> = body_text.lines().collect();
    let list_like_lines = lines
        .iter()
        .filter(|line| {
            let trimmed = line.trim();
            (trimmed.starts_with("•")
                || trimmed.starts_with("-")
                || trimmed.starts_with("*")
                || trimmed.starts_with("1.")
                || trimmed.starts_with("2.")
                || trimmed.starts_with("3."))
                && trimmed.len() > 10
        })
        .count();

    list_like_lines >= 3
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_directory_content_positive() {
        let title = "IPC Member Directory";
        let body = "Our members include:\n• Company A - Electronics manufacturer\n• Company B - Semiconductor solutions";

        assert!(is_directory_content(title, body));
    }

    #[test]
    fn test_is_directory_content_negative() {
        let title = "News Article";
        let body = "Today the market moved up. Investors are happy.";

        assert!(!is_directory_content(title, body));
    }

    #[test]
    fn test_extract_directory_basic() {
        let body = "Chamber of Commerce Member Directory\n\nMembers:\n• Acme Corp - Manufacturing solutions\n• Beta Technologies - Electronics design";
        let extract = extract_directory(
            body,
            "Local Chamber of Commerce Directory",
            "https://example.com/directory",
        );

        assert_eq!(extract.directory_type, "chamber_of_commerce");
        assert!(extract.members.len() >= 2);
        assert!(extract.members.iter().any(|m| m.name.contains("Acme")));
    }

    #[test]
    fn test_extract_members_structured() {
        let text = "• NVIDIA Corporation - Semiconductor and AI solutions (location: Santa Clara, CA)\n• TSMC - Semiconductor foundry [Gold Member]";
        let members = extract_members(text);

        assert_eq!(members.len(), 2);
        assert!(members.iter().any(|m| m.name == "NVIDIA Corporation"));
        assert!(members
            .iter()
            .any(|m| m.category.as_deref() == Some("Gold Member")));
    }

    #[test]
    fn test_is_likely_company_name() {
        assert!(is_likely_company_name("NVIDIA Corporation"));
        assert!(is_likely_company_name("Acme Technologies LLC"));
        assert!(!is_likely_company_name("John Smith"));
        assert!(!is_likely_company_name("Privacy Policy"));
    }
}
