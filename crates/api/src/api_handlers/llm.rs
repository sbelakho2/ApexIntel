use super::super::*;

#[cfg(feature = "llm")]
use apex_llm::prompt_registry::{workflow_prompt, PromptKey, RegisteredPrompt};

#[cfg(feature = "llm")]
use apex_api::routes::llm::LlmGovernanceMetadata;

#[cfg(feature = "llm")]
#[derive(serde::Deserialize, serde::Serialize)]
struct PoiPayload {
    summary: String,
    roles: Vec<String>,
    affiliations: Vec<String>,
    key_facts: Vec<String>,
    risk_indicators: Vec<String>,
}

#[cfg(feature = "llm")]
#[derive(serde::Deserialize, serde::Serialize)]
struct MemoPayload {
    title: String,
    executive_summary: String,
    sections: Vec<MemoSection>,
    recommendations: Vec<String>,
}

#[cfg(feature = "llm")]
fn governance_metadata(
    prompt: RegisteredPrompt,
    validation_issues: Vec<String>,
) -> LlmGovernanceMetadata {
    LlmGovernanceMetadata {
        workflow: prompt.workflow.to_string(),
        prompt_id: prompt.prompt_id.to_string(),
        prompt_version: prompt.version.to_string(),
        quality_gate_passed: validation_issues.is_empty(),
        validation_issues,
    }
}

#[cfg(feature = "llm")]
fn serialize_payload<T: serde::Serialize>(payload: &T) -> JsonValue {
    serde_json::to_value(payload).unwrap_or_else(|_| serde_json::json!({}))
}

#[cfg(feature = "llm")]
async fn ensure_prompt_registered(state: &AppState, prompt: RegisteredPrompt) {
    if let Err(err) = state
        .store
        .ensure_prompt_version(
            prompt.prompt_id,
            prompt.version,
            prompt.workflow,
            prompt.system_prompt,
            &prompt.metadata_value(),
        )
        .await
    {
        tracing::warn!(
            workflow = prompt.workflow,
            prompt_id = prompt.prompt_id,
            prompt_version = prompt.version,
            error = %err,
            "llm prompt registration failed"
        );
    }
}

#[cfg(feature = "llm")]
async fn persist_workflow_run(
    state: &AppState,
    prompt: RegisteredPrompt,
    model_name: &str,
    request_payload: &JsonValue,
    response_payload: &JsonValue,
    validation_issues: &[String],
    duration_ms: u64,
) {
    if let Err(err) = state
        .store
        .record_llm_workflow_run(
            prompt.workflow,
            prompt.prompt_id,
            prompt.version,
            model_name,
            request_payload,
            response_payload,
            validation_issues,
            validation_issues.is_empty(),
            duration_ms as i64,
        )
        .await
    {
        tracing::warn!(
            workflow = prompt.workflow,
            prompt_id = prompt.prompt_id,
            prompt_version = prompt.version,
            error = %err,
            "failed to persist llm workflow run"
        );
    }
}

#[cfg(feature = "llm")]
fn parse_entities_value(value: JsonValue) -> Result<Vec<ExtractedEntity>, String> {
    let entities_value = if value.is_array() {
        value
    } else {
        value
            .get("entities")
            .or_else(|| value.get("items"))
            .or_else(|| value.get("data"))
            .cloned()
            .or_else(|| {
                value
                    .as_object()
                    .and_then(|map| map.values().find(|entry| entry.is_array()).cloned())
            })
            .ok_or_else(|| "Missing 'entities' array in response".to_string())?
    };

    let array = entities_value
        .as_array()
        .ok_or_else(|| "Entities payload is not an array".to_string())?;

    let mut entities = Vec::with_capacity(array.len());
    for (idx, item) in array.iter().enumerate() {
        if let Some(name) = item.as_str() {
            entities.push(ExtractedEntity {
                name: name.to_string(),
                entity_type: "unknown".to_string(),
                confidence: 0.5,
                span_start: None,
                span_end: None,
                canonical: None,
            });
            continue;
        }

        let obj = item
            .as_object()
            .ok_or_else(|| format!("entities[{}] must be an object or string", idx))?;

        let name = obj
            .get("name")
            .and_then(|entry| entry.as_str())
            .ok_or_else(|| format!("entities[{}] missing field 'name'", idx))?;
        let entity_type = obj
            .get("entity_type")
            .or_else(|| obj.get("type"))
            .and_then(|entry| entry.as_str())
            .unwrap_or("unknown");
        let confidence = obj
            .get("confidence")
            .and_then(|entry| entry.as_f64())
            .filter(|entry| entry.is_finite())
            .unwrap_or(0.5);
        let span_start = obj
            .get("span_start")
            .and_then(|entry| entry.as_u64())
            .map(|entry| entry as usize);
        let span_end = obj
            .get("span_end")
            .and_then(|entry| entry.as_u64())
            .map(|entry| entry as usize);
        let canonical = obj
            .get("canonical")
            .and_then(|entry| entry.as_str())
            .map(|entry| entry.to_string());

        entities.push(ExtractedEntity {
            name: name.to_string(),
            entity_type: entity_type.to_string(),
            confidence,
            span_start,
            span_end,
            canonical,
        });
    }

    Ok(entities)
}

#[cfg(feature = "llm")]
fn validate_entities_output(entities: &[ExtractedEntity]) -> Vec<String> {
    let mut issues = Vec::new();
    if entities.is_empty() {
        issues.push("entities must not be empty".to_string());
    }

    for (idx, entity) in entities.iter().enumerate() {
        if entity.name.trim().is_empty() {
            issues.push(format!("entities[{idx}] name must not be empty"));
        }
        if !(0.0..=1.0).contains(&entity.confidence) {
            issues.push(format!(
                "entities[{idx}] confidence must be between 0 and 1"
            ));
        }
        if let (Some(start), Some(end)) = (entity.span_start, entity.span_end) {
            if end < start {
                issues.push(format!("entities[{idx}] span_end must be >= span_start"));
            }
        }
    }

    issues
}

#[cfg(feature = "llm")]
fn validate_recipe_output(value: &JsonValue, existing_ids: &[String]) -> Vec<String> {
    let mut issues = validators::validate_recipe_json(value);
    let existing_refs = existing_ids.iter().map(String::as_str).collect::<Vec<_>>();
    if !validators::check_unique_id(value, &existing_refs) {
        issues.push("recipe id must be unique and non-empty".to_string());
    }
    if let Some(narrative) = value
        .get("narrative_template")
        .and_then(|entry| entry.as_str())
    {
        issues.extend(
            validators::check_content_quality(narrative)
                .into_iter()
                .map(|issue| format!("narrative_template: {issue}")),
        );
    }
    if let Some(playbook) = value
        .get("action_playbook")
        .and_then(|entry| entry.as_str())
    {
        issues.extend(
            validators::check_content_quality(playbook)
                .into_iter()
                .map(|issue| format!("action_playbook: {issue}")),
        );
    }
    issues
}

#[cfg(feature = "llm")]
fn validate_poi_output(payload: &PoiPayload) -> Vec<String> {
    let mut issues = validators::check_content_quality(&payload.summary)
        .into_iter()
        .map(|issue| format!("summary: {issue}"))
        .collect::<Vec<_>>();

    if payload.roles.is_empty() {
        issues.push("roles must not be empty".to_string());
    }
    if payload.affiliations.is_empty() {
        issues.push("affiliations must not be empty".to_string());
    }
    if payload.key_facts.is_empty() {
        issues.push("key_facts must not be empty".to_string());
    }
    if payload.risk_indicators.is_empty() {
        issues.push("risk_indicators must not be empty".to_string());
    }

    issues
}

#[cfg(feature = "llm")]
fn validate_memo_output(payload: &MemoPayload) -> Vec<String> {
    let mut issues = validators::check_content_quality(&payload.executive_summary)
        .into_iter()
        .map(|issue| format!("executive_summary: {issue}"))
        .collect::<Vec<_>>();

    if payload.title.trim().is_empty() {
        issues.push("title must not be empty".to_string());
    }
    if payload.sections.is_empty() {
        issues.push("sections must not be empty".to_string());
    }
    if payload.recommendations.is_empty() {
        issues.push("recommendations must not be empty".to_string());
    }

    for (idx, section) in payload.sections.iter().enumerate() {
        if section.heading.trim().is_empty() {
            issues.push(format!("sections[{idx}].heading must not be empty"));
        }
        issues.extend(
            validators::check_content_quality(&section.content)
                .into_iter()
                .map(|issue| format!("sections[{idx}].content: {issue}")),
        );
    }

    issues
}

#[cfg(feature = "llm")]
pub(crate) async fn llm_extract_entities(
    State(state): State<AppState>,
    Json(payload): Json<ExtractEntitiesRequest>,
) -> (StatusCode, Json<ApiResponse<ExtractEntitiesResponse>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();

    let issues = payload.validate();
    if !issues.is_empty() {
        let api_err = ApiError::validation("payload", issues.join("; "));
        return (
            StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::UNPROCESSABLE_ENTITY),
            Json(error_response(api_err)),
        );
    }

    let runtime = match state.llm.as_ref() {
        Some(runtime) => runtime,
        None => return llm_service_unavailable("LLM not configured"),
    };

    let client = OpenAiCompatibleClient::new(runtime.lightweight.clone());
    let prompt = workflow_prompt(PromptKey::EntityExtraction);
    ensure_prompt_registered(&state, prompt).await;

    let mut user = format!("Text:\n{}\n", payload.text);
    if let Some(doc_type) = &payload.doc_type {
        user.push_str(&format!("Doc type: {}\n", doc_type));
    }
    if let Some(types) = &payload.entity_types {
        user.push_str(&format!("Entity types: {:?}\n", types));
    }
    user.push_str("Return JSON only.");
    let request_payload = serialize_payload(&payload);

    let raw = match client.generate_json(prompt.system_prompt, &user).await {
        Ok(value) => value,
        Err(err) => {
            tracing::error!(request_id = %request_id, "LLM entity extraction failed: {err:#}");
            let api_err = ApiError::internal("LLM entity extraction failed");
            return (
                StatusCode::from_u16(api_err.http_status())
                    .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(error_response(api_err)),
            );
        }
    };

    let value = match validators::parse_json_response(&raw) {
        Ok(value) => value,
        Err(_) => {
            let strict_user = format!("{}\nSTRICT JSON ONLY. No trailing text.", user);
            let retry_raw = match client
                .generate_json(prompt.system_prompt, &strict_user)
                .await
            {
                Ok(value) => value,
                Err(err) => {
                    tracing::error!(request_id = %request_id, "LLM entity extraction retry failed: {err:#}");
                    let api_err = ApiError::internal("LLM entity extraction failed");
                    return (
                        StatusCode::from_u16(api_err.http_status())
                            .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                        Json(error_response(api_err)),
                    );
                }
            };
            match validators::parse_json_response(&retry_raw) {
                Ok(value) => value,
                Err(err) => {
                    let duration_ms = start.elapsed().as_millis() as u64;
                    let validation_issues = vec![err.clone()];
                    persist_workflow_run(
                        &state,
                        prompt,
                        &client.config().model_name,
                        &request_payload,
                        &serde_json::json!({"raw": retry_raw}),
                        &validation_issues,
                        duration_ms,
                    )
                    .await;
                    let api_err = ApiError::validation("llm", err);
                    return (
                        StatusCode::from_u16(api_err.http_status())
                            .unwrap_or(StatusCode::UNPROCESSABLE_ENTITY),
                        Json(error_response(api_err)),
                    );
                }
            }
        }
    };

    let entities = match parse_entities_value(value.clone()) {
        Ok(value) => value,
        Err(err) => {
            let duration_ms = start.elapsed().as_millis() as u64;
            let validation_issues = vec![err.clone()];
            persist_workflow_run(
                &state,
                prompt,
                &client.config().model_name,
                &request_payload,
                &serde_json::json!({"parsed": value}),
                &validation_issues,
                duration_ms,
            )
            .await;
            let api_err = ApiError::validation("entities", err);
            return (
                StatusCode::from_u16(api_err.http_status())
                    .unwrap_or(StatusCode::UNPROCESSABLE_ENTITY),
                Json(error_response(api_err)),
            );
        }
    };

    let validation_issues = validate_entities_output(&entities);
    if !validation_issues.is_empty() {
        let duration_ms = start.elapsed().as_millis() as u64;
        persist_workflow_run(
            &state,
            prompt,
            &client.config().model_name,
            &request_payload,
            &serde_json::json!({"entities": entities}),
            &validation_issues,
            duration_ms,
        )
        .await;
        let api_err = ApiError::validation("entities", validation_issues.join("; "));
        return (
            StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::UNPROCESSABLE_ENTITY),
            Json(error_response(api_err)),
        );
    }

    let duration_ms = start.elapsed().as_millis() as u64;
    persist_workflow_run(
        &state,
        prompt,
        &client.config().model_name,
        &request_payload,
        &serde_json::json!({"entities": entities}),
        &[],
        duration_ms,
    )
    .await;
    let response = ExtractEntitiesResponse {
        entities,
        task: LlmTask::EntityExtraction,
        model_used: client.config().model_name.clone(),
        processing_ms: duration_ms,
        governance: governance_metadata(prompt, Vec::new()),
    };
    let meta = ResponseMeta::now()
        .with_request_id(request_id)
        .with_duration(duration_ms);

    (StatusCode::OK, Json(success_with_meta(response, meta)))
}

#[cfg(not(feature = "llm"))]
pub(crate) async fn llm_extract_entities(
    State(_state): State<AppState>,
    Json(_payload): Json<ExtractEntitiesRequest>,
) -> (StatusCode, Json<ApiResponse<ExtractEntitiesResponse>>) {
    llm_service_unavailable("LLM feature disabled")
}

#[cfg(feature = "llm")]
pub(crate) async fn llm_generate_recipe(
    State(state): State<AppState>,
    Json(payload): Json<GenerateRecipeRequest>,
) -> (StatusCode, Json<ApiResponse<GenerateRecipeResponse>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();

    let issues = payload.validate();
    if !issues.is_empty() {
        let api_err = ApiError::validation("payload", issues.join("; "));
        return (
            StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::UNPROCESSABLE_ENTITY),
            Json(error_response(api_err)),
        );
    }

    let runtime = match state.llm.as_ref() {
        Some(runtime) => runtime,
        None => return llm_service_unavailable("LLM not configured"),
    };

    let client = OpenAiCompatibleClient::new(runtime.primary.clone());
    let prompt = workflow_prompt(PromptKey::RecipeHypothesis);
    ensure_prompt_registered(&state, prompt).await;

    let user = format!(
        "Pattern description: {}\nOutcome: {}\nSignals: {:?}\nExisting IDs: {:?}\nRegions: {:?}\nReturn JSON only.",
        payload.pattern_description,
        payload.outcome,
        payload.signals,
        payload.existing_recipe_ids,
        payload.regions
    );
    let request_payload = serialize_payload(&payload);

    let raw = match client.generate_json(prompt.system_prompt, &user).await {
        Ok(value) => value,
        Err(err) => {
            tracing::error!(request_id = %request_id, "LLM recipe generation failed: {err:#}");
            let api_err = ApiError::internal("LLM recipe generation failed");
            return (
                StatusCode::from_u16(api_err.http_status())
                    .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(error_response(api_err)),
            );
        }
    };

    let recipe_json = match validators::parse_json_response(&raw) {
        Ok(value) => value,
        Err(err) => {
            let duration_ms = start.elapsed().as_millis() as u64;
            let validation_issues = vec![err.clone()];
            persist_workflow_run(
                &state,
                prompt,
                &client.config().model_name,
                &request_payload,
                &serde_json::json!({"raw": raw}),
                &validation_issues,
                duration_ms,
            )
            .await;
            let api_err = ApiError::validation("llm", err);
            return (
                StatusCode::from_u16(api_err.http_status())
                    .unwrap_or(StatusCode::UNPROCESSABLE_ENTITY),
                Json(error_response(api_err)),
            );
        }
    };

    let recipe_issues = validate_recipe_output(&recipe_json, &payload.existing_recipe_ids);
    if !recipe_issues.is_empty() {
        let duration_ms = start.elapsed().as_millis() as u64;
        persist_workflow_run(
            &state,
            prompt,
            &client.config().model_name,
            &request_payload,
            &recipe_json,
            &recipe_issues,
            duration_ms,
        )
        .await;
        let api_err = ApiError::validation("recipe_json", recipe_issues.join("; "));
        return (
            StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::UNPROCESSABLE_ENTITY),
            Json(error_response(api_err)),
        );
    }

    let duration_ms = start.elapsed().as_millis() as u64;
    persist_workflow_run(
        &state,
        prompt,
        &client.config().model_name,
        &request_payload,
        &recipe_json,
        &[],
        duration_ms,
    )
    .await;
    let response = GenerateRecipeResponse {
        recipe_json,
        task: LlmTask::RecipeHypothesis,
        model_used: client.config().model_name.clone(),
        processing_ms: duration_ms,
        governance: governance_metadata(prompt, Vec::new()),
    };
    let meta = ResponseMeta::now()
        .with_request_id(request_id)
        .with_duration(duration_ms);

    (StatusCode::OK, Json(success_with_meta(response, meta)))
}

#[cfg(not(feature = "llm"))]
pub(crate) async fn llm_generate_recipe(
    State(_state): State<AppState>,
    Json(_payload): Json<GenerateRecipeRequest>,
) -> (StatusCode, Json<ApiResponse<GenerateRecipeResponse>>) {
    llm_service_unavailable("LLM feature disabled")
}

#[cfg(feature = "llm")]
pub(crate) async fn llm_synthesize_poi(
    State(state): State<AppState>,
    Json(payload): Json<SynthesizePoiRequest>,
) -> (StatusCode, Json<ApiResponse<SynthesizePoiResponse>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();

    let issues = payload.validate();
    if !issues.is_empty() {
        let api_err = ApiError::validation("payload", issues.join("; "));
        return (
            StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::UNPROCESSABLE_ENTITY),
            Json(error_response(api_err)),
        );
    }

    let runtime = match state.llm.as_ref() {
        Some(runtime) => runtime,
        None => return llm_service_unavailable("LLM not configured"),
    };

    let client = OpenAiCompatibleClient::new(runtime.primary.clone());
    let prompt = workflow_prompt(PromptKey::PoiSynthesis);
    ensure_prompt_registered(&state, prompt).await;

    let fragments_text = payload
        .fragments
        .iter()
        .enumerate()
        .map(|(idx, fragment)| {
            format!(
                "[{}] source_url={:?} source_type={:?} date={:?}\n{}",
                idx + 1,
                fragment.source_url,
                fragment.source_type,
                fragment.date,
                fragment.text
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    let user = format!(
        "Person: {}\nKnown titles: {:?}\nFragments:\n{}\nReturn JSON only.",
        payload.person_name, payload.known_titles, fragments_text
    );
    let request_payload = serialize_payload(&payload);

    let raw = match client.generate_json(prompt.system_prompt, &user).await {
        Ok(value) => value,
        Err(err) => {
            tracing::error!(request_id = %request_id, "LLM POI synthesis failed: {err:#}");
            let api_err = ApiError::internal("LLM POI synthesis failed");
            return (
                StatusCode::from_u16(api_err.http_status())
                    .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(error_response(api_err)),
            );
        }
    };

    let value = match validators::parse_json_response(&raw) {
        Ok(value) => value,
        Err(err) => {
            let duration_ms = start.elapsed().as_millis() as u64;
            let validation_issues = vec![err.clone()];
            persist_workflow_run(
                &state,
                prompt,
                &client.config().model_name,
                &request_payload,
                &serde_json::json!({"raw": raw}),
                &validation_issues,
                duration_ms,
            )
            .await;
            let api_err = ApiError::validation("llm", err);
            return (
                StatusCode::from_u16(api_err.http_status())
                    .unwrap_or(StatusCode::UNPROCESSABLE_ENTITY),
                Json(error_response(api_err)),
            );
        }
    };

    let payload_value: PoiPayload = match serde_json::from_value(value) {
        Ok(value) => value,
        Err(err) => {
            let duration_ms = start.elapsed().as_millis() as u64;
            let validation_issues = vec![format!("Invalid POI payload: {}", err)];
            persist_workflow_run(
                &state,
                prompt,
                &client.config().model_name,
                &request_payload,
                &serde_json::json!({"raw": raw}),
                &validation_issues,
                duration_ms,
            )
            .await;
            let api_err = ApiError::validation("poi", format!("Invalid POI payload: {}", err));
            return (
                StatusCode::from_u16(api_err.http_status())
                    .unwrap_or(StatusCode::UNPROCESSABLE_ENTITY),
                Json(error_response(api_err)),
            );
        }
    };

    let validation_issues = validate_poi_output(&payload_value);
    if !validation_issues.is_empty() {
        let duration_ms = start.elapsed().as_millis() as u64;
        persist_workflow_run(
            &state,
            prompt,
            &client.config().model_name,
            &request_payload,
            &serialize_payload(&payload_value),
            &validation_issues,
            duration_ms,
        )
        .await;
        let api_err = ApiError::validation("poi", validation_issues.join("; "));
        return (
            StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::UNPROCESSABLE_ENTITY),
            Json(error_response(api_err)),
        );
    }

    let duration_ms = start.elapsed().as_millis() as u64;
    persist_workflow_run(
        &state,
        prompt,
        &client.config().model_name,
        &request_payload,
        &serialize_payload(&payload_value),
        &[],
        duration_ms,
    )
    .await;
    let response = SynthesizePoiResponse {
        person_name: payload.person_name,
        summary: payload_value.summary,
        roles: payload_value.roles,
        affiliations: payload_value.affiliations,
        key_facts: payload_value.key_facts,
        risk_indicators: payload_value.risk_indicators,
        source_count: payload.fragments.len(),
        task: LlmTask::PoiSynthesis,
        model_used: client.config().model_name.clone(),
        processing_ms: duration_ms,
        governance: governance_metadata(prompt, Vec::new()),
    };
    let meta = ResponseMeta::now()
        .with_request_id(request_id)
        .with_duration(duration_ms);

    (StatusCode::OK, Json(success_with_meta(response, meta)))
}

#[cfg(not(feature = "llm"))]
pub(crate) async fn llm_synthesize_poi(
    State(_state): State<AppState>,
    Json(_payload): Json<SynthesizePoiRequest>,
) -> (StatusCode, Json<ApiResponse<SynthesizePoiResponse>>) {
    llm_service_unavailable("LLM feature disabled")
}

#[cfg(feature = "llm")]
pub(crate) async fn llm_generate_memo(
    State(state): State<AppState>,
    Json(payload): Json<GenerateMemoRequest>,
) -> (StatusCode, Json<ApiResponse<GenerateMemoResponse>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();

    let issues = payload.validate();
    if !issues.is_empty() {
        let api_err = ApiError::validation("payload", issues.join("; "));
        return (
            StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::UNPROCESSABLE_ENTITY),
            Json(error_response(api_err)),
        );
    }

    let runtime = match state.llm.as_ref() {
        Some(runtime) => runtime,
        None => return llm_service_unavailable("LLM not configured"),
    };

    let client = OpenAiCompatibleClient::new(runtime.primary.clone());
    let prompt = workflow_prompt(PromptKey::MemoGeneration);
    ensure_prompt_registered(&state, prompt).await;

    let context_text = payload
        .context_items
        .iter()
        .enumerate()
        .map(|(idx, item)| {
            format!(
                "[{}] {}\n{}\nsource={:?} date={:?}",
                idx + 1,
                item.title,
                item.content,
                item.source,
                item.date
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    let user = format!(
        "Topic: {}\nAudience: {:?}\nMax words: {:?}\nContext:\n{}\nReturn JSON only.",
        payload.topic, payload.audience, payload.max_words, context_text
    );
    let request_payload = serialize_payload(&payload);

    let raw = match client.generate_json(prompt.system_prompt, &user).await {
        Ok(value) => value,
        Err(err) => {
            tracing::error!(request_id = %request_id, "LLM memo generation failed: {err:#}");
            let api_err = ApiError::internal("LLM memo generation failed");
            return (
                StatusCode::from_u16(api_err.http_status())
                    .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(error_response(api_err)),
            );
        }
    };

    let value = match validators::parse_json_response(&raw) {
        Ok(value) => value,
        Err(err) => {
            let duration_ms = start.elapsed().as_millis() as u64;
            let validation_issues = vec![err.clone()];
            persist_workflow_run(
                &state,
                prompt,
                &client.config().model_name,
                &request_payload,
                &serde_json::json!({"raw": raw}),
                &validation_issues,
                duration_ms,
            )
            .await;
            let api_err = ApiError::validation("llm", err);
            return (
                StatusCode::from_u16(api_err.http_status())
                    .unwrap_or(StatusCode::UNPROCESSABLE_ENTITY),
                Json(error_response(api_err)),
            );
        }
    };

    let payload_value: MemoPayload = match serde_json::from_value(value) {
        Ok(value) => value,
        Err(err) => {
            let duration_ms = start.elapsed().as_millis() as u64;
            let validation_issues = vec![format!("Invalid memo payload: {}", err)];
            persist_workflow_run(
                &state,
                prompt,
                &client.config().model_name,
                &request_payload,
                &serde_json::json!({"raw": raw}),
                &validation_issues,
                duration_ms,
            )
            .await;
            let api_err = ApiError::validation("memo", format!("Invalid memo payload: {}", err));
            return (
                StatusCode::from_u16(api_err.http_status())
                    .unwrap_or(StatusCode::UNPROCESSABLE_ENTITY),
                Json(error_response(api_err)),
            );
        }
    };

    let validation_issues = validate_memo_output(&payload_value);
    if !validation_issues.is_empty() {
        let duration_ms = start.elapsed().as_millis() as u64;
        persist_workflow_run(
            &state,
            prompt,
            &client.config().model_name,
            &request_payload,
            &serialize_payload(&payload_value),
            &validation_issues,
            duration_ms,
        )
        .await;
        let api_err = ApiError::validation("memo", validation_issues.join("; "));
        return (
            StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::UNPROCESSABLE_ENTITY),
            Json(error_response(api_err)),
        );
    }

    let duration_ms = start.elapsed().as_millis() as u64;
    persist_workflow_run(
        &state,
        prompt,
        &client.config().model_name,
        &request_payload,
        &serialize_payload(&payload_value),
        &[],
        duration_ms,
    )
    .await;
    let response = GenerateMemoResponse {
        title: payload_value.title,
        executive_summary: payload_value.executive_summary,
        sections: payload_value.sections,
        recommendations: payload_value.recommendations,
        task: LlmTask::MemoGeneration,
        model_used: client.config().model_name.clone(),
        processing_ms: duration_ms,
        governance: governance_metadata(prompt, Vec::new()),
    };
    let meta = ResponseMeta::now()
        .with_request_id(request_id)
        .with_duration(duration_ms);

    (StatusCode::OK, Json(success_with_meta(response, meta)))
}

#[cfg(not(feature = "llm"))]
pub(crate) async fn llm_generate_memo(
    State(_state): State<AppState>,
    Json(_payload): Json<GenerateMemoRequest>,
) -> (StatusCode, Json<ApiResponse<GenerateMemoResponse>>) {
    llm_service_unavailable("LLM feature disabled")
}

#[cfg(all(test, feature = "llm"))]
mod tests {
    use super::parse_entities_value;

    #[test]
    fn test_parse_entities_value_accepts_named_object_array() {
        let parsed = parse_entities_value(serde_json::json!({
            "entities": [
                {"name": "Starz Electronics", "entity_type": "company", "confidence": 0.9}
            ]
        }))
        .expect("entities payload should parse");

        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].name, "Starz Electronics");
        assert_eq!(parsed[0].entity_type, "company");
    }

    #[test]
    fn test_parse_entities_value_accepts_string_array_fallback() {
        let parsed = parse_entities_value(serde_json::json!(["Tangier"]))
            .expect("string entity list should parse");

        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].name, "Tangier");
        assert_eq!(parsed[0].entity_type, "unknown");
        assert_eq!(parsed[0].confidence, 0.5);
    }
}
