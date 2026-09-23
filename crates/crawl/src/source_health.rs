//! Source Health Monitoring for ApexIntel OSINT Platform
//!
//! Implements:
//! - Real-time source availability tracking
//! - Automated source quality scoring
//! - Dead source retirement workflow
//! - New source discovery pipeline

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::{debug, info};

use crate::errors::{CrawlError, CrawlFailureCategory};
use crate::source_scoring::ScoringConfig;
use crate::sources_registry::{Category, Region};

// ─────────────────────────────────────────────────────────────────────────────
// Health Status
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HealthStatus {
    /// Source is healthy and performing normally
    Healthy,
    /// Source has some issues but still functional
    Degraded,
    /// Source is experiencing significant problems
    Unhealthy,
    /// Source has failed completely and should be retired
    Dead,
    /// Source is newly discovered and being evaluated
    Unknown,
}

impl HealthStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            HealthStatus::Healthy => "healthy",
            HealthStatus::Degraded => "degraded",
            HealthStatus::Unhealthy => "unhealthy",
            HealthStatus::Dead => "dead",
            HealthStatus::Unknown => "unknown",
        }
    }

    pub fn from_score(score: f64) -> Self {
        const EPSILON: f64 = 1e-9;
        if score >= 0.8 - EPSILON {
            HealthStatus::Healthy
        } else if score >= 0.6 - EPSILON {
            HealthStatus::Degraded
        } else if score >= 0.3 - EPSILON {
            HealthStatus::Unhealthy
        } else {
            HealthStatus::Dead
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Health Metrics
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthMetrics {
    /// Source identifier (slug)
    pub source_id: String,
    /// Current health status
    pub status: HealthStatus,
    /// Composite health score (0.0 - 1.0)
    pub health_score: f64,
    /// Success rate over the monitoring window
    pub success_rate: f64,
    /// Average response time in milliseconds
    pub avg_response_time_ms: f64,
    /// Number of consecutive failures
    pub consecutive_failures: u32,
    /// Number of consecutive successes
    pub consecutive_successes: u32,
    /// Last successful crawl timestamp
    pub last_success: Option<DateTime<Utc>>,
    /// Last failed crawl timestamp
    pub last_failure: Option<DateTime<Utc>>,
    /// Total crawl attempts in window
    pub total_attempts: u64,
    /// Total successful crawls in window
    pub total_successes: u64,
    /// Total failed crawls in window
    pub total_failures: u64,
    /// Uptime percentage
    pub uptime_percent: f64,
    /// Time since last activity
    pub time_since_last_activity: Duration,
    /// Last check timestamp
    pub last_check: DateTime<Utc>,
}

impl Default for HealthMetrics {
    fn default() -> Self {
        Self {
            source_id: String::new(),
            status: HealthStatus::Unknown,
            health_score: 0.5,
            success_rate: 1.0,
            avg_response_time_ms: 0.0,
            consecutive_failures: 0,
            consecutive_successes: 0,
            last_success: None,
            last_failure: None,
            total_attempts: 0,
            total_successes: 0,
            total_failures: 0,
            uptime_percent: 100.0,
            time_since_last_activity: Duration::MAX,
            last_check: Utc::now(),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Crawl Result
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct CrawlResult {
    pub source_id: String,
    pub url: String,
    pub success: bool,
    pub status_code: Option<u16>,
    pub response_time_ms: u64,
    pub content_length: Option<usize>,
    pub error_category: Option<CrawlFailureCategory>,
    pub timestamp: DateTime<Utc>,
}

impl CrawlResult {
    pub fn success(
        source_id: String,
        url: String,
        response_time_ms: u64,
        content_length: usize,
    ) -> Self {
        Self {
            source_id,
            url,
            success: true,
            status_code: Some(200),
            response_time_ms,
            content_length: Some(content_length),
            error_category: None,
            timestamp: Utc::now(),
        }
    }

    pub fn failure(source_id: String, url: String, error: &CrawlError) -> Self {
        let (status_code, category) = match error {
            CrawlError::HttpStatus { status, .. } => (Some(*status), CrawlFailureCategory::Unknown),
            CrawlError::Transport { category, .. } => (None, *category),
            _ => (None, error.category()),
        };

        Self {
            source_id,
            url,
            success: false,
            status_code,
            response_time_ms: 0,
            content_length: None,
            error_category: Some(category),
            timestamp: Utc::now(),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Health Configuration
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct HealthConfig {
    /// Window for calculating health metrics (in seconds)
    pub metrics_window_secs: u64,
    /// Minimum number of attempts before scoring a source
    pub min_attempts_for_score: u32,
    /// Success rate threshold for healthy status
    pub healthy_success_rate: f64,
    /// Consecutive failure threshold for unhealthy
    pub failure_threshold_for_unhealthy: u32,
    /// Consecutive failure threshold for dead status
    pub failure_threshold_for_dead: u32,
    /// Maximum response time for healthy status (ms)
    pub max_response_time_ms: u64,
    /// Time after which a source is considered stale (hours)
    pub stale_threshold_hours: u64,
    /// Enable automatic retirement of dead sources
    pub auto_retire_dead: bool,
    /// Time before retiring a source after marking dead (hours)
    pub retirement_delay_hours: u64,
    /// Enable new source discovery
    pub enable_discovery: bool,
}

impl Default for HealthConfig {
    fn default() -> Self {
        Self {
            metrics_window_secs: 3600,
            min_attempts_for_score: 10,
            healthy_success_rate: 0.95,
            failure_threshold_for_unhealthy: 3,
            failure_threshold_for_dead: 10,
            max_response_time_ms: 5000,
            stale_threshold_hours: 168,
            auto_retire_dead: true,
            retirement_delay_hours: 24,
            enable_discovery: true,
        }
    }
}

impl HealthConfig {
    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();
        if self.metrics_window_secs == 0 {
            errors.push("metrics_window_secs must be > 0".into());
        }
        if self.failure_threshold_for_dead <= self.failure_threshold_for_unhealthy {
            errors.push(
                "failure_threshold_for_dead must be > failure_threshold_for_unhealthy".into(),
            );
        }
        if self.healthy_success_rate < 0.0 || self.healthy_success_rate > 1.0 {
            errors.push("healthy_success_rate must be in [0, 1]".into());
        }
        errors
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Health Monitor
// ─────────────────────────────────────────────────────────────────────────────

pub struct SourceHealthMonitor {
    config: HealthConfig,
    _scoring_config: ScoringConfig,
    /// Current health metrics per source
    metrics: Arc<RwLock<HashMap<String, HealthMetrics>>>,
    /// Rolling history of crawl results
    history: Arc<RwLock<HashMap<String, Vec<CrawlResult>>>>,
    /// Sources pending retirement
    pending_retirement: Arc<RwLock<HashMap<String, DateTime<Utc>>>>,
    /// Newly discovered source candidates
    discovered_sources: Arc<RwLock<Vec<DiscoveredSource>>>,
    /// Dead sources that have been retired
    retired_sources: Arc<RwLock<HashMap<String, RetiredSource>>>,
}

impl SourceHealthMonitor {
    pub fn new(config: HealthConfig) -> Self {
        Self {
            config,
            _scoring_config: ScoringConfig::default(),
            metrics: Arc::new(RwLock::new(HashMap::new())),
            history: Arc::new(RwLock::new(HashMap::new())),
            pending_retirement: Arc::new(RwLock::new(HashMap::new())),
            discovered_sources: Arc::new(RwLock::new(Vec::new())),
            retired_sources: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn with_scoring_config(config: HealthConfig, scoring_config: ScoringConfig) -> Self {
        Self {
            config,
            _scoring_config: scoring_config,
            metrics: Arc::new(RwLock::new(HashMap::new())),
            history: Arc::new(RwLock::new(HashMap::new())),
            pending_retirement: Arc::new(RwLock::new(HashMap::new())),
            discovered_sources: Arc::new(RwLock::new(Vec::new())),
            retired_sources: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    // ── Recording ──────────────────────────────────────────────────────────

    /// Record a successful crawl
    pub async fn record_success(&self, result: CrawlResult) {
        let source_id = result.source_id.clone();
        self.record_result(result).await;

        {
            let mut metrics = self.metrics.write().await;
            let m = metrics.entry(source_id.clone()).or_default();
            m.source_id = source_id.clone();
            m.consecutive_successes += 1;
            m.consecutive_failures = 0;
            m.last_success = Some(Utc::now());
            m.total_successes += 1;
            m.total_attempts += 1;

            // Update health score inline
            if m.total_attempts > 0 {
                m.success_rate = m.total_successes as f64 / m.total_attempts as f64;
            }
            m.uptime_percent = m.success_rate * 100.0;
            m.time_since_last_activity = if let Some(last) = m.last_success {
                Utc::now()
                    .signed_duration_since(last)
                    .to_std()
                    .unwrap_or(Duration::ZERO)
            } else {
                Duration::ZERO
            };
            m.health_score = self.calculate_score(m);
            m.status = HealthStatus::from_score(m.health_score);
            m.last_check = Utc::now();
        }

        // Check for retirement
        if let Some(m) = self.metrics.read().await.get(&source_id) {
            if m.consecutive_failures >= self.config.failure_threshold_for_dead {
                self.initiate_retirement(&source_id).await;
            }
        }
    }

    /// Record a failed crawl
    pub async fn record_failure(&self, result: CrawlResult) {
        let source_id = result.source_id.clone();
        self.record_result(result).await;

        {
            let mut metrics = self.metrics.write().await;
            let m = metrics.entry(source_id.clone()).or_default();
            m.source_id = source_id.clone();
            m.consecutive_failures += 1;
            m.consecutive_successes = 0;
            m.last_failure = Some(Utc::now());
            m.total_failures += 1;
            m.total_attempts += 1;

            // Update health score inline
            if m.total_attempts > 0 {
                m.success_rate = m.total_successes as f64 / m.total_attempts as f64;
            }
            m.uptime_percent = m.success_rate * 100.0;
            m.time_since_last_activity = if let Some(last) = m.last_failure {
                Utc::now()
                    .signed_duration_since(last)
                    .to_std()
                    .unwrap_or(Duration::ZERO)
            } else {
                Duration::ZERO
            };
            m.health_score = self.calculate_score(m);
            m.status = HealthStatus::from_score(m.health_score);
            m.last_check = Utc::now();
        }

        // Check for retirement
        if let Some(m) = self.metrics.read().await.get(&source_id) {
            if m.consecutive_failures >= self.config.failure_threshold_for_dead {
                self.initiate_retirement(&source_id).await;
            }
        }
    }

    async fn record_result(&self, result: CrawlResult) {
        let mut history = self.history.write().await;
        let entries = history
            .entry(result.source_id.clone())
            .or_insert_with(Vec::new);
        entries.push(result);

        // Trim old entries
        let cutoff_ts =
            Utc::now() - chrono::Duration::seconds(self.config.metrics_window_secs as i64);
        entries.retain(|r| r.timestamp > cutoff_ts);
    }

    fn calculate_score(&self, m: &HealthMetrics) -> f64 {
        // Need minimum attempts for meaningful score
        if m.total_attempts < self.config.min_attempts_for_score as u64 {
            return 0.5; // Neutral
        }

        let mut score = 0.0;

        // Success rate factor (0-0.5)
        let success_weight = 0.5;
        score += m.success_rate * success_weight;

        // Consecutive success bonus (0-0.2)
        let consecutive_bonus = (m.consecutive_successes as f64 / 10.0).min(1.0) * 0.2;
        score += consecutive_bonus;

        // Response time factor (0-0.2)
        // Assume 5000ms is the worst acceptable response time
        let response_factor = if m.avg_response_time_ms > 0.0 {
            ((self.config.max_response_time_ms as f64
                - m.avg_response_time_ms
                    .min(self.config.max_response_time_ms as f64))
                / self.config.max_response_time_ms as f64)
                .max(0.0)
        } else {
            0.5 // Neutral if no data
        };
        score += response_factor * 0.2;

        // Consecutive failure penalty (0-0.1)
        let failure_penalty = (m.consecutive_failures as f64 / 10.0).min(1.0) * 0.1;
        score -= failure_penalty;

        score.clamp(0.0, 1.0)
    }

    // ── Status Management ──────────────────────────────────────────────────

    /// Get current health status for a source
    pub async fn get_status(&self, source_id: &str) -> Option<HealthStatus> {
        self.metrics.read().await.get(source_id).map(|m| m.status)
    }

    /// Get detailed health metrics for a source
    pub async fn get_metrics(&self, source_id: &str) -> Option<HealthMetrics> {
        self.metrics.read().await.get(source_id).cloned()
    }

    /// Get all sources by status
    pub async fn get_sources_by_status(&self, status: HealthStatus) -> Vec<String> {
        self.metrics
            .read()
            .await
            .iter()
            .filter(|(_, m)| m.status == status)
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Get aggregate health statistics
    pub async fn get_statistics(&self) -> HealthStatistics {
        let metrics = self.metrics.read().await;

        let total = metrics.len() as u64;
        let healthy = metrics
            .values()
            .filter(|m| m.status == HealthStatus::Healthy)
            .count() as u64;
        let degraded = metrics
            .values()
            .filter(|m| m.status == HealthStatus::Degraded)
            .count() as u64;
        let unhealthy = metrics
            .values()
            .filter(|m| m.status == HealthStatus::Unhealthy)
            .count() as u64;
        let dead = metrics
            .values()
            .filter(|m| m.status == HealthStatus::Dead)
            .count() as u64;
        let unknown = metrics
            .values()
            .filter(|m| m.status == HealthStatus::Unknown)
            .count() as u64;

        let avg_score = if total > 0 {
            metrics.values().map(|m| m.health_score).sum::<f64>() / total as f64
        } else {
            0.0
        };

        HealthStatistics {
            total_sources: total,
            healthy,
            degraded,
            unhealthy,
            dead,
            unknown,
            avg_health_score: avg_score,
        }
    }

    /// Get all health metrics
    pub async fn get_all_metrics(&self) -> Vec<HealthMetrics> {
        self.metrics.read().await.values().cloned().collect()
    }

    // ── Retirement Workflow ─────────────────────────────────────────────────

    /// Initiate retirement process for a dead source
    async fn initiate_retirement(&self, source_id: &str) {
        let mut pending = self.pending_retirement.write().await;
        if !pending.contains_key(source_id) {
            pending.insert(source_id.to_string(), Utc::now());
            info!(source_id = %source_id, "source marked for retirement");
        }
    }

    /// Process pending retirements
    pub async fn process_retirements(&self) -> Vec<String> {
        let mut pending = self.pending_retirement.write().await;
        let mut to_retire = Vec::new();

        let now = Utc::now();
        let delay = chrono::Duration::hours(self.config.retirement_delay_hours as i64);

        pending.retain(|source_id, marked_at| {
            if now.signed_duration_since(*marked_at) >= delay {
                to_retire.push(source_id.clone());
                false
            } else {
                true
            }
        });

        drop(pending);

        // Record retirements
        for source_id in &to_retire {
            let mut retired = self.retired_sources.write().await;
            if let Some(metrics) = self.metrics.read().await.get(source_id).cloned() {
                retired.insert(
                    source_id.clone(),
                    RetiredSource {
                        source_id: source_id.clone(),
                        retired_at: Utc::now(),
                        metrics,
                    },
                );
            }
        }

        if !to_retire.is_empty() {
            info!(count = to_retire.len(), "retired dead sources");
        }

        to_retire
    }

    /// Get list of sources pending retirement
    pub async fn get_pending_retirements(&self) -> Vec<(String, DateTime<Utc>)> {
        self.pending_retirement
            .read()
            .await
            .iter()
            .map(|(k, v)| (k.clone(), *v))
            .collect()
    }

    /// Get list of retired sources
    pub async fn get_retired_sources(&self) -> Vec<RetiredSource> {
        self.retired_sources
            .read()
            .await
            .values()
            .cloned()
            .collect()
    }

    /// Manually revive a retired source
    pub async fn revive_source(&self, source_id: &str) {
        // Remove from retired
        self.retired_sources.write().await.remove(source_id);

        // Reset metrics
        let mut metrics = self.metrics.write().await;
        if let Some(m) = metrics.get_mut(source_id) {
            m.status = HealthStatus::Unknown;
            m.consecutive_failures = 0;
            m.consecutive_successes = 0;
            m.health_score = 0.5;
        }
    }

    // ── New Source Discovery ───────────────────────────────────────────────

    /// Suggest a new source for investigation
    pub async fn suggest_source(&self, suggestion: NewSourceSuggestion) {
        let mut sources = self.discovered_sources.write().await;

        // Avoid duplicates
        if sources.iter().any(|s| s.url == suggestion.url) {
            return;
        }

        let discovered_url = suggestion.url.clone();
        sources.push(DiscoveredSource {
            url: suggestion.url,
            source_type: suggestion.source_type,
            discovered_at: Utc::now(),
            priority: suggestion.priority,
            region: suggestion.region,
            category: suggestion.category,
            discovered_via: suggestion.discovered_via,
            confidence: suggestion.confidence,
        });

        debug!(url = %discovered_url, "new source discovered");
    }

    /// Get pending source discoveries
    pub async fn get_discoveries(&self) -> Vec<DiscoveredSource> {
        self.discovered_sources.read().await.clone()
    }

    /// Mark a discovery as evaluated (and optionally promote it)
    pub async fn evaluate_discovery(&self, url: &str, promoted: bool) {
        let mut sources = self.discovered_sources.write().await;
        sources.retain(|s| s.url != url);

        if promoted {
            info!(url = %url, "source promoted to registry");
        }
    }

    /// Get discovery statistics
    pub async fn get_discovery_stats(&self) -> DiscoveryStats {
        let sources = self.discovered_sources.read().await;

        let mut by_priority = HashMap::new();
        for s in sources.iter() {
            *by_priority.entry(s.priority).or_insert(0) += 1;
        }

        let mut by_type = HashMap::new();
        for s in sources.iter() {
            *by_type.entry(s.source_type).or_insert(0) += 1;
        }

        DiscoveryStats {
            total_discoveries: sources.len(),
            by_priority,
            by_type,
            avg_confidence: if sources.is_empty() {
                0.0
            } else {
                sources.iter().map(|s| s.confidence).sum::<f64>() / sources.len() as f64
            },
        }
    }

    // ── Cleanup ────────────────────────────────────────────────────────────

    /// Clean up stale entries
    pub async fn cleanup(&self) {
        let stale_threshold = Duration::from_secs(self.config.stale_threshold_hours * 3600);

        let mut metrics = self.metrics.write().await;
        metrics.retain(|_, m| {
            m.time_since_last_activity < stale_threshold || m.status == HealthStatus::Dead
        });

        let mut history = self.history.write().await;
        let cutoff_ts =
            Utc::now() - chrono::Duration::seconds(self.config.metrics_window_secs as i64);
        for entries in history.values_mut() {
            entries.retain(|r| r.timestamp > cutoff_ts);
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Supporting Types
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthStatistics {
    pub total_sources: u64,
    pub healthy: u64,
    pub degraded: u64,
    pub unhealthy: u64,
    pub dead: u64,
    pub unknown: u64,
    pub avg_health_score: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetiredSource {
    pub source_id: String,
    pub retired_at: DateTime<Utc>,
    pub metrics: HealthMetrics,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredSource {
    pub url: String,
    pub source_type: SourceType,
    pub discovered_at: DateTime<Utc>,
    pub priority: u8,
    pub region: Option<Region>,
    pub category: Option<Category>,
    pub discovered_via: DiscoveryMethod,
    pub confidence: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SourceType {
    News,
    Government,
    Corporate,
    SocialMedia,
    Forum,
    Blog,
    Academic,
    Patent,
    Sanctions,
    Procurement,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DiscoveryMethod {
    Manual,
    AutomatedScan,
    LinkAnalysis,
    UserSuggestion,
    RecipeFire,
}

#[derive(Debug, Clone)]
pub struct NewSourceSuggestion {
    pub url: String,
    pub source_type: SourceType,
    pub priority: u8,
    pub region: Option<Region>,
    pub category: Option<Category>,
    pub discovered_via: DiscoveryMethod,
    pub confidence: f64,
}

impl Default for NewSourceSuggestion {
    fn default() -> Self {
        Self {
            url: String::new(),
            source_type: SourceType::Other,
            priority: 5,
            region: None,
            category: None,
            discovered_via: DiscoveryMethod::Manual,
            confidence: 0.5,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryStats {
    pub total_discoveries: usize,
    pub by_priority: HashMap<u8, usize>,
    pub by_type: HashMap<SourceType, usize>,
    pub avg_confidence: f64,
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn create_monitor() -> SourceHealthMonitor {
        SourceHealthMonitor::new(HealthConfig::default())
    }

    #[tokio::test]
    async fn test_record_success_updates_metrics() {
        let monitor = create_monitor();
        let result = CrawlResult::success(
            "test-source".into(),
            "https://example.com".into(),
            500,
            1000,
        );
        monitor.record_success(result).await;

        let metrics = monitor.get_metrics("test-source").await;
        assert!(metrics.is_some());
        let m = metrics.unwrap();
        assert_eq!(m.consecutive_successes, 1);
        assert_eq!(m.consecutive_failures, 0);
        assert_eq!(m.total_successes, 1);
    }

    #[tokio::test]
    async fn test_record_failure_updates_metrics() {
        let monitor = create_monitor();
        let error = CrawlError::Transport {
            url: "https://example.com".into(),
            message: "timeout".into(),
            category: CrawlFailureCategory::Timeout,
        };
        let result =
            CrawlResult::failure("test-source".into(), "https://example.com".into(), &error);
        monitor.record_failure(result).await;

        let metrics = monitor.get_metrics("test-source").await;
        assert!(metrics.is_some());
        let m = metrics.unwrap();
        assert_eq!(m.consecutive_failures, 1);
        assert!(m.last_failure.is_some());
    }

    #[tokio::test]
    async fn test_health_status_from_score() {
        assert_eq!(HealthStatus::from_score(0.9), HealthStatus::Healthy);
        assert_eq!(HealthStatus::from_score(0.7), HealthStatus::Degraded);
        assert_eq!(HealthStatus::from_score(0.4), HealthStatus::Unhealthy);
        assert_eq!(HealthStatus::from_score(0.1), HealthStatus::Dead);
    }

    #[tokio::test]
    async fn test_get_sources_by_status() {
        let monitor = create_monitor();

        // Add some sources with multiple attempts to get above min_attempts_for_score
        for _ in 0..30 {
            let success = CrawlResult::success("source-1".into(), "https://a.com".into(), 100, 500);
            monitor.record_success(success).await;
        }

        let error = CrawlError::Transport {
            url: "https://b.com".into(),
            message: "error".into(),
            category: CrawlFailureCategory::Network,
        };
        let failure = CrawlResult::failure("source-2".into(), "https://b.com".into(), &error);
        monitor.record_failure(failure).await;

        let healthy = monitor.get_sources_by_status(HealthStatus::Healthy).await;
        assert!(healthy.contains(&"source-1".to_string()));
    }

    #[tokio::test]
    async fn test_statistics() {
        let monitor = create_monitor();

        let success = CrawlResult::success("source-1".into(), "https://a.com".into(), 100, 500);
        monitor.record_success(success).await;

        let stats = monitor.get_statistics().await;
        assert_eq!(stats.total_sources, 1);
    }

    #[tokio::test]
    async fn test_new_source_discovery() {
        let monitor = create_monitor();

        let suggestion = NewSourceSuggestion {
            url: "https://newsource.com/feed".into(),
            source_type: SourceType::News,
            priority: 2,
            region: Some(Region::Europe),
            category: Some(Category::News),
            discovered_via: DiscoveryMethod::AutomatedScan,
            confidence: 0.8,
        };

        monitor.suggest_source(suggestion).await;

        let discoveries = monitor.get_discoveries().await;
        assert_eq!(discoveries.len(), 1);
        assert_eq!(discoveries[0].url, "https://newsource.com/feed");
    }

    #[test]
    fn test_config_validation() {
        let valid = HealthConfig::default();
        assert!(valid.validate().is_empty());

        let invalid = HealthConfig {
            failure_threshold_for_dead: 2,
            failure_threshold_for_unhealthy: 5,
            ..Default::default()
        };
        assert!(!invalid.validate().is_empty());
    }

    #[tokio::test]
    async fn test_discovery_stats() {
        let monitor = create_monitor();

        let suggestion = NewSourceSuggestion {
            url: "https://test.com".into(),
            source_type: SourceType::News,
            priority: 2,
            discovered_via: DiscoveryMethod::Manual,
            confidence: 0.7,
            ..Default::default()
        };

        monitor.suggest_source(suggestion).await;

        let stats = monitor.get_discovery_stats().await;
        assert_eq!(stats.total_discoveries, 1);
        assert!((stats.avg_confidence - 0.7).abs() < 0.01);
    }

    #[test]
    fn test_crawl_result_factory() {
        let success = CrawlResult::success("test".into(), "https://test.com".into(), 100, 500);
        assert!(success.success);
        assert_eq!(success.status_code, Some(200));

        let error = CrawlError::HttpStatus {
            url: "https://test.com".into(),
            status: 500,
            retry_after_secs: None,
            body_excerpt: None,
        };
        let failure = CrawlResult::failure("test".into(), "https://test.com".into(), &error);
        assert!(!failure.success);
        assert_eq!(failure.status_code, Some(500));
    }

    #[test]
    fn test_crawl_error_is_critical() {
        let timeout = CrawlError::Transport {
            url: "https://test.com".into(),
            message: "timeout".into(),
            category: CrawlFailureCategory::Timeout,
        };
        assert!(timeout.is_critical());

        let server_error = CrawlError::HttpStatus {
            url: "https://test.com".into(),
            status: 503,
            retry_after_secs: None,
            body_excerpt: None,
        };
        assert!(server_error.is_critical());

        let not_found = CrawlError::HttpStatus {
            url: "https://test.com".into(),
            status: 404,
            retry_after_secs: None,
            body_excerpt: None,
        };
        assert!(!not_found.is_critical());
    }
}
