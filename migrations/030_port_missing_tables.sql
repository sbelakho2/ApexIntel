-- ApexIntel Schema Migration – port 14 missing tables from legacy crate-level migrations.
--
-- These tables are referenced by Rust code in crates/store/src/postgres/ but were never
-- ported to the workspace-level migrations/ directory (the single source of truth used by
-- sqlx::migrate!("../../migrations") in crates/store/src/postgres.rs).
--
-- Sources:
--   crates/store/migrations/0006_notifications_and_worker_state.sql
--   crates/store/migrations/0007_identity_tags_and_delivery.sql
--   crates/store/migrations/0008_llm_governance.sql
--   crates/store/migrations/0013_insight_feedback_loop.sql
--   crawl_logs: inferred from crates/store/src/postgres/analytics.rs (get_crawl_stats)
--
-- Notes:
--   * All statements are idempotent (CREATE TABLE / INDEX IF NOT EXISTS).
--   * f64 Rust fields map to DOUBLE PRECISION (e.g. insight_firings.confidence),
--     never DECIMAL/NUMERIC, to match sqlx::FromRow decoding.
--   * No CREATE INDEX CONCURRENTLY (incompatible with sqlx transaction wrapping).
--   * No explicit BEGIN/COMMIT (sqlx wraps each migration in a transaction).

-- =============================================================================
-- From 0006_notifications_and_worker_state.sql
-- =============================================================================

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

-- =============================================================================
-- From 0007_identity_tags_and_delivery.sql
-- (only the 4 tables missing from the workspace migrations)
-- =============================================================================

CREATE TABLE IF NOT EXISTS api_key_owners (
    key_id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES analyst_users(id) ON DELETE CASCADE,
    role TEXT NOT NULL,
    display_name TEXT NOT NULL,
    last_seen_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_api_key_owners_user_id
    ON api_key_owners (user_id);

CREATE TABLE IF NOT EXISTS sla_reminder_state (
    warning_id TEXT NOT NULL,
    reminder_kind TEXT NOT NULL,
    delivery_key TEXT NOT NULL,
    detail JSONB NOT NULL DEFAULT '{}'::jsonb,
    sent_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (warning_id, reminder_kind)
);

CREATE TABLE IF NOT EXISTS notification_delivery_state (
    delivery_key TEXT PRIMARY KEY,
    channel TEXT NOT NULL,
    destination TEXT NOT NULL,
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    status TEXT NOT NULL DEFAULT 'pending',
    attempts INTEGER NOT NULL DEFAULT 0,
    last_attempt_at TIMESTAMPTZ,
    next_retry_at TIMESTAMPTZ,
    delivered_at TIMESTAMPTZ,
    last_error TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_notification_delivery_state_status_retry
    ON notification_delivery_state (status, next_retry_at, updated_at DESC);

CREATE TABLE IF NOT EXISTS notification_delivery_attempts (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    delivery_key TEXT NOT NULL REFERENCES notification_delivery_state(delivery_key) ON DELETE CASCADE,
    attempted_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    status TEXT NOT NULL,
    error TEXT
);

CREATE INDEX IF NOT EXISTS idx_notification_delivery_attempts_delivery_key
    ON notification_delivery_attempts (delivery_key, attempted_at DESC);

-- =============================================================================
-- From 0008_llm_governance.sql
-- =============================================================================

CREATE TABLE IF NOT EXISTS prompt_versions (
    prompt_id TEXT NOT NULL,
    version TEXT NOT NULL,
    workflow TEXT NOT NULL,
    system_prompt TEXT NOT NULL,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (prompt_id, version)
);

CREATE INDEX IF NOT EXISTS idx_prompt_versions_workflow
    ON prompt_versions (workflow, created_at DESC);

CREATE TABLE IF NOT EXISTS llm_workflow_runs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workflow TEXT NOT NULL,
    prompt_id TEXT NOT NULL,
    prompt_version TEXT NOT NULL,
    model_name TEXT NOT NULL,
    request_payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    response_payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    validation_issues JSONB NOT NULL DEFAULT '[]'::jsonb,
    quality_gate_passed BOOLEAN NOT NULL DEFAULT FALSE,
    duration_ms BIGINT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    FOREIGN KEY (prompt_id, prompt_version) REFERENCES prompt_versions(prompt_id, version)
);

CREATE INDEX IF NOT EXISTS idx_llm_workflow_runs_workflow_created_at
    ON llm_workflow_runs (workflow, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_llm_workflow_runs_prompt
    ON llm_workflow_runs (prompt_id, prompt_version, created_at DESC);

CREATE TABLE IF NOT EXISTS llm_improvement_runs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    run_kind TEXT NOT NULL,
    run_key TEXT NOT NULL,
    metrics JSONB NOT NULL DEFAULT '{}'::jsonb,
    artifacts JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (run_kind, run_key)
);

CREATE INDEX IF NOT EXISTS idx_llm_improvement_runs_kind_created_at
    ON llm_improvement_runs (run_kind, created_at DESC);

CREATE TABLE IF NOT EXISTS llm_training_datasets (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    dataset_name TEXT NOT NULL,
    dataset_version TEXT NOT NULL,
    source_run_kind TEXT NOT NULL,
    source_run_key TEXT NOT NULL,
    manifest JSONB NOT NULL DEFAULT '{}'::jsonb,
    example_count BIGINT NOT NULL DEFAULT 0,
    examples_jsonl TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (dataset_name, dataset_version)
);

CREATE INDEX IF NOT EXISTS idx_llm_training_datasets_name_created_at
    ON llm_training_datasets (dataset_name, created_at DESC);

-- =============================================================================
-- From 0013_insight_feedback_loop.sql
-- =============================================================================

CREATE TABLE IF NOT EXISTS insight_feedback_events (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    insight_id    UUID NOT NULL REFERENCES insights(id) ON DELETE CASCADE,
    entity_id     UUID,
    recipe_code   TEXT,
    feedback_type TEXT NOT NULL,
    user_id       TEXT NOT NULL,
    notes         TEXT,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (insight_id, user_id, feedback_type)
);

CREATE INDEX IF NOT EXISTS idx_insight_feedback_insight ON insight_feedback_events(insight_id);
CREATE INDEX IF NOT EXISTS idx_insight_feedback_entity_recipe ON insight_feedback_events(entity_id, recipe_code, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_insight_feedback_created ON insight_feedback_events(created_at DESC);

CREATE TABLE IF NOT EXISTS insight_firings (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    insight_id    UUID REFERENCES insights(id) ON DELETE SET NULL,
    entity_id     UUID NOT NULL,
    recipe_code   TEXT NOT NULL,
    insight_type  TEXT,
    title         TEXT NOT NULL,
    summary       TEXT NOT NULL,
    insight_hash  TEXT NOT NULL,
    confidence    DOUBLE PRECISION,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_insight_firings_entity_recipe ON insight_firings(entity_id, recipe_code, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_insight_firings_hash ON insight_firings(insight_hash);
CREATE INDEX IF NOT EXISTS idx_insight_firings_created ON insight_firings(created_at DESC);

-- =============================================================================
-- crawl_logs
-- Inferred from crates/store/src/postgres/analytics.rs::get_crawl_stats, which reads:
--   status (TEXT: 'success' | 'failed'),
--   new_observations, changed_pages, bytes_fetched (BIGINT, summed),
--   created_at (TIMESTAMPTZ, filtered with >= $1).
-- Each row represents one crawled source attempt.
-- =============================================================================

CREATE TABLE IF NOT EXISTS crawl_logs (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    source_url        TEXT,
    status            TEXT NOT NULL DEFAULT 'success',
    new_observations  BIGINT NOT NULL DEFAULT 0,
    changed_pages     BIGINT NOT NULL DEFAULT 0,
    bytes_fetched     BIGINT NOT NULL DEFAULT 0,
    error             TEXT,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_crawl_logs_created_at
    ON crawl_logs (created_at DESC);
