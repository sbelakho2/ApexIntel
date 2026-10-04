#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::*;

fn normalize_feedback_type(value: &str) -> Option<String> {
    let normalized = value.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "bookmarked" | "actioned" | "dismissed" | "false_positive" | "false_negative"
        | "true_positive" | "relevant" | "irrelevant" | "viewed" => Some(normalized),
        _ => None,
    }
}

fn resolve_bookmarked_by(bookmarked: Option<&str>, user_id: &str) -> Option<String> {
    if bookmarked == Some("true") {
        Some(user_id.to_string())
    } else {
        None
    }
}

fn should_diversify_feed(filters: &InsightListFilters) -> bool {
    filters.exclude_internal
        && filters.bookmarked_by.is_none()
        && filters.insight_types.is_empty()
        && filters.search.is_none()
        && filters.regions.is_empty()
        && filters.date_from.is_none()
        && filters.date_to.is_none()
}

pub(crate) async fn list_insights(
    State(state): State<AppState>,
    Extension(auth_ctx): Extension<ApiAuthContext>,
    Query(params): Query<ListInsightsQuery>,
) -> (
    StatusCode,
    Json<ApiResponse<PagedResponse<InsightResponse>>>,
) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let (page, per_page, _offset) = match validate_pagination(params.page, params.per_page) {
        Ok(value) => value,
        Err(err) => {
            return (
                StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::BAD_REQUEST),
                Json(error_response(err)),
            );
        }
    };

    let date_from = match parse_date_start(&params.date_from) {
        Ok(value) => value,
        Err(msg) => {
            let api_err = ApiError::bad_request(msg);
            return (StatusCode::BAD_REQUEST, Json(error_response(api_err)));
        }
    };
    let date_to = match parse_query_date(&params.date_to, "date_to") {
        Ok(value) => value,
        Err(api_err) => {
            return (StatusCode::BAD_REQUEST, Json(error_response(api_err)));
        }
    };

    let regions = parse_csv_upper_strict(params.regions.as_deref());
    if let Err(api_err) = validate_region_codes(&regions) {
        return (
            StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::BAD_REQUEST),
            Json(error_response(api_err)),
        );
    }

    let filters = InsightListFilters {
        regions,
        date_from,
        date_to,
        search: match params.search.as_deref() {
            Some(value) => match validate_search_text(value, 500) {
                Ok(value) => value,
                Err(msg) => {
                    let api_err = ApiError::validation("search", msg);
                    return (
                        StatusCode::from_u16(api_err.http_status())
                            .unwrap_or(StatusCode::BAD_REQUEST),
                        Json(error_response(api_err)),
                    );
                }
            },
            None => None,
        },
        insight_types: params
            .insight_type
            .as_deref()
            .filter(|value| !value.is_empty())
            .map(|value| vec![value.to_string()])
            .unwrap_or_default(),
        exclude_internal: true,
        bookmarked_by: resolve_bookmarked_by(params.bookmarked.as_deref(), &auth_ctx.user_id),
    };

    let total = match tracing::info_span!("db.count_insights", request_id = %request_id)
        .in_scope(|| state.store.count_insights(&filters))
        .await
    {
        Ok(value) => value.max(0) as u64,
        Err(err) => {
            tracing::error!(request_id = %request_id, "count insights failed: {err:#}");
            let api_err = ApiError::internal("Failed to count insights");
            return (
                StatusCode::from_u16(api_err.http_status())
                    .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(error_response(api_err)),
            );
        }
    };

    let clamped_page = clamp_page(page, per_page, total);
    let clamped_offset = ((clamped_page - 1) as i64).saturating_mul(per_page as i64);

    let diversify_feed = should_diversify_feed(&filters);
    let (candidate_limit, candidate_offset, diversified_start) = if diversify_feed {
        let window = (per_page as i64).saturating_mul(5).max(per_page as i64);
        let lookbehind = (per_page as i64).saturating_mul(2);
        let candidate_offset = clamped_offset.saturating_sub(lookbehind);
        (
            window,
            candidate_offset,
            (clamped_offset - candidate_offset) as usize,
        )
    } else {
        (per_page as i64, clamped_offset, 0)
    };

    let rows = match tracing::info_span!("db.list_insights", request_id = %request_id)
        .in_scope(|| {
            state
                .store
                .list_insights(&filters, candidate_limit, candidate_offset)
        })
        .await
    {
        Ok(value) => value,
        Err(err) => {
            tracing::error!(request_id = %request_id, "list insights failed: {err:#}");
            let api_err = ApiError::internal("Failed to list insights");
            return (
                StatusCode::from_u16(api_err.http_status())
                    .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(error_response(api_err)),
            );
        }
    };

    let mut items: Vec<InsightResponse> = rows.into_iter().map(insight_row_to_response).collect();
    let insight_uuids: Vec<Uuid> = items
        .iter()
        .filter_map(|item| Uuid::parse_str(&item.id).ok())
        .collect();
    if !insight_uuids.is_empty() {
        match state
            .store
            .get_bookmarked_insight_ids(&auth_ctx.user_id, &insight_uuids)
            .await
        {
            Ok(bookmarked_ids) => {
                let bookmarked: std::collections::HashSet<String> =
                    bookmarked_ids.iter().map(|id| id.to_string()).collect();
                for item in &mut items {
                    item.bookmarked = Some(bookmarked.contains(&item.id));
                }
            }
            Err(err) => {
                tracing::error!(request_id = %request_id, "get_bookmarked_insight_ids failed: {err:#}");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(error_response(ApiError::internal(
                        "Failed to load bookmark state",
                    ))),
                );
            }
        }
        let quality_scores = match state
            .store
            .get_insight_feedback_scores(&insight_uuids)
            .await
        {
            Ok(scores) => scores,
            Err(err) => {
                tracing::error!(request_id = %request_id, "get_insight_feedback_scores failed: {err:#}");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(error_response(ApiError::internal(
                        "Failed to load insight quality scores",
                    ))),
                );
            }
        };
        for item in &mut items {
            if let Ok(insight_id) = Uuid::parse_str(&item.id) {
                item.quality_score = quality_scores.get(&insight_id).copied();
            }
        }
        apex_api::routes::insights::rank_insights(&mut items);
    }

    if diversify_feed {
        let start = diversified_start;
        let end = (start + per_page as usize).min(items.len());
        items = if start < items.len() {
            items[start..end].to_vec()
        } else {
            Vec::new()
        };
    }

    let payload = PagedResponse {
        items,
        total,
        page: clamped_page,
        per_page,
    };
    let duration_ms = start.elapsed().as_millis() as u64;
    let meta = ResponseMeta::now()
        .with_request_id(request_id)
        .with_duration(duration_ms);
    log_latency("list_insights", duration_ms);

    (StatusCode::OK, Json(success_with_meta(payload, meta)))
}

pub(crate) async fn bookmark_insight(
    State(state): State<AppState>,
    Extension(auth_ctx): Extension<ApiAuthContext>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let uid = match Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(error_response(ApiError::bad_request("Invalid UUID"))),
            )
        }
    };
    match state.store.get_insight(uid).await {
        Ok(Some(_)) => {}
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(error_response(ApiError::not_found("Insight", &id))),
            )
        }
        Err(err) => {
            tracing::error!("bookmark lookup failed: {err:#}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to check insight",
                ))),
            );
        }
    }
    match state
        .store
        .bookmark_insight_scoped(uid, &auth_ctx.user_id, auth_ctx.role.as_str(), None)
        .await
    {
        Ok(created) => {
            // false-success-classification: best-effort — audit-trail write after the primary mutation succeeded
            let _ = state
                .store
                .record_insight_feedback(uid, &auth_ctx.user_id, "bookmarked", None)
                .await;
            // false-success-classification: best-effort — audit-trail write after the primary mutation succeeded
            let _ = state
                .store
                .record_audit_event(
                    &auth_ctx.user_id,
                    "insight_bookmarked",
                    &serde_json::json!({"insight_id": uid, "created": created}),
                )
                .await;
            let status = if created {
                "created"
            } else {
                "already_bookmarked"
            };
            let meta = ResponseMeta::now();
            (
                StatusCode::OK,
                Json(success_with_meta(
                    serde_json::json!({ "status": status, "insight_id": id, "bookmarked": true }),
                    meta,
                )),
            )
        }
        Err(err) => {
            tracing::error!("bookmark_insight failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to bookmark insight",
                ))),
            )
        }
    }
}

pub(crate) async fn record_insight_feedback(
    State(state): State<AppState>,
    Extension(auth_ctx): Extension<ApiAuthContext>,
    Path(id): Path<String>,
    Json(payload): Json<apex_api::routes::insights::InsightFeedbackRequest>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let uid = match Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(error_response(ApiError::bad_request("Invalid UUID"))),
            )
        }
    };
    let feedback_type = match normalize_feedback_type(&payload.feedback_type) {
        Some(value) => value,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(error_response(ApiError::validation(
                    "feedback_type",
                    "Unsupported insight feedback type",
                ))),
            )
        }
    };

    match state.store.get_insight(uid).await {
        Ok(Some(_)) => {}
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(error_response(ApiError::not_found("Insight", &id))),
            )
        }
        Err(err) => {
            tracing::error!("feedback lookup failed: {err:#}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to check insight",
                ))),
            );
        }
    }

    match state
        .store
        .record_insight_feedback(
            uid,
            &auth_ctx.user_id,
            &feedback_type,
            payload.notes.as_deref(),
        )
        .await
    {
        Ok(_) => {
            let _ = state
                .store
                .record_audit_event(
                    &auth_ctx.user_id,
                    "insight_feedback_recorded",
                    &serde_json::json!({
                        "insight_id": uid,
                        "feedback_type": feedback_type,
                        "notes": payload.notes,
                    }),
                )
                .await;
            let quality_scores = match state.store.get_insight_feedback_scores(&[uid]).await {
                Ok(scores) => scores,
                Err(err) => {
                    tracing::error!("feedback score load failed: {err:#}");
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(error_response(ApiError::internal(
                            "Failed to load insight quality score",
                        ))),
                    );
                }
            };
            let quality_score = quality_scores.get(&uid).copied();
            (
                StatusCode::OK,
                Json(success_with_meta(
                    serde_json::json!({
                        "status": "recorded",
                        "insight_id": id,
                        "feedback_type": feedback_type,
                        "quality_score": quality_score,
                    }),
                    ResponseMeta::now(),
                )),
            )
        }
        Err(err) => {
            tracing::error!("record_insight_feedback failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to record insight feedback",
                ))),
            )
        }
    }
}

pub(crate) async fn unbookmark_insight(
    State(state): State<AppState>,
    Extension(auth_ctx): Extension<ApiAuthContext>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let uid = match Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(error_response(ApiError::bad_request("Invalid UUID"))),
            )
        }
    };
    match state
        .store
        .unbookmark_insight_scoped(uid, &auth_ctx.user_id, auth_ctx.role.as_str())
        .await
    {
        Ok(_removed) => {
            // false-success-classification: best-effort — audit-trail write after the primary mutation succeeded
            let _ = state
                .store
                .record_audit_event(
                    &auth_ctx.user_id,
                    "insight_unbookmarked",
                    &serde_json::json!({"insight_id": uid}),
                )
                .await;
            let meta = ResponseMeta::now();
            (
                StatusCode::OK,
                Json(success_with_meta(
                    serde_json::json!({ "status": "removed", "insight_id": id, "bookmarked": false }),
                    meta,
                )),
            )
        }
        Err(err) => {
            tracing::error!("unbookmark_insight failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to unbookmark insight",
                ))),
            )
        }
    }
}

#[cfg_attr(not(feature = "llm"), allow(dead_code))]
fn strip_warning_label(s: &str) -> String {
    let patterns = [
        "THREAT:",
        "EVIDENCE:",
        "IMPACT:",
        "RESPONSE:",
        "1.",
        "2.",
        "3.",
        "4.",
    ];
    let mut result = s.to_string();
    for pattern in patterns {
        if result.starts_with(pattern) {
            result = result[pattern.len()..].trim().to_string();
            break;
        }
    }
    result
}

/// POST /api/insights/:id/analyze — enqueue a durable analysis run (#169).
///
/// The request never runs the model: it verifies the insight exists, inserts
/// a `queued` run (deduplicating onto an in-flight one) and the payload-bound
/// worker trigger in one transaction, then answers `202 Accepted` with
/// `{run_id, status}`. Poll `GET /api/insights/:id/analyze/latest` for the
/// status and the persisted result. The model call itself runs in the worker
/// (`JobKind::InsightAnalysis`), so a client disconnect can no longer discard
/// the analysis.
#[cfg(feature = "llm")]
pub(crate) async fn analyze_insight(
    State(state): State<AppState>,
    Extension(auth_ctx): Extension<ApiAuthContext>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let uid = match Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(error_response(ApiError::bad_request("Invalid UUID"))),
            )
        }
    };

    if state.llm.is_none() {
        return llm_service_unavailable("LLM not configured");
    }

    match state.store.get_insight(uid).await {
        Ok(Some(_)) => {}
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(error_response(ApiError::not_found("Insight", &id))),
            )
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "analyze_insight: get_insight failed: {err:#}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal("Failed to load insight"))),
            );
        }
    }

    match state
        .store
        .enqueue_insight_analysis(uid, Some(auth_ctx.user_id.as_str()))
        .await
    {
        Ok((run, deduplicated)) => {
            let duration_ms = start.elapsed().as_millis() as u64;
            log_latency("analyze_insight", duration_ms);
            (
                StatusCode::ACCEPTED,
                Json(success_with_meta(
                    serde_json::json!({
                        "run_id": run.id,
                        "insight_id": run.insight_id,
                        "status": run.status,
                        "deduplicated": deduplicated,
                    }),
                    ResponseMeta::now()
                        .with_request_id(request_id)
                        .with_duration(duration_ms),
                )),
            )
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "analyze_insight: enqueue failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to enqueue insight analysis",
                ))),
            )
        }
    }
}

/// GET /api/insights/:id/analyze/latest — the most recent durable
/// insight-analysis run for an insight, whatever its status (#169).
///
/// `result` carries the persisted analysis JSON once the run succeeded;
/// `error` carries the worker's recorded failure reason otherwise. Answers
/// 404 when the insight has never been analyzed.
pub(crate) async fn get_latest_insight_analysis_run(
    State(state): State<AppState>,
    Extension(_auth_ctx): Extension<ApiAuthContext>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let uid = match Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(error_response(ApiError::bad_request("Invalid UUID"))),
            )
        }
    };

    match state.store.get_latest_insight_analysis_run(uid).await {
        Ok(Some(run)) => (
            StatusCode::OK,
            Json(success_with_meta(
                insight_analysis_run_to_json(&run),
                ResponseMeta::now(),
            )),
        ),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(error_response(ApiError::not_found(
                "Insight analysis run",
                &id,
            ))),
        ),
        Err(err) => {
            tracing::error!("get_latest_insight_analysis_run failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to load insight analysis run",
                ))),
            )
        }
    }
}

/// Serialize a persisted run for the API. `result`/`error` are `null` until
/// the run reaches the corresponding terminal state.
fn insight_analysis_run_to_json(
    run: &apex_store::postgres::InsightAnalysisRunRow,
) -> serde_json::Value {
    serde_json::json!({
        "run_id": run.id,
        "insight_id": run.insight_id,
        "status": run.status,
        "requested_by": run.requested_by,
        "requested_at": run.requested_at,
        "started_at": run.started_at,
        "completed_at": run.completed_at,
        "result": run.result,
        "error": run.error,
    })
}

/// POST /api/insights/:id/investigate — open a deep investigation from an
/// insight's evidence using the `apex_investigation` analysis engine (ACH
/// hypothesis generation, chain-of-thought reasoning, threat modeling,
/// narrative synthesis). Returns a structured investigation result.
///
/// This wires the previously-orphaned `apex_investigation` crate into the live
/// system. The engine is pure CPU (no LLM call), so it works regardless of the
/// `llm` feature flag.
pub(crate) async fn investigate_insight(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let uid = match Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(error_response(ApiError::bad_request("Invalid UUID"))),
            )
        }
    };

    // Load the insight + its entities.
    let insight = match state.store.get_insight(uid).await {
        Ok(Some(i)) => i,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(error_response(ApiError::not_found("Insight", &id))),
            )
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "investigate_insight: get_insight failed: {err:#}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal("Failed to load insight"))),
            );
        }
    };

    let entity_ids: Vec<Uuid> = insight.entity_ids.clone().unwrap_or_default();
    let primary_entity_id = entity_ids.first().copied();

    let company_names = match state.store.get_company_names_by_ids(&entity_ids).await {
        Ok(names) => names,
        Err(err) => {
            tracing::error!(request_id = %request_id, "investigate_insight: get_company_names_by_ids failed: {err:#}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to load insight entities",
                ))),
            );
        }
    };
    let entity_name = company_names
        .first()
        .map(|(_, name, _, _)| name.clone())
        .unwrap_or_else(|| insight.title.clone());

    // Load recent observations for the primary entity as investigation evidence.
    let mut evidence_items: Vec<apex_investigation::reasoning::EvidenceItem> = Vec::new();
    if let Some(eid) = primary_entity_id {
        let obs = match state.store.get_observations_by_entity(eid, 30).await {
            Ok(obs) => obs,
            Err(err) => {
                tracing::error!(request_id = %request_id, "investigate_insight: get_observations_by_entity failed: {err:#}");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(error_response(ApiError::internal(
                        "Failed to load investigation evidence",
                    ))),
                );
            }
        };
        for (i, o) in obs.iter().enumerate() {
            evidence_items.push(apex_investigation::reasoning::EvidenceItem {
                id: format!("obs-{i}"),
                entity_id: eid.to_string(),
                entity_type: "company".to_string(),
                evidence_type: o.observation_type.clone(),
                description: value_as_text(&o.value),
                source: o
                    .provenance
                    .get("source")
                    .and_then(|v| v.as_str())
                    .unwrap_or("observation")
                    .to_string(),
                confidence: o.confidence.unwrap_or(0.7),
                timestamp: o.ts_utc,
                raw_data: o.value.clone(),
            });
        }
    }

    // Add the insight itself as a high-confidence evidence item.
    evidence_items.push(apex_investigation::reasoning::EvidenceItem {
        id: format!("insight-{uid}"),
        entity_id: primary_entity_id.map(|u| u.to_string()).unwrap_or_default(),
        entity_type: "company".to_string(),
        evidence_type: "intelligence_insight".to_string(),
        description: insight.summary.clone(),
        source: "insight_engine".to_string(),
        confidence: insight.confidence.unwrap_or(0.7),
        // Unknown row time stays the explicit epoch sentinel, never "now".
        timestamp: insight
            .created_at
            .unwrap_or(chrono::DateTime::<chrono::Utc>::UNIX_EPOCH),
        raw_data: serde_json::json!({
            "insight_type": insight.insight_type,
            "category": insight.insight_type,
        }),
    });

    // Run the investigation engine. #170: the engine is CPU-bound and can
    // block a runtime worker for the whole analysis, so it runs on a blocking
    // thread with owned inputs; a join failure is an internal error.
    let engine = apex_investigation::investigations::InvestigationEngine::new();
    let investigation_type = match insight.insight_type.as_deref() {
        Some("supply_chain_risk") => {
            apex_investigation::investigations::InvestigationType::SupplyChainAnalysis
        }
        Some("competitor_market") => {
            apex_investigation::investigations::InvestigationType::CompanyDeepDive
        }
        Some("geopolitical_analysis") => {
            apex_investigation::investigations::InvestigationType::GeopoliticalRisk
        }
        Some("security_compliance") | Some("cybersecurity_threat") => {
            apex_investigation::investigations::InvestigationType::ThreatAssessment
        }
        _ => apex_investigation::investigations::InvestigationType::CompanyDeepDive,
    };
    let investigation_type_label = format!("{investigation_type:?}");

    let investigation = engine.create_investigation(
        &format!("Investigation: {}", insight.title),
        investigation_type,
        &entity_name,
        "company",
    );

    let (investigation, result) =
        match run_investigation_blocking(investigation, evidence_items).await {
            Ok(value) => value,
            Err(join_error) => {
                tracing::error!(
                    request_id = %request_id,
                    "investigate_insight: investigation task failed: {join_error}"
                );
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(error_response(ApiError::internal(
                        "Investigation engine failed",
                    ))),
                );
            }
        };

    // Serialize the investigation result for the API response.
    let response = serde_json::json!({
        "investigation_id": investigation.id,
        "status": format!("{:?}", investigation.status),
        "priority": format!("{:?}", investigation.priority),
        "target_entity": entity_name,
        "investigation_type": investigation_type_label,
        "overall_confidence": result.overall_confidence,
        "hypothesis_count": result.hypotheses.len(),
        "gap_count": result.gaps.as_ref().map(|g| {
            g.financial_gaps.len() + g.operational_gaps.len() + g.leadership_gaps.len()
                + g.supply_chain_gaps.len()
        }).unwrap_or(0),
        "reasoning_chain_count": result.reasoning_chains.len(),
        "executive_summary": result.report.as_ref().map(|r| r.executive_summary.clone()),
        "recommendations": result.report.as_ref().map(|r| {
            r.recommendations.iter().map(|rec| serde_json::json!({
                "priority": format!("{:?}", rec.priority),
                "title": rec.title,
                "description": rec.description,
                "expected_impact": rec.expected_impact,
                "effort": format!("{:?}", rec.effort),
                "time_to_implement": rec.time_to_implement,
            })).collect::<Vec<_>>()
        }).unwrap_or_default(),
        "hypotheses": result.hypotheses.iter().take(5).map(|h| serde_json::json!({
            "label": h.label,
            "description": h.description,
            "prior": h.prior,
            "posterior": h.posterior,
            "supporting_evidence_types": h.supporting_evidence.iter().map(|e| format!("{e:?}")).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "threat_model": result.threat_model.as_ref().map(|tm| serde_json::json!({
            "overall_risk_score": tm.overall_risk_score,
            "tier1_supplier_count": tm.tier1_suppliers.len(),
            "single_source_components": tm.single_source_components.len(),
            "disruption_scenarios": tm.disruption_scenarios.len(),
        })),
    });

    let dur = start.elapsed().as_millis() as u64;
    log_latency("investigate_insight", dur);
    (
        StatusCode::OK,
        Json(success_with_meta(
            response,
            ResponseMeta::now()
                .with_request_id(request_id)
                .with_duration(dur),
        )),
    )
}

/// Run the CPU-bound `apex_investigation` engine on a blocking thread (#170).
///
/// The engine performs hypothesis generation, chain-of-thought reasoning and
/// narrative synthesis synchronously; running it directly on the async
/// runtime blocks a worker for the whole analysis. Inputs are moved into the
/// task and the mutated investigation is returned alongside the result.
async fn run_investigation_blocking(
    investigation: apex_investigation::investigations::Investigation,
    evidence_items: Vec<apex_investigation::reasoning::EvidenceItem>,
) -> Result<
    (
        apex_investigation::investigations::Investigation,
        apex_investigation::investigations::InvestigationResult,
    ),
    tokio::task::JoinError,
> {
    tokio::task::spawn_blocking(move || {
        let engine = apex_investigation::investigations::InvestigationEngine::new();
        let mut investigation = investigation;
        let result = engine.run_investigation(&mut investigation, evidence_items);
        (investigation, result)
    })
    .await
}

#[cfg(not(feature = "llm"))]
pub(crate) async fn analyze_insight(
    State(_state): State<AppState>,
    Path(_id): Path<String>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    llm_service_unavailable("LLM feature disabled")
}

/// Best-effort extraction of a human-readable text description from an
/// observation's `value` JSON field. Handles strings, objects with a `content`
/// / `description` / `text` key, and arrays by joining their string elements.
fn value_as_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Object(map) => {
            for key in &["content", "description", "text", "title", "summary"] {
                if let Some(v) = map.get(*key) {
                    if let Some(s) = v.as_str() {
                        if !s.trim().is_empty() {
                            return s.to_string();
                        }
                    }
                }
            }
            value.to_string()
        }
        serde_json::Value::Array(arr) => arr
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect::<Vec<_>>()
            .join("; "),
        _ => value.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{resolve_bookmarked_by, run_investigation_blocking, strip_warning_label};

    #[tokio::test]
    async fn investigation_engine_runs_off_the_async_runtime() {
        let engine = apex_investigation::investigations::InvestigationEngine::new();
        let investigation = engine.create_investigation(
            "Investigation: test",
            apex_investigation::investigations::InvestigationType::CompanyDeepDive,
            "Acme Corp",
            "company",
        );
        let expected_id = investigation.id.clone();

        let (investigation, result) = run_investigation_blocking(investigation, Vec::new())
            .await
            .expect("blocking investigation task must not panic");

        assert_eq!(investigation.id, expected_id);
        assert_eq!(result.investigation_id, expected_id);
    }

    #[test]
    fn test_strip_warning_label_removes_section_prefix() {
        assert_eq!(
            strip_warning_label("RESPONSE: Escalate to the risk team"),
            "Escalate to the risk team"
        );
    }

    #[test]
    fn test_resolve_bookmarked_by_only_accepts_true() {
        assert_eq!(
            resolve_bookmarked_by(Some("true"), "user-123"),
            Some("user-123".to_string())
        );
        assert_eq!(resolve_bookmarked_by(Some("false"), "user-123"), None);
        assert_eq!(resolve_bookmarked_by(None, "user-123"), None);
    }
}
