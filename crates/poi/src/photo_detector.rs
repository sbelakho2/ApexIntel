//! POI photo change detection.
//!
//! When a person's photo_hash changes on their company page, generate
//! a "profile update" artifact — often signals a promotion, new company,
//! or significant career change.
//!
//! Photo hashes are recorded per source document in
//! `poi_artifacts.metadata->>'photo_hash'` (the artifact `url` is the source
//! document).  A hash is only ever compared against the previous hash from
//! the *same* source document (`pa2.url = pa.url`); a photo seen on one site
//! is never associated with a photo seen on another.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ─── Photo change record ────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhotoChange {
    pub person_id: String,
    pub person_name: String,
    pub source_url: String,
    pub old_hash: Option<String>,
    pub new_hash: String,
    pub detected_at: DateTime<Utc>,
    pub change_type: PhotoChangeType,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PhotoChangeType {
    /// First time seeing a photo for this person
    Initial,
    /// Photo was updated (hash changed)
    Updated,
    /// Photo was removed
    Removed,
}

#[derive(Debug, Clone, Serialize)]
pub struct PhotoChangeArtifact {
    pub person_id: String,
    pub person_name: String,
    pub artifact_type: String,
    pub title: String,
    pub description: String,
    pub significance: PhotoSignificance,
    pub detected_at: DateTime<Utc>,
    pub source_url: String,
    pub metadata: PhotoChangeMetadata,
}

#[derive(Debug, Clone, Serialize)]
pub struct PhotoChangeMetadata {
    pub old_hash: Option<String>,
    pub new_hash: String,
    pub change_type: String,
    pub days_since_last_change: Option<i64>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
pub enum PhotoSignificance {
    /// Minor — photo update, could be routine
    Low,
    /// Medium — photo changed on a professional profile
    Medium,
    /// High — photo removed or changed on LinkedIn/company page
    High,
}

// ─── Detection engine ───────────────────────────────────────────────────

/// One row of [`photo_change_detection_sql`] output, mapped to Rust.
///
/// `previous_hash` is the most recent earlier hash *from the same source URL*
/// and is `None` when this is the first photo seen for that source.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DetectedPhotoRow {
    pub person_id: String,
    pub person_name: String,
    pub source_url: String,
    pub current_hash: String,
    pub previous_hash: Option<String>,
    pub detected_at: DateTime<Utc>,
}

/// Convert detector query rows into photo changes (pure; no database).
///
/// Rows whose hash did not change are filtered out by
/// [`PhotoDetector::detect`], so callers may persist one artifact per returned
/// change without re-checking the hashes.
pub fn detect_changes(rows: &[DetectedPhotoRow]) -> Vec<PhotoChange> {
    rows.iter()
        .filter_map(|row| {
            PhotoDetector::detect(
                &row.person_id,
                &row.person_name,
                &row.source_url,
                row.previous_hash.as_deref(),
                &row.current_hash,
                row.detected_at,
            )
        })
        .collect()
}

pub struct PhotoDetector;

impl PhotoDetector {
    /// Detect a photo change by comparing old and new hashes.
    pub fn detect(
        person_id: &str,
        person_name: &str,
        source_url: &str,
        old_hash: Option<&str>,
        new_hash: &str,
        now: DateTime<Utc>,
    ) -> Option<PhotoChange> {
        let change_type = match old_hash {
            None => PhotoChangeType::Initial,
            Some(old) if old == new_hash => return None, // no change
            Some(_) if new_hash.is_empty() => PhotoChangeType::Removed,
            Some(_) => PhotoChangeType::Updated,
        };

        Some(PhotoChange {
            person_id: person_id.into(),
            person_name: person_name.into(),
            source_url: source_url.into(),
            old_hash: old_hash.map(|s| s.into()),
            new_hash: new_hash.into(),
            detected_at: now,
            change_type,
        })
    }

    /// Generate an artifact from a photo change.
    pub fn to_artifact(
        change: &PhotoChange,
        last_change_date: Option<DateTime<Utc>>,
    ) -> PhotoChangeArtifact {
        let significance = Self::assess_significance(change, last_change_date);

        let title = match change.change_type {
            PhotoChangeType::Initial => {
                format!("New profile photo detected for {}", change.person_name)
            }
            PhotoChangeType::Updated => {
                format!("Profile photo updated for {}", change.person_name)
            }
            PhotoChangeType::Removed => {
                format!("Profile photo removed for {}", change.person_name)
            }
        };

        let description = match change.change_type {
            PhotoChangeType::Initial => format!(
                "{}'s profile photo was detected for the first time on {}. \
                 This may indicate a new or recently updated professional profile.",
                change.person_name, change.source_url
            ),
            PhotoChangeType::Updated => format!(
                "{}'s profile photo changed on {}. Photo updates on professional \
                 pages often correlate with promotions, role changes, or company changes.",
                change.person_name, change.source_url
            ),
            PhotoChangeType::Removed => format!(
                "{}'s profile photo was removed from {}. \
                 Photo removal may indicate account deactivation or departure.",
                change.person_name, change.source_url
            ),
        };

        let days_since = last_change_date.map(|d| (change.detected_at - d).num_days());

        PhotoChangeArtifact {
            person_id: change.person_id.clone(),
            person_name: change.person_name.clone(),
            artifact_type: "profile_photo_change".into(),
            title,
            description,
            significance,
            detected_at: change.detected_at,
            source_url: change.source_url.clone(),
            metadata: PhotoChangeMetadata {
                old_hash: change.old_hash.clone(),
                new_hash: change.new_hash.clone(),
                change_type: format!("{:?}", change.change_type),
                days_since_last_change: days_since,
            },
        }
    }

    fn assess_significance(
        change: &PhotoChange,
        last_change_date: Option<DateTime<Utc>>,
    ) -> PhotoSignificance {
        // Removal is always high significance
        if change.change_type == PhotoChangeType::Removed {
            return PhotoSignificance::High;
        }

        // LinkedIn changes are high significance
        if change.source_url.contains("linkedin.com") {
            return PhotoSignificance::High;
        }

        // Frequent changes (within 30 days) are more significant
        if let Some(last) = last_change_date {
            let days = (change.detected_at - last).num_days();
            if days < 30 {
                return PhotoSignificance::Medium;
            }
        }

        // Initial detection is low significance
        if change.change_type == PhotoChangeType::Initial {
            return PhotoSignificance::Low;
        }

        PhotoSignificance::Medium
    }
}

/// SQL to detect photo hash changes across `poi_artifacts`.
///
/// Column mapping to the real schema:
/// - `poi_artifacts.url` is the source document (`source_url` in the output);
/// - the photo hash lives in `poi_artifacts.metadata->>'photo_hash'`;
/// - `poi_artifacts.ts_utc` is the observation time (`crawled_at` in the
///   output) and `persons.name` is the person's display name.
///
/// #164: the previous-hash lookup is restricted to rows from the *same source
/// document* (`pa2.url = pa.url`).  Without that predicate the detector could
/// pair a photo hash from one source URL with a hash from a different source
/// URL and report a spurious "photo changed" event.
pub fn photo_change_detection_sql() -> &'static str {
    r#"
    SELECT
        p.id AS person_id,
        p.name AS person_name,
        pa.url AS source_url,
        pa.metadata->>'photo_hash' AS current_hash,
        pa_prev.photo_hash AS previous_hash,
        pa.ts_utc AS crawled_at
    FROM persons p
    JOIN poi_artifacts pa ON pa.person_id = p.id
    LEFT JOIN LATERAL (
        SELECT pa2.metadata->>'photo_hash' AS photo_hash
        FROM poi_artifacts pa2
        WHERE pa2.person_id = p.id
          AND pa2.url = pa.url
          AND pa2.ts_utc < pa.ts_utc
          AND pa2.metadata->>'photo_hash' IS NOT NULL
        ORDER BY pa2.ts_utc DESC
        LIMIT 1
    ) pa_prev ON true
    WHERE pa.metadata->>'photo_hash' IS NOT NULL
      AND (
        pa_prev.photo_hash IS NULL
        OR pa_prev.photo_hash != pa.metadata->>'photo_hash'
      )
      AND pa.ts_utc > NOW() - INTERVAL '24 hours'
    "#
}

// ─── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::assertions_on_constants
    )]

    use super::*;

    #[test]
    fn test_detect_initial_photo() {
        let change = PhotoDetector::detect(
            "p-001",
            "Ahmed Ben Salah",
            "https://linkedin.com/in/ahmed",
            None,
            "abc123hash",
            Utc::now(),
        );
        assert!(change.is_some());
        assert_eq!(change.unwrap().change_type, PhotoChangeType::Initial);
    }

    #[test]
    fn test_detect_updated_photo() {
        let change = PhotoDetector::detect(
            "p-001",
            "Ahmed Ben Salah",
            "https://linkedin.com/in/ahmed",
            Some("old_hash"),
            "new_hash",
            Utc::now(),
        );
        assert!(change.is_some());
        assert_eq!(change.unwrap().change_type, PhotoChangeType::Updated);
    }

    #[test]
    fn test_detect_no_change() {
        let change = PhotoDetector::detect(
            "p-001",
            "Ahmed Ben Salah",
            "https://linkedin.com/in/ahmed",
            Some("same_hash"),
            "same_hash",
            Utc::now(),
        );
        assert!(change.is_none());
    }

    #[test]
    fn test_detect_removed_photo() {
        let change = PhotoDetector::detect(
            "p-001",
            "Ahmed Ben Salah",
            "https://example.com",
            Some("old_hash"),
            "",
            Utc::now(),
        );
        assert!(change.is_some());
        assert_eq!(change.unwrap().change_type, PhotoChangeType::Removed);
    }

    #[test]
    fn test_artifact_from_update() {
        let change = PhotoChange {
            person_id: "p-001".into(),
            person_name: "Ahmed".into(),
            source_url: "https://linkedin.com/in/ahmed".into(),
            old_hash: Some("old".into()),
            new_hash: "new".into(),
            detected_at: Utc::now(),
            change_type: PhotoChangeType::Updated,
        };
        let artifact = PhotoDetector::to_artifact(&change, None);
        assert!(artifact.title.contains("updated"));
        assert_eq!(artifact.significance, PhotoSignificance::High); // LinkedIn
    }

    #[test]
    fn test_removal_high_significance() {
        let change = PhotoChange {
            person_id: "p-001".into(),
            person_name: "Test".into(),
            source_url: "https://example.com".into(),
            old_hash: Some("old".into()),
            new_hash: "".into(),
            detected_at: Utc::now(),
            change_type: PhotoChangeType::Removed,
        };
        let artifact = PhotoDetector::to_artifact(&change, None);
        assert_eq!(artifact.significance, PhotoSignificance::High);
    }

    #[test]
    fn test_sql_not_empty() {
        assert!(photo_change_detection_sql().contains("poi_artifacts"));
    }

    // ── #164: the previous hash must come from the same source document ──

    #[test]
    fn test_sql_requires_same_source_document_for_previous_hash() {
        let sql = photo_change_detection_sql();
        assert!(
            sql.contains("pa2.url = pa.url"),
            "previous-hash lookup must be constrained to the same source URL:\n{sql}"
        );
    }

    #[test]
    fn test_sql_uses_real_poi_artifacts_columns() {
        let sql = photo_change_detection_sql();
        assert!(sql.contains("pa.url AS source_url"));
        assert!(sql.contains("pa.metadata->>'photo_hash' AS current_hash"));
        assert!(sql.contains("pa.ts_utc AS crawled_at"));
        assert!(sql.contains("p.name AS person_name"));
        // Stale column references from the dead query must be gone.
        assert!(!sql.contains("pa.source_url"));
        assert!(!sql.contains("pa.photo_hash"));
        assert!(!sql.contains("crawled_at <"), "must use ts_utc ordering");
        assert!(!sql.contains("full_name"));
    }

    #[test]
    fn test_detect_changes_maps_rows_and_skips_unchanged() {
        let unchanged = DetectedPhotoRow {
            person_id: "p-1".into(),
            person_name: "Ahmed".into(),
            source_url: "https://example.com/a".into(),
            current_hash: "same".into(),
            previous_hash: Some("same".into()),
            detected_at: Utc::now(),
        };
        let changed = DetectedPhotoRow {
            person_id: "p-1".into(),
            person_name: "Ahmed".into(),
            source_url: "https://example.com/a".into(),
            current_hash: "new".into(),
            previous_hash: Some("old".into()),
            detected_at: Utc::now(),
        };
        let initial = DetectedPhotoRow {
            person_id: "p-2".into(),
            person_name: "Lina".into(),
            source_url: "https://linkedin.com/in/lina".into(),
            current_hash: "first".into(),
            previous_hash: None,
            detected_at: Utc::now(),
        };

        let changes = detect_changes(&[unchanged, changed, initial]);
        assert_eq!(changes.len(), 2, "unchanged hashes must be filtered out");
        assert_eq!(changes[0].change_type, PhotoChangeType::Updated);
        assert_eq!(changes[0].old_hash.as_deref(), Some("old"));
        assert_eq!(changes[1].change_type, PhotoChangeType::Initial);
        assert_eq!(changes[1].person_id, "p-2");
    }
}
