-- ════════════════════════════════════════════════════════════════════════════
-- Data Retention Policies
-- Automated cleanup functions for unbounded tables.
-- Run: psql $DATABASE_URL < migrations/20260301_data_retention.sql
-- ════════════════════════════════════════════════════════════════════════════

BEGIN;

-- Purge observations older than N days (default: 365)
CREATE OR REPLACE FUNCTION cleanup_old_observations(retention_days INT DEFAULT 365)
RETURNS BIGINT AS $$
DECLARE
    deleted BIGINT;
BEGIN
    DELETE FROM observations
    WHERE ts_utc < now() - (retention_days || ' days')::interval;
    GET DIAGNOSTICS deleted = ROW_COUNT;
    INSERT INTO audit_log (event_type, detail)
    VALUES ('data_retention', jsonb_build_object(
        'table', 'observations',
        'retention_days', retention_days,
        'rows_deleted', deleted,
        'executed_at', now()
    ));
    RETURN deleted;
END;
$$ LANGUAGE plpgsql;

-- Purge crawl telemetry older than 90 days
CREATE OR REPLACE FUNCTION cleanup_crawl_telemetry(retention_days INT DEFAULT 90)
RETURNS BIGINT AS $$
DECLARE
    deleted BIGINT;
BEGIN
    DELETE FROM crawl_telemetry
    WHERE created_at < now() - (retention_days || ' days')::interval;
    GET DIAGNOSTICS deleted = ROW_COUNT;
    INSERT INTO audit_log (event_type, detail)
    VALUES ('data_retention', jsonb_build_object(
        'table', 'crawl_telemetry',
        'retention_days', retention_days,
        'rows_deleted', deleted,
        'executed_at', now()
    ));
    RETURN deleted;
END;
$$ LANGUAGE plpgsql;

-- Purge old social signals (180 days)
CREATE OR REPLACE FUNCTION cleanup_social_signals(retention_days INT DEFAULT 180)
RETURNS BIGINT AS $$
DECLARE
    deleted BIGINT;
BEGIN
    DELETE FROM social_signals
    WHERE ts_utc < now() - (retention_days || ' days')::interval;
    GET DIAGNOSTICS deleted = ROW_COUNT;
    INSERT INTO audit_log (event_type, detail)
    VALUES ('data_retention', jsonb_build_object(
        'table', 'social_signals',
        'retention_days', retention_days,
        'rows_deleted', deleted,
        'executed_at', now()
    ));
    RETURN deleted;
END;
$$ LANGUAGE plpgsql;

-- Purge old page fingerprints (keep latest 3 per URL)
CREATE OR REPLACE FUNCTION cleanup_page_fingerprints()
RETURNS BIGINT AS $$
DECLARE
    deleted BIGINT;
BEGIN
    DELETE FROM page_fingerprints pf
    WHERE pf.id NOT IN (
        SELECT id FROM (
            SELECT id, ROW_NUMBER() OVER (PARTITION BY url ORDER BY ts DESC) AS rn
            FROM page_fingerprints
        ) ranked
        WHERE ranked.rn <= 3
    );
    GET DIAGNOSTICS deleted = ROW_COUNT;
    INSERT INTO audit_log (event_type, detail)
    VALUES ('data_retention', jsonb_build_object(
        'table', 'page_fingerprints',
        'keep_per_url', 3,
        'rows_deleted', deleted,
        'executed_at', now()
    ));
    RETURN deleted;
END;
$$ LANGUAGE plpgsql;

-- Purge old feature rows (keep 12 months)
CREATE OR REPLACE FUNCTION cleanup_feature_rows(retention_days INT DEFAULT 365)
RETURNS BIGINT AS $$
DECLARE
    deleted BIGINT;
    cutoff_bucket BIGINT;
BEGIN
    cutoff_bucket := EXTRACT(EPOCH FROM now() - (retention_days || ' days')::interval)::BIGINT;
    DELETE FROM feature_rows WHERE time_bucket < cutoff_bucket;
    GET DIAGNOSTICS deleted = ROW_COUNT;
    INSERT INTO audit_log (event_type, detail)
    VALUES ('data_retention', jsonb_build_object(
        'table', 'feature_rows',
        'retention_days', retention_days,
        'rows_deleted', deleted,
        'executed_at', now()
    ));
    RETURN deleted;
END;
$$ LANGUAGE plpgsql;

-- Purge old audit_log entries (keep 2 years)
CREATE OR REPLACE FUNCTION cleanup_audit_log(retention_days INT DEFAULT 730)
RETURNS BIGINT AS $$
DECLARE
    deleted BIGINT;
BEGIN
    DELETE FROM audit_log WHERE ts < now() - (retention_days || ' days')::interval;
    GET DIAGNOSTICS deleted = ROW_COUNT;
    RETURN deleted;
END;
$$ LANGUAGE plpgsql;

-- Mark graph edges not refreshed in 180 days as stale (Improvement #34)
CREATE OR REPLACE FUNCTION mark_stale_graph_edges(stale_days INT DEFAULT 180)
RETURNS BIGINT AS $$
DECLARE
    updated BIGINT;
BEGIN
    UPDATE graph_edges
    SET stale = TRUE
    WHERE last_seen < now() - (stale_days || ' days')::interval
      AND NOT stale;
    GET DIAGNOSTICS updated = ROW_COUNT;
    INSERT INTO audit_log (event_type, detail)
    VALUES ('stale_edges', jsonb_build_object(
        'stale_days', stale_days,
        'edges_marked', updated,
        'executed_at', now()
    ));
    RETURN updated;
END;
$$ LANGUAGE plpgsql;

-- Flag stale entities (no observations in 90+ days) (Improvement #46)
CREATE OR REPLACE FUNCTION flag_stale_entities(stale_days INT DEFAULT 90)
RETURNS TABLE(entity_type TEXT, entity_id UUID, last_observation TIMESTAMPTZ) AS $$
BEGIN
    -- Stale companies
    RETURN QUERY
    SELECT 'company'::TEXT, c.id, MAX(o.ts_utc)
    FROM companies c
    LEFT JOIN observations o ON o.entity_id = c.id
    GROUP BY c.id
    HAVING MAX(o.ts_utc) < now() - (stale_days || ' days')::interval
       OR MAX(o.ts_utc) IS NULL;

    -- Stale persons
    RETURN QUERY
    SELECT 'person'::TEXT, p.id, MAX(pa.ts_utc)
    FROM persons p
    LEFT JOIN poi_artifacts pa ON pa.person_id = p.id
    GROUP BY p.id
    HAVING MAX(pa.ts_utc) < now() - (stale_days || ' days')::interval
       OR MAX(pa.ts_utc) IS NULL;
END;
$$ LANGUAGE plpgsql;

-- Master retention runner (call from worker weekly job)
CREATE OR REPLACE FUNCTION run_data_retention()
RETURNS TABLE(table_name TEXT, rows_deleted BIGINT) AS $$
BEGIN
    table_name := 'observations';      rows_deleted := cleanup_old_observations(365);   RETURN NEXT;
    table_name := 'crawl_telemetry';   rows_deleted := cleanup_crawl_telemetry(90);     RETURN NEXT;
    table_name := 'social_signals';    rows_deleted := cleanup_social_signals(180);     RETURN NEXT;
    table_name := 'page_fingerprints'; rows_deleted := cleanup_page_fingerprints();     RETURN NEXT;
    table_name := 'feature_rows';      rows_deleted := cleanup_feature_rows(365);       RETURN NEXT;
    table_name := 'audit_log';         rows_deleted := cleanup_audit_log(730);          RETURN NEXT;
    table_name := 'stale_edges';       rows_deleted := mark_stale_graph_edges(180);     RETURN NEXT;
END;
$$ LANGUAGE plpgsql;

COMMIT;
