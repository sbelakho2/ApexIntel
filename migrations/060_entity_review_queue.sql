-- 058_entity_review_queue.sql
--
-- Entity admission review queue (P0 entity-admission audit).
--
-- Dynamic discovery must never create a canonical company that was not
-- evidence-verified. Candidates that fail verification (no registry anchor,
-- insufficient corroboration, contradictory evidence, provider outage) are
-- queued here for analyst review instead. `candidate_id` is the deterministic
-- UUID of the candidate (derived from its normalized name), so repeated
-- nightly passes reuse one pending row instead of flooding the queue.
--
-- Idempotent (IF NOT EXISTS guards); does not touch any applied migration
-- (<= 057).

CREATE TABLE IF NOT EXISTS entity_review_queue (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    candidate_id    UUID NOT NULL,
    candidate_name  TEXT NOT NULL,
    normalized_name TEXT NOT NULL,
    confidence      DOUBLE PRECISION NOT NULL DEFAULT 0,
    outcome         TEXT NOT NULL DEFAULT 'analyst_review',
    review_reasons  TEXT[] NOT NULL DEFAULT '{}',
    evidence        JSONB NOT NULL DEFAULT '[]'::jsonb,
    metadata        JSONB NOT NULL DEFAULT '{}'::jsonb,
    source          TEXT NOT NULL DEFAULT 'dynamic_discovery',
    status          TEXT NOT NULL DEFAULT 'pending',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    resolved_at     TIMESTAMPTZ
);

-- At most one pending review per candidate (deterministic candidate id), so
-- every nightly re-evaluation is idempotent.
CREATE UNIQUE INDEX IF NOT EXISTS idx_entity_review_queue_pending_candidate
    ON entity_review_queue (candidate_id)
    WHERE status = 'pending';

-- Analyst worklist ordering.
CREATE INDEX IF NOT EXISTS idx_entity_review_queue_status_created
    ON entity_review_queue (status, created_at DESC);

-- The application connects as a non-owner role in production; new tables need
-- explicit grants (same pattern as 050_heartbeat_grants.sql).
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'apexintel') THEN
        GRANT SELECT, INSERT, UPDATE, DELETE ON entity_review_queue TO apexintel;
    END IF;
END $$;
