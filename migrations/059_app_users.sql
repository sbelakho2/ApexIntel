-- ════════════════════════════════════════════════════════════════════════════
-- Migration 059: canonical `app_users` identity table
--
-- Audit P0: user identities were stored as unconstrained `TEXT` columns
-- (`user_preferences.user_id`, `watchlists.user_id`, `annotations.user_id`,
-- `insight_bookmarks.user_id`, `user_alert_subscriptions.user_id`). Any typo
-- or renamed principal silently created an orphan identity, and deleting a
-- user left their rows behind.
--
-- This migration:
--   1. creates `app_users` as the canonical identity table;
--   2. backfills it from `analyst_users` and from every distinct `user_id`
--      already present in the user-owned tables (so existing data keeps its
--      owner);
--   3. adds guarded `ON DELETE CASCADE` foreign keys from those tables to
--      `app_users(id)`.
--
-- Safety on existing data: the foreign keys are added `NOT VALID`, so rows
-- whose owner no longer exists do not abort the migration, while every new
-- write is checked and deletes cascade. Backfill uses `ON CONFLICT DO NOTHING`
-- and constraint/column creation is guarded by catalog lookups, so re-running
-- this migration is a no-op.
-- ════════════════════════════════════════════════════════════════════════════

BEGIN;

-- ── 1. Canonical identity table ─────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS app_users (
    id            TEXT        PRIMARY KEY,
    username      TEXT        NOT NULL,
    display_name  TEXT        NOT NULL DEFAULT '',
    email         TEXT,
    role          TEXT        NOT NULL DEFAULT 'analyst',
    is_active     BOOLEAN     NOT NULL DEFAULT TRUE,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_login_at TIMESTAMPTZ
);

-- Non-unique: legacy `analyst_users` rows may share a username across distinct
-- ids, and a unique index would abort the migration on such data. Login is
-- keyed by id; the index only speeds username lookups.
CREATE INDEX IF NOT EXISTS idx_app_users_username ON app_users (username);

COMMENT ON TABLE app_users IS
    'Canonical application identity; user-owned tables reference app_users(id) ON DELETE CASCADE';
COMMENT ON COLUMN app_users.id IS
    'Stable principal id from the signed session / API key owner (WEB_USERS_JSON `id`)';
COMMENT ON COLUMN app_users.last_login_at IS
    'Updated by PgStore::ensure_app_user on every successful login';

-- ── 2. Backfill identities already present in user-owned data ───────────────
-- Legacy analyst_users rows carry the richest profile, so seed from there
-- first; then cover every distinct user_id in the user-owned tables.
DO $$
BEGIN
    IF to_regclass('public.analyst_users') IS NOT NULL THEN
        INSERT INTO app_users (id, username, display_name, email, role, is_active, created_at, updated_at)
        SELECT id, id, COALESCE(NULLIF(display_name, ''), id), email, COALESCE(NULLIF(role, ''), 'analyst'),
               COALESCE(is_active, TRUE), COALESCE(created_at, now()), COALESCE(updated_at, now())
        FROM analyst_users
        WHERE id IS NOT NULL AND id <> ''
        ON CONFLICT (id) DO NOTHING;
    END IF;
END $$;

DO $$
DECLARE
    t TEXT;
    identity_tables TEXT[] := ARRAY[
        'user_preferences',
        'watchlists',
        'annotations',
        'insight_bookmarks',
        'user_alert_subscriptions'
    ];
BEGIN
    FOREACH t IN ARRAY identity_tables
    LOOP
        IF to_regclass('public.' || t) IS NULL THEN
            CONTINUE;
        END IF;
        IF NOT EXISTS (
            SELECT 1 FROM information_schema.columns
            WHERE table_schema = 'public' AND table_name = t AND column_name = 'user_id'
        ) THEN
            CONTINUE;
        END IF;
        EXECUTE format(
            'INSERT INTO app_users (id, username, display_name)
             SELECT DISTINCT user_id, user_id, user_id
             FROM %I
             WHERE user_id IS NOT NULL AND user_id <> ''''
             ON CONFLICT (id) DO NOTHING',
            t
        );
    END LOOP;
END $$;

-- ── 3. Guarded foreign keys to the canonical identity ───────────────────────
-- `ADD COLUMN IF NOT EXISTS user_id` keeps the block safe if a table predates
-- the identity column; the catalog guard makes re-running a no-op. NOT VALID
-- keeps pre-existing orphan rows from aborting the migration; new writes are
-- still checked and parent deletes still cascade.
DO $$
DECLARE
    t TEXT;
    c TEXT;
    identity_tables TEXT[] := ARRAY[
        'user_preferences',
        'watchlists',
        'annotations',
        'insight_bookmarks',
        'user_alert_subscriptions'
    ];
BEGIN
    FOREACH t IN ARRAY identity_tables
    LOOP
        IF to_regclass('public.' || t) IS NULL THEN
            CONTINUE;
        END IF;
        EXECUTE format('ALTER TABLE %I ADD COLUMN IF NOT EXISTS user_id TEXT', t);
        c := t || '_user_id_fkey';
        IF NOT EXISTS (
            SELECT 1 FROM pg_constraint
            WHERE conname = c AND conrelid = to_regclass('public.' || t)
        ) THEN
            EXECUTE format(
                'ALTER TABLE %I ADD CONSTRAINT %I
                   FOREIGN KEY (user_id) REFERENCES app_users(id) ON DELETE CASCADE NOT VALID',
                t, c
            );
        END IF;
    END LOOP;
END $$;

-- ── 4. Application role grants (production runs migrations as admin) ────────
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'apexintel') THEN
        EXECUTE 'GRANT SELECT, INSERT, UPDATE, DELETE ON app_users TO apexintel';
    END IF;
END $$;

COMMIT;
