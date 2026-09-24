//! Browser rendering for JS-heavy sources.
//!
//! There is exactly one production renderer:
//! [`PersistentChromiumBrowser`], which keeps a single Chromium process alive
//! and drives it over the Chrome DevTools Protocol (CDP). It implements the
//! [`BrowserFetcher`] trait, the only interface callers depend on.
//!
//! # Single-renderer policy
//!
//! The former parallel implementations (`BoundedBrowserRunner` spawning one
//! Chrome per fetch and the unused `EnhancedBrowserRenderer`, which duplicated
//! URL validation, timeouts and lazy-load detection) were consolidated into
//! this module. Tests use the fixture-backed `RecordedPagesBrowser`.
//!
//! # Readiness
//!
//! A page is extracted only after the readiness tracker reports
//! [`ReadinessDecision::Ready`]: DOM loaded **and** the network quiet for the
//! configured window (default 1000 ms, the CDP equivalent of Playwright's
//! `networkidle`) **and** the rendered text size stable across two consecutive
//! samples **and** a bounded progressive scroll for lazy-loaded content,
//! all under a hard total render-time cap. A timed-out render extracts the
//! best-effort DOM and flags `ReadinessReport::timed_out`.
//!
//! # Security
//!
//! Chromium runs **without** `--no-sandbox`. The service is deployed under the
//! dedicated non-root `apexintel` account (see `Dockerfile.worker`,
//! `Dockerfile.api` and `DEPLOYMENT.md`); running the browser as a non-root,
//! non-privileged user is what makes the Chromium sandbox work, so the flag
//! must not be reintroduced. URLs are restricted to `http`/`https` and
//! private/loopback/link-local/CGNAT/unique-local/`localhost` targets —
//! including DNS-rebinding answers — are rejected by the validation module.

pub mod validation;

mod cdp;
#[cfg(test)]
pub(crate) mod fixture;
mod readiness;
mod renderer;

use anyhow::Result;
use async_trait::async_trait;

pub use readiness::{
    PageSample, ReadinessDecision, ReadinessReport, ReadinessTracker, RenderPolicy, WaitReason,
    DEFAULT_MAX_RENDER_TIME, DEFAULT_MAX_SCROLL_STEPS, DEFAULT_NETWORK_QUIET_WINDOW,
    DEFAULT_SAMPLE_INTERVAL, MAX_NETWORK_QUIET_WINDOW, MIN_NETWORK_QUIET_WINDOW,
};
pub use renderer::{
    persistent_browser_from_env, BrowserConfig, PersistentChromiumBrowser, BROWSER_START_TIMEOUT,
    DEFAULT_CHROME_BINARY,
};
pub use validation::{
    assert_public_resolution, host_from_url, is_private_host, validate_browser_url,
};

/// A request for one rendered page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserRequest {
    /// URL to render; validated and canonicalised before navigation.
    pub url: String,
    /// Optional per-request readiness override.
    pub policy: Option<RenderPolicy>,
}

impl BrowserRequest {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            policy: None,
        }
    }

    pub fn with_policy(mut self, policy: RenderPolicy) -> Self {
        self.policy = Some(policy);
        self
    }
}

/// The final DOM of a rendered page plus how readiness resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserPage {
    /// Requested URL.
    pub url: String,
    /// URL actually rendered after redirects.
    pub final_url: String,
    /// Serialised final DOM (doctype + `documentElement.outerHTML`).
    pub html: String,
    /// Always `true` for browser-rendered pages.
    pub rendered: bool,
    /// Readiness evidence (samples, quiet window, scroll steps, timeout).
    pub readiness: ReadinessReport,
}

impl BrowserPage {
    /// A page is usable when the DOM is non-empty; timed-out renders can
    /// still carry partial content, which callers may accept consciously.
    pub fn is_usable(&self) -> bool {
        !self.html.trim().is_empty() && !self.readiness.timed_out
    }
}

/// The single abstraction every browser consumer depends on.
///
/// Implementations must validate the request URL, enforce their concurrency
/// budget, and only return once readiness settled or the render-time cap hit.
#[async_trait]
pub trait BrowserFetcher: Send + Sync {
    async fn fetch(&self, req: BrowserRequest) -> Result<BrowserPage>;
}

/// Whether a URL belongs to a source that needs JS rendering.
pub fn supports_url(url: &str) -> bool {
    let js_heavy_domains = [
        "linkedin.com",
        "facebook.com",
        "twitter.com",
        "x.com",
        "instagram.com",
        "reddit.com",
        "youtube.com",
        "github.com",
        "google.com",
        "indeed.com",
        "glassdoor.com",
    ];
    js_heavy_domains.iter().any(|domain| url.contains(domain))
}

/// The production browser fetcher, or `None` when `ENABLE_HEADLESS_BROWSER`
/// is false/absent. Callers keep their ordinary HTTP path when this is `None`.
pub fn from_env() -> Result<Option<PersistentChromiumBrowser>> {
    persistent_browser_from_env()
}

/// Convenience: create a shared browser fetcher from the environment.
pub fn shared_from_env() -> Result<Option<std::sync::Arc<dyn BrowserFetcher>>> {
    Ok(from_env()?
        .map(|browser| std::sync::Arc::new(browser) as std::sync::Arc<dyn BrowserFetcher>))
}

/// The readiness policy used for a request.
pub fn policy_for(config: &BrowserConfig, request: &BrowserRequest) -> RenderPolicy {
    request.policy.unwrap_or_else(|| config.policy())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn supports_url_targets_js_heavy_sources_only() {
        assert!(supports_url("https://www.linkedin.com/company/apexintel/"));
        assert!(supports_url("https://www.facebook.com/apexintel"));
        assert!(supports_url("https://x.com/someone/status/1"));
        assert!(!supports_url("https://example.com/news"));
        assert!(!supports_url("https://crates.io/crates/serde"));
    }

    #[test]
    fn browser_request_builder_carries_policy_override() {
        let request = BrowserRequest::new("https://example.com").with_policy(RenderPolicy {
            max_scroll_steps: 0,
            ..RenderPolicy::default()
        });
        assert_eq!(request.url, "https://example.com");
        assert_eq!(request.policy.unwrap().max_scroll_steps, 0);
    }

    #[tokio::test]
    async fn fixture_fetcher_satisfies_the_trait() {
        let fixture = fixture::RecordedPagesBrowser::new([(
            "https://example.com/".to_string(),
            "<html><body>recorded</body></html>".to_string(),
        )]);
        let fetcher: std::sync::Arc<dyn BrowserFetcher> = std::sync::Arc::new(fixture);
        let page = fetcher
            .fetch(BrowserRequest::new("https://example.com/"))
            .await
            .expect("fixture renders");
        assert!(page.rendered);
        assert!(page.html.contains("recorded"));
        assert!(page.is_usable());
    }
}
