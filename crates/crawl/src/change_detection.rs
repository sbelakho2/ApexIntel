use apex_core::validation::normalize_url;
use regex::Regex;
use sha2::{Digest, Sha256};
use std::sync::LazyLock;
use tracing::instrument;

/// In-memory change detection using content hashing.
/// For production use with PgPool, see store crate.
///
/// # Thread-safety (B281)
///
/// `ChangeDetector` is **not** `Sync` — all mutation methods (`has_changed`,
/// `evict_stale`) take `&mut self`.  For use across async tasks, wrap in
/// `Arc<tokio::sync::Mutex<ChangeDetector>>` or `Arc<std::sync::Mutex<ChangeDetector>>`.
///
/// The static `RE_VOLATILE` regex is initialised once via `LazyLock` and is
/// safe to read from any thread.
pub struct ChangeDetector {
    fingerprints: std::collections::HashMap<String, String>,
    /// Insertion-order tracking for LRU eviction.
    insertion_order: std::collections::VecDeque<String>,
    max_capacity: usize,
}

static RE_VOLATILE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(last\s+updated|updated\s+at|timestamp|©\s*\d{4}|\b\d{4}-\d{2}-\d{2}\b|\b\d{1,2}:\d{2}:\d{2}\b)").unwrap()
});

impl ChangeDetector {
    pub fn new() -> Self {
        Self::with_capacity(100_000)
    }

    pub fn with_capacity(max_capacity: usize) -> Self {
        Self {
            fingerprints: std::collections::HashMap::new(),
            insertion_order: std::collections::VecDeque::new(),
            max_capacity,
        }
    }

    /// Compute SHA-256 of content.
    pub fn content_hash(content: &[u8]) -> String {
        hex::encode(Sha256::digest(content))
    }

    /// Compute a hash with source prefix to avoid cross-source collisions.
    pub fn content_hash_with_source(url: &str, content: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(url.as_bytes());
        hasher.update(b"\n");
        hasher.update(content);
        hex::encode(hasher.finalize())
    }

    /// Check if content has changed for a URL. Updates the stored hash.
    /// Returns `true` if new or changed, `false` if unchanged.
    #[instrument(skip(self, content), fields(url))]
    pub fn has_changed(&mut self, url: &str, content: &[u8]) -> bool {
        let normalized_url = normalize_url(url).unwrap_or_else(|| url.to_string());
        let normalized = normalize_content(content);
        let new_hash = Self::content_hash_with_source(&normalized_url, normalized.as_bytes());
        let changed = match self.fingerprints.get(&normalized_url) {
            None => true,
            Some(old_hash) => *old_hash != new_hash,
        };
        if changed {
            // Evict oldest entries if at capacity
            while self.fingerprints.len() >= self.max_capacity {
                if let Some(oldest) = self.insertion_order.pop_front() {
                    self.fingerprints.remove(&oldest);
                } else {
                    break;
                }
            }
            // LRU refresh: move updated URLs to the back of the queue
            if self.fingerprints.contains_key(&normalized_url) {
                self.insertion_order.retain(|u| u != &normalized_url);
            }
            self.insertion_order.push_back(normalized_url.clone());
            self.fingerprints.insert(normalized_url, new_hash);
        }
        changed
    }

    /// Get the stored hash for a URL, if any.
    pub fn get_hash(&self, url: &str) -> Option<&str> {
        let normalized_url = normalize_url(url).unwrap_or_else(|| url.to_string());
        self.fingerprints.get(&normalized_url).map(|s| s.as_str())
    }

    /// Number of tracked URLs.
    pub fn tracked_count(&self) -> usize {
        self.fingerprints.len()
    }

    /// Clear all tracked fingerprints.
    pub fn clear(&mut self) {
        self.fingerprints.clear();
        self.insertion_order.clear();
    }

    /// Compute a simhash-based near-duplicate score between two texts.
    /// Returns a similarity value between 0.0 (completely different) and 1.0 (identical).
    pub fn text_similarity(a: &str, b: &str) -> f64 {
        if a.is_empty() || b.is_empty() {
            return 0.0;
        }
        if a == b {
            return 1.0;
        }

        // Use a simple Jaccard similarity over word bigrams
        let bigrams_a = text_bigrams(a);
        let bigrams_b = text_bigrams(b);

        if bigrams_a.is_empty() && bigrams_b.is_empty() {
            return 0.0;
        }

        let intersection = bigrams_a.intersection(&bigrams_b).count();
        let union = bigrams_a.union(&bigrams_b).count();

        if union == 0 {
            return 0.0;
        }

        intersection as f64 / union as f64
    }
}

fn normalize_content(content: &[u8]) -> String {
    let text = String::from_utf8_lossy(content);
    let mut lines: Vec<&str> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if RE_VOLATILE.is_match(trimmed) {
            continue;
        }
        lines.push(trimmed);
    }
    lines.join("\n")
}

impl Default for ChangeDetector {
    fn default() -> Self {
        Self::new()
    }
}

fn text_bigrams(text: &str) -> std::collections::HashSet<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut bigrams = std::collections::HashSet::new();
    for pair in words.windows(2) {
        bigrams.insert(format!(
            "{} {}",
            pair[0].to_lowercase(),
            pair[1].to_lowercase()
        ));
    }
    bigrams
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_content_hash() {
        let hash = ChangeDetector::content_hash(b"hello world");
        assert_eq!(hash.len(), 64); // SHA-256 is 32 bytes = 64 hex chars
                                    // Same input = same hash
        assert_eq!(hash, ChangeDetector::content_hash(b"hello world"));
        // Different input = different hash
        assert_ne!(hash, ChangeDetector::content_hash(b"hello world!"));
    }

    #[test]
    fn test_content_hash_with_source_differs() {
        let a = ChangeDetector::content_hash_with_source("https://a.com", b"x");
        let b = ChangeDetector::content_hash_with_source("https://b.com", b"x");
        assert_ne!(a, b);
    }

    #[test]
    fn test_new_url_is_changed() {
        let mut detector = ChangeDetector::new();
        assert!(detector.has_changed("https://example.com", b"content"));
    }

    #[test]
    fn test_same_content_not_changed() {
        let mut detector = ChangeDetector::new();
        assert!(detector.has_changed("https://example.com", b"content"));
        assert!(!detector.has_changed("https://example.com", b"content"));
    }

    #[test]
    fn test_different_content_is_changed() {
        let mut detector = ChangeDetector::new();
        assert!(detector.has_changed("https://example.com", b"content v1"));
        assert!(detector.has_changed("https://example.com", b"content v2"));
    }

    #[test]
    fn test_tracked_count() {
        let mut detector = ChangeDetector::new();
        assert_eq!(detector.tracked_count(), 0);
        detector.has_changed("https://a.com", b"a");
        detector.has_changed("https://b.com", b"b");
        assert_eq!(detector.tracked_count(), 2);
    }

    #[test]
    fn test_get_hash() {
        let mut detector = ChangeDetector::new();
        assert!(detector.get_hash("https://example.com").is_none());
        detector.has_changed("https://example.com", b"content");
        assert!(detector.get_hash("https://example.com").is_some());
    }

    #[test]
    fn test_clear() {
        let mut detector = ChangeDetector::new();
        detector.has_changed("https://a.com", b"a");
        detector.clear();
        assert_eq!(detector.tracked_count(), 0);
    }

    #[test]
    fn test_text_similarity_identical() {
        assert!(
            (ChangeDetector::text_similarity("hello world foo", "hello world foo") - 1.0).abs()
                < f64::EPSILON
        );
    }

    #[test]
    fn test_text_similarity_completely_different() {
        let sim = ChangeDetector::text_similarity("the quick brown fox", "alpha beta gamma delta");
        assert!(sim < 0.1);
    }

    #[test]
    fn test_text_similarity_partial() {
        let sim = ChangeDetector::text_similarity(
            "the quick brown fox jumps over the lazy dog",
            "the quick brown dog jumps over the lazy cat",
        );
        // Most bigrams overlap
        assert!(sim > 0.4);
    }

    #[test]
    fn test_text_similarity_empty() {
        assert!((ChangeDetector::text_similarity("", "")).abs() < 0.01);
        assert!((ChangeDetector::text_similarity("hello world", "") - 0.0).abs() < f64::EPSILON);
    }
}
