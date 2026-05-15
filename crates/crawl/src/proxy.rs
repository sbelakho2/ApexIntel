//! Intelligent proxy rotation with weighted health scoring, proxy-type
//! differentiation, country assignment, session affinity, and multi-endpoint
//! paid proxy pools.
//!
//! # Selection algorithm
//! Free-pool proxies are chosen via **weighted random sampling**:
//! - A proxy's weight = success_score × recency_bonus × kind_multiplier
//! - success_score = (successes + 1) / (successes + failures + 2)  [Laplace smoothing]
//! - recency_bonus = exponential decay based on time since last success
//!
//! Paid proxy endpoints are rotated in round-robin with per-endpoint health
//! tracking so a flapping paid proxy doesn't monopolise the pool.

use std::collections::HashMap;
use std::time::{Duration, Instant};
use url::Url;

// ─────────────────────────────────────────────────────────────────────────────
// Proxy kind
// ─────────────────────────────────────────────────────────────────────────────

/// Classification of proxy networking type.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ProxyKind {
    /// Datacenter IP — cheap, fast, easily blocked.
    Datacenter,
    /// Residential ISP IP — expensive, slow, very hard to block.
    Residential,
    /// Mobile carrier IP — most expensive; looks like a smartphone.
    Mobile,
    /// Unknown / unclassified.
    Unknown,
}

impl ProxyKind {
    /// Base weight multiplier. Residential/mobile proxies have a higher base
    /// weight because they are harder for targets to detect.
    pub fn base_weight(&self) -> f64 {
        match self {
            ProxyKind::Mobile => 2.2,
            ProxyKind::Residential => 1.8,
            ProxyKind::Datacenter => 1.0,
            ProxyKind::Unknown => 1.0,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Health tracking
// ─────────────────────────────────────────────────────────────────────────────

/// Health tracking for a proxy endpoint.
#[derive(Debug, Clone)]
pub struct ProxyHealth {
    /// Consecutive failures since last success.
    pub failures: u32,
    /// Total lifetime failures.
    pub total_failures: u64,
    /// Back-off expiry: proxy is skipped until this instant.
    pub backoff_until: Option<Instant>,
    /// Timestamp of most recent success.
    pub last_success: Option<Instant>,
    /// Total lifetime successes.
    pub success_count: u64,
    /// Proxy kind (inferred or explicitly tagged).
    pub kind: ProxyKind,
    /// ISO 3166-1 alpha-2 country code, e.g. `"US"`, `"IL"`.
    pub country: Option<String>,
}

impl Default for ProxyHealth {
    fn default() -> Self {
        Self {
            failures: 0,
            total_failures: 0,
            backoff_until: None,
            last_success: None,
            success_count: 0,
            kind: ProxyKind::Unknown,
            country: None,
        }
    }
}

impl ProxyHealth {
    /// Composite health score in `[0, 1]`.
    ///
    /// Uses Laplace-smoothed success ratio decayed by time since last use.
    /// An unused proxy scores ~0.5 (neutral prior). A proxy in active backoff
    /// scores 0.0.
    pub fn score(&self) -> f64 {
        if self
            .backoff_until
            .map(|u| Instant::now() < u)
            .unwrap_or(false)
        {
            return 0.0;
        }
        // Laplace-smoothed success rate
        let rate = (self.success_count as f64 + 1.0)
            / (self.success_count as f64 + self.total_failures as f64 + 2.0);
        // Recency bonus — max 0.5, decays with 6-hour half-life
        let recency = match self.last_success {
            Some(t) => 0.5 * (-t.elapsed().as_secs_f64() / 21600.0).exp(),
            None => 0.0,
        };
        (rate + recency).min(1.0)
    }

    /// Final selection weight = health score × kind multiplier.
    pub fn weight(&self) -> f64 {
        self.score() * self.kind.base_weight()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Paid proxy endpoint
// ─────────────────────────────────────────────────────────────────────────────

/// One paid/premium proxy service URL with its own failure tracking.
#[derive(Debug, Clone)]
struct PaidEndpoint {
    url: String,
    failures: u32,
    success_count: u64,
    backoff_until: Option<Instant>,
}

impl PaidEndpoint {
    fn new(url: String) -> Self {
        Self {
            url,
            failures: 0,
            success_count: 0,
            backoff_until: None,
        }
    }

    fn is_available(&self) -> bool {
        self.failures < 8
            && !self
                .backoff_until
                .map(|u| Instant::now() < u)
                .unwrap_or(false)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Rotator
// ─────────────────────────────────────────────────────────────────────────────

/// Intelligent proxy pool with weighted selection and session affinity.
///
/// # Selection priority
/// 1. Healthy paid endpoint — rotated round-robin; skips endpoints in backoff.
/// 2. Weighted random pick from free pool — weight = health_score × kind_multiplier.
/// 3. Pure round-robin fallback when all free weights are zero (all in backoff).
pub struct ProxyRotator {
    proxies: Vec<String>,
    health: HashMap<String, ProxyHealth>,
    current_idx: usize,
    enabled: bool,
    paid_endpoints: Vec<PaidEndpoint>,
    paid_cursor: usize,
    /// session_id → pinned proxy URL.
    sessions: HashMap<String, String>,
}

impl ProxyRotator {
    /// Create a new rotator.
    ///
    /// * `paid_proxy_url` — legacy single paid endpoint (retained for backward-
    ///   compat); use `add_paid_endpoints` for multiple endpoints.
    pub fn new(enabled: bool, paid_proxy_url: Option<String>) -> Self {
        let mut paid_endpoints = Vec::new();
        if let Some(url) = paid_proxy_url {
            paid_endpoints.push(PaidEndpoint::new(url));
        }
        Self {
            proxies: Vec::new(),
            health: HashMap::new(),
            current_idx: 0,
            enabled,
            paid_endpoints,
            paid_cursor: 0,
            sessions: HashMap::new(),
        }
    }

    pub fn add_proxies(&mut self, proxies: Vec<String>) {
        for proxy in proxies {
            if !self.proxies.contains(&proxy) {
                self.proxies.push(proxy);
            }
        }
    }

    /// Register additional paid endpoint URLs.
    pub fn add_paid_endpoints(&mut self, urls: Vec<String>) {
        for url in urls {
            if !self.paid_endpoints.iter().any(|e| e.url == url) {
                self.paid_endpoints.push(PaidEndpoint::new(url));
            }
        }
    }

    /// Tag a free-pool proxy with its kind and optional country.
    pub fn tag_proxy(&mut self, proxy: &str, kind: ProxyKind, country: Option<String>) {
        let h = self.health.entry(proxy.to_string()).or_default();
        h.kind = kind;
        h.country = country;
    }

    pub fn proxy_count(&self) -> usize {
        self.proxies.len()
    }
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    // ── Session affinity ──────────────────────────────────────────

    /// Acquire a sticky proxy for a named scraping session.
    ///
    /// Returns the same proxy on repeated calls with the same `session_id`
    /// until `release_session` is called or the pinned proxy goes unhealthy.
    pub fn get_for_session(&mut self, session_id: &str) -> Option<String> {
        if !self.enabled {
            return None;
        }
        if let Some(pinned) = self.sessions.get(session_id) {
            let ok = self
                .health
                .get(pinned.as_str())
                .map(|h| h.score() > 0.0)
                .unwrap_or(true);
            if ok {
                return Some(pinned.clone());
            }
            self.sessions.remove(session_id);
        }
        let proxy = self.get_next()?;
        self.sessions.insert(session_id.to_string(), proxy.clone());
        Some(proxy)
    }

    /// Release the proxy pinned to a session.
    pub fn release_session(&mut self, session_id: &str) {
        self.sessions.remove(session_id);
    }

    // ── Core selection ────────────────────────────────────────────

    /// Return the best available proxy.
    pub fn get_next(&mut self) -> Option<String> {
        if !self.enabled {
            return None;
        }

        // 1. Try paid endpoints (round-robin, skip degraded)
        if !self.paid_endpoints.is_empty() {
            let n = self.paid_endpoints.len();
            for _ in 0..n {
                let idx = self.paid_cursor % n;
                self.paid_cursor = (self.paid_cursor + 1) % n;
                if self.paid_endpoints[idx].is_available() {
                    return Some(self.paid_endpoints[idx].url.clone());
                }
            }
        }

        if self.proxies.is_empty() {
            return None;
        }

        // 2. Weighted random selection from free pool
        let weights: Vec<f64> = self
            .proxies
            .iter()
            .map(|p| {
                self.health
                    .get(p.as_str())
                    .map(|h| h.weight())
                    .unwrap_or(1.0)
            })
            .collect();
        let total: f64 = weights.iter().sum();

        if total > 0.0 {
            use rand::Rng;
            let mut pick = rand::thread_rng().gen::<f64>() * total;
            for (proxy, w) in self.proxies.iter().zip(&weights) {
                pick -= w;
                if pick <= 0.0 {
                    return Some(proxy.clone());
                }
            }
        }

        // 3. Round-robin fallback when all weights zero
        self.current_idx %= self.proxies.len();
        let idx = self.current_idx;
        self.current_idx = (idx + 1) % self.proxies.len();
        Some(self.proxies[idx].clone())
    }

    /// Return the best healthy proxy geographically matching `country_code`.
    ///
    /// Falls back to `get_next()` if no matching proxy is available.
    pub fn get_for_country(&mut self, country_code: &str) -> Option<String> {
        if !self.enabled {
            return None;
        }
        let cc = country_code.to_uppercase();
        let best = self
            .proxies
            .iter()
            .filter(|p| {
                self.health
                    .get(p.as_str())
                    .and_then(|h| h.country.as_ref())
                    .map(|c| c.eq_ignore_ascii_case(&cc))
                    .unwrap_or(false)
            })
            .max_by(|a, b| {
                let wa = self
                    .health
                    .get(a.as_str())
                    .map(|h| h.weight())
                    .unwrap_or(0.0);
                let wb = self
                    .health
                    .get(b.as_str())
                    .map(|h| h.weight())
                    .unwrap_or(0.0);
                wa.partial_cmp(&wb).unwrap_or(std::cmp::Ordering::Equal)
            })
            .cloned();
        best.or_else(|| self.get_next())
    }

    /// Return the best healthy proxy of the given `kind`.
    pub fn get_of_kind(&mut self, kind: ProxyKind) -> Option<String> {
        if !self.enabled {
            return None;
        }
        let best = self
            .proxies
            .iter()
            .filter(|p| {
                self.health
                    .get(p.as_str())
                    .map(|h| h.kind == kind && h.score() > 0.0)
                    .unwrap_or(false)
            })
            .max_by(|a, b| {
                let wa = self
                    .health
                    .get(a.as_str())
                    .map(|h| h.weight())
                    .unwrap_or(0.0);
                let wb = self
                    .health
                    .get(b.as_str())
                    .map(|h| h.weight())
                    .unwrap_or(0.0);
                wa.partial_cmp(&wb).unwrap_or(std::cmp::Ordering::Equal)
            })
            .cloned();
        best.or_else(|| self.get_next())
    }

    // ── Feedback ──────────────────────────────────────────────────

    pub fn report_success(&mut self, proxy: &str) {
        // Paid endpoint
        if let Some(ep) = self.paid_endpoints.iter_mut().find(|e| e.url == proxy) {
            ep.failures = 0;
            ep.success_count += 1;
            ep.backoff_until = None;
            return;
        }
        let existing = self.health.get(proxy);
        let prev_count = existing.map(|h| h.success_count).unwrap_or(0);
        let prev_total_fail = existing.map(|h| h.total_failures).unwrap_or(0);
        let prev_kind = existing
            .map(|h| h.kind.clone())
            .unwrap_or(ProxyKind::Unknown);
        let prev_country = existing.and_then(|h| h.country.clone());
        self.health.insert(
            proxy.to_string(),
            ProxyHealth {
                failures: 0,
                total_failures: prev_total_fail,
                backoff_until: None,
                last_success: Some(Instant::now()),
                success_count: prev_count + 1,
                kind: prev_kind,
                country: prev_country,
            },
        );
    }

    pub fn report_failure(&mut self, proxy: &str) {
        // Paid endpoint
        if let Some(ep) = self.paid_endpoints.iter_mut().find(|e| e.url == proxy) {
            ep.failures += 1;
            let backoff = Duration::from_secs(60 * 2u64.pow(ep.failures.min(6) - 1));
            ep.backoff_until = Some(Instant::now() + backoff);
            return;
        }
        let existing = self.health.get(proxy);
        let failures = existing.map(|h| h.failures + 1).unwrap_or(1);
        let total_failures = existing.map(|h| h.total_failures + 1).unwrap_or(1);
        let prev_success = existing.and_then(|h| h.last_success);
        let prev_count = existing.map(|h| h.success_count).unwrap_or(0);
        let prev_kind = existing
            .map(|h| h.kind.clone())
            .unwrap_or(ProxyKind::Unknown);
        let prev_country = existing.and_then(|h| h.country.clone());

        let backoff_secs = (30 * 2u64.pow(failures.min(5) - 1)).min(1800);
        self.health.insert(
            proxy.to_string(),
            ProxyHealth {
                failures,
                total_failures,
                backoff_until: Some(Instant::now() + Duration::from_secs(backoff_secs)),
                last_success: prev_success,
                success_count: prev_count,
                kind: prev_kind,
                country: prev_country,
            },
        );

        if failures >= 5 {
            self.proxies.retain(|p| p != proxy);
            self.health.remove(proxy);
        }
    }

    // ── Stats ─────────────────────────────────────────────────────

    pub fn healthy_count(&self) -> usize {
        self.proxies
            .iter()
            .filter(|p| {
                self.health
                    .get(p.as_str())
                    .map(|h| h.failures < 3 && h.score() > 0.0)
                    .unwrap_or(true)
            })
            .count()
    }

    /// Number of healthy paid endpoints.
    pub fn paid_healthy_count(&self) -> usize {
        self.paid_endpoints
            .iter()
            .filter(|e| e.is_available())
            .count()
    }

    /// One-line health summary for logging.
    pub fn health_summary(&self) -> String {
        format!(
            "free={}/{} paid={}/{} sessions={}",
            self.healthy_count(),
            self.proxies.len(),
            self.paid_healthy_count(),
            self.paid_endpoints.len(),
            self.sessions.len(),
        )
    }

    // ── Parsing ───────────────────────────────────────────────────

    /// Parse proxies from newline-delimited text (`ip:port` or full URL).
    pub fn parse_proxy_list(text: &str) -> Vec<String> {
        text.lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty() && l.contains(':'))
            .map(|l| {
                let url_part = l.split('|').next().unwrap_or(l).trim();
                if url_part.starts_with("http") {
                    url_part.to_string()
                } else {
                    format!("http://{}", url_part)
                }
            })
            .filter(|proxy| Url::parse(proxy).is_ok())
            .collect()
    }

    /// Parse an annotated proxy list with kind & country tags.
    ///
    /// Format: `<url>|<kind>|<country_code>` per line.
    /// Example: `http://1.2.3.4:8080|residential|US`
    pub fn parse_annotated_proxy_list(text: &str) -> Vec<(String, ProxyKind, Option<String>)> {
        let mut result = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || !line.contains(':') {
                continue;
            }
            let parts: Vec<&str> = line.splitn(3, '|').collect();
            let raw = parts[0].trim();
            let url = if raw.starts_with("http") {
                raw.to_string()
            } else {
                format!("http://{}", raw)
            };
            if Url::parse(&url).is_err() {
                continue;
            }
            let kind = parts
                .get(1)
                .map(|s| match s.trim().to_lowercase().as_str() {
                    "residential" | "res" => ProxyKind::Residential,
                    "mobile" | "mob" => ProxyKind::Mobile,
                    "datacenter" | "dc" => ProxyKind::Datacenter,
                    _ => ProxyKind::Unknown,
                })
                .unwrap_or(ProxyKind::Unknown);
            let country = parts
                .get(2)
                .map(|s| s.trim().to_uppercase())
                .filter(|s| s.len() == 2);
            result.push((url, kind, country));
        }
        result
    }

    /// Load and tag proxies from an annotated list.
    pub fn load_annotated(&mut self, text: &str) {
        let entries = Self::parse_annotated_proxy_list(text);
        let urls: Vec<String> = entries.iter().map(|(u, _, _)| u.clone()).collect();
        self.add_proxies(urls);
        for (url, kind, country) in entries {
            self.tag_proxy(&url, kind, country);
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

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
        assert_eq!(
            rotator.get_next(),
            Some("http://paid.proxy:8080".to_string())
        );
    }

    #[test]
    fn test_rotation() {
        let mut rotator = ProxyRotator::new(true, None);
        rotator.add_proxies(vec![
            "http://1.1.1.1:8080".to_string(),
            "http://2.2.2.2:8080".to_string(),
            "http://3.3.3.3:8080".to_string(),
        ]);
        let mut seen = std::collections::HashSet::new();
        for _ in 0..30 {
            if let Some(p) = rotator.get_next() {
                seen.insert(p);
            }
        }
        assert!(seen.len() >= 2);
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

    #[test]
    fn test_health_score_unused() {
        let h = ProxyHealth::default();
        // Laplace prior for unseen proxy: (0+1)/(0+0+2) = 0.5
        assert!((h.score() - 0.5).abs() < 0.01);
    }

    #[test]
    fn test_health_score_high_after_successes() {
        let h = ProxyHealth {
            success_count: 20,
            last_success: Some(Instant::now()),
            ..Default::default()
        };
        assert!(h.score() > 0.7);
    }

    #[test]
    fn test_residential_weighted_higher_than_dc() {
        let dc = ProxyHealth {
            kind: ProxyKind::Datacenter,
            success_count: 5,
            last_success: Some(Instant::now()),
            ..Default::default()
        };
        let res = ProxyHealth {
            kind: ProxyKind::Residential,
            success_count: 5,
            last_success: Some(Instant::now()),
            ..Default::default()
        };
        assert!(res.weight() > dc.weight());
    }

    #[test]
    fn test_session_affinity_sticky() {
        let mut rotator = ProxyRotator::new(true, None);
        rotator.add_proxies(vec![
            "http://a:8080".to_string(),
            "http://b:8080".to_string(),
        ]);
        let p1 = rotator
            .get_for_session("s1")
            .unwrap_or_else(|| panic!("session should receive a proxy"));
        let p2 = rotator
            .get_for_session("s1")
            .unwrap_or_else(|| panic!("session should keep its proxy"));
        assert_eq!(p1, p2);
        rotator.release_session("s1");
    }

    #[test]
    fn test_multiple_paid_endpoints_rotate() {
        let mut rotator = ProxyRotator::new(true, None);
        rotator.add_paid_endpoints(vec![
            "http://paid1:8080".to_string(),
            "http://paid2:8080".to_string(),
        ]);
        let p1 = rotator
            .get_next()
            .unwrap_or_else(|| panic!("first paid endpoint should be available"));
        let p2 = rotator
            .get_next()
            .unwrap_or_else(|| panic!("second paid endpoint should be available"));
        assert_ne!(p1, p2);
    }

    #[test]
    fn test_parse_annotated() {
        let text = "1.2.3.4:8080|residential|US\n5.6.7.8:3128|datacenter|IL";
        let entries = ProxyRotator::parse_annotated_proxy_list(text);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].1, ProxyKind::Residential);
        assert_eq!(entries[0].2, Some("US".to_string()));
        assert_eq!(entries[1].1, ProxyKind::Datacenter);
    }

    #[test]
    fn test_health_summary() {
        let r = ProxyRotator::new(false, None);
        assert!(r.health_summary().contains("free="));
    }
}
