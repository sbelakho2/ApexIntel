-- ApexIntel Schema Migration – worker trigger queue + job history status fix
--
-- This migration addresses three issues discovered during worker deployment:
--
-- 1. MISSING TABLE: worker_trigger_queue
--    Referenced by crates/store/src/postgres/admin.rs (queue_job_trigger,
--    pop_job_trigger, complete_job_trigger, timeout_stale_job_triggers) but
--    never created. This table is the API→Worker manual trigger bridge:
--    the API inserts a row, the worker polls every 30s and claims it.
--
-- 2. CHECK CONSTRAINT: worker_job_history.status
--    Migration 2026051402 set CHECK (status IN ('queued','running','completed','failed'))
--    but the worker's format_status() produces 'succeeded', 'skipped', 'pending',
--    'running', 'failed'. The constraint rejects 'succeeded' and 'skipped',
--    preventing job history persistence.
-- ==============================================================================

-- ─── 1. worker_trigger_queue ──────────────────────────────────────────────────
-- Manual trigger queue: API enqueues, worker claims atomically via
-- UPDATE ... FOR UPDATE SKIP LOCKED + pg_try_advisory_xact_lock.
CREATE TABLE IF NOT EXISTS worker_trigger_queue (
    id            UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    job_kind      TEXT        NOT NULL,
    requested_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    claimed_at    TIMESTAMPTZ,
    completed_at  TIMESTAMPTZ,
    recovered_at  TIMESTAMPTZ,
    recovery_count INTEGER    NOT NULL DEFAULT 0,
    error         TEXT
);

-- Index for efficient pop: find unclaimed, uncompleted rows ordered by age.
CREATE INDEX IF NOT EXISTS idx_worker_trigger_queue_claim
    ON worker_trigger_queue (claimed_at, requested_at)
    WHERE completed_at IS NULL;

-- Index for dedup check: find active triggers of the same kind.
CREATE INDEX IF NOT EXISTS idx_worker_trigger_queue_kind_active
    ON worker_trigger_queue (job_kind, claimed_at)
    WHERE completed_at IS NULL;

-- ─── 2. Fix worker_job_history status check constraint ────────────────────────
-- The worker writes: 'succeeded', 'failed', 'skipped', 'running', 'pending'.
-- The old constraint only allowed: 'queued', 'running', 'completed', 'failed'.
DO $$
BEGIN
    ALTER TABLE worker_job_history DROP CONSTRAINT IF EXISTS worker_job_history_status_check;
    ALTER TABLE worker_job_history ADD CONSTRAINT worker_job_history_status_check
        CHECK (status IN ('pending', 'running', 'succeeded', 'failed', 'skipped',
                          'queued', 'completed'));
EXCEPTION WHEN OTHERS THEN
    RAISE NOTICE 'worker_job_history constraint update skipped: %', SQLERRM;
END $$;
