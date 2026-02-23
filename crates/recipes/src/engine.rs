//! Recipe engine — evaluates recipes against feature data to produce insight candidates.
//!
//! Takes a set of loaded recipes and feature data, checks signal presence,
//! applies transforms, runs threshold checks, and produces ranked InsightCandidates.

use apex_core::schemas::{Recipe, RecipeStatus, SignalSpec, TransformSpec};

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// ────────────────────────────────────────────
// Insight Candidate (engine output)
// ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InsightCandidate {
    pub recipe_id: Uuid,
    pub recipe_code: String,
    pub entity_id: String,
    pub confidence: f64,
    pub impact: f64,
    pub narrative_template: String,
    pub action_template: String,
    pub evidence_ids: Vec<String>,
    pub severity: String,
    pub category: String,
}

// ────────────────────────────────────────────
// Feature map — represents available signal data for an entity
// ────────────────────────────────────────────

/// Simple feature map: signal_key -> value.
/// Keys follow the pattern "observation_type.field" (e.g., "JobPost.count", "WebChange.drift").
pub type FeatureMap = HashMap<String, f64>;

// ────────────────────────────────────────────
// Signal checking
// ────────────────────────────────────────────

/// Build a feature key from a signal spec.
pub fn signal_key(spec: &SignalSpec) -> String {
    format!("{}.{}", spec.observation_type, spec.field)
}

/// Check if a single signal condition is satisfied.
pub fn check_signal(spec: &SignalSpec, features: &FeatureMap) -> Option<f64> {
    let key = signal_key(spec);
    let val = features.get(&key)?;

    let threshold = spec.threshold.unwrap_or(0.0);

    let satisfied = match spec.operator.as_str() {
        "increase" => *val > threshold,
        "decrease" => *val < -threshold,
        "above" => *val > threshold,
        "below" => *val < threshold,
        "equals" => (*val - threshold).abs() < 1e-10,
        "contains" => true, // for string matching, presence is enough at this level
        _ => *val != 0.0,   // default: non-zero means signal present
    };

    if satisfied {
        Some(*val)
    } else {
        None
    }
}

/// Check all signals for a recipe. Returns signal values if all are satisfied.
pub fn check_all_signals(recipe: &Recipe, features: &FeatureMap) -> Option<Vec<f64>> {
    let mut values = Vec::new();
    for signal in &recipe.signals {
        match check_signal(signal, features) {
            Some(v) => values.push(v),
            None => return None,
        }
    }
    Some(values)
}

// ────────────────────────────────────────────
// Transform application
// ────────────────────────────────────────────

/// Apply a z-score transform: (value - mean) / std.
pub fn zscore_transform(value: f64, mean: f64, std: f64) -> f64 {
    if std < 1e-12 {
        return 0.0;
    }
    (value - mean) / std
}

/// Apply a percentage change transform.
pub fn pct_change_transform(current: f64, previous: f64) -> f64 {
    if previous.abs() < 1e-12 {
        return 0.0;
    }
    (current - previous) / previous
}

/// Apply transforms to signal values using feature context.
/// Returns transformed values.
pub fn apply_transforms(
    signal_values: &[f64],
    transforms: &[TransformSpec],
    features: &FeatureMap,
) -> Vec<f64> {
    if transforms.is_empty() {
        return signal_values.to_vec();
    }

    let mut result = signal_values.to_vec();

    for (i, transform) in transforms.iter().enumerate() {
        if i >= result.len() {
            break;
        }
        let val = result[i];

        match transform.transform_type.as_str() {
            "zscore" => {
                let mean_key = format!("{}.mean", transform.field);
                let std_key = format!("{}.std", transform.field);
                let mean = features.get(&mean_key).copied().unwrap_or(0.0);
                let std = features.get(&std_key).copied().unwrap_or(1.0);
                result[i] = zscore_transform(val, mean, std);
            }
            "pct_change" => {
                let prev_key = format!("{}.prev", transform.field);
                let prev = features.get(&prev_key).copied().unwrap_or(val);
                result[i] = pct_change_transform(val, prev);
            }
            "count" => {
                // count transform keeps the value as-is (already a count)
            }
            "diff" => {
                let prev_key = format!("{}.prev", transform.field);
                let prev = features.get(&prev_key).copied().unwrap_or(0.0);
                result[i] = val - prev;
            }
            "rolling_mean" => {
                // rolling mean transform — value is already the rolling mean
            }
            _ => {
                // unknown transform, keep value as-is
            }
        }
    }

    result
}

// ────────────────────────────────────────────
// Impact & confidence estimation
// ────────────────────────────────────────────

/// Estimate impact from transformed signal values.
/// Uses the max absolute value normalized to 0-1 range with sigmoid.
pub fn estimate_impact(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let max_abs = values
        .iter()
        .map(|v| v.abs())
        .fold(0.0f64, |a, b| a.max(b));
    // Sigmoid normalization to 0-1
    1.0 / (1.0 + (-max_abs + 2.0).exp())
}

/// Estimate confidence from number of signals and their strengths.
pub fn estimate_confidence(signal_values: &[f64], recipe: &Recipe) -> f64 {
    if signal_values.is_empty() {
        return 0.0;
    }
    // Base confidence from signal count
    let signal_factor = (signal_values.len() as f64 / 3.0).min(1.0);

    // Strength factor: average absolute value normalized
    let avg_strength = signal_values.iter().map(|v| v.abs()).sum::<f64>() / signal_values.len() as f64;
    let strength_factor = (avg_strength / 5.0).min(1.0);

    // Recipe reliability factor based on precision history
    let precision_factor = recipe.precision();

    // Combined confidence
    let raw = 0.4 * signal_factor + 0.3 * strength_factor + 0.3 * precision_factor;
    raw.min(1.0).max(0.0)
}

// ────────────────────────────────────────────
// Recipe evaluation
// ────────────────────────────────────────────

/// Evaluate a single recipe against entity features.
pub fn evaluate_recipe(
    recipe: &Recipe,
    entity_id: &str,
    features: &FeatureMap,
) -> Option<InsightCandidate> {
    // Only evaluate promoted or seed recipes
    if recipe.status != RecipeStatus::Promoted && recipe.status != RecipeStatus::Seed {
        return None;
    }

    // 1. Check signal presence
    let signal_values = check_all_signals(recipe, features)?;

    // 2. Apply transforms
    let transformed = apply_transforms(&signal_values, &recipe.transforms, features);

    // 3. Estimate impact and confidence
    let impact = estimate_impact(&transformed);
    let confidence = estimate_confidence(&signal_values, recipe);

    // 4. Check minimum thresholds
    if impact < 0.1 {
        return None;
    }

    // 5. Build evidence IDs from signal keys
    let evidence_ids: Vec<String> = recipe
        .signals
        .iter()
        .map(|s| signal_key(s))
        .collect();

    Some(InsightCandidate {
        recipe_id: recipe.id,
        recipe_code: recipe.code.clone(),
        entity_id: entity_id.to_string(),
        confidence,
        impact,
        narrative_template: recipe.insight_template.clone(),
        action_template: recipe.action_template.clone(),
        evidence_ids,
        severity: recipe.severity.clone(),
        category: recipe.category.clone(),
    })
}

// ────────────────────────────────────────────
// Recipe Engine
// ────────────────────────────────────────────

/// The recipe engine holds loaded recipes and evaluates them against entity features.
pub struct RecipeEngine {
    recipes: Vec<Recipe>,
}

impl RecipeEngine {
    /// Load recipes into the engine.
    pub fn load(recipes: Vec<Recipe>) -> Self {
        Self { recipes }
    }

    /// Get all loaded recipes.
    pub fn recipes(&self) -> &[Recipe] {
        &self.recipes
    }

    /// Count recipes by status.
    pub fn count_by_status(&self) -> HashMap<String, usize> {
        let mut counts = HashMap::new();
        for r in &self.recipes {
            *counts.entry(r.status.as_str().to_string()).or_default() += 1;
        }
        counts
    }

    /// Get recipes that are active (seed or promoted).
    pub fn active_recipes(&self) -> Vec<&Recipe> {
        self.recipes
            .iter()
            .filter(|r| r.status == RecipeStatus::Promoted || r.status == RecipeStatus::Seed)
            .collect()
    }

    /// Evaluate all active recipes for a given entity.
    pub fn evaluate_all(
        &self,
        entity_id: &str,
        features: &FeatureMap,
    ) -> Vec<InsightCandidate> {
        let mut candidates = Vec::new();

        for recipe in self.active_recipes() {
            if let Some(candidate) = evaluate_recipe(recipe, entity_id, features) {
                candidates.push(candidate);
            }
        }

        // Sort by impact × confidence (descending)
        candidates.sort_by(|a, b| {
            let score_a = a.impact * a.confidence;
            let score_b = b.impact * b.confidence;
            score_b
                .partial_cmp(&score_a)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        candidates
    }

    /// Evaluate recipes for multiple entities.
    pub fn evaluate_batch(
        &self,
        entities: &[(&str, &FeatureMap)],
    ) -> Vec<InsightCandidate> {
        let mut all_candidates = Vec::new();

        for &(entity_id, features) in entities {
            let mut candidates = self.evaluate_all(entity_id, features);
            all_candidates.append(&mut candidates);
        }

        // Re-sort all by score
        all_candidates.sort_by(|a, b| {
            let score_a = a.impact * a.confidence;
            let score_b = b.impact * b.confidence;
            score_b
                .partial_cmp(&score_a)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        all_candidates
    }

    /// Find recipe by code.
    pub fn find_by_code(&self, code: &str) -> Option<&Recipe> {
        self.recipes.iter().find(|r| r.code == code)
    }
}

// ────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_signal(obs_type: &str, field: &str, operator: &str, threshold: Option<f64>) -> SignalSpec {
        SignalSpec {
            observation_type: obs_type.to_string(),
            field: field.to_string(),
            operator: operator.to_string(),
            threshold,
            window_days: Some(30),
            value: None,
        }
    }

    fn make_transform(ttype: &str, field: &str) -> TransformSpec {
        TransformSpec {
            transform_type: ttype.to_string(),
            field: field.to_string(),
            window_days: 30,
            params: serde_json::json!({}),
        }
    }

    fn make_recipe(code: &str, signals: Vec<SignalSpec>) -> Recipe {
        let mut r = Recipe::new(code, format!("Recipe {}", code));
        r.signals = signals;
        r.insight_template = format!("Insight for recipe {}", code);
        r.action_template = format!("Action for recipe {}", code);
        r.severity = "warning".to_string();
        r.category = "demand".to_string();
        r
    }

    #[test]
    fn test_signal_key() {
        let spec = make_signal("JobPost", "count", "above", Some(3.0));
        assert_eq!(signal_key(&spec), "JobPost.count");
    }

    #[test]
    fn test_check_signal_above() {
        let spec = make_signal("JobPost", "count", "above", Some(3.0));
        let mut features = FeatureMap::new();
        features.insert("JobPost.count".to_string(), 5.0);
        assert_eq!(check_signal(&spec, &features), Some(5.0));

        features.insert("JobPost.count".to_string(), 2.0);
        assert_eq!(check_signal(&spec, &features), None);
    }

    #[test]
    fn test_check_signal_increase() {
        let spec = make_signal("WebChange", "drift", "increase", Some(0.1));
        let mut features = FeatureMap::new();
        features.insert("WebChange.drift".to_string(), 0.5);
        assert!(check_signal(&spec, &features).is_some());

        features.insert("WebChange.drift".to_string(), 0.05);
        assert!(check_signal(&spec, &features).is_none());
    }

    #[test]
    fn test_check_signal_below() {
        let spec = make_signal("CommodityPrice", "copper", "below", Some(5000.0));
        let mut features = FeatureMap::new();
        features.insert("CommodityPrice.copper".to_string(), 4500.0);
        assert!(check_signal(&spec, &features).is_some());

        features.insert("CommodityPrice.copper".to_string(), 5500.0);
        assert!(check_signal(&spec, &features).is_none());
    }

    #[test]
    fn test_check_signal_missing() {
        let spec = make_signal("JobPost", "count", "above", Some(3.0));
        let features = FeatureMap::new();
        assert_eq!(check_signal(&spec, &features), None);
    }

    #[test]
    fn test_check_all_signals_all_satisfied() {
        let signals = vec![
            make_signal("JobPost", "count", "above", Some(3.0)),
            make_signal("WebChange", "drift", "increase", Some(0.1)),
        ];
        let mut features = FeatureMap::new();
        features.insert("JobPost.count".to_string(), 5.0);
        features.insert("WebChange.drift".to_string(), 0.5);

        let recipe = make_recipe("A001", signals);
        let result = check_all_signals(&recipe, &features);
        assert!(result.is_some());
        let vals = result.unwrap();
        assert_eq!(vals.len(), 2);
        assert!((vals[0] - 5.0).abs() < 1e-10);
    }

    #[test]
    fn test_check_all_signals_one_missing() {
        let signals = vec![
            make_signal("JobPost", "count", "above", Some(3.0)),
            make_signal("WebChange", "drift", "increase", Some(0.1)),
        ];
        let mut features = FeatureMap::new();
        features.insert("JobPost.count".to_string(), 5.0);
        // WebChange.drift missing

        let recipe = make_recipe("A001", signals);
        assert!(check_all_signals(&recipe, &features).is_none());
    }

    #[test]
    fn test_zscore_transform() {
        assert!((zscore_transform(10.0, 5.0, 2.0) - 2.5).abs() < 1e-10);
        assert!((zscore_transform(5.0, 5.0, 2.0) - 0.0).abs() < 1e-10);
        assert!((zscore_transform(10.0, 5.0, 0.0) - 0.0).abs() < 1e-10); // zero std
    }

    #[test]
    fn test_pct_change_transform() {
        assert!((pct_change_transform(110.0, 100.0) - 0.1).abs() < 1e-10);
        assert!((pct_change_transform(50.0, 100.0) - (-0.5)).abs() < 1e-10);
        assert!((pct_change_transform(10.0, 0.0) - 0.0).abs() < 1e-10); // zero previous
    }

    #[test]
    fn test_apply_transforms_zscore() {
        let signals = vec![10.0];
        let transforms = vec![make_transform("zscore", "JobPost.count")];
        let mut features = FeatureMap::new();
        features.insert("JobPost.count.mean".to_string(), 5.0);
        features.insert("JobPost.count.std".to_string(), 2.0);

        let result = apply_transforms(&signals, &transforms, &features);
        assert!((result[0] - 2.5).abs() < 1e-10);
    }

    #[test]
    fn test_apply_transforms_empty() {
        let signals = vec![10.0, 5.0];
        let result = apply_transforms(&signals, &[], &FeatureMap::new());
        assert_eq!(result, signals);
    }

    #[test]
    fn test_estimate_impact() {
        assert!(estimate_impact(&[]).abs() < 1e-10);
        let imp = estimate_impact(&[5.0]);
        assert!(imp > 0.9); // sigmoid(5-2) = sigmoid(3) ≈ 0.95
        let imp_low = estimate_impact(&[0.5]);
        assert!(imp_low < 0.5); // sigmoid(0.5-2) = sigmoid(-1.5) ≈ 0.18
    }

    #[test]
    fn test_estimate_confidence() {
        let recipe = make_recipe("A001", vec![make_signal("X", "y", "above", Some(0.0))]);
        let conf = estimate_confidence(&[5.0, 3.0], &recipe);
        assert!(conf > 0.0);
        assert!(conf <= 1.0);

        // More signals should give higher confidence
        let conf_many = estimate_confidence(&[5.0, 3.0, 4.0], &recipe);
        assert!(conf_many >= conf);
    }

    #[test]
    fn test_evaluate_recipe_satisfied() {
        let signals = vec![
            make_signal("JobPost", "count", "above", Some(3.0)),
        ];
        let recipe = make_recipe("A001", signals);
        let mut features = FeatureMap::new();
        features.insert("JobPost.count".to_string(), 5.0);

        let candidate = evaluate_recipe(&recipe, "company-123", &features);
        assert!(candidate.is_some());
        let c = candidate.unwrap();
        assert_eq!(c.recipe_code, "A001");
        assert_eq!(c.entity_id, "company-123");
        assert!(c.confidence > 0.0);
        assert!(c.impact > 0.0);
    }

    #[test]
    fn test_evaluate_recipe_not_satisfied() {
        let signals = vec![
            make_signal("JobPost", "count", "above", Some(10.0)),
        ];
        let recipe = make_recipe("A001", signals);
        let mut features = FeatureMap::new();
        features.insert("JobPost.count".to_string(), 5.0);

        let candidate = evaluate_recipe(&recipe, "company-123", &features);
        assert!(candidate.is_none());
    }

    #[test]
    fn test_evaluate_recipe_retired_skipped() {
        let signals = vec![make_signal("JobPost", "count", "above", Some(1.0))];
        let mut recipe = make_recipe("A001", signals);
        recipe.status = RecipeStatus::Retired;

        let mut features = FeatureMap::new();
        features.insert("JobPost.count".to_string(), 5.0);

        assert!(evaluate_recipe(&recipe, "company-123", &features).is_none());
    }

    #[test]
    fn test_engine_evaluate_all() {
        let signals1 = vec![make_signal("JobPost", "count", "above", Some(3.0))];
        let signals2 = vec![make_signal("WebChange", "drift", "increase", Some(0.1))];

        let r1 = make_recipe("A001", signals1);
        let r2 = make_recipe("A002", signals2);

        let engine = RecipeEngine::load(vec![r1, r2]);

        let mut features = FeatureMap::new();
        features.insert("JobPost.count".to_string(), 5.0);
        features.insert("WebChange.drift".to_string(), 0.5);

        let candidates = engine.evaluate_all("entity-1", &features);
        assert_eq!(candidates.len(), 2);
        // Should be sorted by impact * confidence descending
        let score0 = candidates[0].impact * candidates[0].confidence;
        let score1 = candidates[1].impact * candidates[1].confidence;
        assert!(score0 >= score1);
    }

    #[test]
    fn test_engine_evaluate_batch() {
        let signals = vec![make_signal("JobPost", "count", "above", Some(2.0))];
        let recipe = make_recipe("A001", signals);
        let engine = RecipeEngine::load(vec![recipe]);

        let mut f1 = FeatureMap::new();
        f1.insert("JobPost.count".to_string(), 5.0);
        let mut f2 = FeatureMap::new();
        f2.insert("JobPost.count".to_string(), 10.0);

        let entities: Vec<(&str, &FeatureMap)> = vec![("e1", &f1), ("e2", &f2)];
        let candidates = engine.evaluate_batch(&entities);
        assert_eq!(candidates.len(), 2);
    }

    #[test]
    fn test_engine_count_by_status() {
        let mut r1 = make_recipe("A001", vec![]);
        r1.status = RecipeStatus::Seed;
        let mut r2 = make_recipe("A002", vec![]);
        r2.status = RecipeStatus::Promoted;
        let mut r3 = make_recipe("A003", vec![]);
        r3.status = RecipeStatus::Retired;

        let engine = RecipeEngine::load(vec![r1, r2, r3]);
        let counts = engine.count_by_status();
        assert_eq!(*counts.get("seed").unwrap(), 1);
        assert_eq!(*counts.get("promoted").unwrap(), 1);
        assert_eq!(*counts.get("retired").unwrap(), 1);
    }

    #[test]
    fn test_engine_find_by_code() {
        let r1 = make_recipe("A001", vec![]);
        let r2 = make_recipe("B001", vec![]);
        let engine = RecipeEngine::load(vec![r1, r2]);

        assert!(engine.find_by_code("A001").is_some());
        assert!(engine.find_by_code("B001").is_some());
        assert!(engine.find_by_code("C001").is_none());
    }

    #[test]
    fn test_engine_active_recipes() {
        let mut r1 = make_recipe("A001", vec![]);
        r1.status = RecipeStatus::Seed;
        let mut r2 = make_recipe("A002", vec![]);
        r2.status = RecipeStatus::Promoted;
        let mut r3 = make_recipe("A003", vec![]);
        r3.status = RecipeStatus::Retired;
        let mut r4 = make_recipe("A004", vec![]);
        r4.status = RecipeStatus::Staged;

        let engine = RecipeEngine::load(vec![r1, r2, r3, r4]);
        let active = engine.active_recipes();
        assert_eq!(active.len(), 2); // Seed + Promoted
    }
}
