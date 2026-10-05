-- Migration 105: enforce the insight dedup index every deployment needs
-- ════════════════════════════════════════════════════════════════════════════
-- `idx_insights_dedup` (title_hash, COALESCE(entity_ids, '{}')) existed only as
-- an ad-hoc index on one production database: the migration chain never
-- created it. Two consequences:
--
--   1. Fresh deployments silently lost the database-level guarantee that the
--      same insight for the same entities cannot be stored twice.
--   2. `insert_insight`'s new ON CONFLICT target requires the index to exist,
--      so the chain must create it before the code can rely on it.
--
-- The race the index closes: two concurrent synthesis paths both miss the
-- CTE's `existing` probe and insert the same (title_hash, entity_ids) pair;
-- the loser previously failed the whole run with a duplicate-key error.
--
-- Pre-existing duplicates (possible where the index was absent) are pruned
-- first, keeping the newest row per pair; dependent claims/bookmarks cascade
-- from the deleted row as usual.

DO $$
DECLARE
    removed bigint;
BEGIN
    DELETE FROM insights a
    USING insights b
    WHERE a.id <> b.id
      AND a.title_hash IS NOT NULL
      AND a.title_hash = b.title_hash
      AND COALESCE(a.entity_ids, ARRAY[]::uuid[]) = COALESCE(b.entity_ids, ARRAY[]::uuid[])
      AND (
            a.created_at < b.created_at
         OR (a.created_at = b.created_at AND a.id < b.id)
      );
    GET DIAGNOSTICS removed = ROW_COUNT;
    IF removed > 0 THEN
        RAISE NOTICE 'migration 105: removed % duplicate insight row(s)', removed;
    END IF;
END $$;

CREATE UNIQUE INDEX IF NOT EXISTS idx_insights_dedup
    ON insights (title_hash, COALESCE(entity_ids, ARRAY[]::uuid[]));
