//! Structured source-adapter outcomes and parser health metrics.
//!
//! Source adapters historically returned `Vec<T>` (or `Result<Vec<T>>`) and
//! converted deserialization/schema failures into an empty collection or a
//! defaulted value. Downstream that was indistinguishable from a genuine
//! "no items today", so a silently broken parser looked like a healthy empty
//! source. [`ParseOutcome`] encodes the three distinct states:
//!
//! * [`ParseOutcome::FetchFailed`] — the network/HTTP fetch failed.
//! * [`ParseOutcome::ParseFailed`] — a body arrived but deserialization or
//!   schema validation failed. This is a parser incident, not emptiness.
//! * [`ParseOutcome::ParsedSuccessfully`] — deserialization succeeded;
//!   `items` may legitimately be empty.
//!
//! [`PARSER_METRICS`] aggregates per-outcome counters so coverage/admin
//! surfaces can report a parser success rate, and [`redact_sample`] keeps a
//! bounded, secret-masked sample for incident debugging.

use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

/// Outcome of one source-adapter fetch + parse attempt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "outcome")]
pub enum ParseOutcome<T> {
    /// The fetch failed before any body could be parsed.
    FetchFailed {
        error: String,
        http_status: Option<u16>,
    },
    /// A body was fetched but deserialization or schema validation failed.
    /// Contains a bounded, redacted sample for debugging.
    ParseFailed {
        error: String,
        redacted_sample: String,
    },
    /// Deserialization succeeded. `items` may be empty — only this variant
    /// proves the parser contract held.
    ParsedSuccessfully { items: Vec<T> },
}

impl<T> ParseOutcome<T> {
    pub fn fetch_failed(error: impl Into<String>, http_status: Option<u16>) -> Self {
        Self::FetchFailed {
            error: error.into(),
            http_status,
        }
    }

    pub fn parse_failed(error: impl Into<String>, sample: &str) -> Self {
        Self::ParseFailed {
            error: error.into(),
            redacted_sample: redact_sample(sample),
        }
    }

    pub fn parsed(items: Vec<T>) -> Self {
        Self::ParsedSuccessfully { items }
    }

    /// True only for [`ParseOutcome::ParsedSuccessfully`] (including empty).
    pub fn is_parsed_success(&self) -> bool {
        matches!(self, Self::ParsedSuccessfully { .. })
    }

    pub fn is_fetch_failure(&self) -> bool {
        matches!(self, Self::FetchFailed { .. })
    }

    pub fn is_parse_failure(&self) -> bool {
        matches!(self, Self::ParseFailed { .. })
    }

    /// Items when parsing succeeded; empty slice for either failure state.
    pub fn items_ref(&self) -> &[T] {
        match self {
            Self::ParsedSuccessfully { items } => items,
            _ => &[],
        }
    }

    /// Consume the outcome into the items it carries (empty on failure).
    pub fn into_items(self) -> Vec<T> {
        match self {
            Self::ParsedSuccessfully { items } => items,
            _ => Vec::new(),
        }
    }

    pub fn item_count(&self) -> usize {
        self.items_ref().len()
    }

    /// Human-readable failure description, or `None` on success.
    pub fn failure_error(&self) -> Option<&str> {
        match self {
            Self::FetchFailed { error, .. } | Self::ParseFailed { error, .. } => Some(error),
            Self::ParsedSuccessfully { .. } => None,
        }
    }

    /// The redacted body sample recorded for a parse failure.
    pub fn redacted_sample(&self) -> Option<&str> {
        match self {
            Self::ParseFailed {
                redacted_sample, ..
            } => Some(redacted_sample),
            _ => None,
        }
    }

    /// Short machine label for logs and metrics.
    pub fn as_label(&self) -> &'static str {
        match self {
            Self::FetchFailed { .. } => "fetch_failed",
            Self::ParseFailed { .. } => "parse_failed",
            Self::ParsedSuccessfully { .. } => "parsed_successfully",
        }
    }

    /// Map the item type while preserving the outcome state.
    pub fn map_items<U>(self, map: impl FnMut(T) -> U) -> ParseOutcome<U> {
        match self {
            Self::FetchFailed { error, http_status } => {
                ParseOutcome::FetchFailed { error, http_status }
            }
            Self::ParseFailed {
                error,
                redacted_sample,
            } => ParseOutcome::ParseFailed {
                error,
                redacted_sample,
            },
            Self::ParsedSuccessfully { items } => ParseOutcome::ParsedSuccessfully {
                items: items.into_iter().map(map).collect(),
            },
        }
    }
}

/// Maximum characters kept from an untrusted body sample.
const SAMPLE_MAX_CHARS: usize = 320;

/// Bounded, secret-masked rendering of an untrusted body sample.
///
/// Control characters are dropped (whitespace normalized), tokens that look
/// like secrets or credentials are replaced, and the result is capped at
/// [`SAMPLE_MAX_CHARS`]. The output is intended for logs/DB metadata only —
/// never for re-parsing.
pub fn redact_sample(raw: &str) -> String {
    let normalized: String = raw
        .chars()
        .take(SAMPLE_MAX_CHARS * 4)
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect();
    let collapsed = normalized.split_whitespace().collect::<Vec<_>>().join(" ");
    let masked = collapsed
        .split(' ')
        .map(mask_token)
        .collect::<Vec<_>>()
        .join(" ");
    masked.chars().take(SAMPLE_MAX_CHARS).collect()
}

fn mask_token(token: &str) -> String {
    let lower = token.to_lowercase();
    let looks_like_assignment = lower.starts_with("api_key")
        || lower.starts_with("apikey")
        || lower.starts_with("token")
        || lower.starts_with("secret")
        || lower.starts_with("authorization")
        || lower.starts_with("password");
    if looks_like_assignment {
        return "[redacted-credential]".to_string();
    }
    if token.contains('@') && token.contains('.') {
        return "[redacted-email]".to_string();
    }
    if token.len() >= 32
        && token
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return "[redacted-token]".to_string();
    }
    token.to_string()
}

/// Point-in-time parser health counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ParserMetricsSnapshot {
    pub fetch_failed: u64,
    pub parse_failed: u64,
    pub parsed_successfully: u64,
    pub items_parsed: u64,
}

impl ParserMetricsSnapshot {
    /// Fraction of *parsed attempts* that succeeded, in `[0, 1]`.
    ///
    /// `None` when nothing has been recorded yet (no evidence either way).
    /// Fetch failures are excluded from the denominator: they measure
    /// transport health, not parser health.
    pub fn parser_success_rate(&self) -> Option<f64> {
        let attempts = self.parsed_successfully + self.parse_failed;
        if attempts == 0 {
            None
        } else {
            Some(self.parsed_successfully as f64 / attempts as f64)
        }
    }
}

/// Global parser health counters, updated by adapters as outcomes occur.
#[derive(Debug, Default)]
pub struct ParserMetrics {
    fetch_failed: AtomicU64,
    parse_failed: AtomicU64,
    parsed_successfully: AtomicU64,
    items_parsed: AtomicU64,
}

impl ParserMetrics {
    pub const fn new() -> Self {
        Self {
            fetch_failed: AtomicU64::new(0),
            parse_failed: AtomicU64::new(0),
            parsed_successfully: AtomicU64::new(0),
            items_parsed: AtomicU64::new(0),
        }
    }

    /// Record one adapter outcome.
    pub fn record<T>(&self, outcome: &ParseOutcome<T>) {
        match outcome {
            ParseOutcome::FetchFailed { .. } => {
                self.fetch_failed.fetch_add(1, Ordering::Relaxed);
            }
            ParseOutcome::ParseFailed { .. } => {
                self.parse_failed.fetch_add(1, Ordering::Relaxed);
            }
            ParseOutcome::ParsedSuccessfully { items } => {
                self.parsed_successfully.fetch_add(1, Ordering::Relaxed);
                self.items_parsed
                    .fetch_add(items.len() as u64, Ordering::Relaxed);
            }
        }
    }

    pub fn snapshot(&self) -> ParserMetricsSnapshot {
        ParserMetricsSnapshot {
            fetch_failed: self.fetch_failed.load(Ordering::Relaxed),
            parse_failed: self.parse_failed.load(Ordering::Relaxed),
            parsed_successfully: self.parsed_successfully.load(Ordering::Relaxed),
            items_parsed: self.items_parsed.load(Ordering::Relaxed),
        }
    }

    /// Reset all counters (test support).
    pub fn reset(&self) {
        self.fetch_failed.store(0, Ordering::Relaxed);
        self.parse_failed.store(0, Ordering::Relaxed);
        self.parsed_successfully.store(0, Ordering::Relaxed);
        self.items_parsed.store(0, Ordering::Relaxed);
    }
}

/// Process-wide parser health counters.
pub static PARSER_METRICS: ParserMetrics = ParserMetrics::new();

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn parse_failed_is_not_empty_success() {
        let outcome: ParseOutcome<u32> = ParseOutcome::parse_failed("schema changed", "{\"a\":1}");
        assert!(!outcome.is_parsed_success());
        assert!(outcome.is_parse_failure());
        assert_eq!(outcome.item_count(), 0);
        assert!(outcome.items_ref().is_empty());
        assert_eq!(outcome.failure_error(), Some("schema changed"));
        assert_eq!(outcome.as_label(), "parse_failed");
    }

    #[test]
    fn parsed_successfully_may_be_empty() {
        let outcome: ParseOutcome<u32> = ParseOutcome::parsed(vec![]);
        assert!(outcome.is_parsed_success());
        assert!(!outcome.is_parse_failure());
        assert_eq!(outcome.item_count(), 0);
        assert_eq!(outcome.failure_error(), None);
    }

    #[test]
    fn redaction_masks_secrets_and_emails() {
        let raw = "error: contact jane.doe@example.com api_key=SUPERSECRETVALUE1234567890\n\
                   token=abcdefghijklmnopqrstuvwxyz0123456789 control:\u{0007}";
        let redacted = redact_sample(raw);
        assert!(!redacted.contains("jane.doe@example.com"), "{redacted}");
        assert!(
            !redacted.contains("SUPERSECRETVALUE1234567890"),
            "{redacted}"
        );
        assert!(!redacted.contains('\u{0007}'));
        assert!(redacted.contains("[redacted-email]"));
    }

    #[test]
    fn redaction_caps_length() {
        let raw = "a ".repeat(10_000);
        assert!(redact_sample(&raw).chars().count() <= SAMPLE_MAX_CHARS);
    }

    #[test]
    fn parser_metrics_track_outcomes_and_rate() {
        let metrics = ParserMetrics::new();
        metrics.record(&ParseOutcome::<u32>::fetch_failed("timeout", None));
        metrics.record(&ParseOutcome::<u32>::parse_failed("bad json", "x"));
        metrics.record(&ParseOutcome::parsed(vec![1, 2, 3]));
        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.fetch_failed, 1);
        assert_eq!(snapshot.parse_failed, 1);
        assert_eq!(snapshot.parsed_successfully, 1);
        assert_eq!(snapshot.items_parsed, 3);
        assert_eq!(snapshot.parser_success_rate(), Some(0.5));

        let empty = ParserMetrics::new();
        assert_eq!(empty.snapshot().parser_success_rate(), None);
    }

    #[test]
    fn outcome_serde_roundtrip() {
        let outcomes = vec![
            ParseOutcome::<String>::fetch_failed("boom", Some(503)),
            ParseOutcome::parse_failed("bad", "sample"),
            ParseOutcome::parsed(vec!["ok".to_string()]),
        ];
        for outcome in outcomes {
            let json = serde_json::to_string(&outcome).expect("serialize outcome");
            let decoded: ParseOutcome<String> =
                serde_json::from_str(&json).expect("deserialize outcome");
            assert_eq!(decoded, outcome);
        }
    }
}
