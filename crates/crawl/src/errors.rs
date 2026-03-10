use std::time::Duration;

use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Granular crawl failure taxonomy for reporting and alerting (B313).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CrawlFailureCategory {
    Network,
    Timeout,
    RateLimited,
    RobotsDenied,
    InvalidUrl,
    Proxy,
    Parse,
    Upstream,
    Unknown,
}

impl CrawlFailureCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Network => "network",
            Self::Timeout => "timeout",
            Self::RateLimited => "rate_limited",
            Self::RobotsDenied => "robots_denied",
            Self::InvalidUrl => "invalid_url",
            Self::Proxy => "proxy",
            Self::Parse => "parse",
            Self::Upstream => "upstream",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CrawlError {
    #[error("invalid URL for {url}: {message}")]
    InvalidUrl { url: String, message: String },
    #[error("robots.txt denied {url} for user-agent {user_agent}")]
    RobotsDenied { url: String, user_agent: String },
    #[error("proxy unavailable for {url}")]
    ProxyUnavailable { url: String },
    #[error("invalid proxy configuration for {url} via {proxy}: {message}")]
    ProxyConfiguration {
        url: String,
        proxy: String,
        message: String,
    },
    #[error("transport error for {url}: {message}")]
    Transport {
        url: String,
        message: String,
        category: CrawlFailureCategory,
    },
    #[error("HTTP {status} for {url}")]
    HttpStatus {
        url: String,
        status: u16,
        retry_after_secs: Option<u64>,
        body_excerpt: Option<String>,
    },
    #[error("failed reading response body for {url}: {message}")]
    BodyRead { url: String, message: String },
    #[error("parse error for {url}: {message}")]
    Parse { url: String, message: String },
}

impl CrawlError {
    pub fn category(&self) -> CrawlFailureCategory {
        match self {
            Self::InvalidUrl { .. } => CrawlFailureCategory::InvalidUrl,
            Self::RobotsDenied { .. } => CrawlFailureCategory::RobotsDenied,
            Self::ProxyUnavailable { .. } | Self::ProxyConfiguration { .. } => {
                CrawlFailureCategory::Proxy
            }
            Self::Transport { category, .. } => *category,
            Self::HttpStatus { status, .. } => match *status {
                408 | 425 | 429 => CrawlFailureCategory::RateLimited,
                500..=599 => CrawlFailureCategory::Upstream,
                _ => CrawlFailureCategory::Unknown,
            },
            Self::BodyRead { .. } | Self::Parse { .. } => CrawlFailureCategory::Parse,
        }
    }

    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Transport { category, .. } => {
                matches!(
                    category,
                    CrawlFailureCategory::Network | CrawlFailureCategory::Timeout
                )
            }
            Self::HttpStatus { status, .. } => matches!(*status, 403 | 408 | 425 | 429 | 500..=599),
            _ => false,
        }
    }

    pub fn retry_after(&self) -> Option<Duration> {
        match self {
            Self::HttpStatus {
                retry_after_secs: Some(secs),
                ..
            } => Some(Duration::from_secs(*secs)),
            _ => None,
        }
    }

    pub fn from_reqwest(url: &str, error: &reqwest::Error) -> Self {
        let category = if error.is_timeout() {
            CrawlFailureCategory::Timeout
        } else if error.is_connect() || error.is_request() || error.is_body() {
            CrawlFailureCategory::Network
        } else {
            CrawlFailureCategory::Unknown
        };

        Self::Transport {
            url: url.to_string(),
            message: error.to_string(),
            category,
        }
    }

    pub fn from_status(
        url: &str,
        status: StatusCode,
        retry_after_secs: Option<u64>,
        body_excerpt: Option<String>,
    ) -> Self {
        Self::HttpStatus {
            url: url.to_string(),
            status: status.as_u16(),
            retry_after_secs,
            body_excerpt,
        }
    }
}

/// Best-effort classifier from a crawl error message.
pub fn categorize_crawl_failure(error_message: &str) -> CrawlFailureCategory {
    let lower = error_message.to_lowercase();
    if lower.contains("timeout") || lower.contains("timed out") {
        return CrawlFailureCategory::Timeout;
    }
    if lower.contains("429") || lower.contains("rate limit") || lower.contains("retry-after") {
        return CrawlFailureCategory::RateLimited;
    }
    if lower.contains("robots") || lower.contains("disallow") {
        return CrawlFailureCategory::RobotsDenied;
    }
    if lower.contains("invalid url") || lower.contains("url parse") {
        return CrawlFailureCategory::InvalidUrl;
    }
    if lower.contains("proxy") {
        return CrawlFailureCategory::Proxy;
    }
    if lower.contains("parse") || lower.contains("invalid html") || lower.contains("invalid json") {
        return CrawlFailureCategory::Parse;
    }
    if lower.contains("dns")
        || lower.contains("connection")
        || lower.contains("tls")
        || lower.contains("io error")
    {
        return CrawlFailureCategory::Network;
    }
    if lower.contains("5xx")
        || lower.contains("upstream")
        || lower.contains("bad gateway")
        || lower.contains("service unavailable")
    {
        return CrawlFailureCategory::Upstream;
    }
    CrawlFailureCategory::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categorize_timeout() {
        assert_eq!(
            categorize_crawl_failure("request timed out after 30s"),
            CrawlFailureCategory::Timeout
        );
    }

    #[test]
    fn categorize_rate_limited() {
        assert_eq!(
            categorize_crawl_failure("HTTP 429 with Retry-After header"),
            CrawlFailureCategory::RateLimited
        );
    }

    #[test]
    fn categorize_robots_denied() {
        assert_eq!(
            categorize_crawl_failure("robots.txt disallow for /private"),
            CrawlFailureCategory::RobotsDenied
        );
    }

    #[test]
    fn categorize_invalid_url() {
        assert_eq!(
            categorize_crawl_failure("invalid URL: relative URL without base"),
            CrawlFailureCategory::InvalidUrl
        );
    }

    #[test]
    fn categorize_network() {
        assert_eq!(
            categorize_crawl_failure("dns lookup failed: connection reset"),
            CrawlFailureCategory::Network
        );
    }

    #[test]
    fn http_status_429_is_retryable_rate_limit() {
        let error = CrawlError::from_status(
            "https://example.com",
            StatusCode::TOO_MANY_REQUESTS,
            Some(3),
            None,
        );
        assert_eq!(error.category(), CrawlFailureCategory::RateLimited);
        assert!(error.is_retryable());
        assert_eq!(error.retry_after(), Some(Duration::from_secs(3)));
    }

    #[test]
    fn proxy_errors_are_typed() {
        let error = CrawlError::ProxyConfiguration {
            url: "https://example.com".to_string(),
            proxy: "http://bad proxy".to_string(),
            message: "invalid URL".to_string(),
        };
        assert_eq!(error.category(), CrawlFailureCategory::Proxy);
        assert!(!error.is_retryable());
    }

    #[test]
    fn category_string_stable() {
        assert_eq!(CrawlFailureCategory::Upstream.as_str(), "upstream");
    }
}
