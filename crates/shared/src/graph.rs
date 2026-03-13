use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct CommunityCluster {
    pub id: String,
    pub name: Option<String>,
    pub member_count: u32,
    pub modularity_contribution: f64,
    pub top_members: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct BridgeNode {
    pub entity_id: String,
    pub entity_name: String,
    pub betweenness_centrality: f64,
    pub connected_communities: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct TypedEdge {
    pub source: String,
    pub target: String,
    pub edge_type: String,
    pub weight: Option<f64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct GraphNodeLayout {
    pub id: String,
    pub label: String,
    pub x: f64,
    pub y: f64,
    pub community_id: String,
    pub betweenness: f64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct SnapshotDelta {
    pub added_edges: Vec<TypedEdge>,
    pub removed_edges: Vec<TypedEdge>,
    pub added_nodes: Vec<String>,
    pub removed_nodes: Vec<String>,
}