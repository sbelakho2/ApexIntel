use super::super::*;
use axum::response::Html;
use axum::Form;

#[derive(Debug, serde::Deserialize)]
pub(crate) struct TriggerScanFormRequest {
    job_kind: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct RecipeCreateHtmlResponse {
    id: String,
    name: String,
    status: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct RecipeTestHtmlResponse {
    matches: usize,
    sample_warnings: Vec<String>,
}

fn slugify_recipe_code(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut prev_dash = false;
    for ch in input.chars() {
        let c = ch.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() {
            out.push(c);
            prev_dash = false;
        } else if !prev_dash {
            out.push('_');
            prev_dash = true;
        }
    }
    let trimmed = out.trim_matches('_');
    if trimmed.is_empty() {
        "recipe".to_string()
    } else {
        trimmed.to_string()
    }
}

pub(crate) async fn post_trigger_scan_html(
    State(state): State<AppState>,
    form: Result<Form<TriggerScanFormRequest>, axum::extract::rejection::FormRejection>,
) -> impl IntoResponse {
    let requested_kind = form
        .ok()
        .and_then(|Form(f)| f.job_kind)
        .unwrap_or_else(|| "dns_posture_scan".to_string());

    if !is_valid_manual_trigger_kind(&requested_kind) {
        return (
            StatusCode::BAD_REQUEST,
            Html(format!(
                "<div class=\"rounded border border-red-500/30 bg-red-500/10 px-3 py-2 text-xs font-semibold text-red-700\">Unknown scan type: {}</div>",
                requested_kind
            )),
        );
    }

    match state.store.queue_job_trigger(&requested_kind).await {
        Ok(trigger_id) => (
            StatusCode::ACCEPTED,
            Html(format!(
                "<div class=\"apex-card p-4 border-green-500/30 bg-green-500/5\"><div class=\"flex items-center gap-2\"><span class=\"h-2 w-2 rounded-full bg-green-500 animate-pulse\"></span><p class=\"text-sm font-bold text-green-600\">{} queued</p></div><p class=\"mt-1 text-[11px] text-green-700\">Trigger ID: {}</p></div>",
                requested_kind,
                trigger_id
            )),
        ),
        Err(err) => {
            tracing::error!(job_kind = %requested_kind, "queue_job_trigger failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Html("<div class=\"rounded border border-red-500/30 bg-red-500/10 px-3 py-2 text-xs font-semibold text-red-700\">Failed to queue security scan</div>".to_string()),
            )
        }
    }
}

pub(crate) async fn post_recipe_create_html(
    State(state): State<AppState>,
    Json(payload): Json<RecipeBuilderPayload>,
) -> (StatusCode, Json<RecipeCreateHtmlResponse>) {
    let name = payload.name.trim();
    let mut code = slugify_recipe_code(name);
    if code.len() > 56 {
        code.truncate(56);
    }
    let suffix = Uuid::new_v4().simple().to_string();
    let short = &suffix[..8];
    code = format!("{}_{}", code, short);

    let definition = serde_json::json!({
        "name": name,
        "description": payload.description.unwrap_or_default(),
        "severity": payload.severity.unwrap_or_else(|| "medium".to_string()),
        "cooldown_hours": payload.cooldown_hours.unwrap_or(24),
        "enabled": payload.enabled.unwrap_or(true),
        "narrative_template": payload.narrative_template.unwrap_or_default(),
        "signals": payload.signals,
        "transforms": payload.transforms,
        "thresholds": payload.thresholds,
        "actions": payload.actions,
    });

    match state
        .store
        .upsert_recipe_definition(&code, name, "staging", &definition)
        .await
    {
        Ok(_) => (
            StatusCode::CREATED,
            Json(RecipeCreateHtmlResponse {
                id: code,
                name: name.to_string(),
                status: "staging".to_string(),
            }),
        ),
        Err(err) => {
            tracing::error!("recipe create failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(RecipeCreateHtmlResponse {
                    id: String::new(),
                    name: String::new(),
                    status: "error".to_string(),
                }),
            )
        }
    }
}

pub(crate) async fn post_recipe_test_html(
    State(state): State<AppState>,
    Json(payload): Json<RecipeBuilderPayload>,
) -> (StatusCode, Json<RecipeTestHtmlResponse>) {
    let rows = match state.store.list_observations(None, None, 400, 0).await {
        Ok(rows) => rows,
        Err(err) => {
            tracing::error!("recipe test list observations failed: {err:#}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(RecipeTestHtmlResponse {
                    matches: 0,
                    sample_warnings: vec![],
                }),
            );
        }
    };

    let keywords: Vec<String> = payload
        .signals
        .iter()
        .filter_map(|signal| signal.keywords.as_ref())
        .flat_map(|keywords| keywords.split(','))
        .map(str::trim)
        .filter(|keyword| !keyword.is_empty())
        .map(|keyword| keyword.to_ascii_lowercase())
        .collect();

    let mut sample_warnings: Vec<String> = Vec::new();
    let mut matches = 0usize;

    for row in rows {
        let haystack = row.value.to_string().to_ascii_lowercase();
        let matched = if keywords.is_empty() {
            true
        } else {
            keywords.iter().any(|keyword| haystack.contains(keyword))
        };
        if matched {
            matches += 1;
            if sample_warnings.len() < 3 {
                let mut snippet = row.value.to_string().replace('\n', " ");
                if snippet.len() > 140 {
                    snippet.truncate(140);
                    snippet.push_str("...");
                }
                sample_warnings.push(format!(
                    "{} @ {} — {}",
                    row.observation_type,
                    row.ts_utc.format("%Y-%m-%d %H:%M"),
                    snippet
                ));
            }
        }
    }

    (
        StatusCode::OK,
        Json(RecipeTestHtmlResponse {
            matches,
            sample_warnings,
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::slugify_recipe_code;
    use crate::is_valid_manual_trigger_kind;

    #[test]
    fn test_slugify_recipe_code_normalizes_whitespace_and_case() {
        assert_eq!(
            slugify_recipe_code("My Trigger Recipe"),
            "my_trigger_recipe"
        );
    }

    #[test]
    fn test_slugify_recipe_code_falls_back_for_empty_input() {
        assert_eq!(slugify_recipe_code("!!!"), "recipe");
    }

    #[test]
    fn test_manual_trigger_kind_allows_whitelisted_job() {
        assert!(is_valid_manual_trigger_kind("dns_posture_scan"));
        assert!(!is_valid_manual_trigger_kind("drop_database"));
    }
}
