-- ════════════════════════════════════════════════════════════════════════════
-- Migration 096: accept the interrupted worker-job status
-- ════════════════════════════════════════════════════════════════════════════
--
-- A worker shut down mid-run leaves `worker_job_state.last_status` and the
-- matching `worker_job_history.status` rows stuck on 'running' until the next
-- start. The shutdown drain records them as 'interrupted' after the grace
-- period expires, but the history CHECK constraint predates that status and
-- would reject the write. This migration widens the allowed set.
--
-- (`worker_job_state` has no status CHECK, so only the history constraint
-- needs updating.)
--
-- Idempotent: safe to re-apply.

DO $$
BEGIN
    IF EXISTS (SELECT FROM pg_tables WHERE tablename = 'worker_job_history') THEN
        ALTER TABLE worker_job_history DROP CONSTRAINT IF EXISTS worker_job_history_status_check;
        ALTER TABLE worker_job_history ADD CONSTRAINT worker_job_history_status_check
            CHECK (status IN ('pending', 'running', 'succeeded', 'degraded',
                              'failed', 'skipped', 'queued', 'completed',
                              'interrupted'));
    END IF;
END $$;
