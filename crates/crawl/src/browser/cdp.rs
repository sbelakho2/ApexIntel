//! Minimal Chrome DevTools Protocol client used by the persistent renderer.
//!
//! Chrome exposes a WebSocket endpoint when launched with
//! `--remote-debugging-port=0`; this module owns the socket, multiplexes
//! command responses by id, and tracks per-session network activity so the
//! readiness loop can implement a `networkidle`-style quiet window.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot, Mutex as AsyncMutex};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};
use tracing::{debug, trace};

/// Upper bound for a single CDP command round-trip.
pub(crate) const CDP_CALL_TIMEOUT: Duration = Duration::from_secs(20);

type WsStream = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

/// Response channels for in-flight commands, keyed by command id.
type PendingResponses = Arc<AsyncMutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>;

/// Network trackers for flattened sessions, keyed by session id.
type SessionTrackers = Arc<AsyncMutex<HashMap<String, Arc<NetworkTracker>>>>;

/// Counts network requests that Chrome reported but has not finished, and
/// carries the request-allowance policy for its session tree.
#[derive(Debug)]
pub(crate) struct NetworkTracker {
    in_flight: Mutex<HashSet<String>>,
    /// Mirrors `BrowserConfig::allow_private_hosts`: the test-only escape hatch
    /// that permits loopback/private fixture targets.
    allow_private_hosts: bool,
}

impl Default for NetworkTracker {
    fn default() -> Self {
        Self::new(false)
    }
}

impl NetworkTracker {
    fn new(allow_private_hosts: bool) -> Self {
        Self {
            in_flight: Mutex::new(HashSet::new()),
            allow_private_hosts,
        }
    }

    fn record(&self, method: &str, params: &Value) {
        let request_id = params
            .get("requestId")
            .and_then(Value::as_str)
            .map(str::to_string);
        let Some(request_id) = request_id else {
            return;
        };
        let mut in_flight = self
            .in_flight
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match method {
            "Network.requestWillBeSent" => {
                // Redirects re-use the request id; only a new id is in flight.
                in_flight.insert(request_id);
            }
            "Network.loadingFinished" | "Network.loadingFailed" => {
                in_flight.remove(&request_id);
            }
            _ => {}
        }
    }

    fn in_flight(&self) -> usize {
        self.in_flight
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len()
    }
}

/// Connected CDP browser endpoint.
pub(crate) struct CdpClient {
    commands: mpsc::UnboundedSender<Message>,
    pending: PendingResponses,
    sessions: SessionTrackers,
    next_id: Arc<AtomicU64>,
}

/// CDP `Fetch.enable` parameters: pause every request at the request stage so
/// each one can be checked against the SSRF policy before Chrome sends it.
pub(crate) fn fetch_enable_params() -> Value {
    json!({
        "patterns": [{ "urlPattern": "*", "requestStage": "Request" }],
    })
}

/// Fire-and-forget CDP command. Responses are intentionally not awaited: the
/// only callers are event handlers that must keep the protocol stream moving,
/// and a failed continue/fail leaves the request paused (fail closed) until
/// the render budget expires.
fn send_command(
    commands: &mpsc::UnboundedSender<Message>,
    next_id: &AtomicU64,
    session_id: Option<&str>,
    method: &str,
    params: Value,
) {
    let id = next_id.fetch_add(1, Ordering::SeqCst);
    let mut message = json!({
        "id": id,
        "method": method,
        "params": params,
    });
    if let Some(session_id) = session_id {
        message["sessionId"] = Value::String(session_id.to_string());
    }
    let _ = commands.send(Message::Text(message.to_string()));
}

impl CdpClient {
    pub(crate) async fn connect(ws_url: &str) -> Result<Self> {
        let (stream, _) = connect_async(ws_url)
            .await
            .with_context(|| format!("connecting to Chromium DevTools endpoint {ws_url}"))?;

        let pending: PendingResponses = Arc::new(AsyncMutex::new(HashMap::new()));
        let sessions: SessionTrackers = Arc::new(AsyncMutex::new(HashMap::new()));

        let (sink, mut source) = stream.split();
        let (commands, command_rx) = mpsc::unbounded_channel::<Message>();

        tokio::spawn(writer_loop(sink, command_rx));

        let next_id = Arc::new(AtomicU64::new(1));
        let reader_pending = pending.clone();
        let reader_sessions = sessions.clone();
        let reader_commands = commands.clone();
        let reader_next_id = next_id.clone();
        tokio::spawn(async move {
            while let Some(incoming) = source.next().await {
                let message = match incoming {
                    Ok(message) => message,
                    Err(error) => {
                        debug!(error = %error, "cdp: websocket read error");
                        break;
                    }
                };
                match message {
                    Message::Text(text) => {
                        let Ok(value) = serde_json::from_str::<Value>(&text) else {
                            continue;
                        };
                        dispatch(
                            &value,
                            &reader_pending,
                            &reader_sessions,
                            &reader_commands,
                            &reader_next_id,
                        )
                        .await;
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
            fail_pending(&reader_pending, "Chromium DevTools connection closed").await;
        });

        Ok(Self {
            commands,
            pending,
            sessions,
            next_id,
        })
    }

    /// Send a CDP command, optionally scoped to a flattened session.
    pub(crate) async fn call(
        &self,
        session_id: Option<&str>,
        method: &str,
        params: Value,
    ) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let mut message = json!({
            "id": id,
            "method": method,
            "params": params,
        });
        if let Some(session_id) = session_id {
            message["sessionId"] = Value::String(session_id.to_string());
        }

        let (sender, receiver) = oneshot::channel();
        self.pending.lock().await.insert(id, sender);
        if self
            .commands
            .send(Message::Text(message.to_string()))
            .is_err()
        {
            self.pending.lock().await.remove(&id);
            return Err(anyhow!(
                "Chromium DevTools connection is closed (sending {method})"
            ));
        }
        let response = tokio::time::timeout(CDP_CALL_TIMEOUT, receiver)
            .await
            .map_err(|_| anyhow!("CDP command {method} timed out"))?
            .map_err(|_| anyhow!("CDP command {method} dropped"))?;

        let response = response.map_err(|error| anyhow!("CDP command {method} failed: {error}"))?;

        if let Some(protocol_error) = response.get("error") {
            let message = protocol_error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown protocol error");
            return Err(anyhow!("CDP command {method} rejected: {message}"));
        }
        Ok(response.get("result").cloned().unwrap_or(Value::Null))
    }

    /// Register a flattened session and return its network tracker.
    ///
    /// `allow_private_hosts` is the test-only escape hatch; it is propagated to
    /// child sessions through their shared tracker so the request policy is
    /// identical for out-of-process iframes and workers.
    pub(crate) async fn register_session(
        &self,
        session_id: &str,
        allow_private_hosts: bool,
    ) -> Arc<NetworkTracker> {
        let tracker = Arc::new(NetworkTracker::new(allow_private_hosts));
        self.sessions
            .lock()
            .await
            .insert(session_id.to_string(), tracker.clone());
        tracker
    }

    pub(crate) async fn unregister_session(&self, session_id: &str) {
        self.sessions.lock().await.remove(session_id);
    }
}

async fn writer_loop(
    mut sink: futures_util::stream::SplitSink<WsStream, Message>,
    mut commands: mpsc::UnboundedReceiver<Message>,
) {
    while let Some(message) = commands.recv().await {
        if let Err(error) = sink.send(message).await {
            debug!(error = %error, "cdp: websocket write error");
            break;
        }
    }
    let _ = sink.close().await;
}

async fn dispatch(
    value: &Value,
    pending: &PendingResponses,
    sessions: &SessionTrackers,
    commands: &mpsc::UnboundedSender<Message>,
    next_id: &Arc<AtomicU64>,
) {
    if let Some(id) = value.get("id").and_then(Value::as_u64) {
        if let Some(sender) = pending.lock().await.remove(&id) {
            let _ = sender.send(Ok(value.clone()));
        }
        return;
    }
    let Some(method) = value.get("method").and_then(Value::as_str) else {
        return;
    };
    let Some(session_id) = value.get("sessionId").and_then(Value::as_str) else {
        return;
    };
    let params = value.get("params").unwrap_or(&Value::Null);

    match method {
        // A child target (out-of-process iframe, worker) was auto-attached.
        // It must get the same request interception as the page session,
        // otherwise its network traffic bypasses the SSRF guard. Auto-attach
        // is configured with `waitForDebuggerOnStart`, so no child request can
        // be issued before `Fetch.enable` lands.
        "Target.attachedToTarget" => {
            let parent = sessions.lock().await.get(session_id).cloned();
            let (Some(parent), Some(child_session)) =
                (parent, params.get("sessionId").and_then(Value::as_str))
            else {
                return;
            };
            sessions
                .lock()
                .await
                .insert(child_session.to_string(), parent);
            send_command(
                commands,
                next_id,
                Some(child_session),
                "Network.enable",
                json!({}),
            );
            send_command(
                commands,
                next_id,
                Some(child_session),
                "Fetch.enable",
                fetch_enable_params(),
            );
            // Propagate auto-attach so nested out-of-process frames/workers
            // (grandchildren of the page) are intercepted as well.
            send_command(
                commands,
                next_id,
                Some(child_session),
                "Target.setAutoAttach",
                json!({
                    "autoAttach": true,
                    "waitForDebuggerOnStart": true,
                    "flatten": true,
                }),
            );
            if params
                .get("waitingForDebugger")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                send_command(
                    commands,
                    next_id,
                    Some(child_session),
                    "Runtime.runIfWaitingForDebugger",
                    json!({}),
                );
            }
        }
        "Target.detachedFromTarget" => {
            if let Some(child_session) = params.get("sessionId").and_then(Value::as_str) {
                sessions.lock().await.remove(child_session);
            }
        }
        // A request was paused before Chrome sent it. Validate it against the
        // SSRF policy and only then continue; otherwise fail it closed.
        "Fetch.requestPaused" => {
            let tracker = sessions.lock().await.get(session_id).cloned();
            let Some(tracker) = tracker else {
                debug!(session_id, "cdp: fetch pause on unregistered session");
                return;
            };
            let Some(request_id) = params
                .get("requestId")
                .and_then(Value::as_str)
                .map(str::to_string)
            else {
                return;
            };
            let url = params
                .pointer("/request/url")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let allow_private_hosts = tracker.allow_private_hosts;
            trace!(url = %url, allow_private_hosts, "cdp: fetch request paused");
            let commands = commands.clone();
            let next_id = next_id.clone();
            let session_id = session_id.to_string();
            tokio::spawn(async move {
                let allowed =
                    crate::browser::validation::browser_request_allowed(&url, allow_private_hosts)
                        .await;
                let (method, params) = if allowed {
                    ("Fetch.continueRequest", json!({ "requestId": request_id }))
                } else {
                    (
                        "Fetch.failRequest",
                        json!({ "requestId": request_id, "errorReason": "BlockedByClient" }),
                    )
                };
                send_command(&commands, &next_id, Some(&session_id), method, params);
            });
        }
        other => {
            if let Some(tracker) = sessions.lock().await.get(session_id).cloned() {
                tracker.record(other, params);
            }
        }
    }
}

async fn fail_pending(pending: &PendingResponses, reason: &str) {
    let mut pending = pending.lock().await;
    for (_, sender) in pending.drain() {
        let _ = sender.send(Err(reason.to_string()));
    }
}

/// A flattened CDP session scoped to one page target.
#[derive(Clone)]
pub(crate) struct CdpSession {
    client: Arc<CdpClient>,
    session_id: String,
    network: Arc<NetworkTracker>,
}

impl CdpSession {
    pub(crate) fn new(
        client: Arc<CdpClient>,
        session_id: String,
        network: Arc<NetworkTracker>,
    ) -> Self {
        Self {
            client,
            session_id,
            network,
        }
    }

    pub(crate) async fn call(&self, method: &str, params: Value) -> Result<Value> {
        self.client
            .call(Some(&self.session_id), method, params)
            .await
    }

    /// Evaluate a JavaScript expression in the page and return its JSON value.
    pub(crate) async fn evaluate(&self, expression: &str) -> Result<Value> {
        let result = self
            .call(
                "Runtime.evaluate",
                json!({
                    "expression": expression,
                    "returnByValue": true,
                    "awaitPromise": false,
                }),
            )
            .await?;
        if let Some(details) = result.get("exceptionDetails") {
            let text = details
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or("evaluation failed");
            return Err(anyhow!("page evaluation failed: {text}"));
        }
        Ok(result
            .get("result")
            .and_then(|result| result.get("value"))
            .cloned()
            .unwrap_or(Value::Null))
    }

    /// Network requests Chrome has started but not finished.
    pub(crate) fn in_flight_requests(&self) -> usize {
        self.network.in_flight()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn network_tracker_counts_in_flight_requests() {
        let tracker = NetworkTracker::default();
        tracker.record("Network.requestWillBeSent", &json!({"requestId": "a"}));
        tracker.record("Network.requestWillBeSent", &json!({"requestId": "b"}));
        assert_eq!(tracker.in_flight(), 2);

        tracker.record(
            "Network.requestWillBeSent",
            &json!({"requestId": "a", "redirectResponse": {}}),
        );
        assert_eq!(tracker.in_flight(), 2, "redirect must not double-count");

        tracker.record("Network.loadingFinished", &json!({"requestId": "a"}));
        assert_eq!(tracker.in_flight(), 1);

        tracker.record("Network.loadingFailed", &json!({"requestId": "b"}));
        assert_eq!(tracker.in_flight(), 0);

        tracker.record("Network.loadingFinished", &json!({"requestId": "b"}));
        assert_eq!(tracker.in_flight(), 0, "late completion must not underflow");
    }

    #[test]
    fn network_tracker_ignores_unrelated_events() {
        let tracker = NetworkTracker::default();
        tracker.record("Page.frameStoppedLoading", &json!({}));
        tracker.record("Network.responseReceived", &json!({"requestId": "a"}));
        assert_eq!(tracker.in_flight(), 0);
    }
}
