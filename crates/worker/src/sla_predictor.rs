//! Proactive SLA breach predictor.
//!
//! When a warning approaches its SLA deadline (e.g., 75% of SLA time
//! has elapsed without acknowledgment), auto-escalate severity and
//! send a reminder webhook.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

// ─── SLA configuration ─────────────────────────────────────────────────

/// SLA deadlines by severity level.
#[derive(Debug, Clone)]
pub struct SlaConfig {
    /// Hours to acknowledge a critical warning
    pub critical_hours: i64,
    /// Hours for high severity
    pub high_hours: i64,
    /// Hours for medium severity
    pub medium_hours: i64,
    /// Hours for low severity
    pub low_hours: i64,
    /// Percentage of SLA elapsed that triggers escalation (0.0–1.0)
    pub escalation_threshold: f64,
    /// Percentage that triggers a reminder (before escalation)
    pub reminder_threshold: f64,
}

impl Default for SlaConfig {
    fn default() -> Self {
        Self {
            critical_hours: 4,
            high_hours: 12,
            medium_hours: 48,
            low_hours: 168, // 7 days
            escalation_threshold: 0.75,
            reminder_threshold: 0.50,
        }
    }
}

impl SlaConfig {
    /// Get the SLA deadline duration for a severity level.
    pub fn deadline_for(&self, severity: &str) -> Duration {
        let hours = match severity {
            "critical" => self.critical_hours,
            "high" => self.high_hours,
            "medium" => self.medium_hours,
            "low" => self.low_hours,
            _ => self.medium_hours,
        };
        Duration::hours(hours)
    }
}

// ─── Warning state ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WarningState {
    pub id: String,
    pub severity: String,
    pub created_at: DateTime<Utc>,
    pub acknowledged: bool,
    pub acknowledged_at: Option<DateTime<Utc>>,
    pub escalated: bool,
    pub escalation_level: i32,
    pub reminder_sent: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SlaStatus {
    pub warning_id: String,
    pub severity: String,
    pub sla_deadline: DateTime<Utc>,
    pub elapsed_pct: f64,
    pub time_remaining: Duration,
    pub action: SlaAction,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub enum SlaAction {
    /// Within SLA, no action needed.
    Ok,
    /// Approaching SLA, send reminder.
    SendReminder,
    /// SLA threshold exceeded, escalate severity.
    Escalate { new_severity: String },
    /// SLA breached, maximum escalation.
    Breached,
    /// Already acknowledged, no action.
    Acknowledged,
}

// ─── Predictor ──────────────────────────────────────────────────────────

pub struct SlaPredictor {
    config: SlaConfig,
}

impl SlaPredictor {
    pub fn new(config: SlaConfig) -> Self {
        Self { config }
    }

    pub fn with_defaults() -> Self {
        Self::new(SlaConfig::default())
    }

    /// Evaluate the SLA status of a single warning.
    pub fn evaluate(&self, warning: &WarningState, now: DateTime<Utc>) -> SlaStatus {
        let deadline_duration = self.config.deadline_for(&warning.severity);
        let sla_deadline = warning.created_at + deadline_duration;
        let elapsed = now - warning.created_at;
        let elapsed_pct = if deadline_duration.num_seconds() > 0 {
            elapsed.num_seconds() as f64 / deadline_duration.num_seconds() as f64
        } else {
            1.0
        };
        let time_remaining = if sla_deadline > now {
            sla_deadline - now
        } else {
            Duration::zero()
        };

        let action = if warning.acknowledged {
            SlaAction::Acknowledged
        } else if elapsed_pct >= 1.0 {
            SlaAction::Breached
        } else if elapsed_pct >= self.config.escalation_threshold {
            let new_severity = escalate_severity(&warning.severity);
            SlaAction::Escalate { new_severity }
        } else if elapsed_pct >= self.config.reminder_threshold {
            SlaAction::SendReminder
        } else {
            SlaAction::Ok
        };

        SlaStatus {
            warning_id: warning.id.clone(),
            severity: warning.severity.clone(),
            sla_deadline,
            elapsed_pct,
            time_remaining,
            action,
        }
    }

    /// Batch-evaluate all unacknowledged warnings.
    pub fn evaluate_batch(&self, warnings: &[WarningState], now: DateTime<Utc>) -> Vec<SlaStatus> {
        warnings
            .iter()
            .filter(|w| !w.acknowledged)
            .map(|w| self.evaluate(w, now))
            .collect()
    }

    /// Get only warnings that need action (reminder or escalation).
    pub fn actionable(&self, warnings: &[WarningState], now: DateTime<Utc>) -> Vec<SlaStatus> {
        self.evaluate_batch(warnings, now)
            .into_iter()
            .filter(|s| !matches!(s.action, SlaAction::Ok | SlaAction::Acknowledged))
            .collect()
    }
}

/// Escalate severity one level up.
fn escalate_severity(current: &str) -> String {
    match current {
        "low" => "medium".into(),
        "medium" => "high".into(),
        "high" => "critical".into(),
        "critical" => "critical".into(), // already max
        other => other.into(),
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_warning(severity: &str, hours_ago: i64, acknowledged: bool) -> WarningState {
        WarningState {
            id: format!("w-{}", severity),
            severity: severity.into(),
            created_at: Utc::now() - Duration::hours(hours_ago),
            acknowledged,
            acknowledged_at: if acknowledged { Some(Utc::now()) } else { None },
            escalated: false,
            escalation_level: 0,
            reminder_sent: false,
        }
    }

    #[test]
    fn test_sla_ok() {
        let predictor = SlaPredictor::with_defaults();
        let w = make_warning("critical", 1, false); // 1h into 4h SLA = 25%
        let status = predictor.evaluate(&w, Utc::now());
        assert_eq!(status.action, SlaAction::Ok);
        assert!(status.elapsed_pct < 0.5);
    }

    #[test]
    fn test_sla_reminder() {
        let predictor = SlaPredictor::with_defaults();
        let w = make_warning("critical", 2, false); // 2h into 4h SLA = 50%
        let status = predictor.evaluate(&w, Utc::now());
        assert_eq!(status.action, SlaAction::SendReminder);
    }

    #[test]
    fn test_sla_escalation() {
        let predictor = SlaPredictor::with_defaults();
        let w = make_warning("high", 10, false); // 10h into 12h SLA ≈ 83%
        let status = predictor.evaluate(&w, Utc::now());
        assert!(matches!(status.action, SlaAction::Escalate { .. }));
        if let SlaAction::Escalate { new_severity } = &status.action {
            assert_eq!(new_severity, "critical");
        }
    }

    #[test]
    fn test_sla_breached() {
        let predictor = SlaPredictor::with_defaults();
        let w = make_warning("medium", 50, false); // 50h into 48h SLA
        let status = predictor.evaluate(&w, Utc::now());
        assert_eq!(status.action, SlaAction::Breached);
    }

    #[test]
    fn test_acknowledged_no_action() {
        let predictor = SlaPredictor::with_defaults();
        let w = make_warning("critical", 5, true); // past SLA but acknowledged
        let status = predictor.evaluate(&w, Utc::now());
        assert_eq!(status.action, SlaAction::Acknowledged);
    }

    #[test]
    fn test_batch_actionable() {
        let predictor = SlaPredictor::with_defaults();
        let warnings = vec![
            make_warning("critical", 1, false), // ok
            make_warning("critical", 3, false), // needs escalation
            make_warning("high", 6, false),     // reminder
            make_warning("medium", 50, false),  // breached
            make_warning("low", 100, true),     // acknowledged
        ];
        let actionable = predictor.actionable(&warnings, Utc::now());
        assert_eq!(actionable.len(), 3); // escalation + reminder + breached
    }

    #[test]
    fn test_escalate_severity() {
        assert_eq!(escalate_severity("low"), "medium");
        assert_eq!(escalate_severity("medium"), "high");
        assert_eq!(escalate_severity("high"), "critical");
        assert_eq!(escalate_severity("critical"), "critical");
    }
}
