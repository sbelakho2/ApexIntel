use std::collections::{HashMap, HashSet, VecDeque};

use apex_core::similarity::shared_member_count;
use chrono::{DateTime, NaiveDate, Utc};

use apex_core::graph_risk::propagate_weighted_risk;

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EdgeType {
    Generic,
    SupplierOf,
    CompetesWith,
    SubsidiaryOf,
    FormerEmployeeOf,
    BoardMemberOf,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypedEdge {
    pub neighbor_id: String,
    pub weight: f64,
    pub edge_type: EdgeType,
    pub observed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GraphSnapshot {
    pub week_start: NaiveDate,
    pub edges: HashMap<String, Vec<TypedEdge>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CommunityMigration {
    pub node_id: String,
    pub from_community: String,
    pub to_community: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SnapshotDelta {
    pub new_edges: Vec<(String, String, EdgeType)>,
    pub removed_edges: Vec<(String, String, EdgeType)>,
    pub community_migrations: Vec<CommunityMigration>,
}

/// An adjacency list representation for the entity graph.
#[derive(Debug, Clone)]
pub struct AdjacencyGraph {
    /// node_id -> Vec<(neighbor_id, weight)>
    edges: HashMap<String, Vec<(String, f64)>>,
    typed_edges: HashMap<String, Vec<TypedEdge>>,
    snapshots: Vec<GraphSnapshot>,
}

impl AdjacencyGraph {
    pub fn new() -> Self {
        Self {
            edges: HashMap::new(),
            typed_edges: HashMap::new(),
            snapshots: Vec::new(),
        }
    }

    /// Add a directed edge from `from` to `to` with a given weight.
    ///
    /// B152: negative weights are clamped to 0.0.
    pub fn add_edge(&mut self, from: &str, to: &str, weight: f64) {
        self.add_typed_edge(from, to, weight, EdgeType::Generic, None);
    }

    pub fn add_typed_edge(
        &mut self,
        from: &str,
        to: &str,
        weight: f64,
        edge_type: EdgeType,
        observed_at: Option<DateTime<Utc>>,
    ) {
        let w = if !weight.is_finite() || weight < 0.0 {
            0.0
        } else {
            weight
        };
        self.edges
            .entry(from.to_string())
            .or_default()
            .push((to.to_string(), w));
        self.typed_edges
            .entry(from.to_string())
            .or_default()
            .push(TypedEdge {
                neighbor_id: to.to_string(),
                weight: w,
                edge_type,
                observed_at,
            });
    }

    /// Add a bidirectional edge.
    pub fn add_bidi_edge(&mut self, a: &str, b: &str, weight: f64) {
        self.add_edge(a, b, weight);
        self.add_edge(b, a, weight);
    }

    pub fn add_typed_bidi_edge(
        &mut self,
        a: &str,
        b: &str,
        weight: f64,
        edge_type: EdgeType,
        observed_at: Option<DateTime<Utc>>,
    ) {
        self.add_typed_edge(a, b, weight, edge_type.clone(), observed_at);
        self.add_typed_edge(b, a, weight, edge_type, observed_at);
    }

    /// Get neighbors and weights for a node.
    pub fn neighbors(&self, node: &str) -> &[(String, f64)] {
        self.edges.get(node).map(|v| v.as_slice()).unwrap_or(&[])
    }

    pub fn typed_neighbors(&self, node: &str, edge_type: Option<&EdgeType>) -> Vec<TypedEdge> {
        self.typed_edges
            .get(node)
            .into_iter()
            .flat_map(|edges| edges.iter())
            .filter(|edge| {
                edge_type
                    .map(|kind| &edge.edge_type == kind)
                    .unwrap_or(true)
            })
            .cloned()
            .collect()
    }

    /// All node IDs in the graph.
    pub fn nodes(&self) -> HashSet<String> {
        let mut set = HashSet::new();
        for (node, neighbors) in &self.edges {
            set.insert(node.clone());
            for (n, _) in neighbors {
                set.insert(n.clone());
            }
        }
        set
    }

    /// Number of nodes.
    pub fn node_count(&self) -> usize {
        self.nodes().len()
    }

    /// Number of directed edges.
    pub fn edge_count(&self) -> usize {
        self.edges.values().map(|v| v.len()).sum()
    }

    /// Degree of a node (outgoing edges).
    pub fn out_degree(&self, node: &str) -> usize {
        self.edges.get(node).map(|v| v.len()).unwrap_or(0)
    }

    /// Propagate risk from initial_risk scores through the graph.
    ///
    /// Frontier-based BFS propagation for `hops` rounds with decay factor.
    /// Each hop only propagates from nodes discovered in the previous frontier,
    /// preventing additive accumulation that inflates scores.
    pub fn propagate_risk(
        &self,
        initial_risk: &HashMap<String, f64>,
        hops: u8,
        decay: f64,
    ) -> HashMap<String, f64> {
        propagate_weighted_risk(&self.edges, initial_risk, hops, decay)
    }

    /// Compute PageRank-style centrality scores.
    ///
    /// Uses power iteration with damping factor `d` (typically 0.85).
    ///
    /// `iterations` is treated as a minimum iteration budget. The solver keeps
    /// iterating until the L1 delta between successive rank vectors drops below
    /// `1e-6` or a bounded safety cap is reached.
    pub fn pagerank(&self, iterations: usize, damping: f64) -> HashMap<String, f64> {
        self.pagerank_with_convergence(iterations, damping, 1e-6, iterations.max(128))
            .0
    }

    pub fn pagerank_convergence_l1_delta(&self, iterations: usize, damping: f64) -> Option<f64> {
        self.pagerank_with_convergence(iterations, damping, 1e-6, iterations.max(128))
            .1
    }

    fn pagerank_with_convergence(
        &self,
        minimum_iterations: usize,
        damping: f64,
        tolerance: f64,
        max_iterations: usize,
    ) -> (HashMap<String, f64>, Option<f64>) {
        let mut nodes: Vec<String> = self.nodes().into_iter().collect();
        nodes.sort();
        let n = nodes.len();
        if n == 0 {
            return (HashMap::new(), None);
        }

        let init_score = 1.0 / n as f64;
        let mut scores: HashMap<String, f64> = nodes
            .iter()
            .map(|node| (node.clone(), init_score))
            .collect();
        let mut last_delta = None;

        for iteration in 0..max_iterations.max(minimum_iterations) {
            let previous_scores = scores.clone();
            scores = self.pagerank_step(&nodes, &scores, damping);
            let delta: f64 = nodes
                .iter()
                .map(|node| (scores[node] - previous_scores[node]).abs())
                .sum();
            last_delta = Some(delta);

            if iteration + 1 >= minimum_iterations && delta < tolerance {
                break;
            }
        }

        (scores, last_delta)
    }

    fn pagerank_step(
        &self,
        nodes: &[String],
        scores: &HashMap<String, f64>,
        damping: f64,
    ) -> HashMap<String, f64> {
        let n = nodes.len();
        let base = (1.0 - damping) / n as f64;
        let mut new_scores: HashMap<String, f64> =
            nodes.iter().map(|node| (node.clone(), base)).collect();

        let dangling_mass: f64 = nodes
            .iter()
            .filter(|node| self.out_degree(node) == 0)
            .map(|node| scores[node])
            .sum();
        let dangling_share = damping * dangling_mass / n as f64;

        for target in nodes {
            if let Some(score) = new_scores.get_mut(target) {
                *score += dangling_share;
            }
        }

        for node in nodes {
            let neighbors = self.neighbors(node);
            if !neighbors.is_empty() {
                let total_weight: f64 = neighbors.iter().map(|(_, weight)| weight).sum();
                if total_weight > 0.0 {
                    for (neighbor, weight) in neighbors {
                        if let Some(score) = new_scores.get_mut(neighbor) {
                            *score += damping * scores[node] * weight / total_weight;
                        }
                    }
                }
            }
        }

        new_scores
    }

    /// BFS shortest path (hop count) from source to target.
    pub fn shortest_path(&self, source: &str, target: &str) -> Option<Vec<String>> {
        if source == target {
            return Some(vec![source.to_string()]);
        }

        let mut visited: HashSet<String> = HashSet::new();
        let mut queue: VecDeque<(String, Vec<String>)> = VecDeque::new();

        visited.insert(source.to_string());
        queue.push_back((source.to_string(), vec![source.to_string()]));

        while let Some((node, path)) = queue.pop_front() {
            for (neighbor, _) in self.neighbors(&node) {
                if neighbor == target {
                    let mut result = path.clone();
                    result.push(neighbor.clone());
                    return Some(result);
                }
                if !visited.contains(neighbor) {
                    visited.insert(neighbor.clone());
                    let mut new_path = path.clone();
                    new_path.push(neighbor.clone());
                    queue.push_back((neighbor.clone(), new_path));
                }
            }
        }

        None
    }

    /// Find connected components via undirected BFS.
    ///
    /// Builds an undirected adjacency view so that directed edges are traversed
    /// in both directions — otherwise nodes reachable only via incoming edges
    /// may appear as separate components.
    pub fn connected_components(&self) -> Vec<Vec<String>> {
        // Build undirected adjacency for BFS
        let mut undirected: HashMap<String, HashSet<String>> = HashMap::new();
        for (node, neighbors) in &self.edges {
            for (neighbor, _) in neighbors {
                undirected
                    .entry(node.clone())
                    .or_default()
                    .insert(neighbor.clone());
                undirected
                    .entry(neighbor.clone())
                    .or_default()
                    .insert(node.clone());
            }
        }

        let all_nodes = self.nodes();
        let mut visited: HashSet<String> = HashSet::new();
        let mut components = Vec::new();

        for node in &all_nodes {
            if visited.contains(node) {
                continue;
            }
            let mut component = Vec::new();
            let mut queue = VecDeque::new();
            queue.push_back(node.clone());
            visited.insert(node.clone());

            while let Some(n) = queue.pop_front() {
                component.push(n.clone());
                if let Some(neighbors) = undirected.get(&n) {
                    for neighbor in neighbors {
                        if !visited.contains(neighbor) {
                            visited.insert(neighbor.clone());
                            queue.push_back(neighbor.clone());
                        }
                    }
                }
            }
            components.push(component);
        }

        components
    }

    /// Co-appearance count: how many shared neighbors two nodes have.
    pub fn co_appearance_count(&self, a: &str, b: &str) -> usize {
        let neighbors_a: HashSet<String> =
            self.neighbors(a).iter().map(|(n, _)| n.clone()).collect();
        let neighbors_b: HashSet<String> =
            self.neighbors(b).iter().map(|(n, _)| n.clone()).collect();
        shared_member_count(&neighbors_a, &neighbors_b)
    }

    pub fn typed_reachable_nodes(
        &self,
        source: &str,
        max_hops: usize,
        allowed_edge_types: &HashSet<EdgeType>,
    ) -> HashSet<String> {
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        visited.insert(source.to_string());
        queue.push_back((source.to_string(), 0usize));

        while let Some((node, hops)) = queue.pop_front() {
            if hops >= max_hops {
                continue;
            }
            for edge in self.typed_neighbors(&node, None) {
                if !allowed_edge_types.contains(&edge.edge_type) {
                    continue;
                }
                if visited.insert(edge.neighbor_id.clone()) {
                    queue.push_back((edge.neighbor_id, hops + 1));
                }
            }
        }

        visited.remove(source);
        visited
    }

    pub fn louvain_communities(&self) -> Vec<Vec<String>> {
        let weights = self.undirected_weight_map();
        let mut nodes: Vec<String> = weights.keys().cloned().collect();
        nodes.sort();
        let mut community_of: HashMap<String, String> = nodes
            .iter()
            .map(|node| (node.clone(), node.clone()))
            .collect();
        let degrees: HashMap<String, f64> = nodes
            .iter()
            .map(|node| {
                let degree = weights
                    .get(node)
                    .map(|neighbors| neighbors.values().copied().sum())
                    .unwrap_or(0.0);
                (node.clone(), degree)
            })
            .collect();
        let total_weight_twice: f64 = degrees.values().sum::<f64>().max(1e-12);

        let mut changed = true;
        let mut passes = 0;
        while changed && passes < 20 {
            changed = false;
            passes += 1;
            for node in &nodes {
                let node_degree = degrees.get(node).copied().unwrap_or(0.0);
                let current = community_of
                    .get(node)
                    .cloned()
                    .unwrap_or_else(|| node.clone());
                let mut candidate_communities: HashSet<String> = HashSet::from([current.clone()]);
                if let Some(neighbors) = weights.get(node) {
                    for neighbor in neighbors.keys() {
                        if let Some(community) = community_of.get(neighbor) {
                            candidate_communities.insert(community.clone());
                        }
                    }
                }

                let mut best_community = current.clone();
                let mut best_gain = 0.0;
                for candidate in candidate_communities {
                    let gain = self.modularity_gain(
                        node,
                        &candidate,
                        &community_of,
                        &weights,
                        &degrees,
                        node_degree,
                        total_weight_twice,
                    );
                    if gain > best_gain + 1e-12
                        || ((gain - best_gain).abs() < 1e-12 && candidate < best_community)
                    {
                        best_gain = gain;
                        best_community = candidate;
                    }
                }

                if best_community != current {
                    community_of.insert(node.clone(), best_community);
                    changed = true;
                }
            }
        }

        let mut communities: HashMap<String, Vec<String>> = HashMap::new();
        for node in nodes {
            let community = community_of.get(&node).cloned().unwrap_or(node.clone());
            communities.entry(community).or_default().push(node);
        }
        let mut output: Vec<Vec<String>> = communities
            .into_values()
            .map(|mut community| {
                community.sort();
                community
            })
            .collect();
        output.sort_by(|left, right| left[0].cmp(&right[0]));
        output
    }

    pub fn betweenness_centrality(&self) -> HashMap<String, f64> {
        let nodes: Vec<String> = {
            let mut nodes: Vec<String> = self.nodes().into_iter().collect();
            nodes.sort();
            nodes
        };
        let adjacency = self.undirected_neighbors();
        let mut centrality: HashMap<String, f64> =
            nodes.iter().map(|node| (node.clone(), 0.0)).collect();

        for source in &nodes {
            let mut stack = Vec::new();
            let mut predecessors: HashMap<String, Vec<String>> = nodes
                .iter()
                .map(|node| (node.clone(), Vec::new()))
                .collect();
            let mut sigma: HashMap<String, f64> =
                nodes.iter().map(|node| (node.clone(), 0.0)).collect();
            let mut distance: HashMap<String, i64> =
                nodes.iter().map(|node| (node.clone(), -1)).collect();

            sigma.insert(source.clone(), 1.0);
            distance.insert(source.clone(), 0);
            let mut queue = VecDeque::from([source.clone()]);

            while let Some(node) = queue.pop_front() {
                stack.push(node.clone());
                let node_distance = *distance.get(&node).unwrap_or(&-1);
                for neighbor in adjacency
                    .get(&node)
                    .into_iter()
                    .flat_map(|list| list.iter())
                {
                    if *distance.get(neighbor).unwrap_or(&-1) < 0 {
                        queue.push_back(neighbor.clone());
                        distance.insert(neighbor.clone(), node_distance + 1);
                    }
                    if *distance.get(neighbor).unwrap_or(&-1) == node_distance + 1 {
                        let updated_sigma = sigma.get(neighbor).copied().unwrap_or(0.0)
                            + sigma.get(&node).copied().unwrap_or(0.0);
                        sigma.insert(neighbor.clone(), updated_sigma);
                        predecessors
                            .entry(neighbor.clone())
                            .or_default()
                            .push(node.clone());
                    }
                }
            }

            let mut dependency: HashMap<String, f64> =
                nodes.iter().map(|node| (node.clone(), 0.0)).collect();
            while let Some(node) = stack.pop() {
                for predecessor in predecessors
                    .get(&node)
                    .into_iter()
                    .flat_map(|list| list.iter())
                {
                    let sigma_predecessor = sigma.get(predecessor).copied().unwrap_or(0.0);
                    let sigma_node = sigma.get(&node).copied().unwrap_or(1.0);
                    let contribution = if sigma_node > 0.0 {
                        sigma_predecessor / sigma_node
                            * (1.0 + dependency.get(&node).copied().unwrap_or(0.0))
                    } else {
                        0.0
                    };
                    *dependency.entry(predecessor.clone()).or_insert(0.0) += contribution;
                }
                if node != *source {
                    *centrality.entry(node.clone()).or_insert(0.0) +=
                        dependency.get(&node).copied().unwrap_or(0.0);
                }
            }
        }

        for value in centrality.values_mut() {
            *value /= 2.0;
        }
        centrality
    }

    pub fn store_weekly_snapshot(&mut self, week_start: NaiveDate) {
        self.snapshots.push(GraphSnapshot {
            week_start,
            edges: self.typed_edges.clone(),
        });
        self.snapshots
            .sort_by(|left, right| left.week_start.cmp(&right.week_start));
    }

    pub fn weekly_snapshots(&self) -> &[GraphSnapshot] {
        self.snapshots.as_slice()
    }

    pub fn snapshot_delta(
        &self,
        from_week: NaiveDate,
        to_week: NaiveDate,
    ) -> Option<SnapshotDelta> {
        let old_snapshot = self
            .snapshots
            .iter()
            .find(|snapshot| snapshot.week_start == from_week)?;
        let new_snapshot = self
            .snapshots
            .iter()
            .find(|snapshot| snapshot.week_start == to_week)?;
        Some(snapshot_delta_between(old_snapshot, new_snapshot))
    }

    fn undirected_neighbors(&self) -> HashMap<String, Vec<String>> {
        let mut adjacency: HashMap<String, HashSet<String>> = HashMap::new();
        for (node, neighbors) in &self.edges {
            for (neighbor, _) in neighbors {
                adjacency
                    .entry(node.clone())
                    .or_default()
                    .insert(neighbor.clone());
                adjacency
                    .entry(neighbor.clone())
                    .or_default()
                    .insert(node.clone());
            }
        }
        adjacency
            .into_iter()
            .map(|(node, neighbors)| {
                let mut list: Vec<String> = neighbors.into_iter().collect();
                list.sort();
                (node, list)
            })
            .collect()
    }

    fn undirected_weight_map(&self) -> HashMap<String, HashMap<String, f64>> {
        let mut weights: HashMap<String, HashMap<String, f64>> = HashMap::new();
        for (node, neighbors) in &self.edges {
            for (neighbor, weight) in neighbors {
                *weights
                    .entry(node.clone())
                    .or_default()
                    .entry(neighbor.clone())
                    .or_insert(0.0) += *weight;
                *weights
                    .entry(neighbor.clone())
                    .or_default()
                    .entry(node.clone())
                    .or_insert(0.0) += *weight;
            }
        }
        weights
    }

    #[allow(clippy::too_many_arguments)]
    fn modularity_gain(
        &self,
        node: &str,
        candidate_community: &str,
        community_of: &HashMap<String, String>,
        weights: &HashMap<String, HashMap<String, f64>>,
        degrees: &HashMap<String, f64>,
        node_degree: f64,
        total_weight_twice: f64,
    ) -> f64 {
        let k_i_in: f64 = weights
            .get(node)
            .into_iter()
            .flat_map(|neighbors| neighbors.iter())
            .filter(|(neighbor, _)| {
                community_of
                    .get(*neighbor)
                    .map(|community| community == candidate_community)
                    .unwrap_or(false)
            })
            .map(|(_, weight)| *weight)
            .sum();
        let sigma_tot: f64 = community_of
            .iter()
            .filter(|(member, community)| {
                *community == candidate_community && member.as_str() != node
            })
            .map(|(member, _)| degrees.get(member).copied().unwrap_or(0.0))
            .sum();
        k_i_in - sigma_tot * node_degree / total_weight_twice
    }
}

pub fn snapshot_delta_between(
    old_snapshot: &GraphSnapshot,
    new_snapshot: &GraphSnapshot,
) -> SnapshotDelta {
    let old_edges = snapshot_edge_set(old_snapshot);
    let new_edges = snapshot_edge_set(new_snapshot);
    let mut new_edge_list: Vec<(String, String, EdgeType)> =
        new_edges.difference(&old_edges).cloned().collect();
    let mut removed_edge_list: Vec<(String, String, EdgeType)> =
        old_edges.difference(&new_edges).cloned().collect();
    new_edge_list.sort();
    removed_edge_list.sort();

    let old_membership = snapshot_community_membership(old_snapshot);
    let new_membership = snapshot_community_membership(new_snapshot);
    let mut migrations = Vec::new();
    for (node, old_community) in old_membership {
        if let Some(new_community) = new_membership.get(&node) {
            if &old_community != new_community {
                migrations.push(CommunityMigration {
                    node_id: node,
                    from_community: old_community,
                    to_community: new_community.clone(),
                });
            }
        }
    }
    migrations.sort_by(|left, right| left.node_id.cmp(&right.node_id));

    SnapshotDelta {
        new_edges: new_edge_list,
        removed_edges: removed_edge_list,
        community_migrations: migrations,
    }
}

fn snapshot_edge_set(snapshot: &GraphSnapshot) -> HashSet<(String, String, EdgeType)> {
    snapshot
        .edges
        .iter()
        .flat_map(|(from, edges)| {
            edges.iter().map(move |edge| {
                (
                    from.clone(),
                    edge.neighbor_id.clone(),
                    edge.edge_type.clone(),
                )
            })
        })
        .collect()
}

fn snapshot_community_membership(snapshot: &GraphSnapshot) -> HashMap<String, String> {
    let mut graph = AdjacencyGraph::new();
    graph.typed_edges = snapshot.edges.clone();
    for (from, edges) in &snapshot.edges {
        for edge in edges {
            graph
                .edges
                .entry(from.clone())
                .or_default()
                .push((edge.neighbor_id.clone(), edge.weight));
        }
    }
    let communities = graph.louvain_communities();
    let mut membership = HashMap::new();
    for community in communities {
        let community_id = community.first().cloned().unwrap_or_default();
        for node in community {
            membership.insert(node, community_id.clone());
        }
    }
    membership
}

impl Default for AdjacencyGraph {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_close(left: f64, right: f64) {
        assert!(
            (left - right).abs() < 1e-10,
            "expected {left} to be within 1e-10 of {right}"
        );
    }

    fn sample_graph() -> AdjacencyGraph {
        let mut g = AdjacencyGraph::new();
        g.add_bidi_edge("starz", "foxconn", 0.8);
        g.add_bidi_edge("starz", "jabil", 0.6);
        g.add_bidi_edge("foxconn", "apple", 0.9);
        g.add_edge("jabil", "samsung", 0.7);
        g
    }

    #[test]
    fn test_add_and_query_neighbors() {
        let g = sample_graph();
        let neighbors = g.neighbors("starz");
        assert_eq!(neighbors.len(), 2);
    }

    #[test]
    fn test_node_count() {
        let g = sample_graph();
        assert_eq!(g.node_count(), 5);
    }

    #[test]
    fn test_out_degree() {
        let g = sample_graph();
        assert_eq!(g.out_degree("starz"), 2);
        assert_eq!(g.out_degree("foxconn"), 2); // bidi with starz + bidi with apple
        assert_eq!(g.out_degree("samsung"), 0); // only incoming from jabil
    }

    #[test]
    fn test_propagate_risk_basic() {
        let g = sample_graph();
        let mut initial = HashMap::new();
        initial.insert("starz".to_string(), 0.8);

        let result = g.propagate_risk(&initial, 1, 0.5);

        // starz keeps its risk
        assert!(matches!(result.get("starz"), Some(value) if *value >= 0.8));
        // foxconn and jabil should get some risk
        assert!(*result.get("foxconn").unwrap_or(&0.0) > 0.0);
        assert!(*result.get("jabil").unwrap_or(&0.0) > 0.0);
    }

    #[test]
    fn test_propagate_risk_hops_zero_returns_initial_only() {
        let g = sample_graph();
        let mut initial = HashMap::new();
        initial.insert("starz".to_string(), 0.8);

        let result = g.propagate_risk(&initial, 0, 0.5);

        assert_eq!(result.len(), 1);
        assert_close(result.get("starz").copied().unwrap_or_default(), 0.8);
        assert!(!result.contains_key("foxconn"));
    }

    #[test]
    fn test_propagate_risk_negative_decay_treated_as_zero() {
        let g = sample_graph();
        let mut initial = HashMap::new();
        initial.insert("starz".to_string(), 0.8);

        let result = g.propagate_risk(&initial, 2, -0.5);

        assert_close(result.get("starz").copied().unwrap_or_default(), 0.8);
        assert_close(result.get("foxconn").copied().unwrap_or_default(), 0.0);
        assert_close(result.get("jabil").copied().unwrap_or_default(), 0.0);
    }

    #[test]
    fn test_propagate_risk_multi_hop() {
        let g = sample_graph();
        let mut initial = HashMap::new();
        initial.insert("starz".to_string(), 0.9);

        let one_hop = g.propagate_risk(&initial, 1, 0.5);
        let two_hop = g.propagate_risk(&initial, 2, 0.5);

        // After 2 hops, apple should get some risk (starz -> foxconn -> apple)
        let apple_1 = one_hop.get("apple").copied().unwrap_or(0.0);
        let apple_2 = two_hop.get("apple").copied().unwrap_or(0.0);
        assert!(apple_2 > apple_1);
    }

    #[test]
    fn test_propagate_risk_capped_at_1() {
        let mut g = AdjacencyGraph::new();
        // Create a dense graph with high weights
        g.add_edge("a", "b", 1.0);
        g.add_edge("c", "b", 1.0);
        g.add_edge("d", "b", 1.0);

        let mut initial = HashMap::new();
        initial.insert("a".to_string(), 1.0);
        initial.insert("c".to_string(), 1.0);
        initial.insert("d".to_string(), 1.0);

        let result = g.propagate_risk(&initial, 3, 0.9);
        assert!(matches!(result.get("b"), Some(value) if *value <= 1.0));
    }

    #[test]
    fn test_propagate_risk_accumulates_overlapping_sources() {
        let mut g = AdjacencyGraph::new();
        g.add_edge("a", "c", 1.0);
        g.add_edge("b", "c", 1.0);

        let mut initial = HashMap::new();
        initial.insert("a".to_string(), 0.6);
        initial.insert("b".to_string(), 0.5);

        let result = g.propagate_risk(&initial, 1, 1.0);

        // Risk from two sources (0.6 + 0.5 = 1.1) is soft-saturated toward 1.0
        let c_risk = result.get("c").copied().unwrap_or_default();
        assert!(
            c_risk > 0.9 && c_risk <= 1.0,
            "expected saturated risk near 1.0, got {c_risk}"
        );
    }

    #[test]
    fn test_pagerank_converges() {
        let g = sample_graph();
        let scores = g.pagerank(20, 0.85);

        // All scores should be positive
        for score in scores.values() {
            assert!(*score > 0.0);
        }

        // Sum should be approximately 1.0
        let sum: f64 = scores.values().sum();
        assert!((sum - 1.0).abs() < 0.01, "Sum was {}", sum);
    }

    #[test]
    fn pagerank_convergence_assertion() {
        let g = sample_graph();
        let convergence_delta = g
            .pagerank_convergence_l1_delta(30, 0.85)
            .unwrap_or_else(|| panic!("sample graph should produce a convergence delta"));
        assert!(
            convergence_delta < 1e-6,
            "PageRank should converge by 30 iterations; L1 delta was {}",
            convergence_delta
        );
    }

    #[test]
    fn test_pagerank_hub_has_higher_rank() {
        let mut g = AdjacencyGraph::new();
        // Hub topology: center connected to many nodes
        for i in 0..5 {
            g.add_bidi_edge("center", &format!("node_{}", i), 1.0);
        }
        let scores = g.pagerank(30, 0.85);
        let center_score = scores["center"];
        for i in 0..5 {
            assert!(center_score > scores[&format!("node_{}", i)]);
        }
    }

    #[test]
    fn test_shortest_path_direct() {
        let g = sample_graph();
        let path = g
            .shortest_path("starz", "foxconn")
            .unwrap_or_else(|| panic!("direct path should exist"));
        assert_eq!(path, vec!["starz", "foxconn"]);
    }

    #[test]
    fn test_shortest_path_two_hops() {
        let g = sample_graph();
        let path = g
            .shortest_path("starz", "apple")
            .unwrap_or_else(|| panic!("two-hop path should exist"));
        assert_eq!(path, vec!["starz", "foxconn", "apple"]);
    }

    #[test]
    fn test_shortest_path_unreachable() {
        let g = sample_graph();
        // samsung has no outgoing edges to reach starz
        let path = g.shortest_path("samsung", "apple");
        assert!(path.is_none());
    }

    #[test]
    fn test_connected_components() {
        let mut g = AdjacencyGraph::new();
        g.add_bidi_edge("a", "b", 1.0);
        g.add_bidi_edge("c", "d", 1.0);
        // Two disconnected components
        let components = g.connected_components();
        assert_eq!(components.len(), 2);
    }

    #[test]
    fn test_co_appearance_count() {
        let mut g = AdjacencyGraph::new();
        g.add_edge("alice", "event1", 1.0);
        g.add_edge("alice", "event2", 1.0);
        g.add_edge("bob", "event1", 1.0);
        g.add_edge("bob", "event3", 1.0);

        assert_eq!(g.co_appearance_count("alice", "bob"), 1); // event1
    }

    #[test]
    fn test_empty_graph() {
        let g = AdjacencyGraph::new();
        assert_eq!(g.node_count(), 0);
        assert_eq!(g.edge_count(), 0);
        assert!(g.pagerank(10, 0.85).is_empty());
    }

    // ── B152: negative weight validation ──

    #[test]
    fn test_add_edge_negative_weight_clamped() {
        let mut g = AdjacencyGraph::new();
        g.add_edge("a", "b", -0.5);
        let neighbors = g.neighbors("a");
        assert_eq!(neighbors.len(), 1);
        assert!(
            (neighbors[0].1 - 0.0).abs() < f64::EPSILON,
            "Negative weight should be clamped to 0"
        );
    }

    // ── B153: self-loop tests ──

    #[test]
    fn test_self_loop_edge() {
        let mut g = AdjacencyGraph::new();
        g.add_edge("a", "a", 0.5);
        let neighbors = g.neighbors("a");
        assert_eq!(neighbors.len(), 1);
        assert_eq!(neighbors[0].0, "a");
        assert_eq!(g.node_count(), 1);
    }

    #[test]
    fn test_self_loop_propagate_risk() {
        let mut g = AdjacencyGraph::new();
        g.add_edge("a", "a", 1.0);
        let mut initial = HashMap::new();
        initial.insert("a".to_string(), 0.5);
        let result = g.propagate_risk(&initial, 3, 0.9);
        // Risk should never exceed 1.0
        assert!(matches!(result.get("a"), Some(value) if *value <= 1.0));
    }

    #[test]
    fn test_self_loop_shortest_path() {
        let mut g = AdjacencyGraph::new();
        g.add_edge("a", "a", 1.0);
        let path = g
            .shortest_path("a", "a")
            .unwrap_or_else(|| panic!("self-loop path should exist"));
        assert_eq!(path, vec!["a"]);
    }

    // ── B154: disconnected nodes in pagerank ──

    #[test]
    fn test_pagerank_disconnected_nodes() {
        let mut g = AdjacencyGraph::new();
        g.add_edge("a", "b", 1.0);
        g.add_edge("b", "a", 1.0);
        // c is completely disconnected — add as an isolated source with no real target
        g.add_edge("c", "c", 0.0);
        let scores = g.pagerank(30, 0.85);
        // All nodes including the disconnected one should have positive score
        assert!(scores.len() >= 3);
        for score in scores.values() {
            assert!(*score > 0.0, "All nodes should have positive pagerank");
        }
    }

    #[test]
    fn test_pagerank_isolated_nodes_only_uniform_scores() {
        let mut g = AdjacencyGraph::new();
        g.add_edge("a", "a", 0.0);
        g.add_edge("b", "b", 0.0);
        g.add_edge("c", "c", 0.0);

        let scores = g.pagerank(30, 0.85);
        assert_eq!(scores.len(), 3);

        let a = scores["a"];
        let b = scores["b"];
        let c = scores["c"];
        assert!((a - b).abs() < 1e-9);
        assert!((b - c).abs() < 1e-9);
    }

    // ── B155: division by zero guard when total_weight is 0 ──

    #[test]
    fn test_pagerank_zero_weight_edges() {
        let mut g = AdjacencyGraph::new();
        g.add_edge("a", "b", 0.0);
        g.add_edge("b", "a", 0.0);
        // Should not panic
        let scores = g.pagerank(10, 0.85);
        assert!(!scores.is_empty());
    }

    #[test]
    fn test_pagerank_nan_weight_treated_as_zero() {
        let mut g = AdjacencyGraph::new();
        g.add_edge("a", "b", f64::NAN);
        g.add_edge("b", "a", 1.0);

        let scores = g.pagerank(20, 0.85);
        assert_eq!(scores.len(), 2);
        assert!(scores.values().all(|v| v.is_finite()));
    }

    #[test]
    fn test_propagate_risk_decay_zero_no_spread() {
        let g = sample_graph();
        let mut initial = HashMap::new();
        initial.insert("starz".to_string(), 0.8);

        let result = g.propagate_risk(&initial, 3, 0.0);
        assert_close(result.get("starz").copied().unwrap_or_default(), 0.8);
        assert_close(result.get("foxconn").copied().unwrap_or_default(), 0.0);
        assert_close(result.get("jabil").copied().unwrap_or_default(), 0.0);
    }

    // ── B156: shortest_path when target unreachable ──

    #[test]
    fn test_shortest_path_unreachable_directed() {
        let mut g = AdjacencyGraph::new();
        g.add_edge("a", "b", 1.0);
        // b -> a doesn't exist, so "b" cannot reach "a"
        assert!(g.shortest_path("b", "a").is_none());
    }

    #[test]
    fn test_shortest_path_nonexistent_nodes() {
        let g = AdjacencyGraph::new();
        assert!(g.shortest_path("x", "y").is_none());
    }

    #[test]
    fn test_shortest_path_with_cycle() {
        let mut g = AdjacencyGraph::new();
        g.add_edge("a", "b", 1.0);
        g.add_edge("b", "c", 1.0);
        g.add_edge("c", "a", 1.0);
        g.add_edge("c", "d", 1.0);

        let path = g
            .shortest_path("a", "d")
            .unwrap_or_else(|| panic!("cycle graph should have a path to d"));
        assert_eq!(path, vec!["a", "b", "c", "d"]);
    }

    #[test]
    fn test_shortest_path_same_node() {
        let g = sample_graph();
        let path = g
            .shortest_path("starz", "starz")
            .unwrap_or_else(|| panic!("same-node path should exist"));
        assert_eq!(path, vec!["starz"]);
    }

    // ── B157: nodes() caching ──
    // (The fix caches via a HashSet built on each call; verifying consistency.)

    #[test]
    fn test_nodes_consistent_after_mutation() {
        let mut g = AdjacencyGraph::new();
        g.add_edge("a", "b", 1.0);
        let nodes1 = g.nodes();
        g.add_edge("c", "d", 1.0);
        let nodes2 = g.nodes();
        assert_eq!(nodes1.len(), 2);
        assert_eq!(nodes2.len(), 4);
    }

    // ── B158: connected_components in directed-only graphs ──

    #[test]
    fn test_connected_components_directed_only() {
        let mut g = AdjacencyGraph::new();
        g.add_edge("a", "b", 1.0);
        g.add_edge("c", "d", 1.0);
        // directed only — connected_components uses undirected view
        let components = g.connected_components();
        assert_eq!(components.len(), 2);
    }

    #[test]
    fn test_connected_components_directed_chain() {
        let mut g = AdjacencyGraph::new();
        g.add_edge("a", "b", 1.0);
        g.add_edge("b", "c", 1.0);
        // One component via undirected view
        let components = g.connected_components();
        assert_eq!(components.len(), 1);
    }

    // ── B166: empty adjacency safe handling ──

    #[test]
    fn test_empty_graph_propagate_risk() {
        let g = AdjacencyGraph::new();
        let result = g.propagate_risk(&HashMap::new(), 3, 0.5);
        assert!(result.is_empty());
    }

    #[test]
    fn test_empty_graph_shortest_path() {
        let g = AdjacencyGraph::new();
        assert!(g.shortest_path("a", "b").is_none());
    }

    #[test]
    fn test_empty_graph_connected_components() {
        let g = AdjacencyGraph::new();
        let components = g.connected_components();
        assert!(components.is_empty());
    }

    #[test]
    fn test_connected_components_single_isolated_node() {
        let mut g = AdjacencyGraph::new();
        g.add_edge("solo", "solo", 0.0);

        let mut components = g.connected_components();
        assert_eq!(components.len(), 1);
        components[0].sort();
        assert_eq!(components[0], vec!["solo".to_string()]);
    }

    #[test]
    fn test_empty_graph_co_appearance() {
        let g = AdjacencyGraph::new();
        assert_eq!(g.co_appearance_count("a", "b"), 0);
    }

    // ── B167: co_appearance_count with no shared neighbors ──

    #[test]
    fn test_co_appearance_no_shared_neighbors() {
        let mut g = AdjacencyGraph::new();
        g.add_edge("alice", "event1", 1.0);
        g.add_edge("bob", "event2", 1.0);
        assert_eq!(g.co_appearance_count("alice", "bob"), 0);
    }

    #[test]
    fn test_co_appearance_nonexistent_node() {
        let mut g = AdjacencyGraph::new();
        g.add_edge("alice", "event1", 1.0);
        assert_eq!(g.co_appearance_count("alice", "nobody"), 0);
    }

    #[test]
    fn test_typed_neighbors_and_traversal() {
        let mut g = AdjacencyGraph::new();
        g.add_typed_edge("supplier_a", "oem", 0.9, EdgeType::SupplierOf, None);
        g.add_typed_edge("oem", "board_member", 0.8, EdgeType::BoardMemberOf, None);
        g.add_typed_edge("supplier_a", "peer", 0.7, EdgeType::CompetesWith, None);

        let supplier_edges = g.typed_neighbors("supplier_a", Some(&EdgeType::SupplierOf));
        assert_eq!(supplier_edges.len(), 1);
        assert_eq!(supplier_edges[0].neighbor_id, "oem");

        let allowed = HashSet::from([EdgeType::SupplierOf]);
        let reachable = g.typed_reachable_nodes("supplier_a", 2, &allowed);
        assert!(reachable.contains("oem"));
        assert!(!reachable.contains("peer"));
        assert!(!reachable.contains("board_member"));
    }

    #[test]
    fn test_louvain_communities_find_dense_clusters() {
        let mut g = AdjacencyGraph::new();
        for (a, b) in [
            ("a1", "a2"),
            ("a2", "a3"),
            ("a1", "a3"),
            ("b1", "b2"),
            ("b2", "b3"),
            ("b1", "b3"),
        ] {
            g.add_bidi_edge(a, b, 1.0);
        }
        g.add_bidi_edge("a3", "b1", 0.05);

        let communities = g.louvain_communities();
        assert_eq!(
            communities.len(),
            2,
            "expected two dense clusters: {communities:?}"
        );
        assert!(communities
            .iter()
            .any(|community| community
                == &vec!["a1".to_string(), "a2".to_string(), "a3".to_string()]));
        assert!(communities
            .iter()
            .any(|community| community
                == &vec!["b1".to_string(), "b2".to_string(), "b3".to_string()]));
    }

    #[test]
    fn test_betweenness_centrality_identifies_bridge_entity() {
        let mut g = AdjacencyGraph::new();
        g.add_bidi_edge("left", "bridge", 1.0);
        g.add_bidi_edge("bridge", "right", 1.0);
        g.add_bidi_edge("left", "left_leaf", 1.0);
        g.add_bidi_edge("right", "right_leaf", 1.0);

        let centrality = g.betweenness_centrality();
        let bridge = centrality.get("bridge").copied().unwrap_or(0.0);
        assert!(bridge > centrality.get("left").copied().unwrap_or(0.0));
        assert!(bridge > centrality.get("right").copied().unwrap_or(0.0));
    }

    #[test]
    fn test_snapshot_delta_tracks_new_removed_edges_and_migrations() {
        let week1 = NaiveDate::from_ymd_opt(2026, 3, 2)
            .unwrap_or_else(|| panic!("fixed test date should be valid"));
        let week2 = week1 + chrono::Days::new(7);

        let mut graph_week1 = AdjacencyGraph::new();
        graph_week1.add_typed_bidi_edge("a1", "a2", 1.0, EdgeType::CompetesWith, None);
        graph_week1.add_typed_bidi_edge("a2", "x", 1.0, EdgeType::CompetesWith, None);
        graph_week1.add_typed_bidi_edge("b1", "b2", 1.0, EdgeType::CompetesWith, None);
        graph_week1.store_weekly_snapshot(week1);

        let mut graph_week2 = AdjacencyGraph::new();
        graph_week2.add_typed_bidi_edge("a1", "a2", 1.0, EdgeType::CompetesWith, None);
        graph_week2.add_typed_bidi_edge("b1", "b2", 1.0, EdgeType::CompetesWith, None);
        graph_week2.add_typed_bidi_edge("b2", "x", 1.0, EdgeType::CompetesWith, None);
        graph_week2.add_typed_edge("supplier", "oem", 0.9, EdgeType::SupplierOf, None);

        let delta = snapshot_delta_between(
            &graph_week1.weekly_snapshots()[0],
            &GraphSnapshot {
                week_start: week2,
                edges: graph_week2.typed_edges.clone(),
            },
        );

        assert!(delta
            .new_edges
            .iter()
            .any(|(from, to, kind)| from == "supplier"
                && to == "oem"
                && *kind == EdgeType::SupplierOf));
        assert!(delta
            .removed_edges
            .iter()
            .any(|(from, to, kind)| from == "a2" && to == "x" && *kind == EdgeType::CompetesWith));
        assert!(delta
            .community_migrations
            .iter()
            .any(|migration| migration.node_id == "x"));
    }

    #[test]
    fn test_pagerank_empty_graph_returns_empty() {
        let g = AdjacencyGraph::new();
        let scores = g.pagerank(20, 0.85);
        assert!(scores.is_empty());
    }

    #[test]
    fn test_pagerank_single_node_no_edges() {
        let mut g = AdjacencyGraph::new();
        g.add_edge("solo", "solo", 0.0); // self-loop
        let scores = g.pagerank(20, 0.85);
        assert!(!scores.is_empty());
    }
}
