/// Graph-based risk propagation.
///
/// Risk scores are represented as `f64` values in `[0.0, 1.0]`.  Edge weights
/// should be non-negative (negative weights are clamped to `0.0`).  The
/// `decay` factor should be in `(0.0, 1.0]`; values outside this range are
/// clamped so that total risk can never exceed `1.0` per node.
use std::collections::HashMap;

use apex_core::graph_risk::propagate_weighted_risk;

/// Propagate risk scores through a graph adjacency list.
///
/// `adjacency`: node → `Vec<(neighbour, weight)>`.
///   Negative weights are clamped to `0.0` (B263).
/// `initial_risk`: seed risk scores for nodes.
/// `hops`: number of propagation rounds.
/// `decay`: multiplicative decay factor per hop, clamped to `[0.0, 1.0]`.
///
/// Returns final risk scores for all reachable nodes, capped at `1.0`.
pub fn propagate(
    adjacency: &HashMap<String, Vec<(String, f64)>>,
    initial_risk: &HashMap<String, f64>,
    hops: u8,
    decay: f64,
) -> HashMap<String, f64> {
    propagate_weighted_risk(adjacency, initial_risk, hops, decay)
}

/// Compute contagion score: how much risk a node receives from its neighbours.
///
/// Iterates the adjacency list to find all nodes that list `node` as a
/// neighbour, and accumulates `source_risk × edge_weight`.  Returns the
/// value clamped to `[0.0, 1.0]`.
pub fn contagion_score(
    adjacency: &HashMap<String, Vec<(String, f64)>>,
    risk_scores: &HashMap<String, f64>,
    node: &str,
) -> f64 {
    let mut total = 0.0;
    // Check all nodes to find those that have `node` as a neighbor
    for (source, neighbors) in adjacency {
        for (neighbor, weight) in neighbors {
            if neighbor == node {
                total += risk_scores.get(source).unwrap_or(&0.0) * weight;
            }
        }
    }
    total.min(1.0)
}

/// Identify high-risk clusters: connected components of nodes above threshold.
///
/// A cluster is a set of high-risk nodes (score ≥ `threshold`) that are
/// transitively connected in an undirected interpretation of the adjacency
/// list.  Isolated high-risk nodes (no high-risk neighbours) each form their
/// own singleton cluster.
///
/// Returns clusters in BFS discovery order; nodes within each cluster are
/// sorted lexicographically for deterministic output.
pub fn high_risk_cluster(
    adjacency: &HashMap<String, Vec<(String, f64)>>,
    risk_scores: &HashMap<String, f64>,
    threshold: f64,
) -> Vec<Vec<String>> {
    let high_risk: Vec<String> = risk_scores
        .iter()
        .filter(|(_, &v)| v >= threshold)
        .map(|(k, _)| k.clone())
        .collect();

    if high_risk.is_empty() {
        return vec![];
    }

    // O(1) membership set — avoids O(n) `Vec::contains` scans when building
    // the undirected adjacency list below.
    let high_risk_set: std::collections::HashSet<&str> =
        high_risk.iter().map(|s| s.as_str()).collect();

    // Build undirected adjacency among high-risk nodes so cluster detection is
    // order-independent (directed graph: A→B means A and B share a cluster even
    // when BFS starts from B and B has no outgoing edge back to A).
    let mut undirected: std::collections::HashMap<&str, Vec<&str>> =
        std::collections::HashMap::new();
    for (node, neighbors) in adjacency {
        for (neighbor, _) in neighbors {
            if high_risk_set.contains(node.as_str()) && high_risk_set.contains(neighbor.as_str()) {
                undirected
                    .entry(node.as_str())
                    .or_default()
                    .push(neighbor.as_str());
                undirected
                    .entry(neighbor.as_str())
                    .or_default()
                    .push(node.as_str());
            }
        }
    }

    // Simple clustering: group high-risk nodes that are connected (undirected BFS)
    let mut visited: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut clusters = Vec::new();

    for node in &high_risk {
        if visited.contains(node) {
            continue;
        }

        let mut cluster = vec![node.clone()];
        visited.insert(node.clone());

        // BFS within high-risk nodes
        let mut queue = std::collections::VecDeque::new();
        queue.push_back(node.clone());

        while let Some(current) = queue.pop_front() {
            if let Some(neighbors) = undirected.get(current.as_str()) {
                for &neighbor in neighbors {
                    if !visited.contains(neighbor) {
                        visited.insert(neighbor.to_string());
                        cluster.push(neighbor.to_string());
                        queue.push_back(neighbor.to_string());
                    }
                }
            }
        }

        cluster.sort();
        clusters.push(cluster);
    }

    clusters
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_adjacency() -> HashMap<String, Vec<(String, f64)>> {
        let mut adj = HashMap::new();
        adj.insert(
            "A".to_string(),
            vec![("B".to_string(), 0.8), ("C".to_string(), 0.5)],
        );
        adj.insert("B".to_string(), vec![("D".to_string(), 0.7)]);
        adj.insert("C".to_string(), vec![("D".to_string(), 0.3)]);
        adj
    }

    #[test]
    fn test_propagate_one_hop() {
        let adj = sample_adjacency();
        let mut initial = HashMap::new();
        initial.insert("A".to_string(), 0.9);

        let result = propagate(&adj, &initial, 1, 0.5);

        assert!(*result.get("A").unwrap() >= 0.9);
        // B gets: 0.9 * 0.8 * 0.5 = 0.36
        assert!(
            (*result.get("B").unwrap() - 0.36).abs() < 0.01,
            "B = {}",
            result.get("B").unwrap()
        );
        // C gets: 0.9 * 0.5 * 0.5 = 0.225
        assert!((*result.get("C").unwrap() - 0.225).abs() < 0.01);
    }

    #[test]
    fn test_propagate_two_hops() {
        let adj = sample_adjacency();
        let mut initial = HashMap::new();
        initial.insert("A".to_string(), 0.9);

        let result = propagate(&adj, &initial, 2, 0.5);
        // D should receive risk from both B and C after 2 hops
        assert!(*result.get("D").unwrap_or(&0.0) > 0.0);
    }

    #[test]
    fn test_propagate_capped() {
        let mut adj = HashMap::new();
        adj.insert("X".to_string(), vec![("Y".to_string(), 1.0)]);
        adj.insert("Z".to_string(), vec![("Y".to_string(), 1.0)]);

        let mut initial = HashMap::new();
        initial.insert("X".to_string(), 1.0);
        initial.insert("Z".to_string(), 1.0);

        let result = propagate(&adj, &initial, 3, 1.0);
        assert!(*result.get("Y").unwrap() <= 1.0);
    }

    #[test]
    fn test_propagate_empty() {
        let adj = HashMap::new();
        let initial = HashMap::new();
        let result = propagate(&adj, &initial, 5, 0.5);
        assert!(result.is_empty());
    }

    #[test]
    fn test_contagion_score() {
        let adj = sample_adjacency();
        let mut risk = HashMap::new();
        risk.insert("A".to_string(), 0.9);
        risk.insert("B".to_string(), 0.5);

        // D is neighbor of B (weight 0.7) and C (weight 0.3)
        let score = contagion_score(&adj, &risk, "D");
        // B contributes: 0.5 * 0.7 = 0.35
        // C contributes: 0.0 * 0.3 = 0.0 (C has no risk)
        assert!((score - 0.35).abs() < 0.01);
    }

    #[test]
    fn test_high_risk_cluster() {
        let adj = sample_adjacency();
        let mut risk = HashMap::new();
        risk.insert("A".to_string(), 0.9);
        risk.insert("B".to_string(), 0.8);
        risk.insert("C".to_string(), 0.3);
        risk.insert("D".to_string(), 0.1);

        let clusters = high_risk_cluster(&adj, &risk, 0.7);
        // A and B are above threshold and connected
        assert_eq!(clusters.len(), 1);
        assert!(clusters[0].contains(&"A".to_string()));
        assert!(clusters[0].contains(&"B".to_string()));
    }

    #[test]
    fn test_high_risk_cluster_none() {
        let adj = sample_adjacency();
        let mut risk = HashMap::new();
        risk.insert("A".to_string(), 0.1);
        risk.insert("B".to_string(), 0.2);

        let clusters = high_risk_cluster(&adj, &risk, 0.9);
        assert!(clusters.is_empty());
    }

    // ── B263: negative weights in propagate ─────────────────────────────────

    #[test]
    fn test_propagate_negative_weight_treated_as_zero() {
        // A negative weight should not decrease target node's risk.
        let mut adj = HashMap::new();
        adj.insert("A".to_string(), vec![("B".to_string(), -1.0)]);

        let mut initial = HashMap::new();
        initial.insert("A".to_string(), 0.8);
        initial.insert("B".to_string(), 0.5); // B already has some risk

        let result = propagate(&adj, &initial, 1, 1.0);
        // B's risk must not go below its initial value since the negative edge
        // is clamped to 0.0 and contributes nothing.
        let b_risk = *result.get("B").unwrap();
        assert!(
            b_risk >= 0.5,
            "negative weight should not reduce B's risk; got {b_risk}"
        );
        assert!(b_risk <= 1.0, "risk must not exceed 1.0");
    }

    #[test]
    fn test_propagate_negative_decay_treated_as_zero() {
        // Negative decay should clamp to 0 → no propagation at all.
        let adj = sample_adjacency();
        let mut initial = HashMap::new();
        initial.insert("A".to_string(), 0.9);

        let result = propagate(&adj, &initial, 2, -0.5);
        // With decay=0 no risk propagates beyond the seed.
        assert_eq!(*result.get("A").unwrap(), 0.9);
        assert!(
            result.get("B").is_none() || *result.get("B").unwrap() < 1e-12,
            "negative decay must block propagation"
        );
    }

    #[test]
    fn test_propagate_accumulates_overlapping_sources() {
        let mut adj = HashMap::new();
        adj.insert("A".to_string(), vec![("C".to_string(), 1.0)]);
        adj.insert("B".to_string(), vec![("C".to_string(), 1.0)]);

        let mut initial = HashMap::new();
        initial.insert("A".to_string(), 0.6);
        initial.insert("B".to_string(), 0.5);

        let result = propagate(&adj, &initial, 1, 1.0);
        assert_eq!(result.get("C").copied(), Some(1.0));
    }

    // ── B264: isolated high-risk nodes each form singleton clusters ──────────

    #[test]
    fn test_high_risk_cluster_isolated_nodes_form_singletons() {
        // Three high-risk nodes with no edges to each other → three singletons.
        let mut adj: HashMap<String, Vec<(String, f64)>> = HashMap::new();
        // Only cross-edges to low-risk nodes, never between X, Y, Z.
        adj.insert("X".to_string(), vec![("low1".to_string(), 0.5)]);
        adj.insert("Y".to_string(), vec![("low2".to_string(), 0.3)]);
        adj.insert("Z".to_string(), vec![]);

        let mut risk = HashMap::new();
        risk.insert("X".to_string(), 0.9);
        risk.insert("Y".to_string(), 0.85);
        risk.insert("Z".to_string(), 0.95);
        risk.insert("low1".to_string(), 0.1);
        risk.insert("low2".to_string(), 0.2);

        let mut clusters = high_risk_cluster(&adj, &risk, 0.8);
        // Sort clusters lexicographically for deterministic assertion
        clusters.sort_by(|a, b| a[0].cmp(&b[0]));

        assert_eq!(
            clusters.len(),
            3,
            "expected 3 singleton clusters; got {clusters:?}"
        );
        for cluster in &clusters {
            assert_eq!(
                cluster.len(),
                1,
                "each isolated node must form a singleton: {cluster:?}"
            );
        }
        let all_nodes: Vec<_> = clusters.iter().map(|c| c[0].as_str()).collect();
        assert!(all_nodes.contains(&"X"));
        assert!(all_nodes.contains(&"Y"));
        assert!(all_nodes.contains(&"Z"));
    }

    #[test]
    fn test_high_risk_cluster_single_isolated_node() {
        // A single high-risk node with no adjacency → one singleton cluster.
        let adj: HashMap<String, Vec<(String, f64)>> = HashMap::new();
        let mut risk = HashMap::new();
        risk.insert("OnlyNode".to_string(), 0.95);

        let clusters = high_risk_cluster(&adj, &risk, 0.8);
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0], vec!["OnlyNode"]);
    }

    #[test]
    fn test_high_risk_cluster_mixed_connected_and_isolated() {
        // A and B connected (both high-risk); C is isolated high-risk node.
        let mut adj = sample_adjacency(); // A→B, A→C, B→D, C→D
        let mut risk = HashMap::new();
        risk.insert("A".to_string(), 0.9);
        risk.insert("B".to_string(), 0.85);
        risk.insert("C".to_string(), 0.0); // low — sample_adjacency's C has 0.3
        risk.insert("high_isolated".to_string(), 0.95);
        adj.insert("high_isolated".to_string(), vec![]); // no neighbours

        let mut clusters = high_risk_cluster(&adj, &risk, 0.8);

        // A and B form one cluster; high_isolated forms a singleton.
        // C is below threshold so ignored.
        assert!(
            clusters.len() >= 2,
            "expected ≥2 clusters (connected pair + singleton): {clusters:?}"
        );

        // Sort by first element for determinism
        clusters.sort_by(|a, b| a[0].cmp(&b[0]));
        let all_node_names: Vec<_> = clusters.iter().flatten().map(String::as_str).collect();
        assert!(all_node_names.contains(&"A"), "A should be in a cluster");
        assert!(all_node_names.contains(&"B"), "B should be in a cluster");
        assert!(
            all_node_names.contains(&"high_isolated"),
            "isolated node must appear"
        );
    }
}
