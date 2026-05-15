//! Trade show calendar integration.
//!
//! Parses ICS/iCal feeds and static event databases for major electronics
//! industry events. Generates "event proximity" signals when tracked POIs
//! or competitors are exhibiting.
//!
//! Covered events: Electronica, IPC APEX, PCIM, GITEX, NEPCON, SEMICON,
//! CEATEC, productronica, SMT Hybrid, InnoTrans.

use chrono::NaiveDate;
use serde::Serialize;

// ─────────────────────────────────────────────────────────────────────────────
// Types
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct TradeShowEvent {
    pub name: String,
    pub location: String,
    pub country: String,
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
    pub url: String,
    pub industry_tags: Vec<String>,
    pub exhibitor_list_url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExhibitorMatch {
    pub event_name: String,
    pub event_dates: String,
    pub matched_entity: String,
    pub entity_type: String, // "competitor" or "poi"
    pub booth_info: Option<String>,
    pub signal_type: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Static event database (major EMS/electronics shows)
// ─────────────────────────────────────────────────────────────────────────────

/// Get the calendar of major electronics industry trade shows for a given year.
pub fn get_trade_show_calendar(year: i32) -> Vec<TradeShowEvent> {
    vec![
        event(
            "IPC APEX EXPO",
            "Anaheim, CA",
            "US",
            year,
            1,
            21,
            year,
            1,
            23,
            "https://www.ipcapexexpo.org",
            &["PCB", "EMS", "electronics assembly"],
        ),
        event(
            "NEPCON Japan",
            "Tokyo Big Sight",
            "JP",
            year,
            1,
            22,
            year,
            1,
            24,
            "https://www.nepconjapan.jp/en",
            &["electronics", "packaging", "SMT"],
        ),
        event(
            "Embedded World",
            "Nuremberg",
            "DE",
            year,
            3,
            11,
            year,
            3,
            13,
            "https://www.embedded-world.de",
            &["embedded", "IoT", "semiconductors"],
        ),
        event(
            "PCIM Europe",
            "Nuremberg",
            "DE",
            year,
            5,
            6,
            year,
            5,
            8,
            "https://pcim.mesago.com",
            &["power electronics", "semiconductors", "energy"],
        ),
        event(
            "SEMICON West",
            "San Francisco, CA",
            "US",
            year,
            7,
            8,
            year,
            7,
            10,
            "https://www.semiconwest.org",
            &["semiconductor", "fab equipment", "materials"],
        ),
        event(
            "SMT Hybrid Packaging",
            "Nuremberg",
            "DE",
            year,
            5,
            6,
            year,
            5,
            8,
            "https://smt.mesago.com",
            &["SMT", "soldering", "packaging", "EMS"],
        ),
        event(
            "CEATEC",
            "Makuhari Messe, Chiba",
            "JP",
            year,
            10,
            15,
            year,
            10,
            18,
            "https://www.ceatec.com",
            &["electronics", "IoT", "AI", "5G"],
        ),
        event(
            "Electronica",
            "Munich",
            "DE",
            year,
            11,
            11,
            year,
            11,
            14,
            "https://electronica.de",
            &["electronics", "components", "semiconductors", "EMS"],
        ),
        event(
            "productronica",
            "Munich",
            "DE",
            year,
            11,
            11,
            year,
            11,
            14,
            "https://productronica.com",
            &["production", "SMT", "EMS", "PCB"],
        ),
        event(
            "GITEX Technology Week",
            "Dubai World Trade Centre",
            "AE",
            year,
            10,
            14,
            year,
            10,
            18,
            "https://www.gitex.com",
            &["technology", "digital", "IoT", "AI"],
        ),
        event(
            "SEMICON Southeast Asia",
            "Kuala Lumpur",
            "MY",
            year,
            5,
            20,
            year,
            5,
            22,
            "https://www.semiconsea.org",
            &["semiconductor", "packaging", "OSAT"],
        ),
        event(
            "NEPCON Asia",
            "Bangkok",
            "TH",
            year,
            6,
            19,
            year,
            6,
            22,
            "https://www.nepconasia.com",
            &["electronics", "SMT", "packaging"],
        ),
    ]
}

fn valid_date(year: i32, month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, day)
        .unwrap_or_else(|| panic!("invalid static trade-show date {year:04}-{month:02}-{day:02}"))
}

#[allow(clippy::too_many_arguments)]
fn event(
    name: &str,
    location: &str,
    country: &str,
    sy: i32,
    sm: u32,
    sd: u32,
    ey: i32,
    em: u32,
    ed: u32,
    url: &str,
    tags: &[&str],
) -> TradeShowEvent {
    TradeShowEvent {
        name: name.into(),
        location: location.into(),
        country: country.into(),
        start_date: valid_date(sy, sm, sd),
        end_date: valid_date(ey, em, ed),
        url: url.into(),
        industry_tags: tags.iter().map(|t| t.to_string()).collect(),
        exhibitor_list_url: None,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// ICS/iCal parser (basic)
// ─────────────────────────────────────────────────────────────────────────────

/// Parse a basic ICS/iCal string into trade show events.
pub fn parse_ics(ics_content: &str) -> Vec<TradeShowEvent> {
    let mut events = Vec::new();
    let mut in_event = false;
    let mut name = String::new();
    let mut location = String::new();
    let mut start = String::new();
    let mut end = String::new();
    let mut url = String::new();

    for line in ics_content.lines() {
        let line = line.trim();
        if line == "BEGIN:VEVENT" {
            in_event = true;
            name.clear();
            location.clear();
            start.clear();
            end.clear();
            url.clear();
        } else if line == "END:VEVENT" && in_event {
            if let (Some(sd), Some(ed)) = (parse_ics_date(&start), parse_ics_date(&end)) {
                events.push(TradeShowEvent {
                    name: name.clone(),
                    location: location.clone(),
                    country: String::new(),
                    start_date: sd,
                    end_date: ed,
                    url: url.clone(),
                    industry_tags: vec![],
                    exhibitor_list_url: None,
                });
            }
            in_event = false;
        } else if in_event {
            if let Some(v) = line.strip_prefix("SUMMARY:") {
                name = v.to_string();
            } else if let Some(v) = line.strip_prefix("LOCATION:") {
                location = v.to_string();
            } else if let Some(v) = line.strip_prefix("DTSTART") {
                // Handle DTSTART;VALUE=DATE:20260301 or DTSTART:20260301T090000Z
                if let Some(date_part) = v.split(':').next_back() {
                    start = date_part.to_string();
                }
            } else if let Some(v) = line.strip_prefix("DTEND") {
                if let Some(date_part) = v.split(':').next_back() {
                    end = date_part.to_string();
                }
            } else if let Some(v) = line.strip_prefix("URL:") {
                url = v.to_string();
            }
        }
    }
    events
}

fn parse_ics_date(s: &str) -> Option<NaiveDate> {
    let clean = s.split('T').next().unwrap_or(s);
    if clean.len() >= 8 {
        let y: i32 = clean[0..4].parse().ok()?;
        let m: u32 = clean[4..6].parse().ok()?;
        let d: u32 = clean[6..8].parse().ok()?;
        NaiveDate::from_ymd_opt(y, m, d)
    } else {
        None
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Exhibitor matching
// ─────────────────────────────────────────────────────────────────────────────

/// Check if any tracked entities (competitors, POIs) are exhibiting at upcoming events.
pub fn match_exhibitors(
    events: &[TradeShowEvent],
    competitor_names: &[String],
    poi_names: &[String],
    exhibitor_lists: &[(String, Vec<String>)], // (event_name, exhibitor_names)
) -> Vec<ExhibitorMatch> {
    let mut matches = Vec::new();

    for (event_name, exhibitors) in exhibitor_lists {
        let event = events.iter().find(|e| e.name == *event_name);
        let dates = event
            .map(|e| format!("{} – {}", e.start_date, e.end_date))
            .unwrap_or_default();

        for exhibitor in exhibitors {
            let ex_lower = exhibitor.to_lowercase();

            for comp in competitor_names {
                if ex_lower.contains(&comp.to_lowercase())
                    || comp.to_lowercase().contains(&ex_lower)
                {
                    matches.push(ExhibitorMatch {
                        event_name: event_name.clone(),
                        event_dates: dates.clone(),
                        matched_entity: comp.clone(),
                        entity_type: "competitor".into(),
                        booth_info: None,
                        signal_type: "competitor_exhibiting".into(),
                    });
                }
            }

            for poi in poi_names {
                if ex_lower.contains(&poi.to_lowercase()) || poi.to_lowercase().contains(&ex_lower)
                {
                    matches.push(ExhibitorMatch {
                        event_name: event_name.clone(),
                        event_dates: dates.clone(),
                        matched_entity: poi.clone(),
                        entity_type: "poi".into(),
                        booth_info: None,
                        signal_type: "poi_attending".into(),
                    });
                }
            }
        }
    }

    matches
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calendar_has_events() {
        let cal = get_trade_show_calendar(2026);
        assert!(cal.len() >= 10);
        assert!(cal.iter().any(|e| e.name.contains("Electronica")));
        assert!(cal.iter().any(|e| e.name.contains("IPC APEX")));
    }

    #[test]
    fn parse_ics_basic() {
        let ics = "BEGIN:VCALENDAR\n\
BEGIN:VEVENT\n\
SUMMARY:Test Show\n\
LOCATION:Berlin\n\
DTSTART;VALUE=DATE:20260315\n\
DTEND;VALUE=DATE:20260317\n\
URL:https://example.com\n\
END:VEVENT\n\
END:VCALENDAR";
        let events = parse_ics(ics);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].name, "Test Show");
        assert_eq!(
            events[0].start_date,
            NaiveDate::from_ymd_opt(2026, 3, 15)
                .unwrap_or_else(|| panic!("test fixture date should be valid"))
        );
    }

    #[test]
    fn exhibitor_matching() {
        let events = get_trade_show_calendar(2026);
        let competitors = vec!["Flex Ltd".into(), "Jabil".into()];
        let pois = vec![];
        let exhibitors = vec![(
            "Electronica".into(),
            vec!["Flex Ltd".into(), "Infineon".into()],
        )];
        let matches = match_exhibitors(&events, &competitors, &pois, &exhibitors);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].matched_entity, "Flex Ltd");
        assert_eq!(matches[0].entity_type, "competitor");
    }
}
