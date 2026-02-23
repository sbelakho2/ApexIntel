use sha2::{Digest, Sha256};

/// In-memory change detection using content hashing.
/// For production use with PgPool, see store crate.
pub struct ChangeDetector {
    fingerprints: std::collections::HashMap<String, String>,
}

impl ChangeDetector {
    pub fn new() -> Self {
        Self {
            fingerprints: std::collections::HashMap::new(),
        }
    }

    /// Compute SHA-256 of content.
    pub fn content_hash(content: &[u8]) -> String {
        hex::encode(Sha256::digest(content))
    }

    /// Check if content has changed for a URL. Updates the stored hash.
    /// Returns `true` if new or changed, `false` if unchanged.
    pub fn has_changed(&mut self, url: &str, content: &[u8]) -> bool {
        let new_hash = Self::content_hash(content);
        let changed = match self.fingerprints.get(url) {
            None => true,
            Some(old_hash) => *old_hash != new_hash,
        };
        if changed {
            self.fingerprints.insert(url.to_string(), new_hash);
        }
        changed
    }

    /// Get the stored hash for a URL, if any.
    pub fn get_hash(&self, url: &str) -> Option<&str> {
        self.fingerprints.get(url).map(|s| s.as_str())
    }

    /// Number of tracked URLs.
    pub fn tracked_count(&self) -> usize {
        self.fingerprints.len()
    }

    /// Clear all tracked fingerprints.
    pub fn clear(&mut self) {
        self.fingerprints.clear();
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

impl Default for ChangeDetector {
    fn default() -> Self {
        Self::new()
    }
}

fn text_bigrams(text: &str) -> std::collections::HashSet<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut bigrams = std::collections::HashSet::new();
    for pair in words.windows(2) {
        bigrams.insert(format!("{} {}", pair[0].to_lowercase(), pair[1].to_lowercase()));
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
        assert!((ChangeDetector::text_similarity("hello world foo", "hello world foo") - 1.0).abs() < f64::EPSILON);
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
