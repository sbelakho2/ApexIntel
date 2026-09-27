//! Release provenance contract (P1-8).
//!
//! `/api/version` must let an operator match a deployed binary to its release
//! evidence bundle: the running git SHA, the build timestamp, the CI pipeline
//! id (all environment-provided at build/deploy time) and the artifact digest.
//! The handler returns `DeploymentProvenance::from_env`, so these tests pin the
//! lookup contract with an injected environment and verify the route is part
//! of the public catalogued surface.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use apex_api::provenance::DeploymentProvenance;
use apex_api::routes::{all_endpoints, paths};

#[test]
fn api_version_is_a_public_catalogued_endpoint() {
    let endpoint = all_endpoints()
        .into_iter()
        .find(|endpoint| endpoint.path == paths::VERSION)
        .expect("/api/version must be catalogued");

    assert!(!endpoint.auth_required);
    assert_eq!(endpoint.min_role, "public");
    assert!(
        endpoint.description.contains("build timestamp"),
        "the catalogue must document the build timestamp: {}",
        endpoint.description
    );
    assert!(endpoint.description.contains("artifact digest"));
}

#[test]
fn version_payload_exposes_sha_build_pipeline_and_digest() {
    let provenance = DeploymentProvenance::from_lookup("apex-api", |key| match key {
        "APEX_GIT_SHA" => Some("c0ffee1234567890".into()),
        "APEX_BUILD_TIMESTAMP" => Some("2026-09-27T12:00:00Z".into()),
        "APEX_CI_PIPELINE_ID" => Some("4242".into()),
        "APEX_ARTIFACT_DIGEST" => Some("sha256:0123456789abcdef".into()),
        "APEX_DEPLOYED_AT" => Some("2026-09-27T12:05:00Z".into()),
        _ => None,
    });

    let json = serde_json::to_value(&provenance).expect("provenance serializes");
    assert_eq!(json["service"], "apex-api");
    assert_eq!(json["git_sha"], "c0ffee1234567890");
    assert_eq!(json["build_timestamp"], "2026-09-27T12:00:00Z");
    assert_eq!(json["ci_pipeline_id"], "4242");
    assert_eq!(json["artifact_digest"], "sha256:0123456789abcdef");
    assert_eq!(json["deployed_at"], "2026-09-27T12:05:00Z");
    assert_eq!(json["configured"], true);
}

#[test]
fn version_payload_reports_unknown_rather_than_guessing() {
    let provenance = DeploymentProvenance::from_lookup("apex-api", |_| None);

    let json = serde_json::to_value(&provenance).expect("provenance serializes");
    assert_eq!(json["git_sha"], "unknown");
    assert_eq!(json["build_timestamp"], "unknown");
    assert_eq!(json["artifact_digest"], "unknown");
    assert_eq!(json["configured"], false);
}
