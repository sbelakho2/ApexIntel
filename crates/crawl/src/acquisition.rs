//! Structured acquisition outcomes for source adapters (audit P0-10/P0-11).
//!
//! Source adapters historically returned `Result<Vec<T>>` (or a bare
//! `Vec<T>`) and converted every network, HTTP, authentication or parse
//! failure into an ordinary empty collection. Downstream that was
//! indistinguishable from a genuine "no findings today", so a source that was
//! rate-limited, unauthenticated or outright broken still looked like a
//! healthy success and could be promoted to `Operational`.
//!
//! [`AcquisitionOutcome`] makes those states explicit. The *only* variant that
//! counts as a successful zero-findings run is
//! [`AcquisitionOutcome::Success`] with an empty `items` vec; every other
//! variant records a failure, an unavailability or an authentication gap and
//! can never update a source's `last_success_at` or promote it to operational.
//!
//! [`SourceAdapter`] is the trait every source-specific client implements, so
//! an implementation physically cannot represent a fetch/network/auth/parse
//! failure as an empty success. [`ADAPTER_PREREQUISITES`] publishes each
//! adapter's deployment prerequisites (credentials, Tor, browser) so
//! `effective_capability` can resolve capability from what the adapter needs
//! rather than from a static registry declaration.

use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Detail attached to a [`AcquisitionOutcome::FetchFailed`] or
/// [`AcquisitionOutcome::ParseFailed`] outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AcquisitionFailure {
    /// Human-readable failure description (never a secret).
    pub message: String,
    /// HTTP status when a response was received.
    pub http_status: Option<u16>,
    /// Bounded, secret-masked body sample for parse incidents.
    pub redacted_sample: Option<String>,
}

impl AcquisitionFailure {
    pub fn new(message: impl Into<String>, http_status: Option<u16>) -> Self {
        Self {
            message: message.into(),
            http_status,
            redacted_sample: None,
        }
    }

    pub fn with_sample(mut self, sample: &str) -> Self {
        self.redacted_sample = Some(crate::parse_outcome::redact_sample(sample));
        self
    }
}

/// One source-adapter acquisition attempt.
///
/// `Success { items: [] }` is the *only* successful zero-findings run. Every
/// other variant is a non-success outcome: it records failure/unavailable
/// state and never counts as an operational success.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "outcome")]
pub enum AcquisitionOutcome<T> {
    /// The fetch and parse contract both held. `items` may legitimately be
    /// empty; that is a successful zero-findings run.
    Success {
        items: Vec<T>,
        fetched_at: DateTime<Utc>,
    },
    /// The adapter does not apply to this request (for example a US SEC
    /// adapter asked about a private non-US company). Neither a success nor a
    /// failure: it must not update `last_success_at` and must not degrade the
    /// source's health.
    NotApplicable,
    /// The deployment cannot execute the adapter (missing Tor, missing
    /// browser, disabled transport, upstream endpoint gone).
    Unavailable { reason: String },
    /// The upstream rate-limited the request. Never a success.
    RateLimited {
        /// Server-provided `Retry-After`, in seconds, when the response
        /// carried one.
        retry_after: Option<u64>,
    },
    /// The adapter requires credentials the deployment does not have. Never a
    /// success and can never be classed operational.
    AuthenticationRequired,
    /// The network/HTTP fetch failed before a body could be parsed.
    FetchFailed { failure: AcquisitionFailure },
    /// A body arrived but deserialization or schema validation failed.
    ParseFailed { failure: AcquisitionFailure },
}

impl<T> AcquisitionOutcome<T> {
    pub fn success(items: Vec<T>, fetched_at: DateTime<Utc>) -> Self {
        Self::Success { items, fetched_at }
    }

    pub fn success_now(items: Vec<T>) -> Self {
        Self::Success {
            items,
            fetched_at: Utc::now(),
        }
    }

    pub fn rate_limited(retry_after: Option<u64>) -> Self {
        Self::RateLimited { retry_after }
    }

    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self::Unavailable {
            reason: reason.into(),
        }
    }

    pub fn fetch_failed(message: impl Into<String>, http_status: Option<u16>) -> Self {
        Self::FetchFailed {
            failure: AcquisitionFailure::new(message, http_status),
        }
    }

    pub fn parse_failed(message: impl Into<String>, sample: &str) -> Self {
        Self::ParseFailed {
            failure: AcquisitionFailure::new(message, None).with_sample(sample),
        }
    }

    /// True only for [`AcquisitionOutcome::Success`] (including empty items).
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Success { .. })
    }

    /// True when the outcome is a transport/parse failure.
    pub fn is_failure(&self) -> bool {
        matches!(self, Self::FetchFailed { .. } | Self::ParseFailed { .. })
    }

    /// True for every outcome that must record failure/unavailable runtime
    /// state. `NotApplicable` is deliberately excluded: the adapter never ran,
    /// so degrading the source's health would be a lie.
    pub fn records_failure(&self) -> bool {
        self.disposition().records_failure()
    }

    /// True only for outcomes that may update `last_success_at`.
    pub fn updates_last_success_at(&self) -> bool {
        self.is_success()
    }

    /// Items when acquisition succeeded; empty slice otherwise.
    pub fn items_ref(&self) -> &[T] {
        match self {
            Self::Success { items, .. } => items,
            _ => &[],
        }
    }

    /// Consume the outcome into the items it carries (empty on non-success).
    pub fn into_items(self) -> Vec<T> {
        match self {
            Self::Success { items, .. } => items,
            _ => Vec::new(),
        }
    }

    pub fn item_count(&self) -> usize {
        self.items_ref().len()
    }

    pub fn fetched_at(&self) -> Option<DateTime<Utc>> {
        match self {
            Self::Success { fetched_at, .. } => Some(*fetched_at),
            _ => None,
        }
    }

    /// Server-provided retry delay, when rate-limited.
    pub fn retry_after(&self) -> Option<Duration> {
        match self {
            Self::RateLimited { retry_after } => retry_after.map(Duration::from_secs),
            _ => None,
        }
    }

    /// Human-readable failure description, or `None` on success.
    pub fn failure_message(&self) -> Option<&str> {
        match self {
            Self::FetchFailed { failure } | Self::ParseFailed { failure } => {
                Some(failure.message.as_str())
            }
            Self::Unavailable { reason } => Some(reason.as_str()),
            Self::RateLimited { .. } => Some("rate limited by upstream"),
            Self::AuthenticationRequired => Some("authentication required"),
            _ => None,
        }
    }

    /// HTTP status associated with the outcome, when one was observed.
    pub fn http_status(&self) -> Option<u16> {
        match self {
            Self::Success { .. } => Some(200),
            Self::FetchFailed { failure } | Self::ParseFailed { failure } => failure.http_status,
            Self::RateLimited { .. } => Some(429),
            _ => None,
        }
    }

    /// Runtime/capability disposition this outcome maps to.
    pub fn disposition(&self) -> AcquisitionDisposition {
        match self {
            Self::Success { .. } => AcquisitionDisposition::Operational,
            Self::NotApplicable => AcquisitionDisposition::NotApplicable,
            Self::Unavailable { .. } => AcquisitionDisposition::Unavailable,
            Self::RateLimited { .. } => AcquisitionDisposition::RateLimited,
            Self::AuthenticationRequired => AcquisitionDisposition::AuthenticationBlocked,
            Self::FetchFailed { .. } | Self::ParseFailed { .. } => AcquisitionDisposition::Degraded,
        }
    }

    /// Short machine label for logs, metrics and persisted runtime state.
    pub fn as_label(&self) -> &'static str {
        self.disposition().as_str()
    }

    /// Map the item type while preserving the outcome state.
    pub fn map_items<U>(self, map: impl FnMut(T) -> U) -> AcquisitionOutcome<U> {
        match self {
            Self::Success { items, fetched_at } => AcquisitionOutcome::Success {
                items: items.into_iter().map(map).collect(),
                fetched_at,
            },
            Self::NotApplicable => AcquisitionOutcome::NotApplicable,
            Self::Unavailable { reason } => AcquisitionOutcome::Unavailable { reason },
            Self::RateLimited { retry_after } => AcquisitionOutcome::RateLimited { retry_after },
            Self::AuthenticationRequired => AcquisitionOutcome::AuthenticationRequired,
            Self::FetchFailed { failure } => AcquisitionOutcome::FetchFailed { failure },
            Self::ParseFailed { failure } => AcquisitionOutcome::ParseFailed { failure },
        }
    }
}

/// How a [`AcquisitionOutcome`] maps onto runtime health and capability state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AcquisitionDisposition {
    /// A successful fetch+parse contract (including zero findings).
    Operational,
    /// The adapter did not apply; no attempt and no health change.
    NotApplicable,
    /// Transport or parser failure: record a failure, never a success.
    Degraded,
    /// The deployment cannot execute the adapter: unavailable, never a success.
    Unavailable,
    /// Upstream rate limit: record a failure with the retry hint, never a
    /// success.
    RateLimited,
    /// Missing credentials: record the credential gap; the source can never be
    /// operational.
    AuthenticationBlocked,
}

impl AcquisitionDisposition {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Operational => "success",
            Self::NotApplicable => "not_applicable",
            Self::Degraded => "degraded",
            Self::Unavailable => "unavailable",
            Self::RateLimited => "rate_limited",
            Self::AuthenticationBlocked => "authentication_required",
        }
    }

    /// True when the disposition must record failure/unavailable state.
    pub fn records_failure(self) -> bool {
        !matches!(self, Self::Operational | Self::NotApplicable)
    }

    /// True when the disposition may count as an operational success.
    pub fn counts_as_operational_success(self) -> bool {
        matches!(self, Self::Operational)
    }

    /// True when the source's capability is blocked on deployment
    /// prerequisites rather than on a transient runtime failure.
    pub fn is_capability_blocked(self) -> bool {
        matches!(self, Self::Unavailable | Self::AuthenticationBlocked)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn run_counters_never_report_failure_as_success() {
        // Every source failed → Failed, never Succeeded.
        let mut all_failed = AcquisitionRunCounters::default();
        all_failed.record(&AcquisitionOutcome::<u32>::fetch_failed("down", None));
        all_failed.record(&AcquisitionOutcome::<u32>::unavailable("no tor"));
        assert!(matches!(
            all_failed.decision(),
            AcquisitionRunDecision::Failed { .. }
        ));

        // Some optional failures, at least one success → Degraded.
        let mut partial = AcquisitionRunCounters::default();
        partial.record(&AcquisitionOutcome::<u32>::success_now(vec![1]));
        partial.record(&AcquisitionOutcome::<u32>::fetch_failed("down", None));
        assert!(matches!(
            partial.decision(),
            AcquisitionRunDecision::Degraded { .. }
        ));

        // All healthy with zero findings → Succeeded with empty_findings.
        let mut empty = AcquisitionRunCounters::default();
        empty.record(&AcquisitionOutcome::<u32>::success_now(vec![]));
        assert_eq!(
            empty.decision(),
            AcquisitionRunDecision::Succeeded {
                empty_findings: true
            }
        );

        // Nothing applied → Skipped.
        let mut skipped = AcquisitionRunCounters::default();
        skipped.record(&AcquisitionOutcome::<u32>::NotApplicable);
        assert_eq!(skipped.decision(), AcquisitionRunDecision::Skipped);

        // Required failure forces Failed even when optional sources succeeded.
        let mut required = AcquisitionRunCounters::default();
        required.record(&AcquisitionOutcome::<u32>::success_now(vec![1]));
        required.record_required(&AcquisitionOutcome::<u32>::parse_failed("bad", "<xml"));
        assert!(matches!(
            required.decision(),
            AcquisitionRunDecision::Failed { .. }
        ));

        // Persistence failure is never optional.
        let mut persistence = AcquisitionRunCounters::default();
        persistence.record(&AcquisitionOutcome::<u32>::success_now(vec![1]));
        persistence.record_persistence_failure();
        assert!(matches!(
            persistence.decision(),
            AcquisitionRunDecision::Failed { .. }
        ));
    }

    #[test]
    fn only_success_counts_as_a_successful_run_including_empty() {
        let empty: AcquisitionOutcome<u32> = AcquisitionOutcome::success_now(vec![]);
        assert!(empty.is_success());
        assert!(!empty.records_failure());
        assert!(empty.updates_last_success_at());
        assert!(empty.items_ref().is_empty());
        assert_eq!(empty.item_count(), 0);

        let one: AcquisitionOutcome<u32> = AcquisitionOutcome::success_now(vec![1]);
        assert!(one.is_success());
        assert_eq!(one.item_count(), 1);
    }

    #[test]
    fn rate_limited_is_not_a_success() {
        let outcome: AcquisitionOutcome<u32> = AcquisitionOutcome::rate_limited(Some(60));
        assert!(!outcome.is_success());
        assert!(outcome.records_failure());
        assert!(!outcome.updates_last_success_at());
        assert_eq!(outcome.http_status(), Some(429));
        assert_eq!(outcome.retry_after(), Some(Duration::from_secs(60)));
        assert_eq!(outcome.disposition(), AcquisitionDisposition::RateLimited);
    }

    #[test]
    fn authentication_required_is_capability_blocked() {
        let outcome: AcquisitionOutcome<u32> = AcquisitionOutcome::AuthenticationRequired;
        assert!(!outcome.is_success());
        assert!(outcome.records_failure());
        assert!(outcome.disposition().is_capability_blocked());
        assert_eq!(outcome.as_label(), "authentication_required");
    }

    #[test]
    fn every_variant_maps_to_its_runtime_disposition() {
        let cases: Vec<(AcquisitionOutcome<u32>, AcquisitionDisposition)> = vec![
            (
                AcquisitionOutcome::success_now(vec![]),
                AcquisitionDisposition::Operational,
            ),
            (
                AcquisitionOutcome::NotApplicable,
                AcquisitionDisposition::NotApplicable,
            ),
            (
                AcquisitionOutcome::unavailable("no Tor"),
                AcquisitionDisposition::Unavailable,
            ),
            (
                AcquisitionOutcome::rate_limited(None),
                AcquisitionDisposition::RateLimited,
            ),
            (
                AcquisitionOutcome::AuthenticationRequired,
                AcquisitionDisposition::AuthenticationBlocked,
            ),
            (
                AcquisitionOutcome::fetch_failed("timeout", Some(503)),
                AcquisitionDisposition::Degraded,
            ),
            (
                AcquisitionOutcome::parse_failed("bad json", "{\"a\":"),
                AcquisitionDisposition::Degraded,
            ),
        ];
        for (outcome, expected) in cases {
            assert_eq!(outcome.disposition(), expected, "{}", outcome.as_label());
            assert_eq!(
                outcome.records_failure(),
                !matches!(
                    expected,
                    AcquisitionDisposition::Operational | AcquisitionDisposition::NotApplicable
                ),
                "unexpected failure recording for {}",
                expected.as_str()
            );
        }
    }

    #[test]
    fn not_applicable_is_neither_success_nor_failure() {
        let outcome: AcquisitionOutcome<u32> = AcquisitionOutcome::NotApplicable;
        assert!(!outcome.is_success());
        assert!(!outcome.records_failure());
        assert!(!outcome.updates_last_success_at());
        assert!(!outcome.disposition().counts_as_operational_success());
    }

    #[test]
    fn parse_failed_carries_redacted_sample() {
        let outcome: AcquisitionOutcome<u32> = AcquisitionOutcome::parse_failed(
            "schema changed",
            "contact jane.doe@example.com api_key=SUPERSECRET",
        );
        let failure = match &outcome {
            AcquisitionOutcome::ParseFailed { failure } => failure,
            other => panic!("expected ParseFailed, got {other:?}"),
        };
        let sample = failure.redacted_sample.as_deref().unwrap_or_default();
        assert!(!sample.contains("jane.doe@example.com"));
        assert!(sample.contains("[redacted-email]"));
    }

    #[test]
    fn http_failure_never_maps_to_success() {
        let limited: AcquisitionOutcome<u32> = http_failure(429, Some(120), "nvd");
        assert!(!limited.is_success());
        assert_eq!(limited.disposition(), AcquisitionDisposition::RateLimited);
        assert_eq!(limited.retry_after(), Some(Duration::from_secs(120)));

        let unauthorized: AcquisitionOutcome<u32> = http_failure(401, None, "linkedin");
        assert_eq!(
            unauthorized.disposition(),
            AcquisitionDisposition::AuthenticationBlocked
        );

        let forbidden: AcquisitionOutcome<u32> = http_failure(403, None, "twitter");
        assert_eq!(
            forbidden.disposition(),
            AcquisitionDisposition::AuthenticationBlocked
        );

        let server: AcquisitionOutcome<u32> = http_failure(503, None, "openalex");
        assert_eq!(server.disposition(), AcquisitionDisposition::Degraded);
        assert_eq!(server.http_status(), Some(503));
    }

    #[test]
    fn aggregate_prefers_success_and_otherwise_the_most_actionable_failure() {
        let combined: AcquisitionOutcome<u32> = aggregate(vec![
            AcquisitionOutcome::success_now(vec![1]),
            AcquisitionOutcome::rate_limited(Some(10)),
            AcquisitionOutcome::success_now(vec![2]),
        ]);
        assert!(combined.is_success());
        assert_eq!(combined.item_count(), 2);

        let auth: AcquisitionOutcome<u32> = aggregate(vec![
            AcquisitionOutcome::fetch_failed("boom", None),
            AcquisitionOutcome::AuthenticationRequired,
        ]);
        assert_eq!(
            auth.disposition(),
            AcquisitionDisposition::AuthenticationBlocked
        );

        let unavailable: AcquisitionOutcome<u32> = aggregate(vec![
            AcquisitionOutcome::rate_limited(None),
            AcquisitionOutcome::unavailable("no Tor"),
        ]);
        assert_eq!(
            unavailable.disposition(),
            AcquisitionDisposition::Unavailable
        );

        let empty: AcquisitionOutcome<u32> = aggregate(vec![]);
        assert_eq!(empty.disposition(), AcquisitionDisposition::NotApplicable);
    }

    #[test]
    fn propagate_failure_preserves_non_success_variants() {
        let limited: AcquisitionOutcome<u32> = AcquisitionOutcome::rate_limited(Some(5));
        let retyped: AcquisitionOutcome<String> =
            propagate_failure(limited).expect("rate limit must propagate");
        assert_eq!(retyped.disposition(), AcquisitionDisposition::RateLimited);

        let success: AcquisitionOutcome<u32> = AcquisitionOutcome::success_now(vec![]);
        assert!(
            propagate_failure::<u32, String>(success).is_none(),
            "success must not be converted into a failure"
        );

        let not_applicable: AcquisitionOutcome<u32> = AcquisitionOutcome::NotApplicable;
        let retyped: AcquisitionOutcome<String> =
            propagate_failure(not_applicable).expect("not-applicable must propagate");
        assert_eq!(retyped.disposition(), AcquisitionDisposition::NotApplicable);
    }

    #[test]
    fn adapter_prerequisites_are_published_for_credentialed_adapters() {
        for id in ["linkedin", "twitter", "censys"] {
            let descriptor = adapter_descriptor(id).unwrap_or_else(|| panic!("{id} missing"));
            assert!(
                descriptor.prerequisite.requires_credentials,
                "{id} must require credentials"
            );
        }
        let linkedin = adapter_descriptor_for_slug("linkedin_company")
            .unwrap_or_else(|| panic!("linkedin_company must map to an adapter"));
        assert!(linkedin.prerequisite.requires_credentials);
        let twitter = adapter_descriptor_for_slug("twitter_search")
            .unwrap_or_else(|| panic!("twitter_search must map to an adapter"));
        assert!(twitter.prerequisite.requires_credentials);
        assert!(
            !adapter_descriptor("cve")
                .expect("cve descriptor")
                .prerequisite
                .requires_credentials
        );
    }

    #[test]
    fn serde_roundtrip_preserves_variant() {
        let outcomes = vec![
            AcquisitionOutcome::<String>::success_now(vec!["ok".to_string()]),
            AcquisitionOutcome::NotApplicable,
            AcquisitionOutcome::unavailable("no browser"),
            AcquisitionOutcome::rate_limited(Some(30)),
            AcquisitionOutcome::AuthenticationRequired,
            AcquisitionOutcome::fetch_failed("boom", Some(503)),
            AcquisitionOutcome::parse_failed("bad", "sample"),
        ];
        for outcome in outcomes {
            let json = serde_json::to_string(&outcome).expect("serialize outcome");
            let decoded: AcquisitionOutcome<String> =
                serde_json::from_str(&json).expect("deserialize outcome");
            assert_eq!(decoded, outcome);
        }
    }
}

/// Deployment prerequisites an adapter needs before it can run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AdapterPrerequisite {
    /// The adapter needs deployment credentials (API key/OAuth token/basic
    /// auth) and can never be called without them.
    pub requires_credentials: bool,
    /// The adapter routes through the Tor dark-web transport.
    pub requires_tor: bool,
    /// The adapter needs the headless browser renderer.
    pub requires_browser: bool,
}

impl AdapterPrerequisite {
    pub const NONE: Self = Self {
        requires_credentials: false,
        requires_tor: false,
        requires_browser: false,
    };

    pub const CREDENTIALS: Self = Self {
        requires_credentials: true,
        requires_tor: false,
        requires_browser: false,
    };

    pub const TOR: Self = Self {
        requires_credentials: false,
        requires_tor: true,
        requires_browser: false,
    };
}

/// Static deployment prerequisites for one adapter id, plus the registry
/// source slugs it serves.
///
/// This catalogue is the source of truth for
/// [`crate::sources_registry::effective_capability`]: a source served by an
/// adapter that requires credentials it lacks can never be classed
/// operational, even when the registry declared it `Unvalidated`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdapterDescriptor {
    pub id: &'static str,
    pub prerequisite: AdapterPrerequisite,
    /// Registry slugs served by this adapter.
    pub source_slugs: &'static [&'static str],
}

/// Every migrated source adapter with its deployment prerequisites.
pub const ADAPTER_PREREQUISITES: &[AdapterDescriptor] = &[
    AdapterDescriptor {
        id: "cve",
        prerequisite: AdapterPrerequisite::NONE,
        source_slugs: &["nvd_nist_vuln"],
    },
    AdapterDescriptor {
        id: "openalex",
        prerequisite: AdapterPrerequisite::NONE,
        source_slugs: &[],
    },
    AdapterDescriptor {
        id: "sec_edgar",
        prerequisite: AdapterPrerequisite::NONE,
        source_slugs: &["sec_edgar"],
    },
    AdapterDescriptor {
        id: "dark_web_paste",
        prerequisite: AdapterPrerequisite::TOR,
        source_slugs: &[],
    },
    AdapterDescriptor {
        id: "dark_web_paste_sites",
        prerequisite: AdapterPrerequisite::TOR,
        source_slugs: &[],
    },
    AdapterDescriptor {
        id: "dark_web_i2p",
        prerequisite: AdapterPrerequisite::TOR,
        source_slugs: &[],
    },
    AdapterDescriptor {
        id: "dark_web_marketplace",
        prerequisite: AdapterPrerequisite::TOR,
        source_slugs: &[],
    },
    AdapterDescriptor {
        id: "linkedin",
        prerequisite: AdapterPrerequisite::CREDENTIALS,
        source_slugs: &["linkedin_company"],
    },
    AdapterDescriptor {
        id: "github",
        prerequisite: AdapterPrerequisite::NONE,
        source_slugs: &[],
    },
    AdapterDescriptor {
        id: "censys",
        prerequisite: AdapterPrerequisite::CREDENTIALS,
        source_slugs: &[],
    },
    AdapterDescriptor {
        id: "tenders",
        prerequisite: AdapterPrerequisite::NONE,
        source_slugs: &[],
    },
    AdapterDescriptor {
        id: "twitter",
        prerequisite: AdapterPrerequisite::CREDENTIALS,
        source_slugs: &["twitter_search"],
    },
];

/// Look up an adapter's published prerequisite by adapter id.
pub fn adapter_descriptor(id: &str) -> Option<&'static AdapterDescriptor> {
    ADAPTER_PREREQUISITES
        .iter()
        .find(|descriptor| descriptor.id == id)
}

/// Look up the adapter serving a registry slug.
pub fn adapter_descriptor_for_slug(slug: &str) -> Option<&'static AdapterDescriptor> {
    ADAPTER_PREREQUISITES
        .iter()
        .find(|descriptor| descriptor.source_slugs.contains(&slug))
}

/// Resolve the adapter descriptor serving a source: the adapter mapped to its
/// slug wins, otherwise a `JsonApi` strategy's adapter id.
pub fn source_adapter_descriptor(
    slug: &str,
    strategy_adapter_id: Option<&str>,
) -> Option<&'static AdapterDescriptor> {
    adapter_descriptor_for_slug(slug).or_else(|| strategy_adapter_id.and_then(adapter_descriptor))
}

/// Resolve the effective prerequisite for a source: the adapter serving its
/// slug wins, otherwise a `JsonApi` strategy's adapter id, otherwise a declared
/// `RequiresCredentials` capability is *not* trusted on its own (prerequisites
/// are adapter facts, not registry declarations).
pub fn source_prerequisite(
    slug: &str,
    strategy_adapter_id: Option<&str>,
) -> Option<AdapterPrerequisite> {
    source_adapter_descriptor(slug, strategy_adapter_id).map(|descriptor| descriptor.prerequisite)
}

/// Combine per-endpoint outcomes from a multi-endpoint adapter.
///
/// If any endpoint succeeded, the aggregate is a success carrying every
/// successful endpoint's items (a partial success is still a success, with the
/// failing endpoints visible in logs). Otherwise the aggregate is the most
/// actionable failure: authentication gap, then unavailability, then rate
/// limit, then fetch/parse failure. `NotApplicable` only propagates when every
/// endpoint was not applicable.
pub fn aggregate<T>(outcomes: Vec<AcquisitionOutcome<T>>) -> AcquisitionOutcome<T> {
    if outcomes.is_empty() {
        return AcquisitionOutcome::NotApplicable;
    }
    let mut items = Vec::new();
    let mut fetched_at: Option<DateTime<Utc>> = None;
    let mut saw_success = false;
    let mut fallback: Option<AcquisitionOutcome<T>> = None;
    let mut rank = 0u8;
    for outcome in outcomes {
        match outcome {
            AcquisitionOutcome::Success {
                items: batch,
                fetched_at: when,
            } => {
                saw_success = true;
                items.extend(batch);
                fetched_at = Some(fetched_at.map_or(when, |current| current.max(when)));
            }
            other => {
                let severity = match other {
                    AcquisitionOutcome::NotApplicable => 0,
                    AcquisitionOutcome::FetchFailed { .. } => 1,
                    AcquisitionOutcome::ParseFailed { .. } => 2,
                    AcquisitionOutcome::RateLimited { .. } => 3,
                    AcquisitionOutcome::Unavailable { .. } => 4,
                    AcquisitionOutcome::AuthenticationRequired => 5,
                    AcquisitionOutcome::Success { .. } => unreachable!(),
                };
                if severity > rank {
                    rank = severity;
                    fallback = Some(other);
                }
            }
        }
    }
    if saw_success {
        return AcquisitionOutcome::Success {
            items,
            fetched_at: fetched_at.unwrap_or_else(Utc::now),
        };
    }
    fallback.unwrap_or(AcquisitionOutcome::NotApplicable)
}

/// Re-type a non-success outcome for another item type, preserving the exact
/// variant. Returns `None` for [`AcquisitionOutcome::Success`], so callers can
/// chain acquisitions and propagate failures without collapsing them:
///
/// ```ignore
/// if let Some(failure) = propagate_failure(lookup) {
///     return failure;
/// }
/// ```
pub fn propagate_failure<T, U>(outcome: AcquisitionOutcome<T>) -> Option<AcquisitionOutcome<U>> {
    match outcome {
        AcquisitionOutcome::Success { .. } => None,
        AcquisitionOutcome::NotApplicable => Some(AcquisitionOutcome::NotApplicable),
        AcquisitionOutcome::Unavailable { reason } => {
            Some(AcquisitionOutcome::Unavailable { reason })
        }
        AcquisitionOutcome::RateLimited { retry_after } => {
            Some(AcquisitionOutcome::RateLimited { retry_after })
        }
        AcquisitionOutcome::AuthenticationRequired => {
            Some(AcquisitionOutcome::AuthenticationRequired)
        }
        AcquisitionOutcome::FetchFailed { failure } => {
            Some(AcquisitionOutcome::FetchFailed { failure })
        }
        AcquisitionOutcome::ParseFailed { failure } => {
            Some(AcquisitionOutcome::ParseFailed { failure })
        }
    }
}

/// Classify a non-success HTTP response into the correct explicit outcome.
///
/// `429` → [`AcquisitionOutcome::RateLimited`], `401`/`403` →
/// [`AcquisitionOutcome::AuthenticationRequired`], anything else →
/// [`AcquisitionOutcome::FetchFailed`]. Never returns success.
pub fn http_failure<T>(
    status: u16,
    retry_after: Option<u64>,
    context: &str,
) -> AcquisitionOutcome<T> {
    match status {
        429 => AcquisitionOutcome::RateLimited { retry_after },
        401 | 403 => AcquisitionOutcome::AuthenticationRequired,
        other => AcquisitionOutcome::fetch_failed(
            format!("{context}: upstream returned HTTP {other}"),
            Some(other),
        ),
    }
}

/// Read a bounded `Retry-After` (seconds) from a response's headers.
pub fn retry_after_secs(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
}

/// Aggregated acquisition outcomes for one worker job run.
///
/// The audit's shared model: a job whose sources all failed must not report
/// `Succeeded` merely because it collected zero items, and a job with a
/// required-stage failure must not be `Degraded`. Every acquisition-heavy job
/// records its per-source outcomes here and derives its final
/// [`JobRunStatus`] from [`Self::decision`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcquisitionRunCounters {
    pub attempted: u64,
    pub succeeded: u64,
    pub successful_empty: u64,
    pub not_applicable: u64,
    pub unavailable: u64,
    pub authentication_required: u64,
    pub rate_limited: u64,
    pub fetch_failed: u64,
    pub parse_failed: u64,
    /// Required stages whose *persistence* failed (data loss, never optional).
    pub persistence_failed: u64,
    /// Required stages attempted / failed (a required failure forces Failed).
    pub required_attempted: u64,
    pub required_failed: u64,
}

/// The job-level verdict derived from [`AcquisitionRunCounters`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcquisitionRunDecision {
    /// Every attempted source was healthy (zero findings allowed).
    Succeeded { empty_findings: bool },
    /// Some optional sources failed or were unavailable; the run completed.
    Degraded { reason: String },
    /// A required source failed, or a required persistence stage failed.
    Failed { reason: String },
    /// Nothing applied to this run.
    Skipped,
}

impl AcquisitionRunCounters {
    /// Record one optional source's outcome.
    pub fn record<T>(&mut self, outcome: &AcquisitionOutcome<T>) {
        self.record_inner(outcome, false);
    }

    /// Record one required source's outcome.
    pub fn record_required<T>(&mut self, outcome: &AcquisitionOutcome<T>) {
        self.record_inner(outcome, true);
    }

    fn record_inner<T>(&mut self, outcome: &AcquisitionOutcome<T>, required: bool) {
        if required {
            self.required_attempted += 1;
        }
        match outcome {
            AcquisitionOutcome::Success { items, .. } => {
                self.attempted += 1;
                self.succeeded += 1;
                if items.is_empty() {
                    self.successful_empty += 1;
                }
            }
            AcquisitionOutcome::NotApplicable => {
                self.not_applicable += 1;
            }
            AcquisitionOutcome::Unavailable { .. } => {
                self.attempted += 1;
                self.unavailable += 1;
                if required {
                    self.required_failed += 1;
                }
            }
            AcquisitionOutcome::RateLimited { .. } => {
                self.attempted += 1;
                self.rate_limited += 1;
                if required {
                    self.required_failed += 1;
                }
            }
            AcquisitionOutcome::AuthenticationRequired => {
                self.attempted += 1;
                self.authentication_required += 1;
                if required {
                    self.required_failed += 1;
                }
            }
            AcquisitionOutcome::FetchFailed { .. } => {
                self.attempted += 1;
                self.fetch_failed += 1;
                if required {
                    self.required_failed += 1;
                }
            }
            AcquisitionOutcome::ParseFailed { .. } => {
                self.attempted += 1;
                self.parse_failed += 1;
                if required {
                    self.required_failed += 1;
                }
            }
        }
    }

    /// Record a required persistence failure (data was lost).
    pub fn record_persistence_failure(&mut self) {
        self.persistence_failed += 1;
    }

    /// True when at least one source was attempted.
    pub fn attempted_any(&self) -> bool {
        self.attempted > 0
    }

    /// True when nothing succeeded.
    pub fn all_failed(&self) -> bool {
        self.attempted > 0 && self.succeeded == 0
    }

    /// Derive the job verdict from the counters.
    ///
    /// * required failure or persistence failure → `Failed`
    /// * any optional failure/unavailability/rate-limit → `Degraded`
    /// * attempts all healthy (zero findings allowed) → `Succeeded`
    /// * nothing applicable → `Skipped`
    pub fn decision(&self) -> AcquisitionRunDecision {
        if self.required_failed > 0 || self.persistence_failed > 0 {
            return AcquisitionRunDecision::Failed {
                reason: self.failure_reason(),
            };
        }
        if self.attempted == 0 {
            return AcquisitionRunDecision::Skipped;
        }
        if self.succeeded == 0 {
            // Every attempted source failed: the job did not do its work,
            // regardless of whether the sources were optional.
            return AcquisitionRunDecision::Failed {
                reason: self.failure_reason(),
            };
        }
        let optional_failures = self.unavailable
            + self.authentication_required
            + self.rate_limited
            + self.fetch_failed
            + self.parse_failed;
        if optional_failures > 0 {
            return AcquisitionRunDecision::Degraded {
                reason: self.failure_reason(),
            };
        }
        AcquisitionRunDecision::Succeeded {
            empty_findings: self.succeeded == self.successful_empty && self.succeeded > 0,
        }
    }

    /// Human-readable reason naming every non-success class.
    pub fn failure_reason(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if self.persistence_failed > 0 {
            parts.push(format!(
                "{} persistence failure(s)",
                self.persistence_failed
            ));
        }
        if self.required_failed > 0 {
            parts.push(format!(
                "{}/{} required source(s) failed",
                self.required_failed, self.required_attempted
            ));
        }
        if self.unavailable > 0 {
            parts.push(format!("{} unavailable", self.unavailable));
        }
        if self.authentication_required > 0 {
            parts.push(format!(
                "{} missing credentials",
                self.authentication_required
            ));
        }
        if self.rate_limited > 0 {
            parts.push(format!("{} rate limited", self.rate_limited));
        }
        if self.fetch_failed > 0 {
            parts.push(format!("{} fetch failed", self.fetch_failed));
        }
        if self.parse_failed > 0 {
            parts.push(format!("{} parse failed", self.parse_failed));
        }
        if parts.is_empty() {
            "no failures recorded".to_string()
        } else {
            parts.join(", ")
        }
    }

    /// One-line summary for job notes.
    pub fn summary(&self) -> String {
        format!(
            "sources: {} attempted, {} succeeded ({} empty), {} failed/blocked",
            self.attempted,
            self.succeeded,
            self.successful_empty,
            self.unavailable
                + self.authentication_required
                + self.rate_limited
                + self.fetch_failed
                + self.parse_failed
        )
    }
}

/// Source adapter contract: one typed acquisition per source.
///
/// The associated `acquire` return type makes it impossible for an
/// implementation to represent fetch/network/auth/parse failure as an ordinary
/// empty success.
#[async_trait]
pub trait SourceAdapter: Send + Sync {
    /// Item type produced by a successful acquisition.
    type Item: Send + Sync + 'static;
    /// Parameters for one acquisition (query, ticker, org, …).
    type Request: Send + Sync + 'static;

    /// Stable adapter id, matching [`AdapterDescriptor::id`].
    fn adapter_id(&self) -> &'static str;

    /// Deployment prerequisites for this adapter.
    fn prerequisite(&self) -> AdapterPrerequisite;

    /// True when the adapter's credential prerequisite is satisfied by this
    /// instance's configuration.
    fn credentials_configured(&self) -> bool {
        !self.prerequisite().requires_credentials
    }

    /// Run one acquisition. Implementations must return the explicit variant
    /// for every non-success state — never an empty success.
    async fn acquire(&self, request: Self::Request) -> AcquisitionOutcome<Self::Item>;
}
