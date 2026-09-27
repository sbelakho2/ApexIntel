-- ──────────────────────────────────────────────────────────────────────────────
-- Migration 076: durable notification delivery (retry processor) + outbox leases
--
-- Delivery model (audit items 5, 6, 7, 8, 9, 10, 20, 36):
--
--   1. A domain alert/event is persisted (`notification_events`) in the SAME
--      transaction as the alert outbox row (`event_outbox`) that feeds the
--      real-time alert stream.
--   2. Per-channel deliveries are persisted BEFORE any attempt
--      (`notification_delivery_state`, one row per event/channel/destination,
--      status `pending`).
--   3. A worker job claims due deliveries with `FOR UPDATE SKIP LOCKED`, stamps
--      a lease (`lease_owner`/`lease_until`) and COMMITS, then attempts the
--      channel send OUTSIDE the transaction. A crash after the claim leaves the
--      lease to expire, so the row is retried instead of lost.
--   4. Failures are classified retryable vs permanent; retryable failures get
--      exponential backoff + jitter in `next_retry_at`; rows that exhaust their
--      channel attempt budget move to `dead_lettered` (terminal) and require an
--      operator replay.
--
--   The per-channel idempotency identity is
--   (notification_event_id, channel, destination) with the payload hash and
--   attempt embedded in the transport idempotency key, so repeated scheduler
--   runs cannot enqueue uncontrolled duplicates.
--
-- `event_outbox` gains lease + dead-letter columns for the same claim/lease
-- hardening: TX1 claims rows and commits, the publish happens outside any
-- transaction, TX2 stamps `published_at` or the retry/dead-letter outcome.
--
-- Transport guarantee: AT-LEAST-ONCE. The outbox id is sent as the JetStream
-- `Nats-Msg-Id` header and the stream carries a duplicate window, so a
-- redelivery of the same row is deduplicated by the broker; channel transports
-- carry an explicit idempotency key. This is NOT exactly-once delivery.
--
-- Idempotent (IF NOT EXISTS / DO blocks); does not modify any migration <= 068.
-- ──────────────────────────────────────────────────────────────────────────────

-- ── Domain notification events ────────────────────────────────────────────────
-- One durable row per logical notification (e.g. `sla_breach:<warning-id>`).
-- `dedupe_key` is stable across scheduler runs: re-running SLA enforcement for
-- the same warning can only re-enqueue the first time.
CREATE TABLE IF NOT EXISTS notification_events (
    id          UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    dedupe_key  TEXT        NOT NULL UNIQUE,
    source_type TEXT        NOT NULL,
    source_id   TEXT        NOT NULL,
    severity    TEXT        NOT NULL,
    category    TEXT        NOT NULL,
    title       TEXT        NOT NULL,
    body        TEXT        NOT NULL,
    payload     JSONB       NOT NULL DEFAULT '{}'::jsonb,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_notification_events_source
    ON notification_events (source_type, source_id);

-- ── Per-channel delivery state ────────────────────────────────────────────────
-- The table was introduced by 030_port_missing_tables.sql with only
-- `delivery_key`; evolve it into the retry-processor shape requested by the
-- audit. All additions are idempotent and existing (legacy) rows keep working.
ALTER TABLE notification_delivery_state
    ADD COLUMN IF NOT EXISTS notification_event_id UUID;
ALTER TABLE notification_delivery_state
    ADD COLUMN IF NOT EXISTS payload_hash TEXT;
ALTER TABLE notification_delivery_state
    ADD COLUMN IF NOT EXISTS lease_owner TEXT;
ALTER TABLE notification_delivery_state
    ADD COLUMN IF NOT EXISTS lease_until TIMESTAMPTZ;
ALTER TABLE notification_delivery_state
    ADD COLUMN IF NOT EXISTS dead_lettered_at TIMESTAMPTZ;
ALTER TABLE notification_delivery_state
    ADD COLUMN IF NOT EXISTS dead_letter_reason TEXT;
-- Legacy `last_attempt_at` is kept in sync by the new writers as well.
ALTER TABLE notification_delivery_state
    ADD COLUMN IF NOT EXISTS delivered_at TIMESTAMPTZ;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'notification_delivery_state_event_fk'
    ) THEN
        ALTER TABLE notification_delivery_state
            ADD CONSTRAINT notification_delivery_state_event_fk
            FOREIGN KEY (notification_event_id)
            REFERENCES notification_events(id) ON DELETE CASCADE;
    END IF;
END $$;

-- Status vocabulary: pending | delivering | failed | delivered | dead_lettered.
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'notification_delivery_state_status_check'
    ) THEN
        ALTER TABLE notification_delivery_state
            ADD CONSTRAINT notification_delivery_state_status_check
            CHECK (status IN ('pending', 'delivering', 'failed', 'delivered', 'dead_lettered'));
    END IF;
END $$;

-- Per-channel idempotency identity: one delivery row per event/channel/destination.
-- Repeated scheduler runs hit this constraint and enqueue nothing new.
CREATE UNIQUE INDEX IF NOT EXISTS uq_notification_delivery_identity
    ON notification_delivery_state (notification_event_id, channel, destination)
    WHERE notification_event_id IS NOT NULL;

-- The claim query: due rows, oldest first.
CREATE INDEX IF NOT EXISTS idx_notification_delivery_claim
    ON notification_delivery_state (next_retry_at, created_at)
    WHERE status IN ('pending', 'failed') AND dead_lettered_at IS NULL;

-- Admin dead-letter listing / backlog health.
CREATE INDEX IF NOT EXISTS idx_notification_delivery_dead_letters
    ON notification_delivery_state (dead_lettered_at DESC)
    WHERE dead_lettered_at IS NOT NULL;

-- ── Outbox lease + dead-letter state ─────────────────────────────────────────
-- The publisher no longer holds the row lock across the NATS publish: it claims
-- with a lease in TX1, publishes outside the transaction, settles in TX2.
ALTER TABLE event_outbox ADD COLUMN IF NOT EXISTS lease_owner TEXT;
ALTER TABLE event_outbox ADD COLUMN IF NOT EXISTS lease_until TIMESTAMPTZ;
ALTER TABLE event_outbox ADD COLUMN IF NOT EXISTS dead_lettered_at TIMESTAMPTZ;
ALTER TABLE event_outbox ADD COLUMN IF NOT EXISTS dead_letter_reason TEXT;

CREATE INDEX IF NOT EXISTS idx_event_outbox_claimable
    ON event_outbox (created_at)
    WHERE published_at IS NULL AND dead_lettered_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_event_outbox_dead_letters
    ON event_outbox (dead_lettered_at DESC)
    WHERE dead_lettered_at IS NOT NULL;

COMMENT ON TABLE notification_events IS
    'Durable domain notification: persisted atomically with its alert outbox event and per-channel delivery rows';
COMMENT ON TABLE notification_delivery_state IS
    'Per-channel delivery outbox: one row per (notification_event_id, channel, destination), claimed with a lease and retried with exponential backoff until delivered or dead-lettered';
COMMENT ON COLUMN notification_delivery_state.payload_hash IS
    'SHA-256 of the notification payload; part of the per-attempt transport idempotency key';
COMMENT ON COLUMN notification_delivery_state.dead_lettered_at IS
    'Set when the channel attempt budget is exhausted; terminal until an operator replays the row';
COMMENT ON COLUMN event_outbox.lease_owner IS
    'Claim owner while a publisher is attempting delivery; NULL when free';
COMMENT ON COLUMN event_outbox.dead_lettered_at IS
    'Set when MAX_OUTBOX_ATTEMPTS is exhausted; removed from the claim query until admin replay';
COMMENT ON COLUMN event_outbox.published_at IS
    'Set only after the JetStream publish ACK was awaited; at-least-once transport, deduplicated by Nats-Msg-Id';

-- The application connects as a non-owner role in production; new/evolved tables
-- need explicit grants (same pattern as 050_heartbeat_grants.sql).
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'apexintel') THEN
        GRANT SELECT, INSERT, UPDATE, DELETE ON notification_events TO apexintel;
        GRANT SELECT, INSERT, UPDATE, DELETE ON notification_delivery_state TO apexintel;
        GRANT SELECT, INSERT, UPDATE, DELETE ON notification_delivery_attempts TO apexintel;
        GRANT SELECT, INSERT, UPDATE, DELETE ON event_outbox TO apexintel;
    END IF;
END $$;
