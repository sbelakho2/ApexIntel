//! Collaboration Components - Investigation sharing, annotations, team assignments, activity feed.
//!
//! Phase 4.3: User Experience Enhancement

use leptos::*;
use serde_json::Value;

use crate::api_config::api_url;

// ────────────────────────────────────────────
// Types
// ────────────────────────────────────────────

#[derive(Debug, Clone, serde::Deserialize)]
pub struct Annotation {
    pub id: String,
    pub user_id: String,
    pub entity_type: String,
    pub entity_id: String,
    pub body: String,
    pub tags: Vec<String>,
    pub visibility: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct TeamAssignment {
    pub id: String,
    pub team_id: String,
    pub team_name: String,
    pub entity_type: String,
    pub entity_id: String,
    pub assigned_by: String,
    pub assigned_to: String,
    pub role: String,
    pub notes: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct InvestigationShare {
    pub id: String,
    pub workspace_id: String,
    pub shared_by: String,
    pub shared_with: String,
    pub share_type: String,
    pub access_level: String,
    pub message: Option<String>,
    pub expires_at: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ShareRequest {
    pub shared_with: String,
    pub share_type: String,
    pub access_level: String,
    pub message: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct AnnotationRequest {
    pub entity_type: String,
    pub entity_id: String,
    pub body: String,
    pub tags: Vec<String>,
    pub visibility: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct TeamAssignmentRequest {
    pub team_id: String,
    pub team_name: String,
    pub assigned_to: String,
    pub role: String,
    pub notes: Option<String>,
}

// ────────────────────────────────────────────
// Components
// ────────────────────────────────────────────

#[component]
pub fn AnnotationEditor(
    entity_type: String,
    entity_id: String,
    on_save: Callback<Annotation>,
) -> impl IntoView {
    let (body, set_body) = create_signal(String::new());
    let (tags_input, set_tags_input) = create_signal(String::new());
    let (visibility, set_visibility) = create_signal("team".to_string());
    let (is_submitting, set_is_submitting) = create_signal(false);
    let (error, set_error) = create_signal(Option::<String>::None);

    let handle_submit = move |_| {
        if body.get().trim().len() < 3 {
            set_error(Some("Annotation must be at least 3 characters".to_string()));
            return;
        }

        set_is_submitting.set(true);
        set_error.set(None);

        spawn_local(async move {
            let tags: Vec<String> = tags_input
                .get()
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();

            let req = AnnotationRequest {
                entity_type: entity_type.clone(),
                entity_id: entity_id.clone(),
                body: body.get(),
                tags,
                visibility: visibility.get(),
            };

            #[cfg(target_arch = "wasm32")]
            {
                use gloo_net::http::Request;
                use web_sys::RequestCredentials;
                let response = match Request::post(&api_url("/api/annotations"))
                    .credentials(RequestCredentials::SameOrigin)
                    .json(&req) {
                        Ok(r) => r.send().await,
                        Err(e) => { set_error(Some(format!("Serialization error: {}", e))); set_is_submitting.set(false); return; }
                    };
                match response {
                    Ok(resp) => {
                        let status = resp.status();
                        match resp.text().await {
                            Ok(body) => {
                                if status >= 400 {
                                    set_error(Some(format!("Failed to save: {}", status)));
                                } else {
                                    match serde_json::from_str::<serde_json::Value>(&body) {
                                        Ok(json) => {
                                            if let Some(data) = json.get("data") {
                                                if let Ok(annotation) = serde_json::from_value::<Annotation>(data.clone()) {
                                                    on_save.call(annotation);
                                                    set_body.set(String::new());
                                                    set_tags_input.set(String::new());
                                                }
                                            }
                                        }
                                        Err(e) => set_error(Some(format!("Failed to parse response: {}", e))),
                                    }
                                }
                            }
                            Err(e) => set_error(Some(format!("Failed to read response: {}", e))),
                        }
                    }
                    Err(e) => set_error(Some(format!("Request failed: {}", e))),
                }
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                set_error(Some("WASM mutations are only available in the browser runtime".to_string()));
            }

            set_is_submitting.set(false);
        });
    };

    view! {
        <div class="annotation-editor">
            <div class="editor-header">
                <h4>"Add Annotation"</h4>
            </div>
            
            <textarea
                class="apex-textarea"
                placeholder="Write your annotation..."
                value=body
                on:input=move |e| set_body.set(event_target_value(&e))
                rows="4"
            />

            <div class="editor-row">
                <div class="editor-field">
                    <label>"Tags (comma-separated)"</label>
                    <input
                        type="text"
                        class="apex-input"
                        placeholder="e.g., important, follow-up, urgent"
                        value=tags_input
                        on:input=move |e| set_tags_input.set(event_target_value(&e))
                    />
                </div>
                <div class="editor-field">
                    <label>"Visibility"</label>
                    <select
                        class="apex-select"
                        value=visibility
                        on:change=move |e| set_visibility.set(event_target_value(&e))
                    >
                        <option value="private">"Private"</option>
                        <option value="team">"Team"</option>
                        <option value="public">"Public"</option>
                    </select>
                </div>
            </div>

            {error.get().map(|err| view! {
                <div class="editor-error">{err}</div>
            })}

            <div class="editor-actions">
                <button
                    class="apex-btn apex-btn-primary"
                    disabled=is_submitting()
                    on:click=handle_submit
                >
                    {move || if is_submitting() { "Saving..." } else { "Save Annotation" }}
                </button>
            </div>
        </div>
    }
}

#[component]
pub fn AnnotationList(
    annotations: Vec<Annotation>,
    on_delete: Callback<String>,
) -> impl IntoView {
    view! {
        <div class="annotation-list">
            <For each=move || annotations.clone() key=|a| a.id.clone() let:annotation>
                <article class="annotation-item">
                    <div class="annotation-header">
                        <span class="annotation-author">{annotation.user_id}</span>
                        <span class={format!("visibility-badge {}", annotation.visibility)}>
                            {annotation.visibility}
                        </span>
                        <span class="annotation-time">{annotation.created_at}</span>
                    </div>
                    <p class="annotation-body">{annotation.body}</p>
                    <div class="annotation-tags">
                        <For each=move || annotation.tags.clone() key=|t| t.clone() let:tag>
                            <span class="tag">{tag}</span>
                        </For>
                    </div>
                    <div class="annotation-actions">
                        <button
                            class="apex-btn-icon"
                            title="Delete annotation"
                            on:click=move |_| on_delete.call(annotation.id.clone())
                        >
                            "🗑️"
                        </button>
                    </div>
                </article>
            </For>
        </div>
    }
}

#[component]
pub fn TeamAssignmentPanel(
    entity_type: String,
    entity_id: String,
    assignments: Vec<TeamAssignment>,
    on_assign: Callback<TeamAssignment>,
    on_remove: Callback<String>,
) -> impl IntoView {
    let (show_form, set_show_form) = create_signal(false);
    let (team_id, set_team_id) = create_signal(String::new());
    let (team_name, set_team_name) = create_signal(String::new());
    let (assigned_to, set_assigned_to) = create_signal(String::new());
    let (role, set_role) = create_signal("contributor".to_string());
    let (notes, set_notes) = create_signal(String::new());
    let (is_submitting, set_is_submitting) = create_signal(false);

    let handle_submit = move |_| {
        if team_id.get().trim().is_empty() || assigned_to.get().trim().is_empty() {
            return;
        }

        set_is_submitting.set(true);

        spawn_local(async move {
            let req = TeamAssignmentRequest {
                team_id: team_id.get(),
                team_name: team_name.get(),
                assigned_to: assigned_to.get(),
                role: role.get(),
                notes: if notes.get().is_empty() { None } else { Some(notes.get()) },
            };

            #[cfg(target_arch = "wasm32")]
            {
                use gloo_net::http::Request;
                use web_sys::RequestCredentials;
                let response = match Request::post(&api_url("/api/team-assignments"))
                    .credentials(RequestCredentials::SameOrigin)
                    .json(&req) {
                        Ok(r) => r.send().await,
                        Err(_) => { set_is_submitting.set(false); return; }
                    };
                match response {
                    Ok(resp) => {
                        let status = resp.status();
                        if status < 400 {
                            if let Ok(body) = resp.text().await {
                                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&body) {
                                    if let Some(data) = json.get("data") {
                                        if let Ok(assignment) = serde_json::from_value::<TeamAssignment>(data.clone()) {
                                            on_assign.call(assignment);
                                            set_team_id.set(String::new());
                                            set_team_name.set(String::new());
                                            set_assigned_to.set(String::new());
                                            set_notes.set(String::new());
                                            set_show_form.set(false);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Err(_) => {}
                }
            }

            set_is_submitting.set(false);
        });
    };

    view! {
        <div class="team-assignment-panel">
            <div class="panel-header">
                <h4>"Team Assignments"</h4>
                <button
                    class="apex-btn apex-btn-secondary"
                    on:click=move |_| set_show_form.update(|v| *v = !*v)
                >
                    {move || if show_form() { "Cancel" } else { "Assign Team Member" }}
                </button>
            </div>

            {move || if show_form() {
                view! {
                    <div class="assignment-form">
                        <div class="form-row">
                            <div class="form-field">
                                <label>"Team ID"</label>
                                <input
                                    type="text"
                                    class="apex-input"
                                    value=team_id
                                    on:input=move |e| set_team_id.set(event_target_value(&e))
                                    placeholder="team-risk-analysis"
                                />
                            </div>
                            <div class="form-field">
                                <label>"Team Name"</label>
                                <input
                                    type="text"
                                    class="apex-input"
                                    value=team_name
                                    on:input=move |e| set_team_name.set(event_target_value(&e))
                                    placeholder="Risk Analysis Team"
                                />
                            </div>
                        </div>
                        <div class="form-row">
                            <div class="form-field">
                                <label>"Assign To"</label>
                                <input
                                    type="text"
                                    class="apex-input"
                                    value=assigned_to
                                    on:input=move |e| set_assigned_to.set(event_target_value(&e))
                                    placeholder="user@company.com"
                                />
                            </div>
                            <div class="form-field">
                                <label>"Role"</label>
                                <select
                                    class="apex-select"
                                    value=role
                                    on:change=move |e| set_role.set(event_target_value(&e))
                                >
                                    <option value="viewer">"Viewer"</option>
                                    <option value="contributor">"Contributor"</option>
                                    <option value="lead">"Lead"</option>
                                    <option value="admin">"Admin"</option>
                                </select>
                            </div>
                        </div>
                        <div class="form-field">
                            <label>"Notes"</label>
                            <textarea
                                class="apex-textarea"
                                value=notes
                                on:input=move |e| set_notes.set(event_target_value(&e))
                                placeholder="Optional notes..."
                                rows="2"
                            />
                        </div>
                        <button
                            class="apex-btn apex-btn-primary"
                            disabled=is_submitting()
                            on:click=handle_submit
                        >
                            {move || if is_submitting() { "Assigning..." } else { "Assign" }}
                        </button>
                    </div>
                }.into_view()
            } else { view! {}.into_view() }}

            <div class="assignments-list">
                <For each=move || assignments.clone() key=|a| a.id.clone() let:assignment>
                    <div class="assignment-item">
                        <div class="assignment-info">
                            <span class="team-name">{assignment.team_name}</span>
                            <span class="assigned-to">"→ {assignment.assigned_to}"</span>
                            <span class="role-badge">{assignment.role}</span>
                        </div>
                        <button
                            class="apex-btn-icon"
                            title="Remove assignment"
                            on:click=move |_| on_remove.call(assignment.id.clone())
                        >
                            "✕"
                        </button>
                    </div>
                </For>
                {if assignments.is_empty() {
                    view! {
                        <div class="empty-state">
                            <p>"No team assignments yet."</p>
                        </div>
                    }.into_view()
                } else { view! {}.into_view() }}
            </div>
        </div>
    }
}

#[component]
pub fn InvestigationSharePanel(
    workspace_id: String,
    shares: Vec<InvestigationShare>,
    on_share: Callback<InvestigationShare>,
    on_revoke: Callback<String>,
) -> impl IntoView {
    let (show_form, set_show_form) = create_signal(false);
    let (shared_with, set_shared_with) = create_signal(String::new());
    let (share_type, set_share_type) = create_signal("view".to_string());
    let (access_level, set_access_level) = create_signal("read".to_string());
    let (message, set_message) = create_signal(String::new());
    let (is_submitting, set_is_submitting) = create_signal(false);

    let handle_submit = move |_| {
        if shared_with.get().trim().is_empty() {
            return;
        }

        set_is_submitting.set(true);

        spawn_local(async move {
            let req = ShareRequest {
                shared_with: shared_with.get(),
                share_type: share_type.get(),
                access_level: access_level.get(),
                message: if message.get().is_empty() { None } else { Some(message.get()) },
            };

            #[cfg(target_arch = "wasm32")]
            {
                use gloo_net::http::Request;
                use web_sys::RequestCredentials;
                let response = match Request::post(&api_url(&format!("/api/workspaces/{}/shares", workspace_id)))
                    .credentials(RequestCredentials::SameOrigin)
                    .json(&req) {
                        Ok(r) => r.send().await,
                        Err(_) => { set_is_submitting.set(false); return; }
                    };
                match response {
                    Ok(resp) => {
                        let status = resp.status();
                        if status < 400 {
                            if let Ok(body) = resp.text().await {
                                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&body) {
                                    if let Some(data) = json.get("data") {
                                        if let Ok(share) = serde_json::from_value::<InvestigationShare>(data.clone()) {
                                            on_share.call(share);
                                            set_shared_with.set(String::new());
                                            set_message.set(String::new());
                                            set_show_form.set(false);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Err(_) => {}
                }
            }

            set_is_submitting.set(false);
        });
    };

    view! {
        <div class="share-panel">
            <div class="panel-header">
                <h4>"Investigation Sharing"</h4>
                <button
                    class="apex-btn apex-btn-secondary"
                    on:click=move |_| set_show_form.update(|v| *v = !*v)
                >
                    {move || if show_form() { "Cancel" } else { "Share Investigation" }}
                </button>
            </div>

            {move || if show_form() {
                view! {
                    <div class="share-form">
                        <div class="form-field">
                            <label>"Share With (email or user ID)"</label>
                            <input
                                type="text"
                                class="apex-input"
                                value=shared_with
                                on:input=move |e| set_shared_with.set(event_target_value(&e))
                                placeholder="analyst@company.com"
                            />
                        </div>
                        <div class="form-row">
                            <div class="form-field">
                                <label>"Share Type"</label>
                                <select
                                    class="apex-select"
                                    value=share_type
                                    on:change=move |e| set_share_type.set(event_target_value(&e))
                                >
                                    <option value="view">"View Only"</option>
                                    <option value="edit">"Edit"</option>
                                    <option value="collaborate">"Collaborate"</option>
                                </select>
                            </div>
                            <div class="form-field">
                                <label>"Access Level"</label>
                                <select
                                    class="apex-select"
                                    value=access_level
                                    on:change=move |e| set_access_level.set(event_target_value(&e))
                                >
                                    <option value="read">"Read"</option>
                                    <option value="read_write">"Read/Write"</option>
                                    <option value="admin">"Admin"</option>
                                </select>
                            </div>
                        </div>
                        <div class="form-field">
                            <label>"Message (optional)"</label>
                            <textarea
                                class="apex-textarea"
                                value=message
                                on:input=move |e| set_message.set(event_target_value(&e))
                                placeholder="Add a note to the recipient..."
                                rows="2"
                            />
                        </div>
                        <button
                            class="apex-btn apex-btn-primary"
                            disabled=is_submitting()
                            on:click=handle_submit
                        >
                            {move || if is_submitting() { "Sharing..." } else { "Share" }}
                        </button>
                    </div>
                }.into_view()
            } else { view! {}.into_view() }}

            <div class="shares-list">
                <For each=move || shares.clone() key=|s| s.id.clone() let:share>
                    <div class="share-item">
                        <div class="share-info">
                            <span class="shared-with">{share.shared_with}</span>
                            <span class="share-type-badge">{share.share_type}</span>
                            <span class="access-level">{share.access_level}</span>
                            {share.expires_at.map(|exp| view! {
                                <span class="expires">"Expires: {exp}"</span>
                            })}
                        </div>
                        <div class="share-actions">
                            <span class="shared-by">"by {share.shared_by}"</span>
                            <button
                                class="apex-btn-icon"
                                title="Revoke access"
                                on:click=move |_| on_revoke.call(share.id.clone())
                            >
                                "✕"
                            </button>
                        </div>
                    </div>
                </For>
                {if shares.is_empty() {
                    view! {
                        <div class="empty-state">
                            <p>"Not shared with anyone yet."</p>
                        </div>
                    }.into_view()
                } else { view! {}.into_view() }}
            </div>
        </div>
    }
}

#[component]
pub fn ActivityFeedWidget(
    activities: Vec<super::super::routes::analyst::ActivityEntry>,
    max_items: usize,
) -> impl IntoView {
    let display_activities = activities.into_iter().take(max_items).collect::<Vec<_>>();

    view! {
        <div class="activity-feed-widget">
            <div class="feed-header">
                <h4>"Recent Activity"</h4>
            </div>
            <div class="feed-items">
                <For each=move || display_activities.clone() key=|a| a.id.clone() let:activity>
                    <div class="feed-item">
                        <span class="activity-icon">
                            {match activity.action_type.as_str() {
                                "create" => "➕",
                                "update" => "✏️",
                                "delete" => "🗑️",
                                "share" => "📤",
                                "comment" => "💬",
                                "assign" => "👤",
                                _ => "📝",
                            }}
                        </span>
                        <div class="activity-content">
                            <span class="actor">{activity.actor_name}</span>
                            <span class="action">{activity.action_type}</span>
                            {activity.entity_name.map(|name| view! {
                                <span class="entity">{name}</span>
                            })}
                            <span class="time">{activity.created_at}</span>
                        </div>
                    </div>
                </For>
            </div>
        </div>
    }
}
