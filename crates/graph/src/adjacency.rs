use std::collections::{HashMap, HashSet, VecDeque};

/// An adjacency list representation for the entity graph.
#[derive(Debug, Clone)]
pub struct AdjacencyGraph {
    /// node_id -> Vec<(neighbor_id, weight)>
    edges: HashMap<String, Vec<(String, f64)>>,
}

impl AdjacencyGraph {
    pub fn new() -> Self {
        Self {
            edges: HashMap::new(),
        }
    }

    /// Add a directed edge from `from` to `to` with a given weight.
    pub fn add_edge(&mut self, from: &str, to: &str, weight: f64) {
        self.edges
            .entry(from.to_string())
            .or_default()
            .push((to.to_string(), weight));
    }

    /// Add a bidirectional edge.
    pub fn add_bidi_edge(&mut self, a: &str, b: &str, weight: f64) {
        self.add_edge(a, b, weight);
        self.add_edge(b, a, weight);
    }

    /// Get neighbors and weights for a node.
    pub fn neighbors(&self, node: &str) -> &[(String, f64)] {
        self.edges.get(node).map(|v| v.as_slice()).unwrap_or(&[])
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
    /// BFS-style propagation for `hops` rounds with decay factor.
    /// This follows the IMPLEMENTATION.md graph_risk::propagate() algorithm.
    pub fn propagate_risk(
        &self,
        initial_risk: &HashMap<String, f64>,
        hops: u8,
        decay: f64,
    ) -> HashMap<String, f64> {
        let mut risk = initial_risk.clone();

        for _ in 0..hops {
            let mut new_risk = risk.clone();
            for (node, neighbors) in &self.edges {
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

    /// Compute PageRank-style centrality scores.
    ///
    /// Uses power iteration with damping factor `d` (typically 0.85).
    pub fn pagerank(&self, iterations: usize, damping: f64) -> HashMap<String, f64> {
        let nodes: Vec<String> = self.nodes().into_iter().collect();
        let n = nodes.len();
        if n == 0 {
            return HashMap::new();
        }

        let init_score = 1.0 / n as f64;
        let mut scores: HashMap<String, f64> = nodes.iter().map(|n| (n.clone(), init_score)).collect();

        for _ in 0..iterations {
            let base = (1.0 - damping) / nodes.len() as f64;
            let mut new_scores: HashMap<String, f64> =
                nodes.iter().map(|n| (n.clone(), base)).collect();

            for node in &nodes {
                let out_deg = self.out_degree(node);
                if out_deg == 0 {
                    // Distribute score evenly (dangling node)
                    let share = damping * scores[node] / nodes.len() as f64;
                    for target in &nodes {
                        *new_scores.get_mut(target).unwrap() += share;
                    }
                } else {
                    let share = damping * scores[node] / out_deg as f64;
                    for (neighbor, _weight) in self.neighbors(node) {
                        if let Some(s) = new_scores.get_mut(neighbor) {
                            *s += share;
                        }
                    }
                }
            }

            scores = new_scores;
        }

        scores
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

    /// Find connected components via BFS.
    pub fn connected_components(&self) -> Vec<Vec<String>> {
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
                for (neighbor, _) in self.neighbors(&n) {
                    if !visited.contains(neighbor) {
                        visited.insert(neighbor.clone());
                        queue.push_back(neighbor.clone());
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
        neighbors_a.intersection(&neighbors_b).count()
    }
}

impl Default for AdjacencyGraph {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(*result.get("starz").unwrap() >= 0.8);
        // foxconn and jabil should get some risk
        assert!(*result.get("foxconn").unwrap_or(&0.0) > 0.0);
        assert!(*result.get("jabil").unwrap_or(&0.0) > 0.0);
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
        assert!(*result.get("b").unwrap() <= 1.0);
    }

    #[test]
    fn test_pagerank_converges() {
        let g = sample_graph();
        let scores = g.pagerank(20, 0.85);

        // All scores should be positive
        for (_, score) in &scores {
            assert!(*score > 0.0);
        }

        // Sum should be approximately 1.0
        let sum: f64 = scores.values().sum();
        assert!((sum - 1.0).abs() < 0.01, "Sum was {}", sum);
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
        let path = g.shortest_path("starz", "foxconn").unwrap();
        assert_eq!(path, vec!["starz", "foxconn"]);
    }

    #[test]
    fn test_shortest_path_two_hops() {
        let g = sample_graph();
        let path = g.shortest_path("starz", "apple").unwrap();
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
}
