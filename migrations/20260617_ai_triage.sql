-- AI Triage Engine — Phase 2.2
-- Adds triage queue, decisions, and model configuration tables.

-- ─── Triage Queue ──────────────────────────────────────────────────────────
-- Items awaiting or having undergone LLM-based triage scoring.
CREATE TABLE IF NOT EXISTS triage_queue (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    item_type       TEXT NOT NULL CHECK (item_type IN ('insight', 'warning', 'alert')),
    source_id       UUID NOT NULL,
    title           TEXT NOT NULL,
    description     TEXT NOT NULL DEFAULT '',
    entity_id       UUID,
    entity_name     TEXT,
    static_severity TEXT,

    -- LLM triage dimensions (JSONB for flexibility)
    dimensions      JSONB,
    composite_score DOUBLE PRECISION NOT NULL DEFAULT 0.0,

    -- Override fields
    is_overridden   BOOLEAN NOT NULL DEFAULT FALSE,
    override_score  DOUBLE PRECISION,

    -- Status tracking
    status          TEXT NOT NULL DEFAULT 'pending'
                        CHECK (status IN ('pending', 'triaged', 'acknowledged', 'resolved', 'dismissed')),

    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    triaged_at      TIMESTAMPTZ,
    acknowledged_at TIMESTAMPTZ,

    UNIQUE(item_type, source_id)
);

-- Index for priority-ordered queue queries
CREATE INDEX IF NOT EXISTS idx_triage_queue_composite
    ON triage_queue (composite_score DESC, created_at DESC)
    WHERE status = 'pending' OR status = 'triaged';

-- Index for unscored items (worker fetches these)
CREATE INDEX IF NOT EXISTS idx_triage_queue_unscored
    ON triage_queue (created_at ASC)
    WHERE composite_score = 0.0 AND status = 'pending';

-- Index for source lookups
CREATE INDEX IF NOT EXISTS idx_triage_queue_source ON triage_queue (item_type, source_id);
CREATE INDEX IF NOT EXISTS idx_triage_queue_entity  ON triage_queue (entity_id) WHERE entity_id IS NOT NULL;

-- ─── Triage Decisions (History) ────────────────────────────────────────────
-- Every triage decision, including human overrides, for audit and feedback.
CREATE TABLE IF NOT EXISTS triage_decisions (
    id                   UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    queue_item_id        UUID NOT NULL REFERENCES triage_queue(id) ON DELETE CASCADE,
    original_dimensions  JSONB NOT NULL,
    original_composite   DOUBLE PRECISION NOT NULL,
    override_dimensions  JSONB,
    override_composite   DOUBLE PRECISION,
    overridden_by        TEXT,  -- user_id who overrode
    overridden_at        TIMESTAMPTZ,
    decision_type        TEXT NOT NULL DEFAULT 'auto_triage'
                             CHECK (decision_type IN ('auto_triage', 'user_override')),
    created_at           TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_triage_decisions_queue ON triage_decisions (queue_item_id);
CREATE INDEX IF NOT EXISTS idx_triage_decisions_created ON triage_decisions (created_at DESC);
CREATE INDEX IF NOT EXISTS idx_triage_decisions_item ON triage_decisions (queue_item_id);

-- ─── Triage Model Configuration ───────────────────────────────────────────
-- Optional: stored model config/weights for the triage LLM.
CREATE TABLE IF NOT EXISTS triage_models (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    model_name  TEXT NOT NULL,
    weights     JSONB NOT NULL DEFAULT '{}',
    is_active   BOOLEAN NOT NULL DEFAULT FALSE,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
