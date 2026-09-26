-- ════════════════════════════════════════════════════════════════════════════
-- Migration 069: structurally frozen evaluation sets + run artifact invariants
--
-- Audit #11 (a frozen set must be structurally frozen) and #12 (artifact
-- invariants at the database level).
--
-- The 054/063 freeze was only nominal: `learning_eval_examples` could be
-- inserted into a set at any time, "frozen" was not a state the database knew,
-- and a run could record a NULL candidate artifact hash or a baseline hash
-- without a baseline run.
--
-- After this migration:
--
--   learning_eval_sets.state           building | frozen | superseded.
--   learning_eval_sets.finalized_at    set exactly when the set freezes.
--   learning_eval_set_finalize_examples() locks the set, verifies the stored
--                                      example count against `example_count`,
--                                      recomputes the digest over the stored
--                                      content hashes and only then stamps
--                                      `state = 'frozen'` + `finalized_at`.
--                                      create_frozen_learning_eval_set() runs
--                                      this in the same transaction that
--                                      creates the set and its examples:
--                                      create building -> insert examples ->
--                                      compute digest -> verify count -> freeze.
--   learning_eval_examples INSERT      rejected unless the owning set is
--                                      `building`; a frozen/superseded set can
--                                      never grow examples.
--   learning_eval_runs INSERT          rejected unless the set is `frozen`
--                                      with `finalized_at` set and the stored
--                                      count/digest verify exactly.
--   candidate_artifact_hash            NOT NULL. Runs with no hash abort the
--                                      migration loudly: a hash cannot be
--                                      invented, so an operator must repair.
--   baseline_*                         either all three columns are NULL, or
--                                      baseline_run_id and
--                                      baseline_artifact_hash are both set.
--                                      A dangling version label without a
--                                      baseline reference is cleared; a
--                                      hash/run mismatch aborts loudly.
--
-- Legacy rows: a set whose stored examples already verify against
-- `example_count`/`examples_digest` is marked frozen (it was structurally
-- complete); an incomplete legacy set stays `building` so the incremental API
-- can finish it. `frozen` / `superseded` rows are immutable forever.
--
-- Idempotent: guarded DDL, CREATE OR REPLACE, structural no-op re-runs.
-- ════════════════════════════════════════════════════════════════════════════

BEGIN;

-- ── 1. Freeze state on learning_eval_sets ───────────────────────────────────
ALTER TABLE learning_eval_sets
    ADD COLUMN IF NOT EXISTS state TEXT NOT NULL DEFAULT 'building';
ALTER TABLE learning_eval_sets
    ADD COLUMN IF NOT EXISTS finalized_at TIMESTAMPTZ;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'learning_eval_sets_state_check'
          AND conrelid = to_regclass('public.learning_eval_sets')
    ) THEN
        EXECUTE 'ALTER TABLE learning_eval_sets
                   ADD CONSTRAINT learning_eval_sets_state_check
                   CHECK (state IN (''building'', ''frozen'', ''superseded''))';
    END IF;
END $$;

COMMENT ON TABLE learning_eval_sets IS
    'Frozen, versioned evaluation sets. A set is created building, freezes through learning_eval_set_finalize_examples(), and is immutable afterwards.';
COMMENT ON COLUMN learning_eval_sets.state IS
    'building (examples may still be inserted) | frozen (verified, immutable) | superseded (abandoned before freezing, immutable).';
COMMENT ON COLUMN learning_eval_sets.finalized_at IS
    'Set exactly when the set transitioned building -> frozen; NULL while building.';

-- ── 2. Freeze guard ─────────────────────────────────────────────────────────
-- Replaced before the backfill: the 063 guard only tolerated a digest refresh
-- and would reject the state stamp. Rules:
--   * every column except examples_digest/state/finalized_at is immutable;
--   * a building set may refresh its digest only to the exact recomputation
--     over its stored examples, and may freeze only with finalized_at set and
--     a digest + stored count that verify against `example_count`, or be
--     superseded with finalized_at still NULL;
--   * frozen/superseded sets are fully immutable (a digest refresh is not even
--     allowed: that would legitimise out-of-band example tampering);
--   * UPDATE/DELETE on learning_eval_examples still always raises.
CREATE OR REPLACE FUNCTION learning_eval_sets_freeze_guard()
RETURNS trigger AS $$
BEGIN
    IF TG_TABLE_NAME = 'learning_eval_sets' AND TG_OP = 'UPDATE'
       AND (to_jsonb(OLD) - 'examples_digest' - 'state' - 'finalized_at')
           = (to_jsonb(NEW) - 'examples_digest' - 'state' - 'finalized_at')
    THEN
        IF OLD.state = 'building' THEN
            IF NEW.examples_digest IS DISTINCT FROM OLD.examples_digest
               AND NEW.examples_digest IS DISTINCT FROM learning_eval_examples_digest(OLD.id)
            THEN
                RAISE EXCEPTION
                    'learning_eval_set % digest does not match its stored examples',
                    OLD.id;
            END IF;
            IF NEW.state = 'building' THEN
                IF NEW.finalized_at IS NOT DISTINCT FROM OLD.finalized_at THEN
                    RETURN NEW;
                END IF;
            ELSIF NEW.state = 'frozen' THEN
                IF NEW.finalized_at IS NOT NULL
                   AND NEW.examples_digest = learning_eval_examples_digest(OLD.id)
                   AND (SELECT COUNT(*) FROM learning_eval_examples e
                        WHERE e.eval_set_id = OLD.id) = NEW.example_count
                THEN
                    RETURN NEW;
                END IF;
            ELSIF NEW.state = 'superseded' THEN
                IF NEW.finalized_at IS NULL THEN
                    RETURN NEW;
                END IF;
            END IF;
        ELSIF NEW.state = OLD.state
              AND NEW.examples_digest = OLD.examples_digest
              AND NEW.finalized_at IS NOT DISTINCT FROM OLD.finalized_at
        THEN
            RETURN NEW;
        END IF;
    END IF;

    RAISE EXCEPTION
        '% rows are frozen; create a new version instead', TG_TABLE_NAME;
END;
$$ LANGUAGE plpgsql;

-- ── 3. Backfill legacy rows before the new rules are applied ────────────────
-- A legacy set is structurally complete (and was therefore already treated as
-- frozen) exactly when its stored examples verify against its declared count
-- and its digest. Structurally incomplete sets stay building.
UPDATE learning_eval_sets s
SET state = 'frozen',
    finalized_at = COALESCE(s.finalized_at, s.frozen_at, now())
WHERE s.state = 'building'
  AND s.example_count > 0
  AND s.examples_digest = learning_eval_examples_digest(s.id)
  AND (SELECT COUNT(*) FROM learning_eval_examples e WHERE e.eval_set_id = s.id)
      = s.example_count;

-- ── 4. Finalize: verify count, compute digest, freeze ───────────────────────
CREATE OR REPLACE FUNCTION learning_eval_set_finalize_examples(p_eval_set_id UUID)
RETURNS TEXT AS $$
DECLARE
    v_expected   INTEGER;
    v_state      TEXT;
    v_stored     INTEGER;
    v_digest     TEXT;
BEGIN
    SELECT example_count, state
    INTO v_expected, v_state
    FROM learning_eval_sets
    WHERE id = p_eval_set_id
    FOR UPDATE;

    IF NOT FOUND THEN
        RAISE EXCEPTION 'learning_eval_set % does not exist', p_eval_set_id;
    END IF;

    IF v_state = 'superseded' THEN
        RAISE EXCEPTION
            'learning_eval_set % is superseded and cannot be finalized',
            p_eval_set_id;
    END IF;

    SELECT COUNT(*) INTO v_stored
    FROM learning_eval_examples
    WHERE eval_set_id = p_eval_set_id;

    IF v_stored <> v_expected THEN
        RAISE EXCEPTION
            'learning_eval_set % declares % examples but stores %',
            p_eval_set_id, v_expected, v_stored;
    END IF;

    v_digest := learning_eval_examples_digest(p_eval_set_id);

    -- Idempotent: a second finalize of an unchanged frozen set is a no-op and
    -- must not trip the freeze guard.
    UPDATE learning_eval_sets
    SET examples_digest = v_digest,
        state = 'frozen',
        finalized_at = COALESCE(finalized_at, now())
    WHERE id = p_eval_set_id
      AND (examples_digest IS DISTINCT FROM v_digest
           OR state IS DISTINCT FROM 'frozen');

    RETURN v_digest;
END;
$$ LANGUAGE plpgsql;

-- ── 5. Examples may only be inserted while the set is building ──────────────
CREATE OR REPLACE FUNCTION learning_eval_examples_require_building_set()
RETURNS trigger AS $$
DECLARE
    v_state TEXT;
BEGIN
    SELECT state INTO v_state
    FROM learning_eval_sets
    WHERE id = NEW.eval_set_id;

    IF NOT FOUND THEN
        RAISE EXCEPTION
            'learning_eval_set % does not exist', NEW.eval_set_id;
    END IF;

    IF v_state <> 'building' THEN
        RAISE EXCEPTION
            'learning_eval_set % is %; examples may only be inserted while the set is building',
            NEW.eval_set_id, v_state;
    END IF;

    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_learning_eval_examples_require_building ON learning_eval_examples;
CREATE TRIGGER trg_learning_eval_examples_require_building
    BEFORE INSERT ON learning_eval_examples
    FOR EACH ROW EXECUTE FUNCTION learning_eval_examples_require_building_set();

-- ── 6. Runs require a structurally frozen set ───────────────────────────────
CREATE OR REPLACE FUNCTION learning_eval_runs_verify_frozen_set()
RETURNS trigger AS $$
DECLARE
    v_expected    INTEGER;
    v_digest      TEXT;
    v_state       TEXT;
    v_finalized   TIMESTAMPTZ;
    v_stored      INTEGER;
    v_recomputed  TEXT;
BEGIN
    SELECT example_count, examples_digest, state, finalized_at
    INTO v_expected, v_digest, v_state, v_finalized
    FROM learning_eval_sets
    WHERE id = NEW.eval_set_id;

    IF NOT FOUND THEN
        RAISE EXCEPTION
            'evaluation refused: learning_eval_set % does not exist',
            NEW.eval_set_id;
    END IF;

    IF v_state <> 'frozen' OR v_finalized IS NULL THEN
        RAISE EXCEPTION
            'evaluation refused: learning_eval_set % is % (not frozen)',
            NEW.eval_set_id,
            COALESCE(v_state, 'missing');
    END IF;

    IF v_expected <= 0 THEN
        RAISE EXCEPTION
            'evaluation refused: frozen set % declares no examples',
            NEW.eval_set_id;
    END IF;

    SELECT COUNT(*) INTO v_stored
    FROM learning_eval_examples
    WHERE eval_set_id = NEW.eval_set_id;

    IF v_stored <> v_expected THEN
        RAISE EXCEPTION
            'evaluation refused: frozen set % stores % examples but declares %',
            NEW.eval_set_id, v_stored, v_expected;
    END IF;

    v_recomputed := learning_eval_examples_digest(NEW.eval_set_id);

    IF v_recomputed IS DISTINCT FROM v_digest THEN
        RAISE EXCEPTION
            'evaluation refused: frozen set % digest mismatch (recorded %, recomputed %)',
            NEW.eval_set_id, v_digest, v_recomputed;
    END IF;

    IF NEW.eval_set_digest IS DISTINCT FROM v_digest THEN
        RAISE EXCEPTION
            'evaluation refused: run digest % does not match frozen set digest %',
            NEW.eval_set_digest, v_digest;
    END IF;

    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- ── 7. DB-level artifact invariants ─────────────────────────────────────────
-- A candidate hash is mandatory and cannot be invented, so legacy rows without
-- one abort the migration loudly instead of being written off.
DO $$
DECLARE
    v_missing BIGINT;
BEGIN
    SELECT COUNT(*) INTO v_missing
    FROM learning_eval_runs
    WHERE candidate_artifact_hash IS NULL;

    IF v_missing > 0 THEN
        RAISE EXCEPTION
            'migration 069 cannot enforce candidate_artifact_hash NOT NULL: % learning_eval_run row(s) carry no candidate artifact hash; repair them first',
            v_missing;
    END IF;
END $$;

ALTER TABLE learning_eval_runs
    ALTER COLUMN candidate_artifact_hash SET NOT NULL;

-- An un-referenced version label is meaningless provenance (the old store API
-- allowed it); clear it so only real mismatches remain.
UPDATE learning_eval_runs
SET baseline_artifact_version = NULL
WHERE baseline_run_id IS NULL
  AND baseline_artifact_hash IS NULL
  AND baseline_artifact_version IS NOT NULL;

DO $$
DECLARE
    v_bad BIGINT;
BEGIN
    SELECT COUNT(*) INTO v_bad
    FROM learning_eval_runs
    WHERE NOT (
        (baseline_run_id IS NULL
         AND baseline_artifact_hash IS NULL
         AND baseline_artifact_version IS NULL)
        OR (baseline_run_id IS NOT NULL
            AND baseline_artifact_hash IS NOT NULL)
    );

    IF v_bad > 0 THEN
        RAISE EXCEPTION
            'migration 069 cannot enforce the baseline artifact invariant: % learning_eval_run row(s) pair a baseline hash/version with no baseline run (or vice versa); repair them first',
            v_bad;
    END IF;
END $$;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'learning_eval_runs_baseline_artifacts_check'
          AND conrelid = to_regclass('public.learning_eval_runs')
    ) THEN
        EXECUTE 'ALTER TABLE learning_eval_runs
                   ADD CONSTRAINT learning_eval_runs_baseline_artifacts_check
                   CHECK (
                       (baseline_run_id IS NULL
                        AND baseline_artifact_hash IS NULL
                        AND baseline_artifact_version IS NULL)
                       OR (baseline_run_id IS NOT NULL
                           AND baseline_artifact_hash IS NOT NULL)
                   )';
    END IF;
END $$;

COMMENT ON COLUMN learning_eval_runs.candidate_artifact_hash IS
    'Content hash of the exact candidate artifact (prompt/model/rule/...) that was evaluated; NOT NULL enforced by migration 069.';
COMMENT ON COLUMN learning_eval_runs.baseline_artifact_hash IS
    'Content hash of the baseline artifact the candidate was compared against; set exactly when baseline_run_id is set (migration 069).';
COMMENT ON COLUMN learning_eval_runs.baseline_artifact_version IS
    'Version label of the baseline artifact; only meaningful together with baseline_run_id + baseline_artifact_hash (migration 069).';

COMMIT;
