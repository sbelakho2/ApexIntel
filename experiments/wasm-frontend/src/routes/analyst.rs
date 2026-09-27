//! Analyst View - Investigation workspace, entity relationships, source evidence, collaboration.
//!
//! Phase 4.3: User Experience Enhancement

use leptos::*;

use crate::components::{
    cards::{PageHeader, SurfaceCard},
    charts::probability_gauge::ProbabilityGauge,
};

// ────────────────────────────────────────────
// API Response Types
// ────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct InvestigationWorkspace {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub workspace_type: String,
    pub owner_id: String,
    pub team_id: Option<String>,
    pub status: String,
    pub visibility: String,
    pub tags: Vec<String>,
    pub findings: Option<String>,
    pub conclusions: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WorkspaceAssignment {
    pub id: String,
    pub workspace_id: String,
    pub user_id: String,
    pub role: String,
    pub assigned_by: String,
    pub assigned_at: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ActivityEntry {
    pub id: String,
    pub actor_id: String,
    pub actor_name: String,
    pub action_type: String,
    pub entity_type: Option<String>,
    pub entity_id: Option<String>,
    pub entity_name: Option<String>,
    pub details: serde_json::Value,
    pub formatted_details: Option<String>,
    #[serde(default)]
    pub workspace_id: Option<String>,
    #[serde(default)]
    pub team_id: Option<String>,
    #[serde(default)]
    pub visibility: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SourceEvidence {
    pub id: String,
    pub entity_type: String,
    pub entity_id: String,
    pub evidence_type: String,
    pub source_url: String,
    pub source_domain: Option<String>,
    pub source_name: Option<String>,
    pub reliability_score: f64,
    pub excerpt: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EntityRelationship {
    pub source_id: String,
    pub source_type: String,
    pub source_name: String,
    pub target_id: String,
    pub target_type: String,
    pub target_name: String,
    pub edge_type: String,
    pub weight: f64,
    pub confidence: f64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GraphNode {
    pub id: String,
    pub node_type: String,
    pub name: String,
    pub properties: serde_json::Value,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EntityGraphData {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<EntityRelationship>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CreateWorkspaceRequest {
    pub name: String,
    pub description: Option<String>,
    pub workspace_type: String,
    pub visibility: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AddEvidenceRequest {
    pub evidence_type: String,
    pub source_url: String,
    pub source_name: Option<String>,
    pub reliability_score: f64,
    pub excerpt: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ApiEnvelope<T> {
    pub success: bool,
    pub data: Option<T>,
}

// ────────────────────────────────────────────
// API Functions
// ────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
pub async fn fetch_workspaces() -> Result<Vec<InvestigationWorkspace>, String> {
    use gloo_net::http::Request;
    use web_sys::RequestCredentials;
    let response = Request::get(&api_url("/api/workspaces"))
        .credentials(RequestCredentials::SameOrigin)
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| format!("Failed to read body: {}", e))?;
    let envelope: ApiEnvelope<Vec<InvestigationWorkspace>> =
        serde_json::from_str(&body).map_err(|e| format!("Failed to parse: {}", e))?;
    if status >= 400 || !envelope.success {
        return Err(format!("API error (status {})", status));
    }
    envelope
        .data
        .ok_or_else(|| "Failed to fetch workspaces".to_string())
}

#[cfg(not(target_arch = "wasm32"))]
pub async fn fetch_workspaces() -> Result<Vec<InvestigationWorkspace>, String> {
    Err("WASM data fetching is only available in the browser runtime".to_string())
}

#[cfg(target_arch = "wasm32")]
pub async fn fetch_workspace(id: &str) -> Result<InvestigationWorkspace, String> {
    use gloo_net::http::Request;
    use web_sys::RequestCredentials;
    let response = Request::get(&api_url(&format!("/api/workspaces/{}", id)))
        .credentials(RequestCredentials::SameOrigin)
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| format!("Failed to read body: {}", e))?;
    let envelope: ApiEnvelope<InvestigationWorkspace> =
        serde_json::from_str(&body).map_err(|e| format!("Failed to parse: {}", e))?;
    if status >= 400 || !envelope.success {
        return Err(format!("API error (status {})", status));
    }
    envelope
        .data
        .ok_or_else(|| "Failed to fetch workspace".to_string())
}

#[cfg(not(target_arch = "wasm32"))]
pub async fn fetch_workspace(_id: &str) -> Result<InvestigationWorkspace, String> {
    Err("WASM data fetching is only available in the browser runtime".to_string())
}

#[cfg(target_arch = "wasm32")]
pub async fn create_workspace(
    req: CreateWorkspaceRequest,
) -> Result<InvestigationWorkspace, String> {
    use gloo_net::http::Request;
    use web_sys::RequestCredentials;
    let response = Request::post(&api_url("/api/workspaces"))
        .credentials(RequestCredentials::SameOrigin)
        .json(&req)
        .map_err(|e| format!("Serialization error: {}", e))?
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| format!("Failed to read body: {}", e))?;
    let envelope: ApiEnvelope<InvestigationWorkspace> =
        serde_json::from_str(&body).map_err(|e| format!("Failed to parse: {}", e))?;
    if status >= 400 || !envelope.success {
        return Err(format!("API error (status {})", status));
    }
    envelope
        .data
        .ok_or_else(|| "Failed to create workspace".to_string())
}

#[cfg(not(target_arch = "wasm32"))]
pub async fn create_workspace(
    _req: CreateWorkspaceRequest,
) -> Result<InvestigationWorkspace, String> {
    Err("WASM mutations are only available in the browser runtime".to_string())
}

#[cfg(target_arch = "wasm32")]
pub async fn fetch_activity_feed(workspace_id: Option<&str>) -> Result<Vec<ActivityEntry>, String> {
    use gloo_net::http::Request;
    use web_sys::RequestCredentials;
    let url = match workspace_id {
        Some(id) => api_url(&format!("/api/activity-feed?workspace_id={}", id)),
        None => api_url("/api/activity-feed"),
    };
    let response = Request::get(&url)
        .credentials(RequestCredentials::SameOrigin)
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| format!("Failed to read body: {}", e))?;
    let envelope: ApiEnvelope<Vec<ActivityEntry>> =
        serde_json::from_str(&body).map_err(|e| format!("Failed to parse: {}", e))?;
    if status >= 400 || !envelope.success {
        return Err(format!("API error (status {})", status));
    }
    envelope
        .data
        .ok_or_else(|| "Failed to fetch activity feed".to_string())
}

#[cfg(not(target_arch = "wasm32"))]
pub async fn fetch_activity_feed(
    _workspace_id: Option<&str>,
) -> Result<Vec<ActivityEntry>, String> {
    Err("WASM data fetching is only available in the browser runtime".to_string())
}

#[cfg(target_arch = "wasm32")]
pub async fn fetch_entity_relationships(
    _entity_type: &str,
    entity_id: &str,
) -> Result<EntityGraphData, String> {
    use gloo_net::http::Request;
    use web_sys::RequestCredentials;
    let response = Request::get(&api_url(&format!("/api/graph/neighborhood/{}", entity_id)))
        .credentials(RequestCredentials::SameOrigin)
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| format!("Failed to read body: {}", e))?;
    let json: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("Failed to parse: {}", e))?;
    if status >= 400 {
        return Err(format!("API error (status {})", status));
    }
    let nodes = json["nodes"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|n| GraphNode {
                    id: n["id"].as_str().unwrap_or_default().to_string(),
                    node_type: n["node_type"].as_str().unwrap_or_default().to_string(),
                    name: n["name"].as_str().unwrap_or_default().to_string(),
                    properties: n["properties"].clone(),
                })
                .collect()
        })
        .unwrap_or_default();
    let edges = json["edges"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|e| EntityRelationship {
                    source_id: e["source_id"].as_str().unwrap_or_default().to_string(),
                    source_type: e["source_type"].as_str().unwrap_or_default().to_string(),
                    source_name: e["source_name"].as_str().unwrap_or_default().to_string(),
                    target_id: e["target_id"].as_str().unwrap_or_default().to_string(),
                    target_type: e["target_type"].as_str().unwrap_or_default().to_string(),
                    target_name: e["target_name"].as_str().unwrap_or_default().to_string(),
                    edge_type: e["edge_type"].as_str().unwrap_or_default().to_string(),
                    weight: e["weight"].as_f64().unwrap_or(1.0),
                    confidence: e["confidence"].as_f64().unwrap_or(1.0),
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(EntityGraphData { nodes, edges })
}

#[cfg(not(target_arch = "wasm32"))]
pub async fn fetch_entity_relationships(
    _entity_type: &str,
    _entity_id: &str,
) -> Result<EntityGraphData, String> {
    Err("WASM data fetching is only available in the browser runtime".to_string())
}

#[cfg(target_arch = "wasm32")]
pub async fn fetch_source_evidence(
    entity_type: Option<&str>,
    entity_id: Option<&str>,
) -> Result<Vec<SourceEvidence>, String> {
    use gloo_net::http::Request;
    use web_sys::RequestCredentials;
    let base = api_url("/api/evidence");
    let url = if let Some(et) = entity_type {
        if let Some(eid) = entity_id {
            format!("{}?entity_type={}&entity_id={}", base, et, eid)
        } else {
            format!("{}?entity_type={}", base, et)
        }
    } else {
        base
    };
    let response = Request::get(&url)
        .credentials(RequestCredentials::SameOrigin)
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| format!("Failed to read body: {}", e))?;
    let envelope: ApiEnvelope<Vec<SourceEvidence>> =
        serde_json::from_str(&body).map_err(|e| format!("Failed to parse: {}", e))?;
    if status >= 400 || !envelope.success {
        return Err(format!("API error (status {})", status));
    }
    envelope
        .data
        .ok_or_else(|| "Failed to fetch evidence".to_string())
}

#[cfg(not(target_arch = "wasm32"))]
pub async fn fetch_source_evidence(
    _entity_type: Option<&str>,
    _entity_id: Option<&str>,
) -> Result<Vec<SourceEvidence>, String> {
    Err("WASM data fetching is only available in the browser runtime".to_string())
}

// ────────────────────────────────────────────
// Components
// ────────────────────────────────────────────

#[component]
fn WorkspaceCard(workspace: InvestigationWorkspace) -> impl IntoView {
    let status_class = format!("status-{}", workspace.status.to_lowercase());
    let visibility_class = format!("visibility-{}", workspace.visibility.to_lowercase());

    view! {
        <article class="workspace-card">
            <div class="workspace-header">
                <h3 class="workspace-name">{workspace.name}</h3>
                <span class={format!("status-badge {}", status_class)}>{workspace.status}</span>
            </div>
            {workspace.description.map(|desc| view! {
                <p class="workspace-description">{desc}</p>
            })}
            <div class="workspace-meta">
                <span class={format!("visibility-tag {}", visibility_class)}>
                    {workspace.visibility}
                </span>
                <span class="workspace-type">{workspace.workspace_type}</span>
                <span class="workspace-updated">"Updated: {workspace.updated_at}"</span>
            </div>
            <div class="workspace-tags">
                {workspace.tags.iter().map(|tag| view! {
                    <span class="tag">{tag}</span>
                }).collect_view()}
            </div>
            <div class="workspace-footer">
                <span class="owner">"Owner: {workspace.owner_id}"</span>
            </div>
        </article>
    }
}

#[component]
fn EntityNodeCard(node: GraphNode) -> impl IntoView {
    let node_type_class = format!("node-type-{}", node.node_type.to_lowercase());

    view! {
        <div class="entity-node">
            <div class="node-header">
                <span class={format!("node-type-badge {}", node_type_class)}>{node.node_type}</span>
                <h4 class="node-name">{node.name}</h4>
            </div>
            <div class="node-id">"ID: {node.id}"</div>
        </div>
    }
}

#[component]
fn EntityRelationshipRow(relationship: EntityRelationship) -> impl IntoView {
    view! {
        <div class="relationship-row">
            <div class="relationship-endpoint source">
                <span class="endpoint-type">{relationship.source_type}</span>
                <span class="endpoint-name">{relationship.source_name}</span>
            </div>
            <div class="relationship-arrow">
                <span class="edge-type">{relationship.edge_type}</span>
                <span class="edge-weight">"({:.2})"</span>
            </div>
            <div class="relationship-endpoint target">
                <span class="endpoint-type">{relationship.target_type}</span>
                <span class="endpoint-name">{relationship.target_name}</span>
            </div>
            <div class="relationship-confidence">
                <ProbabilityGauge probability=relationship.confidence.clamp(0.0, 1.0) />
            </div>
        </div>
    }
}

#[component]
fn SourceEvidenceCard(evidence: SourceEvidence) -> impl IntoView {
    let reliability_class = if evidence.reliability_score > 0.8 {
        "reliability-high"
    } else if evidence.reliability_score > 0.5 {
        "reliability-medium"
    } else {
        "reliability-low"
    };

    view! {
        <article class="evidence-card">
            <div class="evidence-header">
                <span class="evidence-type">{evidence.evidence_type}</span>
                <span class={format!("reliability-badge {}", reliability_class)}>
                    {format!("{:.0}%", evidence.reliability_score * 100.0)}
                </span>
            </div>
            <div class="evidence-source">
                {evidence.source_name.clone().unwrap_or_else(|| evidence.source_domain.clone().unwrap_or_default())}
            </div>
            <a href={evidence.source_url.clone()} target="_blank" class="evidence-url">
                {evidence.source_url}
            </a>
            {evidence.excerpt.as_ref().map(|excerpt| view! {
                <blockquote class="evidence-excerpt">{excerpt.clone()}</blockquote>
            })}
            <div class="evidence-footer">
                <span class="entity-ref">
                    <span class="ref-type">{evidence.entity_type}</span>
                    ": "
                    <span class="ref-id">{evidence.entity_id}</span>
                </span>
                <span class="evidence-date">{evidence.created_at}</span>
            </div>
        </article>
    }
}

#[component]
fn ActivityFeedItem(activity: ActivityEntry) -> impl IntoView {
    let action_icon = match activity.action_type.as_str() {
        "create" => "➕",
        "update" => "✏️",
        "delete" => "🗑️",
        "share" => "📤",
        "comment" => "💬",
        "assign" => "👤",
        _ => "📝",
    };

    let details_str = activity.formatted_details.clone().unwrap_or_default();
    let has_details = !details_str.is_empty();

    view! {
        <div class="activity-item">
            <span class="activity-icon">{action_icon}</span>
            <div class="activity-content">
                <div class="activity-header">
                    <span class="actor-name">{activity.actor_name}</span>
                    <span class="action-type">{activity.action_type}</span>
                    {activity.entity_name.map(|name| view! {
                        <span class="entity-name">{name}</span>
                    })}
                </div>
                {activity.entity_type.map(|et| view! {
                    <span class="entity-type-badge">{et}</span>
                })}
                {has_details.then(|| view! { <p class="activity-details">{details_str}</p> })}
                <span class="activity-time">{activity.created_at}</span>
            </div>
        </div>
    }
}

#[component]
fn CollaborationPanel(workspace_id: String) -> impl IntoView {
    let activities = create_resource(
        move || workspace_id.clone(),
        |id| async move { fetch_activity_feed(Some(&id)).await },
    );

    view! {
        <SurfaceCard title="Collaboration Activity" subtitle="Recent team activity on this workspace">
            <Suspense fallback=move || view! {
                <p class="muted-copy">"Loading activity..."</p>
            }>
                {move || activities.get().map(|result| match result {
                    Ok(items) => {
                        let items_count = items.len();
                        view! {
                            <div class="activity-feed">
                                <For each=move || items.clone() key=|a| a.id.clone() let:activity>
                                    <ActivityFeedItem activity=activity />
                                </For>
                                {if items_count == 0 {
                                    view! {
                                        <div class="empty-state">
                                            <p>"No recent activity."</p>
                                        </div>
                                    }.into_view()
                                } else { view! {}.into_view() }}
                            </div>
                        }.into_view()
                    }
                    Err(msg) => view! {
                        <p class="error-copy">{msg}</p>
                    }.into_view(),
                })}
            </Suspense>
        </SurfaceCard>
    }
}

#[component]
fn EntityExplorer(entity_id: String, entity_type: String) -> impl IntoView {
    let graph_data = create_resource(
        move || (entity_type.clone(), entity_id.clone()),
        |(et, eid)| async move { fetch_entity_relationships(&et, &eid).await },
    );

    view! {
        <SurfaceCard title="Entity Relationship Explorer" subtitle="Connected entities and relationships">
            <Suspense fallback=move || view! {
                <p class="muted-copy">"Loading relationships..."</p>
            }>
                {move || graph_data.get().map(|result| match result {
                    Ok(data) => {
                        let edges_count = data.edges.len();
                        view! {
                            <div class="entity-explorer">
                                <div class="nodes-section">
                                    <h4>"Related Entities"</h4>
                                    <div class="nodes-grid">
                                        <For each=move || data.nodes.clone() key=|n| n.id.clone() let:node>
                                            <EntityNodeCard node=node />
                                        </For>
                                    </div>
                                </div>
                                <div class="relationships-section">
                                    <h4>"Relationships"</h4>
                                    <div class="relationships-list">
                                        <For each=move || data.edges.clone() key=|e| format!("{}-{}-{}", e.source_id, e.edge_type, e.target_id) let:rel>
                                            <EntityRelationshipRow relationship=rel />
                                        </For>
                                        {if edges_count == 0 {
                                            view! {
                                                <p class="muted-copy">"No relationships found."</p>
                                            }.into_view()
                                        } else { view! {}.into_view() }}
                                    </div>
                                </div>
                            </div>
                        }.into_view()
                    }
                    Err(msg) => view! {
                        <p class="error-copy">{msg}</p>
                    }.into_view(),
                })}
            </Suspense>
        </SurfaceCard>
    }
}

#[component]
fn EvidenceViewer(entity_type: String, entity_id: String) -> impl IntoView {
    let evidence = create_resource(
        move || (entity_type.clone(), entity_id.clone()),
        |(et, eid)| async move { fetch_source_evidence(Some(&et), Some(&eid)).await },
    );

    view! {
        <SurfaceCard title="Source Evidence" subtitle="Supporting evidence for this entity">
            <Suspense fallback=move || view! {
                <p class="muted-copy">"Loading evidence..."</p>
            }>
                {move || evidence.get().map(|result| match result {
                    Ok(items) => {
                        let items_count = items.len();
                        view! {
                            <div class="evidence-grid">
                                <For each=move || items.clone() key=|e| e.id.clone() let:ev>
                                    <SourceEvidenceCard evidence=ev />
                                </For>
                                {if items_count == 0 {
                                    view! {
                                        <div class="empty-state">
                                            <p>"No evidence available."</p>
                                        </div>
                                    }.into_view()
                                } else { view! {}.into_view() }}
                            </div>
                        }.into_view()
                    }
                    Err(msg) => view! {
                        <p class="error-copy">{msg}</p>
                    }.into_view(),
                })}
            </Suspense>
        </SurfaceCard>
    }
}

// ────────────────────────────────────────────
// Main Analyst Investigation Page
// ────────────────────────────────────────────

#[component]
pub fn AnalystPage() -> impl IntoView {
    let workspaces = create_resource(|| (), |_| async { fetch_workspaces().await });
    let (selected_workspace, set_selected_workspace) = create_signal::<Option<String>>(None);

    let workspaces_view = move || {
        workspaces.get().map(|result| match result {
            Ok(items) => {
                let items_len = items.len();
                view! {
                    <div class="workspaces-list">
                        <For each=move || items.clone() key=|w| w.id.clone() let:workspace>
                            <div class="workspace-item" on:click={
                                let ws_id = workspace.id.clone();
                                move |_| {
                                    set_selected_workspace.set(Some(ws_id.clone()));
                                }
                            }>
                                <WorkspaceCard workspace=workspace />
                            </div>
                        </For>
                        {if items_len == 0 {
                            view! {
                                <div class="empty-state">
                                    <p>"No active workspaces."</p>
                                    <button class="apex-btn apex-btn-primary">"Create New"</button>
                                </div>
                            }.into_view()
                        } else {
                            view! {}.into_view()
                        }}
                    </div>
                }
                .into_view()
            }
            Err(msg) => view! {
                <p class="error-copy">{msg}</p>
            }
            .into_view(),
        })
    };

    let workspace_detail_view = move || {
        selected_workspace.get().map(|id| {
            let workspace_resource = create_resource(
                move || id.clone(),
                |wid| async move { fetch_workspace(&wid).await },
            );
            let detail_inner = move || {
                workspace_resource.get().map(|result| match result {
                    Ok(workspace) => {
                        let ws_name = workspace.name.clone();
                        let ws_desc = workspace.description.clone().unwrap_or_default();
                        let ws_type = workspace.workspace_type.clone();
                        let ws_status = workspace.status.clone();
                        let ws_owner = workspace.owner_id.clone();
                        let ws_id = workspace.id.clone();
                        let findings = workspace.findings.clone();
                        let conclusions = workspace.conclusions.clone();
                        view! {
                            <section class="surface-card">
                                <div class="surface-card-header">
                                    <h2 class="surface-card-title">{ws_name}</h2>
                                    {ws_desc.is_empty().then_some(()).map(|_| view! { <p class="surface-card-subtitle">{ws_desc}</p> })}
                                </div>
                                <div class="surface-card-body">
                                    <div class="workspace-details-content">
                                        <div class="details-meta">
                                            <span class="meta-item">"Type: " {ws_type}</span>
                                            <span class="meta-item">"Status: " {ws_status}</span>
                                            <span class="meta-item">"Owner: " {ws_owner}</span>
                                        </div>
                                        {findings.into_iter().map(|f| view! {
                                            <div class="findings-section">
                                                <h4>"Findings"</h4>
                                                <p>{f}</p>
                                            </div>
                                        }).collect_view()}
                                        {conclusions.into_iter().map(|c| view! {
                                            <div class="conclusions-section">
                                                <h4>"Conclusions"</h4>
                                                <p>{c}</p>
                                            </div>
                                        }).collect_view()}
                                    </div>
                                </div>
                            </section>
                            <CollaborationPanel workspace_id=ws_id.clone() />
                            <EntityExplorer
                                entity_id="selected".to_string()
                                entity_type="workspace".to_string()
                            />
                        }.into_view()
                    }
                    Err(msg) => view! {
                        <SurfaceCard title="Error" subtitle="Failed to load workspace">
                            <p class="error-copy">{msg}</p>
                        </SurfaceCard>
                    }.into_view(),
                })
            };
            view! {
                <Suspense fallback=move || view! {
                    <SurfaceCard title="Workspace Details" subtitle="Loading...">
                        <p class="muted-copy">"Loading workspace details..."</p>
                    </SurfaceCard>
                }>
                    {detail_inner}
                </Suspense>
            }.into_view()
        }).unwrap_or_else(|| {
            view! {
                <SurfaceCard title="Select a Workspace" subtitle="Choose a workspace from the left panel">
                    <div class="empty-state">
                        <p>"Select an investigation workspace to view details, collaborate with your team, and explore entity relationships."</p>
                    </div>
                </SurfaceCard>
            }.into_view()
        })
    };

    view! {
        <div class="page analyst-page">
            <PageHeader
                eyebrow="Investigation Tools"
                title="Analyst View"
                subtitle="Investigation workspace, entity relationships, and collaboration tools."
            />
            <div class="analyst-layout">
                <aside class="workspaces-panel">
                    <SurfaceCard title="Investigation Workspaces" subtitle="Active and recent investigations">
                        <Suspense fallback=move || view! {
                            <p class="muted-copy">"Loading workspaces..."</p>
                        }>
                            {workspaces_view}
                        </Suspense>
                    </SurfaceCard>
                </aside>
                <main class="workspace-details">
                    {workspace_detail_view}
                </main>
            </div>
        </div>
    }
}
