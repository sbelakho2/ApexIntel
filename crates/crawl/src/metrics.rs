use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Per-domain byte counters for a single crawler task (not thread-safe).
///
/// **Thread safety contract (B281)**: this struct requires exclusive mutable
/// access for writes.  When multiple concurrent crawler tasks need to share
/// the same counters, use [`SharedDomainByteMetrics`] instead — it wraps this
/// in an `Arc<Mutex<...>>` and exposes a lock-free-style call surface.
///
/// Single-task usage (e.g., in a sequential crawl loop):
/// ```ignore
/// let mut m = DomainByteMetrics::new();
/// m.record_bytes("example.com", 1024);
/// ```
#[derive(Debug, Default, Clone)]
pub struct DomainByteMetrics {
    bytes_by_domain: HashMap<String, u64>,
}

impl DomainByteMetrics {
    /// Create a new, empty counter map.
    pub fn new() -> Self {
        Self { bytes_by_domain: HashMap::new() }
    }

    /// Accumulate `bytes` for `domain` using saturating addition.
    ///
    /// Saturating addition prevents overflow: if a domain's counter would
    /// exceed `u64::MAX`, it stays at `u64::MAX`.
    pub fn record_bytes(&mut self, domain: &str, bytes: u64) {
        let entry = self.bytes_by_domain.entry(domain.to_string()).or_insert(0);
        *entry = entry.saturating_add(bytes);
    }

    /// Return the total bytes recorded for `domain` (0 if unseen).
    pub fn get(&self, domain: &str) -> u64 {
        self.bytes_by_domain.get(domain).copied().unwrap_or(0)
    }

    /// Return the top `limit` domains by total bytes, sorted descending.
    ///
    /// Ties in byte count are broken by domain name in ascending lexicographic
    /// order.  This secondary sort key guarantees a deterministic, stable
    /// snapshot ordering across repeated calls even when multiple domains have
    /// the same cumulative byte count.
    pub fn top_domains(&self, limit: usize) -> Vec<(String, u64)> {
        let mut items: Vec<(String, u64)> = self
            .bytes_by_domain
            .iter()
            .map(|(k, v)| (k.clone(), *v))
            .collect();
        // Primary: bytes DESC; secondary: domain name ASC (deterministic tiebreak)
        items.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        items.truncate(limit);
        items
    }

    /// Return the total bytes across all domains.
    pub fn total_bytes(&self) -> u64 {
        self.bytes_by_domain
            .values()
            .copied()
            .fold(0u64, u64::saturating_add)
    }

    /// Return the number of distinct domains recorded.
    pub fn domain_count(&self) -> usize {
        self.bytes_by_domain.len()
    }
}

// ────────────────────────────────────────────
// Thread-safe wrapper (B281)
// ────────────────────────────────────────────

/// A concurrency-safe wrapper around [`DomainByteMetrics`].
///
/// Wraps the inner map in an `Arc<Mutex<...>>` so multiple async tasks can
/// share and update the same counter set without external synchronisation.
///
/// **Usage in a multi-task crawl pipeline**:
/// ```ignore
/// let shared = SharedDomainByteMetrics::new();
/// let shared2 = shared.clone(); // cheap Arc clone
/// tokio::spawn(async move {
///     shared2.record_bytes("example.com", 512);
/// });
/// let total = shared.total_bytes();
/// ```
///
/// The lock is held only for the duration of each individual operation.
/// Callers that need consistent multi-step reads should call [`snapshot`](Self::snapshot)
/// to obtain a clone with a single lock acquisition.
#[derive(Debug, Clone, Default)]
pub struct SharedDomainByteMetrics {
    inner: Arc<Mutex<DomainByteMetrics>>,
}

impl SharedDomainByteMetrics {
    /// Create a new shared counter.
    pub fn new() -> Self {
        Self { inner: Arc::new(Mutex::new(DomainByteMetrics::new())) }
    }

    /// Accumulate `bytes` for `domain`.
    ///
    /// # Panics
    /// Panics if the internal mutex is poisoned (only possible if another
    /// thread panicked while holding the lock — i.e., a programmer error).
    pub fn record_bytes(&self, domain: &str, bytes: u64) {
        self.inner.lock().expect("DomainByteMetrics mutex poisoned").record_bytes(domain, bytes);
    }

    /// Return the total bytes recorded for `domain` (0 if unseen).
    pub fn get(&self, domain: &str) -> u64 {
        self.inner.lock().expect("DomainByteMetrics mutex poisoned").get(domain)
    }

    /// Return the top `limit` domains by total bytes.
    pub fn top_domains(&self, limit: usize) -> Vec<(String, u64)> {
        self.inner.lock().expect("DomainByteMetrics mutex poisoned").top_domains(limit)
    }

    /// Return total bytes across all domains.
    pub fn total_bytes(&self) -> u64 {
        self.inner.lock().expect("DomainByteMetrics mutex poisoned").total_bytes()
    }

    /// Return the number of distinct domains recorded.
    pub fn domain_count(&self) -> usize {
        self.inner.lock().expect("DomainByteMetrics mutex poisoned").domain_count()
    }

    /// Take a consistent point-in-time snapshot.
    ///
    /// Acquires the lock once and returns a cloned copy, allowing the caller
    /// to inspect multiple fields without racing against concurrent writers.
    pub fn snapshot(&self) -> DomainByteMetrics {
        self.inner.lock().expect("DomainByteMetrics mutex poisoned").clone()
    }
}

// ────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_bytes() {
        let mut metrics = DomainByteMetrics::new();
        metrics.record_bytes("example.com", 100);
        metrics.record_bytes("example.com", 50);
        assert_eq!(metrics.get("example.com"), 150);
    }

    #[test]
    fn test_total_bytes_saturating() {
        let mut m = DomainByteMetrics::new();
        m.record_bytes("a.com", u64::MAX);
        m.record_bytes("b.com", 1);
        // total_bytes must not overflow
        let total = m.total_bytes();
        assert_eq!(total, u64::MAX, "saturating_add must cap at u64::MAX");
    }

    #[test]
    fn test_domain_count() {
        let mut m = DomainByteMetrics::new();
        assert_eq!(m.domain_count(), 0);
        m.record_bytes("a.com", 10);
        m.record_bytes("b.com", 20);
        assert_eq!(m.domain_count(), 2);
        m.record_bytes("a.com", 5); // duplicate domain
        assert_eq!(m.domain_count(), 2);
    }

    #[test]
    fn test_top_domains_sorted_descending() {
        let mut m = DomainByteMetrics::new();
        m.record_bytes("a.com", 100);
        m.record_bytes("b.com", 500);
        m.record_bytes("c.com", 300);
        let top = m.top_domains(2);
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].0, "b.com");
        assert_eq!(top[1].0, "c.com");
    }

    #[test]
    fn test_shared_metrics_clone_shares_state() {
        let shared = SharedDomainByteMetrics::new();
        let cloned = shared.clone();
        shared.record_bytes("x.com", 42);
        // Both handles must see the same value — they share the Arc
        assert_eq!(cloned.get("x.com"), 42);
    }

    #[test]
    fn test_shared_metrics_snapshot_is_independent() {
        let shared = SharedDomainByteMetrics::new();
        shared.record_bytes("x.com", 10);
        let snap = shared.snapshot();
        // Mutate the live counter — snapshot must be unaffected
        shared.record_bytes("x.com", 90);
        assert_eq!(snap.get("x.com"), 10); // snapshot is frozen
        assert_eq!(shared.get("x.com"), 100); // live counter updated
    }

    #[test]
    fn test_shared_metrics_total_bytes() {
        let shared = SharedDomainByteMetrics::new();
        shared.record_bytes("a.com", 200);
        shared.record_bytes("b.com", 300);
        assert_eq!(shared.total_bytes(), 500);
    }

    // B292: top_domains tie-break ordering
    #[test]
    fn test_top_domains_equal_bytes_ordered_by_domain_name_asc() {
        // a.com and b.com both have 100 bytes — secondary sort by name must be stable
        let mut m = DomainByteMetrics::default();
        m.record_bytes("b.com", 100);
        m.record_bytes("a.com", 100);
        m.record_bytes("z.com", 100);
        let top = m.top_domains(3);
        assert_eq!(top[0].0, "a.com", "first tiebreak should be alphabetically first");
        assert_eq!(top[1].0, "b.com");
        assert_eq!(top[2].0, "z.com");
    }

    #[test]
    fn test_top_domains_snapshot_is_deterministic_across_calls() {
        let mut m = DomainByteMetrics::default();
        m.record_bytes("x.com", 50);
        m.record_bytes("m.com", 50);
        m.record_bytes("a.com", 50);
        // Calling top_domains twice must return the same order
        let first = m.top_domains(3);
        let second = m.top_domains(3);
        assert_eq!(first, second, "top_domains must be deterministic across repeated calls");
    }
}
