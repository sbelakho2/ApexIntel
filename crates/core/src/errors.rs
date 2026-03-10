use thiserror::Error;

/// Enhanced error type with remediation hints (B297).
///
/// All ApexError variants should provide actionable guidance when possible.
/// Use `with_hint()` to attach remediation advice to errors.
#[derive(Debug, Error)]
pub enum ApexError {
    #[error("configuration error: {message}{}", hint_suffix(.hint))]
    Config {
        message: String,
        hint: Option<String>,
    },

    #[error("entity not found: {kind} id={id}{}", hint_suffix(.hint))]
    NotFound {
        kind: String,
        id: String,
        hint: Option<String>,
    },

    #[error("validation error: {message}{}", hint_suffix(.hint))]
    Validation {
        message: String,
        hint: Option<String>,
    },

    #[error("parse error: {message}{}", hint_suffix(.hint))]
    Parse {
        message: String,
        hint: Option<String>,
    },

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("url parse error: {0}")]
    Url(#[from] url::ParseError),

    #[error("internal error: {message}{}", hint_suffix(.hint))]
    Internal {
        message: String,
        hint: Option<String>,
    },
}

/// Format hint as a suffix if present.
fn hint_suffix(hint: &Option<String>) -> String {
    match hint {
        Some(h) if !h.is_empty() => format!(" [hint: {}]", h),
        _ => String::new(),
    }
}

impl ApexError {
    /// Create a Config error with an optional hint.
    pub fn config(message: impl Into<String>) -> Self {
        Self::Config {
            message: message.into(),
            hint: None,
        }
    }

    /// Create a Config error with a remediation hint.
    pub fn config_with_hint(message: impl Into<String>, hint: impl Into<String>) -> Self {
        Self::Config {
            message: message.into(),
            hint: Some(hint.into()),
        }
    }

    /// Create a Validation error with an optional hint.
    pub fn validation(message: impl Into<String>) -> Self {
        Self::Validation {
            message: message.into(),
            hint: None,
        }
    }

    /// Create a Validation error with a remediation hint.
    pub fn validation_with_hint(message: impl Into<String>, hint: impl Into<String>) -> Self {
        Self::Validation {
            message: message.into(),
            hint: Some(hint.into()),
        }
    }

    /// Create a Parse error with a hint.
    pub fn parse_with_hint(message: impl Into<String>, hint: impl Into<String>) -> Self {
        Self::Parse {
            message: message.into(),
            hint: Some(hint.into()),
        }
    }

    /// Create an Internal error with a hint.
    pub fn internal_with_hint(message: impl Into<String>, hint: impl Into<String>) -> Self {
        Self::Internal {
            message: message.into(),
            hint: Some(hint.into()),
        }
    }

    /// Create a NotFound error with a hint.
    pub fn not_found_with_hint(
        kind: impl Into<String>,
        id: impl Into<String>,
        hint: impl Into<String>,
    ) -> Self {
        Self::NotFound {
            kind: kind.into(),
            id: id.into(),
            hint: Some(hint.into()),
        }
    }

    /// Attach or update the remediation hint for this error.
    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        let new_hint = Some(hint.into());
        match &mut self {
            Self::Config { hint, .. } => *hint = new_hint,
            Self::Validation { hint, .. } => *hint = new_hint,
            Self::Parse { hint, .. } => *hint = new_hint,
            Self::Internal { hint, .. } => *hint = new_hint,
            Self::NotFound { hint, .. } => *hint = new_hint,
            _ => {} // IO/Json/Url errors don't support hints
        }
        self
    }

    /// Get the hint if present.
    pub fn hint(&self) -> Option<&str> {
        match self {
            Self::Config { hint, .. }
            | Self::Validation { hint, .. }
            | Self::Parse { hint, .. }
            | Self::Internal { hint, .. }
            | Self::NotFound { hint, .. } => hint.as_deref(),
            _ => None,
        }
    }
}

pub type Result<T> = std::result::Result<T, ApexError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let e = ApexError::config("missing DB url");
        assert_eq!(e.to_string(), "configuration error: missing DB url");
    }

    #[test]
    fn test_not_found_display() {
        let e = ApexError::NotFound {
            kind: "Company".into(),
            id: "abc-123".into(),
            hint: None,
        };
        assert!(e.to_string().contains("Company"));
        assert!(e.to_string().contains("abc-123"));
    }

    #[test]
    fn test_validation_error() {
        let e = ApexError::validation("domain cannot be empty");
        assert!(e.to_string().contains("domain cannot be empty"));
    }

    #[test]
    fn test_result_alias() {
        let ok: Result<i32> = Ok(42);
        assert_eq!(ok.unwrap(), 42);

        let err: Result<i32> = Err(ApexError::Internal {
            message: "boom".into(),
            hint: None,
        });
        assert!(err.is_err());
    }

    // ── B297: remediation hint tests ──

    #[test]
    fn test_config_with_hint_includes_hint_in_message() {
        let e = ApexError::config_with_hint(
            "DATABASE_URL not set",
            "set DATABASE_URL in .env or export DATABASE_URL=postgres://...",
        );
        let msg = e.to_string();
        assert!(msg.contains("DATABASE_URL not set"));
        assert!(msg.contains("[hint:"));
        assert!(msg.contains("set DATABASE_URL"));
    }

    #[test]
    fn test_validation_with_hint() {
        let e =
            ApexError::validation_with_hint("email format invalid", "use format: user@domain.com");
        assert!(e.to_string().contains("email format invalid"));
        assert!(e.to_string().contains("[hint:"));
        assert!(e.to_string().contains("user@domain.com"));
    }

    #[test]
    fn test_with_hint_attaches_to_existing_error() {
        let e = ApexError::config("connection refused")
            .with_hint("check that database server is running on port 5432");
        assert!(e.to_string().contains("connection refused"));
        assert!(e.to_string().contains("port 5432"));
        assert_eq!(
            e.hint(),
            Some("check that database server is running on port 5432")
        );
    }

    #[test]
    fn test_hint_accessor_returns_none_when_no_hint() {
        let e = ApexError::validation("bad input");
        assert_eq!(e.hint(), None);
    }

    #[test]
    fn test_hint_accessor_returns_some_when_present() {
        let e = ApexError::parse_with_hint("invalid JSON", "check for trailing commas");
        assert_eq!(e.hint(), Some("check for trailing commas"));
    }

    #[test]
    fn test_not_found_with_hint() {
        let e = ApexError::not_found_with_hint(
            "Recipe",
            "XYZ-001",
            "recipe may have been deprecated, check lifecycle status",
        );
        assert!(e.to_string().contains("Recipe"));
        assert!(e.to_string().contains("XYZ-001"));
        assert!(e.to_string().contains("deprecated"));
    }

    #[test]
    fn test_internal_with_hint() {
        let e = ApexError::internal_with_hint(
            "mutex poisoned",
            "this usually indicates a panic in another thread, check logs",
        );
        assert!(e.to_string().contains("mutex poisoned"));
        assert!(e.to_string().contains("another thread"));
    }

    #[test]
    fn test_hint_empty_string_not_shown() {
        let e = ApexError::Config {
            message: "error".into(),
            hint: Some("".into()),
        };
        assert!(!e.to_string().contains("[hint:"));
    }
}
