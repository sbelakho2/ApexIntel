//! The single sanctioned HTTP client construction point.
//!
//! Every fetch of external (attacker-influenced or crawled) content must
//! build its `reqwest::Client` through [`external_client`] or
//! [`external_client_with`]. The factory applies the crawler's SSRF posture:
//!
//! - the DNS resolver only yields public addresses,
//! - every redirect hop is validated with
//!   [`crate::browser::validation::validate_redirect_target`] and a hard hop
//!   cap (query strings keep their `&`; only schemes and hosts are policed),
//! - bodies are read through the shared capped reader [`read_capped`].
//!
//! `clippy.toml` rejects direct `reqwest::Client::builder` /
//! `reqwest::Client::new` / `reqwest::ClientBuilder::new` construction; the
//! `#[allow(clippy::disallowed_methods)]` attributes inside this module are
//! the only sanctioned construction point. Operator-configured endpoints
//! (LLM servers, webhooks, health probes) are the documented exemption and
//! carry their own one-line allow with a reason.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use reqwest::header::HeaderMap;
use url::Url;

use crate::browser::validation::{is_private_host, validate_redirect_target};

/// Bounded default request timeout for sites that previously had none.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// Maximum redirect hops followed by the guarded policy.
pub const MAX_REDIRECTS: usize = 5;

/// Options for [`external_client_with`].
///
/// [`Default`] is the guarded baseline (30s timeout, public-only DNS,
/// SSRF-validated redirects up to [`MAX_REDIRECTS`], no proxy). Call sites
/// override only what they previously configured explicitly.
pub struct ExternalClientOptions {
    pub timeout: Duration,
    pub proxy: Option<reqwest::Proxy>,
    pub user_agent: Option<String>,
    pub default_headers: Option<HeaderMap>,
    /// Test/dev escape hatch: skip the public-only resolver and redirect
    /// validation. Production must keep this `false`.
    pub allow_private_targets: bool,
    /// Optional TCP connect timeout (reqwest ships none by default).
    pub connect_timeout: Option<Duration>,
    /// Tor-only escape hatch for self-signed onion certificates. Never set
    /// this for clearnet fetches.
    pub danger_accept_invalid_certs: bool,
    /// Persistent cookie jar for session-based scrapers.
    pub cookie_store: bool,
    /// `None` disables redirects entirely (the 3xx is returned to the
    /// caller); `Some(n)` follows up to `n` SSRF-validated hops.
    pub redirect: Option<usize>,
    /// Optional `pool_max_idle_per_host` override.
    pub pool_max_idle_per_host: Option<usize>,
}

impl Default for ExternalClientOptions {
    fn default() -> Self {
        Self {
            timeout: DEFAULT_TIMEOUT,
            proxy: None,
            user_agent: None,
            default_headers: None,
            allow_private_targets: false,
            connect_timeout: None,
            danger_accept_invalid_certs: false,
            cookie_store: false,
            redirect: Some(MAX_REDIRECTS),
            pool_max_idle_per_host: None,
        }
    }
}

/// Build a guarded client from the audit's minimal signature.
#[allow(clippy::disallowed_methods)] // THE sanctioned construction point (see module docs).
pub fn external_client(
    timeout: Duration,
    proxy: Option<reqwest::Proxy>,
) -> reqwest::Result<reqwest::Client> {
    external_client_with(ExternalClientOptions {
        timeout,
        proxy,
        ..ExternalClientOptions::default()
    })
}

/// Build a guarded client with the full option set.
#[allow(clippy::disallowed_methods)] // THE sanctioned construction point (see module docs).
pub fn external_client_with(options: ExternalClientOptions) -> reqwest::Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder()
        .timeout(options.timeout)
        .redirect(redirect_policy(
            options.redirect,
            options.allow_private_targets,
        ));
    if let Some(connect_timeout) = options.connect_timeout {
        builder = builder.connect_timeout(connect_timeout);
    }
    if let Some(user_agent) = options.user_agent {
        builder = builder.user_agent(user_agent);
    }
    if let Some(headers) = options.default_headers {
        builder = builder.default_headers(headers);
    }
    if let Some(proxy) = options.proxy {
        builder = builder.proxy(proxy);
    }
    if options.danger_accept_invalid_certs {
        builder = builder.danger_accept_invalid_certs(true);
    }
    if options.cookie_store {
        builder = builder.cookie_store(true);
    }
    if let Some(max_idle_per_host) = options.pool_max_idle_per_host {
        builder = builder.pool_max_idle_per_host(max_idle_per_host);
    }
    if !options.allow_private_targets {
        builder = builder.dns_resolver(Arc::new(PublicOnlyResolver));
    }
    builder.build()
}

/// Infallible construction shape for legacy constructors that returned
/// `Self` and fell back to `reqwest::Client::new()`.
///
/// reqwest documents `Client::new()` to panic when the TLS backend (or the
/// resolver) cannot be initialised, so this preserves the effective behavior
/// of the fallback while keeping even the degenerate path guarded — the
/// process never gets an unguarded client.
pub fn external_client_or_panic(options: ExternalClientOptions) -> reqwest::Client {
    external_client_with(options)
        .unwrap_or_else(|error| panic!("guarded HTTP client construction failed: {error}"))
}

/// DNS resolver that only yields public addresses. reqwest skips DNS for
/// IP-literal URLs, so [`url_allowed`] is also enforced on entry URLs and the
/// redirect policy classifies every hop.
#[derive(Debug)]
struct PublicOnlyResolver;

impl Resolve for PublicOnlyResolver {
    fn resolve(&self, name: Name) -> Resolving {
        Box::pin(async move {
            let addrs: Vec<SocketAddr> = tokio::net::lookup_host((name.as_str(), 0))
                .await?
                .filter(|addr| !is_private_host(&addr.ip().to_string()))
                .collect();
            if addrs.is_empty() {
                return Err("resolved only to non-public addresses".into());
            }
            Ok(Box::new(addrs.into_iter()) as Addrs)
        })
    }
}

fn redirect_policy(
    max_hops: Option<usize>,
    allow_private_targets: bool,
) -> reqwest::redirect::Policy {
    match max_hops {
        None => reqwest::redirect::Policy::none(),
        Some(max_hops) => reqwest::redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() >= max_hops {
                attempt.error("too many redirects")
            } else if allow_private_targets || redirect_hop_allowed(attempt.url()) {
                attempt.follow()
            } else {
                attempt.stop()
            }
        }),
    }
}

/// Validate one redirect hop with [`validate_redirect_target`]: length-bounded,
/// http(s) only, and private/loopback/link-local/CGNAT/metadata hosts rejected
/// (IP literals are classified directly, so they never reach DNS). Unlike the
/// navigation validator it deliberately does not screen shell metacharacters,
/// so legitimate query strings containing `&` survive redirect following.
fn redirect_hop_allowed(url: &Url) -> bool {
    validate_redirect_target(url.as_str()).is_ok()
}

/// http(s) only, and the host must not be a private/metadata address.
/// reqwest skips DNS for IP-literal hosts, so callers must check every entry
/// URL before handing it to a guarded client.
pub(crate) fn url_allowed(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https")
        && url.host_str().is_some_and(|host| !is_private_host(host))
}

/// Default body cap for ordinary external fetches (10 MiB).
pub const MAX_EXTERNAL_BODY_BYTES: usize = 10 * 1024 * 1024;

/// Body cap for operator-configured bulk list downloads (sanctions, export
/// control); the published files legitimately exceed 10 MiB, but a hostile or
/// broken server must still not be able to exhaust memory.
pub const MAX_BULK_BODY_BYTES: usize = 128 * 1024 * 1024;

/// Read a response body with a hard byte cap, decoding as lossy UTF-8 so a
/// hostile or broken server cannot exhaust memory with an unbounded body.
pub async fn read_capped(resp: reqwest::Response, max: usize) -> anyhow::Result<String> {
    let bytes = read_capped_bytes(resp, max).await?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Read a response body with the same hard byte cap and parse it as JSON.
///
/// Every `.json()` on a guarded client is a residual SSRF-adjacent memory
/// hazard: reqwest buffers the whole body unboundedly before deserializing.
/// This helper applies [`read_capped_bytes`] first, so a hostile server cannot
/// make the process allocate an arbitrary body.
pub async fn read_capped_json<T: serde::de::DeserializeOwned>(
    resp: reqwest::Response,
    max: usize,
) -> anyhow::Result<T> {
    let bytes = read_capped_bytes(resp, max).await?;
    serde_json::from_slice(&bytes).map_err(|error| anyhow::anyhow!("parse JSON body: {error}"))
}

/// Byte-returning variant of [`read_capped`].
pub async fn read_capped_bytes(mut resp: reqwest::Response, max: usize) -> anyhow::Result<Vec<u8>> {
    // Content-length pre-check: never reserve more than the cap even when the
    // server announces a huge body.
    let initial_capacity = match resp.content_length() {
        Some(len) => max.min(usize::try_from(len).unwrap_or(max)),
        None => 0,
    };
    let mut bytes: Vec<u8> = Vec::with_capacity(initial_capacity);
    loop {
        let chunk = resp
            .chunk()
            .await
            .map_err(|error| anyhow::anyhow!("read response body: {}", error.without_url()))?;
        let Some(chunk) = chunk else {
            break;
        };
        if bytes.len() + chunk.len() > max {
            let remaining = max.saturating_sub(bytes.len());
            bytes.extend_from_slice(&chunk[..remaining]);
            break;
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    use std::collections::VecDeque;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::sync::Mutex as TokioMutex;

    /// Minimal one-request-per-response HTTP/1.1 test server on loopback.
    async fn start_test_server(responses: Vec<String>) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local addr");
        let queue = Arc::new(TokioMutex::new(
            responses.into_iter().collect::<VecDeque<_>>(),
        ));
        tokio::spawn({
            let queue = queue.clone();
            async move {
                loop {
                    let next_response = { queue.lock().await.pop_front() };
                    let Some(response) = next_response else {
                        break;
                    };
                    let Ok((mut stream, _)) = listener.accept().await else {
                        break;
                    };
                    let mut buf = [0_u8; 2048];
                    let _ = stream.read(&mut buf).await;
                    let _ = stream.write_all(response.as_bytes()).await;
                }
            }
        });
        addr
    }

    #[tokio::test]
    async fn factory_stops_redirect_to_private_ip_literal() {
        let addr = start_test_server(vec![
            "HTTP/1.1 302 Found\r\nLocation: http://169.254.169.254/latest/meta-data/\r\nConnection: close\r\nContent-Length: 0\r\n\r\n".to_string(),
        ])
        .await;
        let client = external_client(Duration::from_secs(5), None).expect("build guarded client");
        let response = client
            .get(format!("http://{addr}/start"))
            .send()
            .await
            .expect("entry request should reach the loopback test server");

        assert_eq!(
            response.status().as_u16(),
            302,
            "redirect to a private IP literal must be stopped, not followed"
        );
        assert_eq!(
            response.url().host_str(),
            Some("127.0.0.1"),
            "the metadata service must never be contacted"
        );
    }

    #[tokio::test]
    async fn allow_private_targets_escape_hatch_follows_redirects() {
        let target = start_test_server(vec![
            "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Type: text/plain\r\nContent-Length: 2\r\n\r\nok"
                .to_string(),
        ])
        .await;
        let redirect = start_test_server(vec![format!(
            "HTTP/1.1 302 Found\r\nLocation: http://{target}/ok\r\nConnection: close\r\nContent-Length: 0\r\n\r\n"
        )])
        .await;

        let client = external_client_with(ExternalClientOptions {
            timeout: Duration::from_secs(5),
            allow_private_targets: true,
            ..ExternalClientOptions::default()
        })
        .expect("build escape-hatch client");
        let body = client
            .get(format!("http://{redirect}/start"))
            .send()
            .await
            .expect("send")
            .text()
            .await
            .expect("read body");

        assert_eq!(body, "ok");
    }

    fn self_redirect_responses(count: usize) -> Vec<String> {
        (0..count)
            .map(|hop| {
                format!(
                    "HTTP/1.1 302 Found\r\nLocation: /hop-{hop}\r\nConnection: close\r\nContent-Length: 0\r\n\r\n"
                )
            })
            .collect()
    }

    #[tokio::test]
    async fn redirect_chain_is_capped_at_max_redirects() {
        // An endless redirect loop must end in a redirect error after at most
        // MAX_REDIRECTS hops instead of following forever.
        let addr = start_test_server(self_redirect_responses(MAX_REDIRECTS + 3)).await;
        let client = external_client_with(ExternalClientOptions {
            timeout: Duration::from_secs(5),
            allow_private_targets: true,
            ..ExternalClientOptions::default()
        })
        .expect("build client");

        let error = client
            .get(format!("http://{addr}/start"))
            .send()
            .await
            .expect_err("an unbounded redirect chain must fail");
        assert!(
            error.is_redirect(),
            "expected a redirect error, got {error}"
        );
    }

    #[tokio::test]
    async fn redirect_none_returns_the_3xx_to_the_caller() {
        let addr = start_test_server(self_redirect_responses(1)).await;
        let client = external_client_with(ExternalClientOptions {
            timeout: Duration::from_secs(5),
            allow_private_targets: true,
            redirect: None,
            ..ExternalClientOptions::default()
        })
        .expect("build client");

        let response = client
            .get(format!("http://{addr}/start"))
            .send()
            .await
            .expect("a disabled redirect policy returns the response");
        assert_eq!(response.status().as_u16(), 302);
        assert_eq!(response.url().path(), "/start");
    }

    #[tokio::test]
    async fn read_capped_truncates_at_the_limit() {
        let body = "a".repeat(4096);
        let response = format!(
            "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        let addr = start_test_server(vec![response]).await;
        let resp = external_client(Duration::from_secs(5), None)
            .expect("build client")
            .get(format!("http://{addr}/big"))
            .send()
            .await
            .expect("send");

        let text = read_capped(resp, 1024).await.expect("capped read");
        assert_eq!(text.len(), 1024);
        assert!(text.chars().all(|c| c == 'a'));
    }

    #[tokio::test]
    async fn read_capped_returns_short_bodies_intact() {
        let addr = start_test_server(vec![
            "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Type: text/plain\r\nContent-Length: 5\r\n\r\nhello"
                .to_string(),
        ])
        .await;
        let resp = external_client(Duration::from_secs(5), None)
            .expect("build client")
            .get(format!("http://{addr}/small"))
            .send()
            .await
            .expect("send");

        let text = read_capped(resp, 64).await.expect("capped read");
        assert_eq!(text, "hello");
    }

    #[tokio::test]
    async fn read_capped_json_parses_bodies_within_the_cap() {
        let body = r#"{"host":"example.com","count":3}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        let addr = start_test_server(vec![response]).await;
        let resp = external_client(Duration::from_secs(5), None)
            .expect("build client")
            .get(format!("http://{addr}/json"))
            .send()
            .await
            .expect("send");

        #[derive(serde::Deserialize)]
        struct Payload {
            host: String,
            count: u32,
        }
        let payload: Payload = read_capped_json(resp, 1024)
            .await
            .expect("capped JSON read");
        assert_eq!(payload.host, "example.com");
        assert_eq!(payload.count, 3);
    }

    #[tokio::test]
    async fn read_capped_json_never_buffers_an_oversized_hostile_body() {
        // The JSON is cut at the cap *before* deserialization, so a hostile
        // server cannot force an unbounded allocation and the truncated
        // document fails with an explicit parse error.
        let filler = "a".repeat(4096);
        let body = format!(r#"{{"blob":"{filler}"}}"#);
        let response = format!(
            "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        let addr = start_test_server(vec![response]).await;
        let resp = external_client(Duration::from_secs(5), None)
            .expect("build client")
            .get(format!("http://{addr}/hostile.json"))
            .send()
            .await
            .expect("send");

        let error = read_capped_json::<serde_json::Value>(resp, 256)
            .await
            .expect_err("a body truncated at the cap cannot deserialize");
        assert!(
            error.to_string().contains("parse JSON body"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn url_allowed_rejects_private_and_non_http() {
        assert!(url_allowed(
            &Url::parse("https://example.com/article").expect("url parses")
        ));
        assert!(url_allowed(
            &Url::parse("http://93.184.216.34/").expect("url parses")
        ));
        for blocked in [
            "http://127.0.0.1:8222/",
            "http://169.254.169.254/latest/meta-data/",
            "http://[::1]/",
            "http://[::ffff:10.0.0.1]/",
            "http://localhost/",
            "file:///etc/passwd",
            "ftp://example.com/x",
        ] {
            assert!(
                !url_allowed(&Url::parse(blocked).expect("url parses")),
                "must be blocked: {blocked}"
            );
        }
    }
}
