use std::collections::HashMap;
use std::time::{Duration, Instant};
use url::Url;

/// Health tracking for a proxy endpoint.
#[derive(Debug, Clone)]
pub struct ProxyHealth {
    pub failures: u32,
    pub backoff_until: Option<Instant>,
    pub last_success: Option<Instant>,
    pub success_count: u64,
}

impl Default for ProxyHealth {
    fn default() -> Self {
        Self {
            failures: 0,
            backoff_until: None,
            last_success: None,
            success_count: 0,
        }
    }
}

/// Manages a rotating pool of proxy servers with health tracking.
pub struct ProxyRotator {
    proxies: Vec<String>,
    health: HashMap<String, ProxyHealth>,
    current_idx: usize,
    enabled: bool,
    paid_proxy_url: Option<String>,
}

impl ProxyRotator {
    pub fn new(enabled: bool, paid_proxy_url: Option<String>) -> Self {
        Self {
            proxies: Vec::new(),
            health: HashMap::new(),
            current_idx: 0,
            enabled,
            paid_proxy_url,
        }
    }

    pub fn add_proxies(&mut self, proxies: Vec<String>) {
        for proxy in proxies {
            if !self.proxies.contains(&proxy) {
                self.proxies.push(proxy);
            }
        }
    }

    pub fn proxy_count(&self) -> usize {
        self.proxies.len()
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn get_next(&mut self) -> Option<String> {
        if !self.enabled {
            return None;
        }
        if let Some(ref url) = self.paid_proxy_url {
            return Some(url.clone());
        }
        if self.proxies.is_empty() {
            return None;
        }

        // Normalize index after possible proxy removal to prevent OOB.
        self.current_idx = self.current_idx % self.proxies.len();
        let start = self.current_idx;
        let max_tries = self.proxies.len().min(10);

        for _ in 0..max_tries {
            let proxy = &self.proxies[self.current_idx];
            self.current_idx = (self.current_idx + 1) % self.proxies.len();

            if let Some(health) = self.health.get(proxy) {
                if let Some(until) = health.backoff_until {
                    if Instant::now() < until {
                        continue;
                    }
                }
            }
            return Some(proxy.clone());
        }

        // Fallback: return current position proxy regardless
        let idx = start % self.proxies.len();
        Some(self.proxies[idx].clone())
    }

    pub fn report_success(&mut self, proxy: &str) {
        let existing = self.health.get(proxy);
        let prev_count = existing.map(|h| h.success_count).unwrap_or(0);
        self.health.insert(
            proxy.to_string(),
            ProxyHealth {
                failures: 0,
                backoff_until: None,
                last_success: Some(Instant::now()),
                success_count: prev_count + 1,
            },
        );
    }

    pub fn report_failure(&mut self, proxy: &str) {
        let existing = self.health.get(proxy);
        let failures = existing.map(|h| h.failures + 1).unwrap_or(1);
        let prev_success = existing.and_then(|h| h.last_success);
        let prev_count = existing.map(|h| h.success_count).unwrap_or(0);

        let backoff = Duration::from_secs(30 * 2u64.pow(failures.min(5) - 1));

        self.health.insert(
            proxy.to_string(),
            ProxyHealth {
                failures,
                backoff_until: Some(Instant::now() + backoff),
                last_success: prev_success,
                success_count: prev_count,
            },
        );

        // Remove proxy after too many failures
        if failures >= 5 {
            self.proxies.retain(|p| p != proxy);
            self.health.remove(proxy);
        }
    }

    pub fn healthy_count(&self) -> usize {
        self.proxies
            .iter()
            .filter(|p| {
                self.health
                    .get(p.as_str())
                    .map(|h| h.failures < 3)
                    .unwrap_or(true)
            })
            .count()
    }

    /// Parse proxies from a newline-delimited text (ip:port format).
    pub fn parse_proxy_list(text: &str) -> Vec<String> {
        text.lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty() && l.contains(':'))
            .map(|l| {
                if l.starts_with("http") {
                    l.to_string()
                } else {
                    format!("http://{}", l)
                }
            })
            .filter(|proxy| Url::parse(proxy).is_ok())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_disabled_returns_none() {
        let mut rotator = ProxyRotator::new(false, None);
        rotator.add_proxies(vec!["http://1.2.3.4:8080".to_string()]);
        assert_eq!(rotator.get_next(), None);
    }

    #[test]
    fn test_paid_proxy_override() {
        let mut rotator = ProxyRotator::new(true, Some("http://paid.proxy:8080".to_string()));
        rotator.add_proxies(vec!["http://free1:8080".to_string()]);
        assert_eq!(rotator.get_next(), Some("http://paid.proxy:8080".to_string()));
    }

    #[test]
    fn test_rotation() {
        let mut rotator = ProxyRotator::new(true, None);
        rotator.add_proxies(vec![
            "http://1.1.1.1:8080".to_string(),
            "http://2.2.2.2:8080".to_string(),
            "http://3.3.3.3:8080".to_string(),
        ]);

        let first = rotator.get_next().unwrap();
        let second = rotator.get_next().unwrap();
        assert_ne!(first, second);
    }

    #[test]
    fn test_report_success_resets() {
        let mut rotator = ProxyRotator::new(true, None);
        let proxy = "http://1.1.1.1:8080";
        rotator.add_proxies(vec![proxy.to_string()]);
        rotator.report_failure(proxy);
        rotator.report_success(proxy);
        assert_eq!(rotator.health[proxy].failures, 0);
        assert_eq!(rotator.health[proxy].success_count, 1);
    }

    #[test]
    fn test_report_failure_removes_after_threshold() {
        let mut rotator = ProxyRotator::new(true, None);
        let proxy = "http://bad.proxy:8080";
        rotator.add_proxies(vec![proxy.to_string()]);
        for _ in 0..5 {
            rotator.report_failure(proxy);
        }
        assert_eq!(rotator.proxy_count(), 0);
    }

    #[test]
    fn test_parse_proxy_list() {
        let text = "1.2.3.4:8080\n5.6.7.8:3128\n\nhttp://9.10.11.12:8080\n";
        let proxies = ProxyRotator::parse_proxy_list(text);
        assert_eq!(proxies.len(), 3);
        assert_eq!(proxies[0], "http://1.2.3.4:8080");
        assert_eq!(proxies[2], "http://9.10.11.12:8080");
    }

    #[test]
    fn test_empty_pool_returns_none() {
        let mut rotator = ProxyRotator::new(true, None);
        assert_eq!(rotator.get_next(), None);
    }

    #[test]
    fn test_healthy_count() {
        let mut rotator = ProxyRotator::new(true, None);
        rotator.add_proxies(vec![
            "http://good:8080".to_string(),
            "http://bad:8080".to_string(),
        ]);
        rotator.report_failure("http://bad:8080");
        rotator.report_failure("http://bad:8080");
        rotator.report_failure("http://bad:8080");
        assert_eq!(rotator.healthy_count(), 1);
    }
}
