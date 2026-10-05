-- Migration 107: indexed person-name normalization and insight count key
-- ════════════════════════════════════════════════════════════════════════════
-- Two measured hot spots:
--
-- 1. Entity matching runs `SELECT EXISTS(SELECT 1 FROM persons WHERE
--    lower(regexp_replace(name, '\s+', ' ', 'g')) = lower($1))` on every
--    candidate name: production executed 21,917 calls at ~16.7 ms mean
--    (~366 s of database time), because the expression cannot use an index.
--    `name_normalized` stores the canonical `lower(regexp_replace(trim(name),
--    '\s+', ' ', 'g'))` and a btree index makes the lookup an index probe.
--    Both existing query variants switch to the column; the trim keeps the
--    stricter semantics (leading/trailing whitespace no longer blocks a
--    match).
--
-- 2. The insights pagination total computes a COUNT(DISTINCT ...) over a
--    three-part key (~96 ms mean). `concat_ws` is only STABLE, so the key is
--    expressed with immutable `||` concatenation and a matching expression
--    index; the query is rewritten to the identical form.

ALTER TABLE persons
    ADD COLUMN IF NOT EXISTS name_normalized text
        GENERATED ALWAYS AS (lower(regexp_replace(trim(name), '\s+', ' ', 'g'))) STORED;

CREATE INDEX IF NOT EXISTS idx_persons_name_normalized
    ON persons (name_normalized);

CREATE INDEX IF NOT EXISTS idx_insights_count_key
    ON insights ((
        lower(trim(title))
        || '|' || lower(coalesce(insight_type, ''))
        || '|' || lower(coalesce(region, ''))
    ));
