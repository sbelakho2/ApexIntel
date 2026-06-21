//! Operational View - Daily priority queue, supplier risk monitor, pipeline tracker, alert management.
//!
//! Phase 4.3: User Experience Enhancement

use leptos::*;

use crate::api_config::api_url;
use crate::components::{
    cards::{PageHeader, SurfaceCard},
};

// ────────────────────────────────────────────
// API Response Types
// ────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PriorityQueueItem {
    pub id: String,
    pub user_id: String,
    pub queue_date: String,
    pub item_type: String,
    pub item_id: String,
    pub item_title: String,
    pub priority: i32,
    pub status: String,
    pub notes: Option<String>,
    pub completed_at: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SupplierRiskEntry {
    pub id: String,
    pub supplier_id: String,
    pub risk_category: String,
    pub risk_score: f64,
    pub risk_factors: Vec<String>,
    pub mitigation: Option<String>,
    pub owner_id: Option<String>,
    pub status: String,
    pub last_reviewed: Option<String>,
    pub next_review: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PipelineOpportunity {
    pub id: String,
    pub opportunity_id: Option<String>,
    pub title: String,
    pub stage: String,
    pub value_estimate: Option<f64>,
    pub probability: f64,
    pub owner_id: Option<String>,
    pub expected_close: Option<String>,
    pub actual_close: Option<String>,
    pub notes: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AlertItem {
    pub id: String,
    pub alert_type: String,
    pub severity: String,
    pub title: String,
    pub message: String,
    pub entity_id: Option<String>,
    pub entity_type: Option<String>,
    pub created_at: String,
    pub acknowledged: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OperationalSummary {
    pub pending_items: u64,
    pub overdue_items: u64,
    pub completed_today: u64,
    pub high_risk_suppliers: u64,
    pub pipeline_value: i64,
    pub pipeline_count: u64,
    pub active_alerts: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AddToQueueRequest {
    pub item_type: String,
    pub item_id: String,
    pub item_title: String,
    pub priority: i32,
    pub notes: Option<String>,
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
pub async fn fetch_priority_queue(user_id: &str) -> Result<Vec<PriorityQueueItem>, String> {
    use gloo_net::http::Request;
    use web_sys::RequestCredentials;
    let url = format!("{}?user_id={}", api_url("/api/queue"), user_id);
    let response = Request::get(&url)
        .credentials(RequestCredentials::SameOrigin)
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;
    let status = response.status();
    let body = response.text().await.map_err(|e| format!("Failed to read body: {}", e))?;
    let envelope: ApiEnvelope<Vec<PriorityQueueItem>> = serde_json::from_str(&body).map_err(|e| format!("Failed to parse: {}", e))?;
    if status >= 400 || !envelope.success {
        return Err(format!("API error (status {})", status));
    }
    envelope.data.ok_or_else(|| "Failed to fetch priority queue".to_string())
}

#[cfg(not(target_arch = "wasm32"))]
pub async fn fetch_priority_queue(_user_id: &str) -> Result<Vec<PriorityQueueItem>, String> {
    Err("WASM data fetching is only available in the browser runtime".to_string())
}

#[cfg(target_arch = "wasm32")]
pub async fn add_to_queue(req: AddToQueueRequest) -> Result<PriorityQueueItem, String> {
    use gloo_net::http::Request;
    use web_sys::RequestCredentials;
    let response = Request::post(&api_url("/api/queue"))
        .credentials(RequestCredentials::SameOrigin)
        .json(&req)
        .map_err(|e| format!("Serialization error: {}", e))?
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;
    let status = response.status();
    let body = response.text().await.map_err(|e| format!("Failed to read body: {}", e))?;
    let envelope: ApiEnvelope<PriorityQueueItem> = serde_json::from_str(&body).map_err(|e| format!("Failed to parse: {}", e))?;
    if status >= 400 || !envelope.success {
        return Err(format!("API error (status {})", status));
    }
    envelope.data.ok_or_else(|| "Failed to add to queue".to_string())
}

#[cfg(not(target_arch = "wasm32"))]
pub async fn add_to_queue(_req: AddToQueueRequest) -> Result<PriorityQueueItem, String> {
    Err("WASM mutations are only available in the browser runtime".to_string())
}

#[cfg(target_arch = "wasm32")]
pub async fn fetch_supplier_risks() -> Result<Vec<SupplierRiskEntry>, String> {
    use gloo_net::http::Request;
    use web_sys::RequestCredentials;
    let response = Request::get(&api_url("/api/supplier-risk"))
        .credentials(RequestCredentials::SameOrigin)
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;
    let status = response.status();
    let body = response.text().await.map_err(|e| format!("Failed to read body: {}", e))?;
    let envelope: ApiEnvelope<Vec<SupplierRiskEntry>> = serde_json::from_str(&body).map_err(|e| format!("Failed to parse: {}", e))?;
    if status >= 400 || !envelope.success {
        return Err(format!("API error (status {})", status));
    }
    envelope.data.ok_or_else(|| "Failed to fetch supplier risks".to_string())
}

#[cfg(not(target_arch = "wasm32"))]
pub async fn fetch_supplier_risks() -> Result<Vec<SupplierRiskEntry>, String> {
    Err("WASM data fetching is only available in the browser runtime".to_string())
}

#[cfg(target_arch = "wasm32")]
pub async fn fetch_pipeline() -> Result<Vec<PipelineOpportunity>, String> {
    use gloo_net::http::Request;
    use web_sys::RequestCredentials;
    let response = Request::get(&api_url("/api/pipeline"))
        .credentials(RequestCredentials::SameOrigin)
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;
    let status = response.status();
    let body = response.text().await.map_err(|e| format!("Failed to read body: {}", e))?;
    let envelope: ApiEnvelope<Vec<PipelineOpportunity>> = serde_json::from_str(&body).map_err(|e| format!("Failed to parse: {}", e))?;
    if status >= 400 || !envelope.success {
        return Err(format!("API error (status {})", status));
    }
    envelope.data.ok_or_else(|| "Failed to fetch pipeline".to_string())
}

#[cfg(not(target_arch = "wasm32"))]
pub async fn fetch_pipeline() -> Result<Vec<PipelineOpportunity>, String> {
    Err("WASM data fetching is only available in the browser runtime".to_string())
}

#[cfg(target_arch = "wasm32")]
pub async fn fetch_alerts() -> Result<Vec<AlertItem>, String> {
    use gloo_net::http::Request;
    use web_sys::RequestCredentials;
    let response = Request::get(&api_url("/api/alerts"))
        .credentials(RequestCredentials::SameOrigin)
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;
    let status = response.status();
    let body = response.text().await.map_err(|e| format!("Failed to read body: {}", e))?;
    let envelope: ApiEnvelope<Vec<AlertItem>> = serde_json::from_str(&body).map_err(|e| format!("Failed to parse: {}", e))?;
    if status >= 400 || !envelope.success {
        return Err(format!("API error (status {})", status));
    }
    envelope.data.ok_or_else(|| "Failed to fetch alerts".to_string())
}

#[cfg(not(target_arch = "wasm32"))]
pub async fn fetch_alerts() -> Result<Vec<AlertItem>, String> {
    Err("WASM data fetching is only available in the browser runtime".to_string())
}

// ────────────────────────────────────────────
// Components
// ────────────────────────────────────────────

#[component]
fn QueueItemCard(item: PriorityQueueItem) -> impl IntoView {
    let priority_class = if item.priority > 80 {
        "priority-critical"
    } else if item.priority > 50 {
        "priority-high"
    } else {
        "priority-normal"
    };

    let status_class = match item.status.as_str() {
        "completed" => "status-completed",
        "in_progress" => "status-progress",
        _ => "status-pending",
    };

    view! {
        <article class="queue-item">
            <div class="queue-item-header">
                <span class={format!("priority-indicator {}", priority_class)}></span>
                <div class="queue-item-content">
                    <h4 class="queue-item-title">{item.item_title}</h4>
                    <div class="queue-item-meta">
                        <span class="item-type">{item.item_type}</span>
                        <span class="priority-badge">{format!("Priority: {}", item.priority)}</span>
                    </div>
                </div>
                <span class={format!("status-badge {}", status_class)}>{item.status}</span>
            </div>
            {item.notes.map(|notes| view! {
                <p class="queue-item-notes">{notes}</p>
            })}
            <div class="queue-item-footer">
                <span class="created-at">"Created: " {item.created_at}</span>
                {item.completed_at.map(|completed| view! {
                    <span class="completed-at">"Completed: " {completed}</span>
                })}
            </div>
        </article>
    }
}

#[component]
fn SupplierRiskCard(entry: SupplierRiskEntry) -> impl IntoView {
    let risk_class = if entry.risk_score > 0.8 {
        "risk-critical"
    } else if entry.risk_score > 0.5 {
        "risk-high"
    } else {
        "risk-medium"
    };

    view! {
        <article class="supplier-risk-card">
            <div class="risk-header">
                <div class="risk-indicator">
                    <span class={format!("risk-score {}", risk_class)}>
                        {format!("{:.0}%", entry.risk_score * 100.0)}
                    </span>
                </div>
                <div class="risk-info">
                    <h4 class="supplier-id">"Supplier: {entry.supplier_id}"</h4>
                    <span class="risk-category">{entry.risk_category}</span>
                </div>
                <span class={format!("status-badge risk-status {}", entry.status)}>
                    {entry.status}
                </span>
            </div>
            <div class="risk-factors">
                <span class="meta-label">"Risk Factors:"</span>
                <ul class="factors-list">
                    {entry.risk_factors.iter().map(|factor| view! {
                        <li>{factor}</li>
                    }).collect_view()}
                </ul>
            </div>
            {entry.mitigation.map(|m| view! {
                <div class="mitigation-section">
                    <span class="meta-label">"Mitigation:"</span>
                    <p class="mitigation-text">{m}</p>
                </div>
            })}
            <div class="risk-footer">
                <span class="meta-item">"Owner: " {entry.owner_id.clone().unwrap_or_else(|| "Unassigned".to_string())}</span>
                {entry.next_review.map(|review| view! {
                    <span class="meta-item">"Next Review: " {review}</span>
                })}
            </div>
        </article>
    }
}

fn pipeline_stage_label(stage: &str) -> &str {
    match stage {
        "discovery" => "Discovery",
        "qualification" => "Qualification",
        "proposal" => "Proposal",
        "negotiation" => "Negotiation",
        "closed_won" => "Closed Won",
        "closed_lost" => "Closed Lost",
        other => other,
    }
}

fn pipeline_stage_view(opportunities: Vec<PipelineOpportunity>) -> impl IntoView {
    let stages: Vec<&str> = vec!["discovery", "qualification", "proposal", "negotiation", "closed_won", "closed_lost"];

    let stage_items: Vec<(&str, Vec<PipelineOpportunity>)> = stages.into_iter()
        .map(|stage| {
            let items: Vec<PipelineOpportunity> = opportunities.iter()
                .filter(|o| o.stage == stage)
                .cloned()
                .collect();
            (stage, items)
        })
        .collect();

    view! {
        <div class="pipeline-board">
            {stage_items.into_iter().map(|(stage, items)| {
                let count = items.len();
                view! {
                    <div class="pipeline-column">
                        <div class="column-header">
                            <span class="column-title">{pipeline_stage_label(stage)}</span>
                            <span class="column-count">{count}</span>
                        </div>
                        <div class="column-items">
                            {items.into_iter().map(|opp| {
                                view! {
                                    <PipelineCard opportunity=opp.clone() />
                                }
                            }).collect_view()}
                        </div>
                    </div>
                }
            }).collect_view()}
        </div>
    }
}

#[component]
fn PipelineCard(opportunity: PipelineOpportunity) -> impl IntoView {
    let value_formatted = opportunity.value_estimate.map(|v| {
        if v >= 1_000_000.0 {
            format!("${:.1}M", v / 1_000_000.0)
        } else if v >= 1_000.0 {
            format!("${:.0}K", v / 1_000.0)
        } else {
            format!("${:.0}", v)
        }
    });

    view! {
        <article class="pipeline-card">
            <h5 class="pipeline-title">{opportunity.title}</h5>
            <div class="pipeline-meta">
                {value_formatted.map(|v| view! {
                    <span class="pipeline-value">{v}</span>
                })}
                <span class="pipeline-probability">
                    {format!("{:.0}%", opportunity.probability * 100.0)}
                </span>
            </div>
            {opportunity.expected_close.clone().map(|date| view! {
                <span class="expected-close">"Close: " {date}</span>
            })}
            {opportunity.notes.map(|notes| view! {
                <p class="pipeline-notes">{notes}</p>
            })}
        </article>
    }
}

#[component]
fn AlertCard(alert: AlertItem) -> impl IntoView {
    let severity_class = format!("alert-severity-{}", alert.severity.to_lowercase());
    let ack_class = if alert.acknowledged { "acknowledged" } else { "unacknowledged" };

    view! {
        <article class={format!("alert-card {}", ack_class)}>
            <div class="alert-header">
                <span class={format!("severity-chip {}", severity_class)}>
                    {alert.severity}
                </span>
                <span class="alert-type">{alert.alert_type}</span>
            </div>
            <h4 class="alert-title">{alert.title}</h4>
            <p class="alert-message">{alert.message}</p>
            <div class="alert-footer">
                <span class="alert-time">{alert.created_at}</span>
                {alert.entity_type.map(|et| view! {
                    <span class="entity-ref">{et}</span>
                })}
            </div>
        </article>
    }
}

#[component]
fn QueueSummary(pending: u64, overdue: u64, completed: u64) -> impl IntoView {
    view! {
        <div class="queue-summary">
            <div class="summary-item">
                <span class="summary-value">{pending}</span>
                <span class="summary-label">"Pending"</span>
            </div>
            <div class="summary-item overdue">
                <span class="summary-value">{overdue}</span>
                <span class="summary-label">"Overdue"</span>
            </div>
            <div class="summary-item completed">
                <span class="summary-value">{completed}</span>
                <span class="summary-label">"Completed Today"</span>
            </div>
        </div>
    }
}

// ────────────────────────────────────────────
// Main Operational View Page
// ────────────────────────────────────────────

#[component]
pub fn OperationalPage() -> impl IntoView {
    let (active_tab, set_active_tab) = create_signal::<String>("queue".to_string());
    
    let queue_items = create_resource(|| (), |_| async { fetch_priority_queue("system").await });
    let supplier_risks = create_resource(|| (), |_| async { fetch_supplier_risks().await });
    let pipeline = create_resource(|| (), |_| async { fetch_pipeline().await });
    let alerts = create_resource(|| (), |_| async { fetch_alerts().await });

    let pending_count = move || {
        match queue_items.get() {
            Some(Ok(items)) => items.iter().filter(|i| i.status == "pending").count() as u64,
            _ => 0,
        }
    };
    
    let overdue_count = move || {
        match queue_items.get() {
            Some(Ok(items)) => items.iter().filter(|i| i.status == "overdue").count() as u64,
            _ => 0,
        }
    };
    
    let completed_count = move || {
        match queue_items.get() {
            Some(Ok(items)) => items.iter().filter(|i| i.status == "completed").count() as u64,
            _ => 0,
        }
    };

    view! {
        <div class="page operational-page">
            <PageHeader
                eyebrow="Daily Operations"
                title="Operational View"
                subtitle="Daily priority queue, supplier risk monitoring, pipeline tracking, and alerts."
            />

            // Tab Navigation — accessible ARIA tab pattern
            <div class="tab-navigation" role="tablist" aria-label="Operational views">
                <button 
                    class={format!("tab-button {}", if active_tab.get() == "queue" { "active" } else { "" })}
                    role="tab"
                    aria-selected={move || (active_tab.get() == "queue").to_string()}
                    aria-controls="panel-queue"
                    id="tab-queue"
                    on:click=move |_| set_active_tab.set("queue".to_string())
                >
                    "Daily Queue"
                </button>
                <button 
                    class={format!("tab-button {}", if active_tab.get() == "suppliers" { "active" } else { "" })}
                    role="tab"
                    aria-selected={move || (active_tab.get() == "suppliers").to_string()}
                    aria-controls="panel-suppliers"
                    id="tab-suppliers"
                    on:click=move |_| set_active_tab.set("suppliers".to_string())
                >
                    "Supplier Risk"
                </button>
                <button 
                    class={format!("tab-button {}", if active_tab.get() == "pipeline" { "active" } else { "" })}
                    role="tab"
                    aria-selected={move || (active_tab.get() == "pipeline").to_string()}
                    aria-controls="panel-pipeline"
                    id="tab-pipeline"
                    on:click=move |_| set_active_tab.set("pipeline".to_string())
                >
                    "Pipeline"
                </button>
                <button 
                    class={format!("tab-button {}", if active_tab.get() == "alerts" { "active" } else { "" })}
                    role="tab"
                    aria-selected={move || (active_tab.get() == "alerts").to_string()}
                    aria-controls="panel-alerts"
                    id="tab-alerts"
                    on:click=move |_| set_active_tab.set("alerts".to_string())
                >
                    "Alerts"
                </button>
            </div>

            // Tab Content
            <div class="tab-content">
                // Daily Priority Queue
                {move || if active_tab.get() == "queue" {
                    view! {
                        <SurfaceCard title="Daily Priority Queue" subtitle="Your prioritized task list for today">
                            <QueueSummary 
                                pending={pending_count()}
                                overdue={overdue_count()}
                                completed={completed_count()}
                            />
                            <Suspense fallback=move || view! {
                                <p class="muted-copy">"Loading queue items..."</p>
                            }>
                                {move || queue_items.get().map(|result| match result {
                                    Ok(items) => {
                                        let items_count = items.len();
                                        view! {
                                            <div class="queue-list">
                                                <For each=move || items.clone() key=|i| i.id.clone() let:item>
                                                    <QueueItemCard item=item />
                                                </For>
                                                {if items_count == 0 {
                                                    view! {
                                                        <div class="empty-state">
                                                            <p>"No items in your queue today."</p>
                                                            <button class="apex-btn apex-btn-primary">"Add Item"</button>
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
                    }.into_view()
                } else { view! {}.into_view() }}

                // Supplier Risk Monitor
                {move || if active_tab.get() == "suppliers" {
                    view! {
                        <SurfaceCard title="Supplier Risk Monitor" subtitle="Active supplier risks requiring attention">
                            <Suspense fallback=move || view! {
                                <p class="muted-copy">"Loading supplier risks..."</p>
                            }>
                                {move || supplier_risks.get().map(|result| match result {
                                    Ok(entries) => {
                                        let entries_count = entries.len();
                                        view! {
                                            <div class="supplier-risks-grid">
                                                <For each=move || entries.clone() key=|e| e.id.clone() let:entry>
                                                    <SupplierRiskCard entry=entry />
                                                </For>
                                                {if entries_count == 0 {
                                                    view! {
                                                        <div class="empty-state">
                                                            <p>"No supplier risks tracked."</p>
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
                    }.into_view()
                } else { view! {}.into_view() }}

                // Pipeline Tracker
                {move || if active_tab.get() == "pipeline" {
                    view! {
                        <SurfaceCard title="Pipeline Opportunity Tracker" subtitle="Track opportunities through stages">
                            <Suspense fallback=move || view! {
                                <p class="muted-copy">"Loading pipeline..."</p>
                            }>
                                {move || pipeline.get().map(|result| match result {
                                    Ok(opportunities) => view! {
                                        {pipeline_stage_view(opportunities)}
                                    }.into_view(),
                                    Err(msg) => view! {
                                        <p class="error-copy">{msg}</p>
                                    }.into_view(),
                                })}
                            </Suspense>
                        </SurfaceCard>
                    }.into_view()
                } else { view! {}.into_view() }}

                // Alert Management
                {move || if active_tab.get() == "alerts" {
                    view! {
                        <SurfaceCard title="Alert Management" subtitle="Active system alerts requiring attention">
                            <Suspense fallback=move || view! {
                                <p class="muted-copy">"Loading alerts..."</p>
                            }>
                                {move || alerts.get().map(|result| match result {
                                    Ok(items) => {
                                        let items_count = items.len();
                                        view! {
                                            <div class="alerts-list">
                                                <For each=move || items.clone() key=|a| a.id.clone() let:alert>
                                                    <AlertCard alert=alert />
                                                </For>
                                                {if items_count == 0 {
                                                    view! {
                                                        <div class="empty-state">
                                                            <p>"No active alerts."</p>
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
                    }.into_view()
                } else { view! {}.into_view() }}
            </div>
        </div>
    }
}
