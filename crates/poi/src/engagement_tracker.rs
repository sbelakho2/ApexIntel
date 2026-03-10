//! POI engagement activity tracker.
//!
//! Track all outreach attempts (via API: POST /api/persons/:id/engagement)
//! with date, method, outcome, and next-action to build a CRM-lite within
//! ApexIntel.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

// ─── Engagement types ───────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngagementRecord {
    pub id: String,
    pub person_id: String,
    pub person_name: String,
    pub engagement_type: EngagementType,
    pub date: DateTime<Utc>,
    pub method: ContactMethod,
    pub outcome: EngagementOutcome,
    pub notes: String,
    pub next_action: Option<NextAction>,
    pub created_by: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum EngagementType {
    InitialOutreach,
    FollowUp,
    Meeting,
    PhoneCall,
    EmailExchange,
    ConferenceEncounter,
    LinkedInMessage,
    Referral,
    Presentation,
    SiteVisit,
}

impl EngagementType {
    pub fn label(&self) -> &'static str {
        match self {
            Self::InitialOutreach => "Initial Outreach",
            Self::FollowUp => "Follow-Up",
            Self::Meeting => "Meeting",
            Self::PhoneCall => "Phone Call",
            Self::EmailExchange => "Email Exchange",
            Self::ConferenceEncounter => "Conference Encounter",
            Self::LinkedInMessage => "LinkedIn Message",
            Self::Referral => "Referral",
            Self::Presentation => "Presentation",
            Self::SiteVisit => "Site Visit",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ContactMethod {
    Email,
    Phone,
    LinkedIn,
    InPerson,
    VideoCall,
    WhatsApp,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum EngagementOutcome {
    Positive,
    Neutral,
    Negative,
    NoResponse,
    ScheduledFollowUp,
    Declined,
    Referred,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NextAction {
    pub action: String,
    pub due_date: NaiveDate,
    pub assigned_to: String,
    pub priority: String,
}

// ─── Create engagement request ──────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateEngagementRequest {
    pub engagement_type: EngagementType,
    pub method: ContactMethod,
    pub outcome: EngagementOutcome,
    pub notes: String,
    pub next_action: Option<NextAction>,
}

#[derive(Debug, Serialize)]
pub struct EngagementResponse {
    pub engagement: EngagementRecord,
}

#[derive(Debug, Serialize)]
pub struct EngagementListResponse {
    pub engagements: Vec<EngagementRecord>,
    pub total: usize,
    pub summary: EngagementSummary,
}

// ─── Engagement analytics ───────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct EngagementSummary {
    pub total_engagements: usize,
    pub positive_outcomes: usize,
    pub negative_outcomes: usize,
    pub no_response: usize,
    pub pending_follow_ups: usize,
    pub last_engagement_date: Option<DateTime<Utc>>,
    pub avg_response_rate: f64,
    pub engagement_score: f64,
}

pub fn compute_summary(engagements: &[EngagementRecord]) -> EngagementSummary {
    let total = engagements.len();
    let positive = engagements
        .iter()
        .filter(|e| e.outcome == EngagementOutcome::Positive)
        .count();
    let negative = engagements
        .iter()
        .filter(|e| {
            e.outcome == EngagementOutcome::Negative || e.outcome == EngagementOutcome::Declined
        })
        .count();
    let no_response = engagements
        .iter()
        .filter(|e| e.outcome == EngagementOutcome::NoResponse)
        .count();
    let pending = engagements
        .iter()
        .filter(|e| e.next_action.is_some())
        .count();

    let last_date = engagements.iter().map(|e| e.date).max();

    let responded = total.saturating_sub(no_response);
    let response_rate = if total > 0 {
        responded as f64 / total as f64
    } else {
        0.0
    };

    // Engagement score: weighted combination of quantity, recency, and positivity
    let quantity_score = (total as f64).min(10.0) / 10.0;
    let positivity_score = if total > 0 {
        positive as f64 / total as f64
    } else {
        0.0
    };
    let recency_score = last_date
        .map(|d| {
            let days = (Utc::now() - d).num_days();
            if days < 7 {
                1.0
            } else if days < 30 {
                0.7
            } else if days < 90 {
                0.4
            } else {
                0.1
            }
        })
        .unwrap_or(0.0);

    let engagement_score =
        (quantity_score * 0.3 + positivity_score * 0.4 + recency_score * 0.3).clamp(0.0, 1.0);

    EngagementSummary {
        total_engagements: total,
        positive_outcomes: positive,
        negative_outcomes: negative,
        no_response,
        pending_follow_ups: pending,
        last_engagement_date: last_date,
        avg_response_rate: response_rate,
        engagement_score,
    }
}

// ─── SQL queries ────────────────────────────────────────────────────────

pub fn insert_engagement_sql() -> &'static str {
    r#"
    INSERT INTO poi_engagements (
        person_id, engagement_type, contact_method, outcome,
        notes, next_action, created_by, created_at
    ) VALUES ($1, $2, $3, $4, $5, $6, $7, NOW())
    RETURNING id, created_at
    "#
}

pub fn list_engagements_sql() -> &'static str {
    r#"
    SELECT id, person_id, engagement_type, contact_method, outcome,
           notes, next_action, created_by, created_at
    FROM poi_engagements
    WHERE person_id = $1
    ORDER BY created_at DESC
    LIMIT $2 OFFSET $3
    "#
}

pub fn pending_followups_sql() -> &'static str {
    r#"
    SELECT e.*, p.full_name
    FROM poi_engagements e
    JOIN persons p ON p.id = e.person_id
    WHERE e.next_action IS NOT NULL
      AND (e.next_action->>'due_date')::date <= CURRENT_DATE
    ORDER BY (e.next_action->>'due_date')::date ASC
    "#
}

/// Path constants
pub const ENGAGEMENT_PATH: &str = "/api/persons/:id/engagements";
pub const ENGAGEMENT_DETAIL: &str = "/api/persons/:id/engagements/:eid";

// ─── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_engagement(outcome: EngagementOutcome, days_ago: i64) -> EngagementRecord {
        EngagementRecord {
            id: format!("eng-{}", days_ago),
            person_id: "p-001".into(),
            person_name: "Test Person".into(),
            engagement_type: EngagementType::EmailExchange,
            date: Utc::now() - chrono::Duration::days(days_ago),
            method: ContactMethod::Email,
            outcome,
            notes: "Test engagement".into(),
            next_action: None,
            created_by: "analyst".into(),
        }
    }

    #[test]
    fn test_empty_summary() {
        let summary = compute_summary(&[]);
        assert_eq!(summary.total_engagements, 0);
        assert_eq!(summary.engagement_score, 0.0);
    }

    #[test]
    fn test_positive_summary() {
        let engagements = vec![
            make_engagement(EngagementOutcome::Positive, 5),
            make_engagement(EngagementOutcome::Positive, 10),
            make_engagement(EngagementOutcome::Neutral, 15),
        ];
        let summary = compute_summary(&engagements);
        assert_eq!(summary.total_engagements, 3);
        assert_eq!(summary.positive_outcomes, 2);
        assert!(summary.engagement_score > 0.3);
    }

    #[test]
    fn test_no_response_rate() {
        let engagements = vec![
            make_engagement(EngagementOutcome::NoResponse, 5),
            make_engagement(EngagementOutcome::NoResponse, 10),
            make_engagement(EngagementOutcome::Positive, 15),
        ];
        let summary = compute_summary(&engagements);
        assert!((summary.avg_response_rate - 0.333).abs() < 0.01);
    }

    #[test]
    fn test_engagement_type_labels() {
        assert_eq!(EngagementType::InitialOutreach.label(), "Initial Outreach");
        assert_eq!(EngagementType::SiteVisit.label(), "Site Visit");
    }

    #[test]
    fn test_sql_not_empty() {
        assert!(insert_engagement_sql().contains("poi_engagements"));
        assert!(list_engagements_sql().contains("ORDER BY"));
        assert!(pending_followups_sql().contains("due_date"));
    }
}
