-- System broadcasts are persisted explicitly.
--
-- Producers mark deliberate system-wide warnings with
-- `NewWarning::system_broadcast()`, but that intent was lost at persistence:
-- the `warnings` row only kept `entity_ids = '{}'`, which is indistinguishable
-- from an unscoped warning that reaches nobody. The SLA enforcer must not guess
-- "no entities means broadcast" — it now reads this column.
--
-- Semantics:
--   TRUE  -> SLA breach/reminder alerts are explicit `SystemBroadcast` alerts.
--   FALSE + empty entity_ids -> the alert is unscoped and addresses nobody.
ALTER TABLE warnings
    ADD COLUMN IF NOT EXISTS is_system_broadcast BOOLEAN NOT NULL DEFAULT FALSE;

COMMENT ON COLUMN warnings.is_system_broadcast IS
    'Deliberate system-wide warning (see NewWarning::system_broadcast); SLA alerts inherit SystemBroadcast scope only when TRUE.';
