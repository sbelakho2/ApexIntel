//! Recipe lifecycle management — staging, promotion, deprecation.
//!
//! Manages the recipe lifecycle:
//! Seed → Candidate → Staged (observe 4 weeks) → Promoted (production) → Retired
//!
//! Uses in-memory registry for testability without database dependencies.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

use crate::gates::{all_gates_pass, GateConfig, GateEvidence};
use apex_core::schemas::RecipeStatus;

// ────────────────────────────────────────────
// Recipe performance tracking
// ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecipePerformance {
    pub recipe_id: Uuid,
    pub recipe_code: String,
    pub status: RecipeStatus,
    pub staged_at: Option<DateTime<Utc>>,
    pub weeks_in_staging: u32,
    pub true_positives: u32,
    pub false_positives: u32,
    pub total_fires: u32,
    pub evidence_coverage: f64,
    pub last_fired: Option<DateTime<Utc>>,
}

impl RecipePerformance {
    pub fn new(recipe_id: Uuid, recipe_code: impl Into<String>, status: RecipeStatus) -> Self {
        Self {
            recipe_id,
            recipe_code: recipe_code.into(),
            status,
            staged_at: None,
            weeks_in_staging: 0,
            true_positives: 0,
            false_positives: 0,
            total_fires: 0,
            evidence_coverage: 0.0,
            last_fired: None,
        }
    }

    /// Compute precision: true_positives / total_fires.
    pub fn precision(&self) -> f64 {
        if self.total_fires == 0 {
            return 1.0;
        }
        self.true_positives as f64 / self.total_fires as f64
    }

    /// Compute false positive rate: false_positives / total_fires.
    pub fn false_positive_rate(&self) -> f64 {
        if self.total_fires == 0 {
            return 0.0;
        }
        self.false_positives as f64 / self.total_fires as f64
    }

    /// Record a firing event.
    pub fn record_fire(&mut self, is_true_positive: bool) {
        self.total_fires += 1;
        if is_true_positive {
            self.true_positives += 1;
        } else {
            self.false_positives += 1;
        }
        self.last_fired = Some(Utc::now());
    }

    /// Set evidence coverage, clamping to [0, 1].
    pub fn set_evidence_coverage(&mut self, coverage: f64) {
        self.evidence_coverage = coverage.clamp(0.0, 1.0);
    }
}

// ────────────────────────────────────────────
// Promotion criteria
// ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromotionCriteria {
    pub min_weeks_in_staging: u32,
    pub min_precision: f64,
    pub min_evidence_coverage: f64,
    pub max_false_positive_rate: f64,
    pub min_total_fires: u32,
}

impl Default for PromotionCriteria {
    fn default() -> Self {
        Self {
            min_weeks_in_staging: 4,
            min_precision: 0.85,
            min_evidence_coverage: 0.7,
            max_false_positive_rate: 0.02,
            min_total_fires: 3,
        }
    }
}

/// Deprecation criteria for removing underperforming production recipes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeprecationCriteria {
    pub max_precision_for_deprecation: f64,
    pub max_evidence_coverage_for_deprecation: f64,
    pub max_days_since_last_fire: i64,
}

impl Default for DeprecationCriteria {
    fn default() -> Self {
        Self {
            max_precision_for_deprecation: 0.5,
            max_evidence_coverage_for_deprecation: 0.3,
            max_days_since_last_fire: 90,
        }
    }
}

// ────────────────────────────────────────────
// Lifecycle decisions
// ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LifecycleDecision {
    pub recipe_id: Uuid,
    pub recipe_code: String,
    pub action: LifecycleAction,
    pub reason: String,
    pub decided_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum LifecycleAction {
    Stage,
    Promote,
    Deprecate,
    NoChange,
}

/// Check if a candidate should be staged (passes all robustness gates).
pub fn should_stage(evidence: &GateEvidence, config: &GateConfig) -> bool {
    all_gates_pass(evidence, config)
}

/// Check if a staged recipe should be promoted to production.
pub fn should_promote(perf: &RecipePerformance, criteria: &PromotionCriteria) -> bool {
    perf.status == RecipeStatus::Staged
        && perf.weeks_in_staging >= criteria.min_weeks_in_staging
        && perf.precision() >= criteria.min_precision
        && perf.evidence_coverage >= criteria.min_evidence_coverage
        && perf.false_positive_rate() <= criteria.max_false_positive_rate
        && perf.total_fires >= criteria.min_total_fires
}

/// Check if a production recipe should be deprecated.
///
/// Accepts `now` explicitly so the staleness check is deterministic and testable.
pub fn should_deprecate(
    perf: &RecipePerformance,
    criteria: &DeprecationCriteria,
    now: DateTime<Utc>,
) -> bool {
    if perf.status != RecipeStatus::Promoted {
        return false;
    }

    // Deprecate for low precision — require at least 10 fires before precision-based deprecation
    // to avoid retiring a recipe on a single early false positive.
    if perf.total_fires >= 10 && perf.precision() < criteria.max_precision_for_deprecation {
        return true;
    }

    // Deprecate for low evidence coverage
    if perf.evidence_coverage < criteria.max_evidence_coverage_for_deprecation
        && perf.total_fires > 5
    {
        return true;
    }

    // Deprecate if not fired in too long.
    // Fall back to staged_at if the recipe never fired after promotion.
    let last_active = perf.last_fired.or(perf.staged_at);
    if let Some(last) = last_active {
        let days_since = (now - last).num_days();
        if days_since > criteria.max_days_since_last_fire {
            return true;
        }
    }

    false
}

// ────────────────────────────────────────────
// Recipe Registry (in-memory)
// ────────────────────────────────────────────

/// In-memory recipe lifecycle registry.
pub struct RecipeRegistry {
    performances: HashMap<Uuid, RecipePerformance>,
    decisions: Vec<LifecycleDecision>,
    max_decisions: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RecipeRegistrySnapshot {
    performances: HashMap<Uuid, RecipePerformance>,
    decisions: Vec<LifecycleDecision>,
    max_decisions: usize,
}

impl RecipeRegistry {
    pub fn new() -> Self {
        Self {
            performances: HashMap::new(),
            decisions: Vec::new(),
            max_decisions: 5_000,
        }
    }

    /// Record a decision, trimming oldest entries if over capacity.
    fn record_decision(&mut self, decision: LifecycleDecision) {
        if self.decisions.len() >= self.max_decisions {
            let drain_count = self.max_decisions / 5; // drop oldest 20%
            self.decisions.drain(..drain_count);
        }
        tracing::info!(
            recipe_id = %decision.recipe_id,
            recipe_code = %decision.recipe_code,
            action = ?decision.action,
            reason = %decision.reason,
            decided_at = %decision.decided_at,
            "recipe_lifecycle_decision"
        );
        self.decisions.push(decision);
    }

    /// Serialize registry state for optional persistence / crash recovery.
    pub fn to_json_snapshot(&self) -> Result<String, serde_json::Error> {
        let snapshot = RecipeRegistrySnapshot {
            performances: self.performances.clone(),
            decisions: self.decisions.clone(),
            max_decisions: self.max_decisions,
        };
        serde_json::to_string(&snapshot)
    }

    /// Restore registry state from a serialized snapshot.
    pub fn from_json_snapshot(snapshot: &str) -> Result<Self, serde_json::Error> {
        let parsed: RecipeRegistrySnapshot = serde_json::from_str(snapshot)?;
        Ok(Self {
            performances: parsed.performances,
            decisions: parsed.decisions,
            max_decisions: parsed.max_decisions,
        })
    }

    /// Register a recipe for tracking.
    pub fn register(&mut self, perf: RecipePerformance) {
        self.performances.insert(perf.recipe_id, perf);
    }

    /// Get performance for a recipe.
    pub fn get_performance(&self, recipe_id: &Uuid) -> Option<&RecipePerformance> {
        self.performances.get(recipe_id)
    }

    /// Get mutable performance for recording events.
    pub fn get_performance_mut(&mut self, recipe_id: &Uuid) -> Option<&mut RecipePerformance> {
        self.performances.get_mut(recipe_id)
    }

    /// Get all recipes with a given status.
    pub fn by_status(&self, status: &RecipeStatus) -> Vec<&RecipePerformance> {
        self.performances
            .values()
            .filter(|p| &p.status == status)
            .collect()
    }

    /// Stage a candidate recipe (after passing gates).
    pub fn stage(&mut self, recipe_id: &Uuid) -> Option<LifecycleDecision> {
        let perf = self.performances.get_mut(recipe_id)?;
        if perf.status != RecipeStatus::Candidate {
            return None;
        }
        perf.status = RecipeStatus::Staged;
        perf.staged_at = Some(Utc::now());
        perf.weeks_in_staging = 0;

        let decision = LifecycleDecision {
            recipe_id: *recipe_id,
            recipe_code: perf.recipe_code.clone(),
            action: LifecycleAction::Stage,
            reason: "Passed all robustness gates".to_string(),
            decided_at: Utc::now(),
        };
        self.record_decision(decision.clone());
        Some(decision)
    }

    /// Run the weekly promotion board.
    pub fn run_promotion_board(&mut self, criteria: &PromotionCriteria) -> Vec<LifecycleDecision> {
        let staged_ids: Vec<Uuid> = self
            .performances
            .values()
            .filter(|p| p.status == RecipeStatus::Staged)
            .map(|p| p.recipe_id)
            .collect();

        let mut decisions = Vec::new();
        for id in staged_ids {
            let perf = match self.performances.get(&id) {
                Some(p) => p.clone(),
                None => continue,
            };
            if should_promote(&perf, criteria) {
                let p = match self.performances.get_mut(&id) {
                    Some(p) => p,
                    None => continue,
                };
                p.status = RecipeStatus::Promoted;

                let decision = LifecycleDecision {
                    recipe_id: id,
                    recipe_code: p.recipe_code.clone(),
                    action: LifecycleAction::Promote,
                    reason: format!(
                        "Promoted after {} weeks (precision={:.2}, coverage={:.2})",
                        p.weeks_in_staging,
                        perf.precision(),
                        perf.evidence_coverage
                    ),
                    decided_at: Utc::now(),
                };
                self.record_decision(decision.clone());
                decisions.push(decision);
            }
        }
        decisions
    }

    /// Run deprecation sweep on production recipes.
    pub fn run_deprecation_sweep(
        &mut self,
        criteria: &DeprecationCriteria,
    ) -> Vec<LifecycleDecision> {
        let now = Utc::now();
        let promoted_ids: Vec<Uuid> = self
            .performances
            .values()
            .filter(|p| p.status == RecipeStatus::Promoted)
            .map(|p| p.recipe_id)
            .collect();

        let mut decisions = Vec::new();
        for id in promoted_ids {
            let perf = match self.performances.get(&id) {
                Some(p) => p.clone(),
                None => continue,
            };
            if should_deprecate(&perf, criteria, now) {
                let p = match self.performances.get_mut(&id) {
                    Some(p) => p,
                    None => continue,
                };
                p.status = RecipeStatus::Retired;

                let reason = if perf.total_fires >= 10
                    && perf.precision() < criteria.max_precision_for_deprecation
                {
                    format!("Precision too low: {:.2}", perf.precision())
                } else if perf.evidence_coverage < criteria.max_evidence_coverage_for_deprecation
                    && perf.total_fires > 5
                {
                    format!("Evidence coverage too low: {:.2}", perf.evidence_coverage)
                } else {
                    "No fires in too long".to_string()
                };

                let decision = LifecycleDecision {
                    recipe_id: id,
                    recipe_code: p.recipe_code.clone(),
                    action: LifecycleAction::Deprecate,
                    reason,
                    decided_at: Utc::now(),
                };
                self.record_decision(decision.clone());
                decisions.push(decision);
            }
        }
        decisions
    }

    /// Advance staging week count for all staged recipes.
    pub fn advance_staging_week(&mut self) {
        for perf in self.performances.values_mut() {
            if perf.status == RecipeStatus::Staged {
                perf.weeks_in_staging += 1;
            }
        }
    }

    /// Get all lifecycle decisions.
    pub fn decisions(&self) -> &[LifecycleDecision] {
        &self.decisions
    }

    /// Get count of recipes by status.
    pub fn status_counts(&self) -> HashMap<String, usize> {
        let mut counts = HashMap::new();
        for perf in self.performances.values() {
            *counts.entry(perf.status.as_str().to_string()).or_default() += 1;
        }
        counts
    }
}

impl Default for RecipeRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::gates::{GateConfig, GateEvidence};

    fn make_perf(code: &str, status: RecipeStatus) -> RecipePerformance {
        RecipePerformance::new(Uuid::new_v4(), code, status)
    }

    #[test]
    fn test_recipe_performance_precision() {
        let mut perf = make_perf("A001", RecipeStatus::Staged);
        assert_eq!(perf.precision(), 1.0); // no fires

        perf.record_fire(true);
        perf.record_fire(true);
        perf.record_fire(false);
        assert!((perf.precision() - 2.0 / 3.0).abs() < 0.01);
        assert_eq!(perf.total_fires, 3);
        assert_eq!(perf.true_positives, 2);
        assert_eq!(perf.false_positives, 1);
    }

    #[test]
    fn test_recipe_performance_false_positive_rate() {
        let mut perf = make_perf("A001", RecipeStatus::Staged);
        assert_eq!(perf.false_positive_rate(), 0.0);

        for _ in 0..8 {
            perf.record_fire(true);
        }
        for _ in 0..2 {
            perf.record_fire(false);
        }
        assert!((perf.false_positive_rate() - 0.2).abs() < 0.01);
    }

    #[test]
    fn test_should_stage_passes() {
        let evidence = GateEvidence {
            uplift: 2.0,
            mutual_info: 0.15,
            p_value: 0.005,
            q_value: 0.03,
            time_slices_passed: 3,
            total_time_slices: 4,
            entities_passed: 8,
            negative_control_effect: 0.3,
            false_alarm_rate: 0.01,
            counterfactual_change: 0.2,
        };
        assert!(should_stage(&evidence, &GateConfig::default()));
    }

    #[test]
    fn test_should_stage_fails() {
        let evidence = GateEvidence {
            uplift: 1.0,
            mutual_info: 0.05,
            p_value: 0.05,
            q_value: 0.1,
            time_slices_passed: 1,
            total_time_slices: 4,
            entities_passed: 2,
            negative_control_effect: 0.8,
            false_alarm_rate: 0.05,
            counterfactual_change: 0.05,
        };
        assert!(!should_stage(&evidence, &GateConfig::default()));
    }

    #[test]
    fn test_should_promote() {
        let mut perf = make_perf("A001", RecipeStatus::Staged);
        perf.weeks_in_staging = 5;
        perf.evidence_coverage = 0.8;
        for _ in 0..10 {
            perf.record_fire(true);
        }
        // precision = 1.0, FP rate = 0.0
        assert!(should_promote(&perf, &PromotionCriteria::default()));
    }

    #[test]
    fn test_should_not_promote_low_precision() {
        let mut perf = make_perf("A001", RecipeStatus::Staged);
        perf.weeks_in_staging = 5;
        perf.evidence_coverage = 0.8;
        for _ in 0..5 {
            perf.record_fire(true);
        }
        for _ in 0..5 {
            perf.record_fire(false);
        }
        // precision = 0.5, below 0.85 threshold
        assert!(!should_promote(&perf, &PromotionCriteria::default()));
    }

    #[test]
    fn test_should_not_promote_too_early() {
        let mut perf = make_perf("A001", RecipeStatus::Staged);
        perf.weeks_in_staging = 2; // below 4 week minimum
        perf.evidence_coverage = 0.9;
        for _ in 0..10 {
            perf.record_fire(true);
        }
        assert!(!should_promote(&perf, &PromotionCriteria::default()));
    }

    #[test]
    fn test_should_deprecate_low_precision() {
        let mut perf = make_perf("A001", RecipeStatus::Promoted);
        for _ in 0..3 {
            perf.record_fire(true);
        }
        for _ in 0..7 {
            perf.record_fire(false);
        }
        // precision = 0.3, below 0.5
        assert!(should_deprecate(
            &perf,
            &DeprecationCriteria::default(),
            Utc::now()
        ));
    }

    #[test]
    fn test_should_not_deprecate_healthy() {
        let mut perf = make_perf("A001", RecipeStatus::Promoted);
        perf.evidence_coverage = 0.8;
        for _ in 0..9 {
            perf.record_fire(true);
        }
        perf.record_fire(false);
        // precision = 0.9, good
        assert!(!should_deprecate(
            &perf,
            &DeprecationCriteria::default(),
            Utc::now()
        ));
    }

    #[test]
    fn test_should_not_deprecate_non_promoted() {
        let mut perf = make_perf("A001", RecipeStatus::Staged);
        for _ in 0..10 {
            perf.record_fire(false);
        }
        assert!(!should_deprecate(
            &perf,
            &DeprecationCriteria::default(),
            Utc::now()
        ));
    }

    #[test]
    fn test_registry_register_and_get() {
        let mut registry = RecipeRegistry::new();
        let perf = make_perf("A001", RecipeStatus::Candidate);
        let id = perf.recipe_id;
        registry.register(perf);

        assert!(registry.get_performance(&id).is_some());
        assert_eq!(registry.get_performance(&id).unwrap().recipe_code, "A001");
    }

    #[test]
    fn test_registry_by_status() {
        let mut registry = RecipeRegistry::new();
        registry.register(make_perf("A001", RecipeStatus::Candidate));
        registry.register(make_perf("A002", RecipeStatus::Candidate));
        registry.register(make_perf("A003", RecipeStatus::Staged));

        assert_eq!(registry.by_status(&RecipeStatus::Candidate).len(), 2);
        assert_eq!(registry.by_status(&RecipeStatus::Staged).len(), 1);
        assert_eq!(registry.by_status(&RecipeStatus::Promoted).len(), 0);
    }

    #[test]
    fn test_registry_stage() {
        let mut registry = RecipeRegistry::new();
        let perf = make_perf("A001", RecipeStatus::Candidate);
        let id = perf.recipe_id;
        registry.register(perf);

        let decision = registry.stage(&id);
        assert!(decision.is_some());
        let d = decision.unwrap();
        assert_eq!(d.action, LifecycleAction::Stage);

        let updated = registry.get_performance(&id).unwrap();
        assert_eq!(updated.status, RecipeStatus::Staged);
        assert!(updated.staged_at.is_some());
    }

    #[test]
    fn test_registry_stage_wrong_status() {
        let mut registry = RecipeRegistry::new();
        let perf = make_perf("A001", RecipeStatus::Promoted);
        let id = perf.recipe_id;
        registry.register(perf);

        assert!(registry.stage(&id).is_none()); // can only stage candidates
    }

    #[test]
    fn test_registry_promotion_board() {
        let mut registry = RecipeRegistry::new();

        // Recipe ready for promotion
        let mut perf1 = make_perf("A001", RecipeStatus::Staged);
        perf1.weeks_in_staging = 5;
        perf1.evidence_coverage = 0.8;
        for _ in 0..10 {
            perf1.record_fire(true);
        }
        let id1 = perf1.recipe_id;
        registry.register(perf1);

        // Recipe not ready (too early)
        let mut perf2 = make_perf("A002", RecipeStatus::Staged);
        perf2.weeks_in_staging = 1;
        let id2 = perf2.recipe_id;
        registry.register(perf2);

        let decisions = registry.run_promotion_board(&PromotionCriteria::default());
        assert_eq!(decisions.len(), 1);
        assert_eq!(decisions[0].action, LifecycleAction::Promote);

        assert_eq!(
            registry.get_performance(&id1).unwrap().status,
            RecipeStatus::Promoted
        );
        assert_eq!(
            registry.get_performance(&id2).unwrap().status,
            RecipeStatus::Staged
        );
    }

    #[test]
    fn test_registry_deprecation_sweep() {
        let mut registry = RecipeRegistry::new();

        let mut perf = make_perf("A001", RecipeStatus::Promoted);
        for _ in 0..3 {
            perf.record_fire(true);
        }
        for _ in 0..7 {
            perf.record_fire(false);
        }
        let id = perf.recipe_id;
        registry.register(perf);

        let decisions = registry.run_deprecation_sweep(&DeprecationCriteria::default());
        assert_eq!(decisions.len(), 1);
        assert_eq!(decisions[0].action, LifecycleAction::Deprecate);
        assert_eq!(
            registry.get_performance(&id).unwrap().status,
            RecipeStatus::Retired
        );
    }

    #[test]
    fn test_registry_advance_staging_week() {
        let mut registry = RecipeRegistry::new();
        let perf = make_perf("A001", RecipeStatus::Staged);
        let id = perf.recipe_id;
        registry.register(perf);

        registry.advance_staging_week();
        assert_eq!(registry.get_performance(&id).unwrap().weeks_in_staging, 1);

        registry.advance_staging_week();
        assert_eq!(registry.get_performance(&id).unwrap().weeks_in_staging, 2);
    }

    #[test]
    fn test_registry_status_counts() {
        let mut registry = RecipeRegistry::new();
        registry.register(make_perf("A001", RecipeStatus::Candidate));
        registry.register(make_perf("A002", RecipeStatus::Staged));
        registry.register(make_perf("A003", RecipeStatus::Promoted));
        registry.register(make_perf("A004", RecipeStatus::Promoted));

        let counts = registry.status_counts();
        assert_eq!(*counts.get("candidate").unwrap(), 1);
        assert_eq!(*counts.get("staged").unwrap(), 1);
        assert_eq!(*counts.get("promoted").unwrap(), 2);
    }

    #[test]
    fn test_registry_snapshot_roundtrip() {
        let mut registry = RecipeRegistry::new();
        let perf = make_perf("SNAP001", RecipeStatus::Candidate);
        let id = perf.recipe_id;
        registry.register(perf);

        let snapshot = registry
            .to_json_snapshot()
            .expect("snapshot serialization must work");
        let restored =
            RecipeRegistry::from_json_snapshot(&snapshot).expect("snapshot restore must work");

        assert!(restored.get_performance(&id).is_some());
        assert_eq!(
            restored
                .status_counts()
                .get("candidate")
                .copied()
                .unwrap_or(0),
            1
        );
    }

    #[test]
    fn test_registry_decisions_tracked() {
        let mut registry = RecipeRegistry::new();
        let perf = make_perf("A001", RecipeStatus::Candidate);
        let id = perf.recipe_id;
        registry.register(perf);

        registry.stage(&id);
        assert_eq!(registry.decisions().len(), 1);
    }

    #[test]
    fn test_full_lifecycle() {
        let mut registry = RecipeRegistry::new();

        // 1. Start as candidate
        let perf = make_perf("A001", RecipeStatus::Candidate);
        let id = perf.recipe_id;
        registry.register(perf);

        // 2. Stage it
        let decision = registry.stage(&id).unwrap();
        assert_eq!(decision.action, LifecycleAction::Stage);

        // 3. Advance 5 weeks, record good fires
        for _ in 0..5 {
            registry.advance_staging_week();
        }
        let p = registry.get_performance_mut(&id).unwrap();
        p.evidence_coverage = 0.8;
        for _ in 0..10 {
            p.record_fire(true);
        }

        // 4. Promote
        let promotions = registry.run_promotion_board(&PromotionCriteria::default());
        assert_eq!(promotions.len(), 1);
        assert_eq!(
            registry.get_performance(&id).unwrap().status,
            RecipeStatus::Promoted
        );

        // 5. Degrade performance
        let p = registry.get_performance_mut(&id).unwrap();
        for _ in 0..20 {
            p.record_fire(false);
        }
        // precision now = 10/30 ≈ 0.33

        // 6. Deprecate
        let deprecations = registry.run_deprecation_sweep(&DeprecationCriteria::default());
        assert_eq!(deprecations.len(), 1);
        assert_eq!(
            registry.get_performance(&id).unwrap().status,
            RecipeStatus::Retired
        );

        // Total decisions: stage + promote + deprecate = 3
        assert_eq!(registry.decisions().len(), 3);
    }

    // B144: Tests for gate config defaults and threshold behavior
    #[test]
    fn test_gate_config_defaults() {
        let config = GateConfig::default();
        assert!((config.min_uplift - 1.5).abs() < 1e-10);
        assert!((config.min_mutual_info - 0.1).abs() < 1e-10);
        assert!((config.max_p_value - 0.01).abs() < 1e-10);
        assert!((config.max_q_value - 0.05).abs() < 1e-10);
        assert_eq!(config.min_time_slices, 3);
        assert_eq!(config.total_time_slices, 4);
        assert_eq!(config.min_entities, 5);
        assert!((config.max_false_alarm_rate - 0.02).abs() < 1e-10);
        assert!((config.counterfactual_min_change - 0.1).abs() < 1e-10);
    }

    // B144: Promotion criteria defaults
    #[test]
    fn test_promotion_criteria_defaults() {
        let criteria = PromotionCriteria::default();
        assert_eq!(criteria.min_weeks_in_staging, 4);
        assert!((criteria.min_precision - 0.85).abs() < 1e-10);
        assert!((criteria.min_evidence_coverage - 0.7).abs() < 1e-10);
        assert!((criteria.max_false_positive_rate - 0.02).abs() < 1e-10);
        assert_eq!(criteria.min_total_fires, 3);
    }

    // B147: max_decisions trimming preserves most recent entries
    #[test]
    fn test_registry_max_decisions_trimming_preserves_recent() {
        let mut registry = RecipeRegistry::new();
        registry.max_decisions = 10; // small limit for testing

        for i in 0..15 {
            let perf = make_perf(&format!("R{:03}", i), RecipeStatus::Candidate);
            let id = perf.recipe_id;
            registry.register(perf);
            registry.stage(&id); // each stage generates a decision
        }

        // Should have trimmed but still hold recent decisions
        assert!(registry.decisions().len() <= 15);
        // The most recent decision should be for R014 (last staged)
        let last = registry.decisions().last().unwrap();
        assert_eq!(last.recipe_code, "R014");
        assert_eq!(last.action, LifecycleAction::Stage);
    }

    // B148: Deprecation logic when last_fired is None
    #[test]
    fn test_should_deprecate_last_fired_none() {
        let mut perf = make_perf("A001", RecipeStatus::Promoted);
        perf.last_fired = None;
        // Set staged_at to > 90 days ago
        perf.staged_at = Some(Utc::now() - chrono::Duration::days(100));
        perf.evidence_coverage = 0.8;

        let result = should_deprecate(&perf, &DeprecationCriteria::default(), Utc::now());
        assert!(
            result,
            "Should deprecate when last_fired is None and staged_at is old"
        );
    }

    #[test]
    fn test_should_not_deprecate_last_fired_none_recent_staged() {
        let mut perf = make_perf("A001", RecipeStatus::Promoted);
        perf.last_fired = None;
        perf.staged_at = Some(Utc::now() - chrono::Duration::days(10));
        perf.evidence_coverage = 0.8;

        let result = should_deprecate(&perf, &DeprecationCriteria::default(), Utc::now());
        assert!(!result, "Should not deprecate when staged_at is recent");
    }

    // ── B149: evidence_coverage clamping ──

    #[test]
    fn test_set_evidence_coverage_clamps_high() {
        let mut perf = make_perf("EC01", RecipeStatus::Staged);
        perf.set_evidence_coverage(1.5);
        assert!((perf.evidence_coverage - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_set_evidence_coverage_clamps_low() {
        let mut perf = make_perf("EC02", RecipeStatus::Staged);
        perf.set_evidence_coverage(-0.3);
        assert!((perf.evidence_coverage - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_set_evidence_coverage_valid() {
        let mut perf = make_perf("EC03", RecipeStatus::Staged);
        perf.set_evidence_coverage(0.75);
        assert!((perf.evidence_coverage - 0.75).abs() < f64::EPSILON);
    }

    // ── B296: backpressure tests (decisions registry trimming) ──

    #[test]
    fn test_registry_decisions_trimmed_at_max() {
        // Verify that decisions vec never grows beyond max_decisions
        let mut registry = RecipeRegistry::new();
        registry.max_decisions = 100;

        // Add candidate recipes and stage them to generate decisions
        for i in 0..200 {
            let perf = make_perf(&format!("TRIM{:03}", i), RecipeStatus::Candidate);
            let recipe_id = perf.recipe_id;
            registry.register(perf);
            // Staging generates a decision
            registry.stage(&recipe_id);
        }

        // Decisions should be trimmed to max_decisions
        assert_eq!(
            registry.decisions().len(),
            100,
            "decisions must be trimmed to max_decisions limit"
        );
    }

    #[test]
    fn test_registry_decisions_drops_oldest_20_percent() {
        let mut registry = RecipeRegistry::new();
        registry.max_decisions = 100;

        // Add exactly 100 candidate recipes
        for i in 0..100 {
            let perf = make_perf(&format!("D{:03}", i), RecipeStatus::Candidate);
            let recipe_id = perf.recipe_id;
            registry.register(perf);
            registry.stage(&recipe_id);
        }

        assert_eq!(registry.decisions().len(), 100);

        // Add 1 more - should trigger trim of 20 oldest
        let perf = make_perf("D100", RecipeStatus::Candidate);
        let recipe_id = perf.recipe_id;
        registry.register(perf);
        registry.stage(&recipe_id);

        // Should have trimmed 20, then added 1 → 81 total
        assert_eq!(registry.decisions().len(), 81);
        // First 20 decisions were dropped (codes D000-D019)
        // So first decision should be from D020
        assert_eq!(registry.decisions()[0].recipe_code, "D020");
        // Last decision should be D100
        assert_eq!(registry.decisions().last().unwrap().recipe_code, "D100");
    }

    #[test]
    fn test_registry_decisions_no_trim_when_below_limit() {
        let mut registry = RecipeRegistry::new();
        registry.max_decisions = 5000; // Default

        // Add only 50 candidates and stage them
        for i in 0..50 {
            let perf = make_perf(&format!("NT{:02}", i), RecipeStatus::Candidate);
            let recipe_id = perf.recipe_id;
            registry.register(perf);
            registry.stage(&recipe_id);
        }

        // All 50 should be present
        assert_eq!(registry.decisions().len(), 50);
        assert_eq!(registry.decisions()[0].recipe_code, "NT00");
        assert_eq!(registry.decisions()[49].recipe_code, "NT49");
    }

    #[test]
    fn test_registry_decisions_trimming_preserves_recent() {
        let mut registry = RecipeRegistry::new();
        registry.max_decisions = 10;

        // Add 30 candidates, stage them
        for i in 0..30 {
            let perf = make_perf(&format!("REC{:02}", i), RecipeStatus::Candidate);
            let recipe_id = perf.recipe_id;
            registry.register(perf);
            registry.stage(&recipe_id);
        }

        // Should have kept the most recent 10
        assert_eq!(registry.decisions().len(), 10);
        // Should contain REC20 through REC29
        assert_eq!(registry.decisions()[0].recipe_code, "REC20");
        assert_eq!(registry.decisions()[9].recipe_code, "REC29");
    }
}
