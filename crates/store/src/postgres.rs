use anyhow::Result;
use chrono::{DateTime, NaiveDate, Utc};
use serde_json::Value;
use sqlx::postgres::{PgPool, PgPoolOptions};
use sqlx::{Postgres, QueryBuilder, Row, Transaction};
use uuid::Uuid;

use apex_core::entities::*;
use apex_core::validation::normalize_url;

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

fn normalize_optional_text(value: Option<&str>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn normalize_insight_feedback_type(value: &str) -> Option<String> {
    let normalized = value.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "bookmarked" | "actioned" | "dismissed" | "false_positive" | "false_negative"
        | "true_positive" | "relevant" | "irrelevant" | "viewed" => Some(normalized),
        _ => None,
    }
}

fn normalize_dedup_token(token: &str) -> Option<String> {
    let trimmed = token.trim_matches(|character: char| !character.is_ascii_alphanumeric());
    if trimmed.len() <= 2 {
        return None;
    }

    let lowered = trimmed.to_ascii_lowercase();
    if matches!(
        lowered.as_str(),
        "the"
            | "and"
            | "for"
            | "with"
            | "from"
            | "that"
            | "this"
            | "into"
            | "their"
            | "there"
            | "have"
            | "will"
            | "after"
            | "before"
            | "through"
            | "about"
            | "under"
            | "over"
            | "between"
            | "while"
            | "where"
            | "which"
            | "could"
            | "should"
            | "would"
            | "than"
            | "then"
            | "they"
            | "them"
            | "your"
            | "ours"
            | "also"
            | "just"
            | "only"
            | "more"
            | "most"
            | "very"
            | "real"
            | "same"
            | "does"
            | "been"
            | "being"
            | "were"
            | "when"
            | "what"
            | "whose"
            | "amid"
            | "around"
            | "across"
            | "within"
            | "without"
    ) {
        return None;
    }

    let canonical = if lowered.len() > 6 && lowered.ends_with("ies") {
        format!("{}y", &lowered[..lowered.len() - 3])
    } else if lowered.len() > 6 && lowered.ends_with("ing") {
        lowered[..lowered.len() - 3].to_string()
    } else if lowered.len() > 5 && (lowered.ends_with("ed") || lowered.ends_with("es")) {
        lowered[..lowered.len() - 2].to_string()
    } else if lowered.len() > 4 && lowered.ends_with('s') {
        lowered[..lowered.len() - 1].to_string()
    } else {
        lowered
    };

    if canonical.len() <= 2 {
        None
    } else {
        Some(canonical)
    }
}

fn dedup_signature_from_texts(parts: &[&str]) -> Option<String> {
    let mut tokens = parts
        .iter()
        .flat_map(|part| part.split(|character: char| !character.is_ascii_alphanumeric()))
        .filter_map(normalize_dedup_token)
        .collect::<Vec<_>>();

    tokens.sort();
    tokens.dedup();
    if tokens.is_empty() {
        None
    } else {
        Some(tokens.into_iter().take(18).collect::<Vec<_>>().join(" "))
    }
}

fn normalize_warning_title_for_dedup(title: &str) -> String {
    let trimmed = title.trim();
    if let Some(rest) = trimmed.strip_prefix('[') {
        if let Some((_, suffix)) = rest.split_once(']') {
            return suffix.trim().to_string();
        }
    }
    trimmed.to_string()
}

fn is_internal_insight_type(insight_type: Option<&str>) -> bool {
    matches!(
        insight_type.map(|value| value.trim().to_ascii_lowercase()),
        Some(kind)
            if kind.starts_with("llm_")
                || kind == "bias_mitigation"
                || kind == "hypothesis_ach"
    )
}

fn append_internal_insight_filter_sql_clause(qb: &mut QueryBuilder<Postgres>, col_prefix: &str) {
    qb.push("(")
        .push(col_prefix)
        .push("insight_type IS NULL OR (lower(")
        .push(col_prefix)
        .push("insight_type) NOT LIKE 'llm_%' AND lower(")
        .push(col_prefix)
        .push("insight_type) <> 'bias_mitigation' AND lower(")
        .push(col_prefix)
        .push("insight_type) <> 'hypothesis_ach'))");
}

fn filter_visible_insights(rows: Vec<InsightRow>) -> Vec<InsightRow> {
    rows.into_iter()
        .filter(|row| !row.title.trim().is_empty())
        .filter(|row| !is_internal_insight_type(row.insight_type.as_deref()))
        .collect()
}

fn recent_story_dedup_signature(
    summary: &str,
    _insight_type: Option<&str>,
    _entity_ids: Option<&[Uuid]>,
) -> Option<String> {
    dedup_signature_from_texts(&[summary])
}

fn recent_warning_dedup_signature(title: &str, description: Option<&str>) -> Option<String> {
    let normalized_title = normalize_warning_title_for_dedup(title);
    dedup_signature_from_texts(&[normalized_title.as_str(), description.unwrap_or_default()])
}

fn insight_dedup_key(title: &str, insight_type: Option<&str>, region: Option<&str>) -> String {
    format!(
        "{}|{}|{}",
        title.trim().to_lowercase(),
        insight_type.unwrap_or("").to_lowercase(),
        region.unwrap_or("").to_lowercase(),
    )
}

fn append_legacy_malformed_veracity_sql_clause(qb: &mut QueryBuilder<Postgres>, col_prefix: &str) {
    qb.push("(")
        .push(col_prefix)
        .push("insight_type IS NULL OR lower(")
        .push(col_prefix)
        .push("insight_type) <> 'veracity' OR coalesce(trim(")
        .push(col_prefix)
        .push("summary), '') <> '')");
}

mod admin;
pub use admin::is_valid_manual_trigger_kind;
mod analytics;
mod artifacts;
mod battlecards;
pub use battlecards::BattlecardRow;
mod alert_configs;
mod alert_subscriptions;
pub use alert_subscriptions::UserAlertSubscriptionRecord;
mod app_users;
pub use app_users::{AppUserRecord, AppUserSeed};
mod collaboration;
mod companies;
mod entity_review;
pub use entity_review::EntityReviewRow;
mod company_assets;
mod competitors;
pub mod embeddings;
mod graph;
mod heartbeats;
pub use heartbeats::ServiceHeartbeatRow;
pub use heartbeats::{latest_service_instance_heartbeat, WORKER_HEARTBEAT_STALE_AFTER_SECS};
mod history;
mod insights;
pub use insights::InsightClaimRow;
mod learning_eval;
pub use learning_eval::{
    LearningEvalExampleInput, LearningEvalMetricInput, LearningEvalMetricRow, LearningEvalRunInput,
    LearningEvalRunRow,
};
mod llm_cache;
mod llm_governance;
mod logistics;
mod memos;
mod observations;
mod persons;
mod preferences;
mod recipes;
mod sales;
mod security;
mod sources;
pub mod trends;
mod warnings;
pub use warnings::WarningInsertOutcome;

// Source-runtime scheduling row types and backoff policy (migration 047).
pub use sources::{
    ewma_latency_ms, ewma_success_rate, failure_backoff, next_due_after_success, DueSourceRow,
    SourceRuntimeStateRow, EWMA_ALPHA, FAILURE_BACKOFF_LADDER, MAX_FAILURE_BACKOFF,
};

// Sales-activation row types (re-exported for handlers/workers).
pub use sales::{
    BuyingCenterMemberRow, BuyingCenterRow, ClosedDealRow, CompetitorPricingRow, ContactMethodRow,
    CrawlMetricRow, EngagementEventRow, IcpTargetRow, NewBuyingMember, NewClosedDeal,
    NewContactMethod, NewCrawlMetric, NewEngagementEvent, RealSourceTelemetry,
};

#[derive(Debug, Clone, Default)]
pub struct WarningListFilters {
    pub regions: Vec<String>,
    pub severities: Vec<String>,
    pub warning_types: Vec<String>,
    pub acknowledged: Option<bool>,
    pub date_from: Option<DateTime<Utc>>,
    pub date_to: Option<DateTime<Utc>>,
    pub search: Option<String>,
    pub exclude_hygiene_signals: bool,
    pub include_deleted: bool,
}

#[derive(Debug, Clone, Default)]
pub struct InsightListFilters {
    pub regions: Vec<String>,
    pub date_from: Option<DateTime<Utc>>,
    pub date_to: Option<DateTime<Utc>>,
    pub search: Option<String>,
    pub insight_types: Vec<String>,
    pub bookmarked_by: Option<String>,
    pub exclude_internal: bool,
}

#[derive(Debug, Clone, Default)]
pub struct CompanyListFilters {
    pub regions: Vec<String>,
    pub search: Option<String>,
    pub is_competitor: Option<bool>,
}

#[derive(Debug, Clone, Default)]
pub struct PersonListFilters {
    pub regions: Vec<String>,
    pub roles: Vec<String>,
    pub search: Option<String>,
    pub min_priority: Option<f64>,
    pub max_priority: Option<f64>,
}

#[derive(Debug, Clone, Copy)]
pub enum WarningOrderBy {
    CreatedAt,
    Severity,
    WarningType,
}

#[derive(Debug, Clone, Copy)]
pub enum CompanyOrderBy {
    Name,
    Region,
    ThreatScore,
    UpdatedAt,
}

#[derive(Debug, Clone, Copy)]
pub enum PersonOrderBy {
    Name,
    Priority,
    Region,
    UpdatedAt,
}

/// Review outcome for an acknowledged warning.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum WarningReviewOutcome {
    TruePositive,
    FalsePositive,
    Inconclusive,
}

/// Result of acknowledging/reviewing a warning.
#[derive(Debug, Clone)]
pub enum AcknowledgeWarningResult {
    Acknowledged,
    ReviewedExisting,
    AlreadyAcknowledged,
    NotFound,
}

/// User settings preferences — persisted as a JSON blob in the
/// `user_preferences.preferences` JSON object under the `settings_page` key
/// (see `crates/store/src/postgres/preferences.rs`).
///
/// Personal preferences only. System configuration (crawl scheduling, LLM,
/// SMTP, source budgets, retention, alert policy, integrations) is owned by
/// the services that read it from the environment at startup and is rendered
/// read-only — it must never be duplicated here as a pretend control.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct UserSettingsPrefs {
    #[serde(default = "default_session_timeout")]
    pub session_timeout_hours: i64,
    #[serde(default = "default_region_val")]
    pub default_region: String,
    #[serde(default = "default_min_severity")]
    pub minimum_severity: String,
    #[serde(default)]
    pub critical_only_enabled: bool,
    #[serde(default)]
    pub email_digest_enabled: bool,
    #[serde(default = "default_notification_frequency")]
    pub notification_frequency: String,
    #[serde(default)]
    pub email_digest_recipients: String,
    #[serde(default)]
    pub email_digest_categories: Vec<String>,
    #[serde(default = "default_digest_time")]
    pub email_digest_time_cet: String,
    #[serde(default = "default_digest_weekday")]
    pub email_digest_weekday: String,
    #[serde(default)]
    pub email_digest_last_sent_at: Option<DateTime<Utc>>,
    /// Data table density: "comfortable" | "compact".
    #[serde(default = "default_table_layout")]
    pub table_layout: String,
}

fn default_session_timeout() -> i64 {
    24
}
fn default_region_val() -> String {
    "Global".into()
}
fn default_min_severity() -> String {
    "high".into()
}
fn default_notification_frequency() -> String {
    "Daily".into()
}
fn default_digest_time() -> String {
    "08:00".into()
}
fn default_digest_weekday() -> String {
    "Mon".into()
}
fn default_table_layout() -> String {
    "comfortable".into()
}

impl Default for UserSettingsPrefs {
    fn default() -> Self {
        Self {
            session_timeout_hours: default_session_timeout(),
            default_region: default_region_val(),
            minimum_severity: default_min_severity(),
            critical_only_enabled: false,
            email_digest_enabled: false,
            notification_frequency: default_notification_frequency(),
            email_digest_recipients: String::new(),
            email_digest_categories: Vec::new(),
            email_digest_time_cet: default_digest_time(),
            email_digest_weekday: default_digest_weekday(),
            email_digest_last_sent_at: None,
            table_layout: default_table_layout(),
        }
    }
}

/// Recipe quality summary - aggregated from recipe stats.
#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct RecipeQualitySummaryRow {
    pub avg_precision_pct: i64,
    pub coverage_pct: i64,
}

/// Analyst notification record - maps to `analyst_notifications` table.
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

/// Annotation record - maps to `annotations` table.
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

/// LLM governance overview for admin dashboard.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct LlmGovernanceOverview {
    pub prompt_versions: Vec<PromptVersionRecord>,
    pub workflow_runs: Vec<LlmWorkflowRunRecord>,
    pub improvement_runs: Vec<LlmImprovementRunRecord>,
    pub training_datasets: Vec<LlmTrainingDatasetRecord>,
}

/// Admin-specific LLM governance overview (may diverge from the general one in the future).
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct AdminLlmGovernanceOverview {
    pub prompt_versions: Vec<PromptVersionRecord>,
    pub workflow_runs: Vec<LlmWorkflowRunRecord>,
    pub improvement_runs: Vec<LlmImprovementRunRecord>,
    pub training_datasets: Vec<LlmTrainingDatasetRecord>,
}

/// Historical label for quality gate golden set examples.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum HistoricalQualityGateLabel {
    Accepted,
    Rejected,
}

impl HistoricalQualityGateLabel {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
        }
    }
}

/// A single reviewed example in the quality gate golden set.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct QualityGateGoldenSetExample {
    pub source_id: Uuid,
    pub source_kind: String,
    pub content_type: String,
    pub historical_label: HistoricalQualityGateLabel,
    pub title: String,
    pub body: String,
    pub region: Option<String>,
    pub confidence: Option<f64>,
    pub source_urls: Vec<String>,
    pub reviewed_at: DateTime<Utc>,
    pub metadata: serde_json::Value,
}

/// An exported quality gate golden set dataset.
#[derive(Debug, Clone, serde::Serialize)]
pub struct QualityGateGoldenSetExport {
    pub dataset_id: Uuid,
    pub dataset_name: String,
    pub dataset_version: String,
    pub example_count: i64,
    pub accepted_count: usize,
    pub rejected_count: usize,
    pub agreement_target: f64,
    pub examples: Vec<QualityGateGoldenSetExample>,
}

/// Prompt version record - maps to `prompt_versions` table.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct PromptVersionRecord {
    pub prompt_id: String,
    pub version: String,
    pub workflow: String,
    pub system_prompt: String,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

/// LLM workflow run record - maps to `llm_workflow_runs` table.
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

/// LLM improvement run record - maps to `llm_improvement_runs` table.
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

/// LLM training dataset record - maps to `llm_training_datasets` table.
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

#[derive(Clone)]
pub struct PgStore {
    pub pool: PgPool,
}

impl PgStore {
    pub async fn connect(database_url: &str) -> Result<Self> {
        let max_conns: u32 = std::env::var("PG_MAX_CONNECTIONS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(20);
        let pool = PgPoolOptions::new()
            .max_connections(max_conns)
            .min_connections(2)
            .acquire_timeout(std::time::Duration::from_secs(5))
            .idle_timeout(std::time::Duration::from_secs(600))
            .max_lifetime(std::time::Duration::from_secs(1800))
            .after_connect(|conn, _meta| {
                Box::pin(async move {
                    sqlx::query("SET statement_timeout = '30s'")
                        .execute(&mut *conn)
                        .await?;
                    // Default session identity for service (unscoped) work.
                    // Scoped transactions override this locally with the
                    // authenticated user via `begin_scoped`, so RLS policies
                    // can be FORCEd without locking the application role out
                    // of legitimate service paths (digest scan, admin reads).
                    Self::assume_service_identity(&mut *conn).await?;
                    Ok(())
                })
            })
            .connect(database_url)
            .await?;
        Ok(Self { pool })
    }

    pub fn from_pool(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Mark a connection with the default unscoped `service` identity.
    ///
    /// RLS is FORCEd on the user-private tables, and a connection with no
    /// identity matches neither the owner policy (`user_id =
    /// current_user_id()` is NULL) nor the `service` policy. Every pool that
    /// runs unscoped service paths — the API pool and the worker pool — must
    /// apply this in its `after_connect` hook.
    pub async fn assume_service_identity(
        conn: &mut sqlx::PgConnection,
    ) -> std::result::Result<(), sqlx::Error> {
        sqlx::query("SELECT set_config('app.current_user_role', 'service', false)")
            .execute(conn)
            .await?;
        Ok(())
    }

    /// Begin a transaction scoped to an application identity.
    ///
    /// Sets `app.current_user_id` and `app.current_user_role` with
    /// `set_config(..., true)`, so the settings are transaction-local: they are
    /// rolled back with the transaction and can never leak into a pooled
    /// session. The RLS helper functions from migration 020
    /// (`current_user_id()` / `current_user_role()`) read these settings.
    pub async fn begin_scoped(
        &self,
        user_id: &str,
        role: &str,
    ) -> Result<Transaction<'_, Postgres>> {
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "SELECT set_config('app.current_user_id', $1, true), \
                    set_config('app.current_user_role', $2, true)",
        )
        .bind(user_id)
        .bind(role)
        .execute(&mut *tx)
        .await?;
        Ok(tx)
    }

    // --- Schema Migration ---

    /// Run the full schema creation. Idempotent via IF NOT EXISTS.
    ///
    /// `APEX_SKIP_MIGRATIONS` is an escape hatch for deployments whose schema
    /// is managed out-of-band; it does **not** mean "assume the schema is
    /// fine". In skip mode the newest applied migration must match the newest
    /// embedded migration by version and checksum, so no process ever runs
    /// against an unknown or stale schema.
    pub async fn run_migrations(&self) -> Result<()> {
        if skip_migrations_enabled() {
            tracing::info!(
                "APEX_SKIP_MIGRATIONS is set — verifying the applied schema instead of migrating"
            );
            return self.verify_schema_is_current().await;
        }
        sqlx::migrate!("../../migrations").run(&self.pool).await?;
        Ok(())
    }

    /// Verify the applied migration history against the embedded migrations:
    /// the newest versions must be equal, every embedded migration up to the
    /// applied latest must be present, successful, and checksum-identical, and
    /// no applied migration may be unknown to this binary.
    pub async fn verify_schema_is_current(&self) -> Result<()> {
        let migrator = sqlx::migrate!("../../migrations");
        let embedded: Vec<(i64, Vec<u8>)> = migrator
            .iter()
            .map(|migration| (migration.version, migration.checksum.to_vec()))
            .collect();
        let applied = sqlx::query_as::<_, (i64, Vec<u8>, bool)>(
            "SELECT version, checksum, success FROM _sqlx_migrations ORDER BY version ASC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|error| anyhow::anyhow!("failed to read _sqlx_migrations: {error}"))?;
        ensure_migrations_match(&applied, &embedded)
    }
}

fn skip_migrations_enabled() -> bool {
    std::env::var("APEX_SKIP_MIGRATIONS")
        .ok()
        .map(|value| apex_core::env::parse_truthy_flag(&value))
        .unwrap_or(false)
}

/// Fail unless the applied migration history exactly matches the embedded
/// migrations up to the applied latest version.
fn ensure_migrations_match(
    applied: &[(i64, Vec<u8>, bool)],
    embedded: &[(i64, Vec<u8>)],
) -> Result<()> {
    let Some((latest_embedded, _)) = embedded.iter().max_by_key(|(version, _)| *version) else {
        anyhow::bail!("no embedded migrations found");
    };
    let Some((latest_applied, _, _)) = applied.iter().max_by_key(|(version, _, _)| *version) else {
        anyhow::bail!(
            "no migrations have been applied to this database (embedded latest is version {latest_embedded})"
        );
    };
    if latest_applied != latest_embedded {
        anyhow::bail!(
            "database schema is stale: latest applied migration is {latest_applied}, embedded latest is {latest_embedded}"
        );
    }

    for (version, embedded_checksum) in embedded
        .iter()
        .filter(|(version, _)| version <= latest_applied)
    {
        let Some((_, applied_checksum, success)) = applied
            .iter()
            .find(|(applied_version, _, _)| applied_version == version)
        else {
            anyhow::bail!("database schema is missing embedded migration {version}");
        };
        if !success {
            anyhow::bail!("applied migration {version} did not complete successfully");
        }
        if applied_checksum != embedded_checksum {
            anyhow::bail!(
                "database schema checksum mismatch for migration {version}: applied {} vs embedded {}",
                hex::encode(applied_checksum),
                hex::encode(embedded_checksum)
            );
        }
    }

    for (version, _, _) in applied {
        if !embedded
            .iter()
            .any(|(embedded_version, _)| embedded_version == version)
        {
            anyhow::bail!("database schema contains unknown migration {version}");
        }
    }
    Ok(())
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

/// A statistically-mined pattern candidate row to persist for audit and
/// analytics counters in the `pattern_candidates` table.
#[derive(Debug, Clone)]
pub struct MinedPatternCandidate {
    /// Stable code identifying the candidate's outcome stream (e.g. `mined_WebChange`).
    pub recipe_code: String,
    /// Entity/outcome type the pattern was mined for.
    pub entity_type: String,
    /// Human-readable summary of the mined relationship and its statistics.
    pub pattern_label: String,
    /// Whether the candidate cleared the statistical gates (effect, p-value, stability, FDR).
    pub passed_gates: bool,
    /// Confidence score in [0, 1] derived from the candidate's stability.
    pub confidence: f64,
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
    pub precision_current: f64,
    pub precision_baseline: f64,
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
    pub is_competitor: Option<bool>,
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
    pub role_family: String,
    pub country: String,
    pub organization: String,
    pub region: String,
    pub priority_score: f64,
    pub pain_index: f64,
    pub change_risk: f64,
    pub role_drift_score: f64,
    pub engagement_status: String,
    pub updated_at: DateTime<Utc>,
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
    /// Why the signal matters (warnings.impact); drives the warning "Why" brief.
    pub impact: Option<String>,
    /// Recommended next actions (warnings.actions); empty means "derive defaults".
    pub actions: Option<Vec<String>>,
    pub ts_utc: DateTime<Utc>,
    pub acknowledged: bool,
    pub acknowledged_by: Option<String>,
    pub acknowledged_at: Option<DateTime<Utc>>,
    pub acknowledged_note: Option<String>,
    pub review_outcome: Option<String>,
    pub deleted_at: Option<DateTime<Utc>>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

/// Semantic-merge bookkeeping accumulated on the triage queue row backing a
/// warning/insight. Warnings store one row per deduplicated signal; repeats
/// are counted here (`occurrence_count`) with merged source URLs and the
/// first/last observation timestamps, so the UI can answer "occurrences /
/// first seen / last seen / severity escalation" from the merge engine itself.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct TriageMergeInfo {
    pub id: Uuid,
    pub item_type: String,
    pub source_id: String,
    pub static_severity: Option<String>,
    pub occurrence_count: i32,
    pub first_seen_at: DateTime<Utc>,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub merged_source_urls: Vec<String>,
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
    pub metadata: Option<serde_json::Value>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct InsightFeedbackEventRow {
    pub id: Uuid,
    pub insight_id: Uuid,
    pub entity_id: Option<Uuid>,
    pub recipe_code: Option<String>,
    pub feedback_type: String,
    pub user_id: String,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct InsightFiringRow {
    pub id: Uuid,
    pub insight_id: Option<Uuid>,
    pub entity_id: Uuid,
    pub recipe_code: String,
    pub insight_type: Option<String>,
    pub title: String,
    pub summary: String,
    pub insight_hash: String,
    pub confidence: Option<f64>,
    pub created_at: DateTime<Utc>,
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

/// Result of a single recipe threshold auto-calibration adjustment.
/// Returned by [`PgStore::auto_calibrate_recipe_thresholds`].
#[derive(Debug, Clone, serde::Serialize)]
pub struct CalibrationAdjustment {
    pub recipe_code: String,
    pub current_precision: f64,
    pub new_precision: f64,
    pub avg_fp_rate_4w: f64,
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

/// Analytical summary attached to entity dossiers.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DossierAnalysis {
    pub evidence_quality: apex_core::analysis::EvidenceQuality,
    pub temporal_delta: apex_core::analysis::TemporalDelta,
    pub correlated_signals: Vec<apex_core::analysis::WeakSignalCluster>,
    pub competing_hypotheses: Vec<apex_core::analysis::HypothesisScorecard>,
    pub summary: String,
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

// --- Collaboration Record Types (Phase 4.3) ---

/// Strategic opportunity record - maps to `strategic_opportunities` table.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct StrategicOpportunityRecord {
    pub id: Uuid,
    pub title: String,
    pub description: Option<String>,
    pub opportunity_type: String,
    pub priority_score: f64,
    pub confidence: f64,
    pub entity_id: Option<String>,
    pub entity_type: Option<String>,
    pub region: Option<String>,
    pub estimated_value: Option<String>,
    pub recommended_actions: serde_json::Value,
    pub owner_id: Option<String>,
    pub status: String,
    pub due_date: Option<DateTime<Utc>>,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Critical threat record - maps to `critical_threats` table.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct CriticalThreatRecord {
    pub id: Uuid,
    pub title: String,
    pub description: Option<String>,
    pub threat_type: String,
    pub severity: String,
    pub impact_score: f64,
    pub confidence: f64,
    pub entity_id: Option<String>,
    pub entity_type: Option<String>,
    pub region: Option<String>,
    pub mitigation_steps: serde_json::Value,
    pub owner_id: Option<String>,
    pub status: String,
    pub sla_deadline: Option<DateTime<Utc>>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Investigation workspace record - maps to `investigation_workspaces` table.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct InvestigationWorkspaceRecord {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub workspace_type: String,
    pub owner_id: String,
    pub team_id: Option<String>,
    pub status: String,
    pub visibility: String,
    pub tags: Vec<String>,
    pub entity_focus: serde_json::Value,
    pub findings: Option<String>,
    pub conclusions: Option<String>,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub closed_at: Option<DateTime<Utc>>,
}

/// Workspace assignment record - maps to `workspace_assignments` table.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct WorkspaceAssignmentRecord {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub user_id: String,
    pub role: String,
    pub assigned_by: String,
    pub assigned_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Activity feed record - maps to `activity_feed` table.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct ActivityFeedRecord {
    pub id: Uuid,
    pub actor_id: String,
    pub actor_name: String,
    pub action_type: String,
    pub entity_type: Option<String>,
    pub entity_id: Option<String>,
    pub entity_name: Option<String>,
    pub details: serde_json::Value,
    pub workspace_id: Option<Uuid>,
    pub team_id: Option<String>,
    pub visibility: String,
    pub created_at: DateTime<Utc>,
}

/// Investigation share record - maps to `investigation_shares` table.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct InvestigationShareRecord {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub shared_by: String,
    pub shared_with: String,
    pub share_type: String,
    pub access_level: String,
    pub message: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// Priority queue item record - maps to `daily_priority_queue` table.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct PriorityQueueItemRecord {
    pub id: Uuid,
    pub user_id: String,
    pub queue_date: NaiveDate,
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

/// Supplier risk entry record - maps to `supplier_risk_entries` table.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct SupplierRiskEntryRecord {
    pub id: Uuid,
    pub supplier_id: String,
    pub risk_category: String,
    pub risk_score: f64,
    pub risk_factors: serde_json::Value,
    pub mitigation: Option<String>,
    pub owner_id: Option<String>,
    pub status: String,
    pub last_reviewed: Option<DateTime<Utc>>,
    pub next_review: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Pipeline opportunity record - maps to `pipeline_opportunities` table.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct PipelineOpportunityRecord {
    pub id: Uuid,
    pub opportunity_id: Option<String>,
    pub title: String,
    pub stage: String,
    pub value_estimate: Option<f64>,
    pub probability: f64,
    pub owner_id: Option<String>,
    pub expected_close: Option<NaiveDate>,
    pub actual_close: Option<NaiveDate>,
    pub notes: Option<String>,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub closed_at: Option<DateTime<Utc>>,
}

/// Source evidence record - maps to `source_evidence` table.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct SourceEvidenceRecord {
    pub id: Uuid,
    pub entity_type: String,
    pub entity_id: String,
    pub evidence_type: String,
    pub source_url: String,
    pub source_domain: Option<String>,
    pub source_name: Option<String>,
    pub reliability_score: f64,
    pub content_hash: Option<String>,
    pub excerpt: Option<String>,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

/// Team assignment record - maps to `team_assignments` table.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct TeamAssignmentRecord {
    pub id: Uuid,
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

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
            is_competitor: Some(false),
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
            public_bio: Some("Senior procurement executive".into()),
            public_email: Some("ahmed@example.com".into()),
            priority_vector: Some(serde_json::json!({"cost":0.8,"quality":0.6})),
            influence_score: Some(0.7),
            trigger_topics: Some(vec!["cost reduction".into(), "supply chain".into()]),
            decision_style: Some("analytical".into()),
            risk_tolerance: Some("moderate".into()),
            change_appetite: Some("high".into()),
            communication_style: Some("formal".into()),
            decision_mode: Some("data-driven".into()),
            preferred_proof_type: Some("ROI metrics".into()),
            pain_index: Some(0.3),
            change_risk: Some(0.4),
            role_drift_score: Some(0.1),
            metadata: Some(serde_json::json!({})),
            created_at: Some(Utc::now()),
            updated_at: Some(Utc::now()),
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

    #[test]
    fn test_filter_visible_insights_hides_hypothesis_rows() {
        let rows = vec![
            InsightRow {
                id: Uuid::new_v4(),
                title: "Hypothesis check: Flex Ltd leans pivoting".into(),
                summary: "Internal ACH summary".into(),
                insight_type: Some("hypothesis_ach".into()),
                region: Some("EU".into()),
                confidence: Some(0.78),
                evidence_urls: None,
                entity_ids: None,
                tags: None,
                metadata: None,
                created_at: Some(Utc::now()),
                updated_at: Some(Utc::now()),
            },
            InsightRow {
                id: Uuid::new_v4(),
                title: "Bias check for Digi-Key expansion thesis".into(),
                summary: "Internal devil's-advocate challenge reduced confidence".into(),
                insight_type: Some("bias_mitigation".into()),
                region: Some("EU".into()),
                confidence: Some(0.41),
                evidence_urls: None,
                entity_ids: None,
                tags: Some(vec!["bias:challenged".into()]),
                metadata: None,
                created_at: Some(Utc::now()),
                updated_at: Some(Utc::now()),
            },
            InsightRow {
                id: Uuid::new_v4(),
                title: "Procurement signal for Acme".into(),
                summary: "Buyer-side qualification activity surfaced in recent evidence.".into(),
                insight_type: Some("demand_procurement".into()),
                region: Some("EU".into()),
                confidence: Some(0.81),
                evidence_urls: None,
                entity_ids: None,
                tags: None,
                metadata: None,
                created_at: Some(Utc::now()),
                updated_at: Some(Utc::now()),
            },
        ];

        let visible = filter_visible_insights(rows);
        assert_eq!(visible.len(), 1);
        assert_eq!(
            visible[0].insight_type.as_deref(),
            Some("demand_procurement")
        );
    }

    #[test]
    fn test_append_internal_insight_filter_sql_clause_matches_expected() {
        let mut qb = QueryBuilder::<Postgres>::new("SELECT 1 WHERE ");

        append_internal_insight_filter_sql_clause(&mut qb, "");

        assert_eq!(
            qb.sql(),
            "SELECT 1 WHERE (insight_type IS NULL OR (lower(insight_type) NOT LIKE 'llm_%' AND lower(insight_type) <> 'bias_mitigation' AND lower(insight_type) <> 'hypothesis_ach'))"
        );
    }

    #[test]
    fn migrations_match_when_full_history_aligns() {
        let embedded = vec![(52, vec![5u8, 2]), (53, vec![1u8, 2, 3, 4])];
        let applied = vec![(52, vec![5u8, 2], true), (53, vec![1, 2, 3, 4], true)];
        assert!(ensure_migrations_match(&applied, &embedded).is_ok());
    }

    #[test]
    fn migrations_reject_empty_applied_history() {
        let error = ensure_migrations_match(&[], &[(53, vec![1, 2, 3])]).expect_err("must fail");
        assert!(error
            .to_string()
            .contains("no migrations have been applied"));
    }

    #[test]
    fn migrations_reject_failed_latest_row() {
        let error = ensure_migrations_match(&[(53, vec![1, 2, 3], false)], &[(53, vec![1, 2, 3])])
            .expect_err("must fail");
        assert!(error.to_string().contains("did not complete successfully"));
    }

    #[test]
    fn migrations_reject_stale_version() {
        let error = ensure_migrations_match(
            &[(52, vec![1, 2, 3], true)],
            &[(52, vec![1, 2, 3]), (53, vec![1, 2, 3])],
        )
        .expect_err("must fail");
        assert!(error.to_string().contains("schema is stale"));
        assert!(error.to_string().contains("52"));
    }

    #[test]
    fn migrations_reject_checksum_mismatch() {
        let error = ensure_migrations_match(&[(53, vec![9, 9, 9], true)], &[(53, vec![1, 2, 3])])
            .expect_err("must fail");
        assert!(error.to_string().contains("checksum mismatch"));
    }

    #[test]
    fn migrations_reject_missing_earlier_migration() {
        let error = ensure_migrations_match(
            &[(53, vec![1, 2, 3], true)],
            &[(52, vec![5, 2]), (53, vec![1, 2, 3])],
        )
        .expect_err("must fail");
        assert!(error.to_string().contains("missing embedded migration 52"));
    }

    #[test]
    fn migrations_reject_failed_earlier_migration() {
        let error = ensure_migrations_match(
            &[(52, vec![5, 2], false), (53, vec![1, 2, 3], true)],
            &[(52, vec![5, 2]), (53, vec![1, 2, 3])],
        )
        .expect_err("must fail");
        assert!(error.to_string().contains("migration 52"));
        assert!(error.to_string().contains("did not complete successfully"));
    }

    #[test]
    fn migrations_reject_unknown_applied_migration() {
        let error = ensure_migrations_match(
            &[(51, vec![7, 7], true), (53, vec![1, 2, 3], true)],
            &[(53, vec![1, 2, 3])],
        )
        .expect_err("must fail");
        assert!(error.to_string().contains("unknown migration 51"));
    }
}
