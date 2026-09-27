-- 069_login_throttle.sql
--
-- Audit item 1: the login attempt tracker existed only in process memory, so
-- restarts cleared it and multiple API replicas each kept their own counters.
-- A brute-force attempt could be spread across replicas (or resumed after a
-- deploy) and never trip the lockout.
--
-- This migration adds the durable login-throttle table: one row per attempt
-- key (normalised username + trusted client fingerprint). `PgStore` mutates it
-- under `SELECT ... FOR UPDATE`, so increments are atomic and all replicas
-- share one view. Windows and locks are time-bounded columns (TTL semantics),
-- and `expires_at` is a lazy-GC hint for rows that are no longer security
-- relevant. Every state is bounded: admin locks expire at their deadline
-- (24h, refreshed while failures continue), so unauthenticated traffic can
-- never mint rows that outlive the lock; `clear_lock` still releases one
-- earlier.
--
-- Idempotent: the table, index and grant are all guarded by IF NOT EXISTS /
-- catalog checks, so re-application is a no-op.

BEGIN;

CREATE TABLE IF NOT EXISTS login_attempt_throttle (
    attempt_key           TEXT        PRIMARY KEY,
    failure_count_10m     INTEGER     NOT NULL DEFAULT 0 CHECK (failure_count_10m >= 0),
    failure_count_1h      INTEGER     NOT NULL DEFAULT 0 CHECK (failure_count_1h >= 0),
    window_10m_started_at TIMESTAMPTZ,
    window_1h_started_at  TIMESTAMPTZ,
    backoff_until         TIMESTAMPTZ,
    temp_lock_until       TIMESTAMPTZ,
    admin_locked          BOOLEAN     NOT NULL DEFAULT FALSE,
    expires_at            TIMESTAMPTZ,
    updated_at            TIMESTAMPTZ NOT NULL DEFAULT now()
);

COMMENT ON TABLE login_attempt_throttle IS
    'Durable multi-replica login throttle: one row per normalised username + client fingerprint (migration 069)';
COMMENT ON COLUMN login_attempt_throttle.attempt_key IS
    'Opaque throttle key; callers build it from the normalised login name plus a trusted client fingerprint';
COMMENT ON COLUMN login_attempt_throttle.window_10m_started_at IS
    'Start of the current 10-minute sliding window; NULL when no failures are counted';
COMMENT ON COLUMN login_attempt_throttle.window_1h_started_at IS
    'Start of the current 1-hour sliding window; NULL when no failures are counted';
COMMENT ON COLUMN login_attempt_throttle.backoff_until IS
    'Progressive backoff deadline; the attempt is rejected while now() < backoff_until';
COMMENT ON COLUMN login_attempt_throttle.temp_lock_until IS
    'Temporary lock deadline after 10 failures in 10 minutes';
COMMENT ON COLUMN login_attempt_throttle.admin_locked IS
    'After 20 failures in 1 hour the key is locked until an administrator clears it or the bounded 24-hour deadline passes';
COMMENT ON COLUMN login_attempt_throttle.expires_at IS
    'Lazy TTL: rows with expires_at < now() are garbage and are pruned by the next read/write; for admin-locked rows it holds the bounded lock deadline';

-- Supports the lazy TTL sweep; partial so unlocked (security-relevant) rows
-- are not indexed.
CREATE INDEX IF NOT EXISTS idx_login_attempt_throttle_expires_at
    ON login_attempt_throttle (expires_at)
    WHERE expires_at IS NOT NULL;

-- Application role grants (production runs migrations as admin).
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'apexintel') THEN
        EXECUTE 'GRANT SELECT, INSERT, UPDATE, DELETE ON login_attempt_throttle TO apexintel';
    END IF;
END $$;

COMMIT;
