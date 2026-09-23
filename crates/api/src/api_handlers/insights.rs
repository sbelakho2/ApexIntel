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
        if let Ok(bookmarked_ids) = state
            .store
            .get_bookmarked_insight_ids(&auth_ctx.user_id, &insight_uuids)
            .await
        {
            let bookmarked: std::collections::HashSet<String> =
                bookmarked_ids.iter().map(|id| id.to_string()).collect();
            for item in &mut items {
                item.bookmarked = Some(bookmarked.contains(&item.id));
            }
        }
        if let Ok(quality_scores) = state
            .store
            .get_insight_feedback_scores(&insight_uuids)
            .await
        {
            for item in &mut items {
                if let Ok(insight_id) = Uuid::parse_str(&item.id) {
                    item.quality_score = Some(*quality_scores.get(&insight_id).unwrap_or(&0.5));
                }
            }
            apex_api::routes::insights::rank_insights(&mut items);
        }
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
        .bookmark_insight(uid, &auth_ctx.user_id, None)
        .await
    {
        Ok(created) => {
            let _ = state
                .store
                .record_insight_feedback(uid, &auth_ctx.user_id, "bookmarked", None)
                .await;
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
            let quality_score = state
                .store
                .get_insight_feedback_scores(&[uid])
                .await
                .ok()
                .and_then(|scores| scores.get(&uid).copied())
                .unwrap_or(0.5);
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
    match state.store.unbookmark_insight(uid, &auth_ctx.user_id).await {
        Ok(_removed) => {
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
fn strip_analysis_label(s: &str) -> String {
    let mut result = s.to_string();
    if result.len() > 2 && result.as_bytes()[0].is_ascii_digit() && result.as_bytes()[1] == b'.' {
        result = result[2..].trim().to_string();
    }
    let labels = [
        "SITUATION:",
        "ANALYSIS:",
        "RISK:",
        "ACTION:",
        "THREAT:",
        "EVIDENCE:",
        "IMPACT:",
        "RESPONSE:",
        "**SITUATION:**",
        "**ANALYSIS:**",
        "**RISK:**",
        "**ACTION:**",
    ];
    for label in labels {
        if result.starts_with(label) {
            result = result[label.len()..].trim().to_string();
            break;
        }
    }
    result
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

#[cfg(feature = "llm")]
pub(crate) async fn analyze_insight(
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

    let runtime = match state.llm.as_ref() {
        Some(rt) => rt,
        None => return llm_service_unavailable("LLM not configured"),
    };

    let insight = match state.store.get_insight(uid).await {
        Ok(Some(i)) => i,
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
    };

    let entity_ids: Vec<Uuid> = insight.entity_ids.clone().unwrap_or_default();
    let company_names = state
        .store
        .get_company_names_by_ids(&entity_ids)
        .await
        .unwrap_or_default();
    let entity_names: Vec<String> = company_names
        .iter()
        .map(|(_, name, _, _)| name.clone())
        .collect();

    let mut all_observations = Vec::new();
    for eid in &entity_ids {
        let obs = state
            .store
            .get_observations_by_entity(*eid, 30)
            .await
            .unwrap_or_default();
        all_observations.extend(obs);
    }
    all_observations.sort_by_key(|a| std::cmp::Reverse(a.ts_utc));
    all_observations.truncate(40);

    {
        let mut seen_types = std::collections::HashSet::new();
        all_observations.retain(|obs| {
            let key = format!(
                "{}:{}",
                obs.observation_type,
                obs.value.to_string().chars().take(80).collect::<String>()
            );
            seen_types.insert(key)
        });
    }

    let related_warnings = state
        .store
        .get_warnings_by_entity_ids(&entity_ids, 10)
        .await
        .unwrap_or_default();
    let related_insights = state
        .store
        .get_related_insights(&entity_ids, uid, 5)
        .await
        .unwrap_or_default();
    let evidence_urls = insight.evidence_urls.clone().unwrap_or_default();
    let source_count = evidence_urls.len();

    let mut context_parts: Vec<String> = Vec::new();
    if !all_observations.is_empty() {
        context_parts.push(format!("DATA ({} observations):", all_observations.len()));
        for (i, obs) in all_observations.iter().take(6).enumerate() {
            let text = obs
                .value
                .get("excerpt")
                .or(obs.value.get("text"))
                .or(obs.value.get("summary"))
                .or(obs.value.get("title"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let truncated: String = text.chars().take(100).collect();
            context_parts.push(format!(
                "[O{}] {} — {}",
                i + 1,
                obs.observation_type,
                truncated
            ));
        }
    }

    for (i, warning) in related_warnings.iter().take(3).enumerate() {
        context_parts.push(format!(
            "[W{}] {} ({})",
            i + 1,
            warning.title,
            warning.severity
        ));
    }

    let context_block = context_parts.join("\n");
    let entity_names_str = if entity_names.is_empty() {
        "unspecified entities".to_string()
    } else {
        entity_names.join(", ")
    };
    let region = insight.region.as_deref().unwrap_or("Global");
    let insight_type = insight.insight_type.as_deref().unwrap_or("general");
    let system_prompt = concat!(
        "You are a senior OSINT intelligence analyst specializing in electronics, defense, and supply chains. ",
        "Write a brief analytical report. Be specific — name companies, products, events, and dates. ",
        "Cite data references like [O1], [W1] where relevant."
    );

    let summary_truncated: String = insight.summary.chars().take(400).collect();
    let user_prompt = format!(
        r#"Write a 4-paragraph intelligence analysis.

SUBJECT: {entities} ({insight_type}, {region})
CONFIDENCE: {confidence:.0}%

BRIEFING: {summary}

{context}

Write EXACTLY 4 paragraphs, each on a new line:
1. SITUATION: What is happening and why it matters (3-4 sentences)
2. ANALYSIS: What the data tells us — correlate observations, identify patterns (3-4 sentences)
3. RISK: What could go wrong, which sectors are affected, timeline (2-3 sentences)
4. ACTION: Specific recommendations and what to monitor (2-3 sentences)"#,
        entities = entity_names_str,
        insight_type = insight_type,
        region = region,
        confidence = insight.confidence.unwrap_or(0.0) * 100.0,
        summary = summary_truncated,
        context = context_block,
    );

    let mut model_config = runtime.primary.clone();
    model_config.temperature = 0.5;
    model_config.max_tokens = 1024;
    model_config.timeout_seconds = 300;
    let client = OpenAiCompatibleClient::new(model_config);

    let raw_analysis = match client.generate_text(system_prompt, &user_prompt).await {
        Ok(text) => text,
        Err(err) => {
            tracing::error!(request_id = %request_id, "LLM analysis failed: {err:#}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal("LLM analysis failed"))),
            );
        }
    };

    let paragraphs: Vec<String> = {
        let double_split: Vec<&str> = raw_analysis
            .split("\n\n")
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();
        if double_split.len() >= 4 {
            double_split
                .into_iter()
                .map(|s| s.replace('\n', " "))
                .collect()
        } else {
            raw_analysis
                .split('\n')
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect()
        }
    };

    let exec_summary = strip_analysis_label(
        paragraphs
            .first()
            .map(|s| s.as_str())
            .unwrap_or("Analysis unavailable."),
    );
    let detailed = strip_analysis_label(paragraphs.get(1).map(|s| s.as_str()).unwrap_or(""));
    let risk_assessment_text =
        strip_analysis_label(paragraphs.get(2).map(|s| s.as_str()).unwrap_or(""));
    let recommendations_text =
        strip_analysis_label(paragraphs.get(3).map(|s| s.as_str()).unwrap_or(""));

    let risk_lower = risk_assessment_text.to_lowercase();
    let risk_level = if risk_lower.contains("critical") || risk_lower.contains("severe") {
        "critical"
    } else if risk_lower.contains("high") || risk_lower.contains("significant") {
        "high"
    } else if risk_lower.contains("low") || risk_lower.contains("minimal") {
        "low"
    } else {
        "medium"
    };

    let source_diversity = if source_count >= 8 {
        "excellent"
    } else if source_count >= 5 {
        "good"
    } else if source_count >= 3 {
        "moderate"
    } else {
        "limited"
    };

    let data_sufficiency = if source_count >= 6 && all_observations.len() >= 5 {
        "strong"
    } else if source_count >= 3 {
        "adequate"
    } else if source_count >= 1 {
        "limited"
    } else {
        "insufficient"
    };

    let overall_confidence = insight.confidence.unwrap_or(0.0);

    let analysis = serde_json::json!({
        "executive_summary": exec_summary,
        "key_findings": [{
            "finding": detailed.chars().take(200).collect::<String>(),
            "evidence": format!("{} observations, {} sources", all_observations.len(), source_count),
            "confidence": overall_confidence,
            "impact": risk_level,
        }],
        "detailed_analysis": detailed,
        "source_analysis": {
            "total_sources": source_count,
            "observation_signals": all_observations.len(),
            "corroborating_sources": std::cmp::max(1, source_count.saturating_sub(1)),
            "contradicting_signals": 0,
            "source_diversity_assessment": source_diversity,
        },
        "risk_assessment": {
            "overall_risk": risk_level,
            "probability": overall_confidence,
            "time_horizon": "near_term",
            "affected_sectors": entity_names,
            "escalation_potential": risk_assessment_text,
        },
        "correlations": if detailed.len() > 10 { vec![detailed.clone()] } else { vec![] },
        "recommendations": [{
            "action": recommendations_text.chars().take(200).collect::<String>(),
            "priority": if risk_level == "critical" || risk_level == "high" { "high" } else { "medium" },
            "rationale": format!("Based on {} observations from {} sources at {:.0}% confidence",
                all_observations.len(), source_count, overall_confidence * 100.0),
        }],
        "monitoring_indicators": if !recommendations_text.is_empty() {
            vec![recommendations_text.clone()]
        } else {
            vec![format!("Monitor {} for further developments", entity_names_str)]
        },
        "analytical_confidence": {
            "overall": overall_confidence,
            "data_sufficiency": data_sufficiency,
            "key_uncertainties": [],
        },
    });

    let result = serde_json::json!({
        "insight_id": insight.id,
        "insight_title": insight.title,
        "analysis": analysis,
        "context_used": {
            "source_count": source_count,
            "observation_count": all_observations.len(),
            "warning_count": related_warnings.len(),
            "related_insight_count": related_insights.len(),
            "entity_count": entity_ids.len(),
        }
    });

    let dur = start.elapsed().as_millis() as u64;
    log_latency("analyze_insight", dur);
    (
        StatusCode::OK,
        Json(success_with_meta(
            result,
            ResponseMeta::now()
                .with_request_id(request_id)
                .with_duration(dur),
        )),
    )
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
    let company_names = state
        .store
        .get_company_names_by_ids(&entity_ids)
        .await
        .unwrap_or_default();
    let entity_name = company_names
        .first()
        .map(|(_, name, _, _)| name.clone())
        .unwrap_or_else(|| insight.title.clone());

    // Load recent observations for the primary entity as investigation evidence.
    let mut evidence_items: Vec<apex_investigation::reasoning::EvidenceItem> = Vec::new();
    if let Some(eid) = primary_entity_id {
        let obs = state
            .store
            .get_observations_by_entity(eid, 30)
            .await
            .unwrap_or_default();
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
        timestamp: insight.created_at.unwrap_or_else(chrono::Utc::now),
        raw_data: serde_json::json!({
            "insight_type": insight.insight_type,
            "category": insight.insight_type,
        }),
    });

    // Run the investigation engine.
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

    let mut investigation = engine.create_investigation(
        &format!("Investigation: {}", insight.title),
        investigation_type,
        &entity_name,
        "company",
    );

    let result = engine.run_investigation(&mut investigation, evidence_items);

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

#[cfg(not(feature = "llm"))]
pub(crate) async fn analyze_insight(
    State(_state): State<AppState>,
    Path(_id): Path<String>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    llm_service_unavailable("LLM feature disabled")
}

#[cfg(feature = "llm")]
pub(crate) async fn analyze_warning(
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

    let runtime = match state.llm.as_ref() {
        Some(rt) => rt,
        None => return llm_service_unavailable("LLM not configured"),
    };

    let warning = match state.store.get_warning(uid).await {
        Ok(Some(w)) => w,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(error_response(ApiError::not_found("Warning", &id))),
            )
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "analyze_warning: get_warning failed: {err:#}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal("Failed to load warning"))),
            );
        }
    };

    let entity_ids: Vec<Uuid> = warning.entity_ids.clone().unwrap_or_default();
    let company_names = state
        .store
        .get_company_names_by_ids(&entity_ids)
        .await
        .unwrap_or_default();
    let entity_names: Vec<String> = company_names
        .iter()
        .map(|(_, name, _, _)| name.clone())
        .collect();

    let mut all_observations = Vec::new();
    for eid in &entity_ids {
        let obs = state
            .store
            .get_observations_by_entity(*eid, 30)
            .await
            .unwrap_or_default();
        all_observations.extend(obs);
    }
    all_observations.sort_by_key(|a| std::cmp::Reverse(a.ts_utc));
    all_observations.truncate(40);

    {
        let mut seen_types = std::collections::HashSet::new();
        all_observations.retain(|obs| {
            let key = format!(
                "{}:{}",
                obs.observation_type,
                obs.value.to_string().chars().take(80).collect::<String>()
            );
            seen_types.insert(key)
        });
    }

    let related_insights = state
        .store
        .get_insights_by_entity_ids(&entity_ids, 10)
        .await
        .unwrap_or_default();
    let source_urls = warning.source_urls.clone().unwrap_or_default();
    let source_count = source_urls.len();

    let mut context_parts: Vec<String> = Vec::new();
    if !all_observations.is_empty() {
        context_parts.push(format!("DATA ({} observations):", all_observations.len()));
        for (i, obs) in all_observations.iter().take(6).enumerate() {
            let text = obs
                .value
                .get("excerpt")
                .or(obs.value.get("text"))
                .or(obs.value.get("summary"))
                .or(obs.value.get("title"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let truncated: String = text.chars().take(100).collect();
            context_parts.push(format!(
                "[O{}] {} — {}",
                i + 1,
                obs.observation_type,
                truncated
            ));
        }
    }

    for (i, related_insight) in related_insights.iter().take(3).enumerate() {
        context_parts.push(format!("[I{}] {}", i + 1, related_insight.title));
    }

    let context_block = context_parts.join("\n");
    let entity_names_str = if entity_names.is_empty() {
        "unspecified entities".to_string()
    } else {
        entity_names.join(", ")
    };
    let region = warning.region.as_deref().unwrap_or("Global");
    let system_prompt = concat!(
        "You are a senior threat analyst specializing in electronics, defense, and supply chains. ",
        "Write a brief threat assessment. Be specific — name companies, products, events, and dates. ",
        "Cite data references like [O1], [I1] where relevant."
    );

    let desc_truncated: String = warning
        .description
        .as_deref()
        .unwrap_or("")
        .chars()
        .take(300)
        .collect();
    let user_prompt = format!(
        r#"Write a 4-paragraph threat assessment.

WARNING: {title} ({severity} severity)
Type: {warning_type} | Region: {region} | Confidence: {confidence:.0}%
Entities: {entities}

DESCRIPTION: {description}

{context}

Write EXACTLY 4 paragraphs, each on a new line:
1. THREAT: What the threat is and who is affected (3-4 sentences)
2. EVIDENCE: What data supports this assessment, citing observations (3-4 sentences)
3. IMPACT: Business, operational, and financial consequences with timeline (2-3 sentences)
4. RESPONSE: Specific mitigation actions and escalation triggers (2-3 sentences)"#,
        title = warning.title,
        warning_type = warning.warning_type,
        severity = warning.severity,
        region = region,
        confidence = warning.confidence.unwrap_or(0.0) * 100.0,
        description = desc_truncated,
        entities = entity_names_str,
        context = context_block,
    );

    let mut model_config = runtime.primary.clone();
    model_config.temperature = 0.5;
    model_config.max_tokens = 1024;
    model_config.timeout_seconds = 300;
    let client = OpenAiCompatibleClient::new(model_config);

    let raw_analysis = match client.generate_text(system_prompt, &user_prompt).await {
        Ok(text) => text,
        Err(err) => {
            tracing::error!(request_id = %request_id, "LLM warning analysis failed: {err:#}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal("LLM analysis failed"))),
            );
        }
    };

    let paragraphs: Vec<&str> = raw_analysis
        .split('\n')
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();

    let threat_text = strip_warning_label(paragraphs.first().unwrap_or(&"Assessment unavailable."));
    let evidence_text = strip_warning_label(paragraphs.get(1).unwrap_or(&""));
    let impact_text = strip_warning_label(paragraphs.get(2).unwrap_or(&""));
    let response_text = strip_warning_label(paragraphs.get(3).unwrap_or(&""));

    let source_reliability = if source_count >= 5 {
        "high"
    } else if source_count >= 2 {
        "medium"
    } else {
        "low"
    };

    let overall_confidence = warning.confidence.unwrap_or(0.0);
    let data_sufficiency = if source_count >= 4 && all_observations.len() >= 3 {
        "strong"
    } else if source_count >= 2 {
        "adequate"
    } else {
        "limited"
    };

    let analysis = serde_json::json!({
        "threat_assessment": threat_text,
        "severity_justification": evidence_text,
        "key_indicators": [{
            "indicator": evidence_text.chars().take(150).collect::<String>(),
            "evidence": format!("{} observations, {} sources", all_observations.len(), source_count),
            "severity_contribution": warning.severity,
        }],
        "detailed_analysis": format!("{} {}", threat_text, evidence_text),
        "source_analysis": {
            "total_sources": source_count,
            "observation_signals": all_observations.len(),
            "corroborating_sources": std::cmp::max(1, source_count.saturating_sub(1)),
            "source_reliability": source_reliability,
        },
        "impact_assessment": {
            "business_impact": impact_text.chars().take(200).collect::<String>(),
            "operational_impact": impact_text,
            "financial_exposure": if warning.severity == "critical" { "high" } else { "moderate" },
            "timeline": "near_term",
        },
        "response_plan": [{
            "action": response_text.chars().take(200).collect::<String>(),
            "priority": if warning.severity == "critical" { "critical" } else { "high" },
            "owner": "security/risk team",
            "rationale": format!("Based on {} severity warning at {:.0}% confidence",
                warning.severity, overall_confidence * 100.0),
        }],
        "escalation_criteria": if !response_text.is_empty() { vec![response_text.clone()] } else { vec![] },
        "monitoring_indicators": if !impact_text.is_empty() {
            vec![format!("Monitor for: {}", impact_text.chars().take(100).collect::<String>())]
        } else {
            vec![format!("Monitor {} for further developments", entity_names_str)]
        },
        "analytical_confidence": {
            "overall": overall_confidence,
            "data_sufficiency": data_sufficiency,
            "key_assumptions": if paragraphs.len() > 4 {
                paragraphs[4..].iter().map(|s| strip_warning_label(s)).collect::<Vec<_>>()
            } else {
                vec![]
            },
        },
    });

    let result = serde_json::json!({
        "warning_id": warning.id,
        "warning_title": warning.title,
        "analysis": analysis,
        "context_used": {
            "source_count": source_count,
            "observation_count": all_observations.len(),
            "related_insight_count": related_insights.len(),
            "entity_count": entity_ids.len(),
        }
    });

    let dur = start.elapsed().as_millis() as u64;
    log_latency("analyze_warning", dur);
    (
        StatusCode::OK,
        Json(success_with_meta(
            result,
            ResponseMeta::now()
                .with_request_id(request_id)
                .with_duration(dur),
        )),
    )
}

#[cfg(not(feature = "llm"))]
pub(crate) async fn analyze_warning(
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
    use super::{resolve_bookmarked_by, strip_analysis_label, strip_warning_label};

    #[test]
    fn test_strip_analysis_label_removes_numbered_prefix() {
        assert_eq!(
            strip_analysis_label("1. SITUATION: Supply risk rising"),
            "Supply risk rising"
        );
    }

    #[test]
    fn test_strip_analysis_label_removes_markdown_prefix() {
        assert_eq!(
            strip_analysis_label("**ACTION:** Monitor inventory"),
            "Monitor inventory"
        );
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
