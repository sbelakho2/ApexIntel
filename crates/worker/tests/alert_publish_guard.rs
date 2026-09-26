//! Regression tests for `scripts/ci/check_alert_publish.sh`.
//!
//! The guard is the CI enforcement of the "single alert publication path"
//! invariant: `NatsPublisher::publish_alert` may only be called from the alert
//! transport seam, never from a job. These tests run the real script against
//! fixture trees to prove it fails on a violation (not just that it exists).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root resolves")
}

fn guard_script() -> PathBuf {
    repo_root().join("scripts/ci/check_alert_publish.sh")
}

fn run_guard(scan_root: &Path) -> std::process::Output {
    Command::new("bash")
        .arg(guard_script())
        .env("ALERT_PUBLISH_GUARD_ROOT", scan_root)
        .output()
        .expect("failed to run the alert-publish guard script")
}

fn fixture_root(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let root = std::env::temp_dir().join(format!(
        "alert-publish-guard-{label}-{}-{unique}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("crates/worker/src/job_execution")).expect("fixture tree");
    fs::create_dir_all(root.join("crates/worker/src")).expect("fixture tree");
    root
}

#[test]
fn guard_passes_on_the_real_worker_tree() {
    let output = run_guard(&repo_root());
    assert!(
        output.status.success(),
        "alert-publish guard must pass on the repository tree: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn guard_fails_when_a_job_publishes_directly() {
    let root = fixture_root("job");
    fs::write(
        root.join("crates/worker/src/job_execution/rogue_job.rs"),
        "async fn rogue(publisher: NatsPublisher, event: &AlertEvent) {\n    \
         let _ = publisher.publish_alert(event).await;\n}\n",
    )
    .expect("write violating fixture");

    let output = run_guard(&root);
    assert!(
        !output.status.success(),
        "a job publishing directly must fail the guard"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("job calls publish_alert"),
        "the guard must name the violating job call, got: {stderr}"
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn guard_fails_outside_the_transport_module() {
    let root = fixture_root("rogue");
    fs::write(
        root.join("crates/worker/src/rogue_pipeline.rs"),
        "async fn rogue(publisher: NatsPublisher, event: &AlertEvent) {\n    \
         let _ = publisher.publish_alert(event).await;\n}\n",
    )
    .expect("write violating fixture");

    let output = run_guard(&root);
    assert!(
        !output.status.success(),
        "a non-transport module publishing directly must fail the guard"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("outside"),
        "the guard must report the out-of-module publish, got: {stderr}"
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn guard_allows_the_transport_seam_and_publisher_definition() {
    let root = fixture_root("allowed");
    fs::write(
        root.join("crates/worker/src/alert_transport.rs"),
        "impl AlertTransport {\n    \
         async fn publish_event(&self, e: &AlertEvent, id: &str) {\n        \
         let _ = self.publisher.publish_alert_with_msg_id(e, Some(id)).await;\n    }\n}\n",
    )
    .expect("write allowed fixture");
    fs::write(
        root.join("crates/worker/src/nats_stream.rs"),
        "impl NatsPublisher {\n    \
         pub async fn publish_alert(&self, alert: &AlertEvent) { let _ = alert; }\n}\n",
    )
    .expect("write allowed fixture");

    let output = run_guard(&root);
    assert!(
        output.status.success(),
        "the transport seam and publisher definition are the allowed call sites: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let _ = fs::remove_dir_all(&root);
}
