//! Statistical robustness gates for recipe validation.
//!
//! A candidate pattern must pass ALL gates before promotion:
//! 1. Effect size (uplift > 1.5× or MI > 0.1)
//! 2. Significance (p < 0.01)
//! 3. FDR correction (q < 0.05 Benjamini-Hochberg)
//! 4. Temporal stability (works in ≥3 of 4 time slices)
//! 5. Entity stability (works across ≥5 entities)
//! 6. Negative control (effect vanishes when shuffled)
//! 7. False alarm budget (does not exceed FP budget)
//! 8. Counterfactual (insight changes if one signal removed)

use serde::{Deserialize, Serialize};

// ────────────────────────────────────────────
// Gate configuration
// ────────────────────────────────────────────

/// Thresholds that each recipe candidate must satisfy to be promoted (B289).
///
/// # Default values
/// | Field                      | Default | Gate                                              |
/// |---------------------------|---------|--------------------------------------------------|
/// | `min_uplift`              | 1.5     | Gate 1: min odds-ratio uplift                    |
/// | `min_mutual_info`         | 0.1     | Gate 1: MI alternative to uplift                 |
/// | `max_p_value`             | 0.01    | Gate 2: Fisher exact significance threshold      |
/// | `max_q_value`             | 0.05    | Gate 3: Benjamini-Hochberg FDR threshold         |
/// | `min_time_slices`         | 3       | Gate 4: min slices where pattern must hold       |
/// | `total_time_slices`       | 4       | Gate 4: total cross-validation splits            |
/// | `min_entities`            | 5       | Gate 5: entity stability minimum count           |
/// | `max_false_alarm_rate`    | 0.02    | Gate 7: max tolerated FP rate                    |
/// | `counterfactual_min_change`| 0.1    | Gate 8: min effect change when signal removed    |
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateConfig {
    /// Minimum required odds-ratio uplift for Gate 1.  Default: `1.5`.
    pub min_uplift: f64,
    /// Minimum required mutual information for Gate 1 (fallback to uplift).  Default: `0.1`.
    pub min_mutual_info: f64,
    /// Maximum Fisher-exact p-value for Gate 2.  Default: `0.01`.
    pub max_p_value: f64,
    /// Maximum Benjamini-Hochberg q-value for Gate 3.  Default: `0.05`.
    pub max_q_value: f64,
    /// Minimum number of time slices where the pattern must hold (Gate 4).  Default: `3`.
    pub min_time_slices: u32,
    /// Total number of time-based cross-validation splits (Gate 4).  Default: `4`.
    pub total_time_slices: u32,
    /// Minimum entities that must exhibit the pattern (Gate 5).  Default: `5`.
    pub min_entities: u32,
    /// Maximum tolerated false-alarm rate (Gate 7).  Default: `0.02`.
    pub max_false_alarm_rate: f64,
    /// Minimum effect-size change when a signal is removed (Gate 8).  Default: `0.1`.
    pub counterfactual_min_change: f64,
}

impl Default for GateConfig {
    fn default() -> Self {
        Self {
            min_uplift: 1.5,
            min_mutual_info: 0.1,
            max_p_value: 0.01,
            max_q_value: 0.05,
            min_time_slices: 3,
            total_time_slices: 4,
            min_entities: 5,
            max_false_alarm_rate: 0.02,
            counterfactual_min_change: 0.1,
        }
    }
}

impl GateConfig {
    /// Validate that all numeric fields are within their valid operating ranges.
    ///
    /// Returns an empty `Vec` when the config is valid, or a list of
    /// human-readable error strings describing every out-of-range field.
    /// All fields are checked independently — the caller receives a complete
    /// list of problems in a single call rather than fail-fast on the first.
    ///
    /// # Valid ranges
    /// | Field                    | Constraint              | Rationale                          |
    /// |--------------------------|-------------------------|------------------------------------|
    /// | `min_uplift`             | `> 1.0`, finite         | Must exceed baseline               |
    /// | `min_mutual_info`        | `> 0.0`, finite         | Must be positive                   |
    /// | `max_p_value`            | `(0.0, 1.0)`, finite    | Valid probability                  |
    /// | `max_q_value`            | `(0.0, 1.0)`, finite    | Valid probability                  |
    /// | `max_p_value`            | `<= max_q_value`        | p threshold must not exceed q      |
    /// | `min_time_slices`        | `>= 1`                  | At least one slice required        |
    /// | `total_time_slices`      | `>= min_time_slices`    | Must have enough total slices      |
    /// | `min_entities`           | `>= 1`                  | At least one entity required       |
    /// | `max_false_alarm_rate`   | `(0.0, 1.0)`, finite    | Valid probability                  |
    /// | `counterfactual_min_change` | `> 0.0`, finite      | Must be positive                   |
    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();

        if !self.min_uplift.is_finite() || self.min_uplift <= 1.0 {
            errors.push(format!(
                "GateConfig.min_uplift = {} must be > 1.0 and finite",
                self.min_uplift
            ));
        }
        if !self.min_mutual_info.is_finite() || self.min_mutual_info <= 0.0 {
            errors.push(format!(
                "GateConfig.min_mutual_info = {} must be > 0.0 and finite",
                self.min_mutual_info
            ));
        }
        if !self.max_p_value.is_finite() || self.max_p_value <= 0.0 || self.max_p_value >= 1.0 {
            errors.push(format!(
                "GateConfig.max_p_value = {} must be in (0.0, 1.0)",
                self.max_p_value
            ));
        }
        if !self.max_q_value.is_finite() || self.max_q_value <= 0.0 || self.max_q_value >= 1.0 {
            errors.push(format!(
                "GateConfig.max_q_value = {} must be in (0.0, 1.0)",
                self.max_q_value
            ));
        }
        // Cross-field: p threshold must not be looser than q threshold
        if self.max_p_value.is_finite()
            && self.max_q_value.is_finite()
            && self.max_p_value > self.max_q_value
        {
            errors.push(format!(
                "GateConfig.max_p_value ({}) must be <= max_q_value ({}) \
                 (BH correction cannot tighten below raw p)",
                self.max_p_value, self.max_q_value
            ));
        }
        if self.min_time_slices < 1 {
            errors.push(format!(
                "GateConfig.min_time_slices = {} must be >= 1",
                self.min_time_slices
            ));
        }
        if self.total_time_slices < self.min_time_slices {
            errors.push(format!(
                "GateConfig.total_time_slices = {} must be >= min_time_slices = {}",
                self.total_time_slices, self.min_time_slices
            ));
        }
        if self.min_entities < 1 {
            errors.push(format!(
                "GateConfig.min_entities = {} must be >= 1",
                self.min_entities
            ));
        }
        if !self.max_false_alarm_rate.is_finite()
            || self.max_false_alarm_rate <= 0.0
            || self.max_false_alarm_rate >= 1.0
        {
            errors.push(format!(
                "GateConfig.max_false_alarm_rate = {} must be in (0.0, 1.0)",
                self.max_false_alarm_rate
            ));
        }
        if !self.counterfactual_min_change.is_finite() || self.counterfactual_min_change <= 0.0 {
            errors.push(format!(
                "GateConfig.counterfactual_min_change = {} must be > 0.0 and finite",
                self.counterfactual_min_change
            ));
        }

        errors
    }
}

// ────────────────────────────────────────────
// Gate evidence
// ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateEvidence {
    pub uplift: f64,
    pub mutual_info: f64,
    pub p_value: f64,
    pub q_value: f64,
    pub time_slices_passed: u32,
    pub total_time_slices: u32,
    pub entities_passed: u32,
    pub negative_control_effect: f64, // effect size on shuffled data
    pub false_alarm_rate: f64,
    pub counterfactual_change: f64, // change in effect when signal removed
}

// ────────────────────────────────────────────
// Individual gate checks
// ────────────────────────────────────────────

/// Gate 1: Effect size — uplift must exceed threshold OR MI must exceed threshold.
pub fn check_effect_size(evidence: &GateEvidence, config: &GateConfig) -> bool {
    evidence.uplift >= config.min_uplift || evidence.mutual_info >= config.min_mutual_info
}

/// Gate 2: Statistical significance — p-value must be below threshold.
pub fn check_significance(evidence: &GateEvidence, config: &GateConfig) -> bool {
    evidence.p_value <= config.max_p_value
}

/// Gate 3: FDR correction — q-value must be below threshold.
pub fn check_fdr(evidence: &GateEvidence, config: &GateConfig) -> bool {
    evidence.q_value <= config.max_q_value
}

/// Gate 4: Temporal stability — must pass in ≥ min_time_slices of total_time_slices.
pub fn check_temporal_stability(evidence: &GateEvidence, config: &GateConfig) -> bool {
    evidence.time_slices_passed >= config.min_time_slices
}

/// Gate 5: Entity stability — must work across ≥ min_entities.
pub fn check_entity_stability(evidence: &GateEvidence, config: &GateConfig) -> bool {
    evidence.entities_passed >= config.min_entities
}

/// Gate 6: Negative control — effect must vanish when data is shuffled.
/// We check that the negative control effect is much smaller than the real uplift.
pub fn check_negative_control(evidence: &GateEvidence, _config: &GateConfig) -> bool {
    // The shuffled effect should be < 50% of the real uplift.
    // A recipe with essentially zero real uplift cannot demonstrate the control condition.
    if evidence.uplift < f64::EPSILON {
        return false;
    }
    evidence.negative_control_effect / evidence.uplift < 0.5
}

/// Gate 7: False alarm budget — false alarm rate must be within budget.
pub fn check_false_alarm_budget(evidence: &GateEvidence, config: &GateConfig) -> bool {
    evidence.false_alarm_rate <= config.max_false_alarm_rate
}

/// Gate 8: Counterfactual — removing a signal must change the insight meaningfully.
pub fn check_counterfactual(evidence: &GateEvidence, config: &GateConfig) -> bool {
    evidence.counterfactual_change >= config.counterfactual_min_change
}

// ────────────────────────────────────────────
// Gate report
// ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateResult {
    pub name: String,
    pub passed: bool,
    pub detail: String,
}

/// Run all 8 gates and produce a report.
pub fn run_all_gates(evidence: &GateEvidence, config: &GateConfig) -> Vec<GateResult> {
    vec![
        GateResult {
            name: "effect_size".to_string(),
            passed: check_effect_size(evidence, config),
            detail: format!(
                "uplift={:.3}, MI={:.3} (need uplift>={:.1} OR MI>={:.2})",
                evidence.uplift, evidence.mutual_info, config.min_uplift, config.min_mutual_info
            ),
        },
        GateResult {
            name: "significance".to_string(),
            passed: check_significance(evidence, config),
            detail: format!(
                "p={:.4} (need p<{:.3})",
                evidence.p_value, config.max_p_value
            ),
        },
        GateResult {
            name: "fdr_correction".to_string(),
            passed: check_fdr(evidence, config),
            detail: format!(
                "q={:.4} (need q<{:.3})",
                evidence.q_value, config.max_q_value
            ),
        },
        GateResult {
            name: "temporal_stability".to_string(),
            passed: check_temporal_stability(evidence, config),
            detail: format!(
                "{}/{} time slices (need ≥{}/{})",
                evidence.time_slices_passed,
                evidence.total_time_slices,
                config.min_time_slices,
                config.total_time_slices
            ),
        },
        GateResult {
            name: "entity_stability".to_string(),
            passed: check_entity_stability(evidence, config),
            detail: format!(
                "{} entities (need ≥{})",
                evidence.entities_passed, config.min_entities
            ),
        },
        GateResult {
            name: "negative_control".to_string(),
            passed: check_negative_control(evidence, config),
            detail: format!(
                "shuffled_effect={:.3} vs real_uplift={:.3}",
                evidence.negative_control_effect, evidence.uplift
            ),
        },
        GateResult {
            name: "false_alarm_budget".to_string(),
            passed: check_false_alarm_budget(evidence, config),
            detail: format!(
                "FA_rate={:.4} (need ≤{:.3})",
                evidence.false_alarm_rate, config.max_false_alarm_rate
            ),
        },
        GateResult {
            name: "counterfactual".to_string(),
            passed: check_counterfactual(evidence, config),
            detail: format!(
                "change={:.3} (need ≥{:.2})",
                evidence.counterfactual_change, config.counterfactual_min_change
            ),
        },
    ]
}

/// Check if all gates pass.
pub fn all_gates_pass(evidence: &GateEvidence, config: &GateConfig) -> bool {
    let results = run_all_gates(evidence, config);
    results.iter().all(|r| r.passed)
}

/// Count how many gates pass.
pub fn gates_passed_count(evidence: &GateEvidence, config: &GateConfig) -> usize {
    run_all_gates(evidence, config)
        .iter()
        .filter(|r| r.passed)
        .count()
}

/// Get names of failed gates.
pub fn failed_gates(evidence: &GateEvidence, config: &GateConfig) -> Vec<String> {
    run_all_gates(evidence, config)
        .iter()
        .filter(|r| !r.passed)
        .map(|r| r.name.clone())
        .collect()
}

/// Generate a human-readable gate report.
pub fn format_gate_report(evidence: &GateEvidence, config: &GateConfig) -> String {
    let results = run_all_gates(evidence, config);
    let mut lines = Vec::new();
    lines.push("Gate Evaluation Report".to_string());
    lines.push("======================".to_string());

    for result in &results {
        let status = if result.passed { "PASS" } else { "FAIL" };
        lines.push(format!("[{}] {} — {}", status, result.name, result.detail));
    }

    let passed = results.iter().filter(|r| r.passed).count();
    let total = results.len();
    lines.push(format!("\nResult: {}/{} gates passed", passed, total));

    if passed == total {
        lines.push("VERDICT: ELIGIBLE FOR STAGING".to_string());
    } else {
        lines.push("VERDICT: NOT YET ELIGIBLE".to_string());
    }

    lines.join("\n")
}

// ────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn passing_evidence() -> GateEvidence {
        GateEvidence {
            uplift: 2.0,
            mutual_info: 0.15,
            p_value: 0.005,
            q_value: 0.03,
            time_slices_passed: 3,
            total_time_slices: 4,
            entities_passed: 8,
            negative_control_effect: 0.3, // < 50% of uplift 2.0
            false_alarm_rate: 0.01,
            counterfactual_change: 0.2,
        }
    }

    fn failing_evidence() -> GateEvidence {
        GateEvidence {
            uplift: 1.0,           // below 1.5
            mutual_info: 0.05,     // below 0.1
            p_value: 0.05,         // above 0.01
            q_value: 0.1,          // above 0.05
            time_slices_passed: 1, // below 3
            total_time_slices: 4,
            entities_passed: 2,           // below 5
            negative_control_effect: 0.8, // > 50% of uplift
            false_alarm_rate: 0.05,       // above 0.02
            counterfactual_change: 0.05,  // below 0.1
        }
    }

    #[test]
    fn test_check_effect_size_pass() {
        let config = GateConfig::default();
        let ev = passing_evidence();
        assert!(check_effect_size(&ev, &config));
    }

    #[test]
    fn test_check_effect_size_fail() {
        let config = GateConfig::default();
        let ev = failing_evidence();
        assert!(!check_effect_size(&ev, &config));
    }

    #[test]
    fn test_check_effect_size_mi_pass() {
        let config = GateConfig::default();
        let mut ev = failing_evidence();
        ev.mutual_info = 0.2; // MI passes even though uplift fails
        assert!(check_effect_size(&ev, &config));
    }

    #[test]
    fn test_check_significance() {
        let config = GateConfig::default();
        assert!(check_significance(&passing_evidence(), &config));
        assert!(!check_significance(&failing_evidence(), &config));
    }

    #[test]
    fn test_check_fdr() {
        let config = GateConfig::default();
        assert!(check_fdr(&passing_evidence(), &config));
        assert!(!check_fdr(&failing_evidence(), &config));
    }

    #[test]
    fn test_check_temporal_stability() {
        let config = GateConfig::default();
        assert!(check_temporal_stability(&passing_evidence(), &config));
        assert!(!check_temporal_stability(&failing_evidence(), &config));
    }

    #[test]
    fn test_check_entity_stability() {
        let config = GateConfig::default();
        assert!(check_entity_stability(&passing_evidence(), &config));
        assert!(!check_entity_stability(&failing_evidence(), &config));
    }

    #[test]
    fn test_check_negative_control() {
        let config = GateConfig::default();
        assert!(check_negative_control(&passing_evidence(), &config));
        assert!(!check_negative_control(&failing_evidence(), &config));
    }

    #[test]
    fn test_check_false_alarm_budget() {
        let config = GateConfig::default();
        assert!(check_false_alarm_budget(&passing_evidence(), &config));
        assert!(!check_false_alarm_budget(&failing_evidence(), &config));
    }

    #[test]
    fn test_check_counterfactual() {
        let config = GateConfig::default();
        assert!(check_counterfactual(&passing_evidence(), &config));
        assert!(!check_counterfactual(&failing_evidence(), &config));
    }

    #[test]
    fn test_gate_evidence_negative_values_fail_relevant_gates() {
        let config = GateConfig::default();
        let ev = GateEvidence {
            uplift: -1.0,
            mutual_info: -0.1,
            p_value: -0.01,
            q_value: -0.02,
            time_slices_passed: 0,
            total_time_slices: 4,
            entities_passed: 0,
            negative_control_effect: -0.5,
            false_alarm_rate: -0.1,
            counterfactual_change: -0.2,
        };
        assert!(!check_effect_size(&ev, &config));
        assert!(check_significance(&ev, &config)); // <= threshold still passes by rule
        assert!(check_fdr(&ev, &config)); // <= threshold still passes by rule
        assert!(!check_negative_control(&ev, &config));
        assert!(check_false_alarm_budget(&ev, &config)); // <= threshold still passes by rule
    }

    #[test]
    fn test_check_negative_control_tiny_uplift_fails() {
        let config = GateConfig::default();
        let mut ev = passing_evidence();
        ev.uplift = 1e-20;
        ev.negative_control_effect = 0.0;
        assert!(!check_negative_control(&ev, &config));
    }

    #[test]
    fn test_check_significance_boundary_p_value() {
        let config = GateConfig::default();
        let mut ev = passing_evidence();
        ev.p_value = config.max_p_value;
        assert!(check_significance(&ev, &config));
    }

    #[test]
    fn test_check_fdr_boundary_q_value() {
        let config = GateConfig::default();
        let mut ev = passing_evidence();
        ev.q_value = config.max_q_value;
        assert!(check_fdr(&ev, &config));
    }

    #[test]
    fn test_all_gates_pass() {
        let config = GateConfig::default();
        assert!(all_gates_pass(&passing_evidence(), &config));
        assert!(!all_gates_pass(&failing_evidence(), &config));
    }

    #[test]
    fn test_gates_passed_count() {
        let config = GateConfig::default();
        assert_eq!(gates_passed_count(&passing_evidence(), &config), 8);
        assert_eq!(gates_passed_count(&failing_evidence(), &config), 0);
    }

    #[test]
    fn test_failed_gates() {
        let config = GateConfig::default();
        let failed = failed_gates(&passing_evidence(), &config);
        assert!(failed.is_empty());

        let failed = failed_gates(&failing_evidence(), &config);
        assert_eq!(failed.len(), 8);
    }

    #[test]
    fn test_run_all_gates_report() {
        let config = GateConfig::default();
        let results = run_all_gates(&passing_evidence(), &config);
        assert_eq!(results.len(), 8);
        for r in &results {
            assert!(r.passed, "Gate {} should pass: {}", r.name, r.detail);
        }
    }

    #[test]
    fn test_partial_pass() {
        let config = GateConfig::default();
        let mut ev = passing_evidence();
        ev.p_value = 0.05; // only significance fails
        ev.q_value = 0.1; // and FDR fails

        assert!(!all_gates_pass(&ev, &config));
        assert_eq!(gates_passed_count(&ev, &config), 6);
        let failed = failed_gates(&ev, &config);
        assert!(failed.contains(&"significance".to_string()));
        assert!(failed.contains(&"fdr_correction".to_string()));
    }

    #[test]
    fn test_format_gate_report_passing() {
        let config = GateConfig::default();
        let report = format_gate_report(&passing_evidence(), &config);
        assert!(report.contains("PASS"));
        assert!(report.contains("8/8 gates passed"));
        assert!(report.contains("ELIGIBLE FOR STAGING"));
    }

    #[test]
    fn test_format_gate_report_failing() {
        let config = GateConfig::default();
        let report = format_gate_report(&failing_evidence(), &config);
        assert!(report.contains("FAIL"));
        assert!(report.contains("0/8 gates passed"));
        assert!(report.contains("NOT YET ELIGIBLE"));
    }

    // ── B150: counterfactual gate sensitivity tests ──

    #[test]
    fn test_counterfactual_at_boundary() {
        let config = GateConfig::default(); // counterfactual_min_change = 0.1
        let mut ev = passing_evidence();
        // Exactly at boundary
        ev.counterfactual_change = 0.1;
        assert!(check_counterfactual(&ev, &config));
        // Just below boundary
        ev.counterfactual_change = 0.099;
        assert!(!check_counterfactual(&ev, &config));
    }

    #[test]
    fn test_counterfactual_zero_change() {
        let config = GateConfig::default();
        let mut ev = passing_evidence();
        ev.counterfactual_change = 0.0;
        assert!(!check_counterfactual(&ev, &config));
    }

    #[test]
    fn test_counterfactual_large_change() {
        let config = GateConfig::default();
        let mut ev = passing_evidence();
        ev.counterfactual_change = 1.0;
        assert!(check_counterfactual(&ev, &config));
    }

    #[test]
    fn test_custom_config() {
        let config = GateConfig {
            min_uplift: 1.0, // more lenient
            min_mutual_info: 0.05,
            max_p_value: 0.05,
            max_q_value: 0.1,
            min_time_slices: 1,
            total_time_slices: 4,
            min_entities: 2,
            max_false_alarm_rate: 0.1,
            counterfactual_min_change: 0.01,
        };
        // With lenient config, previously failing evidence should mostly pass
        let ev = failing_evidence();
        assert!(check_effect_size(&ev, &config));
        assert!(check_significance(&ev, &config));
        assert!(check_fdr(&ev, &config));
        assert!(check_temporal_stability(&ev, &config));
        assert!(check_entity_stability(&ev, &config));
        assert!(check_false_alarm_budget(&ev, &config));
        assert!(check_counterfactual(&ev, &config));
    }

    // B289: GateConfig default stability
    #[test]
    fn test_gate_config_default_is_stable_and_valid() {
        let a = GateConfig::default();
        let b = GateConfig::default();
        // Deterministic across two calls
        assert!((a.min_uplift - b.min_uplift).abs() < f64::EPSILON);
        assert!((a.max_p_value - b.max_p_value).abs() < f64::EPSILON);
        assert_eq!(a.min_time_slices, b.min_time_slices);
        // Values are in valid ranges
        assert!(
            a.min_uplift > 1.0,
            "min_uplift must be > 1.0 (represents uplift over baseline)"
        );
        assert!(
            a.max_p_value > 0.0 && a.max_p_value < 1.0,
            "max_p_value must be in (0,1)"
        );
        assert!(
            a.max_q_value > 0.0 && a.max_q_value < 1.0,
            "max_q_value must be in (0,1)"
        );
        assert!(
            a.max_p_value <= a.max_q_value,
            "p threshold should be ≤ q threshold"
        );
        assert!(
            a.min_time_slices <= a.total_time_slices,
            "min_time_slices must not exceed total"
        );
        assert!(
            a.max_false_alarm_rate > 0.0 && a.max_false_alarm_rate < 1.0,
            "FAR must be in (0,1)"
        );
        assert!(
            a.counterfactual_min_change > 0.0,
            "min counterfactual change must be positive"
        );
    }

    #[test]
    fn test_gate_config_default_values_match_documentation() {
        let cfg = GateConfig::default();
        assert!(
            (cfg.min_uplift - 1.5).abs() < f64::EPSILON,
            "min_uplift default is 1.5"
        );
        assert!(
            (cfg.min_mutual_info - 0.1).abs() < f64::EPSILON,
            "min_mutual_info default is 0.1"
        );
        assert!(
            (cfg.max_p_value - 0.01).abs() < f64::EPSILON,
            "max_p_value default is 0.01"
        );
        assert!(
            (cfg.max_q_value - 0.05).abs() < f64::EPSILON,
            "max_q_value default is 0.05"
        );
        assert_eq!(cfg.min_time_slices, 3, "min_time_slices default is 3");
        assert_eq!(cfg.total_time_slices, 4, "total_time_slices default is 4");
        assert_eq!(cfg.min_entities, 5, "min_entities default is 5");
        assert!(
            (cfg.max_false_alarm_rate - 0.02).abs() < f64::EPSILON,
            "max_false_alarm_rate default is 0.02"
        );
        assert!(
            (cfg.counterfactual_min_change - 0.1).abs() < f64::EPSILON,
            "counterfactual_min_change default is 0.1"
        );
    }

    // B291: GateConfig::validate
    #[test]
    fn test_gate_config_default_passes_validation() {
        assert!(
            GateConfig::default().validate().is_empty(),
            "default GateConfig must be valid out of the box"
        );
    }

    #[test]
    fn test_gate_config_p_value_at_one_is_invalid() {
        let cfg = GateConfig {
            max_p_value: 1.0,
            ..GateConfig::default()
        };
        let errs = cfg.validate();
        assert!(errs.iter().any(|e| e.contains("max_p_value")));
    }

    #[test]
    fn test_gate_config_p_greater_than_q_cross_field_error() {
        // p=0.1 > q=0.05 violates BH correction ordering
        let cfg = GateConfig {
            max_p_value: 0.1,
            max_q_value: 0.05,
            ..GateConfig::default()
        };
        let errs = cfg.validate();
        assert!(
            errs.iter()
                .any(|e| e.contains("max_p_value") && e.contains("max_q_value")),
            "cross-field error must mention both fields"
        );
    }

    #[test]
    fn test_gate_config_total_slices_below_min_is_invalid() {
        let cfg = GateConfig {
            min_time_slices: 5,
            total_time_slices: 3,
            ..GateConfig::default()
        };
        let errs = cfg.validate();
        assert!(errs.iter().any(|e| e.contains("total_time_slices")));
    }

    #[test]
    fn test_gate_config_infinite_uplift_is_invalid() {
        let cfg = GateConfig {
            min_uplift: f64::INFINITY,
            ..GateConfig::default()
        };
        let errs = cfg.validate();
        assert!(errs.iter().any(|e| e.contains("min_uplift")));
    }

    #[test]
    fn test_gate_config_all_invalid_all_reported() {
        let cfg = GateConfig {
            min_uplift: 0.5,                // <= 1.0
            min_mutual_info: -1.0,          // <= 0
            max_p_value: 0.0,               // <= 0
            max_q_value: 1.1,               // >= 1
            min_time_slices: 0,             // < 1
            total_time_slices: 0,           // < min
            min_entities: 0,                // < 1
            max_false_alarm_rate: 0.0,      // <= 0
            counterfactual_min_change: 0.0, // <= 0
        };
        let errs = cfg.validate();
        // Check all nine broken fields are represented
        assert!(errs.iter().any(|e| e.contains("min_uplift")));
        assert!(errs.iter().any(|e| e.contains("min_mutual_info")));
        assert!(errs.iter().any(|e| e.contains("max_p_value")));
        assert!(errs.iter().any(|e| e.contains("max_q_value")));
        assert!(errs.iter().any(|e| e.contains("min_time_slices")));
        assert!(errs.iter().any(|e| e.contains("min_entities")));
        assert!(errs.iter().any(|e| e.contains("max_false_alarm_rate")));
        assert!(errs.iter().any(|e| e.contains("counterfactual_min_change")));
    }
}
