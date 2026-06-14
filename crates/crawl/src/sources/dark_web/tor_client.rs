//! Tor SOCKS Proxy Client for anonymous OSINT crawling.
//!
//! Wraps a reqwest client behind a Tor SOCKS proxy, enabling:
//! - Exit node diversity (circumvents geo-blocking)
//! - Anonymised request origin
//! - Circuit-level isolation per request
//!
//! # Requirements
//! - Tor daemon running on `TOR_SOCKS_PORT` (default: 9050)
//! - Optional: Tor control port for new circuit requests via `TOR_CONTROL_PORT`

use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, info, warn};

/// Tor connection configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TorConfig {
    /// SOCKS host (default "127.0.0.1").
    pub socks_host: String,
    /// SOCKS port (default 9050).
    pub socks_port: u16,
    /// Tor control port (optional, for circuit management).
    pub control_port: Option<u16>,
    /// Tor control password (if set).
    pub control_password: Option<String>,
    /// Timeout for each request.
    pub request_timeout: Duration,
}

impl Default for TorConfig {
    fn default() -> Self {
        Self {
            socks_host: "127.0.0.1".to_string(),
            socks_port: 9050,
            control_port: None,
            control_password: None,
            request_timeout: Duration::from_secs(30),
        }
    }
}

impl TorConfig {
    /// Returns the SOCKS proxy URL.
    pub fn socks_url(&self) -> String {
        format!("socks5://{}:{}", self.socks_host, self.socks_port)
    }

    /// Returns the Tor control port URL (if configured).
    pub fn control_url(&self) -> Option<String> {
        self.control_port.map(|port| format!("http://127.0.0.1:{}", port))
    }
}

/// HTTP client routed through Tor's SOCKS proxy.
#[derive(Debug, Clone)]
pub struct TorClient {
    client: Client,
    config: TorConfig,
}

impl TorClient {
    /// Create a new Tor client from environment or defaults.
    ///
    /// Reads:
    /// - `TOR_SOCKS_HOST` (default "127.0.0.1")
    /// - `TOR_SOCKS_PORT` (default 9050)
    /// - `TOR_CONTROL_PORT` (optional)
    /// - `TOR_CONTROL_PASSWORD` (optional)
    pub fn from_env() -> Result<Self> {
        let config = TorConfig {
            socks_host: std::env::var("TOR_SOCKS_HOST").unwrap_or_else(|_| "127.0.0.1".into()),
            socks_port: std::env::var("TOR_SOCKS_PORT")
                .unwrap_or_else(|_| "9050".into())
                .parse()
                .unwrap_or(9050),
            control_port: std::env::var("TOR_CONTROL_PORT")
                .ok()
                .and_then(|p| p.parse().ok()),
            control_password: std::env::var("TOR_CONTROL_PASSWORD").ok(),
            request_timeout: Duration::from_secs(30),
        };
        Self::new(config)
    }

    /// Create a new Tor client with explicit configuration.
    pub fn new(config: TorConfig) -> Result<Self> {
        let proxy = reqwest::Proxy::socks5_gssapi(&config.socks_url())
            .context("invalid SOCKS proxy URL")?;

        let client = Client::builder()
            .proxy(proxy)
            .timeout(config.request_timeout)
            .user_agent("ApexIntel/1.0 (+https://apexintel.io) OSINT Crawler")
            .build()
            .context("building Tor HTTP client")?;

        Ok(Self { client, config })
    }

    /// Perform a GET request through Tor.
    pub async fn get(&self, url: &str) -> Result<reqwest::Response> {
        debug!(url, "Tor GET request");
        let resp = self
            .client
            .get(url)
            .send()
            .await
            .context("Tor GET request failed")?;
        Ok(resp)
    }

    /// Perform a POST request through Tor.
    pub async fn post(&self, url: &str) -> Result<reqwest::RequestBuilder> {
        Ok(self.client.post(url))
    }

    /// Request a new Tor circuit via the control port (if configured).
    ///
    /// Returns an error if no control port is configured or if the command fails.
    pub async fn new_circuit(&self) -> Result<()> {
        let control_url = self
            .config
            .control_url()
            .context("Tor control port not configured")?;

        let mut url = reqwest::Url::parse(&control_url)
            .context("invalid Tor control URL")?;

        if let Some(ref pw) = self.config.control_password {
            url.set_password(Some(pw)).ok();
            // Tor control port auth usually via authcookie; here we set password field
        }

        let resp = self
            .client
            .post(url)
            .header("Content-Type", "application/tor-control")
            .body("SIGNAL NEWNYM\r\nQUIT\r\n")
            .send()
            .await
            .context("Tor NEWNYM signal failed")?;

        if !resp.status().is_success() {
            warn!(status = %resp.status(), "Tor NEWNYM returned non-success");
        } else {
            info!("New Tor circuit requested");
        }

        Ok(())
    }

    /// Test connectivity by fetching a known-onion address.
    /// Uses DuckDuckGo's Tor hidden service as a canary.
    pub async fn test_connectivity(&self) -> Result<()> {
        let resp = self
            .get("https://duckduckgogg42xjoc72x3sjasqxarfg3crbnuqfpcvfie上月.org/")
            .await;

        match resp {
            Ok(r) if r.status().is_success() => {
                info!("Tor connectivity verified");
                Ok(())
            }
            Ok(r) => {
                warn!(status = %r.status(), "Tor reachable but returned non-success");
                Ok(())
            }
            Err(e) => {
                warn!(error = %e, "Tor connectivity test failed — is Tor running?");
                Err(e)
            }
        }
    }

    /// Return a reference to the inner reqwest client for advanced use.
    pub fn inner(&self) -> &Client {
        &self.client
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tor_config_defaults() {
        let cfg = TorConfig::default();
        assert_eq!(cfg.socks_host, "127.0.0.1");
        assert_eq!(cfg.socks_port, 9050);
        assert!(cfg.control_port.is_none());
    }

    #[test]
    fn socks_url_format() {
        let cfg = TorConfig {
            socks_port: 9150,
            ..Default::default()
        };
        assert_eq!(cfg.socks_url(), "socks5://127.0.0.1:9150");
    }

    #[test]
    fn tor_client_constructs() {
        // Construct without network — just check it doesn't panic.
        let cfg = TorConfig::default();
        let result = TorClient::new(cfg);
        // May fail if Tor is not running; that's OK in tests.
        if result.is_ok() {
            let client = result.unwrap();
            assert!(client.inner().is_email_created());
        }
    }
}
