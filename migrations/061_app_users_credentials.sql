-- ════════════════════════════════════════════════════════════════════════════
-- Migration 061: credentials and session state on the canonical identity
--
-- Audit P0: environment credentials (`APEX_ADMIN_*`, `WEB_USERS_JSON`) were the
-- only login database, so the `app_users` rows introduced by 059 could not
-- authenticate and two identity stores disagreed. This migration makes
-- `app_users` login-authoritative:
--
--   * `password_hash` — Argon2id PHC string (legacy SHA-256 hex is still
--     verified with a warning by the login handler). NULL means the row has no
--     credentials yet; environment configuration bootstraps those rows only.
--   * `session_version` — signed into new session cookies so sessions can be
--     invalidated by bumping it.
--   * `is_active` is renamed to `enabled` to match the login contract.
--   * `saved_searches.user_id` gets the same `app_users(id) ON DELETE CASCADE`
--     foreign key as the other user-owned tables (059 omitted it).
--
-- Every statement is guarded and idempotent so re-running is a no-op, and 059
-- is left untouched (it has already been applied to production and its
-- checksum must not change).
-- ════════════════════════════════════════════════════════════════════════════

BEGIN;

-- ── 1. `enabled` replaces `is_active` ───────────────────────────────────────
DO $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_schema = 'public' AND table_name = 'app_users'
          AND column_name = 'is_active'
    ) AND NOT EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_schema = 'public' AND table_name = 'app_users'
          AND column_name = 'enabled'
    ) THEN
        EXECUTE 'ALTER TABLE app_users RENAME COLUMN is_active TO enabled';
    ELSIF NOT EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_schema = 'public' AND table_name = 'app_users'
          AND column_name = 'enabled'
    ) THEN
        EXECUTE 'ALTER TABLE app_users ADD COLUMN enabled BOOLEAN NOT NULL DEFAULT TRUE';
    END IF;
END $$;

-- ── 2. Login credentials and session version ────────────────────────────────
ALTER TABLE app_users ADD COLUMN IF NOT EXISTS password_hash TEXT;
ALTER TABLE app_users ADD COLUMN IF NOT EXISTS session_version INTEGER NOT NULL DEFAULT 1;

-- Login resolves a user name to at most one credential-bearing row.
CREATE INDEX IF NOT EXISTS idx_app_users_username_credentials
    ON app_users (username)
    WHERE password_hash IS NOT NULL;

-- ── 3. Canonical identity for saved searches ────────────────────────────────
-- 059 gave the other user-owned tables an `app_users(id)` foreign key but left
-- `saved_searches` out (it had no live call sites then; migration 057 already
-- FORCEd RLS on it). Now that saved searches have a product surface, its
-- `user_id` must reference the canonical identity too. NOT VALID keeps
-- pre-existing orphan rows from aborting the migration; new writes are still
-- checked and parent deletes still cascade.
DO $$
BEGIN
    IF to_regclass('public.saved_searches') IS NOT NULL
       AND EXISTS (
           SELECT 1 FROM information_schema.columns
           WHERE table_schema = 'public' AND table_name = 'saved_searches'
             AND column_name = 'user_id'
       )
       AND NOT EXISTS (
           SELECT 1 FROM pg_constraint
           WHERE conname = 'saved_searches_user_id_fkey'
             AND conrelid = to_regclass('public.saved_searches')
       )
    THEN
        EXECUTE 'ALTER TABLE saved_searches
                   ADD CONSTRAINT saved_searches_user_id_fkey
                   FOREIGN KEY (user_id) REFERENCES app_users(id) ON DELETE CASCADE NOT VALID';
    END IF;
END $$;

COMMENT ON COLUMN app_users.password_hash IS
    'Argon2id PHC string (legacy SHA-256 hex accepted with a warning); NULL = row has no credentials yet, environment bootstrap fills it';
COMMENT ON COLUMN app_users.session_version IS
    'Signed into new session cookies; bump to invalidate a user''s sessions';
COMMENT ON COLUMN app_users.enabled IS
    'Disabled rows can never authenticate or mint a session';
COMMENT ON COLUMN app_users.last_login_at IS
    'Updated by PgStore::record_app_user_login on every successful login';

COMMIT;
