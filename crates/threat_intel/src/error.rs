//! Error types for the Threat Intelligence Module
//!
//! Provides comprehensive error handling with remediation hints following
//! the ApexIntel error pattern (B297).

use thiserror::Error;

/// Enhanced error type with remediation hints for Threat Intelligence Module.
///
/// All ThreatIntelError variants provide actionable guidance when possible.
#[derive(Debug, Error)]
pub enum ThreatIntelError {
    #[error("threat actor not found: {actor_id}{}", hint_suffix(.hint))]
    ThreatActorNotFound {
        actor_id: String,
        hint: Option<String>,
    },

    #[error("TTP not found: {ttp_id} - {reason}{}", hint_suffix(.hint))]
    TtpNotFound {
        ttp_id: String,
        reason: String,
        hint: Option<String>,
    },

    #[error("industry sector error: {message}{}", hint_suffix(.hint))]
    IndustrySector {
        message: String,
        hint: Option<String>,
    },

    #[error("attack surface analysis error: {message}{}", hint_suffix(.hint))]
    AttackSurface {
        message: String,
        hint: Option<String>,
    },

    #[error("supply chain risk error: {message}{}", hint_suffix(.hint))]
    SupplyChainRisk {
        message: String,
        hint: Option<String>,
    },

    #[error("competitive intelligence error: {message}{}", hint_suffix(.hint))]
    CompetitiveIntelligence {
        message: String,
        hint: Option<String>,
    },

    #[error("validation error: {message}{}", hint_suffix(.hint))]
    Validation {
        message: String,
        hint: Option<String>,
    },

    #[error("data integrity error: {message}{}", hint_suffix(.hint))]
    DataIntegrity {
        message: String,
        hint: Option<String>,
    },

    #[error("configuration error: {message}{}", hint_suffix(.hint))]
    Configuration {
        message: String,
        hint: Option<String>,
    },

    #[error("internal error: {message}{}", hint_suffix(.hint))]
    Internal {
        message: String,
        hint: Option<String>,
    },

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("URL parse error: {0}")]
    Url(#[from] url::ParseError),
}

/// Format hint as a suffix if present.
fn hint_suffix(hint: &Option<String>) -> String {
    match hint {
        Some(h) if !h.is_empty() => format!(" [hint: {}]", h),
        _ => String::new(),
    }
}

impl ThreatIntelError {
    // ── Factory constructors ────────────────────────────────────────────────

    /// Create a ThreatActorNotFound error with an optional hint.
    pub fn threat_actor_not_found(actor_id: impl Into<String>) -> Self {
        Self::ThreatActorNotFound {
            actor_id: actor_id.into(),
            hint: None,
        }
    }

    /// Create a ThreatActorNotFound error with a remediation hint.
    pub fn threat_actor_not_found_with_hint(
        actor_id: impl Into<String>,
        hint: impl Into<String>,
    ) -> Self {
        Self::ThreatActorNotFound {
            actor_id: actor_id.into(),
            hint: Some(hint.into()),
        }
    }

    /// Create a TtpNotFound error.
    pub fn ttp_not_found(ttp_id: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::TtpNotFound {
            ttp_id: ttp_id.into(),
            reason: reason.into(),
            hint: None,
        }
    }

    /// Create an IndustrySector error.
    pub fn industry_sector(message: impl Into<String>) -> Self {
        Self::IndustrySector {
            message: message.into(),
            hint: None,
        }
    }

    /// Create an AttackSurface error.
    pub fn attack_surface(message: impl Into<String>) -> Self {
        Self::AttackSurface {
            message: message.into(),
            hint: None,
        }
    }

    /// Create a SupplyChainRisk error.
    pub fn supply_chain_risk(message: impl Into<String>) -> Self {
        Self::SupplyChainRisk {
            message: message.into(),
            hint: None,
        }
    }

    /// Create a CompetitiveIntelligence error.
    pub fn competitive_intelligence(message: impl Into<String>) -> Self {
        Self::CompetitiveIntelligence {
            message: message.into(),
            hint: None,
        }
    }

    /// Create a Validation error.
    pub fn validation(message: impl Into<String>) -> Self {
        Self::Validation {
            message: message.into(),
            hint: None,
        }
    }

    /// Create a DataIntegrity error.
    pub fn data_integrity(message: impl Into<String>) -> Self {
        Self::DataIntegrity {
            message: message.into(),
            hint: None,
        }
    }

    /// Create a Configuration error.
    pub fn configuration(message: impl Into<String>) -> Self {
        Self::Configuration {
            message: message.into(),
            hint: None,
        }
    }

    /// Create an Internal error.
    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal {
            message: message.into(),
            hint: None,
        }
    }

    // ── Hint attachment ─────────────────────────────────────────────────────

    /// Attach or update the remediation hint for this error.
    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        let new_hint = Some(hint.into());
        match &mut self {
            Self::ThreatActorNotFound { hint: h, .. } => *h = new_hint,
            Self::TtpNotFound { hint: h, .. } => *h = new_hint,
            Self::IndustrySector { hint: h, .. } => *h = new_hint,
            Self::AttackSurface { hint: h, .. } => *h = new_hint,
            Self::SupplyChainRisk { hint: h, .. } => *h = new_hint,
            Self::CompetitiveIntelligence { hint: h, .. } => *h = new_hint,
            Self::Validation { hint: h, .. } => *h = new_hint,
            Self::DataIntegrity { hint: h, .. } => *h = new_hint,
            Self::Configuration { hint: h, .. } => *h = new_hint,
            Self::Internal { hint: h, .. } => *h = new_hint,
            _ => {} // IO/Json/Url errors don't support hints
        }
        self
    }

    /// Get the hint if present.
    pub fn hint(&self) -> Option<&str> {
        match self {
            Self::ThreatActorNotFound { hint, .. }
            | Self::TtpNotFound { hint, .. }
            | Self::IndustrySector { hint, .. }
            | Self::AttackSurface { hint, .. }
            | Self::SupplyChainRisk { hint, .. }
            | Self::CompetitiveIntelligence { hint, .. }
            | Self::Validation { hint, .. }
            | Self::DataIntegrity { hint, .. }
            | Self::Configuration { hint, .. }
            | Self::Internal { hint, .. } => hint.as_deref(),
            _ => None,
        }
    }
}

/// Result type alias for Threat Intelligence operations.
pub type Result<T> = std::result::Result<T, ThreatIntelError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_threat_actor_not_found_display() {
        let e = ThreatIntelError::threat_actor_not_found("APT29");
        assert!(e.to_string().contains("APT29"));
    }

    #[test]
    fn test_threat_actor_with_hint() {
        let e = ThreatIntelError::threat_actor_not_found_with_hint(
            "APT29",
            "Check if the actor ID uses the correct format (e.g., APT-29)",
        );
        let msg = e.to_string();
        assert!(msg.contains("APT29"));
        assert!(msg.contains("[hint:"));
    }

    #[test]
    fn test_validation_error() {
        let e = ThreatIntelError::validation("industry sector cannot be empty");
        assert!(e.to_string().contains("industry sector cannot be empty"));
    }

    #[test]
    fn test_with_hint() {
        let e = ThreatIntelError::configuration("missing API key")
            .with_hint("set THREAT_INTEL_API_KEY in configuration");
        assert!(e.to_string().contains("missing API key"));
        assert!(e.hint().is_some());
    }

    #[test]
    fn test_hint_accessor() {
        let e = ThreatIntelError::attack_surface("scan timeout");
        assert_eq!(e.hint(), None);
    }
}
