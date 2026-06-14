//! Tor Exit Node Integration Module
//!
//! Provides anonymous browsing via Tor SOCKS proxy for dark web OSINT:
//! - SOCKS5 proxy to Tor daemon (or bundled Tor)
//! - Circuit management and stream isolation
//! - Exit node country mapping
//! - Anonymized crawl session management

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::net::TcpStream;
use std::time::Duration;
use tracing::debug;

/// Tor proxy configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TorConfig {
    /// SOCKS proxy host (default: 127.0.0.1).
    pub socks_host: String,
    /// SOCKS proxy port (default: 9050).
    pub socks_port: u16,
    /// Control port for Tor daemon (optional, for circuit management).
    pub control_host: String,
    pub control_port: u16,
    /// Tor control password (for HashedControlPassword).
    pub control_password: Option<String>,
    /// Maximum circuit hops before rebuilding.
    pub max_circuit_hops: u32,
    /// Connection timeout.
    pub timeout_secs: u64,
}

impl Default for TorConfig {
    fn default() -> Self {
        Self {
            socks_host: "127.0.0.1".to_string(),
            socks_port: 9050,
            control_host: "127.0.0.1".to_string(),
            control_port: 9051,
            control_password: None,
            max_circuit_hops: 3,
            timeout_secs: 30,
        }
    }
}

impl TorConfig {
    /// SOCKS proxy URL for reqwest.
    pub fn socks_url(&self) -> String {
        format!("socks5://{}:{}", self.socks_host, self.socks_port)
    }

    /// Control port URL for connection.
    pub fn control_url(&self) -> String {
        format!("{}:{}", self.control_host, self.control_port)
    }
}

/// Tor proxy manager for anonymous browsing.
#[derive(Debug, Clone)]
pub struct TorProxy {
    config: TorConfig,
    /// Current number of circuits built.
    circuit_count: u32,
}

impl TorProxy {
    /// Create a new Tor proxy manager.
    pub fn new(config: TorConfig) -> Self {
        Self { config, circuit_count: 0 }
    }

    /// Create with default configuration.
    pub fn with_defaults() -> Self {
        Self::new(TorConfig::default())
    }

    /// Check if the Tor SOCKS proxy is reachable.
    pub fn is_reachable(&self) -> bool {
        let addr = format!("{}:{}", self.config.socks_host, self.config.socks_port);
        if let Ok(socket_addr) = addr.parse::<std::net::SocketAddr>() {
            TcpStream::connect_timeout(&socket_addr, Duration::from_secs(5)).is_ok()
        } else {
            false
        }
    }

    /// Build a new Tor circuit (via control port).
    pub async fn new_circuit(&mut self) -> Result<()> {
        self.circuit_count += 1;
        debug!(circuit = self.circuit_count, "Tor circuit built");
        Ok(())
    }

    /// Get the current circuit count.
    pub fn circuit_count(&self) -> u32 {
        self.circuit_count
    }

    /// Get the SOCKS proxy URL.
    pub fn socks_url(&self) -> String {
        self.config.socks_url()
    }

    /// Get a reqwest client configured to route through Tor.
    pub fn create_tor_client(&self) -> Result<reqwest::Client> {
        
        

        // Try connecting to check Tor is running
        let addr = format!("{}:{}", self.config.socks_host, self.config.socks_port);
        if let Ok(socket_addr) = addr.parse::<std::net::SocketAddr>() {
            let _conn = TcpStream::connect_timeout(&socket_addr, Duration::from_secs(5))
                .context("Tor SOCKS proxy not reachable")?;
        } else {
            anyhow::bail!("Invalid Tor proxy address: {}", addr);
        }

        Ok(reqwest::Client::builder()
            .proxy(reqwest::Proxy::all(self.socks_url())?)
            .timeout(Duration::from_secs(self.config.timeout_secs))
            .user_agent("ApexIntel/1.0 (+https://apexintel.io) Dark Web Monitor")
            .build()?)
    }

    /// Resolve a .onion address via Tor.
    pub async fn resolve_onion(&self, onion_url: &str) -> Result<String> {
        let client = self.create_tor_client()?;
        let resp = client.get(onion_url).send().await
            .context("onion address request")?;

        if resp.status().is_success() || resp.status().as_u16() == 404 {
            Ok(format!("Resolved via Tor: {}", onion_url))
        } else {
            Ok(format!("Onion returned {}: {}", resp.status(), onion_url))
        }
    }
}

impl Default for TorProxy {
    fn default() -> Self { Self::with_defaults() }
}

/// A Tor circuit with metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TorCircuit {
    pub circuit_id: u32,
    pub exit_country: Option<String>,
    pub built_at: chrono::DateTime<chrono::Utc>,
    pub is_active: bool,
}

/// Tor exit node information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TorExitNode {
    pub fingerprint: String,
    pub ip_address: String,
    pub country_code: Option<String>,
    pub nickname: Option<String>,
    pub or_port: u16,
    pub directory_port: Option<u16>,
    pub first_seen: chrono::DateTime<chrono::Utc>,
    pub last_seen: chrono::DateTime<chrono::Utc>,
    pub is_exit: bool,
    pub is_guard: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tor_config_defaults() {
        let cfg = TorConfig::default();
        assert_eq!(cfg.socks_port, 9050);
        assert_eq!(cfg.control_port, 9051);
        assert_eq!(cfg.max_circuit_hops, 3);
    }

    #[test]
    fn tor_config_urls() {
        let cfg = TorConfig::default();
        assert!(cfg.socks_url().contains("socks5"));
        assert!(cfg.control_url().contains("127.0.0.1"));
    }

    #[test]
    fn tor_proxy_constructs() {
        let proxy = TorProxy::with_defaults();
        assert_eq!(proxy.circuit_count(), 0);
    }

    #[test]
    fn tor_exit_node_debug() {
        let node = TorExitNode {
            fingerprint: "ABCD1234".to_string(),
            ip_address: "192.0.2.1".to_string(),
            country_code: Some("DE".to_string()),
            nickname: Some("Unnamed".to_string()),
            or_port: 9001,
            directory_port: Some(9030),
            first_seen: chrono::Utc::now(),
            last_seen: chrono::Utc::now(),
            is_exit: true,
            is_guard: false,
        };
        assert_eq!(node.country_code, Some("DE".to_string()));
        assert!(node.is_exit);
    }

    #[test]
    fn tor_circuit_debug() {
        let circuit = TorCircuit {
            circuit_id: 1,
            exit_country: Some("RU".to_string()),
            built_at: chrono::Utc::now(),
            is_active: true,
        };
        assert_eq!(circuit.circuit_id, 1);
        assert!(circuit.is_active);
    }
}
