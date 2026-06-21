-- ════════════════════════════════════════════════════════════════════════════
-- Psychological Profiles & Threat Intelligence Tables
-- ════════════════════════════════════════════════════════════════════════════
-- Replaces corrupted 20260624_psychological_profiles.sql and adds:
--   1. Psychological profile storage for POIs
--   2. Threat intelligence persistence (threat actors, campaigns, supply chain)
--   3. Behavioral pattern tracking
--   4. Supply chain risk assessment storage
--
-- All tables use proper constraints, indexes, and foreign keys.
-- No stub data — these are persistent stores for real computed/ingested data.
-- ════════════════════════════════════════════════════════════════════════════

BEGIN;

-- ─────────────────────────────────────────────────────────────────────────
-- Psychological Profiles
-- Stores POI psychometric profiles computed from artifact evidence.
-- ─────────────────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS psychological_profiles (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    person_id       UUID NOT NULL,
    -- Core psychometric dimensions
    decision_style  TEXT NOT NULL DEFAULT 'unknown' CHECK (
        decision_style IN (
            'cost_first', 'quality_first', 'speed_first', 'risk_first',
            'compliance_first', 'balanced_analytical', 'unknown'
        )
    ),
    change_appetite TEXT NOT NULL DEFAULT 'unknown' CHECK (
        change_appetite IN (
            'early_adopter', 'pragmatist', 'conservative', 'laggard', 'unknown'
        )
    ),
    pain_index      DOUBLE PRECISION NOT NULL DEFAULT 0.0 CHECK (pain_index >= 0.0 AND pain_index <= 1.0),
    risk_tolerance  DOUBLE PRECISION NOT NULL DEFAULT 0.0 CHECK (risk_tolerance >= 0.0 AND risk_tolerance <= 1.0),
    -- Preferred proof types (JSON array)
    preferred_proof JSONB NOT NULL DEFAULT '[]'::jsonb,
    -- Quality metadata
    enrichment_quality DOUBLE PRECISION NOT NULL DEFAULT 0.0 CHECK (enrichment_quality >= 0.0 AND enrichment_quality <= 1.0),
    evidence_sources   TEXT[] NOT NULL DEFAULT '{}',
    computed_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_validated_at  TIMESTAMPTZ,
    -- Audit
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_psych_profiles_person ON psychological_profiles(person_id);
CREATE INDEX IF NOT EXISTS idx_psych_profiles_pain ON psychological_profiles(pain_index DESC);
CREATE INDEX IF NOT EXISTS idx_psych_profiles_risk ON psychological_profiles(risk_tolerance DESC);

-- ─────────────────────────────────────────────────────────────────────────
-- Behavioral Pattern Events
-- Records detected behavioral changes in POIs over time.
-- ─────────────────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS behavioral_pattern_events (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    person_id       UUID NOT NULL,
    event_type      TEXT NOT NULL CHECK (
        event_type IN (
            'role_change', 'org_change', 'sentiment_shift',
            'influence_spike', 'engagement_opportunity',
            'risk_profile_change', 'public_statement'
        )
    ),
    title           TEXT NOT NULL,
    description     TEXT,
    confidence      DOUBLE PRECISION NOT NULL DEFAULT 0.0 CHECK (confidence >= 0.0 AND confidence <= 1.0),
    evidence_urls   TEXT[] NOT NULL DEFAULT '{}',
    detected_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    recorded_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_behavioral_person ON behavioral_pattern_events(person_id);
CREATE INDEX IF NOT EXISTS idx_behavioral_detected ON behavioral_pattern_events(detected_at DESC);
CREATE INDEX IF NOT EXISTS idx_behavioral_type ON behavioral_pattern_events(event_type);

-- ─────────────────────────────────────────────────────────────────────────
-- Engagement Profiles
-- Persists computed engagement recommendations for POIs.
-- ─────────────────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS engagement_profiles (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    person_id       UUID NOT NULL UNIQUE,
    talking_points  TEXT[] NOT NULL DEFAULT '{}',
    opening_topics  TEXT[] NOT NULL DEFAULT '{}',
    avoid_topics    TEXT[] NOT NULL DEFAULT '{}',
    best_channel    TEXT NOT NULL DEFAULT 'trade_show_referral' CHECK (
        best_channel IN (
            'direct_outreach', 'trade_show_referral',
            'referral_trusted_partner', 'existing_relationship_only'
        )
    ),
    best_timing     TEXT,
    proof_pack      TEXT[] NOT NULL DEFAULT '{}',
    computed_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_engagement_person ON engagement_profiles(person_id);

-- ─────────────────────────────────────────────────────────────────────────
-- Threat Actors
-- Known threat actors tracked by the system.
-- ─────────────────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS threat_actors (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name            TEXT NOT NULL UNIQUE,
    aliases         TEXT[] NOT NULL DEFAULT '{}',
    threat_category TEXT NOT NULL CHECK (
        threat_category IN (
            'nation_state', 'cyber_criminal', 'hacktivist',
            'insider_threat', 'supply_chain', 'unknown'
        )
    ),
    target_sectors  TEXT[] NOT NULL DEFAULT '{}',
    target_regions  TEXT[] NOT NULL DEFAULT '{}',
    sophistication  INTEGER NOT NULL DEFAULT 1 CHECK (sophistication >= 1 AND sophistication <= 5),
    activity_status TEXT NOT NULL DEFAULT 'unknown' CHECK (
        activity_status IN ('active', 'dormant', 'disbanded', 'unknown')
    ),
    last_activity   TIMESTAMPTZ,
    ttp_summary     JSONB NOT NULL DEFAULT '[]'::jsonb,
    description     TEXT,
    source_urls     TEXT[] NOT NULL DEFAULT '{}',
    confidence      DOUBLE PRECISION NOT NULL DEFAULT 0.0 CHECK (confidence >= 0.0 AND confidence <= 1.0),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_threat_actors_category ON threat_actors(threat_category);
CREATE INDEX IF NOT EXISTS idx_threat_actors_sophistication ON threat_actors(sophistication DESC);
CREATE INDEX IF NOT EXISTS idx_threat_actors_status ON threat_actors(activity_status);

-- ─────────────────────────────────────────────────────────────────────────
-- Threat Campaigns
-- Specific campaigns attributed to threat actors.
-- ─────────────────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS threat_campaigns (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    actor_id        UUID NOT NULL REFERENCES threat_actors(id) ON DELETE CASCADE,
    name            TEXT NOT NULL,
    description     TEXT,
    target_sectors  TEXT[] NOT NULL DEFAULT '{}',
    target_regions  TEXT[] NOT NULL DEFAULT '{}',
    mitre_techniques TEXT[] NOT NULL DEFAULT '{}',
    start_date      TIMESTAMPTZ,
    end_date        TIMESTAMPTZ,
    is_ongoing      BOOLEAN NOT NULL DEFAULT false,
    severity        TEXT NOT NULL DEFAULT 'medium' CHECK (
        severity IN ('low', 'medium', 'high', 'critical')
    ),
    confidence      DOUBLE PRECISION NOT NULL DEFAULT 0.0 CHECK (confidence >= 0.0 AND confidence <= 1.0),
    source_urls     TEXT[] NOT NULL DEFAULT '{}',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_campaigns_actor ON threat_campaigns(actor_id);
CREATE INDEX IF NOT EXISTS idx_campaigns_severity ON threat_campaigns(severity);
CREATE INDEX IF NOT EXISTS idx_campaigns_ongoing ON threat_campaigns(is_ongoing) WHERE is_ongoing = true;

-- ─────────────────────────────────────────────────────────────────────────
-- Supply Chain Relationships
-- Tracks supplier relationships and dependencies.
-- ─────────────────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS supply_chain_relationships (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    source_entity_id    UUID NOT NULL,
    source_entity_type  TEXT NOT NULL CHECK (source_entity_type IN ('company', 'supplier')),
    target_entity_id    UUID NOT NULL,
    target_entity_type  TEXT NOT NULL CHECK (target_entity_type IN ('company', 'supplier')),
    relationship_type   TEXT NOT NULL CHECK (
        relationship_type IN ('provides', 'sources_from', 'subcontracted', 'licensed')
    ),
    supplier_tier       INTEGER NOT NULL DEFAULT 1 CHECK (supplier_tier >= 0 AND supplier_tier <= 4),
    critical_component  BOOLEAN NOT NULL DEFAULT false,
    has_alternative     BOOLEAN NOT NULL DEFAULT false,
    contract_value_usd  DOUBLE PRECISION,
    contract_end_date   TIMESTAMPTZ,
    reliability_score   DOUBLE PRECISION DEFAULT 0.0 CHECK (reliability_score >= 0.0 AND reliability_score <= 1.0),
    last_assessed       TIMESTAMPTZ,
    evidence_urls       TEXT[] NOT NULL DEFAULT '{}',
    confidence          DOUBLE PRECISION NOT NULL DEFAULT 0.0 CHECK (confidence >= 0.0 AND confidence <= 1.0),
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_supply_source ON supply_chain_relationships(source_entity_id, source_entity_type);
CREATE INDEX IF NOT EXISTS idx_supply_target ON supply_chain_relationships(target_entity_id, target_entity_type);
CREATE INDEX IF NOT EXISTS idx_supply_tier ON supply_chain_relationships(supplier_tier);
CREATE INDEX IF NOT EXISTS idx_supply_critical ON supply_chain_relationships(critical_component) WHERE critical_component = true;

-- ─────────────────────────────────────────────────────────────────────────
-- Supplier Risk Assessments
-- Individual risk assessments for suppliers.
-- ─────────────────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS supplier_risk_assessments (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    relationship_id UUID REFERENCES supply_chain_relationships(id) ON DELETE SET NULL,
    supplier_name   TEXT NOT NULL,
    risk_category   TEXT NOT NULL CHECK (
        risk_category IN (
            'financial', 'geopolitical', 'operational', 'compliance',
            'cybersecurity', 'reputational', 'single_source', 'logistics'
        )
    ),
    risk_score      DOUBLE PRECISION NOT NULL DEFAULT 0.0 CHECK (risk_score >= 0.0 AND risk_score <= 1.0),
    impact_score    DOUBLE PRECISION NOT NULL DEFAULT 0.0 CHECK (impact_score >= 0.0 AND impact_score <= 1.0),
    probability     DOUBLE PRECISION NOT NULL DEFAULT 0.0 CHECK (probability >= 0.0 AND probability <= 1.0),
    severity        TEXT NOT NULL DEFAULT 'medium' CHECK (
        severity IN ('low', 'medium', 'high', 'critical')
    ),
    description     TEXT,
    mitigation_steps TEXT[] NOT NULL DEFAULT '{}',
    assessed_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    next_review     TIMESTAMPTZ,
    assessor        TEXT,
    source_urls     TEXT[] NOT NULL DEFAULT '{}',
    confidence      DOUBLE PRECISION NOT NULL DEFAULT 0.0 CHECK (confidence >= 0.0 AND confidence <= 1.0),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_supplier_risk_name ON supplier_risk_assessments(supplier_name);
CREATE INDEX IF NOT EXISTS idx_supplier_risk_score ON supplier_risk_assessments(risk_score DESC);
CREATE INDEX IF NOT EXISTS idx_supplier_risk_severity ON supplier_risk_assessments(severity);
CREATE INDEX IF NOT EXISTS idx_supplier_risk_assessed ON supplier_risk_assessments(assessed_at DESC);

-- ─────────────────────────────────────────────────────────────────────────
-- Supplier Capacity & Contact
-- Tracks supplier production capacity and key contacts.
-- ─────────────────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS supplier_capacities (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    supplier_name       TEXT NOT NULL,
    current_utilization DOUBLE PRECISION NOT NULL DEFAULT 0.0 CHECK (current_utilization >= 0.0 AND current_utilization <= 1.0),
    max_capacity        DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    lead_time_days      INTEGER NOT NULL DEFAULT 0 CHECK (lead_time_days >= 0),
    flex_capacity_pct   DOUBLE PRECISION NOT NULL DEFAULT 0.0 CHECK (flex_capacity_pct >= 0.0 AND flex_capacity_pct <= 1.0),
    data_source         TEXT,
    data_freshness      TIMESTAMPTZ,
    confidence          DOUBLE PRECISION NOT NULL DEFAULT 0.0 CHECK (confidence >= 0.0 AND confidence <= 1.0),
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_capacity_supplier ON supplier_capacities(supplier_name);

CREATE TABLE IF NOT EXISTS supplier_contacts (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    supplier_name   TEXT NOT NULL,
    person_id       UUID,
    name            TEXT NOT NULL,
    role            TEXT NOT NULL,
    email           TEXT,
    phone           TEXT,
    is_primary      BOOLEAN NOT NULL DEFAULT false,
    verified        BOOLEAN NOT NULL DEFAULT false,
    source_urls     TEXT[] NOT NULL DEFAULT '{}',
    confidence      DOUBLE PRECISION NOT NULL DEFAULT 0.0 CHECK (confidence >= 0.0 AND confidence <= 1.0),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_contacts_supplier ON supplier_contacts(supplier_name);
CREATE INDEX IF NOT EXISTS idx_contacts_primary ON supplier_contacts(supplier_name) WHERE is_primary = true;

-- ─────────────────────────────────────────────────────────────────────────
-- Sentiment Time Series
-- Entity-level sentiment tracking over time.
-- ─────────────────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS sentiment_time_series (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    entity_id       UUID NOT NULL,
    entity_type     TEXT NOT NULL CHECK (entity_type IN ('company', 'person', 'region', 'product')),
    time_bucket     TIMESTAMPTZ NOT NULL,
    bucket_size     TEXT NOT NULL DEFAULT 'daily' CHECK (bucket_size IN ('hourly', 'daily', 'weekly', 'monthly')),
    mean_score      DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    median_score    DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    std_deviation   DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    sample_count    INTEGER NOT NULL DEFAULT 0,
    positive_count  INTEGER NOT NULL DEFAULT 0,
    neutral_count   INTEGER NOT NULL DEFAULT 0,
    negative_count  INTEGER NOT NULL DEFAULT 0,
    -- Unique constraint: one row per entity per time bucket
    UNIQUE (entity_id, entity_type, time_bucket, bucket_size)
);

CREATE INDEX IF NOT EXISTS idx_sentiment_entity ON sentiment_time_series(entity_id, entity_type);
CREATE INDEX IF NOT EXISTS idx_sentiment_bucket ON sentiment_time_series(time_bucket DESC);
CREATE INDEX IF NOT EXISTS idx_sentiment_score ON sentiment_time_series(mean_score);

-- ─────────────────────────────────────────────────────────────────────────
-- Competitive Intelligence
-- Persisted competitive landscape data.
-- ─────────────────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS competitive_positions (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    competitor_id   UUID NOT NULL,
    competitor_name TEXT NOT NULL,
    industry        TEXT,
    threat_score    DOUBLE PRECISION NOT NULL DEFAULT 0.0 CHECK (threat_score >= 0.0 AND threat_score <= 1.0),
    overlap_score   DOUBLE PRECISION NOT NULL DEFAULT 0.0 CHECK (overlap_score >= 0.0 AND overlap_score <= 1.0),
    capabilities    JSONB NOT NULL DEFAULT '[]'::jsonb,
    market_share_pct DOUBLE PRECISION,
    strengths       TEXT[] NOT NULL DEFAULT '{}',
    weaknesses      TEXT[] NOT NULL DEFAULT '{}',
    assessed_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    source_urls     TEXT[] NOT NULL DEFAULT '{}',
    confidence      DOUBLE PRECISION NOT NULL DEFAULT 0.0 CHECK (confidence >= 0.0 AND confidence <= 1.0),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_comp_position_name ON competitive_positions(competitor_name);
CREATE INDEX IF NOT EXISTS idx_comp_position_threat ON competitive_positions(threat_score DESC);

CREATE TABLE IF NOT EXISTS strategic_predictions (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    title           TEXT NOT NULL,
    description     TEXT,
    category        TEXT NOT NULL CHECK (
        category IN ('market_shift', 'technology_disruption', 'geopolitical',
                     'regulatory_change', 'competitive_move', 'supply_chain')
    ),
    confidence      DOUBLE PRECISION NOT NULL DEFAULT 0.0 CHECK (confidence >= 0.0 AND confidence <= 1.0),
    probability     DOUBLE PRECISION NOT NULL DEFAULT 0.0 CHECK (probability >= 0.0 AND probability <= 1.0),
    impact_score    DOUBLE PRECISION NOT NULL DEFAULT 0.0 CHECK (impact_score >= 0.0 AND impact_score <= 1.0),
    time_horizon    TEXT NOT NULL DEFAULT 'medium_term' CHECK (
        time_horizon IN ('immediate', 'short_term', 'medium_term', 'long_term')
    ),
    entities_affected TEXT[] NOT NULL DEFAULT '{}',
    source_urls     TEXT[] NOT NULL DEFAULT '{}',
    generated_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_predictions_category ON strategic_predictions(category);
CREATE INDEX IF NOT EXISTS idx_predictions_confidence ON strategic_predictions(confidence DESC);

COMMIT;