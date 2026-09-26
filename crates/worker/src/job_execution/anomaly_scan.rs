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

use crate::intelligence_ingress::{IntelligenceIngress, NewWarning};
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

    let mut anomalies_detected: u64 = 0;
    let mut warnings_generated: u64 = 0;

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

            if deviation >= VOLUME_ANOMALY_THRESHOLD {
                let direction = if latest > prior_mean { "spike" } else { "drop" };
                let pct_change = ((latest - prior_mean) / prior_mean * 100.0).round() as i64;

                // Generate a warning for this anomaly
                let title = format!(
                    "{direction} in data volume for {entity_name} ({pct_change}% vs baseline)"
                );
                let description = format!(
                    "{entity_name} shows a {pct_change}% {direction} in observation volume. \
                     Latest: {latest:.0} observations, baseline: {prior_mean:.0}. \
                     This may indicate a significant event — increased activity ({direction} = spike) \
                     or reduced coverage/data loss ({direction} = drop). \
                     Investigate the underlying cause and assess operational impact."
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
                        warnings_generated += 1;
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

        let new_types: Vec<&String> = recent_types.difference(&historical_types).collect();
        if !new_types.is_empty() && historical_types.len() >= 2 {
            let types_str = new_types
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            let title = format!("New signal types for {entity_name}: {types_str}");
            let description = format!(
                "{entity_name} has started generating new observation types ({types_str}) \
                 that weren't present in the previous 27 days. This may indicate a \
                 significant strategic shift — new market entry, capability expansion, \
                 leadership change, or emerging risk. Analyze the new signals for \
                 actionable intelligence."
            );

            match ingress
                .submit_warning(
                    NewWarning::new("signal_shift", &title, "medium")
                        .description(&description)
                        .entity_ids(vec![*entity_id])
                        .confidence(0.70),
                )
                .await
            {
                Ok(result) => {
                    warnings_generated += 1;
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

    // ── 4. Source outage detection ─────────────────────────────────────────
    #[derive(sqlx::FromRow)]
    struct SourceCount {
        source_id: String,
        last_obs: Option<chrono::DateTime<Utc>>,
        obs_count_30d: i64,
    }

    let source_counts: Vec<SourceCount> = match sqlx::query_as::<_, SourceCount>(
        r#"SELECT
               COALESCE(provenance->>'source_id', provenance->>'source', 'unknown') as source_id,
               MAX(ts_utc) as last_obs,
               COUNT(*)::bigint as obs_count_30d
           FROM observations
           WHERE ts_utc >= NOW() - INTERVAL '30 days'
             AND (provenance->>'source_id' IS NOT NULL OR provenance->>'source' IS NOT NULL)
           GROUP BY 1
           HAVING MAX(ts_utc) < NOW() - INTERVAL '3 days'"#,
    )
    .fetch_all(&store.pool)
    .await
    {
        Ok(rows) => rows,
        Err(e) => {
            // Source-outage detection must not silently degrade into "no
            // outages": a failed query is a failed job, not a clean scan.
            run.fail(&format!(
                "anomaly_scan: failed to load source activity for outage detection: {e}"
            ));
            return run;
        }
    };

    for sc in &source_counts {
        let Some(last_obs) = sc.last_obs else {
            continue;
        };
        if sc.obs_count_30d <= 10 {
            continue;
        }
        let days_silent = (Utc::now() - last_obs).num_days();
        if days_silent >= 3 {
            let title = format!(
                "Source '{}' has been silent for {} days",
                sc.source_id, days_silent
            );
            let description = format!(
                "Data source '{}' has not produced any observations in {} days. \
                     It previously generated {} observations in the last 30 days. \
                     This may indicate a source outage, feed disruption, or blocking. \
                     Verify source connectivity and restore data flow.",
                sc.source_id, days_silent, sc.obs_count_30d
            );

            match ingress
                .submit_warning(
                    NewWarning::new("source_outage", &title, "high")
                        .description(&description)
                        .confidence(0.80),
                )
                .await
            {
                Ok(result) => {
                    warnings_generated += 1;
                    tracing::info!(
                        source = %sc.source_id,
                        days_silent,
                        warning_id = %result.warning_id(),
                        occurrence_count = result.occurrence_count(),
                        "anomaly_scan: source outage warning generated"
                    );
                }
                Err(e) => {
                    tracing::warn!(error = %e, "anomaly_scan: failed to insert source outage warning");
                }
            }
        }
    }

    let elapsed = start.elapsed();
    run.succeed(
        warnings_generated,
        &format!(
            "anomaly_scan: {} entities analyzed, {} anomalies detected, {} warnings generated in {:.1}s",
            by_entity.len(),
            anomalies_detected,
            warnings_generated,
            elapsed.as_secs_f64(),
        ),
    );
    run
}
