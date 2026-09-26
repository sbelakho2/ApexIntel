//! Frozen-set evaluation and promotion gating for measurable learning (P0 #38).
//!
//! Every proposed rule / model / prompt promotion must be evaluated against a
//! *frozen* evaluation set and may only be promoted when the candidate shows a
//! statistically meaningful improvement and no critical regression.
//!
//! Three rules are enforced structurally here:
//!
//! 1. **Comparisons are same-set and same-version.** A candidate run and its
//!    baseline must reference the identical frozen evaluation set (id, version
//!    and example digest); otherwise the decision is rejected outright.
//! 2. **A frozen set must be verifiable.** The set carries its example count and
//!    the digest of its immutable examples; a run may only execute when the
//!    digest it recorded matches the frozen set digest, and the database
//!    refuses runs whose stored examples do not reproduce `example_count` and
//!    `examples_digest` (migration `061_learning_eval_examples.sql`).
//! 3. **Raw analyst behaviour is not training truth.** Each metric records the
//!    [`AnalystSignalClass`] it was computed from *and* an explicit
//!    [`MetricObservation::is_training_truth`] opt-in. Only
//!    [`AnalystSignalClass::PositiveConfirmation`] rows may set that flag, and
//!    a positive confirmation is never automatically truth; dismissal/noise and
//!    workflow-convenience actions can never back a promotion.
//!
//! The persisted schema lives in `migrations/054_learning_eval_metrics.sql`
//! (`learning_eval_sets`, `learning_eval_runs`, `learning_eval_metrics`) and
//! `migrations/061_learning_eval_examples.sql` (`learning_eval_examples`).

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
/// Only explicit positive confirmation *may* be training truth. Dismissals are
/// real signal but not ground truth (an analyst may dismiss because of workflow
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
    /// Whether this signal class *may* ever back training truth.
    ///
    /// This is an implication, not an equivalence: a positive confirmation is
    /// not automatically truth. Producers must additionally set
    /// [`MetricObservation::is_training_truth`].
    pub const fn may_be_training_truth(self) -> bool {
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
    /// Explicit opt-in that this observation is training truth.
    ///
    /// A positive-confirmation signal class alone is not truth: the producer
    /// must set this flag deliberately. Only confirmation observations may set
    /// it (`learning_eval_metrics` enforces the implication).
    #[serde(default)]
    pub is_training_truth: bool,
    #[serde(default)]
    pub is_critical: bool,
}

impl MetricObservation {
    /// True when the observation may back a promotion: the producer opted in
    /// *and* the signal class permits truth.
    pub const fn is_truth_evidence(&self) -> bool {
        self.is_training_truth && self.signal_class.may_be_training_truth()
    }
}

/// Reference to the frozen evaluation set a run was measured against.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrozenEvalSetRef {
    pub id: String,
    pub name: String,
    pub version: u32,
    pub example_count: u64,
    /// Digest over the immutable examples of the set; empty for unverified or
    /// legacy references.
    #[serde(default)]
    pub examples_digest: String,
}

impl FrozenEvalSetRef {
    /// A set can only be evaluated when it carries a digest and examples.
    pub fn is_verifiable(&self) -> bool {
        !self.examples_digest.is_empty() && self.example_count > 0
    }
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
    /// Digest recorded on the run when it executed. Must equal
    /// [`FrozenEvalSetRef::examples_digest`] for the run to count.
    #[serde(default)]
    pub eval_set_digest: String,
    pub candidate: CandidateRef,
    /// Content hash of the exact candidate artefact that was evaluated.
    #[serde(default)]
    pub candidate_artifact_hash: String,
    /// Content hash of the baseline artefact the candidate was compared against.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline_artifact_hash: Option<String>,
    /// Version label of the baseline artefact.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline_artifact_version: Option<String>,
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
    /// True when the change is statistically significant *in the improvement
    /// direction* (i.e. `improvement > 0`). Regressions are detected separately
    /// by [`is_significant_regression`].
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
    /// The frozen set does not carry verifiable examples/digest.
    UnverifiableEvalSet {
        run_id: String,
        eval_set_id: String,
        example_count: u64,
        digest: String,
    },
    /// The digest recorded on a run does not match the frozen set digest, so
    /// the run cannot be attributed to the frozen examples.
    FrozenSetDigestMismatch {
        run_id: String,
        run_digest: String,
        frozen_digest: String,
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
    /// The metric row was not backed by an explicit training-truth opt-in on
    /// the required analyst-signal class.
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

/// True when the candidate is significantly *worse* in the metric's direction.
///
/// [`MetricDelta::significant`] only covers the improvement direction, so this
/// check is computed independently from the raw z-score.
fn is_significant_regression(
    metric: LearningMetric,
    delta: &MetricDelta,
    significance_z: f64,
) -> bool {
    match delta.z_score {
        Some(z) => match metric.direction() {
            MetricDirection::HigherIsBetter => z <= -significance_z,
            MetricDirection::LowerIsBetter => z >= significance_z,
        },
        None => false,
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
    for run in [candidate, baseline] {
        if !run.eval_set.is_verifiable() {
            return PromotionDecision::Reject(PromotionRejection::UnverifiableEvalSet {
                run_id: run.run_id.clone(),
                eval_set_id: run.eval_set.id.clone(),
                example_count: run.eval_set.example_count,
                digest: run.eval_set.examples_digest.clone(),
            });
        }
        if run.eval_set_digest != run.eval_set.examples_digest {
            return PromotionDecision::Reject(PromotionRejection::FrozenSetDigestMismatch {
                run_id: run.run_id.clone(),
                run_digest: run.eval_set_digest.clone(),
                frozen_digest: run.eval_set.examples_digest.clone(),
            });
        }
    }

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

        // No observation may claim training truth for a class that can never
        // back it, even when the metric itself is diagnostic.
        for observation in [baseline_obs, candidate_obs] {
            if observation.is_training_truth && !observation.signal_class.may_be_training_truth() {
                return PromotionDecision::Reject(PromotionRejection::NonTruthEvidence {
                    metric,
                    signal_class: observation.signal_class,
                });
            }
        }

        if !metric.gates_promotion() {
            continue;
        }

        // A positive confirmation is not automatically training truth: the
        // gating metric only counts when the producer explicitly opted in.
        if !baseline_obs.is_truth_evidence() || !candidate_obs.is_truth_evidence() {
            let offending = if !candidate_obs.is_truth_evidence() {
                candidate_obs.signal_class
            } else {
                baseline_obs.signal_class
            };
            return PromotionDecision::Reject(PromotionRejection::NonTruthEvidence {
                metric,
                signal_class: offending,
            });
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

        if !metric.is_critical() && is_significant_regression(metric, &delta, config.significance_z)
        {
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
    const FROZEN_SET_DIGEST: &str =
        "d3cbc430b00e82d8b9f3db5d82679544f8df3eaed07302a03e0bf0989b9a1a43";

    fn frozen_set(version: u32) -> FrozenEvalSetRef {
        FrozenEvalSetRef {
            id: FROZEN_SET_ID.to_string(),
            name: "insight_quality_golden_set".to_string(),
            version,
            example_count: 500,
            examples_digest: FROZEN_SET_DIGEST.to_string(),
        }
    }

    fn positive(
        metric: LearningMetric,
        value: f64,
        n: u64,
        is_critical: bool,
    ) -> MetricObservation {
        MetricObservation {
            metric,
            value,
            sample_size: n,
            signal_class: AnalystSignalClass::PositiveConfirmation,
            is_training_truth: true,
            is_critical,
        }
    }

    fn base_observations(
        precision: f64,
        fpr: f64,
        grounding: f64,
        n: u64,
    ) -> Vec<MetricObservation> {
        vec![
            positive(LearningMetric::Precision, precision, n, false),
            positive(LearningMetric::FalsePositiveRate, fpr, n, true),
            positive(LearningMetric::DuplicateRate, 0.08, n, false),
            positive(LearningMetric::GroundingFailureRate, grounding, n, true),
            positive(LearningMetric::AnalystAcceptanceRate, 0.55, n, false),
            MetricObservation {
                metric: LearningMetric::AnalystDismissalRate,
                value: 0.30,
                sample_size: n,
                signal_class: AnalystSignalClass::DismissalNoise,
                is_training_truth: false,
                is_critical: false,
            },
            MetricObservation {
                metric: LearningMetric::TimeToActionHours,
                value: 18.0,
                sample_size: n,
                signal_class: AnalystSignalClass::WorkflowConvenience,
                is_training_truth: false,
                is_critical: false,
            },
            MetricObservation {
                metric: LearningMetric::SourceYield,
                value: 0.40,
                sample_size: n,
                signal_class: AnalystSignalClass::WorkflowConvenience,
                is_training_truth: false,
                is_critical: false,
            },
            positive(LearningMetric::EntityLinkingAccuracy, 0.80, n, false),
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
            eval_set_digest: FROZEN_SET_DIGEST.to_string(),
            candidate: CandidateRef {
                kind: CandidateKind::Prompt,
                reference: candidate_ref.to_string(),
                version: None,
            },
            candidate_artifact_hash: format!("sha256:{candidate_ref}"),
            baseline_artifact_hash: Some("sha256:baseline".to_string()),
            baseline_artifact_version: Some("v1".to_string()),
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
        assert!(AnalystSignalClass::classify_raw_action("confirmed").may_be_training_truth());
        assert!(AnalystSignalClass::classify_raw_action("true_positive").may_be_training_truth());
        assert!(!AnalystSignalClass::classify_raw_action("dismissed").may_be_training_truth());
        assert!(!AnalystSignalClass::classify_raw_action("false_positive").may_be_training_truth());
        assert!(!AnalystSignalClass::classify_raw_action("opened").may_be_training_truth());
        assert!(!AnalystSignalClass::classify_raw_action("bookmarked").may_be_training_truth());
        // Unknown tokens are conservatively workflow convenience, never truth.
        assert!(!AnalystSignalClass::classify_raw_action("??? unexplored").may_be_training_truth());
    }

    #[test]
    fn positive_confirmation_is_not_automatically_training_truth() {
        // A confirmation whose producer did not opt in is not evidence.
        let mut observations = base_observations(0.75, 0.08, 0.04, 200);
        observations[0].is_training_truth = false;
        let baseline = run(
            "base-10",
            "insight_prompt",
            3,
            base_observations(0.60, 0.10, 0.05, 200),
        );
        let candidate = run("cand-10", "insight_prompt", 3, observations);

        let decision = evaluate_promotion(&candidate, &baseline, &PromotionGateConfig::default());

        match decision {
            PromotionDecision::Reject(PromotionRejection::NonTruthEvidence {
                metric,
                signal_class,
            }) => {
                assert_eq!(metric, LearningMetric::Precision);
                assert_eq!(signal_class, AnalystSignalClass::PositiveConfirmation);
            }
            other => panic!("expected non-truth rejection, got {other:?}"),
        }
    }

    #[test]
    fn training_truth_cannot_be_set_on_dismissal_or_noise() {
        // The in-memory gate refuses truth claims on non-confirmation classes.
        let mut observations = base_observations(0.75, 0.08, 0.04, 200);
        observations[5].is_training_truth = true;
        assert!(!observations[5].is_truth_evidence());
        let baseline = run(
            "base-11",
            "insight_prompt",
            3,
            base_observations(0.60, 0.10, 0.05, 200),
        );
        let candidate = run("cand-11", "insight_prompt", 3, observations);

        let decision = evaluate_promotion(&candidate, &baseline, &PromotionGateConfig::default());

        match decision {
            PromotionDecision::Reject(PromotionRejection::NonTruthEvidence {
                metric,
                signal_class,
            }) => {
                assert_eq!(metric, LearningMetric::AnalystDismissalRate);
                assert_eq!(signal_class, AnalystSignalClass::DismissalNoise);
            }
            other => panic!("expected non-truth rejection, got {other:?}"),
        }
    }

    #[test]
    fn digest_mismatch_between_run_and_frozen_set_is_refused() {
        let baseline = run(
            "base-12",
            "insight_prompt",
            3,
            base_observations(0.60, 0.10, 0.05, 200),
        );
        let mut candidate = run(
            "cand-12",
            "insight_prompt",
            3,
            base_observations(0.75, 0.08, 0.04, 200),
        );
        candidate.eval_set_digest = "deadbeef".to_string();

        let decision = evaluate_promotion(&candidate, &baseline, &PromotionGateConfig::default());

        match decision {
            PromotionDecision::Reject(PromotionRejection::FrozenSetDigestMismatch {
                run_id,
                run_digest,
                frozen_digest,
            }) => {
                assert_eq!(run_id, "cand-12");
                assert_eq!(run_digest, "deadbeef");
                assert_eq!(frozen_digest, FROZEN_SET_DIGEST);
            }
            other => panic!("expected digest-mismatch rejection, got {other:?}"),
        }
    }

    #[test]
    fn sets_without_examples_or_digest_are_refused() {
        let baseline = run(
            "base-13",
            "insight_prompt",
            3,
            base_observations(0.60, 0.10, 0.05, 200),
        );
        let mut candidate = run(
            "cand-13",
            "insight_prompt",
            3,
            base_observations(0.75, 0.08, 0.04, 200),
        );
        candidate.eval_set.examples_digest = String::new();

        let decision = evaluate_promotion(&candidate, &baseline, &PromotionGateConfig::default());

        match decision {
            PromotionDecision::Reject(PromotionRejection::UnverifiableEvalSet {
                run_id,
                eval_set_id,
                ..
            }) => {
                assert_eq!(run_id, "cand-13");
                assert_eq!(eval_set_id, FROZEN_SET_ID);
            }
            other => panic!("expected unverifiable-set rejection, got {other:?}"),
        }
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
    fn significant_regression_on_noncritical_metric_is_rejected() {
        let baseline = run(
            "base-9",
            "insight_prompt",
            3,
            base_observations(0.70, 0.10, 0.05, 200),
        );
        let mut observations = base_observations(0.82, 0.09, 0.05, 200);
        for obs in observations.iter_mut() {
            if obs.metric == LearningMetric::DuplicateRate {
                obs.value = 0.20;
            }
        }
        let candidate = run("cand-9", "insight_prompt", 3, observations);

        let decision = evaluate_promotion(&candidate, &baseline, &PromotionGateConfig::default());

        match decision {
            PromotionDecision::Reject(PromotionRejection::SignificantRegression { delta }) => {
                assert_eq!(delta.metric, LearningMetric::DuplicateRate);
                assert!(delta.improvement < 0.0);
                // `significant` covers the improvement direction only; the
                // regression is detected from the raw z-score instead.
                assert!(!delta.significant);
            }
            other => panic!("expected significant-regression rejection, got {other:?}"),
        }
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
