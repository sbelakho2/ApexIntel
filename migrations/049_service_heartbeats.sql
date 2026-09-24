-- 049_service_heartbeats.sql
--
-- Health/freshness must be measured, not hard-coded. This adds a per-instance
-- liveness table written by the API (service='api') and the worker
-- (service='worker') roughly every 30 seconds. Health and capability checks
-- read `last_seen_at` to distinguish "the worker is alive" from "the worker
-- silently stopped" instead of assuming the system is online.
--
-- Append-only and idempotent (IF NOT EXISTS guards), safe to run against
-- existing deployments. Does not touch any applied migration (<= 048).

CREATE TABLE IF NOT EXISTS service_heartbeats (
    id BIGSERIAL PRIMARY KEY,
    service TEXT NOT NULL,
    instance_id TEXT NOT NULL,
    version TEXT NOT NULL DEFAULT '',
    last_seen_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT service_heartbeats_service_instance_key UNIQUE (service, instance_id)
);

CREATE INDEX IF NOT EXISTS idx_service_heartbeats_service_last_seen
    ON service_heartbeats (service, last_seen_at DESC);

-- Bound the table so it cannot grow without limit if instance ids churn
-- (containers get new hostnames/ids on redeploy). Idempotent by nature.
DELETE FROM service_heartbeats
WHERE last_seen_at < NOW() - INTERVAL '30 days';
