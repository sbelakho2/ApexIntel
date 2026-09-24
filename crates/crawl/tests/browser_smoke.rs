//! Manual smoke test for the persistent Chromium/CDP renderer.
//!
//! This test is `#[ignore]`d because it needs a real Chromium binary and
//! public network access. Run it after touching the renderer:
//!
//! ```text
//! HEADLESS_BROWSER_BIN="/path/to/chrome" \
//!   cargo test -p apex-crawl --test browser_smoke -- --ignored --nocapture
//! ```
//!
//! It exercises the whole production path: process launch (without
//! `--no-sandbox`), DevTools connect, context/target creation, readiness
//! sampling, network-quiet detection and DOM extraction.

use std::time::Duration;

use apex_crawl::{BrowserConfig, BrowserFetcher, BrowserRequest, PersistentChromiumBrowser};

#[tokio::test]
#[ignore = "requires a real Chromium binary (HEADLESS_BROWSER_BIN) and public network access"]
async fn renders_public_page_with_readiness() {
    let binary =
        std::env::var("HEADLESS_BROWSER_BIN").unwrap_or_else(|_| "google-chrome".to_string());
    let config = BrowserConfig {
        chrome_binary: binary.into(),
        max_render_time: Duration::from_secs(30),
        ..BrowserConfig::default()
    };
    let browser = PersistentChromiumBrowser::new(config);

    let page = browser
        .fetch(BrowserRequest::new("https://example.com/"))
        .await
        .expect("production renderer should render example.com");

    assert!(
        page.html.contains("Example Domain"),
        "rendered DOM should contain page content: {:.200}",
        page.html
    );
    assert!(page.rendered);
    assert!(page.readiness.dom_loaded, "DOM must reach complete");
    assert!(
        page.readiness.network_quiet_achieved,
        "network quiet window must be satisfied"
    );
    assert!(
        page.readiness.samples >= 2,
        "readiness needs at least two samples"
    );
    assert!(!page.readiness.timed_out);
    assert!(page.is_usable());
}
