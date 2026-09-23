ALTER TABLE warnings
    ADD COLUMN IF NOT EXISTS deleted_at TIMESTAMPTZ;

CREATE INDEX IF NOT EXISTS idx_warnings_deleted_at ON warnings(deleted_at);
CREATE INDEX IF NOT EXISTS idx_warnings_active_created_at ON warnings(created_at DESC) WHERE deleted_at IS NULL;