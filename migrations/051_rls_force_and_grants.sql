-- 051_rls_force_and_grants.sql
--
-- Audit P0 #7 — RLS must be bound to application identity.
--
-- Migration 020 enabled RLS and created owner-scoped policies for user-private
-- tables, but those policies were never exercisable:
--   1. the tables are owned by the migration/admin role, and table owners
--      bypass RLS unless FORCE ROW LEVEL SECURITY is set;
--   2. the application role (`apexintel`) never received table grants, and
--      `app.current_user_id` / `app.current_user_role` were never set by the
--      application. `PgStore::begin_scoped` now sets both transaction-locally.
--
-- This migration forces RLS on the user-private tables that already carry
-- policies in 020, gives the legacy `notifications` table the same
-- user-scoped policy its siblings have, and grants the application role the
-- privileges it needs. Every statement is guarded and idempotent.
--
-- Service/worker connections that must read across users (email digest,
-- alert routing) intentionally run without an identity set; they need a role
-- that owns the tables or has BYPASSRLS (or must be updated to scoped calls).

-- ── 1. User-scoped policy for `notifications` (no policy existed in 020) ────
-- FORCE without a policy would deny every access; the table is user-scoped,
-- so mirror the owner policy used for the other personal tables.
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_tables WHERE schemaname = 'public' AND tablename = 'notifications')
       AND EXISTS (SELECT 1 FROM information_schema.columns
                   WHERE table_schema = 'public' AND table_name = 'notifications'
                     AND column_name = 'user_id') THEN
        EXECUTE 'ALTER TABLE notifications ENABLE ROW LEVEL SECURITY';
        EXECUTE 'DROP POLICY IF EXISTS notifications_owner_all ON notifications';
        EXECUTE 'CREATE POLICY notifications_owner_all ON notifications
                 FOR ALL
                 USING (user_id = current_user_id())
                 WITH CHECK (user_id = current_user_id())';
    END IF;
END $$;

-- ── 2. FORCE ROW LEVEL SECURITY on user-private tables ──────────────────────
-- The owner (migration/admin role) must obey the policies too, so a coding
-- mistake that skips `begin_scoped` cannot silently see another user's data.
DO $$
DECLARE
    t TEXT;
    user_private_tables TEXT[] := ARRAY[
        'user_preferences',
        'watchlists',
        'saved_searches',
        'annotations',
        'insight_bookmarks',
        'notifications'
    ];
BEGIN
    FOREACH t IN ARRAY user_private_tables
    LOOP
        IF EXISTS (SELECT 1 FROM pg_tables WHERE schemaname = 'public' AND tablename = t) THEN
            EXECUTE format('ALTER TABLE %I FORCE ROW LEVEL SECURITY', t);
        END IF;
    END LOOP;
END $$;

-- ── 3. Grants for the application role ──────────────────────────────────────
-- In production migrations run as an admin role while the application connects
-- as `apexintel`; without DML grants on these tables every scoped call fails
-- with "permission denied" before RLS is even consulted.
DO $$
DECLARE
    t TEXT;
    app_tables TEXT[] := ARRAY[
        'user_preferences',
        'watchlists',
        'saved_searches',
        'annotations',
        'insight_bookmarks',
        'notifications'
    ];
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'apexintel') THEN
        FOREACH t IN ARRAY app_tables
        LOOP
            IF EXISTS (SELECT 1 FROM pg_tables WHERE schemaname = 'public' AND tablename = t) THEN
                EXECUTE format('GRANT SELECT, INSERT, UPDATE, DELETE ON %I TO apexintel', t);
            END IF;
        END LOOP;
    END IF;
END $$;
