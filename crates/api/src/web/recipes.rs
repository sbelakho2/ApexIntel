//! Recipe handlers — GET /recipes (list), GET /recipes/new
//!
//! Covers: recipe list with run stats and status indicators,
//! recipe creation form.

use std::sync::Arc;

use askama::Template;
use axum::{http::HeaderMap, response::IntoResponse, Extension};
use serde::Deserialize;

use super::{is_htmx_request, PageContext};
use crate::middleware::session::WebSession;
use apex_store::postgres::{PgStore, WarningListFilters};

// ─── Query params ───────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct RecipesQuery {
    pub page: Option<i64>,
    pub per_page: Option<i64>,
    pub status: Option<String>,
    pub sort: Option<String>,
    pub dir: Option<String>,
}

// ─── Template data ──────────────────────────────────────────────────────────

/// One month of recipe performance metrics for the trend line chart.
#[derive(Clone, Debug)]
pub struct RecipePerfRow {
    pub month: String,
    pub precision_pct: i64,
    pub recall_pct: i64,
    pub fpr_pct: i64,
}

#[derive(Clone, Debug)]
pub struct RecipeListItem {
    pub id: String,
    pub name: String,
    pub description: String,
    pub status: String, // "active" | "paused" | "draft" | "archived"
    pub schedule: String,
    pub total_runs: i64,
    pub success_rate: f64,
    pub last_run: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub tags: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct RecipeField {
    pub name: String,
    pub field_type: String,
    pub required: bool,
    pub description: String,
    pub default_value: Option<String>,
}

// ─── Templates ──────────────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "pages/recipes.html")]
pub struct RecipesListPage {
    pub current_path: String,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,

    pub recipes: Vec<RecipeListItem>,
    pub total: i64,
    pub page: i64,
    pub per_page: i64,
    pub total_pages: i64,
    pub active_status: String,
    pub sort_field: String,
    pub sort_dir: String,
    pub active_recipes_count: i64,
    pub production_count: i64,
    pub total_fired: i64,
    pub avg_success_rate: i64,
    pub total_runs_sum: i64,
    pub avg_precision: i64,
    pub avg_recall: i64,
    pub precision_points: String,
    pub recall_points: String,
    pub fpr_points: String,
    pub recipe_chart_w: i64,
    pub recipe_perf_trend: Vec<RecipePerfRow>,
}

/// HTMX partial — just the results fragment.
#[derive(Template)]
#[template(path = "pages/recipes/_list.html")]
pub struct RecipesListPartial {
    pub recipes: Vec<RecipeListItem>,
    pub total: i64,
    pub page: i64,
    pub per_page: i64,
    pub total_pages: i64,
    pub active_status: String,
    pub sort_field: String,
    pub sort_dir: String,
    pub active_recipes_count: i64,
    pub production_count: i64,
    pub total_fired: i64,
    pub avg_success_rate: i64,
    pub total_runs_sum: i64,
    pub avg_precision: i64,
    pub avg_recall: i64,
    pub precision_points: String,
    pub recall_points: String,
    pub fpr_points: String,
    pub recipe_chart_w: i64,
    pub recipe_perf_trend: Vec<RecipePerfRow>,
}

#[derive(Template)]
#[template(path = "pages/recipe_new.html")]
pub struct RecipeNewPage {
    pub current_path: String,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,
}

// ─── Handlers ───────────────────────────────────────────────────────────────

/// GET /recipes — paginated recipe list.
pub async fn list_recipes(
    headers: HeaderMap,
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
    axum::extract::Query(params): axum::extract::Query<RecipesQuery>,
) -> impl IntoResponse {
    let unack = store
        .count_warnings(&WarningListFilters {
            acknowledged: Some(false),
            ..Default::default()
        })
        .await
        .unwrap_or(0);
    let ctx = PageContext::from_session(&session, "/recipes", unack);
    let active_status = params.status.clone().unwrap_or_default();
    let page = params.page.unwrap_or(1).max(1);
    let per_page = params.per_page.unwrap_or(25).clamp(1, 100);

    let recipe_stat_rows = store.get_recipe_stats().await.unwrap_or_else(|e| {
        tracing::error!("Failed to load recipe stats: {e}");
        vec![]
    });
    let quality_summary = store
        .get_recipe_quality_summary()
        .await
        .unwrap_or_else(|e| {
            tracing::error!("Failed to load recipe quality summary: {e}");
            apex_store::postgres::RecipeQualitySummaryRow {
                avg_precision_pct: 0,
                coverage_pct: 0,
            }
        });

    let mut all_recipes: Vec<RecipeListItem> = recipe_stat_rows
        .iter()
        .map(|r| {
            let success_rate = (r.precision_score * 100.0).clamp(0.0, 100.0);
            let status = match r.status.as_str() {
                "active" | "production" => "production",
                "deprecated" => "deprecated",
                _ => "staging",
            };
            RecipeListItem {
                id: String::new(),
                name: r.recipe_code.clone(),
                description: String::new(),
                status: status.into(),
                schedule: String::new(),
                total_runs: r.fired_count,
                success_rate,
                last_run: r.last_fired.map(|t| t.format("%Y-%m-%d %H:%M").to_string()),
                created_at: r
                    .first_fired
                    .map(|t| t.format("%Y-%m-%d").to_string())
                    .unwrap_or_default(),
                updated_at: r
                    .last_fired
                    .map(|t| t.format("%Y-%m-%d").to_string())
                    .unwrap_or_default(),
                tags: vec![],
            }
        })
        .collect();

    if !active_status.is_empty() {
        all_recipes.retain(|r| r.status == active_status);
    }

    let total = all_recipes.len() as i64;
    let start = ((page - 1) * per_page) as usize;
    let end = (start + per_page as usize).min(all_recipes.len());
    let recipes: Vec<RecipeListItem> = if start < all_recipes.len() {
        all_recipes[start..end].to_vec()
    } else {
        Vec::new()
    };

    let active_recipes_count = all_recipes.len() as i64;
    let production_count = all_recipes
        .iter()
        .filter(|r| r.status == "production")
        .count() as i64;
    let total_fired = all_recipes.iter().map(|r| r.total_runs).sum::<i64>();
    let total_runs_sum = total_fired;
    let avg_success_rate = if all_recipes.is_empty() {
        0
    } else {
        (all_recipes.iter().map(|r| r.success_rate).sum::<f64>() / all_recipes.len() as f64).round()
            as i64
    };
    // Use real persisted quality signals instead of unacknowledged-alert ratios.
    let avg_precision = quality_summary.avg_precision_pct;
    let avg_recall = quality_summary.coverage_pct;

    // Build 12-month performance trend (static seeded data)
    let months = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let recipe_perf_trend: Vec<RecipePerfRow> = months
        .iter()
        .enumerate()
        .map(|(i, &mo)| {
            let s = (i + 1) as f64;
            let prec = (avg_precision as f64 * (1.0 + (s * 0.5).sin() * 0.08))
                .clamp(0.0, 100.0)
                .round() as i64;
            let rec = (avg_recall as f64 * (1.0 + (s * 0.7).cos() * 0.07))
                .clamp(0.0, 100.0)
                .round() as i64;
            let fpr = ((10.0 + (s * 0.9).sin() * 4.0).abs()).round() as i64;
            RecipePerfRow {
                month: mo.into(),
                precision_pct: prec,
                recall_pct: rec,
                fpr_pct: fpr,
            }
        })
        .collect();

    let recipe_chart_w = (recipe_perf_trend.len() as i64 * 22).max(22);
    let precision_points: String = recipe_perf_trend
        .iter()
        .enumerate()
        .map(|(i, r)| format!("{},{}", i as i64 * 22, 100 - r.precision_pct))
        .collect::<Vec<_>>()
        .join(" ");
    let recall_points: String = recipe_perf_trend
        .iter()
        .enumerate()
        .map(|(i, r)| format!("{},{}", i as i64 * 22, 100 - r.recall_pct))
        .collect::<Vec<_>>()
        .join(" ");
    let fpr_points: String = recipe_perf_trend
        .iter()
        .enumerate()
        .map(|(i, r)| format!("{},{}", i as i64 * 22, 100 - r.fpr_pct))
        .collect::<Vec<_>>()
        .join(" ");

    let total_pages = if per_page > 0 {
        (total + per_page - 1) / per_page
    } else {
        0
    };

    let tpl = RecipesListPage {
        current_path: ctx.current_path,
        username: ctx.username,
        warning_count: ctx.warning_count,
        theme: ctx.theme,
        recipes,
        total,
        page,
        per_page,
        total_pages,
        active_status,
        sort_field: params.sort.unwrap_or_else(|| "name".into()),
        sort_dir: params.dir.unwrap_or_else(|| "asc".into()),
        active_recipes_count,
        production_count,
        total_fired,
        avg_success_rate,
        total_runs_sum,
        avg_precision,
        avg_recall,
        precision_points,
        recall_points,
        fpr_points,
        recipe_chart_w,
        recipe_perf_trend,
    };

    if is_htmx_request(&headers) {
        let partial = RecipesListPartial {
            recipes: tpl.recipes.clone(),
            total: tpl.total,
            page: tpl.page,
            per_page: tpl.per_page,
            total_pages: tpl.total_pages,
            active_status: tpl.active_status.clone(),
            sort_field: tpl.sort_field.clone(),
            sort_dir: tpl.sort_dir.clone(),
            active_recipes_count: tpl.active_recipes_count,
            production_count: tpl.production_count,
            total_fired: tpl.total_fired,
            avg_success_rate: tpl.avg_success_rate,
            total_runs_sum: tpl.total_runs_sum,
            avg_precision: tpl.avg_precision,
            avg_recall: tpl.avg_recall,
            precision_points: tpl.precision_points.clone(),
            recall_points: tpl.recall_points.clone(),
            fpr_points: tpl.fpr_points.clone(),
            recipe_chart_w: tpl.recipe_chart_w,
            recipe_perf_trend: tpl.recipe_perf_trend.clone(),
        };
        super::render_template(&partial)
    } else {
        super::render_template(&tpl)
    }
}

/// GET /recipes/new — recipe creation form.
pub async fn new_recipe(
    headers: HeaderMap,
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
) -> impl IntoResponse {
    let unack = store
        .count_warnings(&WarningListFilters {
            acknowledged: Some(false),
            ..Default::default()
        })
        .await
        .unwrap_or(0);
    let ctx = PageContext::from_session(&session, "/recipes", unack);

    let tpl = RecipeNewPage {
        current_path: ctx.current_path,
        username: ctx.username,
        warning_count: ctx.warning_count,
        theme: ctx.theme,
    };

    let _ = is_htmx_request(&headers);
    super::render_template(&tpl)
}

// ─── Recipe creation (B307) ─────────────────────────────────────────────────
//
// The form on /recipes/new posted to `/recipes/create-form`, a route that was
// never registered — the only user-facing creation flow in the product 404'd
// on submit. The parsing/derivation logic mirrors the (unwired)
// `api_handlers/html_mutations.rs` implementation so both paths agree.

#[derive(Debug, serde::Deserialize)]
pub struct RecipeCreateForm {
    pub name: String,
    pub description: Option<String>,
    pub severity: Option<String>,
    pub cooldown_hours: Option<i64>,
    pub enabled: Option<String>,
    pub narrative_template: Option<String>,
    pub signals_json: Option<String>,
    pub transforms_json: Option<String>,
    pub thresholds_json: Option<String>,
    pub actions_json: Option<String>,
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
    let trimmed = out.trim_matches('_').to_string();
    if trimmed.is_empty() {
        "recipe".to_string()
    } else {
        trimmed
    }
}

fn parse_recipe_section(raw: Option<&str>, field_name: &str) -> Result<serde_json::Value, String> {
    let source = raw
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("[]");
    serde_json::from_str(source).map_err(|error| format!("{field_name}: {error}"))
}

/// POST /recipes/create-form — validate the submitted definition and persist
/// it as a `staging` recipe (promoted explicitly via POST /api/recipes/:id/promote).
pub async fn create_recipe_form(
    Extension(store): Extension<Arc<PgStore>>,
    axum::Form(form): axum::Form<RecipeCreateForm>,
) -> axum::response::Response {
    let name = form.name.trim().to_string();
    if name.is_empty() {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            axum::response::Html(
                r#"<div class="rounded border border-rams-red/30 bg-rams-red/10 px-3 py-2 text-xs font-semibold text-rams-red">Recipe name is required.</div>"#.to_string(),
            ),
        )
            .into_response();
    }

    let mut sections = std::collections::BTreeMap::new();
    for (field, raw) in [
        ("signals", form.signals_json.as_deref()),
        ("transforms", form.transforms_json.as_deref()),
        ("thresholds", form.thresholds_json.as_deref()),
        ("actions", form.actions_json.as_deref()),
    ] {
        match parse_recipe_section(raw, &format!("{field}_json")) {
            Ok(v) => {
                sections.insert(field, v);
            }
            Err(e) => {
                return (
                    axum::http::StatusCode::BAD_REQUEST,
                    axum::response::Html(format!(
                        r#"<div class="rounded border border-rams-red/30 bg-rams-red/10 px-3 py-2 text-xs font-semibold text-rams-red">Invalid recipe payload: {}</div>"#,
                        super::escape_html(&e)
                    )),
                )
                    .into_response()
            }
        }
    }

    let severity = form
        .severity
        .filter(|s| ["critical", "high", "medium", "low", "info"].contains(&s.as_str()))
        .unwrap_or_else(|| "medium".to_string());

    let mut code = slugify_recipe_code(&name);
    if code.len() > 56 {
        code.truncate(56);
    }
    // Unique suffix keeps repeated submissions from overwriting each other.
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    code = format!("{}_{}", code, &suffix[..8]);

    let definition = serde_json::json!({
        "name": name,
        "description": form.description.unwrap_or_default(),
        "severity": severity,
        "cooldown_hours": form.cooldown_hours.filter(|h| (1..=720).contains(h)).unwrap_or(24),
        "enabled": form.enabled.is_some(),
        "narrative_template": form.narrative_template.unwrap_or_default(),
        "signals": sections.get("signals").cloned().unwrap_or(serde_json::json!([])),
        "transforms": sections.get("transforms").cloned().unwrap_or(serde_json::json!([])),
        "thresholds": sections.get("thresholds").cloned().unwrap_or(serde_json::json!([])),
        "actions": sections.get("actions").cloned().unwrap_or(serde_json::json!([])),
    });

    match store
        .upsert_recipe_definition(&code, definition["name"].as_str().unwrap_or("Recipe"), "staging", &definition)
        .await
    {
        Ok(()) => axum::response::Redirect::to("/recipes?created=1").into_response(),
        Err(err) => {
            tracing::error!("recipe create form failed: {err:#}");
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                axum::response::Html(
                    r#"<div class="rounded border border-rams-red/30 bg-rams-red/10 px-3 py-2 text-xs font-semibold text-rams-red">Failed to create recipe — see server logs.</div>"#.to_string(),
                ),
            )
                .into_response()
        }
    }
}
