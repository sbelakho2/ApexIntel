-- 052_llm_cache.sql
--
-- LLM work cache (audit P0 #24).
--
-- Deterministic work (RSS/HTML parsing, entity aliases) never reaches an LLM.
-- For the work that does, an identical request — same model version, same
-- prompt version, same evidence ids, same workflow — must never re-run the
-- model. `cache_key` is the SHA-256 of exactly those inputs, so unchanged
-- evidence is served from here and only changed evidence triggers inference.
--
-- Idempotent: safe to re-apply.

CREATE TABLE IF NOT EXISTS llm_cache (
    cache_key      TEXT PRIMARY KEY,
    workflow       TEXT NOT NULL,
    model_version  TEXT NOT NULL,
    prompt_version TEXT NOT NULL,
    evidence_ids   UUID[] NOT NULL DEFAULT '{}'::uuid[],
    response       TEXT NOT NULL,
    hits           BIGINT NOT NULL DEFAULT 0,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_hit_at    TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_llm_cache_workflow_created
    ON llm_cache (workflow, created_at DESC);

-- Runtime role grants (production connects as a non-owner role; see 050).
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'apexintel') THEN
        GRANT SELECT, INSERT, UPDATE, DELETE ON llm_cache TO apexintel;
    END IF;
END $$;
