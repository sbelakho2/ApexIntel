-- ──────────────────────────────────────────────────────────────────────────────
-- Migration 048: per-user alert subscriptions
--
-- Replaces the previous "empty user_ids means broadcast" convention with
-- explicit, entity-scoped opt-ins. A row says: this user wants in-app alerts
-- for this entity, optionally narrowed to one alert category and floored at a
-- minimum severity. `category IS NULL` means "every category".
--
-- The API alert router resolves `AlertAudience::Users([])` against this table;
-- no matching rows means the alert reaches nobody.
--
-- Idempotent: safe to re-run.
-- ──────────────────────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS user_alert_subscriptions (
    id            UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    -- Matches user_preferences.user_id / analyst_users.id (TEXT identities).
    user_id       TEXT        NOT NULL,
    entity_id     UUID        NOT NULL,
    -- NULL = every alert category (e.g. 'warning', 'insight', 'recipe_match').
    category      TEXT,
    min_severity  TEXT        NOT NULL DEFAULT 'medium',
    enabled       BOOLEAN     NOT NULL DEFAULT TRUE,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT user_alert_subscriptions_min_severity_check
        CHECK (lower(min_severity) IN ('info', 'low', 'medium', 'high', 'critical'))
);

-- One subscription per (user, entity, category); NULL category is normalised
-- to a sentinel so the uniqueness constraint covers "all categories" too.
CREATE UNIQUE INDEX IF NOT EXISTS uq_user_alert_subscriptions_natural
    ON user_alert_subscriptions (user_id, entity_id, COALESCE(lower(category), '*'));

CREATE INDEX IF NOT EXISTS idx_user_alert_subscriptions_entity
    ON user_alert_subscriptions (entity_id)
    WHERE enabled;

COMMENT ON TABLE user_alert_subscriptions IS
    'Per-user entity alert opt-ins; resolved by the API alert router for alerts with no explicit addressees';
COMMENT ON COLUMN user_alert_subscriptions.category IS
    'Alert category this subscription is limited to; NULL means every category';
COMMENT ON COLUMN user_alert_subscriptions.min_severity IS
    'Lowest alert severity the user wants for this subscription';
