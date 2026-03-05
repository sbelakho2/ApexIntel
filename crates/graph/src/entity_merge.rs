//! Entity merge/split tracking.
//!
//! When entity resolution merges or splits companies or persons, this module
//! records the event with before/after UUIDs so graph history is never lost.
//!
//! Merge events are stored in the `entity_merges` + `audit_log` tables.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::info;

// ─────────────────────────────────────────────────────────────────────────────
// Types
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityMergeEvent {
    pub merge_id: String,
    pub entity_type: EntityType,
    pub source_ids: Vec<String>,
    pub target_id: String,
    pub reason: MergeReason,
    pub confidence: f64,
    pub merged_by: String,
    pub merged_at: DateTime<Utc>,
    pub field_resolutions: Vec<FieldResolution>,
    pub rollback_available: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum EntityType {
    Company,
    Person,
    Site,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MergeReason {
    /// Automatic entity resolution match
    AutoResolution { similarity_score: f64 },
    /// Manual merge by operator
    ManualMerge { operator_notes: String },
    /// Detected corporate acquisition
    Acquisition { acquiring_company: String },
    /// Name change / rebranding
    NameChange { old_name: String, new_name: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldResolution {
    pub field_name: String,
    pub source_values: Vec<(String, String)>, // (entity_id, value)
    pub resolved_value: String,
    pub resolution_strategy: ResolutionStrategy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ResolutionStrategy {
    KeepNewest,
    KeepHighestConfidence,
    Concatenate,
    ManualChoice,
    KeepNonEmpty,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntitySplitEvent {
    pub split_id: String,
    pub entity_type: EntityType,
    pub source_id: String,
    pub target_ids: Vec<String>,
    pub reason: String,
    pub split_by: String,
    pub split_at: DateTime<Utc>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Merge execution
// ─────────────────────────────────────────────────────────────────────────────

/// Build merge event record for two entities being merged.
pub fn build_merge_event(
    entity_type: EntityType,
    source_ids: Vec<String>,
    target_id: String,
    reason: MergeReason,
    confidence: f64,
    merged_by: &str,
) -> EntityMergeEvent {
    let merge_id = format!(
        "merge_{}_{}",
        match &entity_type {
            EntityType::Company => "company",
            EntityType::Person => "person",
            EntityType::Site => "site",
        },
        Utc::now().timestamp_millis()
    );

    info!(
        merge_id = %merge_id,
        entity_type = ?entity_type,
        sources = ?source_ids,
        target = %target_id,
        "Entity merge event created"
    );

    EntityMergeEvent {
        merge_id,
        entity_type,
        source_ids,
        target_id,
        reason,
        confidence,
        merged_by: merged_by.to_string(),
        merged_at: Utc::now(),
        field_resolutions: Vec::new(),
        rollback_available: true,
    }
}

/// Generate SQL statements to execute a merge in the database.
pub fn generate_merge_sql(event: &EntityMergeEvent) -> Vec<String> {
    let mut stmts = Vec::new();
    let table = match event.entity_type {
        EntityType::Company => "companies",
        EntityType::Person => "persons",
        EntityType::Site => "sites",
    };

    // Helper: escape single quotes for SQL string literals.
    let esc = |s: &str| -> String { s.replace('\'', "''") };

    // 1. Record in entity_merges table
    for source_id in &event.source_ids {
        stmts.push(format!(
            "INSERT INTO entity_merges (source_id, target_id, entity_type, merge_reason, confidence, merged_by) \
             VALUES ('{}', '{}', '{}', '{}', {}, '{}');",
            esc(source_id),
            esc(&event.target_id),
            format!("{:?}", event.entity_type).to_lowercase(),
            serde_json::to_string(&event.reason).unwrap_or_default().replace('\'', "''"),
            event.confidence,
            esc(&event.merged_by)
        ));
    }

    // 2. Redirect graph edges
    for source_id in &event.source_ids {
        stmts.push(format!(
            "UPDATE graph_edges SET source_id = '{}' WHERE source_id = '{}';",
            esc(&event.target_id), esc(source_id)
        ));
        stmts.push(format!(
            "UPDATE graph_edges SET target_id = '{}' WHERE target_id = '{}';",
            esc(&event.target_id), esc(source_id)
        ));
    }

    // 3. Redirect observations
    for source_id in &event.source_ids {
        stmts.push(format!(
            "UPDATE observations SET entity_id = '{}' WHERE entity_id = '{}';",
            esc(&event.target_id), esc(source_id)
        ));
    }

    // 4. Record in audit log
    stmts.push(format!(
        "INSERT INTO audit_log (action, entity_type, entity_id, detail) \
         VALUES ('entity_merge', '{}', '{}', '{}');",
        format!("{:?}", event.entity_type).to_lowercase(),
        esc(&event.target_id),
        serde_json::to_string(event).unwrap_or_default().replace('\'', "''")
    ));

    // 5. Soft-delete source entities (mark as merged)
    for source_id in &event.source_ids {
        stmts.push(format!(
            "UPDATE {} SET metadata = metadata || '{{\"merged_into\": \"{}\"}}' WHERE id = '{}';",
            table, esc(&event.target_id), esc(source_id)
        ));
    }

    stmts
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_merge_event_creates_id() {
        let event = build_merge_event(
            EntityType::Company,
            vec!["src1".into(), "src2".into()],
            "target1".into(),
            MergeReason::AutoResolution {
                similarity_score: 0.95,
            },
            0.95,
            "system",
        );
        assert!(event.merge_id.starts_with("merge_company_"));
        assert_eq!(event.source_ids.len(), 2);
        assert!(event.rollback_available);
    }

    #[test]
    fn generate_sql_statements() {
        let event = build_merge_event(
            EntityType::Person,
            vec!["p1".into()],
            "p2".into(),
            MergeReason::ManualMerge {
                operator_notes: "duplicate".into(),
            },
            1.0,
            "admin",
        );
        let sql = generate_merge_sql(&event);
        assert!(!sql.is_empty());
        assert!(sql.iter().any(|s| s.contains("entity_merges")));
        assert!(sql.iter().any(|s| s.contains("graph_edges")));
        assert!(sql.iter().any(|s| s.contains("audit_log")));
    }

    #[test]
    fn merge_reason_serializes() {
        let reason = MergeReason::Acquisition {
            acquiring_company: "BigCorp".into(),
        };
        let json = serde_json::to_string(&reason).unwrap();
        assert!(json.contains("BigCorp"));
    }

    #[test]
    fn entity_type_equality() {
        assert_eq!(EntityType::Company, EntityType::Company);
        assert_ne!(EntityType::Company, EntityType::Person);
    }
}
