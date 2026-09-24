//! Frozen-set evaluation and promotion gating for measurable learning (P0 #38).
//!
//! Every proposed rule / model / prompt promotion must be evaluated against a
//! *frozen* evaluation set and may only be promoted when the candidate shows a
//! statistically meaningful improvement and no critical regression.
//!
//! Two rules are enforced structurally here:
//!
//! 1. **Comparisons are same-set and same-version.** A candidate run and its
//!    baseline must reference the identical frozen evaluation set (id and
//!    version); otherwise the decision is rejected outright.
//! 2. **Raw analyst behaviour is not training truth.** Each metric records the
//!    [`AnalystSignalClass`] it was computed from. Only
//!    [`AnalystSignalClass::PositiveConfirmation`] rows may back a promotion;
//!    dismissal/noise and workflow-convenience actions are persisted for
//!    diagnosis but never count as evidence.
//!
//! The persisted schema lives in `migrations/051_learning_eval_metrics.sql`
//! (`learning_eval_sets`, `learning_eval_runs`, `learning_eval_metrics`).

use serde::{Deserialize, Serialize};

/// The nine persisted learning metrics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LearningMetric {
    Precision,
    FalsePositiveRate,
    DuplicateRate,
    GroundingFailureRate,
    AnalystAcceptanceRate,
    AnalystDismissalRate,
    TimeToActionHours,
    SourceYield,
    EntityLinkingAccuracy,
}

/// Whether larger metric values are better or worse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricDirection {
    HigherIsBetter,
    LowerIsBetter,
}

/// How an analyst interaction was produced.
///
/// Only explicit positive confirmation is training truth. Dismissals are real
/// signal but not ground truth (an analyst may dismiss because of workflow
/// friction, not because the output was wrong), and workflow-convenience
/// actions (opening, clicking, bookmarking) carry no judgement at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnalystSignalClass {
    PositiveConfirmation,
    DismissalNoise,
    WorkflowConvenience,
}

impl AnalystSignalClass {
    /// Only explicit positive confirmation may be treated as training truth.
    pub const fn is_training_truth(self) -> bool {
        matches!(self, Self::PositiveConfirmation)
    }

    /// Classify a raw analyst action token.
    ///
    /// Unknown tokens are conservatively treated as workflow convenience: the
    /// system never upgrades an unrecognised behaviour into training truth.
    pub fn classify_raw_action(raw: &str) -> Self {
        let normalized = raw.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "confirm" | "confirmed" | "accept" | "accepted" | "approve" | "approved"
            | "promote" | "promoted" | "true_positive" | "actioned" | "action" => {
                Self::PositiveConfirmation
            }
            "dismiss" | "dismissed" | "reject" | "rejected" | "false_positive" | "noise"
            | "mark_noise" | "duplicate" | "irrelevant" | "wrong" => Self::DismissalNoise,
            _ => Self::WorkflowConvenience,
        }
    }
}

impl LearningMetric {
    /// All metrics that must be present on an evaluation run.
    pub const ALL: [LearningMetric; 9] = [
        LearningMetric::Precision,
        LearningMetric::FalsePositiveRate,
        LearningMetric::DuplicateRate,
        LearningMetric::GroundingFailureRate,
        LearningMetric::AnalystAcceptanceRate,
        LearningMetric::AnalystDismissalRate,
        LearningMetric::TimeToActionHours,
        LearningMetric::SourceYield,
        LearningMetric::EntityLinkingAccuracy,
    ];

    /// Stable identifier used in `learning_eval_metrics.metric`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Precision => "precision",
            Self::FalsePositiveRate => "false_positive_rate",
            Self::DuplicateRate => "duplicate_rate",
            Self::GroundingFailureRate => "grounding_failure_rate",
            Self::AnalystAcceptanceRate => "analyst_acceptance_rate",
            Self::AnalystDismissalRate => "analyst_dismissal_rate",
            Self::TimeToActionHours => "time_to_action_hours",
            Self::SourceYield => "source_yield",
            Self::EntityLinkingAccuracy => "entity_linking_accuracy",
        }
    }

    pub const fn direction(self) -> MetricDirection {
        match self {
            Self::FalsePositiveRate
            | Self::DuplicateRate
            | Self::GroundingFailureRate
            | Self::AnalystDismissalRate
            | Self::TimeToActionHours => MetricDirection::LowerIsBetter,
            Self::Precision
            | Self::AnalystAcceptanceRate
            | Self::SourceYield
            | Self::EntityLinkingAccuracy => MetricDirection::HigherIsBetter,
        }
    }

    /// Metrics that can block or justify a promotion.
    ///
    /// Diagnostic metrics (`analyst_dismissal_rate`, `time_to_action_hours`,
    /// `source_yield`) are persisted and versioned but never gate a promotion
    /// on their own: dismissals are not truth and workflow metrics measure
    /// convenience, not correctness.
    pub const fn gates_promotion(self) -> bool {
        matches!(
            self,
            Self::Precision
                | Self::FalsePositiveRate
                | Self::DuplicateRate
                | Self::GroundingFailureRate
                | Self::AnalystAcceptanceRate
                | Self::EntityLinkingAccuracy
        )
    }

    /// The analyst-signal class that must back this metric as evidence.
    pub const fn required_signal_class(self) -> AnalystSignalClass {
        match self {
            Self::AnalystDismissalRate => AnalystSignalClass::DismissalNoise,
            Self::TimeToActionHours | Self::SourceYield => AnalystSignalClass::WorkflowConvenience,
            Self::Precision
            | Self::FalsePositiveRate
            | Self::DuplicateRate
            | Self::GroundingFailureRate
            | Self::AnalystAcceptanceRate
            | Self::EntityLinkingAccuracy => AnalystSignalClass::PositiveConfirmation,
        }
    }

    /// Critical metrics may never regress, even if another metric improves.
    pub const fn is_critical(self) -> bool {
        matches!(self, Self::FalsePositiveRate | Self::GroundingFailureRate)
    }
}

/// One metric observation from a single evaluation run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetricObservation {
    pub metric: LearningMetric,
    pub value: f64,
    pub sample_size: u64,
    pub signal_class: AnalystSignalClass,
    #[serde(default)]
    pub is_critical: bool,
}

/// Reference to the frozen evaluation set a run was measured against.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrozenEvalSetRef {
    pub id: String,
    pub name: String,
    pub version: u32,
    pub example_count: u64,
}

/// Kind of artefact being evaluated for promotion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateKind {
    Rule,
    Model,
    Prompt,
    Recipe,
    Threshold,
}

/// Identity of the proposed change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateRef {
    pub kind: CandidateKind,
    pub reference: String,
    pub version: Option<String>,
}

/// A versioned evaluation of one candidate against one frozen set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvaluationRun {
    pub run_id: String,
    pub eval_set: FrozenEvalSetRef,
    pub candidate: CandidateRef,
    /// Version of the metric schema/gate used for this run.
    pub metrics_version: u32,
    pub observations: Vec<MetricObservation>,
}

impl EvaluationRun {
    pub fn observation(&self, metric: LearningMetric) -> Option<&MetricObservation> {
        self.observations.iter().find(|obs| obs.metric == metric)
    }

    /// Total confirmed samples across gating metrics (0 when empty).
    pub fn gating_sample_size(&self) -> u64 {
        self.observations
            .iter()
            .filter(|obs| obs.metric.gates_promotion())
            .map(|obs| obs.sample_size)
            .max()
            .unwrap_or(0)
    }
}

/// Thresholds for the promotion gate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PromotionGateConfig {
    /// Minimum per-metric sample size for a decision.
    pub min_sample_size: u64,
    /// Minimum absolute improvement (in metric units) to count as meaningful.
    pub min_absolute_improvement: f64,
    /// Two-proportion z threshold for statistical significance (1.96 ≈ p<0.05).
    pub significance_z: f64,
    /// Maximum tolerated regression on a critical metric (0.0 = none allowed).
    pub max_critical_regression: f64,
}

impl Default for PromotionGateConfig {
    fn default() -> Self {
        Self {
            min_sample_size: 30,
            min_absolute_improvement: 0.02,
            significance_z: 1.96,
            max_critical_regression: 0.0,
        }
    }
}

/// Difference between candidate and baseline for one metric.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetricDelta {
    pub metric: LearningMetric,
    pub baseline_value: f64,
    pub candidate_value: f64,
    /// `candidate_value - baseline_value` (positive = larger value).
    pub absolute_delta: f64,
    /// Signed in the metric's improvement direction (positive = better).
    pub improvement: f64,
    pub z_score: Option<f64>,
    pub significant: bool,
}

/// Why a candidate was rejected.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum PromotionRejection {
    /// Candidate and baseline were measured on different frozen sets/versions.
    EvaluationSetMismatch {
        baseline_set: String,
        candidate_set: String,
    },
    MissingMetric {
        metric: LearningMetric,
    },
    InsufficientSamples {
        metric: LearningMetric,
        baseline_samples: u64,
        candidate_samples: u64,
        required: u64,
    },
    /// The metric row was not backed by the required analyst-signal class.
    NonTruthEvidence {
        metric: LearningMetric,
        signal_class: AnalystSignalClass,
    },
    /// A critical metric regressed beyond the configured tolerance.
    CriticalRegression {
        delta: MetricDelta,
    },
    /// A non-critical gating metric regressed significantly.
    SignificantRegression {
        delta: MetricDelta,
    },
    /// No gating metric improved meaningfully.
    NoSignificantImprovement {
        evaluated_metrics: usize,
    },
}

/// Outcome of the promotion gate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum PromotionDecision {
    Promote {
        improvements: Vec<MetricDelta>,
        evaluated_metrics: usize,
    },
    Reject(PromotionRejection),
}

/// Two-proportion z statistic for `candidate/lower` versus `baseline`.
///
/// Returns `None` when either sample is empty or the pooled variance is zero
/// (both proportions identical at 0 or 1, i.e. no evidence of a difference).
fn two_proportion_z(
    candidate_p: f64,
    candidate_n: u64,
    baseline_p: f64,
    baseline_n: u64,
) -> Option<f64> {
    if candidate_n == 0 || baseline_n == 0 {
        return None;
    }
    let n1 = candidate_n as f64;
    let n2 = baseline_n as f64;
    let pooled = (candidate_p * n1 + baseline_p * n2) / (n1 + n2);
    let variance = pooled * (1.0 - pooled) * (1.0 / n1 + 1.0 / n2);
    if !variance.is_finite() || variance <= f64::EPSILON {
        return None;
    }
    Some((candidate_p - baseline_p) / variance.sqrt())
}

fn build_delta(
    metric: LearningMetric,
    baseline: &MetricObservation,
    candidate: &MetricObservation,
    significance_z: f64,
) -> MetricDelta {
    let absolute_delta = candidate.value - baseline.value;
    let improvement = match metric.direction() {
        MetricDirection::HigherIsBetter => absolute_delta,
        MetricDirection::LowerIsBetter => -absolute_delta,
    };
    let z_score = two_proportion_z(
        candidate.value,
        candidate.sample_size,
        baseline.value,
        baseline.sample_size,
    );
    let significant = match z_score {
        Some(z) => match metric.direction() {
            MetricDirection::HigherIsBetter => z >= significance_z,
            MetricDirection::LowerIsBetter => z <= -significance_z,
        },
        None => false,
    };
    MetricDelta {
        metric,
        baseline_value: baseline.value,
        candidate_value: candidate.value,
        absolute_delta,
        improvement,
        z_score,
        significant,
    }
}

/// Compare a candidate run against its baseline on the same frozen set.
///
/// Promotes only when at least one gating metric improves by at least
/// `config.min_absolute_improvement` with statistical significance and no
/// critical metric regresses beyond `config.max_critical_regression`.
pub fn evaluate_promotion(
    candidate: &EvaluationRun,
    baseline: &EvaluationRun,
    config: &PromotionGateConfig,
) -> PromotionDecision {
    if candidate.eval_set != baseline.eval_set {
        return PromotionDecision::Reject(PromotionRejection::EvaluationSetMismatch {
            baseline_set: format!("{}@v{}", baseline.eval_set.id, baseline.eval_set.version),
            candidate_set: format!("{}@v{}", candidate.eval_set.id, candidate.eval_set.version),
        });
    }

    let mut improvements: Vec<MetricDelta> = Vec::new();
    let mut evaluated_metrics = 0usize;

    for metric in LearningMetric::ALL {
        let Some(baseline_obs) = baseline.observation(metric) else {
            return PromotionDecision::Reject(PromotionRejection::MissingMetric { metric });
        };
        let Some(candidate_obs) = candidate.observation(metric) else {
            return PromotionDecision::Reject(PromotionRejection::MissingMetric { metric });
        };

        let required_class = metric.required_signal_class();
        if baseline_obs.signal_class != required_class
            || candidate_obs.signal_class != required_class
        {
            let offending = if candidate_obs.signal_class != required_class {
                candidate_obs.signal_class
            } else {
                baseline_obs.signal_class
            };
            return PromotionDecision::Reject(PromotionRejection::NonTruthEvidence {
                metric,
                signal_class: offending,
            });
        }

        if !metric.gates_promotion() {
            continue;
        }

        if candidate_obs.sample_size < config.min_sample_size
            || baseline_obs.sample_size < config.min_sample_size
        {
            return PromotionDecision::Reject(PromotionRejection::InsufficientSamples {
                metric,
                baseline_samples: baseline_obs.sample_size,
                candidate_samples: candidate_obs.sample_size,
                required: config.min_sample_size,
            });
        }

        evaluated_metrics += 1;
        let delta = build_delta(metric, baseline_obs, candidate_obs, config.significance_z);

        if metric.is_critical() && delta.improvement < -config.max_critical_regression {
            return PromotionDecision::Reject(PromotionRejection::CriticalRegression { delta });
        }

        if !metric.is_critical() && delta.significant && delta.improvement < 0.0 {
            return PromotionDecision::Reject(PromotionRejection::SignificantRegression { delta });
        }

        if delta.significant && delta.improvement >= config.min_absolute_improvement {
            improvements.push(delta);
        }
    }

    if improvements.is_empty() {
        return PromotionDecision::Reject(PromotionRejection::NoSignificantImprovement {
            evaluated_metrics,
        });
    }

    PromotionDecision::Promote {
        improvements,
        evaluated_metrics,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FROZEN_SET_ID: &str = "2f6a0f8e-8f6f-4d4f-9a4f-0f6a8e8f6f4d";

    fn frozen_set(version: u32) -> FrozenEvalSetRef {
        FrozenEvalSetRef {
            id: FROZEN_SET_ID.to_string(),
            name: "insight_quality_golden_set".to_string(),
            version,
            example_count: 500,
        }
    }

    fn base_observations(
        precision: f64,
        fpr: f64,
        grounding: f64,
        n: u64,
    ) -> Vec<MetricObservation> {
        vec![
            MetricObservation {
                metric: LearningMetric::Precision,
                value: precision,
                sample_size: n,
                signal_class: AnalystSignalClass::PositiveConfirmation,
                is_critical: false,
            },
            MetricObservation {
                metric: LearningMetric::FalsePositiveRate,
                value: fpr,
                sample_size: n,
                signal_class: AnalystSignalClass::PositiveConfirmation,
                is_critical: true,
            },
            MetricObservation {
                metric: LearningMetric::DuplicateRate,
                value: 0.08,
                sample_size: n,
                signal_class: AnalystSignalClass::PositiveConfirmation,
                is_critical: false,
            },
            MetricObservation {
                metric: LearningMetric::GroundingFailureRate,
                value: grounding,
                sample_size: n,
                signal_class: AnalystSignalClass::PositiveConfirmation,
                is_critical: true,
            },
            MetricObservation {
                metric: LearningMetric::AnalystAcceptanceRate,
                value: 0.55,
                sample_size: n,
                signal_class: AnalystSignalClass::PositiveConfirmation,
                is_critical: false,
            },
            MetricObservation {
                metric: LearningMetric::AnalystDismissalRate,
                value: 0.30,
                sample_size: n,
                signal_class: AnalystSignalClass::DismissalNoise,
                is_critical: false,
            },
            MetricObservation {
                metric: LearningMetric::TimeToActionHours,
                value: 18.0,
                sample_size: n,
                signal_class: AnalystSignalClass::WorkflowConvenience,
                is_critical: false,
            },
            MetricObservation {
                metric: LearningMetric::SourceYield,
                value: 0.40,
                sample_size: n,
                signal_class: AnalystSignalClass::WorkflowConvenience,
                is_critical: false,
            },
            MetricObservation {
                metric: LearningMetric::EntityLinkingAccuracy,
                value: 0.80,
                sample_size: n,
                signal_class: AnalystSignalClass::PositiveConfirmation,
                is_critical: false,
            },
        ]
    }

    fn run(
        run_id: &str,
        candidate_ref: &str,
        version: u32,
        observations: Vec<MetricObservation>,
    ) -> EvaluationRun {
        EvaluationRun {
            run_id: run_id.to_string(),
            eval_set: frozen_set(version),
            candidate: CandidateRef {
                kind: CandidateKind::Prompt,
                reference: candidate_ref.to_string(),
                version: None,
            },
            metrics_version: 1,
            observations,
        }
    }

    #[test]
    fn meaningful_improvement_on_frozen_set_is_promoted() {
        let baseline = run(
            "base-1",
            "insight_prompt",
            3,
            base_observations(0.60, 0.10, 0.05, 200),
        );
        let candidate = run(
            "cand-1",
            "insight_prompt",
            3,
            base_observations(0.72, 0.09, 0.04, 200),
        );

        let decision = evaluate_promotion(&candidate, &baseline, &PromotionGateConfig::default());

        match decision {
            PromotionDecision::Promote {
                improvements,
                evaluated_metrics,
            } => {
                assert_eq!(evaluated_metrics, 6);
                assert!(improvements
                    .iter()
                    .any(|delta| delta.metric == LearningMetric::Precision && delta.significant));
            }
            other => panic!("expected promotion, got {other:?}"),
        }
    }

    #[test]
    fn critical_regression_is_rejected_even_when_precision_improves() {
        let baseline = run(
            "base-2",
            "insight_prompt",
            3,
            base_observations(0.60, 0.10, 0.05, 200),
        );
        let candidate = run(
            "cand-2",
            "insight_prompt",
            3,
            base_observations(0.80, 0.10, 0.12, 200),
        );

        let decision = evaluate_promotion(&candidate, &baseline, &PromotionGateConfig::default());

        match decision {
            PromotionDecision::Reject(PromotionRejection::CriticalRegression { delta }) => {
                assert_eq!(delta.metric, LearningMetric::GroundingFailureRate);
                assert!(delta.improvement < 0.0);
            }
            other => panic!("expected critical regression rejection, got {other:?}"),
        }
    }

    #[test]
    fn insufficient_samples_are_rejected() {
        let baseline = run(
            "base-3",
            "insight_prompt",
            3,
            base_observations(0.50, 0.10, 0.05, 5),
        );
        let candidate = run(
            "cand-3",
            "insight_prompt",
            3,
            base_observations(0.90, 0.05, 0.02, 5),
        );

        let decision = evaluate_promotion(&candidate, &baseline, &PromotionGateConfig::default());

        match decision {
            PromotionDecision::Reject(PromotionRejection::InsufficientSamples {
                baseline_samples,
                candidate_samples,
                required,
                ..
            }) => {
                assert_eq!(baseline_samples, 5);
                assert_eq!(candidate_samples, 5);
                assert_eq!(required, 30);
            }
            other => panic!("expected insufficient-sample rejection, got {other:?}"),
        }
    }

    #[test]
    fn small_nonsignificant_change_is_rejected() {
        let baseline = run(
            "base-4",
            "insight_prompt",
            3,
            base_observations(0.60, 0.10, 0.05, 200),
        );
        let candidate = run(
            "cand-4",
            "insight_prompt",
            3,
            base_observations(0.61, 0.10, 0.05, 200),
        );

        let decision = evaluate_promotion(&candidate, &baseline, &PromotionGateConfig::default());

        match decision {
            PromotionDecision::Reject(PromotionRejection::NoSignificantImprovement {
                evaluated_metrics,
            }) => assert_eq!(evaluated_metrics, 6),
            other => panic!("expected no-significant-improvement rejection, got {other:?}"),
        }
    }

    #[test]
    fn improvement_on_different_frozen_versions_is_rejected() {
        let baseline = run(
            "base-5",
            "insight_prompt",
            2,
            base_observations(0.60, 0.10, 0.05, 200),
        );
        let candidate = run(
            "cand-5",
            "insight_prompt",
            3,
            base_observations(0.75, 0.08, 0.04, 200),
        );

        let decision = evaluate_promotion(&candidate, &baseline, &PromotionGateConfig::default());

        assert!(matches!(
            decision,
            PromotionDecision::Reject(PromotionRejection::EvaluationSetMismatch { .. })
        ));
    }

    #[test]
    fn workflow_convenience_metrics_cannot_back_a_promotion() {
        let mut candidate_obs = base_observations(0.75, 0.08, 0.04, 200);
        candidate_obs[0].signal_class = AnalystSignalClass::WorkflowConvenience;
        let baseline = run(
            "base-6",
            "insight_prompt",
            3,
            base_observations(0.60, 0.10, 0.05, 200),
        );
        let candidate = run("cand-6", "insight_prompt", 3, candidate_obs);

        let decision = evaluate_promotion(&candidate, &baseline, &PromotionGateConfig::default());

        match decision {
            PromotionDecision::Reject(PromotionRejection::NonTruthEvidence {
                metric,
                signal_class,
            }) => {
                assert_eq!(metric, LearningMetric::Precision);
                assert_eq!(signal_class, AnalystSignalClass::WorkflowConvenience);
            }
            other => panic!("expected non-truth rejection, got {other:?}"),
        }
    }

    #[test]
    fn raw_action_classification_never_upgrades_convenience_to_truth() {
        assert!(AnalystSignalClass::classify_raw_action("confirmed").is_training_truth());
        assert!(AnalystSignalClass::classify_raw_action("true_positive").is_training_truth());
        assert!(!AnalystSignalClass::classify_raw_action("dismissed").is_training_truth());
        assert!(!AnalystSignalClass::classify_raw_action("false_positive").is_training_truth());
        assert!(!AnalystSignalClass::classify_raw_action("opened").is_training_truth());
        assert!(!AnalystSignalClass::classify_raw_action("bookmarked").is_training_truth());
        // Unknown tokens are conservatively workflow convenience, never truth.
        assert!(!AnalystSignalClass::classify_raw_action("??? unexplored").is_training_truth());
    }

    #[test]
    fn missing_metric_is_rejected() {
        let baseline = run(
            "base-7",
            "insight_prompt",
            3,
            base_observations(0.60, 0.10, 0.05, 200),
        );
        let mut observations = base_observations(0.72, 0.09, 0.04, 200);
        observations.retain(|obs| obs.metric != LearningMetric::DuplicateRate);
        let candidate = run("cand-7", "insight_prompt", 3, observations);

        let decision = evaluate_promotion(&candidate, &baseline, &PromotionGateConfig::default());

        match decision {
            PromotionDecision::Reject(PromotionRejection::MissingMetric { metric }) => {
                assert_eq!(metric, LearningMetric::DuplicateRate);
            }
            other => panic!("expected missing-metric rejection, got {other:?}"),
        }
    }

    #[test]
    fn diagnostic_metrics_do_not_gate_promotions() {
        let baseline = run(
            "base-8",
            "insight_prompt",
            3,
            base_observations(0.60, 0.10, 0.05, 200),
        );
        let mut observations = base_observations(0.72, 0.09, 0.04, 200);
        // Workflow metrics regress badly, but they are not promotion evidence.
        for obs in observations.iter_mut() {
            if obs.metric == LearningMetric::TimeToActionHours {
                obs.value = 96.0;
            }
            if obs.metric == LearningMetric::SourceYield {
                obs.value = 0.05;
            }
        }
        let candidate = run("cand-8", "insight_prompt", 3, observations);

        let decision = evaluate_promotion(&candidate, &baseline, &PromotionGateConfig::default());

        assert!(matches!(decision, PromotionDecision::Promote { .. }));
    }

    #[test]
    fn metric_identifiers_match_persistence_schema() {
        let expected = [
            "precision",
            "false_positive_rate",
            "duplicate_rate",
            "grounding_failure_rate",
            "analyst_acceptance_rate",
            "analyst_dismissal_rate",
            "time_to_action_hours",
            "source_yield",
            "entity_linking_accuracy",
        ];
        for (metric, name) in LearningMetric::ALL.iter().zip(expected) {
            assert_eq!(metric.as_str(), name);
        }
    }
}
