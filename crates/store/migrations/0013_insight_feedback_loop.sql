BEGIN;

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

COMMIT;