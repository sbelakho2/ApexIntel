//! UN/World Bank Sanctions List Integration Module
//!
//! Wraps the existing `SanctionsScreener` from `crate::sanctions`
//! and adds structured helpers for ApexIntel entities.

use crate::sanctions::{SanctionsMatch, SanctionsScreener, SanctionsList};
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// High-level sanctions screening result for an entity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityScreeningResult {
    pub entity_name: String,
    pub total_matches: usize,
    pub has_exact_match: bool,
    pub matches_by_list: HashMap<String, usize>,
    pub highest_severity: Option<String>,
    pub screened_at: DateTime<Utc>,
    pub screener_version: String,
}

impl EntityScreeningResult {
    /// Create a result from screening matches.
    pub fn from_matches(name: &str, matches: &[SanctionsMatch]) -> Self {
        let has_exact = matches.iter().any(|m| m.is_exact);
        let mut by_list: HashMap<String, usize> = HashMap::new();
        for m in matches {
            *by_list.entry(m.list.as_str().to_string()).or_insert(0) += 1;
        }
        let highest = matches
            .iter()
            .max_by(|a, b| a.similarity.partial_cmp(&b.similarity).unwrap_or(std::cmp::Ordering::Equal))
            .map(|m| m.list.as_str().to_string());
        Self {
            entity_name: name.to_string(),
            total_matches: matches.len(),
            has_exact_match: has_exact,
            matches_by_list: by_list,
            highest_severity: highest,
            screened_at: Utc::now(),
            screener_version: format!("apex-{}", env!("CARGO_PKG_VERSION")),
        }
    }

    /// Whether this entity requires manual review.
    pub fn needs_review(&self) -> bool {
        self.total_matches > 0 || self.has_exact_match
    }

    /// Risk level assessment.
    pub fn risk_level(&self) -> &'static str {
        if self.has_exact_match {
            "CRITICAL"
        } else if self.total_matches >= 3 {
            "HIGH"
        } else if self.total_matches >= 1 {
            "MEDIUM"
        } else {
            "LOW"
        }
    }
}

/// Sanctions list monitor configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SanctionsMonitorConfig {
    pub lists: Vec<SanctionsList>,
    pub match_threshold: f64,
    pub batch_size: usize,
}

impl Default for SanctionsMonitorConfig {
    fn default() -> Self {
        Self {
            lists: vec![
                SanctionsList::OfacSdn,
                SanctionsList::OfacNs,
                SanctionsList::EuConsolidated,
                SanctionsList::UnSecurity,
                SanctionsList::BisEntityList,
            ],
            match_threshold: 0.92,
            batch_size: 100,
        }
    }
}

/// Sanctions list monitor wrapping the base screener.
#[derive(Debug)]
pub struct SanctionsMonitor {
    screener: SanctionsScreener,
    _config: SanctionsMonitorConfig,
}

impl SanctionsMonitor {
    /// Load sanctions lists from the web.
    pub async fn from_web() -> Result<Self> {
        let screener = SanctionsScreener::load_from_web().await?;
        Ok(Self::with_screener(screener))
    }

    /// Create with an existing screener.
    pub fn with_screener(screener: SanctionsScreener) -> Self {
        Self { screener, _config: SanctionsMonitorConfig::default() }
    }

    /// Screen a single entity name.
    pub fn screen(&self, name: &str) -> Vec<SanctionsMatch> {
        self.screener.screen_entity(name, &[])
    }

    /// Screen a single entity with identifiers.
    pub fn screen_with_ids(&self, name: &str, identifiers: &[(String, String)]) -> Vec<SanctionsMatch> {
        self.screener.screen_entity(name, identifiers)
    }

    /// Screen multiple entities in batch.
    pub fn screen_batch(&self, entities: &[(String, Vec<(String, String)>)]) -> Vec<(String, Vec<SanctionsMatch>)> {
        self.screener.screen_batch(entities)
    }

    /// Get screening result for an entity name.
    pub fn screen_result(&self, name: &str) -> EntityScreeningResult {
        let matches = self.screen(name);
        EntityScreeningResult::from_matches(name, &matches)
    }

    /// Return the total number of entries in the screener.
    pub fn entry_count(&self) -> usize { self.screener.entry_count() }

    /// Return per-list entry counts.
    pub fn list_counts(&self) -> std::collections::HashMap<String, usize> {
        self.screener.list_counts()
    }
}

impl Default for SanctionsMonitor {
    fn default() -> Self {
        Self { screener: SanctionsScreener::empty(), _config: SanctionsMonitorConfig::default() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screening_result_no_matches() {
        let result = EntityScreeningResult::from_matches("Clean Entity", &[]);
        assert_eq!(result.total_matches, 0);
        assert!(!result.needs_review());
        assert_eq!(result.risk_level(), "LOW");
    }

    #[test]
    fn screening_result_exact_match() {
        use crate::sanctions::EntityType;
        let matches = vec![SanctionsMatch {
            query_name: "Viktor Bout".to_string(),
            matched_name: "Viktor Bout".to_string(),
            aliases: vec![],
            similarity: 1.0,
            is_exact: true,
            list: SanctionsList::OfacSdn,
            entry_id: "SDN-001".to_string(),
            programs: vec![],
            entity_type: EntityType::Individual,
            nationalities: vec![],
            identifier_matches: vec![],
            added_date: None,
        }];
        let result = EntityScreeningResult::from_matches("Viktor Bout", &matches);
        assert!(result.has_exact_match);
        assert!(result.needs_review());
        assert_eq!(result.risk_level(), "CRITICAL");
    }

    #[test]
    fn screening_monitor_constructs() {
        let monitor = SanctionsMonitor::default();
        assert_eq!(monitor.entry_count(), 0);
    }
}
