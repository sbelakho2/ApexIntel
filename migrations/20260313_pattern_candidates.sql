-- Pattern candidates table for the mining pipeline.
-- Each row represents a pattern candidate discovered by the nightly
-- pattern_mining job.  The analytics layer only reads COUNT(*)
-- with filters on created_at and passed_gates.

CREATE TABLE IF NOT EXISTS pattern_candidates (
    id              BIGSERIAL       PRIMARY KEY,
    recipe_code     TEXT,
    entity_type     TEXT            NOT NULL DEFAULT 'unknown',
    pattern_label   TEXT,
    passed_gates    BOOLEAN         NOT NULL DEFAULT false,
    confidence      DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    created_at      TIMESTAMPTZ     NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_pattern_candidates_created_at
    ON pattern_candidates (created_at);
CREATE INDEX IF NOT EXISTS idx_pattern_candidates_passed_gates
    ON pattern_candidates (passed_gates) WHERE passed_gates = true;
