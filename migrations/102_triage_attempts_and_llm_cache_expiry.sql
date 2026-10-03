-- Migration 102: triage scoring attempts, override attribution and LLM cache expiry
-- ════════════════════════════════════════════════════════════════════════════
-- Store-side support for four audit items that all need additive columns:
--
--  1. `triage_queue.triage_attempts` — a bounded scoring-attempt counter. The
--     LLM triage loop claims unscored rows and increments this counter; rows
--     whose counter reached the cap are no longer claimable, so one
--     permanently failing item cannot starve the queue.
--  2. `triage_queue.overridden_by` — who performed a manual score override.
--     The previous schema recorded only `is_overridden` / `override_score`,
--     losing the actor for audit.
--  3. `llm_cache.prompt_hash` and `llm_cache.expires_at` — the rendered prompt
--     hash is persisted with each entry (so a cached response is bound to the
--     exact rendered prompt, not only to evidence ids) and entries expire, so
--     a stale response cannot be served forever.
--  4. `recipes.created_by` — attribution for recipes created through the
--     canonical-column writer (the UI recipe-creation path), which previously
--     could not record who authored a recipe.
--
-- Additive and idempotent: every statement is guarded, so re-running is a
-- no-op and no existing row is rewritten.
-- ════════════════════════════════════════════════════════════════════════════

-- ── 1. Triage scoring attempts ───────────────────────────────────────────────

ALTER TABLE triage_queue
    ADD COLUMN IF NOT EXISTS triage_attempts INTEGER NOT NULL DEFAULT 0;

COMMENT ON COLUMN triage_queue.triage_attempts IS
    'Scoring attempts consumed; rows at the configured cap are no longer claimable (never negative)';

-- ── 2. Override attribution ──────────────────────────────────────────────────

ALTER TABLE triage_queue
    ADD COLUMN IF NOT EXISTS overridden_by TEXT;

COMMENT ON COLUMN triage_queue.overridden_by IS
    'Principal (app_users id) that performed the manual score override; NULL when never overridden';

-- ── 3. LLM cache prompt hash and expiry ──────────────────────────────────────

ALTER TABLE llm_cache
    ADD COLUMN IF NOT EXISTS prompt_hash TEXT;

ALTER TABLE llm_cache
    ADD COLUMN IF NOT EXISTS expires_at TIMESTAMPTZ NOT NULL
        DEFAULT (NOW() + INTERVAL '7 days');

COMMENT ON COLUMN llm_cache.prompt_hash IS
    'SHA-256 of the exact rendered prompt; NULL only for legacy rows cached before prompt hashing';

COMMENT ON COLUMN llm_cache.expires_at IS
    'After this instant the entry is a cache miss; never NULL (new rows default to a 7-day TTL)';

CREATE INDEX IF NOT EXISTS idx_llm_cache_expires_at ON llm_cache (expires_at);

-- ── 4. Recipe author attribution ─────────────────────────────────────────────

ALTER TABLE recipes
    ADD COLUMN IF NOT EXISTS created_by TEXT;

COMMENT ON COLUMN recipes.created_by IS
    'Principal (app_users id) that created the recipe through the canonical writer; NULL for seeded rows';

-- ── 5. Dedup-candidate retention ─────────────────────────────────────────────

-- `semantic_dedup_items` accumulates one candidate row per triaged item and
-- had no retention: the dedup window is recent-only, so rows older than the
-- retention horizon are never compared again. This standalone cleanup keeps
-- the table bounded; the frozen master retention function cannot be edited.
CREATE OR REPLACE FUNCTION cleanup_semantic_dedup_items(retention_days INT DEFAULT 30)
RETURNS BIGINT AS $$
DECLARE
    deleted BIGINT;
BEGIN
    IF to_regclass('public.semantic_dedup_items') IS NULL THEN
        RETURN 0;
    END IF;
    DELETE FROM semantic_dedup_items
    WHERE updated_at < now() - make_interval(days => retention_days);
    GET DIAGNOSTICS deleted = ROW_COUNT;
    RETURN deleted;
END;
$$ LANGUAGE plpgsql;

COMMENT ON FUNCTION cleanup_semantic_dedup_items(INT) IS
    'Delete dedup candidates whose last update is older than the retention horizon; safe to call from maintenance jobs';

-- Runtime role grants (production connects as a non-owner role; pattern from 050).
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'apexintel') THEN
        GRANT SELECT, INSERT, UPDATE, DELETE ON triage_queue TO apexintel;
        GRANT SELECT, INSERT, UPDATE, DELETE ON llm_cache TO apexintel;
    END IF;
END $$;
