//! Release evidence contract (P1-8).
//!
//! `scripts/ci/release_evidence.sh` is the exact-SHA release recorder: it runs
//! (or ingests from the owning pipeline steps) every canonical release gate,
//! writes `release-evidence.json`/`.txt` with the SHA, per-gate results,
//! artifact digests and timestamps, and fails when any gate failed. These
//! tests exercise the recorder without the real toolchain: the synthetic
//! self-test proves the bundle fields and the failure propagation, and the
//! `--record` path proves the pipeline-step ingestion CI uses.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("canonicalize repository root")
}

fn script() -> PathBuf {
    repo_root().join("scripts/ci/release_evidence.sh")
}

fn run(args: &[String]) -> Output {
    Command::new("bash")
        .arg(script())
        .args(args)
        .output()
        .unwrap_or_else(|error| panic!("run {}: {error}", script().display()))
}

const CANONICAL_GATES: [&str; 13] = [
    "exact-sha",
    "rustfmt",
    "clippy-default",
    "clippy-all-features",
    "unit-tests",
    "all-features-tests",
    "pg-canonical",
    "migration-bootstrap",
    "browser-integration",
    "ui-journey",
    "tailwind-assets",
    "wasm-shared",
    "container-browser",
];

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "apex-release-evidence-{label}-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn read_bundle(dir: &Path) -> serde_json::Value {
    let path = dir.join("release-evidence.json");
    let bytes =
        std::fs::read(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    serde_json::from_slice(&bytes).expect("release evidence JSON")
}

fn gate_status<'a>(bundle: &'a serde_json::Value, name: &str) -> &'a serde_json::Value {
    bundle["gates"]
        .as_array()
        .expect("gates array")
        .iter()
        .find(|gate| gate["name"] == name)
        .unwrap_or_else(|| panic!("gate {name} is part of the bundle"))
}

#[test]
fn release_evidence_documents_the_canonical_release_contract() {
    let output = run(&["--list-gates".to_string()]);
    assert!(
        output.status.success(),
        "listing gates must succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let gates: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_string)
        .collect();
    assert_eq!(
        gates,
        CANONICAL_GATES
            .iter()
            .map(|gate| gate.to_string())
            .collect::<Vec<_>>(),
        "the recorded contract must contain every release gate in order"
    );
}

#[test]
fn release_evidence_fails_when_a_gate_fails_and_records_every_field() {
    let dir = temp_dir("self-test");
    let output = run(&[
        "--self-test".to_string(),
        "--out".to_string(),
        dir.to_string_lossy().to_string(),
    ]);

    assert!(
        !output.status.success(),
        "a failing gate must fail the evidence run"
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("self-test passed"),
        "the self-test must report its own checks: {}",
        String::from_utf8_lossy(&output.stdout)
    );

    let bundle = read_bundle(&dir);
    assert_eq!(bundle["schema_version"], 1);
    assert!(bundle["generated_at"].is_string());
    assert!(bundle["git_sha"].is_string());
    assert!(bundle["pipeline_id"].is_string());
    assert!(bundle["build_timestamp"].is_string());
    assert!(bundle["artifact_digest"].is_string());
    assert!(bundle["exact_sha"].is_boolean());
    assert!(bundle["artifacts"].is_array());
    assert_eq!(bundle["status"], "failed");
    assert_eq!(bundle["complete"], false);

    assert_eq!(gate_status(&bundle, "selftest-pass")["status"], "passed");
    let failed = gate_status(&bundle, "selftest-fail");
    assert_eq!(failed["status"], "failed");
    assert_eq!(failed["source"], "executed");
    assert_eq!(failed["exit_code"], 1);

    assert!(dir.join("release-evidence.txt").is_file());
    assert!(dir.join("release-evidence.sha256").is_file());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn release_evidence_ingests_pipeline_step_results() {
    let dir = temp_dir("recorded");
    let mut args: Vec<String> = vec![
        "--sha".to_string(),
        "0123456789abcdef0123456789abcdef01234567".to_string(),
        "--out".to_string(),
        dir.to_string_lossy().to_string(),
        "--artifact-digest".to_string(),
        "sha256:feedface".to_string(),
    ];
    for gate in CANONICAL_GATES {
        args.push("--record".to_string());
        args.push(format!("{gate}=passed"));
    }

    let output = run(&args);
    assert!(
        output.status.success(),
        "recording every gate as passed must succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let bundle = read_bundle(&dir);
    assert_eq!(bundle["status"], "passed");
    assert_eq!(bundle["complete"], true);
    assert_eq!(bundle["artifact_digest"], "sha256:feedface");
    for gate in CANONICAL_GATES {
        let entry = gate_status(&bundle, gate);
        assert_eq!(entry["status"], "passed", "{gate} must be recorded");
        assert_eq!(
            entry["source"], "pipeline-step",
            "{gate} must be attributed to its pipeline step"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn release_evidence_marks_gates_that_did_not_run_as_incomplete() {
    let dir = temp_dir("not-run");
    let output = run(&[
        "--sha".to_string(),
        "0123456789abcdef0123456789abcdef01234567".to_string(),
        "--out".to_string(),
        dir.to_string_lossy().to_string(),
        "--record".to_string(),
        "rustfmt=passed".to_string(),
    ]);
    assert!(output.status.success(), "not_run gates are not failures");

    let bundle = read_bundle(&dir);
    assert_eq!(bundle["status"], "passed");
    assert_eq!(
        bundle["complete"], false,
        "a bundle with gates that did not run is not a complete release proof"
    );
    assert_eq!(gate_status(&bundle, "rustfmt")["status"], "passed");
    assert_eq!(gate_status(&bundle, "unit-tests")["status"], "not_run");
    let _ = std::fs::remove_dir_all(&dir);
}
