//! Test for the migration header CI gate (audit #40).
//!
//! Unlike `migrations_integration.rs` this does not touch PostgreSQL: it runs
//! `scripts/ci/check_migration_headers.sh` (the same gate Woodpecker runs) and
//! proves both directions: the repository's migrations pass, and a header that
//! claims a different revision than its filename fails loudly.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("canonicalize repository root")
}

fn run_gate(dir: &Path) -> Output {
    let script = repo_root().join("scripts/ci/check_migration_headers.sh");
    Command::new("bash")
        .arg(&script)
        .arg(dir)
        .output()
        .unwrap_or_else(|err| panic!("run {}: {err}", script.display()))
}

#[test]
fn repository_migration_headers_pass_the_gate() {
    let output = run_gate(&repo_root().join("migrations"));
    assert!(
        output.status.success(),
        "migration header gate failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn header_gate_rejects_a_header_that_names_the_wrong_revision() {
    let dir = std::env::temp_dir().join(format!("apex-migration-headers-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();

    let source = repo_root().join("migrations/069_learning_eval_freeze_state.sql");
    let body = std::fs::read_to_string(&source).unwrap();
    let wrong = body.replacen("Migration 069", "Migration 061", 1);
    assert_ne!(body, wrong, "the fixture must rewrite the header");
    std::fs::write(dir.join("069_learning_eval_freeze_state.sql"), wrong).unwrap();

    let output = run_gate(&dir);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "a header naming the wrong revision must fail the gate\nstderr:\n{stderr}"
    );
    assert!(
        stderr.contains("069_learning_eval_freeze_state.sql"),
        "the failure must name the offending file\nstderr:\n{stderr}"
    );

    std::fs::remove_dir_all(&dir).unwrap();
}
