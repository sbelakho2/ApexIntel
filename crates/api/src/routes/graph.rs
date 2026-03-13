//! Graph route — request/response types and logic for graph exploration endpoints.

use apex_core::validation::validate_nonempty_id;
use serde::{Deserialize, Serialize};

// ────────────────────────────────────────────
// Request types
// ────────────────────────────────────────────

/// Query parameters for neighborhood exploration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NeighborhoodQuery {
    pub depth: Option<u32>,
    pub max_nodes: Option<u32>,
    pub edge_types: Option<String>,
    pub min_weight: Option<f64>,
}

impl Default for NeighborhoodQuery {
    fn default() -> Self {
        Self {
            depth: Some(1),
            max_nodes: Some(50),
            edge_types: None,
            min_weight: None,
        }
    }
}

impl NeighborhoodQuery {
    pub fn sanitize(&self) -> Self {
        Self {
            depth: Some(self.depth.unwrap_or(1).clamp(1, 3)),
            max_nodes: Some(self.max_nodes.unwrap_or(50).clamp(1, 200)),
            edge_types: self.edge_types.clone(),
            min_weight: self.min_weight.map(|w| w.clamp(0.0, 1.0)),
        }
    }
}

/// Query parameters for path finding.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathQuery {
    pub max_hops: Option<u32>,
    pub edge_types: Option<String>,
}

impl Default for PathQuery {
    fn default() -> Self {
        Self {
            max_hops: Some(5),
            edge_types: None,
        }
    }
}

impl PathQuery {
    pub fn sanitize(&self) -> Self {
        Self {
            max_hops: Some(self.max_hops.unwrap_or(5).clamp(1, 10)),
            edge_types: self.edge_types.clone(),
        }
    }
}

// ────────────────────────────────────────────
// Response types
// ────────────────────────────────────────────

/// Graph neighborhood response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NeighborhoodResponse {
    pub center_id: String,
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    pub total_neighbors: u32,
    pub depth_reached: u32,
}

/// Node in a graph visualization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphNode {
    pub id: String,
    pub label: String,
    pub node_type: String,
    pub properties: serde_json::Value,
    pub depth: u32,
}

/// Edge in a graph visualization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphEdge {
    pub source: String,
    pub target: String,
    pub edge_type: String,
    pub weight: f64,
    pub label: Option<String>,
}

/// Shortest path response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathResponse {
    pub from_id: String,
    pub to_id: String,
    pub path: Vec<PathStep>,
    pub total_hops: u32,
    pub total_weight: f64,
    pub found: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathStep {
    pub node_id: String,
    pub node_label: String,
    pub edge_type: Option<String>,
    pub edge_weight: Option<f64>,
}

/// Summary counts for the graph overview endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphOverview {
    pub companies_total: u64,
    pub persons_total: u64,
    pub warnings_total: u64,
    pub insights_total: u64,
}

/// Extended graph overview with edges data for visualization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphOverviewWithEdges {
    pub companies_total: u64,
    pub persons_total: u64,
    pub warnings_total: u64,
    pub insights_total: u64,
    pub edges_total: u64,
    /// Resolved node labels for every unique entity UUID that appears in edges.
    pub nodes: Vec<GraphNodeLabel>,
    pub edges: Vec<GraphEdge>,
    pub edge_type_counts: Vec<EdgeTypeCount>,
}

/// Resolved name + type for a node UUID referenced in graph edges.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphNodeLabel {
    pub id: String,
    pub label: String,
    pub node_type: String, // "company" | "person"
}

/// Edge type count summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EdgeTypeCount {
    pub edge_type: String,
    pub count: u64,
}

// ────────────────────────────────────────────
// Logic
// ────────────────────────────────────────────

/// Parse edge types filter string (comma-separated).
pub fn parse_edge_types(input: &str) -> Vec<String> {
    input
        .split(',')
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Filter graph edges by types.
pub fn filter_edges_by_type<'a>(edges: &'a [GraphEdge], types: &[String]) -> Vec<&'a GraphEdge> {
    if types.is_empty() {
        return edges.iter().collect();
    }
    edges
        .iter()
        .filter(|e| types.contains(&e.edge_type.to_lowercase()))
        .collect()
}

/// Filter edges by minimum weight.
pub fn filter_edges_by_weight(edges: &[GraphEdge], min_weight: f64) -> Vec<&GraphEdge> {
    edges.iter().filter(|e| e.weight >= min_weight).collect()
}

/// Compute graph density (edges / max_possible_edges).
pub fn graph_density(node_count: usize, edge_count: usize) -> f64 {
    if node_count <= 1 {
        return 0.0;
    }
    let max_edges = node_count * (node_count - 1) / 2;
    if max_edges == 0 {
        return 0.0;
    }
    (edge_count as f64 / max_edges as f64).min(1.0)
}

/// Validate entity ID for graph queries.
pub fn validate_entity_id(id: &str) -> Result<String, String> {
    let trimmed = id.trim();
    if let Err(err) = validate_nonempty_id(trimmed, "entity_id") {
        return Err(err.to_string());
    }
    if trimmed.len() > 128 {
        return Err("Entity ID too long (max 128 chars)".to_string());
    }
    Ok(trimmed.to_string())
}

/// Node degree summary.
pub fn node_degrees(edges: &[GraphEdge]) -> Vec<(String, usize)> {
    let mut degrees: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for e in edges {
        *degrees.entry(e.source.clone()).or_insert(0) += 1;
        *degrees.entry(e.target.clone()).or_insert(0) += 1;
    }
    let mut result: Vec<_> = degrees.into_iter().collect();
    result.sort_by(|a, b| b.1.cmp(&a.1));
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_edge(source: &str, target: &str, edge_type: &str, weight: f64) -> GraphEdge {
        GraphEdge {
            source: source.to_string(),
            target: target.to_string(),
            edge_type: edge_type.to_string(),
            weight,
            label: None,
        }
    }

    #[test]
    fn test_neighborhood_query_sanitize() {
        let q = NeighborhoodQuery {
            depth: Some(10),
            max_nodes: Some(500),
            edge_types: None,
            min_weight: Some(2.0),
        };
        let s = q.sanitize();
        assert_eq!(s.depth, Some(3)); // clamped
        assert_eq!(s.max_nodes, Some(200)); // clamped
        assert_eq!(s.min_weight, Some(1.0)); // clamped
    }

    #[test]
    fn test_neighborhood_query_defaults() {
        let q = NeighborhoodQuery {
            depth: None,
            max_nodes: None,
            edge_types: None,
            min_weight: None,
        };
        let s = q.sanitize();
        assert_eq!(s.depth, Some(1));
        assert_eq!(s.max_nodes, Some(50));
    }

    #[test]
    fn test_path_query_sanitize() {
        let q = PathQuery {
            max_hops: Some(100),
            edge_types: None,
        };
        let s = q.sanitize();
        assert_eq!(s.max_hops, Some(10)); // clamped
    }

    #[test]
    fn test_parse_edge_types() {
        let types = parse_edge_types("supplies, competes, employs");
        assert_eq!(types, vec!["supplies", "competes", "employs"]);
    }

    #[test]
    fn test_parse_edge_types_empty() {
        let types = parse_edge_types("");
        assert!(types.is_empty());
    }

    #[test]
    fn test_filter_edges_by_type() {
        let edges = vec![
            make_edge("A", "B", "supplies", 0.8),
            make_edge("B", "C", "competes", 0.6),
            make_edge("A", "C", "supplies", 0.5),
        ];
        let types = vec!["supplies".to_string()];
        let filtered = filter_edges_by_type(&edges, &types);
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn test_filter_edges_by_type_empty() {
        let edges = vec![make_edge("A", "B", "supplies", 0.8)];
        let filtered = filter_edges_by_type(&edges, &[]);
        assert_eq!(filtered.len(), 1); // no filter = all
    }

    #[test]
    fn test_filter_edges_by_weight() {
        let edges = vec![
            make_edge("A", "B", "supplies", 0.8),
            make_edge("B", "C", "competes", 0.3),
            make_edge("A", "C", "supplies", 0.5),
        ];
        let filtered = filter_edges_by_weight(&edges, 0.5);
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn test_graph_density() {
        // 3 nodes, 3 edges → density = 3/3 = 1.0
        assert!((graph_density(3, 3) - 1.0).abs() < 0.001);
        // 4 nodes, 2 edges → density = 2/6 ≈ 0.333
        assert!((graph_density(4, 2) - 0.333).abs() < 0.01);
        // edge cases
        assert_eq!(graph_density(0, 0), 0.0);
        assert_eq!(graph_density(1, 0), 0.0);
    }

    #[test]
    fn test_validate_entity_id() {
        assert!(validate_entity_id("c-12345").is_ok());
        assert!(validate_entity_id("").is_err());
        assert!(validate_entity_id(&"x".repeat(200)).is_err());
    }

    #[test]
    fn test_node_degrees() {
        let edges = vec![
            make_edge("A", "B", "x", 0.5),
            make_edge("A", "C", "x", 0.5),
            make_edge("B", "C", "x", 0.5),
        ];
        let degrees = node_degrees(&edges);
        // A: 2 (A→B, A→C), B: 2 (A→B, B→C), C: 2 (A→C, B→C)
        assert_eq!(degrees.len(), 3);
        for (_, deg) in &degrees {
            assert_eq!(*deg, 2);
        }
    }

    #[test]
    fn test_node_degrees_hub() {
        let edges = vec![
            make_edge("HUB", "A", "x", 0.5),
            make_edge("HUB", "B", "x", 0.5),
            make_edge("HUB", "C", "x", 0.5),
            make_edge("HUB", "D", "x", 0.5),
        ];
        let degrees = node_degrees(&edges);
        assert_eq!(degrees[0].0, "HUB"); // highest degree
        assert_eq!(degrees[0].1, 4);
    }

    #[test]
    fn test_neighborhood_response_serialization() {
        let resp = NeighborhoodResponse {
            center_id: "c-1".to_string(),
            nodes: vec![GraphNode {
                id: "c-1".to_string(),
                label: "Starz".to_string(),
                node_type: "company".to_string(),
                properties: serde_json::json!({"region": "TN"}),
                depth: 0,
            }],
            edges: vec![make_edge("c-1", "c-2", "supplies", 0.8)],
            total_neighbors: 1,
            depth_reached: 1,
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("Starz"));
        assert!(json.contains("supplies"));
    }

    #[test]
    fn test_path_response_serialization() {
        let resp = PathResponse {
            from_id: "A".to_string(),
            to_id: "C".to_string(),
            path: vec![
                PathStep {
                    node_id: "A".to_string(),
                    node_label: "Company A".to_string(),
                    edge_type: None,
                    edge_weight: None,
                },
                PathStep {
                    node_id: "B".to_string(),
                    node_label: "Company B".to_string(),
                    edge_type: Some("supplies".to_string()),
                    edge_weight: Some(0.8),
                },
                PathStep {
                    node_id: "C".to_string(),
                    node_label: "Company C".to_string(),
                    edge_type: Some("competes".to_string()),
                    edge_weight: Some(0.6),
                },
            ],
            total_hops: 2,
            total_weight: 1.4,
            found: true,
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("Company A"));
        assert!(json.contains("\"found\":true"));
    }
}
