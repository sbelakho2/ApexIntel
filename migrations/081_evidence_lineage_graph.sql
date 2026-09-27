-- ════════════════════════════════════════════════════════════════════════════
-- Migration 079: evidence lineage graph (audit evidence-graph item)
-- ════════════════════════════════════════════════════════════════════════════
-- Every artifact in the intelligence pipeline is a node in one chain:
--
--   source_document -> extraction -> observation -> entity_link -> feature ->
--   claim -> insight -> warning -> triage -> notification -> analyst_action ->
--   outcome
--
-- Each node records the stage that produced it, the producer and version,
-- when it was created, the digest of the input it consumed, and the
-- confidence assigned at that stage. Edges connect a node to the node
-- produced from it and carry the transformation name (the target stage).
--
-- The tables are append-oriented and idempotent: (stage, reference) and
-- (transformation, source_node, target_node) are unique, so re-running a
-- pipeline stage re-affirms its rows instead of duplicating the graph.
--
-- Idempotent: safe to re-apply (IF NOT EXISTS / guarded reconciliations).
-- ════════════════════════════════════════════════════════════════════════════

CREATE TABLE IF NOT EXISTS evidence_lineage_nodes (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    stage            TEXT NOT NULL CHECK (stage IN (
                         'source_document', 'extraction', 'observation',
                         'entity_link', 'feature', 'claim', 'insight',
                         'warning', 'triage', 'notification',
                         'analyst_action', 'outcome')),
    -- Stable external reference: artifact UUID for database rows, URL for
    -- source documents, recipe/slug for other producers.
    reference        TEXT NOT NULL,
    producer         TEXT NOT NULL,
    producer_version TEXT NOT NULL,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    input_digest     TEXT,
    confidence       DOUBLE PRECISION
                     CHECK (confidence IS NULL OR (confidence >= 0 AND confidence <= 1)),
    metadata         JSONB NOT NULL DEFAULT '{}',
    UNIQUE (stage, reference)
);

CREATE INDEX IF NOT EXISTS idx_evidence_lineage_nodes_reference
    ON evidence_lineage_nodes (reference);

CREATE TABLE IF NOT EXISTS evidence_lineage_edges (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    transformation   TEXT NOT NULL CHECK (transformation IN (
                         'source_document', 'extraction', 'observation',
                         'entity_link', 'feature', 'claim', 'insight',
                         'warning', 'triage', 'notification',
                         'analyst_action', 'outcome')),
    source_node      UUID NOT NULL REFERENCES evidence_lineage_nodes(id) ON DELETE CASCADE,
    target_node      UUID NOT NULL REFERENCES evidence_lineage_nodes(id) ON DELETE CASCADE,
    producer         TEXT NOT NULL,
    producer_version TEXT NOT NULL,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    input_digest     TEXT,
    confidence       DOUBLE PRECISION
                     CHECK (confidence IS NULL OR (confidence >= 0 AND confidence <= 1)),
    metadata         JSONB NOT NULL DEFAULT '{}',
    CHECK (source_node <> target_node),
    UNIQUE (transformation, source_node, target_node)
);

-- Upstream traversal (insight -> claim -> observation -> document).
CREATE INDEX IF NOT EXISTS idx_evidence_lineage_edges_target
    ON evidence_lineage_edges (target_node, transformation);

-- Downstream traversal (document -> ... -> outcome).
CREATE INDEX IF NOT EXISTS idx_evidence_lineage_edges_source
    ON evidence_lineage_edges (source_node, transformation);

-- Runtime role grants (production connects as a non-owner role; see 050/060).
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'apexintel_app') THEN
        GRANT SELECT, INSERT, UPDATE, DELETE ON evidence_lineage_nodes TO apexintel_app;
        GRANT SELECT, INSERT, UPDATE, DELETE ON evidence_lineage_edges TO apexintel_app;
    END IF;
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'apexintel') THEN
        GRANT SELECT, INSERT, UPDATE, DELETE ON evidence_lineage_nodes TO apexintel;
        GRANT SELECT, INSERT, UPDATE, DELETE ON evidence_lineage_edges TO apexintel;
    END IF;
END $$;
