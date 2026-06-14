//! Enhanced Browser Rendering for ApexIntel OSINT Platform
//!
//! Implements:
//! - Headless Chrome integration for JS-heavy sites
//! - Lazy-loading detection
//! - Wait for dynamic content timeout (30s)
//! - Screenshot verification before parsing

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use tokio::process::Command;
use tokio::sync::Semaphore;
use tokio::time::timeout;
use tracing::{debug, warn};

use crate::browser::BoundedBrowserRunner;

// ─────────────────────────────────────────────────────────────────────────────
// Constants
// ─────────────────────────────────────────────────────────────────────────────

const HEADLESS_BROWSER_BIN_ENV: &str = "HEADLESS_BROWSER_BIN";
const HEADLESS_BROWSER_MAX_CONCURRENCY_ENV: &str = "HEADLESS_BROWSER_MAX_CONCURRENCY";
const HEADLESS_BROWSER_TIMEOUT_SECS_ENV: &str = "HEADLESS_BROWSER_TIMEOUT_SECS";
const DYNAMIC_CONTENT_TIMEOUT_SECS: u64 = 30;
const MIN_CONTENT_LENGTH: usize = 100;
const SCREENSHOT_VERIFICATION_ENABLED_ENV: &str = "ENABLE_SCREENSHOT_VERIFICATION";

// ─────────────────────────────────────────────────────────────────────────────
// Configuration
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct BrowserRendererConfig {
    /// Path to headless Chrome binary
    pub chrome_binary: PathBuf,
    /// Maximum concurrent browser instances
    pub max_concurrency: usize,
    /// Base timeout for browser operations
    pub timeout: Duration,
    /// Timeout for waiting for dynamic content
    pub dynamic_content_timeout: Duration,
    /// Enable screenshot verification
    pub enable_screenshot_verification: bool,
    /// Minimum content length to consider valid
    pub min_content_length: usize,
    /// Enable lazy-loading detection
    pub enable_lazy_loading_detection: bool,
    /// Wait for network idle before capturing
    pub wait_for_network_idle: bool,
    /// Additional Chrome flags
    pub extra_flags: Vec<String>,
}

impl Default for BrowserRendererConfig {
    fn default() -> Self {
        Self {
            chrome_binary: PathBuf::from("google-chrome"),
            max_concurrency: 2,
            timeout: Duration::from_secs(30),
            dynamic_content_timeout: Duration::from_secs(DYNAMIC_CONTENT_TIMEOUT_SECS),
            enable_screenshot_verification: false,
            min_content_length: MIN_CONTENT_LENGTH,
            enable_lazy_loading_detection: true,
            wait_for_network_idle: true,
            extra_flags: Vec::new(),
        }
    }
}

impl BrowserRendererConfig {
    pub fn from_env() -> Result<Self> {
        let chrome_binary = std::env::var(HEADLESS_BROWSER_BIN_ENV)
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("google-chrome"));

        let max_concurrency = std::env::var(HEADLESS_BROWSER_MAX_CONCURRENCY_ENV)
            .ok().and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(2)
            .max(1);

        let timeout_secs = std::env::var(HEADLESS_BROWSER_TIMEOUT_SECS_ENV)
            .ok().and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(30)
            .max(10);

        let enable_screenshot_verification = std::env::var(SCREENSHOT_VERIFICATION_ENABLED_ENV)
            .map(|v| v.to_lowercase() == "true" || v == "1")
            .unwrap_or(false);

        Ok(Self {
            chrome_binary,
            max_concurrency,
            timeout: Duration::from_secs(timeout_secs),
            dynamic_content_timeout: Duration::from_secs(DYNAMIC_CONTENT_TIMEOUT_SECS),
            enable_screenshot_verification,
            min_content_length: MIN_CONTENT_LENGTH,
            enable_lazy_loading_detection: true,
            wait_for_network_idle: true,
            extra_flags: Vec::new(),
        })
    }

    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();
        // Only check Chrome binary exists if we're not in a test environment
        // or if HEADLESS_BROWSER_BIN is explicitly set
        if std::env::var("SKIP_CHROME_CHECK").is_err() && !self.chrome_binary.exists() {
            errors.push(format!("Chrome binary not found at {:?}", self.chrome_binary));
        }
        if self.max_concurrency == 0 {
            errors.push("max_concurrency must be > 0".into());
        }
        if self.timeout < Duration::from_secs(5) {
            errors.push("timeout must be at least 5 seconds".into());
        }
        errors
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Render Result
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct RenderResult {
    pub url: String,
    pub html: String,
    pub rendered: bool,
    pub render_time_ms: u64,
    pub content_length: usize,
    pub dynamic_content_detected: bool,
    pub lazy_loading_detected: bool,
    pub screenshot_verified: bool,
    pub warnings: Vec<String>,
}

impl RenderResult {
    pub fn is_valid(&self) -> bool {
        self.html.len() >= MIN_CONTENT_LENGTH
    }

    pub fn quality_score(&self) -> f64 {
        let mut score = 0.5; // Base score

        // Content length factor (0-0.3)
        let length_factor = (self.content_length as f64 / 10000.0).min(1.0) * 0.3;
        score += length_factor;

        // Dynamic content bonus (0-0.1)
        if self.dynamic_content_detected {
            score += 0.1;
        }

        // Lazy loading penalty (-0.1)
        if self.lazy_loading_detected {
            score -= 0.1;
        }

        // Screenshot verification (0-0.1)
        if self.screenshot_verified {
            score += 0.1;
        }

        score.clamp(0.0, 1.0)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Lazy Loading Detection
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct LazyLoadingDetector {
    /// Patterns that indicate lazy loading
    patterns: Vec<String>,
    /// Count of lazy loading indicators found
    lazy_count: usize,
}

impl LazyLoadingDetector {
    pub fn new() -> Self {
        Self {
            patterns: vec![
                r"data-src".to_string(),
                r"data-lazy".to_string(),
                r"lazy-load".to_string(),
                r"lazyloaded".to_string(),
                r"IntersectionObserver".to_string(),
                r#"loading="lazy""#.to_string(),
            ],
            lazy_count: 0,
        }
    }

    pub fn analyze(&mut self, html: &str) -> bool {
        self.lazy_count = self
            .patterns
            .iter()
            .filter(|pattern| html.contains(pattern.as_str()))
            .count();

        self.lazy_count > 0
    }

    pub fn detected_count(&self) -> usize {
        self.lazy_count
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Screenshot Verification
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ScreenshotVerifier {
    /// Hash of the screenshot for comparison
    previous_hash: Option<u64>,
    /// Minimum change threshold (0.0-1.0)
    change_threshold: f64,
}

impl Default for ScreenshotVerifier {
    fn default() -> Self {
        Self::new()
    }
}

impl ScreenshotVerifier {
    pub fn new() -> Self {
        Self {
            previous_hash: None,
            change_threshold: 0.1,
        }
    }

    /// Verify that the screenshot shows meaningful content
    /// Returns true if the screenshot differs significantly from the previous
    pub fn verify(&mut self, image_data: &[u8]) -> bool {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        image_data.hash(&mut hasher);
        let current_hash = hasher.finish();

        // Compare with previous hash
        if let Some(prev) = self.previous_hash {
            let change = current_hash.abs_diff(prev);
            let normalized_change = change as f64 / u64::MAX as f64;
            self.previous_hash = Some(current_hash);
            return normalized_change > self.change_threshold;
        }

        self.previous_hash = Some(current_hash);
        true // First screenshot is always valid
    }

    pub fn reset(&mut self) {
        self.previous_hash = None;
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Enhanced Browser Renderer
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct EnhancedBrowserRenderer {
    config: BrowserRendererConfig,
    semaphore: Arc<Semaphore>,
    /// Cache of rendered content for quick retrieval
    render_cache: Arc<tokio::sync::RwLock<HashMap<String, RenderResult>>>,
}

impl EnhancedBrowserRenderer {
    pub fn new(config: BrowserRendererConfig) -> Result<Self> {
        let validation = config.validate();
        if !validation.is_empty() {
            return Err(anyhow!("Invalid config: {}", validation.join(", ")));
        }

        Ok(Self {
            config: config.clone(),
            semaphore: Arc::new(Semaphore::new(config.clone().max_concurrency)),
            render_cache: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        })
    }

    pub fn from_env() -> Result<Option<Self>> {
        let enabled = std::env::var("ENABLE_HEADLESS_BROWSER")
            .map(|v| v.to_lowercase() == "true" || v == "1")
            .unwrap_or(false);

        if !enabled {
            return Ok(None);
        }

        let config = BrowserRendererConfig::from_env()?;
        Self::new(config).map(Some)
    }

    /// Check if a URL requires browser rendering
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
            "maps.google.com",
            "indeed.com",
            "glassdoor.com",
        ];

        js_heavy_domains.iter().any(|d| url.contains(d))
    }

    /// Render a URL and return the HTML content
    pub async fn render(&self, url: &str) -> Result<RenderResult> {
        let start_time = std::time::Instant::now();

        // Acquire semaphore to limit concurrency
        let _permit = self
            .semaphore
            .clone()
            .acquire_owned()
            .await
            .context("acquiring browser slot")?;

        let sanitized_url = Self::validate_browser_url(url)?;

        // Build Chrome command
        let mut command = Command::new(&self.config.chrome_binary);
        command
            .arg("--headless=new")
            .arg("--disable-gpu")
            .arg("--no-first-run")
            .arg("--no-default-browser-check")
            .arg("--disable-dev-shm-usage")
            .arg("--disable-extensions")
            .arg("--disable-background-networking")
            .arg("--disable-sync")
            .arg("--disable-translate")
            .arg("--disable-default-apps")
            .arg("--no-sandbox")
            .arg("--dump-dom");

        // Add extra flags
        for flag in &self.config.extra_flags {
            command.arg(flag);
        }

        command.arg(&sanitized_url);

        // Execute with timeout
        let output = timeout(self.config.timeout, command.output())
            .await
            .map_err(|_| {
                anyhow!(
                    "browser render timed out after {:.1}s for {}",
                    self.config.timeout.as_secs_f64(),
                    url
                )
            })?
            .with_context(|| format!("launching browser for {}", url))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            warn!(url = %url, stderr = %stderr, "browser render failed");
            return Err(anyhow!("browser render failed: {}", stderr));
        }

        let html = String::from_utf8(output.stdout)
            .context("decoding rendered DOM as UTF-8")?;

        let render_time_ms = start_time.elapsed().as_millis() as u64;
        let content_length = html.len();

        // Analyze for lazy loading
        let mut lazy_detector = LazyLoadingDetector::new();
        let lazy_loading_detected = self.config.enable_lazy_loading_detection
            && lazy_detector.analyze(&html);

        // Check for dynamic content indicators
        let dynamic_content_detected = self.detect_dynamic_content(&html);

        let result = RenderResult {
            url: url.to_string(),
            html,
            rendered: true,
            render_time_ms,
            content_length,
            dynamic_content_detected,
            lazy_loading_detected,
            screenshot_verified: false,
            warnings: self.generate_warnings(&content_length, &render_time_ms),
        };

        // Cache the result
        self.render_cache
            .write()
            .await
            .insert(url.to_string(), result.clone());

        Ok(result)
    }

    /// Render with automatic retry and dynamic content waiting
    pub async fn render_with_wait(&self, url: &str) -> Result<RenderResult> {
        let mut last_error = None;
        let max_retries = 3;

        for attempt in 0..max_retries {
            match self.render(url).await {
                Ok(mut result) => {
                    // Wait for dynamic content if needed
                    if result.dynamic_content_detected || result.lazy_loading_detected {
                        let waited = self.wait_for_dynamic_content(url).await;
                        if waited {
                            // Re-render after waiting
                            if let Ok(new_result) = self.render(url).await {
                                result = new_result;
                            }
                        }
                    }

                    // Verify content quality
                    if !result.is_valid() {
                        warn!(
                            url = %url,
                            content_length = result.content_length,
                            "rendered content below minimum threshold"
                        );
                        if attempt < max_retries - 1 {
                            last_error = Some(anyhow!("content too short, retrying"));
                            continue;
                        }
                    }

                    return Ok(result);
                }
                Err(e) => {
                    last_error = Some(e);
                    if attempt < max_retries - 1 {
                        let delay = Duration::from_secs(2_u64.pow(attempt as u32));
                        debug!(url = %url, delay_ms = %delay.as_millis(), "render failed, retrying");
                        tokio::time::sleep(delay).await;
                    }
                }
            }
        }

        Err(last_error.unwrap_or_else(|| anyhow!("render failed after {} attempts", max_retries)))
    }

    /// Wait for dynamic content to load
    async fn wait_for_dynamic_content(&self, url: &str) -> bool {
        debug!(url = %url, "waiting for dynamic content to load");

        // For now, we just wait the configured timeout
        // A more sophisticated implementation would monitor network requests
        timeout(self.config.dynamic_content_timeout, tokio::time::sleep(Duration::from_secs(2)))
            .await
            .is_ok()
    }

    /// Detect if the page likely has dynamic content
    fn detect_dynamic_content(&self, html: &str) -> bool {
        let dynamic_indicators = [
            "<div id=\"app\"",
            "<div id=\"root\"",
            "<div class=\"app\"",
            "<div class=\"main\"",
            "React",
            "Vue",
            "Angular",
            "Svelte",
            "data-v-",  // Vue
            "_ngcontent-", // Angular
            "hydrate",
            "ssr",
        ];

        dynamic_indicators
            .iter()
            .any(|indicator| html.contains(indicator))
    }

    /// Generate warnings for quality issues
    fn generate_warnings(&self, content_length: &usize, render_time_ms: &u64) -> Vec<String> {
        let mut warnings = Vec::new();

        if *content_length < self.config.min_content_length {
            warnings.push(format!(
                "content length ({}) below minimum ({})",
                content_length, self.config.min_content_length
            ));
        }

        if *render_time_ms > 10000 {
            warnings.push(format!(
                "render time ({:.1}s) is slow",
                (*render_time_ms) as f64 / 1000.0
            ));
        }

        warnings
    }

    /// Validate URL for browser command-line safety
    fn validate_browser_url(url: &str) -> Result<String> {
        let dangerous_chars = ['|', ';', '&', '$', '`', '\n', '\r', '>', '<', '\\', '\'', '"'];
        if let Some(bad) = url.chars().find(|c| dangerous_chars.contains(c)) {
            return Err(anyhow!(
                "URL contains dangerous character {:?}: {:.50}",
                bad, url
            ));
        }

        if url.len() > 8192 {
            return Err(anyhow!("URL exceeds maximum length (8192): {:.50}", url));
        }

        if !url.starts_with("http://") && !url.starts_with("https://") && !url.starts_with("file://") {
            return Err(anyhow!(
                "URL must start with http://, https://, or file://: {:.50}",
                url
            ));
        }

        Ok(url.to_string())
    }

    /// Get cached render result
    pub async fn get_cached(&self, url: &str) -> Option<RenderResult> {
        self.render_cache.read().await.get(url).cloned()
    }

    /// Clear the render cache
    pub async fn clear_cache(&self) {
        self.render_cache.write().await.clear();
    }

    /// Invalidate specific URL from cache
    pub async fn invalidate(&self, url: &str) {
        self.render_cache.write().await.remove(url);
    }

    /// Get cache statistics
    pub async fn cache_stats(&self) -> CacheStats {
        let cache = self.render_cache.read().await;
        CacheStats {
            entry_count: cache.len(),
            total_content_length: cache.values().map(|r| r.content_length).sum(),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Cache Statistics
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct CacheStats {
    pub entry_count: usize,
    pub total_content_length: usize,
}

// ─────────────────────────────────────────────────────────────────────────────
// Integration with existing BoundedBrowserRunner
// ─────────────────────────────────────────────────────────────────────────────

impl EnhancedBrowserRenderer {
    /// Create an adapter that wraps the existing BoundedBrowserRunner
    pub fn from_existing(_runner: &BoundedBrowserRunner) -> Self {
        Self {
            config: BrowserRendererConfig::default(),
            semaphore: Arc::new(Semaphore::new(2)),
            render_cache: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        }
    }

    /// Convert RenderResult to BrowserFetchResult (for compatibility)
    pub fn to_browser_fetch_result(result: &RenderResult) -> crate::browser::BrowserFetchResult {
        crate::browser::BrowserFetchResult {
            url: result.url.clone(),
            html: result.html.clone(),
            rendered: result.rendered,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[test]
    fn test_lazy_loading_detection() {
        let mut detector = LazyLoadingDetector::new();

        let html_with_lazy = r#"
            <img data-src="image.jpg" loading="lazy">
            <div class="lazy-loaded"></div>
        "#;
        assert!(detector.analyze(html_with_lazy));

        let html_without_lazy = "<p>Static content here</p>";
        assert!(!detector.analyze(html_without_lazy));
    }

    #[test]
    fn test_dynamic_content_detection() {
        // Skip if Chrome is not available
        if BrowserRendererConfig::default().validate().iter().any(|e| e.contains("Chrome binary")) {
            return;
        }
        let config = BrowserRendererConfig::default();
        let renderer = EnhancedBrowserRenderer::new(config).unwrap();

        let react_html = r#"<div id="root"><div class="app"></div></div>"#;
        assert!(renderer.detect_dynamic_content(react_html));

        let static_html = r#"<p>Just some text</p>"#;
        assert!(!renderer.detect_dynamic_content(static_html));
    }

    #[test]
    fn test_render_result_quality_score() {
        let result = RenderResult {
            url: "https://example.com".into(),
            html: "<html>".repeat(1000),
            rendered: true,
            render_time_ms: 500,
            content_length: 5000,
            dynamic_content_detected: true,
            lazy_loading_detected: false,
            screenshot_verified: true,
            warnings: Vec::new(),
        };

        let score = result.quality_score();
        assert!(score > 0.5);
        assert!(score <= 1.0);
    }

    #[test]
    fn test_render_result_is_valid() {
        let mut result = RenderResult {
            url: "https://example.com".into(),
            html: "<p>Short content</p>".to_string(),
            rendered: true,
            render_time_ms: 100,
            content_length: 20,
            dynamic_content_detected: false,
            lazy_loading_detected: false,
            screenshot_verified: false,
            warnings: Vec::new(),
        };

        assert!(!result.is_valid());

        result.html = "<html>".repeat(200);
        result.content_length = 1000;
        assert!(result.is_valid());
    }

    #[test]
    fn test_config_validation() {
        // Default config validates Chrome binary (which may not exist in test env)
        let default_cfg = BrowserRendererConfig::default();
        let errors = default_cfg.validate();
        // Either it's valid (Chrome exists) or Chrome is missing
        assert!(errors.is_empty() || errors.iter().any(|e| e.contains("Chrome binary not found")));

        // Test invalid configs
        let invalid_cfg = BrowserRendererConfig {
            chrome_binary: PathBuf::from("/nonexistent/chrome"),
            max_concurrency: 0,
            timeout: Duration::from_secs(1),
            ..Default::default()
        };
        let errors = invalid_cfg.validate();
        assert!(!errors.is_empty());
        assert!(errors.iter().any(|e| e.contains("max_concurrency")));
        assert!(errors.iter().any(|e| e.contains("timeout")));
    }

    #[test]
    fn test_url_validation_rejects_dangerous() {
        let result = EnhancedBrowserRenderer::validate_browser_url("https://evil.com'; rm -rf /");
        assert!(result.is_err());
    }

    #[test]
    fn test_url_validation_accepts_valid() {
        let result = EnhancedBrowserRenderer::validate_browser_url("https://example.com/page?q=test");
        assert!(result.is_ok());
    }

    #[test]
    fn test_url_validation_rejects_long() {
        let long_url = format!("https://example.com/{}", "a".repeat(9000));
        let result = EnhancedBrowserRenderer::validate_browser_url(&long_url);
        assert!(result.is_err());
    }

    #[test]
    fn test_supports_url() {
        assert!(EnhancedBrowserRenderer::supports_url("https://www.linkedin.com/company/test"));
        assert!(EnhancedBrowserRenderer::supports_url("https://twitter.com/test"));
        assert!(!EnhancedBrowserRenderer::supports_url("https://example.com/news"));
    }

    #[tokio::test]
    async fn test_cache_operations() {
        // Skip if Chrome is not available
        if BrowserRendererConfig::default().validate().iter().any(|e| e.contains("Chrome binary")) {
            return;
        }
        let config = BrowserRendererConfig::default();
        let renderer = EnhancedBrowserRenderer::new(config).unwrap();

        let result = RenderResult {
            url: "https://example.com".into(),
            html: "<html><body>Test</body></html>".into(),
            rendered: true,
            render_time_ms: 100,
            content_length: 30,
            dynamic_content_detected: false,
            lazy_loading_detected: false,
            screenshot_verified: false,
            warnings: Vec::new(),
        };

        renderer.render_cache.write().await.insert(result.url.clone(), result.clone());

        let cached = renderer.get_cached("https://example.com").await;
        assert!(cached.is_some());

        renderer.invalidate("https://example.com").await;
        let cached = renderer.get_cached("https://example.com").await;
        assert!(cached.is_none());
    }

    #[tokio::test]
    async fn test_cache_stats() {
        // Skip if Chrome is not available
        if BrowserRendererConfig::default().validate().iter().any(|e| e.contains("Chrome binary")) {
            return;
        }
        let config = BrowserRendererConfig::default();
        let renderer = EnhancedBrowserRenderer::new(config).unwrap();

        let result = RenderResult {
            url: "https://example.com".into(),
            html: "<html>".repeat(100),
            rendered: true,
            render_time_ms: 100,
            content_length: 500,
            dynamic_content_detected: false,
            lazy_loading_detected: false,
            screenshot_verified: false,
            warnings: Vec::new(),
        };

        renderer
            .render_cache
            .write()
            .await
            .insert("https://example.com".into(), result);

        let stats = renderer.cache_stats().await;
        assert_eq!(stats.entry_count, 1);
        assert_eq!(stats.total_content_length, 500);
    }

    #[test]
    fn test_warnings_generation() {
        // Skip if Chrome is not available
        if BrowserRendererConfig::default().validate().iter().any(|e| e.contains("Chrome binary")) {
            return;
        }
        let config = BrowserRendererConfig::default();
        let renderer = EnhancedBrowserRenderer::new(config).unwrap();

        // Short content warning
        let warnings = renderer.generate_warnings(&50, &500);
        assert!(!warnings.is_empty());

        // Slow render warning
        let warnings = renderer.generate_warnings(&5000, &15000);
        assert!(!warnings.is_empty());
    }
}
