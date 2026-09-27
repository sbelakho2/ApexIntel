-- 073_app_users_role_check.sql
--
-- Audit item 3: `app_users.role` was an unconstrained TEXT column, and the
-- login path silently defaulted a malformed role to `analyst`. An unknown
-- role must never fail open, so this migration:
--
--   1. Canonicalises padded but known roles (e.g. `'admin '` -> `'admin'`),
--      matching the runtime, which trims before parsing. Without this step a
--      padded administrator would be silently demoted to viewer.
--   2. Downgrades remaining rows whose role is not one of the four known roles
--      to `viewer` — the least-privileged role (read-only) — and raises a
--      WARNING for each such row so the operator can see what changed. The
--      alternative (aborting the deployment) would leave the fail-open login
--      path live; downgrading is the fail-closed, availability-preserving
--      choice.
--   3. Adds `app_users_role_check` (NOT VALID, then VALIDATE) so the database
--      itself refuses unknown roles from now on.
--
-- Note: because sqlx runs the file as one transaction, the ADD CONSTRAINT and
-- VALIDATE below execute in the same transaction, so the ACCESS EXCLUSIVE
-- lock is held across the validation scan. app_users is small (operator
-- accounts), so the window is short; splitting them into separate migrations
-- would be required for a large table.
--
-- Idempotent: the UPDATE only touches invalid rows, the constraint is dropped
-- and re-added defensively, and VALIDATE on an already-valid constraint is a
-- no-op.

BEGIN;

-- Freeze writers while rows are normalised and the constraint is added, so a
-- concurrent insert cannot invalidate the scan or the VALIDATE step.
LOCK TABLE app_users IN SHARE ROW EXCLUSIVE MODE;

DO $$
DECLARE
    downgraded INTEGER;
BEGIN
    -- Padded but known roles keep their privilege: the runtime trims before
    -- parsing, so 'admin ' must not be demoted to viewer here.
    UPDATE app_users
    SET role = btrim(role),
        updated_at = now()
    WHERE role <> btrim(role)
      AND btrim(role) IN ('admin', 'analyst', 'viewer', 'service');

    UPDATE app_users
    SET role = 'viewer',
        updated_at = now()
    WHERE role IS NULL
       OR role NOT IN ('admin', 'analyst', 'viewer', 'service');
    GET DIAGNOSTICS downgraded = ROW_COUNT;

    IF downgraded > 0 THEN
        RAISE WARNING
            'migration 071: % app_users row(s) carried an unknown role and were downgraded to viewer',
            downgraded;
    END IF;
END $$;

ALTER TABLE app_users DROP CONSTRAINT IF EXISTS app_users_role_check;
ALTER TABLE app_users ADD CONSTRAINT app_users_role_check
    CHECK (role IN ('admin', 'analyst', 'viewer', 'service')) NOT VALID;
ALTER TABLE app_users VALIDATE CONSTRAINT app_users_role_check;

COMMENT ON COLUMN app_users.role IS
    'One of admin | analyst | viewer | service; enforced by app_users_role_check (migration 071) and by WebUser::api_role on the login path';

COMMIT;
