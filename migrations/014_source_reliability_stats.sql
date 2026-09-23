CREATE TABLE IF NOT EXISTS source_reliability_stats (
    source_domain TEXT PRIMARY KEY,
    tier TEXT NOT NULL,
    observation_count BIGINT NOT NULL DEFAULT 0,
    confirmed_count BIGINT NOT NULL DEFAULT 0,
    observed_reliability DOUBLE PRECISION NOT NULL,
    effective_reliability DOUBLE PRECISION NOT NULL,
    promotion_recommended BOOLEAN NOT NULL DEFAULT FALSE,
    last_refreshed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    promotion_alerted_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_source_reliability_stats_effective
    ON source_reliability_stats (effective_reliability DESC, observation_count DESC);

CREATE INDEX IF NOT EXISTS idx_source_reliability_stats_promotion
    ON source_reliability_stats (promotion_recommended, promotion_alerted_at)
    WHERE promotion_recommended = TRUE;
