-- ════════════════════════════════════════════════════════════════════════════
-- Observation Deduplication Index
-- Prevents duplicate observations from re-crawls via content hash.
-- Run: psql $DATABASE_URL < migrations/20260301_observation_dedup.sql
-- ════════════════════════════════════════════════════════════════════════════

BEGIN;

-- Unique partial index on content hash for dedup
CREATE UNIQUE INDEX IF NOT EXISTS idx_obs_dedup
    ON observations (observation_type, entity_id, (provenance->>'content_hash'))
    WHERE provenance->>'content_hash' IS NOT NULL;

-- Function for upsert-based dedup ingestion
CREATE OR REPLACE FUNCTION ingest_observation_dedup(
    p_type TEXT,
    p_entity_id UUID,
    p_entity_type TEXT,
    p_ts TIMESTAMPTZ,
    p_value JSONB,
    p_provenance JSONB,
    p_confidence FLOAT DEFAULT 1.0,
    p_quality_score FLOAT DEFAULT 1.0
) RETURNS UUID AS $$
DECLARE
    result_id UUID;
    content_hash TEXT;
BEGIN
    content_hash := p_provenance->>'content_hash';

    -- If there's a content hash, use INSERT ... ON CONFLICT for atomic dedup
    IF content_hash IS NOT NULL THEN
        INSERT INTO observations (observation_type, entity_id, entity_type, ts_utc, value, provenance, confidence, quality_score)
        VALUES (p_type, p_entity_id, p_entity_type, p_ts, p_value, p_provenance, p_confidence, p_quality_score)
        ON CONFLICT (observation_type, entity_id, (provenance->>'content_hash'))
            WHERE provenance->>'content_hash' IS NOT NULL
        DO NOTHING
        RETURNING id INTO result_id;

        -- If DO NOTHING fired, fetch existing id
        IF result_id IS NULL THEN
            SELECT id INTO result_id
            FROM observations
            WHERE observation_type = p_type
              AND entity_id = p_entity_id
              AND provenance->>'content_hash' = content_hash;
        END IF;

        RETURN result_id;
    END IF;

    -- No content hash — always insert
    INSERT INTO observations (observation_type, entity_id, entity_type, ts_utc, value, provenance, confidence, quality_score)
    VALUES (p_type, p_entity_id, p_entity_type, p_ts, p_value, p_provenance, p_confidence, p_quality_score)
    RETURNING id INTO result_id;

    RETURN result_id;
END;
$$ LANGUAGE plpgsql;

COMMIT;
