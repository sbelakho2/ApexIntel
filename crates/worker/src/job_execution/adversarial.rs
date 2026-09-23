//! Adversarial Analysis handler.
//!
//! Scheduled every 6 hours: runs placement clustering, source entropy detection,
//! and quarantine management. Detects coordinated misinformation campaigns,
//! adversarial content placement patterns, and manages suspicious sources.
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

use uuid::Uuid;

use crate::*;

/// Execute AdversarialAnalysis job: placement clustering, source entropy, quarantine.
pub(super) async fn run_adversarial_analysis(kind: &JobKind, store: &Arc<PgStore>) -> JobRun {
    let mut run = JobRun::new(kind.clone());
    run.start();
    let total_start = Instant::now();

    let mut placements_detected: u64 = 0;
    let mut sources_quarantined: u64 = 0;
    let mut entropy_alerts: u64 = 0;

    // 1. Run placement clustering: detect coordinated content placement patterns
    match run_placement_clustering(store).await {
        Ok(count) => placements_detected = count,
        Err(e) => {
            tracing::warn!(error = %e, "adversarial_analysis: placement clustering failed");
        }
    }

    // 2. Run source entropy detection: identify suspicious source behavior patterns
    match run_source_entropy_detection(store).await {
        Ok(count) => entropy_alerts = count,
        Err(e) => {
            tracing::warn!(error = %e, "adversarial_analysis: source entropy detection failed");
        }
    }

    // 3. Run quarantine management: release expired items, escalate persistent threats
    match run_quarantine_management(store).await {
        Ok(count) => sources_quarantined = count,
        Err(e) => {
            tracing::warn!(error = %e, "adversarial_analysis: quarantine management failed");
        }
    }

    let elapsed = total_start.elapsed();
    run.succeed(
        placements_detected + entropy_alerts + sources_quarantined,
        &format!(
            "adversarial_analysis: {} placement clusters, {} entropy alerts, \
             {} sources quarantined in {:.1}s",
            placements_detected,
            entropy_alerts,
            sources_quarantined,
            elapsed.as_secs_f64(),
        ),
    );
    run
}

/// Detect coordinated content placement patterns across sources.
/// Looks for clusters of similar content appearing across multiple sources
/// within a short time window (potential coordinated disinformation).
async fn run_placement_clustering(store: &Arc<PgStore>) -> Result<u64, String> {
    // Load recent observations grouped by time windows
    let since = chrono::Utc::now() - chrono::Duration::hours(24);
    let rows = sqlx::query(
        r#"SELECT
               o.id,
               o.ts_utc,
               o.observation_type,
               COALESCE(o.value->>'title', o.value->>'content', o.value->>'description', '') AS content,
               COALESCE(o.value->>'url', o.value->>'source_url', '') AS source_url,
               o.entity_id,
               o.confidence
           FROM observations o
           WHERE o.created_at >= $1
             AND COALESCE(o.value->>'title', '') <> ''
           ORDER BY o.ts_utc DESC
           LIMIT 2000"#,
    )
    .bind(since)
    .fetch_all(&store.pool)
    .await
    .map_err(|e| format!("placement clustering query failed: {e}"))?;

    if rows.is_empty() {
        return Ok(0);
    }

    // Build token sets per observation for Jaccard similarity
    #[allow(clippy::unwrap_used, clippy::expect_used)]
    let obs_tokens: Vec<(Uuid, Vec<String>, chrono::DateTime<chrono::Utc>, String)> = {
        use sqlx::Row;
        rows.iter()
            .filter_map(|r| {
                let id: Uuid = r.try_get("id").ok()?;
                let content: String = r.try_get("content").ok()?;
                let source_url: String = r.try_get("source_url").ok().unwrap_or_default();
                let ts: chrono::DateTime<chrono::Utc> = r.try_get("ts_utc").ok()?;

                if content.len() < 40 {
                    return None;
                }

                let tokens: Vec<String> = content
                    .to_lowercase()
                    .split_whitespace()
                    .filter(|w| w.len() > 3)
                    .map(|w| w.to_string())
                    .collect();

                Some((id, tokens, ts, source_url))
            })
            .collect()
    };

    let mut clusters_found: u64 = 0;
    let mut processed = HashSet::new();

    for i in 0..obs_tokens.len() {
        if processed.contains(&obs_tokens[i].0) {
            continue;
        }

        let mut cluster = Vec::new();
        let mut source_domains = HashSet::new();
        cluster.push(&obs_tokens[i]);

        for j in (i + 1)..obs_tokens.len() {
            if processed.contains(&obs_tokens[j].0) {
                continue;
            }

            let time_diff = (obs_tokens[i].2 - obs_tokens[j].2).num_hours().abs();
            if time_diff > 6 {
                continue;
            }

            let similarity = jaccard_similarity(&obs_tokens[i].1, &obs_tokens[j].1);
            if similarity > 0.5 {
                cluster.push(&obs_tokens[j]);
                if !obs_tokens[j].3.is_empty() {
                    source_domains.insert(extract_domain_from_url(&obs_tokens[j].3));
                }
            }
        }

        if cluster.len() >= 3 && source_domains.len() >= 2 {
            // Coordinated placement detected
            let signal_ids: Vec<Uuid> = cluster.iter().map(|c| c.0).collect();
            let now = chrono::Utc::now();

            let placement_id = Uuid::new_v4();
            #[allow(clippy::unwrap_used, clippy::expect_used)]
            let value = serde_json::json!({
                "placement_id": placement_id.to_string(),
                "source_count": cluster.len(),
                "time_window_hours": 6,
                "token_jaccard": 0.55,
                "signal_ids": signal_ids.iter().map(|id| id.to_string()).collect::<Vec<_>>(),
                "source_domains": source_domains.iter().collect::<Vec<_>>(),
                "detected_at": now.to_rfc3339(),
            });

            let _ = sqlx::query(
                r#"INSERT INTO pattern_candidates
                   (id, pattern_type, entity_type, confidence, source_observations, metadata, created_at)
                   VALUES ($1, 'adversarial_placement', 'system', $2, $3, $4, $5)"#,
            )
            .bind(placement_id)
            .bind(0.55 + (cluster.len() as f64 * 0.05).min(0.2))
            .bind(signal_ids.len() as i32)
            .bind(serde_json::to_value(&signal_ids).unwrap_or_default())
            .bind(value)
            .bind(now)
            .execute(&store.pool)
            .await
            .map_err(|e| format!("failed to store placement cluster: {e}"))?;

            // Generate warning for significant clusters
            if cluster.len() >= 5 {
                let title = format!(
                    "Adversarial Placement: {} signals from {} sources",
                    cluster.len(),
                    source_domains.len()
                );
                let description = format!(
                    "Detected {} similar observations across {} distinct source domains \
                     within 6 hours (Jaccard similarity >= 0.5). Possible coordinated \
                     content placement campaign. Sources: {}",
                    cluster.len(),
                    source_domains.len(),
                    source_domains
                        .iter()
                        .take(5)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", "),
                );

                let _ = store
                    .insert_warning(
                        "adversarial",
                        &title,
                        Some(&description),
                        if cluster.len() >= 10 {
                            "critical"
                        } else {
                            "high"
                        },
                        None,
                        None,
                        None,
                        None,
                        Some(0.80),
                    )
                    .await;
            }

            for item in &cluster {
                processed.insert(item.0);
            }
            clusters_found += 1;
        }
    }

    Ok(clusters_found)
}

/// Detect anomalous source behavior via entropy analysis.
/// High entropy sources (unusual posting patterns, erratic domain behavior)
/// are flagged for quarantine review.
async fn run_source_entropy_detection(store: &Arc<PgStore>) -> Result<u64, String> {
    let since = chrono::Utc::now() - chrono::Duration::days(7);

    // Get source posting statistics
    let rows = sqlx::query(
        r#"SELECT
               COALESCE(o.value->>'source_domain',
                        o.value->>'url',
                        o.value->>'source_url',
                        'unknown') AS source_domain,
               COUNT(*)::integer AS observation_count,
               COUNT(DISTINCT o.observation_type)::integer AS type_diversity,
               COUNT(DISTINCT o.entity_id)::integer AS entity_span,
               AVG(o.confidence)::float AS avg_confidence
           FROM observations o
           WHERE o.created_at >= $1
             AND COALESCE(o.value->>'source_domain', o.value->>'url', '') <> ''
           GROUP BY o.value->>'source_domain', o.value->>'url', o.value->>'source_url'
           HAVING COUNT(*) >= 5
           ORDER BY COUNT(*) DESC
           LIMIT 200"#,
    )
    .bind(since)
    .fetch_all(&store.pool)
    .await
    .map_err(|e| format!("entropy detection query failed: {e}"))?;

    let mut entropy_alerts: u64 = 0;
    let now = chrono::Utc::now();

    for row in &rows {
        use sqlx::Row;
        let source_domain: String = row
            .try_get::<String, _>("source_domain")
            .unwrap_or_else(|_| "unknown".to_string());
        let obs_count: i32 = row.try_get("observation_count").unwrap_or(0);
        let type_diversity: i32 = row.try_get("type_diversity").unwrap_or(0);
        let entity_span: i32 = row.try_get("entity_span").unwrap_or(0);
        let avg_confidence: f64 = row.try_get::<f64, _>("avg_confidence").unwrap_or(0.5);

        // Entropy heuristics: flag sources with unusual patterns
        let is_high_volume_single_type = obs_count > 50 && type_diversity <= 2;
        let is_wide_entity_span = entity_span > 20;
        let is_low_confidence_spam = obs_count > 20 && avg_confidence < 0.4;
        let is_suspicious =
            is_high_volume_single_type || is_wide_entity_span || is_low_confidence_spam;

        if is_suspicious {
            let reason = if is_high_volume_single_type {
                format!(
                    "High volume ({obs_count} observations) with low type diversity ({type_diversity} types)"
                )
            } else if is_wide_entity_span {
                format!(
                    "Unusually wide entity span ({entity_span} entities across {obs_count} observations)"
                )
            } else {
                format!(
                    "Low confidence spam pattern ({obs_count} observations at avg {:.0}% confidence)",
                    avg_confidence * 100.0
                )
            };

            let quarantine_id = Uuid::new_v4();
            let release_at = now + chrono::Duration::hours(24);

            #[allow(clippy::unwrap_used, clippy::expect_used)]
            let value = serde_json::json!({
                "quarantine_id": quarantine_id.to_string(),
                "source_domain": source_domain,
                "reason": reason,
                "observation_count": obs_count,
                "type_diversity": type_diversity,
                "entity_span": entity_span,
                "avg_confidence": avg_confidence,
                "quarantined_at": now.to_rfc3339(),
                "release_at": release_at.to_rfc3339(),
            });

            let _ = sqlx::query(
                r#"INSERT INTO observations
                   (id, observation_type, entity_id, entity_type, ts_utc, value, provenance, confidence)
                   VALUES ($1, 'source_quarantine', NULL, 'system', $2, $3::jsonb, $4::jsonb, $5)"#,
            )
            .bind(quarantine_id)
            .bind(now)
            .bind(value)
            .bind(serde_json::json!({"source": "worker_adversarial_analysis", "method": "entropy_detection"}))
            .bind(0.75)
            .execute(&store.pool)
            .await
            .map_err(|e| format!("failed to store entropy alert: {e}"))?;

            // Update source reliability stats
            let _ = sqlx::query(
                r#"INSERT INTO source_reliability_stats
                   (source_domain, reliability_tier, observation_count, false_positive_rate,
                    last_updated, metadata)
                   VALUES ($1, 'UnderReview', $2, 0.0, $3, $4)
                   ON CONFLICT (source_domain) DO UPDATE
                   SET reliability_tier = 'UnderReview',
                       observation_count = $2,
                       last_updated = $3,
                       metadata = $4"#,
            )
            .bind(&source_domain)
            .bind(obs_count)
            .bind(now)
            .bind(serde_json::json!({"flagged_by": "entropy_detection", "reason": reason}))
            .execute(&store.pool)
            .await
            .map_err(|e| format!("failed to update source reliability: {e}"))?;

            entropy_alerts += 1;
        }
    }

    Ok(entropy_alerts)
}

/// Manage quarantine queue: release expired items, verify persistent threats.
async fn run_quarantine_management(store: &Arc<PgStore>) -> Result<u64, String> {
    let now = chrono::Utc::now();
    let mut managed: u64 = 0;

    // 1. Release expired quarantine entries
    let expired = sqlx::query(
        r#"UPDATE observations
           SET value = value || jsonb_build_object('status', 'released', 'released_at', $2::text)
           WHERE observation_type = 'source_quarantine'
             AND value->>'status' IS NULL
             AND (value->>'release_at')::timestamptz < $1
           RETURNING id"#,
    )
    .bind(now)
    .bind(now.to_rfc3339())
    .fetch_all(&store.pool)
    .await
    .map_err(|e| format!("quarantine release query failed: {e}"))?;

    if !expired.is_empty() {
        use sqlx::Row;
        for row in &expired {
            if let Ok(id) = row.try_get::<Uuid, _>("id") {
                tracing::info!(quarantine_id = %id, "adversarial_analysis: released expired quarantine");
            }
        }
        managed += expired.len() as u64;
    }

    // 2. Escalate persistent quarantine entries (still active after 48h)
    let persistent = sqlx::query(
        r#"SELECT id, value
           FROM observations
           WHERE observation_type = 'source_quarantine'
             AND value->>'status' IS NULL
             AND created_at < $1"#,
    )
    .bind(now - chrono::Duration::hours(48))
    .fetch_all(&store.pool)
    .await
    .map_err(|e| format!("persistent quarantine query failed: {e}"))?;

    for row in &persistent {
        use sqlx::Row;
        let _id: Uuid = match row.try_get("id") {
            Ok(v) => v,
            Err(_) => continue,
        };
        let value: serde_json::Value = match row.try_get("value") {
            Ok(v) => v,
            Err(_) => continue,
        };

        let source_domain = value
            .get("source_domain")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let reason = value
            .get("reason")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        let title = format!("Persistent Quarantine: {}", source_domain);
        let description = format!(
            "Source '{}' has been in quarantine for >48h. Reason: {}. \
             Manual review recommended.",
            source_domain, reason
        );

        let _ = store
            .insert_warning(
                "adversarial",
                &title,
                Some(&description),
                "high",
                None,
                None,
                None,
                None,
                Some(0.85),
            )
            .await;

        managed += 1;
    }

    Ok(managed)
}

/// Compute Jaccard similarity between two token sets.
fn jaccard_similarity(a: &[String], b: &[String]) -> f64 {
    let set_a: HashSet<&String> = a.iter().collect();
    let set_b: HashSet<&String> = b.iter().collect();

    let intersection = set_a.intersection(&set_b).count();
    let union = set_a.union(&set_b).count();

    if union == 0 {
        0.0
    } else {
        intersection as f64 / union as f64
    }
}

/// Extract domain from URL for source grouping.
fn extract_domain_from_url(url: &str) -> String {
    url.trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .split('/')
        .next()
        .unwrap_or("unknown")
        .split(':')
        .next()
        .unwrap_or("unknown")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jaccard_identical() {
        let a = vec!["hello".to_string(), "world".to_string()];
        let b = vec!["hello".to_string(), "world".to_string()];
        assert!((jaccard_similarity(&a, &b) - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_jaccard_disjoint() {
        let a = vec!["apple".to_string()];
        let b = vec!["orange".to_string()];
        assert_eq!(jaccard_similarity(&a, &b), 0.0);
    }

    #[test]
    fn test_jaccard_partial() {
        let a = vec!["hello".to_string(), "world".to_string(), "foo".to_string()];
        let b = vec!["hello".to_string(), "world".to_string(), "bar".to_string()];
        assert!((jaccard_similarity(&a, &b) - 0.5).abs() < 0.01);
    }

    #[test]
    fn test_extract_domain() {
        assert_eq!(
            extract_domain_from_url("https://example.com/path"),
            "example.com"
        );
        assert_eq!(
            extract_domain_from_url("http://sub.example.co.uk:8080/path"),
            "sub.example.co.uk"
        );
    }
}
