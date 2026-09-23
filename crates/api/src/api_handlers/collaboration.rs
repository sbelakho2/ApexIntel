#![allow(clippy::unwrap_used, clippy::expect_used)]
// Collaboration API Handlers
// Phase 4.3: User Experience Enhancement - User Experience Features

use apex_api::destructive_actions::ApiAuthContext;
use apex_api::responses::{success, ApiError, ApiResponse};
use apex_api::routes::collaboration::{
    ActivityEntry, ActivityFeedQuery, AddEvidenceRequest, AddSupplierRiskRequest,
    AddToQueueRequest, AssignUserRequest, CreateOpportunityRequest,
    CreatePipelineOpportunityRequest, CreateTeamAssignmentRequest, CreateThreatRequest,
    CreateWorkspaceRequest, CriticalThreat, InvestigationShare, InvestigationWorkspace,
    PipelineOpportunity, PriorityQueueItem, RecordActivityRequest, ShareWorkspaceRequest,
    SourceEvidence, StrategicOpportunity, SupplierRiskEntry, TeamAssignment,
    UpdatePipelineStageRequest, UpdateQueueItemRequest, UpdateSupplierRiskRequest,
    UpdateWorkspaceRequest, WorkspaceAssignment,
};
use apex_store::postgres::{
    ActivityFeedRecord, CriticalThreatRecord, InvestigationShareRecord,
    InvestigationWorkspaceRecord, PipelineOpportunityRecord, PriorityQueueItemRecord,
    SourceEvidenceRecord, StrategicOpportunityRecord, SupplierRiskEntryRecord,
    TeamAssignmentRecord, WorkspaceAssignmentRecord,
};
use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use chrono::NaiveDate;
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

// ──────────────────────────────────────────────────────────────────────────────
// Validation Helpers
// ──────────────────────────────────────────────────────────────────────────────

pub fn validate_workspace_request(req: &CreateWorkspaceRequest) -> Result<(), ApiError> {
    if req.name.trim().len() < 3 {
        return Err(ApiError::validation(
            "name",
            "must be at least 3 characters",
        ));
    }
    if req.name.trim().len() > 255 {
        return Err(ApiError::validation(
            "name",
            "must not exceed 255 characters",
        ));
    }
    if !["ad-hoc", "structured", "incident", "ongoing"].contains(&req.workspace_type.as_str()) {
        return Err(ApiError::validation(
            "workspace_type",
            "must be one of: ad-hoc, structured, incident, ongoing",
        ));
    }
    if !["private", "team", "organization", "public"].contains(&req.visibility.as_str()) {
        return Err(ApiError::validation(
            "visibility",
            "must be one of: private, team, organization, public",
        ));
    }
    Ok(())
}

pub fn validate_priority(value: i32) -> Result<(), ApiError> {
    if !(1..=100).contains(&value) {
        return Err(ApiError::validation(
            "priority",
            "must be between 1 and 100",
        ));
    }
    Ok(())
}

pub fn validate_confidence(value: f64) -> Result<(), ApiError> {
    if !(0.0..=1.0).contains(&value) {
        return Err(ApiError::validation(
            "confidence",
            "must be between 0.0 and 1.0",
        ));
    }
    Ok(())
}

pub fn validate_severity(value: &str) -> Result<(), ApiError> {
    let lower = value.to_lowercase();
    if !["low", "medium", "high", "critical"].contains(&lower.as_str()) {
        return Err(ApiError::validation(
            "severity",
            "must be one of: low, medium, high, critical",
        ));
    }
    Ok(())
}

pub fn validate_stage(value: &str) -> Result<(), ApiError> {
    if ![
        "discovery",
        "qualification",
        "proposal",
        "negotiation",
        "closed_won",
        "closed_lost",
    ]
    .contains(&value)
    {
        return Err(ApiError::validation("stage",
            "must be one of: discovery, qualification, proposal, negotiation, closed_won, closed_lost"));
    }
    Ok(())
}

fn normalize_optional_text(text: Option<String>) -> Option<String> {
    text.and_then(|t| {
        let trimmed = t.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    })
}

// ──────────────────────────────────────────────────────────────────────────────
// Parse & Conversion Helpers
// ──────────────────────────────────────────────────────────────────────────────

fn parse_uuid(id: &str, field: &str) -> Result<Uuid, ApiError> {
    Uuid::parse_str(id).map_err(|_| ApiError::validation(field, "must be a valid UUID"))
}

fn parse_optional_uuid(id: &Option<String>, field: &str) -> Result<Option<Uuid>, ApiError> {
    id.as_ref().map(|s| parse_uuid(s, field)).transpose()
}

fn parse_optional_date(s: &Option<String>) -> Result<Option<NaiveDate>, ApiError> {
    s.as_ref()
        .map(|d| {
            NaiveDate::parse_from_str(d, "%Y-%m-%d")
                .map_err(|_| ApiError::validation("date", "must be in YYYY-MM-DD format"))
        })
        .transpose()
}

fn store_err(e: anyhow::Error) -> ApiError {
    ApiError::internal(e.to_string())
}

fn opportunity_from_record(r: StrategicOpportunityRecord) -> StrategicOpportunity {
    StrategicOpportunity {
        id: r.id.to_string(),
        title: r.title,
        description: r.description,
        opportunity_type: r.opportunity_type,
        priority_score: r.priority_score,
        confidence: r.confidence,
        entity_id: r.entity_id.map(|u| u.to_string()),
        entity_type: r.entity_type,
        region: r.region,
        estimated_value: r.estimated_value,
        recommended_actions: r.recommended_actions,
        owner_id: r.owner_id,
        status: r.status,
        due_date: r.due_date,
        metadata: r.metadata,
        created_at: r.created_at,
        updated_at: r.updated_at,
    }
}

fn threat_from_record(r: CriticalThreatRecord) -> CriticalThreat {
    CriticalThreat {
        id: r.id.to_string(),
        title: r.title,
        description: r.description,
        threat_type: r.threat_type,
        severity: r.severity,
        impact_score: r.impact_score,
        confidence: r.confidence,
        entity_id: r.entity_id.map(|u| u.to_string()),
        entity_type: r.entity_type,
        region: r.region,
        mitigation_steps: r.mitigation_steps,
        owner_id: r.owner_id,
        status: r.status,
        sla_deadline: r.sla_deadline,
        resolved_at: r.resolved_at,
        metadata: r.metadata,
        created_at: r.created_at,
        updated_at: r.updated_at,
    }
}

fn workspace_from_record(r: InvestigationWorkspaceRecord) -> InvestigationWorkspace {
    InvestigationWorkspace {
        id: r.id.to_string(),
        name: r.name,
        description: r.description,
        workspace_type: r.workspace_type,
        owner_id: r.owner_id,
        team_id: r.team_id,
        status: r.status,
        visibility: r.visibility,
        tags: r.tags,
        entity_focus: r.entity_focus,
        findings: r.findings,
        conclusions: r.conclusions,
        metadata: r.metadata,
        created_at: r.created_at,
        updated_at: r.updated_at,
        closed_at: r.closed_at,
    }
}

fn assignment_from_record(r: WorkspaceAssignmentRecord) -> WorkspaceAssignment {
    WorkspaceAssignment {
        id: r.id.to_string(),
        workspace_id: r.workspace_id.to_string(),
        user_id: r.user_id,
        role: r.role,
        assigned_by: r.assigned_by,
        assigned_at: r.assigned_at,
        updated_at: r.updated_at,
    }
}

fn activity_from_record(r: ActivityFeedRecord) -> ActivityEntry {
    let formatted_details =
        apex_api::routes::collaboration::format_activity_details(&r.action_type, &r.details);
    ActivityEntry {
        id: r.id.to_string(),
        actor_id: r.actor_id,
        actor_name: r.actor_name,
        action_type: r.action_type,
        entity_type: r.entity_type,
        entity_id: r.entity_id.map(|u| u.to_string()),
        entity_name: r.entity_name,
        details: r.details,
        formatted_details,
        workspace_id: r.workspace_id.map(|u| u.to_string()),
        team_id: r.team_id,
        visibility: r.visibility,
        created_at: r.created_at,
    }
}

fn share_from_record(r: InvestigationShareRecord) -> InvestigationShare {
    InvestigationShare {
        id: r.id.to_string(),
        workspace_id: r.workspace_id.to_string(),
        shared_by: r.shared_by,
        shared_with: r.shared_with,
        share_type: r.share_type,
        access_level: r.access_level,
        message: r.message,
        expires_at: r.expires_at,
        created_at: r.created_at,
    }
}

fn queue_item_from_record(r: PriorityQueueItemRecord) -> PriorityQueueItem {
    PriorityQueueItem {
        id: r.id.to_string(),
        user_id: r.user_id,
        queue_date: r.queue_date.to_string(),
        item_type: r.item_type,
        item_id: r.item_id.to_string(),
        item_title: r.item_title,
        priority: r.priority,
        status: r.status,
        notes: r.notes,
        completed_at: r.completed_at,
        created_at: r.created_at,
        updated_at: r.updated_at,
    }
}

fn supplier_risk_from_record(r: SupplierRiskEntryRecord) -> SupplierRiskEntry {
    SupplierRiskEntry {
        id: r.id.to_string(),
        supplier_id: r.supplier_id.to_string(),
        risk_category: r.risk_category,
        risk_score: r.risk_score,
        risk_factors: r.risk_factors,
        mitigation: r.mitigation,
        owner_id: r.owner_id,
        status: r.status,
        last_reviewed: r.last_reviewed,
        next_review: r.next_review,
        created_at: r.created_at,
        updated_at: r.updated_at,
    }
}

fn pipeline_from_record(r: PipelineOpportunityRecord) -> PipelineOpportunity {
    PipelineOpportunity {
        id: r.id.to_string(),
        opportunity_id: r.opportunity_id.map(|u| u.to_string()),
        title: r.title,
        stage: r.stage,
        value_estimate: r.value_estimate,
        probability: r.probability,
        owner_id: r.owner_id,
        expected_close: r.expected_close.map(|d| d.to_string()),
        actual_close: r.actual_close.map(|d| d.to_string()),
        notes: r.notes,
        metadata: r.metadata,
        created_at: r.created_at,
        updated_at: r.updated_at,
        closed_at: r.closed_at,
    }
}

fn evidence_from_record(r: SourceEvidenceRecord) -> SourceEvidence {
    SourceEvidence {
        id: r.id.to_string(),
        entity_type: r.entity_type,
        entity_id: r.entity_id.to_string(),
        evidence_type: r.evidence_type,
        source_url: r.source_url,
        source_domain: r.source_domain,
        source_name: r.source_name,
        reliability_score: r.reliability_score,
        content_hash: r.content_hash,
        excerpt: r.excerpt,
        metadata: r.metadata,
        created_at: r.created_at,
    }
}

fn team_assignment_from_record(r: TeamAssignmentRecord) -> TeamAssignment {
    TeamAssignment {
        id: r.id.to_string(),
        team_id: r.team_id,
        team_name: r.team_name,
        entity_type: r.entity_type,
        entity_id: r.entity_id.to_string(),
        assigned_by: r.assigned_by,
        assigned_to: r.assigned_to,
        role: r.role,
        notes: r.notes,
        created_at: r.created_at,
        updated_at: r.updated_at,
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Executive Dashboard Endpoints
// ──────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ExecutiveQuery {
    pub include_threats: Option<bool>,
    pub include_opportunities: Option<bool>,
    pub priority_threshold: Option<f64>,
    #[allow(dead_code)]
    pub region_filter: Option<String>,
}

/// GET /api/executive/summary
/// Returns the executive dashboard summary with top opportunities, threats, and actions
pub async fn get_executive_summary(
    State(state): State<crate::AppState>,
    Query(query): Query<ExecutiveQuery>,
) -> Result<Json<ApiResponse<Value>>, ApiError> {
    let include_threats = query.include_threats.unwrap_or(true);
    let include_opportunities = query.include_opportunities.unwrap_or(true);
    let priority_threshold = query.priority_threshold.unwrap_or(0.7);

    let limit: i64 = 20;

    let opportunity_records = if include_opportunities {
        state
            .store
            .list_strategic_opportunities(false, limit)
            .await
            .map_err(store_err)?
    } else {
        Vec::new()
    };

    let threat_records = if include_threats {
        state
            .store
            .list_critical_threats(limit)
            .await
            .map_err(store_err)?
    } else {
        Vec::new()
    };

    let top_opportunities: Vec<StrategicOpportunity> = opportunity_records
        .iter()
        .filter(|o| o.priority_score >= priority_threshold)
        .map(|r| opportunity_from_record(r.clone()))
        .collect();

    let critical_threats: Vec<CriticalThreat> = threat_records
        .iter()
        .filter(|t| t.impact_score >= priority_threshold)
        .map(|r| threat_from_record(r.clone()))
        .collect();

    let total_opportunities = opportunity_records.len();
    let total_threats = threat_records.len();
    let high_priority_count = top_opportunities.len() + critical_threats.len();

    let all_confidences: Vec<f64> = opportunity_records
        .iter()
        .map(|o| o.confidence)
        .chain(threat_records.iter().map(|t| t.confidence))
        .collect();

    let average_confidence = if all_confidences.is_empty() {
        0.0
    } else {
        all_confidences.iter().sum::<f64>() / all_confidences.len() as f64
    };

    let mut regions_affected: Vec<String> = opportunity_records
        .iter()
        .filter_map(|o| o.region.clone())
        .chain(threat_records.iter().filter_map(|t| t.region.clone()))
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

/// GET /api/executive/opportunities
/// List strategic opportunities
pub async fn list_opportunities(
    State(state): State<crate::AppState>,
    Query(params): Query<ListOpportunitiesQuery>,
) -> Result<Json<ApiResponse<Vec<StrategicOpportunity>>>, ApiError> {
    let limit = params.limit.unwrap_or(50) as i64;
    let include_closed = params.include_closed.unwrap_or(false);

    let records = state
        .store
        .list_strategic_opportunities(include_closed, limit)
        .await
        .map_err(store_err)?;

    let opportunities: Vec<StrategicOpportunity> = records
        .into_iter()
        .filter(|o| {
            if let Some(threshold) = params.priority_threshold {
                o.priority_score >= threshold
            } else {
                true
            }
        })
        .filter(|o| {
            if let Some(ref region) = params.region {
                o.region.as_deref() == Some(region.as_str())
            } else {
                true
            }
        })
        .map(opportunity_from_record)
        .collect();

    Ok(Json(success(opportunities)))
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct ListOpportunitiesQuery {
    pub priority_threshold: Option<f64>,
    pub region: Option<String>,
    pub limit: Option<u32>,
    pub include_closed: Option<bool>,
}

/// POST /api/executive/opportunities
/// Create a new strategic opportunity
pub async fn create_opportunity(
    State(state): State<crate::AppState>,
    Json(req): Json<CreateOpportunityRequest>,
) -> Result<Json<ApiResponse<StrategicOpportunity>>, ApiError> {
    validate_confidence(req.confidence)?;

    if req.priority_score < 0.0 || req.priority_score > 1.0 {
        return Err(ApiError::validation(
            "priority_score",
            "must be between 0.0 and 1.0",
        ));
    }

    let entity_id = parse_optional_uuid(&req.entity_id, "entity_id")?;

    let record = state
        .store
        .create_strategic_opportunity(
            req.title.trim(),
            normalize_optional_text(req.description).as_deref(),
            &req.opportunity_type,
            req.priority_score,
            req.confidence,
            entity_id,
            req.entity_type.as_deref(),
            req.region.as_deref(),
            req.estimated_value.as_deref(),
            &req.recommended_actions,
            req.owner_id.as_deref(),
            req.due_date,
        )
        .await
        .map_err(store_err)?;

    Ok(Json(success(opportunity_from_record(record))))
}

/// GET /api/executive/threats
/// List critical threats
pub async fn list_threats(
    State(state): State<crate::AppState>,
    Query(params): Query<ListThreatsQuery>,
) -> Result<Json<ApiResponse<Vec<CriticalThreat>>>, ApiError> {
    let limit = params.limit.unwrap_or(50) as i64;

    let records = state
        .store
        .list_critical_threats(limit)
        .await
        .map_err(store_err)?;

    let threats: Vec<CriticalThreat> = records
        .into_iter()
        .filter(|t| {
            if let Some(threshold) = params.impact_threshold {
                t.impact_score >= threshold
            } else {
                true
            }
        })
        .filter(|t| {
            if let Some(ref region) = params.region {
                t.region.as_deref() == Some(region.as_str())
            } else {
                true
            }
        })
        .filter(|t| {
            if let Some(ref severity) = params.severity {
                t.severity.eq_ignore_ascii_case(severity)
            } else {
                true
            }
        })
        .map(threat_from_record)
        .collect();

    Ok(Json(success(threats)))
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct ListThreatsQuery {
    pub impact_threshold: Option<f64>,
    pub region: Option<String>,
    pub severity: Option<String>,
    pub limit: Option<u32>,
}

/// POST /api/executive/threats
/// Create a new critical threat
pub async fn create_threat(
    State(state): State<crate::AppState>,
    Json(req): Json<CreateThreatRequest>,
) -> Result<Json<ApiResponse<CriticalThreat>>, ApiError> {
    validate_confidence(req.confidence)?;
    validate_severity(&req.severity)?;

    if req.impact_score < 0.0 || req.impact_score > 1.0 {
        return Err(ApiError::validation(
            "impact_score",
            "must be between 0.0 and 1.0",
        ));
    }

    let entity_id = parse_optional_uuid(&req.entity_id, "entity_id")?;

    let record = state
        .store
        .create_critical_threat(
            req.title.trim(),
            normalize_optional_text(req.description).as_deref(),
            &req.threat_type,
            &req.severity.to_lowercase(),
            req.impact_score,
            req.confidence,
            entity_id,
            req.entity_type.as_deref(),
            req.region.as_deref(),
            &req.mitigation_steps,
            req.owner_id.as_deref(),
            req.sla_deadline,
        )
        .await
        .map_err(store_err)?;

    Ok(Json(success(threat_from_record(record))))
}

/// GET /api/executive/opportunities/:id
/// Get a specific strategic opportunity
pub async fn get_opportunity(
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<StrategicOpportunity>>, ApiError> {
    let uuid = parse_uuid(&id, "id")?;
    let record = state
        .store
        .get_strategic_opportunity(uuid)
        .await
        .map_err(store_err)?
        .ok_or_else(|| ApiError::not_found("opportunity", &id))?;

    Ok(Json(success(opportunity_from_record(record))))
}

/// PATCH /api/executive/opportunities/:id
/// Update a strategic opportunity
pub async fn update_opportunity(
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
    Json(req): Json<CreateOpportunityRequest>,
) -> Result<Json<ApiResponse<StrategicOpportunity>>, ApiError> {
    validate_confidence(req.confidence)?;

    if req.priority_score < 0.0 || req.priority_score > 1.0 {
        return Err(ApiError::validation(
            "priority_score",
            "must be between 0.0 and 1.0",
        ));
    }

    let uuid = parse_uuid(&id, "id")?;

    // Update status if provided in the opportunity type; otherwise keep active
    let status = if req.opportunity_type.contains("closed") {
        "closed"
    } else {
        "active"
    };

    let record = state
        .store
        .update_strategic_opportunity_status(uuid, status)
        .await
        .map_err(store_err)?
        .ok_or_else(|| ApiError::not_found("opportunity", &id))?;

    Ok(Json(success(opportunity_from_record(record))))
}

/// GET /api/executive/threats/:id
/// Get a specific critical threat
pub async fn get_threat(
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<CriticalThreat>>, ApiError> {
    let uuid = parse_uuid(&id, "id")?;
    let record = state
        .store
        .get_critical_threat(uuid)
        .await
        .map_err(store_err)?
        .ok_or_else(|| ApiError::not_found("threat", &id))?;

    Ok(Json(success(threat_from_record(record))))
}

/// PATCH /api/executive/threats/:id
/// Update a critical threat
pub async fn update_threat(
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
    Json(req): Json<CreateThreatRequest>,
) -> Result<Json<ApiResponse<CriticalThreat>>, ApiError> {
    validate_confidence(req.confidence)?;
    validate_severity(&req.severity)?;

    let uuid = parse_uuid(&id, "id")?;

    let resolved =
        req.severity.eq_ignore_ascii_case("resolved") || req.threat_type.contains("resolved");
    let status = if resolved { "resolved" } else { "active" };

    let record = state
        .store
        .update_critical_threat_status(uuid, status, resolved)
        .await
        .map_err(store_err)?
        .ok_or_else(|| ApiError::not_found("threat", &id))?;

    Ok(Json(success(threat_from_record(record))))
}

// ──────────────────────────────────────────────────────────────────────────────
// Investigation Workspaces Endpoints
// ──────────────────────────────────────────────────────────────────────────────

/// GET /api/workspaces
/// List investigation workspaces
pub async fn list_workspaces(
    State(state): State<crate::AppState>,
    Extension(auth): Extension<ApiAuthContext>,
    Query(params): Query<ListWorkspacesQuery>,
) -> Result<Json<ApiResponse<Vec<InvestigationWorkspace>>>, ApiError> {
    let limit = params.limit.unwrap_or(50) as i64;

    let records = state
        .store
        .list_investigation_workspaces(limit)
        .await
        .map_err(store_err)?;

    let workspaces: Vec<InvestigationWorkspace> =
        records.into_iter().map(workspace_from_record).collect();

    // Filter by visibility based on user role
    let filtered: Vec<InvestigationWorkspace> = workspaces
        .into_iter()
        .filter(|w| {
            if auth.role.can_admin()
                || w.visibility == "public"
                || w.visibility == "organization"
                || w.visibility == "team"
            {
                true
            } else {
                w.owner_id == auth.user_id
            }
        })
        .filter(|w| {
            if let Some(ref ws_type) = params.workspace_type {
                w.workspace_type == *ws_type
            } else {
                true
            }
        })
        .filter(|w| {
            if let Some(ref status) = params.status {
                w.status == *status
            } else {
                true
            }
        })
        .collect();

    Ok(Json(success(filtered)))
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct ListWorkspacesQuery {
    pub workspace_type: Option<String>,
    pub status: Option<String>,
    pub visibility: Option<String>,
    pub limit: Option<u32>,
}

/// POST /api/workspaces
/// Create a new investigation workspace
pub async fn create_workspace(
    State(state): State<crate::AppState>,
    Extension(auth): Extension<ApiAuthContext>,
    Json(req): Json<CreateWorkspaceRequest>,
) -> Result<Json<ApiResponse<InvestigationWorkspace>>, ApiError> {
    validate_workspace_request(&req)?;

    let record = state
        .store
        .create_investigation_workspace(
            req.name.trim(),
            normalize_optional_text(req.description).as_deref(),
            &req.workspace_type,
            &auth.user_id,
            req.team_id.as_deref(),
            &req.visibility,
            &req.tags,
            &req.entity_focus,
        )
        .await
        .map_err(store_err)?;

    Ok(Json(success(workspace_from_record(record))))
}

/// GET /api/workspaces/:id
/// Get a specific workspace
pub async fn get_workspace(
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<InvestigationWorkspace>>, ApiError> {
    let uuid = parse_uuid(&id, "id")?;
    let record = state
        .store
        .get_investigation_workspace(uuid)
        .await
        .map_err(store_err)?
        .ok_or_else(|| ApiError::not_found("workspace", &id))?;

    Ok(Json(success(workspace_from_record(record))))
}

/// PUT /api/workspaces/:id
/// Update a workspace
pub async fn update_workspace(
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateWorkspaceRequest>,
) -> Result<Json<ApiResponse<InvestigationWorkspace>>, ApiError> {
    let uuid = parse_uuid(&id, "id")?;

    let record = state
        .store
        .update_investigation_workspace(
            uuid,
            req.name.as_deref(),
            req.description.as_ref().map(|d| Some(d.as_str())),
            req.status.as_deref(),
            req.tags.as_deref(),
            req.entity_focus.as_ref(),
            req.findings.as_ref().map(|f| Some(f.as_str())),
            req.conclusions.as_ref().map(|c| Some(c.as_str())),
        )
        .await
        .map_err(store_err)?
        .ok_or_else(|| ApiError::not_found("workspace", &id))?;

    Ok(Json(success(workspace_from_record(record))))
}

/// POST /api/workspaces/:id/assignments
/// Assign a user to a workspace
pub async fn assign_user_to_workspace(
    State(state): State<crate::AppState>,
    Extension(auth): Extension<ApiAuthContext>,
    Path(id): Path<String>,
    Json(req): Json<AssignUserRequest>,
) -> Result<Json<ApiResponse<WorkspaceAssignment>>, ApiError> {
    if req.user_id.trim().is_empty() {
        return Err(ApiError::validation("user_id", "cannot be empty"));
    }
    if !["owner", "lead", "contributor", "viewer", "reviewer"].contains(&req.role.as_str()) {
        return Err(ApiError::validation(
            "role",
            "must be one of: owner, lead, contributor, viewer, reviewer",
        ));
    }

    let workspace_uuid = parse_uuid(&id, "workspace_id")?;

    let record = state
        .store
        .create_workspace_assignment(workspace_uuid, &req.user_id, &req.role, &auth.user_id)
        .await
        .map_err(store_err)?;

    Ok(Json(success(assignment_from_record(record))))
}

/// DELETE /api/workspaces/:id
/// Delete a workspace
pub async fn delete_workspace(
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<Value>>, ApiError> {
    let uuid = parse_uuid(&id, "id")?;
    let deleted = state
        .store
        .delete_investigation_workspace(uuid)
        .await
        .map_err(store_err)?;

    if !deleted {
        return Err(ApiError::not_found("workspace", &id));
    }

    Ok(Json(success(serde_json::json!({"deleted": true}))))
}

/// GET /api/workspaces/:id/assignments
/// List workspace assignments
pub async fn list_workspace_assignments(
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<Vec<WorkspaceAssignment>>>, ApiError> {
    let workspace_uuid = parse_uuid(&id, "workspace_id")?;

    let records = state
        .store
        .list_workspace_assignments(workspace_uuid)
        .await
        .map_err(store_err)?;

    let assignments: Vec<WorkspaceAssignment> =
        records.into_iter().map(assignment_from_record).collect();

    Ok(Json(success(assignments)))
}

/// DELETE /api/workspaces/:id/assignments/:user_id
/// Remove a user from a workspace
pub async fn remove_user_from_workspace(
    State(state): State<crate::AppState>,
    Path((workspace_id, user_id)): Path<(String, String)>,
) -> Result<Json<ApiResponse<Value>>, ApiError> {
    let workspace_uuid = parse_uuid(&workspace_id, "workspace_id")?;

    let removed = state
        .store
        .remove_workspace_assignment(workspace_uuid, &user_id)
        .await
        .map_err(store_err)?;

    if !removed {
        return Err(ApiError::not_found("assignment", &user_id));
    }

    Ok(Json(success(serde_json::json!({"removed": true}))))
}

// ──────────────────────────────────────────────────────────────────────────────
// Priority Queue Endpoints
// ──────────────────────────────────────────────────────────────────────────────

/// GET /api/queue
/// List priority queue items
pub async fn list_queue_items(
    State(state): State<crate::AppState>,
    Extension(auth): Extension<ApiAuthContext>,
    Query(params): Query<ListQueueQuery>,
) -> Result<Json<ApiResponse<Vec<PriorityQueueItem>>>, ApiError> {
    let limit = params.limit.unwrap_or(50) as i64;

    let records = state
        .store
        .list_priority_queue_items(&auth.user_id, params.status.as_deref(), limit)
        .await
        .map_err(store_err)?;

    let items: Vec<PriorityQueueItem> = records
        .into_iter()
        .filter(|q| {
            if let Some(ref item_type) = params.item_type {
                q.item_type == *item_type
            } else {
                true
            }
        })
        .map(queue_item_from_record)
        .collect();

    Ok(Json(success(items)))
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct ListQueueQuery {
    pub status: Option<String>,
    pub item_type: Option<String>,
    pub limit: Option<u32>,
}

/// POST /api/queue
/// Add item to priority queue
pub async fn add_to_queue(
    State(state): State<crate::AppState>,
    Extension(auth): Extension<ApiAuthContext>,
    Json(req): Json<AddToQueueRequest>,
) -> Result<Json<ApiResponse<PriorityQueueItem>>, ApiError> {
    validate_priority(req.priority)?;

    if req.item_type.trim().is_empty() || req.item_id.trim().is_empty() {
        return Err(ApiError::validation("item_type/item_id", "cannot be empty"));
    }

    let item_uuid = parse_uuid(&req.item_id, "item_id")?;

    let record = state
        .store
        .create_priority_queue_item(
            &auth.user_id,
            &req.item_type,
            item_uuid,
            &req.item_title,
            req.priority,
            normalize_optional_text(req.notes).as_deref(),
        )
        .await
        .map_err(store_err)?;

    Ok(Json(success(queue_item_from_record(record))))
}

/// PATCH /api/queue/:id
/// Update queue item status
pub async fn update_queue_item(
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateQueueItemRequest>,
) -> Result<Json<ApiResponse<PriorityQueueItem>>, ApiError> {
    if let Some(priority) = req.priority {
        validate_priority(priority)?;
    }
    if let Some(ref status) = req.status {
        if !["pending", "in_progress", "completed", "cancelled"].contains(&status.as_str()) {
            return Err(ApiError::validation(
                "status",
                "must be one of: pending, in_progress, completed, cancelled",
            ));
        }
    }

    let uuid = parse_uuid(&id, "id")?;

    let record = state
        .store
        .update_priority_queue_item(
            uuid,
            req.priority,
            req.status.as_deref(),
            req.notes.as_ref().map(|n| Some(n.as_str())),
        )
        .await
        .map_err(store_err)?
        .ok_or_else(|| ApiError::not_found("queue_item", &id))?;

    Ok(Json(success(queue_item_from_record(record))))
}

// ──────────────────────────────────────────────────────────────────────────────
// Supplier Risk Endpoints
// ──────────────────────────────────────────────────────────────────────────────

/// GET /api/supplier-risk
/// List supplier risk entries
pub async fn list_supplier_risks(
    State(state): State<crate::AppState>,
    Query(params): Query<ListSupplierRisksQuery>,
) -> Result<Json<ApiResponse<Vec<SupplierRiskEntry>>>, ApiError> {
    let limit = params.limit.unwrap_or(50) as i64;

    let records = state
        .store
        .list_supplier_risk_entries(params.status.as_deref(), limit)
        .await
        .map_err(store_err)?;

    let risks: Vec<SupplierRiskEntry> = records
        .into_iter()
        .filter(|r| {
            if let Some(ref category) = params.risk_category {
                r.risk_category == *category
            } else {
                true
            }
        })
        .map(supplier_risk_from_record)
        .collect();

    Ok(Json(success(risks)))
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct ListSupplierRisksQuery {
    pub risk_category: Option<String>,
    pub status: Option<String>,
    pub limit: Option<u32>,
}

/// POST /api/supplier-risk
/// Add supplier risk entry
pub async fn add_supplier_risk(
    State(state): State<crate::AppState>,
    Json(req): Json<AddSupplierRiskRequest>,
) -> Result<Json<ApiResponse<SupplierRiskEntry>>, ApiError> {
    if req.risk_score < 0.0 || req.risk_score > 1.0 {
        return Err(ApiError::validation(
            "risk_score",
            "must be between 0.0 and 1.0",
        ));
    }

    let valid_categories = [
        "financial",
        "operational",
        "compliance",
        "geopolitical",
        "environmental",
        "technological",
        "reputational",
        "strategic",
    ];
    if !valid_categories.contains(&req.risk_category.as_str()) {
        return Err(ApiError::validation("risk_category", "invalid category"));
    }

    let record = state
        .store
        .create_supplier_risk_entry(
            &req.supplier_id,
            &req.risk_category,
            req.risk_score,
            &req.risk_factors,
            req.mitigation.as_deref(),
            req.owner_id.as_deref(),
        )
        .await
        .map_err(store_err)?;

    Ok(Json(success(supplier_risk_from_record(record))))
}

/// PATCH /api/supplier-risk/:id
/// Update supplier risk entry
pub async fn update_supplier_risk(
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateSupplierRiskRequest>,
) -> Result<Json<ApiResponse<SupplierRiskEntry>>, ApiError> {
    if let Some(risk_score) = req.risk_score {
        if !(0.0..=1.0).contains(&risk_score) {
            return Err(ApiError::validation(
                "risk_score",
                "must be between 0.0 and 1.0",
            ));
        }
    }

    let uuid = parse_uuid(&id, "id")?;

    let record = state
        .store
        .update_supplier_risk_entry(
            uuid,
            req.risk_score,
            req.mitigation.as_deref(),
            req.status.as_deref(),
        )
        .await
        .map_err(store_err)?
        .ok_or_else(|| ApiError::not_found("supplier_risk", &id))?;

    Ok(Json(success(supplier_risk_from_record(record))))
}

// ──────────────────────────────────────────────────────────────────────────────
// Pipeline Endpoints
// ──────────────────────────────────────────────────────────────────────────────

/// GET /api/pipeline
/// List pipeline opportunities
pub async fn list_pipeline_opportunities(
    State(state): State<crate::AppState>,
    Query(params): Query<ListPipelineQuery>,
) -> Result<Json<ApiResponse<Vec<PipelineOpportunity>>>, ApiError> {
    let limit = params.limit.unwrap_or(50) as i64;

    let records = state
        .store
        .list_pipeline_opportunities(params.stage.as_deref(), params.owner_id.as_deref(), limit)
        .await
        .map_err(store_err)?;

    let opportunities: Vec<PipelineOpportunity> =
        records.into_iter().map(pipeline_from_record).collect();

    Ok(Json(success(opportunities)))
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct ListPipelineQuery {
    pub stage: Option<String>,
    pub owner_id: Option<String>,
    pub limit: Option<u32>,
}

/// POST /api/pipeline
/// Create pipeline opportunity
pub async fn create_pipeline_opportunity(
    State(state): State<crate::AppState>,
    Json(req): Json<CreatePipelineOpportunityRequest>,
) -> Result<Json<ApiResponse<PipelineOpportunity>>, ApiError> {
    validate_stage(&req.stage)?;
    validate_confidence(req.probability)?;

    let expected_close = parse_optional_date(&req.expected_close)?;

    let record = state
        .store
        .create_pipeline_opportunity(
            req.opportunity_id.as_deref(),
            &req.title,
            &req.stage,
            req.value_estimate,
            req.probability,
            req.owner_id.as_deref(),
            expected_close,
            req.notes.as_deref(),
        )
        .await
        .map_err(store_err)?;

    Ok(Json(success(pipeline_from_record(record))))
}

/// PATCH /api/pipeline/:id/stage
/// Update pipeline stage
pub async fn update_pipeline_stage(
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdatePipelineStageRequest>,
) -> Result<Json<ApiResponse<PipelineOpportunity>>, ApiError> {
    validate_stage(&req.stage)?;

    let uuid = parse_uuid(&id, "id")?;

    let record = state
        .store
        .update_pipeline_stage(uuid, &req.stage, req.notes.as_deref())
        .await
        .map_err(store_err)?
        .ok_or_else(|| ApiError::not_found("pipeline_opportunity", &id))?;

    Ok(Json(success(pipeline_from_record(record))))
}

// ──────────────────────────────────────────────────────────────────────────────
// Activity Feed Endpoints
// ──────────────────────────────────────────────────────────────────────────────

/// GET /api/activity-feed
/// Get activity feed
pub async fn get_activity_feed(
    State(state): State<crate::AppState>,
    Query(params): Query<ActivityFeedQuery>,
) -> Result<Json<ApiResponse<Vec<ActivityEntry>>>, ApiError> {
    let limit = params.limit.unwrap_or(50) as i64;
    let workspace_id = parse_optional_uuid(&params.workspace_id, "workspace_id")?;

    let records = state
        .store
        .list_activity_feed(
            workspace_id,
            params.team_id.as_deref(),
            params.actor_id.as_deref(),
            limit,
        )
        .await
        .map_err(store_err)?;

    let entries: Vec<ActivityEntry> = records
        .into_iter()
        .filter(|e| {
            if let Some(ref action_type) = params.action_type {
                e.action_type == *action_type
            } else {
                true
            }
        })
        .map(activity_from_record)
        .collect();

    Ok(Json(success(entries)))
}

/// POST /api/activity-feed
/// Record new activity
pub async fn record_activity(
    State(state): State<crate::AppState>,
    Extension(auth): Extension<ApiAuthContext>,
    Json(req): Json<RecordActivityRequest>,
) -> Result<Json<ApiResponse<ActivityEntry>>, ApiError> {
    let valid_actions = [
        "create",
        "update",
        "delete",
        "share",
        "assign",
        "comment",
        "resolve",
        "reopen",
        "escalate",
        "deescalate",
        "approve",
        "reject",
        "merge",
        "split",
    ];
    if !valid_actions.contains(&req.action_type.as_str()) {
        return Err(ApiError::validation("action_type", "invalid action type"));
    }

    let workspace_id = parse_optional_uuid(&req.workspace_id, "workspace_id")?;

    // Resolve the actor's display name from the store
    let actor_name = match state.store.get_analyst_user(&auth.user_id).await {
        Ok(Some(user)) => user.display_name,
        _ => auth.user_id.clone(),
    };

    let visibility = if req.visibility.is_empty() {
        "team"
    } else {
        &req.visibility
    };

    let record = state
        .store
        .create_activity_entry(
            &auth.user_id,
            &actor_name,
            &req.action_type,
            req.entity_type.as_deref(),
            req.entity_id.as_deref(),
            req.entity_name.as_deref(),
            &req.details,
            workspace_id,
            req.team_id.as_deref(),
            visibility,
        )
        .await
        .map_err(store_err)?;

    Ok(Json(success(activity_from_record(record))))
}

// ──────────────────────────────────────────────────────────────────────────────
// Investigation Shares Endpoints
// ──────────────────────────────────────────────────────────────────────────────

/// GET /api/workspaces/:id/shares
/// List workspace shares
pub async fn list_workspace_shares(
    State(state): State<crate::AppState>,
    Path(workspace_id): Path<String>,
) -> Result<Json<ApiResponse<Vec<InvestigationShare>>>, ApiError> {
    let workspace_uuid = parse_uuid(&workspace_id, "workspace_id")?;

    let records = state
        .store
        .list_investigation_shares(workspace_uuid)
        .await
        .map_err(store_err)?;

    let shares: Vec<InvestigationShare> = records.into_iter().map(share_from_record).collect();

    Ok(Json(success(shares)))
}

/// POST /api/workspaces/:id/shares
/// Share a workspace
pub async fn share_workspace(
    State(state): State<crate::AppState>,
    Extension(auth): Extension<ApiAuthContext>,
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

    let workspace_uuid = parse_uuid(&workspace_id, "workspace_id")?;

    let record = state
        .store
        .create_investigation_share(
            workspace_uuid,
            &auth.user_id,
            &req.shared_with,
            &req.share_type,
            &req.access_level,
            req.message.as_deref(),
            req.expires_at,
        )
        .await
        .map_err(store_err)?;

    Ok(Json(success(share_from_record(record))))
}

// ──────────────────────────────────────────────────────────────────────────────
// Source Evidence Endpoints
// ──────────────────────────────────────────────────────────────────────────────

/// GET /api/evidence
/// Get source evidence for an entity
pub async fn get_evidence(
    State(state): State<crate::AppState>,
    Query(params): Query<GetEvidenceQuery>,
) -> Result<Json<ApiResponse<Vec<SourceEvidence>>>, ApiError> {
    let limit = params.limit.unwrap_or(50) as i64;
    let records = state
        .store
        .list_source_evidence(
            params.entity_type.as_deref(),
            params.entity_id.as_deref(),
            params.evidence_type.as_deref(),
            limit,
        )
        .await
        .map_err(store_err)?;

    let evidence: Vec<SourceEvidence> = records.into_iter().map(evidence_from_record).collect();

    Ok(Json(success(evidence)))
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct GetEvidenceQuery {
    pub entity_type: Option<String>,
    pub entity_id: Option<String>,
    pub evidence_type: Option<String>,
    pub limit: Option<u32>,
}

/// POST /api/evidence
/// Add source evidence
pub async fn add_evidence(
    State(state): State<crate::AppState>,
    Json(req): Json<AddEvidenceRequest>,
) -> Result<Json<ApiResponse<SourceEvidence>>, ApiError> {
    validate_confidence(req.reliability_score)?;

    let valid_types = [
        "web_content",
        "document",
        "financial_report",
        "news_article",
        "social_media",
        "regulatory_filing",
        "patent",
        "court_record",
        "public_record",
        "analyst_report",
    ];
    if !valid_types.contains(&req.evidence_type.as_str()) {
        return Err(ApiError::validation(
            "evidence_type",
            "invalid evidence type",
        ));
    }

    if req.source_url.trim().is_empty() {
        return Err(ApiError::validation("source_url", "cannot be empty"));
    }

    let source_domain = req
        .source_name
        .as_ref()
        .and_then(|n| n.split('/').next().map(|s| s.to_string()));

    let record = state
        .store
        .create_source_evidence(
            &req.entity_type,
            &req.entity_id,
            &req.evidence_type,
            &req.source_url,
            source_domain.as_deref(),
            req.source_name.as_deref(),
            req.reliability_score,
            req.excerpt.as_deref(),
            &req.metadata,
        )
        .await
        .map_err(store_err)?;

    Ok(Json(success(evidence_from_record(record))))
}

// ──────────────────────────────────────────────────────────────────────────────
// Team Assignment Endpoints
// ──────────────────────────────────────────────────────────────────────────────

/// GET /api/team-assignments
/// List team assignments
pub async fn list_team_assignments(
    State(state): State<crate::AppState>,
    Query(params): Query<ListTeamAssignmentsQuery>,
) -> Result<Json<ApiResponse<Vec<TeamAssignment>>>, ApiError> {
    let records = state
        .store
        .list_team_assignments(params.entity_type.as_deref(), params.entity_id.as_deref())
        .await
        .map_err(store_err)?;

    let assignments: Vec<TeamAssignment> = records
        .into_iter()
        .map(team_assignment_from_record)
        .collect();

    Ok(Json(success(assignments)))
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct ListTeamAssignmentsQuery {
    pub entity_type: Option<String>,
    pub entity_id: Option<String>,
}

/// POST /api/team-assignments
/// Create team assignment
pub async fn create_team_assignment(
    State(state): State<crate::AppState>,
    Extension(auth): Extension<ApiAuthContext>,
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

    let record = state
        .store
        .create_team_assignment(
            &req.team_id,
            &req.team_name,
            &req.entity_type,
            &req.entity_id,
            &auth.user_id,
            &req.assigned_to,
            &req.role,
            req.notes.as_deref(),
        )
        .await
        .map_err(store_err)?;

    Ok(Json(success(team_assignment_from_record(record))))
}
