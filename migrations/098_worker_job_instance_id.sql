-- Migration 098: scope worker job rows to the owning process instance
--
-- The shutdown reconciliation previously touched every replica's live jobs:
-- it updated all rows with status 'running' with no instance predicate, so
-- one worker stopping marked another replica's in-flight jobs 'interrupted'.
-- Record the process instance id when a run is claimed/inserted and filter
-- the reconciliation on it, so a shutdown only touches its own rows.
--
-- Nullable for rows written before this revision; those keep their normal
-- completion path. Idempotent: safe to re-apply.

ALTER TABLE IF EXISTS worker_job_state
    ADD COLUMN IF NOT EXISTS instance_id TEXT;

ALTER TABLE IF EXISTS worker_job_history
    ADD COLUMN IF NOT EXISTS instance_id TEXT;

-- Partial indexes keep the shutdown reconciliation cheap: it only ever looks
-- at running rows for one instance.
CREATE INDEX IF NOT EXISTS idx_worker_job_state_running_instance
    ON worker_job_state (instance_id) WHERE last_status = 'running';

CREATE INDEX IF NOT EXISTS idx_worker_job_history_running_instance
    ON worker_job_history (instance_id) WHERE status = 'running';
