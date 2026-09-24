//! User preferences API — persists UI settings (dark mode, locale,
//! notification preferences) server-side so they sync across devices.
//!
//! Backs the `user_preferences` table from the core schema.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ─── Models ─────────────────────────────────────────────────────────────

/// User preference record stored in PostgreSQL.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserPreferences {
    pub user_id: String,
    /// "light" | "dark" | "system"
    pub theme: String,
    /// IETF language tag: "en", "fr", "ar", "he", etc.
    pub locale: String,
    /// Default region filter
    pub default_region: Option<String>,
    /// Dashboard layout preference
    pub dashboard_layout: String,
    /// Notification channels enabled
    pub notifications: NotificationPrefs,
    /// Columns visible per data table
    pub table_columns: HashMap<String, Vec<String>>,
    /// Custom key-value preferences
    pub custom: HashMap<String, serde_json::Value>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationPrefs {
    pub email_enabled: bool,
    pub slack_enabled: bool,
    pub browser_push: bool,
    /// Minimum severity to notify: "low" | "medium" | "high" | "critical"
    pub min_severity: String,
    /// Quiet hours (UTC)
    pub quiet_start_hour: Option<u8>,
    pub quiet_end_hour: Option<u8>,
}

impl Default for NotificationPrefs {
    fn default() -> Self {
        Self {
            email_enabled: true,
            slack_enabled: false,
            browser_push: true,
            min_severity: "medium".into(),
            quiet_start_hour: None,
            quiet_end_hour: None,
        }
    }
}

impl Default for UserPreferences {
    fn default() -> Self {
        Self {
            user_id: String::new(),
            theme: "system".into(),
            locale: "en".into(),
            default_region: None,
            dashboard_layout: "default".into(),
            notifications: NotificationPrefs::default(),
            table_columns: HashMap::new(),
            custom: HashMap::new(),
            updated_at: Utc::now(),
        }
    }
}

// ─── Request / response types ───────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct UpdatePreferencesRequest {
    pub theme: Option<String>,
    pub locale: Option<String>,
    pub default_region: Option<String>,
    pub dashboard_layout: Option<String>,
    pub notifications: Option<NotificationPrefs>,
    pub table_columns: Option<HashMap<String, Vec<String>>>,
    pub custom: Option<HashMap<String, serde_json::Value>>,
}

#[derive(Debug, Serialize)]
pub struct PreferencesResponse {
    pub preferences: UserPreferences,
}

// ─── Preference merging logic ───────────────────────────────────────────

impl UserPreferences {
    /// Apply a partial update to existing preferences.
    pub fn apply_update(&mut self, update: UpdatePreferencesRequest) {
        if let Some(theme) = update.theme {
            if matches!(theme.as_str(), "light" | "dark" | "system") {
                self.theme = theme;
            }
        }
        if let Some(locale) = update.locale {
            if is_valid_locale(&locale) {
                self.locale = locale;
            }
        }
        if let Some(region) = update.default_region {
            self.default_region = Some(region);
        }
        if let Some(layout) = update.dashboard_layout {
            self.dashboard_layout = layout;
        }
        if let Some(notifs) = update.notifications {
            self.notifications = notifs;
        }
        if let Some(columns) = update.table_columns {
            for (table, cols) in columns {
                self.table_columns.insert(table, cols);
            }
        }
        if let Some(custom) = update.custom {
            for (key, value) in custom {
                self.custom.insert(key, value);
            }
        }
        self.updated_at = Utc::now();
    }
}

/// Validate supported locales.
fn is_valid_locale(locale: &str) -> bool {
    matches!(
        locale,
        "en" | "fr" | "ar" | "he" | "zh" | "ja" | "ko" | "de" | "es" | "pt" | "it" | "ru" | "tr"
    )
}

// ─── Path constants ─────────────────────────────────────────────────────

pub const PREFERENCES_PATH: &str = "/api/preferences";

// ─── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_preferences() {
        let prefs = UserPreferences::default();
        assert_eq!(prefs.theme, "system");
        assert_eq!(prefs.locale, "en");
        assert!(prefs.default_region.is_none());
    }

    #[test]
    fn test_apply_theme_update() {
        let mut prefs = UserPreferences::default();
        prefs.apply_update(UpdatePreferencesRequest {
            theme: Some("dark".into()),
            locale: None,
            default_region: None,
            dashboard_layout: None,
            notifications: None,
            table_columns: None,
            custom: None,
        });
        assert_eq!(prefs.theme, "dark");
    }

    #[test]
    fn test_invalid_theme_rejected() {
        let mut prefs = UserPreferences::default();
        prefs.apply_update(UpdatePreferencesRequest {
            theme: Some("rainbow".into()),
            locale: None,
            default_region: None,
            dashboard_layout: None,
            notifications: None,
            table_columns: None,
            custom: None,
        });
        assert_eq!(prefs.theme, "system"); // unchanged
    }

    #[test]
    fn test_locale_validation() {
        assert!(is_valid_locale("en"));
        assert!(is_valid_locale("ar"));
        assert!(is_valid_locale("he"));
        assert!(!is_valid_locale("xx"));
        assert!(!is_valid_locale(""));
    }

    #[test]
    fn test_table_columns_merge() {
        let mut prefs = UserPreferences::default();
        prefs
            .table_columns
            .insert("warnings".into(), vec!["id".into(), "severity".into()]);
        prefs.apply_update(UpdatePreferencesRequest {
            theme: None,
            locale: None,
            default_region: None,
            dashboard_layout: None,
            notifications: None,
            table_columns: Some(HashMap::from([(
                "companies".into(),
                vec!["name".into(), "country".into()],
            )])),
            custom: None,
        });
        assert!(prefs.table_columns.contains_key("warnings"));
        assert!(prefs.table_columns.contains_key("companies"));
    }

    #[test]
    fn test_notification_defaults() {
        let notifs = NotificationPrefs::default();
        assert!(notifs.email_enabled);
        assert!(!notifs.slack_enabled);
        assert_eq!(notifs.min_severity, "medium");
    }
}
