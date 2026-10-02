-- Migration 101: record attribution for executive collaboration records
-- ════════════════════════════════════════════════════════════════════════════
-- `strategic_opportunities`, `critical_threats`, `supplier_risk` and
-- `pipeline_opportunities` recorded neither who created a row nor whether the
-- free-text `owner_id` named a real principal. A typo in the owner field
-- silently produced an orphan owner, and there was no audit trail of the
-- author.
--
-- This migration, for each of those tables that exists:
--   1. adds a nullable `created_by TEXT` column (the API/web layers bind the
--      authenticated principal's `app_users.id`);
--   2. adds `<table>_created_by_fkey` -> app_users(id) ON DELETE SET NULL;
--   3. adds `<table>_owner_id_fkey`   -> app_users(id) ON DELETE SET NULL.
--
-- Safety on existing data: both foreign keys are added `NOT VALID`, so legacy
-- rows whose owner is not a provisioned user do not abort the migration,
-- while every new or changed value is checked. Deleting a user clears the
-- attribution instead of deleting business records. Every step is guarded by
-- catalog lookups, so re-running this migration is a no-op.
-- ════════════════════════════════════════════════════════════════════════════

DO $$
DECLARE
    t TEXT;
BEGIN
    IF to_regclass('public.app_users') IS NULL THEN
        RAISE EXCEPTION 'migration 101 requires the app_users identity table';
    END IF;

    FOREACH t IN ARRAY ARRAY[
        'strategic_opportunities',
        'critical_threats',
        'supplier_risk',
        'pipeline_opportunities'
    ]
    LOOP
        IF to_regclass('public.' || t) IS NULL THEN
            CONTINUE;
        END IF;

        EXECUTE format('ALTER TABLE %I ADD COLUMN IF NOT EXISTS created_by TEXT', t);

        IF NOT EXISTS (
            SELECT 1 FROM pg_constraint
            WHERE conname = t || '_created_by_fkey'
              AND conrelid = to_regclass('public.' || t)
        ) THEN
            EXECUTE format(
                'ALTER TABLE %I ADD CONSTRAINT %I FOREIGN KEY (created_by) '
                'REFERENCES app_users(id) ON DELETE SET NULL NOT VALID',
                t, t || '_created_by_fkey'
            );
        END IF;

        IF EXISTS (
            SELECT 1 FROM information_schema.columns
            WHERE table_schema = 'public' AND table_name = t AND column_name = 'owner_id'
        ) AND NOT EXISTS (
            SELECT 1 FROM pg_constraint
            WHERE conname = t || '_owner_id_fkey'
              AND conrelid = to_regclass('public.' || t)
        ) THEN
            EXECUTE format(
                'ALTER TABLE %I ADD CONSTRAINT %I FOREIGN KEY (owner_id) '
                'REFERENCES app_users(id) ON DELETE SET NULL NOT VALID',
                t, t || '_owner_id_fkey'
            );
        END IF;

        EXECUTE format(
            'COMMENT ON COLUMN %I.created_by IS %L',
            t, 'app_users.id of the principal that created the row (migration 101)'
        );
    END LOOP;
END
$$;
