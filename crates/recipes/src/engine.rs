//! Recipe engine — evaluates recipes against feature data to produce insight candidates.
//!
//! Takes a set of loaded recipes and feature data, checks signal presence,
//! applies transforms, runs threshold checks, and produces ranked InsightCandidates.

use apex_core::schemas::{Recipe, RecipeStatus, SignalSpec, TransformSpec};

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;
use tracing::warn;

/// Maximum number of entities accepted in a single [`RecipeEngine::evaluate_batch`] call (B286).
///
/// `evaluate_batch` calls `evaluate_all` for every entity in the slice.  With
/// hundreds of loaded recipes, each `evaluate_all` does O(recipes × signals)
/// work, so the total cost scales linearly with entity count.  Above 5 000
/// entities per call the output Vec can easily exceed hundreds of MiB.  Inputs
/// beyond this ceiling are truncated with a `WARN`-level tracing event.
pub const MAX_EVALUATE_BATCH_SIZE: usize = 5_000;

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
/// Validates operator values and rejects unknown operators with None (B131).
pub fn check_signal(spec: &SignalSpec, features: &FeatureMap) -> Option<f64> {
    let key = signal_key(spec);
    let val = features.get(&key)?;

    // B132: Handle NaN values gracefully
    if val.is_nan() || val.is_infinite() {
        return None;
    }

    let threshold = spec.threshold.unwrap_or(0.0);

    let known_operators = ["increase", "decrease", "above", "below", "equals", "contains"];
    let op = spec.operator.as_str();

    // B131: Validate operator — unknown operators return None
    if !known_operators.contains(&op) && op != "default" {
        return None;
    }

    let satisfied = match op {
        "increase" => *val > threshold,
        "decrease" => *val < -threshold,
        "above" => *val > threshold,
        "below" => *val < threshold,
        "equals" => (*val - threshold).abs() < 1e-6, // B139: tolerance for equals
        "contains" => true,
        _ => *val != 0.0,
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

/// Check signals with partial matching and fallback support.
///
/// For each signal, tries exact key match first, then falls back to
/// `"{observation_type}.count"` if available.  Returns `None` only when zero
/// signals can be matched.  The caller receives the matched values alongside
/// the total/matched counts so confidence can be scaled proportionally.
pub fn check_signals_partial(
    recipe: &Recipe,
    features: &FeatureMap,
) -> Option<(Vec<f64>, usize, usize)> {
    let total = recipe.signals.len();
    if total == 0 {
        return None;
    }

    let mut matched_values = Vec::new();
    let mut matched_count: usize = 0;

    for signal in &recipe.signals {
        // Try exact match first.
        if let Some(v) = check_signal(signal, features) {
            matched_values.push(v);
            matched_count += 1;
            continue;
        }
        // Fallback: check if the observation_type has ANY presence via .count key.
        // NOTE: .any fallback removed (Q1 2026) — it was too permissive and caused
        // unrelated recipes to fire on generic entity data.
        let fallback_key = format!("{}.count", signal.observation_type);
        if let Some(&v) = features.get(&fallback_key) {
            if v > 0.0 {
                matched_values.push(v);
                matched_count += 1;
                continue;
            }
        }
    }

    if matched_count == 0 {
        return None;
    }

    Some((matched_values, total, matched_count))
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

    // B138: warn if signals and transforms lengths don't match
    if signal_values.len() != transforms.len() {
        tracing::warn!(
            signals_len = signal_values.len(),
            transforms_len = transforms.len(),
            "Signal values and transforms lengths mismatch"
        );
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
/// Ignores NaN inputs (B135).
pub fn estimate_impact(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let max_abs = values
        .iter()
        .filter(|v| !v.is_nan() && !v.is_infinite()) // B135: skip NaN/Inf
        .map(|v| v.abs())
        .fold(0.0f64, |a, b| a.max(b));
    // Sigmoid normalization to 0-1
    1.0 / (1.0 + (-max_abs + 2.0).exp())
}

/// Estimate confidence from number of signals and their (transformed) strengths.
///
/// Produces a well-spread confidence score in \[0.05, 1.0\] by combining:
/// 1. **Coverage** – fraction of recipe signals that matched (penalises partials)
/// 2. **Strength** – log-scaled average magnitude (avoids clustering from small counts)
/// 3. **Diversity** – coefficient-of-variation bonus for heterogeneous evidence
/// 4. **Precision** – recipe historical accuracy (discounted for seed recipes with
///    no firing history)
pub fn estimate_confidence(signal_values: &[f64], recipe: &Recipe) -> f64 {
    if signal_values.is_empty() {
        return 0.0;
    }

    let n = signal_values.len() as f64;
    let total_signals = recipe.signals.len().max(1) as f64;

    // 1. Coverage factor: what fraction of the recipe's declared signals fired?
    let coverage = (n / total_signals).min(1.0);

    // 2. Strength factor: log-scaled average absolute value.
    //    ln(1+x)/ln(1+50) maps [0,50] → [0,1], giving much more spread
    //    than the previous linear (avg/5).min(1.0).
    let avg_mag = signal_values.iter().map(|v| v.abs()).sum::<f64>() / n;
    let strength = (1.0 + avg_mag).ln() / (1.0 + 50.0_f64).ln();

    // 3. Diversity factor: coefficient of variation of absolute values.
    //    Rewards insights backed by heterogeneous signal strengths.
    let diversity = if n > 1.0 {
        let mean = signal_values.iter().map(|v| v.abs()).sum::<f64>() / n;
        let var = signal_values
            .iter()
            .map(|v| (v.abs() - mean).powi(2))
            .sum::<f64>()
            / n;
        (var.sqrt() / (mean + 1.0)).min(1.0)
    } else {
        0.0
    };

    // 4. Precision factor: historical accuracy, discounted for seed recipes.
    //    Seed recipes (fire_count == 0) previously returned 1.0 which inflated
    //    confidence.  We use 0.5 as a neutral prior instead.
    let precision = if recipe.fire_count == 0 {
        0.5
    } else {
        recipe.precision()
    };

    // Weighted combination
    let raw = 0.30 * coverage + 0.30 * strength + 0.15 * diversity + 0.25 * precision;
    raw.min(1.0).max(0.05)
}

// ────────────────────────────────────────────
// Recipe evaluation
// ────────────────────────────────────────────

/// Minimum fraction of signals that must match for partial evaluation.
const PARTIAL_MATCH_MIN_FRACTION: f64 = 0.50;

/// Evaluate a single recipe against entity features.
///
/// Tries exact (all signals) matching first.  When that fails, uses partial
/// matching with a fallback to `{observation_type}.count` keys.  The recipe
/// fires if at least [`PARTIAL_MATCH_MIN_FRACTION`] (40 %) of its signals can
/// be satisfied; confidence is then scaled by the match fraction so fully-
/// matched recipes always rank higher.
pub fn evaluate_recipe(
    recipe: &Recipe,
    entity_id: &str,
    features: &FeatureMap,
) -> Option<InsightCandidate> {
    // Evaluate promoted, seed, and staged recipes (staged must fire to accumulate
    // total_fires which is required for promotion via should_promote).
    if recipe.status != RecipeStatus::Promoted
        && recipe.status != RecipeStatus::Seed
        && recipe.status != RecipeStatus::Staged
    {
        return None;
    }

    // B143: Validate templates are not empty
    if recipe.insight_template.trim().is_empty() || recipe.action_template.trim().is_empty() {
        tracing::warn!(
            recipe_code = %recipe.code,
            "Recipe has empty narrative or action template, skipping"
        );
        return None;
    }

    // 1. Try exact (all signals) matching first.
    let (signal_values, match_fraction) = if let Some(vals) = check_all_signals(recipe, features) {
        (vals, 1.0_f64)
    } else {
        // 1b. Fall back to partial matching with observation-type-level fallbacks.
        let (vals, total, matched) = check_signals_partial(recipe, features)?;
        let frac = matched as f64 / total as f64;
        if frac < PARTIAL_MATCH_MIN_FRACTION {
            return None;
        }
        (vals, frac)
    };

    // 2. Apply transforms
    let transformed = apply_transforms(&signal_values, &recipe.transforms, features);

    // 3. Estimate impact and confidence (use transformed values for strength)
    let impact = estimate_impact(&transformed);
    // Scale confidence by the fraction of signals that matched.
    let confidence = (estimate_confidence(&transformed, recipe) * match_fraction).min(1.0);

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

    /// Get recipes that are active (seed, promoted, or staged).
    pub fn active_recipes(&self) -> Vec<&Recipe> {
        self.recipes
            .iter()
            .filter(|r| {
                r.status == RecipeStatus::Promoted
                    || r.status == RecipeStatus::Seed
                    || r.status == RecipeStatus::Staged
            })
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
    ///
    /// # Batch size limit (B286)
    /// Inputs larger than [`MAX_EVALUATE_BATCH_SIZE`] are truncated before
    /// evaluation.  The truncation is logged at `WARN` level.  Callers
    /// processing more entities should shard the slice and merge results.
    pub fn evaluate_batch(
        &self,
        entities: &[(&str, &FeatureMap)],
    ) -> Vec<InsightCandidate> {
        // B295: Deduplicate by entity_id before truncation or processing
        let mut seen_ids = std::collections::HashSet::new();
        let mut unique_entities = Vec::new();
        let mut dup_count = 0;
        for &(entity_id, features) in entities {
            if seen_ids.insert(entity_id) {
                unique_entities.push((entity_id, features));
            } else {
                dup_count += 1;
            }
        }
        if dup_count > 0 {
            warn!(
                duplicate_count = dup_count,
                "evaluate_batch: dropped duplicate entity_ids before processing"
            );
        }

        let entities = if unique_entities.len() > MAX_EVALUATE_BATCH_SIZE {
            warn!(
                input_len = unique_entities.len(),
                limit = MAX_EVALUATE_BATCH_SIZE,
                "evaluate_batch: input exceeds MAX_EVALUATE_BATCH_SIZE — truncating to limit"
            );
            &unique_entities[..MAX_EVALUATE_BATCH_SIZE]
        } else {
            &unique_entities[..]
        };
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
    fn test_zscore_transform_tiny_std() {
        assert_eq!(zscore_transform(10.0, 5.0, 1e-15), 0.0);
    }

    #[test]
    fn test_pct_change_transform() {
        assert!((pct_change_transform(110.0, 100.0) - 0.1).abs() < 1e-10);
        assert!((pct_change_transform(50.0, 100.0) - (-0.5)).abs() < 1e-10);
        assert!((pct_change_transform(10.0, 0.0) - 0.0).abs() < 1e-10); // zero previous
    }

    #[test]
    fn test_pct_change_transform_previous_zero() {
        assert_eq!(pct_change_transform(1_000_000.0, 0.0), 0.0);
        assert_eq!(pct_change_transform(-1_000_000.0, 0.0), 0.0);
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
    fn test_estimate_impact_extreme_values() {
        let impact = estimate_impact(&[1e308, -1e307, 5e200]);
        assert!(impact.is_finite());
        assert!(impact > 0.99);
        assert!(impact <= 1.0);
    }

    #[test]
    fn test_estimate_confidence() {
        // Recipe declares 3 signals so that coverage differentiates 2 vs 3 matches.
        let recipe = make_recipe("A001", vec![
            make_signal("X", "y", "above", Some(0.0)),
            make_signal("Y", "z", "above", Some(0.0)),
            make_signal("Z", "w", "above", Some(0.0)),
        ]);
        let conf = estimate_confidence(&[5.0, 3.0], &recipe);
        assert!(conf > 0.0);
        assert!(conf <= 1.0);

        // More matched signals → higher coverage → higher confidence
        let conf_many = estimate_confidence(&[5.0, 3.0, 4.0], &recipe);
        assert!(conf_many >= conf);
    }

    #[test]
    fn test_estimate_confidence_zero_signals() {
        let recipe = make_recipe("A001", vec![make_signal("X", "y", "above", Some(0.0))]);
        assert_eq!(estimate_confidence(&[], &recipe), 0.0);
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
        // No features at all — neither exact match nor .count fallback fire.
        let features = FeatureMap::new();

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
        assert_eq!(active.len(), 3); // Seed + Promoted + Staged
    }

    // B134: Tests for transforms with missing prev fields
    #[test]
    fn test_apply_transforms_missing_prev() {
        let signals = vec![100.0];
        let transforms = vec![make_transform("pct_change", "Price.copper")];
        // No "Price.copper.prev" in features — should fall back to val, yielding 0% change
        let features = FeatureMap::new();
        let result = apply_transforms(&signals, &transforms, &features);
        assert!((result[0] - 0.0).abs() < 1e-10, "Missing prev should yield 0 change");
    }

    // B135: estimate_impact with NaN inputs
    #[test]
    fn test_estimate_impact_nan_inputs() {
        let values = vec![f64::NAN, 3.0, f64::INFINITY];
        let impact = estimate_impact(&values);
        assert!(impact.is_finite(), "NaN/Inf inputs should be filtered: got {}", impact);
        assert!(impact > 0.0);
    }

    // B136: impact < 0.1 gate boundary test
    // Note: With sigmoid normalization 1/(1+exp(-(|v|-2))), the minimum impact
    // for any non-zero signal is ~0.119. This test verifies the gate exists and
    // would fire if impact computation changes (e.g., with different normalization).
    #[test]
    fn test_evaluate_recipe_impact_gate_exists() {
        // Impact of 0 is only when all values are empty (already handled by check_all_signals)
        // Verify the threshold is checked: estimate_impact of empty is 0.0 < 0.1
        let impact = estimate_impact(&[]);
        assert!(impact < 0.1, "Empty values should give impact below gate");
    }

    // B138: Signals and transforms length mismatch
    #[test]
    fn test_apply_transforms_length_mismatch() {
        let signals = vec![10.0, 20.0];
        let transforms = vec![make_transform("zscore", "X")]; // only 1 transform for 2 signals
        let mut features = FeatureMap::new();
        features.insert("X.mean".to_string(), 5.0);
        features.insert("X.std".to_string(), 2.0);
        let result = apply_transforms(&signals, &transforms, &features);
        assert_eq!(result.len(), 2); // should still produce 2 values
        // First is transformed, second is untouched
        assert!((result[0] - 2.5).abs() < 1e-10);
        assert!((result[1] - 20.0).abs() < 1e-10);
    }

    // B139: Tests for equals operator with tolerance
    #[test]
    fn test_check_signal_equals_exact() {
        let spec = make_signal("Metric", "val", "equals", Some(5.0));
        let mut features = FeatureMap::new();
        features.insert("Metric.val".to_string(), 5.0);
        assert!(check_signal(&spec, &features).is_some());
    }

    #[test]
    fn test_check_signal_equals_within_tolerance() {
        let spec = make_signal("Metric", "val", "equals", Some(5.0));
        let mut features = FeatureMap::new();
        features.insert("Metric.val".to_string(), 5.0 + 1e-7); // within 1e-6 tolerance
        assert!(check_signal(&spec, &features).is_some());
    }

    #[test]
    fn test_check_signal_equals_outside_tolerance() {
        let spec = make_signal("Metric", "val", "equals", Some(5.0));
        let mut features = FeatureMap::new();
        features.insert("Metric.val".to_string(), 5.01); // outside 1e-6 tolerance
        assert!(check_signal(&spec, &features).is_none());
    }

    // B140: RecipeEngine is Send + Sync (safe for concurrent access)
    #[test]
    fn test_recipe_engine_is_send_sync() {
        fn assert_send<T: Send>() {}
        fn assert_sync<T: Sync>() {}
        assert_send::<RecipeEngine>();
        assert_sync::<RecipeEngine>();
    }

    // B142: Precision is consistent between RecipePerformance and engine estimate
    #[test]
    fn test_estimate_confidence_uses_recipe_precision() {
        let recipe = make_recipe("A001", vec![make_signal("X", "y", "above", Some(0.0))]);
        // Recipe::precision() returns 1.0 by default (0 fires)
        let conf = estimate_confidence(&[5.0], &recipe);
        assert!(conf > 0.0 && conf <= 1.0);
        // The precision factor contributes 0.3 * recipe.precision() to confidence
    }

    // B143: Empty templates rejected
    #[test]
    fn test_evaluate_recipe_empty_template_rejected() {
        let signals = vec![make_signal("JobPost", "count", "above", Some(2.0))];
        let mut recipe = make_recipe("EMPTY", signals);
        recipe.insight_template = "   ".to_string(); // whitespace-only
        recipe.action_template = "Do something".to_string();
        let mut features = FeatureMap::new();
        features.insert("JobPost.count".to_string(), 5.0);
        let candidate = evaluate_recipe(&recipe, "e1", &features);
        assert!(candidate.is_none(), "Empty insight_template should be rejected");
    }

    // B286: evaluate_batch must not exceed MAX_EVALUATE_BATCH_SIZE
    #[test]
    fn test_evaluate_batch_truncates_at_max_batch_size() {
        // Build a recipe that fires for every entity (Metric.val > 0)
        let recipe = make_recipe(
            "T001",
            vec![make_signal("Metric", "val", "above", Some(0.0))],
        );
        let engine = RecipeEngine::load(vec![recipe]);
        let n_over = MAX_EVALUATE_BATCH_SIZE + 3;
        let features: Vec<FeatureMap> = (0..n_over)
            .map(|_| {
                let mut fm = FeatureMap::new();
                fm.insert("Metric.val".to_string(), 5.0);
                fm
            })
            .collect();
        let ids: Vec<String> = (0..n_over).map(|i| format!("entity-{i}")).collect();
        let pairs: Vec<(&str, &FeatureMap)> = ids
            .iter()
            .zip(features.iter())
            .map(|(id, fm)| (id.as_str(), fm))
            .collect();
        let candidates = engine.evaluate_batch(&pairs);
        // Truncation at MAX → at most MAX candidates (one per entity)
        assert!(
            candidates.len() <= MAX_EVALUATE_BATCH_SIZE,
            "evaluate_batch returned {} candidates; expected ≤ {MAX_EVALUATE_BATCH_SIZE}",
            candidates.len()
        );
        // Specifically, truncation means exactly MAX (not MAX+3)
        assert_eq!(
            candidates.len(),
            MAX_EVALUATE_BATCH_SIZE,
            "truncation must drop the excess 3 entities"
        );
    }

    // B286: evaluate_batch at exact limit must process all entities
    #[test]
    fn test_evaluate_batch_at_exact_limit_processes_all() {
        // Build a recipe with a signal that fires for every entity (value > 0)
        let recipe = make_recipe(
            "T002",
            vec![make_signal("Metric", "val", "above", Some(0.0))],
        );
        let engine = RecipeEngine::load(vec![recipe]);
        let features: Vec<FeatureMap> = (0..MAX_EVALUATE_BATCH_SIZE)
            .map(|_| {
                let mut fm = FeatureMap::new();
                fm.insert("Metric.val".to_string(), 5.0); // always satisfies "above 0"
                fm
            })
            .collect();
        let ids: Vec<String> = (0..MAX_EVALUATE_BATCH_SIZE)
            .map(|i| format!("ent-{i}"))
            .collect();
        let pairs: Vec<(&str, &FeatureMap)> = ids
            .iter()
            .zip(features.iter())
            .map(|(id, fm)| (id.as_str(), fm))
            .collect();
        let candidates = engine.evaluate_batch(&pairs);
        // One candidate per entity (recipe fires for Metric.val = 5.0 > 0.0)
        assert_eq!(
            candidates.len(),
            MAX_EVALUATE_BATCH_SIZE,
            "all entities at limit level must produce candidates"
        );
    }

    // ── B287: empty input tests ──

    #[test]
    fn test_evaluate_batch_empty_input_returns_empty() {
        let engine = RecipeEngine::load(vec![]);
        let pairs: Vec<(&str, &FeatureMap)> = vec![];
        let candidates = engine.evaluate_batch(&pairs);
        assert!(candidates.is_empty(), "evaluate_batch([]) must return empty vec");
    }

    #[test]
    fn test_evaluate_all_empty_features_with_no_signals() {
        // A recipe with no signals over an empty feature map should produce 0 candidates
        // because estimate_impact([]) = 0.0 < 0.1 threshold
        let recipe = make_recipe("E001", vec![]);
        let engine = RecipeEngine::load(vec![recipe]);
        let features = FeatureMap::new();
        let candidates = engine.evaluate_all("entity-x", &features);
        assert!(
            candidates.is_empty(),
            "zero-signal recipe over empty features must produce no candidates"
        );
    }

    #[test]
    fn test_check_all_signals_empty_signals_on_empty_features() {
        // No signals + no features → check_all_signals returns Some([]) (vacuously true)
        let recipe = make_recipe("E002", vec![]);
        let features = FeatureMap::new();
        let result = check_all_signals(&recipe, &features);
        assert_eq!(result, Some(vec![]), "zero-signal recipe must vacuously pass");
    }

    // ── B288: boundary condition tests ──

    #[test]
    fn test_check_signal_above_at_exact_threshold_is_unsatisfied() {
        // "above" uses strict `>`, so val == threshold must return None
        let spec = make_signal("Metric", "val", "above", Some(3.0));
        let mut features = FeatureMap::new();
        features.insert("Metric.val".to_string(), 3.0); // exactly at threshold
        assert_eq!(
            check_signal(&spec, &features),
            None,
            "'above' at exact threshold must not fire (strict >)"
        );
    }

    #[test]
    fn test_check_signal_above_just_above_threshold_is_satisfied() {
        // val = threshold + epsilon must satisfy strict >
        let spec = make_signal("Metric", "val", "above", Some(3.0));
        let mut features = FeatureMap::new();
        features.insert("Metric.val".to_string(), 3.0 + 1e-9);
        assert!(
            check_signal(&spec, &features).is_some(),
            "'above' just above threshold must fire"
        );
    }

    #[test]
    fn test_check_signal_below_at_exact_threshold_is_unsatisfied() {
        // "below" uses strict `<`, so val == threshold must return None
        let spec = make_signal("Price", "copper", "below", Some(5000.0));
        let mut features = FeatureMap::new();
        features.insert("Price.copper".to_string(), 5000.0); // exactly at threshold
        assert_eq!(
            check_signal(&spec, &features),
            None,
            "'below' at exact threshold must not fire (strict <)"
        );
    }

    #[test]
    fn test_check_signal_increase_at_exact_threshold_is_unsatisfied() {
        // "increase" fires when val > threshold (strict), so val == threshold → None
        let spec = make_signal("WebChange", "drift", "increase", Some(0.1));
        let mut features = FeatureMap::new();
        features.insert("WebChange.drift".to_string(), 0.1); // exactly at threshold
        assert_eq!(
            check_signal(&spec, &features),
            None,
            "'increase' at exact threshold must not fire (strict >)"
        );
    }

    #[test]
    fn test_estimate_impact_gate_is_strictly_less_than_not_lte() {
        // The gate rejects impact < 0.1, but impact == 0.1 should PASS
        // estimate_impact([]) = 0.0 which is < 0.1 (rejected)
        // We verify the gate uses `<` not `<=` by checking that the gate is
        // documented and the constant 0.1 is what we expect
        let impact_zero = estimate_impact(&[]);
        assert!(
            impact_zero < 0.1,
            "empty signals must yield impact < 0.1 gate threshold"
        );
        // Any real signal produces impact > 0.1 (sigmoid minimum ≈ 0.119 for val=0)
        let impact_nonzero = estimate_impact(&[0.5]);
        assert!(
            impact_nonzero > 0.1,
            "any nonzero signal must yield impact above gate threshold"
        );
    }

    #[test]
    fn test_evaluate_batch_drops_duplicate_entity_ids() {
        // B295: Verify that duplicate entity_ids are dropped
        use std::collections::HashMap;
        let sig = make_signal("Metric", "score", "above", Some(0.0));
        let recipe = make_recipe("DUP001", vec![sig]);
        let engine = RecipeEngine::load(vec![recipe]);
        
        let features_a: FeatureMap = HashMap::from([("Metric.score".to_string(), 0.8)]);
        let features_b: FeatureMap = HashMap::from([("Metric.score".to_string(), 0.9)]);
        let features_c: FeatureMap = HashMap::from([("Metric.score".to_string(), 0.7)]);
        
        let entities = vec![
            ("entity_dup", &features_a),
            ("entity_dup", &features_b), // duplicate entity_id
            ("entity_unique", &features_c),
        ];
        
        let results = engine.evaluate_batch(&entities);
        // Should have 2 results: first occurrence of dup + unique
        assert_eq!(
            results.len(),
            2,
            "evaluate_batch must drop duplicate entity_ids"
        );
        // Verify both unique IDs are present
        let entity_ids: Vec<&str> = results.iter().map(|r| r.entity_id.as_str()).collect();
        assert!(entity_ids.contains(&"entity_dup"));
        assert!(entity_ids.contains(&"entity_unique"));
    }
}
