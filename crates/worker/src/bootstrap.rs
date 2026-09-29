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
/// The worker must still start when the seed file is missing or the database
/// rejects an individual recipe — but a sync that inserted nothing because a
/// definition failed to serialize or persist is reported as degraded, never
/// as a clean startup.
pub(crate) async fn seed_recipes_from_yaml(pool: &sqlx::PgPool) {
    match load_default_seed_recipes() {
        Ok(recipes) => {
            if recipes.is_empty() {
                tracing::debug!("no seed recipes found");
                return;
            }

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
                Ok(result) if result.errors.is_empty() => {
                    tracing::info!(
                        "recipe seed sync complete: {} inserted, {} skipped (already exist)",
                        result.inserted,
                        result.skipped
                    );
                }
                Ok(result) => {
                    for error in result.errors.iter().take(10) {
                        tracing::error!(%error, "seed recipe insert failed");
                    }
                    tracing::error!(
                        inserted = result.inserted,
                        skipped = result.skipped,
                        errors = result.errors.len(),
                        "recipe seed sync DEGRADED: some recipes were not persisted; recipes in \
                         the database no longer match config/recipes_seed.yaml"
                    );
                }
                Err(e) => {
                    tracing::error!(
                        error = %e,
                        "recipe seed sync FAILED: {} (the sync aborted before completing; \
                         recipes recorded earlier in this run remain committed, so the \
                         database may not match config/recipes_seed.yaml)",
                        e
                    );
                }
            }
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                "recipe seed sync FAILED: could not load config/recipes_seed.yaml"
            );
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
/// Unknown inputs are passed through lowercased; the engine then refuses them
/// (fail closed) rather than treating them as identity.
pub(crate) fn normalize_transform_type(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "zscore" | "z_score" => "zscore".to_string(),
        "pctchange" | "pct_change" | "percentchange" => "pct_change".to_string(),
        "rollingmean" | "rolling_mean" => "rolling_mean".to_string(),
        "count" => "count".to_string(),
        // `lag` is NOT a difference; it is passed through unchanged so the
        // engine refuses it explicitly (x[t-k] != x[t] - x[t-k]).
        "diff" | "difference" => "diff".to_string(),
        other => other.to_ascii_lowercase(),
    }
}

/// Parse one signal from a recipe definition: the seed YAML string form
/// (`JobPost.role_family=Procurement.increase`) or the canonical structured
/// object form (`{observation_type, field, operator, threshold, value}`) that
/// database-written recipes use.
fn parse_signal_value(raw: &serde_yaml::Value) -> Option<SignalSpec> {
    if let Some(as_str) = raw.as_str() {
        return Some(parse_signal_str(as_str));
    }
    let map = raw.as_mapping()?;
    let string_field = |key: &str| {
        map.get(serde_yaml::Value::String(key.to_string()))
            .and_then(serde_yaml::Value::as_str)
    };
    let observation_type =
        string_field("observation_type").or_else(|| string_field("observation"))?;
    Some(SignalSpec {
        observation_type: observation_type.to_string(),
        field: string_field("field").unwrap_or("count").to_string(),
        operator: string_field("operator").unwrap_or("above").to_string(),
        threshold: map
            .get(serde_yaml::Value::String("threshold".to_string()))
            .and_then(serde_yaml::Value::as_f64),
        window_days: map
            .get(serde_yaml::Value::String("window_days".to_string()))
            .and_then(serde_yaml::Value::as_i64)
            .map(|days| days as i32),
        value: string_field("value").map(str::to_string),
    })
}

pub(crate) fn seed_recipe_to_engine_recipe(sr: &apex_worker::recipe_loader::SeedRecipe) -> Recipe {
    let signals: Vec<SignalSpec> = sr.signals.iter().filter_map(parse_signal_value).collect();
    if signals.len() != sr.signals.len() {
        tracing::warn!(
            recipe = %sr.id,
            declared = sr.signals.len(),
            parsed = signals.len(),
            "recipe_loader: some declared signals could not be parsed and are not on the runtime recipe"
        );
    }
    let signal_count = signals.len();
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
    // A four-signal recipe must never fire on two generic observations when it
    // guards a security/compliance decision: those default to an exact match,
    // everything else keeps the historical 50% fraction.
    let lower_category = sr.category.to_ascii_lowercase();
    r.match_policy = if [
        "security",
        "compliance",
        "risk",
        "supply",
        "sanction",
        "fraud",
    ]
    .iter()
    .any(|needle| lower_category.contains(needle))
    {
        apex_core::schemas::MatchPolicy::All
    } else {
        apex_core::schemas::MatchPolicy::default()
    };

    // ── Wire the runtime transforms from the recipe definition ─────────────
    // Transforms are runtime behavior and live on the engine recipe. The test
    // family and thresholds are promotion/discovery criteria (audit P1): they
    // are persisted in the DB `test_config` / `thresholds` columns and
    // evaluated by `apex_recipes::gates`, not carried on the runtime object.
    //
    // The engine applies transforms positionally, one per signal, and each
    // transform must name the feature field whose baseline it reads. Seed YAML
    // historically declares global preprocessing directives (`Lag`/`Count`
    // without a `field`), which the engine cannot honor: carrying them would
    // make `apply_transforms` fail closed and silence every seeded recipe.
    // Only a complete, field-mapped, one-per-signal set of engine-supported
    // transforms is carried; anything else is dropped here with a warning and
    // remains available in the recipe definition / DB columns for
    // promotion-time evaluation.
    let parsed_transforms: Vec<apex_core::schemas::TransformSpec> = sr
        .transforms
        .iter()
        .map(|raw| apex_core::schemas::TransformSpec {
            // YAML uses PascalCase (Lag, Count, ZScore, PctChange, RollingMean);
            // the engine matches lowercase snake-case names. Normalize so the
            // known transforms actually apply; unknown ones are refused by the
            // engine (fail closed) rather than silently passed through.
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

    // The engine owns the supported-transform list (single source of truth).
    let transforms_are_mappable = parsed_transforms.len() == signal_count
        && parsed_transforms.iter().all(|transform| {
            !transform.field.trim().is_empty()
                && apex_recipes::engine::is_supported_transform(&transform.transform_type)
        });
    if !parsed_transforms.is_empty() && !transforms_are_mappable {
        tracing::warn!(
            recipe = %sr.id,
            declared_transforms = parsed_transforms.len(),
            signals = signal_count,
            "recipe_loader: transforms are not a field-mapped one-per-signal set the engine \
             can apply; dropping them from the runtime recipe (they stay in the definition \
             and DB columns)"
        );
    }
    r.transforms = if transforms_are_mappable {
        parsed_transforms
    } else {
        Vec::new()
    };

    r.insight_template = sr.narrative_template.clone();
    r.action_template = action;
    r.severity = severity.to_string();
    r.category = sr.category.clone();
    r
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use apex_worker::recipe_loader::SeedRecipe;

    fn seed(transforms: Vec<serde_yaml::Value>) -> SeedRecipe {
        SeedRecipe {
            id: "T001".to_string(),
            name: "Test".to_string(),
            category: "demand".to_string(),
            join: vec!["company".to_string()],
            outcome: "signal".to_string(),
            signals: vec![
                serde_yaml::Value::String("JobPost.count".to_string()),
                serde_yaml::Value::String("WebChange.drift".to_string()),
            ],
            transforms,
            test: serde_yaml::Value::Null,
            thresholds: serde_yaml::Value::Null,
            narrative_template: "note".to_string(),
            action_playbook: vec!["act".to_string()],
            applicability: serde_yaml::Value::Null,
        }
    }

    /// Global seed directives (no `field`, one per recipe) are not carried:
    /// carrying them would make every seeded recipe fail closed.
    #[test]
    fn unmappable_transform_directives_are_not_carried_into_the_engine() {
        let transform: serde_yaml::Value = serde_yaml::from_str("type: Lag\ndays: 30").unwrap();
        let recipe = seed_recipe_to_engine_recipe(&seed(vec![transform]));
        assert!(
            recipe.transforms.is_empty(),
            "a global directive without a field must not silence the recipe"
        );
    }

    /// A complete field-mapped one-per-signal transform set is carried.
    #[test]
    fn field_mapped_per_signal_transforms_are_carried() {
        let first: serde_yaml::Value =
            serde_yaml::from_str("type: ZScore\nfield: JobPost.count").unwrap();
        let second: serde_yaml::Value =
            serde_yaml::from_str("type: PctChange\nfield: WebChange.drift").unwrap();
        let recipe = seed_recipe_to_engine_recipe(&seed(vec![first, second]));
        assert_eq!(recipe.transforms.len(), 2);
        assert_eq!(recipe.transforms[0].transform_type, "zscore");
        assert_eq!(recipe.transforms[1].transform_type, "pct_change");
    }

    /// An unsupported transform type in an otherwise complete set is still not
    /// carried (the engine would refuse the recipe at runtime).
    #[test]
    fn unsupported_transform_types_are_not_carried() {
        let first: serde_yaml::Value =
            serde_yaml::from_str("type: Lag\nfield: JobPost.count").unwrap();
        let second: serde_yaml::Value =
            serde_yaml::from_str("type: Count\nfield: WebChange.drift").unwrap();
        let recipe = seed_recipe_to_engine_recipe(&seed(vec![first, second]));
        assert!(recipe.transforms.is_empty());
    }

    /// Canonical column recipes may store structured signal objects instead of
    /// seed strings; both forms must reach the engine.
    #[test]
    fn structured_canonical_signals_are_parsed() {
        let mut seed = seed(vec![]);
        seed.signals = vec![serde_yaml::from_str(
            "{observation_type: JobPost, field: role_family, operator: above, threshold: 1.0}",
        )
        .unwrap()];
        let recipe = seed_recipe_to_engine_recipe(&seed);
        assert_eq!(recipe.signals.len(), 1);
        assert_eq!(recipe.signals[0].observation_type, "JobPost");
        assert_eq!(recipe.signals[0].field, "role_family");
        assert_eq!(recipe.signals[0].operator, "above");
        assert_eq!(recipe.signals[0].threshold, Some(1.0));
    }

    /// Regression guard (audit P0/P1): every seeded recipe converts into an
    /// engine recipe with at least one signal and a transform/signal mapping
    /// the engine accepts — the fail-closed transform check must never silence
    /// the bundled seed library.
    #[test]
    fn bundled_seed_recipes_convert_to_evaluatable_engine_recipes() {
        let path = std::path::Path::new("../../config/recipes_seed.yaml");
        if !path.exists() {
            return;
        }
        let seeds = apex_worker::recipe_loader::load_seed_recipes(path).expect("seed YAML parses");
        assert!(!seeds.is_empty(), "seed library must not be empty");
        for seed in &seeds {
            let recipe = seed_recipe_to_engine_recipe(seed);
            assert!(
                !recipe.signals.is_empty(),
                "recipe {} produced no engine signals",
                seed.id
            );
            if !recipe.transforms.is_empty() {
                assert_eq!(
                    recipe.transforms.len(),
                    recipe.signals.len(),
                    "recipe {} carries a partial transform mapping",
                    seed.id
                );
            }
        }
    }
}
