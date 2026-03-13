#![allow(clippy::disallowed_methods)]

use apex_api::middleware::rate_limit::{
    append_rate_limit_headers, rate_limited_response, RateLimitInfo, RateLimitSource,
};
use axum::http::HeaderMap;

#[test]
fn rate_limit_middleware_preserves_headers_and_retry_after_behavior() {
    let info = RateLimitInfo {
        limit_per_min: 25,
        remaining: 12,
        reset_at_unix: 4321,
        source: RateLimitSource::Fallback,
        degraded_mode: true,
    };
    let mut headers = HeaderMap::new();
    append_rate_limit_headers(&mut headers, &info);
    assert_eq!(headers.get("x-ratelimit-limit").and_then(|value| value.to_str().ok()), Some("25"));
    assert_eq!(headers.get("x-ratelimit-remaining").and_then(|value| value.to_str().ok()), Some("12"));
    assert_eq!(headers.get("x-ratelimit-reset").and_then(|value| value.to_str().ok()), Some("4321"));

    let response = rate_limited_response(
        9,
        RateLimitInfo {
            remaining: 0,
            ..info
        },
    );
    assert_eq!(response.headers().get("retry-after").and_then(|value| value.to_str().ok()), Some("9"));
}