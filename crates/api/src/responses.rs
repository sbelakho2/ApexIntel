//! Responses — API envelope types, error formatting, and response builders.
//!
//! Provides the standard wrapper for all JSON API responses, including
//! success envelopes, error bodies, and status code mapping.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ────────────────────────────────────────────
// Success envelope
// ────────────────────────────────────────────

/// Standard API response envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiResponse<T: Serialize> {
    pub success: bool,
    pub data: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ApiError>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<ResponseMeta>,
}

/// Build a success response.
pub fn success<T: Serialize>(data: T) -> ApiResponse<T> {
    ApiResponse {
        success: true,
        data: Some(data),
        error: None,
        meta: Some(ResponseMeta::now()),
    }
}

/// Build a success response with custom meta.
pub fn success_with_meta<T: Serialize>(data: T, meta: ResponseMeta) -> ApiResponse<T> {
    ApiResponse {
        success: true,
        data: Some(data),
        error: None,
        meta: Some(meta),
    }
}

/// Build an error response with no data.
pub fn error_response<T: Serialize>(error: ApiError) -> ApiResponse<T> {
    ApiResponse {
        success: false,
        data: None,
        error: Some(error),
        meta: Some(ResponseMeta::now()),
    }
}

// ────────────────────────────────────────────
// Error types
// ────────────────────────────────────────────

/// Structured API error.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiError {
    pub code: ErrorCode,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<HashMap<String, String>>,
}

impl ApiError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: None,
        }
    }

    pub fn with_details(mut self, details: HashMap<String, String>) -> Self {
        self.details = Some(details);
        self
    }

    pub fn not_found(resource: &str, id: &str) -> Self {
        Self::new(
            ErrorCode::NotFound,
            format!("{} '{}' not found", resource, id),
        )
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::BadRequest, message)
    }

    pub fn unauthorized() -> Self {
        Self::new(ErrorCode::Unauthorized, "Authentication required")
    }

    pub fn forbidden(reason: impl Into<String>) -> Self {
        Self::new(ErrorCode::Forbidden, reason)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InternalError, message)
    }

    pub fn rate_limited(retry_after_secs: u32) -> Self {
        let mut details = HashMap::new();
        details.insert("retry_after".to_string(), retry_after_secs.to_string());
        Self::new(
            ErrorCode::RateLimited,
            format!("Rate limit exceeded. Retry after {} seconds", retry_after_secs),
        )
        .with_details(details)
    }

    pub fn validation(field: &str, message: impl Into<String>) -> Self {
        let mut details = HashMap::new();
        details.insert("field".to_string(), field.to_string());
        Self::new(ErrorCode::ValidationError, message).with_details(details)
    }

    /// Map error code to HTTP status.
    pub fn http_status(&self) -> u16 {
        self.code.http_status()
    }
}

/// Error codes matching HTTP semantics.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ErrorCode {
    BadRequest,
    Unauthorized,
    Forbidden,
    NotFound,
    RateLimited,
    ValidationError,
    Conflict,
    InternalError,
    ServiceUnavailable,
}

impl ErrorCode {
    pub fn http_status(&self) -> u16 {
        match self {
            Self::BadRequest => 400,
            Self::Unauthorized => 401,
            Self::Forbidden => 403,
            Self::NotFound => 404,
            Self::RateLimited => 429,
            Self::ValidationError => 422,
            Self::Conflict => 409,
            Self::InternalError => 500,
            Self::ServiceUnavailable => 503,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::BadRequest => "bad_request",
            Self::Unauthorized => "unauthorized",
            Self::Forbidden => "forbidden",
            Self::NotFound => "not_found",
            Self::RateLimited => "rate_limited",
            Self::ValidationError => "validation_error",
            Self::Conflict => "conflict",
            Self::InternalError => "internal_error",
            Self::ServiceUnavailable => "service_unavailable",
        }
    }
}

// ────────────────────────────────────────────
// Response metadata
// ────────────────────────────────────────────

/// Metadata attached to every response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseMeta {
    pub timestamp: DateTime<Utc>,
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

impl ResponseMeta {
    pub fn now() -> Self {
        Self {
            timestamp: Utc::now(),
            version: "1.0.0".to_string(),
            request_id: None,
            duration_ms: None,
        }
    }

    pub fn with_request_id(mut self, id: impl Into<String>) -> Self {
        self.request_id = Some(id.into());
        self
    }

    pub fn with_duration(mut self, ms: u64) -> Self {
        self.duration_ms = Some(ms);
        self
    }
}

// ────────────────────────────────────────────
// Health check
// ────────────────────────────────────────────

/// Health check response body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthResponse {
    pub status: HealthStatus,
    pub version: String,
    pub uptime_secs: u64,
    pub checks: Vec<ComponentHealth>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum HealthStatus {
    Healthy,
    Degraded,
    Unhealthy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentHealth {
    pub name: String,
    pub status: HealthStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Aggregate component health into an overall status.
pub fn aggregate_health(checks: &[ComponentHealth]) -> HealthStatus {
    if checks.iter().any(|c| c.status == HealthStatus::Unhealthy) {
        HealthStatus::Unhealthy
    } else if checks.iter().any(|c| c.status == HealthStatus::Degraded) {
        HealthStatus::Degraded
    } else {
        HealthStatus::Healthy
    }
}

// ────────────────────────────────────────────
// Admin summaries
// ────────────────────────────────────────────

/// Crawl status summary (for admin endpoint).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrawlStatusSummary {
    pub total_sources: u32,
    pub active_sources: u32,
    pub failed_sources: u32,
    pub last_cycle_duration_secs: Option<u64>,
    pub avg_success_rate: f64,
}

/// Recipe performance summary (for admin endpoint).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecipePerformanceSummary {
    pub total_recipes: u32,
    pub production_count: u32,
    pub staging_count: u32,
    pub deprecated_count: u32,
    pub avg_precision: f64,
    pub avg_recall: f64,
}

/// POI coverage summary (for admin endpoint).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoiCoverageSummary {
    pub total_pois: u32,
    pub by_region: HashMap<String, u32>,
    pub avg_priority: f64,
    pub stale_count: u32,
}

// ────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Success response ──

    #[test]
    fn test_success_response() {
        let resp = success(vec!["a", "b", "c"]);
        assert!(resp.success);
        assert!(resp.data.is_some());
        assert!(resp.error.is_none());
        assert!(resp.meta.is_some());
    }

    #[test]
    fn test_success_serialization() {
        let resp = success("hello");
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"success\":true"));
        assert!(json.contains("\"hello\""));
        assert!(!json.contains("\"error\""));
    }

    #[test]
    fn test_success_with_meta() {
        let meta = ResponseMeta::now()
            .with_request_id("req-123")
            .with_duration(42);
        let resp = success_with_meta(42, meta);
        assert_eq!(resp.meta.as_ref().unwrap().request_id, Some("req-123".to_string()));
        assert_eq!(resp.meta.as_ref().unwrap().duration_ms, Some(42));
    }

    // ── Error response ──

    #[test]
    fn test_error_response() {
        let resp: ApiResponse<()> = error_response(ApiError::not_found("Company", "123"));
        assert!(!resp.success);
        assert!(resp.data.is_none());
        let err = resp.error.unwrap();
        assert_eq!(err.code, ErrorCode::NotFound);
        assert!(err.message.contains("123"));
    }

    #[test]
    fn test_error_serialization() {
        let resp: ApiResponse<String> =
            error_response(ApiError::bad_request("Invalid page number"));
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"success\":false"));
        assert!(json.contains("Invalid page number"));
    }

    // ── ApiError constructors ──

    #[test]
    fn test_not_found_error() {
        let err = ApiError::not_found("Company", "abc");
        assert_eq!(err.http_status(), 404);
        assert!(err.message.contains("Company"));
        assert!(err.message.contains("abc"));
    }

    #[test]
    fn test_unauthorized_error() {
        let err = ApiError::unauthorized();
        assert_eq!(err.http_status(), 401);
    }

    #[test]
    fn test_forbidden_error() {
        let err = ApiError::forbidden("Insufficient permissions");
        assert_eq!(err.http_status(), 403);
    }

    #[test]
    fn test_internal_error() {
        let err = ApiError::internal("Database connection failed");
        assert_eq!(err.http_status(), 500);
    }

    #[test]
    fn test_rate_limited_error() {
        let err = ApiError::rate_limited(60);
        assert_eq!(err.http_status(), 429);
        assert!(err.details.is_some());
        assert_eq!(err.details.as_ref().unwrap().get("retry_after"), Some(&"60".to_string()));
    }

    #[test]
    fn test_validation_error() {
        let err = ApiError::validation("email", "Invalid format");
        assert_eq!(err.http_status(), 422);
        assert_eq!(
            err.details.as_ref().unwrap().get("field"),
            Some(&"email".to_string())
        );
    }

    // ── ErrorCode ──

    #[test]
    fn test_error_code_http_status() {
        assert_eq!(ErrorCode::BadRequest.http_status(), 400);
        assert_eq!(ErrorCode::Unauthorized.http_status(), 401);
        assert_eq!(ErrorCode::Forbidden.http_status(), 403);
        assert_eq!(ErrorCode::NotFound.http_status(), 404);
        assert_eq!(ErrorCode::RateLimited.http_status(), 429);
        assert_eq!(ErrorCode::ValidationError.http_status(), 422);
        assert_eq!(ErrorCode::Conflict.http_status(), 409);
        assert_eq!(ErrorCode::InternalError.http_status(), 500);
        assert_eq!(ErrorCode::ServiceUnavailable.http_status(), 503);
    }

    #[test]
    fn test_error_code_label() {
        assert_eq!(ErrorCode::NotFound.label(), "not_found");
        assert_eq!(ErrorCode::RateLimited.label(), "rate_limited");
    }

    // ── ResponseMeta ──

    #[test]
    fn test_response_meta_now() {
        let meta = ResponseMeta::now();
        assert_eq!(meta.version, "1.0.0");
        assert!(meta.request_id.is_none());
        assert!(meta.duration_ms.is_none());
    }

    #[test]
    fn test_response_meta_builder() {
        let meta = ResponseMeta::now()
            .with_request_id("req-abc")
            .with_duration(150);
        assert_eq!(meta.request_id, Some("req-abc".to_string()));
        assert_eq!(meta.duration_ms, Some(150));
    }

    // ── HealthCheck ──

    #[test]
    fn test_aggregate_health_all_healthy() {
        let checks = vec![
            ComponentHealth {
                name: "db".to_string(),
                status: HealthStatus::Healthy,
                message: None,
            },
            ComponentHealth {
                name: "redis".to_string(),
                status: HealthStatus::Healthy,
                message: None,
            },
        ];
        assert_eq!(aggregate_health(&checks), HealthStatus::Healthy);
    }

    #[test]
    fn test_aggregate_health_degraded() {
        let checks = vec![
            ComponentHealth {
                name: "db".to_string(),
                status: HealthStatus::Healthy,
                message: None,
            },
            ComponentHealth {
                name: "redis".to_string(),
                status: HealthStatus::Degraded,
                message: Some("High latency".to_string()),
            },
        ];
        assert_eq!(aggregate_health(&checks), HealthStatus::Degraded);
    }

    #[test]
    fn test_aggregate_health_unhealthy() {
        let checks = vec![
            ComponentHealth {
                name: "db".to_string(),
                status: HealthStatus::Unhealthy,
                message: Some("Connection refused".to_string()),
            },
            ComponentHealth {
                name: "redis".to_string(),
                status: HealthStatus::Degraded,
                message: None,
            },
        ];
        assert_eq!(aggregate_health(&checks), HealthStatus::Unhealthy);
    }

    #[test]
    fn test_aggregate_health_empty() {
        assert_eq!(aggregate_health(&[]), HealthStatus::Healthy);
    }

    // ── Health serialization ──

    #[test]
    fn test_health_response_serialization() {
        let resp = HealthResponse {
            status: HealthStatus::Healthy,
            version: "1.0.0".to_string(),
            uptime_secs: 3600,
            checks: vec![ComponentHealth {
                name: "db".to_string(),
                status: HealthStatus::Healthy,
                message: None,
            }],
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"Healthy\""));
        assert!(json.contains("3600"));
    }

    // ── Admin summaries ──

    #[test]
    fn test_crawl_status_summary_serialization() {
        let summary = CrawlStatusSummary {
            total_sources: 612,
            active_sources: 580,
            failed_sources: 32,
            last_cycle_duration_secs: Some(300),
            avg_success_rate: 0.947,
        };
        let json = serde_json::to_string(&summary).unwrap();
        assert!(json.contains("612"));
        assert!(json.contains("0.947"));
    }

    #[test]
    fn test_recipe_performance_summary_serialization() {
        let summary = RecipePerformanceSummary {
            total_recipes: 50,
            production_count: 30,
            staging_count: 15,
            deprecated_count: 5,
            avg_precision: 0.88,
            avg_recall: 0.72,
        };
        let json = serde_json::to_string(&summary).unwrap();
        let back: RecipePerformanceSummary = serde_json::from_str(&json).unwrap();
        assert_eq!(back.total_recipes, 50);
    }

    #[test]
    fn test_poi_coverage_summary() {
        let mut by_region = HashMap::new();
        by_region.insert("TN".to_string(), 35);
        by_region.insert("MA".to_string(), 20);
        let summary = PoiCoverageSummary {
            total_pois: 55,
            by_region,
            avg_priority: 0.65,
            stale_count: 3,
        };
        let json = serde_json::to_string(&summary).unwrap();
        assert!(json.contains("55"));
    }
}
