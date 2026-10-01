use crate::*;

pub(super) async fn run_custom_job(name: &str) -> JobRun {
    let mut run = JobRun::new(JobKind::Custom(name.to_string()));
    run.start();
    let key = format!("CUSTOM_JOB_COMMAND_{}", name.to_uppercase());
    match std::env::var(&key) {
        Ok(command) if !command.trim().is_empty() => {
            let allowlist: Vec<String> = std::env::var("CUSTOM_JOB_ALLOWLIST")
                .ok()
                .map(|raw| {
                    raw.split(',')
                        .map(str::trim)
                        .filter(|entry| !entry.is_empty())
                        .map(ToOwned::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            let allowlist_refs: Vec<&str> = allowlist.iter().map(|entry| entry.as_str()).collect();
            if let Err(err) = validate_custom_command(name, &command, &allowlist_refs) {
                run.fail(&format!(
                    "custom command validation failed for {}: {}",
                    key, err
                ));
                return run;
            }
            let argv = match parse_custom_command_argv(&command) {
                Ok(argv) => argv,
                Err(err) => {
                    // validate_custom_command already parsed the command; this
                    // is a defensive re-parse that cannot normally fail.
                    run.fail(&format!(
                        "custom command could not be parsed for {}: {}",
                        key, err
                    ));
                    return run;
                }
            };

            let timeout = std::time::Duration::from_secs(
                std::env::var("CUSTOM_JOB_TIMEOUT_SECS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(300),
            );

            // Audit #75: execute WITHOUT a shell. `argv[0]` was validated as
            // an exact allowlisted binary and every later token is passed as a
            // literal argument, so shell metacharacters cannot spawn a second
            // process. The inherited environment is cleared so the command
            // cannot read worker credentials/secrets (only PATH is provided).
            let child = tokio::process::Command::new(&argv[0])
                .args(&argv[1..])
                .env_clear()
                .env("PATH", "/usr/local/bin:/usr/bin:/bin")
                .kill_on_drop(true)
                .spawn();
            let mut child = match child {
                Ok(child) => child,
                Err(err) => {
                    run.fail(&format!("custom command execution failed: {}", err));
                    return run;
                }
            };

            match tokio::time::timeout(timeout, child.wait()).await {
                Ok(Ok(exit)) if exit.success() => {
                    run.succeed(1, &format!("custom command succeeded: {}", key));
                }
                Ok(Ok(exit)) => {
                    run.fail(&format!("custom command exited with status: {}", exit));
                }
                Ok(Err(err)) => {
                    run.fail(&format!("custom command execution failed: {}", err));
                }
                Err(_) => {
                    // Never leave the timed-out process running: kill it
                    // explicitly (kill_on_drop is only the panic/drop backstop)
                    // before recording the failure.
                    if let Err(err) = child.kill().await {
                        tracing::warn!(
                            job = %name,
                            error = %err,
                            "custom command timed out and could not be killed"
                        );
                    }
                    run.fail(&format!(
                        "custom command timed out after {}s",
                        timeout.as_secs()
                    ));
                }
            }
        }
        _ => {
            run.skip(&format!("custom command env missing: {}", key));
        }
    }
    run
}

#[cfg(all(test, unix))]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn set(key: &str, value: &str) {
        std::env::set_var(key, value);
    }

    fn clear(keys: &[&str]) {
        for key in keys {
            std::env::remove_var(key);
        }
    }

    fn write_executable(path: &std::path::Path, contents: &str) {
        std::fs::write(path, contents).unwrap();
        let mut permissions = std::fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(path, permissions).unwrap();
    }

    /// #75 end-to-end: no shell interprets the command string, the inherited
    /// environment is cleared, an empty allowlist fails closed, and a timed-out
    /// process is actually killed.
    ///
    /// All env-mutating scenarios run in one test so the shared
    /// `CUSTOM_JOB_ALLOWLIST` / `CUSTOM_JOB_TIMEOUT_SECS` variables cannot race
    /// with each other.
    #[tokio::test]
    async fn custom_jobs_are_shell_free_env_cleared_and_killed_on_timeout() {
        let env_keys = [
            "CUSTOM_JOB_ALLOWLIST",
            "CUSTOM_JOB_TIMEOUT_SECS",
            "CUSTOM_JOB_TEST_SECRET",
            "CUSTOM_JOB_COMMAND_SHELLFREE",
            "CUSTOM_JOB_COMMAND_ENVCLEAR",
            "CUSTOM_JOB_COMMAND_SLOW",
            "CUSTOM_JOB_COMMAND_NOALLOW",
        ];

        let tmp = tempfile::tempdir().unwrap();

        // ── Scenario 1: shell metacharacters are inert argv data ───────────
        let marker = tmp.path().join("shell-injected");
        set("CUSTOM_JOB_ALLOWLIST", "/bin/echo");
        set("CUSTOM_JOB_TIMEOUT_SECS", "5");
        set(
            "CUSTOM_JOB_COMMAND_SHELLFREE",
            &format!("/bin/echo safe; touch {}", marker.display()),
        );
        let run = run_custom_job("shellfree").await;
        assert!(
            matches!(run.status, JobStatus::Succeeded { .. }),
            "echo exits zero; got {:?} ({})",
            run.status,
            run.notes
        );
        assert!(
            !marker.exists(),
            "shell metacharacters spawned a second process: {}",
            marker.display()
        );

        // ── Scenario 2: env_clear drops the worker environment ─────────────
        let capture = tmp.path().join("capture.sh");
        let captured = tmp.path().join("captured.txt");
        write_executable(
            &capture,
            "#!/bin/sh\nprintf '%s' \"$CUSTOM_JOB_TEST_SECRET\" > \"$1\"\n",
        );
        set("CUSTOM_JOB_TEST_SECRET", "leaked-worker-secret");
        set("CUSTOM_JOB_ALLOWLIST", &capture.display().to_string());
        set(
            "CUSTOM_JOB_COMMAND_ENVCLEAR",
            &format!("{} {}", capture.display(), captured.display()),
        );
        let run = run_custom_job("envclear").await;
        assert!(
            matches!(run.status, JobStatus::Succeeded { .. }),
            "the capture script exits zero; got {:?} ({})",
            run.status,
            run.notes
        );
        let captured_value = std::fs::read_to_string(&captured).unwrap();
        assert!(
            captured_value.is_empty(),
            "env_clear must drop the inherited environment, captured {captured_value:?}"
        );

        // ── Scenario 3: empty allowlist fails closed ───────────────────────
        set("CUSTOM_JOB_ALLOWLIST", "");
        set("CUSTOM_JOB_COMMAND_NOALLOW", "/bin/echo should-not-run");
        let run = run_custom_job("noallow").await;
        match &run.status {
            JobStatus::Failed { error, .. } => assert!(
                error.contains("CUSTOM_JOB_ALLOWLIST is empty"),
                "empty allowlist must fail closed, got: {error}"
            ),
            other => panic!("empty allowlist must fail closed, got {other:?}"),
        }

        // ── Scenario 4: a timed-out process is killed, not left running ────
        let slow = tmp.path().join("slow.sh");
        let pid_file = tmp.path().join("slow.pid");
        let late_marker = tmp.path().join("slow-finished");
        write_executable(
            &slow,
            "#!/bin/sh\necho $$ > \"$1\"\nsleep 2\necho done > \"$2\"\n",
        );
        // Pre-warm first-exec: macOS Gatekeeper's assessment of a freshly
        // written script can exceed the 1s timeout, which made this test flake
        // (the process was killed before it recorded its pid). Run it once to
        // completion, then clear the artifacts.
        let _ = std::process::Command::new(&slow)
            .arg(&pid_file)
            .arg(&late_marker)
            .status();
        let _ = std::fs::remove_file(&pid_file);
        let _ = std::fs::remove_file(&late_marker);
        set("CUSTOM_JOB_ALLOWLIST", &slow.display().to_string());
        set("CUSTOM_JOB_TIMEOUT_SECS", "1");
        set(
            "CUSTOM_JOB_COMMAND_SLOW",
            &format!(
                "{} {} {}",
                slow.display(),
                pid_file.display(),
                late_marker.display()
            ),
        );
        let run = run_custom_job("slow").await;
        match &run.status {
            JobStatus::Failed { error, .. } => {
                assert!(error.contains("timed out"), "got: {error}")
            }
            other => panic!("timeout must fail the run, got {other:?}"),
        }
        assert!(pid_file.exists(), "the script ran and recorded its pid");
        // If the process survived the timeout it would write the marker after
        // its 2s sleep; wait well past that to prove it was killed.
        tokio::time::sleep(std::time::Duration::from_millis(2_500)).await;
        assert!(
            !late_marker.exists(),
            "the timed-out process must be killed, not left running"
        );

        clear(&env_keys);
    }
}
