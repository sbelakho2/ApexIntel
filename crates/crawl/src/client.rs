use std::sync::Arc;
use std::time::Duration;

use apex_core::text::truncate_utf8;
use reqwest::header::{CACHE_CONTROL, CONTENT_TYPE, RETRY_AFTER, USER_AGENT};
use reqwest::Client;
use tokio::sync::{Mutex, Semaphore};
use tokio::time::sleep;
use tracing::{debug, warn};
use url::Url;

use crate::concurrency::{clamp_http_concurrency, DEFAULT_HTTP_CONCURRENCY};
use crate::errors::{CrawlError, CrawlFailureCategory};
use crate::metrics::SharedDomainByteMetrics;
use crate::proxy::ProxyRotator;
use crate::rate_limit::RateLimitManager;
use crate::robots::{RobotsCache, RobotsRules};

const DEFAULT_USER_AGENT: &str = "ApexIntelBot/1.0 (+https://apex-intel.io/bot)";
const BROWSER_USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

#[derive(Clone)]
pub struct CrawlClientConfig {
    pub timeout: Duration,
    pub user_agent: String,
    pub max_retries: usize,
    /// Fleet-wide ceiling for concurrent ordinary HTTP fetches, clamped to
    /// the supported 8–16 window ([`crate::concurrency`]).
    pub max_concurrency: usize,
    pub enforce_robots_txt: bool,
    pub proxy_rotator: Option<Arc<Mutex<ProxyRotator>>>,
    pub rate_limits: Arc<Mutex<RateLimitManager>>,
    pub robots_cache: Arc<Mutex<RobotsCache>>,
    pub metrics: SharedDomainByteMetrics,
}

impl Default for CrawlClientConfig {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(20),
            user_agent: DEFAULT_USER_AGENT.to_string(),
            max_retries: 2,
            max_concurrency: DEFAULT_HTTP_CONCURRENCY,
            enforce_robots_txt: true,
            proxy_rotator: None,
            rate_limits: Arc::new(Mutex::new(RateLimitManager::new())),
            robots_cache: Arc::new(Mutex::new(RobotsCache::default())),
            metrics: SharedDomainByteMetrics::new(),
        }
    }
}

pub struct CrawlClient {
    client: Client,
    config: CrawlClientConfig,
    /// Global concurrency gate for ordinary HTTP fetches. Per-domain
    /// politeness still comes from `rate_limits`/robots; this only bounds the
    /// fleet-wide parallelism.
    http_gate: Arc<Semaphore>,
    http_concurrency: usize,
}

#[derive(Debug, Clone)]
pub struct CrawlRequest<'a> {
    pub url: &'a str,
    pub source_id: Option<&'a str>,
    pub override_user_agent: Option<&'a str>,
    pub requires_proxy: bool,
    pub prefer_browser_user_agent: bool,
}

impl<'a> CrawlRequest<'a> {
    pub fn new(url: &'a str) -> Self {
        Self {
            url,
            source_id: None,
            override_user_agent: None,
            requires_proxy: false,
            prefer_browser_user_agent: false,
        }
    }

    pub fn source_id(mut self, source_id: &'a str) -> Self {
        self.source_id = Some(source_id);
        self
    }

    pub fn override_user_agent(mut self, user_agent: &'a str) -> Self {
        self.override_user_agent = Some(user_agent);
        self
    }

    pub fn requires_proxy(mut self, requires_proxy: bool) -> Self {
        self.requires_proxy = requires_proxy;
        self
    }

    pub fn prefer_browser_user_agent(mut self, prefer_browser_user_agent: bool) -> Self {
        self.prefer_browser_user_agent = prefer_browser_user_agent;
        self
    }
}

#[derive(Debug, Clone)]
pub struct FetchResponse {
    pub url: String,
    pub status: u16,
    pub body: String,
    pub attempts: usize,
    pub proxy_used: Option<String>,
    pub content_type: Option<String>,
}

impl CrawlClient {
    pub fn new(config: CrawlClientConfig) -> Result<Self, CrawlError> {
        let client = Client::builder()
            .timeout(config.timeout)
            .user_agent(config.user_agent.clone())
            .build()
            .map_err(|error| CrawlError::Transport {
                url: "client_builder".to_string(),
                message: error.to_string(),
                category: CrawlFailureCategory::Network,
            })?;
        let http_concurrency = clamp_http_concurrency(config.max_concurrency);
        Ok(Self {
            client,
            config,
            http_gate: Arc::new(Semaphore::new(http_concurrency)),
            http_concurrency,
        })
    }

    pub fn metrics(&self) -> SharedDomainByteMetrics {
        self.config.metrics.clone()
    }

    /// Effective ordinary HTTP concurrency (clamped into 8–16).
    pub fn http_concurrency(&self) -> usize {
        self.http_concurrency
    }

    pub async fn fetch_text(
        &self,
        request: &CrawlRequest<'_>,
    ) -> Result<FetchResponse, CrawlError> {
        // Bound ordinary HTTP parallelism globally (8–16); each logical fetch
        // counts as one slot across its retries. Per-domain pacing is applied
        // separately by `rate_limits` and robots handling below.
        let _permit = self
            .http_gate
            .clone()
            .acquire_owned()
            .await
            .map_err(|error| CrawlError::Transport {
                url: request.url.to_string(),
                message: format!("HTTP concurrency gate closed: {error}"),
                category: CrawlFailureCategory::Unknown,
            })?;

        let parsed = Url::parse(request.url).map_err(|error| CrawlError::InvalidUrl {
            url: request.url.to_string(),
            message: error.to_string(),
        })?;
        let user_agent = request
            .override_user_agent
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| {
                if request.prefer_browser_user_agent {
                    BROWSER_USER_AGENT.to_string()
                } else {
                    self.config.user_agent.clone()
                }
            });
        let rate_limit_key = request
            .source_id
            .map(ToOwned::to_owned)
            .or_else(|| parsed.host_str().map(ToOwned::to_owned))
            .unwrap_or_else(|| request.url.to_string());

        if self.config.enforce_robots_txt {
            self.ensure_robots_allowed(&parsed, &user_agent).await?;
        }

        let domain = parsed.host_str().unwrap_or("unknown");
        let mut last_error = None;

        for attempt in 0..=self.config.max_retries {
            let proxy_url = if request.requires_proxy {
                self.acquire_proxy(request.url).await?
            } else {
                None
            };

            let client = self.client_for_proxy(proxy_url.as_deref(), &user_agent, request.url)?;
            let response = client
                .get(parsed.clone())
                .header(USER_AGENT, user_agent.clone())
                .send()
                .await;

            match response {
                Ok(resp) if resp.status().is_success() => {
                    let status = resp.status().as_u16();
                    let content_type = resp
                        .headers()
                        .get(CONTENT_TYPE)
                        .and_then(|value| value.to_str().ok())
                        .map(ToOwned::to_owned);
                    let body = resp.text().await.map_err(|error| CrawlError::BodyRead {
                        url: request.url.to_string(),
                        message: error.to_string(),
                    })?;

                    self.config.metrics.record_bytes(domain, body.len() as u64);
                    if let Some(proxy) = proxy_url.as_deref() {
                        self.report_proxy_success(proxy).await;
                    }
                    self.config
                        .rate_limits
                        .lock()
                        .await
                        .record_success(&rate_limit_key);

                    return Ok(FetchResponse {
                        url: request.url.to_string(),
                        status,
                        body,
                        attempts: attempt + 1,
                        proxy_used: proxy_url,
                        content_type,
                    });
                }
                Ok(resp) => {
                    let status = resp.status();
                    let retry_after_secs = resp
                        .headers()
                        .get(RETRY_AFTER)
                        .and_then(|value| value.to_str().ok())
                        .and_then(|value| {
                            RateLimitManager::retry_after_delay(value, chrono::Utc::now())
                        })
                        .map(|delay| delay.as_secs());
                    let body_excerpt = resp
                        .text()
                        .await
                        .ok()
                        .map(|body| truncate_utf8(&body, 180).to_string())
                        .filter(|body| !body.is_empty());
                    let error = CrawlError::from_status(
                        request.url,
                        status,
                        retry_after_secs,
                        body_excerpt,
                    );
                    if let Some(proxy) = proxy_url.as_deref() {
                        self.report_proxy_failure(proxy).await;
                    }
                    if self
                        .handle_retryable_error(&rate_limit_key, &error, attempt)
                        .await
                    {
                        last_error = Some(error);
                        continue;
                    }
                    return Err(error);
                }
                Err(error) => {
                    let error = CrawlError::from_reqwest(request.url, &error);
                    if let Some(proxy) = proxy_url.as_deref() {
                        self.report_proxy_failure(proxy).await;
                    }
                    if self
                        .handle_retryable_error(&rate_limit_key, &error, attempt)
                        .await
                    {
                        last_error = Some(error);
                        continue;
                    }
                    return Err(error);
                }
            }
        }

        Err(last_error.unwrap_or(CrawlError::Transport {
            url: request.url.to_string(),
            message: "request exhausted retries without a terminal response".to_string(),
            category: CrawlFailureCategory::Unknown,
        }))
    }

    async fn handle_retryable_error(
        &self,
        rate_limit_key: &str,
        error: &CrawlError,
        attempt: usize,
    ) -> bool {
        let is_soft = matches!(
            error.category(),
            CrawlFailureCategory::RateLimited | CrawlFailureCategory::Upstream
        );
        let mut rate_limits = self.config.rate_limits.lock().await;
        rate_limits.record_failure(rate_limit_key, is_soft);

        if !error.is_retryable() || attempt >= self.config.max_retries {
            return false;
        }

        let delay = error
            .retry_after()
            .unwrap_or_else(|| rate_limits.get_recommended_delay(rate_limit_key));
        drop(rate_limits);
        debug!(rate_limit_key, attempt = attempt + 1, delay_ms = delay.as_millis(), error = %error, "crawl_client: retrying request");
        sleep(delay).await;
        true
    }

    async fn ensure_robots_allowed(
        &self,
        parsed: &Url,
        user_agent: &str,
    ) -> Result<(), CrawlError> {
        let host = parsed.host_str().ok_or_else(|| CrawlError::InvalidUrl {
            url: parsed.to_string(),
            message: "URL missing host".to_string(),
        })?;

        let rules = {
            let mut cache = self.config.robots_cache.lock().await;
            cache.get(host).cloned()
        };
        let rules = match rules {
            Some(rules) => rules,
            None => self.fetch_robots_rules(parsed, host, user_agent).await,
        };

        if !rules.is_allowed(parsed.path()) {
            return Err(CrawlError::RobotsDenied {
                url: parsed.to_string(),
                user_agent: user_agent.to_string(),
            });
        }

        if let Some(delay) = rules.crawl_delay_duration() {
            sleep(delay).await;
        }
        Ok(())
    }

    async fn fetch_robots_rules(&self, parsed: &Url, host: &str, user_agent: &str) -> RobotsRules {
        let authority = match parsed.port() {
            Some(port) => format!("{host}:{port}"),
            None => host.to_string(),
        };
        let robots_url = format!("{}://{authority}/robots.txt", parsed.scheme());
        let mut rules = match self
            .client
            .get(&robots_url)
            .header(USER_AGENT, user_agent)
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                let max_age = resp
                    .headers()
                    .get(CACHE_CONTROL)
                    .and_then(|value| value.to_str().ok())
                    .and_then(parse_cache_control_max_age);
                match resp.text().await {
                    Ok(text) => {
                        let parsed_rules = RobotsRules::parse(&text, user_agent);
                        match max_age {
                            Some(max_age) => parsed_rules.with_max_age(max_age),
                            None => parsed_rules,
                        }
                    }
                    Err(error) => {
                        warn!(robots_url, error = %error, "crawl_client: failed to read robots body");
                        RobotsRules::parse("", user_agent)
                    }
                }
            }
            Ok(_) => RobotsRules::parse("", user_agent),
            Err(error) => {
                debug!(robots_url, error = %error, "crawl_client: robots fetch failed; defaulting open");
                RobotsRules::parse("", user_agent)
            }
        };

        if rules.max_age.is_none() {
            rules = rules.with_max_age(Duration::from_secs(3600));
        }

        self.config
            .robots_cache
            .lock()
            .await
            .insert(host, rules.clone());
        rules
    }

    async fn acquire_proxy(&self, url: &str) -> Result<Option<String>, CrawlError> {
        match self.config.proxy_rotator.as_ref() {
            Some(rotator) => {
                let mut guard = rotator.lock().await;
                guard
                    .get_next()
                    .map(Some)
                    .ok_or_else(|| CrawlError::ProxyUnavailable {
                        url: url.to_string(),
                    })
            }
            None => Ok(None),
        }
    }

    async fn report_proxy_success(&self, proxy_url: &str) {
        if let Some(rotator) = self.config.proxy_rotator.as_ref() {
            rotator.lock().await.report_success(proxy_url);
        }
    }

    async fn report_proxy_failure(&self, proxy_url: &str) {
        if let Some(rotator) = self.config.proxy_rotator.as_ref() {
            rotator.lock().await.report_failure(proxy_url);
        }
    }

    fn client_for_proxy(
        &self,
        proxy_url: Option<&str>,
        user_agent: &str,
        request_url: &str,
    ) -> Result<Client, CrawlError> {
        match proxy_url {
            Some(proxy_url) => Client::builder()
                .timeout(self.config.timeout)
                .user_agent(user_agent)
                .proxy(reqwest::Proxy::all(proxy_url).map_err(|error| {
                    CrawlError::ProxyConfiguration {
                        url: request_url.to_string(),
                        proxy: proxy_url.to_string(),
                        message: error.to_string(),
                    }
                })?)
                .build()
                .map_err(|error| CrawlError::ProxyConfiguration {
                    url: request_url.to_string(),
                    proxy: proxy_url.to_string(),
                    message: error.to_string(),
                }),
            None => Ok(self.client.clone()),
        }
    }
}

fn parse_cache_control_max_age(value: &str) -> Option<Duration> {
    value
        .split(',')
        .map(|part| part.trim())
        .find_map(|part| part.strip_prefix("max-age="))
        .and_then(|secs| secs.parse::<u64>().ok())
        .map(Duration::from_secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::VecDeque;
    use std::net::SocketAddr;
    use std::sync::Arc as StdArc;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::sync::Mutex as TokioMutex;

    async fn start_test_server(responses: Vec<&'static str>) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap_or_else(|error| panic!("test: bind ephemeral port: {error}"));
        let addr = listener
            .local_addr()
            .unwrap_or_else(|error| panic!("test: get local addr: {error}"));
        let queue = StdArc::new(TokioMutex::new(
            responses
                .into_iter()
                .map(str::to_string)
                .collect::<VecDeque<_>>(),
        ));
        tokio::spawn({
            let queue = queue.clone();
            async move {
                loop {
                    let next_response = { queue.lock().await.pop_front() };
                    let Some(response) = next_response else {
                        break;
                    };

                    let (mut stream, _) = listener
                        .accept()
                        .await
                        .unwrap_or_else(|error| panic!("test: accept connection: {error}"));
                    let mut buf = [0_u8; 2048];
                    let _ = stream
                        .read(&mut buf)
                        .await
                        .unwrap_or_else(|error| panic!("test: read request: {error}"));
                    stream
                        .write_all(response.as_bytes())
                        .await
                        .unwrap_or_else(|error| panic!("test: write response: {error}"));
                }
            }
        });
        addr
    }

    #[allow(clippy::unwrap_used, clippy::expect_used)]
    #[tokio::test]
    async fn fetch_text_retries_after_rate_limit() {
        let addr = start_test_server(vec![
            "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 0\r\nConnection: close\r\nContent-Length: 8\r\n\r\nbackoff!",
            "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Type: text/plain\r\nContent-Length: 2\r\n\r\nok",
        ])
        .await;
        let config = CrawlClientConfig {
            enforce_robots_txt: false,
            max_retries: 1,
            ..CrawlClientConfig::default()
        };
        let client = CrawlClient::new(config)
            .unwrap_or_else(|error| panic!("test: build crawl client: {error}"));
        let response = client
            .fetch_text(&CrawlRequest::new(&format!("http://{addr}/feed")).source_id("test_feed"))
            .await
            .unwrap_or_else(|error| panic!("test: fetch should succeed after retry: {error}"));

        assert_eq!(response.status, 200);
        assert_eq!(response.body, "ok");
        assert_eq!(response.attempts, 2);
        assert_eq!(client.metrics().get("127.0.0.1"), 2);
    }

    #[allow(clippy::unwrap_used, clippy::expect_used)]
    #[tokio::test]
    async fn fetch_text_uses_cached_robots_rules() {
        let config = CrawlClientConfig::default();
        config.robots_cache.lock().await.insert(
            "example.com",
            RobotsRules::parse("User-agent: *\nDisallow: /private\n", DEFAULT_USER_AGENT),
        );
        let client = CrawlClient::new(config)
            .unwrap_or_else(|error| panic!("test: build crawl client: {error}"));
        let error = client
            .fetch_text(&CrawlRequest::new("http://example.com/private/report"))
            .await
            .unwrap_err();

        assert!(matches!(error, CrawlError::RobotsDenied { .. }));
    }

    #[test]
    fn parse_cache_control_max_age_reads_seconds() {
        assert_eq!(
            parse_cache_control_max_age("public, max-age=120"),
            Some(Duration::from_secs(120))
        );
    }

    #[allow(clippy::unwrap_used, clippy::expect_used)]
    #[test]
    fn http_concurrency_defaults_inside_the_supported_window() {
        let client = CrawlClient::new(CrawlClientConfig::default())
            .unwrap_or_else(|error| panic!("test: build crawl client: {error}"));
        assert_eq!(client.http_concurrency(), DEFAULT_HTTP_CONCURRENCY);
        assert_eq!(
            client.http_gate.available_permits(),
            DEFAULT_HTTP_CONCURRENCY,
            "semaphore capacity must equal the configured concurrency"
        );
    }

    #[allow(clippy::unwrap_used, clippy::expect_used)]
    #[test]
    fn http_concurrency_is_clamped_to_the_supported_window() {
        let low = CrawlClient::new(CrawlClientConfig {
            max_concurrency: 1,
            ..CrawlClientConfig::default()
        })
        .unwrap_or_else(|error| panic!("test: build crawl client: {error}"));
        assert_eq!(
            low.http_concurrency(),
            crate::concurrency::MIN_HTTP_CONCURRENCY
        );

        let high = CrawlClient::new(CrawlClientConfig {
            max_concurrency: 1024,
            ..CrawlClientConfig::default()
        })
        .unwrap_or_else(|error| panic!("test: build crawl client: {error}"));
        assert_eq!(
            high.http_concurrency(),
            crate::concurrency::MAX_HTTP_CONCURRENCY
        );
    }

    #[allow(clippy::unwrap_used, clippy::expect_used)]
    #[test]
    fn http_gate_admits_only_the_configured_number_of_fetches() {
        let client = CrawlClient::new(CrawlClientConfig {
            max_concurrency: 8,
            ..CrawlClientConfig::default()
        })
        .unwrap_or_else(|error| panic!("test: build crawl client: {error}"));

        let permits: Vec<_> = (0..8)
            .map(|_| {
                client
                    .http_gate
                    .clone()
                    .try_acquire_owned()
                    .unwrap_or_else(|error| panic!("test: acquire permit: {error}"))
            })
            .collect();
        assert_eq!(permits.len(), 8);
        assert!(
            client.http_gate.clone().try_acquire_owned().is_err(),
            "a ninth concurrent fetch must wait"
        );
    }
}
