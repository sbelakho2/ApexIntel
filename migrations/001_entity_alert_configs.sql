-- ──────────────────────────────────────────────────────────────────────────────
-- Migration 0019: Entity Alert Configurations
--
-- Stores per-entity alert threshold overrides and global default alert settings.
-- Each config is stored as JSONB for flexibility; the application layer
-- enforces schema validation via serde deserialisation.
-- ──────────────────────────────────────────────────────────────────────────────

-- ── Entity-level alert configs ───────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS entity_alert_configs (
    entity_id       TEXT        PRIMARY KEY,
    config          JSONB       NOT NULL DEFAULT '{}'::jsonb,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

COMMENT ON TABLE  entity_alert_configs IS 'Per-entity alert threshold overrides';
COMMENT ON COLUMN entity_alert_configs.config IS 'JSONB blob matching apex_core::alert_config::EntityAlertConfig';

-- ── Global default alert settings ────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS global_alert_defaults (
    id              INTEGER     PRIMARY KEY GENERATED ALWAYS AS IDENTITY,
    config          JSONB       NOT NULL DEFAULT '{}'::jsonb,
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT single_global_row CHECK (id = 1)
);

COMMENT ON TABLE  global_alert_defaults IS 'Singleton row with global default alert thresholds';
COMMENT ON COLUMN global_alert_defaults.config IS 'JSONB blob matching apex_core::alert_config::GlobalAlertDefaults';

-- Seed the global defaults row if it does not exist yet.
-- B353: `id` is GENERATED ALWAYS AS IDENTITY — inserting an explicit value
-- requires OVERRIDING SYSTEM VALUE (this seed previously failed every
-- fresh-database bootstrap with "cannot insert a non-DEFAULT value into
-- column id").
INSERT INTO global_alert_defaults (id, config)
OVERRIDING SYSTEM VALUE
VALUES (
    1,
    '{
        "min_severity": "high",
        "enabled_channels": ["in_app", "email", "slack"],
        "cooldown_minutes": 30,
        "max_daily_alerts": 100
    }'::jsonb
)
ON CONFLICT (id) DO NOTHING;
