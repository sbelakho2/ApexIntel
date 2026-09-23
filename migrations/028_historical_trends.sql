-- ─────────────────────────────────────────────────────────────────────────────
-- Phase 2.5: Historical Trends Aggregation
--
-- Creates trend_rollups table for materialized time-bucketed metric storage,
-- enabling efficient monthly/quarterly/yearly trend queries without computing
-- from raw data on every request.
-- ─────────────────────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS trend_rollups (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    bucket_date DATE NOT NULL,          -- start of the bucket period
    bucket_type VARCHAR(20) NOT NULL,   -- 'daily', 'weekly', 'monthly', 'quarterly', 'yearly'
    entity_type VARCHAR(50),            -- NULL for global, 'company', 'person', 'region'
    entity_id VARCHAR(255),             -- NULL for global
    metric_name VARCHAR(100) NOT NULL,  -- 'warnings', 'insights', 'observations', 'companies_tracked', etc.
    metric_value BIGINT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (bucket_date, bucket_type, entity_type, entity_id, metric_name)
);

CREATE INDEX IF NOT EXISTS idx_trend_rollups_lookup
    ON trend_rollups (bucket_type, bucket_date DESC, metric_name);

CREATE INDEX IF NOT EXISTS idx_trend_rollups_entity
    ON trend_rollups (entity_type, entity_id, bucket_date DESC);

-- Trigger to auto-update updated_at on row modification
CREATE OR REPLACE FUNCTION update_trend_rollups_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_trend_rollups_updated_at ON trend_rollups;
CREATE TRIGGER trg_trend_rollups_updated_at
    BEFORE UPDATE ON trend_rollups
    FOR EACH ROW
    EXECUTE FUNCTION update_trend_rollups_updated_at();
