-- Migration: Weekly Memos and Competitor Changes
-- Adds tables for executive briefings and competitor activity tracking

-- ═══════════════════════════════════════════════════════════════════════════════
-- WEEKLY MEMOS TABLE
-- ═══════════════════════════════════════════════════════════════════════════════

CREATE TABLE IF NOT EXISTS weekly_memos (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    title TEXT NOT NULL,
    week_start DATE NOT NULL,
    week_end DATE NOT NULL,
    executive_summary TEXT NOT NULL,
    sections JSONB NOT NULL DEFAULT '[]',
    key_metrics JSONB NOT NULL DEFAULT '{}',
    action_items JSONB NOT NULL DEFAULT '[]',
    generated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT unique_weekly_memo_week UNIQUE (week_start, week_end)
);

CREATE INDEX IF NOT EXISTS idx_weekly_memos_week ON weekly_memos(week_start DESC, week_end DESC);
CREATE INDEX IF NOT EXISTS idx_weekly_memos_generated ON weekly_memos(generated_at DESC);

-- ═══════════════════════════════════════════════════════════════════════════════
-- COMPETITOR CHANGES TABLE
-- ═══════════════════════════════════════════════════════════════════════════════

CREATE TABLE IF NOT EXISTS competitor_changes (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    competitor_id UUID NOT NULL REFERENCES companies(id) ON DELETE CASCADE,
    change_type TEXT NOT NULL CHECK (change_type IN (
        'new_capability', 'capability_removed', 'market_entry', 'market_exit',
        'leadership_change', 'product_launch', 'price_change', 'partnership', 'acquisition'
    )),
    title TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    detected_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    source_url TEXT,
    impact_score DOUBLE PRECISION NOT NULL DEFAULT 0.5,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_competitor_changes_competitor ON competitor_changes(competitor_id);
CREATE INDEX IF NOT EXISTS idx_competitor_changes_detected ON competitor_changes(detected_at DESC);
CREATE INDEX IF NOT EXISTS idx_competitor_changes_type ON competitor_changes(change_type);

-- ═══════════════════════════════════════════════════════════════════════════════
-- SEED INITIAL WEEKLY MEMO
-- ═══════════════════════════════════════════════════════════════════════════════

INSERT INTO weekly_memos (
    title,
    week_start,
    week_end,
    executive_summary,
    sections,
    key_metrics,
    action_items,
    generated_at
) VALUES (
    'Weekly Intelligence Briefing - Week 9, 2026',
    DATE '2026-02-23',
    DATE '2026-03-01',
    'This week saw significant developments in global EMS supply chains, with Red Sea shipping disruptions continuing to impact European logistics. Foxconn announced expansion of India operations with a new Chennai facility, signaling continued manufacturing diversification. Several competitors showed notable activity in capability expansion and market entries.',
    '[
        {
            "title": "Supply Chain Disruptions",
            "content": "Red Sea shipping route disruptions continue to affect European EMS companies. Alternative routing via Cape of Good Hope adds 10-14 days to delivery schedules. Companies with dual-sourcing strategies showing better resilience.",
            "priority": "critical",
            "related_warnings": [],
            "related_insights": []
        },
        {
            "title": "Competitor Movements",
            "content": "Foxconn expanding India manufacturing with new Chennai facility targeting $2B investment. Jabil and Flex maintaining aggressive M&A posture in European markets. Nordic competitors showing increased interest in defense sector contracts.",
            "priority": "high",
            "related_warnings": [],
            "related_insights": []
        },
        {
            "title": "Technology Trends",
            "content": "AI integration in manufacturing processes accelerating. Smart factory capabilities becoming key differentiator in RFP responses. EV battery module assembly emerging as high-growth capability area.",
            "priority": "medium",
            "related_warnings": [],
            "related_insights": []
        }
    ]',
    '{
        "warnings_total": 47,
        "warnings_critical": 5,
        "insights_generated": 23,
        "companies_monitored": 148,
        "pois_tracked": 54
    }',
    '[
        {"text": "Review Red Sea impact on Q2 delivery commitments", "priority": "critical", "assignee": "Supply Chain"},
        {"text": "Assess Foxconn Chennai implications for customer base", "priority": "high", "assignee": "Strategy"},
        {"text": "Prepare EV capability expansion business case", "priority": "medium", "assignee": "Product"}
    ]',
    NOW()
) ON CONFLICT (week_start, week_end) DO NOTHING;

-- ═══════════════════════════════════════════════════════════════════════════════
-- SEED COMPETITOR CHANGES
-- ═══════════════════════════════════════════════════════════════════════════════

-- Insert sample competitor changes (using existing competitor IDs)
INSERT INTO competitor_changes (competitor_id, change_type, title, description, detected_at, source_url, impact_score)
SELECT
    c.id,
    'new_capability',
    'Added EV Battery Module Assembly',
    'Expanded manufacturing capabilities to include EV battery module assembly and testing facilities.',
    NOW() - INTERVAL '3 days',
    'https://docs.apexintel.local/placeholders/seed-data/ev-expansion',
    0.75
FROM companies c
WHERE c.name = 'Jabil'
    AND COALESCE((c.metadata->>'is_competitor')::boolean, false) = true
ON CONFLICT DO NOTHING;

INSERT INTO competitor_changes (competitor_id, change_type, title, description, detected_at, source_url, impact_score)
SELECT
    c.id,
    'market_entry',
    'Nordic Defense Contract Win',
    'Secured major defense electronics contract with Nordic government agency.',
    NOW() - INTERVAL '5 days',
    'https://docs.apexintel.local/placeholders/seed-data/defense-contract',
    0.85
FROM companies c
WHERE c.name = 'NOTE AB'
    AND COALESCE((c.metadata->>'is_competitor')::boolean, false) = true
ON CONFLICT DO NOTHING;

INSERT INTO competitor_changes (competitor_id, change_type, title, description, detected_at, source_url, impact_score)
SELECT
    c.id,
    'acquisition',
    'Acquired German Specialty EMS Firm',
    'Completed acquisition of specialized medical device manufacturing firm in Munich.',
    NOW() - INTERVAL '7 days',
    'https://docs.apexintel.local/placeholders/seed-data/acquisition',
    0.9
FROM companies c
WHERE c.name = 'Cicor Group'
    AND COALESCE((c.metadata->>'is_competitor')::boolean, false) = true
ON CONFLICT DO NOTHING;

INSERT INTO competitor_changes (competitor_id, change_type, title, description, detected_at, source_url, impact_score)
SELECT
    c.id,
    'leadership_change',
    'New CEO Appointed',
    'Former VP of Operations appointed as new Chief Executive Officer effective March 2026.',
    NOW() - INTERVAL '2 days',
    'https://docs.apexintel.local/placeholders/seed-data/ceo-change',
    0.65
FROM companies c
WHERE c.name = 'Sanmina'
    AND COALESCE((c.metadata->>'is_competitor')::boolean, false) = true
ON CONFLICT DO NOTHING;

INSERT INTO competitor_changes (competitor_id, change_type, title, description, detected_at, source_url, impact_score)
SELECT
    c.id,
    'product_launch',
    'Smart Factory Platform Launch',
    'Launched proprietary AI-powered smart factory management platform for customers.',
    NOW() - INTERVAL '4 days',
    'https://docs.apexintel.local/placeholders/seed-data/smart-factory',
    0.7
FROM companies c
WHERE c.name = 'Flex'
    AND COALESCE((c.metadata->>'is_competitor')::boolean, false) = true
ON CONFLICT DO NOTHING;
