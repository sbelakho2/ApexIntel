-- 084_semantic_dedup_items.sql
--
-- Persistent semantic dedup (audit P0 #9).
--
-- The triage ingress used `SemanticDedup::with_in_memory_fallback()` in
-- production: every worker restart forgot every previously-seen item, so
-- semantic dedup only worked within one process lifetime and a restart
-- re-enqueued duplicates that had already been triaged.
--
-- This migration adds the persistent side of the dedup store:
--
--   semantic_dedup_items — one row per triaged item (or warning) with its
--                          text and, when an embedding client is configured,
--                          its 384-dimensional embedding (the dimension of
--                          the production bge-small-en-v1.5 embedding server,
--                          see 033_embedding_dimension_384.sql). Vector and
--                          pg_trgm nearest-neighbour lookups run against this
--                          table so dedup survives restarts and job reruns.
--
--   semantic_dedup_state — a single row recording which backend the
--                          production worker actually constructed
--                          (`memory` | `pgvector`) and whether semantic
--                          lookup is fully available (`ok`) or degraded
--                          (no embedding client configured). `/api/features`
--                          reports this row so capability checks measure the
--                          real backend instead of assuming it.
--
-- Idempotent: safe to re-apply (IF NOT EXISTS + ON CONFLICT DO NOTHING).

CREATE TABLE IF NOT EXISTS semantic_dedup_items (
    id           BIGSERIAL PRIMARY KEY,
    item_type    TEXT NOT NULL,
    item_id      TEXT NOT NULL,
    title        TEXT NOT NULL,
    text_content TEXT NOT NULL,
    embedding    vector(384),
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT semantic_dedup_items_type_id_key UNIQUE (item_type, item_id)
);

-- Recency scans for a single item type (the recent-candidates fallback).
CREATE INDEX IF NOT EXISTS idx_semantic_dedup_items_type_updated
    ON semantic_dedup_items (item_type, updated_at DESC);

-- Trigram index backing the text-similarity fallback when no embedding
-- client is configured.
CREATE INDEX IF NOT EXISTS idx_semantic_dedup_items_text_trgm
    ON semantic_dedup_items USING gin (text_content gin_trgm_ops);

-- Single-row state table. `id` is a boolean primary key pinned to TRUE so the
-- table can never accumulate more than one row.
CREATE TABLE IF NOT EXISTS semantic_dedup_state (
    id         BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (id),
    backend    TEXT NOT NULL CHECK (backend IN ('memory', 'pgvector')),
    status     TEXT NOT NULL CHECK (status IN ('ok', 'degraded')),
    detail     TEXT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Explicit degraded default: until the worker records the backend it
-- constructed, capabilities report a degraded in-memory fallback rather than
-- assuming pgvector is active.
INSERT INTO semantic_dedup_state (id, backend, status, detail)
VALUES (TRUE, 'memory', 'degraded', 'worker has not recorded a dedup backend yet')
ON CONFLICT (id) DO NOTHING;

-- Runtime role grants (production connects as a non-owner role; see 050/060).
-- Both the new split role (`apexintel_app`) and the legacy role are granted.
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'apexintel_app') THEN
        GRANT SELECT, INSERT, UPDATE, DELETE ON semantic_dedup_items TO apexintel_app;
        GRANT USAGE, SELECT ON SEQUENCE semantic_dedup_items_id_seq TO apexintel_app;
        GRANT SELECT, INSERT, UPDATE, DELETE ON semantic_dedup_state TO apexintel_app;
    END IF;
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'apexintel') THEN
        GRANT SELECT, INSERT, UPDATE, DELETE ON semantic_dedup_items TO apexintel;
        GRANT USAGE, SELECT ON SEQUENCE semantic_dedup_items_id_seq TO apexintel;
        GRANT SELECT, INSERT, UPDATE, DELETE ON semantic_dedup_state TO apexintel;
    END IF;
END $$;
