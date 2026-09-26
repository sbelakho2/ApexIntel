//! Deployment profile (`APEX_PROFILE`) and the capabilities each profile must
//! prove before readiness succeeds.
//!
//! `core` is the default, so an unlabelled process never silently claims the
//! full capability set. `full` is the production-intelligence profile: every
//! measured capability (database, worker heartbeat, LLM build, embeddings,
//! NATS, search index, browser renderer) is required for readiness.

use std::fmt;
use std::str::FromStr;

pub const PROFILE_ENV_VAR: &str = "APEX_PROFILE";

/// Deployment profile selected by `APEX_PROFILE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DeploymentProfile {
    /// Database, worker heartbeat, embeddings and search index are required;
    /// NATS, browser rendering and the LLM stack stay optional.
    #[default]
    Core,
    /// Every measured capability is required, including NATS, browser
    /// rendering and an LLM-enabled build.
    Full,
}

impl DeploymentProfile {
    const CORE_REQUIRED_CAPABILITIES: &'static [&'static str] =
        &["database", "worker_heartbeat", "embeddings", "search_index"];
    const FULL_REQUIRED_CAPABILITIES: &'static [&'static str] = &[
        "database",
        "worker_heartbeat",
        "llm",
        "embeddings",
        "nats",
        "search_index",
        "browser_renderer",
    ];

    /// Resolve the profile from the environment. Unset or blank means `core`.
    pub fn from_env() -> anyhow::Result<Self> {
        match std::env::var(PROFILE_ENV_VAR) {
            Ok(raw) => Self::from_env_value(Some(&raw)),
            Err(std::env::VarError::NotPresent) => Ok(Self::default()),
            Err(error) => Err(anyhow::anyhow!("failed to read {PROFILE_ENV_VAR}: {error}")),
        }
    }

    /// Pure parsing helper so callers (and tests) can validate a raw value.
    pub fn from_env_value(raw: Option<&str>) -> anyhow::Result<Self> {
        match raw {
            None => Ok(Self::default()),
            Some(value) if value.trim().is_empty() => Ok(Self::default()),
            Some(value) => value.trim().parse().map_err(|error| {
                anyhow::anyhow!("invalid {PROFILE_ENV_VAR} value '{value}': {error}")
            }),
        }
    }

    /// Capabilities that must report `ok` for readiness under this profile.
    /// Optional capabilities (NATS, browser renderer, and the LLM stack under
    /// `core`) are excluded.
    pub fn required_capabilities(self) -> &'static [&'static str] {
        match self {
            Self::Core => Self::CORE_REQUIRED_CAPABILITIES,
            Self::Full => Self::FULL_REQUIRED_CAPABILITIES,
        }
    }

    pub fn requires_capability(self, name: &str) -> bool {
        self.required_capabilities().contains(&name)
    }

    /// `full` deployments must be built with the `llm` feature compiled in.
    pub fn requires_llm_build(self) -> bool {
        matches!(self, Self::Full)
    }
}

impl FromStr for DeploymentProfile {
    type Err = String;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "core" => Ok(Self::Core),
            "full" => Ok(Self::Full),
            other => Err(format!("expected 'core' or 'full', got '{other}'")),
        }
    }
}

impl fmt::Display for DeploymentProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Core => "core",
            Self::Full => "full",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_core_when_unset_or_blank() {
        assert_eq!(DeploymentProfile::default(), DeploymentProfile::Core);
        assert_eq!(
            DeploymentProfile::from_env_value(None).expect("unset defaults"),
            DeploymentProfile::Core
        );
        assert_eq!(
            DeploymentProfile::from_env_value(Some("   ")).expect("blank defaults"),
            DeploymentProfile::Core
        );
    }

    #[test]
    fn parses_full_case_insensitively() {
        for raw in ["full", "FULL", " Full "] {
            assert_eq!(
                DeploymentProfile::from_env_value(Some(raw)).expect("parses"),
                DeploymentProfile::Full
            );
        }
    }

    #[test]
    fn rejects_unknown_profile_values() {
        let error = DeploymentProfile::from_env_value(Some("production")).expect_err("must fail");
        assert!(error.to_string().contains("APEX_PROFILE"));
        assert!(error.to_string().contains("production"));
    }

    #[test]
    fn full_requires_every_measured_capability() {
        let required = DeploymentProfile::Full.required_capabilities();
        for name in [
            "database",
            "worker_heartbeat",
            "llm",
            "embeddings",
            "nats",
            "search_index",
            "browser_renderer",
        ] {
            assert!(required.contains(&name), "full must require {name}");
        }
    }

    #[test]
    fn core_permits_nats_browser_and_llm_optional() {
        let core = DeploymentProfile::Core;
        for name in ["nats", "browser_renderer", "llm"] {
            assert!(!core.requires_capability(name), "core permits {name}");
        }
        for name in ["database", "worker_heartbeat", "embeddings", "search_index"] {
            assert!(core.requires_capability(name), "core requires {name}");
        }
    }

    #[test]
    fn only_full_requires_an_llm_build() {
        assert!(DeploymentProfile::Full.requires_llm_build());
        assert!(!DeploymentProfile::Core.requires_llm_build());
    }

    #[test]
    fn display_matches_parse_input() {
        assert_eq!(DeploymentProfile::Core.to_string(), "core");
        assert_eq!(DeploymentProfile::Full.to_string(), "full");
    }
}
