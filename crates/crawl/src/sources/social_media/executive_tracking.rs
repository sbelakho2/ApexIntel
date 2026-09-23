//! Executive Movement Tracking Module
//!
//! Tracks executive movement and career changes:
//! - Executive departures and hires
//! - Board changes
//! - LinkedIn profile updates (promotions, new positions)
//! - News mentions of executives
//! - Organizational structure changes
//!
//! This module aggregates executive signals from multiple sources.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use tracing::debug;

/// An executive profile being tracked.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackedExecutive {
    pub executive_id: String,
    pub full_name: String,
    pub company: String,
    pub previous_title: Option<String>,
    pub current_title: Option<String>,
    pub previous_company: Option<String>,
    pub linkedin_url: Option<String>,
    pub email: Option<String>,
    pub last_known_location: Option<String>,
    pub last_updated: DateTime<Utc>,
}

/// An executive movement event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutiveMovement {
    pub movement_id: String,
    pub executive_id: String,
    pub executive_name: String,
    pub movement_type: MovementType,
    pub from_company: Option<String>,
    pub from_title: Option<String>,
    pub to_company: Option<String>,
    pub to_title: Option<String>,
    pub announcement_date: Option<NaiveDate>,
    pub source: String,
    pub source_url: Option<String>,
    pub headline: String,
    pub description: Option<String>,
    pub confidence: MovementConfidence,
    pub detected_at: DateTime<Utc>,
}

impl ExecutiveMovement {
    /// Whether this is a departure.
    pub fn is_departure(&self) -> bool {
        matches!(
            self.movement_type,
            MovementType::Departure | MovementType::Resignation | MovementType::Termination
        )
    }

    /// Whether this is a hire.
    pub fn is_hire(&self) -> bool {
        matches!(
            self.movement_type,
            MovementType::Hire | MovementType::Promotion | MovementType::InternalMove
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MovementType {
    Hire,
    Departure,
    Resignation,
    Termination,
    Promotion,
    LateralMove,
    BoardChange,
    Retirement,
    InternalMove,
    Unknown,
}

impl MovementType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hire => "hire",
            Self::Departure => "departure",
            Self::Resignation => "resignation",
            Self::Termination => "termination",
            Self::Promotion => "promotion",
            Self::LateralMove => "lateral_move",
            Self::BoardChange => "board_change",
            Self::Retirement => "retirement",
            Self::InternalMove => "internal_move",
            Self::Unknown => "unknown",
        }
    }

    pub fn from_headline(headline: &str) -> Self {
        let lower = headline.to_lowercase();
        if lower.contains("appointed") || lower.contains("hired") || lower.contains("joins") {
            Self::Hire
        } else if lower.contains("resigns")
            || lower.contains("stepping down")
            || lower.contains("depart")
        {
            Self::Resignation
        } else if lower.contains("fired")
            || lower.contains("terminated")
            || lower.contains("removed")
        {
            Self::Termination
        } else if lower.contains("promoted")
            || lower.contains("elevation")
            || lower.contains("rises to")
        {
            Self::Promotion
        } else if lower.contains("leaves") || lower.contains("exits") {
            Self::Departure
        } else if lower.contains("board") || lower.contains("director") {
            Self::BoardChange
        } else if lower.contains("retires") || lower.contains("retirement") {
            Self::Retirement
        } else {
            Self::Unknown
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MovementConfidence {
    Low,
    Medium,
    High,
}

impl MovementConfidence {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

/// Executive tracking configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutiveTrackerConfig {
    /// Companies to track executive movements for.
    pub tracked_companies: Vec<String>,
    /// Executive names to track.
    pub tracked_executives: Vec<String>,
    /// Keywords that indicate movement.
    pub movement_keywords: Vec<String>,
    /// Maximum movements to return.
    pub max_results: u32,
}

impl Default for ExecutiveTrackerConfig {
    fn default() -> Self {
        Self {
            tracked_companies: Vec::new(),
            tracked_executives: Vec::new(),
            movement_keywords: vec![
                "appointed".to_string(),
                "hired".to_string(),
                "joins".to_string(),
                "resigns".to_string(),
                "promoted".to_string(),
                "leaves".to_string(),
                "depart".to_string(),
                "stepping down".to_string(),
                "board".to_string(),
                "CEO".to_string(),
                "CFO".to_string(),
                "CTO".to_string(),
                "COO".to_string(),
                "President".to_string(),
                "VP".to_string(),
            ],
            max_results: 50,
        }
    }
}

impl ExecutiveTrackerConfig {
    pub fn add_company(mut self, company: impl Into<String>) -> Self {
        self.tracked_companies.push(company.into());
        self
    }

    pub fn add_executive(mut self, name: impl Into<String>) -> Self {
        self.tracked_executives.push(name.into());
        self
    }
}

/// Executive movement tracker.
#[derive(Debug, Clone)]
pub struct ExecutiveTracker {
    config: ExecutiveTrackerConfig,
    movements: Vec<ExecutiveMovement>,
}

impl ExecutiveTracker {
    /// Create with configuration.
    pub fn new(config: ExecutiveTrackerConfig) -> Self {
        Self {
            config,
            movements: Vec::new(),
        }
    }

    /// Create with default configuration.
    pub fn with_defaults() -> Self {
        Self::new(ExecutiveTrackerConfig::default())
    }

    /// Detect executive movements from news text.
    pub fn detect_in_text(&self, text: &str, source: &str) -> Vec<ExecutiveMovement> {
        let lower = text.to_lowercase();
        let mut detected = Vec::new();

        for kw in &self.config.movement_keywords {
            if lower.contains(&kw.to_lowercase()) {
                // Try to extract name before the keyword
                let before_keyword: String = lower
                    .split(&kw.to_lowercase())
                    .next()
                    .map(|s| {
                        let parts: Vec<&str> = s.split_whitespace().collect();
                        parts[parts.len().saturating_sub(3)..].join(" ")
                    })
                    .unwrap_or_default();

                let movement_type = MovementType::from_headline(text);
                let confidence = if before_keyword.len() > 5 {
                    MovementConfidence::Medium
                } else {
                    MovementConfidence::Low
                };

                detected.push(ExecutiveMovement {
                    movement_id: format!("mv-{}-{}", source, Utc::now().timestamp()),
                    executive_id: "extracted".to_string(),
                    executive_name: before_keyword.trim().to_string(),
                    movement_type,
                    from_company: None,
                    from_title: None,
                    to_company: None,
                    to_title: None,
                    announcement_date: None,
                    source: source.to_string(),
                    source_url: None,
                    headline: text.chars().take(150).collect(),
                    description: Some(text.to_string()),
                    confidence,
                    detected_at: Utc::now(),
                });
            }
        }

        debug!(count = detected.len(), "Executive movements detected");
        detected
    }

    /// Add a tracked movement.
    pub fn add_movement(&mut self, movement: ExecutiveMovement) {
        self.movements.push(movement);
    }

    /// Get all tracked movements.
    pub fn all_movements(&self) -> &[ExecutiveMovement] {
        &self.movements
    }

    /// Get movements for a specific company.
    pub fn movements_for_company(&self, company: &str) -> Vec<&ExecutiveMovement> {
        self.movements
            .iter()
            .filter(|m| {
                m.from_company
                    .as_ref()
                    .map(|c| c.to_lowercase() == company.to_lowercase())
                    .unwrap_or(false)
                    || m.to_company
                        .as_ref()
                        .map(|c| c.to_lowercase() == company.to_lowercase())
                        .unwrap_or(false)
            })
            .collect()
    }

    /// Get hires.
    pub fn hires(&self) -> Vec<&ExecutiveMovement> {
        self.movements.iter().filter(|m| m.is_hire()).collect()
    }

    /// Get departures.
    pub fn departures(&self) -> Vec<&ExecutiveMovement> {
        self.movements.iter().filter(|m| m.is_departure()).collect()
    }

    /// Return total movement count.
    pub fn movement_count(&self) -> usize {
        self.movements.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn movement_type_from_headline() {
        assert_eq!(
            MovementType::from_headline("John Smith appointed CEO of Acme Corp"),
            MovementType::Hire
        );
        assert_eq!(
            MovementType::from_headline("Jane Doe resigns from XYZ Inc"),
            MovementType::Resignation
        );
        assert_eq!(
            MovementType::from_headline("Bob promoted to CFO at BigCo"),
            MovementType::Promotion
        );
    }

    #[test]
    fn executive_movement_is_hire() {
        let movement = ExecutiveMovement {
            movement_id: "mv-001".to_string(),
            executive_id: "exec-001".to_string(),
            executive_name: "John Doe".to_string(),
            movement_type: MovementType::Hire,
            from_company: None,
            from_title: None,
            to_company: Some("Acme Corp".to_string()),
            to_title: Some("CEO".to_string()),
            announcement_date: Some(Utc::now().date_naive()),
            source: "news".to_string(),
            source_url: None,
            headline: "John Doe joins Acme Corp as CEO".to_string(),
            description: None,
            confidence: MovementConfidence::High,
            detected_at: Utc::now(),
        };
        assert!(movement.is_hire());
        assert!(!movement.is_departure());
    }

    #[test]
    fn executive_tracker_constructs() {
        let tracker = ExecutiveTracker::with_defaults();
        assert_eq!(tracker.movement_count(), 0);
    }

    #[test]
    fn executive_tracker_chaining() {
        let cfg = ExecutiveTrackerConfig::default()
            .add_company("Lockheed Martin")
            .add_company("Raytheon")
            .add_executive("James Smith");
        assert_eq!(cfg.tracked_companies.len(), 2);
        assert_eq!(cfg.tracked_executives.len(), 1);
    }

    #[test]
    fn detect_in_text() {
        let tracker = ExecutiveTracker::with_defaults();
        let movements = tracker.detect_in_text(
            "Robert Johnson appointed Chief Technology Officer at Defense Systems Inc",
            "news_feed",
        );
        assert!(!movements.is_empty());
    }

    #[test]
    fn movement_confidence_as_str() {
        assert_eq!(MovementConfidence::High.as_str(), "high");
        assert_eq!(MovementConfidence::Low.as_str(), "low");
    }
}
