-- ════════════════════════════════════════════════════════════════════════════
-- Unify Duplicate Schemas Between Migration Lineages
--
-- READ ME:
-- This project has TWO migration directories:
--   - migrations/           (primary, date-based naming)
--   - crates/store/migrations/  (secondary, sequential naming)
--
-- Both define the same tables with incompatible schemas.
-- The `migrations/` directory is the canonical source of truth.
-- This migration adds columns to core-schema tables that are expected
-- by the Rust application code (which was written against the store-crate
-- schema), creating a compatible superset schema.
--
-- Idempotent: uses IF NOT EXISTS / IF EXISTS throughout.
-- ════════════════════════════════════════════════════════════════════════════

BEGIN;

-- ══════════════════════════════════════════════════════════════════════════
-- 1. Add store-crate columns to `warnings` table
--    Rust WarningRow expects: recipe_code, source_urls, entity_ids,
--    acknowledged_note, ts_utc
-- ══════════════════════════════════════════════════════════════════════════

ALTER TABLE warnings ADD COLUMN IF NOT EXISTS recipe_code TEXT;
ALTER TABLE warnings ADD COLUMN IF NOT EXISTS source_urls TEXT[] DEFAULT '{}';
ALTER TABLE warnings ADD COLUMN IF NOT EXISTS entity_ids UUID[] DEFAULT '{}';
ALTER TABLE warnings ADD COLUMN IF NOT EXISTS acknowledged_note TEXT;
ALTER TABLE warnings ADD COLUMN IF NOT EXISTS ts_utc TIMESTAMPTZ;

-- Sync ts_utc from created_at for existing rows
UPDATE warnings SET ts_utc = created_at WHERE ts_utc IS NULL;

-- Add FK from warnings.recipe_code to recipes(id) / recipes(code)
-- Since core schema has recipes(id) and store has recipes(code),
-- we reference recipes(id) as the canonical PK.
-- A trigger will keep recipe_code in sync with recipe_id.
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'fk_warnings_recipe_code'
          AND conrelid = 'warnings'::regclass
    ) THEN
        ALTER TABLE warnings
            ADD CONSTRAINT fk_warnings_recipe_code
            FOREIGN KEY (recipe_code) REFERENCES recipes(id) ON DELETE SET NULL;
    END IF;
END $$;

-- Create a trigger to keep recipe_code in sync with recipe_id
CREATE OR REPLACE FUNCTION sync_warning_recipe_code()
RETURNS trigger AS $$
BEGIN
    IF NEW.recipe_code IS NULL AND NEW.recipe_id IS NOT NULL THEN
        NEW.recipe_code := NEW.recipe_id;
    END IF;
    IF NEW.recipe_id IS NULL AND NEW.recipe_code IS NOT NULL THEN
        NEW.recipe_id := NEW.recipe_code;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_sync_warning_recipe_code ON warnings;
CREATE TRIGGER trg_sync_warning_recipe_code
    BEFORE INSERT OR UPDATE OF recipe_id, recipe_code ON warnings
    FOR EACH ROW EXECUTE FUNCTION sync_warning_recipe_code();

-- Create a trigger to sync entity_id <-> entity_ids
CREATE OR REPLACE FUNCTION sync_warning_entity_ids()
RETURNS trigger AS $$
BEGIN
    IF NEW.entity_ids IS NULL OR array_length(NEW.entity_ids, 1) IS NULL THEN
        IF NEW.entity_id IS NOT NULL THEN
            NEW.entity_ids := ARRAY[NEW.entity_id];
        END IF;
    END IF;
    IF NEW.entity_id IS NULL AND NEW.entity_ids IS NOT NULL AND array_length(NEW.entity_ids, 1) >= 1 THEN
        NEW.entity_id := NEW.entity_ids[1];
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_sync_warning_entity_ids ON warnings;
CREATE TRIGGER trg_sync_warning_entity_ids
    BEFORE INSERT OR UPDATE OF entity_id, entity_ids ON warnings
    FOR EACH ROW EXECUTE FUNCTION sync_warning_entity_ids();

-- Indexes for the new columns
CREATE INDEX IF NOT EXISTS idx_warnings_recipe_code ON warnings(recipe_code);
CREATE INDEX IF NOT EXISTS idx_warnings_ts_utc ON warnings(ts_utc DESC);
CREATE INDEX IF NOT EXISTS idx_warnings_entity_ids_gin ON warnings USING gin(entity_ids) WHERE entity_ids IS NOT NULL;

-- ══════════════════════════════════════════════════════════════════════════
-- 2. Add store-crate columns to `insights` table
--    Rust InsightRow expects: summary, evidence_urls, entity_ids, tags,
--    title_hash, metadata
-- ══════════════════════════════════════════════════════════════════════════

ALTER TABLE insights ADD COLUMN IF NOT EXISTS summary TEXT;
ALTER TABLE insights ADD COLUMN IF NOT EXISTS evidence_urls TEXT[] DEFAULT '{}';
ALTER TABLE insights ADD COLUMN IF NOT EXISTS entity_ids UUID[] DEFAULT '{}';
ALTER TABLE insights ADD COLUMN IF NOT EXISTS tags TEXT[] DEFAULT '{}';
ALTER TABLE insights ADD COLUMN IF NOT EXISTS title_hash TEXT;

-- Sync summary from narrative for existing rows
UPDATE insights SET summary = narrative WHERE summary IS NULL AND narrative IS NOT NULL;
-- If both null, set summary to title as fallback
UPDATE insights SET summary = title WHERE summary IS NULL;

-- Sync entity_ids from entity_id
UPDATE insights SET entity_ids = ARRAY[entity_id] WHERE entity_id IS NOT NULL AND (entity_ids IS NULL OR array_length(entity_ids, 1) IS NULL);

-- Compute title_hash for existing rows
UPDATE insights SET title_hash = md5(title) WHERE title_hash IS NULL;

-- Create a trigger to sync entity_id <-> entity_ids for insights
CREATE OR REPLACE FUNCTION sync_insight_entity_ids()
RETURNS trigger AS $$
BEGIN
    IF NEW.entity_ids IS NULL OR array_length(NEW.entity_ids, 1) IS NULL THEN
        IF NEW.entity_id IS NOT NULL THEN
            NEW.entity_ids := ARRAY[NEW.entity_id];
        END IF;
    END IF;
    IF NEW.entity_id IS NULL AND NEW.entity_ids IS NOT NULL AND array_length(NEW.entity_ids, 1) >= 1 THEN
        NEW.entity_id := NEW.entity_ids[1];
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_sync_insight_entity_ids ON insights;
CREATE TRIGGER trg_sync_insight_entity_ids
    BEFORE INSERT OR UPDATE OF entity_id, entity_ids ON insights
    FOR EACH ROW EXECUTE FUNCTION sync_insight_entity_ids();

-- Indexes for the new columns
CREATE INDEX IF NOT EXISTS idx_insights_title_hash ON insights(title_hash) WHERE title_hash IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_insights_entity_ids_gin ON insights USING gin(entity_ids) WHERE entity_ids IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_insights_dedup_window ON insights(insight_type, created_at DESC) WHERE entity_ids IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_insights_updated_at ON insights(updated_at DESC, id ASC);
CREATE INDEX IF NOT EXISTS idx_insights_tags ON insights USING gin(tags);

-- ══════════════════════════════════════════════════════════════════════════
-- 3. Add store-crate columns to `recipes` table
--    Rust expects: code (alias for id), precision_score, definition
-- ══════════════════════════════════════════════════════════════════════════

-- Add code as an alias for id
ALTER TABLE recipes ADD COLUMN IF NOT EXISTS code TEXT;
UPDATE recipes SET code = id WHERE code IS NULL;
ALTER TABLE recipes ALTER COLUMN code SET NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS idx_recipes_code ON recipes(code);

ALTER TABLE recipes ADD COLUMN IF NOT EXISTS precision_score FLOAT;
ALTER TABLE recipes ADD COLUMN IF NOT EXISTS definition JSONB DEFAULT '[]';

-- Sync precision_score from precision for existing rows
UPDATE recipes SET precision_score = precision WHERE precision_score IS NULL AND precision IS NOT NULL;

-- Create a trigger to keep code in sync with id
CREATE OR REPLACE FUNCTION sync_recipe_code()
RETURNS trigger AS $$
BEGIN
    IF NEW.code IS NULL THEN
        NEW.code := NEW.id;
    END IF;
    IF NEW.id IS NULL THEN
        NEW.id := NEW.code;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_sync_recipe_code ON recipes;
CREATE TRIGGER trg_sync_recipe_code
    BEFORE INSERT OR UPDATE OF id, code ON recipes
    FOR EACH ROW EXECUTE FUNCTION sync_recipe_code();

-- ══════════════════════════════════════════════════════════════════════════
-- 4. Add `title_hash` to `insights` for dedup support
--    (already done above, but ensure the NOT NULL constraint is not present)
-- ══════════════════════════════════════════════════════════════════════════

-- (already added above with index)

-- ══════════════════════════════════════════════════════════════════════════
-- 5. Add store-crate JSONB `data` column to feature_rows for analytics
--    Core schema uses typed columns; store schema uses a single data JSONB.
--    Analytics queries (get_drift_stats) reference data->>'drift_score'.
-- ══════════════════════════════════════════════════════════════════════════

ALTER TABLE feature_rows ADD COLUMN IF NOT EXISTS data JSONB DEFAULT '{}';
-- Sync data from typed columns for existing rows
UPDATE feature_rows
SET data = jsonb_build_object(
    'signal_counts', signal_counts,
    'diffs', diffs,
    'pct_changes', pct_changes,
    'regime_flags', regime_flags,
    'volatility', volatility,
    'topic_drift', topic_drift,
    'neighbor_agg_1hop', neighbor_agg_1hop,
    'neighbor_agg_2hop', neighbor_agg_2hop,
    'poi_pain_index', poi_pain_index,
    'poi_role_drift', poi_role_drift,
    'poi_influence_delta', poi_influence_delta,
    'drift_score', CASE
        WHEN poi_role_drift IS NOT NULL THEN poi_role_drift
        WHEN poi_influence_delta IS NOT NULL THEN poi_influence_delta
        ELSE NULL
    END
)
WHERE data = '{}'::jsonb OR data IS NULL;

-- Add bucket_size_days column (store schema uses this in PK)
ALTER TABLE feature_rows ADD COLUMN IF NOT EXISTS bucket_size_days INT;
UPDATE feature_rows
SET bucket_size_days = CASE bucket_size
    WHEN 'daily' THEN 1
    WHEN 'weekly' THEN 7
    WHEN 'monthly' THEN 30
    ELSE 1
END
WHERE bucket_size_days IS NULL;
ALTER TABLE feature_rows ALTER COLUMN bucket_size_days SET DEFAULT 1;

-- ══════════════════════════════════════════════════════════════════════════
-- 6. Add store-crate FK and ref columns to duplicate tables
-- ══════════════════════════════════════════════════════════════════════════

-- Add analyst_users email as reference for FK columns
-- NOTE: analyst_users already exists and has id TEXT PK

-- Add FK from insight_bookmarks.user_id to analyst_users(id)
ALTER TABLE insight_bookmarks DROP CONSTRAINT IF EXISTS fk_insight_bookmarks_user;
-- Only add if analyst_users table exists
DO $$
BEGIN
    IF EXISTS (SELECT FROM pg_tables WHERE tablename = 'analyst_users') THEN
        ALTER TABLE insight_bookmarks
            ADD CONSTRAINT fk_insight_bookmarks_user
            FOREIGN KEY (user_id) REFERENCES analyst_users(id) ON DELETE CASCADE;
    END IF;
END $$;

-- ══════════════════════════════════════════════════════════════════════════
-- 7. Fix recipe_weekly_metrics FK to reference recipes(id) correctly
--    The existing FK references recipes(code) but core schema uses recipes(id)
-- ══════════════════════════════════════════════════════════════════════════

-- Drop and recreate the FK if it references recipes(code)
DO $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'recipe_weekly_metrics_recipe_code_fkey'
        AND conrelid = 'recipe_weekly_metrics'::regclass
    ) THEN
        ALTER TABLE recipe_weekly_metrics
            DROP CONSTRAINT recipe_weekly_metrics_recipe_code_fkey;
    END IF;
END $$;

ALTER TABLE recipe_weekly_metrics
    ADD CONSTRAINT recipe_weekly_metrics_recipe_code_fkey
    FOREIGN KEY (recipe_code) REFERENCES recipes(id) ON DELETE CASCADE;

-- ══════════════════════════════════════════════════════════════════════════
-- 8. Fix materialized views to reference correct core-schema columns
--    mv_company_risk_leaderboard: use entity_id (scalar) instead of entity_ids (array)
--    mv_recipe_performance: use r.id (or r.code now since we added it) instead of r.code
--    Also add proper indexes
-- ══════════════════════════════════════════════════════════════════════════

-- NOTE: The materialized views are dropped and recreated in the
-- 20260301_materialized_views.sql file (which is being fixed separately).
-- This migration just ensures the underlying columns exist.

-- ══════════════════════════════════════════════════════════════════════════
-- 9. Add ON DELETE actions to store-crate-style FKs that are missing them
-- ══════════════════════════════════════════════════════════════════════════

-- Fix role_history.org_id FK to have ON DELETE SET NULL
DO $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'role_history_org_id_fkey'
        AND conrelid = 'role_history'::regclass
    ) THEN
        ALTER TABLE role_history DROP CONSTRAINT role_history_org_id_fkey;
        ALTER TABLE role_history
            ADD CONSTRAINT role_history_org_id_fkey
            FOREIGN KEY (org_id) REFERENCES companies(id) ON DELETE SET NULL;
    END IF;
END $$;

-- Fix dossier_entries.supersedes_id FK to have ON DELETE SET NULL
DO $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'dossier_entries_supersedes_id_fkey'
        AND conrelid = 'dossier_entries'::regclass
    ) THEN
        ALTER TABLE dossier_entries DROP CONSTRAINT dossier_entries_supersedes_id_fkey;
    END IF;
END $$;

ALTER TABLE dossier_entries
    DROP CONSTRAINT IF EXISTS dossier_entries_supersedes_id_fkey,
    ADD CONSTRAINT dossier_entries_supersedes_id_fkey
    FOREIGN KEY (supersedes_id) REFERENCES dossier_entries(id) ON DELETE SET NULL;

-- ══════════════════════════════════════════════════════════════════════════
-- 10. Add missing CHECK constraints on score/confidence columns
-- ══════════════════════════════════════════════════════════════════════════

-- observations.confidence
ALTER TABLE observations DROP CONSTRAINT IF EXISTS observations_confidence_check;
ALTER TABLE observations ADD CONSTRAINT observations_confidence_check
    CHECK (confidence IS NULL OR (confidence >= 0 AND confidence <= 1));

-- observations.quality_score
ALTER TABLE observations DROP CONSTRAINT IF EXISTS observations_quality_score_check;
ALTER TABLE observations ADD CONSTRAINT observations_quality_score_check
    CHECK (quality_score IS NULL OR (quality_score >= 0 AND quality_score <= 1));

-- warnings.confidence
ALTER TABLE warnings DROP CONSTRAINT IF EXISTS warnings_confidence_check;
ALTER TABLE warnings ADD CONSTRAINT warnings_confidence_check
    CHECK (confidence IS NULL OR (confidence >= 0 AND confidence <= 1));

-- insights.confidence
ALTER TABLE insights DROP CONSTRAINT IF EXISTS insights_confidence_check;
ALTER TABLE insights ADD CONSTRAINT insights_confidence_check
    CHECK (confidence IS NULL OR (confidence >= 0 AND confidence <= 1));

-- dossier_entries.confidence
ALTER TABLE dossier_entries DROP CONSTRAINT IF EXISTS dossier_entries_confidence_check;
ALTER TABLE dossier_entries ADD CONSTRAINT dossier_entries_confidence_check
    CHECK (confidence IS NULL OR (confidence >= 0 AND confidence <= 1));

-- competitor_changes.impact_score
ALTER TABLE competitor_changes DROP CONSTRAINT IF EXISTS competitor_changes_impact_score_check;
ALTER TABLE competitor_changes ADD CONSTRAINT competitor_changes_impact_score_check
    CHECK (impact_score >= 0 AND impact_score <= 1);

-- graph_edges.weight
ALTER TABLE graph_edges DROP CONSTRAINT IF EXISTS graph_edges_weight_check;
ALTER TABLE graph_edges ADD CONSTRAINT graph_edges_weight_check
    CHECK (weight IS NULL OR (weight >= 0 AND weight <= 1));

-- graph_edges.confidence
ALTER TABLE graph_edges DROP CONSTRAINT IF EXISTS graph_edges_confidence_check;
ALTER TABLE graph_edges ADD CONSTRAINT graph_edges_confidence_check
    CHECK (confidence IS NULL OR (confidence >= 0 AND confidence <= 1));

-- person_changes.confidence
ALTER TABLE person_changes DROP CONSTRAINT IF EXISTS person_changes_confidence_check;
ALTER TABLE person_changes ADD CONSTRAINT person_changes_confidence_check
    CHECK (confidence IS NULL OR (confidence >= 0 AND confidence <= 1));

-- company_changes.confidence
ALTER TABLE company_changes DROP CONSTRAINT IF EXISTS company_changes_confidence_check;
ALTER TABLE company_changes ADD CONSTRAINT company_changes_confidence_check
    CHECK (confidence IS NULL OR (confidence >= 0 AND confidence <= 1));

-- role_history.confidence
ALTER TABLE role_history DROP CONSTRAINT IF EXISTS role_history_confidence_check;
ALTER TABLE role_history ADD CONSTRAINT role_history_confidence_check
    CHECK (confidence IS NULL OR (confidence >= 0 AND confidence <= 1));

-- pattern_candidates.confidence
ALTER TABLE pattern_candidates DROP CONSTRAINT IF EXISTS pattern_candidates_confidence_check;
ALTER TABLE pattern_candidates ADD CONSTRAINT pattern_candidates_confidence_check
    CHECK (confidence >= 0 AND confidence <= 1);

-- ══════════════════════════════════════════════════════════════════════════
-- 11. Add missing CHECK constraints on status columns
-- ══════════════════════════════════════════════════════════════════════════

-- warnings.severity
ALTER TABLE warnings DROP CONSTRAINT IF EXISTS warnings_severity_check;
ALTER TABLE warnings ADD CONSTRAINT warnings_severity_check
    CHECK (severity IN ('low', 'medium', 'high', 'critical'));

-- replay_jobs.status
ALTER TABLE replay_jobs DROP CONSTRAINT IF EXISTS replay_jobs_status_check;
ALTER TABLE replay_jobs ADD CONSTRAINT replay_jobs_status_check
    CHECK (status IN ('queued', 'running', 'completed', 'failed', 'cancelled'));

-- worker_job_history.status (if table exists)
DO $$
BEGIN
    IF EXISTS (SELECT FROM pg_tables WHERE tablename = 'worker_job_history') THEN
        ALTER TABLE worker_job_history DROP CONSTRAINT IF EXISTS worker_job_history_status_check;
        ALTER TABLE worker_job_history ADD CONSTRAINT worker_job_history_status_check
            CHECK (status IN ('queued', 'running', 'completed', 'failed'));
    END IF;
END $$;

-- analyst_user_roles.role (if table exists)
DO $$
BEGIN
    IF EXISTS (SELECT FROM pg_tables WHERE tablename = 'analyst_user_roles') THEN
        ALTER TABLE analyst_user_roles DROP CONSTRAINT IF EXISTS analyst_user_roles_role_check;
        ALTER TABLE analyst_user_roles ADD CONSTRAINT analyst_user_roles_role_check
            CHECK (role IN ('viewer', 'analyst', 'admin', 'superadmin'));
    END IF;
END $$;

-- api_key_owners.role (if table exists)
DO $$
BEGIN
    IF EXISTS (SELECT FROM pg_tables WHERE tablename = 'api_key_owners') THEN
        ALTER TABLE api_key_owners DROP CONSTRAINT IF EXISTS api_key_owners_role_check;
        ALTER TABLE api_key_owners ADD CONSTRAINT api_key_owners_role_check
            CHECK (role IN ('viewer', 'analyst', 'admin', 'superadmin'));
    END IF;
END $$;

-- ══════════════════════════════════════════════════════════════════════════
-- 12. Add missing indexes on foreign key columns
-- ══════════════════════════════════════════════════════════════════════════

-- role_history indexes (already had idx_role_history_org)
-- Additional index on role_history.org_id standalone
CREATE INDEX IF NOT EXISTS idx_role_history_org_id ON role_history(org_id);

-- dossier_entries.entity_id for direct lookups (already has composite index)
CREATE INDEX IF NOT EXISTS idx_dossier_entries_entity_id ON dossier_entries(entity_id);

-- social_signals entity index
CREATE INDEX IF NOT EXISTS idx_social_entity ON social_signals(entity_name, entity_type);

-- logistics_nodes indexes
CREATE INDEX IF NOT EXISTS idx_logistics_country ON logistics_nodes(country_code);
CREATE INDEX IF NOT EXISTS idx_logistics_type ON logistics_nodes(node_type);

-- regulations indexes
CREATE INDEX IF NOT EXISTS idx_regulations_jurisdiction ON regulations(jurisdiction);
CREATE INDEX IF NOT EXISTS idx_regulations_type ON regulations(regulation_type);

-- insights entity_id scalar index (existing is idx_insights_entity)
CREATE INDEX IF NOT EXISTS idx_insights_entity_id ON insights(entity_id);

-- warnings entity_id scalar index (existing is idx_warnings_entity)
CREATE INDEX IF NOT EXISTS idx_warnings_entity_id ON warnings(entity_id);

-- observations entity_id for direct lookups
CREATE INDEX IF NOT EXISTS idx_observations_entity_id ON observations(entity_id);

-- person_changes.person_id standalone
CREATE INDEX IF NOT EXISTS idx_person_changes_person_id ON person_changes(person_id);

-- company_changes.company_id standalone
CREATE INDEX IF NOT EXISTS idx_company_changes_company_id ON company_changes(company_id);

-- quality_score_breakdown foreign key index (observation_id)
CREATE INDEX IF NOT EXISTS idx_quality_breakdown_observation_id ON quality_score_breakdown(observation_id);

-- gate_decisions entity_id index
CREATE INDEX IF NOT EXISTS idx_gate_decisions_entity_id ON gate_decisions(entity_id);

-- llm_retry_stats entity_id index
CREATE INDEX IF NOT EXISTS idx_llm_retry_entity_id ON llm_retry_stats(entity_id);

-- insight_outcomes entity_id index
CREATE INDEX IF NOT EXISTS idx_insight_outcomes_entity_id ON insight_outcomes(entity_id);

-- ══════════════════════════════════════════════════════════════════════════
-- 13. Change competitor_changes.impact_score from REAL to DOUBLE PRECISION
-- ══════════════════════════════════════════════════════════════════════════

ALTER TABLE competitor_changes
    ALTER COLUMN impact_score TYPE DOUBLE PRECISION;

-- ══════════════════════════════════════════════════════════════════════════
-- 14. Replace example.com URLs in seed data with placeholder documentation
-- ══════════════════════════════════════════════════════════════════════════

UPDATE competitor_changes
SET source_url = 'https://docs.apexintel.local/placeholders/seed-data'
WHERE source_url LIKE 'https://example.com/%';

-- ══════════════════════════════════════════════════════════════════════════
-- 15. Add GDPR anonymization support columns to persons table
-- ══════════════════════════════════════════════════════════════════════════

ALTER TABLE persons ADD COLUMN IF NOT EXISTS anonymized_at TIMESTAMPTZ;
ALTER TABLE persons ADD COLUMN IF NOT EXISTS gdpr_consent_withdrawn_at TIMESTAMPTZ;

-- ══════════════════════════════════════════════════════════════════════════
-- 16. Store-crate style index coverage for enhanced query performance
-- ══════════════════════════════════════════════════════════════════════════

-- Companies
CREATE INDEX IF NOT EXISTS idx_companies_type ON companies(company_type);
CREATE INDEX IF NOT EXISTS idx_companies_updated_at ON companies(updated_at DESC);

-- Warnings
CREATE INDEX IF NOT EXISTS idx_warnings_region ON warnings(region);
CREATE INDEX IF NOT EXISTS idx_warnings_ack ON warnings(acknowledged);

-- Persons
CREATE INDEX IF NOT EXISTS idx_persons_name ON persons(name);

COMMIT;
