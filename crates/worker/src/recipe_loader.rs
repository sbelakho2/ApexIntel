//! Recipe Seed Loader
//!
//! Loads seed recipes from `config/recipes_seed.yaml` and inserts them into the database
//! at worker startup if they don't already exist.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::collections::HashSet;
use std::path::Path;
use tracing::{info, warn};

/// A recipe definition from the seed YAML file
/// 
/// Uses flexible types to handle various YAML formats
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeedRecipe {
    pub id: String,
    pub name: String,
    pub category: String,
    
    /// Join pattern - can be a single string or list of strings
    #[serde(deserialize_with = "string_or_vec", default)]
    pub join: Vec<String>,
    
    pub outcome: String,
    
    /// Signals - flexible YAML structure
    #[serde(default)]
    pub signals: Vec<serde_yaml::Value>,
    
    /// Transforms - flexible YAML structure
    #[serde(default)]
    pub transforms: Vec<serde_yaml::Value>,
    
    /// Test configuration - flexible YAML value
    #[serde(default)]
    pub test: serde_yaml::Value,
    
    /// Thresholds - flexible YAML value
    #[serde(default)]
    pub thresholds: serde_yaml::Value,
    
    #[serde(default)]
    pub narrative_template: String,
    
    #[serde(default)]
    pub action_playbook: Vec<String>,
    
    /// Applicability - can be a list of strings or a map with geos/industries
    #[serde(default)]
    pub applicability: serde_yaml::Value,
}

/// Deserialize either a single string or a list of strings into Vec<String>
fn string_or_vec<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::{self, Visitor};
    
    struct StringOrVec;
    
    impl<'de> Visitor<'de> for StringOrVec {
        type Value = Vec<String>;
        
        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("string or list of strings")
        }
        
        fn visit_str<E>(self, value: &str) -> Result<Vec<String>, E>
        where
            E: de::Error,
        {
            Ok(vec![value.to_string()])
        }
        
        fn visit_seq<A>(self, mut seq: A) -> Result<Vec<String>, A::Error>
        where
            A: de::SeqAccess<'de>,
        {
            let mut vec = Vec::new();
            while let Some(s) = seq.next_element()? {
                vec.push(s);
            }
            Ok(vec)
        }
    }
    
    deserializer.deserialize_any(StringOrVec)
}

/// Get precision from thresholds YAML value  
fn get_precision_from_thresholds(thresholds: &serde_yaml::Value) -> f64 {
    // Try common field names
    for key in ["min_precision", "precision", "min_stability", "min_effect"] {
        if let Some(val) = thresholds.get(key) {
            if let Some(f) = val.as_f64() {
                // Normalize values > 1 to be < 1 (min_effect might be 1.5)
                return if f > 1.0 { 0.8 } else { f };
            }
        }
    }
    0.8 // default
}

/// Recipe seed file structure
/// 
/// Uses flatten to allow extra fields in the YAML that we don't need
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecipeSeedFile {
    #[serde(default)]
    pub version: String,
    
    #[serde(default)]
    pub generated_at: Option<String>,
    
    /// Recipe execution config (ignored but must be present to parse)
    #[serde(default)]
    pub recipe_execution: serde_yaml::Value,
    
    pub recipes: Vec<SeedRecipe>,
}

/// Load recipes from the seed YAML file
pub fn load_seed_recipes<P: AsRef<Path>>(path: P) -> Result<Vec<SeedRecipe>> {
    let path = path.as_ref();
    
    if !path.exists() {
        warn!(path = %path.display(), "Recipe seed file not found");
        return Ok(Vec::new());
    }
    
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read recipe seed file: {}", path.display()))?;
    
    let seed_file: RecipeSeedFile = serde_yaml::from_str(&content)
        .map_err(|e| {
            // Log the detailed error
            tracing::error!("YAML parse error details: {}", e);
            anyhow::anyhow!("Failed to parse recipe seed YAML: {} - {}", path.display(), e)
        })?;
    
    info!(
        count = seed_file.recipes.len(),
        version = %seed_file.version,
        "Loaded seed recipes from YAML"
    );
    
    Ok(seed_file.recipes)
}

/// Load recipes from the default seed path
pub fn load_default_seed_recipes() -> Result<Vec<SeedRecipe>> {
    let default_path = std::env::var("RECIPES_SEED_PATH")
        .unwrap_or_else(|_| "config/recipes_seed.yaml".to_string());
    
    load_seed_recipes(&default_path)
}

/// Recipe insertion context for database operations
pub struct RecipeInsertionResult {
    pub inserted: usize,
    pub skipped: usize,
    pub errors: Vec<String>,
}

impl RecipeInsertionResult {
    pub fn new() -> Self {
        Self {
            inserted: 0,
            skipped: 0,
            errors: Vec::new(),
        }
    }
    
    pub fn total(&self) -> usize {
        self.inserted + self.skipped + self.errors.len()
    }
}

impl Default for RecipeInsertionResult {
    fn default() -> Self {
        Self::new()
    }
}

/// Generate SQL INSERT statements for seed recipes
/// 
/// This returns SQL that can be executed against the database.
/// Existing recipes (by id) are skipped using ON CONFLICT DO NOTHING.
pub fn generate_recipe_insert_sql(recipes: &[SeedRecipe]) -> String {
    let mut sql = String::new();
    sql.push_str("-- Auto-generated seed recipe inserts\n");
    sql.push_str("-- Generated by apex-worker recipe_loader\n\n");
    
    for recipe in recipes {
        // Escape strings for SQL
        let id = escape_sql(&recipe.id);
        let name = escape_sql(&recipe.name);
        let category = escape_sql(&recipe.category);
        let join_json = escape_sql(&serde_json::to_string(&recipe.join).unwrap_or_else(|_| "[]".to_string()));
        let outcome = escape_sql(&recipe.outcome);
        let signals_json = escape_sql(&serde_json::to_string(&recipe.signals).unwrap_or_else(|_| "[]".to_string()));
        let transforms_json = escape_sql(&serde_json::to_string(&recipe.transforms).unwrap_or_else(|_| "[]".to_string()));
        let test_json = escape_sql(&serde_json::to_string(&recipe.test).unwrap_or_else(|_| "{}".to_string()));
        let thresholds_json = escape_sql(&serde_json::to_string(&recipe.thresholds).unwrap_or_else(|_| "{}".to_string()));
        let narrative = escape_sql(&recipe.narrative_template);
        let playbook_json = escape_sql(&serde_json::to_string(&recipe.action_playbook).unwrap_or_else(|_| "[]".to_string()));
        let applicability_json = escape_sql(&serde_json::to_string(&recipe.applicability).unwrap_or_else(|_| "[]".to_string()));
        
        sql.push_str(&format!(
            r#"INSERT INTO recipes (
    id, name, category, join_pattern, outcome,
    signals, transforms, test_config, thresholds,
    narrative_template, action_playbook, applicability,
    lifecycle_state, created_at
) VALUES (
    '{id}', '{name}', '{category}', '{join_json}', '{outcome}',
    '{signals_json}', '{transforms_json}', '{test_json}', '{thresholds_json}',
    '{narrative}', '{playbook_json}', '{applicability_json}',
    'production', NOW()
) ON CONFLICT (id) DO NOTHING;

"#
        ));
    }
    
    sql
}

/// Validate a set of seed recipes for consistency
pub fn validate_seed_recipes(recipes: &[SeedRecipe]) -> Vec<String> {
    let mut errors = Vec::new();
    let mut seen_ids: HashSet<&str> = HashSet::new();
    
    for recipe in recipes {
        // Check for duplicate IDs
        if seen_ids.contains(recipe.id.as_str()) {
            errors.push(format!("Duplicate recipe ID: {}", recipe.id));
        }
        seen_ids.insert(&recipe.id);
        
        // Validate required fields
        if recipe.id.is_empty() {
            errors.push("Recipe has empty ID".to_string());
        }
        if recipe.name.is_empty() {
            errors.push(format!("Recipe {} has empty name", recipe.id));
        }
        if recipe.category.is_empty() {
            errors.push(format!("Recipe {} has empty category", recipe.id));
        }
        if recipe.join.is_empty() {
            errors.push(format!("Recipe {} has no join patterns", recipe.id));
        }
        if recipe.outcome.is_empty() {
            errors.push(format!("Recipe {} has empty outcome", recipe.id));
        }
        
        // Validate precision threshold if present
        let precision = get_precision_from_thresholds(&recipe.thresholds);
        if precision < 0.0 || precision > 1.0 {
            errors.push(format!(
                "Recipe {} has invalid precision: {}",
                recipe.id, precision
            ));
        }
    }
    
    if !errors.is_empty() {
        warn!(count = errors.len(), "Recipe validation found issues");
    }
    
    errors
}

/// Escape a string for safe SQL insertion
fn escape_sql(s: &str) -> String {
    s.replace('\'', "''")
}

/// Print recipe statistics by category
pub fn print_recipe_stats(recipes: &[SeedRecipe]) {
    use std::collections::HashMap;
    
    let mut by_category: HashMap<&str, usize> = HashMap::new();
    for recipe in recipes {
        *by_category.entry(&recipe.category).or_insert(0) += 1;
    }
    
    info!("Recipe statistics:");
    info!("  Total: {}", recipes.len());
    
    let mut categories: Vec<_> = by_category.iter().collect();
    categories.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    
    for (category, count) in categories {
        info!("  {}: {}", category, count);
    }
}

/// Insert seed recipes into the database
/// 
/// Uses ON CONFLICT DO NOTHING to skip recipes that already exist.
/// Returns the number of recipes inserted and skipped.
pub async fn insert_seed_recipes(
    pool: &PgPool,
    recipes: &[SeedRecipe],
) -> Result<RecipeInsertionResult> {
    let mut result = RecipeInsertionResult::new();
    
    for recipe in recipes {
        // Serialize the full recipe as the definition JSONB
        let definition = serde_json::to_value(recipe)
            .unwrap_or_else(|_| serde_json::json!({}));
        
        // Extract precision score from thresholds
        let precision_score = get_precision_from_thresholds(&recipe.thresholds);
        
        let insert_result = sqlx::query(
            r#"
            INSERT INTO recipes (code, name, status, definition, precision_score, created_at, updated_at)
            VALUES ($1, $2, 'seed', $3, $4, NOW(), NOW())
            ON CONFLICT (code) DO NOTHING
            "#
        )
        .bind(&recipe.id) // maps to code in db
        .bind(&recipe.name)
        .bind(&definition)
        .bind(precision_score)
        .execute(pool)
        .await;
        
        match insert_result {
            Ok(r) => {
                if r.rows_affected() > 0 {
                    result.inserted += 1;
                } else {
                    result.skipped += 1;
                }
            }
            Err(e) => {
                result.errors.push(format!("Recipe {}: {}", recipe.id, e));
            }
        }
    }
    
    info!(
        inserted = result.inserted,
        skipped = result.skipped, 
        errors = result.errors.len(),
        "Seed recipe insertion complete"
    );
    
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_sql() {
        assert_eq!(escape_sql("hello"), "hello");
        assert_eq!(escape_sql("it's"), "it''s");
        assert_eq!(escape_sql("'quoted'"), "''quoted''");
    }

    #[test]
    fn test_validate_seed_recipes_empty() {
        let errors = validate_seed_recipes(&[]);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_validate_seed_recipes_valid() {
        let recipe = SeedRecipe {
            id: "A001".to_string(),
            name: "Test Recipe".to_string(),
            category: "supply_chain".to_string(),
            join: vec!["entity".to_string()],
            outcome: "risk".to_string(),
            signals: vec![],
            transforms: vec![],
            test: serde_yaml::Value::Null,
            thresholds: serde_yaml::from_str("min_precision: 0.8").unwrap(),
            narrative_template: "".to_string(),
            action_playbook: vec![],
            applicability: serde_yaml::Value::Null,
        };
        
        let errors = validate_seed_recipes(&[recipe]);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_validate_seed_recipes_duplicate_ids() {
        let recipe1 = SeedRecipe {
            id: "A001".to_string(),
            name: "Recipe 1".to_string(),
            category: "supply_chain".to_string(),
            join: vec!["entity".to_string()],
            outcome: "risk".to_string(),
            signals: vec![],
            transforms: vec![],
            test: serde_yaml::Value::Null,
            thresholds: serde_yaml::Value::Null,
            narrative_template: "".to_string(),
            action_playbook: vec![],
            applicability: serde_yaml::Value::Null,
        };
        
        let recipe2 = SeedRecipe {
            id: "A001".to_string(),
            name: "Recipe 2".to_string(),
            ..recipe1.clone()
        };
        
        let errors = validate_seed_recipes(&[recipe1, recipe2]);
        assert!(errors.iter().any(|e| e.contains("Duplicate")));
    }
    
    #[test]
    fn test_string_or_vec_deserialize() {
        // Test single string
        let yaml = r#"
id: A001
name: Test
category: test
join: Entity
outcome: risk
"#;
        let recipe: SeedRecipe = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(recipe.join, vec!["Entity"]);
        
        // Test vec of strings
        let yaml2 = r#"
id: A002
name: Test2
category: test
join:
  - Entity
  - Person
outcome: risk
"#;
        let recipe2: SeedRecipe = serde_yaml::from_str(yaml2).unwrap();
        assert_eq!(recipe2.join, vec!["Entity", "Person"]);
    }
}
