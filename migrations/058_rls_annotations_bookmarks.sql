-- 058_rls_annotations_bookmarks.sql
--
-- Audit P0 #7 (RLS completion): the remaining user-private tables must be
-- identity-scoped, not merely RLS-enabled.
--
--   1. `annotations` was created by migration 003 in its legacy shape
--      (author_id/author_name/content) while the application writes the 013
--      shape (user_id/body/tags/visibility). Because 013 uses
--      `CREATE TABLE IF NOT EXISTS`, the legacy shape won on fresh and
--      production databases, so every live annotation read/write failed with
--      "column does not exist" and the callers masked it as "no notes". This
--      migration reconciles the shape and backfills the identity column.
--   2. `annotations` and `insight_bookmarks` stayed ENABLE-only in 051 because
--      their call sites were unscoped; the scoped store methods
--      (`list_annotations_scoped`, `upsert_annotation_scoped`,
--      `bookmark_insight_scoped`, `unbookmark_insight_scoped`,
--      `is_insight_bookmarked_scoped`, `get_bookmarked_insight_ids_scoped`)
--      now live, so both can be FORCEd.
--   3. The audit of `information_schema` found other user-private tables with
--      a `user_id` column that were never forced (export_history,
--      insight_feedback_events, analyst_notifications, alert_preferences,
--      priority_queue, daily_priority_queue, notification_preferences,
--      bookmarks, insight_bookmark_collections, user_alert_subscriptions).
--      They are forced here too. Tables whose `user_id` is not a privacy
--      boundary (workspace_assignments, api_key_owners, analyst_user_roles,
--      weekly_memo_recipients) are deliberately excluded.
--
-- Every table in scope gets an owner policy (`user_id = current_user_id()`),
-- a `service` policy (so legitimate unscoped service/admin reads keep
-- working under the default connection identity), ENABLE + FORCE ROW LEVEL
-- SECURITY, and DML grants for the application role. Idempotent.
--
-- Verified by crates/store/tests/migrations_integration.rs
-- (`user_id_tables_are_explicitly_rls_classified`) and
-- crates/store/tests/rls_scoped_integration.rs.

-- ── 1. Reconcile the `annotations` shape ────────────────────────────────────
ALTER TABLE annotations ADD COLUMN IF NOT EXISTS user_id TEXT;
ALTER TABLE annotations ADD COLUMN IF NOT EXISTS body TEXT;
ALTER TABLE annotations ADD COLUMN IF NOT EXISTS tags TEXT[] NOT NULL DEFAULT ARRAY[]::TEXT[];
ALTER TABLE annotations ADD COLUMN IF NOT EXISTS visibility TEXT NOT NULL DEFAULT 'private';

-- Backfill the identity/body columns from the legacy shape when present.
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM information_schema.columns
               WHERE table_schema = 'public' AND table_name = 'annotations'
                 AND column_name = 'author_id') THEN
        EXECUTE 'UPDATE annotations SET user_id = author_id WHERE user_id IS NULL';
        EXECUTE 'ALTER TABLE annotations ALTER COLUMN author_id DROP NOT NULL';
    END IF;
    IF EXISTS (SELECT 1 FROM information_schema.columns
               WHERE table_schema = 'public' AND table_name = 'annotations'
                 AND column_name = 'author_name') THEN
        EXECUTE 'ALTER TABLE annotations ALTER COLUMN author_name DROP NOT NULL';
    END IF;
    IF EXISTS (SELECT 1 FROM information_schema.columns
               WHERE table_schema = 'public' AND table_name = 'annotations'
                 AND column_name = 'content') THEN
        EXECUTE 'UPDATE annotations SET body = content WHERE body IS NULL';
        EXECUTE 'ALTER TABLE annotations ALTER COLUMN content DROP NOT NULL';
    END IF;
END $$;

-- ── 2. Force RLS on every user-private `user_id` table ──────────────────────
DO $$
DECLARE
    t TEXT;
    -- `user_id` on these tables identifies an actor/owner in a shared record,
    -- not a private row set, so they are intentionally not identity-scoped.
    shared_system_tables TEXT[] := ARRAY[
        'workspace_assignments',
        'api_key_owners',
        'analyst_user_roles',
        'weekly_memo_recipients'
    ];
BEGIN
    FOR t IN
        SELECT c.table_name
        FROM information_schema.columns c
        JOIN information_schema.tables tb
          ON tb.table_schema = c.table_schema
         AND tb.table_name = c.table_name
        WHERE c.table_schema = 'public'
          AND c.column_name = 'user_id'
          AND tb.table_type = 'BASE TABLE'
          AND NOT (c.table_name = ANY(shared_system_tables))
        ORDER BY c.table_name
    LOOP
        EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);

        EXECUTE format('DROP POLICY IF EXISTS %I ON %I', t || '_owner_all', t);
        EXECUTE format(
            'CREATE POLICY %I ON %I FOR ALL
               USING (user_id = current_user_id())
               WITH CHECK (user_id = current_user_id())',
            t || '_owner_all', t
        );

        EXECUTE format('DROP POLICY IF EXISTS %I ON %I', t || '_service_all', t);
        EXECUTE format(
            'CREATE POLICY %I ON %I FOR ALL
               USING (current_setting(''app.current_user_role'', true) = ''service'')
               WITH CHECK (current_setting(''app.current_user_role'', true) = ''service'')',
            t || '_service_all', t
        );

        EXECUTE format('ALTER TABLE %I FORCE ROW LEVEL SECURITY', t);

        IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'apexintel') THEN
            EXECUTE format('GRANT SELECT, INSERT, UPDATE, DELETE ON %I TO apexintel', t);
        END IF;
    END LOOP;

    -- Annotation writes replace their tag assignments in the same scoped
    -- transaction, so the application role needs DML on the tag tables.
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'apexintel') THEN
        FOREACH t IN ARRAY ARRAY['tags', 'tag_assignments']
        LOOP
            IF EXISTS (SELECT 1 FROM pg_tables WHERE schemaname = 'public' AND tablename = t) THEN
                EXECUTE format('GRANT SELECT, INSERT, UPDATE, DELETE ON %I TO apexintel', t);
            END IF;
        END LOOP;
    END IF;
END $$;
