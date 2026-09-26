//! Shared intelligence ingress for warnings produced by worker jobs.
//!
//! Warning generation is not one job — anomaly detection, recipe firing,
//! security scans, dark-web monitoring, adversarial analysis, threat-intel
//! enrichment and the continuous-improvement quality gates all produce
//! warnings. Historically each producer called `PgStore::insert_warning`
//! directly, so only `anomaly_scan` wired the semantic triage ingress: the
//! remaining producers silently skipped semantic dedup, alert publication and
//! activity logging while still counting the warning as generated.
//!
//! [`IntelligenceIngress`] is the single ingress every producer now goes
//! through. One instance is constructed at worker startup and passed to jobs
//! (no per-job `TriageIngestor` construction). `submit_warning`:
//!
//! 1. upserts the warning through [`WarningWriter`] (deterministic dedup in
//!    Postgres returns the existing row id for a repeat) and commits the
//!    warning row **and its alert outbox event in one transaction** — there is
//!    no window where a persisted warning can lose its alert,
//! 2. submits the stored warning to [`TriageIngestor`] so semantic dedup
//!    merges repeats (occurrence counts increment instead of duplicate queue
//!    rows),
//! 3. attempts delivery of the committed alert event through [`AlertSink`]:
//!    the JetStream ACK is awaited, then the outbox event is stamped published
//!    (`crate::alert_pipeline` retries anything left unpublished after a
//!    crash),
//! 4. records an activity-feed entry through [`WarningWriter`],
//! 5. returns a [`WarningSubmissionResult`] with the real per-step outcome.
//!
//! A warning insert failure is returned as `Err`; triage/alert/activity
//! failures are reported in the result as degraded (the warning row is already
//! persisted, so dropping the outcome would be a silent data loss).
//!
//! # Audience policy
//! Generic entity warnings use [`AlertAudience::Users`] with no explicit ids:
//! the API alert router resolves the entity's subscribers for the category and
//! severity. [`AlertAudience::Broadcast`] is reserved for warnings explicitly
//! marked via [`NewWarning::system_broadcast`] — an unresolved targeted alert
//! reaches nobody instead of every connected user.
//!
//! # Capability requirement
//! When NATS is required (`REQUIRE_NATS=true` or `APEX_ENV=production`), an
//! unavailable JetStream makes [`AlertSink::publish_alert`] return `Err` and
//! the submission is reported degraded; a missing alert backend is never a
//! silent success.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use apex_core::alert_config::{AlertAudience, AlertSeverity};
use apex_core::triage::TriageItemType;
use apex_store::postgres::PgStore;
use apex_triage::semantic_dedup::{
    IngestOutcome, IngestQueue, SemanticDedup, TriageIngestor, TriageSubmission,
};
use apex_triage::TriageQueue;

use apex_worker::activity_logger::{ActivityEvent, ActivityLogger};
use apex_worker::nats_stream::{nats_required_from_env, AlertEvent, AlertEventType, NatsPublisher};

/// How long startup waits for NATS before falling back to a disabled publisher.
const NATS_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);

/// A warning as produced by a job, before dedup/insert/triage.
///
/// `enabled` controls whether the warning alerts at all (a disabled warning is
/// persisted and triaged but writes no outbox event), and `is_system_broadcast`
/// reserves [`AlertAudience::Broadcast`] for genuine system-wide alerts. The
/// default audience for an entity warning is [`AlertAudience::Users`] with no
/// explicit ids: the API alert router resolves the entity's real subscribers
/// for the warning category and severity, and an unresolved alert reaches
/// nobody instead of every connected user.
#[derive(Debug, Clone)]
pub struct NewWarning {
    warning_type: String,
    title: String,
    description: Option<String>,
    severity: String,
    region: Option<String>,
    recipe_code: Option<String>,
    entity_ids: Vec<Uuid>,
    source_urls: Vec<String>,
    confidence: Option<f64>,
    enabled: bool,
    is_system_broadcast: bool,
}

impl NewWarning {
    /// Create a warning with the three required fields. Alerts are enabled and
    /// targeted (never an implicit broadcast).
    pub fn new(
        warning_type: impl Into<String>,
        title: impl Into<String>,
        severity: impl Into<String>,
    ) -> Self {
        Self {
            warning_type: warning_type.into(),
            title: title.into(),
            description: None,
            severity: severity.into(),
            region: None,
            recipe_code: None,
            entity_ids: Vec::new(),
            source_urls: Vec::new(),
            confidence: None,
            enabled: true,
            is_system_broadcast: false,
        }
    }

    /// Set whether this warning produces an alert event (default `true`).
    /// Disabled warnings are still persisted, deduplicated and triaged.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Mark this warning as a genuine system-wide alert.
    ///
    /// Only operational, non-entity warnings (for example a source outage that
    /// affects every consumer) should use this; entity warnings must stay
    /// targeted so they are resolved against `user_alert_subscriptions`.
    pub fn system_broadcast(mut self) -> Self {
        self.is_system_broadcast = true;
        self
    }

    /// Whether this warning alerts at all.
    pub fn alerts_enabled(&self) -> bool {
        self.enabled
    }

    /// Whether this warning is a deliberate system-wide broadcast.
    pub fn is_system_broadcast(&self) -> bool {
        self.is_system_broadcast
    }

    /// The audience for this warning's alert event.
    pub fn audience(&self) -> AlertAudience {
        if self.is_system_broadcast {
            AlertAudience::Broadcast
        } else {
            AlertAudience::Users(Vec::new())
        }
    }

    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn region(mut self, region: impl Into<String>) -> Self {
        self.region = Some(region.into());
        self
    }

    pub fn recipe_code(mut self, recipe_code: impl Into<String>) -> Self {
        self.recipe_code = Some(recipe_code.into());
        self
    }

    pub fn entity_ids<I: IntoIterator<Item = Uuid>>(mut self, entity_ids: I) -> Self {
        self.entity_ids = entity_ids.into_iter().collect();
        self
    }

    pub fn source_urls<I: IntoIterator<Item = String>>(mut self, source_urls: I) -> Self {
        self.source_urls = source_urls.into_iter().collect();
        self
    }

    pub fn confidence(mut self, confidence: f64) -> Self {
        self.confidence = Some(confidence);
        self
    }
}

/// A persisted warning, with the deterministic dedup outcome attached.
#[derive(Debug, Clone)]
pub struct StoredWarning {
    pub id: Uuid,
    /// `true` when this submission created the row; `false` when it was
    /// deterministically deduplicated into an existing warning.
    pub created: bool,
    pub warning_type: String,
    pub title: String,
    pub description: Option<String>,
    pub severity: String,
    pub region: Option<String>,
    pub recipe_code: Option<String>,
    pub entity_ids: Vec<Uuid>,
    pub source_urls: Vec<String>,
    pub confidence: Option<f64>,
    pub occurred_at: DateTime<Utc>,
    /// The outbox event committed in the same transaction as this warning; the
    /// publisher marks it published only after the JetStream ACK.
    pub outbox_id: Option<Uuid>,
    /// The alert event serialized into the outbox row (audience already
    /// resolved to `Users([])` or `Broadcast`).
    pub alert: Option<AlertEvent>,
}

/// Build the alert event for a persisted warning.
///
/// Used both for the outbox payload (committed with the warning) and for the
/// immediate publish attempt, so the fast path and crash recovery publish the
/// exact same event.
fn build_alert_event(
    outcome_id: Uuid,
    warning: &NewWarning,
    occurred_at: DateTime<Utc>,
) -> AlertEvent {
    AlertEvent {
        id: outcome_id,
        event_type: AlertEventType::NewWarning,
        severity: AlertSeverity::from_str(&warning.severity),
        title: warning.title.clone(),
        description: warning.description.clone().unwrap_or_default(),
        entity_id: warning.entity_ids.first().copied(),
        entity_name: None,
        audience: warning.audience(),
        metadata: serde_json::json!({
            "warning_id": outcome_id,
            "warning_type": warning.warning_type,
            "recipe_code": warning.recipe_code,
            "region": warning.region,
            "confidence": warning.confidence,
        }),
        created_at: occurred_at,
    }
}

/// Outcome of submitting a warning to the semantic triage ingress.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TriageSubmissionOutcome {
    /// A new triage queue row was created.
    Enqueued {
        item_id: Uuid,
        occurrence_count: i64,
    },
    /// The submission merged into an existing queue row (repeat signal).
    Merged {
        item_id: Uuid,
        occurrence_count: i64,
        reason: String,
    },
    /// Triage was unavailable; the warning is persisted but not deduplicated.
    Failed { error: String },
}

impl TriageSubmissionOutcome {
    fn from_ingest(outcome: IngestOutcome) -> Self {
        match outcome {
            IngestOutcome::Enqueued(item) => Self::Enqueued {
                item_id: item.id,
                occurrence_count: item.occurrence_count,
            },
            IngestOutcome::Merged { item, reason } => Self::Merged {
                item_id: item.id,
                occurrence_count: item.occurrence_count,
                reason: reason.as_str().to_string(),
            },
        }
    }

    /// Occurrence count of the resulting triage row (`0` when triage failed).
    pub fn occurrence_count(&self) -> i64 {
        match self {
            Self::Enqueued {
                occurrence_count, ..
            }
            | Self::Merged {
                occurrence_count, ..
            } => *occurrence_count,
            Self::Failed { .. } => 0,
        }
    }

    /// Whether the submission merged into an existing triage row.
    pub fn merged(&self) -> bool {
        matches!(self, Self::Merged { .. })
    }

    pub fn item_id(&self) -> Option<Uuid> {
        match self {
            Self::Enqueued { item_id, .. } | Self::Merged { item_id, .. } => Some(*item_id),
            Self::Failed { .. } => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Enqueued { .. } => "enqueued",
            Self::Merged { .. } => "merged",
            Self::Failed { .. } => "failed",
        }
    }
}

/// Real outcome of one `submit_warning` call.
#[derive(Debug, Clone)]
pub struct WarningSubmissionResult {
    pub warning: StoredWarning,
    pub triage: TriageSubmissionOutcome,
    /// `true` only when an alert sink was configured AND the publish succeeded.
    pub alert_published: bool,
    pub alert_error: Option<String>,
    pub activity_recorded: bool,
    pub activity_error: Option<String>,
}

impl WarningSubmissionResult {
    pub fn warning_id(&self) -> Uuid {
        self.warning.id
    }

    /// `false` when the deterministic warning dedup merged this submission into
    /// an existing row.
    pub fn created(&self) -> bool {
        self.warning.created
    }

    /// Occurrence count of the resulting triage queue row.
    pub fn occurrence_count(&self) -> i64 {
        self.triage.occurrence_count()
    }

    /// Whether the triage ingress merged this submission into an existing row.
    pub fn triage_merged(&self) -> bool {
        self.triage.merged()
    }

    /// `true` when any non-fatal step (triage, alerts, activity) failed.
    pub fn degraded(&self) -> bool {
        matches!(self.triage, TriageSubmissionOutcome::Failed { .. })
            || self.alert_error.is_some()
            || self.activity_error.is_some()
    }
}

/// Per-job aggregation of warning-submission outcomes.
///
/// Distinguishes the stages the audit requires: persisted warnings, completed
/// triage submissions, ACKed alert publications, activity records, and degraded
/// submissions. [`IngressCounters::success_blocker`] encodes the hard rule that
/// a job which persisted warnings but never completed triage must not report
/// success.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IngressCounters {
    /// Warnings whose row committed (created or deduplicated).
    pub warnings_persisted: u64,
    /// Submissions whose semantic triage step succeeded.
    pub triage_completed: u64,
    /// Alerts published and acknowledged by the broker.
    pub alerts_published: u64,
    /// Activity-feed entries recorded.
    pub activities_recorded: u64,
    /// Submissions with at least one degraded (triage/alert/activity) step.
    pub degraded_warnings: u64,
}

impl IngressCounters {
    /// Record one successful `submit_warning` outcome.
    pub fn record(&mut self, result: &WarningSubmissionResult) {
        self.warnings_persisted += 1;
        if !matches!(result.triage, TriageSubmissionOutcome::Failed { .. }) {
            self.triage_completed += 1;
        }
        if result.alert_published {
            self.alerts_published += 1;
        }
        if result.activity_recorded {
            self.activities_recorded += 1;
        }
        if result.degraded() {
            self.degraded_warnings += 1;
        }
    }

    /// The reason this job must not report success, if any.
    ///
    /// Persisted warnings with zero completed triage submissions mean the
    /// alert-generation pipeline never ran: counting that as success hides a
    /// total alerting outage.
    pub fn success_blocker(&self) -> Option<String> {
        if self.warnings_persisted > 0 && self.triage_completed == 0 {
            return Some(format!(
                "{} warning(s) persisted but 0 triage completions — alerts were never generated",
                self.warnings_persisted
            ));
        }
        None
    }

    /// One-line counter summary for job notes.
    pub fn summary(&self) -> String {
        format!(
            "warnings_persisted={} triage_completed={} alerts_published={} \
             activities_recorded={} degraded_warnings={}",
            self.warnings_persisted,
            self.triage_completed,
            self.alerts_published,
            self.activities_recorded,
            self.degraded_warnings
        )
    }
}

/// Persistence half of the ingress.
#[async_trait]
pub trait WarningWriter: Send + Sync {
    /// Deterministically upsert the warning and report whether it was created
    /// or deduplicated into an existing row.
    async fn upsert_warning(&self, warning: &NewWarning) -> anyhow::Result<StoredWarning>;

    /// Record the warning in the activity feed. Implementations that have no
    /// activity store may keep the default no-op.
    async fn record_warning_activity(
        &self,
        _warning: &StoredWarning,
        _triage: &TriageSubmissionOutcome,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    /// Stamp the warning's outbox event published after the broker ACK.
    /// Implementations with no outbox may keep the default no-op.
    async fn mark_alert_published(&self, _outbox_id: Uuid) -> anyhow::Result<()> {
        Ok(())
    }
}

/// Alert/domain-event publication half of the ingress.
#[async_trait]
pub trait AlertSink: Send + Sync {
    /// Whether this sink can actually deliver alerts.
    fn enabled(&self) -> bool {
        true
    }

    /// Whether an unavailable alert backend is a hard failure for this
    /// deployment (production / `REQUIRE_NATS`).
    fn required(&self) -> bool {
        false
    }

    /// Publish an alert event, awaiting the broker ACK before reporting
    /// success.
    async fn publish_alert(&self, event: &AlertEvent) -> anyhow::Result<()>;
}

/// Object-safe view of the ingress used where a concrete generic type is
/// awkward (e.g. job helpers that only need to submit a warning).
#[async_trait]
pub trait WarningSubmitter: Send + Sync {
    async fn submit_warning(&self, warning: NewWarning) -> anyhow::Result<WarningSubmissionResult>;
}

/// Production [`WarningWriter`]: deterministic warning upsert plus activity log.
pub struct WarningService {
    store: Arc<PgStore>,
    activity: ActivityLogger,
}

impl WarningService {
    pub fn new(store: Arc<PgStore>) -> Self {
        let activity = ActivityLogger::new(store.pool.clone());
        Self { store, activity }
    }
}

#[async_trait]
impl WarningWriter for WarningService {
    async fn upsert_warning(&self, warning: &NewWarning) -> anyhow::Result<StoredWarning> {
        let occurred_at = Utc::now();
        let (outcome, outbox_id, alert) = if warning.enabled {
            // One transaction: the warning row and its alert outbox event
            // commit together, so no crash window can lose the alert.
            let (outcome, outbox_id) = self
                .store
                .insert_warning_with_outbox(
                    &warning.warning_type,
                    &warning.title,
                    warning.description.as_deref(),
                    &warning.severity,
                    warning.region.as_deref(),
                    warning.recipe_code.as_deref(),
                    non_empty(warning.entity_ids.clone()),
                    non_empty(warning.source_urls.clone()),
                    warning.confidence,
                    "warning",
                    "new_warning",
                    |outcome| {
                        serde_json::to_value(build_alert_event(outcome.id, warning, occurred_at))
                            .expect("AlertEvent is always serializable")
                    },
                )
                .await?;
            (
                outcome,
                Some(outbox_id),
                Some(build_alert_event(outcome.id, warning, occurred_at)),
            )
        } else {
            let outcome = self
                .store
                .insert_warning_with_outcome(
                    &warning.warning_type,
                    &warning.title,
                    warning.description.as_deref(),
                    &warning.severity,
                    warning.region.as_deref(),
                    warning.recipe_code.as_deref(),
                    non_empty(warning.entity_ids.clone()),
                    non_empty(warning.source_urls.clone()),
                    warning.confidence,
                )
                .await?;
            (outcome, None, None)
        };
        Ok(StoredWarning {
            id: outcome.id,
            created: outcome.created,
            warning_type: warning.warning_type.clone(),
            title: warning.title.clone(),
            description: warning.description.clone(),
            severity: warning.severity.clone(),
            region: warning.region.clone(),
            recipe_code: warning.recipe_code.clone(),
            entity_ids: warning.entity_ids.clone(),
            source_urls: warning.source_urls.clone(),
            confidence: warning.confidence,
            occurred_at,
            outbox_id,
            alert,
        })
    }

    async fn mark_alert_published(&self, outbox_id: Uuid) -> anyhow::Result<()> {
        // `false` means the drain publisher already stamped the event (it
        // published the same outbox row concurrently) or the row is gone; both
        // are duplicate-delivery, not loss, so they are not errors.
        self.store.mark_outbox_published(outbox_id).await?;
        Ok(())
    }

    async fn record_warning_activity(
        &self,
        warning: &StoredWarning,
        triage: &TriageSubmissionOutcome,
    ) -> anyhow::Result<()> {
        let details = serde_json::json!({
            "warning_id": warning.id.to_string(),
            "warning_type": warning.warning_type,
            "severity": warning.severity,
            "created": warning.created,
            "triage": triage.as_str(),
            "occurrence_count": triage.occurrence_count(),
        });
        let entity_id = warning.id.to_string();
        self.activity
            .insert(ActivityEvent {
                actor_id: "system",
                actor_name: "Intelligence Ingress",
                action_type: if warning.created {
                    "warning_created"
                } else {
                    "warning_updated"
                },
                entity_type: Some("warning"),
                entity_id: Some(&entity_id),
                entity_name: Some(&warning.title),
                details: &details,
                workspace_id: None,
                team_id: None,
                visibility: "team",
            })
            .await;
        Ok(())
    }
}

fn non_empty<T>(values: Vec<T>) -> Option<Vec<T>> {
    if values.is_empty() {
        None
    } else {
        Some(values)
    }
}

/// Production [`AlertSink`]: publishes a `new_warning` alert event to the NATS
/// JetStream `alerts` stream. Gracefully degrades to a disabled sink when NATS
/// is not configured/unreachable.
pub struct AlertPublisher {
    publisher: NatsPublisher,
    enabled: bool,
}

impl AlertPublisher {
    pub fn new(publisher: NatsPublisher) -> Self {
        let enabled = publisher.is_connected();
        Self { publisher, enabled }
    }

    /// A publisher that never delivers alerts (no NATS configured).
    pub fn disabled() -> Self {
        Self::new(NatsPublisher::disabled())
    }

    /// A disabled publisher for deployments where NATS is required: every
    /// publish reports an error instead of silently succeeding.
    pub fn disabled_with_requirement(required: bool) -> Self {
        Self::new(NatsPublisher::disabled_with_requirement(required))
    }

    /// Connect to NATS with a bounded timeout.
    pub async fn connect(nats_url: &str) -> Self {
        match tokio::time::timeout(NATS_CONNECT_TIMEOUT, NatsPublisher::connect(nats_url)).await {
            Ok(publisher) => Self::new(publisher),
            Err(_) => {
                let required = nats_required_from_env();
                tracing::warn!(
                    nats_url,
                    timeout_secs = NATS_CONNECT_TIMEOUT.as_secs(),
                    required,
                    "intelligence_ingress: NATS connect timed out; alert publishing degraded"
                );
                Self::disabled_with_requirement(required)
            }
        }
    }
}

#[async_trait]
impl AlertSink for AlertPublisher {
    fn enabled(&self) -> bool {
        self.enabled
    }

    fn required(&self) -> bool {
        self.publisher.required()
    }

    async fn publish_alert(&self, event: &AlertEvent) -> anyhow::Result<()> {
        // `NatsPublisher::publish_alert` awaits the JetStream ACK, returns Err
        // when NATS is required but unavailable, and degrades to a no-op
        // otherwise.
        self.publisher.publish_alert(event).await
    }
}

/// The one shared warning ingress.
///
/// Generic over its three collaborators so tests can inject fakes; the worker
/// binary uses the default production types.
pub struct IntelligenceIngress<W = WarningService, Q = TriageQueue, A = AlertPublisher>
where
    W: WarningWriter,
    Q: IngestQueue,
    A: AlertSink,
{
    warnings: W,
    triage: TriageIngestor<Q>,
    alerts: A,
}

impl<W, Q, A> IntelligenceIngress<W, Q, A>
where
    W: WarningWriter,
    Q: IngestQueue,
    A: AlertSink,
{
    pub fn new(warnings: W, triage: TriageIngestor<Q>, alerts: A) -> Self {
        Self {
            warnings,
            triage,
            alerts,
        }
    }

    pub fn triage(&self) -> &TriageIngestor<Q> {
        &self.triage
    }

    /// Submit one warning through every stage of the pipeline.
    ///
    /// A failed warning upsert is the only hard error: nothing was persisted,
    /// so the caller must not count the warning as generated. Everything after
    /// persistence is best-effort and surfaced in the result as a degraded
    /// outcome rather than an error, because the warning row already exists.
    pub async fn submit_warning(
        &self,
        warning: NewWarning,
    ) -> anyhow::Result<WarningSubmissionResult> {
        let stored = self.warnings.upsert_warning(&warning).await?;

        let submission = TriageSubmission {
            item_type: TriageItemType::Warning,
            // Deterministic producer id: the warning row id. Repeated
            // submissions of the same warning merge by source id.
            source_id: stored.id.to_string(),
            title: stored.title.clone(),
            description: stored.description.clone().unwrap_or_default(),
            entity_id: stored.entity_ids.first().copied(),
            entity_name: None,
            static_severity: Some(stored.severity.clone()),
            dimensions: None,
            observation_ids: Vec::new(),
            source_urls: stored.source_urls.clone(),
        };
        let triage = match self.triage.submit(submission).await {
            Ok(outcome) => TriageSubmissionOutcome::from_ingest(outcome),
            Err(error) => {
                tracing::warn!(
                    warning_id = %stored.id,
                    error = %error,
                    "intelligence_ingress: triage submission failed; warning persisted without semantic dedup"
                );
                TriageSubmissionOutcome::Failed {
                    error: error.to_string(),
                }
            }
        };

        let (alert_published, alert_error) = match stored.alert.as_ref() {
            Some(event) => match self.alerts.publish_alert(event).await {
                Ok(()) if !self.alerts.enabled() => {
                    // NATS is optional and unavailable: the outbox event stays
                    // unpublished for the drain publisher to retry. Not a
                    // failure in a non-required deployment.
                    (false, None)
                }
                Ok(()) => {
                    let marked = match stored.outbox_id {
                        Some(outbox_id) => self.warnings.mark_alert_published(outbox_id).await,
                        None => Ok(()),
                    };
                    match marked {
                        Ok(()) => (true, None),
                        Err(error) => {
                            // The alert reached NATS, but the outbox marker did
                            // not land: the drain may re-publish it (duplicate),
                            // so the submission is reported as degraded.
                            tracing::warn!(
                                warning_id = %stored.id,
                                error = %error,
                                "intelligence_ingress: alert published but outbox marking failed"
                            );
                            (true, Some(format!("alert outbox marking failed: {error}")))
                        }
                    }
                }
                Err(error) => {
                    tracing::warn!(
                        warning_id = %stored.id,
                        error = %error,
                        "intelligence_ingress: alert publication failed"
                    );
                    (false, Some(error.to_string()))
                }
            },
            None => (false, None),
        };

        let (activity_recorded, activity_error) = match self
            .warnings
            .record_warning_activity(&stored, &triage)
            .await
        {
            Ok(()) => (true, None),
            Err(error) => {
                tracing::warn!(
                    warning_id = %stored.id,
                    error = %error,
                    "intelligence_ingress: activity record failed"
                );
                (false, Some(error.to_string()))
            }
        };

        Ok(WarningSubmissionResult {
            warning: stored,
            triage,
            alert_published,
            alert_error,
            activity_recorded,
            activity_error,
        })
    }
}

#[async_trait]
impl<W, Q, A> WarningSubmitter for IntelligenceIngress<W, Q, A>
where
    W: WarningWriter,
    Q: IngestQueue,
    A: AlertSink,
{
    async fn submit_warning(&self, warning: NewWarning) -> anyhow::Result<WarningSubmissionResult> {
        IntelligenceIngress::submit_warning(self, warning).await
    }
}

/// Construct the single production ingress at worker startup.
///
/// The triage ingestor is built once here (never per job). Alert publication is
/// enabled only when `NATS_URL` is configured and reachable within the startup
/// timeout; otherwise the publisher degrades to a disabled sink and every
/// `submit_warning` reports `alert_published = false`.
pub async fn build(store: Arc<PgStore>) -> IntelligenceIngress {
    let triage = TriageIngestor::new(
        TriageQueue::new(store.pool.clone()),
        SemanticDedup::with_in_memory_fallback(),
    );

    let alerts = match std::env::var("NATS_URL") {
        Ok(url) if !url.trim().is_empty() => {
            let publisher = AlertPublisher::connect(url.trim()).await;
            if publisher.enabled {
                tracing::info!(nats_url = %url, "intelligence_ingress: alert publisher connected");
            } else {
                tracing::warn!(
                    nats_url = %url,
                    "intelligence_ingress: NATS unavailable; alerts will be reported as unpublished"
                );
            }
            publisher
        }
        _ => {
            let required = nats_required_from_env();
            if required {
                tracing::error!(
                    "intelligence_ingress: NATS_URL unset but NATS is required \
                     (REQUIRE_NATS/APEX_ENV=production); warnings will be reported degraded"
                );
            } else {
                tracing::info!("intelligence_ingress: NATS_URL unset; alert publishing disabled");
            }
            AlertPublisher::disabled_with_requirement(required)
        }
    };

    IntelligenceIngress::new(WarningService::new(store), triage, alerts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use apex_triage::semantic_dedup::IngestQueueItem;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Mutex;

    /// In-memory [`IngestQueue`] mirroring the SQL semantics the ingress relies
    /// on: exact id dedup, merge increments occurrence count.
    #[derive(Default)]
    struct InMemoryQueue {
        items: Mutex<Vec<IngestQueueItem>>,
    }

    #[async_trait]
    impl IngestQueue for InMemoryQueue {
        async fn find_by_source_id(
            &self,
            item_type: &TriageItemType,
            source_id: &str,
        ) -> anyhow::Result<Option<IngestQueueItem>> {
            let items = self.items.lock().unwrap();
            Ok(items
                .iter()
                .find(|item| &item.item_type == item_type && item.source_id == source_id)
                .cloned())
        }

        async fn find_by_id(&self, id: Uuid) -> anyhow::Result<Option<IngestQueueItem>> {
            let items = self.items.lock().unwrap();
            Ok(items.iter().find(|item| item.id == id).cloned())
        }

        async fn find_recent(
            &self,
            item_type: &TriageItemType,
            _window: chrono::Duration,
        ) -> anyhow::Result<Vec<IngestQueueItem>> {
            let items = self.items.lock().unwrap();
            Ok(items
                .iter()
                .filter(|item| &item.item_type == item_type)
                .cloned()
                .collect())
        }

        async fn insert_submission(
            &self,
            submission: &TriageSubmission,
            high_at: i64,
            critical_at: i64,
        ) -> anyhow::Result<IngestQueueItem> {
            let mut items = self.items.lock().unwrap();
            if let Some(existing) = items.iter_mut().find(|item| {
                item.item_type == submission.item_type && item.source_id == submission.source_id
            }) {
                existing.occurrence_count += 1;
                existing.last_seen_at = Some(Utc::now());
                return Ok(existing.clone());
            }
            let item = IngestQueueItem {
                id: Uuid::new_v4(),
                item_type: submission.item_type.clone(),
                source_id: submission.source_id.clone(),
                title: submission.title.clone(),
                description: submission.description.clone(),
                entity_id: submission.entity_id,
                entity_name: submission.entity_name.clone(),
                static_severity: apex_triage::semantic_dedup::escalate_severity(
                    None,
                    submission.static_severity.as_deref(),
                    1,
                    high_at,
                    critical_at,
                ),
                occurrence_count: 1,
                last_seen_at: Some(Utc::now()),
                merged_observation_ids: submission.observation_ids.clone(),
                merged_source_urls: submission.source_urls.clone(),
                created_at: Utc::now(),
            };
            items.push(item.clone());
            Ok(item)
        }

        async fn merge_submission(
            &self,
            target_id: Uuid,
            submission: &TriageSubmission,
            high_at: i64,
            critical_at: i64,
        ) -> anyhow::Result<IngestQueueItem> {
            let mut items = self.items.lock().unwrap();
            let item = items
                .iter_mut()
                .find(|item| item.id == target_id)
                .ok_or_else(|| anyhow::anyhow!("missing target {target_id}"))?;
            item.occurrence_count += 1;
            item.last_seen_at = Some(Utc::now());
            item.static_severity = apex_triage::semantic_dedup::escalate_severity(
                item.static_severity.as_deref(),
                submission.static_severity.as_deref(),
                item.occurrence_count,
                high_at,
                critical_at,
            );
            for id in &submission.observation_ids {
                if !item.merged_observation_ids.contains(id) {
                    item.merged_observation_ids.push(*id);
                }
            }
            for url in &submission.source_urls {
                if !item.merged_source_urls.contains(url) {
                    item.merged_source_urls.push(url.clone());
                }
            }
            Ok(item.clone())
        }
    }

    /// Deterministic warning store fake: same `(type,title,severity)` returns
    /// the same id with `created = false` on repeat, like the SQL dedup path.
    /// It also emulates the transactional outbox: an enabled warning gets an
    /// outbox id and its alert event, which `mark_alert_published` can stamp.
    #[derive(Default)]
    struct FakeWarningWriter {
        rows: Mutex<HashMap<String, Uuid>>,
        upsert_attempts: AtomicUsize,
        activity_writes: AtomicUsize,
        published_marks: AtomicUsize,
        fail: AtomicBool,
        fail_mark: AtomicBool,
    }

    #[async_trait]
    impl WarningWriter for FakeWarningWriter {
        async fn upsert_warning(&self, warning: &NewWarning) -> anyhow::Result<StoredWarning> {
            self.upsert_attempts.fetch_add(1, Ordering::SeqCst);
            if self.fail.load(Ordering::SeqCst) {
                anyhow::bail!("simulated warning insert failure");
            }
            let key = format!(
                "{}|{}|{}",
                warning.warning_type, warning.title, warning.severity
            );
            let mut rows = self.rows.lock().unwrap();
            let (id, created) = match rows.get(&key) {
                Some(id) => (*id, false),
                None => {
                    let id = Uuid::new_v4();
                    rows.insert(key, id);
                    (id, true)
                }
            };
            let occurred_at = Utc::now();
            let (outbox_id, alert) = if warning.alerts_enabled() {
                (
                    Some(Uuid::new_v4()),
                    Some(build_alert_event(id, warning, occurred_at)),
                )
            } else {
                (None, None)
            };
            Ok(StoredWarning {
                id,
                created,
                warning_type: warning.warning_type.clone(),
                title: warning.title.clone(),
                description: warning.description.clone(),
                severity: warning.severity.clone(),
                region: warning.region.clone(),
                recipe_code: warning.recipe_code.clone(),
                entity_ids: warning.entity_ids.clone(),
                source_urls: warning.source_urls.clone(),
                confidence: warning.confidence,
                occurred_at,
                outbox_id,
                alert,
            })
        }

        async fn record_warning_activity(
            &self,
            _warning: &StoredWarning,
            _triage: &TriageSubmissionOutcome,
        ) -> anyhow::Result<()> {
            self.activity_writes.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn mark_alert_published(&self, _outbox_id: Uuid) -> anyhow::Result<()> {
            if self.fail_mark.load(Ordering::SeqCst) {
                anyhow::bail!("simulated outbox marking failure");
            }
            self.published_marks.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[derive(Default)]
    struct CountingAlertSink {
        published: AtomicUsize,
        enabled: bool,
        required: bool,
    }

    #[async_trait]
    impl AlertSink for CountingAlertSink {
        fn enabled(&self) -> bool {
            self.enabled
        }

        fn required(&self) -> bool {
            self.required
        }

        async fn publish_alert(&self, event: &AlertEvent) -> anyhow::Result<()> {
            if self.required && !self.enabled {
                anyhow::bail!("simulated required NATS outage for alert {}", event.id);
            }
            if !self.enabled {
                return Ok(());
            }
            self.published.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    fn test_ingress(
        writer: FakeWarningWriter,
        alerts: CountingAlertSink,
    ) -> IntelligenceIngress<FakeWarningWriter, InMemoryQueue, CountingAlertSink> {
        let triage = TriageIngestor::new(
            InMemoryQueue::default(),
            SemanticDedup::with_in_memory_fallback(),
        );
        IntelligenceIngress::new(writer, triage, alerts)
    }

    fn sample_warning() -> NewWarning {
        NewWarning::new("volume_anomaly", "Volume spike for Acme", "medium")
            .description("Acme shows a 60% spike in observation volume.")
            .entity_ids(vec![Uuid::new_v4()])
            .confidence(0.75)
    }

    #[tokio::test]
    async fn submit_warning_dedups_and_merges_with_incrementing_occurrence_count() {
        let ingress = test_ingress(
            FakeWarningWriter::default(),
            CountingAlertSink {
                enabled: true,
                ..CountingAlertSink::default()
            },
        );

        let first = ingress.submit_warning(sample_warning()).await.unwrap();
        assert!(first.created());
        assert!(!first.triage_merged());
        assert_eq!(first.occurrence_count(), 1);

        let second = ingress.submit_warning(sample_warning()).await.unwrap();
        assert!(!second.created(), "repeat must dedup into the existing row");
        assert_eq!(second.warning_id(), first.warning_id());
        assert!(
            second.triage_merged(),
            "repeat must merge in semantic triage"
        );
        assert_eq!(second.occurrence_count(), 2);
        assert_eq!(second.triage.item_id(), first.triage.item_id());
        assert!(second.alert_published);
        assert!(second.activity_recorded);
        assert!(!second.degraded());
    }

    #[tokio::test]
    async fn failing_warning_insert_returns_err_and_skips_downstream_counters() {
        let writer = FakeWarningWriter::default();
        writer.fail.store(true, Ordering::SeqCst);
        let ingress = test_ingress(
            writer,
            CountingAlertSink {
                enabled: true,
                ..CountingAlertSink::default()
            },
        );

        let result = ingress.submit_warning(sample_warning()).await;
        assert!(
            result.is_err(),
            "a failed warning insert must surface as Err"
        );
        assert_eq!(
            ingress.warnings.upsert_attempts.load(Ordering::SeqCst),
            1,
            "the failing insert must be attempted exactly once"
        );
        assert_eq!(
            ingress.alerts.published.load(Ordering::SeqCst),
            0,
            "no alert may be published for an unpersisted warning"
        );
        assert_eq!(
            ingress.warnings.activity_writes.load(Ordering::SeqCst),
            0,
            "no activity may be written for an unpersisted warning"
        );
    }

    #[tokio::test]
    async fn triage_failure_is_reported_as_degraded_but_warning_is_kept() {
        struct FailingQueue;

        #[async_trait]
        impl IngestQueue for FailingQueue {
            async fn find_by_source_id(
                &self,
                _item_type: &TriageItemType,
                _source_id: &str,
            ) -> anyhow::Result<Option<IngestQueueItem>> {
                anyhow::bail!("simulated triage outage")
            }

            async fn find_by_id(&self, _id: Uuid) -> anyhow::Result<Option<IngestQueueItem>> {
                anyhow::bail!("simulated triage outage")
            }

            async fn find_recent(
                &self,
                _item_type: &TriageItemType,
                _window: chrono::Duration,
            ) -> anyhow::Result<Vec<IngestQueueItem>> {
                anyhow::bail!("simulated triage outage")
            }

            async fn insert_submission(
                &self,
                _submission: &TriageSubmission,
                _high_at: i64,
                _critical_at: i64,
            ) -> anyhow::Result<IngestQueueItem> {
                anyhow::bail!("simulated triage outage")
            }

            async fn merge_submission(
                &self,
                _target_id: Uuid,
                _submission: &TriageSubmission,
                _high_at: i64,
                _critical_at: i64,
            ) -> anyhow::Result<IngestQueueItem> {
                anyhow::bail!("simulated triage outage")
            }
        }

        let ingress = IntelligenceIngress::new(
            FakeWarningWriter::default(),
            TriageIngestor::new(FailingQueue, SemanticDedup::with_in_memory_fallback()),
            CountingAlertSink {
                enabled: true,
                ..CountingAlertSink::default()
            },
        );

        let result = ingress.submit_warning(sample_warning()).await.unwrap();
        assert!(matches!(
            result.triage,
            TriageSubmissionOutcome::Failed { .. }
        ));
        assert!(result.degraded());
        assert_eq!(result.occurrence_count(), 0);
        assert!(
            result.alert_published,
            "alert still goes out for persisted warning"
        );
    }

    #[tokio::test]
    async fn disabled_alert_sink_reports_unpublished() {
        let ingress = test_ingress(
            FakeWarningWriter::default(),
            CountingAlertSink {
                enabled: false,
                ..CountingAlertSink::default()
            },
        );

        let result = ingress.submit_warning(sample_warning()).await.unwrap();
        assert!(!result.alert_published);
        assert!(
            !result.degraded(),
            "an optional disabled sink is queued, not a failure"
        );
    }

    #[tokio::test]
    async fn required_alert_sink_reports_degraded_when_unavailable() {
        let ingress = test_ingress(
            FakeWarningWriter::default(),
            CountingAlertSink {
                enabled: false,
                required: true,
                ..CountingAlertSink::default()
            },
        );

        let result = ingress.submit_warning(sample_warning()).await.unwrap();
        assert!(!result.alert_published);
        assert!(
            result.degraded(),
            "a required but unavailable alert sink must degrade the submission"
        );
        assert!(result.alert_error.is_some());
    }

    #[tokio::test]
    async fn alert_outbox_is_marked_published_only_after_delivery() {
        let ingress = test_ingress(
            FakeWarningWriter::default(),
            CountingAlertSink {
                enabled: true,
                ..CountingAlertSink::default()
            },
        );

        let result = ingress.submit_warning(sample_warning()).await.unwrap();
        assert!(result.alert_published);
        assert_eq!(
            ingress.warnings.published_marks.load(Ordering::SeqCst),
            1,
            "the outbox event must be stamped after the publish"
        );
    }

    #[tokio::test]
    async fn outbox_marking_failure_degrades_but_does_not_lose_the_warning() {
        let writer = FakeWarningWriter::default();
        writer.fail_mark.store(true, Ordering::SeqCst);
        let ingress = test_ingress(
            writer,
            CountingAlertSink {
                enabled: true,
                ..CountingAlertSink::default()
            },
        );

        let result = ingress.submit_warning(sample_warning()).await.unwrap();
        assert!(
            result.alert_published,
            "the alert reached the broker even though the marker failed"
        );
        assert!(
            result.degraded(),
            "an unstamped outbox event risks a duplicate publish and is degraded"
        );
    }

    #[test]
    fn generic_entity_warning_is_targeted_never_broadcast() {
        let warning =
            NewWarning::new("volume_anomaly", "Spike", "medium").entity_ids(vec![Uuid::new_v4()]);
        assert!(warning.alerts_enabled());
        assert!(!warning.is_system_broadcast());
        assert_eq!(warning.audience(), AlertAudience::Users(Vec::new()));

        let event = build_alert_event(Uuid::new_v4(), &warning, Utc::now());
        assert_eq!(
            event.audience,
            AlertAudience::Users(Vec::new()),
            "entity warnings must resolve real subscribers, never broadcast"
        );
    }

    #[test]
    fn system_broadcast_warning_is_the_only_broadcast_source() {
        let warning =
            NewWarning::new("source_outage", "All sources down", "critical").system_broadcast();
        assert!(warning.is_system_broadcast());
        assert_eq!(warning.audience(), AlertAudience::Broadcast);

        let event = build_alert_event(Uuid::new_v4(), &warning, Utc::now());
        assert_eq!(event.audience, AlertAudience::Broadcast);
    }

    #[test]
    fn alerts_can_be_disabled_without_disabling_persistence() {
        let warning = NewWarning::new("hygiene", "Noise", "low").enabled(false);
        assert!(!warning.alerts_enabled());
        assert_eq!(warning.audience(), AlertAudience::Users(Vec::new()));
    }

    #[tokio::test]
    async fn alerts_disabled_warning_writes_no_outbox_event() {
        let ingress = test_ingress(
            FakeWarningWriter::default(),
            CountingAlertSink {
                enabled: true,
                ..CountingAlertSink::default()
            },
        );

        let result = ingress
            .submit_warning(sample_warning().enabled(false))
            .await
            .unwrap();
        assert!(result.warning.outbox_id.is_none());
        assert!(result.warning.alert.is_none());
        assert!(!result.alert_published);
        assert!(!result.degraded());
        assert_eq!(ingress.alerts.published.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn counters_distinguish_all_stages() {
        let ingress = test_ingress(
            FakeWarningWriter::default(),
            CountingAlertSink {
                enabled: true,
                ..CountingAlertSink::default()
            },
        );
        let mut counters = IngressCounters::default();

        counters.record(&ingress.submit_warning(sample_warning()).await.unwrap());
        assert_eq!(counters.warnings_persisted, 1);
        assert_eq!(counters.triage_completed, 1);
        assert_eq!(counters.alerts_published, 1);
        assert_eq!(counters.activities_recorded, 1);
        assert_eq!(counters.degraded_warnings, 0);
        assert!(counters.success_blocker().is_none());
        assert!(counters.summary().contains("triage_completed=1"));
    }

    #[test]
    fn counters_block_success_when_persisted_without_triage() {
        let counters = IngressCounters {
            warnings_persisted: 7,
            triage_completed: 0,
            alerts_published: 7,
            activities_recorded: 7,
            degraded_warnings: 7,
        };
        let blocker = counters
            .success_blocker()
            .expect("persisted warnings with zero triage must block success");
        assert!(blocker.contains("7 warning(s) persisted"));

        let healthy = IngressCounters {
            warnings_persisted: 7,
            triage_completed: 7,
            ..IngressCounters::default()
        };
        assert!(healthy.success_blocker().is_none());

        let empty = IngressCounters::default();
        assert!(empty.success_blocker().is_none());
    }
}
