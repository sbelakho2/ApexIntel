use anyhow::Result;
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::postgres::{PgPool, PgPoolOptions};
use sqlx::Row;
use sqlx::{Postgres, QueryBuilder};
use uuid::Uuid;

use apex_core::analysis::{EvidenceQuality, HypothesisScorecard, TemporalDelta, WeakSignalCluster};
use apex_core::entities::*;
use apex_core::validation::normalize_url;

mod admin;
mod analytics;
mod artifacts;
mod collaboration;
mod companies;
mod company_assets;
mod competitors;
mod graph;
mod history;
mod insights;
mod llm_governance;
mod logistics;
mod memos;
mod observations;
mod persons;
mod preferences;
mod recipes;
mod security;
mod warnings;

/// Escape ILIKE wildcard characters (`%` and `_`) in user input,
/// then wrap with `%…%` for a contains-match pattern.
fn ilike_pattern(raw: &str) -> String {
    let escaped = raw
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    format!("%{}%", escaped)
}

const MAX_LIST_LIMIT: i64 = 500;
const LEGACY_MALFORMED_VERACITY_TITLE_MARKER: &str = "intelligence veracity:";
const LEGACY_MALFORMED_VERACITY_SUMMARY_MARKERS: [&str; 7] = [
    "additional source reporting:",
    "signal themes detected:",
    "assessment:",
    "actionable:",
    "watch closely:",
    "early signal:",
    "low confidence:",
];

fn clamp_limit(limit: i64) -> i64 {
    limit.clamp(1, MAX_LIST_LIMIT)
}

fn normalize_url_vec(urls: &[String]) -> Vec<String> {
    urls.iter().filter_map(|u| normalize_url(u)).collect()
}

fn validate_tags(tags: &[String]) -> Result<()> {
    for tag in tags {
        if tag.chars().count() > 64 {
            return Err(anyhow::anyhow!("tag too long (max 64 chars)"));
        }
    }
    Ok(())
}

fn insight_dedup_key(title: &str, insight_type: Option<&str>, region: Option<&str>) -> String {
    format!(
        "{}|{}|{}",
        title.trim().to_ascii_lowercase(),
        insight_type.unwrap_or_default().trim().to_ascii_lowercase(),
        region.unwrap_or_default().trim().to_ascii_lowercase(),
    )
}

fn normalize_story_signature(text: &str, max_tokens: usize) -> String {
    let mut normalized = String::with_capacity(text.len());
    for ch in text.chars().flat_map(|ch| ch.to_lowercase()) {
        if ch.is_ascii_alphanumeric() {
            normalized.push(ch);
        } else {
            normalized.push(' ');
        }
    }

    normalized
        .split_whitespace()
        .take(max_tokens)
        .collect::<Vec<_>>()
        .join(" ")
}

fn recent_story_dedup_signature(
    summary: &str,
    insight_type: Option<&str>,
    entity_ids: Option<&[Uuid]>,
) -> Option<String> {
    if entity_ids.is_none_or(|ids| ids.is_empty()) {
        return None;
    }

    let insight_type = insight_type.unwrap_or_default().trim().to_ascii_lowercase();
    if insight_type.starts_with("llm_") || insight_type == "veracity_analysis" {
        return None;
    }

    let signature = normalize_story_signature(summary, 18);
    if signature.split_whitespace().count() < 10 {
        return None;
    }

    Some(signature)
}

fn has_legacy_malformed_veracity_text(title: &str, summary: &str) -> bool {
    let lower_title = title.trim().to_ascii_lowercase();
    let lower_summary = summary.trim().to_ascii_lowercase();

    lower_title.contains(LEGACY_MALFORMED_VERACITY_TITLE_MARKER)
        || LEGACY_MALFORMED_VERACITY_SUMMARY_MARKERS
            .iter()
            .any(|marker| lower_summary.contains(marker))
}

fn append_legacy_malformed_veracity_sql_clause(qb: &mut QueryBuilder<Postgres>, col_prefix: &str) {
    qb.push("NOT (lower(coalesce(")
        .push(col_prefix)
        .push("insight_type, '')) = 'veracity_analysis' AND (")
        .push("lower(")
        .push(col_prefix)
        .push("title) LIKE '%")
        .push(LEGACY_MALFORMED_VERACITY_TITLE_MARKER)
        .push("%'");

    for marker in LEGACY_MALFORMED_VERACITY_SUMMARY_MARKERS {
        qb.push(" OR lower(")
            .push(col_prefix)
            .push("summary) LIKE '%")
            .push(marker)
            .push("%'");
    }

    qb.push("))");
}

fn is_legacy_malformed_veracity_row(row: &InsightRow) -> bool {
    let insight_type = row
        .insight_type
        .as_deref()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if insight_type != "veracity_analysis" {
        return false;
    }

    has_legacy_malformed_veracity_text(&row.title, &row.summary)
}

fn filter_visible_insights(rows: Vec<InsightRow>) -> Vec<InsightRow> {
    rows.into_iter()
        .filter(|row| !is_legacy_malformed_veracity_row(row))
        .collect()
}

fn normalize_optional_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string())
}

#[derive(Debug, Clone, Default)]
pub struct WarningListFilters {
    pub regions: Vec<String>,
    pub severities: Vec<String>,
    pub warning_types: Vec<String>,
    pub acknowledged: Option<bool>,
    pub date_from: Option<DateTime<Utc>>,
    pub date_to: Option<DateTime<Utc>>,
    pub search: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct InsightListFilters {
    pub regions: Vec<String>,
    pub date_from: Option<DateTime<Utc>>,
    pub date_to: Option<DateTime<Utc>>,
    pub search: Option<String>,
    pub insight_types: Vec<String>,
    /// Exclude internal telemetry insight types (e.g. `llm_*`) from results.
    pub exclude_internal: bool,
    /// When Some(user_id), only return bookmarked insights for this user.
    pub bookmarked_by: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct CompanyListFilters {
    pub regions: Vec<String>,
    pub search: Option<String>,
    pub is_competitor: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompanyOrderBy {
    Name,
    Region,
    ThreatScore,
    UpdatedAt,
}

#[derive(Debug, Clone, Default)]
pub struct PersonListFilters {
    pub regions: Vec<String>,
    pub roles: Vec<String>,
    pub search: Option<String>,
    pub min_priority: Option<f64>,
    pub max_priority: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PersonOrderBy {
    Name,
    Priority,
    Region,
    UpdatedAt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarningOrderBy {
    CreatedAt,
    WarningType,
    Severity,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UserSettingsPrefs {
    pub api_key_display: String,
    pub backend_url_display: String,
    pub session_timeout_hours: i64,
    pub default_region: String,
    pub auto_include_neighbors: bool,
    pub daily_crawl_enabled: bool,
    pub crawl_window: String,
    pub slack_enabled: bool,
    pub minimum_severity: String,
    pub auto_cleanup_enabled: bool,
    pub export_format: String,
    pub retention_period: String,
    pub email_digest_enabled: bool,
    pub notification_frequency: String,
    pub critical_only_enabled: bool,
    #[serde(default)]
    pub email_digest_recipients: String,
    #[serde(default)]
    pub email_digest_categories: Vec<String>,
    #[serde(default = "default_email_digest_time_cet")]
    pub email_digest_time_cet: String,
    #[serde(default = "default_email_digest_weekday")]
    pub email_digest_weekday: String,
    #[serde(default)]
    pub email_digest_last_sent_at: Option<String>,
}

fn default_email_digest_time_cet() -> String {
    "08:00".to_string()
}

fn default_email_digest_weekday() -> String {
    "Mon".to_string()
}

#[derive(Debug, Clone)]
pub struct UserPreferencesRecord {
    pub theme: String,
    pub locale: String,
    pub preferences: Value,
    pub updated_at: DateTime<Utc>,
}

impl Default for UserSettingsPrefs {
    fn default() -> Self {
        Self {
            api_key_display: "(not configured)".to_string(),
            backend_url_display: "direct Axum service".to_string(),
            session_timeout_hours: 24,
            default_region: "Global".to_string(),
            auto_include_neighbors: false,
            daily_crawl_enabled: true,
            crawl_window: "01:00-05:00 UTC".to_string(),
            slack_enabled: false,
            minimum_severity: "high".to_string(),
            auto_cleanup_enabled: false,
            export_format: "JSON".to_string(),
            retention_period: "90 days".to_string(),
            email_digest_enabled: false,
            notification_frequency: "Daily".to_string(),
            critical_only_enabled: false,
            email_digest_recipients: String::new(),
            email_digest_categories: Vec::new(),
            email_digest_time_cet: default_email_digest_time_cet(),
            email_digest_weekday: default_email_digest_weekday(),
            email_digest_last_sent_at: None,
        }
    }
}

pub struct PgStore {
    pub pool: PgPool,
}

impl PgStore {
    pub async fn connect(database_url: &str) -> Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(20)
            .acquire_timeout(std::time::Duration::from_secs(5))
            .idle_timeout(std::time::Duration::from_secs(600))
            .max_lifetime(std::time::Duration::from_secs(1800))
            .connect(database_url)
            .await?;
        Ok(Self { pool })
    }

    pub fn from_pool(pool: PgPool) -> Self {
        Self { pool }
    }

    // Domain methods now live in dedicated postgres submodules.

    // ─── Schema Migration ────────────────────────────────────────────────

    /// Run the full schema creation. Idempotent via IF NOT EXISTS.
    pub async fn run_migrations(&self) -> Result<()> {
        // Use versioned migrations from ./migrations so schema changes are tracked over time.
        sqlx::migrate!("./migrations").run(&self.pool).await?;
        Ok(())
    }
}

// ─── Row Types (sqlx::FromRow) ──────────────────────────────────────────────

// ─── Worker Pipeline Stats Types ─────────────────────────────────────────────

/// Stats returned by get_crawl_stats for worker pipeline integration.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct CrawlStats {
    pub sources_attempted: u64,
    pub sources_succeeded: u64,
    pub sources_failed: u64,
    pub new_observations: u64,
    pub changed_pages: u64,
    pub bytes_fetched: u64,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct CrawlStatsRow {
    sources_attempted: Option<i64>,
    sources_succeeded: Option<i64>,
    sources_failed: Option<i64>,
    new_observations: Option<i64>,
    changed_pages: Option<i64>,
    bytes_fetched: Option<i64>,
}

/// Stats for the mining pipeline stage.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct MiningStats {
    pub candidates_found: u64,
    pub candidates_passed_gates: u64,
    pub hypotheses_generated: u64,
    pub recipes_staged: u64,
    pub errors: Vec<String>,
}

/// Stats for the POI refresh pipeline stage.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct PoiStats {
    pub profiles_scanned: u64,
    pub profiles_updated: u64,
    pub new_pois_discovered: u64,
    pub role_changes_detected: u64,
    pub errors: Vec<String>,
}

/// Stats for the drift check pipeline stage.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct DriftStats {
    pub features_checked: u64,
    pub features_drifted: u64,
    pub drift_scores: Vec<(String, f64)>,
    pub alerts_raised: u64,
    pub errors: Vec<String>,
}

/// Row type for staged recipe queries.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct StagedRecipeRow {
    pub recipe_code: String,
    pub name: String,
    pub precision_observed: f64,
    pub recall_observed: f64,
    pub false_positive_rate: f64,
    pub sample_size: i32,
    pub days_in_staging: i32,
    pub created_at: DateTime<Utc>,
}

/// Row type for production recipe queries.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct ProductionRecipeRow {
    pub recipe_code: String,
    pub name: String,
    pub precision_history: Vec<f64>,
    pub false_positive_rate: f64,
    pub fpr_baseline: f64,
    pub warnings_generated_last_week: i64,
    pub last_triggered_at: Option<DateTime<Utc>>,
    pub days_inactive: i32,
    pub created_at: DateTime<Utc>,
}

/// Weekly summary statistics for memo generation.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct WeeklySummaryStats {
    pub companies_monitored: u64,
    pub persons_tracked: u64,
    pub warnings_generated: u64,
    pub insights_produced: u64,
    pub recipes_in_production: u64,
    pub top_regions: Vec<String>,
    pub notable_events: Vec<String>,
}

// ─── Weekly Memo Types ───────────────────────────────────────────────────────

/// A section within a weekly memo.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WeeklyMemoSection {
    pub title: String,
    pub content: String,
    pub priority: String,
    pub related_warnings: Vec<String>,
    pub related_insights: Vec<String>,
}

/// Key metrics for a weekly memo.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WeeklyMemoKeyMetrics {
    pub warnings_total: i64,
    pub warnings_critical: i64,
    pub insights_generated: i64,
    pub companies_monitored: i64,
    pub pois_tracked: i64,
}

/// Action item in a weekly memo.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WeeklyMemoActionItem {
    pub text: String,
    pub priority: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
}

/// Full weekly memo structure matching frontend expectations.
#[derive(Debug, Clone, serde::Serialize)]
pub struct WeeklyMemo {
    pub id: Uuid,
    pub title: String,
    pub week_start: String,
    pub week_end: String,
    pub executive_summary: String,
    pub sections: Vec<WeeklyMemoSection>,
    pub key_metrics: WeeklyMemoKeyMetrics,
    pub action_items: Vec<WeeklyMemoActionItem>,
    pub generated_at: String,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct AnalystUserRecord {
    pub id: String,
    pub display_name: String,
    pub email: Option<String>,
    pub role: String,
    pub notification_channels: Value,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct ApiKeyOwnerRecord {
    pub key_id: String,
    pub user_id: String,
    pub display_name: String,
    pub role: String,
    pub is_active: bool,
    pub notification_channels: Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct SavedSearchRecord {
    pub id: Uuid,
    pub user_id: String,
    pub name: String,
    pub query_text: String,
    pub filters: Value,
    pub default_sort: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct WatchlistRecord {
    pub id: Uuid,
    pub user_id: String,
    pub name: String,
    pub entities: Value,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
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

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
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

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct ExportHistoryRecord {
    pub id: Uuid,
    pub user_id: String,
    pub export_type: String,
    pub format: String,
    pub filters: Value,
    pub row_count: i64,
    pub download_name: Option<String>,
    pub requested_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct AuditLogRecord {
    pub id: Uuid,
    pub event_type: String,
    pub actor: String,
    pub detail: Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct PromptVersionRecord {
    pub prompt_id: String,
    pub version: String,
    pub workflow: String,
    pub system_prompt: String,
    pub metadata: Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct LlmWorkflowRunRecord {
    pub id: Uuid,
    pub workflow: String,
    pub prompt_id: String,
    pub prompt_version: String,
    pub model_name: String,
    pub request_payload: Value,
    pub response_payload: Value,
    pub validation_issues: Value,
    pub quality_gate_passed: bool,
    pub duration_ms: i64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct LlmImprovementRunRecord {
    pub id: Uuid,
    pub run_kind: String,
    pub run_key: String,
    pub metrics: Value,
    pub artifacts: Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct LlmTrainingDatasetRecord {
    pub id: Uuid,
    pub dataset_name: String,
    pub dataset_version: String,
    pub source_run_kind: String,
    pub source_run_key: String,
    pub manifest: Value,
    pub example_count: i64,
    pub examples_jsonl: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct ReplayJobRecord {
    pub id: Uuid,
    pub requested_by: String,
    pub status: String,
    pub request: Value,
    pub total_observations: i64,
    pub processed: i64,
    pub warnings_generated: i64,
    pub errors: i64,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize, serde::Deserialize)]
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

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize, serde::Deserialize)]
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

// ─── Competitor Change Types ─────────────────────────────────────────────────

/// A change event for a competitor.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CompetitorChange {
    pub id: Uuid,
    pub competitor_id: Uuid,
    pub competitor_name: String,
    pub change_type: String,
    pub title: String,
    pub description: String,
    pub detected_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    pub impact_score: f64,
}

/// Competitor with enhanced tracking fields.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CompetitorItem {
    pub id: Uuid,
    pub name: String,
    pub region: String,
    pub entity_type: String,
    pub threat_score: Option<f64>,
    pub capability_overlap: f64,
    pub market_overlap: f64,
    pub last_change_at: Option<String>,
    pub change_count_30d: i64,
    pub capabilities: Vec<String>,
    pub primary_markets: Vec<String>,
}

// ─── Entity Row Types ────────────────────────────────────────────────────────

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct CompanyRow {
    pub id: Uuid,
    pub name: String,
    pub legal_name: Option<String>,
    pub domain: Option<String>,
    pub country_code: Option<String>,
    pub region: Option<String>,
    pub company_type: Option<String>,
    pub industry_tags: Option<Vec<String>>,
    pub employee_estimate: Option<i32>,
    pub revenue_estimate_usd: Option<i64>,
    pub risk_score: Option<f64>,
    pub threat_score: Option<f64>,
    pub overlap_score: Option<f64>,
    pub strategic_relevance: Option<f64>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct SiteRow {
    pub id: Uuid,
    pub company_id: Option<Uuid>,
    pub name: String,
    pub address: Option<String>,
    pub city: Option<String>,
    pub country_code: Option<String>,
    pub region: Option<String>,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub site_type: Option<String>,
    pub capabilities: Option<Vec<String>>,
    pub certifications: Option<Vec<String>>,
    pub employee_estimate: Option<i32>,
    pub free_zone: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct PersonRow {
    pub id: Uuid,
    pub name: String,
    pub name_ar: Option<String>,
    pub name_fr: Option<String>,
    pub primary_org_id: Option<Uuid>,
    pub current_role: Option<String>,
    pub role_family: Option<String>,
    pub region: Option<String>,
    pub country_code: Option<String>,
    pub public_bio: Option<String>,
    pub public_email: Option<String>,
    pub priority_vector: Option<serde_json::Value>,
    pub influence_score: Option<f64>,
    pub trigger_topics: Option<Vec<String>>,
    pub decision_style: Option<String>,
    pub risk_tolerance: Option<String>,
    pub change_appetite: Option<String>,
    pub communication_style: Option<String>,
    pub decision_mode: Option<String>,
    pub preferred_proof_type: Option<String>,
    pub pain_index: Option<f64>,
    pub change_risk: Option<f64>,
    pub role_drift_score: Option<f64>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct PersonListRow {
    pub id: Uuid,
    pub name: String,
    pub role: String,
    pub organization: String,
    pub region: String,
    pub country_code: String,
    pub role_family: String,
    pub priority_score: f64,
    pub engagement_status: String,
    pub updated_at: DateTime<Utc>,
    pub artifact_count: i64,
}

/// Minimal row used by the POI expansion engine as a seed.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct ExpansionSeedRow {
    pub id: Uuid,
    pub name: String,
    pub current_role: String,
    pub role_family: String,
    pub region: String,
    pub country_code: String,
    pub primary_org_id: Option<Uuid>,
    pub org_name: String,
    pub org_domain: Option<String>,
    pub is_competitor: bool,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct WarningRow {
    pub id: Uuid,
    pub recipe_code: Option<String>,
    pub warning_type: String,
    pub title: String,
    pub description: Option<String>,
    pub severity: String,
    pub region: Option<String>,
    pub source_urls: Option<Vec<String>>,
    pub entity_ids: Option<Vec<Uuid>>,
    pub confidence: Option<f64>,
    pub ts_utc: DateTime<Utc>,
    pub acknowledged: bool,
    pub acknowledged_by: Option<String>,
    pub acknowledged_at: Option<DateTime<Utc>>,
    pub acknowledged_note: Option<String>,
    pub review_outcome: Option<String>,
    pub reviewed_by: Option<String>,
    pub reviewed_at: Option<DateTime<Utc>>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct InsightRow {
    pub id: Uuid,
    pub title: String,
    pub summary: String,
    pub insight_type: Option<String>,
    pub region: Option<String>,
    pub confidence: Option<f64>,
    pub evidence_urls: Option<Vec<String>>,
    pub entity_ids: Option<Vec<Uuid>>,
    pub tags: Option<Vec<String>>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct ObservationRow {
    pub id: Uuid,
    pub observation_type: String,
    pub entity_id: Option<Uuid>,
    pub entity_type: Option<String>,
    pub ts_utc: DateTime<Utc>,
    pub value: serde_json::Value,
    pub provenance: serde_json::Value,
    pub confidence: Option<f64>,
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct EdgeRow {
    pub id: Uuid,
    pub source_id: Uuid,
    pub source_type: String,
    pub target_id: Uuid,
    pub target_type: String,
    pub edge_type: String,
    pub weight: Option<f64>,
    pub confidence: Option<f64>,
    pub evidence_ids: Option<Vec<Uuid>>,
    pub metadata: Option<serde_json::Value>,
    pub first_seen: Option<DateTime<Utc>>,
    pub last_seen: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct RecipeStatRow {
    pub recipe_code: String,
    pub status: String,
    pub precision_score: f64,
    pub false_positive_rate: f64,
    pub fired_count: i64,
    pub last_fired: Option<DateTime<Utc>>,
    pub first_fired: Option<DateTime<Utc>>,
    pub active_count: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WarningReviewOutcome {
    TruePositive,
    FalsePositive,
}

impl WarningReviewOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TruePositive => "true_positive",
            Self::FalsePositive => "false_positive",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcknowledgeWarningResult {
    Acknowledged,
    ReviewedExisting,
    AlreadyAcknowledged,
    NotFound,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct RecipeQualitySummaryRow {
    pub avg_precision_pct: i64,
    pub coverage_pct: i64,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct CertificationRow {
    pub id: Uuid,
    pub company_id: Option<Uuid>,
    pub site_id: Option<Uuid>,
    pub standard: String,
    pub status: Option<String>,
    pub issuing_body: Option<String>,
    pub valid_from: Option<NaiveDate>,
    pub valid_until: Option<NaiveDate>,
    pub scope: Option<String>,
    pub evidence_url: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct ArtifactRow {
    pub id: Uuid,
    pub person_id: Option<Uuid>,
    pub artifact_type: String,
    pub title: Option<String>,
    pub content_summary: Option<String>,
    pub url: String,
    pub source_domain: Option<String>,
    pub language: Option<String>,
    pub topics: Option<Vec<String>>,
    pub sentiment_score: Option<f64>,
    pub key_phrases: Option<Vec<String>>,
    pub ts_utc: DateTime<Utc>,
    pub provenance: serde_json::Value,
    pub metadata: Option<serde_json::Value>,
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct CapabilityRow {
    pub id: Uuid,
    pub company_id: Option<Uuid>,
    pub site_id: Option<Uuid>,
    pub capability: String,
    pub proof_grade: Option<String>,
    pub evidence_urls: Option<Vec<String>>,
    pub first_seen: Option<DateTime<Utc>>,
    pub last_confirmed: Option<DateTime<Utc>>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct ProductFamilyRow {
    pub id: Uuid,
    pub company_id: Option<Uuid>,
    pub name: String,
    pub hs_codes: Option<Vec<String>>,
    pub tech_tags: Option<Vec<String>>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct LogisticsNodeRow {
    pub id: Uuid,
    pub name: String,
    pub node_type: Option<String>,
    pub country_code: Option<String>,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct RegulationRow {
    pub id: Uuid,
    pub name: String,
    pub regulation_type: Option<String>,
    pub jurisdiction: Option<String>,
    pub effective_date: Option<NaiveDate>,
    pub summary: Option<String>,
    pub source_url: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default, serde::Serialize, sqlx::FromRow)]
pub struct RegionCount {
    pub region: String,
    pub count: i64,
}

#[derive(Debug, Clone, Default, serde::Serialize, sqlx::FromRow)]
pub struct SeverityCount {
    pub severity: String,
    pub count: i64,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ActivityItem {
    pub label: String,
    pub count: u64,
    pub delta: i64,
}

/// Dashboard overview statistics.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct DashboardStats {
    pub total_companies: u64,
    pub total_persons: u64,
    pub total_warnings: u64,
    pub unacknowledged_warnings: u64,
    pub total_insights: u64,
    pub active_recipes: u64,
    pub new_insights_24h: u64,
    pub new_warnings_24h: u64,
    pub top_regions: Vec<RegionCount>,
    pub threat_distribution: Vec<SeverityCount>,
    pub recent_activity: Vec<ActivityItem>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct PageFingerprintRow {
    pub id: Uuid,
    pub url: String,
    pub content_hash: String,
    pub ts: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct FeatureRowRow {
    pub entity_id: String,
    pub entity_type: String,
    pub time_bucket: i64,
    pub bucket_size_days: i32,
    pub data: serde_json::Value,
    pub created_at: Option<DateTime<Utc>>,
}

/// Admin overview of crawl pipeline health.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AdminCrawlStatus {
    pub total_fingerprints: i64,
    pub domains_tracked: i64,
    pub latest_crawl_ts: Option<DateTime<Utc>>,
    pub crawl_stats: CrawlStats,
}

/// Admin overview of recipe performance.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AdminRecipePerformance {
    pub total_recipes: i64,
    pub production_count: i64,
    pub staging_count: i64,
    pub deprecated_count: i64,
    pub recipes: Vec<RecipeStatRow>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AdminLlmGovernanceOverview {
    pub prompt_versions: Vec<PromptVersionRecord>,
    pub workflow_runs: Vec<LlmWorkflowRunRecord>,
    pub improvement_runs: Vec<LlmImprovementRunRecord>,
    pub training_datasets: Vec<LlmTrainingDatasetRecord>,
}

// ─── New Row Types: Role History, Dossier Entries, Changes ───────────────────

/// A role history entry — one position a POI has held.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct RoleHistoryRow {
    pub id: Uuid,
    pub person_id: Uuid,
    pub org_id: Option<Uuid>,
    pub org_name: String,
    pub title: String,
    pub role_family: Option<String>,
    pub start_date: Option<DateTime<Utc>>,
    pub end_date: Option<DateTime<Utc>>,
    pub source_url: Option<String>,
    pub confidence: Option<f64>,
    pub verified: Option<bool>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

/// A persistent dossier entry — a verified fact attached to a person or company.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct DossierEntryRow {
    pub id: Uuid,
    pub entity_type: String,
    pub entity_id: Uuid,
    pub category: String,
    pub title: String,
    pub content: String,
    pub source_urls: Option<Vec<String>>,
    pub confidence: Option<f64>,
    pub verified: Option<bool>,
    pub supersedes_id: Option<Uuid>,
    pub valid_from: Option<DateTime<Utc>>,
    pub valid_until: Option<DateTime<Utc>>,
    pub author: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: Option<DateTime<Utc>>,
}

/// A changelog entry for a company.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct CompanyChangeRow {
    pub id: Uuid,
    pub company_id: Uuid,
    pub change_type: String,
    pub field_name: Option<String>,
    pub old_value: Option<String>,
    pub new_value: Option<String>,
    pub description: Option<String>,
    pub source_url: Option<String>,
    pub detected_at: Option<DateTime<Utc>>,
    pub confidence: Option<f64>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: Option<DateTime<Utc>>,
}

/// A changelog entry for a person/POI.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct PersonChangeRow {
    pub id: Uuid,
    pub person_id: Uuid,
    pub change_type: String,
    pub field_name: Option<String>,
    pub old_value: Option<String>,
    pub new_value: Option<String>,
    pub description: Option<String>,
    pub source_url: Option<String>,
    pub detected_at: Option<DateTime<Utc>>,
    pub confidence: Option<f64>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: Option<DateTime<Utc>>,
}

/// Admin overview of POI coverage.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AdminPoiCoverage {
    pub total_persons: i64,
    pub with_artifacts: i64,
    pub avg_artifacts_per_person: f64,
    pub total_artifacts: i64,
    pub poi_stats: PoiStats,
}

/// Company dossier: company + sites + capabilities + certifications + edges + dossier_entries + changes
#[derive(Debug, Clone, serde::Serialize)]
pub struct CompanyDossier {
    pub company: CompanyRow,
    pub sites: Vec<SiteRow>,
    pub capabilities: Vec<CapabilityRow>,
    pub certifications: Vec<CertificationRow>,
    pub product_families: Vec<ProductFamilyRow>,
    pub edges: Vec<EdgeRow>,
    pub dossier_entries: Vec<DossierEntryRow>,
    pub recent_changes: Vec<CompanyChangeRow>,
    pub analysis: DossierAnalysis,
}

/// Person dossier: person + artifacts + observations + edges + role_history + dossier_entries + changes
#[derive(Debug, Clone, serde::Serialize)]
pub struct PersonDossier {
    pub person: PersonRow,
    pub artifacts: Vec<ArtifactRow>,
    pub observations: Vec<ObservationRow>,
    pub edges: Vec<EdgeRow>,
    pub role_history: Vec<RoleHistoryRow>,
    pub dossier_entries: Vec<DossierEntryRow>,
    pub recent_changes: Vec<PersonChangeRow>,
    pub analysis: DossierAnalysis,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DossierAnalysis {
    pub evidence_quality: EvidenceQuality,
    pub temporal_delta: TemporalDelta,
    pub correlated_signals: Vec<WeakSignalCluster>,
    pub competing_hypotheses: Vec<HypothesisScorecard>,
    pub summary: String,
}

/// Engagement guide for a POI
#[derive(Debug, Clone, serde::Serialize)]
pub struct PersonEngagement {
    pub person_id: Uuid,
    pub name: String,
    pub role: Option<String>,
    pub priority_score: f64,
    pub engagement_status: String,
    pub co_appearances: Vec<EdgeRow>,
    pub recent_observations: Vec<ObservationRow>,
}

impl PgStore {}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::Execute;

    /// These tests verify query construction and row type structure.
    /// Full integration tests require a running PostgreSQL instance.

    #[test]
    fn test_company_row_fields() {
        // Verify CompanyRow has all expected fields via construction
        let row = CompanyRow {
            id: Uuid::new_v4(),
            name: "Starz Electronics".into(),
            legal_name: Some("Starz Electronics SARL".into()),
            domain: Some("starz-electronics.com".into()),
            country_code: Some("TN".into()),
            region: Some("TN".into()),
            company_type: Some("ems".into()),
            industry_tags: Some(vec!["automotive".into(), "industrial".into()]),
            employee_estimate: Some(500),
            revenue_estimate_usd: Some(50_000_000),
            risk_score: Some(0.3),
            threat_score: Some(0.1),
            overlap_score: Some(0.8),
            strategic_relevance: Some(0.9),
            metadata: Some(serde_json::json!({})),
            created_at: Some(Utc::now()),
            updated_at: Some(Utc::now()),
        };
        assert_eq!(row.name, "Starz Electronics");
        assert_eq!(row.country_code.as_deref(), Some("TN"));

        // Verify serialization
        let json = serde_json::to_value(&row).unwrap();
        assert_eq!(json["name"], "Starz Electronics");
        assert!(json["industry_tags"].is_array());
    }

    #[test]
    fn test_observation_row_fields() {
        let row = ObservationRow {
            id: Uuid::new_v4(),
            observation_type: "JobPost".into(),
            entity_id: Some(Uuid::new_v4()),
            entity_type: Some("company".into()),
            ts_utc: Utc::now(),
            value: serde_json::json!({"role": "SQE", "role_family": "quality"}),
            provenance: serde_json::json!({"url": "https://jobs.example.com"}),
            confidence: Some(0.95),
            created_at: Some(Utc::now()),
        };
        assert_eq!(row.observation_type, "JobPost");
        assert_eq!(row.value["role"], "SQE");
    }

    #[test]
    fn test_edge_row_fields() {
        let row = EdgeRow {
            id: Uuid::new_v4(),
            source_id: Uuid::new_v4(),
            source_type: "company".into(),
            target_id: Uuid::new_v4(),
            target_type: "company".into(),
            edge_type: "competitor".into(),
            weight: Some(0.8),
            confidence: Some(0.9),
            evidence_ids: Some(vec![Uuid::new_v4()]),
            metadata: Some(serde_json::json!({})),
            first_seen: Some(Utc::now()),
            last_seen: Some(Utc::now()),
        };
        assert_eq!(row.edge_type, "competitor");
        assert!(row.weight.unwrap() > 0.5);
    }

    #[test]
    fn test_person_row_fields() {
        let row = PersonRow {
            id: Uuid::new_v4(),
            name: "Ahmed Ben Ali".into(),
            name_ar: Some("أحمد بن علي".into()),
            name_fr: Some("Ahmed Ben Ali".into()),
            primary_org_id: Some(Uuid::new_v4()),
            current_role: Some("VP Procurement".into()),
            role_family: Some("procurement".into()),
            region: Some("TN".into()),
            country_code: Some("TN".into()),
            priority_vector: Some(serde_json::json!({"cost":0.8,"quality":0.6})),
            influence_score: Some(0.7),
            metadata: Some(serde_json::json!({})),
            created_at: Some(Utc::now()),
            updated_at: Some(Utc::now()),
            public_bio: None,
            public_email: None,
            trigger_topics: None,
            decision_style: None,
            risk_tolerance: None,
            change_appetite: None,
            communication_style: None,
            decision_mode: None,
            preferred_proof_type: None,
            pain_index: None,
            change_risk: None,
            role_drift_score: None,
        };
        assert_eq!(row.name, "Ahmed Ben Ali");
        assert_eq!(row.role_family.as_deref(), Some("procurement"));
    }

    #[test]
    fn test_certification_row_fields() {
        let row = CertificationRow {
            id: Uuid::new_v4(),
            company_id: Some(Uuid::new_v4()),
            site_id: None,
            standard: "IATF_16949".into(),
            status: Some("active".into()),
            issuing_body: Some("TUV".into()),
            valid_from: Some(NaiveDate::from_ymd_opt(2023, 1, 1).unwrap()),
            valid_until: Some(NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()),
            scope: Some("Automotive EMS".into()),
            evidence_url: Some("https://example.com/cert".into()),
            metadata: Some(serde_json::json!({})),
            created_at: Some(Utc::now()),
            updated_at: Some(Utc::now()),
        };
        assert_eq!(row.standard, "IATF_16949");
        assert!(row.valid_until.unwrap() > row.valid_from.unwrap());
    }

    #[test]
    fn test_malformed_veracity_filter_only_hides_legacy_template_rows() {
        let malformed_veracity = InsightRow {
            id: Uuid::new_v4(),
            title: "Intel suppliers: Intelligence Veracity: early signal".into(),
            summary: "Additional source reporting: forum reposts only. Assessment: low confidence."
                .into(),
            insight_type: Some("veracity_analysis".into()),
            region: Some("global".into()),
            confidence: Some(0.22),
            evidence_urls: Some(vec!["https://example.com/post".into()]),
            entity_ids: Some(vec![Uuid::new_v4()]),
            tags: Some(vec!["veracity".into()]),
            created_at: Some(Utc::now()),
            updated_at: Some(Utc::now()),
        };
        let valid_veracity = InsightRow {
            id: Uuid::new_v4(),
            title: "European Commission: likely market activity development".into(),
            summary: "A Commission-linked procurement notice and follow-on trade reporting point to a likely sourcing move in the next quarter.".into(),
            insight_type: Some("veracity_analysis".into()),
            region: Some("eu".into()),
            confidence: Some(0.73),
            evidence_urls: Some(vec!["https://example.com/notice".into()]),
            entity_ids: Some(vec![Uuid::new_v4()]),
            tags: Some(vec!["veracity".into()]),
            created_at: Some(Utc::now()),
            updated_at: Some(Utc::now()),
        };
        let non_veracity = InsightRow {
            id: Uuid::new_v4(),
            title: "Intel suppliers: Intelligence Veracity: appears in quoted article title".into(),
            summary: "Assessment wording inside other insight types should not be filtered here."
                .into(),
            insight_type: Some("supply_chain_event".into()),
            region: Some("global".into()),
            confidence: Some(0.61),
            evidence_urls: Some(vec!["https://example.com/article".into()]),
            entity_ids: Some(vec![Uuid::new_v4()]),
            tags: Some(vec!["supply-chain".into()]),
            created_at: Some(Utc::now()),
            updated_at: Some(Utc::now()),
        };

        assert!(is_legacy_malformed_veracity_row(&malformed_veracity));
        assert!(!is_legacy_malformed_veracity_row(&valid_veracity));
        assert!(!is_legacy_malformed_veracity_row(&non_veracity));

        let visible = filter_visible_insights(vec![
            malformed_veracity,
            valid_veracity.clone(),
            non_veracity.clone(),
        ]);

        assert_eq!(visible.len(), 2);
        assert!(visible.iter().any(|row| row.id == valid_veracity.id));
        assert!(visible.iter().any(|row| row.id == non_veracity.id));
    }

    #[test]
    fn story_signature_normalization_collapses_punctuation_variants() {
        let left = normalize_story_signature(
            "BAE Systems' Welsh munitions factory remains delayed as of March 2026.",
            18,
        );
        let right = normalize_story_signature(
            "BAE Systems Welsh munitions factory remains delayed, as of March 2026!",
            18,
        );

        assert_eq!(left, right);
    }

    #[test]
    fn recent_story_dedup_signature_requires_entity_ids_and_skips_internal_types() {
        let entity_ids = vec![Uuid::new_v4()];
        assert!(recent_story_dedup_signature(
            "BAE Systems Welsh munitions factory remains delayed as of March 2026 and creates a qualification window for alternative manufacturing partners.",
            Some("demand_procurement"),
            Some(&entity_ids),
        )
        .is_some());

        assert!(recent_story_dedup_signature(
            "BAE Systems Welsh munitions factory remains delayed as of March 2026 and creates a qualification window for alternative manufacturing partners.",
            Some("llm_eval_report"),
            Some(&entity_ids),
        )
        .is_none());

        assert!(recent_story_dedup_signature(
            "BAE Systems Welsh munitions factory remains delayed as of March 2026 and creates a qualification window for alternative manufacturing partners.",
            Some("demand_procurement"),
            None,
        )
        .is_none());
    }

    #[test]
    fn insert_insight_query_includes_recent_story_signature_dedup() {
        let source = include_str!("postgres.rs");

        assert!(source.contains("i.created_at > NOW() - INTERVAL '14 days'"));
        assert!(source.contains("trim(regexp_replace(regexp_replace(lower(coalesce(i.summary, '')), '[^a-z0-9]+', ' ', 'g'), '\\s+', ' ', 'g')) = $10"));
    }

    #[test]
    fn test_malformed_veracity_sql_clause_covers_prefixed_and_unprefixed_count_paths() {
        let mut unprefixed = QueryBuilder::<Postgres>::new("SELECT 1 WHERE ");
        append_legacy_malformed_veracity_sql_clause(&mut unprefixed, "");
        let unprefixed_sql = unprefixed.build().sql().to_string();

        assert!(unprefixed_sql.contains("lower(coalesce(insight_type, '')) = 'veracity_analysis'"));
        assert!(unprefixed_sql.contains("lower(title) LIKE '%intelligence veracity:%'"));
        assert!(unprefixed_sql.contains("lower(summary) LIKE '%low confidence:%'"));

        let mut prefixed = QueryBuilder::<Postgres>::new("SELECT 1 WHERE ");
        append_legacy_malformed_veracity_sql_clause(&mut prefixed, "i.");
        let prefixed_sql = prefixed.build().sql().to_string();

        assert!(prefixed_sql.contains("lower(coalesce(i.insight_type, '')) = 'veracity_analysis'"));
        assert!(prefixed_sql.contains("lower(i.title) LIKE '%intelligence veracity:%'"));
        assert!(prefixed_sql.contains("lower(i.summary) LIKE '%assessment:%'"));
        assert!(prefixed_sql.contains("lower(i.summary) LIKE '%low confidence:%'"));
    }

    #[test]
    fn test_recipe_weekly_queries_use_snapshot_history_and_review_outcomes() {
        let source = format!(
            "{}\n{}",
            include_str!("postgres.rs"),
            include_str!("postgres/recipes.rs")
        );

        assert!(source
            .contains("ARRAY_AGG(precision_score ORDER BY week_start ASC) AS precision_history"));
        assert!(source.contains("w.review_outcome IN ('true_positive', 'false_positive')"));
        assert!(source.contains("INSERT INTO recipe_weekly_metrics"));
    }

    #[test]
    fn test_artifact_row_fields() {
        let row = ArtifactRow {
            id: Uuid::new_v4(),
            person_id: Some(Uuid::new_v4()),
            artifact_type: "press_quote".into(),
            title: Some("Industry 4.0 Panel".into()),
            content_summary: Some("Discussed manufacturing automation".into()),
            url: "https://example.com/article".into(),
            source_domain: Some("example.com".into()),
            language: Some("en".into()),
            topics: Some(vec!["automation".into(), "industry_4_0".into()]),
            sentiment_score: Some(0.7),
            key_phrases: Some(vec!["smart factory".into()]),
            ts_utc: Utc::now(),
            provenance: serde_json::json!({"url": "https://example.com/article"}),
            metadata: Some(serde_json::json!({})),
            created_at: Some(Utc::now()),
        };
        assert_eq!(row.artifact_type, "press_quote");
        assert!(row.topics.as_ref().unwrap().contains(&"automation".into()));
    }

    #[test]
    fn test_site_row_fields() {
        let row = SiteRow {
            id: Uuid::new_v4(),
            company_id: Some(Uuid::new_v4()),
            name: "Sousse Plant".into(),
            address: Some("Zone Industrielle".into()),
            city: Some("Sousse".into()),
            country_code: Some("TN".into()),
            region: Some("TN".into()),
            lat: Some(35.8256),
            lon: Some(10.6369),
            site_type: Some("plant".into()),
            capabilities: Some(vec!["SMT".into(), "THT".into(), "AOI".into()]),
            certifications: Some(vec!["ISO_9001".into(), "IATF_16949".into()]),
            employee_estimate: Some(300),
            free_zone: Some("TAC".into()),
            metadata: Some(serde_json::json!({})),
            created_at: Some(Utc::now()),
            updated_at: Some(Utc::now()),
        };
        assert_eq!(row.name, "Sousse Plant");
        assert!(row.lat.unwrap() > 35.0);
        assert!(row.capabilities.as_ref().unwrap().contains(&"SMT".into()));
    }
}
