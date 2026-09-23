-- ════════════════════════════════════════════════════════════════════════════
-- Migration 20260701: Sales Activation Layer
-- ════════════════════════════════════════════════════════════════════════════
-- PURPOSE: Close the activation gap that separates ApexIntel's intelligence
-- core from a revenue-driving B2B sales-OSINT product. Adds the schema needed
-- for: real win/loss analysis, competitor pricing intelligence, verified
-- contact-data capture, outreach engagement tracking, buying-center graph
-- modeling, ICP-fit scoring, and real crawl-source telemetry.
--
-- Every statement is idempotent (IF NOT EXISTS / DO blocks) so the migration
-- is safe to re-run on an already-partially-migrated database.
-- ════════════════════════════════════════════════════════════════════════════

BEGIN;

-- ─────────────────────────────────────────────────────────────────────────────
-- 1. CLOSED DEALS  (feeds BattlecardGenerator::generate_win_loss + WinLossAnalyzer)
--    The previous generate_win_loss() returned an empty default because there
--    was no table to read real deal outcomes from.
-- ─────────────────────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS closed_deals (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    our_company_id    UUID NOT NULL REFERENCES companies(id) ON DELETE CASCADE,
    competitor_id     UUID REFERENCES companies(id) ON DELETE SET NULL,
    opportunity_id    UUID REFERENCES pipeline_opportunities(id) ON DELETE SET NULL,
    deal_name         TEXT NOT NULL,
    account_name      TEXT,
    deal_value        DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    -- closed_won=true means we won; false means we lost (often to competitor_id).
    won               BOOLEAN NOT NULL DEFAULT FALSE,
    loss_reason       TEXT,
    -- freeform + normalized buckets for loss-reason analytics
    loss_reason_category TEXT
        CHECK (loss_reason_category IS NULL OR loss_reason_category IN (
            'price', 'features', 'relationship', 'timing', 'competition',
            'procurement', 'technical_fit', 'risk', 'no_decision', 'other'
        )),
    closed_at         TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    owner_id          TEXT,
    source            TEXT NOT NULL DEFAULT 'manual'
        CHECK (source IN ('manual', 'starzcrm', 'salesforce', 'hubspot', 'api')),
    external_ref      TEXT,
    metadata          JSONB NOT NULL DEFAULT '{}',
    created_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(our_company_id, external_ref)
);

CREATE INDEX IF NOT EXISTS idx_closed_deals_competitor
    ON closed_deals(competitor_id) WHERE competitor_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_closed_deals_our_company
    ON closed_deals(our_company_id, closed_at DESC);
CREATE INDEX IF NOT EXISTS idx_closed_deals_won
    ON closed_deals(our_company_id, won, closed_at DESC);
CREATE INDEX IF NOT EXISTS idx_closed_deals_loss_reason
    ON closed_deals(loss_reason_category) WHERE loss_reason_category IS NOT NULL;
-- Partial unique index backing the ON CONFLICT (... ) WHERE external_ref IS NOT NULL
-- upsert in store::upsert_closed_deal. The table-level UNIQUE allows NULL
-- external_ref rows to coexist (Postgres treats NULLs as distinct), while this
-- partial index gives the partial-conflict target a definite index to match.
CREATE UNIQUE INDEX IF NOT EXISTS uq_closed_deals_external_ref
    ON closed_deals(our_company_id, external_ref) WHERE external_ref IS NOT NULL;


-- ─────────────────────────────────────────────────────────────────────────────
-- 2. COMPETITOR PRICING INTELLIGENCE  (feeds BattlecardGenerator::generate_pricing)
--    Replaces the hardcoded "Unknown — no pricing intelligence collected".
-- ─────────────────────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS competitor_pricing (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    competitor_id     UUID NOT NULL REFERENCES companies(id) ON DELETE CASCADE,
    our_company_id    UUID REFERENCES companies(id) ON DELETE CASCADE,
    product_category  TEXT NOT NULL DEFAULT 'general',
    pricing_model     TEXT NOT NULL DEFAULT 'unknown'
        CHECK (pricing_model IN (
            'one_time', 'subscription', 'usage_based', 'tiered',
            'per_unit', 'value_based', 'quote_based', 'freemium', 'unknown'
        )),
    price_range_low   DOUBLE PRECISION,
    price_range_high  DOUBLE PRECISION,
    currency          TEXT NOT NULL DEFAULT 'USD',
    average_contract_value DOUBLE PRECISION,
    -- Aggressive / Moderate / Conservative / Unknown
    discounting_behavior TEXT NOT NULL DEFAULT 'unknown'
        CHECK (discounting_behavior IN (
            'aggressive', 'moderate', 'conservative', 'unknown'
        )),
    -- Premium / Value / Low-cost / Unknown
    competitive_position TEXT NOT NULL DEFAULT 'unknown'
        CHECK (competitive_position IN (
            'premium', 'value', 'low_cost', 'unknown'
        )),
    evidence_url      TEXT,
    observed_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    source            TEXT NOT NULL DEFAULT 'manual',
    confidence        FLOAT NOT NULL DEFAULT 0.5
        CHECK (confidence >= 0.0 AND confidence <= 1.0),
    metadata          JSONB NOT NULL DEFAULT '{}',
    created_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(competitor_id, product_category, currency)
);

CREATE INDEX IF NOT EXISTS idx_competitor_pricing_comp
    ON competitor_pricing(competitor_id, observed_at DESC);


-- ─────────────────────────────────────────────────────────────────────────────
-- 3. CONTACT METHODS  (verified email/phone/LinkedIn — the ZoomInfo/Apollo gap)
--    persons has public_email/public_phone columns but no enrichment source
--    ever populated them. This table stores verified contacts with provenance.
-- ─────────────────────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS contact_methods (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    person_id       UUID NOT NULL REFERENCES persons(id) ON DELETE CASCADE,
    -- email / phone / linkedin / twitter / direct_dial / mobile
    contact_type    TEXT NOT NULL
        CHECK (contact_type IN (
            'email', 'phone', 'mobile', 'direct_dial', 'linkedin',
            'twitter', 'website', 'other'
        )),
    value           TEXT NOT NULL,
    -- Confidence the contact is valid (0..1) per the verification provider.
    confidence      FLOAT NOT NULL DEFAULT 0.5
        CHECK (confidence >= 0.0 AND confidence <= 1.0),
    -- not_verified / syntax_valid / smtp_verified / guessed / bounced
    verification_status TEXT NOT NULL DEFAULT 'not_verified'
        CHECK (verification_status IN (
            'not_verified', 'syntax_valid', 'smtp_verified',
            'guessed', 'bounced', 'manual_confirmed'
        )),
    verified_at     TIMESTAMPTZ,
    -- enrichment provider provenance
    source          TEXT NOT NULL DEFAULT 'manual'
        CHECK (source IN (
            'manual', 'apollo', 'clearbit', 'hunter', 'snov', 'zoominfo',
            'linkedin', 'website_scrape', 'starzcrm', 'other'
        )),
    is_primary      BOOLEAN NOT NULL DEFAULT FALSE,
    last_seen_at    TIMESTAMPTZ,
    metadata        JSONB NOT NULL DEFAULT '{}',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(person_id, contact_type, value)
);

-- One primary contact per type per person.
CREATE UNIQUE INDEX IF NOT EXISTS uq_contact_methods_primary
    ON contact_methods(person_id, contact_type) WHERE is_primary = TRUE;
CREATE INDEX IF NOT EXISTS idx_contact_methods_person
    ON contact_methods(person_id, contact_type);
CREATE INDEX IF NOT EXISTS idx_contact_methods_verified
    ON contact_methods(contact_type, verification_status)
    WHERE verification_status IN ('smtp_verified', 'manual_confirmed');


-- ─────────────────────────────────────────────────────────────────────────────
-- 4. ENGAGEMENT EVENTS  (wires the orphaned poi::engagement_tracker)
--    The outreach-feedback closed loop: "we contacted X via channel Y, outcome Z".
--    Previously engagement_tracker.rs was real code with NO table/API/caller.
-- ─────────────────────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS engagement_events (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    person_id       UUID NOT NULL REFERENCES persons(id) ON DELETE CASCADE,
    opportunity_id  UUID REFERENCES pipeline_opportunities(id) ON DELETE SET NULL,
    -- email / phone / linkedin / in_person / video_call / conference / referral
    channel         TEXT NOT NULL
        CHECK (channel IN (
            'email', 'phone', 'linkedin', 'in_person',
            'video_call', 'conference', 'referral', 'other'
        )),
    direction       TEXT NOT NULL DEFAULT 'outbound'
        CHECK (direction IN ('outbound', 'inbound')),
    -- positive / neutral / negative / no_response / meeting_booked / reply
    outcome         TEXT NOT NULL DEFAULT 'no_response'
        CHECK (outcome IN (
            'positive', 'neutral', 'negative', 'no_response',
            'meeting_booked', 'reply', 'bounce'
        )),
    -- 0..1 weight used by poi::engagement_tracker::compute_summary.
    outcome_weight  FLOAT NOT NULL DEFAULT 0.0,
    -- freeform subject / message-id / cadence step id for sequencing
    subject         TEXT,
    message_ref     TEXT,
    cadence_step    INT,
    occurred_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    owner_id        TEXT,
    metadata        JSONB NOT NULL DEFAULT '{}',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_engagement_events_person_time
    ON engagement_events(person_id, occurred_at DESC);
CREATE INDEX IF NOT EXISTS idx_engagement_events_channel
    ON engagement_events(channel, occurred_at DESC);
CREATE INDEX IF NOT EXISTS idx_engagement_events_outcome
    ON engagement_events(outcome) WHERE outcome IN ('positive', 'negative', 'meeting_booked');


-- ─────────────────────────────────────────────────────────────────────────────
-- 5. BUYING CENTER GRAPH  (models the deal's decision unit as a graph)
--    pipeline_opportunities previously had NO FK to companies/persons. This
--    adds that linkage and a structured buying-committee model
--    (champion / economic_buyer / decision_maker / influencer / blocker / user).
-- ─────────────────────────────────────────────────────────────────────────────

-- Link opportunities to their target account + competitor.
ALTER TABLE pipeline_opportunities
    ADD COLUMN IF NOT EXISTS company_id UUID REFERENCES companies(id) ON DELETE SET NULL;
ALTER TABLE pipeline_opportunities
    ADD COLUMN IF NOT EXISTS competitor_id UUID REFERENCES companies(id) ON DELETE SET NULL;
CREATE INDEX IF NOT EXISTS idx_pipeline_opportunities_company
    ON pipeline_opportunities(company_id) WHERE company_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS buying_centers (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    opportunity_id    UUID REFERENCES pipeline_opportunities(id) ON DELETE CASCADE,
    company_id        UUID NOT NULL REFERENCES companies(id) ON DELETE CASCADE,
    name              TEXT NOT NULL DEFAULT 'Default Buying Center',
    deal_value        DOUBLE PRECISION,
    status            TEXT NOT NULL DEFAULT 'forming'
        CHECK (status IN ('forming', 'engaged', 'decided', 'dissolved')),
    metadata          JSONB NOT NULL DEFAULT '{}',
    created_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_buying_centers_company
    ON buying_centers(company_id);
CREATE INDEX IF NOT EXISTS idx_buying_centers_opportunity
    ON buying_centers(opportunity_id) WHERE opportunity_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS buying_center_members (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    buying_center_id    UUID NOT NULL REFERENCES buying_centers(id) ON DELETE CASCADE,
    person_id           UUID NOT NULL REFERENCES persons(id) ON DELETE CASCADE,
    -- SaaS-canonical committee roles
    role                TEXT NOT NULL DEFAULT 'influencer'
        CHECK (role IN (
            'champion', 'economic_buyer', 'decision_maker',
            'influencer', 'blocker', 'user', 'gatekeeper', 'coach'
        )),
    influence_score     FLOAT NOT NULL DEFAULT 0.5
        CHECK (influence_score >= 0.0 AND influence_score <= 1.0),
    -- BANT readiness
    budget_authority    BOOLEAN NOT NULL DEFAULT FALSE,
    need_signal         FLOAT NOT NULL DEFAULT 0.0
        CHECK (need_signal >= 0.0 AND need_signal <= 1.0),
    timeline_horizon    TEXT,
    notes               TEXT,
    metadata            JSONB NOT NULL DEFAULT '{}',
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(buying_center_id, person_id)
);

CREATE INDEX IF NOT EXISTS idx_buying_center_members_bc
    ON buying_center_members(buying_center_id, influence_score DESC);
CREATE INDEX IF NOT EXISTS idx_buying_center_members_role
    ON buying_center_members(role);
CREATE INDEX IF NOT EXISTS idx_buying_center_members_person
    ON buying_center_members(person_id);


-- ─────────────────────────────────────────────────────────────────────────────
-- 6. ICP-FIT SCORING  (account-fit score on companies — the 6sense/Clay gap)
-- ─────────────────────────────────────────────────────────────────────────────
ALTER TABLE companies
    ADD COLUMN IF NOT EXISTS icp_fit_score FLOAT NOT NULL DEFAULT 0.0
        CHECK (icp_fit_score >= 0.0 AND icp_fit_score <= 1.0);
ALTER TABLE companies
    ADD COLUMN IF NOT EXISTS intent_signal_score FLOAT NOT NULL DEFAULT 0.0
        CHECK (intent_signal_score >= 0.0 AND intent_signal_score <= 1.0);
ALTER TABLE companies
    ADD COLUMN IF NOT EXISTS tech_stack TEXT[] DEFAULT '{}';
ALTER TABLE companies
    ADD COLUMN IF NOT EXISTS funding_stage TEXT;
ALTER TABLE companies
    ADD COLUMN IF NOT EXISTS headcount_growth_pct FLOAT;
ALTER TABLE companies
    ADD COLUMN IF NOT EXISTS icp_scored_at TIMESTAMPTZ;
ALTER TABLE companies
    ADD COLUMN IF NOT EXISTS icp_breakdown JSONB DEFAULT '{}';

CREATE INDEX IF NOT EXISTS idx_companies_icp_fit
    ON companies(icp_fit_score DESC);
CREATE INDEX IF NOT EXISTS idx_companies_intent
    ON companies(intent_signal_score DESC)
    WHERE intent_signal_score > 0.0;


-- ─────────────────────────────────────────────────────────────────────────────
-- 7. CRAWL METRICS  (replaces fabricated source-scoring telemetry)
--    intelligence.rs:run_source_scoring previously fed made-up numbers
--    (error_rate: 0.05, * 0.3, fake hours_since_last_crawl) into the ranker.
--    This table captures real per-source yield/latency/error telemetry.
-- ─────────────────────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS crawl_metrics (
    id                          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    source_id                   TEXT NOT NULL,
    domain                      TEXT,
    -- Rolling window (UTC day by default)
    window_start                TIMESTAMPTZ NOT NULL,
    window_end                  TIMESTAMPTZ NOT NULL,
    observations_ingested       BIGINT NOT NULL DEFAULT 0,
    observations_in_fires       BIGINT NOT NULL DEFAULT 0,
    observations_in_promotions  BIGINT NOT NULL DEFAULT 0,
    fetch_attempts              BIGINT NOT NULL DEFAULT 0,
    fetch_errors                BIGINT NOT NULL DEFAULT 0,
    median_ingest_latency_secs  FLOAT,
    last_crawl_at               TIMESTAMPTZ,
    observation_types_produced  TEXT[] NOT NULL DEFAULT '{}',
    metadata                    JSONB NOT NULL DEFAULT '{}',
    created_at                  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(source_id, window_start)
);

CREATE INDEX IF NOT EXISTS idx_crawl_metrics_source_time
    ON crawl_metrics(source_id, window_start DESC);
CREATE INDEX IF NOT EXISTS idx_crawl_metrics_window
    ON crawl_metrics(window_start DESC);

-- Helper: upsert convenience view for the latest per-source metrics.
CREATE OR REPLACE VIEW v_latest_crawl_metrics AS
SELECT DISTINCT ON (source_id) *
FROM crawl_metrics
ORDER BY source_id, window_start DESC;


-- ─────────────────────────────────────────────────────────────────────────────
-- 8. CRM SYNC STATE  (idempotency for Starz CRM write-back)
-- ─────────────────────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS crm_sync_state (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    -- 'lead' / 'contact' / 'task' / 'activity' / 'opportunity'
    entity_type     TEXT NOT NULL,
    local_id        TEXT NOT NULL,
    external_id     TEXT NOT NULL,
    crm_system      TEXT NOT NULL DEFAULT 'starzcrm',
    direction       TEXT NOT NULL DEFAULT 'outbound'
        CHECK (direction IN ('inbound', 'outbound')),
    last_synced_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    payload_hash    TEXT,
    metadata        JSONB NOT NULL DEFAULT '{}',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(crm_system, entity_type, local_id)
);

CREATE INDEX IF NOT EXISTS idx_crm_sync_state_external
    ON crm_sync_state(crm_system, entity_type, external_id);

COMMIT;
