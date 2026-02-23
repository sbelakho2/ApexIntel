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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateConfig {
    pub min_uplift: f64,
    pub min_mutual_info: f64,
    pub max_p_value: f64,
    pub max_q_value: f64,
    pub min_time_slices: u32,
    pub total_time_slices: u32,
    pub min_entities: u32,
    pub max_false_alarm_rate: f64,
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
    pub negative_control_effect: f64,  // effect size on shuffled data
    pub false_alarm_rate: f64,
    pub counterfactual_change: f64,    // change in effect when signal removed
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
    // The shuffled effect should be < 50% of the real uplift
    if evidence.uplift <= 0.0 {
        return true;
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
            negative_control_effect: 0.3,  // < 50% of uplift 2.0
            false_alarm_rate: 0.01,
            counterfactual_change: 0.2,
        }
    }

    fn failing_evidence() -> GateEvidence {
        GateEvidence {
            uplift: 1.0,          // below 1.5
            mutual_info: 0.05,     // below 0.1
            p_value: 0.05,         // above 0.01
            q_value: 0.1,          // above 0.05
            time_slices_passed: 1, // below 3
            total_time_slices: 4,
            entities_passed: 2,    // below 5
            negative_control_effect: 0.8, // > 50% of uplift
            false_alarm_rate: 0.05, // above 0.02
            counterfactual_change: 0.05, // below 0.1
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
        ev.q_value = 0.1;  // and FDR fails

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

    #[test]
    fn test_custom_config() {
        let config = GateConfig {
            min_uplift: 1.0,  // more lenient
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
}
