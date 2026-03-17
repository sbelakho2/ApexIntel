-- Performance indexes for frequently queried columns that lack coverage.
--
-- Observations:
-- - insert_insight CTE filters on title_hash, entity_ids, insight_type, created_at
-- - list_insights ORDER BY updated_at DESC
-- - list_warnings uses ROW_NUMBER() OVER (PARTITION BY ... ORDER BY created_at DESC)
-- - get_warnings_by_entity_ids uses entity_ids && $1 (GIN array overlap)
-- - count_insights uses CONCAT_WS on (title, insight_type, region)
-- - analytics queries unnest(entity_ids) with warnings and insights

-- ── insights ────────────────────────────────────────────────────────────────

-- Speed up the insert_insight dedup CTE (title_hash exact match path).
CREATE INDEX IF NOT EXISTS idx_insights_title_hash
    ON insights(title_hash)
    WHERE title_hash IS NOT NULL;

-- Speed up list_insights ORDER BY updated_at DESC, id ASC.
CREATE INDEX IF NOT EXISTS idx_insights_updated_at
    ON insights(updated_at DESC, id ASC);

-- Speed up insert_insight 21-day recency dedup path (entity_ids + type + created_at).
CREATE INDEX IF NOT EXISTS idx_insights_dedup_window
    ON insights(insight_type, created_at DESC)
    WHERE entity_ids IS NOT NULL;

-- Speed up get_related_insights and get_insights_by_entity_ids (array overlap).
CREATE INDEX IF NOT EXISTS idx_insights_entity_ids_gin
    ON insights USING gin(entity_ids)
    WHERE entity_ids IS NOT NULL;

-- ── warnings ────────────────────────────────────────────────────────────────

-- Speed up get_warnings_by_entity_ids (array overlap).
CREATE INDEX IF NOT EXISTS idx_warnings_entity_ids_gin
    ON warnings USING gin(entity_ids)
    WHERE entity_ids IS NOT NULL;

-- Speed up list_warnings / count_warnings that filter on deleted_at IS NULL.
CREATE INDEX IF NOT EXISTS idx_warnings_active_type_severity
    ON warnings(warning_type, severity, created_at DESC)
    WHERE deleted_at IS NULL;

-- Speed up analytics unnest(entity_ids) queries on non-deleted warnings.
CREATE INDEX IF NOT EXISTS idx_warnings_created_at_active
    ON warnings(created_at DESC)
    WHERE deleted_at IS NULL;

-- ── observations ────────────────────────────────────────────────────────────

-- Speed up analytics aggregate_source_reliability_outcomes which joins
-- observations on entity_id with a 30-day window.
CREATE INDEX IF NOT EXISTS idx_obs_entity_ts
    ON observations(entity_id, ts_utc DESC);
