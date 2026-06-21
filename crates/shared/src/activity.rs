//! Activity event types shared across crates.

use serde::{Deserialize, Serialize};

/// An activity feed event — used across API, worker, and frontend.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ActivityEvent {
    pub id: String,
    pub event_type: String,
    pub title: String,
    pub description: Option<String>,
    pub severity: String,
    pub timestamp: String,
    pub entity_name: Option<String>,
    pub entity_id: Option<String>,
    pub source: Option<String>,
    pub source_url: Option<String>,
}

impl ActivityEvent {
    /// Create a new activity event with the given type and title.
    pub fn new(event_type: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: String::new(),
            event_type: event_type.into(),
            title: title.into(),
            description: None,
            severity: "low".to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            entity_name: None,
            entity_id: None,
            source: None,
            source_url: None,
        }
    }

    /// Set the severity level.
    pub fn with_severity(mut self, severity: impl Into<String>) -> Self {
        self.severity = severity.into();
        self
    }

    /// Set the description.
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Set the entity this event relates to.
    pub fn with_entity(mut self, name: impl Into<String>, id: impl Into<String>) -> Self {
        self.entity_name = Some(name.into());
        self.entity_id = Some(id.into());
        self
    }

    /// Set the source of this event.
    pub fn with_source(mut self, source: impl Into<String>, url: Option<String>) -> Self {
        self.source = Some(source.into());
        self.source_url = url;
        self
    }
}

/// Activity event type constants — keep in sync with the DB CHECK constraint.
pub mod event_types {
    pub const INSIGHT_GENERATED: &str = "insight_generated";
    pub const POI_DISCOVERED: &str = "poi_discovered";
    pub const CRAWL_COMPLETED: &str = "crawl_completed";
    pub const COMPANY_DETECTED: &str = "company_detected";
    pub const THREAT_DETECTED: &str = "threat_detected";
    pub const PSYCH_PROFILE_UPDATED: &str = "psych_profile_updated";
    pub const BATTLECARD_GENERATED: &str = "battlecard_generated";
    pub const MEMO_GENERATED: &str = "memo_generated";
    pub const RECIPE_PROMOTED: &str = "recipe_promoted";
    pub const JOB_COMPLETED: &str = "job_completed";
    pub const JOB_FAILED: &str = "job_failed";
    pub const JOB_SKIPPED: &str = "job_skipped";
    pub const POI_UPDATED: &str = "poi_updated";
    pub const COMPANY_UPDATED: &str = "company_updated";
}