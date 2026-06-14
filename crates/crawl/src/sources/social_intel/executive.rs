//! Executive Movement Tracking Module
//!
//! Tracks executive and board-level personnel movements across public sources:
//! - LinkedIn personnel changes
//! - News announcements
//! - Corporate filings (SEC, Companies House)
//! - Trade press mentions
//!
//! # Intelligence Value
//! - Executive departures → signals strategic pivots or distress
//! - New hires → signals expansion into new markets/technologies
//! - Board changes → signals ownership changes or governance shifts

use anyhow::Result;
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{debug, info};

/// An executive movement event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutiveMove {
    /// Movement type.
    pub move_type: MoveType,
    /// Executive's full name.
    pub person_name: String,
    /// Person's LinkedIn URL (if available).
    pub linkedin_url: Option<String>,
    /// Previous company/role.
    pub from_company: Option<String>,
    /// Previous title.
    pub from_title: Option<String>,
    /// New company/role.
    pub to_company: Option<String>,
    /// New title.
    pub to_title: Option<String>,
    /// Date of the movement.
    pub move_date: Option<NaiveDate>,
    /// Source of this intelligence.
    pub source: String,
    /// Source URL.
    pub source_url: Option<String>,
    /// When this was detected.
    pub detected_at: DateTime<Utc>,
    /// Confidence level.
    pub confidence: ExecutiveMoveConfidence,
    /// Additional notes.
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MoveType {
    /// Executive hired / joined a company.
    Hire,
    /// Executive promoted internally.
    Promotion,
    /// Executive departed / left company.
    Departure,
    /// Board appointment.
    BoardAppointment,
    /// Executive moved to a new company.
    LateralMove,
    /// Retirement.
    Retirement,
}

impl MoveType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hire => "hire",
            Self::Promotion => "promotion",
            Self::Departure => "departure",
            Self::BoardAppointment => "board_appointment",
            Self::LateralMove => "lateral_move",
            Self::Retirement => "retirement",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "hire" | "joined" | "new hire" | "appoints" => Some(Self::Hire),
            "promotion" | "promoted" | "elevated" => Some(Self::Promotion),
            "departure" | "left" | "resigned" | "exits" => Some(Self::Departure),
            "board" | "board appointment" | "director" => Some(Self::BoardAppointment),
            "lateral" | "moves to" | "joins from" => Some(Self::LateralMove),
            "retirement" | "retired" => Some(Self::Retirement),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecutiveMoveConfidence {
    Low,
    Medium,
    High,
}

impl ExecutiveMoveConfidence {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

/// A tracked executive or board member.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackedExecutive {
    pub name: String,
    pub company: String,
    pub title: Option<String>,
    pub linkedin_url: Option<String>,
    pub prior_companies: Vec<String>,
    /// Last known move.
    pub last_move: Option<ExecutiveMove>,
}

impl TrackedExecutive {
    /// Check if this move represents a change from the tracked state.
    pub fn detect_new_move(&self, move_event: &ExecutiveMove) -> bool {
        if let Some(ref last) = self.last_move {
            // Same person moved again
            last.move_date < move_event.move_date
                || last.move_type != move_event.move_type
                || last.to_company.as_ref() != move_event.to_company.as_ref()
        } else {
            true
        }
    }
}

/// Executive movement monitor.
#[derive(Debug, Clone)]
pub struct ExecutiveMonitor {
    tracked_executives: HashMap<String, TrackedExecutive>,
    /// Movement event history.
    events: Vec<ExecutiveMove>,
}

impl Default for ExecutiveMonitor {
    fn default() -> Self {
        Self::new()
    }
}

impl ExecutiveMonitor {
    /// Create a new executive monitor.
    pub fn new() -> Self {
        Self {
            tracked_executives: HashMap::new(),
            events: Vec::new(),
        }
    }

    /// Register an executive to track.
    pub fn track_executive(&mut self, exec: TrackedExecutive) {
        let key = exec.name.to_lowercase();
        self.tracked_executives.insert(key, exec);
    }

    /// Register multiple executives.
    pub fn track_executives(&mut self, execs: impl IntoIterator<Item = TrackedExecutive>) {
        for exec in execs {
            self.track_executive(exec);
        }
    }

    /// Record a detected executive move.
    pub fn record_move(&mut self, move_event: ExecutiveMove) {
        let key = move_event.person_name.to_lowercase();

        // Check if this is a new move for a tracked executive
        if let Some(exec) = self.tracked_executives.get_mut(&key) {
            if exec.detect_new_move(&move_event) {
                exec.last_move = Some(move_event.clone());
            }
        }

        self.events.push(move_event);
        debug!(
            person = %move_event.person_name,
            move_type = %move_event.move_type.as_str(),
            "Executive move recorded"
        );
    }

    /// Record multiple moves.
    pub fn record_moves(&mut self, moves: impl IntoIterator<Item = ExecutiveMove>) {
        for m in moves {
            self.record_move(m);
        }
    }

    /// Get all recorded moves, newest first.
    pub fn all_moves(&self) -> Vec<&ExecutiveMove> {
        let mut events: Vec<_> = self.events.iter().collect();
        events.sort_by(|a, b| b.detected_at.cmp(&a.detected_at));
        events
    }

    /// Get moves for a specific person.
    pub fn moves_for_person(&self, name: &str) -> Vec<&ExecutiveMove> {
        self.events
            .iter()
            .filter(|e| e.person_name.to_lowercase().contains(&name.to_lowercase()))
            .collect()
    }

    /// Get the most recent move for each tracked executive.
    pub fn latest_moves(&self) -> Vec<&ExecutiveMove> {
        let mut latest: HashMap<String, &ExecutiveMove> = HashMap::new();
        for event in &self.events {
            let key = event.person_name.to_lowercase();
            if let Some(existing) = latest.get(&key) {
                if event.detected_at > existing.detected_at {
                    latest.insert(key, event);
                }
            } else {
                latest.insert(key, event);
            }
        }
        latest.into_values().collect()
    }

    /// Get moves for a specific company.
    pub fn moves_for_company(&self, company: &str) -> Vec<&ExecutiveMove> {
        self.events
            .iter()
            .filter(|e| {
                e.from_company
                    .as_ref()
                    .map(|c| c.to_lowercase().contains(&company.to_lowercase()))
                    .unwrap_or(false)
                    || e.to_company
                        .as_ref()
                        .map(|c| c.to_lowercase().contains(&company.to_lowercase()))
                        .unwrap_or(false)
            })
            .collect()
    }

    /// Return the total number of tracked executives.
    pub fn tracked_count(&self) -> usize {
        self.tracked_executives.len()
    }

    /// Return the total number of recorded moves.
    pub fn event_count(&self) -> usize {
        self.events.len()
    }

    /// Parse a news headline into an executive move event.
    pub fn parse_headline(headline: &str, source: &str, source_url: Option<&str>) -> Option<ExecutiveMove> {
        let lower = headline.to_lowercase();

        let move_type = if lower.contains("joins") || lower.contains("appointed") || lower.contains("new chief") {
            MoveType::Hire
        } else if lower.contains("promoted") || lower.contains("elevated to") {
            MoveType::Promotion
        } else if lower.contains("leaves") || lower.contains("departed") || lower.contains("resigns") {
            MoveType::Departure
        } else if lower.contains("board") {
            MoveType::BoardAppointment
        } else if lower.contains("retires") || lower.contains("retirement") {
            MoveType::Retirement
        } else {
            return None;
        };

        Some(ExecutiveMove {
            move_type,
            person_name: headline.split_whitespace().take(4).collect::<Vec<_>>().join(" "),
            linkedin_url: None,
            from_company: None,
            from_title: None,
            to_company: None,
            to_title: None,
            move_date: None,
            source: source.to_string(),
            source_url: source_url.map(|s| s.to_string()),
            detected_at: Utc::now(),
            confidence: ExecutiveMoveConfidence::Medium,
            notes: Some(headline.to_string()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn move_type_from_str() {
        assert_eq!(MoveType::from_str("joined"), Some(MoveType::Hire));
        assert_eq!(MoveType::from_str("PROMOTED"), Some(MoveType::Promotion));
        assert_eq!(MoveType::from_str("resigned"), Some(MoveType::Departure));
    }

    #[test]
    fn move_type_as_str() {
        assert_eq!(MoveType::Hire.as_str(), "hire");
        assert_eq!(MoveType::BoardAppointment.as_str(), "board_appointment");
    }

    #[test]
    fn executive_monitor_new() {
        let monitor = ExecutiveMonitor::new();
        assert_eq!(monitor.tracked_count(), 0);
        assert_eq!(monitor.event_count(), 0);
    }

    #[test]
    fn track_and_record_move() {
        let mut monitor = ExecutiveMonitor::new();

        monitor.track_executive(TrackedExecutive {
            name: "John Smith".to_string(),
            company: "DefenseCorp".to_string(),
            title: Some("CFO".to_string()),
            linkedin_url: None,
            prior_companies: vec![],
            last_move: None,
        });

        monitor.record_move(ExecutiveMove {
            move_type: MoveType::Hire,
            person_name: "John Smith".to_string(),
            linkedin_url: None,
            from_company: None,
            from_title: None,
            to_company: Some("NewCo".to_string()),
            to_title: Some("CEO".to_string()),
            move_date: None,
            source: "linkedin".to_string(),
            source_url: None,
            detected_at: Utc::now(),
            confidence: ExecutiveMoveConfidence::High,
            notes: None,
        });

        assert_eq!(monitor.tracked_count(), 1);
        assert_eq!(monitor.event_count(), 1);
    }

    #[test]
    fn moves_for_company() {
        let mut monitor = ExecutiveMonitor::new();
        monitor.record_move(ExecutiveMove {
            move_type: MoveType::Hire,
            person_name: "Jane Doe".to_string(),
            linkedin_url: None,
            from_company: Some("OldCorp".to_string()),
            from_title: None,
            to_company: Some("ElbitSystems".to_string()),
            to_title: None,
            move_date: None,
            source: "news".to_string(),
            source_url: None,
            detected_at: Utc::now(),
            confidence: ExecutiveMoveConfidence::High,
            notes: None,
        });

        let moves = monitor.moves_for_company("elbit");
        assert_eq!(moves.len(), 1);
    }

    #[test]
    fn parse_headline() {
        let event = ExecutiveMonitor::parse_headline(
            "John Smith appointed CEO at DefenseCorp",
            "news",
            Some("https://news.example.com"),
        );
        assert!(event.is_some());
        let event = event.unwrap();
        assert_eq!(event.move_type, MoveType::Hire);
        assert_eq!(event.confidence.as_str(), "medium");
    }

    #[test]
    fn parse_headline_no_match() {
        let event = ExecutiveMonitor::parse_headline(
            "Company reports quarterly earnings",
            "finance",
            None,
        );
        assert!(event.is_none());
    }
}
