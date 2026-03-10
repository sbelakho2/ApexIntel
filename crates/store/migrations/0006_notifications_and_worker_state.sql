-- ApexIntel Schema Migration – notifications inbox and worker scheduler state

CREATE TABLE IF NOT EXISTS analyst_notifications (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id TEXT NOT NULL,
    category TEXT NOT NULL,
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    entity_type TEXT,
    entity_id TEXT,
    action_url TEXT,
    is_read BOOLEAN NOT NULL DEFAULT FALSE,
    read_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_analyst_notifications_user_created
    ON analyst_notifications (user_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_analyst_notifications_user_unread
    ON analyst_notifications (user_id, created_at DESC)
    WHERE is_read = FALSE;

CREATE TABLE IF NOT EXISTS worker_job_state (
    job_kind TEXT PRIMARY KEY,
    last_run TIMESTAMPTZ,
    last_status TEXT,
    last_error TEXT,
    last_duration_ms BIGINT,
    consecutive_failures INTEGER NOT NULL DEFAULT 0,
    max_consecutive_failures INTEGER NOT NULL DEFAULT 5,
    circuit_open BOOLEAN NOT NULL DEFAULT FALSE,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS worker_job_history (
    run_id TEXT PRIMARY KEY,
    job_kind TEXT NOT NULL,
    status TEXT NOT NULL,
    started_at TIMESTAMPTZ NOT NULL,
    finished_at TIMESTAMPTZ,
    duration_ms BIGINT,
    items_processed BIGINT NOT NULL DEFAULT 0,
    notes TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_worker_job_history_kind_started
    ON worker_job_history (job_kind, started_at DESC);

ALTER TABLE worker_trigger_queue
    ADD COLUMN IF NOT EXISTS recovery_count INTEGER NOT NULL DEFAULT 0;

ALTER TABLE worker_trigger_queue
    ADD COLUMN IF NOT EXISTS recovered_at TIMESTAMPTZ;