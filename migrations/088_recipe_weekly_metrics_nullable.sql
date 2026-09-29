-- ════════════════════════════════════════════════════════════════════════════
-- Migration 088: recipe_weekly_metrics keeps unknown quality unknown
-- ════════════════════════════════════════════════════════════════════════════
--
-- `precision_score` and `false_positive_rate` were NOT NULL DEFAULT 0.0, so a
-- week with no reviewed outcomes was persisted as a measured 0% — which the
-- (now real) historical chart would then plot as genuine history. Both columns
-- become nullable: NULL means "no reviewed sample this week".
--
-- Existing all-zero rows without any reviewed warnings are demoted to NULL
-- (they were written by the old default, not by a measurement). Rows that had
-- reviews keep their values.
--
-- Idempotent: safe to re-apply.

ALTER TABLE recipe_weekly_metrics
    ALTER COLUMN precision_score DROP NOT NULL,
    ALTER COLUMN precision_score DROP DEFAULT,
    ALTER COLUMN false_positive_rate DROP NOT NULL,
    ALTER COLUMN false_positive_rate DROP DEFAULT;

UPDATE recipe_weekly_metrics
SET precision_score = NULL
WHERE COALESCE(reviewed_warnings, 0) = 0
  AND precision_score = 0.0;

UPDATE recipe_weekly_metrics
SET false_positive_rate = NULL
WHERE COALESCE(reviewed_warnings, 0) = 0
  AND false_positive_rate = 0.0;

COMMENT ON COLUMN recipe_weekly_metrics.precision_score IS
    'Empirical precision TP/(TP+FP) over reviewed warnings; NULL = nothing reviewed (never a synthesized value)';
COMMENT ON COLUMN recipe_weekly_metrics.false_positive_rate IS
    'False-positive rate over reviewed warnings; NULL = nothing reviewed';

-- Runtime role grants (production connects as a non-owner role).
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'apexintel_app') THEN
        GRANT SELECT, INSERT, UPDATE, DELETE ON recipe_weekly_metrics TO apexintel_app;
    END IF;
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'apexintel') THEN
        GRANT SELECT, INSERT, UPDATE, DELETE ON recipe_weekly_metrics TO apexintel;
    END IF;
END $$;
