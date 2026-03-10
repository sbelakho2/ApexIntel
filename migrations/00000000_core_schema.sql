-- ════════════════════════════════════════════════════════════════════════════
-- ApexIntel Core Schema
-- All tables required by PgStore, recipe engine, and analytics pipeline.
-- Idempotent: uses IF NOT EXISTS throughout.
-- Run: psql $DATABASE_URL < migrations/00000000_core_schema.sql
-- ════════════════════════════════════════════════════════════════════════════

BEGIN;

-- ─── Extensions ──────────────────────────────────────────────────────────────
CREATE EXTENSION IF NOT EXISTS "uuid-ossp";
CREATE EXTENSION IF NOT EXISTS "pg_trgm";
CREATE EXTENSION IF NOT EXISTS "btree_gist";

-- ─── companies ───────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS companies (
    id                    UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name                  TEXT NOT NULL,
    legal_name            TEXT,
    domain                TEXT UNIQUE,
    country_code          TEXT,
    region                TEXT,
    company_type          TEXT,
    industry_tags         TEXT[] DEFAULT '{}',
    employee_estimate     INT,
    revenue_estimate_usd  BIGINT,
    risk_score            FLOAT DEFAULT 0,
    threat_score          FLOAT DEFAULT 0,
    overlap_score         FLOAT DEFAULT 0,
    strategic_relevance   FLOAT DEFAULT 0,
    is_competitor         BOOLEAN DEFAULT FALSE,
    metadata              JSONB DEFAULT '{}',
    created_at            TIMESTAMPTZ DEFAULT now(),
    updated_at            TIMESTAMPTZ DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_companies_domain     ON companies(domain);
CREATE INDEX IF NOT EXISTS idx_companies_region     ON companies(region);
CREATE INDEX IF NOT EXISTS idx_companies_competitor ON companies(is_competitor) WHERE is_competitor;
CREATE INDEX IF NOT EXISTS idx_companies_name_trgm  ON companies USING gin(name gin_trgm_ops);

-- ─── sites ───────────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS sites (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    company_id        UUID REFERENCES companies(id) ON DELETE CASCADE,
    name              TEXT NOT NULL,
    address           TEXT,
    city              TEXT,
    country_code      TEXT,
    region            TEXT,
    lat               DOUBLE PRECISION,
    lon               DOUBLE PRECISION,
    site_type         TEXT,
    capabilities      TEXT[] DEFAULT '{}',
    certifications    TEXT[] DEFAULT '{}',
    employee_estimate INT,
    free_zone         TEXT,
    metadata          JSONB DEFAULT '{}',
    created_at        TIMESTAMPTZ DEFAULT now(),
    updated_at        TIMESTAMPTZ DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_sites_company ON sites(company_id);
CREATE INDEX IF NOT EXISTS idx_sites_region  ON sites(region);

-- ─── product_families ────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS product_families (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    company_id  UUID REFERENCES companies(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    hs_codes    TEXT[] DEFAULT '{}',
    tech_tags   TEXT[] DEFAULT '{}',
    metadata    JSONB DEFAULT '{}',
    created_at  TIMESTAMPTZ DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_pf_company ON product_families(company_id);

-- ─── capabilities ────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS capabilities (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    company_id      UUID REFERENCES companies(id) ON DELETE CASCADE,
    site_id         UUID REFERENCES sites(id) ON DELETE SET NULL,
    capability      TEXT NOT NULL,
    proof_grade     TEXT DEFAULT 'D',
    evidence_urls   TEXT[] DEFAULT '{}',
    first_seen      TIMESTAMPTZ DEFAULT now(),
    last_confirmed  TIMESTAMPTZ DEFAULT now(),
    metadata        JSONB DEFAULT '{}'
);
CREATE INDEX IF NOT EXISTS idx_cap_company    ON capabilities(company_id);
CREATE INDEX IF NOT EXISTS idx_cap_capability ON capabilities(capability);

-- ─── certifications ──────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS certifications (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    company_id      UUID REFERENCES companies(id) ON DELETE CASCADE,
    site_id         UUID REFERENCES sites(id) ON DELETE SET NULL,
    standard        TEXT NOT NULL,
    status          TEXT DEFAULT 'active',
    issuing_body    TEXT,
    valid_from      DATE,
    valid_until     DATE,
    scope           TEXT,
    evidence_url    TEXT,
    metadata        JSONB DEFAULT '{}',
    created_at      TIMESTAMPTZ DEFAULT now(),
    updated_at      TIMESTAMPTZ DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_cert_company  ON certifications(company_id);
CREATE INDEX IF NOT EXISTS idx_cert_standard ON certifications(standard);
CREATE INDEX IF NOT EXISTS idx_cert_expiry   ON certifications(valid_until) WHERE status = 'active';

-- ─── logistics_nodes ─────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS logistics_nodes (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name         TEXT NOT NULL,
    node_type    TEXT,
    country_code TEXT,
    lat          DOUBLE PRECISION,
    lon          DOUBLE PRECISION,
    metadata     JSONB DEFAULT '{}',
    created_at   TIMESTAMPTZ DEFAULT now()
);

-- ─── regulations ─────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS regulations (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name            TEXT NOT NULL,
    regulation_type TEXT,
    jurisdiction    TEXT,
    effective_date  DATE,
    summary         TEXT,
    source_url      TEXT,
    metadata        JSONB DEFAULT '{}',
    created_at      TIMESTAMPTZ DEFAULT now()
);

-- ─── persons (POI) ──────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS persons (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name                TEXT NOT NULL,
    name_ar             TEXT,
    name_fr             TEXT,
    name_he             TEXT,
    name_zh             TEXT,
    primary_org_id      UUID REFERENCES companies(id) ON DELETE SET NULL,
    current_role        TEXT,
    role_family         TEXT,
    region              TEXT,
    country_code        TEXT,
    public_bio          TEXT,
    public_email        TEXT,
    photo_hash          TEXT,
    priority_vector     JSONB DEFAULT '{"cost":0,"quality":0,"speed":0,"resilience":0,"compliance":0,"security":0}',
    decision_mode       TEXT,
    influence_score     FLOAT DEFAULT 0,
    role_drift_score    FLOAT DEFAULT 0,
    change_risk         FLOAT DEFAULT 0,
    pain_index          FLOAT DEFAULT 0,
    preferred_proof_type TEXT,
    trigger_topics      TEXT[] DEFAULT '{}',
    decision_style      TEXT,
    risk_tolerance      TEXT,
    change_appetite     TEXT,
    communication_style TEXT,
    metadata            JSONB DEFAULT '{}',
    created_at          TIMESTAMPTZ DEFAULT now(),
    updated_at          TIMESTAMPTZ DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_persons_org        ON persons(primary_org_id);
CREATE INDEX IF NOT EXISTS idx_persons_region     ON persons(region);
CREATE INDEX IF NOT EXISTS idx_persons_name_trgm  ON persons USING gin(name gin_trgm_ops);

-- ─── poi_artifacts ───────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS poi_artifacts (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    person_id       UUID REFERENCES persons(id) ON DELETE CASCADE,
    artifact_type   TEXT NOT NULL,
    title           TEXT,
    content_summary TEXT,
    url             TEXT NOT NULL,
    source_domain   TEXT,
    language        TEXT,
    topics          TEXT[] DEFAULT '{}',
    sentiment_score FLOAT,
    key_phrases     TEXT[] DEFAULT '{}',
    ts_utc          TIMESTAMPTZ NOT NULL,
    provenance      JSONB NOT NULL DEFAULT '{}',
    metadata        JSONB DEFAULT '{}',
    created_at      TIMESTAMPTZ DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_poi_artifacts_person ON poi_artifacts(person_id, ts_utc DESC);
CREATE INDEX IF NOT EXISTS idx_poi_artifacts_type   ON poi_artifacts(artifact_type, ts_utc DESC);

-- ─── observations ────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS observations (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    observation_type TEXT NOT NULL,
    entity_id        UUID,
    entity_type      TEXT,
    ts_utc           TIMESTAMPTZ NOT NULL,
    value            JSONB NOT NULL,
    provenance       JSONB NOT NULL DEFAULT '{}',
    confidence       FLOAT DEFAULT 1.0,
    quality_score    FLOAT DEFAULT 1.0,
    created_at       TIMESTAMPTZ DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_obs_type_entity ON observations(observation_type, entity_id, ts_utc DESC);
CREATE INDEX IF NOT EXISTS idx_obs_type_ts     ON observations(observation_type, ts_utc DESC);

-- ─── graph_edges ─────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS graph_edges (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    source_id   UUID NOT NULL,
    source_type TEXT NOT NULL,
    target_id   UUID NOT NULL,
    target_type TEXT NOT NULL,
    edge_type   TEXT NOT NULL,
    weight      FLOAT DEFAULT 1.0,
    confidence  FLOAT DEFAULT 1.0,
    evidence_ids UUID[] DEFAULT '{}',
    metadata    JSONB DEFAULT '{}',
    first_seen  TIMESTAMPTZ DEFAULT now(),
    last_seen   TIMESTAMPTZ DEFAULT now(),
    stale       BOOLEAN DEFAULT FALSE,
    UNIQUE(source_id, source_type, target_id, target_type, edge_type)
);
CREATE INDEX IF NOT EXISTS idx_graph_source    ON graph_edges(source_id, source_type, edge_type);
CREATE INDEX IF NOT EXISTS idx_graph_target    ON graph_edges(target_id, target_type, edge_type);
CREATE INDEX IF NOT EXISTS idx_graph_not_stale ON graph_edges(source_id) WHERE NOT stale;

-- ─── warnings ────────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS warnings (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    warning_type    TEXT NOT NULL,
    severity        TEXT NOT NULL DEFAULT 'medium',
    title           TEXT NOT NULL,
    description     TEXT,
    entity_id       UUID,
    entity_type     TEXT,
    region          TEXT,
    evidence        JSONB DEFAULT '[]',
    recipe_id       TEXT,
    confidence      FLOAT DEFAULT 0.5,
    impact          TEXT DEFAULT 'medium',
    actions         TEXT[] DEFAULT '{}',
    acknowledged    BOOLEAN DEFAULT FALSE,
    acknowledged_at TIMESTAMPTZ,
    acknowledged_by TEXT,
    review_outcome  TEXT,
    reviewed_by     TEXT,
    reviewed_at     TIMESTAMPTZ,
    sla_hours       INT DEFAULT 24,
    sla_deadline    TIMESTAMPTZ,
    escalation_level INT DEFAULT 0,
    metadata        JSONB DEFAULT '{}',
    created_at      TIMESTAMPTZ DEFAULT now(),
    updated_at      TIMESTAMPTZ DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_warnings_type     ON warnings(warning_type, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_warnings_severity ON warnings(severity, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_warnings_entity   ON warnings(entity_id);
CREATE INDEX IF NOT EXISTS idx_warnings_unack    ON warnings(acknowledged, sla_deadline) WHERE NOT acknowledged;

-- ─── recipe_weekly_metrics ───────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS recipe_weekly_metrics (
    recipe_code             TEXT NOT NULL REFERENCES recipes(id) ON DELETE CASCADE,
    week_start              DATE NOT NULL,
    precision_score         DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    false_positive_rate     DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    warnings_generated      BIGINT NOT NULL DEFAULT 0,
    reviewed_warnings       BIGINT NOT NULL DEFAULT 0,
    false_positive_warnings BIGINT NOT NULL DEFAULT 0,
    snapshot_at             TIMESTAMPTZ NOT NULL DEFAULT now(),
    created_at              TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at              TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (recipe_code, week_start)
);
CREATE INDEX IF NOT EXISTS idx_recipe_weekly_metrics_recipe_week ON recipe_weekly_metrics(recipe_code, week_start DESC);
CREATE INDEX IF NOT EXISTS idx_recipe_weekly_metrics_week ON recipe_weekly_metrics(week_start DESC);

-- ─── insights ────────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS insights (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    insight_type TEXT NOT NULL,
    title        TEXT NOT NULL,
    narrative    TEXT,
    entity_id    UUID,
    entity_type  TEXT,
    region       TEXT,
    recipe_id    TEXT,
    confidence   FLOAT DEFAULT 0.5,
    impact       TEXT DEFAULT 'medium',
    actions      TEXT[] DEFAULT '{}',
    evidence     JSONB DEFAULT '[]',
    metadata     JSONB DEFAULT '{}',
    created_at   TIMESTAMPTZ DEFAULT now(),
    updated_at   TIMESTAMPTZ DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_insights_type   ON insights(insight_type, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_insights_entity ON insights(entity_id);

-- ─── page_fingerprints (change detection) ────────────────────────────────────
CREATE TABLE IF NOT EXISTS page_fingerprints (
    id           BIGSERIAL PRIMARY KEY,
    url          TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    diff_summary TEXT,
    ts           TIMESTAMPTZ DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_pf_url ON page_fingerprints(url, ts DESC);

-- ─── social_signals ──────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS social_signals (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    platform         TEXT NOT NULL,
    source_id        TEXT NOT NULL,
    entity_name      TEXT,
    entity_type      TEXT,
    signal_type      TEXT,
    content          TEXT,
    sentiment        FLOAT DEFAULT 0,
    engagement_score FLOAT DEFAULT 0,
    language         TEXT,
    ts_utc           TIMESTAMPTZ,
    url              TEXT,
    topics           TEXT[] DEFAULT '{}',
    mentions         TEXT[] DEFAULT '{}',
    created_at       TIMESTAMPTZ DEFAULT now(),
    UNIQUE(platform, source_id)
);
CREATE INDEX IF NOT EXISTS idx_social_platform ON social_signals(platform, ts_utc DESC);

-- ─── recipes (runtime state) ─────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS recipes (
    id                  TEXT PRIMARY KEY,
    name                TEXT NOT NULL,
    category            TEXT,
    status              TEXT NOT NULL DEFAULT 'production',
    join_type           TEXT,
    outcome             TEXT,
    signals             JSONB DEFAULT '[]',
    transforms          JSONB DEFAULT '[]',
    test_config         JSONB DEFAULT '{}',
    thresholds          JSONB DEFAULT '{}',
    narrative_template  TEXT,
    action_playbook     JSONB DEFAULT '[]',
    applicability       JSONB DEFAULT '{}',
    priority_tier       TEXT DEFAULT 'P2',
    precision           FLOAT,
    recall              FLOAT,
    false_positive_rate FLOAT,
    last_fired          TIMESTAMPTZ,
    fire_count          INT DEFAULT 0,
    created_at          TIMESTAMPTZ DEFAULT now(),
    updated_at          TIMESTAMPTZ DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_recipes_status ON recipes(status);

-- ─── feature_rows (materialized feature store) ──────────────────────────────
CREATE TABLE IF NOT EXISTS feature_rows (
    id                    BIGSERIAL PRIMARY KEY,
    entity_id             UUID NOT NULL,
    entity_type           TEXT NOT NULL,
    time_bucket           BIGINT NOT NULL,
    bucket_size           TEXT NOT NULL DEFAULT 'daily',
    signal_counts         JSONB DEFAULT '{}',
    diffs                 JSONB DEFAULT '{}',
    pct_changes           JSONB DEFAULT '{}',
    regime_flags          JSONB DEFAULT '{}',
    volatility            JSONB DEFAULT '{}',
    topic_drift           JSONB DEFAULT '{}',
    neighbor_agg_1hop     JSONB DEFAULT '{}',
    neighbor_agg_2hop     JSONB DEFAULT '{}',
    poi_pain_index        FLOAT,
    poi_role_drift        FLOAT,
    poi_influence_delta   FLOAT,
    created_at            TIMESTAMPTZ DEFAULT now(),
    UNIQUE(entity_id, entity_type, time_bucket, bucket_size)
);
CREATE INDEX IF NOT EXISTS idx_feature_entity ON feature_rows(entity_id, entity_type, time_bucket DESC);

-- ─── audit_log ───────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS audit_log (
    id          BIGSERIAL PRIMARY KEY,
    event_type  TEXT NOT NULL,
    entity_id   TEXT,
    entity_type TEXT,
    actor       TEXT DEFAULT 'system',
    detail      JSONB DEFAULT '{}',
    ts          TIMESTAMPTZ DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_audit_ts ON audit_log(ts DESC);
CREATE INDEX IF NOT EXISTS idx_audit_type ON audit_log(event_type, ts DESC);

-- ─── crawl_telemetry ─────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS crawl_telemetry (
    id                    BIGSERIAL PRIMARY KEY,
    source_id             TEXT NOT NULL,
    fetch_ts              TIMESTAMPTZ DEFAULT now(),
    http_status           INT,
    elapsed_ms            INT,
    bytes_fetched         BIGINT,
    observations_produced INT DEFAULT 0,
    error_message         TEXT,
    created_at            TIMESTAMPTZ DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_crawl_source ON crawl_telemetry(source_id, fetch_ts DESC);

-- ─── user_preferences ────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS user_preferences (
    user_id     TEXT PRIMARY KEY,
    theme       TEXT DEFAULT 'system',
    locale      TEXT DEFAULT 'en',
    preferences JSONB DEFAULT '{}',
    updated_at  TIMESTAMPTZ DEFAULT now()
);

-- ─── poi_engagements (CRM-lite) ─────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS poi_engagements (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    person_id   UUID REFERENCES persons(id) ON DELETE CASCADE,
    method      TEXT NOT NULL,
    outcome     TEXT,
    notes       TEXT,
    next_action TEXT,
    next_date   DATE,
    actor       TEXT DEFAULT 'system',
    ts          TIMESTAMPTZ DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_poi_eng_person ON poi_engagements(person_id, ts DESC);

-- ─── entity_merges (merge/split tracking) ────────────────────────────────────
CREATE TABLE IF NOT EXISTS entity_merges (
    id            BIGSERIAL PRIMARY KEY,
    entity_type   TEXT NOT NULL,
    survivor_id   UUID NOT NULL,
    merged_ids    UUID[] NOT NULL,
    merge_reason  TEXT,
    merged_by     TEXT DEFAULT 'system',
    merged_at     TIMESTAMPTZ DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_merge_survivor ON entity_merges(survivor_id);

COMMIT;
