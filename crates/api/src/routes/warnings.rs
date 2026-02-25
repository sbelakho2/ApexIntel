//! Warnings route — request/response types and logic for the warnings endpoints.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use apex_core::validation::{normalize_email, validate_nonempty_id, validate_uuid};

// ────────────────────────────────────────────
// Request types
// ────────────────────────────────────────────

/// Query parameters for listing warnings.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListWarningsQuery {
    pub page: Option<u32>,
    pub per_page: Option<u32>,
    pub regions: Option<String>,
    pub severities: Option<String>,
    pub warning_types: Option<String>,
    pub date_from: Option<String>,
    pub date_to: Option<String>,
    pub search: Option<String>,
    pub acknowledged: Option<bool>,
    pub sort_by: Option<WarningSortField>,
    pub sort_dir: Option<SortDirection>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum WarningSortField {
    CreatedAt,
    Severity,
    Type,
}

impl WarningSortField {
    pub fn from_str_loose(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "created_at" | "created" | "date" => Some(Self::CreatedAt),
            "severity" | "sev" => Some(Self::Severity),
            "type" | "warning_type" => Some(Self::Type),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum SortDirection {
    Asc,
    Desc,
}

impl Default for SortDirection {
    fn default() -> Self {
        Self::Desc
    }
}

impl SortDirection {
    pub fn from_str_loose(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "asc" | "ascending" => Self::Asc,
            _ => Self::Desc,
        }
    }
}

/// Request body for acknowledging a warning.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcknowledgeRequest {
    pub user_id: String,
    pub note: Option<String>,
}

// ────────────────────────────────────────────
// Response types
// ────────────────────────────────────────────

/// Single warning in list/detail responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WarningResponse {
    pub id: String,
    pub title: String,
    pub description: String,
    pub severity: String,
    pub warning_type: String,
    pub region: String,
    pub source_urls: Vec<String>,
    pub entity_ids: Vec<String>,
    pub recipe_id: Option<String>,
    pub confidence: f64,
    pub acknowledged: bool,
    pub acknowledged_by: Option<String>,
    pub acknowledged_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Acknowledge response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcknowledgeResponse {
    pub warning_id: String,
    pub acknowledged: bool,
    pub acknowledged_by: String,
    pub acknowledged_at: DateTime<Utc>,
}

// ────────────────────────────────────────────
// Logic
// ────────────────────────────────────────────

/// Validate a warning ID.
pub fn validate_warning_id(id: &str) -> Result<Uuid, String> {
    validate_uuid(id, "warning_id").map_err(|e| e.to_string())?;
    Uuid::parse_str(id.trim()).map_err(|_| format!("Invalid warning ID: '{}'", id))
}

/// Validate an acknowledge request.
pub fn validate_acknowledge(req: &AcknowledgeRequest) -> Result<(), String> {
    if req.user_id.trim().is_empty() {
        return Err("user_id is required".to_string());
    }
    if req.user_id.chars().count() > 200 {
        return Err("user_id must be <= 200 characters".to_string());
    }
    if let Some(email) = normalize_email(&req.user_id) {
        if !email.contains('@') && req.user_id.contains('@') {
            return Err("user_id must be a valid email when '@' is present".to_string());
        }
    }
    if let Some(note) = &req.note {
        if note.chars().count() > 1000 {
            return Err("note must be <= 1000 characters".to_string());
        }
    }
    Ok(())
}

/// Sort warnings in-place by field and direction.
pub fn sort_warnings(
    warnings: &mut [WarningResponse],
    field: &WarningSortField,
    direction: &SortDirection,
) {
    warnings.sort_by(|a, b| {
        let cmp = match field {
            WarningSortField::CreatedAt => a.created_at.cmp(&b.created_at),
            WarningSortField::Severity => {
                severity_rank(&a.severity).cmp(&severity_rank(&b.severity))
            }
            WarningSortField::Type => a.warning_type.cmp(&b.warning_type),
        };
        match direction {
            SortDirection::Asc => cmp,
            SortDirection::Desc => cmp.reverse(),
        }
    });
}

/// Map severity label to numeric rank for sorting.
fn severity_rank(sev: &str) -> u8 {
    match sev.to_lowercase().as_str() {
        "critical" => 4,
        "high" => 3,
        "medium" => 2,
        "low" => 1,
        _ => 0,
    }
}

/// Filter warnings by acknowledged status.
pub fn filter_by_acknowledged(warnings: &[WarningResponse], acked: Option<bool>) -> Vec<WarningResponse> {
    match acked {
        Some(true) => warnings.iter().filter(|w| w.acknowledged).cloned().collect(),
        Some(false) => warnings.iter().filter(|w| !w.acknowledged).cloned().collect(),
        None => warnings.to_vec(),
    }
}

/// Count warnings by severity.
pub fn count_by_severity(warnings: &[WarningResponse]) -> Vec<(String, usize)> {
    let mut counts = std::collections::HashMap::new();
    for w in warnings {
        *counts.entry(w.severity.clone()).or_insert(0usize) += 1;
    }
    let mut result: Vec<_> = counts.into_iter().collect();
    result.sort_by(|a, b| severity_rank(&b.0).cmp(&severity_rank(&a.0)));
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_warning(severity: &str, warning_type: &str, acked: bool, minutes_ago: i64) -> WarningResponse {
        let now = Utc::now();
        WarningResponse {
            id: Uuid::new_v4().to_string(),
            title: format!("{} warning", severity),
            description: "Test".to_string(),
            severity: severity.to_string(),
            warning_type: warning_type.to_string(),
            region: "TN".to_string(),
            source_urls: vec!["https://example.com".to_string()],
            entity_ids: vec![],
            recipe_id: None,
            confidence: 0.9,
            acknowledged: acked,
            acknowledged_by: None,
            acknowledged_at: None,
            created_at: now - chrono::Duration::minutes(minutes_ago),
            updated_at: now,
        }
    }

    #[test]
    fn test_validate_warning_id_valid() {
        let id = Uuid::new_v4().to_string();
        assert!(validate_warning_id(&id).is_ok());
    }

    #[test]
    fn test_validate_warning_id_invalid() {
        assert!(validate_warning_id("not-a-uuid").is_err());
    }

    #[test]
    fn test_validate_acknowledge_ok() {
        let req = AcknowledgeRequest {
            user_id: "user-1".to_string(),
            note: Some("Noted".to_string()),
        };
        assert!(validate_acknowledge(&req).is_ok());
    }

    #[test]
    fn test_validate_acknowledge_empty_user() {
        let req = AcknowledgeRequest {
            user_id: "  ".to_string(),
            note: None,
        };
        assert!(validate_acknowledge(&req).is_err());
    }

    #[test]
    fn test_validate_acknowledge_long_note() {
        let req = AcknowledgeRequest {
            user_id: "user-1".to_string(),
            note: Some("x".repeat(1001)),
        };
        assert!(validate_acknowledge(&req).is_err());
    }

    #[test]
    fn test_validate_acknowledge_long_user_id() {
        let req = AcknowledgeRequest {
            user_id: "u".repeat(201),
            note: None,
        };
        assert!(validate_acknowledge(&req).is_err());
    }

    #[test]
    fn test_sort_warnings_by_severity_desc() {
        let mut warnings = vec![
            make_warning("low", "security", false, 10),
            make_warning("critical", "supply_chain", false, 5),
            make_warning("medium", "market", false, 20),
        ];
        sort_warnings(&mut warnings, &WarningSortField::Severity, &SortDirection::Desc);
        assert_eq!(warnings[0].severity, "critical");
        assert_eq!(warnings[1].severity, "medium");
        assert_eq!(warnings[2].severity, "low");
    }

    #[test]
    fn test_sort_warnings_by_created_asc() {
        let mut warnings = vec![
            make_warning("high", "security", false, 5),  // newer
            make_warning("high", "security", false, 30), // older
            make_warning("high", "security", false, 15),
        ];
        sort_warnings(&mut warnings, &WarningSortField::CreatedAt, &SortDirection::Asc);
        // oldest first
        assert!(warnings[0].created_at <= warnings[1].created_at);
        assert!(warnings[1].created_at <= warnings[2].created_at);
    }

    #[test]
    fn test_filter_by_acknowledged() {
        let warnings = vec![
            make_warning("high", "security", true, 10),
            make_warning("low", "market", false, 5),
            make_warning("medium", "regulatory", true, 20),
        ];
        assert_eq!(filter_by_acknowledged(&warnings, Some(true)).len(), 2);
        assert_eq!(filter_by_acknowledged(&warnings, Some(false)).len(), 1);
        assert_eq!(filter_by_acknowledged(&warnings, None).len(), 3);
    }

    #[test]
    fn test_count_by_severity() {
        let warnings = vec![
            make_warning("high", "a", false, 1),
            make_warning("high", "b", false, 2),
            make_warning("low", "c", false, 3),
            make_warning("critical", "d", false, 4),
        ];
        let counts = count_by_severity(&warnings);
        // sorted by rank descending: critical, high, low
        assert_eq!(counts[0].0, "critical");
        assert_eq!(counts[0].1, 1);
        assert_eq!(counts[1].0, "high");
        assert_eq!(counts[1].1, 2);
    }

    #[test]
    fn test_sort_direction_default() {
        assert_eq!(SortDirection::default(), SortDirection::Desc);
    }

    #[test]
    fn test_sort_direction_from_str() {
        assert_eq!(SortDirection::from_str_loose("asc"), SortDirection::Asc);
        assert_eq!(SortDirection::from_str_loose("ascending"), SortDirection::Asc);
        assert_eq!(SortDirection::from_str_loose("desc"), SortDirection::Desc);
        assert_eq!(SortDirection::from_str_loose("xyz"), SortDirection::Desc);
    }

    #[test]
    fn test_warning_sort_field_from_str() {
        assert_eq!(
            WarningSortField::from_str_loose("severity"),
            Some(WarningSortField::Severity)
        );
        assert_eq!(
            WarningSortField::from_str_loose("date"),
            Some(WarningSortField::CreatedAt)
        );
        assert_eq!(WarningSortField::from_str_loose("xyz"), None);
    }

    #[test]
    fn test_warning_response_serialization() {
        let w = make_warning("high", "security", false, 5);
        let json = serde_json::to_string(&w).unwrap();
        assert!(json.contains("\"severity\":\"high\""));
        assert!(json.contains("\"warning_type\":\"security\""));
    }
}
