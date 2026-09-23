-- StarzCRM Integration
-- Read-only MySQL pull from the collocated StarzCRM instance.
--
-- Table: starzcrm_sync_state
-- Tracks incremental sync cursor (last_deal_id) for the hourly pull job.
--
-- Table: starzcrm_deals
-- Stores pulled deal data with extracted competitor mentions for win/loss analysis.

-- ─────────────────────────────────────────────────────────────────────────────
-- StarzCRM sync state (singleton row, cursor-based)
-- ─────────────────────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS starzcrm_sync_state (
    id INT PRIMARY KEY DEFAULT 1,
    last_synced_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_deal_id BIGINT,
    last_account_id BIGINT,
    rows_pulled INT NOT NULL DEFAULT 0,
    errors INT NOT NULL DEFAULT 0,
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

-- ─────────────────────────────────────────────────────────────────────────────
-- StarzCRM pulled deals
-- ─────────────────────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS starzcrm_deals (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    external_deal_id BIGINT NOT NULL,
    external_account_id BIGINT,
    account_name TEXT,
    deal_name TEXT,
    stage TEXT,
    amount NUMERIC,
    currency TEXT,
    close_date DATE,
    competitors_mentioned TEXT[],
    won_lost_reason TEXT,
    owner_email TEXT,
    synced_at TIMESTAMPTZ DEFAULT NOW(),
    UNIQUE (external_deal_id)
);

CREATE INDEX IF NOT EXISTS idx_starzcrm_deals_competitors
    ON starzcrm_deals USING GIN (competitors_mentioned);

CREATE INDEX IF NOT EXISTS idx_starzcrm_deals_stage
    ON starzcrm_deals(stage);

CREATE INDEX IF NOT EXISTS idx_starzcrm_deals_synced_at
    ON starzcrm_deals(synced_at DESC);
