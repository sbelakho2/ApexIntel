//! Multi-hop evidence chain tracker.
//!
//! Models the epistemic chain from raw observations to high-level conclusions.
//! Each `EvidenceNode` holds a claim; `EvidenceEdge` records how one claim
//! supports or contradicts another, along with a confidence weight.
//!
//! # Use-cases
//! - Audit trail: "why did the pipeline produce this insight?"
//! - Confidence propagation: propagate source-document confidence up to
//!   derived conclusions.
//! - Contradiction detection: flag chains that contain conflicting evidence.
//!
//! # Example
//! ```rust
//! use apex_insights::evidence_chain::{EvidenceChain, EvidenceNode, EvidenceEdge, EdgeKind};
//! let mut chain = EvidenceChain::new();
//! let a = chain.add_node(EvidenceNode::observation("Missile shipment spotted at port", 0.85));
//! let b = chain.add_node(EvidenceNode::inference("Escalation probable", 0.0));
//! chain.add_edge(EvidenceEdge { from: a, to: b, kind: EdgeKind::Supports, weight: 0.9 });
//! chain.propagate_confidence();
//! println!("{}", chain.node(b).map(|n| n.confidence).unwrap_or(0.0));  // 0.85 * 0.9 ≈ 0.765
//! ```

use anyhow::{bail, Context, Result};
use apex_llm::LlmClient;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use tracing::warn;

// ─────────────────────────────────────────────────────────────────────────────
// Node
// ─────────────────────────────────────────────────────────────────────────────

/// The kind of a node in the chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeKind {
    /// Raw document / crawl extract; highest epistemic primacy.
    Observation,
    /// Claim derived from one or more observations or other inferences.
    Inference,
    /// A conclusion (leaf) produced for an insight card or recommendation.
    Conclusion,
    /// A hypothetical or predictive claim (lower confidence weight).
    Hypothesis,
}

/// One claim in the evidence graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceNode {
    pub id: usize,
    pub kind: NodeKind,
    /// Natural-language claim text.
    pub claim: String,
    /// Source URL or document identifier (if applicable).
    pub source: Option<String>,
    /// Confidence in this claim [0, 1]. Propagated from parents.
    pub confidence: f64,
    /// Optional citation string.
    pub citation: Option<String>,
    /// Optional entity this claim pertains to.
    pub entity_id: Option<String>,
    /// Observation time used for freshness-aware propagation.
    pub observed_at: Option<DateTime<Utc>>,
}

impl EvidenceNode {
    pub fn observation(claim: impl Into<String>, confidence: f64) -> Self {
        Self {
            id: 0, // set by EvidenceChain::add_node
            kind: NodeKind::Observation,
            claim: claim.into(),
            source: None,
            confidence: confidence.clamp(0.0, 1.0),
            citation: None,
            entity_id: None,
            observed_at: None,
        }
    }

    pub fn inference(claim: impl Into<String>, initial_confidence: f64) -> Self {
        Self {
            id: 0,
            kind: NodeKind::Inference,
            claim: claim.into(),
            source: None,
            confidence: initial_confidence.clamp(0.0, 1.0),
            citation: None,
            entity_id: None,
            observed_at: None,
        }
    }

    pub fn conclusion(claim: impl Into<String>) -> Self {
        Self {
            id: 0,
            kind: NodeKind::Conclusion,
            claim: claim.into(),
            source: None,
            confidence: 0.0,
            citation: None,
            entity_id: None,
            observed_at: None,
        }
    }

    pub fn hypothesis(claim: impl Into<String>, confidence: f64) -> Self {
        Self {
            id: 0,
            kind: NodeKind::Hypothesis,
            claim: claim.into(),
            source: None,
            confidence: confidence.clamp(0.0, 1.0),
            citation: None,
            entity_id: None,
            observed_at: None,
        }
    }

    /// Builder-style source setter.
    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    pub fn with_observed_at(mut self, observed_at: DateTime<Utc>) -> Self {
        self.observed_at = Some(observed_at);
        self
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Edge
// ─────────────────────────────────────────────────────────────────────────────

/// How one node relates to another.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EdgeKind {
    /// Node A supports / provides evidence for node B.
    Supports,
    /// Node A partially supports node B (weaker).
    WeaklySupports,
    /// Node A contradicts / undermines node B.
    Contradicts,
    /// Node A is a source document for node B.
    Cites,
    /// Node A is a refinement / sub-claim of node B.
    Refines,
}

/// Directed edge in the evidence graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceEdge {
    pub from: usize,
    pub to: usize,
    pub kind: EdgeKind,
    /// Transmission weight [0, 1]: how strongly the source confidence propagates.
    pub weight: f64,
}

// ─────────────────────────────────────────────────────────────────────────────
// Chain
// ─────────────────────────────────────────────────────────────────────────────

/// The full evidence graph for one analytic output.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EvidenceChain {
    nodes: Vec<EvidenceNode>,
    edges: Vec<EvidenceEdge>,
}

impl EvidenceChain {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a node, returning its assigned id.
    pub fn add_node(&mut self, mut node: EvidenceNode) -> usize {
        let id = self.nodes.len();
        node.id = id;
        self.nodes.push(node);
        id
    }

    /// Add a directed edge.
    pub fn add_edge(&mut self, edge: EvidenceEdge) {
        self.edges.push(edge);
    }

    /// Immutable access to a node by id.
    pub fn node(&self, id: usize) -> Option<&EvidenceNode> {
        self.nodes.get(id)
    }

    /// Mutable access to a node by id.
    pub fn node_mut(&mut self, id: usize) -> Option<&mut EvidenceNode> {
        self.nodes.get_mut(id)
    }

    pub fn nodes(&self) -> &[EvidenceNode] {
        &self.nodes
    }

    pub fn edges(&self) -> &[EvidenceEdge] {
        &self.edges
    }

    // ── Confidence propagation ──────────────────────────────────────────────

    /// BFS-propagate confidence from sources (Observation/Hypothesis) to
    /// Inference/Conclusion nodes through `Supports`, `WeaklySupports`, and
    /// `Cites` edges. `Contradicts` edges reduce the target confidence.
    pub fn propagate_confidence(&mut self) {
        self.propagate_confidence_at(Utc::now());
    }

    pub fn propagate_confidence_at(&mut self, now: DateTime<Utc>) {
        // Build adjacency list (from → [(to, edge_index)])
        let mut adj: HashMap<usize, Vec<usize>> = HashMap::new();
        for (i, edge) in self.edges.iter().enumerate() {
            adj.entry(edge.from).or_default().push(i);
        }

        // Topological-ish BFS from all leaf-less sources
        let mut in_degree: HashMap<usize, usize> = HashMap::new();
        for edge in &self.edges {
            *in_degree.entry(edge.to).or_insert(0) += 1;
        }

        let mut queue: VecDeque<usize> = (0..self.nodes.len())
            .filter(|id| *in_degree.get(id).unwrap_or(&0) == 0)
            .collect();

        // Track accumulated confidence for each node
        let mut contributions: HashMap<usize, Vec<f64>> = HashMap::new();

        while let Some(src) = queue.pop_front() {
            let src_conf = if contributions.contains_key(&src) {
                let vals = &contributions[&src];
                let support_vals = vals.iter().copied().filter(|c| *c > 0.0);
                let contradiction_vals = vals.iter().copied().filter(|c| *c < 0.0).map(f64::abs);

                let support_mass =
                    1.0 - support_vals.fold(1.0_f64, |acc, c| acc * (1.0 - c.clamp(0.0, 1.0)));
                let contradiction_mass = 1.0
                    - contradiction_vals.fold(1.0_f64, |acc, c| acc * (1.0 - c.clamp(0.0, 1.0)));

                let prior = self.nodes[src].confidence.clamp(0.0, 1.0);
                let combined_support = 1.0 - (1.0 - prior) * (1.0 - support_mass);
                (combined_support * (1.0 - contradiction_mass)).clamp(0.0, 1.0)
            } else {
                self.nodes[src].confidence
            };

            // Update this node's confidence
            self.nodes[src].confidence = src_conf;

            if let Some(edge_indices) = adj.get(&src).cloned() {
                for ei in edge_indices {
                    let edge = &self.edges[ei];
                    let freshness = node_freshness_weight(&self.nodes[src], now);
                    let delta = match edge.kind {
                        EdgeKind::Supports | EdgeKind::Cites => src_conf * edge.weight * freshness,
                        EdgeKind::WeaklySupports | EdgeKind::Refines => {
                            src_conf * edge.weight * 0.5 * freshness
                        }
                        EdgeKind::Contradicts => -(src_conf * edge.weight * freshness),
                    };
                    contributions.entry(edge.to).or_default().push(delta);

                    // Decrement in-degree; enqueue when all inputs processed
                    let deg = in_degree.entry(edge.to).or_insert(1);
                    *deg = deg.saturating_sub(1);
                    if *deg == 0 {
                        queue.push_back(edge.to);
                    }
                }
            }
        }
    }

    // ── Cycle detection ────────────────────────────────────────────────────

    /// Returns `true` if the graph contains a cycle (problematic for propagation).
    pub fn has_cycle(&self) -> bool {
        let mut visited = HashSet::new();
        let mut rec_stack = HashSet::new();

        for start in 0..self.nodes.len() {
            if self.dfs_cycle(start, &mut visited, &mut rec_stack) {
                return true;
            }
        }
        false
    }

    fn dfs_cycle(
        &self,
        node: usize,
        visited: &mut HashSet<usize>,
        rec: &mut HashSet<usize>,
    ) -> bool {
        if rec.contains(&node) {
            return true;
        }
        if visited.contains(&node) {
            return false;
        }
        visited.insert(node);
        rec.insert(node);

        for edge in &self.edges {
            if edge.from == node && self.dfs_cycle(edge.to, visited, rec) {
                return true;
            }
        }

        rec.remove(&node);
        false
    }

    // ── Contradiction detection ─────────────────────────────────────────────

    /// Returns all pairs (node_a, node_b) where node_a contradicts node_b
    /// and both have confidence above `threshold`.
    pub fn contradictions(&self, threshold: f64) -> Vec<(usize, usize)> {
        self.edges
            .iter()
            .filter(|e| {
                e.kind == EdgeKind::Contradicts
                    && self
                        .nodes
                        .get(e.from)
                        .map(|n| n.confidence >= threshold)
                        .unwrap_or(false)
                    && self.node_has_supported_claim_confidence(e.to, threshold)
            })
            .map(|e| (e.from, e.to))
            .collect()
    }

    fn node_has_supported_claim_confidence(&self, node_idx: usize, threshold: f64) -> bool {
        if self
            .nodes
            .get(node_idx)
            .map(|n| n.confidence >= threshold)
            .unwrap_or(false)
        {
            return true;
        }

        self.edges
            .iter()
            .filter(|edge| edge.to == node_idx && edge.kind == EdgeKind::Supports)
            .any(|edge| {
                self.nodes
                    .get(edge.from)
                    .map(|source| source.confidence * edge.weight >= threshold)
                    .unwrap_or(false)
            })
    }

    // ── Chain strength ───────────────────────────────────────────────────────

    /// Compute the average confidence of all leaf (conclusion) nodes.
    pub fn chain_strength(&self) -> f64 {
        let conclusions: Vec<f64> = self
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Conclusion)
            .map(|n| n.confidence)
            .collect();

        if conclusions.is_empty() {
            // Fall back to average over all nodes
            if self.nodes.is_empty() {
                return 0.0;
            }
            return self.nodes.iter().map(|n| n.confidence).sum::<f64>() / self.nodes.len() as f64;
        }

        conclusions.iter().sum::<f64>() / conclusions.len() as f64
    }

    // ── Source coverage ──────────────────────────────────────────────────────

    /// Returns the number of distinct source documents referenced.
    pub fn source_count(&self) -> usize {
        self.nodes
            .iter()
            .filter_map(|n| n.source.as_deref())
            .collect::<HashSet<_>>()
            .len()
    }

    // ── Summary ─────────────────────────────────────────────────────────────

    pub fn summary(&self) -> ChainSummary {
        ChainSummary {
            node_count: self.nodes.len(),
            edge_count: self.edges.len(),
            chain_strength: self.chain_strength(),
            source_count: self.source_count(),
            has_contradictions: !self.contradictions(0.5).is_empty(),
            has_cycle: self.has_cycle(),
        }
    }
}

fn node_freshness_weight(node: &EvidenceNode, now: DateTime<Utc>) -> f64 {
    let Some(observed_at) = node.observed_at else {
        return 1.0;
    };
    let age_days = (now - observed_at).num_hours().max(0) as f64 / 24.0;
    if age_days <= 7.0 {
        1.0
    } else {
        0.25_f64.powf((age_days - 7.0) / 83.0).clamp(0.0, 1.0)
    }
}

/// Lightweight summary of an evidence chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainSummary {
    pub node_count: usize,
    pub edge_count: usize,
    pub chain_strength: f64,
    pub source_count: usize,
    pub has_contradictions: bool,
    pub has_cycle: bool,
}

// ─────────────────────────────────────────────────────────────────────────────
// LLM chain builder
// ─────────────────────────────────────────────────────────────────────────────

/// Uses an LLM to convert a set of raw text snippets into a structured
/// evidence chain with inferred relationships.
pub struct LlmChainBuilder {
    llm: std::sync::Arc<dyn LlmClient>,
}

impl LlmChainBuilder {
    pub fn new(llm: std::sync::Arc<dyn LlmClient>) -> Self {
        Self { llm }
    }

    /// Build an evidence chain for `conclusion_claim` from `source_snippets`.
    ///
    /// Each snippet in `source_snippets` becomes an `Observation` node.
    /// The LLM is asked to produce intermediate inference steps and rate
    /// how well each snippet supports the conclusion.
    pub async fn build(
        &self,
        conclusion_claim: &str,
        source_snippets: &[(String, String, f64)], // (id, text, raw_confidence)
    ) -> Result<EvidenceChain> {
        if source_snippets.is_empty() {
            bail!("Cannot build evidence chain without source snippets");
        }

        let mut chain = EvidenceChain::new();

        // Add observation nodes
        let obs_ids: Vec<usize> = source_snippets
            .iter()
            .map(|(src_id, text, conf)| {
                let node =
                    EvidenceNode::observation(text.clone(), *conf).with_source(src_id.clone());
                chain.add_node(node)
            })
            .collect();

        // Ask LLM to rate each snippet's relevance to the conclusion
        let snippets_text = source_snippets
            .iter()
            .enumerate()
            .map(|(i, (sid, text, _))| format!("[{}] ({}): {}", i, sid, text))
            .collect::<Vec<_>>()
            .join("\n");

        let system = "You are a rigorous intelligence analyst. \
            Rate how strongly each numbered evidence snippet supports a given conclusion, \
            from 0.0 (irrelevant) to 1.0 (strongly supports). \
            Return a JSON array of objects with fields: index (int), weight (float), brief_reason (str).";

        let user = format!(
            "Conclusion: {}\n\nEvidence snippets:\n{}",
            conclusion_claim, snippets_text
        );

        let json_response = self
            .llm
            .generate_json(system, &user)
            .await
            .context("LLM chain builder call failed")?;

        // Parse weights from LLM response
        let weights: Vec<serde_json::Value> =
            serde_json::from_str(&json_response).unwrap_or_default();

        // Add conclusion node
        let conclusion_id = chain.add_node(EvidenceNode::conclusion(conclusion_claim));

        // Wire observations to conclusion via LLM-derived weights
        for item in &weights {
            let idx = item.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            let weight = item
                .get("weight")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.5)
                .clamp(0.0, 1.0);

            if let Some(&obs_id) = obs_ids.get(idx) {
                chain.add_edge(EvidenceEdge {
                    from: obs_id,
                    to: conclusion_id,
                    kind: EdgeKind::Supports,
                    weight,
                });
            }
        }

        // Fall back: wire all observations equally if LLM returned nothing useful
        if weights.is_empty() {
            warn!("LLM returned no weights; using default weight 0.5 for all observations");
            let default_weight = 0.5;
            for &obs_id in &obs_ids {
                chain.add_edge(EvidenceEdge {
                    from: obs_id,
                    to: conclusion_id,
                    kind: EdgeKind::Supports,
                    weight: default_weight,
                });
            }
        }

        chain.propagate_confidence();
        Ok(chain)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    #![allow(
        clippy::disallowed_methods,
        clippy::field_reassign_with_default,
        clippy::manual_range_contains,
        clippy::needless_borrows_for_generic_args,
        clippy::cloned_ref_to_slice_refs
    )]

    use super::*;
    use chrono::Duration;

    fn build_simple_chain() -> EvidenceChain {
        let mut chain = EvidenceChain::new();
        let a = chain.add_node(EvidenceNode::observation("Obs A", 0.9));
        let b = chain.add_node(EvidenceNode::observation("Obs B", 0.8));
        let c = chain.add_node(EvidenceNode::inference("Inferred C", 0.0));
        let d = chain.add_node(EvidenceNode::conclusion("Conclusion D"));
        chain.add_edge(EvidenceEdge {
            from: a,
            to: c,
            kind: EdgeKind::Supports,
            weight: 0.9,
        });
        chain.add_edge(EvidenceEdge {
            from: b,
            to: c,
            kind: EdgeKind::Supports,
            weight: 0.8,
        });
        chain.add_edge(EvidenceEdge {
            from: c,
            to: d,
            kind: EdgeKind::Supports,
            weight: 0.95,
        });
        chain
    }

    #[test]
    fn confidence_propagation_increases_leaf_confidence() {
        let mut chain = build_simple_chain();
        assert!(
            (chain.node(3).map(|n| n.confidence).unwrap_or_default()).abs() < f64::EPSILON,
            "conclusion starts at 0"
        );
        chain.propagate_confidence();
        assert!(
            chain.node(3).map(|n| n.confidence).unwrap_or_default() > 0.5,
            "conclusion should have substantial confidence after propagation"
        );
    }

    #[test]
    fn contradiction_edge_reduces_confidence() {
        let mut chain = EvidenceChain::new();
        let a = chain.add_node(EvidenceNode::observation("Support", 0.9));
        let b = chain.add_node(EvidenceNode::observation("Contradict", 0.8));
        let c = chain.add_node(EvidenceNode::conclusion("Claim"));
        chain.add_edge(EvidenceEdge {
            from: a,
            to: c,
            kind: EdgeKind::Supports,
            weight: 0.9,
        });
        chain.add_edge(EvidenceEdge {
            from: b,
            to: c,
            kind: EdgeKind::Contradicts,
            weight: 0.8,
        });
        chain.propagate_confidence();

        let contradictions = chain.contradictions(0.5);
        assert!(!contradictions.is_empty());
    }

    #[test]
    fn freshness_weight_bands_old_evidence() {
        let now = Utc::now();
        let recent =
            EvidenceNode::observation("recent", 0.9).with_observed_at(now - Duration::days(5));
        let mid = EvidenceNode::observation("mid", 0.9).with_observed_at(now - Duration::days(45));
        let stale =
            EvidenceNode::observation("stale", 0.9).with_observed_at(now - Duration::days(90));

        let recent_weight = node_freshness_weight(&recent, now);
        let mid_weight = node_freshness_weight(&mid, now);
        let stale_weight = node_freshness_weight(&stale, now);

        assert!((recent_weight - 1.0).abs() < 1e-9);
        assert!(mid_weight < recent_weight);
        assert!((stale_weight - 0.25).abs() < 0.03, "stale={stale_weight}");
    }

    #[test]
    fn stale_evidence_reduces_chain_strength() {
        let now = Utc::now();
        let mut chain = EvidenceChain::new();
        let recent = chain.add_node(
            EvidenceNode::observation("Recent support", 0.9)
                .with_observed_at(now - Duration::days(5)),
        );
        let stale = chain.add_node(
            EvidenceNode::observation("Stale support", 0.9)
                .with_observed_at(now - Duration::days(90)),
        );
        let conclusion = chain.add_node(EvidenceNode::conclusion("Conclusion"));
        chain.add_edge(EvidenceEdge {
            from: recent,
            to: conclusion,
            kind: EdgeKind::Supports,
            weight: 1.0,
        });
        chain.add_edge(EvidenceEdge {
            from: stale,
            to: conclusion,
            kind: EdgeKind::Supports,
            weight: 1.0,
        });

        chain.propagate_confidence_at(now);

        let conclusion_confidence = chain.node(conclusion).unwrap().confidence;
        assert!(conclusion_confidence > 0.9);
        assert!(conclusion_confidence < 1.0);
    }

    #[test]
    fn cycle_detection_works() {
        let mut chain = EvidenceChain::new();
        let a = chain.add_node(EvidenceNode::observation("A", 0.9));
        let b = chain.add_node(EvidenceNode::inference("B", 0.0));
        chain.add_edge(EvidenceEdge {
            from: a,
            to: b,
            kind: EdgeKind::Supports,
            weight: 1.0,
        });
        assert!(!chain.has_cycle());

        // Add back edge to create cycle
        chain.add_edge(EvidenceEdge {
            from: b,
            to: a,
            kind: EdgeKind::Supports,
            weight: 1.0,
        });
        assert!(chain.has_cycle());
    }

    #[test]
    fn chain_strength_returns_conclusion_average() {
        let mut chain = build_simple_chain();
        chain.propagate_confidence();
        let strength = chain.chain_strength();
        assert!(strength > 0.0 && strength <= 1.0);
    }

    #[test]
    fn summary_is_consistent() {
        let chain = build_simple_chain();
        let summary = chain.summary();
        assert_eq!(summary.node_count, 4);
        assert_eq!(summary.edge_count, 3);
    }
}
