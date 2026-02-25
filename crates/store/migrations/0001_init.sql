-- ApexIntel Schema Migration – init.sql
-- Idempotent: uses IF NOT EXISTS throughout.

CREATE TABLE IF NOT EXISTS companies (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name TEXT NOT NULL,
    legal_name TEXT,
    domain TEXT UNIQUE,
    country_code TEXT,
    region TEXT,
    company_type TEXT,
    industry_tags TEXT[],
    employee_estimate INT,
    revenue_estimate_usd BIGINT,
    risk_score FLOAT DEFAULT 0,
    threat_score FLOAT DEFAULT 0,
    overlap_score FLOAT DEFAULT 0,
    strategic_relevance FLOAT DEFAULT 0,
    metadata JSONB DEFAULT '{}',
    created_at TIMESTAMPTZ DEFAULT now(),
    updated_at TIMESTAMPTZ DEFAULT now()
);

CREATE TABLE IF NOT EXISTS sites (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    company_id UUID REFERENCES companies(id),
    name TEXT NOT NULL,
    address TEXT,
    city TEXT,
    country_code TEXT,
    region TEXT,
    lat DOUBLE PRECISION,
    lon DOUBLE PRECISION,
    site_type TEXT,
    capabilities TEXT[],
    certifications TEXT[],
    employee_estimate INT,
    free_zone TEXT,
    metadata JSONB DEFAULT '{}',
    created_at TIMESTAMPTZ DEFAULT now(),
    updated_at TIMESTAMPTZ DEFAULT now()
);

CREATE TABLE IF NOT EXISTS product_families (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    company_id UUID REFERENCES companies(id),
    name TEXT NOT NULL,
    hs_codes TEXT[],
    tech_tags TEXT[],
    metadata JSONB DEFAULT '{}',
    created_at TIMESTAMPTZ DEFAULT now()
);

CREATE TABLE IF NOT EXISTS capabilities (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    company_id UUID REFERENCES companies(id),
    site_id UUID REFERENCES sites(id),
    capability TEXT NOT NULL,
    proof_grade TEXT,
    evidence_urls TEXT[],
    first_seen TIMESTAMPTZ DEFAULT now(),
    last_confirmed TIMESTAMPTZ DEFAULT now(),
    metadata JSONB DEFAULT '{}'
);

CREATE TABLE IF NOT EXISTS certifications (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    company_id UUID REFERENCES companies(id),
    site_id UUID REFERENCES sites(id),
    standard TEXT NOT NULL,
    status TEXT DEFAULT 'active',
    issuing_body TEXT,
    valid_from DATE,
    valid_until DATE,
    scope TEXT,
    evidence_url TEXT,
    metadata JSONB DEFAULT '{}',
    created_at TIMESTAMPTZ DEFAULT now(),
    updated_at TIMESTAMPTZ DEFAULT now()
);

CREATE TABLE IF NOT EXISTS logistics_nodes (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name TEXT NOT NULL,
    node_type TEXT,
    country_code TEXT,
    lat DOUBLE PRECISION,
    lon DOUBLE PRECISION,
    metadata JSONB DEFAULT '{}'
);

CREATE TABLE IF NOT EXISTS regulations (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name TEXT NOT NULL,
    regulation_type TEXT,
    jurisdiction TEXT,
    effective_date DATE,
    summary TEXT,
    source_url TEXT,
    metadata JSONB DEFAULT '{}',
    created_at TIMESTAMPTZ DEFAULT now()
);

CREATE TABLE IF NOT EXISTS persons (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name TEXT NOT NULL,
    name_ar TEXT,
    name_fr TEXT,
    primary_org_id UUID REFERENCES companies(id),
    current_role TEXT,
    role_family TEXT,
    region TEXT,
    country_code TEXT,
    public_bio TEXT,
    public_email TEXT,
    photo_hash TEXT,
    priority_vector JSONB DEFAULT '{"cost":0,"quality":0,"speed":0,"resilience":0,"compliance":0,"security":0}',
    decision_mode TEXT,
    influence_score FLOAT DEFAULT 0,
    role_drift_score FLOAT DEFAULT 0,
    change_risk FLOAT DEFAULT 0,
    pain_index FLOAT DEFAULT 0,
    preferred_proof_type TEXT,
    trigger_topics TEXT[],
    decision_style TEXT,
    risk_tolerance TEXT,
    change_appetite TEXT,
    communication_style TEXT,
    metadata JSONB DEFAULT '{}',
    created_at TIMESTAMPTZ DEFAULT now(),
    updated_at TIMESTAMPTZ DEFAULT now()
);

CREATE TABLE IF NOT EXISTS poi_artifacts (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    person_id UUID REFERENCES persons(id),
    artifact_type TEXT NOT NULL,
    title TEXT,
    content_summary TEXT,
    url TEXT NOT NULL,
    source_domain TEXT,
    language TEXT,
    topics TEXT[],
    sentiment_score FLOAT,
    key_phrases TEXT[],
    ts_utc TIMESTAMPTZ NOT NULL,
    provenance JSONB NOT NULL,
    metadata JSONB DEFAULT '{}',
    created_at TIMESTAMPTZ DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_poi_artifacts_person ON poi_artifacts(person_id, ts_utc DESC);
CREATE INDEX IF NOT EXISTS idx_poi_artifacts_type ON poi_artifacts(artifact_type, ts_utc DESC);

CREATE TABLE IF NOT EXISTS observations (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    observation_type TEXT NOT NULL,
    entity_id UUID,
    entity_type TEXT,
    ts_utc TIMESTAMPTZ NOT NULL,
    value JSONB NOT NULL,
    provenance JSONB NOT NULL,
    confidence FLOAT DEFAULT 1.0,
    created_at TIMESTAMPTZ DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_obs_type_entity ON observations(observation_type, entity_id, ts_utc DESC);
CREATE INDEX IF NOT EXISTS idx_obs_type_ts ON observations(observation_type, ts_utc DESC);

CREATE TABLE IF NOT EXISTS graph_edges (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    source_id UUID NOT NULL,
    source_type TEXT NOT NULL,
    target_id UUID NOT NULL,
    target_type TEXT NOT NULL,
    edge_type TEXT NOT NULL,
    weight FLOAT DEFAULT 1.0,
    confidence FLOAT DEFAULT 1.0,
    evidence_ids UUID[],
    metadata JSONB DEFAULT '{}',
    first_seen TIMESTAMPTZ DEFAULT now(),
    last_seen TIMESTAMPTZ DEFAULT now(),
    UNIQUE(source_id, source_type, target_id, target_type, edge_type)
);

CREATE INDEX IF NOT EXISTS idx_graph_source ON graph_edges(source_id, source_type, edge_type);
CREATE INDEX IF NOT EXISTS idx_graph_target ON graph_edges(target_id, target_type, edge_type);

-- Page fingerprints for change detection
CREATE TABLE IF NOT EXISTS page_fingerprints (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    url TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    ts TIMESTAMPTZ DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_page_fp_url ON page_fingerprints(url, ts DESC);

-- Feature store (materialized feature rows)
CREATE TABLE IF NOT EXISTS feature_rows (
    entity_id TEXT NOT NULL,
    entity_type TEXT NOT NULL,
    time_bucket BIGINT NOT NULL,
    bucket_size_days INT NOT NULL,
    data JSONB NOT NULL,
    created_at TIMESTAMPTZ DEFAULT now(),
    PRIMARY KEY (entity_id, entity_type, time_bucket, bucket_size_days)
);

-- Recipe registry
CREATE TABLE IF NOT EXISTS recipes (
    code TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    status TEXT DEFAULT 'seed',
    definition JSONB NOT NULL,
    precision_score FLOAT,
    created_at TIMESTAMPTZ DEFAULT now(),
    updated_at TIMESTAMPTZ DEFAULT now()
);

-- Warnings (fired recipe instances)
CREATE TABLE IF NOT EXISTS warnings (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    recipe_code TEXT REFERENCES recipes(code),
    warning_type TEXT NOT NULL,
    title TEXT NOT NULL,
    description TEXT,
    severity TEXT NOT NULL,
    region TEXT,
    source_urls TEXT[],
    entity_ids UUID[],
    confidence FLOAT DEFAULT 1.0,
    ts_utc TIMESTAMPTZ NOT NULL,
    acknowledged BOOLEAN DEFAULT FALSE,
    acknowledged_by TEXT,
    acknowledged_at TIMESTAMPTZ,
    acknowledged_note TEXT,
    created_at TIMESTAMPTZ DEFAULT now(),
    updated_at TIMESTAMPTZ DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_warnings_ts ON warnings(ts_utc DESC);
CREATE INDEX IF NOT EXISTS idx_warnings_recipe ON warnings(recipe_code, ts_utc DESC);

-- Insights (analytics layer)
CREATE TABLE IF NOT EXISTS insights (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    title TEXT NOT NULL,
    summary TEXT NOT NULL,
    insight_type TEXT,
    region TEXT,
    confidence FLOAT DEFAULT 0.0,
    evidence_urls TEXT[],
    entity_ids UUID[],
    tags TEXT[],
    created_at TIMESTAMPTZ DEFAULT now(),
    updated_at TIMESTAMPTZ DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_insights_region ON insights(region, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_insights_type ON insights(insight_type, created_at DESC);

-- Outcome events for the learning loop
CREATE TABLE IF NOT EXISTS outcome_events (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    event TEXT NOT NULL,
    entity_id UUID NOT NULL,
    entity_type TEXT NOT NULL,
    ts_utc TIMESTAMPTZ NOT NULL,
    details JSONB DEFAULT '{}',
    source_observation_ids UUID[],
    created_at TIMESTAMPTZ DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_outcome_entity ON outcome_events(entity_id, ts_utc DESC);
CREATE INDEX IF NOT EXISTS idx_outcome_event ON outcome_events(event, ts_utc DESC);

-- Foreign key column indexes (PostgreSQL does not auto-index FK columns)
CREATE INDEX IF NOT EXISTS idx_sites_company ON sites(company_id);
CREATE INDEX IF NOT EXISTS idx_product_families_company ON product_families(company_id);
CREATE INDEX IF NOT EXISTS idx_capabilities_company ON capabilities(company_id);
CREATE INDEX IF NOT EXISTS idx_capabilities_site ON capabilities(site_id);
CREATE INDEX IF NOT EXISTS idx_certifications_company ON certifications(company_id);
CREATE INDEX IF NOT EXISTS idx_certifications_site ON certifications(site_id);
CREATE INDEX IF NOT EXISTS idx_persons_org ON persons(primary_org_id);

-- Filter/sort column indexes
CREATE INDEX IF NOT EXISTS idx_companies_region ON companies(region);
CREATE INDEX IF NOT EXISTS idx_companies_type ON companies(company_type);
CREATE INDEX IF NOT EXISTS idx_companies_name ON companies(name);
CREATE INDEX IF NOT EXISTS idx_warnings_severity ON warnings(severity);
CREATE INDEX IF NOT EXISTS idx_warnings_type ON warnings(warning_type);
CREATE INDEX IF NOT EXISTS idx_warnings_ack ON warnings(acknowledged);
CREATE INDEX IF NOT EXISTS idx_persons_region ON persons(region);
CREATE INDEX IF NOT EXISTS idx_warnings_region ON warnings(region);
CREATE INDEX IF NOT EXISTS idx_companies_updated_at ON companies(updated_at DESC);
