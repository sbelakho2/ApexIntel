//! Dark web forum scan job execution.
//!
//! Periodically scans configured dark web forums, paste sites, and ransomware
//! blogs for mentions of monitored entities.  Matches are stored as
//! `Observation` records (type `DarkWebPost`) and high-relevance matches
//! trigger security warnings.
//!
//! # Configuration contract
//!
//! There is no DB-backed dark web configuration table/recipe yet, so the job
//! reads its configuration from the environment (documented in `.env.example`):
//!
//! * `DARKWEB_TOR_PROXY` — optional Tor SOCKS5 proxy URL. When unset the
//!   monitor runs clearnet-only.
//! * `DARKWEB_FORUMS` — optional JSON array of forums. When set, it replaces
//!   the built-in default forum list.
//! * `DARKWEB_MONITORING_RULES` — optional JSON array of monitoring rules.
//!   When set, it replaces the default keyword set used for matching.
//!
//! Unset variables fall back to the built-in defaults. A variable that is set
//! but malformed fails the job: silently ignoring operator configuration is
//! precisely the failure mode where configured forums/rules never took effect.

use std::sync::Arc;

use apex_crawl::dark_web::{DarkWebForum, DarkWebMonitor, DarkWebPost, MonitoringRule, ScanReport};

use crate::intelligence_ingress::{
    IngressCounters, IntelligenceIngress, NewWarning, WarningSubmitter,
};
use crate::*;

/// Relevance score at or above which a match generates a security warning.
const WARNING_RELEVANCE_THRESHOLD: f64 = 0.7;

/// Parse a JSON array, attributing any parse error to the source variable.
fn parse_json_array<T: serde::de::DeserializeOwned>(
    variable: &str,
    raw: &str,
) -> anyhow::Result<Vec<T>> {
    let parsed: Vec<T> = serde_json::from_str(raw)
        .map_err(|e| anyhow::anyhow!("{variable}: invalid JSON array: {e}"))?;
    tracing::info!(
        variable,
        count = parsed.len(),
        "dark_web_scan: loaded configuration"
    );
    Ok(parsed)
}

/// Load an optional JSON-array configuration from the environment.
///
/// Returns `Ok(None)` when the variable is unset or blank, and an error when it
/// is set but cannot be parsed so that misconfiguration is never silently
/// replaced with defaults.
fn load_env_json_array<T: serde::de::DeserializeOwned>(
    variable: &str,
) -> anyhow::Result<Option<Vec<T>>> {
    match std::env::var(variable) {
        Ok(raw) if !raw.trim().is_empty() => parse_json_array(variable, &raw).map(Some),
        _ => Ok(None),
    }
}

/// Apply configured forums/rules to the monitor.
///
/// `None` leaves the built-in default in place; `Some` replaces it.
fn apply_monitor_config(
    monitor: &mut DarkWebMonitor,
    forums: Option<Vec<DarkWebForum>>,
    rules: Option<Vec<MonitoringRule>>,
) {
    if let Some(forums) = forums {
        tracing::info!(
            count = forums.len(),
            "dark_web_scan: applying configured forums"
        );
        monitor.set_forums(forums);
    }
    if let Some(rules) = rules {
        tracing::info!(
            count = rules.len(),
            "dark_web_scan: applying configured monitoring rules"
        );
        monitor.set_rules(rules);
    }
}

/// Real persistence counters for one dark web scan.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct DarkWebCounters {
    /// Matching posts returned by the monitor.
    posts_seen: u64,
    /// Observation rows newly inserted.
    observations_inserted: u64,
    /// Observation inserts that failed.
    observation_insert_errors: u64,
    /// Warning rows successfully inserted.
    warnings_inserted: u64,
    /// Warning inserts that failed.
    warning_insert_errors: u64,
    /// Per-stage ingress outcome counters (persisted / triage / alert / activity).
    ingress_counters: IngressCounters,
}

impl DarkWebCounters {
    /// Total persistence failures (observations + warnings).
    fn persistence_errors(self) -> u64 {
        self.observation_insert_errors + self.warning_insert_errors
    }
}

/// Persistence operations required by the dark web scan.
///
/// Implemented for [`PgStore`] in production; tests inject a failing mock so
/// persistence-error handling is exercised without a database. Warnings do not
/// go through this trait: they are submitted through the shared
/// [`WarningSubmitter`] ingress so they get semantic triage dedup too.
trait DarkWebPersistence {
    /// Store one observation. Returns `true` when a new row was created and
    /// `false` when the deterministic id already existed.
    async fn insert_observation(&self, post: &DarkWebPost) -> anyhow::Result<bool>;
}

fn warning_title(post: &DarkWebPost) -> String {
    format!(
        "Dark web mention: {} — {}",
        post.forum_name, post.thread_title
    )
}

fn warning_description(post: &DarkWebPost) -> String {
    format!(
        "High-relevance dark web post detected on '{}' (score: {:.2}). \
         Author: {}. Matched keywords: {}. Entities: {}. \
         Snippet: {}",
        post.forum_name,
        post.relevance_score,
        post.author,
        post.matched_keywords.join(", "),
        post.entities_mentioned.join(", "),
        post.content_snippet,
    )
}

fn warning_severity(post: &DarkWebPost) -> &'static str {
    if post.relevance_score >= 0.9 {
        "critical"
    } else {
        "high"
    }
}

impl DarkWebPersistence for PgStore {
    async fn insert_observation(&self, post: &DarkWebPost) -> anyhow::Result<bool> {
        let value = serde_json::json!({
            "forum_name": post.forum_name,
            "thread_title": post.thread_title,
            "author": post.author,
            "content_snippet": post.content_snippet,
            "posted_at": post.posted_at.to_rfc3339(),
            "url": post.url,
            "matched_keywords": post.matched_keywords,
            "relevance_score": post.relevance_score,
            "entities_mentioned": post.entities_mentioned,
        });

        let provenance = serde_json::json!({
            "source": "worker_dark_web_scan",
            "forum": post.forum_name,
            "post_id": post.id,
            "content_hash": format!("dw_{}", post.id),
        });

        // B326: deterministic ID from (forum, post id, thread title) — the
        // generic-scrape path generates unstable post ids, so include the
        // content key too. Every 6h scan previously re-inserted the same posts
        // as new rows.
        let obs_id = apex_core::entities::Observation::deterministic_id(
            "darkweb",
            &format!("{}|{}|{}", post.forum_name, post.id, post.thread_title),
        );

        let result = sqlx::query(
            r#"INSERT INTO observations
               (id, observation_type, entity_id, entity_type, ts_utc, value, provenance, confidence)
               VALUES ($1, 'DarkWebPost', NULL, NULL, $2, $3::jsonb, $4::jsonb, $5)
               ON CONFLICT (id) DO NOTHING"#,
        )
        .bind(obs_id)
        .bind(post.posted_at)
        .bind(&value)
        .bind(&provenance)
        .bind(post.relevance_score)
        .execute(&self.pool)
        .await
        .map_err(|e| anyhow::anyhow!("observation insert failed for post {}: {e}", post.id))?;

        Ok(result.rows_affected() > 0)
    }
}

/// Build the warning submitted for a high-relevance dark web post.
fn dark_web_warning(post: &DarkWebPost) -> NewWarning {
    NewWarning::new("dark_web", warning_title(post), warning_severity(post))
        .description(warning_description(post))
        .source_urls(vec![post.url.clone()])
        .confidence(post.relevance_score)
}

/// Persist matching posts and tally real outcomes.
///
/// `warnings_inserted` only advances after a successful ingress submission;
/// failed submissions are counted in `warning_insert_errors` so callers can
/// report a degraded run instead of full success.
async fn persist_dark_web_posts<P, I>(
    persistence: &P,
    ingress: &I,
    posts: &[DarkWebPost],
) -> DarkWebCounters
where
    P: DarkWebPersistence + ?Sized,
    I: WarningSubmitter + ?Sized,
{
    let mut counters = DarkWebCounters {
        posts_seen: posts.len() as u64,
        ..DarkWebCounters::default()
    };

    for post in posts {
        // B327: only warn on first store — the deduped re-scan previously
        // re-warned for the same post every 6h.
        let newly_stored = match persistence.insert_observation(post).await {
            Ok(true) => {
                counters.observations_inserted += 1;
                true
            }
            Ok(false) => false,
            Err(e) => {
                counters.observation_insert_errors += 1;
                tracing::warn!(
                    post_id = %post.id,
                    forum = %post.forum_name,
                    error = %e,
                    "dark_web_scan: failed to store observation"
                );
                false
            }
        };

        if !newly_stored || post.relevance_score < WARNING_RELEVANCE_THRESHOLD {
            continue;
        }

        match ingress.submit_warning(dark_web_warning(post)).await {
            Ok(result) => {
                counters.warnings_inserted += 1;
                counters.ingress_counters.record(&result);
            }
            Err(e) => {
                counters.warning_insert_errors += 1;
                tracing::warn!(
                    post_id = %post.id,
                    forum = %post.forum_name,
                    error = %e,
                    "dark_web_scan: failed to store warning"
                );
            }
        }
    }

    counters
}

/// Human-readable summary of a scan including every real counter.
fn scan_summary(report: &ScanReport, counters: DarkWebCounters) -> String {
    format!(
        "dark_web_scan: scanned {}/{} active forums ({} failed), {} posts seen, \
         {} observations inserted ({} errors), {} warnings inserted ({} errors); {}",
        report.forums_scanned,
        report.forums_scanned + report.forums_failed,
        report.forums_failed,
        counters.posts_seen,
        counters.observations_inserted,
        counters.observation_insert_errors,
        counters.warnings_inserted,
        counters.warning_insert_errors,
        counters.ingress_counters.summary(),
    )
}

/// Turn real counters into the terminal job result.
///
/// A run with persistence failures is reported as failed (degraded) with the
/// error counts — never as a plain success.
fn complete_dark_web_scan(run: &mut JobRun, report: &ScanReport, counters: DarkWebCounters) {
    let summary = scan_summary(report, counters);
    tracing::info!(
        posts_seen = counters.posts_seen,
        observations_inserted = counters.observations_inserted,
        observation_insert_errors = counters.observation_insert_errors,
        warnings_inserted = counters.warnings_inserted,
        warning_insert_errors = counters.warning_insert_errors,
        forums_scanned = report.forums_scanned,
        forums_failed = report.forums_failed,
        "dark_web_scan: scan complete"
    );

    if counters.persistence_errors() > 0 {
        run.items_processed = counters.observations_inserted;
        run.fail(&format!(
            "{summary} — persistence degraded: {} observation insert error(s), \
             {} warning insert error(s)",
            counters.observation_insert_errors, counters.warning_insert_errors
        ));
    } else if let Some(reason) = counters.ingress_counters.success_blocker() {
        run.degrade(
            counters.observations_inserted,
            &format!("{summary} — {reason}"),
        );
    } else {
        run.succeed(counters.observations_inserted, &summary);
    }
}

/// Run a dark web forum scan cycle.
///
/// 1. Build a [`DarkWebMonitor`] (optionally with Tor SOCKS5 proxy).
/// 2. Apply configured forums/rules from the environment.
/// 3. Scan all active forums.
/// 4. Store matching posts as observations.
/// 5. Generate warnings for high-relevance matches.
/// 6. Report real counters; fail the run when persistence is incomplete.
pub(super) async fn run_dark_web_scan(
    kind: &JobKind,
    store: &Arc<PgStore>,
    ingress: &Arc<IntelligenceIngress>,
) -> JobRun {
    let mut run = JobRun::new(kind.clone());
    run.start();

    // Load Tor proxy config from environment (optional)
    let tor_proxy_url = std::env::var("DARKWEB_TOR_PROXY").ok();
    let mut monitor = match DarkWebMonitor::new(tor_proxy_url) {
        Ok(m) => m,
        Err(e) => {
            run.fail(&format!("dark_web_scan: failed to build monitor: {e}"));
            return run;
        }
    };

    let forums = match load_env_json_array::<DarkWebForum>("DARKWEB_FORUMS") {
        Ok(forums) => forums,
        Err(e) => {
            run.fail(&format!("dark_web_scan: {e}"));
            return run;
        }
    };
    let rules = match load_env_json_array::<MonitoringRule>("DARKWEB_MONITORING_RULES") {
        Ok(rules) => rules,
        Err(e) => {
            run.fail(&format!("dark_web_scan: {e}"));
            return run;
        }
    };
    apply_monitor_config(&mut monitor, forums, rules);

    let report = monitor.scan_all_detailed().await;
    let counters = persist_dark_web_posts(store.as_ref(), ingress.as_ref(), &report.posts).await;
    complete_dark_web_scan(&mut run, &report, counters);
    run
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_post(id: &str, relevance_score: f64) -> DarkWebPost {
        DarkWebPost {
            id: id.into(),
            forum_name: "BreachForums".into(),
            thread_title: "Acme Corp database leak".into(),
            author: "anon".into(),
            content_snippet: "selling a full database dump of acme corp".into(),
            posted_at: chrono::Utc::now(),
            url: format!("https://breachforums.st/thread/{id}"),
            matched_keywords: vec!["database".into(), "dump".into()],
            relevance_score,
            entities_mentioned: vec!["dump@example.com".into()],
        }
    }

    fn test_report(posts: Vec<DarkWebPost>, forums_scanned: u64, forums_failed: u64) -> ScanReport {
        ScanReport {
            posts,
            forums_scanned,
            forums_failed,
        }
    }

    #[derive(Debug, Default)]
    struct MockPersistence {
        fail_observations: bool,
    }

    impl DarkWebPersistence for MockPersistence {
        async fn insert_observation(&self, _post: &DarkWebPost) -> anyhow::Result<bool> {
            if self.fail_observations {
                anyhow::bail!("simulated observation insert failure");
            }
            Ok(true)
        }
    }

    /// Injected warning ingress: fails or succeeds without a database.
    #[derive(Debug, Default)]
    struct MockWarningIngress {
        fail: bool,
    }

    fn fake_submission() -> crate::intelligence_ingress::WarningSubmissionResult {
        use crate::intelligence_ingress::{StoredWarning, TriageSubmissionOutcome};
        crate::intelligence_ingress::WarningSubmissionResult {
            warning: StoredWarning {
                id: Uuid::new_v4(),
                created: true,
                warning_type: "dark_web".to_string(),
                title: "test".to_string(),
                description: None,
                severity: "high".to_string(),
                region: None,
                recipe_code: None,
                entity_ids: Vec::new(),
                source_urls: Vec::new(),
                confidence: None,
                occurred_at: chrono::Utc::now(),
                outbox_id: None,
                alert: None,
            },
            triage: TriageSubmissionOutcome::Enqueued {
                item_id: Uuid::new_v4(),
                occurrence_count: 1,
            },
            alert_published: false,
            alert_error: None,
            activity_recorded: true,
            activity_error: None,
        }
    }

    #[async_trait::async_trait]
    impl WarningSubmitter for MockWarningIngress {
        async fn submit_warning(
            &self,
            _warning: NewWarning,
        ) -> anyhow::Result<crate::intelligence_ingress::WarningSubmissionResult> {
            if self.fail {
                anyhow::bail!("simulated warning insert failure");
            }
            Ok(fake_submission())
        }
    }

    fn custom_forum(name: &str) -> DarkWebForum {
        DarkWebForum {
            name: name.into(),
            base_url: "https://example.invalid/forum".into(),
            forum_type: apex_crawl::dark_web::ForumType::Leak,
            access_method: apex_crawl::dark_web::AccessMethod::Clearnet,
            is_active: true,
            last_checked: None,
            topics_of_interest: vec!["leaks".into()],
        }
    }

    fn custom_rule(id: &str, keyword: &str) -> MonitoringRule {
        MonitoringRule {
            id: id.into(),
            name: format!("rule {id}"),
            keywords: vec![keyword.into()],
            entity_ids: vec![],
            min_relevance: 0.5,
            notification_channels: vec![],
        }
    }

    // ── Configuration application ───────────────────────────────────────────

    #[test]
    fn configured_forums_and_rules_are_applied_to_monitor() {
        let mut monitor = DarkWebMonitor::new(None).unwrap();
        assert_eq!(
            monitor.forums.len(),
            apex_crawl::dark_web::default_forums().len(),
            "sanity: monitor starts with default forums"
        );

        apply_monitor_config(
            &mut monitor,
            Some(vec![custom_forum("Custom Forum")]),
            Some(vec![custom_rule("r1", "acme-inc")]),
        );

        assert_eq!(
            monitor.forums.len(),
            1,
            "configured forums must replace defaults"
        );
        assert_eq!(monitor.forums[0].name, "Custom Forum");
        assert_eq!(monitor.rules.len(), 1, "configured rules must be applied");
        assert_eq!(monitor.rules[0].id, "r1");
        assert_eq!(monitor.rules[0].keywords, vec!["acme-inc".to_string()]);
    }

    #[test]
    fn unset_configuration_keeps_monitor_defaults() {
        let mut monitor = DarkWebMonitor::new(None).unwrap();
        let default_forums = monitor.forums.len();

        apply_monitor_config(&mut monitor, None, None);

        assert_eq!(monitor.forums.len(), default_forums);
        assert!(monitor.rules.is_empty());
    }

    #[test]
    fn parsed_forum_json_is_applied_to_monitor() {
        let raw = r#"[{
            "name": "Parsed Forum",
            "base_url": "https://parsed.invalid",
            "forum_type": "General",
            "access_method": "Clearnet",
            "is_active": true,
            "last_checked": null,
            "topics_of_interest": []
        }]"#;
        let forums = parse_json_array::<DarkWebForum>("DARKWEB_FORUMS", raw).unwrap();
        let mut monitor = DarkWebMonitor::new(None).unwrap();
        apply_monitor_config(&mut monitor, Some(forums), None);
        assert_eq!(monitor.forums.len(), 1);
        assert_eq!(monitor.forums[0].name, "Parsed Forum");
    }

    #[test]
    fn malformed_configuration_json_is_rejected() {
        let err = parse_json_array::<DarkWebForum>("DARKWEB_FORUMS", "{not json}")
            .expect_err("malformed JSON must not parse");
        assert!(err.to_string().contains("DARKWEB_FORUMS"), "{err}");

        let err = parse_json_array::<MonitoringRule>("DARKWEB_MONITORING_RULES", "[] trailing")
            .expect_err("malformed JSON must not parse");
        assert!(
            err.to_string().contains("DARKWEB_MONITORING_RULES"),
            "{err}"
        );
    }

    // ── Persistence counters ────────────────────────────────────────────────

    #[tokio::test]
    async fn warning_insert_failure_counts_error_and_not_success() {
        let ingress = MockWarningIngress { fail: true };
        let counters = persist_dark_web_posts(
            &MockPersistence::default(),
            &ingress,
            &[test_post("post-1", 0.95)],
        )
        .await;

        assert_eq!(counters.posts_seen, 1);
        assert_eq!(counters.observations_inserted, 1);
        assert_eq!(
            counters.warnings_inserted, 0,
            "failed insert is not a warning"
        );
        assert_eq!(counters.warning_insert_errors, 1);
        assert_eq!(counters.persistence_errors(), 1);
    }

    #[tokio::test]
    async fn observation_insert_failure_counts_error_and_skips_warning() {
        let persistence = MockPersistence {
            fail_observations: true,
        };
        let ingress = MockWarningIngress::default();
        let counters =
            persist_dark_web_posts(&persistence, &ingress, &[test_post("post-1", 0.95)]).await;

        assert_eq!(counters.posts_seen, 1);
        assert_eq!(counters.observations_inserted, 0);
        assert_eq!(counters.observation_insert_errors, 1);
        assert_eq!(counters.warnings_inserted, 0);
        assert_eq!(counters.warning_insert_errors, 0);
    }

    #[tokio::test]
    async fn successful_persistence_counts_inserted_rows() {
        let persistence = MockPersistence::default();
        let ingress = MockWarningIngress::default();
        let posts = vec![test_post("post-1", 0.95), test_post("post-2", 0.5)];
        let counters = persist_dark_web_posts(&persistence, &ingress, &posts).await;

        assert_eq!(counters.posts_seen, 2);
        assert_eq!(counters.observations_inserted, 2);
        assert_eq!(
            counters.warnings_inserted, 1,
            "only the high-relevance post warns"
        );
        assert_eq!(counters.persistence_errors(), 0);
    }

    // ── Job result ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn job_fails_when_persistence_errors_occur() {
        let persistence = MockPersistence::default();
        let ingress = MockWarningIngress { fail: true };
        let report = test_report(vec![test_post("post-1", 0.95)], 1, 0);
        let counters = persist_dark_web_posts(&persistence, &ingress, &report.posts).await;

        let mut run = JobRun::new(JobKind::DarkWebScan);
        run.start();
        complete_dark_web_scan(&mut run, &report, counters);

        match &run.status {
            JobStatus::Failed { error, .. } => {
                assert!(
                    error.contains("1 warning insert error(s)"),
                    "failure must report warning insert errors: {error}"
                );
                assert!(
                    error.contains("persistence degraded"),
                    "failure must be flagged as degraded: {error}"
                );
            }
            other => panic!("persistence errors must fail the job, got {other:?}"),
        }
        assert_eq!(run.items_processed, 1, "stored observations still counted");
    }

    #[tokio::test]
    async fn job_is_degraded_when_warnings_persist_without_triage() {
        let report = test_report(vec![], 1, 0);
        let counters = DarkWebCounters {
            posts_seen: 1,
            observations_inserted: 1,
            warnings_inserted: 3,
            ingress_counters: IngressCounters {
                warnings_persisted: 3,
                triage_completed: 0,
                alerts_published: 0,
                activities_recorded: 3,
                degraded_warnings: 3,
            },
            ..DarkWebCounters::default()
        };

        let mut run = JobRun::new(JobKind::DarkWebScan);
        run.start();
        complete_dark_web_scan(&mut run, &report, counters);

        match &run.status {
            JobStatus::Degraded { reason, .. } => {
                assert!(
                    reason.contains("3 warning(s) persisted but 0 triage completions"),
                    "degraded reason must name the triage outage: {reason}"
                );
            }
            other => panic!("persisted warnings without triage must degrade, got {other:?}"),
        }
        assert_eq!(run.items_processed, 1);
    }

    #[tokio::test]
    async fn job_succeeds_and_reports_real_counters_when_persistence_ok() {
        let persistence = MockPersistence::default();
        let ingress = MockWarningIngress::default();
        let report = test_report(
            vec![test_post("post-1", 0.95), test_post("post-2", 0.2)],
            2,
            1,
        );
        let counters = persist_dark_web_posts(&persistence, &ingress, &report.posts).await;

        let mut run = JobRun::new(JobKind::DarkWebScan);
        run.start();
        complete_dark_web_scan(&mut run, &report, counters);

        assert!(matches!(run.status, JobStatus::Succeeded { .. }));
        assert_eq!(run.items_processed, 2);
        assert!(
            run.notes.contains("scanned 2/3 active forums (1 failed)"),
            "{}",
            run.notes
        );
        assert!(run.notes.contains("2 posts seen"), "{}", run.notes);
        assert!(
            run.notes.contains("2 observations inserted (0 errors)"),
            "{}",
            run.notes
        );
        assert!(
            run.notes.contains("1 warnings inserted (0 errors)"),
            "{}",
            run.notes
        );
    }

    #[tokio::test]
    async fn empty_scan_succeeds_with_zero_counters() {
        let persistence = MockPersistence::default();
        let ingress = MockWarningIngress::default();
        let report = test_report(vec![], 1, 0);
        let counters = persist_dark_web_posts(&persistence, &ingress, &report.posts).await;

        let mut run = JobRun::new(JobKind::DarkWebScan);
        run.start();
        complete_dark_web_scan(&mut run, &report, counters);

        assert!(matches!(run.status, JobStatus::Succeeded { .. }));
        assert_eq!(run.items_processed, 0);
        assert!(run.notes.contains("0 posts seen"), "{}", run.notes);
    }
}
