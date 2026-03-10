use std::collections::HashMap;

/// Canonical weighted risk propagation shared by graph and stats modules.
///
/// Risk scores are bounded to `[0.0, 1.0]`, negative edge weights are treated
/// as `0.0`, and each hop only propagates the newly absorbed frontier so decay
/// remains geometric rather than compounding previously accumulated totals.
pub fn propagate_weighted_risk(
    adjacency: &HashMap<String, Vec<(String, f64)>>,
    initial_risk: &HashMap<String, f64>,
    hops: u8,
    decay: f64,
) -> HashMap<String, f64> {
    let effective_decay = decay.clamp(0.0, 1.0);
    let mut total_risk = initial_risk.clone();
    let mut frontier = initial_risk.clone();

    for _ in 0..hops {
        let mut new_frontier: HashMap<String, f64> = HashMap::new();
        for (node, neighbors) in adjacency {
            if let Some(&risk) = frontier.get(node) {
                for (neighbor, weight) in neighbors {
                    let effective_weight = weight.clamp(0.0, 1.0);
                    let propagated = risk * effective_weight * effective_decay;
                    if propagated > 1e-12 {
                        *new_frontier.entry(neighbor.clone()).or_insert(0.0) += propagated;
                    }
                }
            }
        }

        for (node, delta) in new_frontier.iter_mut() {
            let entry = total_risk.entry(node.clone()).or_insert(0.0);
            let old = *entry;
            *entry = (old + *delta).min(1.0);
            *delta = *entry - old;
        }

        frontier = new_frontier;
    }

    total_risk
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn propagate_weighted_risk_accumulates_overlapping_sources() {
        let mut adjacency = HashMap::new();
        adjacency.insert("A".to_string(), vec![("C".to_string(), 1.0)]);
        adjacency.insert("B".to_string(), vec![("C".to_string(), 1.0)]);

        let mut initial = HashMap::new();
        initial.insert("A".to_string(), 0.6);
        initial.insert("B".to_string(), 0.5);

        let result = propagate_weighted_risk(&adjacency, &initial, 1, 1.0);

        assert_eq!(result.get("C").copied(), Some(1.0));
    }

    #[test]
    fn propagate_weighted_risk_negative_decay_blocks_propagation() {
        let mut adjacency = HashMap::new();
        adjacency.insert("A".to_string(), vec![("B".to_string(), 1.0)]);

        let mut initial = HashMap::new();
        initial.insert("A".to_string(), 0.8);

        let result = propagate_weighted_risk(&adjacency, &initial, 2, -0.5);

        assert_eq!(result.get("A").copied(), Some(0.8));
        assert_eq!(result.get("B").copied().unwrap_or(0.0), 0.0);
    }
}
