use serde::{Deserialize, Serialize};

/// Granular crawl failure taxonomy for reporting and alerting (B313).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CrawlFailureCategory {
    Network,
    Timeout,
    RateLimited,
    RobotsDenied,
    InvalidUrl,
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
            Self::Parse => "parse",
            Self::Upstream => "upstream",
            Self::Unknown => "unknown",
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
    if lower.contains("parse") || lower.contains("invalid html") || lower.contains("invalid json") {
        return CrawlFailureCategory::Parse;
    }
    if lower.contains("dns") || lower.contains("connection") || lower.contains("tls") || lower.contains("io error") {
        return CrawlFailureCategory::Network;
    }
    if lower.contains("5xx") || lower.contains("upstream") || lower.contains("bad gateway") || lower.contains("service unavailable") {
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
    fn category_string_stable() {
        assert_eq!(CrawlFailureCategory::Upstream.as_str(), "upstream");
    }
}
