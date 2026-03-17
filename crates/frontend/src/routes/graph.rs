use std::collections::{HashMap, HashSet};

use apex_shared::{BridgeNode, CommunityCluster, GraphNodeLayout, SnapshotDelta, TypedEdge};
use leptos::*;

use crate::{api, components::{cards::{PageHeader, SurfaceCard}, charts::community_graph::CommunityGraph}};

fn snapshot_delta(previous: &api::GraphOverview, current: &api::GraphOverview) -> SnapshotDelta {
    let previous_edges = previous
        .edges
        .iter()
        .map(|edge| (edge.source.clone(), edge.target.clone(), edge.edge_type.clone()))
        .collect::<HashSet<_>>();
    let current_edges = current
        .edges
        .iter()
        .map(|edge| (edge.source.clone(), edge.target.clone(), edge.edge_type.clone()))
        .collect::<HashSet<_>>();
    let previous_nodes = previous.nodes.iter().map(|node| node.id.clone()).collect::<HashSet<_>>();
    let current_nodes = current.nodes.iter().map(|node| node.id.clone()).collect::<HashSet<_>>();

    SnapshotDelta {
        added_edges: current
            .edges
            .iter()
            .filter(|edge| !previous_edges.contains(&(edge.source.clone(), edge.target.clone(), edge.edge_type.clone())))
            .map(|edge| TypedEdge {
                source: edge.source.clone(),
                target: edge.target.clone(),
                edge_type: edge.edge_type.clone(),
                weight: Some(edge.weight),
            })
            .collect(),
        removed_edges: previous
            .edges
            .iter()
            .filter(|edge| !current_edges.contains(&(edge.source.clone(), edge.target.clone(), edge.edge_type.clone())))
            .map(|edge| TypedEdge {
                source: edge.source.clone(),
                target: edge.target.clone(),
                edge_type: edge.edge_type.clone(),
                weight: Some(edge.weight),
            })
            .collect(),
        added_nodes: current_nodes.difference(&previous_nodes).cloned().collect(),
        removed_nodes: previous_nodes.difference(&current_nodes).cloned().collect(),
    }
}

#[cfg(target_arch = "wasm32")]
fn load_previous_graph_snapshot() -> Option<api::GraphOverview> {
    let window = web_sys::window()?;
    let storage = window.local_storage().ok()??;
    let value = storage.get_item("apex.graph.snapshot").ok()??;
    serde_json::from_str(&value).ok()
}

#[cfg(not(target_arch = "wasm32"))]
fn load_previous_graph_snapshot() -> Option<api::GraphOverview> {
    None
}

#[cfg(target_arch = "wasm32")]
fn persist_graph_snapshot(graph: &api::GraphOverview) {
    if let Some(window) = web_sys::window() {
        if let Ok(Some(storage)) = window.local_storage() {
            if let Ok(json) = serde_json::to_string(graph) {
                let _ = storage.set_item("apex.graph.snapshot", &json);
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn persist_graph_snapshot(_graph: &api::GraphOverview) {}

fn detect_communities(graph: &api::GraphOverview) -> HashMap<String, String> {
    let mut adjacency: HashMap<String, Vec<String>> = HashMap::new();
    let mut labels = HashMap::new();

    for node in &graph.nodes {
        adjacency.entry(node.id.clone()).or_default();
        labels.insert(node.id.clone(), node.id.clone());
    }

    for edge in &graph.edges {
        adjacency.entry(edge.source.clone()).or_default().push(edge.target.clone());
        adjacency.entry(edge.target.clone()).or_default().push(edge.source.clone());
    }

    for _ in 0..8 {
        for node in &graph.nodes {
            let Some(neighbors) = adjacency.get(&node.id) else { continue; };
            let mut counts: HashMap<String, usize> = HashMap::new();
            for neighbor in neighbors {
                if let Some(label) = labels.get(neighbor) {
                    *counts.entry(label.clone()).or_insert(0) += 1;
                }
            }
            if let Some((label, _)) = counts.into_iter().max_by_key(|(_, count)| *count) {
                labels.insert(node.id.clone(), label);
            }
        }
    }

    labels
}

fn build_communities(graph: &api::GraphOverview, labels: &HashMap<String, String>) -> Vec<CommunityCluster> {
    let mut grouped: HashMap<String, Vec<String>> = HashMap::new();
    let names = graph
        .nodes
        .iter()
        .map(|node| (node.id.clone(), node.label.clone()))
        .collect::<HashMap<_, _>>();

    for node in &graph.nodes {
        grouped
            .entry(labels.get(&node.id).cloned().unwrap_or_else(|| node.id.clone()))
            .or_default()
            .push(node.id.clone());
    }

    grouped
        .into_iter()
        .enumerate()
        .map(|(index, (community_id, members))| CommunityCluster {
            id: community_id.clone(),
            name: Some(format!("Cluster {}", index + 1)),
            member_count: members.len() as u32,
            modularity_contribution: members.len() as f64 / graph.nodes.len().max(1) as f64,
            top_members: members
                .iter()
                .take(4)
                .filter_map(|id| names.get(id).cloned())
                .collect(),
        })
        .collect()
}

fn bridge_nodes(graph: &api::GraphOverview, labels: &HashMap<String, String>) -> Vec<BridgeNode> {
    let mut degrees: HashMap<String, usize> = HashMap::new();
    let name_map = graph
        .nodes
        .iter()
        .map(|node| (node.id.clone(), node.label.clone()))
        .collect::<HashMap<_, _>>();

    for edge in &graph.edges {
        *degrees.entry(edge.source.clone()).or_insert(0) += 1;
        *degrees.entry(edge.target.clone()).or_insert(0) += 1;
    }

    let max_degree = degrees.values().copied().max().unwrap_or(1) as f64;
    let mut bridges = degrees
        .into_iter()
        .map(|(node_id, degree)| BridgeNode {
            entity_id: node_id.clone(),
            entity_name: name_map.get(&node_id).cloned().unwrap_or(node_id.clone()),
            betweenness_centrality: degree as f64 / max_degree,
            connected_communities: graph
                .edges
                .iter()
                .filter(|edge| edge.source == node_id || edge.target == node_id)
                .filter_map(|edge| {
                    let other = if edge.source == node_id { &edge.target } else { &edge.source };
                    labels.get(other).cloned()
                })
                .collect::<HashSet<_>>()
                .into_iter()
                .collect(),
        })
        .collect::<Vec<_>>();
    bridges.sort_by(|a, b| b.betweenness_centrality.partial_cmp(&a.betweenness_centrality).unwrap_or(std::cmp::Ordering::Equal));
    bridges.truncate(8);
    bridges
}

fn force_layout(graph: &api::GraphOverview, labels: &HashMap<String, String>) -> Vec<GraphNodeLayout> {
    let count = graph.nodes.len().max(1) as f64;
    let mut positions = graph
        .nodes
        .iter()
        .enumerate()
        .map(|(index, node)| {
            let angle = index as f64 / count * std::f64::consts::TAU;
            (node.id.clone(), (500.0 + angle.cos() * 220.0, 340.0 + angle.sin() * 220.0))
        })
        .collect::<HashMap<_, _>>();

    for _ in 0..40 {
        let mut delta = graph
            .nodes
            .iter()
            .map(|node| (node.id.clone(), (0.0, 0.0)))
            .collect::<HashMap<_, _>>();

        for source in &graph.nodes {
            for target in &graph.nodes {
                if source.id == target.id {
                    continue;
                }
                let (sx, sy) = positions[&source.id];
                let (tx, ty) = positions[&target.id];
                let dx = sx - tx;
                let dy = sy - ty;
                let distance_sq = (dx * dx + dy * dy).max(25.0);
                let repulsion = 2400.0 / distance_sq;
                if let Some(entry) = delta.get_mut(&source.id) {
                    entry.0 += dx * repulsion;
                    entry.1 += dy * repulsion;
                }
            }
        }

        for edge in &graph.edges {
            let (sx, sy) = positions[&edge.source];
            let (tx, ty) = positions[&edge.target];
            let dx = tx - sx;
            let dy = ty - sy;
            let attraction = 0.0012 * edge.weight.max(0.3);
            if let Some(source_delta) = delta.get_mut(&edge.source) {
                source_delta.0 += dx * attraction;
                source_delta.1 += dy * attraction;
            }
            if let Some(target_delta) = delta.get_mut(&edge.target) {
                target_delta.0 -= dx * attraction;
                target_delta.1 -= dy * attraction;
            }
        }

        for node in &graph.nodes {
            let (dx, dy) = delta[&node.id];
            let (x, y) = positions[&node.id];
            positions.insert(node.id.clone(), ((x + dx).clamp(80.0, 920.0), (y + dy).clamp(80.0, 680.0)));
        }
    }

    let degree_map = graph.edges.iter().fold(HashMap::<String, usize>::new(), |mut map, edge| {
        *map.entry(edge.source.clone()).or_insert(0) += 1;
        *map.entry(edge.target.clone()).or_insert(0) += 1;
        map
    });
    let max_degree = degree_map.values().copied().max().unwrap_or(1) as f64;

    graph
        .nodes
        .iter()
        .map(|node| {
            let (x, y) = positions[&node.id];
            GraphNodeLayout {
                id: node.id.clone(),
                label: node.label.clone(),
                x,
                y,
                community_id: labels.get(&node.id).cloned().unwrap_or_else(|| node.id.clone()),
                betweenness: degree_map.get(&node.id).copied().unwrap_or(0) as f64 / max_degree,
            }
        })
        .collect()
}

#[component]
pub fn GraphPage() -> impl IntoView {
    let graph = create_resource(|| (), |_| async { api::fetch_graph().await });

    view! {
        <div class="page">
            <PageHeader
                eyebrow="Graph Intelligence"
                title="Community Graph"
                subtitle="The graph route now consumes the live `/api/graph` payload and computes a client-side force layout in Rust."
            />

            <Suspense fallback=move || view! { <SurfaceCard title="Community View" subtitle="Loading graph overview."><p class="muted-copy">"Loading..."</p></SurfaceCard> }>
                {move || graph.get().map(|result| match result {
                    Ok(graph) => {
                        let diff = load_previous_graph_snapshot().map(|previous| snapshot_delta(&previous, &graph));
                        persist_graph_snapshot(&graph);
                        let community_labels = detect_communities(&graph);
                        let communities = build_communities(&graph, &community_labels);
                        let bridges = bridge_nodes(&graph, &community_labels);
                        let nodes = force_layout(&graph, &community_labels);
                        let added_edges_count = diff.as_ref().map(|delta| delta.added_edges.len()).unwrap_or_default();
                        let removed_edges_count = diff.as_ref().map(|delta| delta.removed_edges.len()).unwrap_or_default();
                        let node_change_count = diff.as_ref().map(|delta| delta.added_nodes.len() + delta.removed_nodes.len()).unwrap_or_default();
                        let delta_view = if diff.is_some() {
                            view! {
                                <SurfaceCard title="Snapshot Delta" subtitle="Differences versus the last graph snapshot stored by this browser.">
                                    <div class="three-up">
                                        <div class="stat-mini">{format!("{} added edges", added_edges_count)}</div>
                                        <div class="stat-mini">{format!("{} removed edges", removed_edges_count)}</div>
                                        <div class="stat-mini">{format!("{} node changes", node_change_count)}</div>
                                    </div>
                                </SurfaceCard>
                            }.into_view()
                        } else {
                            view! {}.into_view()
                        };
                        let edges = graph.edges.iter().map(|edge| TypedEdge {
                            source: edge.source.clone(),
                            target: edge.target.clone(),
                            edge_type: edge.edge_type.clone(),
                            weight: Some(edge.weight),
                        }).collect::<Vec<_>>();

                        view! {
                            <div class="page">
                                {delta_view}
                                <SurfaceCard title="Community View" subtitle="Graph topology is live; layout and clustering are computed inside the WASM frontend.">
                                    <CommunityGraph nodes=nodes communities=communities bridges=bridges edges=edges />
                                </SurfaceCard>
                            </div>
                        }.into_view()
                    }
                    Err(message) => view! { <SurfaceCard title="Community View" subtitle="The graph request failed."><p class="error-copy">{message}</p></SurfaceCard> }.into_view(),
                })}
            </Suspense>
        </div>
    }
}