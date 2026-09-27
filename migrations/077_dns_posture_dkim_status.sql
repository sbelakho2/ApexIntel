-- ════════════════════════════════════════════════════════════════════════════
-- Migration 077: DKIM tri-state on DNS posture entries
-- DKIM tri-state on DNS posture entries
-- ════════════════════════════════════════════════════════════════════════════
-- `has_dkim BOOLEAN` could not distinguish "no DKIM record on the known
-- selectors" from "the resolver timed out / failed, so DKIM state is
-- unknown". Warning generation must not report a missing-DKIM finding for an
-- indeterminate lookup, so store the tri-state alongside the boolean:
--
--   confirmed_present | not_observed_on_known_selectors | unknown
--
-- `has_dkim` remains for compatibility and is only TRUE for
-- `confirmed_present`; dashboards that need the distinction read dkim_status.
-- ════════════════════════════════════════════════════════════════════════════

ALTER TABLE dns_posture_entries
    ADD COLUMN IF NOT EXISTS dkim_status TEXT NOT NULL DEFAULT 'unknown';

ALTER TABLE dns_posture_entries
    ADD COLUMN IF NOT EXISTS dkim_unknown_reason TEXT;

COMMENT ON COLUMN dns_posture_entries.dkim_status IS
    'Tri-state DKIM observation: confirmed_present | not_observed_on_known_selectors | unknown';
