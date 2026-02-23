use anyhow::Result;
use apex_api::responses::{aggregate_health, ComponentHealth, HealthResponse, HealthStatus};
use apex_api::routes;
use axum::{routing::get, Json, Router};
use chrono::{DateTime, Utc};
use std::sync::OnceLock;
use tower_http::{cors::CorsLayer, trace::TraceLayer};

static STARTED_AT: OnceLock<DateTime<Utc>> = OnceLock::new();

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_target(false).init();
    STARTED_AT.get_or_init(Utc::now);

    let app = Router::new()
        .route("/api/health", get(health))
        .route("/api/endpoints", get(endpoints))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http());

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await?;
    tracing::info!("ApexIntel API listening on :8080");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn health() -> Json<HealthResponse> {
    let started_at = STARTED_AT.get_or_init(Utc::now);
    let uptime_secs = Utc::now()
        .signed_duration_since(*started_at)
        .num_seconds()
        .max(0) as u64;

    let checks = vec![
        ComponentHealth {
            name: "api".to_string(),
            status: HealthStatus::Healthy,
            message: None,
        },
        ComponentHealth {
            name: "routes".to_string(),
            status: HealthStatus::Healthy,
            message: Some(format!("{} endpoints registered", routes::all_endpoints().len())),
        },
    ];

    Json(HealthResponse {
        status: aggregate_health(&checks),
        version: env!("CARGO_PKG_VERSION").to_string(),
        uptime_secs,
        checks,
    })
}

async fn endpoints() -> Json<Vec<routes::EndpointDef>> {
    Json(routes::all_endpoints())
}
