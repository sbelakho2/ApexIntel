-- Battlecard Workflow Engine
-- Tracks competitive battlecards with JSONB sections for each analysis dimension.
-- Each battlecard maps one "our company" to one competitor.

CREATE TABLE battlecards (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    our_company_id UUID NOT NULL REFERENCES companies(id),
    competitor_id UUID NOT NULL REFERENCES companies(id),
    title TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'draft' CHECK (status IN ('draft','published','archived')),

    -- JSONB sections (each nullable until generated)
    positioning JSONB DEFAULT NULL,
    pricing JSONB DEFAULT NULL,
    feature_matrix JSONB DEFAULT NULL,
    strengths JSONB DEFAULT NULL,
    weaknesses JSONB DEFAULT NULL,
    objection_handlers JSONB DEFAULT NULL,
    kill_shots JSONB DEFAULT NULL,
    recent_news JSONB DEFAULT NULL,
    win_loss JSONB DEFAULT NULL,

    -- Metadata
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_by TEXT,
    regenerated_at TIMESTAMPTZ,

    -- Constraints
    UNIQUE(our_company_id, competitor_id)
);

CREATE INDEX idx_battlecards_status ON battlecards(status);
CREATE INDEX idx_battlecards_competitor ON battlecards(competitor_id);
CREATE INDEX idx_battlecards_updated_at ON battlecards(updated_at DESC);
