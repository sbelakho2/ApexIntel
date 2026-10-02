-- Migration 099: remove seeded buyer role-slot persons
-- ════════════════════════════════════════════════════════════════════════════
-- The 040_seed_buyer_relevant_pois.sql seed inserted placeholder "role slot"
-- persons: each row's name is a job title (usually, but not always, the same
-- string as `current_role`) and carries metadata
-- {"source":"seed_buyer_roles","role_slot":true}. Insight recipes then treated
-- those non-people as buying contacts ("contact the VP Procurement ...").
--
-- This migration removes exactly the never-enriched placeholders:
--
--   metadata->>'source' = 'seed_buyer_roles'
--   AND metadata->>'role_slot' = 'true'
--   AND NOT EXISTS (poi_artifacts for the person) -- never enriched by a crawl
--
-- The two metadata keys are the seed's own markers (set on every one of its
-- rows) and are the only reliable identity signal: two of the seeded rows
-- deliberately spell `name` differently from `current_role`
-- ('SVP Supply Chain' vs 'Senior Vice President, Supply Chain',
-- 'VP Supply Chain & Procurement' vs 'VP Supply Chain and Procurement'), so
-- name/role equality would under-delete. Anything a crawler actually enriched
-- has at least one poi_artifacts row, so artifact-bearing rows are excluded
-- from the candidate set entirely and none of their dependents (annotations,
-- edges, profiles, ...) are touched.
--
-- Dependents are detached first (FK children explicitly, then the UUID/text
-- entity references listed below) and only then is the person deleted, all in
-- the single transaction sqlx opens for this file. The migration is idempotent:
-- on a second run the candidate set is empty and every statement is a no-op.
--
-- Historical records that are not person-owned are deliberately preserved:
-- activity_feed rows, audit_log, analyst_notifications and warnings remain as
-- immutable history even though they may mention a removed title.
--
-- NOTE FOR OPERATORS: insights themselves are kept; only any `entity_ids`
-- entry pointing at a removed placeholder is dropped here. Insight narratives
-- that recommended a role-slot contact (e.g. "contact the VP Procurement at
-- <company>") are now stale and should be regenerated -- or explicitly
-- annotated as historical -- by the normal insight recipes. The worker's
-- company insight prompt already reports "No tracked personnel with
-- buyer-relevant roles yet" once these rows are gone, which is the truthful
-- state until POI discovery finds real named buyers.
--
-- No BEGIN/COMMIT anywhere: sqlx wraps each migration in its own transaction.
-- ════════════════════════════════════════════════════════════════════════════

-- Candidate snapshot. A temp table keeps the predicate in one place so every
-- detach statement and the final delete provably use the same row set. It is
-- dropped again at the end (and defensively at the start, so a manually
-- re-run file with a stale temp table cannot fail).
DROP TABLE IF EXISTS pg_temp._role_slot_person_ids;
CREATE TEMP TABLE _role_slot_person_ids AS
SELECT p.id
FROM persons p
WHERE p.metadata->>'source' = 'seed_buyer_roles'
  AND p.metadata->>'role_slot' = 'true'
  AND NOT EXISTS (SELECT 1 FROM poi_artifacts pa WHERE pa.person_id = p.id);

-- ── 1. FK children (all ON DELETE CASCADE; deleted explicitly so the detach
--      order is documented and reviewable). ─────────────────────────────────
-- poi_artifacts is a no-op by construction: a person with artifacts is not a
-- candidate (it is treated as enriched). It is kept for order completeness.
DELETE FROM poi_artifacts      WHERE person_id IN (SELECT id FROM pg_temp._role_slot_person_ids);
DELETE FROM poi_engagements    WHERE person_id IN (SELECT id FROM pg_temp._role_slot_person_ids);
DELETE FROM role_history       WHERE person_id IN (SELECT id FROM pg_temp._role_slot_person_ids);
DELETE FROM person_changes     WHERE person_id IN (SELECT id FROM pg_temp._role_slot_person_ids);
DELETE FROM contact_methods    WHERE person_id IN (SELECT id FROM pg_temp._role_slot_person_ids);
DELETE FROM engagement_events  WHERE person_id IN (SELECT id FROM pg_temp._role_slot_person_ids);
DELETE FROM buying_center_members WHERE person_id IN (SELECT id FROM pg_temp._role_slot_person_ids);

-- ── 2. Person-owned columns without an FK (person UUID stored as text). ─────
DELETE FROM supplier_contacts
 WHERE person_id IN (SELECT id FROM pg_temp._role_slot_person_ids);
DELETE FROM psychological_profiles
 WHERE person_id IN (SELECT id::text FROM pg_temp._role_slot_person_ids);
DELETE FROM behavioral_pattern_events
 WHERE person_id IN (SELECT id::text FROM pg_temp._role_slot_person_ids);
DELETE FROM engagement_profiles
 WHERE person_id IN (SELECT id::text FROM pg_temp._role_slot_person_ids);

-- ── 3. Entity references to the placeholder (entity_type + entity_id). ──────
DELETE FROM annotations
 WHERE entity_type = 'person'
   AND entity_id IN (SELECT id::text FROM pg_temp._role_slot_person_ids);
DELETE FROM entity_alert_configs
 WHERE entity_id IN (SELECT id::text FROM pg_temp._role_slot_person_ids);
DELETE FROM user_alert_subscriptions
 WHERE entity_id IN (SELECT id FROM pg_temp._role_slot_person_ids);
DELETE FROM graph_edges
 WHERE (source_type = 'person' AND source_id IN (SELECT id FROM pg_temp._role_slot_person_ids))
    OR (target_type = 'person' AND target_id IN (SELECT id FROM pg_temp._role_slot_person_ids));
DELETE FROM observation_entity_graph
 WHERE entity_type = 'person'
   AND entity_id IN (SELECT id FROM pg_temp._role_slot_person_ids);
DELETE FROM insight_firings
 WHERE entity_id IN (SELECT id FROM pg_temp._role_slot_person_ids);
DELETE FROM embeddings
 WHERE entity_type = 'person'
   AND entity_id IN (SELECT id::text FROM pg_temp._role_slot_person_ids);

-- Detach the placeholder from insight links (insights are preserved). The
-- 021-era `sync_insight_entity_ids` trigger mirrors the singular `entity_id`
-- and the `entity_ids` array, so clearing the array alone would be undone while
-- `entity_id` still names the person: clear both, and let the trigger promote
-- another remaining entity when one exists.
--
-- Databases whose `insights` table predates the 021 unify step (the
-- consolidated production lineage) have no singular `entity_id` column at
-- all; guard on its existence so both shapes converge. The guard is also
-- correct for fresh bootstraps, where the column always exists.
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM information_schema.columns
               WHERE table_name = 'insights' AND column_name = 'entity_id') THEN
        UPDATE insights i
           SET entity_ids = array_remove(i.entity_ids, c.id),
               entity_id  = CASE WHEN i.entity_id = c.id THEN NULL ELSE i.entity_id END,
               updated_at = now()
          FROM pg_temp._role_slot_person_ids c
         WHERE i.entity_ids @> ARRAY[c.id]
            OR i.entity_id = c.id;
    ELSE
        UPDATE insights i
           SET entity_ids = array_remove(i.entity_ids, c.id),
               updated_at = now()
          FROM pg_temp._role_slot_person_ids c
         WHERE i.entity_ids @> ARRAY[c.id];
    END IF;
END $$;

-- ── 4. Delete the placeholder person, re-checking the full predicate so an
--      enrichment that appeared after the snapshot still wins. ─────────────
DELETE FROM persons p
 USING pg_temp._role_slot_person_ids c
 WHERE p.id = c.id
   AND p.metadata->>'source' = 'seed_buyer_roles'
   AND p.metadata->>'role_slot' = 'true'
   AND NOT EXISTS (SELECT 1 FROM poi_artifacts pa WHERE pa.person_id = p.id);

DROP TABLE IF EXISTS pg_temp._role_slot_person_ids;

-- Verification (run manually):
--   SELECT count(*) FROM persons
--    WHERE metadata->>'source' = 'seed_buyer_roles'
--      AND metadata->>'role_slot' = 'true'
--      AND NOT EXISTS (SELECT 1 FROM poi_artifacts pa WHERE pa.person_id = persons.id);
--   -- expected 0 after this migration.
