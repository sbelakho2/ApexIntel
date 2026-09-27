-- ──────────────────────────────────────────────────────────────────────────────
-- Migration 080: notification delivery readiness indexes + payload reconciliation
--
-- The full-profile `notification_delivery` capability probe measures the
-- retry processor from live state (`notification_delivery_state` +
-- `notification_delivery_attempts`):
--
--   * overdue backlog and oldest-overdue age      -> idx_notification_delivery_claim (076)
--   * stuck `delivering` leases (lease_until<now) -> idx_notification_delivery_state_stuck_leases
--   * reclaiming expired leases                   -> idx_notification_delivery_claim_reclaim
--   * dead-letter count and recent change rate    -> idx_notification_delivery_dead_letters (076)
--   * recent attempt success ratio over a window  -> idx_notification_delivery_attempts_recent
--
-- The attempt log is written in the same transaction as each settlement
-- (delivered / failed / dead-lettered), so the ratio is exact rather than
-- inferred from mutable state.
--
-- The second half reconciles rows enqueued by the pre-fix writer, which stored
-- the raw `PendingAlert` as the per-channel payload while the retry processor
-- deserializes the `{ "alert": ... }` transport wrapper. Left alone, every one
-- of those rows would dead-letter as "missing field `alert`" instead of being
-- delivered.
--
-- Idempotent (IF NOT EXISTS / guarded UPDATE); does not modify any migration
-- <= 078.
-- ──────────────────────────────────────────────────────────────────────────────

-- A claimed row past its lease means the attempt (or its settlement) died:
-- the probe treats these as stuck deliveries, not as healthy in-flight work.
CREATE INDEX IF NOT EXISTS idx_notification_delivery_state_stuck_leases
    ON notification_delivery_state (lease_until)
    WHERE status = 'delivering';

-- Those expired-lease rows are claimable again (a crash after the channel
-- accepted the send must be retried, not stuck forever). The 076 claim index
-- only covers pending/failed, so give the reclaim path its own partial index.
CREATE INDEX IF NOT EXISTS idx_notification_delivery_claim_reclaim
    ON notification_delivery_state (lease_until, next_retry_at, created_at)
    WHERE status = 'delivering';

-- Recent-window attempt log scan for the success-ratio / dead-letter-rate
-- readiness figures.
CREATE INDEX IF NOT EXISTS idx_notification_delivery_attempts_recent
    ON notification_delivery_attempts (attempted_at DESC, status);

-- Wrap legacy raw-alert payloads in the transport shape. Only rows created by
-- the durable enqueue path are touched (`notification_event_id IS NOT NULL`),
-- never arbitrary rows written by other collaborators. The existing
-- `payload_hash` is intentionally kept: the stable transport key stays
-- consistent across every attempt of a row.
UPDATE notification_delivery_state
   SET payload = jsonb_build_object('alert', payload),
       updated_at = now()
 WHERE notification_event_id IS NOT NULL
   AND NOT (payload ? 'alert')
   AND status IN ('pending', 'failed', 'delivering', 'dead_lettered');
