-- Migration 104: durable insight analysis runs + payload-carrying triggers
-- ════════════════════════════════════════════════════════════════════════════
-- Insight analysis used to run synchronously inside the API request: the
-- endpoint held a request open for the full model call, a client disconnect
-- discarded the work, and nothing was persisted so the same click always paid
-- for the same inference again.
--
-- This migration makes the work durable:
--
--   1. `worker_trigger_queue.payload` — a manual trigger can carry the exact
--      run it must execute instead of only naming a job kind, so a
--      payload-bound job (insight analysis) never has to guess which row it
--      was queued for.
--
--   2. `insight_analysis_runs` — one row per analysis request. The API inserts
--      a `queued` row (and the trigger that references it) in one transaction;
--      the worker claims the row (`queued` → `running`), runs the model
--      outside the request path, and records the result JSON or the error.
--      A partial unique index keeps at most one queued/running run per
--      insight, so repeated clicks deduplicate instead of stacking inference.
--
-- Idempotent: every statement is `IF NOT EXISTS`/guarded, so re-applying is a
-- no-op.
-- ════════════════════════════════════════════════════════════════════════════

-- ── 1. Payload-carrying manual triggers ─────────────────────────────────────
-- Existing triggers keep a NULL payload and behave exactly as before.
ALTER TABLE worker_trigger_queue ADD COLUMN IF NOT EXISTS payload JSONB;

-- ── 2. Durable insight analysis runs ────────────────────────────────────────
CREATE TABLE IF NOT EXISTS insight_analysis_runs (
    id           UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    insight_id   UUID        NOT NULL REFERENCES insights(id) ON DELETE CASCADE,
    status       TEXT        NOT NULL DEFAULT 'queued'
        CHECK (status IN ('queued', 'running', 'succeeded', 'failed')),
    requested_by TEXT,
    requested_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    started_at   TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    result       JSONB,
    error        TEXT,
    CONSTRAINT insight_analysis_runs_success_has_result
        CHECK (status <> 'succeeded' OR result IS NOT NULL),
    CONSTRAINT insight_analysis_runs_failure_has_error
        CHECK (status <> 'failed' OR error IS NOT NULL)
);

-- Dedupe in-flight work: at most one queued/running run per insight. A second
-- click while a run is queued or executing resolves onto that run.
CREATE UNIQUE INDEX IF NOT EXISTS uq_insight_analysis_runs_inflight
    ON insight_analysis_runs (insight_id)
    WHERE status IN ('queued', 'running');

-- Latest-run lookup for `GET /api/insights/:id/analyze/latest`.
CREATE INDEX IF NOT EXISTS idx_insight_analysis_runs_insight_requested
    ON insight_analysis_runs (insight_id, requested_at DESC, id DESC);

-- Status polling / stuck-run inspection.
CREATE INDEX IF NOT EXISTS idx_insight_analysis_runs_status
    ON insight_analysis_runs (status, requested_at);
