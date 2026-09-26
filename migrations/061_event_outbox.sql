-- ──────────────────────────────────────────────────────────────────────────────
-- Migration 061: transactional event outbox
--
-- Warning persistence and alert publication must not be two independent
-- best-effort operations: a crash between the warning INSERT commit and the
-- NATS publish silently loses the alert, and a publish before commit can alert
-- on a warning that never existed. The outbox closes both holes.
--
-- Producers write the warning row and its `new_warning` event row in ONE
-- transaction. The single outbox publisher then drains unpublished rows with
-- `SELECT ... FOR UPDATE SKIP LOCKED`, publishes each payload to NATS
-- JetStream, awaits the real publish ACK, and only then stamps `published_at`
-- (all inside the same transaction, so a crash mid-publish rolls the lock back
-- and the event is retried instead of lost).
--
-- `aggregate_type`/`aggregate_id` identify the source row (`warning` + its
-- UUID); `event_type` is the alert event type (`new_warning`); `payload` is the
-- serialized AlertEvent; `attempts`/`last_error` carry the retry bookkeeping.
--
-- Idempotent (IF NOT EXISTS guards); does not touch any applied migration
-- (<= 060).
-- ──────────────────────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS event_outbox (
    id             UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    aggregate_type TEXT        NOT NULL,
    aggregate_id   UUID        NOT NULL,
    event_type     TEXT        NOT NULL,
    payload        JSONB       NOT NULL,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    published_at   TIMESTAMPTZ,
    attempts       INTEGER     NOT NULL DEFAULT 0,
    last_error     TEXT
);

-- The publisher's drain query: oldest unpublished events first. The partial
-- predicate keeps the index small — published rows are never scanned.
CREATE INDEX IF NOT EXISTS idx_event_outbox_unpublished
    ON event_outbox (created_at)
    WHERE published_at IS NULL;

-- Lookups by source aggregate (e.g. "was this warning's alert published?").
CREATE INDEX IF NOT EXISTS idx_event_outbox_aggregate
    ON event_outbox (aggregate_type, aggregate_id);

COMMENT ON TABLE event_outbox IS
    'Transactional outbox: events committed atomically with their source row, published once NATS acknowledges them';
COMMENT ON COLUMN event_outbox.payload IS
    'Serialized alert/domain event handed to the NATS publisher';
COMMENT ON COLUMN event_outbox.published_at IS
    'Set only after the JetStream publish ACK was awaited; NULL while pending or after a failed attempt';

-- The application connects as a non-owner role in production; new tables need
-- explicit grants (same pattern as 050_heartbeat_grants.sql / 060_entity_review_queue.sql).
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'apexintel') THEN
        GRANT SELECT, INSERT, UPDATE, DELETE ON event_outbox TO apexintel;
    END IF;
END $$;
