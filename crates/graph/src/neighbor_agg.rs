use std::collections::HashMap;
use crate::adjacency::AdjacencyGraph;

/// Influence score formula from IMPLEMENTATION.md:
/// 0.3 × GraphCentrality + 0.4 × RoleSeniority + 0.3 × PublicRecurrence
pub fn influence_score(graph_centrality: f64, role_seniority: f64, public_recurrence: f64) -> f64 {
    let raw = 0.3 * graph_centrality + 0.4 * role_seniority + 0.3 * public_recurrence;
    raw.clamp(0.0, 100.0)
}

/// Map RoleSeniority string to a numeric 0-100 score.
pub fn seniority_to_score(seniority: &str) -> f64 {
    match seniority.to_lowercase().as_str() {
        "c-level" | "c_level" | "clevel" => 95.0,
        "vp" | "vice president" | "vice_president" => 85.0,
        "director" => 70.0,
        "senior manager" | "senior_manager" => 60.0,
        "manager" => 50.0,
        "lead" | "team lead" | "team_lead" => 40.0,
        "senior" | "senior engineer" => 30.0,
        "mid" | "engineer" => 20.0,
        "junior" => 10.0,
        _ => 15.0,
    }
}

/// Compute recurrence score from observation counts.
///
/// The more times a person appears in public sources, the higher the recurrence.
/// Normalized to 0-100 using sigmoid-like scaling.
pub fn recurrence_score(appearance_count: usize) -> f64 {
    // Sigmoid with midpoint at 10 appearances, scaled to 0-100
    let x = appearance_count as f64;
    let raw = 100.0 / (1.0 + (-0.3 * (x - 10.0)).exp());
    raw.clamp(0.0, 100.0)
}

/// Aggregate neighbor features for a node.
///
/// Given a graph and feature vectors for each node, compute the aggregated
/// feature for `node_id` as the weighted average of its neighbors' features.
pub fn aggregate_neighbor_features(
    graph: &AdjacencyGraph,
    features: &HashMap<String, Vec<f64>>,
    node_id: &str,
) -> Option<Vec<f64>> {
    let neighbors = graph.neighbors(node_id);
    if neighbors.is_empty() {
        return None;
    }

    let mut total_weight = 0.0;
    let mut agg: Option<Vec<f64>> = None;

    for (neighbor, weight) in neighbors {
        if let Some(feat) = features.get(neighbor) {
            total_weight += weight;
            match &mut agg {
                None => {
                    agg = Some(feat.iter().map(|v| v * weight).collect());
                }
                Some(a) => {
                    for (i, v) in feat.iter().enumerate() {
                        if i < a.len() {
                            a[i] += v * weight;
                        }
                    }
                }
            }
        }
    }

    if total_weight > 0.0 {
        agg.map(|a| a.iter().map(|v| v / total_weight).collect())
    } else {
        None
    }
}

/// Compute centrality-based influence for all nodes in a graph.
///
/// Returns a HashMap of node_id -> centrality score (0-100).
pub fn compute_centrality_scores(graph: &AdjacencyGraph) -> HashMap<String, f64> {
    let pr = graph.pagerank(30, 0.85);

    if pr.is_empty() {
        return HashMap::new();
    }

    // Find max pagerank to normalize to 0-100
    let max_pr = pr.values().cloned().fold(0.0_f64, f64::max);
    if max_pr < 1e-12 {
        return pr.into_iter().map(|(k, _)| (k, 0.0)).collect();
    }

    pr.into_iter()
        .map(|(k, v)| (k, (v / max_pr) * 100.0))
        .collect()
}

/// Role drift score: measures how much a person's role has changed recently.
///
/// Takes a sequence of (timestamp, seniority_score) and returns 0-1.
pub fn role_drift_score(history: &[(i64, f64)]) -> f64 {
    if history.len() < 2 {
        return 0.0;
    }

    let mut sorted: Vec<(i64, f64)> = history.to_vec();
    sorted.sort_by_key(|(ts, _)| *ts);

    let recent = sorted.last().unwrap().1;
    let prev = sorted[sorted.len() - 2].1;

    let delta = (recent - prev).abs() / 100.0;
    delta.clamp(0.0, 1.0)
}

/// Network leverage: find all shared connections between a person and the Starz ecosystem.
pub fn network_leverage(
    graph: &AdjacencyGraph,
    person: &str,
    ecosystem_nodes: &[String],
) -> Vec<String> {
    let person_neighbors: std::collections::HashSet<String> = graph
        .neighbors(person)
        .iter()
        .map(|(n, _)| n.clone())
        .collect();

    let mut shared = Vec::new();
    for eco_node in ecosystem_nodes {
        if person_neighbors.contains(eco_node) {
            shared.push(eco_node.clone());
        }
        // Also check if any of eco_node's neighbors overlap
        for (n, _) in graph.neighbors(eco_node) {
            if person_neighbors.contains(n.as_str()) && !shared.contains(&n) {
                shared.push(n.clone());
            }
        }
    }

    shared.sort();
    shared.dedup();
    shared
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_influence_score_balanced() {
        let score = influence_score(80.0, 90.0, 70.0);
        // 0.3*80 + 0.4*90 + 0.3*70 = 24 + 36 + 21 = 81.0
        assert!((score - 81.0).abs() < 0.01);
    }

    #[test]
    fn test_influence_score_capped() {
        let score = influence_score(100.0, 100.0, 100.0);
        assert!((score - 100.0).abs() < 0.01);
    }

    #[test]
    fn test_influence_score_zero() {
        let score = influence_score(0.0, 0.0, 0.0);
        assert!((score - 0.0).abs() < 0.01);
    }

    #[test]
    fn test_seniority_to_score_c_level() {
        assert!((seniority_to_score("C-Level") - 95.0).abs() < 0.01);
    }

    #[test]
    fn test_seniority_to_score_director() {
        assert!((seniority_to_score("Director") - 70.0).abs() < 0.01);
    }

    #[test]
    fn test_seniority_to_score_unknown() {
        assert!((seniority_to_score("intern") - 15.0).abs() < 0.01);
    }

    #[test]
    fn test_recurrence_score_low() {
        let score = recurrence_score(1);
        assert!(score < 10.0);
    }

    #[test]
    fn test_recurrence_score_medium() {
        let score = recurrence_score(10);
        // At midpoint, sigmoid is ~50
        assert!(score > 40.0 && score < 60.0);
    }

    #[test]
    fn test_recurrence_score_high() {
        let score = recurrence_score(30);
        assert!(score > 90.0);
    }

    #[test]
    fn test_aggregate_neighbor_features() {
        let mut g = AdjacencyGraph::new();
        g.add_edge("center", "a", 0.5);
        g.add_edge("center", "b", 0.5);

        let mut features = HashMap::new();
        features.insert("a".to_string(), vec![10.0, 20.0]);
        features.insert("b".to_string(), vec![30.0, 40.0]);

        let result = aggregate_neighbor_features(&g, &features, "center").unwrap();
        // Weighted average: (10*0.5 + 30*0.5)/1.0 = 20.0, (20*0.5 + 40*0.5)/1.0 = 30.0
        assert!((result[0] - 20.0).abs() < 0.01);
        assert!((result[1] - 30.0).abs() < 0.01);
    }

    #[test]
    fn test_aggregate_neighbor_features_none() {
        let g = AdjacencyGraph::new();
        let features = HashMap::new();
        // no neighbors
        assert!(aggregate_neighbor_features(&g, &features, "isolated").is_none());
    }

    #[test]
    fn test_compute_centrality_scores() {
        let mut g = AdjacencyGraph::new();
        g.add_bidi_edge("center", "a", 1.0);
        g.add_bidi_edge("center", "b", 1.0);
        g.add_bidi_edge("center", "c", 1.0);

        let scores = compute_centrality_scores(&g);
        // Center should have the highest score (100)
        assert!((scores["center"] - 100.0).abs() < 1.0);
    }

    #[test]
    fn test_role_drift_score_no_change() {
        let history = vec![(1000, 70.0), (2000, 70.0)];
        assert!((role_drift_score(&history) - 0.0).abs() < 0.01);
    }

    #[test]
    fn test_role_drift_score_big_change() {
        let history = vec![(1000, 30.0), (2000, 95.0)];
        let drift = role_drift_score(&history);
        assert!(drift > 0.5);
    }

    #[test]
    fn test_role_drift_score_single_entry() {
        let history = vec![(1000, 70.0)];
        assert!((role_drift_score(&history) - 0.0).abs() < 0.01);
    }

    #[test]
    fn test_network_leverage() {
        let mut g = AdjacencyGraph::new();
        g.add_edge("alice", "supplier_a", 1.0);
        g.add_edge("alice", "supplier_b", 1.0);
        g.add_edge("alice", "unrelated", 1.0);

        let ecosystem = vec!["supplier_a".to_string(), "supplier_b".to_string()];
        let leverage = network_leverage(&g, "alice", &ecosystem);
        assert_eq!(leverage.len(), 2);
        assert!(leverage.contains(&"supplier_a".to_string()));
        assert!(leverage.contains(&"supplier_b".to_string()));
    }

    #[test]
    fn test_network_leverage_indirect() {
        let mut g = AdjacencyGraph::new();
        g.add_edge("alice", "bridge", 1.0);
        g.add_edge("starz_node", "bridge", 1.0);

        let ecosystem = vec!["starz_node".to_string()];
        let leverage = network_leverage(&g, "alice", &ecosystem);
        // bridge is a shared connection
        assert!(leverage.contains(&"bridge".to_string()));
    }
}
