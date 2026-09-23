use rand::{rngs::StdRng, Rng, SeedableRng};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum DecayModel {
    Linear(f64),
    Exponential { lambda: f64 },
    InverseSquare,
    Step { max_hops: u8 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContagionDistributionSummary {
    pub per_node_mean: HashMap<String, f64>,
    pub per_node_p05: HashMap<String, f64>,
    pub per_node_p95: HashMap<String, f64>,
}

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
    propagate_weighted_risk_with_decay(adjacency, initial_risk, hops, DecayModel::Linear(decay))
}

/// Propagates risk through the graph with decay, tracking saturation accurately.
///
/// The propagation maintains accurate risk accumulation by:
/// 1. Clamping individual propagated values before accumulation
/// 2. Using C1-continuous soft saturation (piecewise exponential with knee at 0.8) instead of hard clamping to preserve relative differences
/// 3. Clamping final results to [0,1] only at output time
pub fn propagate_weighted_risk_with_decay(
    adjacency: &HashMap<String, Vec<(String, f64)>>,
    initial_risk: &HashMap<String, f64>,
    hops: u8,
    decay_model: DecayModel,
) -> HashMap<String, f64> {
    let mut total_risk = initial_risk.clone();
    let mut frontier = initial_risk.clone();

    for hop_idx in 0..hops {
        let mut new_frontier: HashMap<String, f64> = HashMap::new();
        let hop_decay = decay_model.hop_multiplier(hop_idx + 1);
        for (node, neighbors) in adjacency {
            if let Some(&risk) = frontier.get(node) {
                for (neighbor, weight) in neighbors {
                    let effective_weight = weight.clamp(0.0, 1.0);
                    let propagated = risk * effective_weight * hop_decay;
                    if propagated > 1e-12 {
                        *new_frontier.entry(neighbor.clone()).or_insert(0.0) += propagated;
                    }
                }
            }
        }

        for (node, delta) in new_frontier.iter_mut() {
            let entry = total_risk.entry(node.clone()).or_insert(0.0);
            let old = *entry;
            *entry = soft_saturate(old + *delta);
            // Frontier delta = actual risk increment (for next-hop propagation)
            *delta = *entry - old;
        }

        frontier = new_frontier;
    }

    // Final clamp to ensure output is strictly in [0,1]
    for value in total_risk.values_mut() {
        *value = value.clamp(0.0, 1.0);
    }

    total_risk
}

/// C1-continuous soft saturation that preserves relative risk differences.
///
/// Below the knee (0.8) the function is the identity, preserving exact values
/// in the normal range.  Above the knee it transitions to an exponential
/// approach toward 1.0, so nodes hit by many risk sources remain
/// distinguishable instead of all hard-clamping to 1.0.
///
/// Properties:
/// - f(0) = 0, f(KNEE) = KNEE, lim f(x->inf) = 1
/// - f and f' are continuous everywhere (C1 at the knee)
/// - Monotonically increasing
fn soft_saturate(x: f64) -> f64 {
    // Risk level at which the curve starts bending toward 1.0.
    const KNEE: f64 = 0.8;
    if x.is_nan() {
        // Non-finite input must not poison the [0,1] risk contract.
        return 0.0;
    }
    if x <= 0.0 {
        0.0
    } else if x <= KNEE {
        x
    } else {
        let headroom = 1.0 - KNEE;
        // 1 - exp(0) = 0 at the knee, giving continuity: KNEE + 0 = KNEE.
        // Derivative at the knee: headroom * (1/headroom) * exp(0) = 1, matching
        // the identity slope from the left (C1).
        KNEE + headroom * (1.0 - (-(x - KNEE) / headroom).exp())
    }
}

pub fn validate_monotonic_non_increasing(
    adjacency: &HashMap<String, Vec<(String, f64)>>,
    initial_risk: &HashMap<String, f64>,
    hops: u8,
    decay_model: DecayModel,
) -> bool {
    let hop_maxima = incremental_hop_maxima(adjacency, initial_risk, hops, decay_model);
    hop_maxima
        .windows(2)
        .all(|window| window[1] <= window[0] + 1e-12)
}

pub fn simulate_contagion_distribution(
    adjacency: &HashMap<String, Vec<(String, f64)>>,
    initial_risk: &HashMap<String, f64>,
    hops: u8,
    decay_model: DecayModel,
    edge_failure_probability: f64,
    simulations: usize,
    seed: u64,
) -> ContagionDistributionSummary {
    let mut rng = StdRng::seed_from_u64(seed);
    let simulations = simulations.max(1);
    let failure_probability = edge_failure_probability.clamp(0.0, 1.0);
    let mut per_node_samples: HashMap<String, Vec<f64>> = HashMap::new();
    let mut all_nodes: Vec<String> = adjacency.keys().cloned().collect();

    for neighbors in adjacency.values() {
        for (neighbor, _) in neighbors {
            if !all_nodes.iter().any(|node| node == neighbor) {
                all_nodes.push(neighbor.clone());
            }
        }
    }

    for node in initial_risk.keys() {
        if !all_nodes.iter().any(|existing| existing == node) {
            all_nodes.push(node.clone());
        }
    }

    for _ in 0..simulations {
        let sampled_adjacency: HashMap<String, Vec<(String, f64)>> = adjacency
            .iter()
            .map(|(from, neighbors)| {
                let sampled_neighbors: Vec<(String, f64)> = neighbors
                    .iter()
                    .filter_map(|(neighbor, weight)| {
                        if rng.gen_bool(1.0 - failure_probability) {
                            Some((neighbor.clone(), *weight))
                        } else {
                            None
                        }
                    })
                    .collect();
                (from.clone(), sampled_neighbors)
            })
            .collect();
        let propagated =
            propagate_weighted_risk_with_decay(&sampled_adjacency, initial_risk, hops, decay_model);
        for node in &all_nodes {
            let risk = propagated.get(node).copied().unwrap_or(0.0);
            per_node_samples.entry(node.clone()).or_default().push(risk);
        }
    }

    let per_node_mean = per_node_samples
        .iter()
        .map(|(node, samples)| {
            let mean = samples.iter().sum::<f64>() / samples.len() as f64;
            (node.clone(), mean)
        })
        .collect();
    let per_node_p05 = per_node_samples
        .iter()
        .map(|(node, samples)| (node.clone(), percentile(samples, 0.05)))
        .collect();
    let per_node_p95 = per_node_samples
        .iter()
        .map(|(node, samples)| (node.clone(), percentile(samples, 0.95)))
        .collect();

    ContagionDistributionSummary {
        per_node_mean,
        per_node_p05,
        per_node_p95,
    }
}

fn incremental_hop_maxima(
    adjacency: &HashMap<String, Vec<(String, f64)>>,
    initial_risk: &HashMap<String, f64>,
    hops: u8,
    decay_model: DecayModel,
) -> Vec<f64> {
    let mut frontier = initial_risk.clone();
    let mut maxima = Vec::new();

    for hop_idx in 0..hops {
        let mut new_frontier: HashMap<String, f64> = HashMap::new();
        let hop_decay = decay_model.hop_multiplier(hop_idx + 1);
        for (node, neighbors) in adjacency {
            if let Some(&risk) = frontier.get(node) {
                for (neighbor, weight) in neighbors {
                    let effective_weight = weight.clamp(0.0, 1.0);
                    let propagated = risk * effective_weight * hop_decay;
                    if propagated > 1e-12 {
                        *new_frontier.entry(neighbor.clone()).or_insert(0.0) += propagated;
                    }
                }
            }
        }
        maxima.push(new_frontier.values().copied().fold(0.0_f64, f64::max));
        frontier = new_frontier;
    }

    maxima
}

fn percentile(samples: &[f64], quantile: f64) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let index = ((sorted.len() - 1) as f64 * quantile.clamp(0.0, 1.0)).round() as usize;
    sorted[index]
}

impl DecayModel {
    pub fn hop_multiplier(self, hop: u8) -> f64 {
        match self {
            Self::Linear(factor) => factor.clamp(0.0, 1.0).powi(hop as i32),
            Self::Exponential { lambda } => (-(lambda.max(0.0)) * hop as f64).exp().clamp(0.0, 1.0),
            Self::InverseSquare => 1.0 / (1.0 + hop as f64).powi(2),
            Self::Step { max_hops } => {
                if hop <= max_hops {
                    1.0
                } else {
                    0.0
                }
            }
        }
    }
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

        // With soft saturation the raw sum 1.1 maps to ~0.95 instead of
        // hard-clamping to 1.0, preserving relative risk ordering.
        let c_risk = result.get("C").copied().unwrap_or(0.0);
        assert!(
            c_risk > 0.9,
            "expected high risk from two overlapping sources, got {c_risk}"
        );
        assert!(c_risk <= 1.0, "risk must not exceed 1.0, got {c_risk}");
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

    #[test]
    fn monotonic_validation_holds_for_exponential_decay() {
        let mut adjacency = HashMap::new();
        adjacency.insert("A".to_string(), vec![("B".to_string(), 1.0)]);
        adjacency.insert("B".to_string(), vec![("C".to_string(), 1.0)]);
        adjacency.insert("C".to_string(), vec![("D".to_string(), 1.0)]);

        let mut initial = HashMap::new();
        initial.insert("A".to_string(), 1.0);

        assert!(validate_monotonic_non_increasing(
            &adjacency,
            &initial,
            3,
            DecayModel::Exponential { lambda: 0.7 },
        ));
    }

    #[test]
    fn step_decay_blocks_after_cutoff() {
        let mut adjacency = HashMap::new();
        adjacency.insert("A".to_string(), vec![("B".to_string(), 1.0)]);
        adjacency.insert("B".to_string(), vec![("C".to_string(), 1.0)]);

        let mut initial = HashMap::new();
        initial.insert("A".to_string(), 1.0);

        let result = propagate_weighted_risk_with_decay(
            &adjacency,
            &initial,
            3,
            DecayModel::Step { max_hops: 1 },
        );
        assert!(result.contains_key("B"));
        assert!(!result.contains_key("C") || result.get("C").copied().unwrap_or(0.0) < 1e-12);
    }

    #[test]
    fn contagion_simulation_reports_distribution_bounds() {
        let mut adjacency = HashMap::new();
        adjacency.insert("A".to_string(), vec![("B".to_string(), 1.0)]);
        adjacency.insert("B".to_string(), vec![("C".to_string(), 1.0)]);

        let mut initial = HashMap::new();
        initial.insert("A".to_string(), 0.9);

        let summary = simulate_contagion_distribution(
            &adjacency,
            &initial,
            2,
            DecayModel::Linear(0.5),
            0.2,
            128,
            42,
        );
        let mean_b = summary.per_node_mean.get("B").copied().unwrap_or(0.0);
        let p05_b = summary.per_node_p05.get("B").copied().unwrap_or(0.0);
        let p95_b = summary.per_node_p95.get("B").copied().unwrap_or(0.0);
        assert!(mean_b > 0.0);
        assert!(p05_b <= mean_b && mean_b <= p95_b);
    }
}
