//! Source-pipeline dogfood: adversarial, assert-based checks for the entire
//! source acquisition → parse → ingest → detection → warning path.
//!
//! This is the second half of the repo's dogfood framework (the analytical
//! pipeline has its own harness). It exists because the 2026-10-06 incidents
//! — mass 403 blocking of 78 sources by a bot User-Agent, "silent for N days"
//! false positives on healthy quiet feeds, ingestion accounting that counted
//! no-op upserts as ingested — all lived here, and the analytical dogfood by
//! construction could not see them.
//!
//! Modes:
//!   (default)              registry invariants only — runs in CI, no network.
//!   --db <DATABASE_URL>    asserts against REAL runtime state and warnings:
//!                          UA-policy regressions, impossible states,
//!                          detection false positives, warning spam.
//!   --live                 additionally fetches a paced sample of enabled
//!                          HTTP sources with the real fetch policy and
//!                          classifies failures (UA-block vs unreachable).
//!   --sample N              limit for --live (default 40).
//!   --since <RFC3339>       policy-fix cutoff for regression checks
//!                          (default: 2026-10-06T14:20:00Z, the UA-policy
//!                          deploy).
//!
//! Exit code 0 = all assertions held. Any violation prints a FAIL line and
//! exits 1 — this harness is meant to break builds and deployments.

#![allow(clippy::unwrap_used, clippy::expect_used)] // An audit harness must fail loudly, not degrade silently.

use apex_crawl::fetch_policy::{prefer_browser_user_agent, user_agent_for_fetch};
use apex_crawl::sources::{all_sources, FetchStrategy, Source};
use sqlx::Row;
use std::time::Duration;

/// Merge floors mirror the registry invariant tests (may only increase).
const MERGED_REGISTRY_FLOOR: usize = 570;
const DEFAULT_POLICY_SINCE: &str = "2026-10-06T14:20:00Z";

struct Harness {
    failures: Vec<String>,
    checks: u64,
}

impl Harness {
    fn new() -> Self {
        Self {
            failures: Vec::new(),
            checks: 0,
        }
    }

    fn check(&mut self, condition: bool, label: impl Into<String>) {
        self.checks += 1;
        let label = label.into();
        if condition {
            println!("  ok   {label}");
        } else {
            println!("  FAIL {label}");
            self.failures.push(label);
        }
    }

    fn fail(&mut self, label: impl Into<String>) {
        self.check(false, label);
    }

    fn finish(&self) -> i32 {
        println!(
            "\nsource dogfood: {} checks, {} failures",
            self.checks,
            self.failures.len()
        );
        if self.failures.is_empty() {
            println!("PASS: source pipeline assertions held");
            0
        } else {
            println!("FAIL: source pipeline assertions violated");
            for failure in &self.failures {
                println!("  - {failure}");
            }
            1
        }
    }
}

// ── 1. Registry invariants (CI mode) ────────────────────────────────────────

fn registry_invariants(harness: &mut Harness) {
    println!("[registry invariants]");
    let sources: Vec<Source> = all_sources();

    harness.check(
        sources.len() >= MERGED_REGISTRY_FLOOR,
        format!(
            "registry has {} sources (floor {MERGED_REGISTRY_FLOOR})",
            sources.len()
        ),
    );
    let disabled = sources.iter().filter(|source| !source.enabled).count();
    harness.check(
        disabled == 0,
        format!("0 excluded sources (found {disabled})"),
    );

    let mut seen = std::collections::HashSet::new();
    let duplicates: Vec<&str> = sources
        .iter()
        .filter(|source| !seen.insert(source.slug.as_str()))
        .map(|source| source.slug.as_str())
        .collect();
    harness.check(
        duplicates.is_empty(),
        format!("unique slugs (duplicates: {duplicates:?})"),
    );

    harness.check(
        sources
            .iter()
            .any(|source| source.slug == "telegram_intelslavaz"),
        "IntelSlavaZ is registered",
    );

    let telegram_channels = sources
        .iter()
        .filter(|source| source.slug.starts_with("telegram_") && source.slug != "telegram_channels")
        .count();
    harness.check(
        telegram_channels >= 12,
        format!("{telegram_channels} telegram channels monitored (>= 12)"),
    );

    // Onion rules: an onion endpoint must be fetchable through the Tor path
    // (needs_proxy) and every onion URL must actually be an .onion host.
    let mut onion_problems: Vec<&str> = Vec::new();
    for source in &sources {
        let endpoint = source.rss_url.as_deref().unwrap_or(source.url.as_str());
        if endpoint.contains(".onion") {
            if !source.needs_proxy && !apex_crawl::sources::is_onion_source(source) {
                onion_problems.push(source.slug.as_str());
            }
            if !apex_crawl::sources::is_onion_endpoint(endpoint) {
                onion_problems.push(source.slug.as_str());
            }
        }
    }
    harness.check(
        onion_problems.is_empty(),
        format!("onion sources use the Tor path ({onion_problems:?})"),
    );

    // Every non-onion source must have a parseable http(s) endpoint.
    let mut bad_urls: Vec<&str> = Vec::new();
    for source in &sources {
        if apex_crawl::sources::is_onion_source(source) {
            continue;
        }
        let endpoint = source.rss_url.as_deref().unwrap_or(source.url.as_str());
        if !(endpoint.starts_with("http://") || endpoint.starts_with("https://")) {
            bad_urls.push(source.slug.as_str());
        }
    }
    harness.check(
        bad_urls.is_empty(),
        format!("all clearnet endpoints are http(s) ({bad_urls:?})"),
    );

    // The UA policy itself: default must be browser-class.
    if std::env::var("APEX_CRAWL_USE_BOT_UA").is_err() {
        harness.check(
            prefer_browser_user_agent(),
            "fetch policy defaults to browser user-agent",
        );
        harness.check(
            user_agent_for_fetch(true).contains("Mozilla/5.0"),
            "browser user-agent looks browser-class",
        );
    }
}

// ── 2. Runtime-state honesty (--db) ─────────────────────────────────────────

#[derive(Debug)]
struct RuntimeRow {
    slug: String,
    failures: i32,
    http_status: Option<i32>,
    error: Option<String>,
    last_attempt: Option<chrono::DateTime<chrono::Utc>>,
    last_success: Option<chrono::DateTime<chrono::Utc>>,
    last_item_at: Option<chrono::DateTime<chrono::Utc>>,
}

async fn runtime_invariants(
    harness: &mut Harness,
    pool: &sqlx::PgPool,
    policy_since: chrono::DateTime<chrono::Utc>,
) -> Vec<String> {
    println!("[runtime-state honesty]");
    let rows = sqlx::query(
        "SELECT source_slug, consecutive_failures, last_http_status, last_error,
                last_attempt_at, last_success_at, last_item_at
         FROM source_runtime_state",
    )
    .fetch_all(pool)
    .await
    .expect("load runtime state");

    let runtime: Vec<RuntimeRow> = rows
        .iter()
        .map(|row| RuntimeRow {
            slug: row.get("source_slug"),
            failures: row.get("consecutive_failures"),
            http_status: row.get("last_http_status"),
            error: row.get("last_error"),
            last_attempt: row.get("last_attempt_at"),
            last_success: row.get("last_success_at"),
            last_item_at: row.get("last_item_at"),
        })
        .collect();

    // a. Capability gaps must not accumulate failure counts.
    let poisoned: Vec<&str> = runtime
        .iter()
        .filter(|row| {
            row.error
                .as_deref()
                .is_some_and(|error| error.starts_with("unavailable:") && row.failures >= 3)
        })
        .map(|row| row.slug.as_str())
        .collect();
    harness.check(
        poisoned.is_empty(),
        format!("capability gaps keep failure counts at 0 ({poisoned:?})"),
    );

    // b. UA-policy regressions: a 403 recorded after the policy deploy is a
    //    *suspect*. Classification requires proving the endpoint works with
    //    the browser UA: reachable-with-browser + recorded-403 means the
    //    policy was not applied (hard failure); browser-UA failure too means
    //    the host is genuinely closed to us (note, not a regression). The
    //    suspects are returned and classified in the live section.
    let ua_suspects: Vec<String> = runtime
        .iter()
        .filter(|row| {
            row.http_status == Some(403)
                && row
                    .last_attempt
                    .is_some_and(|attempt| attempt > policy_since)
                && row
                    .error
                    .as_deref()
                    .is_some_and(|error| error.contains("authentication_required"))
        })
        .map(|row| row.slug.clone())
        .collect();
    if ua_suspects.is_empty() {
        harness.check(true, "no post-fix 403 suspects");
    } else {
        println!(
            "  note {} post-fix 403 suspects (classified by --live): {ua_suspects:?}",
            ua_suspects.len()
        );
    }

    // c. Impossible state: feed freshness newer than the last success.
    let impossible: Vec<&str> = runtime
        .iter()
        .filter(|row| match (row.last_item_at, row.last_success) {
            (Some(item), Some(success)) => item > success + chrono::Duration::hours(1),
            _ => false,
        })
        .map(|row| row.slug.as_str())
        .collect();
    harness.check(
        impossible.is_empty(),
        format!("no impossible feed-freshness states ({impossible:?})"),
    );

    let now = chrono::Utc::now();
    let stale = runtime
        .iter()
        .filter(|row| {
            row.last_attempt
                .map(|attempt| now - attempt > chrono::Duration::days(7))
                .unwrap_or(true)
        })
        .count();
    println!(
        "  note {stale} of {} sources not attempted in 7 days",
        runtime.len()
    );
    let failing = runtime.iter().filter(|row| row.failures >= 3).count();
    println!("  note {failing} sources with >= 3 consecutive failures");
    ua_suspects
}

// ── 3. Detection cross-check against generated warnings (--db) ──────────────

async fn detection_crosscheck(
    harness: &mut Harness,
    pool: &sqlx::PgPool,
    policy_since: chrono::DateTime<chrono::Utc>,
) {
    println!("[detection cross-check]");

    let rows = sqlx::query(
        "SELECT title, warning_type, created_at
         FROM warnings
         WHERE warning_type IN ('source_outage', 'source_fetch_failure')
           AND created_at > $1
         ORDER BY created_at DESC
         LIMIT 100",
    )
    .bind(policy_since)
    .fetch_all(pool)
    .await
    .expect("load source warnings");

    // a. Removed wording must never reappear.
    let spam: Vec<String> = rows
        .iter()
        .map(|row| row.get::<String, _>("title"))
        .filter(|title| {
            let lower = title.to_ascii_lowercase();
            lower.contains("silent for") || lower.contains("not been active")
        })
        .collect();
    harness.check(
        spam.is_empty(),
        format!("no day-count/`silent for` warning titles ({spam:?})"),
    );

    // b. Every post-fix source_outage warning must name a source whose feed
    //    is genuinely fresh (an ingestion stall), not a quiet feed.
    let mut outage_problems = Vec::new();
    let mut fetch_problems = Vec::new();
    for row in &rows {
        let title: String = row.get("title");
        let warning_type: String = row.get("warning_type");
        let Some(slug) = extract_source_slug(&title) else {
            continue;
        };
        let runtime = sqlx::query(
            "SELECT consecutive_failures, last_error, last_item_at, last_success_at
             FROM source_runtime_state WHERE source_slug = $1",
        )
        .bind(&slug)
        .fetch_optional(pool)
        .await
        .expect("load runtime row for warning");

        let Some(runtime) = runtime else {
            continue; // source not in runtime state; detection skips those too
        };

        if warning_type == "source_outage" {
            let last_item_at: Option<chrono::DateTime<chrono::Utc>> = runtime.get("last_item_at");
            let fresh_feed = last_item_at
                .map(|item| chrono::Utc::now() - item <= chrono::Duration::days(3))
                .unwrap_or(false);
            if !fresh_feed {
                outage_problems.push(format!(
                    "{slug}: outage warning but feed is quiet (not an ingestion stall)"
                ));
            }
        } else if warning_type == "source_fetch_failure" {
            let failures: i32 = runtime.get("consecutive_failures");
            let error: Option<String> = runtime.get("last_error");
            if failures < 3 {
                fetch_problems.push(format!(
                    "{slug}: fetch-failure warning with only {failures} failures"
                ));
            }
            if error
                .as_deref()
                .is_some_and(|error| error.starts_with("unavailable:"))
            {
                fetch_problems.push(format!("{slug}: capability gap reported as fetch failure"));
            }
        }
    }
    harness.check(
        outage_problems.is_empty(),
        format!("outage warnings are genuine ingestion stalls ({outage_problems:?})"),
    );
    harness.check(
        fetch_problems.is_empty(),
        format!("fetch-failure warnings match failing runtime rows ({fetch_problems:?})"),
    );
    println!("  note {} post-fix source warnings checked", rows.len());
}

fn extract_source_slug(title: &str) -> Option<String> {
    let start = title.find('\'')? + 1;
    let end = title[start..].find('\'')? + start;
    Some(title[start..end].to_string())
}

// ── 4. Live endpoint sampling (--live) ──────────────────────────────────────

async fn live_endpoint_sample(harness: &mut Harness, sample: usize, ua_suspects: &[String]) {
    println!("[live endpoint sample]");
    let sources: Vec<Source> = all_sources();
    let mut candidates: Vec<&Source> = sources
        .iter()
        .filter(|source| {
            source.enabled
                && source.strategy() != FetchStrategy::Browser
                && !apex_crawl::sources::is_onion_source(source)
        })
        .collect();
    candidates.sort_by(|left, right| left.slug.cmp(&right.slug));
    if candidates.is_empty() {
        return;
    }
    // Deterministic stride sample across the alphabet so the audit touches
    // the whole registry, not just tier-1 sources.
    let stride = (candidates.len() / sample.max(1)).max(1);
    let sampled: Vec<&Source> = candidates
        .iter()
        .step_by(stride)
        .take(sample)
        .copied()
        .collect();

    let client = apex_crawl::client::CrawlClient::new(apex_crawl::client::CrawlClientConfig {
        enforce_robots_txt: false,
        allow_private_targets: false,
        ..apex_crawl::client::CrawlClientConfig::default()
    })
    .expect("crawl client");

    let mut ok = 0usize;
    let mut unreachable = Vec::new();
    for source in sampled {
        let endpoint = source.rss_url.as_deref().unwrap_or(source.url.as_str());
        let browser_request = apex_crawl::client::CrawlRequest::new(endpoint)
            .prefer_browser_user_agent(true)
            .override_user_agent(user_agent_for_fetch(true))
            .requires_proxy(source.needs_proxy);
        match client.fetch_text(&browser_request).await {
            Ok(_) => ok += 1,
            Err(browser_error) => {
                // Distinguish UA-block (bot UA would also fail; browser UA
                // fails too?) from unreachable. A host that blocks the
                // browser UA as well is reported as unreachable.
                unreachable.push(format!("{}: {browser_error}", source.slug));
            }
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }

    harness.check(
        !unreachable.is_empty() || ok > 0,
        "live sample produced results",
    );
    println!(
        "  note {ok}/{} sampled sources reachable under the browser-UA policy; {} unreachable",
        ok + unreachable.len(),
        unreachable.len()
    );
    if !unreachable.is_empty() {
        println!("  note unreachable sample (first 10):");
        for entry in unreachable.iter().take(10) {
            println!("       {entry}");
        }
    }
    // A sampled-registry reachability floor: the registry contains known
    // bot-blocked publishers (they fail honestly and are reported as fetch
    // failures); the harness asserts the floor that makes auditing useful
    // rather than guaranteeing every host is reachable.
    let total = ok + unreachable.len();
    let ratio = if total == 0 {
        0.0
    } else {
        ok as f64 / total as f64
    };
    harness.check(
        ratio >= 0.55,
        format!("live reachability ratio {ratio:.2} >= 0.55 (registry is usable)"),
    );

    // Classify the post-fix 403 suspects by refetching with the browser UA.
    // Reachable + recorded-403 means the fetch policy was not applied to that
    // source (hard failure); failing under the browser UA too means the host
    // is genuinely closed to us (honest note, not a regression).
    if !ua_suspects.is_empty() {
        let all = all_sources();
        let by_slug: std::collections::HashMap<&str, &Source> = all
            .iter()
            .map(|source| (source.slug.as_str(), source))
            .collect();
        let mut regressions = Vec::new();
        let mut genuinely_blocked = Vec::new();
        for slug in ua_suspects {
            let Some(source) = by_slug.get(slug.as_str()) else {
                continue;
            };
            let endpoint = source.rss_url.as_deref().unwrap_or(source.url.as_str());
            let request = apex_crawl::client::CrawlRequest::new(endpoint)
                .prefer_browser_user_agent(true)
                .override_user_agent(user_agent_for_fetch(true))
                .requires_proxy(source.needs_proxy);
            match client.fetch_text(&request).await {
                Ok(_) => regressions.push(format!(
                    "{slug}: recorded 403 but browser-UA fetch succeeds — policy not applied"
                )),
                Err(error) => genuinely_blocked.push(format!("{slug}: {error}")),
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
        harness.check(
            regressions.is_empty(),
            format!("no UA-policy regressions among post-fix 403 suspects ({regressions:?})"),
        );
        for entry in &genuinely_blocked {
            println!("  note genuinely blocked under browser UA: {entry}");
        }
    }
}

// ── main ────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    let mut harness = Harness::new();
    let mut db_url: Option<String> = None;
    let mut live = false;
    let mut sample = 40usize;
    let mut policy_since = chrono::DateTime::parse_from_rfc3339(DEFAULT_POLICY_SINCE)
        .expect("default cutoff parses")
        .with_timezone(&chrono::Utc);

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--db" => db_url = args.next(),
            "--live" => live = true,
            "--sample" => {
                sample = args
                    .next()
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(40);
            }
            "--since" => {
                if let Some(value) = args.next() {
                    if let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(&value) {
                        policy_since = parsed.with_timezone(&chrono::Utc);
                    }
                }
            }
            other => {
                eprintln!("unknown argument: {other}");
                std::process::exit(2);
            }
        }
    }

    println!("ApexIntel source-pipeline dogfood");
    println!("=================================");

    registry_invariants(&mut harness);

    let mut ua_suspects: Vec<String> = Vec::new();
    if let Some(url) = db_url {
        let pool = sqlx::PgPool::connect(&url)
            .await
            .expect("connect to postgres for --db checks");
        ua_suspects = runtime_invariants(&mut harness, &pool, policy_since).await;
        detection_crosscheck(&mut harness, &pool, policy_since).await;
    } else {
        println!("[runtime-state honesty] skipped (pass --db <DATABASE_URL>)");
        println!("[detection cross-check] skipped (pass --db <DATABASE_URL>)");
    }

    if live {
        live_endpoint_sample(&mut harness, sample, &ua_suspects).await;
    } else {
        println!("[live endpoint sample] skipped (pass --live)");
    }

    std::process::exit(harness.finish());
}
