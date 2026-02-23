use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Cached robots.txt rules for a domain.
#[derive(Debug, Clone)]
pub struct RobotsRules {
    pub disallowed: Vec<String>,
    pub allowed: Vec<String>,
    pub crawl_delay: Option<f64>,
    pub sitemaps: Vec<String>,
    pub fetched_at: Instant,
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
                if value == "*" && !found_specific {
                    in_matching_section = true;
                } else if ua_lower.contains(&value) || value.contains(&ua_lower) {
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
                    crawl_delay = Some(d);
                }
            }
        }

        Self {
            disallowed,
            allowed,
            crawl_delay,
            sitemaps,
            fetched_at: Instant::now(),
        }
    }

    /// Check if a path is allowed according to these rules.
    pub fn is_allowed(&self, path: &str) -> bool {
        // Check allow rules first (more specific)
        for pattern in &self.allowed {
            if path_matches(path, pattern) {
                return true;
            }
        }
        // Check disallow rules
        for pattern in &self.disallowed {
            if path_matches(path, pattern) {
                return false;
            }
        }
        true // default: allowed
    }

    /// Get the crawl delay as Duration.
    pub fn crawl_delay_duration(&self) -> Option<Duration> {
        self.crawl_delay.map(|d| Duration::from_secs_f64(d))
    }
}

fn path_matches(path: &str, pattern: &str) -> bool {
    if pattern == "/" {
        return true; // Disallow all
    }
    if pattern.ends_with('*') {
        let prefix = &pattern[..pattern.len() - 1];
        return path.starts_with(prefix);
    }
    if pattern.ends_with('$') {
        let exact = &pattern[..pattern.len() - 1];
        return path == exact;
    }
    path.starts_with(pattern)
}

/// Cache of robots.txt rules per domain.
pub struct RobotsCache {
    cache: HashMap<String, RobotsRules>,
    ttl: Duration,
}

impl RobotsCache {
    pub fn new(ttl: Duration) -> Self {
        Self {
            cache: HashMap::new(),
            ttl,
        }
    }

    pub fn get(&self, domain: &str) -> Option<&RobotsRules> {
        self.cache.get(domain).filter(|r| r.fetched_at.elapsed() < self.ttl)
    }

    pub fn insert(&mut self, domain: &str, rules: RobotsRules) {
        self.cache.insert(domain.to_string(), rules);
    }

    pub fn cached_count(&self) -> usize {
        self.cache.len()
    }

    pub fn evict_expired(&mut self) {
        self.cache.retain(|_, r| r.fetched_at.elapsed() < self.ttl);
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
        let delay = rules.crawl_delay_duration().unwrap();
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
    }

    #[test]
    fn test_disallow_all() {
        let robots = "User-agent: *\nDisallow: /\n";
        let rules = RobotsRules::parse(robots, "MyBot");
        assert!(!rules.is_allowed("/anything"));
        assert!(!rules.is_allowed("/"));
    }
}
