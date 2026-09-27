-- 072_app_users_unique_username.sql
--
-- Audit item 2: `app_users.username` was never unique (059 created a
-- non-unique index and 065 added another non-unique one), so
-- `find_app_user_by_username` had to guess between duplicates and login names
-- were not canonical. This migration makes the login name canonical:
--
--   1. Detects collisions between credential-bearing rows (two rows with the
--      same lower(username) that can both authenticate). That ambiguity cannot
--      be resolved safely — choosing either row would change who owns the
--      account — so the migration REFUSES with a clear error that lists the
--      conflicting names and ids. Operators resolve those rows manually and
--      re-run the migration.
--   2. Deterministically renames the non-canonical duplicates that CAN be
--      resolved: for each lower(username) group the canonical row is the
--      credential-bearing row (or the oldest row when none has credentials),
--      and every other row is renamed to `<username>~<id>` (with a numeric
--      suffix only if that exact name is taken). Renames are ordered by id, so
--      the outcome is identical on every replica and every re-run.
--   3. Creates `uq_app_users_username_ci`, the unique expression index that
--      keeps credential-bearing login names unique case-insensitively.
--
-- Idempotent: after a successful run no group has more than one canonical row
-- and the index already exists, so re-application is a no-op.

BEGIN;

-- Freeze writers while names are canonicalised and the index is built, so a
-- concurrent insert cannot invalidate the collision checks or the index build.
LOCK TABLE app_users IN SHARE ROW EXCLUSIVE MODE;

-- ── 1. Refuse to guess between credential-bearing collisions ───────────────
DO $$
DECLARE
    collisions TEXT;
BEGIN
    SELECT string_agg(
               format('%s (ids: %s)', username_key, ids),
               E'\n' ORDER BY username_key
           )
    INTO collisions
    FROM (
        SELECT lower(username) AS username_key,
               string_agg(id, ', ' ORDER BY id) AS ids
        FROM app_users
        WHERE password_hash IS NOT NULL
        GROUP BY lower(username)
        HAVING COUNT(*) > 1
    ) ambiguous;

    IF collisions IS NOT NULL THEN
        RAISE EXCEPTION USING
            MESSAGE = 'migration 070: ambiguous duplicate login names among credential-bearing app_users rows; resolve these rows manually, then re-run the migration',
            DETAIL = collisions,
            HINT = 'Keep the row whose credentials must win and rename or remove the other(s); the migration cannot choose safely.';
    END IF;
END $$;

-- ── 2. Deterministically rename non-canonical duplicates ───────────────────
DO $$
DECLARE
    rec RECORD;
    candidate TEXT;
    suffix INTEGER;
BEGIN
    FOR rec IN
        SELECT id, username
        FROM (
            SELECT id,
                   username,
                   row_number() OVER (
                       PARTITION BY lower(username)
                       ORDER BY (password_hash IS NOT NULL) DESC,
                                created_at ASC,
                                id ASC
                   ) AS rank
            FROM app_users
        ) ranked
        WHERE rank > 1
        ORDER BY id
    LOOP
        suffix := 0;
        LOOP
            candidate := CASE
                WHEN suffix = 0 THEN rec.username || '~' || rec.id
                ELSE rec.username || '~' || rec.id || '~' || suffix::TEXT
            END;
            EXIT WHEN NOT EXISTS (
                SELECT 1 FROM app_users
                WHERE lower(username) = lower(candidate)
                  AND id <> rec.id
            );
            suffix := suffix + 1;
        END LOOP;

        UPDATE app_users
        SET username = candidate,
            updated_at = now()
        WHERE id = rec.id;
    END LOOP;
END $$;

-- ── 3. Canonical, case-insensitive uniqueness for logins ───────────────────
-- Partial index: rows without credentials (identity placeholders for API keys,
-- bootstrap rows) are not logins and may keep legacy names.
CREATE UNIQUE INDEX IF NOT EXISTS uq_app_users_username_ci
    ON app_users (lower(username))
    WHERE password_hash IS NOT NULL;

COMMENT ON INDEX uq_app_users_username_ci IS
    'Canonical login names: at most one credential-bearing app_users row per lower(username) (migration 070)';

COMMIT;
