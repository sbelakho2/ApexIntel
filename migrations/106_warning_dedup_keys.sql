-- Migration 106: indexed warning dedup keys
-- ════════════════════════════════════════════════════════════════════════════
-- The warning feed deduplicates revisions with a window function whose
-- PARTITION BY recomputes four normalized expressions (strip the leading
-- `[region]` tag, lower/trim type and severity, collapse the description to
-- 380 normalized characters) for EVERY warnings row on every list/graph
-- request. Production measured ~922 ms mean per call at 14k rows, dominating
-- the page's time.
--
-- The five normalized expressions become STORED generated columns, and a
-- composite index over (activity bucket, keys, freshness order) lets the
-- query's DISTINCT ON walk the index in order instead of sorting a full
-- window. The generated expressions are immutable, so they stay in sync
-- automatically on every INSERT/UPDATE.

ALTER TABLE warnings
    ADD COLUMN IF NOT EXISTS dedup_title_key text
        GENERATED ALWAYS AS (lower(trim(regexp_replace(title, '^\[[^]]+\]\s*', '')))) STORED,
    ADD COLUMN IF NOT EXISTS dedup_type_key text
        GENERATED ALWAYS AS (lower(trim(warning_type))) STORED,
    ADD COLUMN IF NOT EXISTS dedup_severity_key text
        GENERATED ALWAYS AS (lower(trim(severity))) STORED,
    ADD COLUMN IF NOT EXISTS dedup_region_key text
        GENERATED ALWAYS AS (coalesce(lower(region), '')) STORED,
    ADD COLUMN IF NOT EXISTS dedup_description_key text
        GENERATED ALWAYS AS (
            left(trim(regexp_replace(
                regexp_replace(lower(coalesce(description, '')), '[^a-z0-9]+', ' ', 'g'),
                '\s+', ' ', 'g'
            )), 380)
        ) STORED;

CREATE INDEX IF NOT EXISTS idx_warnings_dedup_keys ON warnings (
    (CASE WHEN deleted_at IS NULL THEN 'active' ELSE 'deleted' END),
    dedup_title_key,
    dedup_type_key,
    dedup_severity_key,
    dedup_region_key,
    dedup_description_key,
    updated_at DESC NULLS LAST,
    created_at DESC NULLS LAST,
    ts_utc DESC,
    id DESC
);
