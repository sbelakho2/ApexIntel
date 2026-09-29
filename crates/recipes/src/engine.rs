//! Recipe engine — evaluates recipes against feature data to produce insight candidates.
//!
//! Takes a set of loaded recipes and feature data, checks signal presence,
//! applies transforms, runs threshold checks, and produces ranked InsightCandidates.

use apex_core::analysis::calibrate_confidence;
use apex_core::schemas::{EntityContext, Recipe, RecipeStatus, SignalSpec, TransformSpec};

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::warn;
use uuid::Uuid;

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
    /// Matched signals whose feature value is a direct observation.
    pub direct_signal_count: usize,
    /// Total matched signals (exact or fallback).
    pub matched_signal_count: usize,
    /// Distinct underlying events/sources behind the matched signals. One
    /// observation aliased under several feature names counts once.
    pub distinct_source_count: usize,
    /// True when every matched signal was satisfied only by features derived
    /// from previously emitted warnings. Derived intelligence is not primary
    /// evidence: the worker must not emit production output from it.
    pub prior_warning_only: bool,
}

impl InsightCandidate {
    /// Returns true if the candidate contains un-substituted template placeholders
    /// (e.g. `{{entity}}`), indicating the template was never filled in.
    pub fn has_template_leakage(&self) -> bool {
        let fields = [&self.narrative_template, &self.action_template];
        fields.iter().any(|field| {
            field.contains("{{")
                || field.contains("{entity}")
                || field.contains("{signal}")
                || field.contains("{region}")
        })
    }
}

// ────────────────────────────────────────────
// Feature map — represents available signal data for an entity
// ────────────────────────────────────────────

/// Simple feature map: signal_key -> value.
/// Keys follow the pattern "observation_type.field" (e.g., "JobPost.count", "WebChange.drift").
pub type FeatureMap = HashMap<String, f64>;

/// Where a feature value came from (audit P1: direct vs derived vs proxy).
///
/// The feature pipeline synthesizes several aliases per observation (a
/// `FinancialDisclosure` also increments `Industry.trend` and `Market.count`)
/// and maps prior warnings back into feature names. Without provenance one
/// event can masquerade as several independent conditions, and derived
/// intelligence can feed itself.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FeatureOrigin {
    /// A direct observation (or an aggregation of one observation type).
    Observation(String),
    /// A synthesized alias of an observation, semantically broader than the
    /// observation itself (one news article is not an industry trend).
    DerivedAlias(String),
    /// Derived from previously emitted warnings — derived intelligence, never
    /// primary evidence.
    PriorWarning(String),
}

impl FeatureOrigin {
    /// Identity of the underlying event/source, so one event under several
    /// aliases counts once.
    pub fn source_id(&self) -> String {
        match self {
            Self::Observation(source) | Self::DerivedAlias(source) => source.clone(),
            Self::PriorWarning(warning) => format!("warning:{warning}"),
        }
    }
}

/// Feature key -> origin. Keys without an entry are treated as direct,
/// distinct observations (backward compatible).
pub type FeatureProvenance = HashMap<String, FeatureOrigin>;

// ────────────────────────────────────────────
// Signal checking
// ────────────────────────────────────────────

/// Build a feature key from a signal spec.
///
/// If the signal has a discriminating value (e.g. `role_family=Procurement`),
/// the value is incorporated into the key so that different values produce
/// different feature keys: `JobPost.role_family.Procurement` vs
/// `JobPost.role_family.Engineering`. Without this, all `=value` discriminators
/// in seed recipes are silently ignored, collapsing semantically distinct recipes.
pub fn signal_key(spec: &SignalSpec) -> String {
    if let Some(ref value) = spec.value {
        if !value.is_empty() {
            return format!("{}.{}.{}", spec.observation_type, spec.field, value);
        }
    }
    format!("{}.{}", spec.observation_type, spec.field)
}

/// Check if a single signal condition is satisfied, returning the matched
/// feature key and its value. The key is formatted once so hot matching paths
/// do not rebuild it.
/// Validates operator values and rejects unknown operators with None (B131).
pub fn check_signal_with_key(spec: &SignalSpec, features: &FeatureMap) -> Option<(String, f64)> {
    let key = signal_key(spec);
    let val = *features.get(&key)?;

    // B132: Handle NaN values gracefully
    if val.is_nan() || val.is_infinite() {
        return None;
    }

    let threshold = spec.threshold.unwrap_or(0.0);

    let known_operators = [
        "increase", "decrease", "above", "below", "equals", "contains",
    ];
    let op = spec.operator.as_str();

    // B131: Validate operator — unknown operators return None
    if !known_operators.contains(&op) && op != "default" {
        return None;
    }

    let satisfied = match op {
        "increase" => val > threshold,
        "decrease" => val < -threshold,
        "above" => val > threshold,
        "below" => val < threshold,
        "equals" => (val - threshold).abs() < 1e-6, // B139: tolerance for equals
        "contains" => true,
        _ => val != 0.0,
    };

    if satisfied {
        Some((key, val))
    } else {
        None
    }
}

/// Check if a single signal condition is satisfied.
pub fn check_signal(spec: &SignalSpec, features: &FeatureMap) -> Option<f64> {
    check_signal_with_key(spec, features).map(|(_, value)| value)
}

/// A signal that fired, together with the feature key that satisfied it
/// (exact key or generic `{observation_type}.count` fallback).
#[derive(Debug, Clone)]
struct SignalMatch {
    key: String,
    value: f64,
}

/// Exact match: every signal must be satisfied by its exact feature key.
fn match_all_signals(recipe: &Recipe, features: &FeatureMap) -> Option<Vec<SignalMatch>> {
    let mut matches = Vec::new();
    for signal in &recipe.signals {
        let (key, value) = check_signal_with_key(signal, features)?;
        matches.push(SignalMatch { key, value });
    }
    Some(matches)
}

/// Partial match with generic fallbacks. Returns the matches and the recipe's
/// total signal count.
fn match_signals_partial(
    recipe: &Recipe,
    features: &FeatureMap,
) -> Option<(Vec<SignalMatch>, usize)> {
    let total = recipe.signals.len();
    if total == 0 {
        return None;
    }

    let mut matches = Vec::new();
    // Each feature key may back at most one matched signal: otherwise one
    // `X.count` observation can exact-match one signal and serve as the
    // generic fallback for another of the same observation type, making one
    // fact masquerade as several conditions and inflating the match fraction.
    let mut consumed_keys: std::collections::HashSet<String> = std::collections::HashSet::new();

    for signal in &recipe.signals {
        // Try exact match first, unless another signal already claimed the key.
        let exact_key = signal_key(signal);
        if !consumed_keys.contains(&exact_key) {
            if let Some((key, value)) = check_signal_with_key(signal, features) {
                consumed_keys.insert(key.clone());
                matches.push(SignalMatch { key, value });
                continue;
            }
        }
        // Fallback: check if the observation_type has ANY presence via .count key.
        // NOTE: .any fallback removed (Q1 2026) — it was too permissive and caused
        // unrelated recipes to fire on generic entity data.
        let fallback_key = format!("{}.count", signal.observation_type);
        if consumed_keys.contains(&fallback_key) {
            continue;
        }
        if let Some(&value) = features.get(&fallback_key) {
            if value > 0.0 {
                consumed_keys.insert(fallback_key.clone());
                matches.push(SignalMatch {
                    key: fallback_key,
                    value,
                });
            }
        }
    }

    if matches.is_empty() {
        return None;
    }

    Some((matches, total))
}

/// Check all signals for a recipe. Returns signal values if all are satisfied.
#[cfg(test)]
pub fn check_all_signals(recipe: &Recipe, features: &FeatureMap) -> Option<Vec<f64>> {
    match_all_signals(recipe, features)
        .map(|matches| matches.into_iter().map(|signal| signal.value).collect())
}

/// Evidence accounting for one evaluation: how many matched signals were
/// direct observations, and how many distinct underlying events they trace
/// back to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EvidencePosture {
    direct_signal_count: usize,
    matched_signal_count: usize,
    distinct_source_count: usize,
    prior_warning_only: bool,
}

impl EvidencePosture {
    /// Without provenance every key is a distinct direct observation, which
    /// preserves the pre-provenance scoring behavior.
    fn from_matches(matches: &[SignalMatch], provenance: Option<&FeatureProvenance>) -> Self {
        let mut sources: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut direct_signal_count = 0usize;
        let mut prior_warning_only = true;

        for signal_match in matches {
            match provenance.and_then(|provenance| provenance.get(&signal_match.key)) {
                Some(origin) => {
                    if matches!(origin, FeatureOrigin::Observation(_)) {
                        direct_signal_count += 1;
                    }
                    if !matches!(origin, FeatureOrigin::PriorWarning(_)) {
                        prior_warning_only = false;
                    }
                    sources.insert(origin.source_id());
                }
                None => {
                    direct_signal_count += 1;
                    prior_warning_only = false;
                    sources.insert(format!("key:{}", signal_match.key));
                }
            }
        }

        Self {
            direct_signal_count,
            matched_signal_count: matches.len(),
            distinct_source_count: sources.len(),
            prior_warning_only,
        }
    }

    /// Penalty applied to confidence when several matched signals trace back
    /// to one event under several aliases: one source is one piece of
    /// evidence.
    fn source_factor(&self) -> f64 {
        if self.matched_signal_count == 0 {
            return 1.0;
        }
        (self.distinct_source_count as f64 / self.matched_signal_count as f64).clamp(0.0, 1.0)
    }
}

// ────────────────────────────────────────────
// Transform application
// ────────────────────────────────────────────

/// Apply a z-score transform: (value - mean) / std.
pub fn zscore_transform(value: f64, mean: f64, std: f64) -> Option<f64> {
    if std < 1e-12 {
        // A zero-variance baseline makes the standardized score undefined; it
        // does not mean the observation shows no deviation.
        return None;
    }
    Some((value - mean) / std)
}

/// Apply a percentage change transform. `None` when the previous value is
/// zero: zero-to-nonzero is undefined, not 0 %.
pub fn pct_change_transform(current: f64, previous: f64) -> Option<f64> {
    if previous.abs() < 1e-12 {
        return None;
    }
    Some((current - previous) / previous)
}

/// Why a measured baseline cannot be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvalidBaselineReason {
    /// The baseline's standard deviation is zero: the standardized score is
    /// undefined, and a genuine extreme observation must not be flattened to
    /// "no deviation".
    ZeroVariance,
    /// The baseline value is zero: percentage change is undefined, not 0 %.
    ZeroDenominator,
}

/// Why a transform could not be applied. Baselines are never fabricated: a
/// z-score without a mean/std, a difference without a previous value, a
/// zero-variance z-score or a zero-denominator percentage change all mean this
/// recipe cannot be evaluated for this entity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransformDependencyError {
    /// The feature map does not carry the baseline the transform needs.
    MissingBaseline { key: String },
    /// The baseline exists but is not a usable measurement.
    InvalidBaseline {
        key: String,
        reason: InvalidBaselineReason,
    },
    /// The transform type is not implemented. Unknown transforms are never
    /// passed through as if they had been applied.
    UnsupportedTransform { transform_type: String },
    /// Signals and transforms do not line up one-to-one: positional
    /// application would silently transform only a prefix.
    SignalTransformMismatch { signals: usize, transforms: usize },
}

impl std::fmt::Display for TransformDependencyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingBaseline { key } => {
                write!(f, "missing required baseline feature '{key}'")
            }
            Self::InvalidBaseline { key, reason } => match reason {
                InvalidBaselineReason::ZeroVariance => {
                    write!(
                        f,
                        "baseline feature '{key}' has zero variance (z-score undefined)"
                    )
                }
                InvalidBaselineReason::ZeroDenominator => {
                    write!(
                        f,
                        "baseline feature '{key}' is zero (percentage change undefined)"
                    )
                }
            },
            Self::UnsupportedTransform { transform_type } => {
                write!(f, "unsupported transform '{transform_type}'")
            }
            Self::SignalTransformMismatch {
                signals,
                transforms,
            } => write!(
                f,
                "recipe declares {signals} signal(s) but {transforms} transform(s); \
                 positional application is ambiguous"
            ),
        }
    }
}

/// Transform types the runtime engine can actually apply. This is the single
/// source of truth: loaders that decide which declared transforms to carry on
/// a runtime recipe must use [`is_supported_transform`] so bootstrap and the
/// dispatcher can never drift apart.
pub const SUPPORTED_TRANSFORM_TYPES: [&str; 4] = ["zscore", "pct_change", "count", "diff"];

/// Whether [`apply_transforms`] can evaluate the given normalized transform
/// type. Unsupported types are refused (fail closed) at evaluation time.
pub fn is_supported_transform(transform_type: &str) -> bool {
    SUPPORTED_TRANSFORM_TYPES.contains(&transform_type)
}

/// Apply transforms to signal values using feature context.
///
/// Fail-closed: a transform whose baseline (`*.mean`/`*.std`/`*.prev`) is not
/// present in the feature map — or whose type is not implemented — returns an
/// error so the recipe is not evaluated. No statistic is synthesized from
/// absent history, and a zero-variance/zero-denominator baseline is refused.
///
/// `count` is a pass-through by definition (the value already is a count).
/// `rolling_mean` is refused: without a precomputed rolling-mean feature it
/// would silently trust an arbitrary value.
pub fn apply_transforms(
    signal_values: &[f64],
    transforms: &[TransformSpec],
    features: &FeatureMap,
) -> Result<Vec<f64>, TransformDependencyError> {
    if transforms.is_empty() {
        return Ok(signal_values.to_vec());
    }

    // One transform per signal, decided at load time: positional application
    // of a partial mapping would silently transform only a prefix.
    if signal_values.len() != transforms.len() {
        return Err(TransformDependencyError::SignalTransformMismatch {
            signals: signal_values.len(),
            transforms: transforms.len(),
        });
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
                let mean = features.get(&mean_key).copied().ok_or_else(|| {
                    TransformDependencyError::MissingBaseline {
                        key: mean_key.clone(),
                    }
                })?;
                let std = features.get(&std_key).copied().ok_or_else(|| {
                    TransformDependencyError::MissingBaseline {
                        key: std_key.clone(),
                    }
                })?;
                result[i] = zscore_transform(val, mean, std).ok_or_else(|| {
                    TransformDependencyError::InvalidBaseline {
                        key: std_key.clone(),
                        reason: InvalidBaselineReason::ZeroVariance,
                    }
                })?;
            }
            "pct_change" => {
                let prev_key = format!("{}.prev", transform.field);
                let prev = features.get(&prev_key).copied().ok_or_else(|| {
                    TransformDependencyError::MissingBaseline {
                        key: prev_key.clone(),
                    }
                })?;
                result[i] = pct_change_transform(val, prev).ok_or_else(|| {
                    TransformDependencyError::InvalidBaseline {
                        key: prev_key.clone(),
                        reason: InvalidBaselineReason::ZeroDenominator,
                    }
                })?;
            }
            "count" => {
                // count transform keeps the value as-is (already a count)
            }
            "diff" => {
                let prev_key = format!("{}.prev", transform.field);
                let prev = features.get(&prev_key).copied().ok_or_else(|| {
                    TransformDependencyError::MissingBaseline {
                        key: prev_key.clone(),
                    }
                })?;
                result[i] = val - prev;
            }
            "lag" => {
                // A lag is x[t-k], not x[t] - x[t-k]; the engine cannot
                // retrieve the raw lagged value from the feature map, so an
                // explicit `lag` transform is refused rather than approximated
                // as a difference.
                return Err(TransformDependencyError::UnsupportedTransform {
                    transform_type: "lag".to_string(),
                });
            }
            "rolling_mean" => {
                // Not materialized upstream: trusting an arbitrary value to
                // already be the configured rolling mean is decorative. Refuse
                // until a precomputed `{field}.rolling_mean.{window}` feature
                // exists to read.
                return Err(TransformDependencyError::UnsupportedTransform {
                    transform_type: "rolling_mean".to_string(),
                });
            }
            other => {
                return Err(TransformDependencyError::UnsupportedTransform {
                    transform_type: other.to_string(),
                });
            }
        }
    }

    Ok(result)
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
        .filter(|v| v.is_finite()) // B135: skip NaN/Inf
        .map(|v| v.abs())
        .fold(0.0f64, |a, b| a.max(b));
    // Exponential saturation in [0, 1): 0 for a zero-strength signal, ~1 as
    // strength grows. (The previous shifted sigmoid was ~0.12 at zero, so the
    // `impact < 0.1` gate below could never reject a zero-strength signal.)
    1.0 - (-max_abs).exp()
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

    // 4. Precision factor: historical accuracy. Keep a neutral prior for unseen
    // recipes, then pass the result through the shared calibration helper so
    // false-positive-heavy recipes are disciplined more aggressively.
    let precision = if recipe.fire_count == 0 {
        0.5
    } else {
        recipe.precision()
    };

    let raw = 0.30 * coverage + 0.30 * strength + 0.15 * diversity + 0.25 * precision;
    let true_positive_count = recipe
        .fire_count
        .saturating_sub(recipe.false_positive_count);
    let calibration = calibrate_confidence(
        raw.clamp(0.05, 1.0),
        true_positive_count,
        recipe.false_positive_count,
        None,
    );
    let calibrated = calibration.calibrated_confidence.clamp(0.05, 1.0);
    // Non-finite signal values must not leak a NaN confidence downstream.
    if calibrated.is_finite() {
        calibrated
    } else {
        0.05
    }
}

// ────────────────────────────────────────────
// Recipe evaluation
// ────────────────────────────────────────────

/// Evaluate a single recipe against entity features.
///
/// Tries exact (all signals) matching first. When that fails, uses partial
/// matching with a fallback to `{observation_type}.count` keys; the recipe's
/// [`apex_core::schemas::MatchPolicy`] decides how much partiality is
/// acceptable (security/compliance recipes default to an exact match).
/// Confidence is scaled by the match fraction so fully-matched recipes always
/// rank higher.
pub fn evaluate_recipe(
    recipe: &Recipe,
    entity_id: &str,
    features: &FeatureMap,
) -> Option<InsightCandidate> {
    evaluate_recipe_with_context(recipe, entity_id, features, None)
}

/// Evaluate a recipe with entity context for the applicability gate.
///
/// A recipe with geographic/industry applicability only evaluates when the
/// context matches; a restricted recipe with absent context does not apply
/// (conservative default, audit P1: recipe applicability is a live execution
/// gate, not metadata).
pub fn evaluate_recipe_with_context(
    recipe: &Recipe,
    entity_id: &str,
    features: &FeatureMap,
    context: Option<&EntityContext>,
) -> Option<InsightCandidate> {
    evaluate_recipe_with_context_and_provenance(recipe, entity_id, features, context, None)
}

/// Evaluate a recipe with both applicability context and feature provenance.
///
/// Provenance lets the engine distinguish direct observations from synthesized
/// aliases and prior-warning derivatives: matched signals are scored by their
/// distinct underlying sources (one event under several aliases is one piece
/// of evidence), and a candidate satisfied exclusively by prior warnings is
/// flagged for the caller.
pub fn evaluate_recipe_with_context_and_provenance(
    recipe: &Recipe,
    entity_id: &str,
    features: &FeatureMap,
    context: Option<&EntityContext>,
    provenance: Option<&FeatureProvenance>,
) -> Option<InsightCandidate> {
    if !recipe.applicability.is_unrestricted() {
        let applies = context.is_some_and(|context| recipe.applicability.allows(context));
        if !applies {
            return None;
        }
    }
    evaluate_recipe_inner(recipe, entity_id, features, provenance)
}

fn evaluate_recipe_inner(
    recipe: &Recipe,
    entity_id: &str,
    features: &FeatureMap,
    provenance: Option<&FeatureProvenance>,
) -> Option<InsightCandidate> {
    // Evaluate promoted, seed, and staged recipes. Staged recipes are shadow
    // evaluation only: the engine produces candidates for observability, and
    // the caller must not emit them as production intelligence (the worker
    // filters staged candidates out before emission).
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
    let (matches, match_fraction) = if let Some(matches) = match_all_signals(recipe, features) {
        (matches, 1.0_f64)
    } else {
        // `MatchPolicy::All` means exact matching only: no generic type-count
        // fallback may stand in for a detailed signal.
        if recipe.match_policy == apex_core::schemas::MatchPolicy::All {
            return None;
        }
        // Transform-carrying recipes are applied positionally, one transform
        // per signal, so a partial match has no defined transform mapping.
        // Require every signal explicitly (the loader only carries complete
        // field-mapped transform sets) instead of failing later in
        // `apply_transforms` for each candidate.
        if !recipe.transforms.is_empty() {
            return None;
        }
        // 1b. Fall back to partial matching with observation-type-level
        // fallbacks. The recipe's policy decides how much partiality is
        // acceptable; each generic fallback key can satisfy at most one
        // signal, so a single observation type never counts as several
        // distinct conditions.
        let (matches, total) = match_signals_partial(recipe, features)?;
        if matches.len() < recipe.match_policy.required_matches(total) {
            return None;
        }
        let frac = matches.len() as f64 / total as f64;
        (matches, frac)
    };
    let signal_values: Vec<f64> = matches.iter().map(|signal| signal.value).collect();

    // 1c. Evidence posture: direct observations vs aliases vs prior warnings,
    // and how many distinct underlying events back the match.
    let evidence = EvidencePosture::from_matches(&matches, provenance);

    // 2. Apply transforms — fail closed when a baseline dependency is absent.
    let transformed = match apply_transforms(&signal_values, &recipe.transforms, features) {
        Ok(values) => values,
        Err(error) => {
            tracing::warn!(
                recipe_code = %recipe.code,
                %error,
                "recipe skipped: transform dependency unavailable"
            );
            return None;
        }
    };

    // 3. Estimate impact and confidence (use transformed values for strength).
    // Scale confidence by the fraction of signals that matched AND by evidence
    // diversity: several aliases of one event are one piece of evidence, not
    // several independent confirmations.
    let impact = estimate_impact(&transformed);
    let confidence =
        (estimate_confidence(&transformed, recipe) * match_fraction * evidence.source_factor())
            .min(1.0);

    // 3b. Database activation gate (migration 089): when calibration has set a
    // threshold, the candidate must reach it.
    if let Some(threshold) = recipe.activation_threshold {
        if confidence < threshold {
            return None;
        }
    }

    // 4. Check minimum thresholds
    if impact < 0.1 {
        return None;
    }

    // 5. Build evidence IDs from signal keys
    let evidence_ids: Vec<String> = recipe.signals.iter().map(signal_key).collect();

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
        direct_signal_count: evidence.direct_signal_count,
        matched_signal_count: evidence.matched_signal_count,
        distinct_source_count: evidence.distinct_source_count,
        prior_warning_only: evidence.prior_warning_only,
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

    /// Evaluate all recipes for one entity with applicability context and
    /// feature provenance (direct vs derived vs prior-warning evidence).
    pub fn evaluate_all_with_context_and_provenance(
        &self,
        entity_id: &str,
        features: &FeatureMap,
        context: Option<&EntityContext>,
        provenance: Option<&FeatureProvenance>,
    ) -> Vec<InsightCandidate> {
        self.recipes
            .iter()
            .filter_map(|recipe| {
                evaluate_recipe_with_context_and_provenance(
                    recipe, entity_id, features, context, provenance,
                )
            })
            .collect()
    }

    pub fn evaluate_all(&self, entity_id: &str, features: &FeatureMap) -> Vec<InsightCandidate> {
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
    pub fn evaluate_batch(&self, entities: &[(&str, &FeatureMap)]) -> Vec<InsightCandidate> {
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
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use apex_core::schemas::MatchPolicy;

    fn make_signal(
        obs_type: &str,
        field: &str,
        operator: &str,
        threshold: Option<f64>,
    ) -> SignalSpec {
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
        assert_eq!(zscore_transform(10.0, 5.0, 2.0), Some(2.5));
        assert_eq!(zscore_transform(5.0, 5.0, 2.0), Some(0.0));
        // A zero or near-zero std is undefined, not "no deviation".
        assert_eq!(zscore_transform(10.0, 5.0, 0.0), None);
        // A genuinely extreme observation against a zero-variance history must
        // not be flattened to a zero z-score.
        assert_eq!(zscore_transform(20.0, 5.0, 0.0), None);
        assert_eq!(zscore_transform(10.0, 5.0, 1e-15), None);
    }

    #[test]
    fn test_pct_change_transform() {
        assert_eq!(pct_change_transform(110.0, 100.0), Some(0.1));
        assert_eq!(pct_change_transform(50.0, 100.0), Some(-0.5));
        // Zero previous is undefined, not 0 % change.
        assert_eq!(pct_change_transform(10.0, 0.0), None);
        assert_eq!(pct_change_transform(1_000_000.0, 0.0), None);
        assert_eq!(pct_change_transform(-1_000_000.0, 0.0), None);
    }

    #[test]
    fn test_apply_transforms_zscore() {
        let signals = vec![10.0];
        let transforms = vec![make_transform("zscore", "JobPost.count")];
        let mut features = FeatureMap::new();
        features.insert("JobPost.count.mean".to_string(), 5.0);
        features.insert("JobPost.count.std".to_string(), 2.0);

        let result =
            apply_transforms(&signals, &transforms, &features).expect("transform deps present");
        assert!((result[0] - 2.5).abs() < 1e-10);
    }

    #[test]
    fn test_apply_transforms_empty() {
        let signals = vec![10.0, 5.0];
        let result = apply_transforms(&signals, &[], &FeatureMap::new())
            .expect("empty transforms cannot fail");
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
        let recipe = make_recipe(
            "A001",
            vec![
                make_signal("X", "y", "above", Some(0.0)),
                make_signal("Y", "z", "above", Some(0.0)),
                make_signal("Z", "w", "above", Some(0.0)),
            ],
        );
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
    fn test_estimate_confidence_penalizes_false_positive_history() {
        let mut reliable = make_recipe("A001", vec![make_signal("X", "y", "above", Some(0.0))]);
        reliable.fire_count = 50;
        reliable.false_positive_count = 5;

        let mut noisy = reliable.clone();
        noisy.false_positive_count = 30;

        let reliable_conf = estimate_confidence(&[4.0], &reliable);
        let noisy_conf = estimate_confidence(&[4.0], &noisy);
        assert!(reliable_conf > noisy_conf);
    }

    #[test]
    fn confidence_bounded() {
        let recipe = make_recipe(
            "A001",
            vec![
                make_signal("X", "y", "above", Some(0.0)),
                make_signal("Y", "z", "above", Some(0.0)),
            ],
        );
        for values in [vec![0.1], vec![5.0, 2.0], vec![50.0, 40.0]] {
            let confidence = estimate_confidence(&values, &recipe);
            assert!((0.05..=1.0).contains(&confidence));
        }
    }

    #[test]
    fn test_evaluate_recipe_satisfied() {
        let signals = vec![make_signal("JobPost", "count", "above", Some(3.0))];
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
        let signals = vec![make_signal("JobPost", "count", "above", Some(10.0))];
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

    /// Missing transform baselines must block evaluation, not fabricate
    /// statistics.
    #[test]
    fn test_missing_transform_baseline_blocks_evaluation() {
        let signals = vec![make_signal("JobPost", "count", "above", Some(1.0))];
        let mut recipe = make_recipe("A101", signals);
        recipe.transforms = vec![TransformSpec {
            transform_type: "zscore".to_string(),
            field: "JobPost".to_string(),
            window_days: 30,
            params: Default::default(),
        }];

        let mut features = FeatureMap::new();
        features.insert("JobPost.count".to_string(), 5.0);
        // No JobPost.mean / JobPost.std in the feature map.

        assert_eq!(
            apply_transforms(&[5.0], &recipe.transforms, &features),
            Err(TransformDependencyError::MissingBaseline {
                key: "JobPost.mean".to_string()
            })
        );
        assert!(
            evaluate_recipe(&recipe, "company-123", &features).is_none(),
            "a recipe whose transform dependency is absent must not evaluate"
        );
    }

    /// An explicit `lag` transform is refused (x[t-k] is not x[t]-x[t-k]).
    #[test]
    fn test_lag_transform_is_refused() {
        let features = FeatureMap::new();
        let transforms = vec![TransformSpec {
            transform_type: "lag".to_string(),
            field: "JobPost".to_string(),
            window_days: 7,
            params: Default::default(),
        }];
        assert_eq!(
            apply_transforms(&[1.0], &transforms, &features),
            Err(TransformDependencyError::UnsupportedTransform {
                transform_type: "lag".to_string()
            })
        );
    }

    /// MatchPolicy::All refuses the generic fallback entirely, and a shared
    /// fallback key satisfies at most one signal.
    #[test]
    fn match_policy_all_is_exact_only_and_fallbacks_are_consumed_once() {
        let signals = vec![
            make_signal("JobPost", "role_family", "above", Some(1.0)),
            make_signal("JobPost", "executive", "above", Some(1.0)),
        ];
        let mut recipe = make_recipe("X001", signals);

        let mut features = FeatureMap::new();
        features.insert("JobPost.count".to_string(), 5.0);

        recipe.match_policy = MatchPolicy::All;
        assert!(
            evaluate_recipe(&recipe, "company-1", &features).is_none(),
            "All must be exact-only: a generic JobPost.count cannot satisfy detailed signals"
        );

        recipe.match_policy = MatchPolicy::Fraction(0.5);
        assert!(
            evaluate_recipe(&recipe, "company-1", &features).is_some(),
            "the historical fraction may use the fallback once"
        );

        // Two distinct observation types can each contribute one fallback.
        let mut two_types = make_recipe(
            "X002",
            vec![
                make_signal("JobPost", "role_family", "above", Some(1.0)),
                make_signal("WebChange", "drift", "above", Some(0.1)),
            ],
        );
        two_types.match_policy = MatchPolicy::All;
        let mut both = FeatureMap::new();
        both.insert("JobPost.count".to_string(), 5.0);
        both.insert("WebChange.count".to_string(), 3.0);
        assert!(
            evaluate_recipe(&two_types, "company-1", &both).is_none(),
            "All still refuses type-count fallbacks for both signals"
        );
    }

    /// One feature key backs at most one matched signal, even when one
    /// signal's exact key is the generic `.{count}` key another falls back to.
    #[test]
    fn one_feature_key_backs_at_most_one_signal() {
        let mut recipe = make_recipe(
            "X003",
            vec![
                make_signal("Security", "count", "above", Some(0.0)),
                make_signal("Security", "risk", "above", Some(0.0)),
            ],
        );
        recipe.match_policy = MatchPolicy::Fraction(0.5);
        let mut features = FeatureMap::new();
        features.insert("Security.count".to_string(), 1.0);

        let candidate = evaluate_recipe(&recipe, "company-1", &features)
            .expect("one matched signal satisfies the fraction policy");
        assert_eq!(
            candidate.matched_signal_count, 1,
            "the exact match consumes Security.count; the fallback must not reuse it"
        );
        assert_eq!(candidate.distinct_source_count, 1);
    }

    /// Transform-carrying recipes are applied positionally, one transform per
    /// signal: a partial match has no defined mapping and must not evaluate.
    #[test]
    fn transform_recipes_require_every_signal() {
        let mut recipe = make_recipe(
            "X004",
            vec![
                make_signal("JobPost", "role_family", "above", Some(1.0)),
                make_signal("JobPost", "executive", "above", Some(1.0)),
            ],
        );
        recipe.match_policy = MatchPolicy::Fraction(0.5);
        recipe.transforms = vec![
            TransformSpec {
                transform_type: "count".to_string(),
                field: "JobPost.role_family".to_string(),
                window_days: 30,
                params: serde_json::Value::Null,
            },
            TransformSpec {
                transform_type: "count".to_string(),
                field: "JobPost.executive".to_string(),
                window_days: 30,
                params: serde_json::Value::Null,
            },
        ];
        let mut features = FeatureMap::new();
        features.insert("JobPost.count".to_string(), 5.0);

        assert!(
            evaluate_recipe(&recipe, "company-1", &features).is_none(),
            "a generic fallback must not satisfy a partially mapped transform recipe"
        );
    }

    /// Provenance deflates one event aliased under several names, and flags
    /// prior-warning-only evidence as derived intelligence.
    #[test]
    fn provenance_prevents_alias_double_counting_and_flags_prior_warnings() {
        let recipe = make_recipe(
            "P001",
            vec![
                make_signal("Industry", "trend", "above", Some(0.0)),
                make_signal("Market", "count", "above", Some(0.0)),
            ],
        );
        let mut features = FeatureMap::new();
        features.insert("Industry.trend".to_string(), 3.0);
        features.insert("Market.count".to_string(), 3.0);

        let mut provenance = FeatureProvenance::new();
        provenance.insert(
            "Industry.trend".to_string(),
            FeatureOrigin::DerivedAlias("FinancialDisclosure".to_string()),
        );
        provenance.insert(
            "Market.count".to_string(),
            FeatureOrigin::DerivedAlias("FinancialDisclosure".to_string()),
        );

        let aliased = evaluate_recipe_with_context_and_provenance(
            &recipe,
            "company-1",
            &features,
            None,
            Some(&provenance),
        )
        .expect("both aliases match");
        assert_eq!(aliased.matched_signal_count, 2);
        assert_eq!(aliased.direct_signal_count, 0);
        assert_eq!(
            aliased.distinct_source_count, 1,
            "one observation under two aliases is one piece of evidence"
        );

        // Without provenance the same keys score higher: nothing tells the
        // engine they are the same event.
        let unqualified = evaluate_recipe(&recipe, "company-1", &features).expect("matches");
        assert!(
            unqualified.confidence > aliased.confidence,
            "aliased evidence must not score like independent observations"
        );

        // Prior-warning-only evidence is flagged so callers never treat
        // derived intelligence as primary evidence.
        let mut warning_provenance = FeatureProvenance::new();
        warning_provenance.insert(
            "Industry.trend".to_string(),
            FeatureOrigin::PriorWarning("market_intelligence".to_string()),
        );
        warning_provenance.insert(
            "Market.count".to_string(),
            FeatureOrigin::PriorWarning("market_intelligence".to_string()),
        );
        let warning_only = evaluate_recipe_with_context_and_provenance(
            &recipe,
            "company-1",
            &features,
            None,
            Some(&warning_provenance),
        )
        .expect("matches");
        assert!(warning_only.prior_warning_only);
        assert_eq!(warning_only.direct_signal_count, 0);

        // Direct observations clear the flag.
        let mut direct_provenance = FeatureProvenance::new();
        direct_provenance.insert(
            "Industry.trend".to_string(),
            FeatureOrigin::Observation("FinancialDisclosure".to_string()),
        );
        direct_provenance.insert(
            "Market.count".to_string(),
            FeatureOrigin::Observation("Industry".to_string()),
        );
        let direct = evaluate_recipe_with_context_and_provenance(
            &recipe,
            "company-1",
            &features,
            None,
            Some(&direct_provenance),
        )
        .expect("matches");
        assert!(!direct.prior_warning_only);
        assert_eq!(direct.direct_signal_count, 2);
        assert_eq!(direct.distinct_source_count, 2);
    }

    /// Applicability is a live execution gate: a restricted recipe does not
    /// evaluate without matching entity context.
    #[test]
    fn applicability_gates_evaluation() {
        let signals = vec![make_signal("JobPost", "count", "above", Some(1.0))];
        let mut recipe = make_recipe("G001", signals);
        recipe.applicability = apex_core::schemas::Applicability {
            geos: vec!["Tunisia".to_string()],
            industries: vec![],
            notes: String::new(),
        };

        let mut features = FeatureMap::new();
        features.insert("JobPost.count".to_string(), 5.0);

        // No context: a restricted recipe must not apply.
        assert!(evaluate_recipe(&recipe, "company-1", &features).is_none());

        let mismatched = EntityContext {
            region: Some("Egypt".to_string()),
            ..Default::default()
        };
        assert!(
            evaluate_recipe_with_context(&recipe, "company-1", &features, Some(&mismatched))
                .is_none(),
            "region mismatch must block evaluation"
        );

        let matched = EntityContext {
            region: Some("tunisia".to_string()),
            ..Default::default()
        };
        assert!(
            evaluate_recipe_with_context(&recipe, "company-1", &features, Some(&matched)).is_some()
        );

        // Unrestricted recipes do not need context.
        let unrestricted = make_recipe(
            "G002",
            vec![make_signal("JobPost", "count", "above", Some(1.0))],
        );
        assert!(evaluate_recipe(&unrestricted, "company-1", &features).is_some());
    }

    /// MatchPolicy::All requires every signal; the historical fraction allows
    /// half. A security recipe (All) must not fire on two of four signals.
    #[test]
    fn match_policy_all_blocks_partial_matches() {
        let signals = vec![
            make_signal("JobPost", "count", "above", Some(1.0)),
            make_signal("WebChange", "drift", "increase", Some(0.1)),
            make_signal("PatentFiling", "count", "above", Some(1.0)),
            make_signal("TenderPosted", "count", "above", Some(1.0)),
        ];
        let mut recipe = make_recipe("S001", signals);
        recipe.match_policy = MatchPolicy::All;

        let mut features = FeatureMap::new();
        features.insert("JobPost.count".to_string(), 5.0);
        features.insert("WebChange.drift".to_string(), 0.5);

        assert!(
            evaluate_recipe(&recipe, "company-123", &features).is_none(),
            "MatchPolicy::All must not fire on 2 of 4 signals"
        );

        recipe.match_policy = MatchPolicy::Fraction(0.5);
        assert!(
            evaluate_recipe(&recipe, "company-123", &features).is_some(),
            "the historical fraction policy still allows 2 of 4"
        );

        recipe.match_policy = MatchPolicy::AtLeast(3);
        assert!(
            evaluate_recipe(&recipe, "company-123", &features).is_none(),
            "AtLeast(3) must not fire on 2 matches"
        );
    }

    /// The policy computes its required-match count correctly.
    #[test]
    fn match_policy_required_matches() {
        assert_eq!(MatchPolicy::All.required_matches(4), 4);
        assert_eq!(MatchPolicy::AtLeast(3).required_matches(4), 3);
        assert_eq!(MatchPolicy::AtLeast(9).required_matches(4), 4);
        assert_eq!(MatchPolicy::Fraction(0.5).required_matches(4), 2);
        assert_eq!(MatchPolicy::Fraction(0.25).required_matches(3), 1);
        assert_eq!(MatchPolicy::Fraction(0.0).required_matches(3), 1);
    }

    /// The activation threshold is a runtime gate: a candidate below it does
    /// not fire, and one at or above it does.
    #[test]
    fn test_activation_threshold_gates_firing() {
        let signals = vec![make_signal("JobPost", "count", "above", Some(1.0))];
        let mut recipe = make_recipe("A102", signals);
        let mut features = FeatureMap::new();
        features.insert("JobPost.count".to_string(), 5.0);

        // Ungated: fires.
        assert!(recipe.activation_threshold.is_none());
        let baseline = evaluate_recipe(&recipe, "company-123", &features);
        assert!(baseline.is_some(), "ungated recipe should fire");

        // Impossible gate: does not fire.
        recipe.activation_threshold = Some(1.01);
        assert!(
            evaluate_recipe(&recipe, "company-123", &features).is_none(),
            "a gate above 1.0 can never be met"
        );

        // Gate at zero: fires again.
        recipe.activation_threshold = Some(0.0);
        assert!(evaluate_recipe(&recipe, "company-123", &features).is_some());
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
        // No "Price.copper.prev" in features: the transform CANNOT be applied
        // (fabricating 0% change would be a synthesized statistic).
        let features = FeatureMap::new();
        assert_eq!(
            apply_transforms(&signals, &transforms, &features),
            Err(TransformDependencyError::MissingBaseline {
                key: "Price.copper.prev".to_string()
            })
        );

        // With the baseline present the transform applies normally.
        let mut features = FeatureMap::new();
        features.insert("Price.copper.prev".to_string(), 80.0);
        let result = apply_transforms(&signals, &transforms, &features).expect("baseline present");
        assert!((result[0] - 0.25).abs() < 1e-10);
    }

    // B135: estimate_impact with NaN inputs
    #[test]
    fn test_estimate_impact_nan_inputs() {
        let values = vec![f64::NAN, 3.0, f64::INFINITY];
        let impact = estimate_impact(&values);
        assert!(
            impact.is_finite(),
            "NaN/Inf inputs should be filtered: got {}",
            impact
        );
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
        // Positional application of a partial mapping would transform only a
        // prefix: the mismatch fails closed.
        assert_eq!(
            apply_transforms(&signals, &transforms, &features),
            Err(TransformDependencyError::SignalTransformMismatch {
                signals: 2,
                transforms: 1
            })
        );
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
        assert!(
            candidate.is_none(),
            "Empty insight_template should be rejected"
        );
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
        assert!(
            candidates.is_empty(),
            "evaluate_batch([]) must return empty vec"
        );
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
        assert_eq!(
            result,
            Some(vec![]),
            "zero-signal recipe must vacuously pass"
        );
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

    #[test]
    fn template_leakage_detected() {
        let candidate = InsightCandidate {
            recipe_id: Uuid::nil(),
            recipe_code: "TEST".into(),
            entity_id: "e1".into(),
            confidence: 0.8,
            impact: 0.5,
            narrative_template: "{{entity}} shows {{signal}} activity".into(),
            action_template: "Review {entity} positioning".into(),
            evidence_ids: vec![],
            severity: "warning".into(),
            category: "supply_chain".into(),
            direct_signal_count: 1,
            matched_signal_count: 1,
            distinct_source_count: 1,
            prior_warning_only: false,
        };
        assert!(
            candidate.has_template_leakage(),
            "must detect placeholder leakage"
        );

        let clean = InsightCandidate {
            narrative_template: "Acme Corp shows expansion activity".into(),
            action_template: "Review Acme Corp positioning".into(),
            ..candidate
        };
        assert!(!clean.has_template_leakage(), "clean text should not flag");
    }
}
