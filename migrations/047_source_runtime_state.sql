-- 047_source_runtime_state.sql
--
-- Persistent per-source crawl scheduling state (P0 #1: the collection system
-- does not actually crawl the advertised source universe).
--
-- The crawl cycle previously re-selected the same registry prefix every run
-- because it had no memory of when each source was last attempted or
-- succeeded.  This table gives the weighted-fair scheduler in `apex-crawl`
-- the state it needs: due time, consecutive failures, exponential-failure
-- backoff window (via `next_due_at`/`circuit_open_until`) and last error.
--
-- Idempotent (IF NOT EXISTS) so it can be re-applied safely.

CREATE TABLE IF NOT EXISTS source_runtime_state (
    source_slug TEXT PRIMARY KEY,
    last_attempt_at TIMESTAMPTZ,
    last_success_at TIMESTAMPTZ,
    next_due_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    consecutive_failures INTEGER NOT NULL DEFAULT 0,
    rolling_success_rate DOUBLE PRECISION,
    rolling_latency_ms DOUBLE PRECISION,
    last_http_status INTEGER,
    circuit_open_until TIMESTAMPTZ,
    etag TEXT,
    last_modified TEXT,
    last_error TEXT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_source_runtime_due
ON source_runtime_state(next_due_at, circuit_open_until);
