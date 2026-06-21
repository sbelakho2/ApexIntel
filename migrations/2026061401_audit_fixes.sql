-- audit_fixes.sql: Indexes, constraints, and schema hardening from 2026-06-14 audit
-- All statements wrapped in DO blocks for production schema compatibility.

-- ─── Performance indexes on frequently-queried FK columns ───────────────────────
DO $$ BEGIN CREATE INDEX IF NOT EXISTS idx_warnings_entity_id     ON warnings(entity_id);     EXCEPTION WHEN OTHERS THEN RAISE NOTICE 'idx_warnings_entity_id skipped: %', SQLERRM; END $$;
DO $$ BEGIN CREATE INDEX IF NOT EXISTS idx_warnings_recipe_code   ON warnings(recipe_id);     EXCEPTION WHEN OTHERS THEN RAISE NOTICE 'idx_warnings_recipe_code skipped: %', SQLERRM; END $$;
DO $$ BEGIN CREATE INDEX IF NOT EXISTS idx_insights_recipe_id     ON insights(recipe_id);     EXCEPTION WHEN OTHERS THEN RAISE NOTICE 'idx_insights_recipe_id skipped: %', SQLERRM; END $$;
DO $$ BEGIN CREATE INDEX IF NOT EXISTS idx_observations_entity_id ON observations(entity_id); EXCEPTION WHEN OTHERS THEN RAISE NOTICE 'idx_observations_entity_id skipped: %', SQLERRM; END $$;
DO $$ BEGIN CREATE INDEX IF NOT EXISTS idx_observations_ts        ON observations(ts_utc);    EXCEPTION WHEN OTHERS THEN RAISE NOTICE 'idx_observations_ts skipped: %', SQLERRM; END $$;

-- ─── Backfill nulls before applying NOT NULL constraints ────────────────────────
DO $$ BEGIN UPDATE warnings  SET severity = 'low'     WHERE severity IS NULL;     EXCEPTION WHEN OTHERS THEN RAISE NOTICE 'warnings severity backfill skipped: %', SQLERRM; END $$;
DO $$ BEGIN UPDATE insights  SET confidence = 0.5     WHERE confidence IS NULL;   EXCEPTION WHEN OTHERS THEN RAISE NOTICE 'insights confidence backfill skipped: %', SQLERRM; END $$;
DO $$ BEGIN UPDATE observations SET ts_utc = now() WHERE ts_utc IS NULL;          EXCEPTION WHEN OTHERS THEN RAISE NOTICE 'observations ts_utc backfill skipped: %', SQLERRM; END $$;

-- ─── Add NOT NULL constraints with defaults ─────────────────────────────────────
DO $$ BEGIN ALTER TABLE warnings    ALTER COLUMN severity   SET NOT NULL; EXCEPTION WHEN OTHERS THEN RAISE NOTICE 'warnings severity NOT NULL skipped: %', SQLERRM; END $$;
DO $$ BEGIN ALTER TABLE warnings    ALTER COLUMN severity   SET DEFAULT 'low'; EXCEPTION WHEN OTHERS THEN RAISE NOTICE 'warnings severity DEFAULT skipped: %', SQLERRM; END $$;
DO $$ BEGIN ALTER TABLE insights    ALTER COLUMN confidence SET NOT NULL; EXCEPTION WHEN OTHERS THEN RAISE NOTICE 'insights confidence NOT NULL skipped: %', SQLERRM; END $$;
DO $$ BEGIN ALTER TABLE insights    ALTER COLUMN confidence SET DEFAULT 0.5; EXCEPTION WHEN OTHERS THEN RAISE NOTICE 'insights confidence DEFAULT skipped: %', SQLERRM; END $$;
DO $$ BEGIN ALTER TABLE observations ALTER COLUMN ts_utc SET NOT NULL;      EXCEPTION WHEN OTHERS THEN RAISE NOTICE 'observations ts_utc NOT NULL skipped: %', SQLERRM; END $$;
DO $$ BEGIN ALTER TABLE observations ALTER COLUMN ts_utc SET DEFAULT now(); EXCEPTION WHEN OTHERS THEN RAISE NOTICE 'observations ts_utc DEFAULT skipped: %', SQLERRM; END $$;

-- ─── Add foreign key constraints ────────────────────────────────────────────────
-- NOTE: Foreign keys referencing entities(id) are intentionally omitted because
-- the core schema has no `entities` table (entities are polymorphic across
-- companies/persons). Only the insights -> recipes FK is valid here.
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'fk_insights_recipe'
    ) THEN
        ALTER TABLE insights ADD CONSTRAINT fk_insights_recipe
            FOREIGN KEY (recipe_id) REFERENCES recipes(id) ON DELETE CASCADE;
    END IF;
EXCEPTION WHEN OTHERS THEN
    RAISE NOTICE 'fk_insights_recipe skipped: %', SQLERRM;
END $$;

-- ─── Add CHECK constraints for data validity ────────────────────────────────────
DO $$
BEGIN
    -- Drop old constraint first to avoid conflicts with unified schema's warnings_severity_check
    BEGIN
        ALTER TABLE warnings DROP CONSTRAINT IF EXISTS warnings_severity_check;
    EXCEPTION WHEN OTHERS THEN
        RAISE NOTICE 'drop warnings_severity_check skipped: %', SQLERRM;
    END;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'chk_warnings_severity'
    ) THEN
        ALTER TABLE warnings ADD CONSTRAINT chk_warnings_severity
            CHECK (severity IN ('critical', 'high', 'medium', 'low', 'info'));
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'chk_insights_confidence'
    ) THEN
        ALTER TABLE insights ADD CONSTRAINT chk_insights_confidence
            CHECK (confidence >= 0.0 AND confidence <= 1.0);
    END IF;
EXCEPTION WHEN OTHERS THEN
    RAISE NOTICE 'CHECK constraints skipped: %', SQLERRM;
END $$;

-- ─── Add columns to warnings table from store migration if missing ──────────────
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM information_schema.columns
                   WHERE table_name = 'warnings' AND column_name = 'source_urls') THEN
        ALTER TABLE warnings ADD COLUMN source_urls TEXT[] DEFAULT '{}';
    END IF;

    IF NOT EXISTS (SELECT 1 FROM information_schema.columns
                   WHERE table_name = 'warnings' AND column_name = 'entity_ids') THEN
        ALTER TABLE warnings ADD COLUMN entity_ids UUID[] DEFAULT '{}';
    END IF;

    IF NOT EXISTS (SELECT 1 FROM information_schema.columns
                   WHERE table_name = 'warnings' AND column_name = 'ts_utc') THEN
        ALTER TABLE warnings ADD COLUMN ts_utc TIMESTAMPTZ DEFAULT now();
    END IF;

    IF NOT EXISTS (SELECT 1 FROM information_schema.columns
                   WHERE table_name = 'warnings' AND column_name = 'acknowledged_note') THEN
        ALTER TABLE warnings ADD COLUMN acknowledged_note TEXT;
    END IF;

    IF NOT EXISTS (SELECT 1 FROM information_schema.columns
                   WHERE table_name = 'warnings' AND column_name = 'company_name') THEN
        ALTER TABLE warnings ADD COLUMN company_name TEXT;
    END IF;

    IF NOT EXISTS (SELECT 1 FROM information_schema.columns
                   WHERE table_name = 'warnings' AND column_name = 'deleted_at') THEN
        ALTER TABLE warnings ADD COLUMN deleted_at TIMESTAMPTZ;
    END IF;
EXCEPTION WHEN OTHERS THEN
    RAISE NOTICE 'warnings add columns skipped: %', SQLERRM;
END $$;

-- ─── Ensure entity_alert_configs has canonical columns ──────────────────────────
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM information_schema.columns
                   WHERE table_name = 'entity_alert_configs' AND column_name = 'alert_enabled') THEN
        ALTER TABLE entity_alert_configs ADD COLUMN alert_enabled BOOLEAN DEFAULT true;
    END IF;

    IF NOT EXISTS (SELECT 1 FROM information_schema.columns
                   WHERE table_name = 'entity_alert_configs' AND column_name = 'notification_channels') THEN
        ALTER TABLE entity_alert_configs ADD COLUMN notification_channels TEXT[] DEFAULT '{}';
    END IF;

    IF NOT EXISTS (SELECT 1 FROM information_schema.columns
                   WHERE table_name = 'entity_alert_configs' AND column_name = 'updated_at') THEN
        ALTER TABLE entity_alert_configs ADD COLUMN updated_at TIMESTAMPTZ DEFAULT now();
    END IF;
EXCEPTION WHEN OTHERS THEN
    RAISE NOTICE 'entity_alert_configs add columns skipped: %', SQLERRM;
END $$;
