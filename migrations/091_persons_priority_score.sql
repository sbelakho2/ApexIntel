-- ════════════════════════════════════════════════════════════════════════════
-- Migration 091: persons.priority_score — canonical priority, filterable in SQL
-- ════════════════════════════════════════════════════════════════════════════
--
-- The POI model computes priority as the weighted composite of the stored
-- `priority_vector`; `influence_score` is a separate measurement. Several SQL
-- paths still filtered/ordered "priority" by `influence_score`, which
-- contradicted the canonical model and reintroduced unknown-influence-as-zero.
--
-- This adds a nullable `priority_score` column mirroring the composite exactly
-- as the Rust model computes it (default weights sum to 1.0, each vector field
-- required; a partial vector yields NULL, never a zero score). Application
-- writes keep it synchronized whenever `priority_vector` is written.
--
-- Idempotent: safe to re-apply.

ALTER TABLE persons
    ADD COLUMN IF NOT EXISTS priority_score DOUBLE PRECISION;

-- One-time backfill. Runtime writes compute the score with the single Rust
-- implementation (`apex_core::priority`), so this SQL expression exists only
-- for pre-existing rows; keep it in sync with DEFAULT_PRIORITY_WEIGHTS.
-- The expression mirrors StoredPriorityVector::composite_with_weights with the
-- default weights (0.25/0.20/0.20/0.15/0.20, total 1.0). A vector missing any
-- field deserializes to None in Rust, so the SQL must yield NULL too.
UPDATE persons
SET priority_score = LEAST(
        1.0,
        GREATEST(
            0.0,
            0.25 * (priority_vector ->> 'decision_power')::DOUBLE PRECISION
                + 0.20 * (priority_vector ->> 'domain_relevance')::DOUBLE PRECISION
                + 0.20 * (priority_vector ->> 'network_centrality')::DOUBLE PRECISION
                + 0.15 * (priority_vector ->> 'engagement_potential')::DOUBLE PRECISION
                + 0.20 * (priority_vector ->> 'intelligence_value')::DOUBLE PRECISION
        )
    )
WHERE priority_vector IS NOT NULL
  AND priority_vector ? 'decision_power'
  AND priority_vector ? 'domain_relevance'
  AND priority_vector ? 'network_centrality'
  AND priority_vector ? 'engagement_potential'
  AND priority_vector ? 'intelligence_value'
  AND jsonb_typeof(priority_vector -> 'decision_power') = 'number'
  AND jsonb_typeof(priority_vector -> 'domain_relevance') = 'number'
  AND jsonb_typeof(priority_vector -> 'network_centrality') = 'number'
  AND jsonb_typeof(priority_vector -> 'engagement_potential') = 'number'
  AND jsonb_typeof(priority_vector -> 'intelligence_value') = 'number';

CREATE INDEX IF NOT EXISTS idx_persons_priority_score
    ON persons (priority_score DESC NULLS LAST);

COMMENT ON COLUMN persons.priority_score IS
    'Weighted composite of priority_vector (canonical priority). NULL = not measured; never derived from influence_score';

-- Runtime role grants (production connects as a non-owner role).
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'apexintel_app') THEN
        GRANT SELECT, INSERT, UPDATE, DELETE ON persons TO apexintel_app;
    END IF;
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'apexintel') THEN
        GRANT SELECT, INSERT, UPDATE, DELETE ON persons TO apexintel;
    END IF;
END $$;
