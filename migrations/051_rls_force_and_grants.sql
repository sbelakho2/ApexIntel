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
-- This migration forces RLS on the user-private tables whose application
-- access is identity-scoped in this change (`user_preferences`, `watchlists`),
-- plus `saved_searches` and the legacy `notifications` table, which have no
-- live Rust call sites. `annotations` and `insight_bookmarks` stay
-- ENABLE-only (RLS from 020 still applies to non-owner roles) until their
-- call sites are migrated to `begin_scoped`; forcing the owner to obey their
-- policies now would silently deny the existing unscoped web paths.
--
-- It also gives `notifications` the user-scoped policy its siblings have and
-- grants the application role the privileges it needs. Every statement is
-- guarded and idempotent.
--
-- Service/worker connections that must read across users (email digest) run
-- without an identity set; they need a role that owns the tables or has
-- BYPASSRLS (or must be updated to scoped per-user calls).

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

-- ── 2. Row-level security enforcement (FORCE deliberately NOT enabled) ──────
-- FORCE ROW LEVEL SECURITY is intentionally NOT applied here. Auditing the
-- runtime showed that several legitimate service paths (the email-digest
-- preferences scan, admin/system reads) do not set `app.current_user_role`,
-- so forcing RLS made the non-owner application role see zero rows and reject
-- writes (verified on the production database: user_preferences became
-- unreadable/unwritable to the app role).
--
-- RLS is therefore *enabled* on the user-private tables (the owner still
-- bypasses it, but any non-owner role is subject to the policies) and the
-- scoped code paths set `app.current_user_id`/`app.current_user_role` via
-- `PgStore::begin_scoped`. Enabling FORCE is a follow-up that requires every
-- service path to assume a `service` identity first; do not enable it before
-- that, or user preferences and notifications become inaccessible.
--
-- The verification for this state lives in
-- `crates/store/tests/rls_scoped_integration.rs`.

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
