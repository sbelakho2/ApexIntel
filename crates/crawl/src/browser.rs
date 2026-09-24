use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use apex_core::env::parse_truthy_flag;
use tokio::process::Command;
use tokio::sync::Semaphore;

const HEADLESS_BROWSER_BIN_ENV: &str = "HEADLESS_BROWSER_BIN";
const HEADLESS_BROWSER_MAX_CONCURRENCY_ENV: &str = "HEADLESS_BROWSER_MAX_CONCURRENCY";
const HEADLESS_BROWSER_TIMEOUT_SECS_ENV: &str = "HEADLESS_BROWSER_TIMEOUT_SECS";

#[derive(Debug, Clone)]
pub struct BrowserFetchResult {
    pub url: String,
    pub html: String,
    pub rendered: bool,
}

#[derive(Debug, Clone)]
enum BrowserMode {
    ChromeDumpDom { binary: PathBuf },
    RecordedFixtures { pages: Arc<HashMap<String, String>> },
}

#[derive(Clone)]
pub struct BoundedBrowserRunner {
    semaphore: Arc<Semaphore>,
    timeout: Duration,
    mode: BrowserMode,
}

impl BoundedBrowserRunner {
    pub fn from_env() -> Result<Option<Self>> {
        let enabled = std::env::var("ENABLE_HEADLESS_BROWSER")
            .ok()
            .map(|value| parse_truthy_flag(&value))
            .unwrap_or(false);
        if !enabled {
            return Ok(None);
        }

        let binary = std::env::var(HEADLESS_BROWSER_BIN_ENV)
            .ok()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("google-chrome"));
        let max_concurrency = std::env::var(HEADLESS_BROWSER_MAX_CONCURRENCY_ENV)
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(2)
            .max(1);
        let timeout = Duration::from_secs(
            std::env::var(HEADLESS_BROWSER_TIMEOUT_SECS_ENV)
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(20)
                .max(1),
        );

        Ok(Some(Self {
            semaphore: Arc::new(Semaphore::new(max_concurrency)),
            timeout,
            mode: BrowserMode::ChromeDumpDom { binary },
        }))
    }

    pub fn supports_url(url: &str) -> bool {
        url.contains("linkedin.com/") || url.contains("facebook.com/")
    }

    pub async fn fetch(&self, url: &str) -> Result<BrowserFetchResult> {
        let _permit = self
            .semaphore
            .clone()
            .acquire_owned()
            .await
            .context("acquiring headless browser slot")?;

        match &self.mode {
            BrowserMode::ChromeDumpDom { binary } => self.fetch_with_chrome(binary, url).await,
            BrowserMode::RecordedFixtures { pages } => pages
                .get(url)
                .cloned()
                .map(|html| BrowserFetchResult {
                    url: url.to_string(),
                    html,
                    rendered: true,
                })
                .ok_or_else(|| anyhow!("no recorded browser fixture for {url}")),
        }
    }

    async fn fetch_with_chrome(&self, binary: &PathBuf, url: &str) -> Result<BrowserFetchResult> {
        // Validate URL to prevent command injection via malicious URLs
        let sanitized_url = Self::validate_browser_url(url)?;
        // Re-resolve and block private/loopback targets (SSRF / rebinding).
        Self::assert_public_resolution(&sanitized_url).await?;

        let mut command = Command::new(binary);
        command
            .arg("--headless=new")
            .arg("--disable-gpu")
            .arg("--no-first-run")
            .arg("--no-default-browser-check")
            .arg("--dump-dom")
            .arg(&sanitized_url);

        let output = tokio::time::timeout(self.timeout, command.output())
            .await
            .map_err(|_| {
                anyhow!(
                    "headless browser timed out after {}s",
                    self.timeout.as_secs()
                )
            })?
            .with_context(|| format!("launching headless browser {:?}", binary))?;

        if !output.status.success() {
            return Err(anyhow!(
                "headless browser failed for {}: {}",
                url,
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        let html = String::from_utf8(output.stdout).context("decoding rendered DOM as UTF-8")?;
        if html.trim().is_empty() {
            return Err(anyhow!("headless browser returned an empty DOM for {url}"));
        }

        Ok(BrowserFetchResult {
            url: url.to_string(),
            html,
            rendered: true,
        })
    }

    pub fn from_recorded_pages(pages: HashMap<String, String>) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(1)),
            timeout: Duration::from_secs(1),
            mode: BrowserMode::RecordedFixtures {
                pages: Arc::new(pages),
            },
        }
    }

    /// Validate that a URL is safe to pass as a command-line argument to a headless browser.
    /// Returns the URL if valid, or an error if it contains potentially dangerous characters.
    fn validate_browser_url(url: &str) -> Result<String> {
        // Reject URLs containing shell metacharacters that could enable argument injection
        let dangerous_chars = [
            '|', ';', '&', '$', '`', '\n', '\r', '>', '<', '\\', '\'', '"',
        ];
        if let Some(bad) = url.chars().find(|c| dangerous_chars.contains(c)) {
            return Err(anyhow!(
                "URL contains dangerous character {:?} which may enable command injection: {url:.50}",
                bad
            ));
        }

        // Ensure URL is reasonably sized and starts with a valid scheme
        if url.len() > 8192 {
            return Err(anyhow!(
                "URL exceeds maximum allowed length (8192): {url:.50}"
            ));
        }
        // Only web URLs may be browsed. `file://`, `ftp://`, `data:`,
        // `javascript:`, `chrome:` and `about:` are rejected: this service
        // follows discovered URLs and must not read local files or browser
        // internals.
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Err(anyhow!(
                "URL must use the http:// or https:// scheme: {url:.50}"
            ));
        }
        if let Some(host) = Self::host_from_url(url) {
            if Self::is_private_host(&host) {
                return Err(anyhow!(
                    "refusing to browse private/loopback/metadata host {host}: {url:.50}"
                ));
            }
        }

        Ok(url.to_string())
    }

    /// Extract the lowercased host from an http(s) URL (no DNS).
    pub fn host_from_url(url: &str) -> Option<String> {
        let rest = url.split_once("://")?.1;
        let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
        let authority = authority.rsplit('@').next().unwrap_or(authority);
        if authority.starts_with('[') {
            // IPv6 literal: [::1]:8080
            let end = authority.find(']')?;
            return Some(authority[1..end].to_ascii_lowercase());
        }
        let host = authority.split(':').next().unwrap_or("");
        if host.is_empty() {
            None
        } else {
            Some(host.to_ascii_lowercase())
        }
    }

    /// True for loopback/private/link-local/CGNAT/metadata addresses and
    /// `localhost` names. Used to block SSRF from discovered URLs.
    pub fn is_private_host(host: &str) -> bool {
        if host == "localhost" || host.ends_with(".localhost") {
            return true;
        }
        match host.parse::<std::net::IpAddr>() {
            Ok(std::net::IpAddr::V4(v4)) => {
                v4.is_private()
                    || v4.is_loopback()
                    || v4.is_link_local()
                    || v4.is_unspecified()
                    || v4.is_broadcast()
                    // 100.64.0.0/10 carrier-grade NAT
                    || (v4.octets()[0] == 100 && (64..=127).contains(&v4.octets()[1]))
            }
            Ok(std::net::IpAddr::V6(v6)) => {
                v6.is_loopback()
                    || v6.is_unspecified()
                    || v6.is_unique_local()
                    || v6.is_unicast_link_local()
            }
            Err(_) => false,
        }
    }

    /// Re-resolve the host immediately before browsing and reject any private
    /// address, reducing DNS-rebinding exposure.
    async fn assert_public_resolution(url: &str) -> Result<()> {
        let Some(host) = Self::host_from_url(url) else {
            return Ok(());
        };
        if Self::is_private_host(&host) {
            return Err(anyhow!("refusing to browse private host {host}"));
        }
        if host.parse::<std::net::IpAddr>().is_ok() {
            return Ok(());
        }
        // Owned tuple keeps the lookup future independent of `host`'s borrow.
        match tokio::net::lookup_host((host.clone(), 443)).await {
            Ok(addrs) => {
                for addr in addrs {
                    if Self::is_private_host(&addr.ip().to_string()) {
                        return Err(anyhow!(
                            "host {host} resolves to private address {} — refusing",
                            addr.ip()
                        ));
                    }
                }
                Ok(())
            }
            // Resolution failure is handled by the browser itself.
            Err(_) => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::unwrap_used, clippy::expect_used)]
    #[tokio::test]
    async fn recorded_browser_fixture_returns_rendered_html() {
        let runner = BoundedBrowserRunner::from_recorded_pages(HashMap::from([(
            "https://www.linkedin.com/company/apexintel/".to_string(),
            "<html><body>rendered</body></html>".to_string(),
        )]));

        let page = runner
            .fetch("https://www.linkedin.com/company/apexintel/")
            .await
            .unwrap_or_else(|error| panic!("recorded browser fixture should fetch: {error}"));

        assert!(page.rendered);
        assert!(page.html.contains("rendered"));
    }

    #[test]
    fn headless_policy_is_limited_to_dynamic_sources() {
        assert!(BoundedBrowserRunner::supports_url(
            "https://www.linkedin.com/company/apexintel/"
        ));
        assert!(BoundedBrowserRunner::supports_url(
            "https://www.facebook.com/apexintel"
        ));
        assert!(!BoundedBrowserRunner::supports_url(
            "https://example.com/news"
        ));
    }
}
