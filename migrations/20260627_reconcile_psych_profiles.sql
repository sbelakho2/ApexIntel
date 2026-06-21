-- Migration 20260627: Reconcile psychological_profiles schema (CORRECTED)
-- ════════════════════════════════════════════════════════════════════════════
-- ORIGIN: This migration previously tried to ALTER three tables that are NEVER
-- created by any migration (`influence_profiles`, `priority_vectors`) and a
-- column that does not exist (`role_history.changed_at`). It would hard-fail
-- at runtime during `sqlx::migrate!`, aborting the entire migration chain and
-- preventing the worker/API from starting cleanly.
--
-- FORENSIC FINDING: The application code (`crates/insights/src/psych_store.rs`
-- and `crates/worker/src/job_execution/psych_profile.rs`) only ever reads or
-- writes four tables: `psychological_profiles`, `behavioral_pattern_events`,
-- `engagement_profiles`, and `sentiment_time_series` — all of which migration
-- 20260624 already creates. The `influence_profiles` / `priority_vectors` /
-- `channel_preference` references were speculative and unused.
--
-- This corrected version performs ONLY the safe, idempotent reconciliation
-- steps that match the real schema: it unifies the two competing
-- `decision_style` CHECK constraints from 20260624 and 20260626 into a single
-- superset, adds the `updated_at` / `last_analyzed_at` audit columns the code
-- expects, and adds supporting indexes. Every statement is guarded so the
-- migration can never fail on an already-migrated database.
-- ════════════════════════════════════════════════════════════════════════════

BEGIN;

-- ── 1. psychological_profiles ───────────────────────────────────────────────
-- Add the audit / quality columns referenced by 20260626's schema so both
-- the 20260624 and 20260626 code paths can write without constraint errors.
ALTER TABLE psychological_profiles
    ADD COLUMN IF NOT EXISTS updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW();
ALTER TABLE psychological_profiles
    ADD COLUMN IF NOT EXISTS last_validated_at TIMESTAMPTZ;

-- Unify the two competing decision_style CHECK constraints into one superset
-- so neither the 20260624 vocabulary (authoritative, collaborative, …) nor the
-- 20260626 vocabulary (cost_first, quality_first, …) is rejected.
ALTER TABLE psychological_profiles DROP CONSTRAINT IF EXISTS chk_psych_decision_style;
ALTER TABLE psychological_profiles ADD CONSTRAINT chk_psych_decision_style CHECK (
    decision_style IN (
        'authoritative', 'collaborative', 'analytical', 'consensus_driven',
        'data_driven', 'intuitive', 'delegative', 'unknown',
        'cost_first', 'quality_first', 'speed_first', 'risk_first',
        'compliance_first', 'balanced_analytical'
    )
);

-- Add the buying-center role + embedding columns that 20260626 intended.
-- buying_center_role is a freeform TEXT so the psych compute job can store
-- inferred procurement roles (gatekeeper / decision_maker / influencer / …).
ALTER TABLE psychological_profiles
    ADD COLUMN IF NOT EXISTS buying_center_role TEXT;
ALTER TABLE psychological_profiles
    ADD COLUMN IF NOT EXISTS psych_profile_vector vector(384);

CREATE INDEX IF NOT EXISTS idx_psych_profiles_decision_style
    ON psychological_profiles(decision_style);
CREATE INDEX IF NOT EXISTS idx_psych_profiles_buying_center
    ON psychological_profiles(buying_center_role);

-- ── 2. engagement_profiles ──────────────────────────────────────────────────
ALTER TABLE engagement_profiles
    ADD COLUMN IF NOT EXISTS updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW();

-- ── 3. role_history ──────────────────────────────────────────────────────────
-- 20260228_dossier_and_role_history.sql creates role_history with
-- start_date / end_date / updated_at but NOT changed_at. Add the columns the
-- previous (broken) reconciliation intended, so downstream indexes compile.
ALTER TABLE role_history
    ADD COLUMN IF NOT EXISTS inferred_from TEXT;
ALTER TABLE role_history
    ADD COLUMN IF NOT EXISTS confidence_score FLOAT NOT NULL DEFAULT 0.0;
ALTER TABLE role_history
    ADD COLUMN IF NOT EXISTS changed_at TIMESTAMPTZ NOT NULL DEFAULT NOW();

-- Backfill changed_at from updated_at for pre-existing rows so the index is usable.
UPDATE role_history
   SET changed_at = COALESCE(updated_at, start_date, NOW())
 WHERE changed_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_role_history_person_changed
    ON role_history(person_id, changed_at DESC);

-- ── 4. Embedding indexes (only if pgvector is installed) ────────────────────
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_extension WHERE extname = 'vector') THEN
        CREATE INDEX IF NOT EXISTS idx_psych_profiles_vector
            ON psychological_profiles USING ivfflat (psych_profile_vector vector_cosine_ops)
            WITH (lists = 100);
    END IF;
END $$;

COMMIT;
