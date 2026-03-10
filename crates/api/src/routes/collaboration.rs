use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

fn normalize_optional_text(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalystUser {
    pub id: String,
    pub display_name: String,
    pub email: Option<String>,
    pub role: String,
    pub notification_channels: Value,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpsertAnalystUserRequest {
    pub display_name: String,
    pub email: Option<String>,
    pub role: String,
    #[serde(default)]
    pub notification_channels: Value,
    pub is_active: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedSearch {
    pub id: String,
    pub user_id: String,
    pub name: String,
    pub query_text: String,
    pub filters: Value,
    pub default_sort: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpsertSavedSearchRequest {
    pub name: String,
    pub query_text: String,
    #[serde(default)]
    pub filters: Value,
    pub default_sort: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Watchlist {
    pub id: String,
    pub user_id: String,
    pub name: String,
    pub entities: Value,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpsertWatchlistRequest {
    pub name: String,
    #[serde(default)]
    pub entities: Value,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Annotation {
    pub id: String,
    pub user_id: String,
    pub entity_type: String,
    pub entity_id: String,
    pub body: String,
    pub tags: Vec<String>,
    pub visibility: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpsertAnnotationRequest {
    pub entity_type: String,
    pub entity_id: String,
    pub body: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub visibility: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnnotationQuery {
    pub entity_type: Option<String>,
    pub entity_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportHistoryEntry {
    pub id: String,
    pub user_id: String,
    pub export_type: String,
    pub format: String,
    pub filters: Value,
    pub row_count: i64,
    pub download_name: Option<String>,
    pub requested_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditLogQuery {
    pub actor: Option<String>,
    pub event_type: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditLogEntry {
    pub id: String,
    pub event_type: String,
    pub actor: String,
    pub detail: Value,
    pub created_at: DateTime<Utc>,
}

pub fn validate_user_request(request: &UpsertAnalystUserRequest) -> Result<(), String> {
    if request.display_name.trim().len() < 2 {
        return Err("display_name must be at least 2 characters".to_string());
    }
    let role = request.role.trim().to_ascii_lowercase();
    if !matches!(role.as_str(), "admin" | "analyst" | "viewer" | "service") {
        return Err("role must be one of admin, analyst, viewer, service".to_string());
    }
    Ok(())
}

pub fn validate_saved_search_request(request: &UpsertSavedSearchRequest) -> Result<(), String> {
    if request.name.trim().is_empty() {
        return Err("name is required".to_string());
    }
    if request.query_text.trim().len() < 2 {
        return Err("query_text must be at least 2 characters".to_string());
    }
    Ok(())
}

pub fn validate_watchlist_request(request: &UpsertWatchlistRequest) -> Result<(), String> {
    if request.name.trim().is_empty() {
        return Err("name is required".to_string());
    }
    if !request.entities.is_array() {
        return Err("entities must be a JSON array".to_string());
    }
    Ok(())
}

pub fn validate_annotation_request(request: &UpsertAnnotationRequest) -> Result<(), String> {
    if request.entity_type.trim().is_empty() {
        return Err("entity_type is required".to_string());
    }
    if request.entity_id.trim().is_empty() {
        return Err("entity_id is required".to_string());
    }
    if request.body.trim().len() < 3 {
        return Err("body must be at least 3 characters".to_string());
    }
    Ok(())
}

pub fn sanitized_email(value: Option<String>) -> Option<String> {
    normalize_optional_text(value)
}

pub fn sanitized_notes(value: Option<String>) -> Option<String> {
    normalize_optional_text(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_saved_search_rejects_short_query() {
        let request = UpsertSavedSearchRequest {
            name: "Focus".to_string(),
            query_text: "a".to_string(),
            filters: Value::Null,
            default_sort: None,
        };

        assert!(validate_saved_search_request(&request).is_err());
    }

    #[test]
    fn sanitized_notes_trims_blanks() {
        assert_eq!(
            sanitized_notes(Some("  note  ".to_string())),
            Some("note".to_string())
        );
        assert_eq!(sanitized_notes(Some("   ".to_string())), None);
    }
}
