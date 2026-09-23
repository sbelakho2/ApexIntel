//! POI engagement activity tracker.
//!
//! Tracks outreach attempts with CRM-lite functionality: date, method,
//! outcome, and next-action recording. Engagement scores are computed
//! from real artifact data and role history.
//!
//! ## Engagement scoring
//!
//! Two scoring dimensions are combined:
//!
//! | Dimension        | Source                         | Weight |
//! |------------------|--------------------------------|--------|
//! | Outreach history | `EngagementRecord` outcomes     | 0.50   |
//! | Artifact + role  | `PoiProfile` artifacts & roles  | 0.50   |
//!
//! Callers with a full `PoiProfile` should use
//! `compute_profile_engagement_score`; callers with only outreach
//! records can fall back to `compute_summary`.

use crate::model::PoiProfile;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A single outreach attempt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngagementRecord {
    pub person_id: String,
    pub date: DateTime<Utc>,
    pub method: String,
    pub outcome: EngagementOutcome,
    pub notes: String,
    pub next_action: Option<String>,
    pub next_due: Option<DateTime<Utc>>,
}

/// Outcome classification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EngagementOutcome {
    Positive,
    Neutral,
    Negative,
    NoResponse,
}

impl EngagementOutcome {
    fn score(&self) -> f64 {
        match self {
            EngagementOutcome::Positive => 1.0,
            EngagementOutcome::Neutral => 0.5,
            EngagementOutcome::Negative => 0.0,
            EngagementOutcome::NoResponse => 0.1,
        }
    }
}

/// Aggregated summary of engagement activity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngagementSummary {
    pub total_attempts: usize,
    pub positive_count: usize,
    pub neutral_count: usize,
    pub negative_count: usize,
    pub no_response_count: usize,
    pub last_contact_date: Option<DateTime<Utc>>,
    pub next_action_due: Option<DateTime<Utc>>,
    pub engagement_score: f64,
    pub activity_trend: EngagementTrend,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum EngagementTrend {
    Warming,
    Cooling,
    Stable,
    InsufficientData,
}

/// Compute an engagement summary from individual records.
pub fn compute_summary(records: &[EngagementRecord], _now: DateTime<Utc>) -> EngagementSummary {
    let total = records.len();
    let mut positive = 0;
    let mut neutral = 0;
    let mut negative = 0;
    let mut no_response = 0;
    let mut weighted_outcome = 0.0;

    let last_contact = records.iter().map(|r| r.date).max();
    let next_due = records.iter().filter_map(|r| r.next_due).max();

    for r in records {
        match r.outcome {
            EngagementOutcome::Positive => {
                positive += 1;
                weighted_outcome += 1.0;
            }
            EngagementOutcome::Neutral => {
                neutral += 1;
                weighted_outcome += 0.5;
            }
            EngagementOutcome::Negative => {
                negative += 1;
                weighted_outcome += 0.0;
            }
            EngagementOutcome::NoResponse => {
                no_response += 1;
                weighted_outcome += 0.1;
            }
        }
    }

    let outreach_score = if total > 0 {
        weighted_outcome / total as f64
    } else {
        0.0
    };

    let trend = if total < 2 {
        EngagementTrend::InsufficientData
    } else {
        // Split into older half and newer half to detect trend direction
        let mut sorted: Vec<&EngagementRecord> = records.iter().collect();
        sorted.sort_by_key(|r| r.date);
        let mid = sorted.len() / 2;
        let older: Vec<&EngagementRecord> = sorted[..mid].to_vec();
        let newer: Vec<&EngagementRecord> = sorted[mid..].to_vec();

        if older.is_empty() || newer.is_empty() {
            EngagementTrend::Stable
        } else {
            let older_score: f64 =
                older.iter().map(|r| r.outcome.score()).sum::<f64>() / older.len() as f64;
            let newer_score: f64 =
                newer.iter().map(|r| r.outcome.score()).sum::<f64>() / newer.len() as f64;

            if newer_score > older_score + 0.15 {
                EngagementTrend::Warming
            } else if newer_score < older_score - 0.15 {
                EngagementTrend::Cooling
            } else {
                EngagementTrend::Stable
            }
        }
    };

    EngagementSummary {
        total_attempts: total,
        positive_count: positive,
        neutral_count: neutral,
        negative_count: negative,
        no_response_count: no_response,
        last_contact_date: last_contact,
        next_action_due: next_due,
        engagement_score: outreach_score,
        activity_trend: trend,
    }
}

/// Compute engagement score using the full profile (artifact + role data).
///
/// Weights: 50% outreach records, 30% artifact recency, 20% role velocity.
pub fn compute_profile_engagement_score(
    profile: &PoiProfile,
    records: &[EngagementRecord],
    now: DateTime<Utc>,
) -> f64 {
    let summary = compute_summary(records, now);

    // Artifact recency score: more recent artifacts → higher engagement signal
    let artifact_score = if profile.artifacts.is_empty() {
        0.0
    } else {
        let recent_count = profile
            .artifacts
            .iter()
            .filter(|a| {
                let days = (now.timestamp() - a.ts_utc) / 86400;
                days < 90
            })
            .count();
        (recent_count as f64 / 10.0).min(1.0)
    };

    // Role velocity: more role changes → more active career → more engagement potential
    let role_score = if profile.role_history.len() >= 3 {
        (profile.role_history.len() as f64 / 6.0).min(1.0)
    } else {
        0.2
    };

    summary.engagement_score * 0.50 + artifact_score * 0.30 + role_score * 0.20
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;

    #[test]
    fn empty_records_produce_zero_score() {
        let summary = compute_summary(&[], Utc::now());
        assert_eq!(summary.engagement_score, 0.0);
        assert_eq!(summary.total_attempts, 0);
    }

    #[test]
    fn positive_outcomes_score_high() {
        let now = Utc::now();
        let records = vec![
            EngagementRecord {
                person_id: "test".into(),
                date: now - chrono::Duration::days(10),
                method: "email".into(),
                outcome: EngagementOutcome::Positive,
                notes: "Good response".into(),
                next_action: None,
                next_due: None,
            },
            EngagementRecord {
                person_id: "test".into(),
                date: now - chrono::Duration::days(5),
                method: "call".into(),
                outcome: EngagementOutcome::Positive,
                notes: "Meeting booked".into(),
                next_action: None,
                next_due: None,
            },
        ];
        let summary = compute_summary(&records, now);
        assert!(summary.engagement_score > 0.8);
    }

    #[test]
    fn trend_is_warming_when_recent_scores_improve() {
        let now = Utc::now();
        let records = vec![
            EngagementRecord {
                person_id: "test".into(),
                date: now - chrono::Duration::days(60),
                method: "email".into(),
                outcome: EngagementOutcome::NoResponse,
                notes: "".into(),
                next_action: None,
                next_due: None,
            },
            EngagementRecord {
                person_id: "test".into(),
                date: now - chrono::Duration::days(30),
                method: "email".into(),
                outcome: EngagementOutcome::Neutral,
                notes: "".into(),
                next_action: None,
                next_due: None,
            },
            EngagementRecord {
                person_id: "test".into(),
                date: now - chrono::Duration::days(10),
                method: "call".into(),
                outcome: EngagementOutcome::Positive,
                notes: "".into(),
                next_action: None,
                next_due: None,
            },
        ];
        let summary = compute_summary(&records, now);
        assert_eq!(summary.activity_trend, EngagementTrend::Warming);
    }

    #[test]
    fn profile_engagement_combines_all_sources() {
        let now = Utc::now();
        let profile = PoiProfile {
            person_id: "test".into(),
            name: "Test".into(),
            name_variants: vec![],
            org: "Corp".into(),
            org_id: None,
            current_role: "VP".into(),
            role_family: RoleFamily::Executive,
            region: "US".into(),
            country_code: "US".into(),
            public_bio: String::new(),
            public_email: None,
            artifacts: vec![PoiArtifact {
                artifact_type: "article".into(),
                title: "Recent news".into(),
                content_summary: "Coverage".into(),
                source_url: Some("https://example.com".into()),
                ts_utc: now.timestamp() - 86400 * 10,
            }],
            priority_vector: PriorityVector::zero(),
            psychological: PsychProfile::default_profile(),
            influence: InfluenceProfile {
                influence_score: 0.0,
                graph_centrality: 0.0,
                public_recurrence: 0.0,
                role_seniority_score: 0.0,
                network_size: 0,
            },
            engagement: None,
            role_history: vec![
                RoleHistoryEntry {
                    org: "Corp1".into(),
                    title: "Manager".into(),
                    role_family: RoleFamily::Operations,
                    start_ts: 1500000000,
                    end_ts: Some(1600000000),
                },
                RoleHistoryEntry {
                    org: "Corp2".into(),
                    title: "Director".into(),
                    role_family: RoleFamily::Executive,
                    start_ts: 1600000000,
                    end_ts: Some(1650000000),
                },
                RoleHistoryEntry {
                    org: "Corp".into(),
                    title: "VP".into(),
                    role_family: RoleFamily::Executive,
                    start_ts: 1650000000,
                    end_ts: None,
                },
            ],
            last_updated_utc: now.timestamp(),
            profile_completeness: 0.0,
        };
        let records = vec![EngagementRecord {
            person_id: "test".into(),
            date: now - chrono::Duration::days(5),
            method: "email".into(),
            outcome: EngagementOutcome::Positive,
            notes: "Good response".into(),
            next_action: None,
            next_due: None,
        }];
        let score = compute_profile_engagement_score(&profile, &records, now);
        assert!(score > 0.5, "Expected strong engagement score, got {score}");
    }
}
