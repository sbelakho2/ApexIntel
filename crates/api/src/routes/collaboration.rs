//! Collaboration routes — Investigation workspaces, team assignments, activity feed.
//!
//! Phase 4.3: User Experience Enhancement

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use apex_store::postgres::{InvestigationWorkspaceRecord, PgStore};

use crate::responses::ApiError;

// ────────────────────────────────────────────
// Investigation Workspace Types
// ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvestigationWorkspace {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub workspace_type: String,
    pub owner_id: String,
    pub team_id: Option<String>,
    pub status: String,
    pub visibility: String,
    pub tags: Vec<String>,
    pub entity_focus: Value,
    pub findings: Option<String>,
    pub conclusions: Option<String>,
    pub metadata: Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub closed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateWorkspaceRequest {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub workspace_type: String,
    #[serde(default)]
    pub team_id: Option<String>,
    #[serde(default)]
    pub visibility: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub entity_focus: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateWorkspaceRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub entity_focus: Option<Value>,
    #[serde(default)]
    pub findings: Option<String>,
    #[serde(default)]
    pub conclusions: Option<String>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceAssignment {
    pub id: String,
    pub workspace_id: String,
    pub user_id: String,
    pub role: String,
    pub assigned_by: String,
    pub assigned_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssignUserRequest {
    pub user_id: String,
    #[serde(default)]
    pub role: String,
}

// ────────────────────────────────────────────
// Activity Feed Types
// ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityEntry {
    pub id: String,
    pub actor_id: String,
    pub actor_name: String,
    pub action_type: String,
    pub entity_type: Option<String>,
    pub entity_id: Option<String>,
    pub entity_name: Option<String>,
    pub details: Value,
    pub formatted_details: String,
    pub workspace_id: Option<String>,
    pub team_id: Option<String>,
    pub visibility: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ActivityFeedQuery {
    pub workspace_id: Option<String>,
    pub team_id: Option<String>,
    pub actor_id: Option<String>,
    pub action_type: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordActivityRequest {
    pub action_type: String,
    pub entity_type: Option<String>,
    pub entity_id: Option<String>,
    pub entity_name: Option<String>,
    #[serde(default)]
    pub details: Value,
    #[serde(default)]
    pub workspace_id: Option<String>,
    #[serde(default)]
    pub team_id: Option<String>,
    #[serde(default)]
    pub visibility: String,
}

/// Format a JSON value for inline display — returns the string for strings,
/// empty string for null, serialized JSON for everything else.
pub fn fmt_json_value(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Null => String::new(),
        serde_json::Value::String(s) => s.clone(),
        _ => serde_json::to_string(v).unwrap_or_default(),
    }
}

/// Format activity-feed details into a human-readable summary string based on the
/// `action_type`.  System events logged by [`ActivityLogger`] store structured JSON
/// in `details`; this function extracts the relevant fields and produces a short
/// description that can be safely rendered as inline text.
pub fn format_activity_details(action_type: &str, details: &serde_json::Value) -> String {
    let obj = match details {
        serde_json::Value::Object(m) => m,
        serde_json::Value::String(s) => return s.clone(),
        serde_json::Value::Null => return String::new(),
        other => return serde_json::to_string(other).unwrap_or_default(),
    };

    match action_type {
        // ── System event types (from ActivityLogger) ──────────────────────
        "insight_generated" => {
            let title = obj.get("title").and_then(|v| v.as_str()).unwrap_or("");
            let confidence = obj.get("confidence").and_then(|v| v.as_f64());
            // An absent confidence is unknown, never "0%" (audit sweep).
            match (title.is_empty(), confidence) {
                (false, Some(confidence)) => {
                    format!("\"{}\" (conf: {:.0}%)", title, confidence * 100.0)
                }
                (false, None) => format!("\"{}\"", title),
                (true, Some(confidence)) => {
                    format!("Confidence: {:.0}%", confidence * 100.0)
                }
                (true, None) => String::new(),
            }
        }
        "poi_discovered" => {
            let role = obj.get("role").and_then(|v| v.as_str()).unwrap_or("");
            let company = obj.get("company").and_then(|v| v.as_str()).unwrap_or("");
            if !role.is_empty() && !company.is_empty() {
                format!("{} at {}", role, company)
            } else if !role.is_empty() {
                role.to_string()
            } else if !company.is_empty() {
                format!("at {}", company)
            } else {
                String::new()
            }
        }
        "crawl_completed" => {
            let urls = obj.get("urls_crawled").and_then(|v| v.as_u64());
            let obs = obj.get("new_observations").and_then(|v| v.as_u64());
            let secs = obj.get("duration_secs").and_then(|v| v.as_f64());
            // Omit unmeasured counters instead of reporting them as 0.
            match (urls, obs, secs) {
                (Some(urls), Some(obs), Some(secs)) => {
                    format!("{urls} pages crawled, {obs} new observations (in {secs:.0}s)")
                }
                (urls, obs, secs) => {
                    let mut parts: Vec<String> = Vec::new();
                    if let Some(urls) = urls {
                        parts.push(format!("{urls} pages crawled"));
                    }
                    if let Some(obs) = obs {
                        parts.push(format!("{obs} new observations"));
                    }
                    if let Some(secs) = secs {
                        parts.push(format!("in {secs:.0}s"));
                    }
                    parts.join(", ")
                }
            }
        }
        "company_detected" => {
            let signal = obj
                .get("signal_type")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let region = obj.get("region").and_then(|v| v.as_str());
            match (signal, region) {
                (s, Some(r)) if !s.is_empty() => format!("{} · {}", s, r),
                (s, _) if !s.is_empty() => s.to_string(),
                _ => String::new(),
            }
        }
        "threat_detected" => {
            let ttype = obj
                .get("threat_type")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let sev = obj.get("severity").and_then(|v| v.as_str()).unwrap_or("");
            if !ttype.is_empty() && !sev.is_empty() {
                format!("{} [{}]", ttype, sev)
            } else if !ttype.is_empty() {
                ttype.to_string()
            } else if !sev.is_empty() {
                format!("Severity: {}", sev)
            } else {
                String::new()
            }
        }
        "psych_profile_updated" => {
            let quality = obj.get("profile_quality").and_then(|v| v.as_f64());
            match quality {
                Some(quality) => format!("Profile quality: {quality:.2}"),
                None => "Profile quality not recorded".to_string(),
            }
        }
        "battlecard_generated" => {
            let comp = obj.get("competitor").and_then(|v| v.as_str()).unwrap_or("");
            if !comp.is_empty() {
                format!("vs {}", comp)
            } else {
                String::new()
            }
        }
        "memo_generated" => {
            let title = obj.get("title").and_then(|v| v.as_str()).unwrap_or("");
            let count = obj.get("entity_count").and_then(|v| v.as_u64());
            // An absent count is omitted, not claimed as 0 entities.
            match (title.is_empty(), count) {
                (false, Some(count)) => format!("\"{title}\" ({count} entities)"),
                (false, None) => format!("\"{title}\""),
                (true, Some(count)) => format!("{count} entities"),
                (true, None) => String::new(),
            }
        }
        "recipe_promoted" => {
            let code = obj
                .get("recipe_code")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let cat = obj.get("category").and_then(|v| v.as_str()).unwrap_or("");
            if !code.is_empty() && !cat.is_empty() {
                format!("{} [{}]", code, cat)
            } else if !code.is_empty() {
                code.to_string()
            } else if !cat.is_empty() {
                cat.to_string()
            } else {
                String::new()
            }
        }
        "job_completed" | "job_failed" | "job_skipped" | "job_unknown" => {
            let job_kind = obj.get("job_kind").and_then(|v| v.as_str()).unwrap_or("");
            let items = obj.get("items_processed").and_then(|v| v.as_u64());
            let dur = obj.get("duration_ms").and_then(|v| v.as_u64());
            let notes = obj.get("notes").and_then(|v| v.as_str()).unwrap_or("");
            let status = obj
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or(action_type);
            // Only measured counters appear; a missing value is not 0 items.
            let measured = match (items, dur) {
                (Some(items), Some(dur)) => format!(" ({items} items, {dur}ms)"),
                (Some(items), None) => format!(" ({items} items)"),
                (None, Some(dur)) => format!(" ({dur}ms)"),
                (None, None) => String::new(),
            };
            let mut out = format!("{job_kind} — {status}{measured}");
            if !notes.is_empty() {
                out.push_str(&format!(" — {}", notes));
            }
            out
        }

        // ── Collaboration CRUD action types ───────────────────────────────
        // For these, details often contains a free-text message or comment.
        _ => {
            // Try to extract a "message" or "comment" field first
            if let Some(msg) = obj
                .get("message")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
            {
                return msg.to_string();
            }
            if let Some(comment) = obj
                .get("comment")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
            {
                return comment.to_string();
            }
            // Fall back to generic JSON rendering
            fmt_json_value(details)
        }
    }
}

// ────────────────────────────────────────────
// Investigation Shares
// ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvestigationShare {
    pub id: String,
    pub workspace_id: String,
    pub shared_by: String,
    pub shared_with: String,
    pub share_type: String,
    pub access_level: String,
    pub message: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShareWorkspaceRequest {
    pub shared_with: String,
    #[serde(default)]
    pub share_type: String,
    #[serde(default)]
    pub access_level: String,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub expires_at: Option<DateTime<Utc>>,
}

// ────────────────────────────────────────────
// Daily Priority Queue Types
// ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriorityQueueItem {
    pub id: String,
    pub user_id: String,
    pub queue_date: String,
    pub item_type: String,
    pub item_id: String,
    pub item_title: String,
    pub priority: i32,
    pub status: String,
    pub notes: Option<String>,
    pub completed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AddToQueueRequest {
    pub item_type: String,
    pub item_id: String,
    pub item_title: String,
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateQueueItemRequest {
    #[serde(default)]
    pub priority: Option<i32>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
}

// ────────────────────────────────────────────
// Supplier Risk Types
// ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SupplierRiskEntry {
    pub id: String,
    pub supplier_id: String,
    pub risk_category: String,
    pub risk_score: f64,
    pub risk_factors: Value,
    pub mitigation: Option<String>,
    pub owner_id: Option<String>,
    pub created_by: Option<String>,
    pub status: String,
    pub last_reviewed: Option<DateTime<Utc>>,
    pub next_review: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AddSupplierRiskRequest {
    pub supplier_id: String,
    pub risk_category: String,
    #[serde(default)]
    pub risk_score: f64,
    #[serde(default)]
    pub risk_factors: Value,
    #[serde(default)]
    pub mitigation: Option<String>,
    #[serde(default)]
    pub owner_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateSupplierRiskRequest {
    #[serde(default)]
    pub risk_score: Option<f64>,
    #[serde(default)]
    pub risk_factors: Option<Value>,
    #[serde(default)]
    pub mitigation: Option<String>,
    #[serde(default)]
    pub owner_id: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
}

// ────────────────────────────────────────────
// Pipeline Opportunity Types
// ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineOpportunity {
    pub id: String,
    pub opportunity_id: Option<String>,
    pub title: String,
    pub stage: String,
    pub value_estimate: Option<f64>,
    pub probability: f64,
    pub owner_id: Option<String>,
    pub created_by: Option<String>,
    pub expected_close: Option<String>,
    pub actual_close: Option<String>,
    pub notes: Option<String>,
    pub metadata: Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub closed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreatePipelineOpportunityRequest {
    #[serde(default)]
    pub opportunity_id: Option<String>,
    pub title: String,
    #[serde(default)]
    pub stage: String,
    #[serde(default)]
    pub value_estimate: Option<f64>,
    #[serde(default)]
    pub probability: f64,
    #[serde(default)]
    pub owner_id: Option<String>,
    #[serde(default)]
    pub expected_close: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdatePipelineStageRequest {
    pub stage: String,
    #[serde(default)]
    pub notes: Option<String>,
}

// ────────────────────────────────────────────
// Source Evidence Types
// ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceEvidence {
    pub id: String,
    pub entity_type: String,
    pub entity_id: String,
    pub evidence_type: String,
    pub source_url: String,
    pub source_domain: Option<String>,
    pub source_name: Option<String>,
    pub reliability_score: f64,
    pub content_hash: Option<String>,
    pub excerpt: Option<String>,
    pub metadata: Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AddEvidenceRequest {
    pub entity_type: String,
    pub entity_id: String,
    pub evidence_type: String,
    pub source_url: String,
    #[serde(default)]
    pub source_name: Option<String>,
    #[serde(default)]
    pub reliability_score: f64,
    #[serde(default)]
    pub excerpt: Option<String>,
    #[serde(default)]
    pub metadata: Value,
}

// ────────────────────────────────────────────
// Team Assignment Types
// ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamAssignment {
    pub id: String,
    pub team_id: String,
    pub team_name: String,
    pub entity_type: String,
    pub entity_id: String,
    pub assigned_by: String,
    pub assigned_to: String,
    pub role: String,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateTeamAssignmentRequest {
    pub team_id: String,
    pub team_name: String,
    pub entity_type: String,
    pub entity_id: String,
    pub assigned_to: String,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub notes: Option<String>,
}

// ────────────────────────────────────────────
// Strategic Opportunities & Critical Threats
// ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategicOpportunity {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub opportunity_type: String,
    pub priority_score: f64,
    pub confidence: f64,
    pub entity_id: Option<String>,
    pub entity_type: Option<String>,
    pub region: Option<String>,
    pub estimated_value: Option<String>,
    pub recommended_actions: Value,
    pub owner_id: Option<String>,
    pub created_by: Option<String>,
    pub status: String,
    pub due_date: Option<DateTime<Utc>>,
    pub metadata: Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CriticalThreat {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub threat_type: String,
    pub severity: String,
    pub impact_score: f64,
    pub confidence: f64,
    pub entity_id: Option<String>,
    pub entity_type: Option<String>,
    pub region: Option<String>,
    pub mitigation_steps: Value,
    pub owner_id: Option<String>,
    pub created_by: Option<String>,
    pub status: String,
    pub sla_deadline: Option<DateTime<Utc>>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub metadata: Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateOpportunityRequest {
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    pub opportunity_type: String,
    #[serde(default)]
    pub priority_score: f64,
    #[serde(default)]
    pub confidence: f64,
    #[serde(default)]
    pub entity_id: Option<String>,
    #[serde(default)]
    pub entity_type: Option<String>,
    #[serde(default)]
    pub region: Option<String>,
    #[serde(default)]
    pub estimated_value: Option<String>,
    #[serde(default)]
    pub recommended_actions: Value,
    #[serde(default)]
    pub owner_id: Option<String>,
    #[serde(default)]
    pub due_date: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateThreatRequest {
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    pub threat_type: String,
    #[serde(default)]
    pub severity: String,
    #[serde(default)]
    pub impact_score: f64,
    #[serde(default)]
    pub confidence: f64,
    #[serde(default)]
    pub entity_id: Option<String>,
    #[serde(default)]
    pub entity_type: Option<String>,
    #[serde(default)]
    pub region: Option<String>,
    #[serde(default)]
    pub mitigation_steps: Value,
    #[serde(default)]
    pub owner_id: Option<String>,
    #[serde(default)]
    pub sla_deadline: Option<DateTime<Utc>>,
}

/// PATCH payload for a strategic opportunity. Every field is optional; absent
/// fields are left untouched by the store (`COALESCE`), and only the fields
/// that are present are validated.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct UpdateOpportunityRequest {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub opportunity_type: Option<String>,
    #[serde(default)]
    pub priority_score: Option<f64>,
    #[serde(default)]
    pub confidence: Option<f64>,
    #[serde(default)]
    pub entity_id: Option<String>,
    #[serde(default)]
    pub entity_type: Option<String>,
    #[serde(default)]
    pub region: Option<String>,
    #[serde(default)]
    pub estimated_value: Option<String>,
    #[serde(default)]
    pub recommended_actions: Option<Value>,
    #[serde(default)]
    pub owner_id: Option<String>,
    #[serde(default)]
    pub due_date: Option<DateTime<Utc>>,
    #[serde(default)]
    pub status: Option<String>,
}

/// PATCH payload for a critical threat. `status` accepts the full status set
/// including `resolved`; `severity` is validated only when present.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct UpdateThreatRequest {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub threat_type: Option<String>,
    #[serde(default)]
    pub severity: Option<String>,
    #[serde(default)]
    pub impact_score: Option<f64>,
    #[serde(default)]
    pub confidence: Option<f64>,
    #[serde(default)]
    pub entity_id: Option<String>,
    #[serde(default)]
    pub entity_type: Option<String>,
    #[serde(default)]
    pub region: Option<String>,
    #[serde(default)]
    pub mitigation_steps: Option<Value>,
    #[serde(default)]
    pub owner_id: Option<String>,
    #[serde(default)]
    pub sla_deadline: Option<DateTime<Utc>>,
    #[serde(default)]
    pub status: Option<String>,
}

// ────────────────────────────────────────────
// Validation Functions
//
// Canonical value sets shared by the JSON API and the server-rendered web
// forms. The API is the reference: the database check constraints mirror
// these exact strings.
// ────────────────────────────────────────────

pub const WORKSPACE_TYPES: [&str; 4] = ["ad-hoc", "structured", "incident", "ongoing"];
pub const WORKSPACE_VISIBILITIES: [&str; 4] = ["private", "team", "organization", "public"];
pub const EVIDENCE_TYPES: [&str; 10] = [
    "web_content",
    "document",
    "financial_report",
    "news_article",
    "social_media",
    "regulatory_filing",
    "patent",
    "court_record",
    "public_record",
    "analyst_report",
];
pub const RISK_CATEGORIES: [&str; 8] = [
    "financial",
    "operational",
    "compliance",
    "geopolitical",
    "environmental",
    "technological",
    "reputational",
    "strategic",
];
pub const WORKSPACE_ASSIGNMENT_ROLES: [&str; 5] =
    ["owner", "lead", "contributor", "viewer", "reviewer"];
pub const TEAM_ASSIGNMENT_ROLES: [&str; 4] = ["lead", "contributor", "reviewer", "observer"];
pub const SHARE_TYPES: [&str; 3] = ["view", "collaborate", "embed"];
pub const ACCESS_LEVELS: [&str; 3] = ["read", "read_write", "admin"];
pub const PIPELINE_STAGES: [&str; 6] = [
    "discovery",
    "qualification",
    "proposal",
    "negotiation",
    "closed_won",
    "closed_lost",
];
pub const THREAT_STATUSES: [&str; 4] = ["active", "monitoring", "resolved", "escalated"];
/// The statuses accepted by the `strategic_opportunities` check constraint.
pub const OPPORTUNITY_STATUSES: [&str; 4] = ["active", "pursued", "completed", "abandoned"];

/// Unit-interval check that also rejects `NaN`/`inf`, which
/// `serde_urlencoded` happily parses from web form values.
fn validate_unit_interval(field: &str, value: f64) -> Result<(), ApiError> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(ApiError::validation(
            field,
            "must be a finite number between 0.0 and 1.0",
        ));
    }
    Ok(())
}

fn validate_member(field: &str, value: &str, allowed: &[&str]) -> Result<(), ApiError> {
    if !allowed.contains(&value) {
        return Err(ApiError::validation(
            field,
            format!("must be one of: {}", allowed.join(", ")),
        ));
    }
    Ok(())
}

/// Canonical workspace-name rule, shared by the API and the web forms:
/// 3–255 characters after trimming.
pub fn validate_workspace_name(name: &str) -> Result<(), ApiError> {
    let trimmed = name.trim();
    if trimmed.len() < 3 {
        return Err(ApiError::validation(
            "name",
            "must be at least 3 characters",
        ));
    }
    if trimmed.len() > 255 {
        return Err(ApiError::validation(
            "name",
            "must not exceed 255 characters",
        ));
    }
    Ok(())
}

pub fn validate_workspace_type(value: &str) -> Result<(), ApiError> {
    validate_member("workspace_type", value.trim(), &WORKSPACE_TYPES)
}

pub fn validate_visibility(value: &str) -> Result<(), ApiError> {
    validate_member("visibility", value.trim(), &WORKSPACE_VISIBILITIES)
}

/// Literals accepted by `activity_feed.chk_activity_action_type` (migration
/// 100, which widens the 002/062 baseline with every type the code writes).
/// `POST /api/activity` validates against this list so an unknown value is a
/// 400 instead of a database CHECK violation surfacing as a 500.
pub const ACTIVITY_ACTION_TYPES: [&str; 33] = [
    // Collaboration actions (migration 002)
    "create",
    "update",
    "delete",
    "share",
    "assign",
    "comment",
    "resolve",
    "reopen",
    "escalate",
    "deescalate",
    "approve",
    "reject",
    "merge",
    "split",
    // System event types
    "insight_generated",
    "poi_discovered",
    "crawl_completed",
    "company_detected",
    "threat_detected",
    "psych_profile_updated",
    "battlecard_generated",
    "memo_generated",
    "recipe_promoted",
    // Job lifecycle events (062 adds job_degraded)
    "job_completed",
    "job_degraded",
    "job_failed",
    "job_skipped",
    // Triage fallback (crates/triage/src/router_integration.rs)
    "triage_alert",
    "triage_queue_update",
    "triage_status_change",
    // Event outbox dead-letter operator alert
    "alert_dead_lettered",
    // Warning ingress lifecycle
    "warning_created",
    "warning_updated",
];

pub fn validate_activity_action_type(value: &str) -> Result<(), ApiError> {
    validate_member("action_type", value.trim(), &ACTIVITY_ACTION_TYPES)
}

pub fn validate_workspace_request(req: &CreateWorkspaceRequest) -> Result<(), ApiError> {
    validate_workspace_name(&req.name)?;
    validate_workspace_type(&req.workspace_type)?;
    validate_visibility(&req.visibility)?;
    Ok(())
}

pub fn validate_priority(value: i32) -> Result<(), ApiError> {
    if !(1..=100).contains(&value) {
        return Err(ApiError::validation(
            "priority",
            "must be between 1 and 100",
        ));
    }
    Ok(())
}

pub fn validate_confidence(value: f64) -> Result<(), ApiError> {
    validate_unit_interval("confidence", value)
}

pub fn validate_priority_score(value: f64) -> Result<(), ApiError> {
    validate_unit_interval("priority_score", value)
}

pub fn validate_impact_score(value: f64) -> Result<(), ApiError> {
    validate_unit_interval("impact_score", value)
}

pub fn validate_risk_score(value: f64) -> Result<(), ApiError> {
    validate_unit_interval("risk_score", value)
}

pub fn validate_reliability_score(value: f64) -> Result<(), ApiError> {
    validate_unit_interval("reliability_score", value)
}

pub fn validate_probability(value: f64) -> Result<(), ApiError> {
    validate_unit_interval("probability", value)
}

pub fn validate_severity(value: &str) -> Result<(), ApiError> {
    let normalized = value.trim().to_ascii_lowercase();
    validate_member(
        "severity",
        &normalized,
        &["low", "medium", "high", "critical"],
    )
}

pub fn validate_stage(value: &str) -> Result<(), ApiError> {
    validate_member("stage", value.trim(), &PIPELINE_STAGES)
}

pub fn validate_evidence_type(value: &str) -> Result<(), ApiError> {
    validate_member("evidence_type", value.trim(), &EVIDENCE_TYPES)
}

pub fn validate_risk_category(value: &str) -> Result<(), ApiError> {
    validate_member("risk_category", value.trim(), &RISK_CATEGORIES)
}

pub fn validate_workspace_assignment_role(value: &str) -> Result<(), ApiError> {
    validate_member("role", value.trim(), &WORKSPACE_ASSIGNMENT_ROLES)
}

pub fn validate_team_assignment_role(value: &str) -> Result<(), ApiError> {
    validate_member("role", value.trim(), &TEAM_ASSIGNMENT_ROLES)
}

pub fn validate_share_type(value: &str) -> Result<(), ApiError> {
    validate_member("share_type", value.trim(), &SHARE_TYPES)
}

pub fn validate_access_level(value: &str) -> Result<(), ApiError> {
    validate_member("access_level", value.trim(), &ACCESS_LEVELS)
}

pub fn validate_threat_status(value: &str) -> Result<(), ApiError> {
    validate_member("status", value.trim(), &THREAT_STATUSES)
}

pub fn validate_opportunity_status(value: &str) -> Result<(), ApiError> {
    validate_member("status", value.trim(), &OPPORTUNITY_STATUSES)
}

/// Trims surrounding whitespace from an optional string, returning `None` when
/// the value is absent or empty after trimming. Used to sanitise free-text
/// fields before persistence so blank submissions are stored as `NULL`.
pub fn normalize_optional_text(text: Option<String>) -> Option<String> {
    text.and_then(|t| {
        let trimmed = t.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

// ────────────────────────────────────────────
// Store error mapping
// ────────────────────────────────────────────

/// Map a store failure to a client-visible error without leaking database
/// detail (table/column/constraint names). The full error is logged with an
/// incident id the client can quote.
pub fn store_error(error: anyhow::Error) -> ApiError {
    let incident = uuid::Uuid::new_v4();
    tracing::error!(error = %error, %incident, "collaboration store error");
    ApiError::internal(format!("internal error · incident {incident}"))
}

/// Resolves a caller-supplied record owner to a provisioned `app_users.id`.
///
/// Blank means "no owner" (`None`; on updates the stored owner is kept). Any
/// other value must name an existing user: `owner_id` on opportunities,
/// threats, supplier risk and pipeline rows references `app_users(id)`
/// (migration 101), so an unknown id is a 400, never a constraint 500.
pub async fn resolve_record_owner(
    store: &PgStore,
    owner_id: Option<&str>,
) -> Result<Option<String>, ApiError> {
    let Some(owner_id) = owner_id.map(str::trim).filter(|id| !id.is_empty()) else {
        return Ok(None);
    };
    match store.get_app_user(owner_id).await.map_err(store_error)? {
        Some(user) => Ok(Some(user.id)),
        None => Err(ApiError::validation(
            "owner_id",
            "must be an existing user id",
        )),
    }
}

// ────────────────────────────────────────────
// Workspace authorization
// ────────────────────────────────────────────

/// Access level a caller needs on an investigation workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WsAccess {
    Read,
    Write,
    Manage,
}

/// Shared authorization guard for every workspace surface (API handlers and
/// web pages). Denials are reported as `404 not found` so the existence of a
/// private workspace is not leaked.
///
/// Rules:
///   * owner or platform admin: full access;
///   * an unexpired share grants read, `read_write` write, `admin` manage;
///   * a workspace assignment grants read, owner/lead/contributor write,
///     owner/lead manage;
///   * `organization`/`public` visibility grants read to everyone.
///   * `team` and `private` grant nothing beyond the rules above: a
///     workspace's team is its assignees and share recipients (there is no
///     separate team-membership model), and the create form says so.
pub async fn authorize_workspace(
    store: &PgStore,
    id: uuid::Uuid,
    user: &str,
    is_admin: bool,
    need: WsAccess,
) -> Result<InvestigationWorkspaceRecord, ApiError> {
    let workspace = store
        .get_investigation_workspace(id)
        .await
        .map_err(store_error)?
        .ok_or_else(|| ApiError::not_found("workspace", &id.to_string()))?;

    if is_admin || workspace.owner_id == user {
        return Ok(workspace);
    }

    let now = Utc::now();
    let share = store
        .list_investigation_shares(id)
        .await
        .map_err(store_error)?
        .into_iter()
        .filter(|s| s.shared_with == user && s.expires_at.is_none_or(|expires| expires > now))
        .map(|s| s.access_level)
        .max_by_key(|level| match level.as_str() {
            "admin" => 3,
            "read_write" => 2,
            _ => 1,
        });
    let role = store
        .list_workspace_assignments(id)
        .await
        .map_err(store_error)?
        .into_iter()
        .find(|assignment| assignment.user_id == user)
        .map(|assignment| assignment.role);

    let read = matches!(workspace.visibility.as_str(), "organization" | "public")
        || share.is_some()
        || role.is_some();
    let write = matches!(share.as_deref(), Some("read_write" | "admin"))
        || matches!(role.as_deref(), Some("owner" | "lead" | "contributor"));
    let manage =
        share.as_deref() == Some("admin") || matches!(role.as_deref(), Some("owner" | "lead"));

    let allowed = match need {
        WsAccess::Read => read,
        WsAccess::Write => write,
        WsAccess::Manage => manage,
    };
    if allowed {
        Ok(workspace)
    } else {
        Err(ApiError::not_found("workspace", &id.to_string()))
    }
}

// ────────────────────────────────────────────
// Executive Dashboard Types
// ────────────────────────────────────────────

/// Query parameters for the executive dashboard summary endpoint.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExecutiveQuery {
    pub include_threats: Option<bool>,
    pub include_opportunities: Option<bool>,
    pub region_filter: Option<String>,
    pub priority_threshold: Option<f64>,
}

/// A recommended next action surfaced on the executive dashboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecommendedAction {
    pub id: String,
    pub title: String,
    pub description: String,
    pub priority: String,
    pub owner: String,
    pub due_date: Option<String>,
    pub related_entity_id: Option<String>,
    pub related_entity_type: Option<String>,
}

// ────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_workspace_request_accepts_valid_name() {
        let req = CreateWorkspaceRequest {
            name: "Valid Workspace Name".to_string(),
            description: None,
            workspace_type: "ad-hoc".to_string(),
            team_id: None,
            visibility: "private".to_string(),
            tags: vec![],
            entity_focus: Value::Null,
        };
        assert!(validate_workspace_request(&req).is_ok());
    }

    #[test]
    fn validate_workspace_request_rejects_short_name() {
        let req = CreateWorkspaceRequest {
            name: "X".to_string(),
            description: None,
            workspace_type: "".to_string(),
            team_id: None,
            visibility: "".to_string(),
            tags: vec![],
            entity_focus: Value::Null,
        };
        assert!(validate_workspace_request(&req).is_err());
    }

    #[test]
    fn validate_priority_accepts_valid_range() {
        assert!(validate_priority(1).is_ok());
        assert!(validate_priority(50).is_ok());
        assert!(validate_priority(100).is_ok());
    }

    #[test]
    fn validate_priority_rejects_out_of_range() {
        assert!(validate_priority(0).is_err());
        assert!(validate_priority(101).is_err());
    }

    #[test]
    fn validate_confidence_accepts_valid_range() {
        assert!(validate_confidence(0.0).is_ok());
        assert!(validate_confidence(0.5).is_ok());
        assert!(validate_confidence(1.0).is_ok());
    }

    #[test]
    fn validate_confidence_rejects_out_of_range() {
        assert!(validate_confidence(-0.1).is_err());
        assert!(validate_confidence(1.1).is_err());
    }

    #[test]
    fn validate_confidence_rejects_nan_and_infinity() {
        assert!(validate_confidence(f64::NAN).is_err());
        assert!(validate_confidence(f64::INFINITY).is_err());
        assert!(validate_confidence(f64::NEG_INFINITY).is_err());
    }

    #[test]
    fn validate_evidence_types_match_the_database_constraint() {
        assert!(validate_evidence_type("news_article").is_ok());
        assert!(validate_evidence_type("web_content").is_ok());
        assert!(validate_evidence_type("news").is_err());
    }

    #[test]
    fn validate_roles_match_the_database_constraints() {
        assert!(validate_workspace_assignment_role("contributor").is_ok());
        assert!(validate_workspace_assignment_role("observer").is_err());
        assert!(validate_team_assignment_role("observer").is_ok());
        assert!(validate_team_assignment_role("owner").is_err());
    }

    #[test]
    fn validate_severity_accepts_valid_values() {
        assert!(validate_severity("critical").is_ok());
        assert!(validate_severity("HIGH").is_ok());
        assert!(validate_severity("Medium").is_ok());
    }

    #[test]
    fn validate_severity_rejects_invalid_values() {
        assert!(validate_severity("extreme").is_err());
        assert!(validate_severity("lowest").is_err());
    }

    #[test]
    fn validate_activity_action_type_matches_the_database_constraint() {
        // Every literal from migration 100's chk_activity_action_type.
        for value in [
            "create",
            "update",
            "delete",
            "share",
            "assign",
            "comment",
            "resolve",
            "reopen",
            "escalate",
            "deescalate",
            "approve",
            "reject",
            "merge",
            "split",
            "insight_generated",
            "poi_discovered",
            "crawl_completed",
            "company_detected",
            "threat_detected",
            "psych_profile_updated",
            "battlecard_generated",
            "memo_generated",
            "recipe_promoted",
            "job_completed",
            "job_degraded",
            "job_failed",
            "job_skipped",
            "triage_alert",
            "triage_queue_update",
            "triage_status_change",
            "alert_dead_lettered",
            "warning_created",
            "warning_updated",
        ] {
            assert!(
                validate_activity_action_type(value).is_ok(),
                "{value} must be accepted"
            );
        }
        assert_eq!(ACTIVITY_ACTION_TYPES.len(), 33);
    }

    #[test]
    fn validate_activity_action_type_rejects_unknown_values() {
        assert!(validate_activity_action_type("").is_err());
        assert!(validate_activity_action_type("drop_table").is_err());
        assert!(validate_activity_action_type("CREATE").is_err());
        assert!(validate_activity_action_type("job_started").is_err());
    }

    #[test]
    fn validate_stage_accepts_valid_values() {
        assert!(validate_stage("discovery").is_ok());
        assert!(validate_stage("proposal").is_ok());
        assert!(validate_stage("closed_won").is_ok());
    }

    #[test]
    fn validate_stage_rejects_invalid_values() {
        assert!(validate_stage("unknown").is_err());
        assert!(validate_stage("early").is_err());
    }
}

#[cfg(test)]
mod comprehensive_tests {
    use super::*;

    // ── Investigation Workspace Tests ─────────────────────────────────────────

    #[test]
    fn investigation_workspace_serialization() {
        let workspace = InvestigationWorkspace {
            id: "ws-123".to_string(),
            name: "Supply Chain Analysis".to_string(),
            description: Some("Investigate supply chain risks".to_string()),
            workspace_type: "structured".to_string(),
            owner_id: "analyst-1".to_string(),
            team_id: Some("team-security".to_string()),
            status: "active".to_string(),
            visibility: "team".to_string(),
            tags: vec!["supply-chain".to_string(), "risk".to_string()],
            entity_focus: serde_json::json!(["company-123"]),
            findings: Some("Initial findings".to_string()),
            conclusions: Some("Final conclusions".to_string()),
            metadata: serde_json::json!({"priority": "high"}),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            closed_at: None,
        };

        let json = serde_json::to_string(&workspace).unwrap();
        let parsed: InvestigationWorkspace = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.name, "Supply Chain Analysis");
        assert_eq!(parsed.workspace_type, "structured");
        assert_eq!(parsed.tags.len(), 2);
    }

    #[test]
    fn create_workspace_request_validation() {
        let req = CreateWorkspaceRequest {
            name: "Valid Workspace".to_string(),
            description: Some("A valid workspace".to_string()),
            workspace_type: "ad-hoc".to_string(),
            team_id: Some("team-1".to_string()),
            visibility: "private".to_string(),
            tags: vec!["tag1".to_string()],
            entity_focus: serde_json::json!([]),
        };

        assert!(validate_workspace_request(&req).is_ok());
    }

    #[test]
    fn create_workspace_request_empty_name_rejected() {
        let req = CreateWorkspaceRequest {
            name: "".to_string(),
            description: None,
            workspace_type: "".to_string(),
            team_id: None,
            visibility: "".to_string(),
            tags: vec![],
            entity_focus: Value::Null,
        };

        assert!(validate_workspace_request(&req).is_err());
    }

    #[test]
    fn create_workspace_request_short_name_rejected() {
        let req = CreateWorkspaceRequest {
            name: "X".to_string(),
            description: None,
            workspace_type: "".to_string(),
            team_id: None,
            visibility: "".to_string(),
            tags: vec![],
            entity_focus: Value::Null,
        };

        assert!(validate_workspace_request(&req).is_err());
    }

    #[test]
    fn create_workspace_request_invalid_type_rejected() {
        let req = CreateWorkspaceRequest {
            name: "Valid Workspace Name".to_string(),
            description: None,
            workspace_type: "invalid".to_string(),
            team_id: None,
            visibility: "".to_string(),
            tags: vec![],
            entity_focus: Value::Null,
        };

        assert!(validate_workspace_request(&req).is_err());
    }

    // ── Priority Queue Tests ───────────────────────────────────────────────────

    #[test]
    fn priority_queue_item_serialization() {
        let item = PriorityQueueItem {
            id: "item-123".to_string(),
            user_id: "user-456".to_string(),
            queue_date: "2024-01-15".to_string(),
            item_type: "warning".to_string(),
            item_id: "warning-789".to_string(),
            item_title: "Review critical alert".to_string(),
            priority: 85,
            status: "pending".to_string(),
            notes: Some("Requires immediate attention".to_string()),
            completed_at: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        let json = serde_json::to_string(&item).unwrap();
        let parsed: PriorityQueueItem = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.priority, 85);
        assert_eq!(parsed.status, "pending");
        assert!(parsed.notes.is_some());
    }

    #[test]
    fn add_to_queue_request_serialization() {
        let req = AddToQueueRequest {
            item_type: "insight".to_string(),
            item_id: "insight-123".to_string(),
            item_title: "Follow up on market insight".to_string(),
            priority: 60,
            notes: Some("Check competitor activity".to_string()),
        };

        let json = serde_json::to_string(&req).unwrap();
        let parsed: AddToQueueRequest = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.item_type, "insight");
        assert_eq!(parsed.priority, 60);
    }

    #[test]
    fn update_queue_item_request_serialization() {
        let req = UpdateQueueItemRequest {
            priority: Some(90),
            status: Some("in_progress".to_string()),
            notes: Some("Updated notes".to_string()),
        };

        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("90"));
        assert!(json.contains("in_progress"));
    }

    // ── Supplier Risk Tests ────────────────────────────────────────────────────

    #[test]
    fn supplier_risk_entry_serialization() {
        let entry = SupplierRiskEntry {
            id: "risk-123".to_string(),
            supplier_id: "supplier-456".to_string(),
            risk_category: "financial".to_string(),
            risk_score: 0.75,
            risk_factors: serde_json::json!(["High debt", "Unstable revenue"]),
            mitigation: Some("Diversify suppliers".to_string()),
            owner_id: Some("risk-analyst".to_string()),
            created_by: Some("admin".to_string()),
            status: "active".to_string(),
            last_reviewed: Some(chrono::Utc::now()),
            next_review: Some(chrono::Utc::now()),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        let json = serde_json::to_string(&entry).unwrap();
        let parsed: SupplierRiskEntry = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.risk_category, "financial");
        assert_eq!(parsed.risk_score, 0.75);
    }

    #[test]
    fn add_supplier_risk_request_serialization() {
        let req = AddSupplierRiskRequest {
            supplier_id: "00000000-0000-0000-0000-000000000001".to_string(),
            risk_category: "operational".to_string(),
            risk_score: 0.55,
            risk_factors: serde_json::json!(["Single source dependency"]),
            mitigation: None,
            owner_id: None,
        };

        let json = serde_json::to_string(&req).unwrap();
        let parsed: AddSupplierRiskRequest = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.risk_category, "operational");
        assert!(parsed.mitigation.is_none());
    }

    // ── Pipeline Opportunity Tests ─────────────────────────────────────────────

    #[test]
    fn pipeline_opportunity_serialization() {
        let opp = PipelineOpportunity {
            id: "pipeline-123".to_string(),
            opportunity_id: Some("opp-456".to_string()),
            title: "New Client Acquisition".to_string(),
            stage: "proposal".to_string(),
            value_estimate: Some(2_000_000.0),
            probability: 0.65,
            owner_id: Some("sales-lead".to_string()),
            created_by: Some("admin".to_string()),
            expected_close: Some("2024-06-30".to_string()),
            actual_close: None,
            notes: Some("Strong interest from client".to_string()),
            metadata: serde_json::json!({"source": "referral"}),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            closed_at: None,
        };

        let json = serde_json::to_string(&opp).unwrap();
        let parsed: PipelineOpportunity = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.stage, "proposal");
        assert_eq!(parsed.value_estimate, Some(2_000_000.0));
        assert_eq!(parsed.probability, 0.65);
    }

    #[test]
    fn create_pipeline_opportunity_request_serialization() {
        let req = CreatePipelineOpportunityRequest {
            opportunity_id: Some("opp-789".to_string()),
            title: "Enterprise Deal".to_string(),
            stage: "discovery".to_string(),
            value_estimate: Some(5_000_000.0),
            probability: 0.3,
            owner_id: Some("account-manager".to_string()),
            expected_close: Some("2024-09-30".to_string()),
            notes: None,
        };

        let json = serde_json::to_string(&req).unwrap();
        let parsed: CreatePipelineOpportunityRequest = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.stage, "discovery");
        assert_eq!(parsed.probability, 0.3);
    }

    #[test]
    fn update_pipeline_stage_request_serialization() {
        let req = UpdatePipelineStageRequest {
            stage: "negotiation".to_string(),
            notes: Some("Contract terms being finalized".to_string()),
        };

        let json = serde_json::to_string(&req).unwrap();
        let parsed: UpdatePipelineStageRequest = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.stage, "negotiation");
    }

    // ── Source Evidence Tests ───────────────────────────────────────────────────

    #[test]
    fn source_evidence_serialization() {
        let evidence = SourceEvidence {
            id: "evidence-123".to_string(),
            entity_type: "company".to_string(),
            entity_id: "company-456".to_string(),
            evidence_type: "web_content".to_string(),
            source_url: "https://news.example.com/article".to_string(),
            source_domain: Some("news.example.com".to_string()),
            source_name: Some("Example News".to_string()),
            reliability_score: 0.82,
            content_hash: Some("abc123".to_string()),
            excerpt: Some("Key quote from the article...".to_string()),
            metadata: serde_json::json!({"crawled_at": "2024-01-15"}),
            created_at: chrono::Utc::now(),
        };

        let json = serde_json::to_string(&evidence).unwrap();
        let parsed: SourceEvidence = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.evidence_type, "web_content");
        assert_eq!(parsed.reliability_score, 0.82);
    }

    #[test]
    fn add_evidence_request_serialization() {
        let req = AddEvidenceRequest {
            entity_type: "company".to_string(),
            entity_id: "00000000-0000-0000-0000-000000000001".to_string(),
            evidence_type: "financial_report".to_string(),
            source_url: "https://sec.gov/filings/123".to_string(),
            source_name: Some("SEC Filing".to_string()),
            reliability_score: 0.95,
            excerpt: Some("Revenue increased 20%".to_string()),
            metadata: serde_json::json!({"filing_date": "2024-01-10"}),
        };

        let json = serde_json::to_string(&req).unwrap();
        let parsed: AddEvidenceRequest = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.evidence_type, "financial_report");
        assert_eq!(parsed.reliability_score, 0.95);
    }

    // ── Team Assignment Tests ───────────────────────────────────────────────────

    #[test]
    fn team_assignment_serialization() {
        let assignment = TeamAssignment {
            id: "assignment-123".to_string(),
            team_id: "team-security".to_string(),
            team_name: "Security Team".to_string(),
            entity_type: "warning".to_string(),
            entity_id: "warning-456".to_string(),
            assigned_by: "admin-1".to_string(),
            assigned_to: "analyst-2".to_string(),
            role: "lead".to_string(),
            notes: Some("Critical security issue".to_string()),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        let json = serde_json::to_string(&assignment).unwrap();
        let parsed: TeamAssignment = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.team_id, "team-security");
        assert_eq!(parsed.role, "lead");
    }

    #[test]
    fn create_team_assignment_request_serialization() {
        let req = CreateTeamAssignmentRequest {
            team_id: "team-compliance".to_string(),
            team_name: "Compliance Team".to_string(),
            entity_type: "warning".to_string(),
            entity_id: "00000000-0000-0000-0000-000000000001".to_string(),
            assigned_to: "compliance-officer".to_string(),
            role: "contributor".to_string(),
            notes: Some("Regulatory requirement".to_string()),
        };

        let json = serde_json::to_string(&req).unwrap();
        let parsed: CreateTeamAssignmentRequest = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.team_id, "team-compliance");
        assert_eq!(parsed.role, "contributor");
    }

    // ── Activity Feed Tests ────────────────────────────────────────────────────

    #[test]
    fn activity_entry_serialization() {
        let entry = ActivityEntry {
            id: "activity-123".to_string(),
            actor_id: "user-456".to_string(),
            actor_name: "John Doe".to_string(),
            action_type: "create".to_string(),
            entity_type: Some("warning".to_string()),
            entity_id: Some("warning-789".to_string()),
            entity_name: Some("Security Alert #1234".to_string()),
            details: serde_json::json!({"severity": "high"}),
            formatted_details: String::new(),
            workspace_id: Some("ws-111".to_string()),
            team_id: Some("team-security".to_string()),
            visibility: "team".to_string(),
            created_at: chrono::Utc::now(),
        };

        let json = serde_json::to_string(&entry).unwrap();
        let parsed: ActivityEntry = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.action_type, "create");
        assert!(parsed.entity_id.is_some());
        assert!(parsed.workspace_id.is_some());
    }

    #[test]
    fn activity_feed_query_default_values() {
        let query = ActivityFeedQuery::default();

        assert!(query.workspace_id.is_none());
        assert!(query.team_id.is_none());
        assert!(query.actor_id.is_none());
        assert!(query.action_type.is_none());
        assert!(query.limit.is_none());
        assert!(query.offset.is_none());
    }

    #[test]
    fn record_activity_request_serialization() {
        let req = RecordActivityRequest {
            action_type: "update".to_string(),
            entity_type: Some("insight".to_string()),
            entity_id: Some("insight-123".to_string()),
            entity_name: Some("Market Trend Analysis".to_string()),
            details: serde_json::json!({"changes": ["added evidence"]}),
            workspace_id: None,
            team_id: Some("team-analytics".to_string()),
            visibility: "team".to_string(),
        };

        let json = serde_json::to_string(&req).unwrap();
        let parsed: RecordActivityRequest = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.action_type, "update");
        assert!(parsed.workspace_id.is_none());
        assert!(parsed.team_id.is_some());
    }

    // ── Investigation Share Tests ───────────────────────────────────────────────

    #[test]
    fn investigation_share_serialization() {
        let share = InvestigationShare {
            id: "share-123".to_string(),
            workspace_id: "ws-456".to_string(),
            shared_by: "analyst-1".to_string(),
            shared_with: "analyst-2".to_string(),
            share_type: "collaborate".to_string(),
            access_level: "read_write".to_string(),
            message: Some("Let's work on this together".to_string()),
            expires_at: Some(chrono::Utc::now()),
            created_at: chrono::Utc::now(),
        };

        let json = serde_json::to_string(&share).unwrap();
        let parsed: InvestigationShare = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.share_type, "collaborate");
        assert_eq!(parsed.access_level, "read_write");
        assert!(parsed.message.is_some());
    }

    #[test]
    fn share_workspace_request_serialization() {
        let req = ShareWorkspaceRequest {
            shared_with: "external-partner@company.com".to_string(),
            share_type: "view".to_string(),
            access_level: "read".to_string(),
            message: Some("For review purposes only".to_string()),
            expires_at: None,
        };

        let json = serde_json::to_string(&req).unwrap();
        let parsed: ShareWorkspaceRequest = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.share_type, "view");
        assert_eq!(parsed.access_level, "read");
        assert!(parsed.expires_at.is_none());
    }

    // ── Strategic Opportunity Tests ────────────────────────────────────────────

    #[test]
    fn strategic_opportunity_serialization() {
        let opp = StrategicOpportunity {
            id: "opp-123".to_string(),
            title: "New Market Entry".to_string(),
            description: Some("Expand to APAC region".to_string()),
            opportunity_type: "market_expansion".to_string(),
            priority_score: 0.88,
            confidence: 0.75,
            entity_id: Some("company-456".to_string()),
            entity_type: Some("company".to_string()),
            region: Some("APAC".to_string()),
            estimated_value: Some("$10M".to_string()),
            recommended_actions: serde_json::json!(["Hire local team", "Partner with distributor"]),
            owner_id: Some("expansion-lead".to_string()),
            created_by: Some("admin".to_string()),
            status: "active".to_string(),
            due_date: Some(chrono::Utc::now()),
            metadata: serde_json::json!({"source": "market_analysis"}),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        let json = serde_json::to_string(&opp).unwrap();
        let parsed: StrategicOpportunity = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.title, "New Market Entry");
        assert_eq!(parsed.priority_score, 0.88);
        assert_eq!(parsed.region, Some("APAC".to_string()));
    }

    #[test]
    fn create_opportunity_request_serialization() {
        let req = CreateOpportunityRequest {
            title: "Technology Partnership".to_string(),
            description: Some("Partner with tech leader".to_string()),
            opportunity_type: "partnership".to_string(),
            priority_score: 0.72,
            confidence: 0.68,
            entity_id: None,
            entity_type: None,
            region: Some("NA".to_string()),
            estimated_value: Some("$3M".to_string()),
            recommended_actions: serde_json::json!(["Evaluate fit", "Negotiate terms"]),
            owner_id: Some("bd-team".to_string()),
            due_date: Some(chrono::Utc::now()),
        };

        let json = serde_json::to_string(&req).unwrap();
        let parsed: CreateOpportunityRequest = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.opportunity_type, "partnership");
        assert_eq!(parsed.priority_score, 0.72);
    }

    // ── Critical Threat Tests ─────────────────────────────────────────────────

    #[test]
    fn critical_threat_serialization() {
        let threat = CriticalThreat {
            id: "threat-123".to_string(),
            title: "Regulatory Change Risk".to_string(),
            description: Some("New regulations may impact operations".to_string()),
            threat_type: "regulatory".to_string(),
            severity: "high".to_string(),
            impact_score: 0.85,
            confidence: 0.78,
            entity_id: Some("company-456".to_string()),
            entity_type: Some("company".to_string()),
            region: Some("EMEA".to_string()),
            mitigation_steps: serde_json::json!([
                "Monitor regulatory updates",
                "Engage compliance team"
            ]),
            owner_id: Some("compliance-lead".to_string()),
            created_by: Some("admin".to_string()),
            status: "active".to_string(),
            sla_deadline: Some(chrono::Utc::now()),
            resolved_at: None,
            metadata: serde_json::json!({"regulatory_body": "EU Commission"}),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        let json = serde_json::to_string(&threat).unwrap();
        let parsed: CriticalThreat = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.title, "Regulatory Change Risk");
        assert_eq!(parsed.severity, "high");
        assert_eq!(parsed.impact_score, 0.85);
    }

    #[test]
    fn create_threat_request_serialization() {
        let req = CreateThreatRequest {
            title: "Cybersecurity Vulnerability".to_string(),
            description: Some("Potential data breach risk".to_string()),
            threat_type: "cybersecurity".to_string(),
            severity: "critical".to_string(),
            impact_score: 0.95,
            confidence: 0.85,
            entity_id: None,
            entity_type: None,
            region: None,
            mitigation_steps: serde_json::json!(["Patch vulnerability", "Monitor systems"]),
            owner_id: Some("security-team".to_string()),
            sla_deadline: Some(chrono::Utc::now()),
        };

        let json = serde_json::to_string(&req).unwrap();
        let parsed: CreateThreatRequest = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.threat_type, "cybersecurity");
        assert_eq!(parsed.severity, "critical");
        assert_eq!(parsed.impact_score, 0.95);
    }

    // ── Workspace Assignment Tests ────────────────────────────────────────────

    #[test]
    fn workspace_assignment_serialization() {
        let assignment = WorkspaceAssignment {
            id: "assignment-123".to_string(),
            workspace_id: "ws-456".to_string(),
            user_id: "analyst-789".to_string(),
            role: "contributor".to_string(),
            assigned_by: "admin-1".to_string(),
            assigned_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        let json = serde_json::to_string(&assignment).unwrap();
        let parsed: WorkspaceAssignment = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.role, "contributor");
        assert_eq!(parsed.user_id, "analyst-789");
    }

    #[test]
    fn assign_user_request_serialization() {
        let req = AssignUserRequest {
            user_id: "new-analyst".to_string(),
            role: "viewer".to_string(),
        };

        let json = serde_json::to_string(&req).unwrap();
        let parsed: AssignUserRequest = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.user_id, "new-analyst");
        assert_eq!(parsed.role, "viewer");
    }

    // ── Executive Query Tests ──────────────────────────────────────────────────

    #[test]
    fn executive_query_default_values() {
        let query = ExecutiveQuery::default();

        assert!(query.include_threats.is_none());
        assert!(query.include_opportunities.is_none());
        assert!(query.region_filter.is_none());
        assert!(query.priority_threshold.is_none());
    }

    #[test]
    fn executive_query_with_all_filters() {
        let query = ExecutiveQuery {
            include_threats: Some(true),
            include_opportunities: Some(true),
            region_filter: Some("NA".to_string()),
            priority_threshold: Some(0.7),
        };

        assert_eq!(query.include_threats, Some(true));
        assert_eq!(query.include_opportunities, Some(true));
        assert_eq!(query.region_filter, Some("NA".to_string()));
        assert_eq!(query.priority_threshold, Some(0.7));
    }

    // ── Recommended Action Tests ─────────────────────────────────────────────

    #[test]
    fn recommended_action_serialization() {
        let action = RecommendedAction {
            id: "action-123".to_string(),
            title: "Address Supply Chain Risk".to_string(),
            description: "Identify and mitigate supply chain vulnerabilities".to_string(),
            priority: "critical".to_string(),
            owner: "supply-chain-manager".to_string(),
            due_date: Some("2024-04-15T00:00:00Z".to_string()),
            related_entity_id: Some("supplier-456".to_string()),
            related_entity_type: Some("company".to_string()),
        };

        let json = serde_json::to_string(&action).unwrap();
        let parsed: RecommendedAction = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.priority, "critical");
        assert!(parsed.due_date.is_some());
        assert_eq!(parsed.related_entity_type, Some("company".to_string()));
    }

    // ── Update Workspace Request Tests ─────────────────────────────────────────

    #[test]
    fn update_workspace_request_serialization() {
        let req = UpdateWorkspaceRequest {
            name: Some("Updated Workspace Name".to_string()),
            description: Some("Updated description".to_string()),
            status: Some("closed".to_string()),
            tags: Some(vec!["updated".to_string(), "closed".to_string()]),
            entity_focus: Some(serde_json::json!(["entity-1", "entity-2"])),
            findings: Some("Final findings".to_string()),
            conclusions: Some("Final conclusions".to_string()),
            metadata: None,
        };

        let json = serde_json::to_string(&req).unwrap();
        let parsed: UpdateWorkspaceRequest = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.name, Some("Updated Workspace Name".to_string()));
        assert_eq!(parsed.status, Some("closed".to_string()));
        assert!(parsed.findings.is_some());
    }

    #[test]
    fn update_workspace_request_partial_update() {
        let req = UpdateWorkspaceRequest {
            name: None,
            description: Some("Only description updated".to_string()),
            status: None,
            tags: None,
            entity_focus: None,
            findings: None,
            conclusions: None,
            metadata: None,
        };

        let json = serde_json::to_string(&req).unwrap();
        let parsed: UpdateWorkspaceRequest = serde_json::from_str(&json).unwrap();

        assert!(parsed.name.is_none());
        assert!(parsed.description.is_some());
        assert!(parsed.status.is_none());
    }

    // ── Validation Edge Cases ─────────────────────────────────────────────────

    #[test]
    fn validate_priority_boundary_values() {
        assert!(validate_priority(1).is_ok());
        assert!(validate_priority(100).is_ok());
        assert!(validate_priority(50).is_ok());
    }

    #[test]
    fn validate_priority_out_of_range() {
        assert!(validate_priority(0).is_err());
        assert!(validate_priority(101).is_err());
        assert!(validate_priority(-1).is_err());
    }

    #[test]
    fn validate_confidence_boundary_values() {
        assert!(validate_confidence(0.0).is_ok());
        assert!(validate_confidence(1.0).is_ok());
        assert!(validate_confidence(0.5).is_ok());
    }

    #[test]
    fn validate_confidence_out_of_range() {
        assert!(validate_confidence(-0.1).is_err());
        assert!(validate_confidence(1.1).is_err());
    }

    #[test]
    fn validate_severity_all_valid_values() {
        assert!(validate_severity("low").is_ok());
        assert!(validate_severity("medium").is_ok());
        assert!(validate_severity("high").is_ok());
        assert!(validate_severity("critical").is_ok());
    }

    #[test]
    fn validate_severity_case_insensitive() {
        assert!(validate_severity("LOW").is_ok());
        assert!(validate_severity("Medium").is_ok());
        assert!(validate_severity("HIGH").is_ok());
        assert!(validate_severity("CRITICAL").is_ok());
    }

    #[test]
    fn validate_severity_invalid_values() {
        assert!(validate_severity("extreme").is_err());
        assert!(validate_severity("minor").is_err());
        assert!(validate_severity("major").is_err());
    }

    #[test]
    fn validate_stage_all_valid_values() {
        assert!(validate_stage("discovery").is_ok());
        assert!(validate_stage("qualification").is_ok());
        assert!(validate_stage("proposal").is_ok());
        assert!(validate_stage("negotiation").is_ok());
        assert!(validate_stage("closed_won").is_ok());
        assert!(validate_stage("closed_lost").is_ok());
    }

    #[test]
    fn validate_stage_invalid_values() {
        assert!(validate_stage("pending").is_err());
        assert!(validate_stage("started").is_err());
        assert!(validate_stage("finished").is_err());
    }

    // ── Normalization Tests ───────────────────────────────────────────────────

    #[test]
    fn normalize_optional_text_trims_whitespace() {
        let result = normalize_optional_text(Some("  test  ".to_string()));
        assert_eq!(result, Some("test".to_string()));
    }

    #[test]
    fn normalize_optional_text_removes_empty() {
        let result = normalize_optional_text(Some("   ".to_string()));
        assert!(result.is_none());
    }

    #[test]
    fn normalize_optional_text_handles_none() {
        let result = normalize_optional_text(None);
        assert!(result.is_none());
    }

    #[test]
    fn normalize_optional_text_preserves_valid() {
        let result = normalize_optional_text(Some("valid text".to_string()));
        assert_eq!(result, Some("valid text".to_string()));
    }

    // ── Activity detail formatting must not fabricate measurements ───────────

    #[test]
    fn activity_details_omit_unmeasured_values_instead_of_zeroing_them() {
        let text = format_activity_details(
            "insight_generated",
            &serde_json::json!({"title": "Chip shortage"}),
        );
        assert_eq!(text, "\"Chip shortage\"");
        assert!(!text.contains("0%"));

        let text = format_activity_details("crawl_completed", &serde_json::json!({}));
        assert_eq!(text, "");

        let text = format_activity_details("psych_profile_updated", &serde_json::json!({}));
        assert!(text.contains("not recorded"));

        let text =
            format_activity_details("memo_generated", &serde_json::json!({"title": "Week 1"}));
        assert_eq!(text, "\"Week 1\"");

        let text = format_activity_details(
            "job_completed",
            &serde_json::json!({"job_kind": "crawl", "status": "ok"}),
        );
        assert_eq!(text, "crawl — ok");
    }

    #[test]
    fn activity_details_preserve_measured_values() {
        let text = format_activity_details(
            "crawl_completed",
            &serde_json::json!({
                "urls_crawled": 8,
                "new_observations": 3,
                "duration_secs": 12,
            }),
        );
        assert_eq!(text, "8 pages crawled, 3 new observations (in 12s)");

        let text = format_activity_details(
            "job_completed",
            &serde_json::json!({
                "job_kind": "crawl",
                "status": "ok",
                "items_processed": 4,
                "duration_ms": 1200,
            }),
        );
        assert_eq!(text, "crawl — ok (4 items, 1200ms)");

        let text = format_activity_details(
            "insight_generated",
            &serde_json::json!({"title": "Chip shortage", "confidence": 0.87}),
        );
        assert_eq!(text, "\"Chip shortage\" (conf: 87%)");
    }
}
