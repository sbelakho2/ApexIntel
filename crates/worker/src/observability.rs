// Used by both `#[cfg(feature = "llm")]` and `#[cfg(not(feature = "llm"))]` code paths.
#![allow(dead_code)]

//! Observability infrastructure for quality score tracking and gate decisions.
//! This module provides:
//! - Quality score breakdown logging
//! - Structured gate decision logging
//! - Helpers for eventual Prometheus metrics export

use std::sync::{
    atomic::{AtomicU64, Ordering},
    RwLock, RwLockReadGuard, RwLockWriteGuard,
};
use std::time::Instant;
use uuid::Uuid;

fn read_lock<T>(lock: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    lock.read().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn write_lock<T>(lock: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    lock.write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Quality score breakdown record for logging and analysis.
#[derive(Debug, Clone)]
pub struct QualityScoreBreakdown {
    pub observation_id: Uuid,
    pub source_score: f64,
    pub confidence_score: f64,
    pub freshness_score: f64,
    pub final_score: f64,
    pub entity_id: Option<Uuid>,
    pub category: Option<String>,
}

impl QualityScoreBreakdown {
    /// Log the quality score breakdown using structured logging.
    pub fn log(&self) {
        tracing::info!(
            observation_id = %self.observation_id,
            source_score = %self.source_score,
            confidence_score = %self.confidence_score,
            freshness_score = %self.freshness_score,
            final_score = %self.final_score,
            entity_id = ?self.entity_id,
            category = ?self.category,
            "quality_score_breakdown"
        );
    }
}

/// Gate decision record for tracking quality gate outcomes.
#[derive(Debug, Clone)]
pub struct GateDecisionRecord {
    pub gate_name: String,
    pub input_hash: String,
    pub score: f64,
    pub threshold: f64,
    pub decision: GateDecision,
    pub veto: bool,
    pub latency: std::time::Duration,
    pub entity_id: Option<Uuid>,
    pub category: Option<String>,
    pub attempt: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateDecision {
    Pass,
    Fail,
    SoftFail,
}

impl std::fmt::Display for GateDecision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GateDecision::Pass => write!(f, "pass"),
            GateDecision::Fail => write!(f, "fail"),
            GateDecision::SoftFail => write!(f, "soft_fail"),
        }
    }
}

impl GateDecisionRecord {
    /// Log the gate decision using structured logging.
    pub fn log(&self) {
        tracing::info!(
            gate_name = %self.gate_name,
            input_hash = %self.input_hash,
            score = %self.score,
            threshold = %self.threshold,
            decision = %self.decision,
            veto = %self.veto,
            latency_ms = %self.latency.as_millis(),
            entity_id = ?self.entity_id,
            category = ?self.category,
            attempt = %self.attempt,
            "gate_decision"
        );
    }
}

/// Timer for measuring gate evaluation latency.
pub struct GateTimer {
    start: Instant,
    gate_name: String,
}

impl GateTimer {
    pub fn start(gate_name: impl Into<String>) -> Self {
        Self {
            start: Instant::now(),
            gate_name: gate_name.into(),
        }
    }

    pub fn elapsed(&self) -> std::time::Duration {
        self.start.elapsed()
    }

    pub fn finish(
        self,
        score: f64,
        threshold: f64,
        passed: bool,
        veto: bool,
    ) -> GateDecisionRecord {
        let latency = self.start.elapsed();
        GateDecisionRecord {
            gate_name: self.gate_name,
            input_hash: String::new(), // Can be filled in by caller
            score,
            threshold,
            decision: if passed {
                GateDecision::Pass
            } else if veto {
                GateDecision::Fail
            } else {
                GateDecision::SoftFail
            },
            veto,
            latency,
            entity_id: None,
            category: None,
            attempt: 1,
        }
    }
}

/// In-memory metrics counters for Prometheus export.
/// These can be scraped by a metrics endpoint.
pub struct WorkerMetrics {
    /// Total gate evaluations by gate name
    gate_evaluations: std::sync::RwLock<std::collections::HashMap<String, AtomicU64>>,
    /// Gate failures by gate name
    gate_failures: std::sync::RwLock<std::collections::HashMap<String, AtomicU64>>,
    /// LLM retry counts
    llm_retries: AtomicU64,
    /// LLM successes
    llm_successes: AtomicU64,
    /// LLM failures
    llm_failures: AtomicU64,
    /// Insights accepted
    insights_accepted: AtomicU64,
    /// Insights rejected
    insights_rejected: AtomicU64,
    /// Insights with fallback
    insights_fallback: AtomicU64,
    /// Notification deliveries claimed by the retry processor
    notification_deliveries_claimed: AtomicU64,
    /// Notification deliveries accepted by their channel
    notification_deliveries_delivered: AtomicU64,
    /// Notification deliveries rescheduled with backoff
    notification_deliveries_retried: AtomicU64,
    /// Notification deliveries moved to the terminal dead-letter state
    notification_deliveries_dead_lettered: AtomicU64,
    /// Outbox alert events moved to the terminal dead-letter state
    outbox_events_dead_lettered: AtomicU64,
    /// LLM self-improvement stage failures by stage name (a failed stage is
    /// never reported as "no findings").
    self_improvement_stage_failures:
        std::sync::RwLock<std::collections::HashMap<String, AtomicU64>>,
    /// Self-improvement cycles where a metric had nothing to evaluate
    /// (`NotMeasured`), which must never be read as a bad score.
    self_improvement_not_evaluated: AtomicU64,
}

impl Default for WorkerMetrics {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkerMetrics {
    pub fn new() -> Self {
        Self {
            gate_evaluations: std::sync::RwLock::new(std::collections::HashMap::new()),
            gate_failures: std::sync::RwLock::new(std::collections::HashMap::new()),
            llm_retries: AtomicU64::new(0),
            llm_successes: AtomicU64::new(0),
            llm_failures: AtomicU64::new(0),
            insights_accepted: AtomicU64::new(0),
            insights_rejected: AtomicU64::new(0),
            insights_fallback: AtomicU64::new(0),
            notification_deliveries_claimed: AtomicU64::new(0),
            notification_deliveries_delivered: AtomicU64::new(0),
            notification_deliveries_retried: AtomicU64::new(0),
            notification_deliveries_dead_lettered: AtomicU64::new(0),
            outbox_events_dead_lettered: AtomicU64::new(0),
            self_improvement_stage_failures: std::sync::RwLock::new(
                std::collections::HashMap::new(),
            ),
            self_improvement_not_evaluated: AtomicU64::new(0),
        }
    }

    pub fn record_gate_evaluation(&self, gate_name: &str, passed: bool) {
        // Increment evaluation count
        {
            let mut evals = write_lock(&self.gate_evaluations);
            evals
                .entry(gate_name.to_string())
                .or_insert_with(|| AtomicU64::new(0))
                .fetch_add(1, Ordering::Relaxed);
        }

        // Increment failure count if failed
        if !passed {
            let mut fails = write_lock(&self.gate_failures);
            fails
                .entry(gate_name.to_string())
                .or_insert_with(|| AtomicU64::new(0))
                .fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn record_llm_retry(&self) {
        self.llm_retries.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_llm_success(&self) {
        self.llm_successes.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_llm_failure(&self) {
        self.llm_failures.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_insight_accepted(&self) {
        self.insights_accepted.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_insight_rejected(&self) {
        self.insights_rejected.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_insight_fallback(&self) {
        self.insights_fallback.fetch_add(1, Ordering::Relaxed);
    }

    /// Record one retry-processor cycle's outcome counters.
    pub fn record_notification_delivery_cycle(
        &self,
        outcome: &apex_worker::notification_delivery::DeliveryCycleOutcome,
    ) {
        self.notification_deliveries_claimed
            .fetch_add(outcome.claimed as u64, Ordering::Relaxed);
        self.notification_deliveries_delivered
            .fetch_add(outcome.delivered as u64, Ordering::Relaxed);
        self.notification_deliveries_retried
            .fetch_add(outcome.retried as u64, Ordering::Relaxed);
        self.notification_deliveries_dead_lettered
            .fetch_add(outcome.dead_lettered as u64, Ordering::Relaxed);
    }

    /// Record an outbox alert event that exhausted its publish attempts.
    pub fn record_outbox_dead_lettered(&self) {
        self.outbox_events_dead_lettered
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Record one failed self-improvement stage (critique, prompt
    /// improvements, failure hypotheses, ...).
    pub fn record_self_improvement_stage_failure(&self, stage_name: &str) {
        let mut failures = write_lock(&self.self_improvement_stage_failures);
        failures
            .entry(stage_name.to_string())
            .or_insert_with(|| AtomicU64::new(0))
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Record a self-improvement metric that had nothing to evaluate. This is
    /// deliberately separate from a failure and from a zero score.
    pub fn record_self_improvement_not_evaluated(&self) {
        self.self_improvement_not_evaluated
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Observed stage-failure count for one self-improvement stage.
    pub fn self_improvement_stage_failure_count(&self, stage_name: &str) -> u64 {
        read_lock(&self.self_improvement_stage_failures)
            .get(stage_name)
            .map_or(0, |count| count.load(Ordering::Relaxed))
    }

    /// Number of self-improvement cycles where a metric was not evaluated.
    pub fn self_improvement_not_evaluated_count(&self) -> u64 {
        self.self_improvement_not_evaluated.load(Ordering::Relaxed)
    }

    /// Get gate fire rate (failures / evaluations) for a specific gate.
    pub fn gate_fire_rate(&self, gate_name: &str) -> Option<f64> {
        let evals = read_lock(&self.gate_evaluations);
        let fails = read_lock(&self.gate_failures);

        let eval_count = evals.get(gate_name).map(|a| a.load(Ordering::Relaxed))?;
        let fail_count = fails
            .get(gate_name)
            .map(|a| a.load(Ordering::Relaxed))
            .unwrap_or(0);

        if eval_count == 0 {
            return None;
        }

        Some(fail_count as f64 / eval_count as f64)
    }

    /// Get LLM retry rate.
    pub fn llm_retry_rate(&self) -> f64 {
        let total =
            self.llm_successes.load(Ordering::Relaxed) + self.llm_failures.load(Ordering::Relaxed);
        if total == 0 {
            return 0.0;
        }
        self.llm_retries.load(Ordering::Relaxed) as f64 / total as f64
    }

    /// Get insight acceptance ratio.
    pub fn insight_acceptance_ratio(&self) -> f64 {
        let total = self.insights_accepted.load(Ordering::Relaxed)
            + self.insights_rejected.load(Ordering::Relaxed)
            + self.insights_fallback.load(Ordering::Relaxed);
        if total == 0 {
            return 0.0;
        }
        self.insights_accepted.load(Ordering::Relaxed) as f64 / total as f64
    }

    /// Export metrics in Prometheus text format.
    /// This can be used by a /metrics endpoint.
    pub fn to_prometheus_text(&self) -> String {
        let mut output = String::new();

        // LLM metrics
        output.push_str("# HELP apexintel_worker_llm_retries_total Total LLM retry attempts\n");
        output.push_str("# TYPE apexintel_worker_llm_retries_total counter\n");
        output.push_str(&format!(
            "apexintel_worker_llm_retries_total {}\n",
            self.llm_retries.load(Ordering::Relaxed)
        ));

        output.push_str(
            "# HELP apexintel_worker_llm_successes_total Total LLM successful completions\n",
        );
        output.push_str("# TYPE apexintel_worker_llm_successes_total counter\n");
        output.push_str(&format!(
            "apexintel_worker_llm_successes_total {}\n",
            self.llm_successes.load(Ordering::Relaxed)
        ));

        output.push_str("# HELP apexintel_worker_llm_failures_total Total LLM failures\n");
        output.push_str("# TYPE apexintel_worker_llm_failures_total counter\n");
        output.push_str(&format!(
            "apexintel_worker_llm_failures_total {}\n",
            self.llm_failures.load(Ordering::Relaxed)
        ));

        // Insight metrics
        output
            .push_str("# HELP apexintel_worker_insights_accepted_total Total insights accepted\n");
        output.push_str("# TYPE apexintel_worker_insights_accepted_total counter\n");
        output.push_str(&format!(
            "apexintel_worker_insights_accepted_total {}\n",
            self.insights_accepted.load(Ordering::Relaxed)
        ));

        output
            .push_str("# HELP apexintel_worker_insights_rejected_total Total insights rejected\n");
        output.push_str("# TYPE apexintel_worker_insights_rejected_total counter\n");
        output.push_str(&format!(
            "apexintel_worker_insights_rejected_total {}\n",
            self.insights_rejected.load(Ordering::Relaxed)
        ));

        output.push_str(
            "# HELP apexintel_worker_insights_fallback_total Total insights using fallback\n",
        );
        output.push_str("# TYPE apexintel_worker_insights_fallback_total counter\n");
        output.push_str(&format!(
            "apexintel_worker_insights_fallback_total {}\n",
            self.insights_fallback.load(Ordering::Relaxed)
        ));

        // Notification delivery metrics (durable retry processor + outbox).
        output.push_str(
            "# HELP apexintel_worker_notification_deliveries_claimed_total Notification deliveries claimed by the retry processor\n",
        );
        output.push_str("# TYPE apexintel_worker_notification_deliveries_claimed_total counter\n");
        output.push_str(&format!(
            "apexintel_worker_notification_deliveries_claimed_total {}\n",
            self.notification_deliveries_claimed.load(Ordering::Relaxed)
        ));
        output.push_str(
            "# HELP apexintel_worker_notification_deliveries_delivered_total Notification deliveries accepted by their channel\n",
        );
        output
            .push_str("# TYPE apexintel_worker_notification_deliveries_delivered_total counter\n");
        output.push_str(&format!(
            "apexintel_worker_notification_deliveries_delivered_total {}\n",
            self.notification_deliveries_delivered
                .load(Ordering::Relaxed)
        ));
        output.push_str(
            "# HELP apexintel_worker_notification_deliveries_retried_total Notification deliveries rescheduled with backoff\n",
        );
        output.push_str("# TYPE apexintel_worker_notification_deliveries_retried_total counter\n");
        output.push_str(&format!(
            "apexintel_worker_notification_deliveries_retried_total {}\n",
            self.notification_deliveries_retried.load(Ordering::Relaxed)
        ));
        output.push_str(
            "# HELP apexintel_worker_notification_deliveries_dead_lettered_total Notification deliveries moved to the terminal dead-letter state\n",
        );
        output.push_str(
            "# TYPE apexintel_worker_notification_deliveries_dead_lettered_total counter\n",
        );
        output.push_str(&format!(
            "apexintel_worker_notification_deliveries_dead_lettered_total {}\n",
            self.notification_deliveries_dead_lettered
                .load(Ordering::Relaxed)
        ));
        output.push_str(
            "# HELP apexintel_worker_outbox_events_dead_lettered_total Outbox alert events moved to the terminal dead-letter state\n",
        );
        output.push_str("# TYPE apexintel_worker_outbox_events_dead_lettered_total counter\n");
        output.push_str(&format!(
            "apexintel_worker_outbox_events_dead_lettered_total {}\n",
            self.outbox_events_dead_lettered.load(Ordering::Relaxed)
        ));

        // Self-improvement truth semantics: failed stages are counted
        // separately from stages that had nothing to evaluate.
        output.push_str(
            "# HELP apexintel_worker_self_improvement_stage_failures_total Failed self-improvement stages by stage name\n",
        );
        output.push_str("# TYPE apexintel_worker_self_improvement_stage_failures_total counter\n");
        {
            let failures = read_lock(&self.self_improvement_stage_failures);
            for (stage_name, count) in failures.iter() {
                output.push_str(&format!(
                    "apexintel_worker_self_improvement_stage_failures_total{{stage=\"{}\"}} {}\n",
                    stage_name,
                    count.load(Ordering::Relaxed)
                ));
            }
        }
        output.push_str(
            "# HELP apexintel_worker_self_improvement_not_evaluated_total Self-improvement metrics with nothing to evaluate\n",
        );
        output.push_str("# TYPE apexintel_worker_self_improvement_not_evaluated_total counter\n");
        output.push_str(&format!(
            "apexintel_worker_self_improvement_not_evaluated_total {}\n",
            self.self_improvement_not_evaluated.load(Ordering::Relaxed)
        ));

        // Gate metrics
        output.push_str(
            "# HELP apexintel_worker_gate_evaluations_total Gate evaluations by gate name\n",
        );
        output.push_str("# TYPE apexintel_worker_gate_evaluations_total counter\n");
        {
            let evals = read_lock(&self.gate_evaluations);
            for (gate_name, count) in evals.iter() {
                output.push_str(&format!(
                    "apexintel_worker_gate_evaluations_total{{gate=\"{}\"}} {}\n",
                    gate_name,
                    count.load(Ordering::Relaxed)
                ));
            }
        }

        output.push_str("# HELP apexintel_worker_gate_failures_total Gate failures by gate name\n");
        output.push_str("# TYPE apexintel_worker_gate_failures_total counter\n");
        {
            let fails = read_lock(&self.gate_failures);
            for (gate_name, count) in fails.iter() {
                output.push_str(&format!(
                    "apexintel_worker_gate_failures_total{{gate=\"{}\"}} {}\n",
                    gate_name,
                    count.load(Ordering::Relaxed)
                ));
            }
        }

        output
    }
}

/// Global worker metrics instance.
/// Use `WORKER_METRICS.record_*` methods to record metrics.
pub static WORKER_METRICS: std::sync::LazyLock<WorkerMetrics> =
    std::sync::LazyLock::new(WorkerMetrics::new);

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::field_reassign_with_default
    )]

    use super::*;

    #[test]
    fn gate_timer_measures_latency() {
        let timer = GateTimer::start("test_gate");
        std::thread::sleep(std::time::Duration::from_millis(10));
        let record = timer.finish(0.8, 0.5, true, false);
        assert!(record.latency.as_millis() >= 10);
        assert_eq!(record.gate_name, "test_gate");
        assert_eq!(record.decision, GateDecision::Pass);
    }

    #[test]
    fn metrics_tracks_gate_evaluations() {
        let metrics = WorkerMetrics::new();
        metrics.record_gate_evaluation("test_gate", true);
        metrics.record_gate_evaluation("test_gate", false);
        metrics.record_gate_evaluation("test_gate", true);

        let rate = metrics.gate_fire_rate("test_gate").unwrap();
        assert!((rate - 1.0 / 3.0).abs() < 0.01);
    }

    #[test]
    fn metrics_tracks_llm_retries() {
        let metrics = WorkerMetrics::new();
        metrics.record_llm_retry();
        metrics.record_llm_retry();
        metrics.record_llm_success();

        let rate = metrics.llm_retry_rate();
        assert!((rate - 2.0).abs() < 0.01);
    }

    #[test]
    fn prometheus_output_is_valid() {
        let metrics = WorkerMetrics::new();
        metrics.record_insight_accepted();
        metrics.record_insight_rejected();

        let output = metrics.to_prometheus_text();
        assert!(output.contains("apexintel_worker_insights_accepted_total 1"));
        assert!(output.contains("apexintel_worker_insights_rejected_total 1"));
    }

    #[test]
    fn stage_failures_are_distinct_from_not_evaluated() {
        let metrics = WorkerMetrics::new();
        metrics.record_self_improvement_stage_failure("critique");
        metrics.record_self_improvement_not_evaluated();

        assert_eq!(metrics.self_improvement_stage_failure_count("critique"), 1);
        assert_eq!(metrics.self_improvement_stage_failure_count("absent"), 0);
        assert_eq!(metrics.self_improvement_not_evaluated_count(), 1);

        let output = metrics.to_prometheus_text();
        assert!(output.contains(
            "apexintel_worker_self_improvement_stage_failures_total{stage=\"critique\"} 1"
        ));
        assert!(output.contains("apexintel_worker_self_improvement_not_evaluated_total 1"));
    }
}
