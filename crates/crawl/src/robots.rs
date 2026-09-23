use std::collections::HashMap;
use std::time::{Duration, Instant};

const MIN_CRAWL_DELAY_SECS: f64 = 0.5;
const MAX_CRAWL_DELAY_SECS: f64 = 60.0;

/// Cached robots.txt rules for a domain.
#[derive(Debug, Clone)]
pub struct RobotsRules {
    pub disallowed: Vec<String>,
    pub allowed: Vec<String>,
    pub crawl_delay: Option<f64>,
    pub sitemaps: Vec<String>,
    pub fetched_at: Instant,
    pub max_age: Option<Duration>,
}

impl RobotsRules {
    /// Parse robots.txt content for a given user-agent.
    pub fn parse(content: &str, user_agent: &str) -> Self {
        let mut disallowed = Vec::new();
        let mut allowed = Vec::new();
        let mut crawl_delay = None;
        let mut sitemaps = Vec::new();

        let ua_lower = user_agent.to_lowercase();
        let mut in_matching_section = false;
        let mut found_specific = false;

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            let lower = line.to_lowercase();

            if lower.starts_with("user-agent:") {
                let value = line[11..].trim().to_lowercase();
                // Guard against malformed robots.txt with empty User-Agent:
                // ("anything".contains("") == true in Rust, which would match all crawlers)
                if value.is_empty() {
                    continue;
                }
                if value == "*" && !found_specific {
                    in_matching_section = true;
                } else if ua_lower.contains(&value) {
                    if !found_specific {
                        // Clear wildcard rules, use specific ones
                        disallowed.clear();
                        allowed.clear();
                        crawl_delay = None;
                        found_specific = true;
                    }
                    in_matching_section = true;
                } else {
                    in_matching_section = false;
                }
                continue;
            }

            if lower.starts_with("sitemap:") {
                let sitemap_url = line[8..].trim();
                if !sitemap_url.is_empty() {
                    sitemaps.push(sitemap_url.to_string());
                }
                continue;
            }

            if !in_matching_section {
                continue;
            }

            if lower.starts_with("disallow:") {
                let path = line[9..].trim();
                if !path.is_empty() {
                    disallowed.push(path.to_string());
                }
            } else if lower.starts_with("allow:") {
                let path = line[6..].trim();
                if !path.is_empty() {
                    allowed.push(path.to_string());
                }
            } else if lower.starts_with("crawl-delay:") {
                let value = line[12..].trim();
                if let Ok(d) = value.parse::<f64>() {
                    // Reject NaN/Inf/negative delays: `Duration::from_secs_f64`
                    // panics on them, so a hostile robots.txt could crash a crawl.
                    if d.is_finite() && d >= 0.0 {
                        crawl_delay = Some(d);
                    }
                }
            }
        }

        Self {
            disallowed,
            allowed,
            crawl_delay,
            sitemaps,
            fetched_at: Instant::now(),
            max_age: None,
        }
    }

    /// Check if a path is allowed according to these rules.
    /// Per RFC 9309, the longest matching pattern wins.
    pub fn is_allowed(&self, path: &str) -> bool {
        let mut best_allow: Option<usize> = None;
        let mut best_disallow: Option<usize> = None;

        for pattern in &self.allowed {
            if path_matches(path, pattern) {
                let len = pattern.len();
                if best_allow.is_none_or(|prev| len > prev) {
                    best_allow = Some(len);
                }
            }
        }
        for pattern in &self.disallowed {
            if path_matches(path, pattern) {
                let len = pattern.len();
                if best_disallow.is_none_or(|prev| len > prev) {
                    best_disallow = Some(len);
                }
            }
        }

        match (best_allow, best_disallow) {
            (Some(a), Some(d)) => a >= d, // equal length: allow wins (per RFC 9309)
            (None, Some(_)) => false,     // only disallow matched
            _ => true,                    // no match or only allow matched → allowed
        }
    }

    /// Get the crawl delay as Duration.
    pub fn crawl_delay_duration(&self) -> Option<Duration> {
        self.crawl_delay.and_then(|d| {
            if !d.is_finite() {
                return None;
            }
            let clamped = d.clamp(MIN_CRAWL_DELAY_SECS, MAX_CRAWL_DELAY_SECS);
            Some(Duration::from_secs_f64(clamped))
        })
    }

    /// Set cache max-age for these rules (from Cache-Control headers).
    pub fn with_max_age(mut self, max_age: Duration) -> Self {
        self.max_age = Some(max_age);
        self
    }
}

fn path_matches(path: &str, pattern: &str) -> bool {
    if pattern == "/" {
        return true; // Disallow all
    }
    // Extract end-of-path anchor ($) first, then handle wildcards.
    let (pat, must_end) = if let Some(stripped) = pattern.strip_suffix('$') {
        (stripped, true)
    } else {
        (pattern, false)
    };
    // Support * at any position per RFC 9309 §2.2.2
    if pat.contains('*') {
        let segments: Vec<&str> = pat.split('*').collect();
        let mut pos = 0;
        for (i, seg) in segments.iter().enumerate() {
            if seg.is_empty() {
                continue;
            }
            if i == 0 {
                // First segment must be a prefix
                if !path[pos..].starts_with(seg) {
                    return false;
                }
                pos += seg.len();
            } else {
                // Subsequent segments must appear in order
                match path[pos..].find(seg) {
                    Some(idx) => pos += idx + seg.len(),
                    None => return false,
                }
            }
        }
        // If anchored, the match must consume the entire path
        if must_end {
            pos == path.len()
        } else {
            true
        }
    } else if must_end {
        path == pat
    } else {
        path.starts_with(pat)
    }
}

/// Cache of robots.txt rules per domain.
pub struct RobotsCache {
    cache: HashMap<String, RobotsRules>,
    ttl: Duration,
    cache_hits: u64,
    cache_misses: u64,
}

impl RobotsCache {
    pub fn new(ttl: Duration) -> Self {
        Self {
            cache: HashMap::new(),
            ttl,
            cache_hits: 0,
            cache_misses: 0,
        }
    }

    pub fn get(&mut self, domain: &str) -> Option<&RobotsRules> {
        let result = self.cache.get(domain).filter(|r| {
            let ttl = r.max_age.map(|v| v.min(self.ttl)).unwrap_or(self.ttl);
            r.fetched_at.elapsed() < ttl
        });
        if result.is_some() {
            self.cache_hits = self.cache_hits.saturating_add(1);
        } else {
            self.cache_misses = self.cache_misses.saturating_add(1);
        }
        result
    }

    pub fn insert(&mut self, domain: &str, rules: RobotsRules) {
        // Auto-evict expired entries on every insert to bound memory growth
        self.evict_expired();
        self.cache.insert(domain.to_string(), rules);
    }

    pub fn cached_count(&self) -> usize {
        self.cache.len()
    }

    pub fn evict_expired(&mut self) {
        self.cache.retain(|_, r| r.fetched_at.elapsed() < self.ttl);
    }

    /// Number of successful cache lookups.
    pub fn cache_hits(&self) -> u64 {
        self.cache_hits
    }

    /// Number of failed cache lookups.
    pub fn cache_misses(&self) -> u64 {
        self.cache_misses
    }

    /// Cache hit-rate in `[0.0, 1.0]` when there have been lookups, else `0.0`.
    pub fn hit_rate(&self) -> f64 {
        let total = self.cache_hits.saturating_add(self.cache_misses);
        if total == 0 {
            0.0
        } else {
            self.cache_hits as f64 / total as f64
        }
    }
}

impl Default for RobotsCache {
    fn default() -> Self {
        Self::new(Duration::from_secs(3600))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_ROBOTS: &str = "\
User-agent: *
Disallow: /admin/
Disallow: /private
Allow: /admin/public
Crawl-delay: 2

User-agent: Googlebot
Disallow: /secret/

Sitemap: https://example.com/sitemap.xml
";

    #[test]
    fn test_parse_wildcard_rules() {
        let rules = RobotsRules::parse(SAMPLE_ROBOTS, "MyBot");
        assert!(rules.disallowed.contains(&"/admin/".to_string()));
        assert!(rules.disallowed.contains(&"/private".to_string()));
        assert!(rules.allowed.contains(&"/admin/public".to_string()));
        assert_eq!(rules.crawl_delay, Some(2.0));
    }

    #[test]
    fn test_parse_specific_agent() {
        let rules = RobotsRules::parse(SAMPLE_ROBOTS, "Googlebot");
        assert!(rules.disallowed.contains(&"/secret/".to_string()));
        // Should not have wildcard rules
        assert!(!rules.disallowed.contains(&"/admin/".to_string()));
    }

    #[test]
    fn test_parse_sitemaps() {
        let rules = RobotsRules::parse(SAMPLE_ROBOTS, "MyBot");
        assert_eq!(rules.sitemaps.len(), 1);
        assert!(rules.sitemaps[0].contains("sitemap.xml"));
    }

    #[test]
    fn test_is_allowed_basic() {
        let rules = RobotsRules::parse(SAMPLE_ROBOTS, "MyBot");
        assert!(rules.is_allowed("/"));
        assert!(rules.is_allowed("/public/page"));
        assert!(!rules.is_allowed("/admin/settings"));
        assert!(!rules.is_allowed("/private"));
        assert!(rules.is_allowed("/admin/public"));
    }

    #[test]
    fn test_path_matches_prefix() {
        assert!(path_matches("/admin/page", "/admin/"));
        assert!(!path_matches("/public/page", "/admin/"));
    }

    #[test]
    fn test_path_matches_wildcard() {
        assert!(path_matches("/search?q=test", "/search*"));
        assert!(!path_matches("/about", "/search*"));
    }

    #[test]
    fn test_path_matches_exact() {
        assert!(path_matches("/exact", "/exact$"));
        assert!(!path_matches("/exact/more", "/exact$"));
    }

    #[test]
    fn test_crawl_delay_duration() {
        let rules = RobotsRules::parse(SAMPLE_ROBOTS, "MyBot");
        let delay = rules
            .crawl_delay_duration()
            .unwrap_or_else(|| panic!("sample robots should include crawl delay"));
        assert_eq!(delay, Duration::from_secs(2));
    }

    #[test]
    fn test_empty_robots() {
        let rules = RobotsRules::parse("", "MyBot");
        assert!(rules.disallowed.is_empty());
        assert!(rules.allowed.is_empty());
        assert!(rules.crawl_delay.is_none());
        assert!(rules.is_allowed("/anything"));
    }

    #[test]
    fn test_robots_cache() {
        let mut cache = RobotsCache::new(Duration::from_secs(3600));
        let rules = RobotsRules::parse(SAMPLE_ROBOTS, "MyBot");
        cache.insert("example.com", rules);
        assert_eq!(cache.cached_count(), 1);
        assert!(cache.get("example.com").is_some());
        assert!(cache.get("other.com").is_none());
        assert_eq!(cache.cache_hits(), 1);
        assert_eq!(cache.cache_misses(), 1);
        assert!((cache.hit_rate() - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_disallow_all() {
        let robots = "User-agent: *\nDisallow: /\n";
        let rules = RobotsRules::parse(robots, "MyBot");
        assert!(!rules.is_allowed("/anything"));
        assert!(!rules.is_allowed("/"));
    }
}
