//! Stale entity pruner.
//!
//! Flag companies and persons that haven't had any new observations
//! in 90+ days for review/archival — prevents the system from tracking
//! dissolved or irrelevant entities.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ─── Configuration ──────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct StalePrunerConfig {
    /// Days without observations before an entity is flagged stale
    pub stale_threshold_days: i64,
    /// Days without observations before recommending archival
    pub archive_threshold_days: i64,
    /// Whether to auto-flag entities (vs. report-only mode)
    pub auto_flag: bool,
    /// Entity types to check
    pub entity_types: Vec<String>,
}

impl Default for StalePrunerConfig {
    fn default() -> Self {
        Self {
            stale_threshold_days: 90,
            archive_threshold_days: 180,
            auto_flag: false,
            entity_types: vec!["company".into(), "person".into()],
        }
    }
}

// ─── Entity record ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityStatus {
    pub id: String,
    pub name: String,
    pub entity_type: String,
    pub last_observation_at: Option<DateTime<Utc>>,
    pub observation_count: i64,
    pub created_at: DateTime<Utc>,
    pub region: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StaleReport {
    pub generated_at: DateTime<Utc>,
    pub config: StaleReportConfig,
    pub stale_entities: Vec<StaleEntity>,
    pub archive_candidates: Vec<StaleEntity>,
    pub summary: StaleSummary,
}

#[derive(Debug, Clone, Serialize)]
pub struct StaleReportConfig {
    pub stale_threshold_days: i64,
    pub archive_threshold_days: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct StaleEntity {
    pub id: String,
    pub name: String,
    pub entity_type: String,
    pub days_since_observation: i64,
    pub total_observations: i64,
    pub region: Option<String>,
    pub recommendation: StaleRecommendation,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub enum StaleRecommendation {
    /// Active refresh — schedule an immediate crawl
    RefreshNow,
    /// Review — human should decide if this entity is still relevant
    Review,
    /// Archive — entity appears dissolved or irrelevant
    Archive,
    /// Keep — entity has enough history to warrant retention
    Keep,
}

#[derive(Debug, Clone, Serialize)]
pub struct StaleSummary {
    pub total_entities_checked: usize,
    pub stale_count: usize,
    pub archive_count: usize,
    pub by_type: Vec<(String, usize)>,
    pub by_region: Vec<(String, usize)>,
}

// ─── Pruner engine ──────────────────────────────────────────────────────

pub struct StalePruner {
    config: StalePrunerConfig,
}

impl StalePruner {
    pub fn new(config: StalePrunerConfig) -> Self {
        Self { config }
    }

    pub fn with_defaults() -> Self {
        Self::new(StalePrunerConfig::default())
    }

    /// Evaluate a single entity for staleness.
    pub fn evaluate(&self, entity: &EntityStatus, now: DateTime<Utc>) -> Option<StaleEntity> {
        let days_since = entity
            .last_observation_at
            .map(|last| (now - last).num_days())
            .unwrap_or(999);

        if days_since < self.config.stale_threshold_days {
            return None; // still fresh
        }

        let recommendation =
            if days_since >= self.config.archive_threshold_days && entity.observation_count < 5 {
                StaleRecommendation::Archive
            } else if days_since >= self.config.archive_threshold_days {
                StaleRecommendation::Review
            } else if entity.observation_count > 20 {
                // High-activity entity that went quiet — probably worth refreshing
                StaleRecommendation::RefreshNow
            } else {
                StaleRecommendation::Review
            };

        Some(StaleEntity {
            id: entity.id.clone(),
            name: entity.name.clone(),
            entity_type: entity.entity_type.clone(),
            days_since_observation: days_since,
            total_observations: entity.observation_count,
            region: entity.region.clone(),
            recommendation,
        })
    }

    /// Batch-evaluate all entities and produce a report.
    pub fn analyze(&self, entities: &[EntityStatus], now: DateTime<Utc>) -> StaleReport {
        let stale: Vec<StaleEntity> = entities
            .iter()
            .filter_map(|e| self.evaluate(e, now))
            .collect();

        let archive_candidates: Vec<StaleEntity> = stale
            .iter()
            .filter(|e| {
                e.recommendation == StaleRecommendation::Archive
                    || e.recommendation == StaleRecommendation::Review
                        && e.days_since_observation >= self.config.archive_threshold_days
            })
            .cloned()
            .collect();

        // Build summary
        let mut by_type: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        let mut by_region: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        for entity in &stale {
            *by_type.entry(entity.entity_type.clone()).or_default() += 1;
            if let Some(ref region) = entity.region {
                *by_region.entry(region.clone()).or_default() += 1;
            }
        }

        StaleReport {
            generated_at: now,
            config: StaleReportConfig {
                stale_threshold_days: self.config.stale_threshold_days,
                archive_threshold_days: self.config.archive_threshold_days,
            },
            stale_entities: stale.clone(),
            archive_candidates,
            summary: StaleSummary {
                total_entities_checked: entities.len(),
                stale_count: stale.len(),
                archive_count: stale
                    .iter()
                    .filter(|e| e.recommendation == StaleRecommendation::Archive)
                    .count(),
                by_type: by_type.into_iter().collect(),
                by_region: by_region.into_iter().collect(),
            },
        }
    }

    /// SQL to find stale entities.
    pub fn stale_entities_sql(&self) -> String {
        format!(
            r#"
            SELECT
                e.id, e.name, e.entity_type,
                MAX(o.observed_at) AS last_observation_at,
                COUNT(o.id) AS observation_count,
                e.created_at, e.region
            FROM (
                SELECT id, canonical_name AS name, 'company' AS entity_type, created_at, region
                FROM companies
                UNION ALL
                SELECT id, full_name AS name, 'person' AS entity_type, created_at, NULL AS region
                FROM persons
            ) e
            LEFT JOIN observations o ON o.entity_id = e.id
            GROUP BY e.id, e.name, e.entity_type, e.created_at, e.region
            HAVING MAX(o.observed_at) IS NULL
               OR MAX(o.observed_at) < NOW() - INTERVAL '{} days'
            ORDER BY MAX(o.observed_at) ASC NULLS FIRST
            "#,
            self.config.stale_threshold_days
        )
    }

    /// SQL to flag stale entities.
    pub fn flag_stale_sql() -> &'static str {
        r#"
        UPDATE companies SET metadata = jsonb_set(
            COALESCE(metadata, '{}'),
            '{stale}',
            'true'
        )
        WHERE id = ANY($1)
        "#
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn make_entity(
        id: &str,
        entity_type: &str,
        days_since_obs: i64,
        obs_count: i64,
    ) -> EntityStatus {
        EntityStatus {
            id: id.into(),
            name: format!("Entity {}", id),
            entity_type: entity_type.into(),
            last_observation_at: Some(Utc::now() - Duration::days(days_since_obs)),
            observation_count: obs_count,
            created_at: Utc::now() - Duration::days(365),
            region: Some("TN".into()),
        }
    }

    #[test]
    fn test_fresh_entity_not_stale() {
        let pruner = StalePruner::with_defaults();
        let entity = make_entity("e-1", "company", 30, 10);
        assert!(pruner.evaluate(&entity, Utc::now()).is_none());
    }

    #[test]
    fn test_stale_entity_detected() {
        let pruner = StalePruner::with_defaults();
        let entity = make_entity("e-2", "company", 100, 5);
        let result = pruner.evaluate(&entity, Utc::now());
        assert!(result.is_some());
    }

    #[test]
    fn test_archive_recommendation() {
        let pruner = StalePruner::with_defaults();
        let entity = make_entity("e-3", "company", 200, 3);
        let result = pruner.evaluate(&entity, Utc::now()).unwrap();
        assert_eq!(result.recommendation, StaleRecommendation::Archive);
    }

    #[test]
    fn test_refresh_recommendation() {
        let pruner = StalePruner::with_defaults();
        let entity = make_entity("e-4", "company", 95, 50); // many observations, recently went stale
        let result = pruner.evaluate(&entity, Utc::now()).unwrap();
        assert_eq!(result.recommendation, StaleRecommendation::RefreshNow);
    }

    #[test]
    fn test_batch_report() {
        let pruner = StalePruner::with_defaults();
        let entities = vec![
            make_entity("e-1", "company", 30, 10), // fresh
            make_entity("e-2", "company", 100, 5), // stale
            make_entity("e-3", "person", 200, 2),  // archive
            make_entity("e-4", "company", 50, 15), // fresh
        ];
        let report = pruner.analyze(&entities, Utc::now());
        assert_eq!(report.summary.total_entities_checked, 4);
        assert_eq!(report.summary.stale_count, 2);
        assert!(report.summary.archive_count >= 1);
    }

    #[test]
    fn test_no_observations_entity() {
        let pruner = StalePruner::with_defaults();
        let entity = EntityStatus {
            id: "e-5".into(),
            name: "GhostCorp".into(),
            entity_type: "company".into(),
            last_observation_at: None,
            observation_count: 0,
            created_at: Utc::now() - Duration::days(365),
            region: None,
        };
        let result = pruner.evaluate(&entity, Utc::now()).unwrap();
        assert_eq!(result.recommendation, StaleRecommendation::Archive);
    }

    #[test]
    fn test_sql_generation() {
        let pruner = StalePruner::with_defaults();
        let sql = pruner.stale_entities_sql();
        assert!(sql.contains("90 days"));
        assert!(sql.contains("companies"));
        assert!(sql.contains("persons"));
    }
}
