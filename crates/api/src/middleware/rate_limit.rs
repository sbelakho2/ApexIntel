use axum::http::{header, HeaderMap, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use redis::AsyncCommands;

use crate::rate_limit::{classify_endpoint, RateLimiter};
use crate::responses::{error_response, ApiError};

const RATE_LIMIT_POLICY: &str = "requests_per_minute; window=60s";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateLimitSource {
    Redis,
    Fallback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateLimitInfo {
    pub limit_per_min: u32,
    pub remaining: u32,
    pub reset_at_unix: i64,
    pub source: RateLimitSource,
    pub degraded_mode: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateLimitDecision {
    Allowed(RateLimitInfo),
    Limited {
        info: RateLimitInfo,
        retry_after_secs: u32,
    },
}

pub async fn enforce_rate_limit(
    redis: Option<redis::aio::ConnectionManager>,
    fallback_rate_limiter: &RateLimiter,
    key_id: &str,
    configured_limit_per_min: u32,
    path: &str,
    method: &Method,
    now: DateTime<Utc>,
) -> RateLimitDecision {
    let minute_bucket = now.timestamp() / 60;
    let reset_at_unix = (minute_bucket + 1) * 60;

    if let Some(redis_mgr) = redis {
        let rate_limit_key = format!("rl:{key_id}:{minute_bucket}");
        let mut connection = redis_mgr;
        let result: Result<i64, redis::RedisError> = async {
            let count: i64 = connection.incr(&rate_limit_key, 1i64).await?;
            if count == 1 {
                let _: () = connection.expire(&rate_limit_key, 65i64).await?;
            }
            Ok(count)
        }
        .await;

        match result {
            Ok(count) => {
                let count = count.max(0) as u32;
                let remaining = configured_limit_per_min.saturating_sub(count);
                let info = RateLimitInfo {
                    limit_per_min: configured_limit_per_min,
                    remaining,
                    reset_at_unix,
                    source: RateLimitSource::Redis,
                    degraded_mode: false,
                };

                if count > configured_limit_per_min {
                    return RateLimitDecision::Limited {
                        info,
                        retry_after_secs: (reset_at_unix - now.timestamp()).max(0) as u32,
                    };
                }

                return RateLimitDecision::Allowed(info);
            }
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    key_id,
                    path,
                    method = %method,
                    rate_limit_mode = "fallback",
                    "Redis rate limit check failed"
                );
            }
        }
    }

    let tier = classify_endpoint(path, method.as_str());
    let effective_limit = configured_limit_per_min.min(tier.max_requests());
    let bucket_name = format!("{}:{}", method.as_str(), path);
    let result = fallback_rate_limiter.check_with_limit(
        key_id,
        &bucket_name,
        effective_limit,
        tier.window(),
    );
    let retry_after_secs = result.retry_after_secs as u32;
    let info = RateLimitInfo {
        limit_per_min: result.limit,
        remaining: result.remaining,
        reset_at_unix: now.timestamp() + retry_after_secs as i64,
        source: RateLimitSource::Fallback,
        degraded_mode: true,
    };

    if result.allowed {
        RateLimitDecision::Allowed(info)
    } else {
        RateLimitDecision::Limited {
            info,
            retry_after_secs,
        }
    }
}

pub fn append_rate_limit_headers(headers: &mut HeaderMap, info: &RateLimitInfo) {
    headers.insert(
        header::HeaderName::from_static("x-ratelimit-limit"),
        header::HeaderValue::from_str(&info.limit_per_min.to_string())
            .unwrap_or_else(|_| header::HeaderValue::from_static("120")),
    );
    headers.insert(
        header::HeaderName::from_static("x-ratelimit-remaining"),
        header::HeaderValue::from_str(&info.remaining.to_string())
            .unwrap_or_else(|_| header::HeaderValue::from_static("0")),
    );
    headers.insert(
        header::HeaderName::from_static("x-ratelimit-reset"),
        header::HeaderValue::from_str(&info.reset_at_unix.to_string())
            .unwrap_or_else(|_| header::HeaderValue::from_static("0")),
    );
    headers.insert(
        header::HeaderName::from_static("x-ratelimit-policy"),
        header::HeaderValue::from_static(RATE_LIMIT_POLICY),
    );
}

pub fn rate_limited_response(retry_after_secs: u32, info: RateLimitInfo) -> Response {
    let api_error = ApiError::rate_limited(retry_after_secs);
    let mut response = (
        StatusCode::TOO_MANY_REQUESTS,
        axum::Json(error_response::<serde_json::Value>(api_error)),
    )
        .into_response();

    append_rate_limit_headers(response.headers_mut(), &info);
    response.headers_mut().insert(
        header::HeaderName::from_static("retry-after"),
        header::HeaderValue::from_str(&retry_after_secs.to_string())
            .unwrap_or_else(|_| header::HeaderValue::from_static("60")),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn rate_limit_middleware_preserves_headers_and_retry_after_behavior() {
        let response = rate_limited_response(
            7,
            RateLimitInfo {
                limit_per_min: 10,
                remaining: 0,
                reset_at_unix: 1234,
                source: RateLimitSource::Fallback,
                degraded_mode: true,
            },
        );
        let headers = response.headers();
        assert_eq!(
            headers.get("retry-after").and_then(|value| value.to_str().ok()),
            Some("7")
        );
        assert_eq!(
            headers
                .get("x-ratelimit-policy")
                .and_then(|value| value.to_str().ok()),
            Some(RATE_LIMIT_POLICY)
        );
    }
}