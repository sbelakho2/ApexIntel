//! Ignored integration test: a `Browser`-strategy source is rendered with the
//! real persistent Chromium renderer against a local JS-only fixture page, and
//! the extracted text carries the JS-injected content — i.e. the crawl path
//! ingests the RENDERED DOM, not the raw fixture HTML.
//!
//! `#[ignore]`d by default (pattern: `crates/store/tests/migrations_integration.rs`);
//! requires a real Chromium binary:
//!
//! ```text
//! HEADLESS_BROWSER_BIN="/path/to/chrome" \
//!   cargo test -p apex-crawl --test browser_dispatch_render -- --ignored --nocapture
//! ```
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use apex_crawl::browser::{BrowserFetcher, BrowserRequest};
use apex_crawl::sources::{
    dispatch_source_fetch, Category, FetchDispatch, FetchStrategy, Region, Source, SourceCapability,
};
use apex_crawl::{BrowserConfig, PersistentChromiumBrowser};

/// JS-only fixture: the result element is empty in the served HTML and only
/// the script fills it in.
const FIXTURE: &str = "<!DOCTYPE html><html><body>\
<div id=\"result\"></div>\
<script>document.getElementById('result').innerHTML = 'ApexIntel browser rendered';</script>\
</body></html>";

/// Tiny in-process HTTP server for the fixture page.
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
                let (status, body): (&str, &str) = if request.starts_with("GET /favicon.ico") {
                    ("404 Not Found", "")
                } else {
                    ("200 OK", FIXTURE)
                };
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

fn browser_strategy_source(url: &str) -> Source {
    Source {
        slug: "js_only_social_source".to_string(),
        name: "JS-only social source".to_string(),
        url: url.to_string(),
        search_param: None,
        region: Region::Global,
        category: Category::SocialMedia,
        tier: 4,
        needs_proxy: false,
        rss_url: None,
        enabled: true,
        min_interval_minutes: 60,
        fetch_strategy: None,
        capability: SourceCapability::Operational,
        notes: None,
    }
}

#[tokio::test]
#[ignore = "requires a real Chromium binary (HEADLESS_BROWSER_BIN)"]
async fn browser_dispatch_renders_js_only_page_and_ingests_rendered_text() {
    let binary = match std::env::var("HEADLESS_BROWSER_BIN") {
        Ok(binary) if !binary.trim().is_empty() => PathBuf::from(binary),
        _ => {
            eprintln!("skipping: set HEADLESS_BROWSER_BIN to a Chromium binary");
            return;
        }
    };

    let (addr, server) = start_fixture_server().await;
    let source = browser_strategy_source(&format!("http://{addr}/"));
    assert_eq!(source.strategy(), FetchStrategy::Browser);

    let config = BrowserConfig {
        chrome_binary: binary,
        max_render_time: Duration::from_secs(20),
        quiet_window: Duration::from_millis(750),
        sample_interval: Duration::from_millis(200),
        max_scroll_steps: 1,
        allow_private_hosts: true,
        ..BrowserConfig::default()
    };
    let browser: Arc<dyn BrowserFetcher> = Arc::new(PersistentChromiumBrowser::new(config));

    let dispatch = dispatch_source_fetch(&source, true).expect("browser capability is enabled");
    assert_eq!(dispatch, FetchDispatch::Browser);

    let page = browser
        .fetch(BrowserRequest::new(source.url.clone()))
        .await
        .expect("real renderer renders the JS-only fixture");
    server.abort();

    assert!(page.rendered, "page must be marked as browser-rendered");
    assert!(
        page.html.contains("ApexIntel browser rendered"),
        "rendered DOM must contain JS-injected content: {:.300}",
        page.html
    );

    // The crawl cycle ingests parsed page text, not the raw fixture HTML:
    // the JS-injected marker must survive content extraction.
    let extracted =
        apex_parse::html::extract_page(&page.html).expect("extract the rendered page text");
    assert!(
        extracted.body_text.contains("ApexIntel browser rendered"),
        "extracted text must contain the rendered marker: {:.300}",
        extracted.body_text
    );
}
