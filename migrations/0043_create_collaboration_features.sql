-- ApexIntel Phase 4.3: User Experience Enhancement - Extended Collaboration Features
-- Migration: 0043_create_collaboration_features
-- Description: Creates additional tables for annotations, comments, notifications,
--              and notification preferences

BEGIN;

-- ─────────────────────────────────────────────────────────────────────────────
-- Annotations and Comments
-- ─────────────────────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS annotations (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    entity_type VARCHAR(50) NOT NULL,
    entity_id VARCHAR(100) NOT NULL,
    parent_id UUID REFERENCES annotations(id) ON DELETE CASCADE,
    author_id VARCHAR(100) NOT NULL,
    author_name VARCHAR(255) NOT NULL,
    content TEXT NOT NULL,
    annotation_type VARCHAR(50) NOT NULL DEFAULT 'comment',
    metadata JSONB DEFAULT '{}',
    is_resolved BOOLEAN NOT NULL DEFAULT FALSE,
    resolved_by VARCHAR(100),
    resolved_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    
    CONSTRAINT chk_annotation_type CHECK (annotation_type IN (
        'comment', 'note', 'question', 'suggestion', 'correction', 'approval', 'rejection'
    )),
    CONSTRAINT chk_annotation_entity CHECK (entity_type IN (
        'company', 'person', 'warning', 'insight', 'workspace', 'opportunity', 'threat', 'evidence'
    ))
);

CREATE INDEX idx_annotations_entity ON annotations(entity_type, entity_id);
CREATE INDEX idx_annotations_parent ON annotations(parent_id) WHERE parent_id IS NOT NULL;
CREATE INDEX idx_annotations_author ON annotations(author_id);
CREATE INDEX idx_annotations_resolved ON annotations(is_resolved) WHERE is_resolved = FALSE;
CREATE INDEX idx_annotations_created ON annotations(created_at DESC);

-- ─────────────────────────────────────────────────────────────────────────────
-- Notifications
-- ─────────────────────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS notifications (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id VARCHAR(100) NOT NULL,
    notification_type VARCHAR(50) NOT NULL,
    title VARCHAR(500) NOT NULL,
    message TEXT,
    entity_type VARCHAR(50),
    entity_id VARCHAR(100),
    entity_name VARCHAR(255),
    severity VARCHAR(20) NOT NULL DEFAULT 'info',
    is_read BOOLEAN NOT NULL DEFAULT FALSE,
    is_dismissed BOOLEAN NOT NULL DEFAULT FALSE,
    action_url VARCHAR(500),
    metadata JSONB DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    read_at TIMESTAMPTZ,
    
    CONSTRAINT chk_notification_type CHECK (notification_type IN (
        'assignment', 'mention', 'comment', 'share', 'update', 'deadline', 'escalation',
        'resolution', 'approval', 'rejection', 'alert', 'system'
    )),
    CONSTRAINT chk_notification_severity CHECK (severity IN ('info', 'warning', 'critical'))
);

CREATE INDEX idx_notifications_user ON notifications(user_id);
CREATE INDEX idx_notifications_unread ON notifications(user_id, is_read) WHERE is_read = FALSE;
CREATE INDEX idx_notifications_entity ON notifications(entity_type, entity_id) WHERE entity_type IS NOT NULL;
CREATE INDEX idx_notifications_created ON notifications(created_at DESC);
CREATE INDEX idx_notifications_type ON notifications(notification_type);

-- ─────────────────────────────────────────────────────────────────────────────
-- Notification Preferences
-- ─────────────────────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS notification_preferences (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id VARCHAR(100) NOT NULL UNIQUE,
    
    -- Email preferences
    email_enabled BOOLEAN NOT NULL DEFAULT TRUE,
    email_on_assignment BOOLEAN NOT NULL DEFAULT TRUE,
    email_on_mention BOOLEAN NOT NULL DEFAULT TRUE,
    email_on_comment BOOLEAN NOT NULL DEFAULT TRUE,
    email_on_share BOOLEAN NOT NULL DEFAULT TRUE,
    email_on_deadline BOOLEAN NOT NULL DEFAULT TRUE,
    email_on_escalation BOOLEAN NOT NULL DEFAULT TRUE,
    email_digest_frequency VARCHAR(20) DEFAULT 'daily',
    
    -- In-app preferences
    in_app_enabled BOOLEAN NOT NULL DEFAULT TRUE,
    in_app_on_assignment BOOLEAN NOT NULL DEFAULT TRUE,
    in_app_on_mention BOOLEAN NOT NULL DEFAULT TRUE,
    in_app_on_comment BOOLEAN NOT NULL DEFAULT TRUE,
    in_app_on_share BOOLEAN NOT NULL DEFAULT TRUE,
    in_app_on_deadline BOOLEAN NOT NULL DEFAULT TRUE,
    in_app_on_escalation BOOLEAN NOT NULL DEFAULT TRUE,
    
    -- Severity thresholds
    min_severity_for_email VARCHAR(20) DEFAULT 'warning',
    min_severity_for_in_app VARCHAR(20) DEFAULT 'info',
    
    -- Quiet hours
    quiet_hours_enabled BOOLEAN NOT NULL DEFAULT FALSE,
    quiet_hours_start TIME,
    quiet_hours_end TIME,
    quiet_hours_timezone VARCHAR(50) DEFAULT 'UTC',
    
    -- Team preferences
    team_notifications BOOLEAN NOT NULL DEFAULT TRUE,
    mention_notifications BOOLEAN NOT NULL DEFAULT TRUE,
    
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    
    CONSTRAINT chk_digest_frequency CHECK (email_digest_frequency IN ('realtime', 'hourly', 'daily', 'weekly')),
    CONSTRAINT chk_min_severity CHECK (min_severity_for_email IN ('info', 'warning', 'critical', 'none')),
    CONSTRAINT chk_min_severity_in_app CHECK (min_severity_for_in_app IN ('info', 'warning', 'critical', 'none'))
);

CREATE INDEX idx_notification_prefs_user ON notification_preferences(user_id);

-- ─────────────────────────────────────────────────────────────────────────────
-- Bookmarks and Saves
-- ─────────────────────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS bookmarks (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id VARCHAR(100) NOT NULL,
    entity_type VARCHAR(50) NOT NULL,
    entity_id VARCHAR(100) NOT NULL,
    folder VARCHAR(100),
    notes TEXT,
    tags TEXT[] DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    
    CONSTRAINT chk_bookmark_entity CHECK (entity_type IN (
        'company', 'person', 'warning', 'insight', 'workspace', 'opportunity', 'threat', 'recipe'
    )),
    UNIQUE (user_id, entity_type, entity_id)
);

CREATE INDEX idx_bookmarks_user ON bookmarks(user_id);
CREATE INDEX idx_bookmarks_entity ON bookmarks(entity_type, entity_id);
CREATE INDEX idx_bookmarks_folder ON bookmarks(user_id, folder) WHERE folder IS NOT NULL;

-- ─────────────────────────────────────────────────────────────────────────────
-- Review Cycles for Operational View
-- ─────────────────────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS review_cycles (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name VARCHAR(255) NOT NULL,
    description TEXT,
    cycle_type VARCHAR(50) NOT NULL DEFAULT 'daily',
    start_date DATE NOT NULL,
    end_date DATE,
    status VARCHAR(50) NOT NULL DEFAULT 'active',
    metadata JSONB DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    
    CONSTRAINT chk_cycle_type CHECK (cycle_type IN ('daily', 'weekly', 'monthly', 'quarterly', 'ad-hoc')),
    CONSTRAINT chk_cycle_status CHECK (status IN ('active', 'completed', 'cancelled'))
);

CREATE INDEX idx_review_cycles_status ON review_cycles(status);
CREATE INDEX idx_review_cycles_dates ON review_cycles(start_date, end_date);

-- ─────────────────────────────────────────────────────────────────────────────
-- Review Cycle Items (actual tasks within a review cycle)
-- ─────────────────────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS review_cycle_items (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    cycle_id UUID NOT NULL REFERENCES review_cycles(id) ON DELETE CASCADE,
    item_type VARCHAR(50) NOT NULL,
    item_id VARCHAR(100) NOT NULL,
    item_title VARCHAR(500) NOT NULL,
    priority INTEGER NOT NULL DEFAULT 50,
    status VARCHAR(50) NOT NULL DEFAULT 'pending',
    assigned_to VARCHAR(100),
    due_date TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    notes TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    
    CONSTRAINT chk_item_type CHECK (item_type IN ('warning', 'insight', 'opportunity', 'threat', 'review', 'task')),
    CONSTRAINT chk_item_priority CHECK (priority >= 1 AND priority <= 100),
    CONSTRAINT chk_item_status CHECK (status IN ('pending', 'in_progress', 'completed', 'skipped', 'escalated'))
);

CREATE INDEX idx_review_cycle_items_cycle ON review_cycle_items(cycle_id);
CREATE INDEX idx_review_cycle_items_assigned ON review_cycle_items(assigned_to) WHERE assigned_to IS NOT NULL;
CREATE INDEX idx_review_cycle_items_status ON review_cycle_items(status);
CREATE INDEX idx_review_cycle_items_due ON review_cycle_items(due_date) WHERE due_date IS NOT NULL;

-- ─────────────────────────────────────────────────────────────────────────────
-- Tags and Labels
-- ─────────────────────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS tags (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name VARCHAR(100) NOT NULL UNIQUE,
    description TEXT,
    color VARCHAR(7) DEFAULT '#6B7280',
    category VARCHAR(50),
    usage_count INTEGER NOT NULL DEFAULT 0,
    created_by VARCHAR(100),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    
    CONSTRAINT chk_tag_color CHECK (color ~ '^#[0-9A-Fa-f]{6}$')
);

CREATE INDEX idx_tags_name ON tags(name);
CREATE INDEX idx_tags_category ON tags(category) WHERE category IS NOT NULL;

-- ─────────────────────────────────────────────────────────────────────────────
-- Tag Assignments
-- ─────────────────────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS tag_assignments (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tag_id UUID NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    entity_type VARCHAR(50) NOT NULL,
    entity_id VARCHAR(100) NOT NULL,
    created_by VARCHAR(100) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    
    CONSTRAINT chk_tag_entity CHECK (entity_type IN (
        'company', 'person', 'warning', 'insight', 'workspace', 'opportunity', 'threat'
    )),
    UNIQUE (tag_id, entity_type, entity_id)
);

CREATE INDEX idx_tag_assignments_tag ON tag_assignments(tag_id);
CREATE INDEX idx_tag_assignments_entity ON tag_assignments(entity_type, entity_id);

-- ─────────────────────────────────────────────────────────────────────────────
-- Update Timestamp Triggers
-- ─────────────────────────────────────────────────────────────────────────────

CREATE TRIGGER update_annotations_updated_at
    BEFORE UPDATE ON annotations
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

CREATE TRIGGER update_notifications_prefs_updated_at
    BEFORE UPDATE ON notification_preferences
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

CREATE TRIGGER update_review_cycles_updated_at
    BEFORE UPDATE ON review_cycles
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

CREATE TRIGGER update_review_cycle_items_updated_at
    BEFORE UPDATE ON review_cycle_items
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

-- ─────────────────────────────────────────────────────────────────────────────
-- Helper Functions
-- ─────────────────────────────────────────────────────────────────────────────

-- Function to create notification
CREATE OR REPLACE FUNCTION create_notification(
    p_user_id VARCHAR,
    p_notification_type VARCHAR,
    p_title VARCHAR,
    p_message TEXT DEFAULT NULL,
    p_entity_type VARCHAR DEFAULT NULL,
    p_entity_id VARCHAR DEFAULT NULL,
    p_entity_name VARCHAR DEFAULT NULL,
    p_severity VARCHAR DEFAULT 'info',
    p_action_url VARCHAR DEFAULT NULL,
    p_metadata JSONB DEFAULT '{}'
) RETURNS UUID AS $$
DECLARE
    v_notification_id UUID;
BEGIN
    INSERT INTO notifications (
        user_id, notification_type, title, message, entity_type, entity_id,
        entity_name, severity, action_url, metadata
    ) VALUES (
        p_user_id, p_notification_type, p_title, p_message, p_entity_type, p_entity_id,
        p_entity_name, p_severity, p_action_url, p_metadata
    ) RETURNING id INTO v_notification_id;
    
    RETURN v_notification_id;
END;
$$ LANGUAGE plpgsql;

-- Function to get unread notification count
CREATE OR REPLACE FUNCTION get_unread_notification_count(p_user_id VARCHAR)
RETURNS INTEGER AS $$
DECLARE
    v_count INTEGER;
BEGIN
    SELECT COUNT(*) INTO v_count
    FROM notifications
    WHERE user_id = p_user_id
        AND is_read = FALSE
        AND is_dismissed = FALSE;
    
    RETURN v_count;
END;
$$ LANGUAGE plpgsql;

-- Function to mark notifications as read
CREATE OR REPLACE FUNCTION mark_notifications_read(
    p_user_id VARCHAR,
    p_notification_ids UUID[] DEFAULT NULL
) RETURNS INTEGER AS $$
DECLARE
    v_count INTEGER;
BEGIN
    IF p_notification_ids IS NULL THEN
        UPDATE notifications
        SET is_read = TRUE, read_at = NOW()
        WHERE user_id = p_user_id AND is_read = FALSE
        RETURNING COUNT(*) INTO v_count;
    ELSE
        UPDATE notifications
        SET is_read = TRUE, read_at = NOW()
        WHERE user_id = p_user_id AND id = ANY(p_notification_ids)
        RETURNING COUNT(*) INTO v_count;
    END IF;
    
    RETURN v_count;
END;
$$ LANGUAGE plpgsql;

-- Function to increment tag usage
CREATE OR REPLACE FUNCTION increment_tag_usage(p_tag_name VARCHAR)
RETURNS VOID AS $$
BEGIN
    UPDATE tags SET usage_count = usage_count + 1 WHERE name = p_tag_name;
END;
$$ LANGUAGE plpgsql;

-- Function to decrement tag usage
CREATE OR REPLACE FUNCTION decrement_tag_usage(p_tag_name VARCHAR)
RETURNS VOID AS $$
BEGIN
    UPDATE tags SET usage_count = GREATEST(usage_count - 1, 0) WHERE name = p_tag_name;
END;
$$ LANGUAGE plpgsql;

-- Function to auto-complete review cycle items
CREATE OR REPLACE FUNCTION auto_complete_review_items(
    p_cycle_id UUID
) RETURNS INTEGER AS $$
DECLARE
    v_count INTEGER;
BEGIN
    UPDATE review_cycle_items
    SET status = 'completed', completed_at = NOW()
    WHERE cycle_id = p_cycle_id
        AND status IN ('pending', 'in_progress')
        AND due_date < NOW();
    
    GET DIAGNOSTICS v_count = ROW_COUNT;
    RETURN v_count;
END;
$$ LANGUAGE plpgsql;

-- ─────────────────────────────────────────────────────────────────────────────
-- Additional Indexes for Performance
-- ─────────────────────────────────────────────────────────────────────────────

-- Composite index for common queries
CREATE INDEX idx_annotations_entity_author ON annotations(entity_type, entity_id, author_id);
CREATE INDEX idx_notifications_user_unread ON notifications(user_id, is_read, created_at DESC) 
    WHERE is_read = FALSE AND is_dismissed = FALSE;
CREATE INDEX idx_bookmarks_user_entity ON bookmarks(user_id, entity_type, entity_id);

COMMIT;
