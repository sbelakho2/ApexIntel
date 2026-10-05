//! Recipe handlers — GET /recipes (list), GET /recipes/new
//!
//! Covers: recipe list with run stats and status indicators,
//! recipe creation form.

use std::collections::HashMap;
use std::sync::Arc;

use askama::Template;
use axum::{http::HeaderMap, response::IntoResponse, Extension};
use serde::Deserialize;

use super::{is_htmx_request, PageContext};
use crate::middleware::session::WebSession;
use apex_core::data_state::{DataState, DegradedNotice};
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
    /// Formatted measured precision ("72%") or "not measured" — an
    /// unmeasured month is never rendered as 0%.
    pub precision_display: String,
    /// Formatted measured false-positive rate or "not measured".
    pub fpr_display: String,
    pub reviewed_warnings: i64,
}

/// One plotted point with its tooltip, so the template never has to compute
/// arithmetic over a missing measurement.
#[derive(Clone, Debug)]
pub struct RecipePerfPoint {
    pub x: i64,
    pub y: i64,
    pub title: String,
}

#[derive(Clone, Debug)]
pub struct RecipeListItem {
    pub id: String,
    pub name: String,
    pub description: String,
    pub status: String, // "active" | "paused" | "draft" | "archived"
    pub schedule: String,
    pub total_runs: i64,
    /// Empirical precision percentage from reviewed outcomes; `None` when
    /// nothing was reviewed ("not measured" is not 0%).
    pub success_rate: Option<f64>,
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
    pub can_admin: bool,
    pub can_write: bool,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,
    pub status_strip: crate::system_status::StatusStrip,

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
    pub avg_success_rate: Option<i64>,
    pub total_runs_sum: i64,
    pub avg_precision: Option<i64>,
    pub avg_coverage: i64,
    pub precision_points: String,
    pub fpr_points: String,
    pub precision_circles: Vec<RecipePerfPoint>,
    pub fpr_circles: Vec<RecipePerfPoint>,
    pub recipe_chart_w: i64,
    pub recipe_perf_trend: Vec<RecipePerfRow>,
    pub degraded_notice: Option<String>,
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
    pub avg_success_rate: Option<i64>,
    pub total_runs_sum: i64,
    pub avg_precision: Option<i64>,
    pub avg_coverage: i64,
    pub precision_points: String,
    pub fpr_points: String,
    pub precision_circles: Vec<RecipePerfPoint>,
    pub fpr_circles: Vec<RecipePerfPoint>,
    pub recipe_chart_w: i64,
    pub recipe_perf_trend: Vec<RecipePerfRow>,
    pub degraded_notice: Option<String>,
}

#[derive(Template)]
#[template(path = "pages/recipe_new.html")]
pub struct RecipeNewPage {
    pub current_path: String,
    pub can_admin: bool,
    pub can_write: bool,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,
    pub status_strip: crate::system_status::StatusStrip,
    pub degraded_notice: Option<String>,
    /// Submitted values re-rendered after a validation/storage error, so a
    /// failed creation never discards the analyst's input.
    pub form: RecipeFormValues,
    pub form_error: Option<String>,
    /// `(category, selected)` pairs, so the template never compares an
    /// `&&str` loop item against the owned form value.
    pub categories: Vec<(&'static str, bool)>,
    /// `(severity, selected)` pairs for the same reason.
    pub severities: Vec<(&'static str, bool)>,
}

/// Sort the recipe list by the requested field/direction (#148). Unknown
/// fields fall back to name ascending. An unmeasured success rate (`None`)
/// sorts last when descending, so "not measured" is never ranked as best.
pub fn sort_recipe_items(items: &mut [RecipeListItem], field: &str, dir: &str) {
    let descending = dir.eq_ignore_ascii_case("desc");
    items.sort_by(|a, b| {
        let ordering = match field {
            "runs" | "total_runs" => a.total_runs.cmp(&b.total_runs),
            "success_rate" => a
                .success_rate
                .unwrap_or(f64::NEG_INFINITY)
                .total_cmp(&b.success_rate.unwrap_or(f64::NEG_INFINITY)),
            "last_run" => a.last_run.cmp(&b.last_run),
            "created_at" => a.created_at.cmp(&b.created_at),
            "status" => a.status.cmp(&b.status),
            // "name" and any unrecognized field: stable name ordering.
            _ => a
                .name
                .to_ascii_lowercase()
                .cmp(&b.name.to_ascii_lowercase()),
        };
        if descending {
            ordering.reverse()
        } else {
            ordering
        }
    });
}

// ─── Handlers ───────────────────────────────────────────────────────────────

/// GET /recipes — paginated recipe list.
pub async fn list_recipes(
    headers: HeaderMap,
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
    axum::extract::Query(params): axum::extract::Query<RecipesQuery>,
) -> impl IntoResponse {
    let mut degraded_notice: Option<String> = None;
    let unack_state = DataState::from_result(
        store
            .count_warnings(&WarningListFilters {
                acknowledged: Some(false),
                ..Default::default()
            })
            .await,
        "count_warnings failed (web recipes list)",
        |_| false,
    );
    DegradedNotice::capture(&unack_state, &mut degraded_notice);
    let ctx = PageContext::from_session(&session, "/recipes", unack_state.into_loaded_or(0));
    let active_status = params.status.clone().unwrap_or_default();
    let page = params.page.unwrap_or(1).max(1);
    let per_page = params.per_page.unwrap_or(25).clamp(1, 100);

    let recipe_stat_state = DataState::from_result(
        store.get_recipe_stats().await,
        "get_recipe_stats failed (web recipes list)",
        Vec::is_empty,
    );
    DegradedNotice::capture(&recipe_stat_state, &mut degraded_notice);
    let recipe_stat_rows = recipe_stat_state.into_items();
    let quality_state = DataState::from_result(
        store.get_recipe_quality_summary().await,
        "get_recipe_quality_summary failed (web recipes list)",
        |_| false,
    );
    DegradedNotice::capture(&quality_state, &mut degraded_notice);
    let quality_summary =
        quality_state.into_loaded_or(apex_store::postgres::RecipeQualitySummaryRow {
            avg_precision_pct: None,
            avg_model_confidence_pct: 0,
            coverage_pct: 0,
        });

    // #148: fill the display fields from the canonical `recipes` rows (real
    // name, category/narrative-derived description, tags) instead of leaving
    // them blank and showing the code as the name.
    let recipe_rows_state = DataState::from_result(
        store.list_recipes_for_engine().await,
        "list_recipes_for_engine failed (web recipes list)",
        Vec::is_empty,
    );
    DegradedNotice::capture(&recipe_rows_state, &mut degraded_notice);
    let recipe_rows: std::collections::HashMap<String, apex_store::postgres::RecipeEngineRow> =
        recipe_rows_state
            .into_items()
            .into_iter()
            .map(|row| (row.code.clone(), row))
            .collect();

    let mut all_recipes: Vec<RecipeListItem> = recipe_stat_rows
        .iter()
        .map(|r| {
            let success_rate = r.precision_score.map(|p| (p * 100.0).clamp(0.0, 100.0));
            let status = match r.status.as_str() {
                "active" | "production" => "production",
                "deprecated" => "deprecated",
                _ => "staging",
            };
            let recipe_row = recipe_rows.get(&r.recipe_code);
            let definition = recipe_row.and_then(|row| row.definition.as_ref());
            let description = definition
                .and_then(|value| value.get("description"))
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .or_else(|| {
                    recipe_row
                        .and_then(|row| row.narrative_template.clone())
                        .filter(|value| !value.trim().is_empty())
                })
                .unwrap_or_default();
            let tags = definition
                .and_then(|value| value.get("tags"))
                .and_then(|value| value.as_array())
                .map(|tags| {
                    tags.iter()
                        .filter_map(|tag| tag.as_str())
                        .map(str::trim)
                        .filter(|tag| !tag.is_empty())
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            RecipeListItem {
                id: r.recipe_code.clone(),
                name: recipe_row
                    .map(|row| row.name.clone())
                    .filter(|name| !name.trim().is_empty())
                    .unwrap_or_else(|| r.recipe_code.clone()),
                description,
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
                tags,
            }
        })
        .collect();

    if !active_status.is_empty() {
        all_recipes.retain(|r| r.status == active_status);
    }

    // #148: apply the requested ordering (the query params existed but were
    // only echoed into the template and never sorted the rows).
    let sort_field_param = params.sort.clone().unwrap_or_else(|| "name".into());
    let sort_dir_param = params.dir.clone().unwrap_or_else(|| "asc".into());
    sort_recipe_items(&mut all_recipes, &sort_field_param, &sort_dir_param);

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
    // Average over measured successes only; nothing measured stays "—".
    let measured_success_rates: Vec<f64> =
        all_recipes.iter().filter_map(|r| r.success_rate).collect();
    let avg_success_rate = if measured_success_rates.is_empty() {
        None
    } else {
        Some(
            (measured_success_rates.iter().sum::<f64>() / measured_success_rates.len() as f64)
                .round() as i64,
        )
    };
    // Use real persisted quality signals instead of unacknowledged-alert ratios.
    let avg_precision = quality_summary.avg_precision_pct;
    let avg_coverage = quality_summary.coverage_pct;

    // Real 12-month history: monthly aggregates over the persisted weekly
    // `recipe_weekly_metrics` snapshots. A month with no measurement is not
    // plotted; when nothing has been measured the chart renders an explicit
    // "no measurements yet" state instead of a synthesized trend.
    let monthly_state = DataState::from_result(
        store.list_recipe_monthly_performance(12).await,
        "list_recipe_monthly_performance failed (web recipes list)",
        Vec::is_empty,
    );
    DegradedNotice::capture(&monthly_state, &mut degraded_notice);
    let monthly_performance = monthly_state.into_items();

    let measured_months: Vec<_> = monthly_performance
        .iter()
        .filter(|month| month.precision_pct.is_some() || month.fpr_pct.is_some())
        .collect();
    let recipe_perf_trend: Vec<RecipePerfRow> = measured_months
        .iter()
        .map(|month| RecipePerfRow {
            month: month.month_start.format("%b %Y").to_string(),
            precision_display: month
                .precision_pct
                .map(|value| format!("{}%", value.round() as i64))
                .unwrap_or_else(|| "not measured".to_string()),
            fpr_display: month
                .fpr_pct
                .map(|value| format!("{}%", value.round() as i64))
                .unwrap_or_else(|| "not measured".to_string()),
            reviewed_warnings: month.reviewed_warnings,
        })
        .collect();

    let recipe_chart_w = (recipe_perf_trend.len() as i64 * 22).max(22);
    let precision_points: String = measured_months
        .iter()
        .enumerate()
        .filter_map(|(i, month)| {
            month
                .precision_pct
                .map(|value| format!("{},{}", i as i64 * 22, 100 - value.round() as i64))
        })
        .collect::<Vec<_>>()
        .join(" ");
    let fpr_points: String = measured_months
        .iter()
        .enumerate()
        .filter_map(|(i, month)| {
            month
                .fpr_pct
                .map(|value| format!("{},{}", i as i64 * 22, 100 - value.round() as i64))
        })
        .collect::<Vec<_>>()
        .join(" ");
    let precision_circles: Vec<RecipePerfPoint> = measured_months
        .iter()
        .enumerate()
        .filter_map(|(i, month)| {
            month.precision_pct.map(|value| RecipePerfPoint {
                x: i as i64 * 22,
                y: 100 - value.round() as i64,
                title: format!(
                    "{} precision {}% ({} reviewed)",
                    month.month_start.format("%b %Y"),
                    value.round() as i64,
                    month.reviewed_warnings
                ),
            })
        })
        .collect();
    let fpr_circles: Vec<RecipePerfPoint> = measured_months
        .iter()
        .enumerate()
        .filter_map(|(i, month)| {
            month.fpr_pct.map(|value| RecipePerfPoint {
                x: i as i64 * 22,
                y: 100 - value.round() as i64,
                title: format!(
                    "{} false positive rate {}%",
                    month.month_start.format("%b %Y"),
                    value.round() as i64
                ),
            })
        })
        .collect();

    let total_pages = if per_page > 0 {
        (total + per_page - 1) / per_page
    } else {
        0
    };

    let tpl = RecipesListPage {
        current_path: ctx.current_path,
        can_admin: ctx.can_admin,
        can_write: ctx.can_write,
        status_strip: crate::system_status::StatusStrip::current(),
        username: ctx.username,
        warning_count: ctx.warning_count,
        theme: ctx.theme,
        recipes,
        total,
        page,
        per_page,
        total_pages,
        active_status,
        sort_field: sort_field_param,
        sort_dir: sort_dir_param,
        active_recipes_count,
        production_count,
        total_fired,
        avg_success_rate,
        total_runs_sum,
        avg_precision,
        avg_coverage,
        precision_points,
        fpr_points,
        precision_circles,
        fpr_circles,
        recipe_chart_w,
        recipe_perf_trend,
        degraded_notice: degraded_notice.clone(),
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
            avg_coverage: tpl.avg_coverage,
            precision_points: tpl.precision_points.clone(),
            fpr_points: tpl.fpr_points.clone(),
            precision_circles: tpl.precision_circles.clone(),
            fpr_circles: tpl.fpr_circles.clone(),
            recipe_chart_w: tpl.recipe_chart_w,
            recipe_perf_trend: tpl.recipe_perf_trend.clone(),
            degraded_notice: tpl.degraded_notice.clone(),
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
    let (context, degraded_notice) = recipe_new_page_context(&session, &store).await;
    let tpl = RecipeNewPage {
        current_path: context.current_path,
        can_admin: context.can_admin,
        can_write: context.can_write,
        status_strip: crate::system_status::StatusStrip::current(),
        username: context.username,
        warning_count: context.warning_count,
        theme: context.theme,
        degraded_notice,
        form: RecipeFormValues::default(),
        form_error: None,
        categories: category_options(&RecipeFormValues::default().category),
        severities: severity_options(&RecipeFormValues::default().severity),
    };

    let _ = is_htmx_request(&headers);
    super::render_template(&tpl)
}

async fn recipe_new_page_context(
    session: &WebSession,
    store: &Arc<PgStore>,
) -> (super::PageContext, Option<String>) {
    let mut degraded_notice: Option<String> = None;
    let unack_state = DataState::from_result(
        store
            .count_warnings(&WarningListFilters {
                acknowledged: Some(false),
                ..Default::default()
            })
            .await,
        "count_warnings failed (web new recipe page)",
        |_| false,
    );
    DegradedNotice::capture(&unack_state, &mut degraded_notice);
    let context = PageContext::from_session(session, "/recipes", unack_state.into_loaded_or(0));
    (context, degraded_notice)
}

// ─── Recipe creation (B307, #151/#147) ──────────────────────────────────────
//
// The form on /recipes/new posted to `/recipes/create-form`, a route that was
// never registered — the only user-facing creation flow in the product 404'd
// on submit. Creation now validates the submitted definition against the
// engine's real vocabulary (operators/transforms), requires everything the
// runtime needs to evaluate the recipe, records who created it, and persists
// the canonical `recipes` columns the worker reconstructs from. A definition
// written here is therefore a recipe the engine can actually run.

use apex_core::schemas::{SignalSpec, TransformSpec};

/// Lifecycle status the creation flow writes. `build_engine_recipes` maps this
/// to `RecipeStatus::Staged` (shadow evaluation until an explicit promotion),
/// so a UI-created recipe is loaded, never skipped as an unknown status.
pub const RECIPE_CREATE_STATUS: &str = "staging";

const RECIPE_NAME_MAX_LEN: usize = 200;

/// Canonical categories the worker scores and gates by. A free-form string
/// would fall through every category-specific gate.
pub const RECIPE_CATEGORIES: [&str; 7] = [
    "demand_procurement",
    "competitor_market",
    "regulatory_policy",
    "supply_chain_risk",
    "cybersecurity_threat",
    "strategic_poi",
    "geopolitical_analysis",
];

/// Analyst-facing severity the narrative template may reference via
/// `{{severity}}`. Stored in the recipe definition; the runtime alert severity
/// still comes from the evaluated thresholds.
pub const RECIPE_SEVERITIES: [&str; 4] = ["low", "medium", "high", "critical"];

#[derive(Debug, Default, serde::Deserialize)]
pub struct RecipeCreateForm {
    pub name: String,
    pub category: Option<String>,
    pub join_type: Option<String>,
    pub outcome: Option<String>,
    pub severity: Option<String>,
    pub description: Option<String>,
    pub narrative_template: Option<String>,
    pub signals_json: Option<String>,
    pub transforms_json: Option<String>,
    pub thresholds_json: Option<String>,
    pub actions_json: Option<String>,
    // ── Structured builder rows (no-JS form). Each row is one repeated key
    //    per column; fully-empty rows are skipped when materializing JSON.
    #[serde(default)]
    pub signal_observation: Vec<String>,
    #[serde(default)]
    pub signal_field: Vec<String>,
    #[serde(default)]
    pub signal_operator: Vec<String>,
    #[serde(default)]
    pub signal_threshold: Vec<String>,
    #[serde(default)]
    pub signal_window_days: Vec<String>,
    #[serde(default)]
    pub transform_kind: Vec<String>,
    #[serde(default)]
    pub transform_field: Vec<String>,
    #[serde(default)]
    pub transform_window_days: Vec<String>,
    #[serde(default)]
    pub threshold_metric: Vec<String>,
    #[serde(default)]
    pub threshold_operator: Vec<String>,
    #[serde(default)]
    pub threshold_value: Vec<String>,
    #[serde(default)]
    pub action_row: Vec<String>,
}

impl RecipeCreateForm {
    /// Parse the recipe builder's `application/x-www-form-urlencoded` body.
    ///
    /// The builder posts one repeated key per row column
    /// (`signal_observation=…&signal_observation=…`), which
    /// `serde_urlencoded` cannot deserialize into a `Vec`: it fails with
    /// "invalid type: string, expected a sequence", so every structured
    /// submission used to be rejected with a bare 422. Grouping the pairs
    /// here preserves row order and repeats the way the HTML form defines
    /// them, while single-valued fields take their last occurrence.
    fn from_urlencoded(body: &[u8]) -> Self {
        const ROW_KEYS: [&str; 12] = [
            "signal_observation",
            "signal_field",
            "signal_operator",
            "signal_threshold",
            "signal_window_days",
            "transform_kind",
            "transform_field",
            "transform_window_days",
            "threshold_metric",
            "threshold_operator",
            "threshold_value",
            "action_row",
        ];
        let mut rows: HashMap<&str, Vec<String>> = HashMap::new();
        let mut singles: HashMap<String, String> = HashMap::new();
        for (key, value) in url::form_urlencoded::parse(body) {
            if let Some(row_key) = ROW_KEYS.iter().find(|row_key| **row_key == key.as_ref()) {
                rows.entry(row_key).or_default().push(value.into_owned());
            } else {
                singles.insert(key.into_owned(), value.into_owned());
            }
        }
        let mut take = |key: &str| singles.remove(key);
        let mut take_row = |key: &'static str| rows.remove(key).unwrap_or_default();
        RecipeCreateForm {
            name: take("name").unwrap_or_default(),
            category: take("category"),
            join_type: take("join_type"),
            outcome: take("outcome"),
            severity: take("severity"),
            description: take("description"),
            narrative_template: take("narrative_template"),
            signals_json: take("signals_json"),
            transforms_json: take("transforms_json"),
            thresholds_json: take("thresholds_json"),
            actions_json: take("actions_json"),
            signal_observation: take_row("signal_observation"),
            signal_field: take_row("signal_field"),
            signal_operator: take_row("signal_operator"),
            signal_threshold: take_row("signal_threshold"),
            signal_window_days: take_row("signal_window_days"),
            transform_kind: take_row("transform_kind"),
            transform_field: take_row("transform_field"),
            transform_window_days: take_row("transform_window_days"),
            threshold_metric: take_row("threshold_metric"),
            threshold_operator: take_row("threshold_operator"),
            threshold_value: take_row("threshold_value"),
            action_row: take_row("action_row"),
        }
    }
}

fn row(rows: &[String], index: usize) -> Option<&str> {
    rows.get(index)
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
}

fn parse_row_number(raw: Option<&str>) -> Option<serde_json::Value> {
    raw.and_then(|value| value.trim().parse::<f64>().ok())
        .map(|value| {
            if value.fract() == 0.0 && value.abs() < 1e15 {
                serde_json::Value::from(value as i64)
            } else {
                serde_json::Value::from(value)
            }
        })
}

/// Materialize the canonical `*_json` sections from the structured no-JS
/// builder rows. Rows the analyst left blank are skipped; only sections with
/// at least one filled row are generated, so a hand-pasted JSON section (if
/// any) still wins when the row arrays are empty.
fn materialize_builder_rows(form: &mut RecipeCreateForm) {
    if !form.signal_observation.iter().any(|v| !v.trim().is_empty())
        && !form.signal_field.iter().any(|v| !v.trim().is_empty())
    {
        // no structured rows supplied — leave any JSON as-is
    } else {
        let mut signals: Vec<serde_json::Value> = Vec::new();
        let len = form
            .signal_observation
            .len()
            .max(form.signal_field.len())
            .max(form.signal_operator.len());
        for index in 0..len {
            let observation = row(&form.signal_observation, index);
            let field = row(&form.signal_field, index).unwrap_or("count");
            let operator = row(&form.signal_operator, index).unwrap_or("above");
            if observation.is_none() && field == "count" && operator == "above" {
                continue; // untouched default row
            }
            let Some(observation) = observation else {
                continue;
            };
            let mut object = serde_json::Map::new();
            object.insert(
                "observation_type".to_string(),
                serde_json::Value::from(observation),
            );
            object.insert("field".to_string(), serde_json::Value::from(field));
            object.insert("operator".to_string(), serde_json::Value::from(operator));
            if let Some(threshold) = parse_row_number(row(&form.signal_threshold, index)) {
                object.insert("threshold".to_string(), threshold);
            }
            if let Some(window) = parse_row_number(row(&form.signal_window_days, index)) {
                object.insert("window_days".to_string(), window);
            }
            signals.push(serde_json::Value::Object(object));
        }
        if !signals.is_empty() {
            form.signals_json = Some(serde_json::to_string(&signals).unwrap_or_default());
        }
    }

    if form.transform_kind.iter().any(|v| !v.trim().is_empty()) {
        let mut transforms: Vec<serde_json::Value> = Vec::new();
        let len = form.transform_kind.len();
        for index in 0..len {
            let Some(kind) = row(&form.transform_kind, index) else {
                continue;
            };
            let mut object = serde_json::Map::new();
            object.insert("transform_type".to_string(), serde_json::Value::from(kind));
            object.insert(
                "field".to_string(),
                serde_json::Value::from(row(&form.transform_field, index).unwrap_or("count")),
            );
            let window = row(&form.transform_window_days, index)
                .and_then(|value| value.parse::<i32>().ok())
                .unwrap_or(30);
            object.insert("window_days".to_string(), serde_json::Value::from(window));
            object.insert(
                "params".to_string(),
                serde_json::Value::Object(serde_json::Map::new()),
            );
            transforms.push(serde_json::Value::Object(object));
        }
        if !transforms.is_empty() {
            form.transforms_json = Some(serde_json::to_string(&transforms).unwrap_or_default());
        }
    }

    if form.threshold_metric.iter().any(|v| !v.trim().is_empty()) {
        let mut thresholds: Vec<serde_json::Value> = Vec::new();
        let len = form.threshold_metric.len();
        for index in 0..len {
            let Some(metric) = row(&form.threshold_metric, index) else {
                continue;
            };
            let mut object = serde_json::Map::new();
            object.insert("metric".to_string(), serde_json::Value::from(metric));
            object.insert(
                "operator".to_string(),
                serde_json::Value::from(row(&form.threshold_operator, index).unwrap_or(">=")),
            );
            if let Some(value) = parse_row_number(row(&form.threshold_value, index)) {
                object.insert("value".to_string(), value);
            }
            thresholds.push(serde_json::Value::Object(object));
        }
        if !thresholds.is_empty() {
            form.thresholds_json = Some(serde_json::to_string(&thresholds).unwrap_or_default());
        }
    }

    if form.action_row.iter().any(|v| !v.trim().is_empty()) {
        let actions: Vec<String> = form
            .action_row
            .iter()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect();
        if !actions.is_empty() {
            form.actions_json = Some(serde_json::to_string(&actions).unwrap_or_default());
        }
    }
}

/// One structured signal row as submitted (padded to the fixed row count on
/// re-render so a rejected submission keeps the analyst's input).
#[derive(Debug, Clone, Default)]
pub struct SignalRowValues {
    pub observation: String,
    pub field: String,
    pub operator: String,
    pub threshold: String,
    pub window_days: String,
}

#[derive(Debug, Clone, Default)]
pub struct TransformRowValues {
    pub kind: String,
    pub field: String,
    pub window_days: String,
}

#[derive(Debug, Clone, Default)]
pub struct ThresholdRowValues {
    pub metric: String,
    pub operator: String,
    pub value: String,
}

const SIGNAL_ROW_COUNT: usize = 3;
const TRANSFORM_ROW_COUNT: usize = 2;
const THRESHOLD_ROW_COUNT: usize = 2;
const ACTION_ROW_COUNT: usize = 3;

/// Raw submitted values, kept verbatim so a rejected submission re-renders the
/// form with the analyst's input instead of an empty page.
#[derive(Debug, Clone)]
pub struct RecipeFormValues {
    pub name: String,
    pub category: String,
    pub join_type: String,
    pub outcome: String,
    pub severity: String,
    pub description: String,
    pub narrative_template: String,
    pub signals_json: String,
    pub transforms_json: String,
    pub thresholds_json: String,
    pub actions_json: String,
    pub signal_rows: Vec<SignalRowValues>,
    pub transform_rows: Vec<TransformRowValues>,
    pub threshold_rows: Vec<ThresholdRowValues>,
    pub action_rows: Vec<String>,
}

impl Default for RecipeFormValues {
    fn default() -> Self {
        Self {
            name: String::new(),
            category: RECIPE_CATEGORIES[0].to_string(),
            join_type: String::new(),
            outcome: String::new(),
            severity: RECIPE_SEVERITIES[1].to_string(),
            description: String::new(),
            // Scaffold the two required narrative fields so a first recipe is
            // creatable without the user inventing boilerplate; both remain
            // editable and are validated exactly as before.
            narrative_template: "{{entity_name}}: {{summary}}".to_string(),
            signals_json: String::new(),
            transforms_json: String::new(),
            thresholds_json: String::new(),
            actions_json: String::new(),
            signal_rows: vec![SignalRowValues::default(); SIGNAL_ROW_COUNT],
            transform_rows: vec![TransformRowValues::default(); TRANSFORM_ROW_COUNT],
            threshold_rows: vec![ThresholdRowValues::default(); THRESHOLD_ROW_COUNT],
            action_rows: {
                let mut rows = vec![String::new(); ACTION_ROW_COUNT];
                if let Some(first) = rows.first_mut() {
                    *first = "Review the matched signals and decide whether to engage.".to_string();
                }
                rows
            },
        }
    }
}

impl RecipeFormValues {
    fn from_submitted(form: &RecipeCreateForm) -> Self {
        let defaults = Self::default();
        let mut signal_rows: Vec<SignalRowValues> = (0..SIGNAL_ROW_COUNT)
            .map(|index| SignalRowValues {
                observation: form
                    .signal_observation
                    .get(index)
                    .cloned()
                    .unwrap_or_default(),
                field: form.signal_field.get(index).cloned().unwrap_or_default(),
                operator: form.signal_operator.get(index).cloned().unwrap_or_default(),
                threshold: form
                    .signal_threshold
                    .get(index)
                    .cloned()
                    .unwrap_or_default(),
                window_days: form
                    .signal_window_days
                    .get(index)
                    .cloned()
                    .unwrap_or_default(),
            })
            .collect();
        if let Some(first) = signal_rows.first_mut() {
            if first.operator.is_empty() {
                first.operator = "above".to_string();
            }
        }
        let transform_rows: Vec<TransformRowValues> = (0..TRANSFORM_ROW_COUNT)
            .map(|index| TransformRowValues {
                kind: form.transform_kind.get(index).cloned().unwrap_or_default(),
                field: form.transform_field.get(index).cloned().unwrap_or_default(),
                window_days: form
                    .transform_window_days
                    .get(index)
                    .cloned()
                    .unwrap_or_default(),
            })
            .collect();
        let mut threshold_rows: Vec<ThresholdRowValues> = (0..THRESHOLD_ROW_COUNT)
            .map(|index| ThresholdRowValues {
                metric: form
                    .threshold_metric
                    .get(index)
                    .cloned()
                    .unwrap_or_default(),
                operator: form
                    .threshold_operator
                    .get(index)
                    .cloned()
                    .unwrap_or_default(),
                value: form.threshold_value.get(index).cloned().unwrap_or_default(),
            })
            .collect();
        if let Some(first) = threshold_rows.first_mut() {
            if first.operator.is_empty() {
                first.operator = ">=".to_string();
            }
        }
        let mut action_rows: Vec<String> = (0..ACTION_ROW_COUNT)
            .map(|index| form.action_row.get(index).cloned().unwrap_or_default())
            .collect();
        if action_rows.iter().all(|value| value.trim().is_empty()) {
            action_rows[0] = String::new();
        }
        Self {
            name: form.name.clone(),
            category: form
                .category
                .clone()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or(defaults.category),
            join_type: form.join_type.clone().unwrap_or_default(),
            outcome: form.outcome.clone().unwrap_or_default(),
            severity: form
                .severity
                .clone()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or(defaults.severity),
            description: form.description.clone().unwrap_or_default(),
            narrative_template: form.narrative_template.clone().unwrap_or_default(),
            signals_json: form.signals_json.clone().unwrap_or_default(),
            transforms_json: form.transforms_json.clone().unwrap_or_default(),
            thresholds_json: form.thresholds_json.clone().unwrap_or_default(),
            actions_json: form.actions_json.clone().unwrap_or_default(),
            signal_rows,
            transform_rows,
            threshold_rows,
            action_rows,
        }
    }
}

/// A submitted definition that passed engine-level validation. The `*_json`
/// values are exactly what is persisted into the canonical `recipes` columns.
#[derive(Debug)]
pub struct ValidatedRecipeDefinition {
    pub name: String,
    pub category: String,
    pub join_type: Option<String>,
    pub outcome: Option<String>,
    pub severity: String,
    pub description: String,
    pub narrative_template: String,
    pub signals: Vec<SignalSpec>,
    pub signals_json: serde_json::Value,
    pub transforms: Vec<TransformSpec>,
    pub transforms_json: serde_json::Value,
    pub thresholds_json: serde_json::Value,
    pub action_playbook: Vec<String>,
    pub action_playbook_json: serde_json::Value,
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

/// Normalize a declared transform type to the engine's snake-case vocabulary.
/// Mirrors the worker's loader (`normalize_transform_type`) so validation here
/// agrees with what the runtime will actually map; `lag`/`rolling_mean` pass
/// through and are then refused by the engine.
fn normalize_transform_type(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "zscore" | "z_score" => "zscore".to_string(),
        "pctchange" | "pct_change" | "percentchange" => "pct_change".to_string(),
        "rollingmean" | "rolling_mean" => "rolling_mean".to_string(),
        "count" => "count".to_string(),
        "diff" | "difference" => "diff".to_string(),
        other => other.to_string(),
    }
}

fn require_non_empty(field: &str, value: Option<&str>) -> Result<String, String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("{field} is required"))
}

fn parse_recipe_section(raw: Option<&str>, field_name: &str) -> Result<serde_json::Value, String> {
    let source = raw
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("[]");
    serde_json::from_str(source).map_err(|error| format!("{field_name}: {error}"))
}

/// Parse and validate the signals section into engine [`SignalSpec`]s.
///
/// The stored JSON must stay in the canonical structured shape (`field`,
/// `operator`, `threshold`, ...), because the worker reconstructs runtime
/// signals from these columns: a shape only this handler understood would
/// create a recipe that never runs.
fn parse_and_validate_signals(
    raw: Option<&str>,
) -> Result<(Vec<SignalSpec>, serde_json::Value), String> {
    let value = parse_recipe_section(raw, "signals_json")?;
    let items = value
        .as_array()
        .ok_or_else(|| "signals_json must be a JSON array".to_string())?;
    if items.is_empty() {
        return Err("signals_json must contain at least one signal".to_string());
    }

    let mut normalized = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let object = item
            .as_object()
            .ok_or_else(|| format!("signals_json[{index}] must be a JSON object"))?;
        let mut signal = object.clone();
        // The worker also accepts `observation`; canonicalize on the way in.
        if !signal.contains_key("observation_type") {
            if let Some(observation) = signal.get("observation").cloned() {
                signal.insert("observation_type".to_string(), observation);
            }
        }
        if !signal.contains_key("field") {
            signal.insert(
                "field".to_string(),
                serde_json::Value::String("count".to_string()),
            );
        }
        if !signal.contains_key("operator") {
            signal.insert(
                "operator".to_string(),
                serde_json::Value::String("above".to_string()),
            );
        }
        normalized.push(serde_json::Value::Object(signal));
    }

    let signals: Vec<SignalSpec> = serde_json::from_value(serde_json::Value::Array(normalized))
        .map_err(|error| format!("signals_json: {error}"))?;

    for (index, signal) in signals.iter().enumerate() {
        if signal.observation_type.trim().is_empty() {
            return Err(format!(
                "signals_json[{index}].observation_type is required"
            ));
        }
        if signal.field.trim().is_empty() {
            return Err(format!("signals_json[{index}].field is required"));
        }
        if !apex_recipes::engine::is_supported_operator(&signal.operator) {
            return Err(format!(
                "signals_json[{index}].operator '{}' is not supported (expected one of: increase, decrease, above, below, equals, contains)",
                signal.operator
            ));
        }
    }

    let normalized_json = serde_json::to_value(&signals)
        .map_err(|error| format!("signals_json: failed to serialize: {error}"))?;
    Ok((signals, normalized_json))
}

/// Parse and validate the transforms section into engine [`TransformSpec`]s.
///
/// Transforms are stored with the `type` key the worker's loader reads, in a
/// field-mapped one-per-signal set — the only shape the engine applies. An
/// unmappable set is rejected here instead of being silently dropped at
/// runtime (which would evaluate the recipe untransformed).
fn parse_and_validate_transforms(
    raw: Option<&str>,
    signal_count: usize,
) -> Result<(Vec<TransformSpec>, serde_json::Value), String> {
    #[derive(serde::Deserialize)]
    struct SubmittedTransform {
        #[serde(rename = "transform_type", default)]
        engine_type: Option<String>,
        #[serde(rename = "type", default)]
        declared_type: Option<String>,
        #[serde(default)]
        field: Option<String>,
        #[serde(default)]
        window_days: Option<i32>,
        #[serde(flatten)]
        extra: std::collections::BTreeMap<String, serde_json::Value>,
    }

    let value = parse_recipe_section(raw, "transforms_json")?;
    let items = value
        .as_array()
        .ok_or_else(|| "transforms_json must be a JSON array".to_string())?;
    if items.is_empty() {
        return Ok((Vec::new(), serde_json::Value::Array(Vec::new())));
    }

    if items.len() != signal_count {
        return Err(format!(
            "transforms_json must declare exactly one transform per signal ({signal_count} signals, {} transforms)",
            items.len()
        ));
    }

    let mut stored = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let submitted: SubmittedTransform = serde_json::from_value(item.clone())
            .map_err(|error| format!("transforms_json[{index}]: {error}"))?;
        let normalized_type = normalize_transform_type(
            submitted
                .engine_type
                .as_deref()
                .or(submitted.declared_type.as_deref())
                .unwrap_or_default(),
        );
        if !apex_recipes::engine::is_supported_transform(&normalized_type) {
            return Err(format!(
                "transforms_json[{index}].type '{}' is not supported (expected one of: zscore, pct_change, count, diff)",
                normalized_type
            ));
        }
        let field = submitted
            .field
            .as_deref()
            .map(str::trim)
            .filter(|field| !field.is_empty())
            .ok_or_else(|| format!("transforms_json[{index}].field is required"))?;

        let mut stored_transform = serde_json::Map::new();
        stored_transform.insert(
            "type".to_string(),
            serde_json::Value::String(normalized_type),
        );
        stored_transform.insert(
            "field".to_string(),
            serde_json::Value::String(field.to_string()),
        );
        stored_transform.insert(
            "window_days".to_string(),
            serde_json::Value::from(submitted.window_days.unwrap_or(0)),
        );
        for (key, value) in submitted.extra {
            stored_transform.entry(key).or_insert(value);
        }
        stored.push(serde_json::Value::Object(stored_transform));
    }

    let transforms: Vec<TransformSpec> = serde_json::from_value(serde_json::Value::Array(
        stored
            .iter()
            .map(|transform| {
                let mut engine_shape = transform.clone();
                if let Some(object) = engine_shape.as_object_mut() {
                    if let Some(transform_type) = object.remove("type") {
                        object.insert("transform_type".to_string(), transform_type);
                    }
                    if !object.contains_key("params") {
                        object.insert("params".to_string(), serde_json::json!({}));
                    }
                }
                engine_shape
            })
            .collect(),
    ))
    .map_err(|error| format!("transforms_json: {error}"))?;

    Ok((transforms, serde_json::Value::Array(stored)))
}

/// Parse the action playbook into the `Vec<String>` shape the worker's loader
/// reads. Objects are accepted for convenience by picking their human-readable
/// text field, but the persisted value is always a string array.
fn parse_action_playbook(raw: Option<&str>) -> Result<(Vec<String>, serde_json::Value), String> {
    let value = parse_recipe_section(raw, "actions_json")?;
    let items = value
        .as_array()
        .ok_or_else(|| "actions_json must be a JSON array".to_string())?;

    let mut actions = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let text = match item {
            serde_json::Value::String(text) => text.trim().to_string(),
            serde_json::Value::Object(fields) => ["action", "text", "template", "description"]
                .iter()
                .find_map(|key| fields.get(*key).and_then(serde_json::Value::as_str))
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_string)
                .ok_or_else(|| {
                    format!("actions_json[{index}]: object needs a non-empty 'action' string")
                })?,
            _ => {
                return Err(format!(
                    "actions_json[{index}] must be a string or an object with an 'action' string"
                ))
            }
        };
        if text.is_empty() {
            return Err(format!("actions_json[{index}] must not be empty"));
        }
        actions.push(text);
    }

    if actions.is_empty() {
        return Err(
            "actions_json must contain at least one action (the action template is required)"
                .to_string(),
        );
    }

    Ok((
        actions.clone(),
        serde_json::to_value(&actions)
            .map_err(|error| format!("actions_json: failed to serialize: {error}"))?,
    ))
}

/// Validate a submitted form into the canonical definition the store persists.
///
/// Required fields: name, category, narrative template, at least one valid
/// signal, and an action template. Everything the runtime needs to load and
/// evaluate the recipe is checked here, so creation cannot produce a recipe
/// that is silently excluded or never fires.
pub fn validate_recipe_definition(
    form: &RecipeCreateForm,
) -> Result<ValidatedRecipeDefinition, String> {
    let name = require_non_empty("name", Some(form.name.as_str()))?;
    if name.chars().count() > RECIPE_NAME_MAX_LEN {
        return Err(format!(
            "name must be at most {RECIPE_NAME_MAX_LEN} characters"
        ));
    }

    let category = require_non_empty("category", form.category.as_deref())?;
    if !RECIPE_CATEGORIES.contains(&category.as_str()) {
        return Err(format!(
            "category '{category}' is not supported (expected one of: {})",
            RECIPE_CATEGORIES.join(", ")
        ));
    }

    let narrative_template =
        require_non_empty("narrative_template", form.narrative_template.as_deref())?;

    let (signals, signals_json) = parse_and_validate_signals(form.signals_json.as_deref())?;
    let (transforms, transforms_json) =
        parse_and_validate_transforms(form.transforms_json.as_deref(), signals.len())?;
    let thresholds_json = parse_recipe_section(form.thresholds_json.as_deref(), "thresholds_json")?;
    if !thresholds_json.is_array() && !thresholds_json.is_object() {
        return Err("thresholds_json must be a JSON array or object".to_string());
    }
    let (action_playbook, action_playbook_json) =
        parse_action_playbook(form.actions_json.as_deref())?;

    let join_type = form
        .join_type
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let outcome = form
        .outcome
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);

    let severity = form
        .severity
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(RECIPE_SEVERITIES[1]);
    if !RECIPE_SEVERITIES.contains(&severity) {
        return Err(format!(
            "severity '{severity}' is not supported (expected one of: {})",
            RECIPE_SEVERITIES.join(", ")
        ));
    }
    let description = form
        .description
        .as_deref()
        .map(str::trim)
        .unwrap_or_default()
        .to_string();

    Ok(ValidatedRecipeDefinition {
        name,
        category,
        join_type,
        outcome,
        severity: severity.to_string(),
        description,
        narrative_template,
        signals,
        signals_json,
        transforms,
        transforms_json,
        thresholds_json,
        action_playbook,
        action_playbook_json,
    })
}

fn category_options(selected: &str) -> Vec<(&'static str, bool)> {
    RECIPE_CATEGORIES
        .iter()
        .map(|category| (*category, *category == selected))
        .collect()
}

fn severity_options(selected: &str) -> Vec<(&'static str, bool)> {
    RECIPE_SEVERITIES
        .iter()
        .map(|severity| (*severity, *severity == selected))
        .collect()
}

async fn render_recipe_new_error(
    session: &WebSession,
    store: &Arc<PgStore>,
    values: RecipeFormValues,
    error: String,
    status: axum::http::StatusCode,
) -> axum::response::Response {
    let (context, degraded_notice) = recipe_new_page_context(session, store).await;
    let tpl = RecipeNewPage {
        current_path: context.current_path,
        can_admin: context.can_admin,
        can_write: context.can_write,
        status_strip: crate::system_status::StatusStrip::current(),
        username: context.username,
        warning_count: context.warning_count,
        theme: context.theme,
        degraded_notice,
        categories: category_options(&values.category),
        severities: severity_options(&values.severity),
        form: values,
        form_error: Some(error),
    };
    super::render_template_with_status(status, &tpl)
}

/// POST /recipes/create-form — validate the submitted definition and persist
/// it as a `staging` recipe (promoted explicitly via POST /api/recipes/:id/promote).
pub async fn create_recipe_form(
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
    axum::extract::RawForm(body): axum::extract::RawForm,
) -> axum::response::Response {
    let mut form = RecipeCreateForm::from_urlencoded(&body);
    materialize_builder_rows(&mut form);
    let submitted_values = RecipeFormValues::from_submitted(&form);

    let definition = match validate_recipe_definition(&form) {
        Ok(definition) => definition,
        Err(error) => {
            return render_recipe_new_error(
                &session,
                &store,
                submitted_values,
                error,
                axum::http::StatusCode::BAD_REQUEST,
            )
            .await
        }
    };

    let mut code = slugify_recipe_code(&definition.name);
    if code.len() > 56 {
        code.truncate(56);
    }
    // Unique suffix keeps repeated submissions from overwriting each other.
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    code = format!("{}_{}", code, &suffix[..8]);

    // Canonical persistence: the worker reconstructs runtime recipes from
    // exactly these columns, so the created recipe executes without any
    // definition-blob dependency.
    match store
        .insert_recipe_canonical(
            &code,
            &definition.name,
            &definition.category,
            definition.join_type.as_deref().unwrap_or(""),
            definition.outcome.as_deref().unwrap_or(""),
            &definition.signals_json,
            &definition.transforms_json,
            &definition.thresholds_json,
            &definition.narrative_template,
            &definition.action_playbook_json,
            &definition.severity,
            &definition.description,
            session.user_id.as_str(),
        )
        .await
    {
        Ok(_) => axum::response::Redirect::to("/recipes?created=1").into_response(),
        Err(err) => {
            tracing::error!(recipe_code = %code, "recipe create form failed: {err:#}");
            render_recipe_new_error(
                &session,
                &store,
                submitted_values,
                "Failed to create recipe — the definition was valid but could not be stored. See server logs.".to_string(),
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            )
            .await
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn valid_form() -> RecipeCreateForm {
        RecipeCreateForm {
            name: "Competitor Patent Surge".to_string(),
            category: Some("competitor_market".to_string()),
            join_type: Some("company".to_string()),
            outcome: Some("signal".to_string()),
            narrative_template: Some("{{entity_name}} filed a patent surge".to_string()),
            signals_json: Some(
                r#"[{"observation_type":"Patent","field":"count","operator":"above","threshold":3,"window_days":30}]"#
                    .to_string(),
            ),
            transforms_json: Some("[]".to_string()),
            thresholds_json: Some(r#"[{"metric":"count","operator":">=","value":3}]"#.to_string()),
            actions_json: Some(r#"["Review the patent filings before outreach."]"#.to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn repeated_row_keys_deserialize_into_ordered_row_vectors() {
        // The structured builder posts one repeated key per row column;
        // serde_urlencoded rejects that shape, so the handler parses pairs.
        let body = "name=Patent+surge&category=competitor_market&severity=high\
                    &signal_observation=NewsArticle&signal_observation=JobPost\
                    &signal_field=count&signal_field=count\
                    &signal_operator=above&signal_operator=increase\
                    &signal_threshold=3&signal_threshold=&signal_window_days=30\
                    &signal_window_days=14&action_row=Review";
        let form = RecipeCreateForm::from_urlencoded(body.as_bytes());
        assert_eq!(form.name, "Patent surge");
        assert_eq!(form.severity.as_deref(), Some("high"));
        assert_eq!(form.signal_observation, vec!["NewsArticle", "JobPost"]);
        assert_eq!(form.signal_operator, vec!["above", "increase"]);
        assert_eq!(form.signal_threshold, vec!["3", ""]);
        assert_eq!(form.signal_window_days, vec!["30", "14"]);
        assert_eq!(form.action_row, vec!["Review"]);

        // Duplicate single-valued keys keep the last value (browser order).
        let form = RecipeCreateForm::from_urlencoded(b"name=First&name=Second");
        assert_eq!(form.name, "Second");
    }

    #[test]
    fn valid_submission_parses_into_engine_types() {
        let definition = validate_recipe_definition(&valid_form()).expect("valid form");
        assert_eq!(definition.name, "Competitor Patent Surge");
        assert_eq!(definition.category, "competitor_market");
        assert_eq!(definition.signals.len(), 1);
        assert!(definition.transforms.is_empty());
        assert_eq!(
            definition.action_playbook,
            vec!["Review the patent filings before outreach.".to_string()]
        );
        assert_eq!(
            definition.action_playbook_json,
            serde_json::json!(["Review the patent filings before outreach."])
        );
    }

    #[test]
    fn submission_without_category_is_refused() {
        let mut form = valid_form();
        form.category = Some("  ".to_string());
        let error = validate_recipe_definition(&form).expect_err("category required");
        assert!(error.contains("category is required"), "{error}");
    }

    #[test]
    fn submission_with_unknown_category_is_refused() {
        let mut form = valid_form();
        form.category = Some("dabbling".to_string());
        let error = validate_recipe_definition(&form).expect_err("category whitelisted");
        assert!(error.contains("not supported"), "{error}");
    }

    #[test]
    fn submission_without_narrative_template_is_refused() {
        let mut form = valid_form();
        form.narrative_template = Some("   ".to_string());
        let error = validate_recipe_definition(&form).expect_err("narrative required");
        assert!(error.contains("narrative_template is required"), "{error}");
    }

    #[test]
    fn submission_without_signals_is_refused() {
        let mut form = valid_form();
        form.signals_json = Some("[]".to_string());
        let error = validate_recipe_definition(&form).expect_err("signals required");
        assert!(error.contains("at least one signal"), "{error}");
    }

    #[test]
    fn submission_with_unsupported_operator_is_refused() {
        let mut form = valid_form();
        form.signals_json = Some(
            r#"[{"observation_type":"Patent","field":"count","operator":"roughly","threshold":3}]"#
                .to_string(),
        );
        let error = validate_recipe_definition(&form).expect_err("operator validated");
        assert!(error.contains("is not supported"), "{error}");
    }

    #[test]
    fn submission_with_unsupported_transform_is_refused() {
        let mut form = valid_form();
        form.transforms_json =
            Some(r#"[{"type":"lag","field":"Patent.count","window_days":7}]"#.to_string());
        let error = validate_recipe_definition(&form).expect_err("transform validated");
        assert!(error.contains("is not supported"), "{error}");

        // rolling_mean is declared by the engine's docs but not implemented.
        form.transforms_json =
            Some(r#"[{"type":"rolling_mean","field":"Patent.count","window_days":7}]"#.to_string());
        let error = validate_recipe_definition(&form).expect_err("rolling_mean refused");
        assert!(error.contains("is not supported"), "{error}");
    }

    #[test]
    fn submission_with_mismatched_transforms_is_refused() {
        let mut form = valid_form();
        form.transforms_json = Some(
            r#"[{"type":"count","field":"Patent.count"},{"type":"count","field":"Patent.count"}]"#
                .to_string(),
        );
        let error = validate_recipe_definition(&form).expect_err("one transform per signal");
        assert!(error.contains("one transform per signal"), "{error}");
    }

    #[test]
    fn submission_without_actions_is_refused() {
        let mut form = valid_form();
        form.actions_json = Some("[]".to_string());
        let error = validate_recipe_definition(&form).expect_err("action template required");
        assert!(error.contains("action template is required"), "{error}");
    }

    #[test]
    fn transform_type_is_normalized_to_the_engine_vocabulary() {
        let mut form = valid_form();
        form.transforms_json =
            Some(r#"[{"type":"ZScore","field":"Patent.count","window_days":7}]"#.to_string());
        let definition = validate_recipe_definition(&form).expect("zscore is supported");
        assert_eq!(definition.transforms.len(), 1);
        assert_eq!(definition.transforms[0].transform_type, "zscore");
        // Stored in the `type` shape the worker's loader reads.
        assert_eq!(
            definition.transforms_json[0]["type"],
            serde_json::Value::String("zscore".to_string())
        );
        assert_eq!(
            definition.transforms_json[0]["field"],
            serde_json::Value::String("Patent.count".to_string())
        );
    }

    #[test]
    fn action_objects_are_stored_as_strings() {
        let mut form = valid_form();
        form.actions_json = Some(
            r#"[{"action":"Investigate the supplier"},{"text":"Notify the account team"}]"#
                .to_string(),
        );
        let definition = validate_recipe_definition(&form).expect("objects accepted");
        assert_eq!(
            definition.action_playbook,
            vec![
                "Investigate the supplier".to_string(),
                "Notify the account team".to_string()
            ]
        );
        assert!(definition.action_playbook_json.is_array());
    }

    #[test]
    fn submission_without_name_is_refused() {
        let mut form = valid_form();
        form.name = "   ".to_string();
        let error = validate_recipe_definition(&form).expect_err("name required");
        assert!(error.contains("name is required"), "{error}");
    }

    /// The created definition must survive the worker's runtime loader: a
    /// recipe is otherwise stored but never evaluated (the original #151/#147
    /// symptom). This mirrors the exclusion conditions in
    /// `crates/worker/src/job_execution/recipes.rs::build_engine_recipes`
    /// (unrecognized status, absent signals, empty narrative) and the extra
    /// skips in `seed_recipe_to_engine_recipe` / `evaluate_recipe_inner`
    /// (unparseable signals, empty action template):
    ///
    /// * `RECIPE_CREATE_STATUS` must map to a loaded `RecipeStatus`;
    /// * `narrative_template` and the persisted signals array must be
    ///   non-empty;
    /// * every persisted signal must deserialize back into a `SignalSpec`
    ///   with an observation type and field;
    /// * the action playbook must produce a non-empty action template;
    /// * any transforms must be a one-per-signal, field-mapped, supported set
    ///   (otherwise the loader drops them and the recipe runs untransformed).
    #[test]
    fn ui_created_recipe_is_runnable_by_build_engine_recipes() {
        let definition = validate_recipe_definition(&valid_form()).expect("valid form");

        let mapped_status = match RECIPE_CREATE_STATUS {
            "staging" => "staged",
            "production" | "active" | "promoted" => "promoted",
            "seed" => "seed",
            other => panic!("build_engine_recipes would exclude status {other:?}"),
        };
        assert_eq!(mapped_status, "staged");
        assert!(!definition.narrative_template.trim().is_empty());
        let stored_signals = definition
            .signals_json
            .as_array()
            .expect("signals stored as an array");
        assert!(!stored_signals.is_empty(), "excluded: no signals");

        let signals: Vec<SignalSpec> =
            serde_json::from_value(definition.signals_json.clone()).expect("engine SignalSpec");
        assert_eq!(signals.len(), definition.signals.len());
        assert!(
            signals
                .iter()
                .all(|signal| !signal.observation_type.trim().is_empty()
                    && !signal.field.trim().is_empty()),
            "every stored signal must parse into a runtime SignalSpec"
        );
        assert!(
            !definition.action_playbook.is_empty(),
            "empty action template would skip evaluation"
        );

        if !definition.transforms.is_empty() {
            let stored_transforms = definition
                .transforms_json
                .as_array()
                .expect("transforms stored as an array");
            assert_eq!(
                stored_transforms.len(),
                signals.len(),
                "loader only carries a one-per-signal transform set"
            );
            for transform in stored_transforms {
                let transform_type = transform
                    .get("type")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default();
                assert!(
                    apex_recipes::engine::is_supported_transform(transform_type),
                    "{transform_type} would be dropped by the loader"
                );
                assert!(
                    !transform
                        .get("field")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .trim()
                        .is_empty(),
                    "field-mapped transforms only"
                );
            }
        }
    }
    // ── #148: sorting ────────────────────────────────────────────────────

    fn item(name: &str, runs: i64, rate: Option<f64>) -> RecipeListItem {
        RecipeListItem {
            id: name.to_string(),
            name: name.to_string(),
            description: String::new(),
            status: "production".to_string(),
            schedule: String::new(),
            total_runs: runs,
            success_rate: rate,
            last_run: None,
            created_at: String::new(),
            updated_at: String::new(),
            tags: vec![],
        }
    }

    #[test]
    fn recipe_sort_applies_field_and_direction() {
        let mut items = vec![
            item("Beta", 5, Some(40.0)),
            item("Alpha", 20, Some(80.0)),
            item("Gamma", 1, None),
        ];

        sort_recipe_items(&mut items, "name", "asc");
        let names: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, vec!["Alpha", "Beta", "Gamma"]);

        sort_recipe_items(&mut items, "runs", "desc");
        let runs: Vec<i64> = items.iter().map(|i| i.total_runs).collect();
        assert_eq!(runs, vec![20, 5, 1]);

        // Unmeasured success sorts last descending, never as 0%.
        sort_recipe_items(&mut items, "success_rate", "desc");
        let rates: Vec<Option<f64>> = items.iter().map(|i| i.success_rate).collect();
        assert_eq!(rates, vec![Some(80.0), Some(40.0), None]);

        // Unknown field falls back to name.
        sort_recipe_items(&mut items, "not_a_field", "desc");
        let names: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, vec!["Gamma", "Beta", "Alpha"]);
    }
}
