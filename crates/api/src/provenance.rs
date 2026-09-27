//! Deployment provenance — the build/deploy identity of the running binary.
//!
//! Production operators must be able to answer "which commit, pipeline, and
//! artifact is this process running, and when was it deployed?" without shell
//! access. `/api/version` (and the `deployment` capability in
//! `/api/health/capabilities`) answers exactly that from environment variables
//! recorded at deploy time by `scripts/ops/record_deployment.sh`.
//!
//! Every field is optional: a local `cargo run` reports `"unknown"` rather than
//! inventing values, and `configured` is only true when the identity-defining
//! `git_sha` is present.

use serde::Serialize;

/// Deployment identity of the running process.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DeploymentProvenance {
    /// Human-readable service name (`apex-api`).
    pub service: String,
    /// Cargo package version.
    pub version: String,
    /// Git commit the binary was built from, or `"unknown"`.
    pub git_sha: String,
    /// CI pipeline identifier (Woodpecker pipeline number/id), or `"unknown"`.
    pub ci_pipeline_id: String,
    /// Content digest of the deployed artifact, or `"unknown"`.
    pub artifact_digest: String,
    /// RFC 3339 timestamp of when this artifact was deployed, or `"unknown"`.
    pub deployed_at: String,
    /// True when a git SHA was recorded for this process.
    pub configured: bool,
}

impl DeploymentProvenance {
    /// Read provenance from the process environment.
    ///
    /// Recognised variables (first non-empty wins):
    /// - `APEX_GIT_SHA` / `GIT_SHA` / `CI_COMMIT_SHA`
    /// - `APEX_CI_PIPELINE_ID` / `CI_PIPELINE_ID` / `CI_PIPELINE_NUMBER`
    /// - `APEX_ARTIFACT_DIGEST` / `ARTIFACT_DIGEST`
    /// - `APEX_DEPLOYED_AT` / `DEPLOYED_AT`
    pub fn from_env(service: &str) -> Self {
        let git_sha = first_env(&["APEX_GIT_SHA", "GIT_SHA", "CI_COMMIT_SHA"]);
        let ci_pipeline_id = first_env(&[
            "APEX_CI_PIPELINE_ID",
            "CI_PIPELINE_ID",
            "CI_PIPELINE_NUMBER",
        ]);
        let artifact_digest = first_env(&["APEX_ARTIFACT_DIGEST", "ARTIFACT_DIGEST"]);
        let deployed_at = first_env(&["APEX_DEPLOYED_AT", "DEPLOYED_AT"]);
        let configured = git_sha != UNKNOWN;

        Self {
            service: service.to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            git_sha,
            ci_pipeline_id,
            artifact_digest,
            deployed_at,
            configured,
        }
    }

    /// One-line summary embedded in the `/api/health/capabilities` report.
    pub fn summary(&self) -> String {
        format!(
            "git {} · pipeline {} · digest {} · deployed {}",
            self.git_sha, self.ci_pipeline_id, self.artifact_digest, self.deployed_at
        )
    }
}

const UNKNOWN: &str = "unknown";

fn first_env(keys: &[&str]) -> String {
    for key in keys {
        if let Ok(value) = std::env::var(key) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
    }
    UNKNOWN.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_names_every_provenance_field() {
        let provenance = DeploymentProvenance {
            service: "apex-api".into(),
            version: "1.0.0".into(),
            git_sha: "abc123".into(),
            ci_pipeline_id: "42".into(),
            artifact_digest: "sha256:deadbeef".into(),
            deployed_at: "2026-09-26T00:00:00Z".into(),
            configured: true,
        };

        let summary = provenance.summary();
        assert!(summary.contains("abc123"));
        assert!(summary.contains("42"));
        assert!(summary.contains("sha256:deadbeef"));
        assert!(summary.contains("2026-09-26T00:00:00Z"));
    }

    #[test]
    fn default_provenance_is_explicitly_unknown() {
        let provenance = DeploymentProvenance {
            service: "apex-api".into(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            git_sha: UNKNOWN.into(),
            ci_pipeline_id: UNKNOWN.into(),
            artifact_digest: UNKNOWN.into(),
            deployed_at: UNKNOWN.into(),
            configured: false,
        };

        assert!(!provenance.configured);
        assert!(provenance.summary().contains("git unknown"));
    }
}
