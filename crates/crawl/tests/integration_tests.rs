//! Integration tests for the crawl crate
//!
//! These tests verify the interaction between different crawl components.

use std::time::Duration;

use apex_crawl::{
    CachedContent, CircuitState, ContentCache, CrawlError, CrawlFailureCategory, HealthConfig,
    HealthStatus, RetryConfig, RetryEngine, SourceHealthMonitor,
};

// ─────────────────────────────────────────────────────────────────────────────
// Retry Engine Integration Tests
// ─────────────────────────────────────────────────────────────────────────────

#[allow(clippy::unwrap_used, clippy::expect_used)]
mod retry_engine_tests {
    use super::*;

    #[tokio::test]
    async fn test_retry_engine_caches_successful_content() {
        let engine = RetryEngine::with_default_config();

        // Cache some content
        engine
            .cache_content(
                "https://example.com".to_string(),
                "cached content".to_string(),
                Some("text/html".to_string()),
                200,
                None,
            )
            .await;

        let cached = engine.get_cached("https://example.com").await;
        assert!(cached.is_some());
        assert_eq!(cached.unwrap().content, "cached content");
    }

    #[tokio::test]
    async fn test_retry_engine_circuit_breaker_tracking() {
        let engine = RetryEngine::with_default_config();

        // Record failures
        for _ in 0..5 {
            engine.record_failure("unstable.example.com").await;
        }

        // Circuit should now be open
        let state = engine.circuit_state("unstable.example.com").await;
        assert_eq!(state, Some(CircuitState::Open));

        // Domain should not be available
        assert!(!engine.is_domain_available("unstable.example.com").await);
    }

    #[tokio::test]
    async fn test_retry_engine_circuit_recovery() {
        let engine = RetryEngine::with_default_config();

        // Trigger circuit breaker
        for _ in 0..5 {
            engine.record_failure("recovering.example.com").await;
        }

        // Reset the circuit
        engine.reset_circuit("recovering.example.com").await;

        // Should be closed now
        let state = engine.circuit_state("recovering.example.com").await;
        assert_eq!(state, Some(CircuitState::Closed));
    }

    #[test]
    fn test_retry_config_defaults() {
        let config = RetryConfig::default();
        assert_eq!(config.max_retries, 5);
        assert_eq!(config.jitter_factor, 0.3);
        assert!(config.validate().is_empty());
    }

    #[test]
    fn test_retry_config_custom_validation() {
        let config = RetryConfig {
            base_delay: Duration::ZERO,
            max_delay: Duration::from_secs(60),
            max_retries: 3,
            jitter_factor: 0.5,
            exponential_base: 2.0,
        };
        assert!(!config.validate().is_empty());
    }

    #[test]
    fn test_exponential_delay_growth() {
        let engine = RetryEngine::with_default_config();

        let delay_0 = engine.calculate_delay(0);
        let delay_1 = engine.calculate_delay(1);
        let delay_2 = engine.calculate_delay(2);
        let delay_3 = engine.calculate_delay(3);

        // Verify exponential growth
        assert!(delay_1 > delay_0);
        assert!(delay_2 > delay_1);
        assert!(delay_3 > delay_2);

        // Verify delay is capped at max_delay
        let config = RetryConfig::default();
        for attempt in 0..20 {
            let delay = engine.calculate_delay(attempt);
            assert!(delay <= config.max_delay);
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Content Cache Integration Tests
// ─────────────────────────────────────────────────────────────────────────────

#[allow(clippy::unwrap_used, clippy::expect_used)]
mod content_cache_tests {
    use super::*;

    #[test]
    fn test_cache_lru_eviction() {
        let mut cache = ContentCache::new(3, Duration::from_secs(3600));

        cache.put(
            "https://a.com",
            CachedContent {
                url: "https://a.com".to_string(),
                content: "A".to_string(),
                content_type: None,
                cached_at: Duration::ZERO,
                max_age: Duration::from_secs(3600),
                status: 200,
            },
        );
        cache.put(
            "https://b.com",
            CachedContent {
                url: "https://b.com".to_string(),
                content: "B".to_string(),
                content_type: None,
                cached_at: Duration::from_secs(1),
                max_age: Duration::from_secs(3600),
                status: 200,
            },
        );
        cache.put(
            "https://c.com",
            CachedContent {
                url: "https://c.com".to_string(),
                content: "C".to_string(),
                content_type: None,
                cached_at: Duration::from_secs(2),
                max_age: Duration::from_secs(3600),
                status: 200,
            },
        );

        // Add a 4th entry - should evict oldest
        cache.put(
            "https://d.com",
            CachedContent {
                url: "https://d.com".to_string(),
                content: "D".to_string(),
                content_type: None,
                cached_at: Duration::from_secs(3),
                max_age: Duration::from_secs(3600),
                status: 200,
            },
        );

        // "a.com" should be evicted
        assert!(cache.get("https://a.com").is_none());
        assert!(cache.get("https://b.com").is_some());
        assert!(cache.get("https://c.com").is_some());
        assert!(cache.get("https://d.com").is_some());
    }

    #[test]
    fn test_cache_domain_invalidation() {
        let mut cache = ContentCache::new(10, Duration::from_secs(3600));

        cache.put(
            "https://example.com/page1",
            CachedContent {
                url: "https://example.com/page1".to_string(),
                content: "Page 1".to_string(),
                content_type: None,
                cached_at: Duration::ZERO,
                max_age: Duration::from_secs(3600),
                status: 200,
            },
        );
        cache.put(
            "https://example.com/page2",
            CachedContent {
                url: "https://example.com/page2".to_string(),
                content: "Page 2".to_string(),
                content_type: None,
                cached_at: Duration::ZERO,
                max_age: Duration::from_secs(3600),
                status: 200,
            },
        );
        cache.put(
            "https://other.com/page1",
            CachedContent {
                url: "https://other.com/page1".to_string(),
                content: "Other".to_string(),
                content_type: None,
                cached_at: Duration::ZERO,
                max_age: Duration::from_secs(3600),
                status: 200,
            },
        );

        // Invalidate all of example.com
        cache.invalidate_domain("example.com");

        assert!(cache.get("https://example.com/page1").is_none());
        assert!(cache.get("https://example.com/page2").is_none());
        assert!(cache.get("https://other.com/page1").is_some());
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Source Health Monitor Integration Tests
// ─────────────────────────────────────────────────────────────────────────────

#[allow(clippy::unwrap_used, clippy::expect_used)]
mod source_health_tests {
    use super::*;

    #[tokio::test]
    async fn test_health_monitor_tracks_success() {
        let monitor = SourceHealthMonitor::new(HealthConfig::default());

        let result = apex_crawl::source_health::CrawlResult::success(
            "test-source".to_string(),
            "https://example.com".to_string(),
            500,
            1000,
        );
        monitor.record_success(result).await;

        let metrics = monitor.get_metrics("test-source").await;
        assert!(metrics.is_some());

        let m = metrics.unwrap();
        assert_eq!(m.consecutive_successes, 1);
        assert_eq!(m.consecutive_failures, 0);
        assert!(m.last_success.is_some());
    }

    #[tokio::test]
    async fn test_health_monitor_tracks_failure() {
        let monitor = SourceHealthMonitor::new(HealthConfig::default());

        let error = CrawlError::Transport {
            url: "https://example.com".to_string(),
            message: "connection timeout".to_string(),
            category: CrawlFailureCategory::Timeout,
        };
        let result = apex_crawl::source_health::CrawlResult::failure(
            "failing-source".to_string(),
            "https://example.com".to_string(),
            &error,
        );
        monitor.record_failure(result).await;

        let metrics = monitor.get_metrics("failing-source").await;
        assert!(metrics.is_some());

        let m = metrics.unwrap();
        assert_eq!(m.consecutive_failures, 1);
        assert!(m.last_failure.is_some());
    }

    #[tokio::test]
    async fn test_health_status_from_score() {
        assert_eq!(HealthStatus::from_score(0.9), HealthStatus::Healthy);
        assert_eq!(HealthStatus::from_score(0.8), HealthStatus::Healthy);
        assert_eq!(HealthStatus::from_score(0.7), HealthStatus::Degraded);
        assert_eq!(HealthStatus::from_score(0.6), HealthStatus::Degraded);
        assert_eq!(HealthStatus::from_score(0.5), HealthStatus::Unhealthy);
        assert_eq!(HealthStatus::from_score(0.4), HealthStatus::Unhealthy);
        assert_eq!(HealthStatus::from_score(0.2), HealthStatus::Dead);
        assert_eq!(HealthStatus::from_score(0.0), HealthStatus::Dead);
    }

    #[tokio::test]
    async fn test_health_statistics() {
        let config = HealthConfig::default();
        let min_attempts = config.min_attempts_for_score;
        let monitor = SourceHealthMonitor::new(config);

        // A source only earns a real health score after `min_attempts_for_score`
        // crawls; below that threshold it stays neutral. Record enough sustained
        // successes for the source to be classified Healthy.
        for _ in 0..min_attempts {
            let result = apex_crawl::source_health::CrawlResult::success(
                "healthy-source".to_string(),
                "https://example.com".to_string(),
                100,
                500,
            );
            monitor.record_success(result).await;
        }

        let stats = monitor.get_statistics().await;
        assert_eq!(stats.total_sources, 1);
        assert_eq!(stats.healthy, 1);
    }

    #[tokio::test]
    async fn test_new_source_discovery() {
        let monitor = SourceHealthMonitor::new(HealthConfig::default());

        let suggestion = apex_crawl::source_health::NewSourceSuggestion {
            url: "https://newsource.com/rss".to_string(),
            source_type: apex_crawl::source_health::SourceType::News,
            priority: 2,
            region: Some(apex_crawl::sources_registry::Region::Europe),
            category: Some(apex_crawl::sources_registry::Category::News),
            discovered_via: apex_crawl::source_health::DiscoveryMethod::AutomatedScan,
            confidence: 0.85,
        };

        monitor.suggest_source(suggestion).await;

        let discoveries = monitor.get_discoveries().await;
        assert_eq!(discoveries.len(), 1);
        assert_eq!(discoveries[0].url, "https://newsource.com/rss");
    }

    #[test]
    fn test_health_config_validation() {
        let valid = HealthConfig::default();
        assert!(valid.validate().is_empty());

        let invalid = HealthConfig {
            failure_threshold_for_dead: 2,
            failure_threshold_for_unhealthy: 5,
            ..Default::default()
        };
        let errors = invalid.validate();
        assert!(!errors.is_empty());
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Error Handling Integration Tests
// ─────────────────────────────────────────────────────────────────────────────

#[allow(clippy::unwrap_used, clippy::expect_used)]
mod error_handling_tests {
    use super::*;

    #[test]
    fn test_error_criticality_classification() {
        // Network errors are critical
        let network_error = CrawlError::Transport {
            url: "https://example.com".to_string(),
            message: "connection refused".to_string(),
            category: CrawlFailureCategory::Network,
        };
        assert!(network_error.is_critical());

        // Timeout errors are critical
        let timeout_error = CrawlError::Transport {
            url: "https://example.com".to_string(),
            message: "timed out".to_string(),
            category: CrawlFailureCategory::Timeout,
        };
        assert!(timeout_error.is_critical());

        // 5xx server errors are critical
        let server_error = CrawlError::HttpStatus {
            url: "https://example.com".to_string(),
            status: 503,
            retry_after_secs: None,
            body_excerpt: None,
        };
        assert!(server_error.is_critical());

        // 4xx client errors are not critical
        let client_error = CrawlError::HttpStatus {
            url: "https://example.com".to_string(),
            status: 404,
            retry_after_secs: None,
            body_excerpt: None,
        };
        assert!(!client_error.is_critical());

        // Robots denied is not critical
        let robots_error = CrawlError::RobotsDenied {
            url: "https://example.com".to_string(),
            user_agent: "ApexIntelBot".to_string(),
        };
        assert!(!robots_error.is_critical());
    }

    #[test]
    fn test_error_retryability() {
        let network_error = CrawlError::Transport {
            url: "https://example.com".to_string(),
            message: "connection reset".to_string(),
            category: CrawlFailureCategory::Network,
        };
        assert!(network_error.is_retryable());

        let rate_limit_error = CrawlError::HttpStatus {
            url: "https://example.com".to_string(),
            status: 429,
            retry_after_secs: Some(60),
            body_excerpt: None,
        };
        assert!(rate_limit_error.is_retryable());

        let not_found_error = CrawlError::HttpStatus {
            url: "https://example.com".to_string(),
            status: 404,
            retry_after_secs: None,
            body_excerpt: None,
        };
        assert!(!not_found_error.is_retryable());
    }

    #[test]
    fn test_retry_after_extraction() {
        let error_with_retry = CrawlError::HttpStatus {
            url: "https://example.com".to_string(),
            status: 429,
            retry_after_secs: Some(120),
            body_excerpt: None,
        };
        assert_eq!(
            error_with_retry.retry_after(),
            Some(Duration::from_secs(120))
        );

        let error_without_retry = CrawlError::HttpStatus {
            url: "https://example.com".to_string(),
            status: 500,
            retry_after_secs: None,
            body_excerpt: None,
        };
        assert_eq!(error_without_retry.retry_after(), None);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// End-to-End Workflow Tests
// ─────────────────────────────────────────────────────────────────────────────

#[allow(clippy::unwrap_used, clippy::expect_used)]
mod e2e_workflow_tests {
    use super::*;

    #[tokio::test]
    async fn test_source_health_to_retirement_workflow() {
        let config = HealthConfig {
            failure_threshold_for_unhealthy: 2,
            failure_threshold_for_dead: 3,
            auto_retire_dead: true,
            retirement_delay_hours: 0, // Immediate for testing
            ..Default::default()
        };

        let monitor = SourceHealthMonitor::new(config);

        // Record failures until unhealthy
        for _ in 0..2 {
            let error = CrawlError::Transport {
                url: "https://dying.com".to_string(),
                message: "timeout".to_string(),
                category: CrawlFailureCategory::Timeout,
            };
            let result = apex_crawl::source_health::CrawlResult::failure(
                "dying-source".to_string(),
                "https://dying.com".to_string(),
                &error,
            );
            monitor.record_failure(result).await;
        }

        // Should be unhealthy
        let status = monitor.get_status("dying-source").await;
        assert_eq!(status, Some(HealthStatus::Unhealthy));

        // Record one more failure to trigger dead status
        let error = CrawlError::Transport {
            url: "https://dying.com".to_string(),
            message: "timeout".to_string(),
            category: CrawlFailureCategory::Timeout,
        };
        let result = apex_crawl::source_health::CrawlResult::failure(
            "dying-source".to_string(),
            "https://dying.com".to_string(),
            &error,
        );
        monitor.record_failure(result).await;

        // Process retirements
        let retirements = monitor.process_retirements().await;
        assert!(retirements.contains(&"dying-source".to_string()));

        // Source should be in retired list
        let retired = monitor.get_retired_sources().await;
        assert!(retired.iter().any(|r| r.source_id == "dying-source"));
    }

    #[tokio::test]
    async fn test_source_revive_workflow() {
        let config = HealthConfig::default();
        let monitor = SourceHealthMonitor::new(config);

        // Record a failure first
        let error = CrawlError::Transport {
            url: "https://example.com".to_string(),
            message: "timeout".to_string(),
            category: CrawlFailureCategory::Timeout,
        };
        let result = apex_crawl::source_health::CrawlResult::failure(
            "revival-source".to_string(),
            "https://example.com".to_string(),
            &error,
        );
        monitor.record_failure(result).await;

        // Revive the source
        monitor.revive_source("revival-source").await;

        // Status should be reset
        let status = monitor.get_status("revival-source").await;
        assert_eq!(status, Some(HealthStatus::Unknown));
    }

    #[tokio::test]
    async fn test_retry_engine_with_circuit_breaker() {
        let engine = RetryEngine::with_default_config();

        // Trigger circuit breaker
        for _ in 0..5 {
            engine.record_failure("troubled-domain.com").await;
        }

        // Try to execute - should get circuit breaker error
        let result = engine
            .execute_with_retry("troubled-domain.com", || async {
                Err(CrawlError::Transport {
                    url: "https://troubled-domain.com".to_string(),
                    message: "still failing".to_string(),
                    category: CrawlFailureCategory::Network,
                })
            })
            .await;

        // The circuit is open, but after the timeout we should still be able to get errors
        // This tests the retry logic integration
        assert!(result.is_err());
    }
}
