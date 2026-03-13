use std::collections::HashMap;

use apex_shared::{BridgeNode, CommunityCluster, GraphNodeLayout, TypedEdge};
use leptos::*;

const COMMUNITY_PALETTE: [&str; 10] = [
    "var(--community-1)", "var(--community-2)", "var(--community-3)", "var(--community-4)",
    "var(--community-5)", "var(--community-6)", "var(--community-7)", "var(--community-8)",
    "var(--community-9)", "var(--community-10)",
];

fn palette_map(communities: &[CommunityCluster]) -> HashMap<String, &'static str> {
    communities
        .iter()
        .enumerate()
        .map(|(index, community)| (community.id.clone(), COMMUNITY_PALETTE[index % COMMUNITY_PALETTE.len()]))
        .collect()
}

fn edge_color(edge_type: &str) -> &'static str {
    match edge_type {
        "supplier_of" => "var(--success)",
        "competes_with" => "var(--destructive)",
        "subsidiary_of" => "var(--accent)",
        _ => "var(--chart-grid-soft)",
    }
}

fn edge_label(edge_type: &str) -> &'static str {
    match edge_type {
        "supplier_of" => "Supplier",
        "competes_with" => "Competitor",
        "subsidiary_of" => "Subsidiary",
        _ => "Linked",
    }
}

#[derive(Clone, Copy, Debug)]
struct GraphViewport {
    scale: f64,
    offset_x: f64,
    offset_y: f64,
}

impl Default for GraphViewport {
    fn default() -> Self {
        Self {
            scale: 1.0,
            offset_x: 0.0,
            offset_y: 0.0,
        }
    }
}

fn clamp_scale(scale: f64) -> f64 {
    scale.clamp(0.55, 2.5)
}

#[component]
pub fn CommunityGraph(
    nodes: Vec<GraphNodeLayout>,
    edges: Vec<TypedEdge>,
    communities: Vec<CommunityCluster>,
    bridges: Vec<BridgeNode>,
) -> impl IntoView {
    let colors = palette_map(&communities);
    let index = nodes.iter().map(|node| (node.id.clone(), node.clone())).collect::<HashMap<_, _>>();
    let adjacency = edges.iter().fold(HashMap::<String, Vec<String>>::new(), |mut map, edge| {
        map.entry(edge.source.clone()).or_default().push(edge.target.clone());
        map.entry(edge.target.clone()).or_default().push(edge.source.clone());
        map
    });
    let community_index = communities
        .iter()
        .map(|community| (community.id.clone(), community.clone()))
        .collect::<HashMap<_, _>>();
    let bridge_index = bridges
        .iter()
        .map(|bridge| (bridge.entity_id.clone(), bridge.clone()))
        .collect::<HashMap<_, _>>();

    let viewport = create_rw_signal(GraphViewport::default());
    let dragging = create_rw_signal(false);
    let drag_origin = create_rw_signal((0.0_f64, 0.0_f64));
    let pan_origin = create_rw_signal((0.0_f64, 0.0_f64));
    let selected_node_id = create_rw_signal(nodes.first().map(|node| node.id.clone()).unwrap_or_default());
    let hovered_node_id = create_rw_signal(None::<String>);

    let active_node_id = move || {
        hovered_node_id.get().or_else(|| {
            let selected = selected_node_id.get();
            if selected.is_empty() {
                None
            } else {
                Some(selected)
            }
        })
    };

    let transform = move || {
        let state = viewport.get();
        format!(
            "translate({:.2} {:.2}) scale({:.3})",
            state.offset_x,
            state.offset_y,
            state.scale
        )
    };

    let zoom_by = move |factor: f64| {
        viewport.update(|state| {
            state.scale = clamp_scale(state.scale * factor);
        });
    };

    let reset_view = move |_| viewport.set(GraphViewport::default());

    let edge_view = edges
        .iter()
        .filter_map(|edge| {
            let source = index.get(&edge.source)?;
            let target = index.get(&edge.target)?;
            let source_id = source.id.clone();
            let target_id = target.id.clone();
            let source_label = source.label.clone();
            let target_label = target.label.clone();
            let edge_type = edge.edge_type.clone();
            let stroke = edge_color(&edge_type);
            let edge_title = format!(
                "{} between {} and {} ({:.1} strength)",
                edge_label(&edge_type),
                source_label,
                target_label,
                edge.weight.unwrap_or(1.0)
            );
            Some(view! {
                <line
                    class=move || {
                        let is_active = active_node_id()
                            .map(|id| id == source_id || id == target_id)
                            .unwrap_or(true);
                        if is_active {
                            "graph-edge"
                        } else {
                            "graph-edge graph-edge-dimmed"
                        }
                    }
                    x1=format!("{:.2}", source.x)
                    y1=format!("{:.2}", source.y)
                    x2=format!("{:.2}", target.x)
                    y2=format!("{:.2}", target.y)
                    stroke=stroke
                    stroke-width=format!("{:.1}", edge.weight.unwrap_or(1.0) * 1.8)
                >
                    <title>{edge_title}</title>
                </line>
            })
        })
        .collect_view();

    let node_view = nodes
        .iter()
        .map(|node| {
            let radius = 8.0 + node.betweenness * 26.0;
            let color = colors.get(&node.community_id).copied().unwrap_or("var(--muted)");
            let is_bridge = bridges.iter().any(|bridge| bridge.entity_id == node.id);
            let stroke = if is_bridge { "var(--warning)" } else { "var(--card)" };
            let stroke_width = if is_bridge { "3" } else { "1.5" };
            let node_id = node.id.clone();
            let node_id_for_mouse_enter = node.id.clone();
            let node_id_for_focus = node.id.clone();
            let node_id_for_click = node.id.clone();
            let node_id_for_keydown = node.id.clone();
            let node_label = node.label.clone();
            let community_name = community_index
                .get(&node.community_id)
                .and_then(|community| community.name.clone())
                .unwrap_or_else(|| node.community_id.clone());
            let title = if is_bridge {
                format!("{} in {}. Bridge node with centrality {:.2}", node_label, community_name, node.betweenness)
            } else {
                format!("{} in {}. Centrality {:.2}", node_label, community_name, node.betweenness)
            };

            view! {
                <g
                    class=move || {
                        let hovered = hovered_node_id.get();
                        let selected = selected_node_id.get();
                        let is_active = hovered.as_ref().map(|id| id == &node_id).unwrap_or(false) || selected == node_id;
                        if is_active {
                            "graph-node graph-node-active"
                        } else {
                            "graph-node"
                        }
                    }
                    tabindex="0"
                    role="button"
                    on:mouseenter=move |_| hovered_node_id.set(Some(node_id_for_mouse_enter.clone()))
                    on:mouseleave=move |_| hovered_node_id.set(None)
                    on:focus=move |_| hovered_node_id.set(Some(node_id_for_focus.clone()))
                    on:blur=move |_| hovered_node_id.set(None)
                    on:click=move |_| selected_node_id.set(node_id_for_click.clone())
                    on:keydown=move |ev| {
                        let key = ev.key();
                        if key == "Enter" || key == " " {
                            ev.prevent_default();
                            selected_node_id.set(node_id_for_keydown.clone());
                        }
                    }
                >
                    <circle
                        class="graph-node-circle"
                        cx=format!("{:.2}", node.x)
                        cy=format!("{:.2}", node.y)
                        r=format!("{radius:.2}")
                        fill=color
                        stroke=stroke
                        stroke-width=stroke_width
                    />
                    <text
                        x=format!("{:.2}", node.x)
                        y=format!("{:.2}", node.y + radius + 14.0)
                        text-anchor="middle"
                        font-size="11"
                        font-weight="700"
                    >
                        {node.label.clone()}
                    </text>
                    <title>{title}</title>
                </g>
            }
        })
        .collect_view();

    let selected_node_view = move || {
        let selected = selected_node_id.get();
        let Some(node) = index.get(&selected).cloned() else {
            return view! { <p class="muted-copy">"Select a node to inspect its neighborhood."</p> }.into_view();
        };

        let community = community_index
            .get(&node.community_id)
            .cloned();
        let neighbors = adjacency
            .get(&node.id)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|neighbor_id| index.get(&neighbor_id).map(|neighbor| neighbor.label.clone()))
            .take(8)
            .collect::<Vec<_>>();
        let bridge = bridge_index.get(&node.id).cloned();
        let community_label = community
            .as_ref()
            .and_then(|community| community.name.clone())
            .unwrap_or_else(|| node.community_id.clone());
        let bridge_view = if let Some(bridge_entry) = bridge.clone() {
            view! {
                <div class="timeline-item">
                    <strong>"Bridge Signal"</strong>
                    <span class="muted-copy">
                        {format!("Connects {} communities", bridge_entry.connected_communities.len())}
                    </span>
                </div>
            }
            .into_view()
        } else {
            view! { <></> }.into_view()
        };
        let community_view = if let Some(community_entry) = community.clone() {
            view! {
                <div class="timeline-item">
                    <strong>"Cluster Members"</strong>
                    <div class="code-list">
                        <For
                            each=move || community_entry.top_members.clone()
                            key=|member| member.clone()
                            let:member
                        >
                            <span class="code-pill">{member}</span>
                        </For>
                    </div>
                </div>
            }
            .into_view()
        } else {
            view! { <></> }.into_view()
        };

        view! {
            <div class="graph-side-panel">
                <div class="graph-side-header">
                    <h3 class="graph-side-title">{node.label.clone()}</h3>
                    <span class="source-badge tier-established">{community_label}</span>
                </div>
                <div class="timeline-list">
                    <div class="timeline-item">
                        <strong>"Connectivity"</strong>
                        <span class="muted-copy">{format!("{:.0}% centrality", node.betweenness * 100.0)}</span>
                    </div>
                    <div class="timeline-item">
                        <strong>"Neighborhood"</strong>
                        <span class="muted-copy">{format!("{} adjacent nodes", neighbors.len())}</span>
                        <div class="code-list">
                            <For each=move || neighbors.clone() key=|label| label.clone() let:label>
                                <span class="code-pill">{label}</span>
                            </For>
                        </div>
                    </div>
                    {bridge_view}
                    {community_view}
                </div>
            </div>
        }
        .into_view()
    };

    view! {
        <div class="chart-layout chart-layout-graph">
            <div class="chart-scroll-shell chart-scroll-shell-wide">
                <div class="graph-toolbar">
                    <div class="graph-toolbar-copy">"Drag to pan. Use controls or wheel to zoom."</div>
                    <div class="graph-toolbar-actions">
                        <button type="button" class="pagination-button" on:click=move |_| zoom_by(1.18)>"Zoom In"</button>
                        <button type="button" class="pagination-button" on:click=move |_| zoom_by(0.84)>"Zoom Out"</button>
                        <button type="button" class="pagination-button" on:click=reset_view>"Reset"</button>
                    </div>
                </div>
                <svg viewBox="0 0 1000 760" class="community-graph" aria-label="Community intelligence graph">
                    <rect x="0" y="0" width="1000" height="760" fill="var(--card)" rx="12" />
                    <rect
                        x="0"
                        y="0"
                        width="1000"
                        height="760"
                        fill="transparent"
                        class="graph-pan-surface"
                        on:mousedown=move |ev| {
                            dragging.set(true);
                            drag_origin.set((ev.client_x() as f64, ev.client_y() as f64));
                            let state = viewport.get_untracked();
                            pan_origin.set((state.offset_x, state.offset_y));
                        }
                        on:mousemove=move |ev| {
                            if !dragging.get() {
                                return;
                            }
                            let (start_x, start_y) = drag_origin.get();
                            let (origin_x, origin_y) = pan_origin.get();
                            let state = viewport.get_untracked();
                            let scale = state.scale.max(0.01);
                            let delta_x = (ev.client_x() as f64 - start_x) / scale;
                            let delta_y = (ev.client_y() as f64 - start_y) / scale;
                            viewport.set(GraphViewport {
                                scale: state.scale,
                                offset_x: origin_x + delta_x,
                                offset_y: origin_y + delta_y,
                            });
                        }
                        on:mouseup=move |_| dragging.set(false)
                        on:mouseleave=move |_| dragging.set(false)
                        on:wheel=move |ev| {
                            ev.prevent_default();
                            let factor = if ev.delta_y() < 0.0 { 1.08 } else { 0.92 };
                            viewport.update(|state| {
                                state.scale = clamp_scale(state.scale * factor);
                            });
                        }
                    />
                    <g transform=transform>
                        {edge_view}
                        {node_view}
                    </g>
                </svg>
            </div>
            {selected_node_view}
        </div>
    }
}