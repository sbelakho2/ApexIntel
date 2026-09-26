//! Alert evaluator — subscribes to domain events, evaluates them against
//! configured alert rules, and publishes `AlertEvent`s to NATS JetStream.
//!
//! # Architecture
//!
//! 1. Load rules from `config/runtime/alert-rules.yaml` at startup.
//! 2. Accept domain events (new insights, warnings, system metrics) via
//!    [`AlertEvaluator::evaluate`].
//! 3. For each event, check every rule whose `source` matches the event type.
//! 4. If the condition is satisfied, produce an [`AlertEvent`] and publish it.
//! 5. Deduplicate: prevent the same (rule, entity) pair from re-firing within
//!    a configurable cooldown window (default 5 minutes).
//!
//! # Integration
//!
//! The transactional outbox publisher (`crate::alert_pipeline`) calls
//! [`AlertEvaluator::evaluate`] for each committed warning event and awaits the
//! JetStream ACK for every rule-derived alert before stamping the outbox row
//! published.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::info;
use uuid::Uuid;

use apex_worker::nats_stream::{AlertEvent, AlertEventType};

// ─────────────────────────────────────────────────────────────────────────────
// Rule types — mirrors `config/runtime/alert-rules.yaml`
// ─────────────────────────────────────────────────────────────────────────────

/// Top-level alert rules configuration file.
#[derive(Debug, Clone, Deserialize)]
pub struct AlertRulesConfig {
    pub version: u32,
    pub routes: Option<AlertRoutes>,
    pub rules: Vec<AlertRule>,
}

/// Routing destinations for fired alerts.
#[derive(Debug, Clone, Deserialize)]
pub struct AlertRoutes {
    pub pager_webhook_env: Option<String>,
    pub warning_email_env: Option<String>,
}

/// A single alert rule.
#[derive(Debug, Clone, Deserialize)]
pub struct AlertRule {
    pub name: String,
    /// The source metric/event type this rule watches (e.g. `warning_count`,
    /// `api_health_deep`, `crawl_success_total`).
    pub source: String,
    /// A condition expression (PromQL-like for metrics, or simple comparison
    /// for events). The evaluator parses this per source type.
    pub condition: String,
    /// Severity of the fired alert: `critical`, `warning`, `info`.
    pub severity: String,
    /// How long the condition must hold before firing (human-readable, e.g.
    /// `2m`, `5m`). For event-based rules this is the cooldown duration.
    #[serde(default = "default_for")]
    pub r#for: String,
    /// Notification destinations.
    pub notify: Vec<String>,
}

fn default_for() -> String {
    "5m".to_string()
}

/// Parse a human-readable duration like `2m`, `30s`, `1h` into [`Duration`].
fn parse_duration(input: &str) -> Duration {
    let input = input.trim();
    if let Some(secs) = input.strip_suffix('s') {
        if let Ok(n) = secs.parse::<u64>() {
            return Duration::from_secs(n);
        }
    }
    if let Some(mins) = input.strip_suffix('m') {
        if let Ok(n) = mins.parse::<u64>() {
            return Duration::from_secs(n * 60);
        }
    }
    if let Some(hours) = input.strip_suffix('h') {
        if let Ok(n) = hours.parse::<u64>() {
            return Duration::from_secs(n * 3600);
        }
    }
    // Default fallback
    Duration::from_secs(300)
}

// ─────────────────────────────────────────────────────────────────────────────
// Domain event types the evaluator can process
// ─────────────────────────────────────────────────────────────────────────────

/// A domain event that may trigger alert rules.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainEvent {
    /// Unique event identifier.
    pub id: Uuid,
    /// The source type matches `rule.source` for routing.
    pub source: String,
    /// Event severity string (`critical`, `warning`, `info`).
    pub severity: String,
    /// Human-readable title.
    pub title: String,
    /// Detailed description.
    pub description: String,
    /// Optional entity this event relates to.
    pub entity_id: Option<Uuid>,
    /// Optional entity name.
    pub entity_name: Option<String>,
    /// Free-form metadata for rule evaluation.
    pub metadata: serde_json::Value,
    /// Timestamp of the event.
    pub created_at: DateTime<Utc>,
}

impl DomainEvent {
    /// Create a new domain event from an insight or warning.
    pub fn new(
        source: impl Into<String>,
        severity: impl Into<String>,
        title: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            source: source.into(),
            severity: severity.into(),
            title: title.into(),
            description: description.into(),
            entity_id: None,
            entity_name: None,
            metadata: serde_json::Value::Object(Default::default()),
            created_at: Utc::now(),
        }
    }

    /// Attach entity information.
    pub fn with_entity(mut self, id: Uuid, name: impl Into<String>) -> Self {
        self.entity_id = Some(id);
        self.entity_name = Some(name.into());
        self
    }

    /// Attach custom metadata.
    pub fn with_metadata(mut self, meta: serde_json::Value) -> Self {
        self.metadata = meta;
        self
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Dedup key
// ─────────────────────────────────────────────────────────────────────────────

/// Unique key for deduplication: (rule_name, entity_id).
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct DedupKey {
    rule_name: String,
    entity_id: Option<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// AlertEvaluator
// ─────────────────────────────────────────────────────────────────────────────

/// Evaluates domain events against alert rules and publishes fired alerts.
pub struct AlertEvaluator {
    /// Loaded alert rules.
    rules: Vec<AlertRule>,
    /// Dedup cache: maps (rule, entity) → last fired timestamp.
    dedup: Arc<RwLock<HashMap<DedupKey, Instant>>>,
    /// Cooldown duration for dedup (parsed from rule `for` field).
    cooldown: Duration,
}

impl AlertEvaluator {
    /// Load alert rules from a YAML file.
    pub fn load_from_path(path: impl AsRef<Path>) -> Result<Self> {
        let content =
            std::fs::read_to_string(path.as_ref()).context("failed to read alert-rules.yaml")?;
        Self::load_from_yaml(&content)
    }

    /// Load alert rules from a YAML string.
    pub fn load_from_yaml(yaml: &str) -> Result<Self> {
        let config: AlertRulesConfig =
            serde_yaml::from_str(yaml).context("failed to parse alert-rules.yaml")?;

        info!(
            "Loaded {} alert rules (v{})",
            config.rules.len(),
            config.version
        );

        Ok(Self {
            rules: config.rules,
            dedup: Arc::new(RwLock::new(HashMap::new())),
            cooldown: Duration::from_secs(300), // default 5 min
        })
    }

    /// Set a custom cooldown duration (overrides rule `for` field).
    pub fn with_cooldown(mut self, cooldown: Duration) -> Self {
        self.cooldown = cooldown;
        self
    }

    /// Evaluate a domain event against all loaded rules.
    ///
    /// Returns a list of [`AlertEvent`]s that should be fired (i.e. rules whose
    /// conditions were met and whose dedup window has passed).
    pub async fn evaluate(&self, event: &DomainEvent) -> Vec<AlertEvent> {
        let mut fired = Vec::new();

        for rule in &self.rules {
            // Source matching: accept if the rule source matches or is a prefix
            if !self.source_matches(&rule.source, &event.source) {
                continue;
            }

            // Severity/condition check
            if !self.condition_satisfied(rule, event) {
                continue;
            }

            // Dedup check
            let dedup_key = DedupKey {
                rule_name: rule.name.clone(),
                entity_id: event.entity_id.map(|id| id.to_string()),
            };

            let mut dedup = self.dedup.write().await;
            let rule_cooldown = parse_duration(&rule.r#for);
            let should_fire = match dedup.get(&dedup_key) {
                Some(last_fired) if last_fired.elapsed() < rule_cooldown => {
                    false // Still in cooldown
                }
                _ => true,
            };

            if !should_fire {
                continue;
            }

            // Record firing time
            dedup.insert(dedup_key.clone(), Instant::now());

            // Build severity
            let alert_severity = match rule.severity.to_lowercase().as_str() {
                "critical" => apex_core::alert_config::AlertSeverity::Critical,
                "warning" => apex_core::alert_config::AlertSeverity::Medium,
                _ => apex_core::alert_config::AlertSeverity::Info,
            };

            // Map source to AlertEventType
            let event_type = self.map_source_to_event_type(&rule.source);

            // Build AlertEvent
            let alert = AlertEvent {
                id: Uuid::new_v4(),
                event_type,
                severity: alert_severity,
                title: format!("[{}] {}", rule.name, event.title),
                description: format!(
                    "Rule: {}\nCondition: {}\nEvent: {}\n{}",
                    rule.name, rule.condition, event.title, event.description
                ),
                entity_id: event.entity_id,
                entity_name: event.entity_name.clone(),
                // Rule firings are not system-wide: leave the audience empty so
                // the API router resolves the entity's real subscribers. An
                // unresolved alert reaches nobody — only deliberate
                // system-wide alerts use `AlertAudience::Broadcast`.
                audience: apex_core::alert_config::AlertAudience::Users(vec![]),
                metadata: serde_json::json!({
                    "rule_name": rule.name,
                    "rule_source": rule.source,
                    "rule_severity": rule.severity,
                    "event_id": event.id,
                    "event_source": event.source,
                    "notify": rule.notify,
                }),
                created_at: Utc::now(),
            };

            fired.push(alert);
        }

        fired
    }

    /// Release the dedup/cooldown entries a previous [`Self::evaluate`] recorded
    /// for this event.
    ///
    /// The outbox publisher calls this when a rule-derived alert failed to
    /// publish: the cooldown was already recorded before the publish, and
    /// without releasing it the retry would silently suppress the rule alert.
    /// Releasing may re-deliver rule alerts that were published before the
    /// failure in the same batch; at-least-once beats silent loss.
    pub async fn forget_firings(&self, event: &DomainEvent) {
        let entity_id = event.entity_id.map(|id| id.to_string());
        let mut dedup = self.dedup.write().await;
        for rule in &self.rules {
            if !self.source_matches(&rule.source, &event.source) {
                continue;
            }
            dedup.remove(&DedupKey {
                rule_name: rule.name.clone(),
                entity_id: entity_id.clone(),
            });
        }
    }

    // ─── Internal helpers ─────────────────────────────────────────────────

    /// Check if a rule source matches an event source.
    fn source_matches(&self, rule_source: &str, event_source: &str) -> bool {
        rule_source == event_source || event_source.starts_with(rule_source) || rule_source == "*"
    }

    /// Check whether a rule's condition is satisfied for the given event.
    ///
    /// For event-based rules, this maps to:
    /// - Severity-based: condition like `severity >= warning`
    /// - Simple equality: condition like `status != ok`
    /// - Always match (if condition is empty or just "true")
    fn condition_satisfied(&self, rule: &AlertRule, event: &DomainEvent) -> bool {
        let cond = rule.condition.trim();

        // Empty condition or "true" — always match
        if cond.is_empty() || cond == "true" {
            return true;
        }

        // `severity >= X` — compare severity levels
        if let Some(threshold) = cond.strip_prefix("severity >= ") {
            let threshold = threshold.trim().to_lowercase();
            let event_sev = event.severity.to_lowercase();
            return severity_ge(&event_sev, &threshold);
        }

        if let Some(threshold) = cond.strip_prefix("severity > ") {
            let threshold = threshold.trim().to_lowercase();
            let event_sev = event.severity.to_lowercase();
            return severity_gt(&event_sev, &threshold);
        }

        if let Some(expected) = cond.strip_prefix("severity == ") {
            let expected = expected.trim().to_lowercase();
            return event.severity.to_lowercase() == expected;
        }

        // `status != ok` — check if event indicates a problem
        if cond == "status != ok" {
            return event.severity.to_lowercase() != "info"
                && event.severity.to_lowercase() != "ok";
        }

        // `status == X`
        if let Some(expected) = cond.strip_prefix("status == ") {
            let expected = expected.trim().to_lowercase();
            return event.severity.to_lowercase() == expected;
        }

        // Default: parse as a simple numeric comparison on metadata fields
        self.evaluate_metadata_condition(cond, event)
    }

    /// Evaluate conditions that reference metadata fields, e.g.
    /// `consecutive_failures >= 3` or `circuit_open == true`.
    fn evaluate_metadata_condition(&self, cond: &str, event: &DomainEvent) -> bool {
        // Try to parse as `field op value`
        let parts: Vec<&str> = cond.split_whitespace().collect();
        if parts.len() != 3 {
            // If we can't parse it, default to matching (allow the rule to fire)
            return true;
        }

        let field = parts[0];
        let op = parts[1];
        let value = parts[2];

        // Look up the field in metadata
        let field_value = match event.metadata.get(field) {
            Some(v) => v,
            None => return false,
        };

        match op {
            ">=" | ">" | "<" | "<=" | "==" | "!=" => {
                // Numeric comparison
                let field_num = field_value.as_f64();
                let value_num = value.parse::<f64>().ok();
                match (field_num, value_num) {
                    (Some(a), Some(b)) => match op {
                        ">=" => a >= b,
                        ">" => a > b,
                        "<" => a < b,
                        "<=" => a <= b,
                        "==" => (a - b).abs() < f64::EPSILON,
                        "!=" => (a - b).abs() >= f64::EPSILON,
                        _ => true,
                    },
                    _ => {
                        // Boolean comparison (`circuit_open == true`).
                        if let Some(field_bool) = field_value.as_bool() {
                            let expected = value.eq_ignore_ascii_case("true");
                            return match op {
                                "==" => field_bool == expected,
                                "!=" => field_bool != expected,
                                _ => true,
                            };
                        }
                        // String comparison for non-numeric
                        let field_str = field_value.as_str().unwrap_or("");
                        match op {
                            "==" => field_str == value,
                            "!=" => field_str != value,
                            _ => true,
                        }
                    }
                }
            }
            _ => true,
        }
    }

    /// Map a rule source name to an [`AlertEventType`].
    fn map_source_to_event_type(&self, source: &str) -> AlertEventType {
        match source {
            s if s.contains("crawl") || s.contains("insight") => AlertEventType::NewInsight,
            s if s.contains("warning") => AlertEventType::NewWarning,
            s if s.contains("recipe") => AlertEventType::RecipeMatch,
            s if s.contains("competitor") => AlertEventType::CompetitorChange,
            s if s.contains("supply") || s.contains("supplier") => AlertEventType::SupplyChainRisk,
            _ => AlertEventType::SystemAlert,
        }
    }
}

// ─── Severity comparison helpers ─────────────────────────────────────────

fn severity_level(sev: &str) -> u8 {
    match sev {
        "critical" => 4,
        "high" => 3,
        "warning" => 2,
        "info" | "ok" => 1,
        _ => 0,
    }
}

fn severity_ge(event: &str, threshold: &str) -> bool {
    severity_level(event) >= severity_level(threshold)
}

fn severity_gt(event: &str, threshold: &str) -> bool {
    severity_level(event) > severity_level(threshold)
}

// ─── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_rules_yaml() -> &'static str {
        r#"
version: 1
routes:
  pager_webhook_env: PAGE_WEBHOOK_URL
  warning_email_env: ALERT_EMAIL_TO
rules:
  - name: critical-warning-detected
    source: warning_count
    condition: severity >= warning
    severity: critical
    for: 5m
    notify:
      - pager_webhook
  - name: high-priority-insight
    source: insight_count
    condition: severity >= high
    severity: warning
    for: 2m
    notify:
      - email
  - name: always-fire-test
    source: test_event
    condition: "true"
    severity: info
    for: 1m
    notify: []
"#
    }

    #[test]
    fn test_load_rules() {
        let evaluator = AlertEvaluator::load_from_yaml(sample_rules_yaml()).unwrap();
        assert_eq!(evaluator.rules.len(), 3);
    }

    #[test]
    fn test_parse_duration() {
        assert_eq!(parse_duration("30s"), Duration::from_secs(30));
        assert_eq!(parse_duration("5m"), Duration::from_secs(300));
        assert_eq!(parse_duration("1h"), Duration::from_secs(3600));
        assert_eq!(parse_duration("invalid"), Duration::from_secs(300));
    }

    #[test]
    fn test_source_matches() {
        let evaluator = AlertEvaluator::load_from_yaml(sample_rules_yaml()).unwrap();
        assert!(evaluator.source_matches("warning_count", "warning_count"));
        assert!(evaluator.source_matches("*", "anything"));
        assert!(!evaluator.source_matches("insight_count", "warning_count"));
    }

    #[tokio::test]
    async fn test_evaluate_matches_severity() {
        let evaluator = AlertEvaluator::load_from_yaml(sample_rules_yaml()).unwrap();

        let event = DomainEvent::new(
            "warning_count",
            "critical",
            "Critical warning spike",
            "A critical warning was detected in the latest batch",
        );

        let alerts = evaluator.evaluate(&event).await;
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].event_type, AlertEventType::NewWarning);
    }

    #[tokio::test]
    async fn test_evaluate_no_match_low_severity() {
        let evaluator = AlertEvaluator::load_from_yaml(sample_rules_yaml()).unwrap();

        let event = DomainEvent::new(
            "warning_count",
            "info",
            "Info event",
            "This should not trigger the warning rule",
        );

        let alerts = evaluator.evaluate(&event).await;
        assert_eq!(alerts.len(), 0);
    }

    #[tokio::test]
    async fn test_dedup_suppresses_duplicates() {
        let evaluator = AlertEvaluator::load_from_yaml(sample_rules_yaml()).unwrap();

        let event = DomainEvent::new(
            "test_event",
            "info",
            "Test event",
            "Should fire once per cooldown",
        );

        // First fire
        let alerts = evaluator.evaluate(&event).await;
        assert_eq!(alerts.len(), 1);

        // Second fire immediately — should be deduped
        let alerts = evaluator.evaluate(&event).await;
        assert_eq!(alerts.len(), 0);
    }

    #[tokio::test]
    async fn test_different_entities_not_deduped() {
        let evaluator = AlertEvaluator::load_from_yaml(sample_rules_yaml()).unwrap();

        // Distinct entity ids: the dedup key is (rule, entity), so reusing
        // `Uuid::nil()` for both events would (correctly) suppress the second.
        let event1 = DomainEvent::new("test_event", "info", "Event 1", "")
            .with_entity(Uuid::new_v4(), "entity-a");
        let event2 = DomainEvent::new("test_event", "info", "Event 2", "")
            .with_entity(Uuid::new_v4(), "entity-b");

        let alerts = evaluator.evaluate(&event1).await;
        assert_eq!(alerts.len(), 1);

        let alerts = evaluator.evaluate(&event2).await;
        assert_eq!(alerts.len(), 1);
    }

    #[test]
    fn test_severity_levels() {
        assert_eq!(severity_level("critical"), 4);
        assert_eq!(severity_level("high"), 3);
        assert_eq!(severity_level("warning"), 2);
        assert_eq!(severity_level("info"), 1);
        assert_eq!(severity_level("unknown"), 0);
        assert!(severity_ge("critical", "warning"));
        assert!(!severity_ge("info", "warning"));
    }

    #[test]
    fn test_metadata_condition_numeric() {
        let evaluator = AlertEvaluator::load_from_yaml(sample_rules_yaml()).unwrap();

        let event = DomainEvent::new("test", "info", "", "").with_metadata(serde_json::json!({
            "consecutive_failures": 5
        }));

        // `consecutive_failures >= 3` should match
        let result = evaluator.evaluate_metadata_condition("consecutive_failures >= 3", &event);
        assert!(result);

        // `consecutive_failures >= 10` should not match
        let result = evaluator.evaluate_metadata_condition("consecutive_failures >= 10", &event);
        assert!(!result);
    }

    #[test]
    fn test_metadata_condition_string() {
        let evaluator = AlertEvaluator::load_from_yaml(sample_rules_yaml()).unwrap();

        let event = DomainEvent::new("test", "info", "", "").with_metadata(serde_json::json!({
            "circuit_open": true
        }));

        let result = evaluator.evaluate_metadata_condition("circuit_open == true", &event);
        assert!(result);
    }

    #[test]
    fn test_map_source_to_event_type() {
        let evaluator = AlertEvaluator::load_from_yaml(sample_rules_yaml()).unwrap();

        assert_eq!(
            evaluator.map_source_to_event_type("warning_count"),
            AlertEventType::NewWarning
        );
        assert_eq!(
            evaluator.map_source_to_event_type("insight_count"),
            AlertEventType::NewInsight
        );
        assert_eq!(
            evaluator.map_source_to_event_type("competitor_change"),
            AlertEventType::CompetitorChange
        );
        assert_eq!(
            evaluator.map_source_to_event_type("supply_chain_risk"),
            AlertEventType::SupplyChainRisk
        );
        assert_eq!(
            evaluator.map_source_to_event_type("api_health_deep"),
            AlertEventType::SystemAlert
        );
    }
}
