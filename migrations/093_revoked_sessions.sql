-- ════════════════════════════════════════════════════════════════════════════
-- Migration 093: revoked browser sessions
-- ════════════════════════════════════════════════════════════════════════════
--
-- Logout previously only cleared the cookie, leaving a copied cookie valid
-- until its signed expiry (up to 168h). Each session now carries a `jti`
-- claim; logout records it here and the session authority rejects any session
-- whose `jti` is revoked. Rows are purged from the status heartbeat once the
-- session's own expiry passes, so the table stays bounded by sessions revoked
-- within the maximum session lifetime.
--
-- Idempotent: safe to re-apply.

CREATE TABLE IF NOT EXISTS revoked_sessions (
    jti UUID PRIMARY KEY,
    expires_at TIMESTAMPTZ NOT NULL,
    revoked_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_revoked_sessions_expires_at
    ON revoked_sessions (expires_at);

-- Runtime role grants (production connects as a non-owner role).
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'apexintel_app') THEN
        GRANT SELECT, INSERT, DELETE ON revoked_sessions TO apexintel_app;
    END IF;
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'apexintel') THEN
        GRANT SELECT, INSERT, DELETE ON revoked_sessions TO apexintel;
    END IF;
END $$;
