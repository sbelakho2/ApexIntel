-- Migration 108: insight_correlations — persisted cross-signal correlation mining results
-- ════════════════════════════════════════════════════════════════════════════
-- The correlation engines (crates/insights/src/correlation.rs,
-- cross_entity_correlation.rs) were compute-only; discovered correlations were
-- logged and dropped, so the recipe pipeline could never deepen from them.
-- This table persists the weekly correlation-mining pass so (a) the audit
-- trail shows every discovered correlation, and (b) top correlations are
-- staged as deep-insight recipe candidates.

CREATE TABLE IF NOT EXISTS insight_correlations (
    id                      UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    correlation_kind        TEXT NOT NULL,
    signal_domain_a         TEXT NOT NULL,
    signal_domain_b         TEXT NOT NULL,
    entity_a                TEXT,
    entity_b                TEXT,
    -- Effect direction and strength of the measured association.
    strength                DOUBLE PRECISION NOT NULL,
    p_value                 DOUBLE PRECISION,
    lag_days                INTEGER,
    evidence_count          BIGINT NOT NULL DEFAULT 0,
    -- 'discovered' | 'staged' | 'promoted' | 'dismissed'
    status                  TEXT NOT NULL DEFAULT 'discovered',
    -- Which recipes were deepened from this correlation (recipe codes).
    derived_recipe_codes    TEXT[] NOT NULL DEFAULT '{}',
    first_seen_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    created_at              TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at              TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_insight_correlations_identity
    ON insight_correlations (correlation_kind, signal_domain_a, signal_domain_b,
                             COALESCE(entity_a, ''), COALESCE(entity_b, ''));

CREATE INDEX IF NOT EXISTS idx_insight_correlations_status
    ON insight_correlations (status, last_seen_at DESC);

CREATE INDEX IF NOT EXISTS idx_insight_correlations_strength
    ON insight_correlations (strength DESC)
    WHERE status = 'discovered';
