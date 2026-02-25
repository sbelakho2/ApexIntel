use std::collections::HashMap;
use std::time::{Duration, Instant};
use serde::{Deserialize, Serialize};

const BASE_DELAY_SECS: f64 = 3.0;
const MIN_DELAY_SECS: f64 = 0.5;
const MAX_DELAY_SECS: f64 = 60.0;
const MAX_COOLDOWN_SECS: f64 = 1800.0; // 30 min
const FAILURE_THRESHOLD: f64 = 3.0;
const SUCCESS_STREAK_RESET: u32 = 2;
const JITTER_FACTOR: f64 = 0.3;

/// Per-engine rate limit state.
#[derive(Debug, Clone)]
pub struct EngineState {
    pub failures: f64,
    pub last_failure: Option<Instant>,
    pub last_activity: Instant,
    pub backoff: Duration,
    pub success_streak: u32,
}

impl Default for EngineState {
    fn default() -> Self {
        Self {
            failures: 0.0,
            last_failure: None,
            last_activity: Instant::now(),
            backoff: Duration::ZERO,
            success_streak: 0,
        }
    }
}

/// Serialisable snapshot for persistence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineStateSnapshot {
    pub failures: f64,
    pub backoff_millis: u64,
    pub success_streak: u32,
}

/// Manages per-engine rate limiting with backoff and jitter.
///
/// # Thread-safety (B281)
///
/// `RateLimitManager` is **not** `Sync` — mutation methods (`record_success`,
/// `record_failure`, `evict_stale`) take `&mut self`.  For shared access from
/// concurrent crawler tasks, wrap in `Arc<tokio::sync::Mutex<RateLimitManager>>`.
///
/// Prefer a per-task-owned instance where only one task drives a given engine
/// shard; this avoids lock contention on hot crawl loops.
pub struct RateLimitManager {
    engine_states: HashMap<String, EngineState>,
    /// Maximum number of tracked engines before evicting stale entries.
    max_tracked: usize,
}

impl RateLimitManager {
    pub fn new() -> Self {
        Self {
            engine_states: HashMap::new(),
            max_tracked: 10_000,
        }
    }

    /// Evict healthy engines that haven't been active for > 1 hour.
    pub fn evict_stale(&mut self) {
        let stale_threshold = Duration::from_secs(3600);
        self.engine_states.retain(|_, state| {
            state.last_activity.elapsed() < stale_threshold || state.failures > 0.0
        });
    }

    pub fn record_success(&mut self, engine: &str) {
        if self.engine_states.len() >= self.max_tracked && !self.engine_states.contains_key(engine) {
            self.evict_stale();
        }
        let state = self.engine_states.entry(engine.to_string()).or_default();
        state.last_activity = Instant::now();
        state.success_streak += 1;
        if state.success_streak >= SUCCESS_STREAK_RESET {
            state.failures = 0.0;
            state.backoff = Duration::ZERO;
        }
    }

    pub fn record_failure(&mut self, engine: &str, is_soft: bool) {
        let state = self.engine_states.entry(engine.to_string()).or_default();
        state.last_activity = Instant::now();
        state.success_streak = 0;
        state.last_failure = Some(Instant::now());
        state.failures += if is_soft { 0.5 } else { 1.0 };

        if state.failures >= FAILURE_THRESHOLD {
            let multiplier = 2f64.powf((state.failures - FAILURE_THRESHOLD).min(6.0));
            let backoff_secs = (BASE_DELAY_SECS * multiplier * 10.0).min(MAX_COOLDOWN_SECS);
            state.backoff = Duration::from_secs_f64(backoff_secs);
        }
    }

    pub fn is_available(&self, engine: &str) -> bool {
        if let Some(state) = self.engine_states.get(engine) {
            if let Some(last_failure) = state.last_failure {
                if last_failure.elapsed() < state.backoff {
                    return false;
                }
            }
        }
        true
    }

    pub fn get_recommended_delay(&self, engine: &str) -> Duration {
        let state = self.engine_states.get(engine);
        let mut delay = BASE_DELAY_SECS;

        if let Some(s) = state {
            if s.failures > 0.0 {
                delay *= 1.0 + s.failures * 0.5;
            }
        }
        delay = delay.clamp(MIN_DELAY_SECS, MAX_DELAY_SECS);

        // Deterministic jitter based on engine name hash for testability
        let jitter_seed: f64 = engine.bytes().map(|b| b as f64).sum::<f64>() % 100.0 / 100.0;
        let jitter = delay * JITTER_FACTOR * jitter_seed;
        Duration::from_secs_f64(delay + jitter)
    }

    /// Parse Retry-After header value (seconds or HTTP-date) into a delay.
    pub fn retry_after_delay(header_value: &str, now: chrono::DateTime<chrono::Utc>) -> Option<Duration> {
        let trimmed = header_value.trim();
        if let Ok(secs) = trimmed.parse::<u64>() {
            return Some(Duration::from_secs(secs));
        }
        if let Ok(dt) = chrono::DateTime::parse_from_rfc2822(trimmed) {
            let utc = dt.with_timezone(&chrono::Utc);
            if utc > now {
                return Some(Duration::from_secs((utc - now).num_seconds() as u64));
            }
        }
        None
    }

    pub fn health_score(&self, engine: &str) -> u8 {
        let state = self.engine_states.get(engine);
        let mut score: i32 = 100;
        if let Some(s) = state {
            score -= (s.failures * 15.0).min(60.0) as i32;
            score += (s.success_streak.min(2) as i32) * 10;
            if s.backoff > Duration::ZERO {
                score -= 30;
            }
        }
        score.clamp(0, 100) as u8
    }

    pub fn get_engines_by_health(&self, engines: &[String]) -> Vec<String> {
        let mut available: Vec<_> = engines
            .iter()
            .filter(|e| self.is_available(e.as_str()))
            .cloned()
            .collect();
        available.sort_by(|a, b| self.health_score(b.as_str()).cmp(&self.health_score(a.as_str())));
        available
    }

    pub fn snapshot(&self) -> HashMap<String, EngineStateSnapshot> {
        self.engine_states
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    EngineStateSnapshot {
                        failures: v.failures,
                        backoff_millis: v.backoff.as_millis() as u64,
                        success_streak: v.success_streak,
                    },
                )
            })
            .collect()
    }
}

impl Default for RateLimitManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_engine_is_available() {
        let mgr = RateLimitManager::new();
        assert!(mgr.is_available("google"));
    }

    #[test]
    fn test_health_score_fresh() {
        let mgr = RateLimitManager::new();
        assert_eq!(mgr.health_score("google"), 100);
    }

    #[test]
    fn test_record_success_resets_failures() {
        let mut mgr = RateLimitManager::new();
        mgr.record_failure("google", false);
        mgr.record_failure("google", false);
        // two successes should reset
        mgr.record_success("google");
        mgr.record_success("google");
        assert_eq!(mgr.engine_states["google"].failures, 0.0);
    }

    #[test]
    fn test_soft_failure_counts_half() {
        let mut mgr = RateLimitManager::new();
        mgr.record_failure("bing", true);
        assert!((mgr.engine_states["bing"].failures - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_backoff_after_threshold() {
        let mut mgr = RateLimitManager::new();
        for _ in 0..4 {
            mgr.record_failure("google", false);
        }
        let state = &mgr.engine_states["google"];
        assert!(state.backoff > Duration::ZERO);
    }

    #[test]
    fn test_unavailable_during_backoff() {
        let mut mgr = RateLimitManager::new();
        for _ in 0..5 {
            mgr.record_failure("google", false);
        }
        // Should be unavailable because backoff is > 0 and last_failure was just now
        assert!(!mgr.is_available("google"));
    }

    #[test]
    fn test_recommended_delay_increases_with_failures() {
        let mut mgr = RateLimitManager::new();
        let delay_fresh = mgr.get_recommended_delay("google");
        mgr.record_failure("google", false);
        mgr.record_failure("google", false);
        let delay_after = mgr.get_recommended_delay("google");
        assert!(delay_after > delay_fresh);
    }

    #[test]
    fn test_recommended_delay_jitter_is_deterministic_per_engine() {
        let mgr = RateLimitManager::new();
        let d1 = mgr.get_recommended_delay("engine-a");
        let d2 = mgr.get_recommended_delay("engine-a");
        assert_eq!(d1, d2);
    }

    #[test]
    fn test_recommended_delay_jitter_varies_between_engines() {
        let mgr = RateLimitManager::new();
        let d1 = mgr.get_recommended_delay("engine-a");
        let d2 = mgr.get_recommended_delay("engine-b");
        assert_ne!(d1, d2);
    }

    #[test]
    fn test_health_score_decreases_with_failures() {
        let mut mgr = RateLimitManager::new();
        let score_before = mgr.health_score("google");
        mgr.record_failure("google", false);
        mgr.record_failure("google", false);
        let score_after = mgr.health_score("google");
        assert!(score_after < score_before);
    }

    #[test]
    fn test_engines_by_health_sorting() {
        let mut mgr = RateLimitManager::new();
        mgr.record_failure("bad_engine", false);
        mgr.record_failure("bad_engine", false);
        mgr.record_failure("bad_engine", false);
        mgr.record_success("good_engine");

        let engines = vec!["bad_engine".to_string(), "good_engine".to_string()];
        let sorted = mgr.get_engines_by_health(&engines);
        assert_eq!(sorted[0], "good_engine");
    }

    #[test]
    fn test_snapshot() {
        let mut mgr = RateLimitManager::new();
        mgr.record_failure("google", false);
        let snap = mgr.snapshot();
        assert!(snap.contains_key("google"));
        assert!((snap["google"].failures - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_retry_after_seconds() {
        let now = chrono::Utc::now();
        let delay = RateLimitManager::retry_after_delay("120", now).unwrap();
        assert_eq!(delay.as_secs(), 120);
    }

    #[test]
    fn test_retry_after_http_date_future() {
        let now = chrono::DateTime::parse_from_rfc2822("Wed, 21 Oct 2015 07:27:00 GMT")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let delay = RateLimitManager::retry_after_delay("Wed, 21 Oct 2015 07:28:00 GMT", now)
            .unwrap();
        assert_eq!(delay.as_secs(), 60);
    }
}
