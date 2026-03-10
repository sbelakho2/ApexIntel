ALTER TABLE warnings
    ADD COLUMN IF NOT EXISTS review_outcome TEXT,
    ADD COLUMN IF NOT EXISTS reviewed_by TEXT,
    ADD COLUMN IF NOT EXISTS reviewed_at TIMESTAMPTZ;

ALTER TABLE warnings
    DROP CONSTRAINT IF EXISTS warnings_review_outcome_check;

ALTER TABLE warnings
    ADD CONSTRAINT warnings_review_outcome_check
    CHECK (review_outcome IS NULL OR review_outcome IN ('true_positive', 'false_positive'));

CREATE TABLE IF NOT EXISTS recipe_weekly_metrics (
    recipe_code TEXT NOT NULL REFERENCES recipes(code) ON DELETE CASCADE,
    week_start DATE NOT NULL,
    precision_score DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    false_positive_rate DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    warnings_generated BIGINT NOT NULL DEFAULT 0,
    reviewed_warnings BIGINT NOT NULL DEFAULT 0,
    false_positive_warnings BIGINT NOT NULL DEFAULT 0,
    snapshot_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (recipe_code, week_start)
);

CREATE INDEX IF NOT EXISTS idx_recipe_weekly_metrics_recipe_week
    ON recipe_weekly_metrics(recipe_code, week_start DESC);

CREATE INDEX IF NOT EXISTS idx_recipe_weekly_metrics_week
    ON recipe_weekly_metrics(week_start DESC);