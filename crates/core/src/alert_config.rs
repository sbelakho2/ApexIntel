//! Alert threshold configuration types.
//!
//! These types define per-entity and global alert thresholds that control
//! which alerts are dispatched through the notification pipeline.
//! Configuration can be persisted as JSONB and exposed through the API.

use serde::{Deserialize, Serialize};

// ─────────────────────────────────────────────────────────────────────────────
// AlertSeverity
// ─────────────────────────────────────────────────────────────────────────────

/// Severity of an outgoing alert.
///
/// Ordering is by increasing severity: Info < Low < Medium < High < Critical.
/// This enum is compatible with `apex_worker::notifications::AlertSeverity`
/// which re-exports this definition.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertSeverity {
    #[default]
    Info,
    Low,
    Medium,
    High,
    Critical,
}

impl AlertSeverity {
    /// Return the static string representation.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Critical => "critical",
        }
    }

    /// Parse from a string (case-insensitive).  Falls back to `Info` on
    /// unrecognised input.
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "critical" => Self::Critical,
            "high" => Self::High,
            "medium" => Self::Medium,
            "low" => Self::Low,
            _ => Self::Info,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// AlertChannel
// ─────────────────────────────────────────────────────────────────────────────

/// Supported alert delivery channels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertChannel {
    /// In-app notification bell.
    InApp,
    /// SMTP email alert.
    Email,
    /// Slack / Slack-compatible webhook.
    Slack,
    /// Generic HTTP webhook.
    Webhook,
}

impl AlertChannel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InApp => "in_app",
            Self::Email => "email",
            Self::Slack => "slack",
            Self::Webhook => "webhook",
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// AlertOverride
// ─────────────────────────────────────────────────────────────────────────────

/// Per-alert-type override rules within an [`EntityAlertConfig`].
///
/// When present, these rules take precedence over the entity-level defaults
/// for the matching `alert_type`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertOverride {
    /// The alert type this override applies to
    /// (e.g. `"warning"`, `"insight"`, `"recipe_match"`, `"poi_update"`).
    pub alert_type: String,

    /// Minimum severity threshold for this alert type.
    pub min_severity: AlertSeverity,

    /// Whether alerts of this type are enabled at all.
    pub enabled: bool,

    /// Cooldown period in minutes (0 = inherit entity-level cooldown).
    #[serde(default)]
    pub cooldown_minutes: u32,

    /// Maximum daily alerts for this type (0 = inherit entity-level limit).
    #[serde(default)]
    pub max_daily: u32,
}

// ─────────────────────────────────────────────────────────────────────────────
// EntityAlertConfig
// ─────────────────────────────────────────────────────────────────────────────

/// Per-entity alert threshold configuration.
///
/// Each tracked entity (company, person, …) can have its own alert profile
/// that overrides the [`GlobalAlertDefaults`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityAlertConfig {
    /// Entity this configuration applies to.
    pub entity_id: String,

    /// Minimum severity threshold for all alert types on this entity.
    pub min_severity: AlertSeverity,

    /// Enabled delivery channels for this entity.
    pub enabled_channels: Vec<AlertChannel>,

    /// Cooldown period in minutes between consecutive alerts for the same entity.
    pub cooldown_minutes: u32,

    /// Maximum number of alerts allowed per day for this entity (0 = unlimited).
    pub max_daily_alerts: u32,

    /// Per-alert-type overrides (optional).  When empty, the entity-level
    /// fields apply to all types.
    #[serde(default)]
    pub override_rules: Vec<AlertOverride>,

    /// Whether this configuration is active.  When `false` all alerts for
    /// this entity are suppressed.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

impl EntityAlertConfig {
    /// Returns `true` when the given alert should **not** be dispatched.
    ///
    /// Checks per-alert-type override rules first, then falls back to the
    /// entity-level `min_severity`.
    pub fn is_alert_suppressed(&self, alert_type: &str, severity: AlertSeverity) -> bool {
        if !self.enabled {
            return true;
        }

        // Check for alert-type-specific override first
        for rule in &self.override_rules {
            if rule.alert_type == alert_type {
                if !rule.enabled {
                    return true;
                }
                if severity < rule.min_severity {
                    return true;
                }
                return false;
            }
        }

        // Fall back to global entity threshold
        severity < self.min_severity
    }
}

impl Default for EntityAlertConfig {
    fn default() -> Self {
        Self {
            entity_id: String::new(),
            min_severity: AlertSeverity::Medium,
            enabled_channels: vec![AlertChannel::InApp],
            cooldown_minutes: 60,
            max_daily_alerts: 50,
            override_rules: Vec::new(),
            enabled: true,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// GlobalAlertDefaults
// ─────────────────────────────────────────────────────────────────────────────

/// Global default alert thresholds applied to all entities that do not have
/// an explicit [`EntityAlertConfig`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalAlertDefaults {
    /// Default minimum severity threshold.
    pub min_severity: AlertSeverity,

    /// Default set of enabled delivery channels.
    pub enabled_channels: Vec<AlertChannel>,

    /// Default cooldown period between alerts for the same entity (minutes).
    pub cooldown_minutes: u32,

    /// Default maximum daily alerts per entity (0 = unlimited).
    pub max_daily_alerts: u32,
}

impl Default for GlobalAlertDefaults {
    fn default() -> Self {
        Self {
            min_severity: AlertSeverity::High,
            enabled_channels: vec![
                AlertChannel::InApp,
                AlertChannel::Email,
                AlertChannel::Slack,
            ],
            cooldown_minutes: 30,
            max_daily_alerts: 100,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── AlertSeverity ──────────────────────────────────────────────────────────

    #[test]
    fn severity_ordering() {
        assert!(AlertSeverity::Info < AlertSeverity::Low);
        assert!(AlertSeverity::Low < AlertSeverity::Medium);
        assert!(AlertSeverity::Medium < AlertSeverity::High);
        assert!(AlertSeverity::High < AlertSeverity::Critical);
    }

    #[test]
    fn severity_as_str() {
        assert_eq!(AlertSeverity::Info.as_str(), "info");
        assert_eq!(AlertSeverity::Critical.as_str(), "critical");
    }

    #[test]
    fn severity_from_str_case_insensitive() {
        assert_eq!(AlertSeverity::from_str("CRITICAL"), AlertSeverity::Critical);
        assert_eq!(AlertSeverity::from_str("High"), AlertSeverity::High);
        assert_eq!(AlertSeverity::from_str("medium"), AlertSeverity::Medium);
        assert_eq!(AlertSeverity::from_str("unknown"), AlertSeverity::Info);
    }

    #[test]
    fn severity_default_is_info() {
        assert_eq!(AlertSeverity::default(), AlertSeverity::Info);
    }

    #[test]
    fn severity_serde_roundtrip() {
        let json = serde_json::to_string(&AlertSeverity::Critical).unwrap();
        assert_eq!(json, "\"critical\"");
        let back: AlertSeverity = serde_json::from_str(&json).unwrap();
        assert_eq!(back, AlertSeverity::Critical);
    }

    // ── AlertChannel ──────────────────────────────────────────────────────────

    #[test]
    fn channel_as_str() {
        assert_eq!(AlertChannel::InApp.as_str(), "in_app");
        assert_eq!(AlertChannel::Slack.as_str(), "slack");
    }

    #[test]
    fn channel_serde_roundtrip() {
        let json = serde_json::to_string(&AlertChannel::Email).unwrap();
        assert_eq!(json, "\"email\"");
        let back: AlertChannel = serde_json::from_str(&json).unwrap();
        assert_eq!(back, AlertChannel::Email);
    }

    // ── EntityAlertConfig ─────────────────────────────────────────────────────

    #[test]
    fn default_config_is_not_suppressing_medium_or_higher() {
        let cfg = EntityAlertConfig::default();
        // Default min_severity = Medium, so Medium and above pass
        assert!(!cfg.is_alert_suppressed("warning", AlertSeverity::Medium));
        assert!(!cfg.is_alert_suppressed("warning", AlertSeverity::Critical));
        // Info and Low should be suppressed
        assert!(cfg.is_alert_suppressed("warning", AlertSeverity::Info));
        assert!(cfg.is_alert_suppressed("warning", AlertSeverity::Low));
    }

    #[test]
    fn disabled_config_suppresses_all() {
        let cfg = EntityAlertConfig {
            enabled: false,
            ..Default::default()
        };
        assert!(cfg.is_alert_suppressed("warning", AlertSeverity::Critical));
        assert!(cfg.is_alert_suppressed("insight", AlertSeverity::Info));
    }

    #[test]
    fn override_rule_takes_precedence() {
        let cfg = EntityAlertConfig {
            min_severity: AlertSeverity::Low, // entity level
            override_rules: vec![AlertOverride {
                alert_type: "warning".into(),
                min_severity: AlertSeverity::Critical,
                enabled: true,
                cooldown_minutes: 0,
                max_daily: 0,
            }],
            ..Default::default()
        };
        // warning type uses override: only Critical passes
        assert!(!cfg.is_alert_suppressed("warning", AlertSeverity::Critical));
        assert!(cfg.is_alert_suppressed("warning", AlertSeverity::Medium));
        // insight type uses entity level: Low passes
        assert!(!cfg.is_alert_suppressed("insight", AlertSeverity::Low));
        assert!(cfg.is_alert_suppressed("insight", AlertSeverity::Info));
    }

    #[test]
    fn override_disabled_type_suppresses_all_severities() {
        let cfg = EntityAlertConfig {
            override_rules: vec![AlertOverride {
                alert_type: "recipe_match".into(),
                min_severity: AlertSeverity::Info,
                enabled: false,
                cooldown_minutes: 0,
                max_daily: 0,
            }],
            ..Default::default()
        };
        assert!(cfg.is_alert_suppressed("recipe_match", AlertSeverity::Critical));
    }

    // ── GlobalAlertDefaults ───────────────────────────────────────────────────

    #[test]
    fn global_defaults_are_high_severity() {
        let d = GlobalAlertDefaults::default();
        assert_eq!(d.min_severity, AlertSeverity::High);
        assert_eq!(d.cooldown_minutes, 30);
        assert_eq!(d.max_daily_alerts, 100);
        assert!(d.enabled_channels.contains(&AlertChannel::Email));
    }
}
