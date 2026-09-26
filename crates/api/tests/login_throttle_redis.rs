//! Opt-in integration test for the Redis login-throttle backend.
//!
//! `#[ignore]`d by default; reads `TEST_REDIS_URL` or `REDIS_URL` and proves
//! the Lua read-modify-write is atomic and shared across throttle instances.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use apex_api::login_throttle::{LoginThrottle, LoginThrottleBackend};
use chrono::Duration;

fn redis_url() -> String {
    std::env::var("TEST_REDIS_URL")
        .or_else(|_| std::env::var("REDIS_URL"))
        .expect("TEST_REDIS_URL or REDIS_URL must be set")
}

#[tokio::test]
#[ignore = "requires Redis; run with --ignored"]
async fn redis_login_throttle_is_shared_and_clears_on_success() {
    let client = redis::Client::open(redis_url()).expect("parse redis url");
    let connection = client
        .get_connection_manager()
        .await
        .expect("connect to redis");

    let first = LoginThrottle::new(None, Some(connection.clone()));
    let second = LoginThrottle::new(None, Some(connection));
    assert_eq!(first.backend(), LoginThrottleBackend::Redis);
    assert!(first.is_durable());

    let key = format!("redis-it-{}", uuid::Uuid::new_v4());
    let now = chrono::Utc::now();

    let backoff = first.record_failure(&key, now).await;
    assert!(!backoff.allowed);
    assert_eq!(backoff.retry_after_secs, 1);

    for offset in 1..10 {
        first
            .record_failure(&key, now + Duration::seconds(offset))
            .await;
    }
    let locked = second.evaluate(&key, now + Duration::seconds(10)).await;
    assert!(!locked.allowed, "the lockout must be shared through Redis");
    assert!(locked.retry_after_secs >= 590);
    assert_eq!(locked.failure_count_10m, 10);

    first.record_success(&key).await;
    let cleared = second.evaluate(&key, now + Duration::seconds(11)).await;
    assert!(cleared.allowed);
    assert_eq!(cleared.failure_count_10m, 0);
}

#[tokio::test]
#[ignore = "requires Redis; run with --ignored"]
async fn redis_login_throttle_requires_admin_unlock() {
    let client = redis::Client::open(redis_url()).expect("parse redis url");
    let connection = client
        .get_connection_manager()
        .await
        .expect("connect to redis");
    let throttle = LoginThrottle::new(None, Some(connection));
    let key = format!("redis-admin-it-{}", uuid::Uuid::new_v4());
    let now = chrono::Utc::now();

    for offset in 0..20 {
        throttle
            .record_failure(&key, now + Duration::minutes(offset))
            .await;
    }
    let locked = throttle.evaluate(&key, now + Duration::minutes(20)).await;
    assert!(locked.admin_unlock_required);
    assert!(!locked.allowed);

    assert!(throttle.clear_lock(&key).await);
    assert!(
        throttle
            .evaluate(&key, now + Duration::minutes(21))
            .await
            .allowed
    );
}
