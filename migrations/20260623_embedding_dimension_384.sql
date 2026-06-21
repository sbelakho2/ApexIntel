-- Embedding dimension change: vector(4096) -> vector(384)
--
-- The dedicated embedding server uses bge-small-en-v1.5 which produces
-- 384-dimensional vectors. The embeddings table was originally created with
-- vector(4096) (matching the Qwen3 hidden size), but the production LLM server
-- cannot serve embeddings without the --embeddings flag (which would disable
-- chat completions). A separate small embedding model is now used instead.
--
-- This migration is safe to run multiple times (idempotent via exception guard).
-- The table is expected to be empty or contain only compatible-dimension rows.

-- Remove any existing rows with incompatible dimensions first,
-- since pgvector cannot implicitly cast between different vector dimensions.
DO $$
BEGIN
    DELETE FROM embeddings WHERE vector_dims(embedding) != 384;
EXCEPTION WHEN undefined_function THEN
    -- vector_dims not available (older pgvector), delete all rows
    DELETE FROM embeddings;
END $$;

ALTER TABLE embeddings ALTER COLUMN embedding TYPE vector(384) USING embedding::vector(384);
