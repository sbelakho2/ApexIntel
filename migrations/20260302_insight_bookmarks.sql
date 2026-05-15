-- ════════════════════════════════════════════════════════════════════════════
-- Insight Bookmarks
-- Allows users to bookmark/save insights for quick retrieval.
-- ════════════════════════════════════════════════════════════════════════════

BEGIN;

CREATE TABLE IF NOT EXISTS insight_bookmarks (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    insight_id  UUID NOT NULL REFERENCES insights(id) ON DELETE CASCADE,
    user_id     TEXT NOT NULL,
    note        TEXT,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(insight_id, user_id)
);

CREATE INDEX IF NOT EXISTS idx_bookmarks_user      ON insight_bookmarks(user_id);
CREATE INDEX IF NOT EXISTS idx_bookmarks_insight    ON insight_bookmarks(insight_id);
CREATE INDEX IF NOT EXISTS idx_bookmarks_created    ON insight_bookmarks(created_at DESC);

COMMIT;
