-- ──────────────────────────────────────────────────────────────────────────────
-- Migration 051: measurable learning — versioned evaluation metrics (P0 #38)
--
-- The improvement loop previously compared candidate rules/models/prompts
-- against whatever data happened to be around, and mixed explicit analyst
-- confirmations with passive dismissal clicks and workflow convenience
-- actions. That makes "improvement" unfalsifiable.
--
-- This migration introduces the persistence layer for a measurable gate:
--
--   learning_eval_sets    — frozen, versioned evaluation sets. A set is
--                           immutable once created; new examples require a new
--                           (name, version) row. A trigger rejects UPDATE and
--                           DELETE on this table.
--   learning_eval_runs    — one candidate evaluation against one frozen set,
--                           carrying the promotion decision. A partial unique
--                           index guarantees at most one `promoted` run per
--                           (set, candidate kind, candidate ref).
--   learning_eval_metrics — the metrics recorded for each run:
--                             precision, false_positive_rate, duplicate_rate,
--                             grounding_failure_rate, analyst_acceptance_rate,
--                             analyst_dismissal_rate, time_to_action_hours,
--                             source_yield, entity_linking_accuracy
--                           Each metric row carries the analyst-signal class it
--                           was computed from. Only `positive_confirmation` is
--                           training truth; `dismissal_noise` and
--                           `workflow_convenience` are recorded for diagnosis
--                           but never count as evidence for a promotion.
--
-- Idempotent: safe to re-run (IF NOT EXISTS / CREATE OR REPLACE / guarded
-- triggers and grants).
-- ──────────────────────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS learning_eval_sets (
    id            UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    name          TEXT        NOT NULL,
    version       INTEGER     NOT NULL CHECK (version >= 1),
    description   TEXT,
    example_count INTEGER     NOT NULL DEFAULT 0 CHECK (example_count >= 0),
    frozen_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    frozen_by     TEXT,
    metadata      JSONB       NOT NULL DEFAULT '{}'::jsonb,
    CONSTRAINT learning_eval_sets_name_version_key UNIQUE (name, version)
);

COMMENT ON TABLE learning_eval_sets IS
    'Frozen, versioned evaluation sets. Immutable: a trigger rejects UPDATE/DELETE.';
COMMENT ON COLUMN learning_eval_sets.version IS
    'Monotonic version within a set name; new examples require a new version.';
COMMENT ON COLUMN learning_eval_sets.frozen_at IS
    'Timestamp the set contents were frozen; every run records the set it used.';

CREATE TABLE IF NOT EXISTS learning_eval_runs (
    id                UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    eval_set_id       UUID        NOT NULL REFERENCES learning_eval_sets(id) ON DELETE RESTRICT,
    candidate_kind    TEXT        NOT NULL
        CHECK (candidate_kind IN ('rule', 'model', 'prompt', 'recipe', 'threshold')),
    candidate_ref     TEXT        NOT NULL,
    candidate_version TEXT,
    baseline_run_id   UUID        REFERENCES learning_eval_runs(id) ON DELETE SET NULL,
    status            TEXT        NOT NULL DEFAULT 'candidate'
        CHECK (status IN ('candidate', 'promoted', 'rejected', 'rolled_back')),
    decision          TEXT        CHECK (decision IN ('promote', 'reject')),
    decision_reason   TEXT,
    metrics_version   INTEGER     NOT NULL DEFAULT 1 CHECK (metrics_version >= 1),
    sample_size       INTEGER     NOT NULL DEFAULT 0 CHECK (sample_size >= 0),
    metrics_snapshot  JSONB       NOT NULL DEFAULT '{}'::jsonb,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    decided_at        TIMESTAMPTZ
);

COMMENT ON TABLE learning_eval_runs IS
    'One candidate evaluation against a frozen learning_eval_set, with its promotion decision.';
COMMENT ON COLUMN learning_eval_runs.metrics_version IS
    'Version of the metric schema/gate used for this run (bump when the gate changes).';

CREATE INDEX IF NOT EXISTS idx_learning_eval_runs_candidate
    ON learning_eval_runs (candidate_kind, candidate_ref, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_learning_eval_runs_eval_set
    ON learning_eval_runs (eval_set_id, created_at DESC);

-- At most one promoted run per (frozen set, candidate kind, candidate ref).
CREATE UNIQUE INDEX IF NOT EXISTS uq_learning_eval_runs_promoted
    ON learning_eval_runs (eval_set_id, candidate_kind, candidate_ref)
    WHERE status = 'promoted';

CREATE TABLE IF NOT EXISTS learning_eval_metrics (
    id                UUID             PRIMARY KEY DEFAULT gen_random_uuid(),
    run_id            UUID             NOT NULL REFERENCES learning_eval_runs(id) ON DELETE CASCADE,
    metric            TEXT             NOT NULL
        CHECK (metric IN (
            'precision',
            'false_positive_rate',
            'duplicate_rate',
            'grounding_failure_rate',
            'analyst_acceptance_rate',
            'analyst_dismissal_rate',
            'time_to_action_hours',
            'source_yield',
            'entity_linking_accuracy'
        )),
    value             DOUBLE PRECISION NOT NULL,
    sample_size       INTEGER          NOT NULL CHECK (sample_size >= 0),
    signal_class      TEXT             NOT NULL DEFAULT 'positive_confirmation'
        CHECK (signal_class IN ('positive_confirmation', 'dismissal_noise', 'workflow_convenience')),
    is_critical       BOOLEAN          NOT NULL DEFAULT FALSE,
    is_training_truth BOOLEAN          NOT NULL DEFAULT FALSE,
    metadata          JSONB            NOT NULL DEFAULT '{}'::jsonb,
    created_at        TIMESTAMPTZ      NOT NULL DEFAULT now(),
    CONSTRAINT learning_eval_metrics_run_metric_class_key
        UNIQUE (run_id, metric, signal_class),
    -- Only explicit positive confirmation may ever be marked training truth.
    CONSTRAINT learning_eval_metrics_truth_class_check
        CHECK (is_training_truth = (signal_class = 'positive_confirmation'))
);

COMMENT ON TABLE learning_eval_metrics IS
    'Per-run evaluation metrics, versioned by run + metrics_version.';
COMMENT ON COLUMN learning_eval_metrics.signal_class IS
    'Analyst-signal class the metric was computed from: positive_confirmation (truth), dismissal_noise, workflow_convenience.';
COMMENT ON COLUMN learning_eval_metrics.is_training_truth IS
    'True only for positive_confirmation rows; enforced by a CHECK constraint.';

CREATE INDEX IF NOT EXISTS idx_learning_eval_metrics_run
    ON learning_eval_metrics (run_id);

CREATE INDEX IF NOT EXISTS idx_learning_eval_metrics_truth
    ON learning_eval_metrics (metric, created_at DESC)
    WHERE is_training_truth;

-- Read-only convenience: metrics that may be used as promotion evidence.
CREATE OR REPLACE VIEW learning_training_truth_metrics AS
    SELECT *
    FROM learning_eval_metrics
    WHERE is_training_truth;

COMMENT ON VIEW learning_training_truth_metrics IS
    'learning_eval_metrics restricted to positive_confirmation rows (the only training truth).';

-- ── Freeze guard: evaluation sets are append-only ────────────────────────────
CREATE OR REPLACE FUNCTION learning_eval_sets_freeze_guard()
RETURNS trigger AS $$
BEGIN
    RAISE EXCEPTION
        'learning_eval_sets rows are frozen; create a new (name, version) instead';
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_learning_eval_sets_freeze ON learning_eval_sets;
CREATE TRIGGER trg_learning_eval_sets_freeze
    BEFORE UPDATE OR DELETE ON learning_eval_sets
    FOR EACH ROW EXECUTE FUNCTION learning_eval_sets_freeze_guard();

-- ── Grants for the application role (same pattern as 049/050) ────────────────
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'apexintel') THEN
        GRANT SELECT, INSERT, UPDATE, DELETE ON learning_eval_sets TO apexintel;
        GRANT SELECT, INSERT, UPDATE, DELETE ON learning_eval_runs TO apexintel;
        GRANT SELECT, INSERT, UPDATE, DELETE ON learning_eval_metrics TO apexintel;
        GRANT SELECT ON learning_training_truth_metrics TO apexintel;
    END IF;
END $$;
