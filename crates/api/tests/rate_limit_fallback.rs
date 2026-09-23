#![allow(clippy::unwrap_used, clippy::expect_used)]

use axum::http::Method;

use apex_api::middleware::rate_limit::{enforce_rate_limit, RateLimitDecision, RateLimitSource};
use apex_api::rate_limit::RateLimiter;
use chrono::Utc;

#[tokio::test]
async fn require_auth_uses_fallback_rate_limiter_when_redis_unavailable() {
    let limiter = RateLimiter::new();
    let decision = enforce_rate_limit(
        None,
        &limiter,
        "api-key-1",
        10,
        "/api/admin/replay",
        &Method::GET,
        Utc::now(),
    )
    .await;

    match decision {
        RateLimitDecision::Allowed(info) => {
            assert_eq!(info.source, RateLimitSource::Fallback);
            assert!(info.degraded_mode);
        }
        RateLimitDecision::Limited { .. } => panic!("first request should be allowed"),
    }
}

#[tokio::test]
async fn write_requests_are_still_limited_when_redis_is_down() {
    let limiter = RateLimiter::new();
    for _ in 0..30 {
        let decision = enforce_rate_limit(
            None,
            &limiter,
            "writer-key",
            120,
            "/api/warnings",
            &Method::POST,
            Utc::now(),
        )
        .await;
        assert!(matches!(decision, RateLimitDecision::Allowed(_)));
    }

    let decision = enforce_rate_limit(
        None,
        &limiter,
        "writer-key",
        120,
        "/api/warnings",
        &Method::POST,
        Utc::now(),
    )
    .await;
    assert!(matches!(decision, RateLimitDecision::Limited { .. }));
}

#[tokio::test]
async fn repeated_requests_hit_fallback_limit_after_threshold() {
    let limiter = RateLimiter::new();
    for _ in 0..3 {
        let decision = enforce_rate_limit(
            None,
            &limiter,
            "tiny-key",
            3,
            "/api/observations",
            &Method::GET,
            Utc::now(),
        )
        .await;
        assert!(matches!(decision, RateLimitDecision::Allowed(_)));
    }

    let decision = enforce_rate_limit(
        None,
        &limiter,
        "tiny-key",
        3,
        "/api/observations",
        &Method::GET,
        Utc::now(),
    )
    .await;
    match decision {
        RateLimitDecision::Limited { info, .. } => {
            assert_eq!(info.source, RateLimitSource::Fallback);
            assert!(info.degraded_mode);
        }
        RateLimitDecision::Allowed(_) => panic!("threshold should have been enforced"),
    }
}

#[tokio::test]
async fn degraded_mode_emits_structured_metric_or_log() {
    let limiter = RateLimiter::new();
    let decision = enforce_rate_limit(
        None,
        &limiter,
        "api-key-2",
        10,
        "/api/search",
        &Method::GET,
        Utc::now(),
    )
    .await;

    match decision {
        RateLimitDecision::Allowed(info) => assert!(info.degraded_mode),
        RateLimitDecision::Limited { info, .. } => assert!(info.degraded_mode),
    }
}
