-- Migration 100: converge the activity_feed schema across the 002/022 split
-- ════════════════════════════════════════════════════════════════════════════
-- `activity_feed` is defined twice in the migration chain:
--
--   * migrations/002_create_collaboration_tables.sql
--       VARCHAR(n) columns, entity_id VARCHAR(100), a CHECK on action_type,
--       a CHECK on visibility, and a workspace_id FK to investigation_workspaces
--       ON DELETE SET NULL.
--   * migrations/022_views_and_ux_enhancements.sql
--       TEXT columns, entity_id UUID, no CHECKs, no FK, and the four composite
--       (column, created_at DESC) indexes used by the feed queries.
--
-- Whichever file ran first wins the CREATE TABLE IF NOT EXISTS race, so the
-- two definitions produced divergent databases. The Rust surface is what
-- settles the target shape:
--
--   * store::ActivityFeedRecord decodes entity_id as Option<String>, and every
--     writer binds entity_id/actor_id as strings (worker ActivityLogger,
--     triage fallback, event_outbox, battlecards, API handlers);
--   * the feed queries filter on workspace_id / team_id / actor_id and order by
--     created_at DESC, i.e. the 022 composite indexes;
--   * the code writes action types beyond the 002 list: the triage fallback
--     ('triage_alert', 'triage_queue_update', 'triage_status_change'), the
--     outbox ('alert_dead_lettered') and intelligence ingress
--     ('warning_created', 'warning_updated'); the 002/062 CHECK rejected them.
--
-- Convergence rules: only widen (VARCHAR -> TEXT) and never truncate; the
-- entity_id direction is CASE-insensitive because `USING entity_id::text` is
-- lossless from UUID and a no-op from text. Every statement is idempotent and
-- no existing row is deleted or rewritten destructively; the only data touch is
-- clearing workspace_id values that have no parent workspace, so the FK added
-- below is valid (a dangling workspace reference was already invisible to the
-- feed's EXISTS-based visibility filter).
--
-- No BEGIN/COMMIT anywhere: sqlx wraps each migration in its own transaction.
-- ════════════════════════════════════════════════════════════════════════════

-- ── 1. Column that 097 adds; repeated for databases that stopped before 097. ─
ALTER TABLE activity_feed
    ADD COLUMN IF NOT EXISTS updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW();

-- ── 2. Widen every VARCHAR(n) to TEXT and converge entity_id to TEXT. ───────
ALTER TABLE activity_feed
    ALTER COLUMN actor_id    TYPE TEXT,
    ALTER COLUMN actor_name  TYPE TEXT,
    ALTER COLUMN action_type TYPE TEXT,
    ALTER COLUMN entity_type TYPE TEXT,
    ALTER COLUMN entity_id   TYPE TEXT USING entity_id::text,
    ALTER COLUMN entity_name TYPE TEXT,
    ALTER COLUMN team_id     TYPE TEXT,
    ALTER COLUMN visibility  TYPE TEXT;

-- ── 3. Action-type CHECK: the 002/062 baseline plus every literal the code
--      writes today. Existing rows only ever came from the narrower baseline,
--      so revalidation cannot fail. ─────────────────────────────────────────
ALTER TABLE activity_feed DROP CONSTRAINT IF EXISTS chk_activity_action_type;
ALTER TABLE activity_feed ADD CONSTRAINT chk_activity_action_type CHECK (action_type IN (
    'create', 'update', 'delete', 'share', 'assign', 'comment', 'resolve',
    'reopen', 'escalate', 'deescalate', 'approve', 'reject', 'merge', 'split',
    -- System event types
    'insight_generated', 'poi_discovered', 'crawl_completed', 'company_detected',
    'threat_detected', 'psych_profile_updated', 'battlecard_generated',
    'memo_generated', 'recipe_promoted',
    -- Job lifecycle events
    'job_completed', 'job_degraded', 'job_failed', 'job_skipped',
    -- Triage fallback (crates/triage/src/router_integration.rs)
    'triage_alert', 'triage_queue_update', 'triage_status_change',
    -- Event outbox dead-letter operator alert
    'alert_dead_lettered',
    -- Warning ingress lifecycle
    'warning_created', 'warning_updated'
));

-- ── 4. Visibility CHECK (defined by 002 only; kept when already present). ───
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'chk_activity_visibility'
          AND conrelid = 'activity_feed'::regclass
    ) THEN
        ALTER TABLE activity_feed
            ADD CONSTRAINT chk_activity_visibility
            CHECK (visibility IN ('private', 'team', 'organization', 'public'));
    END IF;
END $$;

-- ── 5. Workspace FK (defined by 002 only; repair dangling references first so
--      the constraint can be validated). ────────────────────────────────────
DO $$
BEGIN
    IF to_regclass('public.investigation_workspaces') IS NOT NULL
       AND NOT EXISTS (
           SELECT 1 FROM pg_constraint
           WHERE conrelid = 'activity_feed'::regclass
             AND contype = 'f'
             AND confrelid = 'investigation_workspaces'::regclass
       )
    THEN
        UPDATE activity_feed a
           SET workspace_id = NULL
         WHERE a.workspace_id IS NOT NULL
           AND NOT EXISTS (
               SELECT 1 FROM investigation_workspaces w WHERE w.id = a.workspace_id
           );
        ALTER TABLE activity_feed
            ADD CONSTRAINT activity_feed_workspace_id_fkey
            FOREIGN KEY (workspace_id)
            REFERENCES investigation_workspaces(id)
            ON DELETE SET NULL;
    END IF;
END $$;

-- ── 6. Indexes the feed queries rely on (022's set; created IF NOT EXISTS so
--      databases that got 022's CREATE TABLE already own them). ─────────────
CREATE INDEX IF NOT EXISTS idx_feed_workspace ON activity_feed(workspace_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_feed_team      ON activity_feed(team_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_feed_actor     ON activity_feed(actor_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_feed_recent    ON activity_feed(created_at DESC);
