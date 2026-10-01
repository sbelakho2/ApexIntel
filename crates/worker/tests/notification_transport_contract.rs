//! Pure transport-contract tests for the durable notification router and the
//! Slack webhook client. No database, no NATS, no SMTP: a loopback TCP
//! receiver stands in for the remote endpoint.
//!
//! Covers audit #70 (fresh webhook URL resolution), #72 (per-format bodies),
//! #73 (no URL in transport errors) and #77 (publish targets, permanent 4xx,
//! Retry-After).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use apex_core::alert_config::AlertScope;
use apex_worker::notification_delivery::{
    ChannelTransport, ConfiguredChannelRouter, NotificationDelivery, NotificationDeliveryPayload,
};
use apex_worker::notifications::{
    AlertSeverity, NotificationConfig, PendingAlert, WebhookConfig, WebhookFormat,
};
use apex_worker::slack::{
    AlertType, SlackConfig, SlackMessage, SlackMessageSeverity, SlackWebhook,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// One captured HTTP request.
#[derive(Debug, Clone)]
struct CapturedRequest {
    head: String,
    path: String,
    body: String,
}

/// Read one complete HTTP/1.1 request (headers + content-length body).
async fn read_request(socket: &mut TcpStream) -> Option<String> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let read = socket.read(&mut chunk).await.ok()?;
        if read == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..read]);
        if let Some(headers_end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..headers_end]).to_string();
            let content_length = head
                .lines()
                .find_map(|line| {
                    let (key, value) = line.split_once(':')?;
                    key.trim()
                        .eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().unwrap_or(0))
                })
                .unwrap_or(0);
            if buf.len() >= headers_end + 4 + content_length {
                break;
            }
        }
    }
    Some(String::from_utf8_lossy(&buf).to_string())
}

/// Spawn a minimal HTTP receiver that answers the `index`-th request with
/// `responses[index]` (the last response repeats when more requests arrive).
async fn spawn_receiver(responses: Vec<String>) -> (String, Arc<Mutex<Vec<CapturedRequest>>>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind receiver");
    let addr = listener.local_addr().expect("receiver addr");
    let captured: Arc<Mutex<Vec<CapturedRequest>>> = Arc::new(Mutex::new(Vec::new()));
    let captured_task = Arc::clone(&captured);
    let responses = Arc::new(responses);
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            let captured = Arc::clone(&captured_task);
            let responses = Arc::clone(&responses);
            tokio::spawn(async move {
                let Some(raw) = read_request(&mut socket).await else {
                    return;
                };
                let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((raw.as_str(), ""));
                let path = head
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/")
                    .to_string();
                let index = {
                    let mut captured = captured.lock().unwrap();
                    let index = captured.len();
                    captured.push(CapturedRequest {
                        head: head.to_string(),
                        path,
                        body: body.to_string(),
                    });
                    index
                };
                let response = responses
                    .get(index)
                    .or_else(|| responses.last())
                    .cloned()
                    .unwrap_or_else(ok_response);
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.shutdown().await;
            });
        }
    });
    (format!("http://{addr}"), captured)
}

fn ok_response() -> String {
    "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string()
}

fn not_found_response() -> String {
    "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string()
}

fn too_many_requests_with_retry_after(secs: u64) -> String {
    format!(
        "HTTP/1.1 429 Too Many Requests\r\nRetry-After: {secs}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    )
}

fn test_alert(source: &str) -> PendingAlert {
    let mut alert = PendingAlert::new(
        source,
        AlertScope::entity("entity-1", Some("Acme & Sons <https://evil.example|Co>")),
        "Breach <!channel>",
        "Body <https://evil.example|Click> & more",
        AlertSeverity::Critical,
        0.95,
    );
    alert.category = "warning".to_string();
    alert.region = Some("<https://evil.example|Region>".to_string());
    alert
}

fn delivery_for(channel: &str, destination: &str, alert: PendingAlert) -> NotificationDelivery {
    NotificationDelivery {
        delivery_key: format!("test:{channel}"),
        notification_event_id: None,
        channel: channel.to_string(),
        destination: destination.to_string(),
        attempts: 1,
        idempotency_key: "stable-idempotency-key".to_string(),
        payload: NotificationDeliveryPayload {
            alert,
            subject: None,
            body: String::new(),
        },
    }
}

fn router_with(webhooks: Vec<WebhookConfig>) -> ConfiguredChannelRouter {
    ConfiguredChannelRouter::new(NotificationConfig {
        webhooks,
        email: None,
        log_min_severity: AlertSeverity::Low,
    })
    .expect("router builds")
}

// ── #70: freshly resolved webhook URL ───────────────────────────────────────

#[tokio::test]
async fn router_posts_to_the_freshly_resolved_webhook_url_not_the_stored_destination() {
    let (base, received) = spawn_receiver(vec![ok_response()]).await;
    let router = router_with(vec![WebhookConfig {
        name: "slack".to_string(),
        url: format!("{base}/freshly-resolved"),
        bearer_token: None,
        format: WebhookFormat::Slack,
        min_severity: AlertSeverity::Low,
        min_priority: 0.0,
    }]);

    // The stored destination is deliberately a stale URL: the router must
    // resolve the channel name against the CURRENT config and post there.
    let delivery = delivery_for(
        "slack",
        "https://stale.invalid/rotated-secret-path",
        test_alert("fresh-url:w1"),
    );
    router
        .deliver(&delivery)
        .await
        .expect("the receiver accepts the delivery");

    let requests = received.lock().unwrap();
    assert_eq!(
        requests.len(),
        1,
        "the resolved webhook URL must be posted to"
    );
    assert_eq!(requests[0].path, "/freshly-resolved");
    assert!(
        !requests[0].head.contains("stale.invalid"),
        "the stored destination must never be contacted: {}",
        requests[0].head
    );
    let body: serde_json::Value = serde_json::from_str(&requests[0].body).expect("body is JSON");
    assert!(body.get("text").is_some(), "configured Slack body is sent");
}

// ── #72: per-format bodies ──────────────────────────────────────────────────

#[tokio::test]
async fn webhook_formats_render_slack_teams_and_generic_json_bodies() {
    let (slack_base, slack_received) = spawn_receiver(vec![ok_response()]).await;
    let (teams_base, teams_received) = spawn_receiver(vec![ok_response()]).await;
    let (json_base, json_received) = spawn_receiver(vec![ok_response()]).await;

    let router = router_with(vec![
        WebhookConfig {
            name: "slack".to_string(),
            url: format!("{slack_base}/slack"),
            bearer_token: None,
            format: WebhookFormat::Slack,
            min_severity: AlertSeverity::Low,
            min_priority: 0.0,
        },
        WebhookConfig {
            name: "teams".to_string(),
            url: format!("{teams_base}/teams"),
            bearer_token: None,
            format: WebhookFormat::Teams,
            min_severity: AlertSeverity::Low,
            min_priority: 0.0,
        },
        WebhookConfig {
            name: "generic".to_string(),
            url: format!("{json_base}/json"),
            bearer_token: None,
            format: WebhookFormat::Json,
            min_severity: AlertSeverity::Low,
            min_priority: 0.0,
        },
    ]);

    for (channel, destination) in [
        ("slack", format!("{slack_base}/slack")),
        ("teams", format!("{teams_base}/teams")),
        ("generic", format!("{json_base}/json")),
    ] {
        router
            .deliver(&delivery_for(
                channel,
                &destination,
                test_alert(&format!("format:{channel}")),
            ))
            .await
            .expect("receiver accepts");
    }

    let slack: serde_json::Value =
        serde_json::from_str(&slack_received.lock().unwrap()[0].body).expect("slack JSON");
    assert!(
        slack.get("blocks").and_then(|b| b.as_array()).is_some(),
        "Slack format must render Block Kit blocks: {slack}"
    );
    assert!(
        slack.get("text").is_some(),
        "Slack format has fallback text"
    );

    let teams: serde_json::Value =
        serde_json::from_str(&teams_received.lock().unwrap()[0].body).expect("teams JSON");
    assert_eq!(teams["@type"], serde_json::json!("MessageCard"));
    assert!(teams["sections"].is_array(), "Teams MessageCard sections");
    assert!(teams.get("blocks").is_none(), "Teams is not Block Kit");

    let json: serde_json::Value =
        serde_json::from_str(&json_received.lock().unwrap()[0].body).expect("generic JSON");
    assert_eq!(json["title"], serde_json::json!("Breach <!channel>"));
    assert_eq!(json["severity"], serde_json::json!("critical"));
    assert!(json.get("description").is_some());
    assert!(json.get("entity").is_some());
    assert!(json.get("link").is_some());
    assert!(
        json.get("blocks").is_none(),
        "generic JSON is not Block Kit"
    );
}

// ── #73: transport errors never embed the credential URL ────────────────────

#[tokio::test]
async fn transport_error_message_never_contains_the_webhook_url() {
    // Reserve and drop a port so connecting is refused deterministically.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);

    let secret = "supersecret-webhook-token-9876";
    let url = format!("http://{addr}/{secret}");
    let router = router_with(vec![WebhookConfig {
        name: "slack".to_string(),
        url: url.clone(),
        bearer_token: Some("bearer-secret-should-not-log".to_string()),
        format: WebhookFormat::Slack,
        min_severity: AlertSeverity::Low,
        min_priority: 0.0,
    }]);

    let error = router
        .deliver(&delivery_for(
            "slack",
            "https://stale.invalid/rotated-secret-path",
            test_alert("error-redaction:w1"),
        ))
        .await
        .expect_err("a refused connection is a failure");
    let message = format!("{error:?} {error}");
    assert!(
        !message.contains(secret),
        "reqwest error leaked the webhook path: {message}"
    );
    assert!(
        !message.contains("http://"),
        "reqwest error leaked the webhook URL: {message}"
    );
    assert!(
        !message.contains("bearer-secret-should-not-log"),
        "an error must not leak the bearer token: {message}"
    );
}

// ── #77: Slack send routing, permanent 4xx, Retry-After ─────────────────────

#[tokio::test]
async fn slack_send_uses_explicit_channel_targets_and_defaults_only_as_fallback() {
    let (specific_base, specific_received) = spawn_receiver(vec![ok_response()]).await;
    let (default_base, default_received) = spawn_receiver(vec![ok_response()]).await;

    let mut webhooks = HashMap::new();
    webhooks.insert(
        "security".to_string(),
        vec![format!("{specific_base}/security")],
    );
    let config = SlackConfig {
        webhooks,
        default_urls: vec![format!("{default_base}/default")],
        timeout_secs: 10,
    };
    let webhook = SlackWebhook::new(&config).expect("webhook builds");

    let security = SlackMessage::new(
        SlackMessageSeverity::Critical,
        AlertType::Security,
        "Security alert",
        "Breach detected",
    );
    webhook
        .send(&security)
        .await
        .expect("security send succeeds");
    assert_eq!(
        specific_received.lock().unwrap().len(),
        1,
        "the channel-specific target receives the message"
    );
    assert_eq!(
        default_received.lock().unwrap().len(),
        0,
        "defaults must not be posted in addition to explicit targets"
    );

    // No explicit channel for `general`: the default URL is the fallback.
    let general = SlackMessage::new(
        SlackMessageSeverity::Info,
        AlertType::General,
        "General alert",
        "Nothing special",
    );
    webhook.send(&general).await.expect("general send succeeds");
    assert_eq!(
        default_received.lock().unwrap().len(),
        1,
        "defaults are used only when the channel has no explicit target"
    );
}

#[tokio::test]
async fn slack_send_does_not_retry_a_permanent_4xx() {
    let (base, received) = spawn_receiver(vec![not_found_response()]).await;
    let mut webhooks = HashMap::new();
    webhooks.insert("security".to_string(), vec![format!("{base}/revoked")]);
    let config = SlackConfig {
        webhooks,
        default_urls: Vec::new(),
        timeout_secs: 10,
    };
    let webhook = SlackWebhook::new(&config).expect("webhook builds");

    let message = SlackMessage::new(
        SlackMessageSeverity::Critical,
        AlertType::Security,
        "Security alert",
        "Breach detected",
    );
    let error = webhook
        .send(&message)
        .await
        .expect_err("HTTP 404 is a permanent failure");
    let rendered = format!("{error:?} {error}");
    assert!(
        !rendered.contains(&format!("{base}/revoked")),
        "the failed URL must be redacted in the error: {rendered}"
    );

    // Wait well past the first retry backoff: a permanent failure must not
    // produce a second attempt.
    tokio::time::sleep(Duration::from_millis(1_500)).await;
    assert_eq!(
        received.lock().unwrap().len(),
        1,
        "a permanent 4xx must not be retried"
    );
}

#[tokio::test]
async fn slack_send_honors_retry_after_on_429() {
    let (base, received) =
        spawn_receiver(vec![too_many_requests_with_retry_after(1), ok_response()]).await;
    let mut webhooks = HashMap::new();
    webhooks.insert("security".to_string(), vec![format!("{base}/limited")]);
    let config = SlackConfig {
        webhooks,
        default_urls: Vec::new(),
        timeout_secs: 10,
    };
    let webhook = SlackWebhook::new(&config).expect("webhook builds");

    let message = SlackMessage::new(
        SlackMessageSeverity::Critical,
        AlertType::Security,
        "Security alert",
        "Breach detected",
    );
    let started = Instant::now();
    webhook.send(&message).await.expect("the retry succeeds");
    let elapsed = started.elapsed();

    assert_eq!(
        received.lock().unwrap().len(),
        2,
        "the 429 is retried once and then succeeds"
    );
    assert!(
        elapsed >= Duration::from_millis(900),
        "Retry-After: 1 must be honored, elapsed was {elapsed:?}"
    );
}
