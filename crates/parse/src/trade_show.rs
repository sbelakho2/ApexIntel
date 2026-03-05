use std::sync::LazyLock;

use chrono::{DateTime, Utc};
use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};

use crate::normalizer;
use apex_core::validation::normalize_url;

static RE_LOCATION: LazyLock<Regex> = LazyLock::new(|| {
    RegexBuilder::new(r"(?i)(?:venue|location|lieu|held at|held in)[:\s]+([^\n.;]+)")
        .size_limit(200_000)
        .dfa_size_limit(200_000)
        .build()
        .unwrap()
});

static DATE_RANGE_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r"(?i)(\d{1,2}[-–]\d{1,2}\s+(?:January|February|March|April|May|June|July|August|September|October|November|December)\s+\d{4})",
        r"(?i)((?:January|February|March|April|May|June|July|August|September|October|November|December)\s+\d{1,2}[-–]\d{1,2},?\s+\d{4})",
        r"(\d{4}[-/]\d{2}[-/]\d{2}\s*(?:to|[-–])\s*\d{4}[-/]\d{2}[-/]\d{2})",
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

static RE_BOOTH: LazyLock<Regex> = LazyLock::new(|| {
    RegexBuilder::new(r"(?i)(?:booth|stand|kiosk)[:\s]*([A-Z0-9][\w\-]+)")
        .size_limit(200_000)
        .dfa_size_limit(200_000)
        .build()
        .unwrap()
});

static RE_HALL: LazyLock<Regex> = LazyLock::new(|| {
    RegexBuilder::new(r"(?i)(?:hall|pavilion|halle)[:\s]*(\w+)")
        .size_limit(200_000)
        .dfa_size_limit(200_000)
        .build()
        .unwrap()
});

static RE_COUNTRY: LazyLock<Regex> = LazyLock::new(|| {
    RegexBuilder::new(r"(?i)(?:country|pays)[:\s]+(\w[\w\s]+)")
        .size_limit(200_000)
        .dfa_size_limit(200_000)
        .build()
        .unwrap()
});

static RE_SPEAKERS: LazyLock<Regex> = LazyLock::new(|| {
    // Use (?-i:...) around the name capture so that [A-Z] / [a-z] retain
    // their case-sensitivity — otherwise the outer (?i) makes them
    // equivalent and any word sequence matches as a "name".
    RegexBuilder::new(
        r"(?i)(?:speaker|keynote|presenter|panelist)[:\s]+(?-i:([A-Z][a-z]+(?:\s[A-Z][a-z]+)+))(?:\s*,\s*(.+?))?(?:\s+(?:at|from|of)\s+(.+?))?(?:\.|$|\n)"
    )
    .size_limit(200_000)
    .dfa_size_limit(200_000)
    .build()
    .unwrap()
});

/// Patterns that introduce a session/presentation topic near a speaker mention.
static RE_TOPIC: LazyLock<Regex> = LazyLock::new(|| {
    RegexBuilder::new(
        r#"(?i)(?:speaking\s+on|presenting|session[:\s]+|topic[:\s]+|talk[:\s]+|presentation[:\s]+)["\u2018\u201c]?([^\n"\u2019\u201d.;]{5,120})["\u2019\u201d]?"#
    )
    .size_limit(200_000)
    .dfa_size_limit(200_000)
    .build()
    .unwrap()
});

/// Extracted exhibitor or speaker from a trade show / conference.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExhibitorExtract {
    pub name: String,
    pub booth: Option<String>,
    pub hall: Option<String>,
    pub country: Option<String>,
    pub category: Option<String>,
    pub website: Option<String>,
    pub description: String,
}

/// Extracted trade show / conference event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeShowExtract {
    pub event_name: String,
    pub location: Option<String>,
    pub date_range: Option<String>,
    pub exhibitors: Vec<ExhibitorExtract>,
    pub speakers: Vec<SpeakerExtract>,
    pub url: String,
    pub extracted_at: DateTime<Utc>,
}

/// Extracted conference speaker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpeakerExtract {
    pub name: String,
    pub title: Option<String>,
    pub company: Option<String>,
    pub topic: Option<String>,
}

/// Extract trade show information from page text.
pub fn extract_trade_show(
    body_text: &str,
    title: &str,
    url: &str,
) -> TradeShowExtract {
    let normalized_body = normalizer::normalize_whitespace(body_text);
    let location = extract_location(&normalized_body);
    let date_range = extract_date_range(&normalized_body);
    let exhibitors = extract_exhibitors(&normalized_body);
    let speakers = extract_speakers(&normalized_body);
    let normalized_url = normalize_url(url).unwrap_or_else(|| url.to_string());

    TradeShowExtract {
        event_name: normalizer::normalize_whitespace(title),
        location,
        date_range,
        exhibitors,
        speakers,
        url: normalized_url,
        extracted_at: Utc::now(),
    }
}

fn extract_location(text: &str) -> Option<String> {
    RE_LOCATION.captures(text)
        .map(|c| normalizer::normalize_whitespace(c.get(1).unwrap().as_str()))
}

fn extract_date_range(text: &str) -> Option<String> {
    // Pattern: "January 15-17, 2025" or "15-17 March 2025" or "2025-03-15 to 2025-03-17"
    for re in DATE_RANGE_PATTERNS.iter() {
        if let Some(m) = re.find(text) {
            let raw = normalizer::normalize_whitespace(m.as_str());
            if normalizer::is_valid_date_range(&raw) {
                return Some(raw);
            }
            continue;
        }
    }
    None
}

/// Extract exhibitors from a structured exhibitor list.
pub fn extract_exhibitors(text: &str) -> Vec<ExhibitorExtract> {
    let mut exhibitors = Vec::new();

    // Pattern: "Company Name - Booth A123 - Hall 5 - Country"

    // Try line-by-line extraction for exhibitor lists
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.len() < 3 {
            continue;
        }

        // Lines with booth numbers are likely exhibitor entries
        let has_booth = RE_BOOTH.is_match(trimmed);

        if has_booth {
            let booth = RE_BOOTH.captures(trimmed)
                .map(|c| c.get(1).unwrap().as_str().to_string());

            let hall = RE_HALL.captures(trimmed)
                .map(|c| c.get(1).unwrap().as_str().to_string());

            let country = RE_COUNTRY.captures(trimmed)
                .map(|c| normalizer::normalize_whitespace(c.get(1).unwrap().as_str()));

            // Name is typically the first part before any delimiter
            let name = trimmed.split(&['-', '–', '|', ','][..])
                .next()
                .map(|s| normalizer::normalize_whitespace(s))
                .unwrap_or_default();

            if !name.is_empty() {
                exhibitors.push(ExhibitorExtract {
                    name,
                    booth,
                    hall,
                    country,
                    category: None,
                    website: None,
                    description: normalizer::truncate(trimmed, 200),
                });
            }
        }
    }

    exhibitors
}

/// Extract speakers from conference programs.
pub fn extract_speakers(text: &str) -> Vec<SpeakerExtract> {
    let mut speakers = Vec::new();

    // Pattern: "Name, Title at Company" or "Name (Company)"
    for caps in RE_SPEAKERS.captures_iter(text) {
        let name = normalizer::normalize_whitespace(caps.get(1).unwrap().as_str());
        let title = caps.get(2).map(|m| normalizer::normalize_whitespace(m.as_str()));
        let company = caps.get(3).map(|m| normalizer::normalize_whitespace(m.as_str()));
        let topic = extract_speaker_topic(text, &name);

        speakers.push(SpeakerExtract {
            name,
            title,
            company,
            topic,
        });
    }

    speakers
}

/// Look for a session topic within the same paragraph or nearby lines as the
/// speaker's name.  Returns the trimmed topic string if found.
fn extract_speaker_topic(text: &str, name: &str) -> Option<String> {
    // Grab up to 300 chars of context after the name's first occurrence.
    let start = text.find(name)?;
    let context_end = (start + 300).min(text.len());
    let context = &text[start..context_end];

    RE_TOPIC.captures(context)
        .and_then(|c| c.get(1))
        .map(|m| normalizer::normalize_whitespace(m.as_str()))
        .filter(|s| !s.is_empty())
}

/// Known EMS-relevant trade shows.
pub fn is_ems_trade_show(event_name: &str) -> bool {
    let lower = event_name.to_lowercase();
    let known_shows = [
        "productronica",
        "smtconnect",
        "smt connect",
        "ipc apex expo",
        "nepcon",
        "electronica",
        "semicon",
        "pcb expo",
        "jisso",
        "internepcon",
        "siane",
        "eletec",
        "elec expo",
        "sistep",
        "midest",
    ];
    known_shows.iter().any(|show| lower.contains(show))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_trade_show_basic() {
        let body = "IPC APEX Expo 2025. Venue: San Diego Convention Center. \
                    January 21-23, 2025. Over 400 exhibitors.";
        let show = extract_trade_show(body, "IPC APEX Expo 2025", "https://ipcapexexpo.org");

        assert_eq!(show.event_name, "IPC APEX Expo 2025");
        assert!(show.location.is_some());
        assert!(show.date_range.is_some());
    }

    #[test]
    fn test_extract_location() {
        let text = "Venue: Messe München, Munich, Germany";
        let loc = extract_location(text);
        assert!(loc.is_some());
        assert!(loc.unwrap().contains("München"));
    }

    #[test]
    fn test_extract_date_range() {
        assert!(extract_date_range("January 15-17, 2025").is_some());
        assert!(extract_date_range("15-17 March 2025").is_some());
        assert!(extract_date_range("2025-03-15 to 2025-03-17").is_some());
    }

    #[test]
    fn test_extract_exhibitors() {
        let text = "Exhibitor list:\n\
                    Foxconn - Booth A101 - Hall 5\n\
                    Jabil Inc - Booth B202 - Hall 3\n\
                    Starz Electronics - Booth C303 - Hall 1";
        let exhibitors = extract_exhibitors(text);
        assert_eq!(exhibitors.len(), 3);
        assert!(exhibitors[0].name.contains("Foxconn"));
        assert!(exhibitors[0].booth.is_some());
    }

    #[test]
    fn test_extract_speakers() {
        let text = "Speaker: John Smith, VP Engineering at Flex Ltd. \
                    Speaker: Marie Dupont, CTO at Starz Electronics.";
        let speakers = extract_speakers(text);
        assert_eq!(speakers.len(), 2);
        assert!(speakers[0].name.contains("John Smith"));
    }

    #[test]
    fn test_is_ems_trade_show() {
        assert!(is_ems_trade_show("IPC APEX Expo 2025"));
        assert!(is_ems_trade_show("Productronica Munich 2025"));
        assert!(is_ems_trade_show("SMTconnect Nuremberg"));
        assert!(is_ems_trade_show("NEPCON Japan 2025"));
        assert!(!is_ems_trade_show("CES 2025"));
        assert!(!is_ems_trade_show("Mobile World Congress"));
    }

    #[test]
    fn test_exhibitor_booth_parsing() {
        let text = "Acme Corp - Booth F42 - Hall 2 - Country: Germany";
        let exhibitors = extract_exhibitors(text);
        assert_eq!(exhibitors.len(), 1);
        let ex = &exhibitors[0];
        assert_eq!(ex.booth.as_deref(), Some("F42"));
        assert_eq!(ex.hall.as_deref(), Some("2"));
    }

    #[test]
    fn test_ems_relevant_tunisian_shows() {
        assert!(is_ems_trade_show("ELETEC Tunis 2025"));
        assert!(is_ems_trade_show("Elec Expo Casablanca"));
        assert!(is_ems_trade_show("SIANE Toulouse 2025"));
    }
}
