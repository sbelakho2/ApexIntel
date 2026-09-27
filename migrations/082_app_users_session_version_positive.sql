-- 082_app_users_session_version_positive.sql
--
-- Audit P0-2: every signed browser cookie carries a `session_version` claim
-- (`sv`) that must match the canonical `app_users` row. A cookie without the
-- principal claims, or with `sv = 0`, is a pre-principal legacy cookie and is
-- rejected — it forces reauthentication instead of being upgraded to a
-- default role.
--
-- `session_version` was added NOT NULL DEFAULT 1 by
-- `065_app_users_credentials.sql`, but nothing prevented a zero/negative value
-- from being written later. This migration
-- makes the runtime contract explicit in the schema:
--
--   1. normalise any row with a version below 1 to 1 (versions are only ever
--      incremented to revoke sessions, so no valid session can carry the old
--      value);
--   2. add `app_users_session_version_positive` (CHECK session_version >= 1),
--      mirroring the runtime, which treats `sv <= 0` as stale.
--
-- Idempotent: the UPDATE only touches invalid rows and the constraint is only
-- added when absent.

BEGIN;

UPDATE app_users SET session_version = 1 WHERE session_version < 1;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'app_users_session_version_positive'
          AND conrelid = to_regclass('public.app_users')
    ) THEN
        ALTER TABLE app_users
            ADD CONSTRAINT app_users_session_version_positive
            CHECK (session_version >= 1);
    END IF;
END $$;

COMMENT ON COLUMN app_users.session_version IS
    'Signed into session cookies as `sv` and must be >= 1; a cookie whose sv does not match (or is 0) is revoked and forces reauthentication';

COMMIT;
