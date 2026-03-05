-- ════════════════════════════════════════════════════════════════════════════
-- Materialized Views for Dashboard Performance
-- Pre-computed aggregations refreshed by nightly worker job.
-- Run: psql $DATABASE_URL < migrations/20260301_materialized_views.sql
-- ════════════════════════════════════════════════════════════════════════════

BEGIN;

-- Warning summary by region, severity, and date
CREATE MATERIALIZED VIEW IF NOT EXISTS mv_warning_summary AS
SELECT
    date_trunc('day', created_at)::date AS day,
    COALESCE(region, '__null__') AS region,
    severity,
    warning_type,
    COUNT(*)                                          AS total_count,
    COUNT(*) FILTER (WHERE acknowledged)              AS ack_count,
    AVG(EXTRACT(EPOCH FROM
        COALESCE(acknowledged_at, now()) - created_at
    ))                                                AS avg_response_secs
FROM warnings
WHERE created_at > now() - interval '90 days'
GROUP BY 1, 2, 3, 4;

CREATE UNIQUE INDEX IF NOT EXISTS idx_mv_warn_summary
    ON mv_warning_summary (day, region, severity, warning_type);

-- Company risk leaderboard
CREATE MATERIALIZED VIEW IF NOT EXISTS mv_company_risk_leaderboard AS
SELECT
    c.id,
    c.name,
    c.domain,
    c.region,
    c.company_type,
    c.threat_score,
    c.overlap_score,
    c.risk_score,
    COUNT(DISTINCT w.id)        AS active_warnings,
    COUNT(DISTINCT i.id)        AS recent_insights,
    MAX(w.created_at)           AS last_warning_at
FROM companies c
LEFT JOIN warnings w ON c.id = ANY(w.entity_ids) AND NOT w.acknowledged
LEFT JOIN insights i ON c.id = ANY(i.entity_ids) AND i.created_at > now() - interval '30 days'
GROUP BY c.id, c.name, c.domain, c.region, c.company_type,
         c.threat_score, c.overlap_score, c.risk_score;

CREATE UNIQUE INDEX IF NOT EXISTS idx_mv_company_risk
    ON mv_company_risk_leaderboard (id);

-- POI coverage by region and role family
CREATE MATERIALIZED VIEW IF NOT EXISTS mv_poi_coverage AS
SELECT
    COALESCE(p.region, '__null__') AS region,
    COALESCE(p.role_family, '__null__') AS role_family,
    COUNT(*)                                                  AS total_pois,
    COUNT(*) FILTER (WHERE p.influence_score > 50)            AS high_influence,
    AVG(p.influence_score)                                    AS avg_influence,
    AVG(p.pain_index)                                         AS avg_pain_index,
    COUNT(*) FILTER (WHERE p.updated_at > now() - interval '7 days') AS refreshed_last_week
FROM persons p
GROUP BY 1, 2;

CREATE UNIQUE INDEX IF NOT EXISTS idx_mv_poi_coverage
    ON mv_poi_coverage (region, role_family);

-- Recipe performance summary
CREATE MATERIALIZED VIEW IF NOT EXISTS mv_recipe_performance AS
SELECT
    r.code              AS recipe_code,
    r.name,
    r.status,
    r.precision_score,
    COUNT(DISTINCT w.id) AS warnings_generated_30d
FROM recipes r
LEFT JOIN warnings w ON w.recipe_code = r.code AND w.created_at > now() - interval '30 days'
GROUP BY r.code, r.name, r.status, r.precision_score;

CREATE UNIQUE INDEX IF NOT EXISTS idx_mv_recipe_perf
    ON mv_recipe_performance (recipe_code);

-- Observation volume by type and day (for drift detection — Improvement #28)
CREATE MATERIALIZED VIEW IF NOT EXISTS mv_observation_volume AS
SELECT
    date_trunc('day', ts_utc)::date AS day,
    observation_type,
    COALESCE(entity_type, '__null__') AS entity_type,
    COUNT(*)            AS obs_count,
    AVG(confidence)     AS avg_confidence
FROM observations
WHERE ts_utc > now() - interval '90 days'
GROUP BY 1, 2, 3;

CREATE UNIQUE INDEX IF NOT EXISTS idx_mv_obs_vol
    ON mv_observation_volume (day, observation_type, entity_type);

-- Certification expiry tracker (Improvement #39)
CREATE MATERIALIZED VIEW IF NOT EXISTS mv_cert_expiry_tracker AS
SELECT
    cert.id AS cert_id,
    cert.company_id,
    c.name AS company_name,
    c.company_type,
    cert.standard,
    cert.issuing_body,
    cert.valid_until,
    cert.status,
    (cert.valid_until - CURRENT_DATE) AS days_until_expiry
FROM certifications cert
JOIN companies c ON c.id = cert.company_id
WHERE cert.status = 'active'
  AND cert.valid_until IS NOT NULL
  AND cert.valid_until < CURRENT_DATE + interval '180 days';

CREATE UNIQUE INDEX IF NOT EXISTS idx_mv_cert_expiry
    ON mv_cert_expiry_tracker (cert_id);

-- Audit log table (used by refresh function and entity_merge)
CREATE TABLE IF NOT EXISTS audit_log (
    id          BIGSERIAL PRIMARY KEY,
    action      TEXT,
    event_type  TEXT,
    entity_type TEXT,
    entity_id   TEXT,
    detail      JSONB DEFAULT '{}',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_audit_log_created ON audit_log (created_at DESC);

-- Refresh all materialized views (call from worker nightly job)
CREATE OR REPLACE FUNCTION refresh_all_materialized_views()
RETURNS void AS $$
BEGIN
    REFRESH MATERIALIZED VIEW CONCURRENTLY mv_warning_summary;
    REFRESH MATERIALIZED VIEW CONCURRENTLY mv_company_risk_leaderboard;
    REFRESH MATERIALIZED VIEW CONCURRENTLY mv_poi_coverage;
    REFRESH MATERIALIZED VIEW CONCURRENTLY mv_recipe_performance;
    REFRESH MATERIALIZED VIEW CONCURRENTLY mv_observation_volume;
    REFRESH MATERIALIZED VIEW CONCURRENTLY mv_cert_expiry_tracker;
    INSERT INTO audit_log (event_type, detail)
    VALUES ('matview_refresh', jsonb_build_object(
        'views_refreshed', 6,
        'executed_at', now()
    ));
END;
$$ LANGUAGE plpgsql;

COMMIT;
