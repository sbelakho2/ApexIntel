-- 045_schema_reconciliation.sql
--
-- The Rust code was written against a superset of shapes from the legacy
-- crates/store/migrations lineage, while only this root lineage is applied.
-- This migration reconciles the applied schema with what the code actually
-- reads/writes. It is append-only and idempotent (guarded by IF NOT EXISTS /
-- IF EXISTS) so it is safe to run against existing deployments.

-- ─── Missing columns the code references ────────────────────────────────────
ALTER TABLE companies ADD COLUMN IF NOT EXISTS narrative TEXT;
ALTER TABLE companies ADD COLUMN IF NOT EXISTS canonical_name TEXT;
ALTER TABLE persons   ADD COLUMN IF NOT EXISTS full_name TEXT;
ALTER TABLE persons   ADD COLUMN IF NOT EXISTS narrative TEXT;
ALTER TABLE persons   ADD COLUMN IF NOT EXISTS last_psych_enrichment_at TIMESTAMPTZ;
ALTER TABLE insights  ADD COLUMN IF NOT EXISTS description TEXT;
ALTER TABLE graph_edges ADD COLUMN IF NOT EXISTS archived BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE engagement_profiles ADD COLUMN IF NOT EXISTS updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW();
ALTER TABLE triage_queue ADD COLUMN IF NOT EXISTS updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW();
ALTER TABLE triage_queue ADD COLUMN IF NOT EXISTS resolved_at TIMESTAMPTZ;
ALTER TABLE user_preferences ADD COLUMN IF NOT EXISTS prefs JSONB NOT NULL DEFAULT '{}';

-- Tags: the code upserts on a normalised label. `label`/`normalized_label`
-- exist in the production lineage but not the repository-root lineage, so add
-- them defensively and backfill only from columns that actually exist.
ALTER TABLE tags ADD COLUMN IF NOT EXISTS label TEXT;
ALTER TABLE tags ADD COLUMN IF NOT EXISTS normalized_label TEXT;
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM information_schema.columns
               WHERE table_name = 'tags' AND column_name = 'label') THEN
        EXECUTE 'UPDATE tags SET normalized_label = lower(label)
                 WHERE normalized_label IS NULL AND label IS NOT NULL';
    END IF;
END $$;
CREATE UNIQUE INDEX IF NOT EXISTS uq_tags_normalized_label
    ON tags (normalized_label)
    WHERE normalized_label IS NOT NULL;

ALTER TABLE tag_assignments ADD COLUMN IF NOT EXISTS subject_type TEXT;
ALTER TABLE tag_assignments ADD COLUMN IF NOT EXISTS subject_id TEXT;
ALTER TABLE tag_assignments ADD COLUMN IF NOT EXISTS source TEXT;
CREATE UNIQUE INDEX IF NOT EXISTS uq_tag_assignments_natural
    ON tag_assignments (tag_id, subject_type, subject_id, source)
    WHERE subject_type IS NOT NULL AND subject_id IS NOT NULL;

-- ─── Missing uniqueness the code's ON CONFLICT targets rely on ──────────────
CREATE UNIQUE INDEX IF NOT EXISTS uq_analyst_user_roles_user_role
    ON analyst_user_roles (user_id, role);

CREATE UNIQUE INDEX IF NOT EXISTS uq_team_assignments_natural
    ON team_assignments (entity_type, entity_id, team_id);

CREATE UNIQUE INDEX IF NOT EXISTS uq_feature_rows_bucket_days
    ON feature_rows (entity_id, entity_type, time_bucket, bucket_size_days);

-- ─── Numeric contract ───────────────────────────────────────────────────────
-- NOTE: the strategic/threat/supplier/pipeline/source-evidence score columns
-- remain DECIMAL(5,4)/DECIMAL(15,2). Converting them to DOUBLE PRECISION is
-- blocked because public views (e.g. executive_summary_view) depend on them;
-- doing it safely requires dropping and recreating those views in dependency
-- order. Tracked as a follow-up rather than risking the bootstrap path here.

-- ─── Tables referenced by code that no migration created ────────────────────
CREATE TABLE IF NOT EXISTS academic_publications (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    entity_type   TEXT NOT NULL,
    entity_id     UUID,
    title         TEXT NOT NULL,
    authors       TEXT[] NOT NULL DEFAULT '{}',
    abstract      TEXT,
    venue         TEXT,
    published_at  TIMESTAMPTZ,
    doi           TEXT,
    url           TEXT,
    citation_count INTEGER,
    metadata      JSONB NOT NULL DEFAULT '{}',
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_academic_publications_entity
    ON academic_publications (entity_type, entity_id);

CREATE TABLE IF NOT EXISTS dead_letter_queue (
    id            BIGSERIAL PRIMARY KEY,
    job_kind      TEXT NOT NULL,
    payload       JSONB NOT NULL DEFAULT '{}',
    error         TEXT,
    attempts      INTEGER NOT NULL DEFAULT 0,
    first_failed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_failed_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    resolved_at   TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS idx_dead_letter_unresolved
    ON dead_letter_queue (resolved_at) WHERE resolved_at IS NULL;
