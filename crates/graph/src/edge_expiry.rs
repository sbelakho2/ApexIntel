//! Graph edge expiry policy.
//!
//! Graph edges that haven't been refreshed (`last_seen`) in 180 days
//! are flagged as stale and excluded from influence score calculations.
//! Provides both detection and cleanup utilities.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

// ─── Configuration ──────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct EdgeExpiryConfig {
    /// Days without refresh before an edge is stale
    pub stale_threshold_days: i64,
    /// Days without refresh before an edge is archived
    pub archive_threshold_days: i64,
    /// Whether to auto-archive expired edges
    pub auto_archive: bool,
}

impl Default for EdgeExpiryConfig {
    fn default() -> Self {
        Self {
            stale_threshold_days: 180,
            archive_threshold_days: 365,
            auto_archive: false,
        }
    }
}

// ─── Edge record ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphEdge {
    pub id: String,
    pub source_id: String,
    pub target_id: String,
    pub edge_type: String,
    pub weight: f64,
    pub last_seen: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub stale: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExpiryReport {
    pub total_edges: usize,
    pub active_edges: usize,
    pub stale_edges: usize,
    pub archive_candidates: usize,
    pub stale_edge_ids: Vec<String>,
    pub archive_edge_ids: Vec<String>,
}

// ─── Expiry engine ──────────────────────────────────────────────────────

pub struct EdgeExpiryEngine {
    config: EdgeExpiryConfig,
}

impl EdgeExpiryEngine {
    pub fn new(config: EdgeExpiryConfig) -> Self {
        Self { config }
    }

    pub fn with_defaults() -> Self {
        Self::new(EdgeExpiryConfig::default())
    }

    /// Check if a single edge is stale.
    pub fn is_stale(&self, edge: &GraphEdge, now: DateTime<Utc>) -> bool {
        let age = now - edge.last_seen;
        age > Duration::days(self.config.stale_threshold_days)
    }

    /// Check if a single edge should be archived.
    pub fn is_archive_candidate(&self, edge: &GraphEdge, now: DateTime<Utc>) -> bool {
        let age = now - edge.last_seen;
        age > Duration::days(self.config.archive_threshold_days)
    }

    /// Analyze a batch of edges and produce an expiry report.
    pub fn analyze(&self, edges: &[GraphEdge], now: DateTime<Utc>) -> ExpiryReport {
        let mut stale_ids = Vec::new();
        let mut archive_ids = Vec::new();

        for edge in edges {
            if self.is_archive_candidate(edge, now) {
                archive_ids.push(edge.id.clone());
                stale_ids.push(edge.id.clone());
            } else if self.is_stale(edge, now) {
                stale_ids.push(edge.id.clone());
            }
        }

        ExpiryReport {
            total_edges: edges.len(),
            active_edges: edges.len() - stale_ids.len(),
            stale_edges: stale_ids.len(),
            archive_candidates: archive_ids.len(),
            stale_edge_ids: stale_ids,
            archive_edge_ids: archive_ids,
        }
    }

    /// Mark stale edges in place. Returns count of newly stale edges.
    pub fn mark_stale(&self, edges: &mut [GraphEdge], now: DateTime<Utc>) -> usize {
        let mut count = 0;
        for edge in edges.iter_mut() {
            let should_be_stale = self.is_stale(edge, now);
            if should_be_stale && !edge.stale {
                edge.stale = true;
                count += 1;
            }
        }
        count
    }

    /// Filter edges to only active (non-stale) for influence calculations.
    pub fn active_edges<'a>(
        &self,
        edges: &'a [GraphEdge],
        now: DateTime<Utc>,
    ) -> Vec<&'a GraphEdge> {
        edges.iter().filter(|e| !self.is_stale(e, now)).collect()
    }

    /// SQL to flag stale edges in the database. Uses `$1` for parameterized bind.
    pub fn flag_stale_sql(&self) -> String {
        "UPDATE graph_edges SET stale = true \
         WHERE last_seen < NOW() - ($1::text || ' days')::INTERVAL AND stale = false"
            .to_string()
    }

    /// SQL to archive (soft-delete) very old edges. Uses `$1` for parameterized bind.
    pub fn archive_sql(&self) -> String {
        "UPDATE graph_edges SET archived = true \
         WHERE last_seen < NOW() - ($1::text || ' days')::INTERVAL"
            .to_string()
    }

    /// SQL to exclude stale edges from influence queries.
    pub fn active_edges_where_clause() -> &'static str {
        "AND (stale = false OR stale IS NULL)"
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_edge(id: &str, days_old: i64) -> GraphEdge {
        GraphEdge {
            id: id.into(),
            source_id: "src-1".into(),
            target_id: "tgt-1".into(),
            edge_type: "supplies".into(),
            weight: 0.8,
            last_seen: Utc::now() - Duration::days(days_old),
            created_at: Utc::now() - Duration::days(days_old + 30),
            stale: false,
        }
    }

    #[test]
    fn test_fresh_edge_not_stale() {
        let engine = EdgeExpiryEngine::with_defaults();
        let edge = make_edge("e-1", 30);
        assert!(!engine.is_stale(&edge, Utc::now()));
    }

    #[test]
    fn test_old_edge_is_stale() {
        let engine = EdgeExpiryEngine::with_defaults();
        let edge = make_edge("e-2", 200);
        assert!(engine.is_stale(&edge, Utc::now()));
    }

    #[test]
    fn test_very_old_edge_archive_candidate() {
        let engine = EdgeExpiryEngine::with_defaults();
        let edge = make_edge("e-3", 400);
        assert!(engine.is_archive_candidate(&edge, Utc::now()));
    }

    #[test]
    fn test_analyze_batch() {
        let engine = EdgeExpiryEngine::with_defaults();
        let edges = vec![
            make_edge("e-1", 30),  // active
            make_edge("e-2", 100), // active
            make_edge("e-3", 200), // stale
            make_edge("e-4", 400), // archive candidate
        ];
        let report = engine.analyze(&edges, Utc::now());
        assert_eq!(report.total_edges, 4);
        assert_eq!(report.active_edges, 2);
        assert_eq!(report.stale_edges, 2);
        assert_eq!(report.archive_candidates, 1);
    }

    #[test]
    fn test_mark_stale() {
        let engine = EdgeExpiryEngine::with_defaults();
        let mut edges = vec![make_edge("e-1", 30), make_edge("e-2", 200)];
        let count = engine.mark_stale(&mut edges, Utc::now());
        assert_eq!(count, 1);
        assert!(!edges[0].stale);
        assert!(edges[1].stale);
    }

    #[test]
    fn test_active_edges_filter() {
        let engine = EdgeExpiryEngine::with_defaults();
        let edges = vec![
            make_edge("e-1", 30),
            make_edge("e-2", 200),
            make_edge("e-3", 60),
        ];
        let active = engine.active_edges(&edges, Utc::now());
        assert_eq!(active.len(), 2);
    }

    #[test]
    fn test_sql_generation() {
        let engine = EdgeExpiryEngine::with_defaults();
        let sql = engine.flag_stale_sql();
        assert!(sql.contains("$1"), "SQL must use parameterized $1 bind");
        let archive = engine.archive_sql();
        assert!(archive.contains("$1"), "SQL must use parameterized $1 bind");
    }
}
