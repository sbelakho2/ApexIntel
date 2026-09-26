-- ──────────────────────────────────────────────────────────────────────────────
-- Migration 069: alert-rule engine state
--
-- Readiness must prove the alert-rule engine is actually loaded, not merely
-- that a YAML file exists: `/api/health/ready` under APEX_PROFILE=full requires
-- rule_count > 0, last_reload_success = TRUE, a non-empty config_hash, and a
-- freshly refreshed row. The worker writes this singleton row at startup and
-- refreshes it (re-reading/re-hashing the rules file) on its heartbeat cadence,
-- so a stale row means the worker stopped refreshing engine state — a real
-- operational signal instead of a configuration assumption.
--
-- Idempotent (IF NOT EXISTS); does not touch any applied migration (<= 068).
-- ──────────────────────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS alert_engine_state (
    id                  TEXT        PRIMARY KEY DEFAULT 'default',
    rules_path          TEXT        NOT NULL,
    rule_count          INTEGER     NOT NULL DEFAULT 0,
    config_hash         TEXT,
    last_reload_success BOOLEAN     NOT NULL DEFAULT FALSE,
    last_reload_error   TEXT,
    last_reload_at      TIMESTAMPTZ,
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now()
);

COMMENT ON TABLE alert_engine_state IS
    'Singleton row exposing the live alert-rule engine state (rule count, config hash, reload result) for readiness probes';
COMMENT ON COLUMN alert_engine_state.config_hash IS
    'SHA-256 of the exact rules file content the worker parsed; NULL until a successful read';
COMMENT ON COLUMN alert_engine_state.last_reload_success IS
    'FALSE when the rules file was missing/unparseable/empty at the last refresh; the row still refreshes so staleness remains measurable';

-- The application connects as a non-owner role in production; new tables need
-- explicit grants (same pattern as 050_heartbeat_grants.sql / 061_event_outbox.sql).
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'apexintel') THEN
        GRANT SELECT, INSERT, UPDATE, DELETE ON alert_engine_state TO apexintel;
    END IF;
END $$;
