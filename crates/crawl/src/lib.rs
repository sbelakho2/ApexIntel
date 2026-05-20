pub mod academic;
pub mod breach;
pub mod browser;
pub mod browser_renderer;
pub mod change_detection;
pub mod client;
pub mod ct;
pub mod diff_engine;
pub mod dns;
pub mod errors;
pub mod governor_limiter;
pub mod headers;
pub mod metrics;
pub mod person_scraper;
pub mod poi_expansion;
pub mod proxy;
pub mod rate_limit;
pub mod retry_engine;
pub mod robots;
pub mod rss;
pub mod sanctions;
pub mod search_rotation;
pub mod social;
pub mod source_entropy;
pub mod source_health;
pub mod source_scoring;
pub mod sources;
pub mod sources_registry;
pub mod tor_client;
pub mod trade_shows;

// Re-export commonly used types for convenience
pub use errors::{CrawlError, CrawlFailureCategory};
pub use retry_engine::{
    RetryEngine, RetryConfig, ContentCache, CachedContent, CircuitState, CircuitStats,
    RetryDecision, RetryEngineError, DomainCircuitBreaker,
};
pub use browser_renderer::{
    EnhancedBrowserRenderer, BrowserRendererConfig, RenderResult, CacheStats,
    LazyLoadingDetector, ScreenshotVerifier,
};
pub use source_health::{
    SourceHealthMonitor, HealthConfig, HealthMetrics, HealthStatus, HealthStatistics,
    CrawlResult, RetiredSource, DiscoveredSource, NewSourceSuggestion,
    SourceType, DiscoveryMethod, DiscoveryStats,
};
