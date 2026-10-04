//! Deep health check module — comprehensive infrastructure validation.
//!
//! Covers: PostgreSQL, Redis, NATS, MinIO/S3, LLM server, and schema integrity.
//! The overall status is `healthy` | `degraded` | `unhealthy` based on the
//! worst component status.
//!
//! Integrate into the Axum router as:
//! ```ignore
//! .route("/api/health/deep", get(deep_health_handler))
//! ```

use serde::Serialize;
use std::time::Instant;

use apex_store::s3::ObjectStore;

// ─────────────────────────────────────────────────────────────────────────────
// Public response types
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct DeepHealthCheck {
    pub status: String,
    pub version: String,
    pub uptime_seconds: u64,
    pub checks: Vec<ComponentCheck>,
    pub timestamp: String,
}

#[derive(Debug, Serialize)]
pub struct ComponentCheck {
    pub component: String,
    pub status: String,
    pub latency_ms: u64,
    pub message: Option<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Deep health check runner
// ─────────────────────────────────────────────────────────────────────────────

#[allow(clippy::unwrap_used, clippy::expect_used)]
pub async fn deep_health_check(
    pool: &sqlx::PgPool,
    redis_url: &str,
    nats_url: &str,
    minio_endpoint: &str,
    minio_bucket: &str,
    llm_base_url: &str,
    start_time: Instant,
) -> DeepHealthCheck {
    // Run all checks concurrently for faster response.
    let (pg_check, redis_check, nats_check, minio_check, llm_check, table_check) = tokio::join!(
        check_postgres(pool),
        check_redis(redis_url),
        check_nats(nats_url),
        check_minio(minio_endpoint, minio_bucket),
        check_llm(llm_base_url),
        check_schema(pool),
    );

    let checks = vec![
        pg_check,
        redis_check,
        nats_check,
        minio_check,
        llm_check,
        table_check,
    ];

    // ── Overall status ──────────────────────────────────────────
    let overall = if checks.iter().any(|c| c.status == "error") {
        "unhealthy"
    } else if checks.iter().any(|c| c.status == "degraded") {
        "degraded"
    } else {
        "healthy"
    };

    DeepHealthCheck {
        status: overall.into(),
        version: env!("CARGO_PKG_VERSION").into(),
        uptime_seconds: start_time.elapsed().as_secs(),
        checks,
        timestamp: chrono::Utc::now().to_rfc3339(),
    }
}

async fn check_postgres(pool: &sqlx::PgPool) -> ComponentCheck {
    let start = Instant::now();
    match sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(pool)
        .await
    {
        Ok(_) => {
            let size = pool.size();
            let idle = pool.num_idle();
            ComponentCheck {
                component: "postgresql".into(),
                status: "ok".into(),
                latency_ms: start.elapsed().as_millis() as u64,
                message: Some(format!(
                    "pool: {}/{} active, {} idle",
                    size - idle as u32,
                    size,
                    idle
                )),
            }
        }
        Err(e) => ComponentCheck {
            component: "postgresql".into(),
            status: "error".into(),
            latency_ms: start.elapsed().as_millis() as u64,
            message: Some(format!("Connection failed: {}", e)),
        },
    }
}

async fn check_redis(redis_url: &str) -> ComponentCheck {
    let start = Instant::now();
    let result = async {
        let client = redis::Client::open(redis_url)?;
        let mut conn = client.get_multiplexed_async_connection().await?;
        redis::cmd("PING").query_async::<String>(&mut conn).await
    }
    .await;
    match result {
        Ok(_) => ComponentCheck {
            component: "redis".into(),
            status: "ok".into(),
            latency_ms: start.elapsed().as_millis() as u64,
            message: None,
        },
        Err(e) => ComponentCheck {
            component: "redis".into(),
            status: "error".into(),
            latency_ms: start.elapsed().as_millis() as u64,
            message: Some(format!("Failed: {}", e)),
        },
    }
}

async fn check_nats(nats_url: &str) -> ComponentCheck {
    let start = Instant::now();
    match async_nats::connect(nats_url).await {
        Ok(_) => ComponentCheck {
            component: "nats".into(),
            status: "ok".into(),
            latency_ms: start.elapsed().as_millis() as u64,
            message: None,
        },
        Err(e) => ComponentCheck {
            component: "nats".into(),
            status: "error".into(),
            latency_ms: start.elapsed().as_millis() as u64,
            message: Some(format!("Connection failed: {}", e)),
        },
    }
}

/// Outcome of probing the configured MinIO endpoint and bucket, kept separate
/// from the HTTP mapping so the mapping is unit-testable without a live MinIO.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MinioProbe {
    /// Endpoint reachable and the configured bucket exists.
    BucketAvailable,
    /// Endpoint reachable but the configured bucket does not exist.
    BucketMissing,
    /// Endpoint reachable; bucket existence could not be verified because
    /// `MINIO_ACCESS_KEY` / `MINIO_SECRET_KEY` are not configured.
    BucketUnverified,
    /// Endpoint could not be reached (or the probe failed).
    EndpointUnreachable(String),
}

/// Map a [`MinioProbe`] to the reported component check.
pub(crate) fn minio_check_from_probe(
    probe: MinioProbe,
    bucket: &str,
    latency_ms: u64,
) -> ComponentCheck {
    match probe {
        MinioProbe::BucketAvailable => ComponentCheck {
            component: "minio".into(),
            status: "ok".into(),
            latency_ms,
            message: Some(format!("endpoint reachable; bucket '{bucket}' present")),
        },
        MinioProbe::BucketMissing => ComponentCheck {
            component: "minio".into(),
            status: "degraded".into(),
            latency_ms,
            message: Some(format!(
                "bucket '{bucket}' does not exist on the configured endpoint"
            )),
        },
        MinioProbe::BucketUnverified => ComponentCheck {
            component: "minio".into(),
            status: "ok".into(),
            latency_ms,
            message: Some(format!(
                "endpoint reachable; bucket '{bucket}' not verified (set MINIO_ACCESS_KEY and \
                 MINIO_SECRET_KEY to probe it)"
            )),
        },
        MinioProbe::EndpointUnreachable(message) => ComponentCheck {
            component: "minio".into(),
            status: "error".into(),
            latency_ms,
            message: Some(format!("Unreachable: {message}")),
        },
    }
}

/// Probe the configured MinIO endpoint and bucket.
///
/// With `MINIO_ACCESS_KEY` / `MINIO_SECRET_KEY` configured this performs a
/// bucket `HEAD` through [`ObjectStore`], so the check validates the bucket
/// name the operator configured — not just that some MinIO process answers
/// `/minio/health/live`. Without credentials the bucket cannot be
/// authenticated for; the unauthenticated liveness endpoint is used and the
/// report says the bucket was not verified.
// Operator-configured MinIO health probe endpoint, not crawled content.
#[allow(clippy::disallowed_methods)]
pub(crate) async fn check_minio(minio_endpoint: &str, minio_bucket: &str) -> ComponentCheck {
    let start = Instant::now();
    let probe = match ObjectStore::from_env_credentials(minio_endpoint, minio_bucket).await {
        Ok(Some(store)) => match store.bucket_exists().await {
            Ok(true) => MinioProbe::BucketAvailable,
            Ok(false) => MinioProbe::BucketMissing,
            Err(error) => MinioProbe::EndpointUnreachable(error.to_string()),
        },
        Ok(None) => {
            match reqwest::Client::new()
                .get(format!("{minio_endpoint}/minio/health/live"))
                .timeout(std::time::Duration::from_secs(5))
                .send()
                .await
            {
                Ok(resp) if resp.status().is_success() => MinioProbe::BucketUnverified,
                Ok(resp) => MinioProbe::EndpointUnreachable(format!("HTTP {}", resp.status())),
                Err(error) => MinioProbe::EndpointUnreachable(error.to_string()),
            }
        }
        Err(error) => MinioProbe::EndpointUnreachable(error.to_string()),
    };
    minio_check_from_probe(probe, minio_bucket, start.elapsed().as_millis() as u64)
}

// Operator-configured LLM server health probe endpoint, not crawled content.
#[allow(clippy::disallowed_methods)]
async fn check_llm(llm_base_url: &str) -> ComponentCheck {
    let start = Instant::now();
    match reqwest::Client::new()
        .get(format!("{}/health", llm_base_url))
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => ComponentCheck {
            component: "llm_server".into(),
            status: "ok".into(),
            latency_ms: start.elapsed().as_millis() as u64,
            message: None,
        },
        Ok(resp) => ComponentCheck {
            component: "llm_server".into(),
            status: "degraded".into(),
            latency_ms: start.elapsed().as_millis() as u64,
            message: Some(format!("HTTP {}", resp.status())),
        },
        Err(_) => ComponentCheck {
            component: "llm_server".into(),
            status: "degraded".into(),
            latency_ms: start.elapsed().as_millis() as u64,
            message: Some("LLM server unreachable — rule-based fallback active".into()),
        },
    }
}

async fn check_schema(pool: &sqlx::PgPool) -> ComponentCheck {
    let start = Instant::now();
    match sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM information_schema.tables WHERE table_schema = 'public'",
    )
    .fetch_one(pool)
    .await
    {
        Ok(count) if count >= 15 => ComponentCheck {
            component: "schema_integrity".into(),
            status: "ok".into(),
            latency_ms: start.elapsed().as_millis() as u64,
            message: Some(format!("{} tables present", count)),
        },
        Ok(count) => ComponentCheck {
            component: "schema_integrity".into(),
            status: "degraded".into(),
            latency_ms: start.elapsed().as_millis() as u64,
            message: Some(format!(
                "Only {} tables — expected ≥15. Run migrations.",
                count
            )),
        },
        Err(e) => ComponentCheck {
            component: "schema_integrity".into(),
            status: "error".into(),
            latency_ms: start.elapsed().as_millis() as u64,
            message: Some(format!("Query failed: {}", e)),
        },
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn component_check_serializes() {
        let check = ComponentCheck {
            component: "test".into(),
            status: "ok".into(),
            latency_ms: 42,
            message: None,
        };
        let json = serde_json::to_string(&check).unwrap();
        assert!(json.contains("\"component\":\"test\""));
    }

    #[test]
    fn deep_health_check_serializes() {
        let hc = DeepHealthCheck {
            status: "healthy".into(),
            version: "1.0.0".into(),
            uptime_seconds: 3600,
            checks: vec![],
            timestamp: "2026-03-01T00:00:00Z".into(),
        };
        let json = serde_json::to_string(&hc).unwrap();
        assert!(json.contains("\"healthy\""));
    }

    #[test]
    fn minio_probe_maps_existing_bucket_to_ok() {
        let check = minio_check_from_probe(MinioProbe::BucketAvailable, "apexintel", 7);
        assert_eq!(check.component, "minio");
        assert_eq!(check.status, "ok");
        assert_eq!(check.latency_ms, 7);
        let message = check.message.expect("ok check names the bucket");
        assert!(message.contains("apexintel"), "{message}");
    }

    #[test]
    fn minio_probe_maps_missing_bucket_to_degraded_naming_the_bucket() {
        let check = minio_check_from_probe(MinioProbe::BucketMissing, "apex-raw", 11);
        assert_eq!(check.status, "degraded");
        let message = check.message.expect("degraded check explains why");
        assert!(
            message.contains("apex-raw"),
            "a missing bucket report must name the configured bucket: {message}"
        );
    }

    #[test]
    fn minio_probe_maps_unreachable_endpoint_to_error() {
        let check = minio_check_from_probe(
            MinioProbe::EndpointUnreachable("connection refused".to_string()),
            "apexintel",
            3,
        );
        assert_eq!(check.status, "error");
        let message = check.message.expect("error check carries the cause");
        assert!(message.contains("connection refused"), "{message}");
    }

    #[test]
    fn minio_probe_without_credentials_reports_unverified_bucket() {
        let check = minio_check_from_probe(MinioProbe::BucketUnverified, "apexintel", 5);
        assert_eq!(check.status, "ok");
        let message = check.message.expect("unverified check explains the gap");
        assert!(message.contains("apexintel"), "{message}");
        assert!(
            message.contains("MINIO_ACCESS_KEY"),
            "the operator must be told how to enable the bucket probe: {message}"
        );
    }
}
