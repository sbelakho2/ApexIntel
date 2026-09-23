-- Migration: Quality Score Breakdown Logging (IMP-ARCH-15)
-- Records detailed quality score components for each observation

CREATE TABLE IF NOT EXISTS quality_score_breakdown (
    id SERIAL PRIMARY KEY,
    observation_id UUID NOT NULL,
    source_score DOUBLE PRECISION NOT NULL,
    confidence_score DOUBLE PRECISION NOT NULL,
    freshness_score DOUBLE PRECISION NOT NULL,
    final_score DOUBLE PRECISION NOT NULL,
    entity_id UUID,
    category TEXT,
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_quality_breakdown_obs ON quality_score_breakdown(observation_id);
CREATE INDEX IF NOT EXISTS idx_quality_breakdown_recorded ON quality_score_breakdown(recorded_at);
CREATE INDEX IF NOT EXISTS idx_quality_breakdown_entity ON quality_score_breakdown(entity_id);

-- Migration: Gate Decision Logging (IMP-ARCH-16)
-- Records structured gate decisions for analytics

CREATE TABLE IF NOT EXISTS gate_decisions (
    id SERIAL PRIMARY KEY,
    gate_name TEXT NOT NULL,
    input_hash TEXT NOT NULL,
    score DOUBLE PRECISION NOT NULL,
    threshold DOUBLE PRECISION NOT NULL,
    decision TEXT NOT NULL CHECK (decision IN ('pass', 'fail', 'soft_fail')),
    veto BOOLEAN NOT NULL DEFAULT FALSE,
    latency_ms INTEGER NOT NULL,
    entity_id UUID,
    category TEXT,
    attempt INTEGER NOT NULL DEFAULT 1,
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_gate_decisions_name ON gate_decisions(gate_name);
CREATE INDEX IF NOT EXISTS idx_gate_decisions_recorded ON gate_decisions(recorded_at);
CREATE INDEX IF NOT EXISTS idx_gate_decisions_entity ON gate_decisions(entity_id);

-- Aggregate view for gate fire rates (IMP-ARCH-17)
CREATE OR REPLACE VIEW gate_fire_rates AS
SELECT 
    gate_name,
    decision,
    COUNT(*) as count,
    AVG(score) as avg_score,
    AVG(latency_ms) as avg_latency_ms,
    DATE_TRUNC('hour', recorded_at) as hour
FROM gate_decisions
WHERE recorded_at > NOW() - INTERVAL '7 days'
GROUP BY gate_name, decision, DATE_TRUNC('hour', recorded_at);

-- LLM retry tracking table (IMP-ARCH-17)
CREATE TABLE IF NOT EXISTS llm_retry_stats (
    id SERIAL PRIMARY KEY,
    entity_id UUID,
    category TEXT NOT NULL,
    attempt_count INTEGER NOT NULL,
    success BOOLEAN NOT NULL,
    failure_reasons TEXT[],
    total_latency_ms INTEGER NOT NULL,
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_llm_retry_recorded ON llm_retry_stats(recorded_at);
CREATE INDEX IF NOT EXISTS idx_llm_retry_entity ON llm_retry_stats(entity_id);

-- Insight acceptance/rejection tracking (IMP-ARCH-17)
CREATE TABLE IF NOT EXISTS insight_outcomes (
    id SERIAL PRIMARY KEY,
    entity_id UUID,
    category TEXT NOT NULL,
    outcome TEXT NOT NULL CHECK (outcome IN ('accepted', 'rejected', 'fallback')),
    confidence DOUBLE PRECISION,
    rejection_reason TEXT,
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_insight_outcomes_recorded ON insight_outcomes(recorded_at);
CREATE INDEX IF NOT EXISTS idx_insight_outcomes_outcome ON insight_outcomes(outcome);

-- Aggregate view for insight ratios
CREATE OR REPLACE VIEW insight_acceptance_ratios AS
SELECT 
    category,
    outcome,
    COUNT(*) as count,
    AVG(confidence) as avg_confidence,
    DATE_TRUNC('day', recorded_at) as day
FROM insight_outcomes
WHERE recorded_at > NOW() - INTERVAL '30 days'
GROUP BY category, outcome, DATE_TRUNC('day', recorded_at);
