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

            let timeout = std::time::Duration::from_secs(
                std::env::var("CUSTOM_JOB_TIMEOUT_SECS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(300),
            );
            let status = tokio::time::timeout(
                timeout,
                tokio::process::Command::new("sh")
                    .arg("-c")
                    .arg(&command)
                    .status(),
            )
            .await;
            match status {
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
