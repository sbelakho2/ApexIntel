/// Graph-based risk propagation.

use std::collections::HashMap;

/// Propagate risk scores through a graph adjacency list.
///
/// `adjacency`: node -> Vec<(neighbor, weight)>
/// `initial_risk`: seed risk scores for nodes
/// `hops`: number of propagation rounds
/// `decay`: multiplicative decay factor per hop
///
/// Returns final risk scores for all reachable nodes, capped at 1.0.
pub fn propagate(
    adjacency: &HashMap<String, Vec<(String, f64)>>,
    initial_risk: &HashMap<String, f64>,
    hops: u8,
    decay: f64,
) -> HashMap<String, f64> {
    let mut risk = initial_risk.clone();

    for _ in 0..hops {
        let mut new_risk = risk.clone();
        for (node, neighbors) in adjacency {
            for (neighbor, weight) in neighbors {
                let propagated = risk.get(node).unwrap_or(&0.0) * weight * decay;
                let entry = new_risk.entry(neighbor.clone()).or_insert(0.0);
                *entry = (*entry + propagated).min(1.0);
            }
        }
        risk = new_risk;
    }
    risk
}

/// Compute contagion score: how much risk a node receives from its neighbors.
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

/// Identify high-risk clusters: nodes above threshold and their immediate neighbors.
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

    // Simple clustering: group high-risk nodes that are neighbors
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
            if let Some(neighbors) = adjacency.get(&current) {
                for (neighbor, _) in neighbors {
                    if high_risk.contains(neighbor) && !visited.contains(neighbor) {
                        visited.insert(neighbor.clone());
                        cluster.push(neighbor.clone());
                        queue.push_back(neighbor.clone());
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
        assert!((*result.get("B").unwrap() - 0.36).abs() < 0.01, "B = {}", result.get("B").unwrap());
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
        adj.insert(
            "X".to_string(),
            vec![("Y".to_string(), 1.0)],
        );
        adj.insert(
            "Z".to_string(),
            vec![("Y".to_string(), 1.0)],
        );

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
}
