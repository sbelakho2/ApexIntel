//! Persistent Chromium renderer (single production browser implementation).
//!
//! One Chromium process is launched lazily on first use and kept alive for the
//! lifetime of the worker. Every fetch opens an isolated browser context and a
//! page target inside that process, drives the page through the readiness
//! policy, extracts the final DOM, and disposes the context. Because the
//! process is persistent, profile setup and browser startup happen once, not
//! once per URL.
//!
//! # Sandbox
//!
//! Chromium is launched **without** `--no-sandbox`. The supported deployment
//! runs the worker/API binaries under the dedicated non-root `apexintel`
//! service account (`useradd -r apexintel` in `Dockerfile.api` /
//! `Dockerfile.worker`, `USER apexintel`), which is exactly the configuration
//! Chromium's setuid sandbox expects. Do not add `--no-sandbox` back to make
//! the browser start as root; fix the service account instead.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tracing::{debug, warn};
use url::Url;

use crate::browser::cdp::{CdpClient, CdpSession};
use crate::browser::readiness::{
    PageSample, ReadinessDecision, ReadinessTracker, RenderPolicy, DEFAULT_MAX_RENDER_TIME,
    DEFAULT_MAX_SCROLL_STEPS, DEFAULT_NETWORK_QUIET_WINDOW, DEFAULT_SAMPLE_INTERVAL,
    MAX_NETWORK_QUIET_WINDOW, MIN_NETWORK_QUIET_WINDOW,
};
use crate::browser::validation::{assert_public_resolution, validate_browser_url};
use crate::browser::{policy_for, BrowserFetcher, BrowserPage, BrowserRequest};
use crate::concurrency::{
    BrowserConcurrencyGate, BROWSER_CONTEXTS, BROWSER_PROCESSES, GLOBAL_BROWSER_CONCURRENCY,
};
use apex_core::env::{parse_truthy_flag, ENABLE_HEADLESS_BROWSER};

/// Default Chromium binary name resolved from `PATH`.
pub const DEFAULT_CHROME_BINARY: &str = "google-chrome";

/// How long to wait for Chromium to announce its DevTools endpoint.
pub const BROWSER_START_TIMEOUT: Duration = Duration::from_secs(20);

/// Extra time allowed after the readiness wait for DOM extraction before the
/// whole render is aborted.
const EXTRACTION_GRACE: Duration = Duration::from_secs(15);

/// Lower bound for the total render-time cap.
const MIN_RENDER_TIME_SECS: u64 = 5;

const BIN_ENV: &str = "HEADLESS_BROWSER_BIN";
const MAX_CONCURRENCY_ENV: &str = "HEADLESS_BROWSER_MAX_CONCURRENCY";
const TIMEOUT_SECS_ENV: &str = "HEADLESS_BROWSER_TIMEOUT_SECS";
const QUIET_WINDOW_MS_ENV: &str = "HEADLESS_BROWSER_QUIET_WINDOW_MS";
const SCROLL_STEPS_ENV: &str = "HEADLESS_BROWSER_SCROLL_STEPS";
const EXTRA_FLAGS_ENV: &str = "HEADLESS_BROWSER_EXTRA_FLAGS";

/// Renderer configuration. The concurrency fields default to the fixed
/// browser budget (one process, two contexts, global two) and can only be
/// lowered, never raised.
#[derive(Debug, Clone)]
pub struct BrowserConfig {
    pub chrome_binary: PathBuf,
    pub browser_processes: usize,
    pub contexts: usize,
    pub global_concurrency: usize,
    pub max_render_time: Duration,
    pub quiet_window: Duration,
    pub sample_interval: Duration,
    pub max_scroll_steps: u32,
    pub extra_flags: Vec<String>,
}

impl Default for BrowserConfig {
    fn default() -> Self {
        Self {
            chrome_binary: PathBuf::from(DEFAULT_CHROME_BINARY),
            browser_processes: BROWSER_PROCESSES,
            contexts: BROWSER_CONTEXTS,
            global_concurrency: GLOBAL_BROWSER_CONCURRENCY,
            max_render_time: DEFAULT_MAX_RENDER_TIME,
            quiet_window: DEFAULT_NETWORK_QUIET_WINDOW,
            sample_interval: DEFAULT_SAMPLE_INTERVAL,
            max_scroll_steps: DEFAULT_MAX_SCROLL_STEPS,
            extra_flags: Vec::new(),
        }
    }
}

impl BrowserConfig {
    pub fn from_env() -> Result<Self> {
        let mut config = Self::default();

        if let Ok(binary) = std::env::var(BIN_ENV) {
            if !binary.trim().is_empty() {
                config.chrome_binary = PathBuf::from(binary);
            }
        }

        if let Some(configured) = std::env::var(MAX_CONCURRENCY_ENV)
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
        {
            config.global_concurrency = configured.clamp(1, GLOBAL_BROWSER_CONCURRENCY);
            config.contexts = config.global_concurrency;
        }

        if let Some(seconds) = std::env::var(TIMEOUT_SECS_ENV)
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
        {
            config.max_render_time = Duration::from_secs(seconds.max(MIN_RENDER_TIME_SECS));
        }

        if let Some(millis) = std::env::var(QUIET_WINDOW_MS_ENV)
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
        {
            config.quiet_window = Duration::from_millis(millis)
                .clamp(MIN_NETWORK_QUIET_WINDOW, MAX_NETWORK_QUIET_WINDOW);
        }

        if let Some(steps) = std::env::var(SCROLL_STEPS_ENV)
            .ok()
            .and_then(|value| value.parse::<u32>().ok())
        {
            config.max_scroll_steps = steps.min(10);
        }

        if let Ok(flags) = std::env::var(EXTRA_FLAGS_ENV) {
            config.extra_flags = flags
                .split_whitespace()
                .map(str::to_string)
                .collect::<Vec<_>>();
        }

        Ok(config)
    }

    /// The readiness policy derived from this configuration.
    pub fn policy(&self) -> RenderPolicy {
        RenderPolicy {
            max_render_time: self.max_render_time,
            quiet_window: self.quiet_window,
            sample_interval: self.sample_interval,
            max_scroll_steps: self.max_scroll_steps,
        }
        .normalized()
    }
}

/// Marker error for a render that hit its total render-time cap. The
/// persistent process is healthy in this case and is kept warm; the page
/// itself is still cleaned up (target/context disposed).
#[derive(Debug, thiserror::Error)]
#[error("browser render exceeded {budget:?} for {url}")]
struct RenderTimeout {
    budget: Duration,
    url: String,
}

/// A live Chromium process plus its DevTools connection.
struct BrowserProcess {
    child: Child,
    client: Arc<CdpClient>,
    _profile: tempfile::TempDir,
}

impl BrowserProcess {
    fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

/// The single production [`BrowserFetcher`]: one persistent Chromium process,
/// isolated contexts per fetch, readiness-gated DOM extraction.
pub struct PersistentChromiumBrowser {
    config: BrowserConfig,
    gate: BrowserConcurrencyGate,
    process: Mutex<Option<BrowserProcess>>,
}

impl PersistentChromiumBrowser {
    pub fn new(config: BrowserConfig) -> Self {
        let global = config
            .global_concurrency
            .clamp(1, GLOBAL_BROWSER_CONCURRENCY);
        Self {
            config,
            gate: BrowserConcurrencyGate::new(global),
            process: Mutex::new(None),
        }
    }

    pub fn config(&self) -> &BrowserConfig {
        &self.config
    }

    /// Number of persistent Chromium processes this renderer keeps.
    pub fn browser_processes(&self) -> usize {
        BROWSER_PROCESSES
    }

    /// Maximum concurrent contexts/pages inside the process.
    pub fn contexts_per_process(&self) -> usize {
        self.config.contexts.min(BROWSER_CONTEXTS)
    }

    /// Global browser concurrency (permits on the fetch gate).
    pub fn global_concurrency(&self) -> usize {
        self.gate.limit()
    }

    async fn ensure_process(&self) -> Result<Arc<CdpClient>> {
        let mut guard = self.process.lock().await;
        if let Some(process) = guard.as_mut() {
            if process.is_alive() {
                return Ok(process.client.clone());
            }
            debug!("browser: Chromium process exited; restarting");
            *guard = None;
        }
        let process = self.spawn_process().await?;
        let client = process.client.clone();
        *guard = Some(process);
        Ok(client)
    }

    async fn process_healthy(&self) -> bool {
        match self.process.lock().await.as_mut() {
            Some(process) => process.is_alive(),
            None => false,
        }
    }

    async fn discard_process(&self) {
        if let Some(mut process) = self.process.lock().await.take() {
            let _ = process.child.kill().await;
            let _ = process.child.wait().await;
        }
    }

    async fn spawn_process(&self) -> Result<BrowserProcess> {
        let profile = tempfile::Builder::new()
            .prefix("apex-chromium-")
            .tempdir()
            .context("creating Chromium profile directory")?;
        let args = chromium_launch_args(&self.config, profile.path());

        let mut command = Command::new(&self.config.chrome_binary);
        command
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let mut child = command.spawn().with_context(|| {
            format!(
                "launching Chromium {:?} (must run under the dedicated non-root service account; \
                 --no-sandbox is intentionally not used)",
                self.config.chrome_binary
            )
        })?;

        let stderr = child
            .stderr
            .take()
            .context("Chromium stderr was not piped")?;
        let ws_url =
            match tokio::time::timeout(BROWSER_START_TIMEOUT, read_devtools_url(stderr)).await {
                Ok(Ok(url)) => url,
                Ok(Err(error)) => {
                    let _ = child.kill().await;
                    return Err(error);
                }
                Err(_) => {
                    let _ = child.kill().await;
                    return Err(anyhow!(
                    "Chromium did not expose a DevTools endpoint within {BROWSER_START_TIMEOUT:?}"
                ));
                }
            };

        let client = Arc::new(CdpClient::connect(&ws_url).await?);
        debug!(ws_url = %ws_url, "browser: persistent Chromium ready");
        Ok(BrowserProcess {
            child,
            client,
            _profile: profile,
        })
    }

    async fn render(&self, parsed: &Url, policy: &RenderPolicy) -> Result<BrowserPage> {
        let client = self.ensure_process().await?;

        let context = client
            .call(None, "Target.createBrowserContext", json!({}))
            .await?;
        let context_id = context
            .get("browserContextId")
            .and_then(Value::as_str)
            .context("CDP did not return a browserContextId")?
            .to_string();

        let target = client
            .call(
                None,
                "Target.createTarget",
                json!({ "url": "about:blank", "browserContextId": context_id }),
            )
            .await?;
        let target_id = target
            .get("targetId")
            .and_then(Value::as_str)
            .context("CDP did not return a targetId")?
            .to_string();

        let attached = client
            .call(
                None,
                "Target.attachToTarget",
                json!({ "targetId": target_id, "flatten": true }),
            )
            .await?;
        let session_id = attached
            .get("sessionId")
            .and_then(Value::as_str)
            .context("CDP did not return a sessionId")?
            .to_string();

        let network = client.register_session(&session_id).await;
        let session = CdpSession::new(client.clone(), session_id.clone(), network);

        let budget = policy
            .max_render_time
            .checked_add(EXTRACTION_GRACE)
            .unwrap_or(policy.max_render_time);
        let outcome =
            tokio::time::timeout(budget, self.render_in_session(&session, parsed, policy)).await;

        let _ = client
            .call(None, "Target.closeTarget", json!({ "targetId": target_id }))
            .await;
        let _ = client
            .call(
                None,
                "Target.disposeBrowserContext",
                json!({ "browserContextId": context_id }),
            )
            .await;
        client.unregister_session(&session_id).await;

        match outcome {
            Ok(result) => result,
            Err(_) => Err(RenderTimeout {
                budget,
                url: parsed.to_string(),
            }
            .into()),
        }
    }

    async fn render_in_session(
        &self,
        session: &CdpSession,
        parsed: &Url,
        policy: &RenderPolicy,
    ) -> Result<BrowserPage> {
        session.call("Page.enable", json!({})).await?;
        session.call("Network.enable", json!({})).await?;

        let navigation = session
            .call("Page.navigate", json!({ "url": parsed.as_str() }))
            .await?;
        if let Some(error) = navigation.get("errorText").and_then(Value::as_str) {
            return Err(anyhow!("navigation to {parsed} failed: {error}"));
        }

        let start = Instant::now();
        let mut tracker = ReadinessTracker::new(*policy);
        let mut timed_out = false;
        loop {
            let sample = self.sample_page(session).await?;
            match tracker.observe(sample, start.elapsed()) {
                ReadinessDecision::Waiting(_) => {
                    tokio::time::sleep(policy.sample_interval).await;
                }
                ReadinessDecision::Scroll { step } => {
                    debug!(url = %parsed, step, "browser: progressive scroll for lazy content");
                    let _ = session
                        .evaluate("window.scrollBy(0, window.innerHeight); true")
                        .await;
                    tokio::time::sleep(policy.sample_interval).await;
                }
                ReadinessDecision::Ready => break,
                ReadinessDecision::TimedOut => {
                    timed_out = true;
                    break;
                }
            }
        }

        if timed_out && !tracker.dom_loaded() {
            return Err(anyhow!(
                "browser render timed out before the DOM loaded for {parsed}"
            ));
        }

        let html = self.extract_dom(session).await?;
        if html.trim().is_empty() {
            return Err(anyhow!("browser returned an empty DOM for {parsed}"));
        }
        let final_url = session
            .evaluate("location.href")
            .await
            .ok()
            .and_then(|value| value.as_str().map(str::to_string))
            .unwrap_or_else(|| parsed.to_string());
        let readiness = if timed_out {
            tracker.timed_out_report(start.elapsed())
        } else {
            tracker.report(start.elapsed())
        };
        if timed_out {
            warn!(url = %parsed, "browser: extracting DOM after render-time cap");
        }

        Ok(BrowserPage {
            url: parsed.to_string(),
            final_url,
            html,
            rendered: true,
            readiness,
        })
    }

    async fn sample_page(&self, session: &CdpSession) -> Result<PageSample> {
        let snapshot = session
            .evaluate(
                "(() => ({ \
                     readyState: document.readyState, \
                     contentSize: document.body \
                         ? document.body.innerText.length \
                         : (document.documentElement ? document.documentElement.innerHTML.length : 0) \
                 }))()",
            )
            .await?;
        Ok(PageSample {
            dom_loaded: snapshot.get("readyState").and_then(Value::as_str) == Some("complete"),
            network_in_flight: session.in_flight_requests(),
            content_size: snapshot
                .get("contentSize")
                .and_then(Value::as_u64)
                .unwrap_or(0) as usize,
        })
    }

    async fn extract_dom(&self, session: &CdpSession) -> Result<String> {
        let value = session
            .evaluate(
                "(function () { \
                     var doctype = document.doctype ? '<!DOCTYPE ' + document.doctype.name + '>' : ''; \
                     return doctype + (document.documentElement ? document.documentElement.outerHTML : ''); \
                 })()",
            )
            .await?;
        value
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| anyhow!("DOM extraction did not return a string"))
    }
}

#[async_trait]
impl BrowserFetcher for PersistentChromiumBrowser {
    async fn fetch(&self, req: BrowserRequest) -> Result<BrowserPage> {
        let _permit = self
            .gate
            .acquire()
            .await
            .context("acquiring browser concurrency slot")?;

        let parsed = validate_browser_url(&req.url)?;
        assert_public_resolution(&parsed).await?;
        let policy = policy_for(&self.config, &req);

        match self.render(&parsed, &policy).await {
            Ok(page) => Ok(page),
            Err(first_error) => {
                // A render-time cap is a page property, not a browser fault:
                // the process stays warm and the page was already cleaned up.
                if !process_must_restart(&first_error) {
                    return Err(first_error);
                }
                // Every other failure drops the process: a wedged DevTools
                // connection can survive page-level errors, so the next fetch
                // starts from a fresh browser. Retry immediately only when the
                // process actually died.
                let was_healthy = self.process_healthy().await;
                self.discard_process().await;
                if was_healthy {
                    return Err(first_error);
                }
                warn!(url = %req.url, error = %first_error, "browser: Chromium died mid-render; restarting and retrying once");
                self.render(&parsed, &policy).await.with_context(|| {
                    format!(
                        "browser render failed twice for {}; first error: {first_error}",
                        req.url
                    )
                })
            }
        }
    }
}

/// Create the production renderer when `ENABLE_HEADLESS_BROWSER` is enabled.
pub fn persistent_browser_from_env() -> Result<Option<PersistentChromiumBrowser>> {
    let enabled = std::env::var(ENABLE_HEADLESS_BROWSER)
        .ok()
        .map(|value| parse_truthy_flag(&value))
        .unwrap_or(false);
    if !enabled {
        return Ok(None);
    }
    Ok(Some(PersistentChromiumBrowser::new(
        BrowserConfig::from_env()?,
    )))
}

/// Whether a render failure requires discarding the persistent process.
/// Render-time caps leave a healthy browser warm; every other failure (CDP
/// transport, navigation, empty DOM) gets a fresh process on the next fetch.
fn process_must_restart(error: &anyhow::Error) -> bool {
    error.downcast_ref::<RenderTimeout>().is_none()
}

/// Chromium launch arguments. Deliberately **no** `--no-sandbox`: the service
/// runs as the non-root `apexintel` account so the Chromium sandbox is active.
fn chromium_launch_args(config: &BrowserConfig, profile_dir: &Path) -> Vec<String> {
    let mut args = vec![
        "--headless=new".to_string(),
        "--disable-gpu".to_string(),
        "--no-first-run".to_string(),
        "--no-default-browser-check".to_string(),
        "--disable-dev-shm-usage".to_string(),
        "--disable-extensions".to_string(),
        "--disable-background-networking".to_string(),
        "--disable-sync".to_string(),
        "--disable-translate".to_string(),
        "--disable-default-apps".to_string(),
        "--disable-component-update".to_string(),
        "--metrics-recording-only".to_string(),
        "--mute-audio".to_string(),
        "--no-pings".to_string(),
        "--remote-debugging-port=0".to_string(),
        format!("--user-data-dir={}", profile_dir.display()),
    ];
    args.extend(config.extra_flags.iter().cloned());
    args.push("about:blank".to_string());
    args
}

async fn read_devtools_url(stderr: tokio::process::ChildStderr) -> Result<String> {
    let mut lines = BufReader::new(stderr).lines();
    while let Some(line) = lines
        .next_line()
        .await
        .context("reading Chromium stderr while waiting for DevTools endpoint")?
    {
        let Some((_, url)) = line.split_once("DevTools listening on ") else {
            continue;
        };
        let url = url.trim().to_string();
        if url.is_empty() {
            continue;
        }
        // Keep draining stderr so Chromium never blocks on a full pipe.
        tokio::spawn(async move { while let Ok(Some(_)) = lines.next_line().await {} });
        return Ok(url);
    }
    Err(anyhow!(
        "Chromium exited before announcing its DevTools endpoint"
    ))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn launch_args_never_disable_the_sandbox() {
        let config = BrowserConfig::default();
        let args = chromium_launch_args(&config, Path::new("/tmp/apex-profile"));
        assert!(
            !args.iter().any(|arg| arg == "--no-sandbox"),
            "Chromium must run sandboxed under the non-root service account"
        );
        assert!(args.iter().any(|arg| arg == "--headless=new"));
        assert!(args.contains(&"--remote-debugging-port=0".to_string()));
        assert!(args.contains(&"--user-data-dir=/tmp/apex-profile".to_string()));
        assert_eq!(args.last().map(String::as_str), Some("about:blank"));
    }

    #[test]
    fn launch_args_append_extra_flags_before_the_url() {
        let config = BrowserConfig {
            extra_flags: vec!["--lang=en-US".to_string()],
            ..BrowserConfig::default()
        };
        let args = chromium_launch_args(&config, Path::new("/tmp/p"));
        let flag_index = args
            .iter()
            .position(|arg| arg == "--lang=en-US")
            .expect("extra flag present");
        assert!(flag_index < args.len() - 1, "extra flags precede the URL");
    }

    #[test]
    fn default_concurrency_matches_the_browser_budget() {
        let browser = PersistentChromiumBrowser::new(BrowserConfig::default());
        assert_eq!(browser.browser_processes(), 1);
        assert_eq!(browser.contexts_per_process(), 2);
        assert_eq!(browser.global_concurrency(), 2);
    }

    #[test]
    fn concurrency_cannot_exceed_the_browser_budget() {
        let config = BrowserConfig {
            global_concurrency: 64,
            contexts: 64,
            ..BrowserConfig::default()
        };
        let browser = PersistentChromiumBrowser::new(config);
        assert_eq!(browser.global_concurrency(), GLOBAL_BROWSER_CONCURRENCY);
        assert_eq!(browser.contexts_per_process(), BROWSER_CONTEXTS);
        assert_eq!(
            PersistentChromiumBrowser::new(BrowserConfig {
                global_concurrency: 0,
                contexts: 0,
                ..BrowserConfig::default()
            })
            .global_concurrency(),
            1
        );
    }

    #[test]
    fn config_policy_matches_contract() {
        let policy = BrowserConfig::default().policy();
        assert!(
            (MIN_NETWORK_QUIET_WINDOW..=MAX_NETWORK_QUIET_WINDOW).contains(&policy.quiet_window)
        );
        assert_eq!(policy.max_scroll_steps, DEFAULT_MAX_SCROLL_STEPS);
        assert_eq!(policy.sample_interval, DEFAULT_SAMPLE_INTERVAL);
    }

    #[test]
    fn config_from_env_rejects_quiet_windows_outside_the_band() {
        // Pure clamp helper exercised through the policy normalisation used by
        // `from_env`; env mutation is avoided to keep tests parallel-safe.
        let too_short = RenderPolicy {
            quiet_window: Duration::from_millis(1),
            ..RenderPolicy::default()
        }
        .normalized();
        assert_eq!(too_short.quiet_window, MIN_NETWORK_QUIET_WINDOW);

        let too_long = RenderPolicy {
            quiet_window: Duration::from_secs(60),
            ..RenderPolicy::default()
        }
        .normalized();
        assert_eq!(too_long.quiet_window, MAX_NETWORK_QUIET_WINDOW);
    }

    #[test]
    fn render_timeouts_keep_the_process_but_other_errors_restart_it() {
        let timeout: anyhow::Error = RenderTimeout {
            budget: Duration::from_secs(30),
            url: "https://example.com/".to_string(),
        }
        .into();
        assert!(
            !process_must_restart(&timeout),
            "a render-time cap must not throw away a healthy browser"
        );
        assert!(timeout.to_string().contains("example.com"));

        let transport: anyhow::Error = anyhow!("CDP command Runtime.evaluate failed: closed");
        assert!(
            process_must_restart(&transport),
            "transport failures must get a fresh process"
        );
    }

    /// Real-browser verification of the readiness contract. Ignored by
    /// default: run with a Chromium binary, e.g.
    /// `HEADLESS_BROWSER_BIN=chrome-headless-shell cargo test -p apex-crawl
    ///  readiness_waits -- --ignored`.
    #[tokio::test]
    #[ignore = "requires a real Chromium binary (HEADLESS_BROWSER_BIN)"]
    async fn readiness_waits_for_late_network_activity_and_lazy_scroll() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test server");
        let addr = listener.local_addr().expect("test server addr");
        let server = tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                tokio::spawn(async move {
                    let mut buffer = [0_u8; 4096];
                    let read = stream.read(&mut buffer).await.unwrap_or(0);
                    let request = String::from_utf8_lossy(&buffer[..read]);
                    let (delay, body): (Duration, &str) = if request.starts_with("GET /late") {
                        (Duration::from_millis(400), "late-network-text")
                    } else if request.starts_with("GET /favicon.ico") {
                        (Duration::ZERO, "")
                    } else {
                        (
                            Duration::ZERO,
                            "<!DOCTYPE html><html><body><div id=\"content\">initial</div>\
                             <script>\
                               setTimeout(function () {\
                                 fetch('/late').then(function (r) { return r.text(); })\
                                   .then(function (t) { document.getElementById('content').textContent += t; });\
                               }, 300);\
                               var grown = false;\
                               window.addEventListener('scroll', function () {\
                                 if (!grown) { grown = true; var d = document.createElement('div');\
                                   d.textContent = 'lazy-content-loaded'; document.body.appendChild(d); }\
                               });\
                             </script></body></html>",
                        )
                    };
                    if !delay.is_zero() {
                        tokio::time::sleep(delay).await;
                    }
                    let status = if request.starts_with("GET /favicon.ico") {
                        "HTTP/1.1 404 Not Found"
                    } else {
                        "HTTP/1.1 200 OK"
                    };
                    let response = format!(
                        "{status}\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                });
            }
        });

        let binary = match std::env::var("HEADLESS_BROWSER_BIN") {
            Ok(binary) if !binary.trim().is_empty() => PathBuf::from(binary),
            _ => {
                server.abort();
                eprintln!("skipping: set HEADLESS_BROWSER_BIN to a Chromium binary");
                return;
            }
        };
        let config = BrowserConfig {
            chrome_binary: binary,
            max_render_time: Duration::from_secs(20),
            quiet_window: Duration::from_millis(750),
            sample_interval: Duration::from_millis(200),
            max_scroll_steps: 3,
            ..BrowserConfig::default()
        };
        let browser = PersistentChromiumBrowser::new(config.clone());
        let url = Url::parse(&format!("http://{addr}/")).expect("test url");
        let page = browser
            .render(&url, &config.policy())
            .await
            .expect("readiness-gated render");
        server.abort();

        assert!(
            page.html.contains("initial"),
            "base content missing: {:.200}",
            page.html
        );
        assert!(
            page.html.contains("late-network-text"),
            "renderer extracted before the late fetch settled: {:.200}",
            page.html
        );
        assert!(
            page.html.contains("lazy-content-loaded"),
            "progressive scroll did not trigger lazy content: {:.200}",
            page.html
        );
        assert!(page.readiness.dom_loaded);
        assert!(page.readiness.network_quiet_achieved);
        assert!(
            page.readiness.samples >= 2,
            "readiness must sample at least twice"
        );
        assert!(
            page.readiness.scroll_steps >= 1,
            "lazy content requires at least one scroll step"
        );
        assert!(
            !page.readiness.timed_out,
            "should settle well within budget"
        );
    }
}
