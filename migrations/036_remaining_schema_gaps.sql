-- ApexIntel Schema Migration – eight remaining tables referenced by Rust code
-- but not yet present in workspace-level migrations/.
--
-- Tables:
--   1. llm_response_cache           – LLM response caching
--   2. insight_generation_log       – Track every insight generation run
--   3. recipe_quality_benchmarks    – Quality targets per recipe
--   4. alert_rules                  – Runtime alert rule configurations
--   5. api_rate_limits              – Track API rate limiting
--   6. analyst_user_roles           – Missing table for RLS and collaboration
--   7. weekly_memo_recipients       – Memo distribution tracking
--   8. insight_bookmark_collections – Bookmark organisation
--
-- All statements are idempotent (IF NOT EXISTS).
-- Uses DOUBLE PRECISION for f64 Rust fields (never DECIMAL/NUMERIC).
-- No explicit BEGIN/COMMIT (sqlx wraps each migration in a transaction).
-- =============================================================================

-- 1. LLM Response Cache -------------------------------------------------------
CREATE TABLE IF NOT EXISTS llm_response_cache (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    prompt_hash     TEXT NOT NULL UNIQUE,
    workflow        TEXT NOT NULL,
    model_name      TEXT NOT NULL,
    response        JSONB NOT NULL DEFAULT '{}'::jsonb,
    tokens_used     INTEGER NOT NULL DEFAULT 0,
    cached_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at      TIMESTAMPTZ NOT NULL DEFAULT (now() + interval '24 hours')
);
CREATE INDEX IF NOT EXISTS idx_llm_cache_hash    ON llm_response_cache (prompt_hash);
-- B353: now() is STABLE and cannot appear in an index predicate; index
-- unexpired-vs-expired via the boolean expression instead.
CREATE INDEX IF NOT EXISTS idx_llm_cache_expires ON llm_response_cache (expires_at) WHERE expires_at IS NOT NULL;

-- 2. Insight Generation Log ---------------------------------------------------
CREATE TABLE IF NOT EXISTS insight_generation_log (
    id                   UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    run_id               TEXT NOT NULL UNIQUE,
    generator_name       TEXT NOT NULL,
    entities_processed   INTEGER NOT NULL DEFAULT 0,
    insights_generated   INTEGER NOT NULL DEFAULT 0,
    insights_published   INTEGER NOT NULL DEFAULT 0,
    failures             INTEGER NOT NULL DEFAULT 0,
    duration_ms          BIGINT NOT NULL DEFAULT 0,
    model_used           TEXT,
    metrics              JSONB DEFAULT '{}'::jsonb,
    created_at           TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_insight_gen_log_run_id    ON insight_generation_log (run_id);
CREATE INDEX IF NOT EXISTS idx_insight_gen_log_created   ON insight_generation_log (created_at DESC);
CREATE INDEX IF NOT EXISTS idx_insight_gen_log_generator ON insight_generation_log (generator_name, created_at DESC);

-- 3. Recipe Quality Benchmarks ------------------------------------------------
CREATE TABLE IF NOT EXISTS recipe_quality_benchmarks (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    recipe_code     TEXT NOT NULL,
    benchmark_name  TEXT NOT NULL,
    target_value    DOUBLE PRECISION NOT NULL,
    current_value   DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    last_measured_at TIMESTAMPTZ,
    trend_direction TEXT DEFAULT 'stable',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (recipe_code, benchmark_name)
);
CREATE INDEX IF NOT EXISTS idx_recipe_benchmarks_code ON recipe_quality_benchmarks (recipe_code);

-- 4. Alert Rules --------------------------------------------------------------
CREATE TABLE IF NOT EXISTS alert_rules (
    id                    UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    rule_name             TEXT NOT NULL UNIQUE,
    category              TEXT NOT NULL,
    condition_json        JSONB NOT NULL DEFAULT '{}'::jsonb,
    severity              TEXT NOT NULL DEFAULT 'medium',
    enabled               BOOLEAN NOT NULL DEFAULT TRUE,
    cooldown_minutes      INTEGER NOT NULL DEFAULT 60,
    last_fired_at         TIMESTAMPTZ,
    notification_channels TEXT[] DEFAULT '{}',
    metadata              JSONB DEFAULT '{}'::jsonb,
    created_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at            TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_alert_rules_enabled ON alert_rules (enabled, category) WHERE enabled = TRUE;

-- 5. API Rate Limits ----------------------------------------------------------
CREATE TABLE IF NOT EXISTS api_rate_limits (
    key_id         TEXT NOT NULL,
    window_start   TIMESTAMPTZ NOT NULL,
    request_count  INTEGER NOT NULL DEFAULT 0,
    max_requests   INTEGER NOT NULL DEFAULT 100,
    reset_at       TIMESTAMPTZ NOT NULL,
    blocked_until  TIMESTAMPTZ,
    PRIMARY KEY (key_id, window_start)
);
CREATE INDEX IF NOT EXISTS idx_api_rate_limits_reset ON api_rate_limits (reset_at);

-- 6. Analyst User Roles -------------------------------------------------------
-- Referenced by api_key_owners FK, RLS policies in 20260514_enable_rls.sql,
-- and INSERT/SELECT in crates/store/src/postgres/collaboration.rs
CREATE TABLE IF NOT EXISTS analyst_user_roles (
    user_id    TEXT PRIMARY KEY,
    role       TEXT NOT NULL DEFAULT 'analyst',
    granted_by TEXT,
    granted_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_analyst_user_roles_role ON analyst_user_roles (role);

-- 7. Weekly Memo Recipients ---------------------------------------------------
CREATE TABLE IF NOT EXISTS weekly_memo_recipients (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    memo_id    UUID NOT NULL,
    user_id    TEXT NOT NULL,
    channel    TEXT NOT NULL DEFAULT 'email',
    status     TEXT NOT NULL DEFAULT 'pending',
    sent_at    TIMESTAMPTZ,
    opened_at  TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (memo_id, user_id, channel)
);
CREATE INDEX IF NOT EXISTS idx_weekly_memo_recipients_memo   ON weekly_memo_recipients (memo_id);
CREATE INDEX IF NOT EXISTS idx_weekly_memo_recipients_status ON weekly_memo_recipients (status);

-- 8. Insight Bookmark Collections ---------------------------------------------
CREATE TABLE IF NOT EXISTS insight_bookmark_collections (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id     TEXT NOT NULL,
    name        TEXT NOT NULL,
    description TEXT,
    is_default  BOOLEAN NOT NULL DEFAULT FALSE,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (user_id, name)
);
CREATE INDEX IF NOT EXISTS idx_bookmark_collections_user ON insight_bookmark_collections (user_id);