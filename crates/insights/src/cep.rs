//! Complex Event Processing (CEP) engine for real-time security warning generation.
//!
//! Correlates incoming [`Observation`] events against multi-signal security
//! pattern rules and emits structured [`CepAlert`] records when rule thresholds
//! are breached.
//!
//! # Design
//! The CEP engine is intentionally stateless across restarts — it operates on a
//! sliding in-memory window of observations (configurable, default 24 hours).
//! It is designed to be driven from a NATS subscriber in the worker crate.
//!
//! # Pattern types
//! - **Threshold** — a single observation type exceeds a count/value threshold
//!   within a time window.
//! - **Sequence** — two or more event types appear in order within a window.
//! - **Correlation** — co-occurrence of two event types on the same entity.
//! - **Absence** — an expected event fails to occur within a deadline.
//!
//! # SLA tracking
//! Each [`CepAlert`] carries an `sla_deadline` timestamp.  The worker's
//! escalation handler must acknowledge or auto-escalate alerts past their SLA.
//!
//! # Usage
//! ```no_run
//! use apex_insights::cep::{CepEngine, CepRule, PatternType, CepSlaConfig};
//!
//! let mut engine = CepEngine::new(CepSlaConfig::default());
//! // Register rules
//! engine.register_rule(CepRule::threshold("dns_anomaly", "dns_anomaly", 3, 3600, "P0"));
//! // Feed observations as they arrive from NATS
//! // let alerts = engine.process_observation(&obs);
//! ```

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use tracing::{debug, info};
use uuid::Uuid;

// ─────────────────────────────────────────────────────────────────────────────
// SLA configuration
// ─────────────────────────────────────────────────────────────────────────────

/// Per-priority SLA deadlines for security alert acknowledgement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CepSlaConfig {
    /// P0 (critical) — must be acknowledged within this many seconds.
    pub p0_seconds: i64,
    /// P1 (high) — acknowledgement deadline.
    pub p1_seconds: i64,
    /// P2 (medium) — acknowledgement deadline.
    pub p2_seconds: i64,
    /// P3 (low/informational) — acknowledgement deadline.
    pub p3_seconds: i64,
}

impl Default for CepSlaConfig {
    fn default() -> Self {
        Self {
            p0_seconds: 900,    // 15 minutes
            p1_seconds: 3600,   // 1 hour
            p2_seconds: 14400,  // 4 hours
            p3_seconds: 86400,  // 24 hours
        }
    }
}

impl CepSlaConfig {
    /// Get the SLA deadline in seconds for a given priority string.
    pub fn deadline_seconds(&self, priority: &str) -> i64 {
        match priority {
            "P0" | "p0" | "critical" => self.p0_seconds,
            "P1" | "p1" | "high" => self.p1_seconds,
            "P2" | "p2" | "medium" => self.p2_seconds,
            _ => self.p3_seconds,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Pattern definitions
// ─────────────────────────────────────────────────────────────────────────────

/// The type of CEP pattern evaluated by a rule.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PatternType {
    /// N or more observations of the same type within a time window.
    Threshold {
        /// Observation type to count (matches `Observation::observation_type` string).
        obs_type: String,
        /// Minimum count to trigger.
        count: usize,
        /// Window size in seconds.
        window_seconds: i64,
    },
    /// Two different observation types must co-occur on the same entity_id within a window.
    Correlation {
        obs_type_a: String,
        obs_type_b: String,
        window_seconds: i64,
    },
    /// obs_type_a must occur before obs_type_b within a window (same entity).
    Sequence {
        obs_type_first: String,
        obs_type_second: String,
        window_seconds: i64,
    },
    /// obs_type must appear at least N times within window for the same entity.
    PerEntityThreshold {
        obs_type: String,
        count: usize,
        window_seconds: i64,
    },
    /// No observation of obs_type within a maximum expected interval (heartbeat check).
    Absence {
        obs_type: String,
        max_gap_seconds: i64,
    },
}

/// A single CEP rule: name, pattern, priority, and output template.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CepRule {
    /// Unique rule identifier.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Pattern to evaluate.
    pub pattern: PatternType,
    /// Priority: P0 (critical) through P3 (info).
    pub priority: String,
    /// Warning type to emit when triggered.
    pub warning_type: String,
    /// Summary template (supports `{entity_id}`, `{count}`, `{obs_type}` substitutions).
    pub title_template: String,
    /// Detail template.
    pub description_template: String,
    /// Minimum confidence for emitted alert [0, 1].
    pub confidence: f64,
    /// Whether this rule is currently active.
    pub enabled: bool,
}

impl CepRule {
    /// Create a threshold rule.
    pub fn threshold(
        id: impl Into<String>,
        obs_type: impl Into<String>,
        count: usize,
        window_seconds: i64,
        priority: impl Into<String>,
    ) -> Self {
        let obs_type = obs_type.into();
        let priority = priority.into();
        let title = format!("{{count}} {obs_type} events in {{window}}s window");
        let desc = format!("Threshold rule: {count}+ observations of type {obs_type} detected");
        Self {
            id: id.into(),
            name: format!("{obs_type} threshold ({count}/{window_seconds}s)"),
            pattern: PatternType::Threshold { obs_type: obs_type.clone(), count, window_seconds },
            priority,
            warning_type: obs_type,
            title_template: title,
            description_template: desc,
            confidence: 0.85,
            enabled: true,
        }
    }

    /// Create a correlation rule (co-occurrence of two event types).
    pub fn correlation(
        id: impl Into<String>,
        obs_type_a: impl Into<String>,
        obs_type_b: impl Into<String>,
        window_seconds: i64,
        priority: impl Into<String>,
    ) -> Self {
        let a = obs_type_a.into();
        let b = obs_type_b.into();
        let name = format!("{a} + {b} correlation");
        Self {
            id: id.into(),
            name: name.clone(),
            pattern: PatternType::Correlation {
                obs_type_a: a.clone(),
                obs_type_b: b.clone(),
                window_seconds,
            },
            priority: priority.into(),
            warning_type: format!("{a}_{b}_correlation"),
            title_template: format!("Correlated: {{entity_id}} shows both {a} and {b}"),
            description_template: format!("Correlation detected: {a} and {b} co-occurred within {window_seconds}s"),
            confidence: 0.9,
            enabled: true,
        }
    }

    /// Create a per-entity threshold rule.
    pub fn per_entity_threshold(
        id: impl Into<String>,
        obs_type: impl Into<String>,
        count: usize,
        window_seconds: i64,
        priority: impl Into<String>,
    ) -> Self {
        let obs_type = obs_type.into();
        let priority = priority.into();
        Self {
            id: id.into(),
            name: format!("{obs_type} per-entity threshold ({count}/{window_seconds}s)"),
            pattern: PatternType::PerEntityThreshold { obs_type: obs_type.clone(), count, window_seconds },
            priority: priority.clone(),
            warning_type: format!("{obs_type}_entity_threshold"),
            title_template: format!("Entity exceeded {count} {obs_type} events in {window_seconds}s"),
            description_template: format!("Per-entity threshold: {{entity_id}} triggered {count}+ {obs_type} observations"),
            confidence: 0.88,
            enabled: true,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Event types
// ─────────────────────────────────────────────────────────────────────────────

/// A normalised security event fed into the CEP engine.
///
/// Maps from raw `Observation` records with security-relevant types.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityEvent {
    /// Original observation id.
    pub observation_id: Uuid,
    /// Observation type string (e.g. `"dns_anomaly"`, `"cert_transparency"`, `"breach_detected"`).
    pub event_type: String,
    /// Associated entity id (company or person).
    pub entity_id: Option<Uuid>,
    /// Event timestamp.
    pub ts_utc: DateTime<Utc>,
    /// Confidence [0, 1].
    pub confidence: f64,
    /// Raw JSON payload for downstream enrichment.
    pub payload: serde_json::Value,
}

impl SecurityEvent {
    pub fn new(
        observation_id: Uuid,
        event_type: impl Into<String>,
        entity_id: Option<Uuid>,
        ts_utc: DateTime<Utc>,
        confidence: f64,
        payload: serde_json::Value,
    ) -> Self {
        Self {
            observation_id,
            event_type: event_type.into(),
            entity_id,
            ts_utc,
            confidence,
            payload,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// CEP alert (output)
// ─────────────────────────────────────────────────────────────────────────────

/// An alert emitted by the CEP engine when a rule is triggered.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CepAlert {
    /// Unique alert id.
    pub id: Uuid,
    /// Rule that triggered this alert.
    pub rule_id: String,
    /// Warning type for downstream dispatch.
    pub warning_type: String,
    /// Alert title.
    pub title: String,
    /// Alert description.
    pub description: String,
    /// Priority: P0–P3.
    pub priority: String,
    /// Severity string: critical | high | medium | low.
    pub severity: String,
    /// Entity associated with the alert (if per-entity trigger).
    pub entity_id: Option<Uuid>,
    /// Observations that contributed to this trigger.
    pub contributing_events: Vec<Uuid>,
    /// Confidence [0, 1].
    pub confidence: f64,
    /// When the alert fired.
    pub fired_at: DateTime<Utc>,
    /// SLA deadline for acknowledgement.
    pub sla_deadline: DateTime<Utc>,
    /// Whether this alert has been dispatched to notification channels.
    pub dispatched: bool,
}

impl CepAlert {
    fn new(
        rule: &CepRule,
        entity_id: Option<Uuid>,
        contributing: Vec<Uuid>,
        sla_seconds: i64,
        title: String,
        description: String,
    ) -> Self {
        let now = Utc::now();
        let severity = match rule.priority.as_str() {
            "P0" => "critical",
            "P1" => "high",
            "P2" => "medium",
            _ => "low",
        };
        Self {
            id: Uuid::new_v4(),
            rule_id: rule.id.clone(),
            warning_type: rule.warning_type.clone(),
            title,
            description,
            priority: rule.priority.clone(),
            severity: severity.to_string(),
            entity_id,
            contributing_events: contributing,
            confidence: rule.confidence,
            fired_at: now,
            sla_deadline: now + Duration::seconds(sla_seconds),
            dispatched: false,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// CEP engine
// ─────────────────────────────────────────────────────────────────────────────

/// Core Complex Event Processing engine.
///
/// Maintains a sliding event window and evaluates registered rules on each
/// incoming `SecurityEvent`.
pub struct CepEngine {
    rules: Vec<CepRule>,
    /// Sliding window of recent security events.
    event_window: VecDeque<SecurityEvent>,
    /// Maximum age of events to retain (default 24 hours).
    window_duration: Duration,
    sla_config: CepSlaConfig,
    /// Deduplication: (rule_id, entity_or_global) → last_fired timestamp.
    /// Prevents the same rule from firing again within a cooldown period.
    last_fired: HashMap<String, DateTime<Utc>>,
    /// Cooldown in seconds before the same rule may re-fire (default 300s).
    cooldown_seconds: i64,
    /// Accumulated alerts not yet retrieved.
    pending_alerts: Vec<CepAlert>,
}

impl CepEngine {
    /// Create a new engine with the given SLA configuration.
    pub fn new(sla_config: CepSlaConfig) -> Self {
        Self {
            rules: Vec::new(),
            event_window: VecDeque::new(),
            window_duration: Duration::hours(24),
            sla_config,
            last_fired: HashMap::new(),
            cooldown_seconds: 300,
            pending_alerts: Vec::new(),
        }
    }

    /// Create engine pre-loaded with the default security rule set.
    pub fn with_default_rules(sla_config: CepSlaConfig) -> Self {
        let mut engine = Self::new(sla_config);
        engine.register_default_rules();
        engine
    }

    /// Set the sliding window duration.
    pub fn window_duration(mut self, dur: Duration) -> Self {
        self.window_duration = dur;
        self
    }

    /// Set rule cooldown period.
    pub fn cooldown_seconds(mut self, secs: i64) -> Self {
        self.cooldown_seconds = secs;
        self
    }

    /// Register a single rule.
    pub fn register_rule(&mut self, rule: CepRule) {
        self.rules.push(rule);
    }

    /// Register a slice of rules.
    pub fn register_rules(&mut self, rules: impl IntoIterator<Item = CepRule>) {
        self.rules.extend(rules);
    }

    /// Register the default security rule set covering:
    /// - DNS anomalies
    /// - Certificate transparency alerts
    /// - Breach detections
    /// - Sanctions hits
    /// - Credential exposure
    pub fn register_default_rules(&mut self) {
        let rules = vec![
            // ── DNS Security ───────────────────────────────────────────────
            CepRule::threshold(
                "dns_anomaly_burst",
                "dns_anomaly",
                3, 3600,
                "P1",
            ),
            CepRule::per_entity_threshold(
                "entity_dns_anomaly",
                "dns_anomaly",
                2, 7200,
                "P1",
            ),

            // ── Certificate Transparency ───────────────────────────────────
            CepRule::threshold(
                "cert_wildcard_burst",
                "ct_wildcard_issued",
                1, 3600,
                "P1",
            ),
            CepRule {
                id: "cert_unexpected_issuer".into(),
                name: "Unexpected cert issuer detected".into(),
                pattern: PatternType::Threshold {
                    obs_type: "ct_unexpected_issuer".into(),
                    count: 1,
                    window_seconds: 86400,
                },
                priority: "P0".into(),
                warning_type: "cert_hijack_attempt".into(),
                title_template: "Unexpected certificate issuer detected for {entity_id}".into(),
                description_template: "A certificate for a monitored domain was issued by an unexpected CA — possible MITM or domain hijack.".into(),
                confidence: 0.92,
                enabled: true,
            },

            // ── Breach / Credential Exposure ───────────────────────────────
            CepRule {
                id: "breach_critical".into(),
                name: "Critical breach detected".into(),
                pattern: PatternType::Threshold {
                    obs_type: "breach_critical".into(),
                    count: 1,
                    window_seconds: 86400,
                },
                priority: "P0".into(),
                warning_type: "credential_breach".into(),
                title_template: "Critical data breach detected: {entity_id}".into(),
                description_template: "A critical-severity breach exposing credentials or sensitive PII was detected.".into(),
                confidence: 0.95,
                enabled: true,
            },
            CepRule::threshold(
                "breach_medium_burst",
                "breach_medium",
                2, 86400,
                "P1",
            ),

            // ── Sanctions Hits ─────────────────────────────────────────────
            CepRule {
                id: "sanctions_exact_hit".into(),
                name: "Exact sanctions list match".into(),
                pattern: PatternType::Threshold {
                    obs_type: "sanctions_hit_exact".into(),
                    count: 1,
                    window_seconds: 86400,
                },
                priority: "P0".into(),
                warning_type: "sanctions_match".into(),
                title_template: "Sanctions list exact match: {entity_id}".into(),
                description_template: "An entity in the system matched exactly against a sanctions list entry.".into(),
                confidence: 1.0,
                enabled: true,
            },
            CepRule {
                id: "sanctions_fuzzy_cluster".into(),
                name: "Multiple fuzzy sanctions matches".into(),
                pattern: PatternType::PerEntityThreshold {
                    obs_type: "sanctions_hit_fuzzy".into(),
                    count: 2,
                    window_seconds: 86400,
                },
                priority: "P1".into(),
                warning_type: "sanctions_potential_match".into(),
                title_template: "Multiple sanctions fuzzy matches for {entity_id}".into(),
                description_template: "An entity matched against 2+ sanctions entries above the fuzzy threshold.".into(),
                confidence: 0.75,
                enabled: true,
            },

            // ── Lookalike Domain / Phishing ────────────────────────────────
            CepRule::threshold(
                "lookalike_domain_burst",
                "lookalike_domain",
                2, 86400,
                "P1",
            ),
            CepRule {
                id: "lookalike_plus_cert".into(),
                name: "Lookalike domain with new certificate".into(),
                pattern: PatternType::Correlation {
                    obs_type_a: "lookalike_domain".into(),
                    obs_type_b: "ct_lookalike_cert".into(),
                    window_seconds: 86400 * 3,
                },
                priority: "P0".into(),
                warning_type: "phishing_infrastructure".into(),
                title_template: "Active phishing infrastructure detected for {entity_id}".into(),
                description_template: "A lookalike domain for a monitored entity has acquired a TLS certificate — active phishing campaign likely.".into(),
                confidence: 0.92,
                enabled: true,
            },

            // ── Composite: Breach + Sanctions ──────────────────────────────
            CepRule {
                id: "breach_and_sanctions".into(),
                name: "Breach detected + sanctions exposure".into(),
                pattern: PatternType::Correlation {
                    obs_type_a: "breach_medium".into(),
                    obs_type_b: "sanctions_hit_fuzzy".into(),
                    window_seconds: 86400 * 7,
                },
                priority: "P0".into(),
                warning_type: "high_risk_entity".into(),
                title_template: "HIGH RISK: {entity_id} has breach and sanctions exposure".into(),
                description_template: "Entity shows both data breach exposure and sanctions list proximity — requires immediate compliance review.".into(),
                confidence: 0.88,
                enabled: true,
            },
        ];
        self.register_rules(rules);
    }

    /// Process a single incoming security event.
    ///
    /// Adds the event to the sliding window, evicts stale events, then
    /// evaluates all enabled rules.  Returns a `Vec<CepAlert>` for any
    /// rules that fired.
    pub fn process_event(&mut self, event: SecurityEvent) -> Vec<CepAlert> {
        // Add to window
        self.event_window.push_back(event);

        // Evict events older than window_duration
        let cutoff = Utc::now() - self.window_duration;
        while self.event_window.front().map(|e| e.ts_utc < cutoff).unwrap_or(false) {
            self.event_window.pop_front();
        }

        // Evaluate all enabled rules
        let rules: Vec<CepRule> = self.rules.iter().filter(|r| r.enabled).cloned().collect();
        let mut new_alerts = Vec::new();

        for rule in &rules {
            if let Some(alert) = self.evaluate_rule(rule) {
                new_alerts.push(alert);
            }
        }

        if !new_alerts.is_empty() {
            info!(count = new_alerts.len(), "CEP engine fired {} alert(s)", new_alerts.len());
        }

        self.pending_alerts.extend(new_alerts.clone());
        new_alerts
    }

    /// Process a batch of events at once.
    pub fn process_batch(&mut self, events: Vec<SecurityEvent>) -> Vec<CepAlert> {
        let mut all_alerts = Vec::new();
        for event in events {
            all_alerts.extend(self.process_event(event));
        }
        all_alerts
    }

    /// Drain all pending alerts that have not been retrieved yet.
    pub fn drain_alerts(&mut self) -> Vec<CepAlert> {
        std::mem::take(&mut self.pending_alerts)
    }

    /// Return the current sliding window size (number of retained events).
    pub fn window_size(&self) -> usize {
        self.event_window.len()
    }

    /// Return count of registered rules.
    pub fn rule_count(&self) -> usize {
        self.rules.len()
    }

    // ── Rule evaluators ───────────────────────────────────────────────────────

    fn evaluate_rule(&mut self, rule: &CepRule) -> Option<CepAlert> {
        match &rule.pattern.clone() {
            PatternType::Threshold { obs_type, count, window_seconds } => {
                self.eval_threshold(rule, obs_type, *count, *window_seconds, None)
            }
            PatternType::PerEntityThreshold { obs_type, count, window_seconds } => {
                self.eval_per_entity_threshold(rule, obs_type, *count, *window_seconds)
            }
            PatternType::Correlation { obs_type_a, obs_type_b, window_seconds } => {
                self.eval_correlation(rule, obs_type_a, obs_type_b, *window_seconds)
            }
            PatternType::Sequence { obs_type_first, obs_type_second, window_seconds } => {
                self.eval_sequence(rule, obs_type_first, obs_type_second, *window_seconds)
            }
            PatternType::Absence { obs_type, max_gap_seconds } => {
                self.eval_absence(rule, obs_type, *max_gap_seconds)
            }
        }
    }

    fn is_in_cooldown(&self, rule: &CepRule, entity_key: &str) -> bool {
        let dedup_key = format!("{}:{}", rule.id, entity_key);
        if let Some(&last_fired) = self.last_fired.get(&dedup_key) {
            Utc::now() - last_fired < Duration::seconds(self.cooldown_seconds)
        } else {
            false
        }
    }

    fn record_fire(&mut self, rule: &CepRule, entity_key: &str) {
        let dedup_key = format!("{}:{}", rule.id, entity_key);
        self.last_fired.insert(dedup_key, Utc::now());
    }

    fn eval_threshold(
        &mut self,
        rule: &CepRule,
        obs_type: &str,
        count: usize,
        window_seconds: i64,
        entity_filter: Option<Uuid>,
    ) -> Option<CepAlert> {
        let cutoff = Utc::now() - Duration::seconds(window_seconds);
        let entity_key = entity_filter.map(|u| u.to_string()).unwrap_or_else(|| "global".into());

        if self.is_in_cooldown(rule, &entity_key) {
            return None;
        }

        // Collect owned data first to release the immutable borrow of event_window
        // before calling record_fire() (which mutably borrows self).
        let (matching_ids, entity_id, matched_count) = {
            let matching: Vec<&SecurityEvent> = self.event_window
                .iter()
                .filter(|e| {
                    e.event_type == obs_type
                        && e.ts_utc >= cutoff
                        && entity_filter.map(|id| e.entity_id == Some(id)).unwrap_or(true)
                })
                .collect();
            let ids: Vec<Uuid> = matching.iter().map(|e| e.observation_id).collect();
            let eid = matching.first().and_then(|e| e.entity_id);
            let len = matching.len();
            (ids, eid, len)
        };

        if matched_count >= count {
            self.record_fire(rule, &entity_key);
            let sla_secs = self.sla_config.deadline_seconds(&rule.priority);

            let title = rule.title_template
                .replace("{count}", &matched_count.to_string())
                .replace("{entity_id}", &entity_id.map(|u| u.to_string()).unwrap_or_else(|| "N/A".into()))
                .replace("{obs_type}", obs_type)
                .replace("{window}", &window_seconds.to_string());

            let description = rule.description_template
                .replace("{count}", &matched_count.to_string())
                .replace("{entity_id}", &entity_id.map(|u| u.to_string()).unwrap_or_else(|| "N/A".into()))
                .replace("{obs_type}", obs_type);

            debug!(rule_id = %rule.id, count = matched_count, "CEP threshold rule fired");
            Some(CepAlert::new(rule, entity_id, matching_ids, sla_secs, title, description))
        } else {
            None
        }
    }

    fn eval_per_entity_threshold(
        &mut self,
        rule: &CepRule,
        obs_type: &str,
        count: usize,
        window_seconds: i64,
    ) -> Option<CepAlert> {
        let cutoff = Utc::now() - Duration::seconds(window_seconds);

        // Collect observation IDs (owned) grouped by entity — avoids holding references
        // into event_window across the record_fire() mutable borrow boundary.
        let mut entity_obs: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
        for e in &self.event_window {
            if e.event_type == obs_type && e.ts_utc >= cutoff {
                if let Some(eid) = e.entity_id {
                    entity_obs.entry(eid).or_default().push(e.observation_id);
                }
            }
        }

        for (entity_id, obs_ids) in entity_obs {
            if obs_ids.len() >= count {
                let entity_key = entity_id.to_string();
                if self.is_in_cooldown(rule, &entity_key) {
                    continue;
                }
                self.record_fire(rule, &entity_key);
                let sla_secs = self.sla_config.deadline_seconds(&rule.priority);

                let title = rule.title_template
                    .replace("{entity_id}", &entity_id.to_string())
                    .replace("{count}", &obs_ids.len().to_string())
                    .replace("{obs_type}", obs_type);

                let description = rule.description_template
                    .replace("{entity_id}", &entity_id.to_string())
                    .replace("{count}", &obs_ids.len().to_string());

                debug!(rule_id = %rule.id, entity = %entity_id, "CEP per-entity threshold fired");
                return Some(CepAlert::new(rule, Some(entity_id), obs_ids, sla_secs, title, description));
            }
        }
        None
    }

    fn eval_correlation(
        &mut self,
        rule: &CepRule,
        obs_type_a: &str,
        obs_type_b: &str,
        window_seconds: i64,
    ) -> Option<CepAlert> {
        let cutoff = Utc::now() - Duration::seconds(window_seconds);

        // Collect owned observation IDs to avoid holding references across record_fire().
        let mut entities_a: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
        let mut entities_b: HashMap<Uuid, Vec<Uuid>> = HashMap::new();

        for e in &self.event_window {
            if e.ts_utc < cutoff { continue; }
            if let Some(eid) = e.entity_id {
                if e.event_type == obs_type_a {
                    entities_a.entry(eid).or_default().push(e.observation_id);
                } else if e.event_type == obs_type_b {
                    entities_b.entry(eid).or_default().push(e.observation_id);
                }
            }
        }

        for (entity_id, ids_a) in &entities_a {
            if let Some(ids_b) = entities_b.get(entity_id) {
                let entity_key = entity_id.to_string();
                if self.is_in_cooldown(rule, &entity_key) {
                    continue;
                }
                let mut contributing: Vec<Uuid> = ids_a.clone();
                contributing.extend_from_slice(ids_b);
                let eid = *entity_id;
                self.record_fire(rule, &entity_key);
                let sla_secs = self.sla_config.deadline_seconds(&rule.priority);

                let title = rule.title_template
                    .replace("{entity_id}", &eid.to_string());
                let description = rule.description_template
                    .replace("{entity_id}", &eid.to_string());

                debug!(rule_id = %rule.id, entity = %eid, "CEP correlation rule fired");
                return Some(CepAlert::new(rule, Some(eid), contributing, sla_secs, title, description));
            }
        }
        None
    }

    fn eval_sequence(
        &mut self,
        rule: &CepRule,
        obs_type_first: &str,
        obs_type_second: &str,
        window_seconds: i64,
    ) -> Option<CepAlert> {
        let cutoff = Utc::now() - Duration::seconds(window_seconds);

        // Collect earliest timestamps AND observation IDs for obs_type_first per entity (owned).
        let mut entity_first: HashMap<Uuid, (DateTime<Utc>, Uuid)> = HashMap::new();
        for e in &self.event_window {
            if e.ts_utc < cutoff { continue; }
            if let Some(eid) = e.entity_id {
                if e.event_type == obs_type_first {
                    entity_first.entry(eid)
                        .and_modify(|(t, id)| {
                            if e.ts_utc < *t {
                                *t = e.ts_utc;
                                *id = e.observation_id; // track ID of earliest first-event
                            }
                        })
                        .or_insert((e.ts_utc, e.observation_id));
                }
            }
        }

        // Find a second event that follows a first for the same entity.
        // Carry both event IDs for the contributing_events list.
        let trigger: Option<(Uuid, Uuid, Uuid)> = self.event_window.iter()
            .filter(|e| e.ts_utc >= cutoff && e.event_type == obs_type_second)
            .find_map(|e| {
                if let Some(eid) = e.entity_id {
                    if let Some(&(first_ts, first_obs_id)) = entity_first.get(&eid) {
                        if first_ts < e.ts_utc {
                            return Some((eid, first_obs_id, e.observation_id));
                        }
                    }
                }
                None
            });

        if let Some((entity_id, first_obs_id, second_obs_id)) = trigger {
            let entity_key = entity_id.to_string();
            if !self.is_in_cooldown(rule, &entity_key) {
                self.record_fire(rule, &entity_key);
                let sla_secs = self.sla_config.deadline_seconds(&rule.priority);
                let title = rule.title_template.replace("{entity_id}", &entity_id.to_string());
                let description = rule.description_template.replace("{entity_id}", &entity_id.to_string());
                return Some(CepAlert::new(rule, Some(entity_id), vec![first_obs_id, second_obs_id], sla_secs, title, description));
            }
        }
        None
    }

    fn eval_absence(
        &mut self,
        rule: &CepRule,
        obs_type: &str,
        max_gap_seconds: i64,
    ) -> Option<CepAlert> {
        let now = Utc::now();
        let cutoff = now - Duration::seconds(max_gap_seconds);

        // Check if there's been at least one event of this type within the expected window
        let has_recent = self.event_window
            .iter()
            .any(|e| e.event_type == obs_type && e.ts_utc >= cutoff);

        if !has_recent {
            if self.is_in_cooldown(rule, "absence") {
                return None;
            }
            self.record_fire(rule, "absence");
            let sla_secs = self.sla_config.deadline_seconds(&rule.priority);
            let title = rule.title_template
                .replace("{obs_type}", obs_type)
                .replace("{gap}", &max_gap_seconds.to_string());
            let description = rule.description_template
                .replace("{obs_type}", obs_type)
                .replace("{gap}", &max_gap_seconds.to_string());
            return Some(CepAlert::new(rule, None, vec![], sla_secs, title, description));
        }
        None
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// SLA enforcement helper
// ─────────────────────────────────────────────────────────────────────────────

/// Check which alerts have breached their SLA (deadline passed without acknowledgement).
///
/// `alerts` — list of pending alerts with their acknowledgement status.
/// Returns only those that are past SLA deadline.
pub fn find_sla_breached_alerts(alerts: &[(CepAlert, bool)]) -> Vec<&CepAlert> {
    let now = Utc::now();
    alerts
        .iter()
        .filter(|(alert, acked)| !acked && now > alert.sla_deadline)
        .map(|(alert, _)| alert)
        .collect()
}

/// Compute seconds remaining until the SLA deadline.
///
/// Returns a negative number if the SLA has already been breached.
pub fn sla_seconds_remaining(alert: &CepAlert) -> i64 {
    (alert.sla_deadline - Utc::now()).num_seconds()
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_event(event_type: &str, entity_id: Option<Uuid>, ts: DateTime<Utc>) -> SecurityEvent {
        SecurityEvent::new(
            Uuid::new_v4(),
            event_type,
            entity_id,
            ts,
            0.95,
            serde_json::json!({}),
        )
    }

    #[test]
    fn threshold_rule_fires_when_count_reached() {
        let mut engine = CepEngine::new(CepSlaConfig::default());
        engine.register_rule(CepRule::threshold("r1", "dns_anomaly", 3, 3600, "P1"));

        let now = Utc::now();
        // Feed 2 events — should not fire
        let a1 = engine.process_event(make_event("dns_anomaly", None, now));
        assert!(a1.is_empty(), "Should not fire with 1 event");

        let a2 = engine.process_event(make_event("dns_anomaly", None, now));
        assert!(a2.is_empty(), "Should not fire with 2 events");

        // 3rd event should trigger
        let a3 = engine.process_event(make_event("dns_anomaly", None, now));
        assert!(!a3.is_empty(), "Should fire with 3 events");
        assert_eq!(a3[0].rule_id, "r1");
        assert_eq!(a3[0].severity, "high");
    }

    #[test]
    fn threshold_rule_cooldown_prevents_double_fire() {
        let mut engine = CepEngine::new(CepSlaConfig::default());
        engine.register_rule(CepRule::threshold("r1", "dns_anomaly", 1, 3600, "P1"));

        let now = Utc::now();
        let a1 = engine.process_event(make_event("dns_anomaly", None, now));
        assert!(!a1.is_empty(), "Should fire on first threshold");

        // Immediate second feed — cooldown should suppress
        let a2 = engine.process_event(make_event("dns_anomaly", None, now));
        assert!(a2.is_empty(), "Cooldown should suppress re-fire");
    }

    #[test]
    fn per_entity_threshold_fires_for_specific_entity() {
        let mut engine = CepEngine::new(CepSlaConfig::default());
        engine.register_rule(CepRule::per_entity_threshold("e1", "breach_detected", 2, 3600, "P0"));

        let entity = Uuid::new_v4();
        let other_entity = Uuid::new_v4();
        let now = Utc::now();

        // Events from another entity should not count
        engine.process_event(make_event("breach_detected", Some(other_entity), now));
        engine.process_event(make_event("breach_detected", Some(other_entity), now));

        let a = engine.process_event(make_event("breach_detected", Some(entity), now));
        assert!(a.is_empty());

        // Second event from target entity should fire
        let b = engine.process_event(make_event("breach_detected", Some(entity), now));
        assert!(!b.is_empty(), "Should fire when entity hits threshold");
        assert_eq!(b[0].entity_id, Some(entity));
    }

    #[test]
    fn correlation_rule_fires_on_both_types() {
        let mut engine = CepEngine::new(CepSlaConfig::default());
        engine.register_rule(CepRule::correlation(
            "corr1",
            "type_a",
            "type_b",
            3600,
            "P0",
        ));

        let entity = Uuid::new_v4();
        let now = Utc::now();

        // Only type_a — should not fire
        let a = engine.process_event(make_event("type_a", Some(entity), now));
        assert!(a.is_empty());

        // Adding type_b should trigger correlation
        let b = engine.process_event(make_event("type_b", Some(entity), now));
        assert!(!b.is_empty(), "Correlation should fire with both types present");
        assert_eq!(b[0].rule_id, "corr1");
    }

    #[test]
    fn sla_config_p0_deadline() {
        let config = CepSlaConfig::default();
        assert_eq!(config.deadline_seconds("P0"), 900);
        assert_eq!(config.deadline_seconds("P1"), 3600);
        assert_eq!(config.deadline_seconds("P2"), 14400);
        assert_eq!(config.deadline_seconds("P3"), 86400);
    }

    #[test]
    fn sla_seconds_remaining_future() {
        let mut engine = CepEngine::new(CepSlaConfig { p0_seconds: 900, ..Default::default() });
        engine.register_rule(CepRule {
            id: "r0".into(),
            name: "test".into(),
            pattern: PatternType::Threshold { obs_type: "x".into(), count: 1, window_seconds: 3600 },
            priority: "P0".into(),
            warning_type: "xw".into(),
            title_template: "T".into(),
            description_template: "D".into(),
            confidence: 1.0,
            enabled: true,
        });
        let now = Utc::now();
        let alerts = engine.process_event(make_event("x", None, now));
        assert!(!alerts.is_empty());
        let remaining = sla_seconds_remaining(&alerts[0]);
        assert!(remaining > 0, "SLA should still have time remaining");
        assert!(remaining <= 900, "SLA should be at most 900s");
    }

    #[test]
    fn default_rules_registered() {
        let engine = CepEngine::with_default_rules(CepSlaConfig::default());
        assert!(engine.rule_count() > 8, "Default rules should include 8+ rules");
    }

    #[test]
    fn drain_alerts_clears_pending() {
        let mut engine = CepEngine::new(CepSlaConfig::default());
        engine.register_rule(CepRule::threshold("r", "ev", 1, 3600, "P1"));
        let now = Utc::now();
        engine.process_event(make_event("ev", None, now));
        let drained = engine.drain_alerts();
        assert!(!drained.is_empty());
        let drained2 = engine.drain_alerts();
        assert!(drained2.is_empty(), "Second drain should be empty");
    }
}
