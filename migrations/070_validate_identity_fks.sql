-- ════════════════════════════════════════════════════════════════════════════
-- Migration 070: validate the canonical identity foreign keys (audit #13)
--
-- Migrations 059/065 added the `app_users(id)` foreign keys `NOT VALID` so
-- pre-existing orphan owners could not abort the migration. `NOT VALID` checks
-- every new write but leaves the old rows unverified, and nothing forced the
-- constraints to ever be validated. This migration closes that hole in one
-- atomic pass:
--
--   1. `identity_orphan_quarantine` — row-level archive of owners that cannot
--      be resolved; the complete row is preserved as JSONB, never dropped.
--   2. (Re-)creates any missing identity constraint as `NOT VALID`.
--   3. Maps recoverable orphan owners onto the canonical identity:
--        a. ids still present in `analyst_users` are re-adopted into
--           `app_users` (the legacy store was never backfilled again after
--           059);
--        b. whitespace/case variants that uniquely match one `app_users.id`;
--        c. whitespace/case variants that uniquely match one
--           `app_users.username`.
--   4. Quarantines the remaining impossible owners: each full row is archived
--      into `identity_orphan_quarantine` and removed from the owning table.
--   5. Asserts loudly that no orphan owner remains, `VALIDATE CONSTRAINT` for
--      every identity FK, then asserts every one is `convalidated`.
--
-- Policy: the migration refuses to guess. An owner that matches more than one
-- canonical user is quarantined rather than mis-attributed, and any orphan
-- that survives remediation (for example because RLS hid it from the
-- migration) aborts the migration loudly instead of shipping a constraint
-- that only looks validated.
--
-- Idempotent: constraints are re-added only when missing, remediation is a
-- no-op once no orphans remain, and VALIDATE/assertions are safe to re-run.
-- ════════════════════════════════════════════════════════════════════════════

BEGIN;

-- ── 1. Row-level quarantine archive ─────────────────────────────────────────
CREATE TABLE IF NOT EXISTS identity_orphan_quarantine (
    id              UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    source_table    TEXT        NOT NULL,
    constraint_name TEXT        NOT NULL,
    owner_id        TEXT,
    row_data        JSONB       NOT NULL,
    reason          TEXT        NOT NULL,
    quarantined_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

COMMENT ON TABLE identity_orphan_quarantine IS
    'Full rows whose canonical-app_users owner could not be resolved, archived by migration 070 before being removed from the owning table.';
COMMENT ON COLUMN identity_orphan_quarantine.owner_id IS
    'The user_id the row pointed at when it could not be mapped to app_users.id.';

CREATE INDEX IF NOT EXISTS idx_identity_orphan_quarantine_table
    ON identity_orphan_quarantine (source_table, quarantined_at DESC);

-- ── 2. Assertion helpers ────────────────────────────────────────────────────
-- Every single-column foreign key on `user_id` referencing `app_users` is an
-- identity FK; the report lists the ones that still own orphan rows.
CREATE OR REPLACE FUNCTION identity_orphan_report()
RETURNS TABLE (source_table TEXT, constraint_name TEXT, orphan_count BIGINT)
LANGUAGE plpgsql STABLE AS $$
DECLARE
    r RECORD;
    n BIGINT;
BEGIN
    FOR r IN
        SELECT c.conname, c.conrelid::regclass AS tbl
        FROM pg_constraint c
        JOIN pg_attribute a
          ON a.attrelid = c.conrelid
         AND a.attnum = c.conkey[1]
         AND a.attname = 'user_id'
        WHERE c.contype = 'f'
          AND c.confrelid = to_regclass('public.app_users')
          AND array_length(c.conkey, 1) = 1
        ORDER BY c.conrelid::regclass::text, c.conname
    LOOP
        EXECUTE format(
            'SELECT COUNT(*) FROM %s t
              WHERE t.user_id IS NOT NULL
                AND NOT EXISTS (SELECT 1 FROM app_users u WHERE u.id = t.user_id)',
            r.tbl
        ) INTO n;

        IF n > 0 THEN
            source_table := r.tbl::text;
            constraint_name := r.conname;
            orphan_count := n;
            RETURN NEXT;
        END IF;
    END LOOP;
END;
$$;

COMMENT ON FUNCTION identity_orphan_report() IS
    'Identity FKs that still own rows whose user_id is not present in app_users (empty when the schema is clean).';

CREATE OR REPLACE FUNCTION assert_no_identity_orphans()
RETURNS void
LANGUAGE plpgsql AS $$
DECLARE
    r RECORD;
    v_details TEXT := '';
BEGIN
    FOR r IN SELECT * FROM identity_orphan_report() LOOP
        v_details := v_details
            || format('%s(%s): %s row(s); ',
                      r.source_table, r.constraint_name, r.orphan_count);
    END LOOP;

    IF v_details <> '' THEN
        RAISE EXCEPTION 'identity orphan owners remain: %', v_details;
    END IF;
END;
$$;

COMMENT ON FUNCTION assert_no_identity_orphans() IS
    'Raises when any identity FK still owns rows with no matching app_users row.';

CREATE OR REPLACE FUNCTION assert_identity_fks_validated()
RETURNS void
LANGUAGE plpgsql AS $$
DECLARE
    v_details TEXT;
BEGIN
    SELECT string_agg(format('%s on %s', c.conname, c.conrelid::regclass),
                      ', ' ORDER BY c.conname)
    INTO v_details
    FROM pg_constraint c
    JOIN pg_attribute a
      ON a.attrelid = c.conrelid
     AND a.attnum = c.conkey[1]
     AND a.attname = 'user_id'
    WHERE c.contype = 'f'
      AND c.confrelid = to_regclass('public.app_users')
      AND array_length(c.conkey, 1) = 1
      AND NOT c.convalidated;

    IF v_details IS NOT NULL THEN
        RAISE EXCEPTION 'identity foreign keys left NOT VALID: %', v_details;
    END IF;
END;
$$;

COMMENT ON FUNCTION assert_identity_fks_validated() IS
    'Raises when any single-column user_id foreign key to app_users is still NOT VALID.';

-- ── 3. Remediation ──────────────────────────────────────────────────────────
DO $$
DECLARE
    t TEXT;
    c TEXT;
    identity_tables TEXT[] := ARRAY[
        'user_preferences',
        'watchlists',
        'annotations',
        'insight_bookmarks',
        'user_alert_subscriptions',
        'saved_searches'
    ];
BEGIN
    FOREACH t IN ARRAY identity_tables
    LOOP
        IF to_regclass('public.' || t) IS NULL THEN
            CONTINUE;
        END IF;
        IF NOT EXISTS (
            SELECT 1 FROM information_schema.columns
            WHERE table_schema = 'public' AND table_name = t
              AND column_name = 'user_id'
        ) THEN
            CONTINUE;
        END IF;

        c := t || '_user_id_fkey';

        IF NOT EXISTS (
            SELECT 1 FROM pg_constraint
            WHERE conname = c AND conrelid = to_regclass('public.' || t)
        ) THEN
            EXECUTE format(
                'ALTER TABLE %I ADD CONSTRAINT %I
                   FOREIGN KEY (user_id) REFERENCES app_users(id)
                   ON DELETE CASCADE NOT VALID',
                t, c
            );
        END IF;

        -- (a) Re-adopt legacy analyst_users identities that still own rows.
        IF to_regclass('public.analyst_users') IS NOT NULL THEN
            EXECUTE format($fmt$
                INSERT INTO app_users
                    (id, username, display_name, email, role, enabled,
                     created_at, updated_at)
                SELECT au.id,
                       au.id,
                       COALESCE(NULLIF(to_jsonb(au)->>'display_name', ''), au.id),
                       to_jsonb(au)->>'email',
                       COALESCE(NULLIF(to_jsonb(au)->>'role', ''), 'analyst'),
                       COALESCE((to_jsonb(au)->>'is_active')::boolean, TRUE),
                       COALESCE((to_jsonb(au)->>'created_at')::timestamptz, now()),
                       COALESCE((to_jsonb(au)->>'updated_at')::timestamptz, now())
                FROM analyst_users au
                WHERE au.id IS NOT NULL
                  AND au.id <> ''
                  AND EXISTS (SELECT 1 FROM %I src WHERE src.user_id = au.id)
                ON CONFLICT (id) DO NOTHING
            $fmt$, t);
        END IF;

        -- (b) Unique whitespace/case variant of a canonical id.
        EXECUTE format($fmt$
            UPDATE %I t
            SET user_id = u.id
            FROM app_users u
            WHERE t.user_id IS NOT NULL
              AND NOT EXISTS (SELECT 1 FROM app_users x WHERE x.id = t.user_id)
              AND lower(btrim(t.user_id)) = lower(btrim(u.id))
              AND u.id <> t.user_id
              AND (SELECT COUNT(*) FROM app_users u2
                   WHERE lower(btrim(u2.id)) = lower(btrim(t.user_id))) = 1
        $fmt$, t);

        -- (c) Unique whitespace/case variant of a canonical username.
        EXECUTE format($fmt$
            UPDATE %I t
            SET user_id = u.id
            FROM app_users u
            WHERE t.user_id IS NOT NULL
              AND NOT EXISTS (SELECT 1 FROM app_users x WHERE x.id = t.user_id)
              AND lower(btrim(t.user_id)) = lower(btrim(u.username))
              AND (SELECT COUNT(*) FROM app_users u2
                   WHERE lower(btrim(u2.username)) = lower(btrim(t.user_id))) = 1
        $fmt$, t);

        -- (d) Everything left is impossible to map: archive then remove.
        EXECUTE format($fmt$
            INSERT INTO identity_orphan_quarantine
                (source_table, constraint_name, owner_id, row_data, reason)
            SELECT %L, %L, t.user_id, to_jsonb(t),
                   'owner is not present in app_users and could not be mapped; archived by migration 070'
            FROM %I t
            WHERE t.user_id IS NOT NULL
              AND NOT EXISTS (SELECT 1 FROM app_users u WHERE u.id = t.user_id)
        $fmt$, t, c, t);

        EXECUTE format($fmt$
            DELETE FROM %I t
            WHERE t.user_id IS NOT NULL
              AND NOT EXISTS (SELECT 1 FROM app_users u WHERE u.id = t.user_id)
        $fmt$, t);
    END LOOP;
END $$;

-- ── 4. Fail loudly, then validate every identity FK ─────────────────────────
SELECT assert_no_identity_orphans();

DO $$
DECLARE
    r RECORD;
BEGIN
    FOR r IN
        SELECT c.conname, c.conrelid::regclass AS tbl
        FROM pg_constraint c
        JOIN pg_attribute a
          ON a.attrelid = c.conrelid
         AND a.attnum = c.conkey[1]
         AND a.attname = 'user_id'
        WHERE c.contype = 'f'
          AND c.confrelid = to_regclass('public.app_users')
          AND array_length(c.conkey, 1) = 1
        ORDER BY c.conname
    LOOP
        EXECUTE format('ALTER TABLE %s VALIDATE CONSTRAINT %I',
                       r.tbl, r.conname);
    END LOOP;
END $$;

SELECT assert_identity_fks_validated();

COMMIT;
