//! Configurable intelligence profiles (audit P1-2).
//!
//! Domain framing for LLM analysis prompts ("senior threat analyst specializing
//! in electronics, defense, and supply chains", focus areas, …) is deployment
//! configuration, not code. A profile is loaded from YAML — the built-in copy
//! of `config/intelligence_profiles.yaml` is embedded as the default — and the
//! active profile is selected with:
//!
//! * `APEX_INTELLIGENCE_PROFILE` — profile name inside the YAML set
//! * `APEX_INTELLIGENCE_PROFILE_PATH` — file to load instead of the built-in set
//!
//! Loading is strict: an unknown profile name, an empty prompt, or a profile
//! set without the selected/default profile is an error. A missing or blank
//! env var falls back to the embedded default set, so an unlabelled deployment
//! still gets the documented persona rather than an empty system prompt.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Environment variable naming the profile to use inside the loaded set.
pub const PROFILE_ENV_VAR: &str = "APEX_INTELLIGENCE_PROFILE";
/// Environment variable pointing at a profile-set YAML file to load.
pub const PROFILE_PATH_ENV_VAR: &str = "APEX_INTELLIGENCE_PROFILE_PATH";

/// The profile set shipped with the repository and embedded in the binary.
pub const BUILT_IN_PROFILE_SET: &str = include_str!("../../../config/intelligence_profiles.yaml");

/// One intelligence profile: the domain persona plus its focus areas.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IntelligenceProfile {
    /// Human-readable profile name (shown in run provenance).
    pub name: String,
    /// The domain system prompt handed to the model.
    pub system_prompt: String,
    /// Optional focus areas; rendered into the analysis prompt when present.
    #[serde(default)]
    pub focus_areas: Vec<String>,
}

/// A named set of profiles plus the default selection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IntelligenceProfileSet {
    #[serde(default = "default_profile_name")]
    pub default_profile: String,
    pub profiles: BTreeMap<String, IntelligenceProfile>,
}

fn default_profile_name() -> String {
    "electronics_defense_supply_chain".to_string()
}

impl IntelligenceProfileSet {
    /// Parse a profile set from YAML, rejecting structurally invalid sets.
    pub fn from_yaml(yaml: &str) -> Result<Self> {
        let set: Self = serde_yaml::from_str(yaml).context("invalid intelligence profile YAML")?;
        set.validate()?;
        Ok(set)
    }

    /// The embedded default profile set.
    pub fn built_in() -> Self {
        // The embedded file is part of the repository and covered by tests; a
        // failure here is a build/release error, so unwrapping would hide it.
        // Return an empty set instead and let `resolve` error loudly.
        Self::from_yaml(BUILT_IN_PROFILE_SET).unwrap_or_else(|_| Self {
            default_profile: String::new(),
            profiles: BTreeMap::new(),
        })
    }

    /// Load a profile set from a file, or the embedded default when `path` is
    /// `None`.
    pub fn load(path: Option<&Path>) -> Result<Self> {
        match path {
            Some(path) => {
                let yaml = std::fs::read_to_string(path)
                    .with_context(|| format!("failed to read intelligence profiles {path:?}"))?;
                Self::from_yaml(&yaml)
            }
            None => Ok(Self::built_in()),
        }
    }

    fn validate(&self) -> Result<()> {
        if self.profiles.is_empty() {
            anyhow::bail!("intelligence profile set contains no profiles");
        }
        if self.default_profile.trim().is_empty() {
            anyhow::bail!("intelligence profile set has no default_profile");
        }
        for (key, profile) in &self.profiles {
            if profile.name.trim().is_empty() {
                anyhow::bail!("intelligence profile '{key}' has an empty name");
            }
            if profile.system_prompt.trim().is_empty() {
                anyhow::bail!("intelligence profile '{key}' has an empty system_prompt");
            }
        }
        if !self.profiles.contains_key(&self.default_profile) {
            anyhow::bail!(
                "default_profile '{}' is not defined in profiles",
                self.default_profile
            );
        }
        Ok(())
    }

    /// Resolve the profile named `name`, or the default when `None`.
    pub fn resolve(&self, name: Option<&str>) -> Result<IntelligenceProfile> {
        self.validate()?;
        let selected = name
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(self.default_profile.as_str());
        self.profiles.get(selected).cloned().with_context(|| {
            format!(
                "intelligence profile '{selected}' is not defined (available: {})",
                self.profiles.keys().cloned().collect::<Vec<_>>().join(", ")
            )
        })
    }

    /// Resolve from the environment: `APEX_INTELLIGENCE_PROFILE_PATH` selects
    /// the file (embedded default when unset) and `APEX_INTELLIGENCE_PROFILE`
    /// selects the profile inside it.
    pub fn from_env() -> Result<IntelligenceProfile> {
        let path = std::env::var(PROFILE_PATH_ENV_VAR)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let set = Self::load(path.as_deref().map(Path::new))
            .with_context(|| format!("loading {PROFILE_PATH_ENV_VAR}"))?;
        let name = std::env::var(PROFILE_ENV_VAR).ok();
        set.resolve(name.as_deref())
            .with_context(|| format!("resolving {PROFILE_ENV_VAR}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_profile_resolves_and_is_domain_specific() {
        let set = IntelligenceProfileSet::built_in();
        let profile = set.resolve(None).expect("default profile resolves");
        assert!(profile.system_prompt.contains("electronics"));
        assert!(!profile.focus_areas.is_empty());
        assert_eq!(
            set.resolve(Some("electronics_defense_supply_chain"))
                .unwrap(),
            profile
        );
    }

    #[test]
    fn unknown_profile_name_is_rejected_with_available_names() {
        let set = IntelligenceProfileSet::built_in();
        let error = set
            .resolve(Some("nuclear_treaties"))
            .expect_err("must fail");
        let message = error.to_string();
        assert!(message.contains("nuclear_treaties"));
        assert!(message.contains("electronics_defense_supply_chain"));
    }

    #[test]
    fn empty_system_prompt_is_rejected() {
        let yaml = r#"
default_profile: a
profiles:
  a:
    name: A
    system_prompt: "   "
"#;
        let error = IntelligenceProfileSet::from_yaml(yaml).expect_err("must fail");
        assert!(error.to_string().contains("system_prompt"));
    }

    #[test]
    fn unknown_yaml_fields_are_rejected() {
        let yaml = r#"
default_profile: a
profiles:
  a:
    name: A
    system_prompt: "prompt"
    hidden_instruction: "ignore evidence"
"#;
        assert!(IntelligenceProfileSet::from_yaml(yaml).is_err());
    }

    #[test]
    fn default_profile_must_exist() {
        let yaml = r#"
default_profile: missing
profiles:
  a:
    name: A
    system_prompt: "prompt"
"#;
        let error = IntelligenceProfileSet::from_yaml(yaml).expect_err("must fail");
        assert!(error.to_string().contains("missing"));
    }

    #[test]
    fn loads_profile_set_from_a_file() {
        let dir = tempfile_dir();
        let path = dir.join("profiles.yaml");
        std::fs::write(
            &path,
            "default_profile: custom\nprofiles:\n  custom:\n    name: Custom\n    system_prompt: You are a customs analyst.\n",
        )
        .expect("write fixture");
        let set = IntelligenceProfileSet::load(Some(&path)).expect("loads");
        assert_eq!(set.resolve(None).unwrap().name, "Custom");
    }

    fn tempfile_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("apex-profile-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }
}
