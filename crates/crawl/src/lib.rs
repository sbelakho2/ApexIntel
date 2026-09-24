pub mod academic;
pub mod breach;
pub mod browser;
pub mod change_detection;
pub mod client;
pub mod concurrency;
pub mod contact_enrichment;
pub mod ct;
pub mod cve;
pub mod dark_web;
pub mod diff_engine;
pub mod dns;
pub mod errors;
pub mod governor_limiter;
pub mod headers;
pub mod metrics;
pub mod openalex;
pub mod person_scraper;
pub mod poi_expansion;
pub mod proxy;
pub mod rate_limit;
pub mod rdap;
pub mod retry;
pub mod retry_engine;
pub mod robots;
pub mod rss;
pub mod sanctions;
pub mod search_rotation;
pub mod sec_edgar;
pub mod social;
pub mod source_entropy;
pub mod source_health;
pub mod source_scoring;
pub mod sources;
pub mod sources_registry;
pub mod tor_client;
pub mod trade_shows;

// Re-export commonly used types for convenience
pub use browser::{
    from_env as browser_from_env, host_from_url, is_private_host, supports_url,
    validate_browser_url, BrowserConfig, BrowserFetcher, BrowserPage, BrowserRequest, PageSample,
    PersistentChromiumBrowser, ReadinessDecision, ReadinessReport, ReadinessTracker, RenderPolicy,
    WaitReason,
};
pub use concurrency::{
    clamp_http_concurrency, BrowserConcurrencyGate, BROWSER_CONTEXTS, BROWSER_PROCESSES,
    DEFAULT_HTTP_CONCURRENCY, GLOBAL_BROWSER_CONCURRENCY, MAX_HTTP_CONCURRENCY,
    MIN_HTTP_CONCURRENCY,
};
pub use errors::{CrawlError, CrawlFailureCategory};
pub use retry_engine::{
    CachedContent, CircuitState, CircuitStats, ContentCache, DomainCircuitBreaker, RetryConfig,
    RetryDecision, RetryEngine, RetryEngineError,
};
pub use source_health::{
    CrawlResult, DiscoveredSource, DiscoveryMethod, DiscoveryStats, HealthConfig, HealthMetrics,
    HealthStatistics, HealthStatus, NewSourceSuggestion, RetiredSource, SourceHealthMonitor,
    SourceType,
};
