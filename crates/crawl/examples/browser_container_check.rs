//! Container-level browser integration probe.
//!
//! Renders a JS-only fixture through the real persistent Chromium renderer and
//! fails unless the extracted DOM carries BOTH markers:
//!
//! - `late-network-text`: a `fetch()` resolved 400 ms after load (the renderer
//!   must wait for the network to settle), and
//! - `lazy-content-loaded`: DOM added by the scroll listener (the renderer's
//!   bounded progressive scroll must trigger lazy content).
//!
//! The worker container image builds this probe into its `browser-check` stage
//! (`Dockerfile.worker`) and `scripts/ci/browser_container_check.sh` runs it
//! inside that image, proving the shipped image can render browser-strategy
//! sources end to end. It is not part of the production image.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::time::Duration;

use apex_crawl::browser::{BrowserFetcher, BrowserRequest};
use apex_crawl::{BrowserConfig, PersistentChromiumBrowser};

/// The page is tall so `window.scrollBy(...)` always produces a scroll event
/// even on a large headless viewport.
const FIXTURE: &str = "<!DOCTYPE html><html><body>\
<div id=\"content\">initial</div>\
<div style=\"height:3000px\"></div>\
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
</script></body></html>";

/// Tiny in-process HTTP server: `/late` answers after 400 ms with the
/// late-network marker; every other path serves the fixture.
async fn start_fixture_server() -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fixture server");
    let addr = listener.local_addr().expect("fixture server addr");
    let handle = tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let mut buffer = [0_u8; 4096];
                let read = stream.read(&mut buffer).await.unwrap_or(0);
                if read == 0 {
                    return;
                }
                let request = String::from_utf8_lossy(&buffer[..read]);
                let (status, delay, body): (&str, Duration, &str) =
                    if request.starts_with("GET /late") {
                        ("200 OK", Duration::from_millis(400), "late-network-text")
                    } else if request.starts_with("GET /favicon.ico") {
                        ("404 Not Found", Duration::ZERO, "")
                    } else {
                        ("200 OK", Duration::ZERO, FIXTURE)
                    };
                if !delay.is_zero() {
                    tokio::time::sleep(delay).await;
                }
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
            });
        }
    });
    (addr, handle)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let binary = std::env::var("HEADLESS_BROWSER_BIN")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("HEADLESS_BROWSER_BIN must point at a Chromium binary"))?;

    let (addr, server) = start_fixture_server().await;
    let config = BrowserConfig {
        chrome_binary: binary,
        max_render_time: Duration::from_secs(30),
        quiet_window: Duration::from_millis(750),
        sample_interval: Duration::from_millis(200),
        max_scroll_steps: 3,
        // The fixture is a loopback server; this is the test-only opt-in.
        allow_private_hosts: true,
        ..BrowserConfig::default()
    };
    let browser = PersistentChromiumBrowser::new(config);
    let page = browser
        .fetch(BrowserRequest::new(format!("http://{addr}/")))
        .await
        .map_err(|error| anyhow::anyhow!("container browser render failed: {error:#}"))?;
    server.abort();

    for marker in ["initial", "late-network-text", "lazy-content-loaded"] {
        anyhow::ensure!(
            page.html.contains(marker),
            "rendered DOM is missing {marker:?}: {:.300}",
            page.html
        );
    }
    anyhow::ensure!(page.rendered, "page must be marked as browser-rendered");
    anyhow::ensure!(
        page.readiness.network_quiet_achieved,
        "renderer extracted before the network settled: {:?}",
        page.readiness
    );
    anyhow::ensure!(
        page.readiness.scroll_steps >= 1,
        "lazy content requires at least one progressive scroll step: {:?}",
        page.readiness
    );
    anyhow::ensure!(
        !page.readiness.timed_out,
        "fixture must settle well within the render budget: {:?}",
        page.readiness
    );

    println!("BROWSER_CONTAINER_CHECK_OK");
    Ok(())
}
