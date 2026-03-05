-- ApexIntel Schema Migration – worker trigger queue
-- Allows the API to queue manual job runs that the worker picks up.

CREATE TABLE IF NOT EXISTS worker_trigger_queue (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    job_kind TEXT NOT NULL,
    requested_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    claimed_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    error TEXT
);

CREATE INDEX IF NOT EXISTS idx_worker_trigger_queue_unclaimed
    ON worker_trigger_queue (requested_at)
    WHERE claimed_at IS NULL;
