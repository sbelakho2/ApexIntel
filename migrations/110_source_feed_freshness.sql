-- Migration 110: feed freshness on source runtime state
-- ════════════════════════════════════════════════════════════════════════════
-- "No new observations" alone cannot distinguish a healthy quiet feed from a
-- broken ingestion path: KrebsOnSecurity legitimately publishes nothing for a
-- week, while a parser that silently drops every item looks identical from the
-- observation stream. The crawl cycle now records what the *source itself*
-- said at the last successful fetch:
--
--   * last_item_at    — newest published timestamp seen in the parsed feed
--                       (NULL for HTML sources without item dates)
--   * last_item_count — how many items the last successful parse produced
--
-- With these, outage detection becomes honest:
--   - fetch OK + last_item_at recent + nothing ingested  ⇒ ingestion stalled
--     (a real pipeline bug, high severity);
--   - fetch OK + last_item_at old                        ⇒ quiet feed
--     (healthy, no warning);
--   - fetch failing                                      ⇒ transport failure,
--     reported with the actual error instead of "silent for N days".

ALTER TABLE source_runtime_state
    ADD COLUMN IF NOT EXISTS last_item_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS last_item_count INTEGER;

COMMENT ON COLUMN source_runtime_state.last_item_at IS
    'Newest published timestamp seen in the last successful feed parse (NULL for sources without item dates)';
COMMENT ON COLUMN source_runtime_state.last_item_count IS
    'Number of items the last successful parse produced (feed items or extracted page count)';
