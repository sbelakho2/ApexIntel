//! Fixture-backed [`BrowserFetcher`] used by tests.
//!
//! Production code never constructs this; it keeps the browser tests and the
//! LinkedIn dynamic-page test hermetic (no Chromium required).

use std::collections::HashMap;
use std::time::Duration;

use anyhow::{anyhow, Result};
use async_trait::async_trait;

use crate::browser::readiness::ReadinessReport;
use crate::browser::{BrowserFetcher, BrowserPage, BrowserRequest};

/// A fetcher that serves recorded pages by exact URL.
pub(crate) struct RecordedPagesBrowser {
    pages: HashMap<String, String>,
}

impl RecordedPagesBrowser {
    pub(crate) fn new(pages: impl IntoIterator<Item = (String, String)>) -> Self {
        Self {
            pages: pages.into_iter().collect(),
        }
    }
}

#[async_trait]
impl BrowserFetcher for RecordedPagesBrowser {
    async fn fetch(&self, req: BrowserRequest) -> Result<BrowserPage> {
        let html = self
            .pages
            .get(&req.url)
            .cloned()
            .ok_or_else(|| anyhow!("no recorded browser fixture for {}", req.url))?;
        Ok(BrowserPage {
            url: req.url.clone(),
            final_url: req.url,
            html,
            rendered: true,
            readiness: ReadinessReport {
                samples: 2,
                waited: Duration::ZERO,
                network_quiet_achieved: true,
                content_stable: true,
                dom_loaded: true,
                scroll_steps: 0,
                timed_out: false,
            },
        })
    }
}
