-- ════════════════════════════════════════════════════════════════════════════
-- Migration 089: split recipe configuration from measured quality
-- ════════════════════════════════════════════════════════════════════════════
--
-- `recipes.precision_score` conflated two different quantities:
--
--   * the configured detection threshold seeded from a recipe definition
--     (`min_precision` / `precision` / `min_stability` / `min_effect` keys),
--     which `auto_calibrate_recipe_thresholds()` mutates; and
--   * measured recipe precision (TP / (TP + FP)) that readers increasingly
--     assumed it to be.
--
-- A learned threshold therefore looked like measured quality, and a measured
-- quality write would have corrupted the runtime detection threshold.
--
-- This migration separates them:
--
--   * `configured_min_precision` — the recipe's configured minimum precision
--     (renamed from `precision_score`, preserving existing values);
--   * `activation_threshold` — the runtime threshold the auto-calibrator is
--     allowed to move (seeded from the configured value).
--
-- Measured precision lives only in `recipe_weekly_metrics.precision_score`
-- (NULL when unreviewed) and in queries that compute TP/(TP+FP) directly.
--
-- Idempotent: safe to re-apply.

DO $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_name = 'recipes' AND column_name = 'precision_score'
    ) AND NOT EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_name = 'recipes' AND column_name = 'configured_min_precision'
    ) THEN
        ALTER TABLE recipes RENAME COLUMN precision_score TO configured_min_precision;
    END IF;
END $$;

ALTER TABLE recipes
    ADD COLUMN IF NOT EXISTS activation_threshold DOUBLE PRECISION;

-- Backfill the configured value from the current-schema column when the
-- legacy alias was empty (the schema-unification migration kept `precision`
-- and `precision_score` in sync, but a row could exist with only one).
UPDATE recipes
SET configured_min_precision = precision
WHERE configured_min_precision IS NULL
  AND precision IS NOT NULL;

-- Seed the runtime threshold from the configured value exactly once.
UPDATE recipes
SET activation_threshold = configured_min_precision
WHERE activation_threshold IS NULL
  AND configured_min_precision IS NOT NULL;

COMMENT ON COLUMN recipes.configured_min_precision IS
    'Configured minimum precision from the recipe definition (configuration, not measured quality)';
COMMENT ON COLUMN recipes.activation_threshold IS
    'Runtime activation threshold the auto-calibrator may adjust; never measured quality. Measured precision lives in recipe_weekly_metrics';

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
