-- ──────────────────────────────────────────────────────────────────────────────
-- Migration 061: frozen evaluation examples (P0 audit #6A)
--
-- `learning_eval_sets` (054) only declared `example_count`: the "frozen" set had
-- no actual, immutable examples behind it, so a run could be evaluated against
-- whatever data happened to be around. This migration makes the freeze real:
--
--   learning_eval_examples — the immutable examples of a frozen set. Each row
--                            carries the input/expected payloads, provenance and
--                            a content hash computed by the database from the
--                            canonical payload text. UPDATE and DELETE are
--                            rejected by the shared freeze guard (the same
--                            function that freezes `learning_eval_sets`).
--   learning_eval_sets.examples_digest
--                          — sha256 over the sorted `content_hash` set, kept in
--                            sync by `learning_eval_set_finalize_examples()`.
--                            The freeze guard only ever admits an update that
--                            writes the correctly recomputed digest; every other
--                            column stays immutable.
--   learning_eval_runs      — additionally records the digest of the frozen set
--                            it used plus the candidate/baseline artifact hash
--                            and version. A BEFORE INSERT trigger recomputes the
--                            example count and digest and refuses (raises) any
--                            run whose set does not match `example_count` and
--                            `examples_digest` exactly.
--
-- Idempotent: safe to re-run (IF NOT EXISTS / CREATE OR REPLACE / guarded
-- triggers and grants).
-- ──────────────────────────────────────────────────────────────────────────────

-- ── Immutable examples ────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS learning_eval_examples (
    id               UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    eval_set_id      UUID        NOT NULL REFERENCES learning_eval_sets(id) ON DELETE RESTRICT,
    example_key      TEXT        NOT NULL CHECK (length(btrim(example_key)) > 0),
    input_payload    JSONB       NOT NULL,
    expected_payload JSONB       NOT NULL,
    provenance       JSONB       NOT NULL DEFAULT '{}'::jsonb,
    content_hash     TEXT        NOT NULL,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT learning_eval_examples_set_key_key UNIQUE (eval_set_id, example_key),
    CONSTRAINT learning_eval_examples_set_hash_key UNIQUE (eval_set_id, content_hash)
);

COMMENT ON TABLE learning_eval_examples IS
    'Immutable examples of a frozen learning_eval_set. UPDATE/DELETE rejected by learning_eval_sets_freeze_guard().';
COMMENT ON COLUMN learning_eval_examples.content_hash IS
    'sha256 of the canonical payload text; computed by trigger, never caller-supplied.';

-- The content hash is derived from the payloads by the database so a caller can
-- neither forge it nor freeze an example whose hash does not describe its row.
CREATE OR REPLACE FUNCTION learning_eval_example_content_hash(
    p_input    JSONB,
    p_expected JSONB,
    p_provenance JSONB
) RETURNS TEXT AS $$
    SELECT encode(
        sha256(convert_to(
            COALESCE(p_input::text, '') || E'\n' ||
            COALESCE(p_expected::text, '') || E'\n' ||
            COALESCE(p_provenance::text, ''),
            'UTF8'
        )),
        'hex'
    );
$$ LANGUAGE sql IMMUTABLE;

CREATE OR REPLACE FUNCTION learning_eval_examples_set_content_hash()
RETURNS trigger AS $$
BEGIN
    NEW.content_hash := learning_eval_example_content_hash(
        NEW.input_payload, NEW.expected_payload, NEW.provenance);
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_learning_eval_examples_content_hash ON learning_eval_examples;
CREATE TRIGGER trg_learning_eval_examples_content_hash
    BEFORE INSERT ON learning_eval_examples
    FOR EACH ROW EXECUTE FUNCTION learning_eval_examples_set_content_hash();

-- The examples of a set are append-only: creation may add them, nothing may
-- mutate or remove them afterwards.
DROP TRIGGER IF EXISTS trg_learning_eval_examples_freeze ON learning_eval_examples;
CREATE TRIGGER trg_learning_eval_examples_freeze
    BEFORE UPDATE OR DELETE ON learning_eval_examples
    FOR EACH ROW EXECUTE FUNCTION learning_eval_sets_freeze_guard();

CREATE INDEX IF NOT EXISTS idx_learning_eval_examples_set
    ON learning_eval_examples (eval_set_id, example_key);

-- ── Frozen-set digest ─────────────────────────────────────────────────────────
-- Digest over the sorted content-hash set. Empty sets hash the empty string so
-- legacy sets created before this migration carry a well-defined digest.
CREATE OR REPLACE FUNCTION learning_eval_examples_digest(p_eval_set_id UUID)
RETURNS TEXT AS $$
    SELECT encode(
        sha256(convert_to(
            COALESCE(string_agg(content_hash, E'\n' ORDER BY content_hash), ''),
            'UTF8'
        )),
        'hex'
    )
    FROM learning_eval_examples
    WHERE eval_set_id = p_eval_set_id;
$$ LANGUAGE sql STABLE;

ALTER TABLE learning_eval_sets
    ADD COLUMN IF NOT EXISTS examples_digest TEXT NOT NULL
        DEFAULT encode(sha256(convert_to('', 'UTF8')), 'hex');

COMMENT ON COLUMN learning_eval_sets.examples_digest IS
    'sha256 over the sorted learning_eval_examples.content_hash set; finalize via learning_eval_set_finalize_examples().';

-- Finalize (or refresh) the digest of a set after its examples were inserted.
-- Refuses when the stored example count does not match the declared
-- example_count; otherwise the digest is the exact recomputation over the
-- frozen examples.
CREATE OR REPLACE FUNCTION learning_eval_set_finalize_examples(p_eval_set_id UUID)
RETURNS TEXT AS $$
DECLARE
    v_expected INTEGER;
    v_stored   INTEGER;
    v_digest   TEXT;
BEGIN
    SELECT example_count INTO v_expected
    FROM learning_eval_sets
    WHERE id = p_eval_set_id
    FOR UPDATE;

    IF NOT FOUND THEN
        RAISE EXCEPTION 'learning_eval_set % does not exist', p_eval_set_id;
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

    -- Idempotent: a second finalize must not trip the freeze guard with a
    -- no-op UPDATE.
    UPDATE learning_eval_sets
    SET examples_digest = v_digest
    WHERE id = p_eval_set_id
      AND examples_digest IS DISTINCT FROM v_digest;

    RETURN v_digest;
END;
$$ LANGUAGE plpgsql;

-- The freeze guard admits exactly one mutation: writing the correctly
-- recomputed digest onto a set. Everything else (and every UPDATE/DELETE on
-- learning_eval_examples, which shares this guard) is rejected.
CREATE OR REPLACE FUNCTION learning_eval_sets_freeze_guard()
RETURNS trigger AS $$
BEGIN
    -- `NEW`/`OLD` only carry the columns of the triggering table, so the
    -- digest-finalization branch must be entered before any digest field is
    -- referenced (learning_eval_examples has no examples_digest column).
    IF TG_TABLE_NAME = 'learning_eval_sets' AND TG_OP = 'UPDATE' THEN
        IF NEW.examples_digest IS DISTINCT FROM OLD.examples_digest
           AND NEW.examples_digest = learning_eval_examples_digest(OLD.id)
           AND (to_jsonb(OLD) - 'examples_digest') = (to_jsonb(NEW) - 'examples_digest')
        THEN
            RETURN NEW;
        END IF;
    END IF;

    RAISE EXCEPTION
        '% rows are frozen; create a new version instead', TG_TABLE_NAME;
END;
$$ LANGUAGE plpgsql;

-- ── Evaluation runs record the frozen artefacts they used ─────────────────────
ALTER TABLE learning_eval_runs
    ADD COLUMN IF NOT EXISTS eval_set_digest        TEXT NOT NULL DEFAULT '',
    ADD COLUMN IF NOT EXISTS candidate_artifact_hash TEXT,
    ADD COLUMN IF NOT EXISTS baseline_artifact_hash  TEXT,
    ADD COLUMN IF NOT EXISTS baseline_artifact_version TEXT;

COMMENT ON COLUMN learning_eval_runs.eval_set_digest IS
    'Digest of the frozen example set this run was measured against; must match learning_eval_sets.examples_digest.';
COMMENT ON COLUMN learning_eval_runs.candidate_artifact_hash IS
    'Content hash of the exact candidate artifact (prompt/model/rule/...) that was evaluated.';
COMMENT ON COLUMN learning_eval_runs.baseline_artifact_hash IS
    'Content hash of the baseline artifact the candidate was compared against (with baseline_run_id).';
COMMENT ON COLUMN learning_eval_runs.baseline_artifact_version IS
    'Version label of the baseline artifact the candidate was compared against.';

-- Structurally refuse to execute a run against an unverifiable frozen set:
-- the stored example count must equal `example_count`, the digest recomputed
-- from the stored content hashes must equal `examples_digest`, and the run must
-- record that same digest.
CREATE OR REPLACE FUNCTION learning_eval_runs_verify_frozen_set()
RETURNS trigger AS $$
DECLARE
    v_expected   INTEGER;
    v_digest     TEXT;
    v_stored     INTEGER;
    v_recomputed TEXT;
BEGIN
    SELECT example_count, examples_digest
    INTO v_expected, v_digest
    FROM learning_eval_sets
    WHERE id = NEW.eval_set_id;

    IF NOT FOUND THEN
        RAISE EXCEPTION
            'evaluation refused: learning_eval_set % does not exist',
            NEW.eval_set_id;
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

DROP TRIGGER IF EXISTS trg_learning_eval_runs_verify_frozen_set ON learning_eval_runs;
CREATE TRIGGER trg_learning_eval_runs_verify_frozen_set
    BEFORE INSERT ON learning_eval_runs
    FOR EACH ROW EXECUTE FUNCTION learning_eval_runs_verify_frozen_set();

-- ── Grants for the application role (same pattern as 049/050/054) ─────────────
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'apexintel') THEN
        GRANT SELECT, INSERT, UPDATE, DELETE ON learning_eval_examples TO apexintel;
        GRANT EXECUTE ON FUNCTION learning_eval_examples_digest(UUID) TO apexintel;
        GRANT EXECUTE ON FUNCTION learning_eval_set_finalize_examples(UUID) TO apexintel;
    END IF;
END $$;
