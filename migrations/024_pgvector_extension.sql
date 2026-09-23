-- Enable pgvector extension for embedding/vector search support.
-- Requires: pgvector extension installed on the PostgreSQL server.
--   See: https://github.com/pgvector/pgvector
--
-- The embeddings table stores dense vector representations (4096-dim, from
-- Qwen3-30B-A3B via llama.cpp) for entity text chunks, enabling cosine-similarity
-- semantic search across companies, persons, insights, warnings, and observations.

CREATE EXTENSION IF NOT EXISTS vector;

-- ─── Embeddings Table ────────────────────────────────────────────────────────
--
-- Each row stores one vector embedding for a single text chunk of an entity.
-- Entity types are stored as a text discriminator (e.g. 'company', 'person').
-- The unique constraint prevents duplicate embeddings for the same entity chunk + model.

CREATE TABLE embeddings (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    entity_type TEXT        NOT NULL,
    entity_id   TEXT        NOT NULL,
    chunk_index INTEGER     NOT NULL,
    embedding   vector(4096) NOT NULL,
    source_text TEXT        NOT NULL,
    model_name  TEXT        NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- Prevent duplicate embeddings for the same entity chunk + model
    UNIQUE (entity_type, entity_id, chunk_index, model_name)
);

-- ─── Indexes ─────────────────────────────────────────────────────────────────

-- IVFFlat approximate nearest neighbour index for cosine distance.
-- Lists = 100 is a reasonable default for up to ~1M rows; adjust based on data size.
-- B353: ivfflat supports at most 2000 dimensions — the original 4096-dim
-- column made this statement (and therefore every fresh bootstrap) fail with
-- "column cannot have more than 2000 dimensions". The column is later
-- narrowed to vector(384) by 20260623_embedding_dimension_384.sql; create
-- the ANN index only when the active dimension fits ivfflat's limit.
DO $$
DECLARE
    dims INTEGER := 0;
BEGIN
    SELECT COALESCE(MAX(a.atttypmod - 4), 0) INTO dims
      FROM pg_attribute a
     WHERE a.attname = 'embedding'
       AND a.attrelid = 'embeddings'::regclass;
    IF dims > 0 AND dims <= 2000 THEN
        EXECUTE 'CREATE INDEX idx_embeddings_vector ON embeddings '
                'USING ivfflat (embedding vector_cosine_ops) WITH (lists = 100)';
    END IF;
END $$;

-- B-tree indexes for entity lookups and incremental indexing queries.
CREATE INDEX idx_embeddings_entity ON embeddings (entity_type, entity_id);
CREATE INDEX idx_embeddings_model   ON embeddings (model_name);
CREATE INDEX idx_embeddings_updated ON embeddings (updated_at DESC);

-- ─── Trigger: auto-update updated_at ─────────────────────────────────────────

CREATE OR REPLACE FUNCTION update_embeddings_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_embeddings_updated_at
    BEFORE UPDATE ON embeddings
    FOR EACH ROW
    EXECUTE FUNCTION update_embeddings_updated_at();
