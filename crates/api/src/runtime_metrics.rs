use crate::*;

use axum::response::IntoResponse;
use lazy_static::lazy_static;
use prometheus::{Encoder, GaugeVec, HistogramOpts, HistogramVec, IntCounterVec, Opts, Registry, TextEncoder};

lazy_static! {
    static ref METRICS_REGISTRY: Registry = Registry::new();

    static ref HTTP_REQUESTS_TOTAL: IntCounterVec = IntCounterVec::new(
        Opts::new("http_requests_total", "Total number of HTTP requests"),
        &["method", "path", "status"]
    ).unwrap_or_else(|err| panic!("failed to create http_requests_total: {err}"));

    static ref HTTP_REQUEST_DURATION_SECONDS: HistogramVec = HistogramVec::new(
        HistogramOpts::new("http_request_duration_seconds", "HTTP request duration in seconds")
            .buckets(vec![0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0]),
        &["method", "path"]
    ).unwrap_or_else(|err| panic!("failed to create http_request_duration_seconds: {err}"));

    static ref WARNINGS_TOTAL: GaugeVec = GaugeVec::new(
        Opts::new("apexintel_warnings_total", "Total warnings by severity"),
        &["severity"]
    ).unwrap_or_else(|err| panic!("failed to create apexintel_warnings_total: {err}"));

    static ref INSIGHTS_TOTAL: GaugeVec = GaugeVec::new(
        Opts::new("apexintel_insights_total", "Total insights by category"),
        &["category"]
    ).unwrap_or_else(|err| panic!("failed to create apexintel_insights_total: {err}"));

    static ref COMPANIES_TRACKED: prometheus::Gauge = prometheus::Gauge::new(
        "apexintel_companies_tracked", "Number of companies tracked"
    ).unwrap_or_else(|err| panic!("failed to create apexintel_companies_tracked: {err}"));

    static ref PERSONS_TRACKED: prometheus::Gauge = prometheus::Gauge::new(
        "apexintel_persons_tracked", "Number of persons of interest tracked"
    ).unwrap_or_else(|err| panic!("failed to create apexintel_persons_tracked: {err}"));

    static ref RECIPES_ACTIVE: prometheus::Gauge = prometheus::Gauge::new(
        "apexintel_recipes_active", "Number of active recipes"
    ).unwrap_or_else(|err| panic!("failed to create apexintel_recipes_active: {err}"));

    static ref CRAWL_QUEUE_SIZE: prometheus::Gauge = prometheus::Gauge::new(
        "apexintel_crawl_queue_size", "Current crawl queue size"
    ).unwrap_or_else(|err| panic!("failed to create apexintel_crawl_queue_size: {err}"));

    static ref RECIPE_FIRINGS_TOTAL: IntCounterVec = IntCounterVec::new(
        Opts::new("apexintel_recipe_firings_total", "Total recipe firings"),
        &["recipe_id", "outcome"]
    ).unwrap_or_else(|err| panic!("failed to create apexintel_recipe_firings_total: {err}"));
}

pub(crate) fn init_metrics() {
    METRICS_REGISTRY.register(Box::new(HTTP_REQUESTS_TOTAL.clone())).ok();
    METRICS_REGISTRY.register(Box::new(HTTP_REQUEST_DURATION_SECONDS.clone())).ok();
    METRICS_REGISTRY.register(Box::new(WARNINGS_TOTAL.clone())).ok();
    METRICS_REGISTRY.register(Box::new(INSIGHTS_TOTAL.clone())).ok();
    METRICS_REGISTRY.register(Box::new(COMPANIES_TRACKED.clone())).ok();
    METRICS_REGISTRY.register(Box::new(PERSONS_TRACKED.clone())).ok();
    METRICS_REGISTRY.register(Box::new(RECIPES_ACTIVE.clone())).ok();
    METRICS_REGISTRY.register(Box::new(CRAWL_QUEUE_SIZE.clone())).ok();
    METRICS_REGISTRY.register(Box::new(RECIPE_FIRINGS_TOTAL.clone())).ok();
}

pub(crate) async fn metrics(State(state): State<AppState>) -> impl IntoResponse {
    if let Ok(stats) = state.store.get_dashboard_stats().await {
        COMPANIES_TRACKED.set(stats.total_companies as f64);
        PERSONS_TRACKED.set(stats.total_persons as f64);
        RECIPES_ACTIVE.set(stats.active_recipes as f64);
        WARNINGS_TOTAL.with_label_values(&["total"]).set(stats.total_warnings as f64);
        WARNINGS_TOTAL.with_label_values(&["active"]).set(stats.unacknowledged_warnings as f64);
        INSIGHTS_TOTAL.with_label_values(&["total"]).set(stats.total_insights as f64);
    }

    let encoder = TextEncoder::new();
    let metric_families = METRICS_REGISTRY.gather();
    let mut buffer = Vec::new();
    if let Err(err) = encoder.encode(&metric_families, &mut buffer) {
        tracing::error!("Failed to encode metrics: {}", err);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/plain")],
            "Failed to encode metrics".to_string(),
        );
    }

    let output = String::from_utf8(buffer).unwrap_or_else(|_| "Encoding error".to_string());
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/plain; version=0.0.4; charset=utf-8")],
        output,
    )
}