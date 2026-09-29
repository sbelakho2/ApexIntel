-- ════════════════════════════════════════════════════════════════════════════
-- Migration 092: configured_activation_threshold (rollout-safe addition)
-- ════════════════════════════════════════════════════════════════════════════
--
-- `configured_min_precision` is configuration named as if it were measured
-- quality, and migration 089 split the measured precision out of it. The
-- runtime now reads `configured_activation_threshold`.
--
-- The rename is staged rather than in place:
--
--   * this migration ADDS `configured_activation_threshold` and backfills it
--     from `configured_min_precision`;
--   * `configured_min_precision` is left in place, so old binaries keep
--     querying it during a rolling deploy and a rollback to the previous image
--     remains possible (the column is NOT NULL DEFAULT 0.0, so new-binary
--     inserts that omit it still succeed);
--   * a follow-up migration can drop `configured_min_precision` once no old
--     binary is running.
--
-- Idempotent: safe to re-apply.

ALTER TABLE recipes
    ADD COLUMN IF NOT EXISTS configured_activation_threshold DOUBLE PRECISION;

DO $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_name = 'recipes' AND column_name = 'configured_min_precision'
    ) THEN
        EXECUTE 'UPDATE recipes
                    SET configured_activation_threshold = configured_min_precision
                  WHERE configured_activation_threshold IS NULL
                    AND configured_min_precision IS NOT NULL';
    END IF;
END $$;

COMMENT ON COLUMN recipes.configured_activation_threshold IS
    'Configured activation threshold (configuration, used only until calibration writes activation_threshold); never measured precision';

COMMENT ON COLUMN recipes.configured_min_precision IS
    'Legacy configuration alias superseded by configured_activation_threshold (migration 092); retained for rolling-deploy/rollback compatibility, drop in a follow-up migration';

-- Runtime role grants (production connects as a non-owner role).
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'apexintel_app') THEN
        GRANT SELECT, INSERT, UPDATE, DELETE ON recipes TO apexintel_app;
    END IF;
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'apexintel') THEN
        GRANT SELECT, INSERT, UPDATE, DELETE ON recipes TO apexintel;
    END IF;
END $$;
