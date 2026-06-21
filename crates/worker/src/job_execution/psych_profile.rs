//! Psychological Profile Computation handler.
//!
//! Scheduled daily: recalculates psych profiles for all tracked POIs using the
//! **canonical** [`PsychComputeEngine`] from `apex_insights`. The engine reads
//! each person's recent `poi_artifacts`, derives an evidence-backed
//! [`RawProfileSnapshot`], and persists the results to the dedicated tables:
//!
//! - `psychological_profiles`   (decision style, change appetite, pain index, …)
//! - `behavioral_pattern_events` (detected sentiment/priority/engagement shifts)
//! - `engagement_profiles`       (recommended talking points & channels)
//!
//! This replaces the previous local heuristic implementation which wrote
//! simplified values to `observations` / `persons.metadata`. Those duplicate
//! writes were removed because no API consumes `persons.metadata->'psych_profile'`;
//! the canonical tables above are now the single source of truth.
//!
//! The job is resilient: a failure for one person is logged and counted but
//! never aborts the whole run.
use std::sync::Arc;
use std::time::Instant;

use crate::*;

#[cfg(feature = "llm")]
use apex_insights::psych_compute::{PsychComputeEngine, PsychObservation, RawProfileSnapshot};
use chrono::Utc;

/// Lightweight artifact row fetched from `poi_artifacts` for psych computation.
#[derive(Debug, sqlx::FromRow)]
struct ArtifactRow {
    title: String,
    content_summary: Option<String>,
    ts_utc: Option<chrono::DateTime<Utc>>,
    source_url: Option<String>,
}

/// Execute PsychProfileCompute job: recalculate psych profiles for all POIs.
///
/// Delegates to the canonical engine when the `llm` feature is enabled (the
/// default). Without it, the job skips gracefully since `apex-insights` is the
/// sole provider of the compute engine.
pub(super) async fn run_psych_profile_compute(kind: &JobKind, store: &Arc<PgStore>) -> JobRun {
    #[cfg(feature = "llm")]
    {
        run_psych_profile_compute_inner(kind, store).await
    }
    #[cfg(not(feature = "llm"))]
    {
        let _ = store;
        let mut run = JobRun::new(kind.clone());
        run.start();
        run.skip("psych_profile_compute: requires the 'llm' feature (apex-insights)");
        run
    }
}

#[cfg(feature = "llm")]
async fn run_psych_profile_compute_inner(kind: &JobKind, store: &Arc<PgStore>) -> JobRun {
    let mut run = JobRun::new(kind.clone());
    run.start();
    let total_start = Instant::now();

    let persons = store
        .list_persons(
            &apex_store::postgres::PersonListFilters {
                regions: vec![],
                roles: vec![],
                search: None,
                min_priority: None,
                max_priority: None,
            },
            Some(apex_store::postgres::PersonOrderBy::Name),
            false,
            1000,
            0,
        )
        .await
        .unwrap_or_default();

    if persons.is_empty() {
        run.skip("psych_profile_compute: no persons found in database");
        return run;
    }

    // One canonical engine instance reused across all persons (it only holds
    // a clone of the connection pool).
    let engine = PsychComputeEngine::new(store.pool.clone());

    let mut profiles_computed: u64 = 0;
    let mut profiles_failed: u64 = 0;
    let mut profiles_skipped: u64 = 0;

    for person in &persons {
        // Load the person's recent artifacts (title, summary, timestamp, source URL).
        let artifacts: Vec<ArtifactRow> = sqlx::query_as(
            "SELECT title, content_summary, ts_utc, source_url \
             FROM poi_artifacts WHERE person_id = $1 \
             ORDER BY ts_utc DESC LIMIT 200",
        )
        .bind(person.id)
        .fetch_all(&store.pool)
        .await
        .unwrap_or_default();

        if artifacts.is_empty() {
            profiles_skipped += 1;
            continue;
        }

        // Build structured observations from the raw artifact rows.
        let observations: Vec<PsychObservation> = artifacts
            .iter()
            .map(|a| PsychObservation {
                text: format!("{} {}", a.title, a.content_summary.as_deref().unwrap_or("")),
                source_url: a.source_url.clone(),
                source_domain: a.source_url.as_deref().and_then(extract_domain),
                observed_at: a.ts_utc.unwrap_or_else(Utc::now),
                sentiment_score: None,
            })
            .collect();

        let snapshot = RawProfileSnapshot {
            person_id: person.id.to_string(),
            person_name: person.name.clone(),
            current_title: person.role.clone(),
            role_family: person.role_family.clone(),
            organization: person.organization.clone(),
            // Career length / job-change counts are not derivable from
            // poi_artifacts alone; the engine degrades gracefully when absent.
            career_length_years: None,
            job_change_count: None,
            public_statements_count: observations.len() as u32,
            observations,
        };

        // Run the canonical pipeline. The engine persists to
        // psychological_profiles, behavioral_pattern_events, and
        // engagement_profiles internally.
        match engine.compute_and_persist(&snapshot).await {
            Ok(result) => {
                profiles_computed += 1;
                tracing::info!(
                    person_id = %person.id,
                    decision_style = %result.decision_style,
                    pain_index = %result.pain_index,
                    enrichment = %result.enrichment_quality,
                    "psych_profile_compute: profile computed and persisted"
                );
                // Surface the profile update in the activity feed so analysts
                // can see psychometric enrichment happening in real time.
                let activity_logger =
                    apex_worker::activity_logger::ActivityLogger::new(store.pool.clone());
                activity_logger
                    .log_psych_profile_updated(
                        &person.name,
                        result.enrichment_quality,
                        Some(&person.id.to_string()),
                    )
                    .await;
            }
            Err(error) => {
                profiles_failed += 1;
                tracing::warn!(
                    person_id = %person.id,
                    error = %error,
                    "psych_profile_compute: failed for person, continuing"
                );
            }
        }
    }

    let elapsed = total_start.elapsed();
    run.succeed(
        profiles_computed,
        &format!(
            "psych_profile_compute: {} persons processed, {} profiles computed, \
             {} failed, {} skipped (no artifacts) in {:.1}s",
            persons.len(),
            profiles_computed,
            profiles_failed,
            profiles_skipped,
            elapsed.as_secs_f64(),
        ),
    );
    run
}

/// Extract the registrable host (without scheme, port, or leading `www.`)
/// from a URL. Used to populate `PsychObservation::source_domain`.
#[cfg(feature = "llm")]
fn extract_domain(url: &str) -> Option<String> {
    let after_scheme = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);
    let host = after_scheme.split(['/', '?', '#', '&']).next()?;
    let host = host.split(':').next()?; // drop port
    let cleaned = host.trim().trim_start_matches("www.");
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned.to_string())
    }
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "llm")]
    use super::*;

    #[cfg(feature = "llm")]
    #[test]
    fn test_extract_domain_strips_scheme_and_path() {
        assert_eq!(
            extract_domain("https://www.example.com/path/to/page?x=1"),
            Some("example.com".to_string())
        );
        assert_eq!(
            extract_domain("http://news.example.org/article"),
            Some("news.example.org".to_string())
        );
    }

    #[cfg(feature = "llm")]
    #[test]
    fn test_extract_domain_handles_port_and_bare_host() {
        assert_eq!(extract_domain("https://api.example.com:8443"), Some("api.example.com".to_string()));
        assert_eq!(extract_domain("example.com"), Some("example.com".to_string()));
    }

    #[cfg(feature = "llm")]
    #[test]
    fn test_extract_domain_rejects_empty() {
        assert_eq!(extract_domain(""), None);
        assert_eq!(extract_domain("https://"), None);
    }
}
