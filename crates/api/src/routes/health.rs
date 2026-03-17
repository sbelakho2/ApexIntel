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

pub async fn deep_health_check(
    pool: &sqlx::PgPool,
    redis_url: &str,
    nats_url: &str,
    minio_endpoint: &str,
    llm_base_url: &str,
    start_time: Instant,
) -> DeepHealthCheck {
    // Run all checks concurrently for faster response.
    let (pg_check, redis_check, nats_check, minio_check, llm_check, table_check) = tokio::join!(
        check_postgres(pool),
        check_redis(redis_url),
        check_nats(nats_url),
        check_minio(minio_endpoint),
        check_llm(llm_base_url),
        check_schema(pool),
    );

    let checks = vec![pg_check, redis_check, nats_check, minio_check, llm_check, table_check];

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
                message: Some(format!("pool: {}/{} active, {} idle", size - idle as u32, size, idle)),
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

async fn check_minio(minio_endpoint: &str) -> ComponentCheck {
    let start = Instant::now();
    match reqwest::Client::new()
        .get(format!("{}/minio/health/live", minio_endpoint))
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => ComponentCheck {
            component: "minio".into(),
            status: "ok".into(),
            latency_ms: start.elapsed().as_millis() as u64,
            message: None,
        },
        Ok(resp) => ComponentCheck {
            component: "minio".into(),
            status: "degraded".into(),
            latency_ms: start.elapsed().as_millis() as u64,
            message: Some(format!("HTTP {}", resp.status())),
        },
        Err(e) => ComponentCheck {
            component: "minio".into(),
            status: "error".into(),
            latency_ms: start.elapsed().as_millis() as u64,
            message: Some(format!("Unreachable: {}", e)),
        },
    }
}

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
}
