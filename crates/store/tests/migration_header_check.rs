//! Test for the migration identity CI gate (audit #40; extended for stale
//! body comments and frozen byte verification).
//!
//! Unlike `migrations_integration.rs` this does not touch PostgreSQL: it runs
//! `scripts/ci/check_migration_headers.sh` (the same gate Woodpecker runs) and
//! proves every direction:
//!   * the repository's migrations pass,
//!   * a header that claims a different revision than its filename fails,
//!   * a body comment or SQL string that names a different revision fails,
//!   * frozen files are only skipped when allowlisted with their exact digest,
//!   * a frozen entry that no longer exists fails (no silent deletion), and
//!   * an applied migration cannot escape the allowlist.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use sha2::{Digest, Sha256};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("canonicalize repository root")
}

fn run_gate_with_manifest(dir: &Path, manifest: Option<&Path>) -> Output {
    let script = repo_root().join("scripts/ci/check_migration_headers.sh");
    let mut command = Command::new("bash");
    command.arg(&script).arg(dir);
    if let Some(manifest) = manifest {
        command.env("FROZEN_MANIFEST", manifest);
    }
    command
        .output()
        .unwrap_or_else(|err| panic!("run {}: {err}", script.display()))
}

fn run_gate(dir: &Path) -> Output {
    run_gate_with_manifest(dir, None)
}

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "apex-migration-headers-{label}-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// Write an allowlist containing `<digest>  <filename>` for each file in the
/// fixture directory.
fn write_manifest_for(dir: &Path, files: &[&str]) -> PathBuf {
    let manifest = dir.join("frozen.txt");
    let mut body = String::from("# fixture allowlist\n");
    for file in files {
        let bytes = std::fs::read(dir.join(file)).unwrap();
        body.push_str(&format!("{}  {}\n", sha256_hex(&bytes), file));
    }
    std::fs::write(&manifest, body).unwrap();
    manifest
}

/// Minimal allowlist for fixtures without frozen files.
fn write_empty_manifest(dir: &Path) -> PathBuf {
    let manifest = dir.join("frozen.txt");
    std::fs::write(&manifest, "# no frozen migrations in this fixture\n").unwrap();
    manifest
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
    let dir = temp_dir("header");
    let source = repo_root().join("migrations/069_learning_eval_freeze_state.sql");
    let body = std::fs::read_to_string(&source).unwrap();
    let wrong = body.replacen("Migration 069", "Migration 061", 1);
    assert_ne!(body, wrong, "the fixture must rewrite the header");
    std::fs::write(dir.join("069_learning_eval_freeze_state.sql"), wrong).unwrap();
    let manifest = write_empty_manifest(&dir);

    let output = run_gate_with_manifest(&dir, Some(&manifest));
    let stderr = stderr(&output);
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

#[test]
fn body_gate_rejects_a_comment_that_names_the_wrong_revision() {
    let dir = temp_dir("body");
    std::fs::write(
        dir.join("079_pending_fixture.sql"),
        "-- Migration 079: pending fixture\n--\n-- (migration 070) stale renumbering text\nCREATE TABLE IF NOT EXISTS pending_fixture (id INT);\n",
    )
    .unwrap();
    let manifest = write_empty_manifest(&dir);

    let output = run_gate_with_manifest(&dir, Some(&manifest));
    let stderr = stderr(&output);
    assert!(
        !output.status.success(),
        "a body comment naming the wrong revision must fail the gate\nstderr:\n{stderr}"
    );
    assert!(
        stderr.contains("079_pending_fixture.sql") && stderr.contains("migration 070"),
        "the failure must name the file and the stale claim\nstderr:\n{stderr}"
    );

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn body_gate_rejects_a_stale_sql_string_identity_claim() {
    let dir = temp_dir("sql-string");
    std::fs::write(
        dir.join("079_pending_fixture.sql"),
        "-- Migration 079: pending fixture\n--\nDO $$\nBEGIN\n  RAISE WARNING 'migration 071: fixture warning';\nEND $$;\n",
    )
    .unwrap();
    let manifest = write_empty_manifest(&dir);

    let output = run_gate_with_manifest(&dir, Some(&manifest));
    let stderr = stderr(&output);
    assert!(
        !output.status.success(),
        "a SQL string naming the wrong revision must fail the gate\nstderr:\n{stderr}"
    );
    assert!(
        stderr.contains("079_pending_fixture.sql") && stderr.contains("migration 071"),
        "the failure must name the file and the stale claim\nstderr:\n{stderr}"
    );

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn body_gate_allows_plural_history_references() {
    let dir = temp_dir("history");
    std::fs::write(
        dir.join("079_pending_fixture.sql"),
        "-- Migration 079: pending fixture\n--\n-- Migrations 059/065 added the old constraints; see 030_port_missing_tables.sql.\nCREATE TABLE IF NOT EXISTS pending_fixture (id INT);\n",
    )
    .unwrap();
    let manifest = write_empty_manifest(&dir);

    let output = run_gate_with_manifest(&dir, Some(&manifest));
    assert!(
        output.status.success(),
        "explicit historical cross-references must be allowed\nstderr:\n{}",
        stderr(&output)
    );

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn frozen_allowlist_skips_production_applied_files_with_stale_claims() {
    let dir = temp_dir("frozen");
    // 065 is production-applied and its header still says "Migration 061"
    // (renumbered lineage). The explicit allowlist must skip it.
    let source = repo_root().join("migrations/065_app_users_credentials.sql");
    let bytes = std::fs::read(&source).unwrap();
    std::fs::write(dir.join("065_app_users_credentials.sql"), bytes).unwrap();
    std::fs::write(
        dir.join("079_pending_fixture.sql"),
        "-- Migration 079: pending fixture\nCREATE TABLE IF NOT EXISTS pending_fixture (id INT);\n",
    )
    .unwrap();
    let manifest = write_manifest_for(&dir, &["065_app_users_credentials.sql"]);

    let output = run_gate_with_manifest(&dir, Some(&manifest));
    assert!(
        output.status.success(),
        "an allowlisted production-applied file must be skipped\nstderr:\n{}",
        stderr(&output)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("1 pending checked") && stdout.contains("1 production-applied frozen"),
        "the summary must count the frozen file as skipped: {stdout}"
    );

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn gate_rejects_a_frozen_file_whose_bytes_changed() {
    let dir = temp_dir("frozen-tampered");
    let source = repo_root().join("migrations/065_app_users_credentials.sql");
    let bytes = std::fs::read(&source).unwrap();
    std::fs::write(dir.join("065_app_users_credentials.sql"), &bytes).unwrap();
    std::fs::write(
        dir.join("079_pending_fixture.sql"),
        "-- Migration 079: pending fixture\nCREATE TABLE IF NOT EXISTS pending_fixture (id INT);\n",
    )
    .unwrap();
    // Record the digest of the original bytes, then append to the file.
    let manifest = dir.join("frozen.txt");
    std::fs::write(
        &manifest,
        format!("{}  065_app_users_credentials.sql\n", sha256_hex(&bytes)),
    )
    .unwrap();
    std::fs::write(
        dir.join("065_app_users_credentials.sql"),
        [bytes.as_slice(), b"\n-- tampered\n"].concat(),
    )
    .unwrap();

    let output = run_gate_with_manifest(&dir, Some(&manifest));
    let stderr = stderr(&output);
    assert!(
        !output.status.success(),
        "editing an applied migration must fail the gate\nstderr:\n{stderr}"
    );
    assert!(
        stderr.contains("065_app_users_credentials.sql") && stderr.contains("immutable"),
        "the failure must name the file and the frozen contract\nstderr:\n{stderr}"
    );

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn gate_requires_allowlisted_migrations_to_exist() {
    let dir = temp_dir("missing-frozen");
    std::fs::write(
        dir.join("079_pending_fixture.sql"),
        "-- Migration 079: pending fixture\nCREATE TABLE IF NOT EXISTS pending_fixture (id INT);\n",
    )
    .unwrap();
    let manifest = dir.join("frozen.txt");
    std::fs::write(
        &manifest,
        format!("{}  065_app_users_credentials.sql\n", sha256_hex(b"absent")),
    )
    .unwrap();

    let output = run_gate_with_manifest(&dir, Some(&manifest));
    let stderr = stderr(&output);
    assert!(
        !output.status.success(),
        "a deleted/renamed applied migration must fail the gate\nstderr:\n{stderr}"
    );
    assert!(
        stderr.contains("065_app_users_credentials.sql") && stderr.contains("missing from"),
        "the failure must name the missing allowlisted file\nstderr:\n{stderr}"
    );

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn gate_rejects_a_malformed_allowlist_entry() {
    let dir = temp_dir("malformed-manifest");
    std::fs::write(
        dir.join("079_pending_fixture.sql"),
        "-- Migration 079: pending fixture\nCREATE TABLE IF NOT EXISTS pending_fixture (id INT);\n",
    )
    .unwrap();
    let manifest = dir.join("frozen.txt");
    std::fs::write(&manifest, "065_app_users_credentials.sql\n").unwrap();

    let output = run_gate_with_manifest(&dir, Some(&manifest));
    let stderr = stderr(&output);
    assert!(
        !output.status.success(),
        "a filename-only allowlist entry must fail the gate\nstderr:\n{stderr}"
    );
    assert!(
        stderr.contains("malformed frozen allowlist entry"),
        "the failure must explain the expected format\nstderr:\n{stderr}"
    );

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn gate_requires_applied_migrations_to_be_allowlisted_explicitly() {
    let dir = temp_dir("missing-allowlist");
    for file in [
        "065_app_users_credentials.sql",
        "068_drop_legacy_bookmark_fk.sql",
    ] {
        let source = repo_root().join("migrations").join(file);
        let bytes = std::fs::read(&source).unwrap();
        std::fs::write(dir.join(file), bytes).unwrap();
    }
    std::fs::write(
        dir.join("079_pending_fixture.sql"),
        "-- Migration 079: pending fixture\nCREATE TABLE IF NOT EXISTS pending_fixture (id INT);\n",
    )
    .unwrap();
    // Only 065 is allowlisted; 068 is applied but missing from the manifest.
    let manifest = write_manifest_for(&dir, &["065_app_users_credentials.sql"]);

    let output = run_gate_with_manifest(&dir, Some(&manifest));
    let stderr = stderr(&output);
    assert!(
        !output.status.success(),
        "an applied migration missing from the allowlist must fail\nstderr:\n{stderr}"
    );
    assert!(
        stderr.contains("068_drop_legacy_bookmark_fk.sql") && stderr.contains("missing from"),
        "the failure must point at the allowlist\nstderr:\n{stderr}"
    );

    std::fs::remove_dir_all(&dir).unwrap();
}
