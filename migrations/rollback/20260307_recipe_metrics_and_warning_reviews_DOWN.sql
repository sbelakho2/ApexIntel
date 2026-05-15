-- Rollback: 20260307_recipe_metrics_and_warning_reviews
-- Drops the recipe_weekly_metrics table and reverts warning review columns.

DROP INDEX IF EXISTS idx_recipe_weekly_metrics_week;
DROP INDEX IF EXISTS idx_recipe_weekly_metrics_recipe_week;
DROP TABLE IF EXISTS recipe_weekly_metrics;

ALTER TABLE warnings DROP CONSTRAINT IF EXISTS warnings_review_outcome_check;
ALTER TABLE warnings DROP COLUMN IF EXISTS review_outcome;
ALTER TABLE warnings DROP COLUMN IF EXISTS reviewed_by;
ALTER TABLE warnings DROP COLUMN IF EXISTS reviewed_at;
