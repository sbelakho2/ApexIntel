use super::super::*;
use apex_api::routes::admin::{
    AdminLlmGovernanceResponse, AdminLlmImprovementRunSummary, AdminLlmTrainingDatasetSummary,
    AdminLlmWorkflowRunSummary, AdminPromptVersionSummary,
};
use apex_api::routes::replay::{
    estimate_duration, ReplayProgress, ReplayRequest, ReplayResponse, ReplayStatus,
};

fn count_validation_issues(value: &serde_json::Value) -> usize {
    value.as_array().map(|items| items.len()).unwrap_or(0)
}

fn prompt_version_summary(
    record: apex_store::postgres::PromptVersionRecord,
) -> AdminPromptVersionSummary {
    AdminPromptVersionSummary {
        prompt_id: record.prompt_id,
        version: record.version,
        workflow: record.workflow,
        metadata: record.metadata,
        created_at: record.created_at,
    }
}

fn workflow_run_summary(
    record: apex_store::postgres::LlmWorkflowRunRecord,
) -> AdminLlmWorkflowRunSummary {
    AdminLlmWorkflowRunSummary {
        id: record.id.to_string(),
        workflow: record.workflow,
        prompt_id: record.prompt_id,
        prompt_version: record.prompt_version,
        model_name: record.model_name,
        quality_gate_passed: record.quality_gate_passed,
        validation_issue_count: count_validation_issues(&record.validation_issues),
        duration_ms: record.duration_ms,
        created_at: record.created_at,
    }
}

fn improvement_run_summary(
    record: apex_store::postgres::LlmImprovementRunRecord,
) -> AdminLlmImprovementRunSummary {
    AdminLlmImprovementRunSummary {
        id: record.id.to_string(),
        run_kind: record.run_kind,
        run_key: record.run_key,
        metrics: record.metrics,
        created_at: record.created_at,
    }
}

fn training_dataset_summary(
    record: apex_store::postgres::LlmTrainingDatasetRecord,
) -> AdminLlmTrainingDatasetSummary {
    AdminLlmTrainingDatasetSummary {
        id: record.id.to_string(),
        dataset_name: record.dataset_name,
        dataset_version: record.dataset_version,
        source_run_kind: record.source_run_kind,
        source_run_key: record.source_run_key,
        manifest: record.manifest,
        example_count: record.example_count,
        created_at: record.created_at,
    }
}

#[derive(Debug, serde::Deserialize)]
pub(crate) struct TriggerScanRequest {
    job_kind: String,
}

#[derive(Debug, serde::Serialize)]
pub(crate) struct TriggerScanResponse {
    trigger_id: String,
    job_kind: String,
}

fn build_trigger_scan_response(trigger_id: String, job_kind: String) -> TriggerScanResponse {
    TriggerScanResponse {
        trigger_id,
        job_kind,
    }
}

pub(crate) async fn get_admin_crawl_status(
    State(state): State<AppState>,
) -> (StatusCode, Json<ApiResponse<AdminCrawlStatus>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    match state.store.get_admin_crawl_status().await {
        Ok(status) => {
            let duration_ms = start.elapsed().as_millis() as u64;
            log_latency("get_admin_crawl_status", duration_ms);
            (
                StatusCode::OK,
                Json(success_with_meta(
                    status,
                    ResponseMeta::now()
                        .with_request_id(request_id)
                        .with_duration(duration_ms),
                )),
            )
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "admin crawl status failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to load crawl status",
                ))),
            )
        }
    }
}

pub(crate) async fn get_admin_recipe_performance(
    State(state): State<AppState>,
) -> (StatusCode, Json<ApiResponse<AdminRecipePerformance>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    match state.store.get_admin_recipe_performance().await {
        Ok(perf) => {
            let duration_ms = start.elapsed().as_millis() as u64;
            log_latency("get_admin_recipe_performance", duration_ms);
            (
                StatusCode::OK,
                Json(success_with_meta(
                    perf,
                    ResponseMeta::now()
                        .with_request_id(request_id)
                        .with_duration(duration_ms),
                )),
            )
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "admin recipe performance failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to load recipe performance",
                ))),
            )
        }
    }
}

pub(crate) async fn get_admin_poi_coverage(
    State(state): State<AppState>,
) -> (StatusCode, Json<ApiResponse<AdminPoiCoverage>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    match state.store.get_admin_poi_coverage().await {
        Ok(coverage) => {
            let duration_ms = start.elapsed().as_millis() as u64;
            log_latency("get_admin_poi_coverage", duration_ms);
            (
                StatusCode::OK,
                Json(success_with_meta(
                    coverage,
                    ResponseMeta::now()
                        .with_request_id(request_id)
                        .with_duration(duration_ms),
                )),
            )
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "admin poi coverage failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to load POI coverage",
                ))),
            )
        }
    }
}

pub(crate) async fn get_admin_llm_governance(
    State(state): State<AppState>,
) -> (StatusCode, Json<ApiResponse<AdminLlmGovernanceResponse>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    match state.store.get_admin_llm_governance_overview(25).await {
        Ok(overview) => {
            let duration_ms = start.elapsed().as_millis() as u64;
            log_latency("get_admin_llm_governance", duration_ms);
            (
                StatusCode::OK,
                Json(success_with_meta(
                    AdminLlmGovernanceResponse {
                        prompt_versions: overview
                            .prompt_versions
                            .into_iter()
                            .map(prompt_version_summary)
                            .collect(),
                        workflow_runs: overview
                            .workflow_runs
                            .into_iter()
                            .map(workflow_run_summary)
                            .collect(),
                        improvement_runs: overview
                            .improvement_runs
                            .into_iter()
                            .map(improvement_run_summary)
                            .collect(),
                        training_datasets: overview
                            .training_datasets
                            .into_iter()
                            .map(training_dataset_summary)
                            .collect(),
                    },
                    ResponseMeta::now()
                        .with_request_id(request_id)
                        .with_duration(duration_ms),
                )),
            )
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "admin llm governance failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to load LLM governance overview",
                ))),
            )
        }
    }
}

pub(crate) async fn post_trigger_scan(
    State(state): State<AppState>,
    Json(body): Json<TriggerScanRequest>,
) -> (StatusCode, Json<ApiResponse<TriggerScanResponse>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();

    if !is_valid_manual_trigger_kind(&body.job_kind) {
        let api_err =
            ApiError::validation("job_kind", format!("Unknown job kind: {}", body.job_kind));
        return (StatusCode::BAD_REQUEST, Json(error_response(api_err)));
    }

    match state.store.queue_job_trigger(&body.job_kind).await {
        Ok(trigger_id) => {
            let duration_ms = start.elapsed().as_millis() as u64;
            log_latency("post_trigger_scan", duration_ms);
            tracing::info!(request_id = %request_id, job_kind = %body.job_kind, trigger_id = %trigger_id, "manual job trigger queued");
            (
                StatusCode::ACCEPTED,
                Json(success_with_meta(
                    build_trigger_scan_response(trigger_id, body.job_kind),
                    ResponseMeta::now()
                        .with_request_id(request_id)
                        .with_duration(duration_ms),
                )),
            )
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "queue_job_trigger failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to queue job trigger",
                ))),
            )
        }
    }
}

fn replay_progress_from_record(record: apex_store::postgres::ReplayJobRecord) -> ReplayProgress {
    let progress_pct = if record.total_observations > 0 {
        ((record.processed as f64 / record.total_observations as f64) * 100.0).clamp(0.0, 100.0)
    } else if record.status == ReplayStatus::Completed.as_str() {
        100.0
    } else {
        0.0
    };

    ReplayProgress {
        job_id: record.id.to_string(),
        status: ReplayStatus::from_str(&record.status),
        total_observations: record.total_observations.max(0) as usize,
        processed: record.processed.max(0) as usize,
        warnings_generated: record.warnings_generated.max(0) as usize,
        errors: record.errors.max(0) as usize,
        started_at: record.started_at.unwrap_or(record.created_at),
        completed_at: record.completed_at,
        progress_pct,
    }
}

async fn execute_replay_job(
    state: AppState,
    job_id: Uuid,
    actor: String,
    body: ReplayRequest,
) -> Result<ReplayProgress, anyhow::Error> {
    let total_observations = state
        .store
        .count_replay_candidates(
            body.from_date,
            body.to_date,
            body.observation_types.as_deref(),
            body.entity_ids.as_deref(),
            body.limit() as i64,
        )
        .await?;

    let warnings_generated = if body.emit_warnings() {
        total_observations
    } else {
        0
    };

    let updated = state
        .store
        .update_replay_job(
            job_id,
            ReplayStatus::Completed.as_str(),
            total_observations,
            total_observations,
            warnings_generated,
            0,
            Some(Utc::now()),
        )
        .await?;

    let detail = serde_json::json!({
        "job_id": job_id,
        "from_date": body.from_date,
        "to_date": body.to_date,
        "processed": total_observations,
        "warnings_generated": warnings_generated,
    });
    let _ = state
        .store
        .record_audit_event(&actor, "replay_completed", &detail)
        .await;

    Ok(replay_progress_from_record(updated))
}

pub(crate) async fn post_replay(
    State(state): State<AppState>,
    Extension(auth_ctx): Extension<ApiAuthContext>,
    Json(body): Json<ReplayRequest>,
) -> (StatusCode, Json<ApiResponse<ReplayResponse>>) {
    if let Err(message) = body.validate() {
        return (
            StatusCode::BAD_REQUEST,
            Json(error_response(ApiError::bad_request(message))),
        );
    }

    let request_json = match serde_json::to_value(&body) {
        Ok(value) => value,
        Err(err) => {
            tracing::error!("serialize replay request failed: {err:#}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to serialize replay request",
                ))),
            );
        }
    };

    let total_observations = match state
        .store
        .count_replay_candidates(
            body.from_date,
            body.to_date,
            body.observation_types.as_deref(),
            body.entity_ids.as_deref(),
            body.limit() as i64,
        )
        .await
    {
        Ok(count) => count.max(0) as usize,
        Err(err) => {
            tracing::error!("count replay candidates failed: {err:#}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to count replay candidates",
                ))),
            );
        }
    };

    let recipe_count = state
        .store
        .get_admin_recipe_performance()
        .await
        .map(|perf| perf.production_count.max(1) as usize)
        .unwrap_or(1);

    let job = match state
        .store
        .create_replay_job(&auth_ctx.user_id, &request_json)
        .await
    {
        Ok(job) => job,
        Err(err) => {
            tracing::error!("create replay job failed: {err:#}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to create replay job",
                ))),
            );
        }
    };

    let job_id = job.id;
    let _ = state
        .store
        .record_audit_event(
            &auth_ctx.user_id,
            "replay_requested",
            &serde_json::json!({"job_id": job_id, "request": request_json}),
        )
        .await;

    if body.is_background() {
        let state_clone = state.clone();
        let actor = auth_ctx.user_id.clone();
        let body_clone = body.clone();
        tokio::spawn(async move {
            if let Err(err) =
                execute_replay_job(state_clone.clone(), job_id, actor.clone(), body_clone).await
            {
                tracing::error!(job_id = %job_id, "background replay failed: {err:#}");
                let _ = state_clone
                    .store
                    .update_replay_job(
                        job_id,
                        ReplayStatus::Failed.as_str(),
                        0,
                        0,
                        0,
                        1,
                        Some(Utc::now()),
                    )
                    .await;
                let _ = state_clone
                    .store
                    .record_audit_event(
                        &actor,
                        "replay_failed",
                        &serde_json::json!({"job_id": job_id, "error": err.to_string()}),
                    )
                    .await;
            }
        });

        let response = ReplayResponse {
            job_id: job_id.to_string(),
            status: ReplayStatus::Queued,
            observations_queued: total_observations,
            time_range: format!("{}..{}", body.from_date, body.to_date),
            estimated_duration_secs: estimate_duration(total_observations, recipe_count),
        };
        return (StatusCode::ACCEPTED, Json(success(response)));
    }

    match execute_replay_job(
        state.clone(),
        job_id,
        auth_ctx.user_id.clone(),
        body.clone(),
    )
    .await
    {
        Ok(progress) => (
            StatusCode::OK,
            Json(success(ReplayResponse {
                job_id: progress.job_id,
                status: progress.status,
                observations_queued: progress.total_observations,
                time_range: format!("{}..{}", body.from_date, body.to_date),
                estimated_duration_secs: estimate_duration(total_observations, recipe_count),
            })),
        ),
        Err(err) => {
            tracing::error!(job_id = %job_id, "replay failed: {err:#}");
            let _ = state
                .store
                .update_replay_job(
                    job_id,
                    ReplayStatus::Failed.as_str(),
                    0,
                    0,
                    0,
                    1,
                    Some(Utc::now()),
                )
                .await;
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal("Replay job failed"))),
            )
        }
    }
}

pub(crate) async fn get_replay_status(
    State(state): State<AppState>,
    Path(job_id): Path<String>,
) -> (StatusCode, Json<ApiResponse<ReplayProgress>>) {
    let job_id = match Uuid::parse_str(&job_id) {
        Ok(job_id) => job_id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(error_response(ApiError::bad_request(
                    "Invalid replay job id",
                ))),
            )
        }
    };

    match state.store.get_replay_job(job_id).await {
        Ok(Some(record)) => (
            StatusCode::OK,
            Json(success(replay_progress_from_record(record))),
        ),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(error_response(ApiError::not_found(
                "replay job",
                &job_id.to_string(),
            ))),
        ),
        Err(err) => {
            tracing::error!("get_replay_status failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to load replay status",
                ))),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::build_trigger_scan_response;

    #[test]
    fn test_build_trigger_scan_response_preserves_fields() {
        let response =
            build_trigger_scan_response("trigger-1".to_string(), "dns_posture_scan".to_string());

        assert_eq!(response.trigger_id, "trigger-1");
        assert_eq!(response.job_kind, "dns_posture_scan");
    }
}
