-- ApexIntel Schema Migration – identities, normalized tags, and delivery tracking

CREATE TABLE IF NOT EXISTS audit_log (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    event_type TEXT NOT NULL,
    actor TEXT NOT NULL,
    detail JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_audit_log_created_at
    ON audit_log (created_at DESC);

CREATE TABLE IF NOT EXISTS analyst_users (
    id TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    email TEXT,
    role TEXT NOT NULL DEFAULT 'viewer',
    notification_channels JSONB NOT NULL DEFAULT '{}'::jsonb,
    is_active BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS analyst_user_roles (
    user_id TEXT NOT NULL REFERENCES analyst_users(id) ON DELETE CASCADE,
    role TEXT NOT NULL,
    granted_by TEXT,
    granted_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, role)
);

INSERT INTO analyst_user_roles (user_id, role, granted_by)
SELECT id, LOWER(COALESCE(NULLIF(role, ''), 'viewer')), 'migration_0007'
FROM analyst_users
ON CONFLICT (user_id, role) DO NOTHING;

CREATE TABLE IF NOT EXISTS api_key_owners (
    key_id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES analyst_users(id) ON DELETE CASCADE,
    role TEXT NOT NULL,
    display_name TEXT NOT NULL,
    last_seen_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_api_key_owners_user_id
    ON api_key_owners (user_id);

CREATE TABLE IF NOT EXISTS saved_searches (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id TEXT NOT NULL REFERENCES analyst_users(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    query_text TEXT NOT NULL,
    filters JSONB NOT NULL DEFAULT '{}'::jsonb,
    default_sort TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_saved_searches_user_updated
    ON saved_searches (user_id, updated_at DESC);

CREATE TABLE IF NOT EXISTS watchlists (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id TEXT NOT NULL REFERENCES analyst_users(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    entities JSONB NOT NULL DEFAULT '[]'::jsonb,
    notes TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_watchlists_user_updated
    ON watchlists (user_id, updated_at DESC);

CREATE TABLE IF NOT EXISTS annotations (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id TEXT NOT NULL REFERENCES analyst_users(id) ON DELETE CASCADE,
    entity_type TEXT NOT NULL,
    entity_id TEXT NOT NULL,
    body TEXT NOT NULL,
    tags TEXT[] NOT NULL DEFAULT ARRAY[]::TEXT[],
    visibility TEXT NOT NULL DEFAULT 'private',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_annotations_entity_updated
    ON annotations (entity_type, entity_id, updated_at DESC);

CREATE INDEX IF NOT EXISTS idx_annotations_user_updated
    ON annotations (user_id, updated_at DESC);

CREATE TABLE IF NOT EXISTS export_history (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id TEXT NOT NULL REFERENCES analyst_users(id) ON DELETE CASCADE,
    export_type TEXT NOT NULL,
    format TEXT NOT NULL,
    filters JSONB NOT NULL DEFAULT '{}'::jsonb,
    row_count BIGINT NOT NULL DEFAULT 0,
    download_name TEXT,
    requested_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_export_history_user_requested
    ON export_history (user_id, requested_at DESC);

CREATE TABLE IF NOT EXISTS tags (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    label TEXT NOT NULL,
    normalized_label TEXT NOT NULL UNIQUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS tag_assignments (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tag_id UUID NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    subject_type TEXT NOT NULL,
    subject_id TEXT NOT NULL,
    source TEXT NOT NULL DEFAULT 'manual',
    created_by TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tag_id, subject_type, subject_id, source)
);

CREATE INDEX IF NOT EXISTS idx_tag_assignments_subject
    ON tag_assignments (subject_type, subject_id, created_at DESC);

WITH normalized_tags AS (
    SELECT TRIM(tag) AS label, LOWER(TRIM(tag)) AS normalized_label
    FROM annotations a
    CROSS JOIN LATERAL unnest(a.tags) AS tag
    WHERE TRIM(tag) <> ''
    UNION
    SELECT TRIM(tag) AS label, LOWER(TRIM(tag)) AS normalized_label
    FROM insights i
    CROSS JOIN LATERAL unnest(COALESCE(i.tags, ARRAY[]::TEXT[])) AS tag
    WHERE TRIM(tag) <> ''
)
INSERT INTO tags (label, normalized_label)
SELECT MIN(label) AS label, normalized_label
FROM normalized_tags
GROUP BY normalized_label
ON CONFLICT (normalized_label) DO NOTHING;

INSERT INTO tag_assignments (tag_id, subject_type, subject_id, source)
SELECT DISTINCT t.id, 'annotation', a.id::text, 'annotation_tag_array'
FROM annotations a
CROSS JOIN LATERAL unnest(a.tags) AS tag
JOIN tags t ON t.normalized_label = LOWER(TRIM(tag))
WHERE TRIM(tag) <> ''
ON CONFLICT (tag_id, subject_type, subject_id, source) DO NOTHING;

INSERT INTO tag_assignments (tag_id, subject_type, subject_id, source)
SELECT DISTINCT t.id, 'insight', i.id::text, 'insight_tag_array'
FROM insights i
CROSS JOIN LATERAL unnest(COALESCE(i.tags, ARRAY[]::TEXT[])) AS tag
JOIN tags t ON t.normalized_label = LOWER(TRIM(tag))
WHERE TRIM(tag) <> ''
ON CONFLICT (tag_id, subject_type, subject_id, source) DO NOTHING;

CREATE TABLE IF NOT EXISTS sla_reminder_state (
    warning_id TEXT NOT NULL,
    reminder_kind TEXT NOT NULL,
    delivery_key TEXT NOT NULL,
    detail JSONB NOT NULL DEFAULT '{}'::jsonb,
    sent_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (warning_id, reminder_kind)
);

CREATE TABLE IF NOT EXISTS notification_delivery_state (
    delivery_key TEXT PRIMARY KEY,
    channel TEXT NOT NULL,
    destination TEXT NOT NULL,
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    status TEXT NOT NULL DEFAULT 'pending',
    attempts INTEGER NOT NULL DEFAULT 0,
    last_attempt_at TIMESTAMPTZ,
    next_retry_at TIMESTAMPTZ,
    delivered_at TIMESTAMPTZ,
    last_error TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_notification_delivery_state_status_retry
    ON notification_delivery_state (status, next_retry_at, updated_at DESC);

CREATE TABLE IF NOT EXISTS notification_delivery_attempts (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    delivery_key TEXT NOT NULL REFERENCES notification_delivery_state(delivery_key) ON DELETE CASCADE,
    attempted_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    status TEXT NOT NULL,
    error TEXT
);

CREATE INDEX IF NOT EXISTS idx_notification_delivery_attempts_delivery_key
    ON notification_delivery_attempts (delivery_key, attempted_at DESC);