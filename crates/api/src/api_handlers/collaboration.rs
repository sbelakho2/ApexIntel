// Collaboration API Handlers
// Phase 4.3: User Experience Enhancement - User Experience Features

use apex_api::destructive_actions::ApiAuthContext;
use apex_api::responses::{success, ApiError, ApiResponse};
use apex_api::routes::collaboration::{
    ActivityEntry, ActivityFeedQuery, AddEvidenceRequest, AddSupplierRiskRequest,
    AddToQueueRequest, AssignUserRequest, CreateOpportunityRequest, CreatePipelineOpportunityRequest,
    CreateTeamAssignmentRequest, CreateThreatRequest, CreateWorkspaceRequest, CriticalThreat,
    InvestigationShare, InvestigationWorkspace, PipelineOpportunity, PriorityQueueItem,
    RecordActivityRequest, ShareWorkspaceRequest, SourceEvidence, StrategicOpportunity,
    SupplierRiskEntry, TeamAssignment, UpdatePipelineStageRequest, UpdateQueueItemRequest,
    UpdateWorkspaceRequest, WorkspaceAssignment,
};
use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use chrono::Utc;
use serde::Deserialize;
use serde_json::Value;

// ──────────────────────────────────────────────────────────────────────────────
// Validation Helpers
// ──────────────────────────────────────────────────────────────────────────────

pub fn validate_workspace_request(req: &CreateWorkspaceRequest) -> Result<(), ApiError> {
    if req.name.trim().len() < 3 {
        return Err(ApiError::validation("name", "must be at least 3 characters"));
    }
    if req.name.trim().len() > 255 {
        return Err(ApiError::validation("name", "must not exceed 255 characters"));
    }
    if !["ad-hoc", "structured", "incident", "ongoing"].contains(&req.workspace_type.as_str()) {
        return Err(ApiError::validation("workspace_type",
            "must be one of: ad-hoc, structured, incident, ongoing"));
    }
    if !["private", "team", "organization", "public"].contains(&req.visibility.as_str()) {
        return Err(ApiError::validation("visibility",
            "must be one of: private, team, organization, public"));
    }
    Ok(())
}

pub fn validate_priority(value: i32) -> Result<(), ApiError> {
    if value < 1 || value > 100 {
        return Err(ApiError::validation("priority", "must be between 1 and 100"));
    }
    Ok(())
}

pub fn validate_confidence(value: f64) -> Result<(), ApiError> {
    if value < 0.0 || value > 1.0 {
        return Err(ApiError::validation("confidence", "must be between 0.0 and 1.0"));
    }
    Ok(())
}

pub fn validate_severity(value: &str) -> Result<(), ApiError> {
    let lower = value.to_lowercase();
    if !["low", "medium", "high", "critical"].contains(&lower.as_str()) {
        return Err(ApiError::validation("severity",
            "must be one of: low, medium, high, critical"));
    }
    Ok(())
}

pub fn validate_stage(value: &str) -> Result<(), ApiError> {
    if !["discovery", "qualification", "proposal", "negotiation", "closed_won", "closed_lost"]
        .contains(&value)
    {
        return Err(ApiError::validation("stage",
            "must be one of: discovery, qualification, proposal, negotiation, closed_won, closed_lost"));
    }
    Ok(())
}

fn normalize_optional_text(text: Option<String>) -> Option<String> {
    text.map(|t| {
        let trimmed = t.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    }).flatten()
}

// ──────────────────────────────────────────────────────────────────────────────
// Executive Dashboard Endpoints
// ──────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ExecutiveQuery {
    pub include_threats: Option<bool>,
    pub include_opportunities: Option<bool>,
    pub priority_threshold: Option<f64>,
    pub region_filter: Option<String>,
}

/// GET /api/v1/collaboration/executive-summary
/// Returns the executive dashboard summary with top opportunities, threats, and actions
pub async fn get_executive_summary(
    State(state): State<crate::AppState>,
    Query(query): Query<ExecutiveQuery>,
) -> Result<Json<ApiResponse<Value>>, ApiError> {
    let include_threats = query.include_threats.unwrap_or(true);
    let include_opportunities = query.include_opportunities.unwrap_or(true);
    let priority_threshold = query.priority_threshold.unwrap_or(0.7);

    // Fetch opportunities (placeholder — no store method exists yet)
    let top_opportunities: Vec<StrategicOpportunity> = vec![];
    let critical_threats: Vec<CriticalThreat> = vec![];

    let total_opportunities = top_opportunities.len();
    let total_threats = critical_threats.len();
    let high_priority_count = 0;

    let all_confidences: Vec<f64> = top_opportunities.iter()
        .map(|o| o.confidence)
        .chain(critical_threats.iter().map(|t| t.confidence))
        .collect();

    let average_confidence = if all_confidences.is_empty() {
        0.0
    } else {
        all_confidences.iter().sum::<f64>() / all_confidences.len() as f64
    };

    let mut regions_affected: Vec<String> = top_opportunities.iter()
        .filter_map(|o| o.region.clone())
        .chain(critical_threats.iter().filter_map(|t| t.region.clone()))
        .collect();
    regions_affected.sort();
    regions_affected.dedup();

    let summary = serde_json::json!({
        "total_opportunities": total_opportunities,
        "total_threats": total_threats,
        "high_priority_count": high_priority_count,
        "regions_affected": regions_affected,
        "average_confidence": average_confidence,
        "top_opportunities": top_opportunities,
        "critical_threats": critical_threats,
    });

    Ok(Json(success(summary)))
}

/// GET /api/v1/collaboration/opportunities
/// List strategic opportunities
pub async fn list_opportunities(
    State(_state): State<crate::AppState>,
    Query(_params): Query<ListOpportunitiesQuery>,
) -> Result<Json<ApiResponse<Vec<StrategicOpportunity>>>, ApiError> {
    // Placeholder — no store method exists yet
    let opportunities: Vec<StrategicOpportunity> = vec![];

    Ok(Json(success(opportunities)))
}

#[derive(Debug, Deserialize)]
pub struct ListOpportunitiesQuery {
    pub priority_threshold: Option<f64>,
    pub region: Option<String>,
    pub limit: Option<u32>,
    pub include_closed: Option<bool>,
}

/// POST /api/v1/collaboration/opportunities
/// Create a new strategic opportunity
pub async fn create_opportunity(
    State(_state): State<crate::AppState>,
    Json(req): Json<CreateOpportunityRequest>,
) -> Result<Json<ApiResponse<StrategicOpportunity>>, ApiError> {
    validate_confidence(req.confidence)?;

    if req.priority_score < 0.0 || req.priority_score > 1.0 {
        return Err(ApiError::validation("priority_score", "must be between 0.0 and 1.0"));
    }

    let now = Utc::now();
    let opportunity = StrategicOpportunity {
        id: uuid::Uuid::new_v4().to_string(),
        title: req.title.trim().to_string(),
        description: normalize_optional_text(req.description),
        opportunity_type: req.opportunity_type,
        priority_score: req.priority_score,
        confidence: req.confidence,
        entity_id: req.entity_id,
        entity_type: req.entity_type,
        region: req.region,
        estimated_value: req.estimated_value,
        recommended_actions: req.recommended_actions,
        owner_id: req.owner_id,
        status: "active".to_string(),
        due_date: req.due_date,
        metadata: serde_json::json!({}),
        created_at: now,
        updated_at: now,
    };

    Ok(Json(success(opportunity)))
}

/// GET /api/v1/collaboration/threats
/// List critical threats
pub async fn list_threats(
    State(_state): State<crate::AppState>,
    Query(_params): Query<ListThreatsQuery>,
) -> Result<Json<ApiResponse<Vec<CriticalThreat>>>, ApiError> {
    // Placeholder — no store method exists yet
    let threats: Vec<CriticalThreat> = vec![];

    Ok(Json(success(threats)))
}

#[derive(Debug, Deserialize)]
pub struct ListThreatsQuery {
    pub impact_threshold: Option<f64>,
    pub region: Option<String>,
    pub severity: Option<String>,
    pub limit: Option<u32>,
}

/// POST /api/v1/collaboration/threats
/// Create a new critical threat
pub async fn create_threat(
    State(_state): State<crate::AppState>,
    Json(req): Json<CreateThreatRequest>,
) -> Result<Json<ApiResponse<CriticalThreat>>, ApiError> {
    validate_confidence(req.confidence)?;
    validate_severity(&req.severity)?;

    if req.impact_score < 0.0 || req.impact_score > 1.0 {
        return Err(ApiError::validation("impact_score", "must be between 0.0 and 1.0"));
    }

    let now = Utc::now();
    let threat = CriticalThreat {
        id: uuid::Uuid::new_v4().to_string(),
        title: req.title.trim().to_string(),
        description: normalize_optional_text(req.description),
        threat_type: req.threat_type,
        severity: req.severity.to_lowercase(),
        impact_score: req.impact_score,
        confidence: req.confidence,
        entity_id: req.entity_id,
        entity_type: req.entity_type,
        region: req.region,
        mitigation_steps: req.mitigation_steps,
        owner_id: req.owner_id,
        status: "active".to_string(),
        sla_deadline: req.sla_deadline,
        resolved_at: None,
        metadata: serde_json::json!({}),
        created_at: now,
        updated_at: now,
    };

    Ok(Json(success(threat)))
}

/// GET /api/v1/executive/opportunities/:id
/// Get a specific strategic opportunity (placeholder)
pub async fn get_opportunity(
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<StrategicOpportunity>>, ApiError> {
    Err(ApiError::not_found("opportunity", &id))
}

/// PATCH /api/v1/executive/opportunities/:id
/// Update a strategic opportunity (placeholder)
pub async fn update_opportunity(
    Path(id): Path<String>,
    Json(_req): Json<CreateOpportunityRequest>,
) -> Result<Json<ApiResponse<StrategicOpportunity>>, ApiError> {
    Err(ApiError::not_found("opportunity", &id))
}

/// GET /api/v1/executive/threats/:id
/// Get a specific critical threat (placeholder)
pub async fn get_threat(
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<CriticalThreat>>, ApiError> {
    Err(ApiError::not_found("threat", &id))
}

/// PATCH /api/v1/executive/threats/:id
/// Update a critical threat (placeholder)
pub async fn update_threat(
    Path(id): Path<String>,
    Json(_req): Json<CreateThreatRequest>,
) -> Result<Json<ApiResponse<CriticalThreat>>, ApiError> {
    Err(ApiError::not_found("threat", &id))
}

// ──────────────────────────────────────────────────────────────────────────────
// Investigation Workspaces Endpoints
// ──────────────────────────────────────────────────────────────────────────────

/// GET /api/v1/collaboration/workspaces
/// List investigation workspaces
pub async fn list_workspaces(
    State(_state): State<crate::AppState>,
    Extension(auth): Extension<ApiAuthContext>,
    Query(params): Query<ListWorkspacesQuery>,
) -> Result<Json<ApiResponse<Vec<InvestigationWorkspace>>>, ApiError> {
    // Placeholder — no store method exists yet
    let workspaces: Vec<InvestigationWorkspace> = vec![];

    // Filter by visibility based on user role
    let filtered: Vec<InvestigationWorkspace> = workspaces
        .into_iter()
        .filter(|w| {
            if auth.role.can_admin() {
                true
            } else if w.visibility == "public" || w.visibility == "organization" {
                true
            } else if w.visibility == "team" {
                // Check if user is team member
                true
            } else {
                w.owner_id == auth.user_id
            }
        })
        .collect();

    Ok(Json(success(filtered)))
}

#[derive(Debug, Deserialize)]
pub struct ListWorkspacesQuery {
    pub workspace_type: Option<String>,
    pub status: Option<String>,
    pub visibility: Option<String>,
    pub limit: Option<u32>,
}

/// POST /api/v1/collaboration/workspaces
/// Create a new investigation workspace
pub async fn create_workspace(
    State(_state): State<crate::AppState>,
    Extension(auth): Extension<ApiAuthContext>,
    Json(req): Json<CreateWorkspaceRequest>,
) -> Result<Json<ApiResponse<InvestigationWorkspace>>, ApiError> {
    validate_workspace_request(&req)?;

    let now = Utc::now();
    let workspace = InvestigationWorkspace {
        id: uuid::Uuid::new_v4().to_string(),
        name: req.name.trim().to_string(),
        description: normalize_optional_text(req.description),
        workspace_type: req.workspace_type,
        owner_id: auth.user_id.clone(),
        team_id: req.team_id,
        status: "active".to_string(),
        visibility: req.visibility,
        tags: req.tags,
        entity_focus: req.entity_focus,
        findings: None,
        conclusions: None,
        metadata: serde_json::json!({}),
        created_at: now,
        updated_at: now,
        closed_at: None,
    };

    Ok(Json(success(workspace)))
}

/// GET /api/v1/collaboration/workspaces/:id
/// Get a specific workspace
pub async fn get_workspace(
    State(_state): State<crate::AppState>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<InvestigationWorkspace>>, ApiError> {
    // In production, fetch from database
    let workspace = InvestigationWorkspace {
        id,
        name: "Sample Workspace".to_string(),
        description: None,
        workspace_type: "ad-hoc".to_string(),
        owner_id: "user-1".to_string(),
        team_id: None,
        status: "active".to_string(),
        visibility: "team".to_string(),
        tags: vec![],
        entity_focus: serde_json::json!([]),
        findings: None,
        conclusions: None,
        metadata: serde_json::json!({}),
        created_at: Utc::now(),
        updated_at: Utc::now(),
        closed_at: None,
    };

    Ok(Json(success(workspace)))
}

/// PUT /api/v1/collaboration/workspaces/:id
/// Update a workspace
pub async fn update_workspace(
    State(_state): State<crate::AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateWorkspaceRequest>,
) -> Result<Json<ApiResponse<InvestigationWorkspace>>, ApiError> {
    // In production, validate ownership and update in database
    let workspace = InvestigationWorkspace {
        id,
        name: req.name.unwrap_or_else(|| "Updated Workspace".to_string()),
        description: req.description,
        workspace_type: "ad-hoc".to_string(),
        owner_id: "user-1".to_string(),
        team_id: None,
        status: req.status.unwrap_or_else(|| "active".to_string()),
        visibility: "team".to_string(),
        tags: req.tags.unwrap_or_default(),
        entity_focus: req.entity_focus.unwrap_or(serde_json::json!([])),
        findings: req.findings,
        conclusions: req.conclusions,
        metadata: serde_json::json!({}),
        created_at: Utc::now(),
        updated_at: Utc::now(),
        closed_at: None,
    };

    Ok(Json(success(workspace)))
}

/// POST /api/v1/collaboration/workspaces/:id/assign
/// Assign a user to a workspace
pub async fn assign_user_to_workspace(
    State(_state): State<crate::AppState>,
    Path(id): Path<String>,
    Json(req): Json<AssignUserRequest>,
) -> Result<Json<ApiResponse<WorkspaceAssignment>>, ApiError> {
    if req.user_id.trim().is_empty() {
        return Err(ApiError::validation("user_id", "cannot be empty"));
    }
    if !["owner", "lead", "contributor", "viewer", "reviewer"].contains(&req.role.as_str()) {
        return Err(ApiError::validation("role",
            "must be one of: owner, lead, contributor, viewer, reviewer"));
    }

    let assignment = WorkspaceAssignment {
        id: uuid::Uuid::new_v4().to_string(),
        workspace_id: id,
        user_id: req.user_id,
        role: req.role,
        assigned_by: "system".to_string(),
        assigned_at: Utc::now(),
        updated_at: Utc::now(),
    };

    Ok(Json(success(assignment)))
}

/// DELETE /api/v1/workspaces/:id
/// Delete a workspace (placeholder)
pub async fn delete_workspace(
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<Value>>, ApiError> {
    Err(ApiError::not_found("workspace", &id))
}

/// GET /api/v1/workspaces/:id/assignments
/// List workspace assignments (placeholder)
pub async fn list_workspace_assignments(
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<Vec<WorkspaceAssignment>>>, ApiError> {
    Err(ApiError::not_found("workspace", &id))
}

/// DELETE /api/v1/workspaces/:id/assignments/:user_id
/// Remove a user from a workspace (placeholder)
pub async fn remove_user_from_workspace(
    Path((_workspace_id, _user_id)): Path<(String, String)>,
) -> Result<Json<ApiResponse<Value>>, ApiError> {
    Err(ApiError::not_found("assignment", ""))
}

// ──────────────────────────────────────────────────────────────────────────────
// Priority Queue Endpoints
// ──────────────────────────────────────────────────────────────────────────────

/// GET /api/v1/collaboration/queue
/// List priority queue items
pub async fn list_queue_items(
    State(_state): State<crate::AppState>,
    Extension(_auth): Extension<ApiAuthContext>,
    Query(_params): Query<ListQueueQuery>,
) -> Result<Json<ApiResponse<Vec<PriorityQueueItem>>>, ApiError> {
    // Placeholder — no store method exists yet
    let items: Vec<PriorityQueueItem> = vec![];

    Ok(Json(success(items)))
}

#[derive(Debug, Deserialize)]
pub struct ListQueueQuery {
    pub status: Option<String>,
    pub item_type: Option<String>,
    pub limit: Option<u32>,
}

/// POST /api/v1/collaboration/queue
/// Add item to priority queue
pub async fn add_to_queue(
    State(_state): State<crate::AppState>,
    Extension(auth): Extension<ApiAuthContext>,
    Json(req): Json<AddToQueueRequest>,
) -> Result<Json<ApiResponse<PriorityQueueItem>>, ApiError> {
    validate_priority(req.priority)?;

    if req.item_type.trim().is_empty() || req.item_id.trim().is_empty() {
        return Err(ApiError::validation("item_type/item_id", "cannot be empty"));
    }

    let now = Utc::now();
    let item = PriorityQueueItem {
        id: uuid::Uuid::new_v4().to_string(),
        user_id: auth.user_id.clone(),
        queue_date: now.format("%Y-%m-%d").to_string(),
        item_type: req.item_type,
        item_id: req.item_id,
        item_title: req.item_title,
        priority: req.priority,
        status: "pending".to_string(),
        notes: normalize_optional_text(req.notes),
        completed_at: None,
        created_at: now,
        updated_at: now,
    };

    Ok(Json(success(item)))
}

/// PATCH /api/v1/collaboration/queue/:id
/// Update queue item status
pub async fn update_queue_item(
    State(_state): State<crate::AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateQueueItemRequest>,
) -> Result<Json<ApiResponse<PriorityQueueItem>>, ApiError> {
    if let Some(priority) = req.priority {
        validate_priority(priority)?;
    }
    if let Some(ref status) = req.status {
        if !["pending", "in_progress", "completed", "cancelled"].contains(&status.as_str()) {
            return Err(ApiError::validation("status",
                "must be one of: pending, in_progress, completed, cancelled"));
        }
    }

    let item = PriorityQueueItem {
        id,
        user_id: "user-1".to_string(),
        queue_date: Utc::now().format("%Y-%m-%d").to_string(),
        item_type: "warning".to_string(),
        item_id: "item-1".to_string(),
        item_title: "Updated Item".to_string(),
        priority: req.priority.unwrap_or(50),
        status: req.status.clone().unwrap_or_else(|| "pending".to_string()),
        notes: req.notes,
        completed_at: req.status.as_ref().filter(|s| *s == "completed").map(|_| Utc::now()),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    Ok(Json(success(item)))
}

// ──────────────────────────────────────────────────────────────────────────────
// Supplier Risk Endpoints
// ──────────────────────────────────────────────────────────────────────────────

/// GET /api/v1/collaboration/supplier-risks
/// List supplier risk entries
pub async fn list_supplier_risks(
    State(_state): State<crate::AppState>,
    Query(_params): Query<ListSupplierRisksQuery>,
) -> Result<Json<ApiResponse<Vec<SupplierRiskEntry>>>, ApiError> {
    // Placeholder — no store method exists yet
    let risks: Vec<SupplierRiskEntry> = vec![];

    Ok(Json(success(risks)))
}

#[derive(Debug, Deserialize)]
pub struct ListSupplierRisksQuery {
    pub risk_category: Option<String>,
    pub status: Option<String>,
    pub limit: Option<u32>,
}

/// POST /api/v1/collaboration/supplier-risks
/// Add supplier risk entry
pub async fn add_supplier_risk(
    State(_state): State<crate::AppState>,
    Json(req): Json<AddSupplierRiskRequest>,
) -> Result<Json<ApiResponse<SupplierRiskEntry>>, ApiError> {
    if req.risk_score < 0.0 || req.risk_score > 1.0 {
        return Err(ApiError::validation("risk_score", "must be between 0.0 and 1.0"));
    }

    let valid_categories = ["financial", "operational", "compliance", "geopolitical",
        "environmental", "technological", "reputational", "strategic"];
    if !valid_categories.contains(&req.risk_category.as_str()) {
        return Err(ApiError::validation("risk_category", "invalid category"));
    }

    let now = Utc::now();
    let entry = SupplierRiskEntry {
        id: uuid::Uuid::new_v4().to_string(),
        supplier_id: "supplier-1".to_string(),
        risk_category: req.risk_category,
        risk_score: req.risk_score,
        risk_factors: req.risk_factors,
        mitigation: req.mitigation,
        owner_id: req.owner_id,
        status: "active".to_string(),
        last_reviewed: None,
        next_review: None,
        created_at: now,
        updated_at: now,
    };

    Ok(Json(success(entry)))
}

/// PATCH /api/v1/supplier-risk/:id
/// Update supplier risk entry (placeholder)
pub async fn update_supplier_risk(
    Path(id): Path<String>,
    Json(_req): Json<AddSupplierRiskRequest>,
) -> Result<Json<ApiResponse<SupplierRiskEntry>>, ApiError> {
    Err(ApiError::not_found("supplier_risk", &id))
}

// ──────────────────────────────────────────────────────────────────────────────
// Pipeline Endpoints
// ──────────────────────────────────────────────────────────────────────────────

/// GET /api/v1/collaboration/pipeline
/// List pipeline opportunities
pub async fn list_pipeline_opportunities(
    State(_state): State<crate::AppState>,
    Query(_params): Query<ListPipelineQuery>,
) -> Result<Json<ApiResponse<Vec<PipelineOpportunity>>>, ApiError> {
    // Placeholder — no store method exists yet
    let opportunities: Vec<PipelineOpportunity> = vec![];

    Ok(Json(success(opportunities)))
}

#[derive(Debug, Deserialize)]
pub struct ListPipelineQuery {
    pub stage: Option<String>,
    pub owner_id: Option<String>,
    pub limit: Option<u32>,
}

/// POST /api/v1/collaboration/pipeline
/// Create pipeline opportunity
pub async fn create_pipeline_opportunity(
    State(_state): State<crate::AppState>,
    Json(req): Json<CreatePipelineOpportunityRequest>,
) -> Result<Json<ApiResponse<PipelineOpportunity>>, ApiError> {
    validate_stage(&req.stage)?;

    validate_confidence(req.probability)?;

    let opportunity = PipelineOpportunity {
        id: uuid::Uuid::new_v4().to_string(),
        opportunity_id: req.opportunity_id,
        title: req.title,
        stage: req.stage,
        value_estimate: req.value_estimate,
        probability: req.probability,
        owner_id: req.owner_id,
        expected_close: req.expected_close,
        actual_close: None,
        notes: req.notes,
        metadata: serde_json::json!({}),
        created_at: Utc::now(),
        updated_at: Utc::now(),
        closed_at: None,
    };

    Ok(Json(success(opportunity)))
}

/// PATCH /api/v1/collaboration/pipeline/:id/stage
/// Update pipeline stage
pub async fn update_pipeline_stage(
    State(_state): State<crate::AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdatePipelineStageRequest>,
) -> Result<Json<ApiResponse<PipelineOpportunity>>, ApiError> {
    validate_stage(&req.stage)?;

    let opportunity = PipelineOpportunity {
        id,
        opportunity_id: None,
        title: "Updated Opportunity".to_string(),
        stage: req.stage,
        value_estimate: None,
        probability: 0.5,
        owner_id: None,
        expected_close: None,
        actual_close: None,
        notes: req.notes,
        metadata: serde_json::json!({}),
        created_at: Utc::now(),
        updated_at: Utc::now(),
        closed_at: None,
    };

    Ok(Json(success(opportunity)))
}

// ──────────────────────────────────────────────────────────────────────────────
// Activity Feed Endpoints
// ──────────────────────────────────────────────────────────────────────────────

/// GET /api/v1/collaboration/activity
/// Get activity feed
pub async fn get_activity_feed(
    State(_state): State<crate::AppState>,
    Query(_params): Query<ActivityFeedQuery>,
) -> Result<Json<ApiResponse<Vec<ActivityEntry>>>, ApiError> {
    // Placeholder — no store method exists yet
    let entries: Vec<ActivityEntry> = vec![];

    Ok(Json(success(entries)))
}

/// POST /api/v1/collaboration/activity
/// Record new activity
pub async fn record_activity(
    State(_state): State<crate::AppState>,
    Extension(auth): Extension<ApiAuthContext>,
    Json(req): Json<RecordActivityRequest>,
) -> Result<Json<ApiResponse<ActivityEntry>>, ApiError> {
    let valid_actions = ["create", "update", "delete", "share", "assign", "comment",
        "resolve", "reopen", "escalate", "deescalate", "approve", "reject", "merge", "split"];
    if !valid_actions.contains(&req.action_type.as_str()) {
        return Err(ApiError::validation("action_type", "invalid action type"));
    }

    let entry = ActivityEntry {
        id: uuid::Uuid::new_v4().to_string(),
        actor_id: auth.user_id.clone(),
        actor_name: "unknown".to_string(),
        action_type: req.action_type,
        entity_type: req.entity_type,
        entity_id: req.entity_id,
        entity_name: req.entity_name,
        details: req.details,
        workspace_id: req.workspace_id,
        team_id: req.team_id,
        visibility: req.visibility,
        created_at: Utc::now(),
    };

    Ok(Json(success(entry)))
}

// ──────────────────────────────────────────────────────────────────────────────
// Investigation Shares Endpoints
// ──────────────────────────────────────────────────────────────────────────────

/// GET /api/v1/collaboration/workspaces/:id/shares
/// List workspace shares
pub async fn list_workspace_shares(
    State(_state): State<crate::AppState>,
    Path(workspace_id): Path<String>,
) -> Result<Json<ApiResponse<Vec<InvestigationShare>>>, ApiError> {
    // Placeholder — fetch from database in production
    let shares: Vec<InvestigationShare> = vec![];

    Ok(Json(success(shares)))
}

/// POST /api/v1/collaboration/workspaces/:id/shares
/// Share a workspace
pub async fn share_workspace(
    State(_state): State<crate::AppState>,
    Path(workspace_id): Path<String>,
    Json(req): Json<ShareWorkspaceRequest>,
) -> Result<Json<ApiResponse<InvestigationShare>>, ApiError> {
    if req.shared_with.trim().is_empty() {
        return Err(ApiError::validation("shared_with", "cannot be empty"));
    }

    let valid_types = ["view", "collaborate", "embed"];
    if !valid_types.contains(&req.share_type.as_str()) {
        return Err(ApiError::validation("share_type", "invalid share type"));
    }

    let valid_levels = ["read", "read_write", "admin"];
    if !valid_levels.contains(&req.access_level.as_str()) {
        return Err(ApiError::validation("access_level", "invalid access level"));
    }

    let share = InvestigationShare {
        id: uuid::Uuid::new_v4().to_string(),
        workspace_id,
        shared_by: "user-1".to_string(),
        shared_with: req.shared_with,
        share_type: req.share_type,
        access_level: req.access_level,
        message: req.message,
        expires_at: req.expires_at,
        created_at: Utc::now(),
    };

    Ok(Json(success(share)))
}

// ──────────────────────────────────────────────────────────────────────────────
// Source Evidence Endpoints
// ──────────────────────────────────────────────────────────────────────────────

/// GET /api/v1/collaboration/evidence
/// Get source evidence for an entity
pub async fn get_evidence(
    State(_state): State<crate::AppState>,
    Query(_params): Query<GetEvidenceQuery>,
) -> Result<Json<ApiResponse<Vec<SourceEvidence>>>, ApiError> {
    // Placeholder — no store method exists yet
    let evidence: Vec<SourceEvidence> = vec![];

    Ok(Json(success(evidence)))
}

#[derive(Debug, Deserialize)]
pub struct GetEvidenceQuery {
    pub entity_type: Option<String>,
    pub entity_id: Option<String>,
    pub evidence_type: Option<String>,
    pub limit: Option<u32>,
}

/// POST /api/v1/collaboration/evidence
/// Add source evidence
pub async fn add_evidence(
    State(_state): State<crate::AppState>,
    Json(req): Json<AddEvidenceRequest>,
) -> Result<Json<ApiResponse<SourceEvidence>>, ApiError> {
    validate_confidence(req.reliability_score)?;

    let valid_types = ["web_content", "document", "financial_report", "news_article",
        "social_media", "regulatory_filing", "patent", "court_record",
        "public_record", "analyst_report"];
    if !valid_types.contains(&req.evidence_type.as_str()) {
        return Err(ApiError::validation("evidence_type", "invalid evidence type"));
    }

    if req.source_url.trim().is_empty() {
        return Err(ApiError::validation("source_url", "cannot be empty"));
    }

    let evidence = SourceEvidence {
        id: uuid::Uuid::new_v4().to_string(),
        entity_type: "company".to_string(),
        entity_id: "entity-1".to_string(),
        evidence_type: req.evidence_type,
        source_url: req.source_url,
        source_domain: req.source_name.as_ref().map(|n| n.split('/').collect::<Vec<_>>()[0].to_string()),
        source_name: req.source_name,
        reliability_score: req.reliability_score,
        content_hash: None,
        excerpt: req.excerpt,
        metadata: req.metadata,
        created_at: Utc::now(),
    };

    Ok(Json(success(evidence)))
}

// ──────────────────────────────────────────────────────────────────────────────
// Team Assignment Endpoints
// ──────────────────────────────────────────────────────────────────────────────

/// GET /api/v1/collaboration/teams
/// List team assignments
pub async fn list_team_assignments(
    State(_state): State<crate::AppState>,
    Query(_params): Query<ListTeamAssignmentsQuery>,
) -> Result<Json<ApiResponse<Vec<TeamAssignment>>>, ApiError> {
    // Placeholder — no store method exists yet
    let assignments: Vec<TeamAssignment> = vec![];

    Ok(Json(success(assignments)))
}

#[derive(Debug, Deserialize)]
pub struct ListTeamAssignmentsQuery {
    pub entity_type: Option<String>,
    pub entity_id: Option<String>,
}

/// POST /api/v1/collaboration/teams
/// Create team assignment
pub async fn create_team_assignment(
    State(_state): State<crate::AppState>,
    Json(req): Json<CreateTeamAssignmentRequest>,
) -> Result<Json<ApiResponse<TeamAssignment>>, ApiError> {
    if req.team_id.trim().is_empty() {
        return Err(ApiError::validation("team_id", "cannot be empty"));
    }
    if req.assigned_to.trim().is_empty() {
        return Err(ApiError::validation("assigned_to", "cannot be empty"));
    }

    let valid_roles = ["lead", "contributor", "reviewer", "observer"];
    if !valid_roles.contains(&req.role.as_str()) {
        return Err(ApiError::validation("role", "invalid role"));
    }

    let assignment = TeamAssignment {
        id: uuid::Uuid::new_v4().to_string(),
        team_id: req.team_id,
        team_name: req.team_name,
        entity_type: "warning".to_string(),
        entity_id: "entity-1".to_string(),
        assigned_by: "user-1".to_string(),
        assigned_to: req.assigned_to,
        role: req.role,
        notes: req.notes,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    Ok(Json(success(assignment)))
}
