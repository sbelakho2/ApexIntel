use std::future::Future;
use std::time::Duration;

pub(super) async fn run_stage_with_retry<T, E, F, Fut>(
    stage_name: &str,
    timeout: Duration,
    max_attempts: usize,
    mut operation: F,
) -> Result<T, String>
where
    F: FnMut(usize) -> Fut,
    Fut: Future<Output = Result<T, E>>,
    E: std::fmt::Display,
{
    let max_attempts = max_attempts.max(1);
    let mut last_error = String::new();

    for attempt in 1..=max_attempts {
        match tokio::time::timeout(timeout, operation(attempt)).await {
            Ok(Ok(value)) => return Ok(value),
            Ok(Err(error)) => {
                last_error =
                    format!("{stage_name}: attempt {attempt}/{max_attempts} failed: {error}");
            }
            Err(_) => {
                last_error = format!(
                    "{stage_name}: attempt {attempt}/{max_attempts} timed out after {}s",
                    timeout.as_secs()
                );
            }
        }

        if attempt < max_attempts {
            tokio::time::sleep(Duration::from_millis((attempt as u64) * 250)).await;
        }
    }

    Err(last_error)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    #[tokio::test]
    async fn stage_retry_recovers_from_transient_failure() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&attempts);

        let result = super::run_stage_with_retry(
            "transient_stage",
            Duration::from_millis(50),
            3,
            move |_| {
                let seen = Arc::clone(&seen);
                async move {
                    let current = seen.fetch_add(1, Ordering::SeqCst);
                    if current == 0 {
                        Err("temporary outage")
                    } else {
                        Ok::<_, &str>("recovered")
                    }
                }
            },
        )
        .await;

        assert_eq!(result.unwrap(), "recovered");
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn stage_retry_reports_timeout() {
        let result = super::run_stage_with_retry(
            "slow_stage",
            Duration::from_millis(10),
            2,
            move |_| async move {
                tokio::time::sleep(Duration::from_millis(30)).await;
                Ok::<_, &str>(())
            },
        )
        .await;

        let error = result.unwrap_err();
        assert!(error.contains("slow_stage"));
        assert!(error.contains("timed out"));
    }
}
