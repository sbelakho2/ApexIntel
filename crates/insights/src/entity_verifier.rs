//! # Entity Verifier
//!
//! Heuristic-based company verification that validates extracted company
//! candidates before they are registered in the entity registry.
//!
//! In a full deployment, this would connect to:
//! - SEC EDGAR API for US companies
//! - OpenCorporates for global registry
//! - Company websites for domain verification
//! - D&B API for business verification
//!
//! For now, verification uses heuristic checks, ticker cross-referencing,
//! and multi-source confirmation.

use crate::company_discovery::{CompanyCandidate, DiscoverySource};
use chrono::{DateTime, Utc};
use lru::LruCache;
use std::collections::HashMap;

// ─────────────────────────────────────────────────────────────────────────────
// Types
// ─────────────────────────────────────────────────────────────────────────────

/// Result of verifying a company candidate.
#[derive(Debug, Clone)]
pub struct VerificationResult {
    /// The candidate that was verified.
    pub candidate: CompanyCandidate,
    /// Whether verification passed.
    pub is_verified: bool,
    /// Overall confidence (0.0 – 1.0).
    pub confidence: f64,
    /// Methods used for verification.
    pub verification_methods: Vec<String>,
    /// When verification was performed.
    pub verified_at: DateTime<Utc>,
    /// Additional metadata discovered during verification.
    pub metadata: HashMap<String, String>,
}

/// Heuristic-based entity verifier.
///
/// Uses a combination of:
/// - Name heuristic checks (length, structure, stopwords)
/// - Known ticker cross-referencing
/// - Known domain cross-referencing
/// - Multi-source confirmation scoring
pub struct EntityVerifier {
    /// Known ticker → company name mapping.
    known_tickers: HashMap<String, String>,
    /// Known domain → company name mapping.
    known_domains: HashMap<String, String>,
    /// Cache of recent verification results.
    verification_cache: LruCache<String, VerificationResult>,
}

impl EntityVerifier {
    /// Create a new verifier with default known entities.
    pub fn new() -> Self {
        let mut known_tickers = HashMap::new();
        // Seed with major public companies
        let seed_tickers: &[(&str, &str)] = &[
            ("AAPL", "Apple"),
            ("MSFT", "Microsoft"),
            ("GOOGL", "Alphabet"),
            ("GOOG", "Alphabet"),
            ("AMZN", "Amazon"),
            ("NVDA", "NVIDIA"),
            ("META", "Meta"),
            ("TSLA", "Tesla"),
            ("TSM", "TSMC"),
            ("INTC", "Intel"),
            ("AMD", "AMD"),
            ("QCOM", "Qualcomm"),
            ("AVGO", "Broadcom"),
            ("ASML", "ASML"),
            ("TXN", "Texas Instruments"),
            ("MU", "Micron"),
            ("CRM", "Salesforce"),
            ("ORCL", "Oracle"),
            ("IBM", "IBM"),
            ("CSCO", "Cisco"),
            ("NOC", "Northrop Grumman"),
            ("LMT", "Lockheed Martin"),
            ("RTX", "Raytheon Technologies"),
            ("GD", "General Dynamics"),
            ("BA", "Boeing"),
            ("AIR", "Airbus"),
            ("EADSY", "Airbus"),
            ("BAESY", "BAE Systems"),
            ("ESLT", "Elbit Systems"),
            ("RHM.DE", "Rheinmetall"),
            ("SAAB-B.ST", "Saab"),
            ("LDO.MI", "Leonardo"),
            ("HO.PA", "Thales"),
            ("JBL", "Jabil"),
            ("FLEX", "Flex"),
            ("CLS", "Celestica"),
            ("SANM", "Sanmina"),
            ("PLXS", "Plexus"),
            ("KE", "Kimball Electronics"),
            ("BHE", "Benchmark Electronics"),
        ];
        for (ticker, name) in seed_tickers {
            known_tickers.insert(ticker.to_string(), name.to_string());
        }

        let mut known_domains = HashMap::new();
        let seed_domains: &[(&str, &str)] = &[
            ("apple.com", "Apple"),
            ("microsoft.com", "Microsoft"),
            ("nvidia.com", "NVIDIA"),
            ("tsmc.com", "TSMC"),
            ("intel.com", "Intel"),
            ("amd.com", "AMD"),
            ("ibm.com", "IBM"),
            ("oracle.com", "Oracle"),
            ("cisco.com", "Cisco"),
            ("lockheedmartin.com", "Lockheed Martin"),
            ("northropgrumman.com", "Northrop Grumman"),
            ("raytheon.com", "Raytheon Technologies"),
            ("gdeb.com", "General Dynamics"),
            ("boeing.com", "Boeing"),
            ("airbus.com", "Airbus"),
            ("baesystems.com", "BAE Systems"),
            ("elbitsystems.com", "Elbit Systems"),
            ("rheinmetall.com", "Rheinmetall"),
            ("saab.com", "Saab"),
            ("leonardo.com", "Leonardo"),
            ("thalesgroup.com", "Thales"),
            ("jabil.com", "Jabil"),
            ("flex.com", "Flex"),
            ("celestica.com", "Celestica"),
            ("foxconn.com", "Foxconn"),
        ];
        for (domain, name) in seed_domains {
            known_domains.insert(domain.to_string(), name.to_string());
        }

        Self {
            known_tickers,
            known_domains,
            verification_cache: LruCache::new(
                std::num::NonZeroUsize::new(1000)
                    .unwrap_or_else(|| unreachable!("1000 is non-zero")),
            ),
        }
    }

    /// Verify a company candidate using all available methods.
    ///
    /// Returns a `VerificationResult` with a confidence score and the methods
    /// that contributed to verification.
    pub fn verify(&mut self, candidate: &CompanyCandidate) -> VerificationResult {
        let cache_key = candidate.normalized_name.clone();

        // Check cache first
        if let Some(cached) = self.verification_cache.get(&cache_key) {
            return cached.clone();
        }

        let mut methods: Vec<String> = Vec::new();
        let mut confidence = 0.0;

        // 1. Heuristic check (always applicable) — use raw_name to preserve casing
        let heuristic_score = self.heuristic_check(&candidate.raw_name);
        if heuristic_score > 0.0 {
            methods.push(format!("heuristic_check:{:.2}", heuristic_score));
            confidence += heuristic_score * 0.75;
        }

        // 2. Ticker verification (if ticker metadata available)
        if let Some(ticker) = candidate.metadata.get("ticker") {
            if let Some(exchange) = candidate.metadata.get("exchange") {
                if let Some(name) = self.verify_ticker(exchange, ticker) {
                    methods.push(format!("ticker_match:{}:{}", exchange, ticker));
                    confidence += 0.4;
                    // Boost if normalization matches
                    let normalized_known = crate::company_discovery::normalize_company_name(&name);
                    if normalized_known == candidate.normalized_name {
                        confidence += 0.2;
                    }
                }
            } else if self.known_tickers.contains_key(ticker) {
                methods.push(format!("known_ticker:{}", ticker));
                confidence += 0.35;
            }
        }

        // 3. Website/domain verification
        if let Some(website) = candidate.metadata.get("website") {
            if self.check_website(website) {
                methods.push(format!("website_check:{}", website));
                confidence += 0.3;
            }
        }

        // 4. Multi-source confirmation
        let sources = vec![candidate.source.clone()];
        let ms_score = self.multi_source_confirmation(&candidate.normalized_name, &sources);
        if ms_score > 0.0 {
            methods.push(format!("multi_source:{:.2}", ms_score));
            confidence += ms_score * 0.2;
        }

        // Clamp confidence to [0.0, 1.0]
        let confidence = confidence.min(1.0);

        let is_verified = confidence >= 0.5; // Minimum threshold for verification

        let mut metadata = candidate.metadata.clone();
        metadata.insert(
            "verification_confidence".to_string(),
            format!("{:.4}", confidence),
        );

        let result = VerificationResult {
            candidate: candidate.clone(),
            is_verified,
            confidence,
            verification_methods: methods,
            verified_at: Utc::now(),
            metadata,
        };

        // Cache the result
        self.verification_cache
            .put(cache_key, result.clone());

        result
    }

    /// Check if a website/domain looks valid.
    ///
    /// In production, this would perform a DNS lookup or HTTP HEAD request.
    /// For now, we check if the domain matches known patterns.
    fn check_website(&self, url: &str) -> bool {
        // Extract domain from URL
        let domain = url
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_start_matches("www.")
            .split('/')
            .next()
            .unwrap_or("");

        // Check against known domains
        if self.known_domains.contains_key(domain) {
            return true;
        }

        // Basic validity: must have a dot and TLD
        if let Some(dot_pos) = domain.rfind('.') {
            let tld = &domain[dot_pos + 1..];
            let valid_tlds = [
                "com", "org", "net", "io", "ai", "co", "de", "fr", "jp", "cn",
                "sg", "tw", "uk", "eu", "gov", "edu",
            ];
            valid_tlds.contains(&tld) && domain.len() > 4
        } else {
            false
        }
    }

    /// Cross-reference a ticker with known exchange data.
    fn verify_ticker(&self, exchange: &str, ticker: &str) -> Option<String> {
        // Build exchange-specific key
        let key = format!("{}:{}", exchange, ticker);
        if let Some(name) = self.known_tickers.get(ticker) {
            return Some(name.clone());
        }
        if let Some(name) = self.known_tickers.get(&key) {
            return Some(name.clone());
        }
        None
    }

    /// Check if multiple independent sources mention this company.
    ///
    /// In a full implementation, this would query a database. For now,
    /// returns a simple score based on source diversity.
    fn multi_source_confirmation(&self, _name: &str, sources: &[DiscoverySource]) -> f64 {
        if sources.len() >= 3 {
            0.8
        } else if sources.len() == 2 {
            0.5
        } else if sources.len() == 1 {
            0.2
        } else {
            0.0
        }
    }

    /// Quick heuristic: does the name look like a real company?
    ///
    /// Evaluates:
    /// - Length > 3 characters AND < 100 characters
    /// - Contains at least one alphabetic character
    /// - Not a common word (check against stop words list)
    /// - Contains company suffix OR has 2+ capitalized words OR has embedded capitals
    /// - Not in exclusion list (common locations, person names, etc.)
    pub fn heuristic_check(&self, name: &str) -> f64 {
        let name = name.trim();

        // Length check
        if name.len() <= 3 || name.len() >= 100 {
            return 0.0;
        }

        // Must contain at least one alphabetic character
        if !name.chars().any(|c| c.is_alphabetic()) {
            return 0.0;
        }

        let lower = name.to_lowercase();
        let words: Vec<&str> = lower.split_whitespace().collect();

        // Stopword set: common words that aren't company names
        let stopwords: std::collections::HashSet<&str> = [
            "the", "this", "that", "these", "those", "there", "their", "they",
            "have", "has", "had", "been", "being", "some", "any", "each",
            "every", "both", "few", "more", "most", "other", "into", "over",
            "such", "only", "own", "same", "than", "very", "just", "also",
            "about", "above", "below", "between", "through", "during",
            "before", "after", "where", "which", "what", "when", "why", "how",
            "who", "whom", "with", "without",
        ].into();

        // If ALL words are stopwords, this is not a company name
        if !words.is_empty() && words.iter().all(|w| stopwords.contains(w)) {
            return 0.0;
        }

        // Exclusion list: common locations and phrases
        let exclusion_list: std::collections::HashSet<&str> = [
            "new york", "los angeles", "chicago", "houston", "london", "paris",
            "tokyo", "beijing", "shanghai", "hong kong", "singapore", "dubai",
            "san francisco", "washington", "boston", "seattle", "miami", "dallas",
            "berlin", "munich", "milan", "rome", "madrid", "toronto", "sydney",
            "melbourne", "mumbai", "delhi", "bangalore",
        ].into();
        if exclusion_list.contains(lower.as_str()) {
            return 0.0;
        }

        let mut score: f64 = 0.3; // Base score for passing basic checks

        // Check for company suffix
        let suffixes = [
            "inc", "corp", "ltd", "llc", "plc", "gmbh", "sarl", "ag", "kg",
            "limited", "incorporated", "corporation", "company", "co",
        ];
        let words: Vec<&str> = lower.split_whitespace().collect();
        let has_suffix = words
            .iter()
            .any(|w| suffixes.contains(&w.trim_end_matches('.')));
        if has_suffix {
            score += 0.3;
        }

        // Check for 2+ capitalized words (in original casing)
        let titlecase_words: Vec<&str> = name
            .split_whitespace()
            .filter(|w| {
                let chars: Vec<char> = w.chars().collect();
                chars.len() > 1 && chars[0].is_uppercase() && chars[1..].iter().all(|c| c.is_lowercase() || !c.is_alphabetic())
            })
            .collect();
        if titlecase_words.len() >= 2 {
            score += 0.2;
        }

        // Check for embedded capitals (camelCase like "McDonald's", "iPhone")
        if name.chars().filter(|c| c.is_uppercase()).count() >= 2 && name.len() > 5 {
            score += 0.1;
        }

        // Penalize very short names
        if name.len() < 5 {
            score -= 0.2;
        }

        score.clamp(0.0, 1.0)
    }
}

impl Default for EntityVerifier {
    fn default() -> Self {
        Self::new()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::company_discovery::CompanyCandidate;

    fn make_candidate(name: &str) -> CompanyCandidate {
        CompanyCandidate {
            raw_name: name.to_string(),
            normalized_name: crate::company_discovery::normalize_company_name(name),
            source: DiscoverySource::NewsArticle,
            extraction_confidence: 0.8,
            context_snippet: String::new(),
            metadata: HashMap::new(),
        }
    }

    fn make_candidate_with_ticker(name: &str, ticker: &str, exchange: &str) -> CompanyCandidate {
        let mut metadata = HashMap::new();
        metadata.insert("ticker".to_string(), ticker.to_string());
        metadata.insert("exchange".to_string(), exchange.to_string());
        CompanyCandidate {
            raw_name: name.to_string(),
            normalized_name: crate::company_discovery::normalize_company_name(name),
            source: DiscoverySource::NewsArticle,
            extraction_confidence: 0.8,
            context_snippet: String::new(),
            metadata,
        }
    }

    #[test]
    fn test_heuristic_check_passes_good_name() {
        let verifier = EntityVerifier::new();
        let score = verifier.heuristic_check("NVIDIA Corporation");
        assert!(score > 0.5, "NVIDIA Corporation should score > 0.5, got {:.2}", score);
    }

    #[test]
    fn test_heuristic_check_rejects_short_name() {
        let verifier = EntityVerifier::new();
        let score = verifier.heuristic_check("AB");
        assert_eq!(score, 0.0, "Short name should score 0.0");
    }

    #[test]
    fn test_heuristic_check_rejects_stopwords() {
        let verifier = EntityVerifier::new();
        let score = verifier.heuristic_check("the this that");
        assert_eq!(score, 0.0, "Stopwords should score 0.0");
    }

    #[test]
    fn test_heuristic_check_rejects_location() {
        let verifier = EntityVerifier::new();
        let score = verifier.heuristic_check("New York");
        // "New York" is in exclusion list
        assert_eq!(score, 0.0, "Location should score 0.0");
    }

    #[test]
    fn test_verify_known_ticker() {
        let mut verifier = EntityVerifier::new();
        let candidate = make_candidate_with_ticker("NVIDIA Corporation", "NVDA", "NASDAQ");
        let result = verifier.verify(&candidate);
        assert!(result.is_verified, "NVIDIA should be verified via ticker");
        assert!(result.confidence > 0.5);
    }

    #[test]
    fn test_verify_unknown_company() {
        let mut verifier = EntityVerifier::new();
        let candidate = make_candidate("UnknownStartupXYZ Corp");
        let result = verifier.verify(&candidate);
        // Should still pass heuristic check
        assert!(result.is_verified, "Well-formed name should pass heuristic");
    }

    #[test]
    fn test_verify_garbage_name() {
        let mut verifier = EntityVerifier::new();
        let candidate = CompanyCandidate {
            raw_name: "abc".to_string(),
            normalized_name: "abc".to_string(),
            source: DiscoverySource::NewsArticle,
            extraction_confidence: 0.3,
            context_snippet: String::new(),
            metadata: HashMap::new(),
        };
        let result = verifier.verify(&candidate);
        assert!(!result.is_verified, "Garbage name should not be verified");
    }

    #[test]
    fn test_check_website_valid() {
        let verifier = EntityVerifier::new();
        assert!(verifier.check_website("https://www.apple.com"));
        assert!(verifier.check_website("https://nvidia.com"));
        assert!(verifier.check_website("http://example.org"));
        assert!(!verifier.check_website("not-a-url"));
    }

    #[test]
    #[allow(clippy::disallowed_methods)]
    fn test_verify_ticker_known() {
        let verifier = EntityVerifier::new();
        let result = verifier.verify_ticker("NASDAQ", "NVDA");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "NVIDIA");
    }

    #[test]
    fn test_verify_ticker_unknown() {
        let verifier = EntityVerifier::new();
        let result = verifier.verify_ticker("NASDAQ", "ZZZZZ");
        assert!(result.is_none());
    }

    #[test]
    fn test_heuristic_check_suffix_boost() {
        let verifier = EntityVerifier::new();
        // Has "Inc" suffix
        let with_suffix = verifier.heuristic_check("TechCorp Inc");
        // No suffix
        let without_suffix = verifier.heuristic_check("TechCorp");
        assert!(with_suffix > without_suffix, "Suffix should boost score");
    }

    #[test]
    fn test_verification_cache_hits() {
        let mut verifier = EntityVerifier::new();
        let candidate = make_candidate("NVIDIA Corporation");
        let result1 = verifier.verify(&candidate);
        let result2 = verifier.verify(&candidate);
        assert_eq!(result1.confidence, result2.confidence);
        assert_eq!(result1.verification_methods.len(), result2.verification_methods.len());
    }

    #[test]
    fn test_verifier_default() {
        let verifier = EntityVerifier::default();
        assert!(!verifier.known_tickers.is_empty());
        assert!(!verifier.known_domains.is_empty());
    }
}
