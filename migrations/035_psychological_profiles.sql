-- ════════════════════════════════════════════════════════════════════════════
-- Psychological & Behavioral Intelligence Tables
-- ════════════════════════════════════════════════════════════════════════════
-- Creates the core psychological profiling tables that were previously
-- deferred to migration 20260626. This file now contains the actual DDL
-- so that the psych_store module can write real data.
--
-- Tables created:
--   1. psychological_profiles       — POI psychometric snapshots
--   2. behavioral_pattern_events    — detected behavioral changes
--   3. engagement_profiles          — recommended engagement strategies
--   4. sentiment_time_series        — aggregate sentiment bucketing
--   5. psych_profile_sources        — evidence-to-profile linkage
-- ════════════════════════════════════════════════════════════════════════════

-- ── 1. Psychological Profiles ────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS psychological_profiles (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    person_id TEXT NOT NULL,
    decision_style TEXT NOT NULL DEFAULT 'unknown'
        CHECK (decision_style IN (
            'authoritative', 'collaborative', 'analytical',
            'consensus_driven', 'data_driven', 'intuitive',
            'delegative', 'unknown'
        )),
    change_appetite TEXT NOT NULL DEFAULT 'moderate'
        CHECK (change_appetite IN ('high', 'moderate', 'low', 'resistant')),
    pain_index FLOAT NOT NULL DEFAULT 0.0
        CHECK (pain_index >= 0.0 AND pain_index <= 1.0),
    risk_tolerance FLOAT NOT NULL DEFAULT 0.5
        CHECK (risk_tolerance >= 0.0 AND risk_tolerance <= 1.0),
    preferred_proof TEXT[] NOT NULL DEFAULT '{}',
    enrichment_quality FLOAT NOT NULL DEFAULT 0.0
        CHECK (enrichment_quality >= 0.0 AND enrichment_quality <= 1.0),
    evidence_sources TEXT[] NOT NULL DEFAULT '{}',
    metadata JSONB NOT NULL DEFAULT '{}',
    computed_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_psych_profiles_person
    ON psychological_profiles (person_id, computed_at DESC);

-- ── 2. Behavioral Pattern Events ─────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS behavioral_pattern_events (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    person_id TEXT NOT NULL,
    event_type TEXT NOT NULL
        CHECK (event_type IN (
            'sentiment_shift', 'communication_drift', 'priority_change',
            'risk_appetite_change', 'engagement_surge', 'engagement_drop',
            'role_drift', 'influence_change', 'network_expansion',
            'topic_obsession', 'silence_anomaly', 'language_mirroring'
        )),
    title TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    confidence FLOAT NOT NULL DEFAULT 0.5
        CHECK (confidence >= 0.0 AND confidence <= 1.0),
    evidence_urls TEXT[] NOT NULL DEFAULT '{}',
    detected_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_behavioral_patterns_person
    ON behavioral_pattern_events (person_id, detected_at DESC);

-- ── 3. Engagement Profiles ───────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS engagement_profiles (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    person_id TEXT NOT NULL,
    talking_points TEXT[] NOT NULL DEFAULT '{}',
    opening_topics TEXT[] NOT NULL DEFAULT '{}',
    avoid_topics TEXT[] NOT NULL DEFAULT '{}',
    best_channel TEXT NOT NULL DEFAULT 'email'
        CHECK (best_channel IN (
            'email', 'phone', 'linkedin', 'in_person',
            'video_call', 'conference', 'introduction', 'other'
        )),
    best_timing TEXT,
    proof_pack JSONB NOT NULL DEFAULT '{}',
    generated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_engagement_profiles_person
    ON engagement_profiles (person_id, generated_at DESC);

-- ── 4. Sentiment Time Series ─────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS sentiment_time_series (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    entity_id TEXT NOT NULL,
    entity_type TEXT NOT NULL
        CHECK (entity_type IN ('company', 'person', 'region', 'product')),
    mean_score FLOAT NOT NULL DEFAULT 0.0
        CHECK (mean_score >= -1.0 AND mean_score <= 1.0),
    median_score FLOAT NOT NULL DEFAULT 0.0
        CHECK (median_score >= -1.0 AND median_score <= 1.0),
    std_deviation FLOAT NOT NULL DEFAULT 0.0
        CHECK (std_deviation >= 0.0),
    sample_count INT NOT NULL DEFAULT 0
        CHECK (sample_count >= 0),
    positive_count INT NOT NULL DEFAULT 0
        CHECK (positive_count >= 0),
    neutral_count INT NOT NULL DEFAULT 0
        CHECK (neutral_count >= 0),
    negative_count INT NOT NULL DEFAULT 0
        CHECK (negative_count >= 0),
    window_start TIMESTAMPTZ NOT NULL,
    window_end TIMESTAMPTZ NOT NULL,
    computed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT sentiment_valid_window CHECK (window_end > window_start),
    CONSTRAINT sentiment_counts_match CHECK (
        positive_count + neutral_count + negative_count <= sample_count
    )
);

CREATE INDEX IF NOT EXISTS idx_sentiment_time_series_entity
    ON sentiment_time_series (entity_id, entity_type, window_start);

-- ── 5. Psych Profile Evidence Sources ────────────────────────────────────────
CREATE TABLE IF NOT EXISTS psych_profile_sources (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    profile_id UUID NOT NULL REFERENCES psychological_profiles(id) ON DELETE CASCADE,
    source_url TEXT NOT NULL,
    source_domain TEXT,
    source_type TEXT NOT NULL DEFAULT 'web'
        CHECK (source_type IN (
            'web', 'social_media', 'interview', 'news',
            'public_record', 'corporate_filing', 'other'
        )),
    evidence_text TEXT,
    extraction_confidence FLOAT NOT NULL DEFAULT 0.5
        CHECK (extraction_confidence >= 0.0 AND extraction_confidence <= 1.0),
    collected_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_psych_profile_sources_profile
    ON psych_profile_sources (profile_id);