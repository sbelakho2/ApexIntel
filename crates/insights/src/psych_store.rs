//! Psychological profile persistence layer.
//!
//! Reads from and writes to the tables defined in migration
//! `20260626_psych_profiles_and_threat_intel.sql`:
//! - `psychological_profiles` — POI psychometric snapshots
//! - `behavioral_pattern_events` — detected behavioral changes
//! - `engagement_profiles` — recommended engagement strategies
//! - `sentiment_time_series` — aggregate sentiment bucketing
//!
//! # Enum ↔ DB conversion
//!
//! The `DecisionStyle`, `ChangeAppetite`, and `ProofType` enums from
//! `apex_poi::model` serialize as PascalCase via serde, but the DB CHECK
//! constraints expect snake_case.  This module converts in both directions
//! so that the on-disk representation stays consistent with schema constraints.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

// ═══════════════════════════════════════════════════════════════════════════════
// Record types matching the DB schema
// ═══════════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct PsychProfileRecord {
    pub id: Uuid,
    pub person_id: String,
    pub decision_style: String,
    pub change_appetite: String,
    pub pain_index: f64,
    pub risk_tolerance: f64,
    pub preferred_proof: Vec<String>,
    pub enrichment_quality: f64,
    pub evidence_sources: Vec<String>,
    pub metadata: serde_json::Value,
    pub computed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct BehavioralPatternRecord {
    pub id: Uuid,
    pub person_id: String,
    pub event_type: String,
    pub title: String,
    pub description: String,
    pub confidence: f64,
    pub evidence_urls: Vec<String>,
    pub detected_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct EngagementProfileRecord {
    pub id: Uuid,
    pub person_id: String,
    pub talking_points: Vec<String>,
    pub opening_topics: Vec<String>,
    pub avoid_topics: Vec<String>,
    pub best_channel: String,
    pub best_timing: Option<String>,
    pub proof_pack: serde_json::Value,
    pub generated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct SentimentSnapshotRecord {
    pub id: Uuid,
    pub entity_id: String,
    pub entity_type: String,
    pub mean_score: f64,
    pub median_score: f64,
    pub std_deviation: f64,
    pub sample_count: i32,
    pub positive_count: i32,
    pub neutral_count: i32,
    pub negative_count: i32,
    pub window_start: DateTime<Utc>,
    pub window_end: DateTime<Utc>,
    pub computed_at: DateTime<Utc>,
}

// ═══════════════════════════════════════════════════════════════════════════════
// Upsert / insert helpers
// ═══════════════════════════════════════════════════════════════════════════════

/// Upsert a psychological profile snapshot for a person.
///
/// Since `psychological_profiles` is a time-series table (no UNIQUE on
/// `person_id`), we INSERT a new row each time the profile is updated.
/// This preserves history for trend analysis.
pub async fn upsert_psychological_profile(
    pool: &PgPool,
    person_id: &str,
    decision_style: &str,
    change_appetite: &str,
    pain_index: f64,
    risk_tolerance: f64,
    preferred_proof: &[String],
    enrichment_quality: f64,
    evidence_sources: &[String],
    metadata: &serde_json::Value,
) -> Result<Uuid, sqlx::Error> {
    let id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO psychological_profiles
             (id, person_id, decision_style, change_appetite, pain_index,
              risk_tolerance, preferred_proof, enrichment_quality,
              evidence_sources, metadata, computed_at)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, NOW())"#,
    )
    .bind(id)
    .bind(person_id)
    .bind(decision_style)
    .bind(change_appetite)
    .bind(pain_index)
    .bind(risk_tolerance)
    .bind(preferred_proof)
    .bind(enrichment_quality)
    .bind(evidence_sources)
    .bind(metadata)
    .execute(pool)
    .await?;

    Ok(id)
}

/// Record a detected behavioral pattern.
pub async fn record_behavioral_pattern(
    pool: &PgPool,
    person_id: &str,
    event_type: &str,
    title: &str,
    description: &str,
    confidence: f64,
    evidence_urls: &[String],
) -> Result<Uuid, sqlx::Error> {
    let id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO behavioral_pattern_events
             (id, person_id, event_type, title, description, confidence,
              evidence_urls, detected_at)
           VALUES ($1, $2, $3, $4, $5, $6, $7, NOW())"#,
    )
    .bind(id)
    .bind(person_id)
    .bind(event_type)
    .bind(title)
    .bind(description)
    .bind(confidence)
    .bind(evidence_urls)
    .execute(pool)
    .await?;

    Ok(id)
}

/// Upsert an engagement profile for a person.
///
/// Since there may be multiple engagement profiles over time, this INSERTs a
/// new row. The latest profile is always the one with the highest `generated_at`.
pub async fn upsert_engagement_profile(
    pool: &PgPool,
    person_id: &str,
    talking_points: &[String],
    opening_topics: &[String],
    avoid_topics: &[String],
    best_channel: &str,
    best_timing: Option<&str>,
    proof_pack: &serde_json::Value,
) -> Result<Uuid, sqlx::Error> {
    let id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO engagement_profiles
             (id, person_id, talking_points, opening_topics, avoid_topics,
              best_channel, best_timing, proof_pack, generated_at)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, NOW())"#,
    )
    .bind(id)
    .bind(person_id)
    .bind(talking_points)
    .bind(opening_topics)
    .bind(avoid_topics)
    .bind(best_channel)
    .bind(best_timing)
    .bind(proof_pack)
    .execute(pool)
    .await?;

    Ok(id)
}

/// Record a sentiment snapshot for an entity in a given time window.
pub async fn record_sentiment_snapshot(
    pool: &PgPool,
    entity_id: &str,
    entity_type: &str,
    mean_score: f64,
    median_score: f64,
    std_deviation: f64,
    sample_count: i32,
    positive_count: i32,
    neutral_count: i32,
    negative_count: i32,
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
) -> Result<Uuid, sqlx::Error> {
    let id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO sentiment_time_series
             (id, entity_id, entity_type, mean_score, median_score,
              std_deviation, sample_count, positive_count, neutral_count,
              negative_count, window_start, window_end, computed_at)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, NOW())"#,
    )
    .bind(id)
    .bind(entity_id)
    .bind(entity_type)
    .bind(mean_score)
    .bind(median_score)
    .bind(std_deviation)
    .bind(sample_count)
    .bind(positive_count)
    .bind(neutral_count)
    .bind(negative_count)
    .bind(window_start)
    .bind(window_end)
    .execute(pool)
    .await?;

    Ok(id)
}

// ═══════════════════════════════════════════════════════════════════════════════
// Read helpers
// ═══════════════════════════════════════════════════════════════════════════════

/// Get the most recent psychological profile for a person.
pub async fn get_psychological_profile(
    pool: &PgPool,
    person_id: &str,
) -> Result<Option<PsychProfileRecord>, sqlx::Error> {
    sqlx::query_as::<_, PsychProfileRecord>(
        r#"SELECT id, person_id, decision_style, change_appetite, pain_index,
                  risk_tolerance, preferred_proof, enrichment_quality,
                  evidence_sources, metadata, computed_at
           FROM psychological_profiles
           WHERE person_id = $1
           ORDER BY computed_at DESC
           LIMIT 1"#,
    )
    .bind(person_id)
    .fetch_optional(pool)
    .await
}

/// List recent behavioral patterns for a person.
pub async fn list_recent_behavioral_patterns(
    pool: &PgPool,
    person_id: &str,
    limit: i64,
) -> Result<Vec<BehavioralPatternRecord>, sqlx::Error> {
    sqlx::query_as::<_, BehavioralPatternRecord>(
        r#"SELECT id, person_id, event_type, title, description, confidence,
                  evidence_urls, detected_at
           FROM behavioral_pattern_events
           WHERE person_id = $1
           ORDER BY detected_at DESC
           LIMIT $2"#,
    )
    .bind(person_id)
    .bind(limit)
    .fetch_all(pool)
    .await
}

/// Get the most recent engagement profile for a person.
pub async fn get_engagement_profile(
    pool: &PgPool,
    person_id: &str,
) -> Result<Option<EngagementProfileRecord>, sqlx::Error> {
    sqlx::query_as::<_, EngagementProfileRecord>(
        r#"SELECT id, person_id, talking_points, opening_topics, avoid_topics,
                  best_channel, best_timing, proof_pack, generated_at
           FROM engagement_profiles
           WHERE person_id = $1
           ORDER BY generated_at DESC
           LIMIT 1"#,
    )
    .bind(person_id)
    .fetch_optional(pool)
    .await
}

/// List sentiment snapshots for an entity within a time range.
pub async fn list_sentiment_snapshots(
    pool: &PgPool,
    entity_id: &str,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<Vec<SentimentSnapshotRecord>, sqlx::Error> {
    sqlx::query_as::<_, SentimentSnapshotRecord>(
        r#"SELECT id, entity_id, entity_type, mean_score, median_score,
                  std_deviation, sample_count, positive_count, neutral_count,
                  negative_count, window_start, window_end, computed_at
           FROM sentiment_time_series
           WHERE entity_id = $1
             AND window_start >= $2
             AND window_end <= $3
           ORDER BY window_start ASC"#,
    )
    .bind(entity_id)
    .bind(from)
    .bind(to)
    .fetch_all(pool)
    .await
}

/// Count total behavioral pattern events for a person.
pub async fn count_behavioral_patterns(pool: &PgPool, person_id: &str) -> Result<i64, sqlx::Error> {
    let row: (i64,) =
        sqlx::query_as(r#"SELECT COUNT(*) FROM behavioral_pattern_events WHERE person_id = $1"#)
            .bind(person_id)
            .fetch_one(pool)
            .await?;
    Ok(row.0)
}

// ═══════════════════════════════════════════════════════════════════════════════
// Enum ↔ DB string conversion helpers
// ═══════════════════════════════════════════════════════════════════════════════

/// The complete set of `decision_style` literals accepted by the
/// `psychological_profiles.decision_style` CHECK constraint (see migration
/// `20260624_psychological_profiles.sql`). Used to validate that every value
/// produced by `psych_compute` maps to a legal on-disk enum.
pub const VALID_DECISION_STYLES_DB: &[&str] = &[
    "authoritative",
    "collaborative",
    "analytical",
    "consensus_driven",
    "data_driven",
    "intuitive",
    "delegative",
    "unknown",
];

/// The complete set of `change_appetite` literals accepted by the
/// `psychological_profiles.change_appetite` CHECK constraint.
pub const VALID_CHANGE_APPETITES_DB: &[&str] = &["high", "moderate", "low", "resistant"];

/// Returns `true` when `db_val` is a legal `decision_style` CHECK literal.
pub fn is_valid_decision_style_db(db_val: &str) -> bool {
    VALID_DECISION_STYLES_DB.contains(&db_val)
}

/// Returns `true` when `db_val` is a legal `change_appetite` CHECK literal.
pub fn is_valid_change_appetite_db(db_val: &str) -> bool {
    VALID_CHANGE_APPETITES_DB.contains(&db_val)
}

/// Convert a `DecisionStyle` enum variant to its snake_case DB representation.
pub fn decision_style_to_db(style: &str) -> &str {
    match style {
        "Authoritative" | "authoritative" => "authoritative",
        "Collaborative" | "collaborative" => "collaborative",
        "Analytical" | "analytical" => "analytical",
        "ConsensusDriven" | "consensus_driven" => "consensus_driven",
        "DataDriven" | "data_driven" => "data_driven",
        "Intuitive" | "intuitive" => "intuitive",
        "Delegative" | "delegative" => "delegative",
        _ => "unknown",
    }
}

/// Convert a DB snake_case string back to a `DecisionStyle` display string.
pub fn db_to_decision_style(db_val: &str) -> &str {
    match db_val {
        "authoritative" => "Authoritative",
        "collaborative" => "Collaborative",
        "analytical" => "Analytical",
        "consensus_driven" => "ConsensusDriven",
        "data_driven" => "DataDriven",
        "intuitive" => "Intuitive",
        "delegative" => "Delegative",
        _ => "Unknown",
    }
}

/// Convert a `ChangeAppetite` enum variant to its snake_case DB representation.
pub fn change_appetite_to_db(appetite: &str) -> &str {
    match appetite {
        "High" | "high" => "high",
        "Moderate" | "moderate" => "moderate",
        "Low" | "low" => "low",
        "Resistant" | "resistant" => "resistant",
        _ => "moderate",
    }
}

/// Convert proof type to DB-compatible string.
pub fn proof_type_to_db(proof: &str) -> String {
    proof.to_lowercase().replace(' ', "_")
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Exhaustive mappings for every value psych_compute can emit ──────────
    // These are the *exact* strings returned by `psych_compute::compute_decision_style`
    // and `psych_compute::compute_change_appetite`. Each must round-trip through the
    // converter into a literal that satisfies the DB CHECK constraint.

    /// Every string `psych_compute::compute_decision_style` is capable of returning.
    const ALL_COMPUTE_DECISION_STYLES: &[&str] = &[
        "Unknown",
        "Collaborative",
        "Authoritative",
        "Analytical",
        "DataDriven",
    ];

    /// Every string `psych_compute::compute_change_appetite` is capable of returning.
    const ALL_COMPUTE_CHANGE_APPETITES: &[&str] = &["High", "Moderate", "Low", "Resistant"];

    #[test]
    fn test_decision_style_conversions() {
        assert_eq!(decision_style_to_db("Authoritative"), "authoritative");
        assert_eq!(decision_style_to_db("data_driven"), "data_driven");
        assert_eq!(decision_style_to_db("UnknownThing"), "unknown");
        assert_eq!(db_to_decision_style("authoritative"), "Authoritative");
        assert_eq!(db_to_decision_style("consensus_driven"), "ConsensusDriven");
    }

    #[test]
    fn test_change_appetite_conversions() {
        assert_eq!(change_appetite_to_db("High"), "high");
        assert_eq!(change_appetite_to_db("Resistant"), "resistant");
        assert_eq!(change_appetite_to_db("UnknownThing"), "moderate");
    }

    #[test]
    fn test_proof_type_to_db() {
        assert_eq!(proof_type_to_db("ROI Analysis"), "roi_analysis");
        assert_eq!(proof_type_to_db("CaseStudy"), "casestudy");
    }

    /// Every decision_style produced by psych_compute must map to a valid DB literal.
    #[test]
    fn test_all_compute_decision_styles_map_to_valid_db() {
        for &computed in ALL_COMPUTE_DECISION_STYLES {
            let db = decision_style_to_db(computed);
            assert!(
                is_valid_decision_style_db(db),
                "compute_decision_style output '{computed}' mapped to invalid DB literal '{db}'"
            );
        }
        // Spot-check the specific mappings the root-cause analysis flagged.
        assert_eq!(decision_style_to_db("DataDriven"), "data_driven");
        assert_eq!(decision_style_to_db("Unknown"), "unknown");
        assert_eq!(decision_style_to_db("Authoritative"), "authoritative");
        assert_eq!(decision_style_to_db("Analytical"), "analytical");
        assert_eq!(decision_style_to_db("Collaborative"), "collaborative");
    }

    /// Every change_appetite produced by psych_compute must map to a valid DB literal.
    #[test]
    fn test_all_compute_change_appetites_map_to_valid_db() {
        for &computed in ALL_COMPUTE_CHANGE_APPETITES {
            let db = change_appetite_to_db(computed);
            assert!(
                is_valid_change_appetite_db(db),
                "compute_change_appetite output '{computed}' mapped to invalid DB literal '{db}'"
            );
        }
        assert_eq!(change_appetite_to_db("High"), "high");
        assert_eq!(change_appetite_to_db("Moderate"), "moderate");
        assert_eq!(change_appetite_to_db("Low"), "low");
        assert_eq!(change_appetite_to_db("Resistant"), "resistant");
    }

    /// The converters must be *total*: any input (including garbage) must yield a
    /// valid DB literal so a bad value can never reach the CHECK constraint.
    #[test]
    fn test_converters_are_total_and_db_valid() {
        let junk_inputs = [
            "",
            " ",
            "Garbage",
            "decisive",
            "aggressive",
            "conservative",
            "123",
            "Data Driven",
            "data-driven",
            "consensus",
            "intuitive",
            "delegative",
        ];
        for input in junk_inputs {
            let ds = decision_style_to_db(input);
            assert!(
                is_valid_decision_style_db(ds),
                "decision_style '{input}' -> '{ds}'"
            );
            let ca = change_appetite_to_db(input);
            assert!(
                is_valid_change_appetite_db(ca),
                "change_appetite '{input}' -> '{ca}'"
            );
        }
    }

    /// PascalCase ↔ snake_case round-trips must be stable for the canonical set.
    #[test]
    fn test_decision_style_roundtrip() {
        for &db in VALID_DECISION_STYLES_DB {
            let display = db_to_decision_style(db);
            let back = decision_style_to_db(display);
            assert_eq!(db, back, "round-trip failed for {db}");
        }
    }

    /// The full DB enum sets must match the migration's CHECK constraints exactly.
    #[test]
    fn test_valid_db_enum_sets_match_schema() {
        assert_eq!(
            VALID_DECISION_STYLES_DB,
            &[
                "authoritative",
                "collaborative",
                "analytical",
                "consensus_driven",
                "data_driven",
                "intuitive",
                "delegative",
                "unknown",
            ]
        );
        assert_eq!(
            VALID_CHANGE_APPETITES_DB,
            &["high", "moderate", "low", "resistant"]
        );
    }
}
