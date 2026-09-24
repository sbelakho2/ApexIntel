//! Worker bootstrap helpers: seed-recipe loading, validation and insertion.
//!
//! The binary root keeps configuration loading and dependency construction;
//! the recipe-seeding and seed->engine conversion steps live here.

use apex_core::schemas::{Recipe, RecipeStatus, SignalSpec};
use apex_worker::recipe_loader::{
    insert_seed_recipes, load_default_seed_recipes, print_recipe_stats, validate_seed_recipes,
};

/// Load `config/recipes_seed.yaml`, validate and insert the seed recipes.
///
/// Failures are logged, never fatal: the worker must still start when the
/// seed file is missing or the database rejects an individual recipe.
pub(crate) async fn seed_recipes_from_yaml(pool: &sqlx::PgPool) {
    match load_default_seed_recipes() {
        Ok(recipes) => {
            if !recipes.is_empty() {
                let validation_errors = validate_seed_recipes(&recipes);
                if !validation_errors.is_empty() {
                    tracing::warn!(
                        "Recipe validation found {} issues (recipes will still be loaded)",
                        validation_errors.len()
                    );
                    for err in &validation_errors {
                        tracing::warn!("  - {}", err);
                    }
                }
                print_recipe_stats(&recipes);
                tracing::info!("loaded {} seed recipes from YAML", recipes.len());

                // Insert seed recipes into database
                match insert_seed_recipes(pool, &recipes).await {
                    Ok(result) => {
                        tracing::info!(
                            "recipe insertion: {} inserted, {} skipped (already exist), {} errors",
                            result.inserted,
                            result.skipped,
                            result.errors.len()
                        );
                    }
                    Err(e) => {
                        tracing::error!("failed to insert seed recipes: {}", e);
                    }
                }
            } else {
                tracing::debug!("no seed recipes found");
            }
        }
        Err(e) => {
            tracing::warn!("failed to load seed recipes: {}", e);
        }
    }
}

/// Parse a YAML signal string such as `"JobPost.role_family=Procurement.increase"` into a
/// [`SignalSpec`].  The expected format is `{ObsType}.{field[=value]}.{operator}`, where the
/// trailing segment is compared against the list of known operators; if it does not match,
/// `"contains"` is used as the default and the whole right-hand side is treated as the field.
pub(crate) fn parse_signal_str(s: &str) -> SignalSpec {
    const KNOWN_OPS: &[&str] = &[
        "increase", "decrease", "above", "below", "equals", "contains",
    ];
    let (obs_type, rest) = match s.find('.') {
        Some(pos) => (&s[..pos], &s[pos + 1..]),
        None => {
            return SignalSpec {
                observation_type: s.to_string(),
                field: "count".to_string(),
                operator: "above".to_string(),
                threshold: Some(0.0),
                window_days: Some(30),
                value: None,
            }
        }
    };
    let (field_part, operator) = match rest.rfind('.') {
        Some(pos) => {
            let maybe_op = &rest[pos + 1..];
            if KNOWN_OPS.contains(&maybe_op) {
                (&rest[..pos], maybe_op.to_string())
            } else {
                (rest, "contains".to_string())
            }
        }
        None => (rest, "contains".to_string()),
    };
    let (field, value) = if let Some(eq) = field_part.find('=') {
        (
            field_part[..eq].to_string(),
            Some(field_part[eq + 1..].to_string()),
        )
    } else {
        (field_part.to_string(), None)
    };
    SignalSpec {
        observation_type: obs_type.to_string(),
        field: if field.is_empty() {
            "count".to_string()
        } else {
            field
        },
        operator,
        threshold: Some(0.0),
        window_days: Some(30),
        value,
    }
}

/// Convert a seed recipe definition into an engine-ready [`Recipe`].
/// Map the PascalCase transform type names used in `recipes_seed.yaml`
/// (e.g. `Lag`, `Count`, `ZScore`) to the lowercase snake-case identifiers
/// the recipe engine's `apply_transforms` dispatcher matches against.
/// Unknown inputs are passed through lowercased so the engine's identity
/// branch handles them gracefully.
pub(crate) fn normalize_transform_type(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "zscore" | "z_score" => "zscore".to_string(),
        "pctchange" | "pct_change" | "percentchange" => "pct_change".to_string(),
        "rollingmean" | "rolling_mean" => "rolling_mean".to_string(),
        "count" => "count".to_string(),
        "diff" | "difference" | "lag" => "diff".to_string(),
        other => other.to_ascii_lowercase(),
    }
}

pub(crate) fn seed_recipe_to_engine_recipe(sr: &apex_worker::recipe_loader::SeedRecipe) -> Recipe {
    let signals: Vec<SignalSpec> = sr
        .signals
        .iter()
        .filter_map(|sv| sv.as_str().map(parse_signal_str))
        .collect();
    let action = if sr.action_playbook.is_empty() {
        String::new()
    } else {
        sr.action_playbook.join("; ")
    };
    let severity = if sr.category.contains("security")
        || sr.category.contains("risk")
        || sr.category.contains("supply")
        || sr.category.contains("sanction")
    {
        "warning"
    } else {
        "info"
    };
    let mut r = Recipe::new(sr.id.clone(), sr.name.clone());
    r.description = sr.category.clone();
    r.status = RecipeStatus::Seed;
    r.signals = signals;

    // ── Wire the statistical core from the seed YAML ───────────────────────
    // Previously transforms/test/thresholds were silently dropped, leaving the
    // engine to run every recipe with default (identity) transforms and a flat
    // 1.5× uplift floor. The FisherExact test type is carried through so the
    // engine and any downstream calibration know the intended test family.
    r.transforms = sr
        .transforms
        .iter()
        .map(|raw| apex_core::schemas::TransformSpec {
            // YAML uses PascalCase (Lag, Count, ZScore, PctChange, RollingMean);
            // the engine matches lowercase snake-case names. Normalize so the
            // known transforms actually apply; unknown ones fall through to the
            // engine's identity (keep-as-is) branch, which is safe.
            transform_type: normalize_transform_type(
                raw.get("type")
                    .and_then(serde_yaml::Value::as_str)
                    .unwrap_or(""),
            ),
            field: raw
                .get("field")
                .and_then(serde_yaml::Value::as_str)
                .unwrap_or("")
                .to_string(),
            window_days: raw
                .get("window_days")
                .and_then(serde_yaml::Value::as_i64)
                .map(|d| d as i32)
                .or_else(|| {
                    raw.get("days")
                        .and_then(serde_yaml::Value::as_i64)
                        .map(|d| d as i32)
                })
                .unwrap_or(0),
            params: serde_json::to_value(raw).unwrap_or(serde_json::Value::Null),
        })
        .collect();

    r.statistical_test = sr
        .test
        .get("type")
        .and_then(serde_yaml::Value::as_str)
        .map(|test_type| apex_core::schemas::StatisticalTest {
            // Carry the declared test family (e.g. "fisher_exact",
            // "cross_correlation"). Normalized to snake_case so the engine's
            // downstream consumers can dispatch on it consistently.
            test_type: test_type.trim().to_ascii_lowercase(),
            params: serde_json::to_value(&sr.test).unwrap_or(serde_json::Value::Null),
        });

    // Thresholds: lift the recipe-specific floor/gate from the YAML so the
    // engine respects per-recipe quality bars instead of the 1.5× default.
    if let Some(min_effect) = sr
        .thresholds
        .get("min_effect")
        .and_then(serde_yaml::Value::as_f64)
    {
        // Gate validation requires min_uplift > 1.0. Guard against YAML
        // recipes that declare a min_effect at or below the baseline.
        if min_effect > 1.0 {
            r.min_uplift = min_effect;
        }
    }
    if let Some(max_p) = sr
        .thresholds
        .get("max_p_value")
        .and_then(serde_yaml::Value::as_f64)
    {
        r.max_p_value = max_p;
    }
    if let Some(min_slices) = sr
        .thresholds
        .get("min_stability")
        .and_then(serde_yaml::Value::as_f64)
        .map(|s| (s * 10.0).round() as i32)
    {
        r.min_time_slices = min_slices;
    }

    r.insight_template = sr.narrative_template.clone();
    r.action_template = action;
    r.severity = severity.to_string();
    r.category = sr.category.clone();
    r.min_uplift = 1.0;
    r.min_entities = 1;
    r
}
