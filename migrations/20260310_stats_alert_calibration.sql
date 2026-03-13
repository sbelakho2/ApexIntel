CREATE TABLE IF NOT EXISTS stats_alert_calibration_events (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    entity_id UUID NOT NULL,
    feature_vector JSONB NOT NULL,
    alert_level TEXT NOT NULL,
    predicted_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expected_by TIMESTAMPTZ NOT NULL,
    actual_outcome_within_30d BOOLEAN,
    outcome_source TEXT,
    outcome_reference_id UUID,
    resolved_at TIMESTAMPTZ,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_stats_alert_calibration_expected_by
    ON stats_alert_calibration_events (expected_by, actual_outcome_within_30d);

CREATE INDEX IF NOT EXISTS idx_stats_alert_calibration_entity_predicted
    ON stats_alert_calibration_events (entity_id, predicted_at DESC);
