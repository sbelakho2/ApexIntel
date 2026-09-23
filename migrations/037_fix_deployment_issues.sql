-- Fix deployment issues discovered during verification
-- 1. Add missing columns to source_reliability_stats (code expects reliability_tier, false_positive_rate, last_updated, metadata)
-- 2. Create missing observation_entity_graph table used by insight generation
-- 3. Fix NOT NULL constraint on tier column (code uses reliability_tier, leaving tier NULL)
-- 4. Create trigger to keep tier and reliability_tier in sync

BEGIN;

-- ============================================================
-- Fix 1: source_reliability_stats — add columns the code expects
-- ============================================================

-- Add reliability_tier as an alias for the existing 'tier' column
ALTER TABLE source_reliability_stats
    ADD COLUMN IF NOT EXISTS reliability_tier TEXT NOT NULL DEFAULT 'Unknown';

-- Sync reliability_tier from tier
UPDATE source_reliability_stats
    SET reliability_tier = tier
    WHERE reliability_tier IS DISTINCT FROM tier;

-- Add false_positive_rate (used by adversarial analysis)
ALTER TABLE source_reliability_stats
    ADD COLUMN IF NOT EXISTS false_positive_rate DOUBLE PRECISION NOT NULL DEFAULT 0.0;

-- Add last_updated (used by code instead of last_refreshed_at)
ALTER TABLE source_reliability_stats
    ADD COLUMN IF NOT EXISTS last_updated TIMESTAMPTZ NOT NULL DEFAULT NOW();

-- Sync last_updated from last_refreshed_at
UPDATE source_reliability_stats
    SET last_updated = last_refreshed_at
    WHERE last_updated IS DISTINCT FROM last_refreshed_at;

-- Add metadata JSONB column
ALTER TABLE source_reliability_stats
    ADD COLUMN IF NOT EXISTS metadata JSONB NOT NULL DEFAULT '{}'::jsonb;

-- ============================================================
-- Fix 1b: Fix NOT NULL constraint on tier column
-- The code INSERTs using reliability_tier but tier is NOT NULL with no default.
-- We give tier a default value so INSERTs don't fail.
-- ============================================================

-- Alter tier to have a default value
ALTER TABLE source_reliability_stats
    ALTER COLUMN tier SET DEFAULT 'Unknown';

-- Create a trigger function to keep tier in sync with reliability_tier
CREATE OR REPLACE FUNCTION sync_source_reliability_tier()
RETURNS TRIGGER AS $$
BEGIN
    -- If reliability_tier was set but tier was not, copy it
    IF NEW.reliability_tier IS NOT NULL AND NEW.tier IS NULL THEN
        NEW.tier := NEW.reliability_tier;
    END IF;
    -- If tier was set but reliability_tier was not, copy it
    IF NEW.tier IS NOT NULL AND NEW.reliability_tier IS NULL THEN
        NEW.reliability_tier := NEW.tier;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- Drop trigger if it exists (idempotent)
DROP TRIGGER IF EXISTS trg_sync_source_reliability_tier ON source_reliability_stats;

-- Create trigger before INSERT or UPDATE
CREATE TRIGGER trg_sync_source_reliability_tier
    BEFORE INSERT OR UPDATE ON source_reliability_stats
    FOR EACH ROW
    EXECUTE FUNCTION sync_source_reliability_tier();

-- ============================================================
-- Fix 2: Create observation_entity_graph table
-- ============================================================

CREATE TABLE IF NOT EXISTS observation_entity_graph (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    observation_id UUID NOT NULL REFERENCES observations(id) ON DELETE CASCADE,
    entity_id UUID NOT NULL,
    entity_type TEXT NOT NULL DEFAULT 'company',
    company_id UUID REFERENCES companies(id) ON DELETE CASCADE,
    edge_type TEXT NOT NULL DEFAULT 'mentioned_in',
    weight DOUBLE PRECISION NOT NULL DEFAULT 1.0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(observation_id, entity_id, entity_type, edge_type)
);

CREATE INDEX IF NOT EXISTS idx_oeg_observation_id
    ON observation_entity_graph (observation_id);
CREATE INDEX IF NOT EXISTS idx_oeg_entity_id
    ON observation_entity_graph (entity_id, entity_type);
CREATE INDEX IF NOT EXISTS idx_oeg_company_id
    ON observation_entity_graph (company_id);

-- ============================================================
-- Fix 3: Ensure the core migrations table exists so that
-- embedded migrations can track this
-- ============================================================

CREATE TABLE IF NOT EXISTS _sqlx_migrations (
    version BIGINT PRIMARY KEY,
    description TEXT NOT NULL,
    installed_on TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    success BOOLEAN NOT NULL,
    checksum BYTEA,
    execution_time_ms BIGINT NOT NULL
);

COMMIT;
