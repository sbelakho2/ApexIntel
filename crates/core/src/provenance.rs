use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Every observation and artifact must carry provenance — the chain of
/// where data came from, when it was fetched, and a content hash to
/// detect changes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provenance {
    pub url: String,
    pub fetch_ts: DateTime<Utc>,
    pub content_hash: String,
    pub extractor_version: String,
    pub proxy_used: Option<String>,
    pub http_status: Option<u16>,
    pub response_time_ms: Option<u64>,
}

impl Provenance {
    pub fn new(
        url: impl Into<String>,
        content: &[u8],
        extractor_version: impl Into<String>,
    ) -> Self {
        Self {
            url: url.into(),
            fetch_ts: Utc::now(),
            content_hash: sha256_hex(content),
            extractor_version: extractor_version.into(),
            proxy_used: None,
            http_status: None,
            response_time_ms: None,
        }
    }

    /// Check if content has changed since this provenance was recorded.
    pub fn content_changed(&self, new_content: &[u8]) -> bool {
        sha256_hex(new_content) != self.content_hash
    }

    /// Convert to JSON value for storage in JSONB columns.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::json!({}))
    }
}

/// Compute SHA-256 hex digest of arbitrary bytes.
pub fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sha256_hex_deterministic() {
        let hash1 = sha256_hex(b"hello world");
        let hash2 = sha256_hex(b"hello world");
        assert_eq!(hash1, hash2);
        assert_eq!(hash1.len(), 64); // SHA-256 = 32 bytes = 64 hex chars
    }

    #[test]
    fn test_sha256_hex_known_value() {
        // echo -n "hello world" | sha256sum
        let hash = sha256_hex(b"hello world");
        assert_eq!(
            hash,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }

    #[test]
    fn test_sha256_different_input() {
        let h1 = sha256_hex(b"abc");
        let h2 = sha256_hex(b"abd");
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_provenance_new() {
        let p = Provenance::new(
            "https://example.com/page",
            b"<html>test</html>",
            "html_parser_v1",
        );
        assert_eq!(p.url, "https://example.com/page");
        assert_eq!(p.extractor_version, "html_parser_v1");
        assert_eq!(p.content_hash.len(), 64);
        assert!(p.proxy_used.is_none());
    }

    #[test]
    fn test_content_changed() {
        let p = Provenance::new("https://example.com", b"original content", "v1");
        assert!(!p.content_changed(b"original content"));
        assert!(p.content_changed(b"modified content"));
    }

    #[test]
    fn test_provenance_to_json() {
        let p = Provenance::new("https://test.com", b"data", "v1");
        let json = p.to_json();
        assert_eq!(json["url"], "https://test.com");
        assert_eq!(json["extractor_version"], "v1");
        assert!(json["fetch_ts"].is_string());
    }

    #[test]
    fn test_provenance_serialize_roundtrip() {
        let p = Provenance::new("https://example.com", b"test", "v2");
        let serialized = serde_json::to_string(&p).unwrap();
        let p2: Provenance = serde_json::from_str(&serialized).unwrap();
        assert_eq!(p.url, p2.url);
        assert_eq!(p.content_hash, p2.content_hash);
        assert_eq!(p.extractor_version, p2.extractor_version);
    }
}
