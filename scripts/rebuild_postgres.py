#!/usr/bin/env python3
"""Rebuild postgres.rs: wire in submodules, remove duplicates, add missing types."""

import os

STORE_DIR = os.path.join(os.path.dirname(os.path.dirname(__file__)), "crates", "store", "src")
POSTGRES_RS = os.path.join(STORE_DIR, "postgres.rs")

with open(POSTGRES_RS, "r") as f:
    lines = f.readlines()

total = len(lines)
print(f"Read {total} lines from postgres.rs")

parts = []

# Part 1: Lines 1-38 (imports + free functions)
parts.extend(lines[0:38])

# Insert mod declarations for all 19 submodule files
parts.append("\n")
for m in [
    "admin", "analytics", "artifacts", "collaboration", "companies",
    "company_assets", "competitors", "graph", "history", "insights",
    "llm_governance", "logistics", "memos", "observations", "persons",
    "preferences", "recipes", "security", "warnings",
]:
    parts.append(f"mod {m};\n")
parts.append("\n")

# Part 2: Lines 39-113 (filter structs + enums)
parts.extend(lines[38:113])

# Part 3: Lines 114-196 (UserSettingsPrefs + defaults + impl Default)
parts.extend(lines[113:196])

# Part 4: RecipeQualitySummaryRow with sqlx::FromRow
parts.append("""/// Recipe quality summary - aggregated from recipe stats.
#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct RecipeQualitySummaryRow {
    pub avg_precision_pct: i64,
    pub coverage_pct: i64,
}

""")

# Part 5: AnalystNotificationRecord (replaces NotificationRow)
parts.append("""/// Analyst notification record - maps to `analyst_notifications` table.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize, serde::Deserialize)]
pub struct AnalystNotificationRecord {
    pub id: Uuid,
    pub user_id: String,
    pub category: String,
    pub title: String,
    pub body: String,
    pub entity_type: Option<String>,
    pub entity_id: Option<String>,
    pub action_url: Option<String>,
    pub is_read: bool,
    pub read_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// Legacy alias so existing web templates keep compiling.
pub type NotificationRow = AnalystNotificationRecord;

""")

# Part 6: AnnotationRecord (replaces AnnotationRow)
parts.append("""/// Annotation record - maps to `annotations` table.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize, serde::Deserialize)]
pub struct AnnotationRecord {
    pub id: Uuid,
    pub user_id: String,
    pub entity_type: String,
    pub entity_id: String,
    pub body: String,
    pub tags: Vec<String>,
    pub visibility: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Legacy alias so existing web templates keep compiling.
pub type AnnotationRow = AnnotationRecord;

""")

# Part 7: LlmGovernanceOverview with RENAMED sub-type references
parts.append("""/// LLM governance overview for admin dashboard.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct LlmGovernanceOverview {
    pub prompt_versions: Vec<PromptVersionRecord>,
    pub workflow_runs: Vec<LlmWorkflowRunRecord>,
    pub improvement_runs: Vec<LlmImprovementRunRecord>,
    pub training_datasets: Vec<LlmTrainingDatasetRecord>,
}

""")

# Part 8: PromptVersionRecord with system_prompt + metadata fields
parts.append("""/// Prompt version record - maps to `prompt_versions` table.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct PromptVersionRecord {
    pub prompt_id: String,
    pub version: String,
    pub workflow: String,
    pub system_prompt: String,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

""")

# Part 9: LlmWorkflowRunRecord (renamed, with full fields)
parts.append("""/// LLM workflow run record - maps to `llm_workflow_runs` table.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct LlmWorkflowRunRecord {
    pub id: Uuid,
    pub workflow: String,
    pub prompt_id: String,
    pub prompt_version: String,
    pub model_name: String,
    pub request_payload: serde_json::Value,
    pub response_payload: serde_json::Value,
    pub validation_issues: serde_json::Value,
    pub quality_gate_passed: bool,
    pub duration_ms: i64,
    pub created_at: DateTime<Utc>,
}

/// Legacy alias.
pub type WorkflowRunRecord = LlmWorkflowRunRecord;

""")

# Part 10: LlmImprovementRunRecord (renamed, with full fields)
parts.append("""/// LLM improvement run record - maps to `llm_improvement_runs` table.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct LlmImprovementRunRecord {
    pub id: Uuid,
    pub run_kind: String,
    pub run_key: String,
    pub metrics: serde_json::Value,
    pub artifacts: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

/// Legacy alias.
pub type ImprovementRunRecord = LlmImprovementRunRecord;

""")

# Part 11: LlmTrainingDatasetRecord (renamed)
parts.append("""/// LLM training dataset record - maps to `llm_training_datasets` table.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct LlmTrainingDatasetRecord {
    pub id: Uuid,
    pub dataset_name: String,
    pub dataset_version: String,
    pub source_run_kind: String,
    pub example_count: i64,
    pub created_at: DateTime<Utc>,
}

/// Legacy alias.
pub type TrainingDatasetRecord = LlmTrainingDatasetRecord;

""")

# Part 12: PgStore struct + impl with connect/from_pool/run_migrations ONLY
# Lines 289-308 (PgStore struct + impl start + connect + from_pool)
parts.extend(lines[288:308])

# Close the connect/from_pool section, add run_migrations
parts.append("""
    // --- Schema Migration ---

    /// Run the full schema creation. Idempotent via IF NOT EXISTS.
    pub async fn run_migrations(&self) -> Result<()> {
        sqlx::migrate!("./migrations").run(&self.pool).await?;
        Ok(())
    }
}

""")

# Part 13: Row type definitions between the old impl blocks (lines 2479-3091)
parts.extend(lines[2478:3091])

# Part 14: NEW struct definitions needed by submodules
parts.append("""
// --- Collaboration Record Types ---

/// Analyst user record - maps to `analyst_users` table.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct AnalystUserRecord {
    pub id: String,
    pub display_name: String,
    pub email: Option<String>,
    pub role: String,
    pub notification_channels: serde_json::Value,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// API key owner record.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct ApiKeyOwnerRecord {
    pub key_id: String,
    pub user_id: String,
    pub display_name: String,
    pub role: String,
    pub is_active: bool,
    pub notification_channels: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Saved search record - maps to `saved_searches` table.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct SavedSearchRecord {
    pub id: Uuid,
    pub user_id: String,
    pub name: String,
    pub query_text: String,
    pub filters: serde_json::Value,
    pub default_sort: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Watchlist record - maps to `watchlists` table.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct WatchlistRecord {
    pub id: Uuid,
    pub user_id: String,
    pub name: String,
    pub entities: serde_json::Value,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// User preferences record.
#[derive(Debug, Clone, serde::Serialize)]
pub struct UserPreferencesRecord {
    pub theme: String,
    pub locale: String,
    pub preferences: serde_json::Value,
    pub updated_at: DateTime<Utc>,
}

/// Export history record - maps to `export_history` table.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct ExportHistoryRecord {
    pub id: Uuid,
    pub user_id: String,
    pub export_type: String,
    pub format: String,
    pub filters: serde_json::Value,
    pub row_count: i64,
    pub download_name: Option<String>,
    pub requested_at: DateTime<Utc>,
}

/// Replay job record.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct ReplayJobRecord {
    pub id: Uuid,
    pub user_id: String,
    pub replay_type: String,
    pub entity_type: String,
    pub entity_id: String,
    pub status: String,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub item_count: i64,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// --- Worker State Record Types ---

/// Worker job state record - maps to `worker_job_state` table.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct WorkerJobStateRecord {
    pub job_kind: String,
    pub last_run: Option<DateTime<Utc>>,
    pub last_status: Option<String>,
    pub last_error: Option<String>,
    pub last_duration_ms: Option<i64>,
    pub consecutive_failures: i32,
    pub max_consecutive_failures: i32,
    pub circuit_open: bool,
    pub updated_at: DateTime<Utc>,
}

/// Worker job history record - maps to `worker_job_history` table.
#[derive(Debug, Clone, serde::Serialize)]
pub struct WorkerJobHistoryRecord {
    pub run_id: String,
    pub job_kind: String,
    pub status: String,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub duration_ms: Option<i64>,
    pub items_processed: i64,
    pub notes: String,
    pub created_at: DateTime<Utc>,
}

// --- Analytics Record Types ---

/// Stats alert calibration event record.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct StatsAlertCalibrationEventRecord {
    pub id: Uuid,
    pub entity_id: Uuid,
    pub feature_vector: serde_json::Value,
    pub alert_level: String,
    pub predicted_at: DateTime<Utc>,
    pub expected_by: DateTime<Utc>,
    pub actual_outcome_within_30d: Option<bool>,
    pub outcome_source: Option<String>,
    pub outcome_reference_id: Option<Uuid>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

/// Resolved stats alert calibration sample record.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct ResolvedStatsAlertCalibrationSampleRecord {
    pub id: Uuid,
    pub entity_id: Uuid,
    pub feature_vector: serde_json::Value,
    pub alert_level: String,
    pub predicted_at: DateTime<Utc>,
    pub expected_by: DateTime<Utc>,
    pub actual_outcome_within_30d: Option<bool>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub metadata: serde_json::Value,
}

/// Source reliability aggregate record (computed).
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct SourceReliabilityAggregateRecord {
    pub source_domain: String,
    pub observation_count: i64,
    pub confirmed_count: i64,
}

/// Source reliability stat record - maps to `source_reliability_stats` table.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct SourceReliabilityStatRecord {
    pub source_domain: String,
    pub tier: String,
    pub observation_count: i64,
    pub confirmed_count: i64,
    pub observed_reliability: f64,
    pub effective_reliability: f64,
    pub promotion_recommended: bool,
    pub last_refreshed_at: DateTime<Utc>,
    pub promotion_alerted_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Audit log record - maps to `audit_log` table.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct AuditLogRecord {
    pub id: Uuid,
    pub event_type: String,
    pub actor: String,
    pub detail: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

// --- POI Expansion Seed Row ---

/// Expansion seed for POI discovery.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct ExpansionSeedRow {
    pub id: Uuid,
    pub person_id: Uuid,
    pub seed_type: String,
    pub seed_value: String,
    pub source_url: Option<String>,
    pub priority: f64,
    pub status: String,
    pub created_at: DateTime<Utc>,
}

""")

# Part 15: Tests (lines 4530-4703)
parts.extend(lines[4529:])

with open(POSTGRES_RS, "w") as f:
    f.writelines(parts)

new_count = sum(1 for _ in open(POSTGRES_RS))
print(f"Wrote {new_count} lines to postgres.rs (was {total})")
