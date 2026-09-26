-- ──────────────────────────────────────────────────────────────────────────────
-- Migration 062: accept degraded job statuses
--
-- The worker now reports `JobStatus::Degraded` runs (warnings persisted with zero
-- completed triage submissions, etc.). Two existing CHECK constraints predate
-- the variant and rejected its persisted forms, so degraded runs were silently
-- losing their history row and activity entry:
--   * worker_job_history_status_check (031) allowed pending/running/succeeded/
--     failed/skipped/queued/completed, but not 'degraded'.
--   * chk_activity_action_type (002) allowed job_completed/job_failed/
--     job_skipped, but not 'job_degraded'.
--
-- Idempotent; does not touch any applied migration (<= 061).
-- ──────────────────────────────────────────────────────────────────────────────

-- worker_job_history.status: add 'degraded' to the allowed set.
DO $$
BEGIN
    IF EXISTS (SELECT FROM pg_tables WHERE tablename = 'worker_job_history') THEN
        ALTER TABLE worker_job_history DROP CONSTRAINT IF EXISTS worker_job_history_status_check;
        ALTER TABLE worker_job_history ADD CONSTRAINT worker_job_history_status_check
            CHECK (status IN ('pending', 'running', 'succeeded', 'degraded',
                              'failed', 'skipped', 'queued', 'completed'));
    END IF;
END $$;

-- activity_feed.action_type: add 'job_degraded' to the job lifecycle events.
DO $$
BEGIN
    IF EXISTS (SELECT FROM pg_tables WHERE tablename = 'activity_feed') THEN
        ALTER TABLE activity_feed DROP CONSTRAINT IF EXISTS chk_activity_action_type;
        ALTER TABLE activity_feed ADD CONSTRAINT chk_activity_action_type CHECK (action_type IN (
            'create', 'update', 'delete', 'share', 'assign', 'comment', 'resolve',
            'reopen', 'escalate', 'deescalate', 'approve', 'reject', 'merge', 'split',
            -- System event types
            'insight_generated', 'poi_discovered', 'crawl_completed', 'company_detected',
            'threat_detected', 'psych_profile_updated', 'battlecard_generated',
            'memo_generated', 'recipe_promoted',
            -- Job lifecycle events
            'job_completed', 'job_degraded', 'job_failed', 'job_skipped'
        ));
    END IF;
END $$;
