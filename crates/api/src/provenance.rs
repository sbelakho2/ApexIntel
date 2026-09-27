//! Deployment provenance — the build/deploy identity of the running binary.
//!
//! Production operators must be able to answer "which commit, build, pipeline,
//! and artifact is this process running, and when was it deployed?" without
//! shell access. `/api/version` answers exactly that from environment
//! variables recorded at deploy time by `scripts/ops/record_deployment.sh`
//! and by the release evidence bundle produced by
//! `scripts/ci/release_evidence.sh`.
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
    /// RFC 3339/epoch timestamp of when this binary was built, or `"unknown"`.
    pub build_timestamp: String,
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
    /// - `APEX_BUILD_TIMESTAMP` / `BUILD_TIMESTAMP` / `CI_PIPELINE_CREATED`
    /// - `APEX_CI_PIPELINE_ID` / `CI_PIPELINE_ID` / `CI_PIPELINE_NUMBER`
    /// - `APEX_ARTIFACT_DIGEST` / `ARTIFACT_DIGEST`
    /// - `APEX_DEPLOYED_AT` / `DEPLOYED_AT`
    pub fn from_env(service: &str) -> Self {
        Self::from_lookup(service, |key| std::env::var(key).ok())
    }

    /// Build the provenance from an arbitrary key/value lookup. `from_env` is
    /// this function over `std::env::var`, and tests can inject a deterministic
    /// environment without touching process-global state.
    pub fn from_lookup<F>(service: &str, lookup: F) -> Self
    where
        F: Fn(&str) -> Option<String>,
    {
        let first = |keys: &[&str]| -> String {
            for key in keys {
                if let Some(value) = lookup(key) {
                    let trimmed = value.trim();
                    if !trimmed.is_empty() {
                        return trimmed.to_string();
                    }
                }
            }
            UNKNOWN.to_string()
        };

        let git_sha = first(&["APEX_GIT_SHA", "GIT_SHA", "CI_COMMIT_SHA"]);
        let build_timestamp = first(&[
            "APEX_BUILD_TIMESTAMP",
            "BUILD_TIMESTAMP",
            "CI_PIPELINE_CREATED",
        ]);
        let ci_pipeline_id = first(&[
            "APEX_CI_PIPELINE_ID",
            "CI_PIPELINE_ID",
            "CI_PIPELINE_NUMBER",
        ]);
        let artifact_digest = first(&["APEX_ARTIFACT_DIGEST", "ARTIFACT_DIGEST"]);
        let deployed_at = first(&["APEX_DEPLOYED_AT", "DEPLOYED_AT"]);
        let configured = git_sha != UNKNOWN;

        Self {
            service: service.to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            git_sha,
            build_timestamp,
            ci_pipeline_id,
            artifact_digest,
            deployed_at,
            configured,
        }
    }

    /// One-line summary embedded in the `/api/health/capabilities` report.
    pub fn summary(&self) -> String {
        format!(
            "git {} · build {} · pipeline {} · digest {} · deployed {}",
            self.git_sha,
            self.build_timestamp,
            self.ci_pipeline_id,
            self.artifact_digest,
            self.deployed_at
        )
    }
}

const UNKNOWN: &str = "unknown";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_names_every_provenance_field() {
        let provenance = DeploymentProvenance {
            service: "apex-api".into(),
            version: "1.0.0".into(),
            git_sha: "abc123".into(),
            build_timestamp: "2026-09-27T00:00:00Z".into(),
            ci_pipeline_id: "42".into(),
            artifact_digest: "sha256:deadbeef".into(),
            deployed_at: "2026-09-26T00:00:00Z".into(),
            configured: true,
        };

        let summary = provenance.summary();
        assert!(summary.contains("abc123"));
        assert!(summary.contains("2026-09-27T00:00:00Z"));
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
            build_timestamp: UNKNOWN.into(),
            ci_pipeline_id: UNKNOWN.into(),
            artifact_digest: UNKNOWN.into(),
            deployed_at: UNKNOWN.into(),
            configured: false,
        };

        assert!(!provenance.configured);
        assert!(provenance.summary().contains("git unknown"));
        assert!(provenance.summary().contains("build unknown"));
    }

    #[test]
    fn lookup_prefers_apex_variables_and_reads_the_build_timestamp() {
        let provenance = DeploymentProvenance::from_lookup("apex-api", |key| match key {
            "APEX_GIT_SHA" => Some("0123456789abcdef".into()),
            "CI_COMMIT_SHA" => Some("ignored-generic".into()),
            "BUILD_TIMESTAMP" => Some("2026-09-27T10:00:00Z".into()),
            "APEX_CI_PIPELINE_ID" => Some("1234".into()),
            "CI_PIPELINE_NUMBER" => Some("ignored-generic".into()),
            "APEX_ARTIFACT_DIGEST" => Some("sha256:deadbeef".into()),
            "APEX_DEPLOYED_AT" => Some("2026-09-27T11:00:00Z".into()),
            _ => None,
        });

        assert_eq!(provenance.service, "apex-api");
        assert_eq!(provenance.git_sha, "0123456789abcdef");
        assert_eq!(provenance.build_timestamp, "2026-09-27T10:00:00Z");
        assert_eq!(provenance.ci_pipeline_id, "1234");
        assert_eq!(provenance.artifact_digest, "sha256:deadbeef");
        assert_eq!(provenance.deployed_at, "2026-09-27T11:00:00Z");
        assert!(provenance.configured);
    }

    #[test]
    fn lookup_reports_unknown_for_blank_values() {
        let provenance = DeploymentProvenance::from_lookup("apex-api", |_| Some("   ".into()));

        assert_eq!(provenance.git_sha, UNKNOWN);
        assert_eq!(provenance.build_timestamp, UNKNOWN);
        assert_eq!(provenance.ci_pipeline_id, UNKNOWN);
        assert_eq!(provenance.artifact_digest, UNKNOWN);
        assert!(!provenance.configured);
    }

    #[test]
    fn serialized_provenance_publishes_sha_build_and_digest() {
        let provenance = DeploymentProvenance::from_lookup("apex-api", |key| match key {
            "APEX_GIT_SHA" => Some("feedface".into()),
            "APEX_BUILD_TIMESTAMP" => Some("2026-09-27T10:00:00Z".into()),
            "APEX_ARTIFACT_DIGEST" => Some("sha256:cafebabe".into()),
            _ => None,
        });
        let json = serde_json::to_value(&provenance).expect("provenance serializes");

        assert_eq!(json["git_sha"], "feedface");
        assert_eq!(json["build_timestamp"], "2026-09-27T10:00:00Z");
        assert_eq!(json["artifact_digest"], "sha256:cafebabe");
        assert_eq!(json["configured"], true);
    }
}
