-- ApexIntel: Embedding / Vector Search Foundation
-- Migration: 0019_embeddings
-- Description: Enable pgvector extension, create embeddings table and IVFFlat index.

BEGIN;

-- ─────────────────────────────────────────────────────────────────────────────
-- 1. Enable pgvector extension
-- ─────────────────────────────────────────────────────────────────────────────
CREATE EXTENSION IF NOT EXISTS vector;

-- ─────────────────────────────────────────────────────────────────────────────
-- 2. Embeddings table
-- ─────────────────────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS embeddings (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    -- The entity this embedding belongs to (e.g. company, person, insight, warning, observation)
    entity_type     TEXT        NOT NULL,
    entity_id       TEXT        NOT NULL,
    -- Which text chunk (0-based) within the entity — enables multi-chunk for long content
    chunk_index     INT         NOT NULL DEFAULT 0,
    -- The 4096-dimension vector produced by the embedding model (Qwen3-30B-A3B)
    embedding       vector(4096) NOT NULL,
    -- The source text that was embedded (for debugging / diagnostic)
    source_text     TEXT        NOT NULL DEFAULT '',
    -- Model name used to generate this embedding (e.g. "Qwen3-30B-A3B-Q4_K_M")
    model_name      TEXT        NOT NULL DEFAULT '',
    -- Timestamps
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Dedup constraint: one embedding per (entity_type, entity_id, chunk_index, model_name)
    CONSTRAINT uq_embedding UNIQUE (entity_type, entity_id, chunk_index, model_name)
);

-- ─────────────────────────────────────────────────────────────────────────────
-- 3. IVFFlat index for approximate nearest-neighbour search
--    lists = 100 is reasonable for ~100K vectors; tune after backfill.
-- ─────────────────────────────────────────────────────────────────────────────
CREATE INDEX IF NOT EXISTS idx_embeddings_vector
    ON embeddings
    USING ivf (embedding vector_cosine_ops)
    WITH (lists = 100);

-- ─────────────────────────────────────────────────────────────────────────────
-- 4. B-tree indexes for lookups by entity / model
-- ─────────────────────────────────────────────────────────────────────────────
CREATE INDEX IF NOT EXISTS idx_embeddings_entity
    ON embeddings (entity_type, entity_id);

CREATE INDEX IF NOT EXISTS idx_embeddings_model
    ON embeddings (model_name);

CREATE INDEX IF NOT EXISTS idx_embeddings_created_at
    ON embeddings (created_at);

-- ─────────────────────────────────────────────────────────────────────────────
-- 5. Trigger to auto-update updated_at
-- ─────────────────────────────────────────────────────────────────────────────
CREATE OR REPLACE FUNCTION update_embeddings_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = now();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_embeddings_updated_at ON embeddings;
CREATE TRIGGER trg_embeddings_updated_at
    BEFORE UPDATE ON embeddings
    FOR EACH ROW
    EXECUTE FUNCTION update_embeddings_updated_at();

COMMIT;
