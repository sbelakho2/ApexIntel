-- Rollback: 20260301_observation_dedup
-- Drops the dedup function and index.

DROP FUNCTION IF EXISTS ingest_observation_dedup(TEXT, UUID, TEXT, TIMESTAMPTZ, JSONB, JSONB, FLOAT, FLOAT);

DROP INDEX IF EXISTS idx_obs_dedup;
