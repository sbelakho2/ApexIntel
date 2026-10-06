-- Migration 109: analytical quality ledger, prediction ledger, quality snapshots
-- ════════════════════════════════════════════════════════════════════════════
-- Persists the analytical-excellence review of every generated product:
-- depth dimensions, factuality, argumentation warrants, red-team flags and
-- the editorial verdict.  Three companion tables make the pipeline
-- measurable, self-calibrating and model-portable over years of growth:
--
--   * analytical_quality_scores  — per-product review, model-stamped so a
--     replacement LLM can be compared against its predecessor on identical
--     standards.  Raw rows are retention-pruned (see the function below) while
--     weekly snapshots keep the long-horizon trend forever.
--   * insight_predictions        — the prediction ledger used to fit
--     calibration curves (Brier/ECE) from resolved outcomes; the curve is what
--     turns stated confidence into measured confidence.
--   * analytical_quality_snapshots — weekly aggregates (depth/factuality/
--     warrant distributions, verdict mix, calibration error) for trend gates
--     and long-term storage.
--   * llm_model_registry         — eval-gated model onboarding: a candidate
--     model's scores are recorded before it is promoted to active, so model
--     swaps are measured changes, never blind jumps.

CREATE TABLE IF NOT EXISTS analytical_quality_scores (
    id                      UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    insight_id              UUID REFERENCES insights(id) ON DELETE SET NULL,
    entity_id               UUID,
    recipe_code             TEXT,
    -- Model/prompt provenance: makes cross-model comparisons exact.
    model_id                TEXT,
    prompt_version          TEXT,
    depth_index             DOUBLE PRECISION NOT NULL,
    depth_tier              TEXT NOT NULL,
    depth_components        JSONB NOT NULL DEFAULT '[]',
    factuality_score        DOUBLE PRECISION NOT NULL,
    warrant_overall         DOUBLE PRECISION NOT NULL,
    warrant_weakest         DOUBLE PRECISION NOT NULL,
    source_independence     DOUBLE PRECISION NOT NULL,
    evidence_cap            DOUBLE PRECISION NOT NULL,
    stated_confidence       DOUBLE PRECISION NOT NULL,
    final_confidence        DOUBLE PRECISION NOT NULL,
    hard_violations         BIGINT NOT NULL DEFAULT 0,
    soft_flags              JSONB NOT NULL DEFAULT '[]',
    verdict                 TEXT NOT NULL,
    reasons                 TEXT[] NOT NULL DEFAULT '{}',
    created_at              TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_analytical_quality_created
    ON analytical_quality_scores (created_at DESC);
CREATE INDEX IF NOT EXISTS idx_analytical_quality_verdict
    ON analytical_quality_scores (verdict, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_analytical_quality_recipe
    ON analytical_quality_scores (recipe_code, created_at DESC)
    WHERE recipe_code IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_analytical_quality_model
    ON analytical_quality_scores (model_id, created_at DESC)
    WHERE model_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_analytical_quality_entity
    ON analytical_quality_scores (entity_id, created_at DESC)
    WHERE entity_id IS NOT NULL;

-- Year-scale retention: raw per-product reviews age out, weekly snapshots
-- (below) keep the trend permanently. Idempotent and safe to schedule.
CREATE OR REPLACE FUNCTION prune_analytical_quality_scores(max_age_days integer)
RETURNS bigint
LANGUAGE plpgsql
AS $$
DECLARE
    deleted bigint;
BEGIN
    IF max_age_days IS NULL OR max_age_days < 30 THEN
        RAISE EXCEPTION 'retention floor is 30 days (got %)', max_age_days;
    END IF;
    DELETE FROM analytical_quality_scores
    WHERE created_at < now() - make_interval(days => max_age_days);
    GET DIAGNOSTICS deleted = ROW_COUNT;
    RETURN deleted;
END;
$$;

CREATE TABLE IF NOT EXISTS insight_predictions (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    insight_id          UUID REFERENCES insights(id) ON DELETE SET NULL,
    entity_id           UUID,
    recipe_code         TEXT,
    statement           TEXT NOT NULL,
    probability         DOUBLE PRECISION NOT NULL,
    -- Prediction horizon; resolution compares the entity's observation stream
    -- against this window.
    resolve_by          TIMESTAMPTZ NOT NULL,
    resolved_at         TIMESTAMPTZ,
    resolved_outcome    BOOLEAN,
    brier               DOUBLE PRECISION,
    -- Set when the horizon passed without a resolvable observation; expired
    -- predictions never enter calibration.
    expired_at          TIMESTAMPTZ,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_insight_predictions_due
    ON insight_predictions (resolve_by)
    WHERE resolved_outcome IS NULL AND expired_at IS NULL;
CREATE INDEX IF NOT EXISTS idx_insight_predictions_entity
    ON insight_predictions (entity_id, created_at DESC)
    WHERE entity_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_insight_predictions_recipe
    ON insight_predictions (recipe_code, created_at DESC)
    WHERE recipe_code IS NOT NULL;

CREATE TABLE IF NOT EXISTS analytical_quality_snapshots (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    period_start    DATE NOT NULL UNIQUE,  -- ISO week start (Monday, UTC)
    sample_size     BIGINT NOT NULL,
    snapshot        JSONB NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_analytical_quality_snapshots_period
    ON analytical_quality_snapshots (period_start DESC);

-- Model registry: eval-gated onboarding so replacing the LLM is a measured,
-- reversible change (candidate → active → deprecated), with the candidate's
-- measured scores retained for every comparison.
CREATE TABLE IF NOT EXISTS llm_model_registry (
    id                  TEXT PRIMARY KEY,          -- model identifier as routed
    provider            TEXT NOT NULL DEFAULT 'unknown',
    display_name        TEXT NOT NULL DEFAULT '',
    context_window      BIGINT,
    status              TEXT NOT NULL DEFAULT 'candidate',
    eval_scores         JSONB NOT NULL DEFAULT '{}',
    calibration         JSONB,
    activated_at        TIMESTAMPTZ,
    deprecated_at       TIMESTAMPTZ,
    notes               TEXT,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_llm_model_registry_status
    ON llm_model_registry (status, updated_at DESC);
