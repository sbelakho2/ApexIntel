-- ──────────────────────────────────────────────────────────────────────────────
-- Migration 062: training truth is explicit opt-in (P0 audit #6B)
--
-- 054 enforced `is_training_truth = (signal_class = 'positive_confirmation')`,
-- which auto-promoted every positive confirmation into training truth and made
-- it impossible to record a confirmation that is *not* truth. The intended
-- semantics are an implication: only a positive confirmation MAY be training
-- truth; the producer must opt in explicitly.
--
-- After this migration:
--   * is_training_truth = TRUE requires signal_class = 'positive_confirmation'
--   * signal_class = 'positive_confirmation' does NOT imply truth
--     (the column default stays FALSE)
--
-- Idempotent: drops the old constraint if present and re-adds the implication.
-- ──────────────────────────────────────────────────────────────────────────────

ALTER TABLE learning_eval_metrics
    DROP CONSTRAINT IF EXISTS learning_eval_metrics_truth_class_check;

ALTER TABLE learning_eval_metrics
    ADD CONSTRAINT learning_eval_metrics_truth_class_check
        CHECK (NOT is_training_truth OR signal_class = 'positive_confirmation');

COMMENT ON COLUMN learning_eval_metrics.is_training_truth IS
    'Explicit opt-in that this metric may be used as training truth. Only positive_confirmation rows may set it; a positive confirmation is not automatically truth (default FALSE).';

COMMENT ON VIEW learning_training_truth_metrics IS
    'learning_eval_metrics rows explicitly opted in as training truth; only positive_confirmation rows may carry that opt-in.';
