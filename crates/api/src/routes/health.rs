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
    let mut checks = Vec::new();

    // ── 1. PostgreSQL ───────────────────────────────────────────
    let pg_start = Instant::now();
    let pg_check = match sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(pool)
        .await
    {
        Ok(_) => ComponentCheck {
            component: "postgresql".into(),
            status: "ok".into(),
            latency_ms: pg_start.elapsed().as_millis() as u64,
            message: None,
        },
        Err(e) => ComponentCheck {
            component: "postgresql".into(),
            status: "error".into(),
            latency_ms: pg_start.elapsed().as_millis() as u64,
            message: Some(format!("Connection failed: {}", e)),
        },
    };
    checks.push(pg_check);

    // ── 2. Redis ────────────────────────────────────────────────
    let redis_start = Instant::now();
    let redis_check = match redis::Client::open(redis_url) {
        Ok(client) => match client.get_multiplexed_async_connection().await {
            Ok(mut conn) => match redis::cmd("PING").query_async::<String>(&mut conn).await {
                Ok(_) => ComponentCheck {
                    component: "redis".into(),
                    status: "ok".into(),
                    latency_ms: redis_start.elapsed().as_millis() as u64,
                    message: None,
                },
                Err(e) => ComponentCheck {
                    component: "redis".into(),
                    status: "error".into(),
                    latency_ms: redis_start.elapsed().as_millis() as u64,
                    message: Some(format!("PING failed: {}", e)),
                },
            },
            Err(e) => ComponentCheck {
                component: "redis".into(),
                status: "error".into(),
                latency_ms: redis_start.elapsed().as_millis() as u64,
                message: Some(format!("Connection failed: {}", e)),
            },
        },
        Err(e) => ComponentCheck {
            component: "redis".into(),
            status: "error".into(),
            latency_ms: redis_start.elapsed().as_millis() as u64,
            message: Some(format!("Client creation failed: {}", e)),
        },
    };
    checks.push(redis_check);

    // ── 3. NATS ─────────────────────────────────────────────────
    let nats_start = Instant::now();
    let nats_check = match async_nats::connect(nats_url).await {
        Ok(_) => ComponentCheck {
            component: "nats".into(),
            status: "ok".into(),
            latency_ms: nats_start.elapsed().as_millis() as u64,
            message: None,
        },
        Err(e) => ComponentCheck {
            component: "nats".into(),
            status: "error".into(),
            latency_ms: nats_start.elapsed().as_millis() as u64,
            message: Some(format!("Connection failed: {}", e)),
        },
    };
    checks.push(nats_check);

    // ── 4. MinIO / S3 ──────────────────────────────────────────
    let minio_start = Instant::now();
    let minio_check = match reqwest::Client::new()
        .get(format!("{}/minio/health/live", minio_endpoint))
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => ComponentCheck {
            component: "minio".into(),
            status: "ok".into(),
            latency_ms: minio_start.elapsed().as_millis() as u64,
            message: None,
        },
        Ok(resp) => ComponentCheck {
            component: "minio".into(),
            status: "degraded".into(),
            latency_ms: minio_start.elapsed().as_millis() as u64,
            message: Some(format!("HTTP {}", resp.status())),
        },
        Err(e) => ComponentCheck {
            component: "minio".into(),
            status: "error".into(),
            latency_ms: minio_start.elapsed().as_millis() as u64,
            message: Some(format!("Unreachable: {}", e)),
        },
    };
    checks.push(minio_check);

    // ── 5. LLM Server ──────────────────────────────────────────
    let llm_start = Instant::now();
    let llm_check = match reqwest::Client::new()
        .get(format!("{}/health", llm_base_url))
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => ComponentCheck {
            component: "llm_server".into(),
            status: "ok".into(),
            latency_ms: llm_start.elapsed().as_millis() as u64,
            message: None,
        },
        Ok(resp) => ComponentCheck {
            component: "llm_server".into(),
            status: "degraded".into(),
            latency_ms: llm_start.elapsed().as_millis() as u64,
            message: Some(format!("HTTP {}", resp.status())),
        },
        Err(_) => ComponentCheck {
            component: "llm_server".into(),
            status: "degraded".into(),
            latency_ms: llm_start.elapsed().as_millis() as u64,
            message: Some("LLM server unreachable — rule-based fallback active".into()),
        },
    };
    checks.push(llm_check);

    // ── 6. Schema integrity ─────────────────────────────────────
    let table_start = Instant::now();
    let table_check = match sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM information_schema.tables WHERE table_schema = 'public'",
    )
    .fetch_one(pool)
    .await
    {
        Ok(count) if count >= 15 => ComponentCheck {
            component: "schema_integrity".into(),
            status: "ok".into(),
            latency_ms: table_start.elapsed().as_millis() as u64,
            message: Some(format!("{} tables present", count)),
        },
        Ok(count) => ComponentCheck {
            component: "schema_integrity".into(),
            status: "degraded".into(),
            latency_ms: table_start.elapsed().as_millis() as u64,
            message: Some(format!(
                "Only {} tables — expected ≥15. Run migrations.",
                count
            )),
        },
        Err(e) => ComponentCheck {
            component: "schema_integrity".into(),
            status: "error".into(),
            latency_ms: table_start.elapsed().as_millis() as u64,
            message: Some(format!("Query failed: {}", e)),
        },
    };
    checks.push(table_check);

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
