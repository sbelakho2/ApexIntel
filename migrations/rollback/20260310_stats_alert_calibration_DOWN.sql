-- Rollback: 20260310_stats_alert_calibration
-- Drops the alert calibration events table and its indexes.

DROP INDEX IF EXISTS idx_stats_alert_calibration_entity_predicted;
DROP INDEX IF EXISTS idx_stats_alert_calibration_expected_by;
DROP TABLE IF EXISTS stats_alert_calibration_events;
