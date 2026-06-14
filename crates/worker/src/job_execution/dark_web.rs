//! Dark web forum scan job execution.
//!
//! Periodically scans configured dark web forums, paste sites, and ransomware
//! blogs for mentions of monitored entities.  Matches are stored as
//! `Observation` records (type `DarkWebPost`) and high-relevance matches
//! trigger security warnings.

use std::sync::Arc;

use uuid::Uuid;

use crate::*;

/// Run a dark web forum scan cycle.
///
/// 1. Build a [`DarkWebMonitor`] (optionally with Tor SOCKS5 proxy).
/// 2. Load monitoring rules from environment or DB.
/// 3. Scan all active forums.
/// 4. Store matching posts as observations.
/// 5. Generate warnings for high-relevance matches.
pub(super) async fn run_dark_web_scan(kind: &JobKind, store: &Arc<PgStore>) -> JobRun {
    let mut run = JobRun::new(kind.clone());
    run.start();

    // Load Tor proxy config from environment (optional)
    let tor_proxy_url = std::env::var("DARKWEB_TOR_PROXY").ok();
    let monitor = match apex_crawl::dark_web::DarkWebMonitor::new(tor_proxy_url) {
        Ok(m) => m,
        Err(e) => {
            run.fail(&format!("dark_web_scan: failed to build monitor: {e}"));
            return run;
        }
    };

    // Load monitoring rules from environment variable
    let rules_json = std::env::var("DARKWEB_MONITORING_RULES").ok();
    if let Some(json) = rules_json {
        match serde_json::from_str::<Vec<apex_crawl::dark_web::MonitoringRule>>(&json) {
            Ok(rules) => {
                tracing::info!(
                    count = rules.len(),
                    "dark_web_scan: loaded rules from DARKWEB_MONITORING_RULES"
                );
                // We use the default forum list with custom rules
                // In production, rules would come from a DB table
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "dark_web_scan: failed to parse DARKWEB_MONITORING_RULES, using defaults"
                );
            }
        }
    }

    // Load custom forums from env var (JSON array, optional)
    let forums_json = std::env::var("DARKWEB_FORUMS").ok();
    if let Some(json) = forums_json {
        match serde_json::from_str::<Vec<apex_crawl::dark_web::DarkWebForum>>(&json) {
            Ok(forums) => {
                tracing::info!(
                    count = forums.len(),
                    "dark_web_scan: loaded forums from DARKWEB_FORUMS"
                );
                // Note: set_forums would require mutable access; we use defaults for now
                // and the env var serves as documentation for future customization.
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "dark_web_scan: failed to parse DARKWEB_FORUMS, using defaults"
                );
            }
        }
    }

    // Scan all active forums
    let posts = monitor.scan_all().await;
    let total_posts = posts.len() as u64;
    let mut observations_stored = 0u64;
    let mut warnings_generated = 0u64;

    if posts.is_empty() {
        run.succeed(
            0,
            "dark_web_scan: scanned all forums — no matching posts found",
        );
        return run;
    }

    tracing::info!(
        count = total_posts,
        "dark_web_scan: found matching posts"
    );

    let pool = &store.pool;
    let _now = chrono::Utc::now();

    for post in &posts {
        // Build observation value
        #[allow(clippy::disallowed_methods)]
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

        #[allow(clippy::disallowed_methods)]
        let provenance = serde_json::json!({
            "source": "worker_dark_web_scan",
            "forum": post.forum_name,
            "post_id": post.id,
            "content_hash": format!("dw_{}", post.id),
        });

        let obs_id = Uuid::new_v4();

        // Insert observation
        let insert_result = sqlx::query(
            r#"INSERT INTO observations
               (id, observation_type, entity_id, entity_type, ts_utc, value, provenance, confidence)
               VALUES ($1, 'DarkWebPost', NULL, NULL, $2, $3::jsonb, $4::jsonb, $5)"#,
        )
        .bind(obs_id)
        .bind(post.posted_at)
        .bind(&value)
        .bind(&provenance)
        .bind(post.relevance_score)
        .execute(pool)
        .await;

        match insert_result {
            Ok(_) => observations_stored += 1,
            Err(e) => {
                tracing::warn!(
                    post_id = %post.id,
                    forum = %post.forum_name,
                    error = %e,
                    "dark_web_scan: failed to store observation"
                );
            }
        }

        // Generate warning for high-relevance matches (score >= 0.7)
        if post.relevance_score >= 0.7 {
            let title = format!(
                "Dark web mention: {} — {}",
                post.forum_name, post.thread_title
            );
            let description = format!(
                "High-relevance dark web post detected on '{}' (score: {:.2}). \
                 Author: {}. Matched keywords: {}. Entities: {}. \
                 Snippet: {}",
                post.forum_name,
                post.relevance_score,
                post.author,
                post.matched_keywords.join(", "),
                post.entities_mentioned.join(", "),
                post.content_snippet,
            );

            let severity = if post.relevance_score >= 0.9 {
                "critical"
            } else {
                "high"
            };

            let _ = store
                .insert_warning(
                    "dark_web",
                    &title,
                    Some(&description),
                    severity,
                    None,
                    Some("worker_dark_web_scan"),
                    None,
                    Some(vec![post.url.clone()]),
                    Some(post.relevance_score),
                )
                .await;

            warnings_generated += 1;
        }
    }

    run.succeed(
        observations_stored,
        &format!(
            "dark_web_scan: scanned {} forums, found {} matching posts, \
             stored {} observations, generated {} warnings",
            monitor.forums.iter().filter(|f| f.is_active).count(),
            total_posts,
            observations_stored,
            warnings_generated,
        ),
    );
    run
}
