-- ════════════════════════════════════════════════════════════════════════════
-- Migration 087: crawl_metrics stops fabricating fetch/promotion numbers
-- ════════════════════════════════════════════════════════════════════════════
--
-- `crawl_metrics` stored three fabricated values under authoritative names:
--
--   * `fetch_attempts` held the observation count (not fetch attempts),
--   * `fetch_errors` was hard-coded 0 (errors were never instrumented),
--   * `observations_in_promotions` held the count of observations above a
--     confidence threshold (a proxy, not promotion events).
--
-- Any admin/readiness reader could infer "0 fetch errors" where the truth was
-- "unmeasured". The columns become nullable: NULL means not measured, while a
-- real 0 remains a measured zero. Fresh rows written by the new code leave
-- fetch_errors/fetch_attempts/promotions NULL until the crawler runtime state
-- is joined in.
--
-- Idempotent: safe to re-apply.

ALTER TABLE crawl_metrics
    ALTER COLUMN fetch_attempts DROP NOT NULL,
    ALTER COLUMN fetch_errors DROP NOT NULL,
    ALTER COLUMN observations_in_promotions DROP NOT NULL;

-- The old code wrote observation counts into fetch_attempts and a hard zero
-- into fetch_errors for every row; those values are not fetch telemetry and
-- must not be read as such. Demote them to NULL (unknown) rather than keeping
-- a misleading measured-looking number.
UPDATE crawl_metrics
SET fetch_attempts = NULL
WHERE fetch_attempts IS NOT NULL;
UPDATE crawl_metrics
SET fetch_errors = NULL
WHERE fetch_errors IS NOT NULL;
UPDATE crawl_metrics
SET observations_in_promotions = NULL
WHERE observations_in_promotions IS NOT NULL;

COMMENT ON COLUMN crawl_metrics.fetch_attempts IS
    'Measured per-source fetch attempts; NULL = not instrumented (a NULL is never "0 attempts")';
COMMENT ON COLUMN crawl_metrics.fetch_errors IS
    'Measured per-source fetch errors; NULL = not instrumented (a NULL is never "0 errors")';
COMMENT ON COLUMN crawl_metrics.observations_in_promotions IS
    'Real promotion events per source; NULL = not recorded (confidence proxies do not count)';

-- Runtime role grants (production connects as a non-owner role).
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'apexintel_app') THEN
        GRANT SELECT, INSERT, UPDATE, DELETE ON crawl_metrics TO apexintel_app;
    END IF;
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'apexintel') THEN
        GRANT SELECT, INSERT, UPDATE, DELETE ON crawl_metrics TO apexintel;
    END IF;
END $$;
