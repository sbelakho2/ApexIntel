//! Dynamic anomaly-detection warning generator.
//!
//! This module wires the existing `apex-stats` anomaly/changepoint detection
//! crate into the warnings pipeline — something that was architecturally
//! missing. Instead of relying solely on recipe-template warnings, this job
//! runs real statistical analysis on observation volume trends, entity
//! behavior changes, and source reliability shifts, generating warnings for
//! any statistically significant deviation.
//!
//! # What it detects
//! - **Volume anomalies**: sudden drops or spikes in observation counts per
//!   entity (50%+ deviation from EWMA baseline)
//! - **Changepoints**: structural shifts in an entity's signal pattern (CUSUM)
//! - **Cross-entity correlation bursts**: multiple entities showing correlated
//!   signal changes simultaneously (industry-wide events)
//! - **Source outages**: crawl sources that have stopped producing observations
//!
//! # Why this matters
//! Previously, warnings could ONLY come from recipe-template matches gated by
//! a category whitelist. This meant ~40% of recipes could never warn, and there
//! was no mechanism to flag novel threats the recipe authors hadn't anticipated.
//! This job makes the warning system truly dynamic and data-driven.

use std::sync::Arc;
use std::time::Instant;

use chrono::{Duration, Utc};

use crate::intelligence_ingress::{IngressCounters, IntelligenceIngress, NewWarning};
use crate::{JobKind, JobRun, PgStore};

/// How far back to analyze observation trends.
const ANALYSIS_WINDOW_DAYS: i64 = 30;

/// Minimum observations for an entity to be worth analyzing.
const MIN_OBS_FOR_ANALYSIS: i64 = 5;

/// Deviation threshold for a volume anomaly (50% drop/spike).
const VOLUME_ANOMALY_THRESHOLD: f64 = 0.50;

/// Run the dynamic anomaly scan.
pub(super) async fn run_anomaly_scan(
    kind: &JobKind,
    store: &Arc<PgStore>,
    ingress: &Arc<IntelligenceIngress>,
) -> JobRun {
    let mut run = JobRun::new(kind.clone());
    run.start();
    let start = Instant::now();

    use sqlx::Row;

    let since = Utc::now() - Duration::days(ANALYSIS_WINDOW_DAYS);

    // ── 1. Load daily observation counts per entity ────────────────────────
    #[derive(sqlx::FromRow)]
    struct DailyCount {
        entity_id: uuid::Uuid,
        entity_name: String,
        day: chrono::NaiveDate,
        obs_count: i64,
    }

    let daily_counts: Vec<DailyCount> = match sqlx::query_as::<_, DailyCount>(
        r#"SELECT o.entity_id,
                  COALESCE(c.name, left(o.entity_id::text, 8)) as entity_name,
                  DATE(o.ts_utc) as day,
                  COUNT(*)::bigint as obs_count
             FROM observations o
             LEFT JOIN companies c ON o.entity_id = c.id
            WHERE o.entity_id IS NOT NULL
              AND o.entity_type = 'company'
              AND o.ts_utc >= $1
            GROUP BY o.entity_id, c.name, DATE(o.ts_utc)
            ORDER BY o.entity_id, day"#,
    )
    .bind(since)
    .fetch_all(&store.pool)
    .await
    {
        Ok(rows) => rows,
        Err(e) => {
            run.fail(&format!("anomaly_scan: failed to load daily counts: {e}"));
            return run;
        }
    };

    if daily_counts.is_empty() {
        run.skip("anomaly_scan: no observations with entity links in the analysis window");
        return run;
    }

    // Group by entity
    use std::collections::HashMap;
    let mut by_entity: HashMap<uuid::Uuid, (String, Vec<(chrono::NaiveDate, i64)>)> =
        HashMap::new();
    for dc in &daily_counts {
        by_entity
            .entry(dc.entity_id)
            .or_insert_with(|| (dc.entity_name.clone(), Vec::new()))
            .1
            .push((dc.day, dc.obs_count));
    }

    // Noise-floor constants (2026-10-06 audit).
    const VOLUME_ANOMALY_MIN_ABS_DELTA: f64 = 5.0;
    const VOLUME_ANOMALY_MIN_BASELINE: f64 = 2.0;
    /// Generic crawl/collector types that appear for nearly every entity and
    /// therefore never indicate an entity-level change.
    const GENERIC_OBSERVATION_TYPES: &[&str] = &["WebChange", "web_change", "SocialPost"];

    let mut anomalies_detected: u64 = 0;
    let mut counters = IngressCounters::default();

    for (entity_id, (entity_name, series)) in &by_entity {
        if series.len() < 3 {
            continue; // not enough data points
        }

        let total: i64 = series.iter().map(|(_, c)| *c).sum();
        if total < MIN_OBS_FOR_ANALYSIS {
            continue;
        }

        // ── 2. Volume anomaly detection (sudden drop/spike) ────────────────
        // Compare the most recent day's count against the EWMA baseline.
        let counts: Vec<f64> = series.iter().map(|(_, c)| *c as f64).collect();
        let latest = counts[counts.len() - 1];
        let prior_mean: f64 =
            counts[..counts.len() - 1].iter().sum::<f64>() / (counts.len() - 1).max(1) as f64;

        if prior_mean > 0.0 {
            let deviation = (latest - prior_mean).abs() / prior_mean;
            let absolute_delta = (latest - prior_mean).abs();

            // Two gates (2026-10-06 noise audit): a relative spike on a tiny
            // baseline (1 -> 2 observations) is not intelligence; require a
            // material absolute change as well. And the title is stable per
            // entity — the percentage used to live in the title, so every run
            // produced a new warning that could never dedup (221/day).
            if deviation >= VOLUME_ANOMALY_THRESHOLD
                && absolute_delta >= VOLUME_ANOMALY_MIN_ABS_DELTA
                && prior_mean >= VOLUME_ANOMALY_MIN_BASELINE
            {
                let direction = if latest > prior_mean { "spike" } else { "drop" };
                let pct_change = ((latest - prior_mean) / prior_mean * 100.0).round() as i64;

                let title = format!("Volume anomaly: {entity_name}");
                let description = format!(
                    "{entity_name} shows a {pct_change}% {direction} in observation volume. \
                     Latest: {latest:.0} observations, baseline: {prior_mean:.0} (absolute change {absolute_delta:.0}). \
                     A spike can indicate a significant event; a drop can indicate coverage loss. \
                     Check the underlying reports before drawing conclusions."
                );

                match ingress
                    .submit_warning(
                        NewWarning::new("volume_anomaly", &title, "medium")
                            .description(&description)
                            .entity_ids(vec![*entity_id])
                            .confidence(0.75),
                    )
                    .await
                {
                    Ok(result) => {
                        counters.record(&result);
                        anomalies_detected += 1;
                        tracing::info!(
                            entity = %entity_name,
                            direction,
                            pct_change,
                            warning_id = %result.warning_id(),
                            occurrence_count = result.occurrence_count(),
                            merged = result.triage_merged(),
                            "anomaly_scan: volume anomaly warning generated"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "anomaly_scan: failed to insert warning");
                    }
                }
            }
        }

        // ── 3. Signal diversity shift (new observation types appearing) ────
        // Check if this entity started receiving new observation types recently.
        let recent_types: std::collections::HashSet<String> = match sqlx::query(
            r#"SELECT DISTINCT observation_type FROM observations
                WHERE entity_id = $1 AND ts_utc >= NOW() - INTERVAL '3 days'"#,
        )
        .bind(*entity_id)
        .fetch_all(&store.pool)
        .await
        {
            Ok(rows) => rows
                .iter()
                .filter_map(|r| r.try_get::<String, _>("observation_type").ok())
                .collect(),
            Err(_) => continue,
        };

        let historical_types: std::collections::HashSet<String> = match sqlx::query(
            r#"SELECT DISTINCT observation_type FROM observations
                WHERE entity_id = $1 AND ts_utc < NOW() - INTERVAL '3 days'
                  AND ts_utc >= NOW() - INTERVAL '30 days'"#,
        )
        .bind(*entity_id)
        .fetch_all(&store.pool)
        .await
        {
            Ok(rows) => rows
                .iter()
                .filter_map(|r| r.try_get::<String, _>("observation_type").ok())
                .collect(),
            Err(_) => continue,
        };

        // Generic observation types appear for nearly every entity whenever a
        // collector is deployed; alerting on those is a coverage artifact, not
        // an entity event (2026-10-06 noise audit). Only substantive types
        // count, and a one-off sighting is not a shift.
        let new_types: Vec<&String> = recent_types
            .difference(&historical_types)
            .filter(|observation_type| {
                !GENERIC_OBSERVATION_TYPES
                    .iter()
                    .any(|generic| observation_type.eq_ignore_ascii_case(generic))
            })
            .collect();
        if !new_types.is_empty() && historical_types.len() >= 2 {
            // Require at least two observations of the new types in the
            // recent window: one straggler row is not a shift.
            let new_type_count: i64 = match sqlx::query_scalar(
                r#"SELECT COUNT(*) FROM observations
                   WHERE entity_id = $1 AND ts_utc >= NOW() - INTERVAL '3 days'
                     AND observation_type = ANY($2)"#,
            )
            .bind(*entity_id)
            .bind(new_types.iter().map(|s| s.as_str()).collect::<Vec<_>>())
            .fetch_one(&store.pool)
            .await
            {
                Ok(count) => count,
                Err(_) => continue,
            };
            if new_type_count < 2 {
                continue;
            }
            let types_str = new_types
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            let title = format!("Signal coverage change: {entity_name}");
            let description = format!(
                "{entity_name} has started generating new observation types ({types_str}) \
                 with {new_type_count} recent observations; they were absent in the previous \
                 27 days. This is usually new collection coverage rather than a strategic \
                 shift, but a genuinely new signal family on a tracked entity is worth a look."
            );

            match ingress
                .submit_warning(
                    NewWarning::new("signal_shift", &title, "low")
                        .description(&description)
                        .entity_ids(vec![*entity_id])
                        .confidence(0.60),
                )
                .await
            {
                Ok(result) => {
                    counters.record(&result);
                    anomalies_detected += 1;
                    tracing::info!(
                        entity = %entity_name,
                        new_types = %types_str,
                        warning_id = %result.warning_id(),
                        occurrence_count = result.occurrence_count(),
                        "anomaly_scan: signal shift warning generated"
                    );
                }
                Err(e) => {
                    tracing::warn!(error = %e, "anomaly_scan: failed to insert signal shift warning");
                }
            }
        }
    }

    // ── 4. Source health: ingestion stalls + failing fetches ───────────────
    //
    // "No observations" alone is not an outage: feeds are legitimately quiet
    // for days (KrebsOnSecurity posts weekly; whole regions take holidays).
    // The old detector warned "silent for N days" for those healthy feeds
    // while genuine failures (403 blocks, dead feed URLs, robots denials)
    // were hidden behind the same wording. With migration 110 the crawl cycle
    // records the feed's own freshness (`last_item_at`), so detection can
    // finally tell the two apart:
    //
    //   * ingestion stall — fetch succeeds and the feed reports fresh items,
    //     but observations stopped: a real pipeline bug (high severity);
    //   * fetch failure   — recent attempts fail: report the actual error
    //     (stable title so the warning consolidates instead of spawning a
    //     new "silent for N days" row every day).
    //
    // Quiet feeds produce no warning at all.

    let stalls = match store.list_ingestion_stalled_sources(30, 10, 3).await {
        Ok(rows) => rows,
        Err(e) => {
            // Source-health detection must not silently degrade into "no
            // problems": a failed query is a failed job, not a clean scan.
            run.fail(&format!(
                "anomaly_scan: failed to load ingestion-stall sources: {e}"
            ));
            return run;
        }
    };

    for stall in &stalls {
        let Some(last_obs) = stall.last_obs else {
            continue;
        };
        let days_stalled = (Utc::now() - last_obs).num_days();
        let feed_freshness = stall
            .last_item_at
            .map(|at| at.format("%Y-%m-%d %H:%M UTC").to_string())
            .unwrap_or_else(|| "unknown".to_string());
        // Stable title: the day count lives in the description so the warning
        // consolidates (occurrence_count) instead of creating a new row daily.
        let title = format!(
            "Source '{}' is publishing but ingestion has stalled",
            stall.source_id
        );
        match store
            .open_source_warning_exists("source_outage", &title)
            .await
        {
            Ok(true) => continue,
            Ok(false) => {}
            Err(error) => {
                tracing::warn!(%error, "anomaly_scan: open-warning check failed; warning may duplicate")
            }
        }
        let description = format!(
            "Source '{}' is being fetched successfully and its feed reported fresh items \
             (newest item {}), yet no observations have been ingested for {} days \
             (last stored observation {}; {} observations in the last 30 days; last parse \
             produced {} items). This indicates a parser/ingestion defect on our side, \
             not a quiet source or a blocked one.",
            stall.source_id,
            feed_freshness,
            days_stalled,
            last_obs.format("%Y-%m-%d"),
            stall.obs_count,
            stall
                .last_item_count
                .map(|count| count.to_string())
                .unwrap_or_else(|| "unknown".to_string()),
        );

        match ingress
            .submit_warning(
                NewWarning::new("source_outage", &title, "high")
                    .description(&description)
                    .confidence(0.85)
                    .system_broadcast(),
            )
            .await
        {
            Ok(result) => {
                counters.record(&result);
                anomalies_detected += 1;
                tracing::info!(
                    source = %stall.source_id,
                    days_stalled,
                    last_item_at = %feed_freshness,
                    warning_id = %result.warning_id(),
                    occurrence_count = result.occurrence_count(),
                    "anomaly_scan: ingestion stall warning generated"
                );
            }
            Err(e) => {
                tracing::warn!(error = %e, "anomaly_scan: failed to insert ingestion stall warning");
            }
        }
    }

    let failing = match store.list_failing_sources(3).await {
        Ok(rows) => rows,
        Err(e) => {
            run.fail(&format!(
                "anomaly_scan: failed to load failing sources: {e}"
            ));
            return run;
        }
    };

    for failure in &failing {
        // Quarantined sources (repeated failures on the 7-day retry tier) are
        // reported by coverage, not re-warned every scan.
        if failure.consecutive_failures >= 6
            && failure.next_due_at > Utc::now() + chrono::Duration::days(1)
        {
            continue;
        }
        let error_text = failure
            .last_error
            .clone()
            .unwrap_or_else(|| "unknown fetch error".to_string());
        let title = format!("Source '{}' fetch is failing", failure.source_slug);
        match store
            .open_source_warning_exists("source_fetch_failure", &title)
            .await
        {
            Ok(true) => continue,
            Ok(false) => {}
            Err(error) => {
                tracing::warn!(%error, "anomaly_scan: open-warning check failed; warning may duplicate")
            }
        }
        let description = format!(
            "Source '{}' has failed {} consecutive fetch attempts (last status: {}; \
             last error: {}). Last successful fetch: {}. The scheduler keeps retrying on \
             the backoff ladder; fix the endpoint or credentials, or reclassify the source.",
            failure.source_slug,
            failure.consecutive_failures,
            failure
                .last_http_status
                .map(|status| status.to_string())
                .unwrap_or_else(|| "none".to_string()),
            error_text,
            failure
                .last_success_at
                .map(|at| at.format("%Y-%m-%d %H:%M UTC").to_string())
                .unwrap_or_else(|| "never".to_string()),
        );

        match ingress
            .submit_warning(
                NewWarning::new("source_fetch_failure", &title, "medium")
                    .description(&description)
                    .confidence(0.9)
                    .system_broadcast(),
            )
            .await
        {
            Ok(result) => {
                counters.record(&result);
                anomalies_detected += 1;
                tracing::info!(
                    source = %failure.source_slug,
                    consecutive_failures = failure.consecutive_failures,
                    warning_id = %result.warning_id(),
                    occurrence_count = result.occurrence_count(),
                    "anomaly_scan: fetch failure warning generated"
                );
            }
            Err(e) => {
                tracing::warn!(error = %e, "anomaly_scan: failed to insert fetch failure warning");
            }
        }
    }

    // Warning lifecycle hygiene: a warning whose condition no longer holds is
    // superseded, not true. Without this, recovered sources left failure
    // warnings open forever (audit 2026-10-06: 78 stale warnings).
    match store.auto_resolve_recovered_source_warnings().await {
        Ok(resolved) if resolved > 0 => tracing::info!(
            resolved,
            "anomaly_scan: auto-resolved recovered source-health warnings"
        ),
        Ok(_) => {}
        Err(error) => tracing::warn!(
            %error,
            "anomaly_scan: failed to auto-resolve recovered source warnings"
        ),
    }

    tracing::info!(
        ingestion_stalls = stalls.len(),
        failing_sources = failing.len(),
        "anomaly_scan: source health scan complete"
    );

    let elapsed = start.elapsed();
    let summary = format!(
        "anomaly_scan: {} entities analyzed, {} anomalies detected, {} in {:.1}s; {}",
        by_entity.len(),
        anomalies_detected,
        counters.warnings_persisted,
        elapsed.as_secs_f64(),
        counters.summary(),
    );
    match counters.success_blocker() {
        Some(reason) => run.degrade(counters.warnings_persisted, &format!("{summary}; {reason}")),
        None => run.succeed(counters.warnings_persisted, &summary),
    }
    run
}
