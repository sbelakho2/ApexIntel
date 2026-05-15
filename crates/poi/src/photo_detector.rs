//! POI photo change detection.
//!
//! When a person's photo_hash changes on their company page, generate
//! a "profile update" artifact — often signals a promotion, new company,
//! or significant career change.

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

/// SQL to detect photo hash changes across poi_artifacts.
pub fn photo_change_detection_sql() -> &'static str {
    r#"
    SELECT
        p.id AS person_id,
        p.full_name AS person_name,
        pa.source_url,
        pa.photo_hash AS current_hash,
        pa_prev.photo_hash AS previous_hash,
        pa.crawled_at
    FROM persons p
    JOIN poi_artifacts pa ON pa.person_id = p.id
    LEFT JOIN LATERAL (
        SELECT photo_hash
        FROM poi_artifacts pa2
        WHERE pa2.person_id = p.id
          AND pa2.crawled_at < pa.crawled_at
        ORDER BY pa2.crawled_at DESC
        LIMIT 1
    ) pa_prev ON true
    WHERE pa.photo_hash IS NOT NULL
      AND (pa_prev.photo_hash IS NULL OR pa_prev.photo_hash != pa.photo_hash)
      AND pa.crawled_at > NOW() - INTERVAL '24 hours'
    "#
}

// ─── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    #![allow(clippy::disallowed_methods, clippy::assertions_on_constants)]

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
}
