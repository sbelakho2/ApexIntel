# Notification delivery

Durable alert delivery pipeline (audit items 5–10, 20, 36).

## Pipeline

```text
domain alert/event  ──┐
                      │  ONE transaction
alert outbox row  ────┤  (notification_events + event_outbox + notification_delivery_state)
per-channel rows  ────┘
      │
      ├─ event_outbox  ──> outbox drain ──> NATS JetStream (`alerts.events.*`)
      │                                     Nats-Msg-Id = outbox row id
      │                                     ──> API SSE ──> browser
      │
      └─ notification_delivery_state (pending, one row per event/channel/destination)
                 │
                 └─ notification_delivery worker (every minute)
                        claim with lease (TX1: FOR UPDATE SKIP LOCKED)
                        send outside any transaction
                        settle (TX2): delivered | failed(next_retry_at) | dead_lettered
```

SLA breach and reminder alerts use exactly this pipeline: `run_sla_enforcement`
only enqueues (`notification_events` + `event_outbox` + per-channel rows) and
never publishes or sends directly. There is no private NATS branch in
`NotificationDispatcher`; `NatsPublisher::publish_alert` is reachable only
through `crates/worker/src/alert_transport.rs` (enforced by
`scripts/ci/check_alert_publish.sh`).

## Delivery guarantees

**At-least-once, not exactly-once.**

- The outbox row and its warning/domain row commit atomically, so a crash can
  never lose the alert — only delay it.
- A crashed publisher republishes its last claimed rows after the lease expires.
- The stable outbox row id is sent as the JetStream `Nats-Msg-Id` header and the
  `alerts` stream keeps a 2-hour duplicate window, so the broker suppresses a
  republish of the same row inside that window. Consumers (API SSE router) must
  still tolerate a redelivery outside the window.
- Channel deliveries carry a stable per-attempt idempotency key
  `sha256(notification_event_id | channel | destination | attempt | payload_hash)`
  (webhook `Idempotency-Key` header); channels are expected to deduplicate on
  it.

Exactly-once delivery is explicitly **not** claimed: a channel that accepted a
notification but crashed before the settlement write is retried.

## Claim / lease

- Claims happen in TX1 (`SELECT ... FOR UPDATE SKIP LOCKED` + `lease_owner`,
  `lease_until`, `attempts = attempts + 1`) and commit **before** any network
  call. No database lock is held across a publish or send.
- A crashed claim is retried when its lease expires (outbox: 120s; channel
  deliveries: 120s).
- The attempt is persisted before it happens ("persist before attempting").

## Retry, backoff, dead-letter

- Exponential backoff with jitter: `30s * 2^(attempt-1)`, capped at 1h, scaled
  to 50–100% by jitter.
- Retryable vs permanent classification:
  - HTTP: 408/425/429 and 5xx retry; other 4xx are permanent.
  - SMTP: 4xx retry, 5xx permanent.
- Attempt budgets: webhook/generic 8, email 5. Exhausted or permanent failures
  move to `dead_lettered` (terminal, `dead_lettered_at`, `dead_letter_reason`).
- Outbox events dead-letter after `MAX_OUTBOX_ATTEMPTS` (10) and raise an
  operator alert (error log + `activity_feed` entry + metric).

## Operations

- Admin UI (`/admin` → *Notification Delivery Dead Letters*) lists dead-lettered
  channel deliveries and outbox events and exposes **Replay** (re-queues with a
  fresh attempt budget).
- Metrics (worker Prometheus text): `apexintel_worker_notification_deliveries_*`,
  `apexintel_worker_outbox_events_dead_lettered_total`.
- Readiness: `apex-worker healthcheck` fails when a backlog exceeds its
  threshold:
  - channel deliveries: `NOTIFICATION_DELIVERY_MAX_OVERDUE` (default 250 due
    rows), `NOTIFICATION_DELIVERY_MAX_DEAD_LETTERED` (default 25);
  - alert outbox: `OUTBOX_MAX_OVERDUE` (default 250 rows claimable for more
    than five minutes), `OUTBOX_MAX_DEAD_LETTERED` (default 25).

## Migrations

- `061_event_outbox.sql` — transactional alert outbox.
- `069_notification_delivery.sql` — `notification_events`, per-channel delivery
  state (claims, lease, payload hash, dead-letter), outbox lease/dead-letter
  columns.
