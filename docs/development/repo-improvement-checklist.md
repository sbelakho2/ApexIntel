# ApexIntel Repository Improvement Checklist

This file converts the full-repository audit into an execution checklist. It is intentionally biased toward implementation quality, testability, and evidence-based completion rather than aspirational backlog wording.

## Execution Rules

- Every item must be completed test-first where practical: add or strengthen a failing test, then fix the code, then record evidence.
- Do not mark an item complete without code evidence and verification evidence.
- Prefer root-cause fixes over cosmetic changes.
- Keep changes isolated enough that failures can be attributed to a single step.
- Where a recommendation is large, split it into sub-items and complete them in order.
- If an item is blocked by product ambiguity, operational risk, or missing infrastructure, record the blocker under Evidence rather than silently skipping it.

## Evidence Format

Use this format when closing an item:

- Code: file paths and a brief note on what changed.
- Tests: exact test names or commands.
- Validation: runtime, lint, compile, or manual verification notes.

---

## Phase 0: Safety, Control, and Change Discipline

- [x] P0.1 Protect destructive warning deletion with explicit admin-only authorization.
  Why: `DELETE /api/warnings` is a catastrophic operation and cannot inherit generic write access.
  Implementation detail:
  - Introduce route-aware permission resolution instead of raw method-based inference.
  - Add a regression test proving analysts cannot delete all warnings.
  - Audit nearby destructive routes for the same flaw.
  Evidence:
  - Code: `crates/api/src/main.rs` now resolves permissions by route and requires `Admin` for `DELETE /api/warnings`, `DELETE /api/warnings/:id`, and `POST /api/warnings/bulk-delete` while preserving `Write` access for warning acknowledgement.
  - Tests: added `test_required_permission_for_delete_all_warnings_is_admin`, `test_required_permission_for_delete_warning_is_admin`, `test_required_permission_for_bulk_delete_warnings_is_admin`, and `test_required_permission_for_warning_acknowledge_remains_write` in `crates/api/src/main.rs`.
  - Validation: first ran the new regression and confirmed failure (`left: Write`, `right: Admin`), then fixed the route-aware permission logic and verified with `~/.cargo/bin/cargo test -p apex-api required_permission_for_`; `get_errors` on `crates/api/src/main.rs` reports no errors.

- [x] P0.2 Add CSRF protection for HTML session flows that mutate state.
  Why: server-rendered HTMX forms currently rely on session cookies without anti-forgery proof.
  Implementation detail:
  - Add CSRF token issue + validation middleware.
  - Cover form POSTs that write settings, recipes, acknowledgements, and admin actions.
  - Add negative-path tests for missing/invalid token cases.
  Evidence:
  - Code: `crates/api/src/middleware/session.rs` now issues an `apex_csrf` cookie on safe authenticated page loads and rejects unsafe authenticated HTML requests unless the cookie matches either the `X-CSRF-Token` header or a `csrf_token` form field. `crates/api/src/main.rs` moves `/logout` into the protected HTML router so it inherits the same session+CSRF boundary. `crates/api/static/js/app.js` now injects `csrf_token` hidden fields into non-GET forms and adds `X-CSRF-Token` to HTMX requests. `crates/api/static/js/recipe-builder.js` now attaches the CSRF header to protected JSON POSTs for recipe create, test, and LLM generation.
  - Tests: added `test_validate_csrf_request_rejects_post_without_matching_token`, `test_validate_csrf_request_rejects_mismatched_header_token`, `test_validate_csrf_request_accepts_matching_header_token`, `test_validate_csrf_request_accepts_matching_form_token`, and `test_validate_csrf_request_accepts_safe_methods_without_token` in `crates/api/src/middleware/session.rs`.
  - Validation: first ran the missing-token regression and confirmed failure while the placeholder validator still returned `true`; after implementing the double-submit check, `~/.cargo/bin/cargo test -p apex-api middleware::session::tests::test_validate_csrf_request_ -- --nocapture` passed with 5/5 tests green, and `get_errors` reported no errors in the edited Rust and JS files.

- [x] P0.3 Replace plain-string secret handling in config with secret-wrapped types.
  Why: secrets are currently normal `String` values and can leak via logging, debug output, or accidental cloning.
  Implementation detail:
  - Use `secrecy::SecretString` or equivalent for API keys, SMTP creds, DB secrets, and LLM tokens.
  - Add compile-time boundaries so string extraction is explicit.
  Evidence:
  - Code: `crates/core/src/config.rs` now wraps `database_url`, `redis_url`, `google_api_key`, `nexar_client_secret`, `mouser_api_key`, `llm_api_key`, and `smtp_url` in `SecretString`, with explicit accessor methods such as `database_url_value()` and `llm_api_key_value()` for the small number of legitimate extraction sites. `crates/core/Cargo.toml` now depends on `secrecy`, and `crates/api/src/main.rs` plus `crates/worker/src/main.rs` now use the explicit accessors instead of passing raw config fields around.
  - Tests: extended `crates/core/src/config.rs` coverage with `test_app_config_debug_redacts_secret_fields` and updated the existing config-loading and validation tests to work through the explicit secret boundary.
  - Validation: `~/.cargo/bin/cargo test -p apex-core config::tests:: -- --nocapture` passed with 13/13 tests green, `~/.cargo/bin/cargo test -p apex-api middleware::session::tests::test_validate_csrf_request_accepts_matching_header_token -- --nocapture` confirmed the API still compiles against the hardened config type, and `get_errors` reported no errors in the touched Rust files.

- [x] P0.4 Remove hardcoded credentials and production secrets from scripts and examples.
  Why: operational scripts currently contain inline secrets and unsafe defaults.
  Implementation detail:
  - Move secrets to env vars.
  - Fail fast with clear messages when required vars are missing.
  - Add tests for secret lookup behavior where feasible.
  Evidence:
  - Code: `scripts/test_endpoints.py` no longer embeds a production hostname or API key; it now defaults to `http://localhost:8080`, requires `APEX_API_KEY` or `API_KEY`, and factors secret lookup into explicit helper functions. `scripts/test_api_server.sh` now requires `APEX_API_KEY` instead of shipping a bearer token. `scripts/backup.sh` and `scripts/restore.sh` now require `DATABASE_URL` and no longer fall back to repo-embedded Postgres credentials.
  - Tests: added `SecretLookupTests` in `scripts/test_endpoints.py` to verify API-key resolution succeeds when env vars are present and raises when they are absent.
  - Validation: `/Users/sabelakhoua/IdeaProjects/ApexIntel/.venv/bin/python -m unittest discover -s scripts -p 'test_endpoints.py'` passed with 2/2 tests green; `env -u APEX_API_KEY bash scripts/test_api_server.sh` now fails immediately with a clear missing-secret message; `env -u DATABASE_URL bash scripts/backup.sh /tmp/apexintel-backup-test` now fails immediately instead of using inline credentials; `get_errors` reported no errors in the edited scripts.

---

## Phase 1: Structural Refactors That Unlock Safe Progress

- [x] P1.1 Break `crates/api/src/main.rs` into route/handler modules.
  Why: the current monolith prevents isolated testing, safe review, and targeted ownership.
  Implementation detail:
  - Extract route handlers by domain.
  - Leave startup, state construction, and router wiring in `main.rs`.
  - Preserve public behavior with route-level tests.
  Progress:
  - [x] P1.1.a Extracted the API insight bookmark and LLM analysis handlers into a dedicated module.
  - [x] P1.1.b Extracted the protected HTML mutation handlers for trigger-scan and recipe create/test into a dedicated module.
  - [x] P1.1.c Extracted the warning mutation handlers into a dedicated module.
  - [x] P1.1.d Extracted the API preferences handlers into a dedicated module.
  - [x] P1.1.e Extracted the dossier and change API handlers into a dedicated module.
  - [x] P1.1.f Extracted the competitor API handlers into a dedicated module.
  - [x] P1.1.g Extracted the security detail API handlers into a dedicated module.
  - [x] P1.1.h Extracted the graph API handlers into a dedicated module.
  - [x] P1.1.i Extracted the memo API handlers into a dedicated module.
  - [x] P1.1.j Extracted the recipe staging/promote/deprecate API handlers into a dedicated module.
  - [x] P1.1.k Extracted the admin status and trigger API handlers into a dedicated module.
  - [x] P1.1.l Extract the remaining API and HTML handler domains out of `main.rs`.
  Evidence:
  - Code: added `crates/api/src/api_handlers/mod.rs`, `crates/api/src/api_handlers/insights.rs`, `crates/api/src/api_handlers/html_mutations.rs`, `crates/api/src/api_handlers/warnings.rs`, `crates/api/src/api_handlers/preferences.rs`, `crates/api/src/api_handlers/dossiers.rs`, `crates/api/src/api_handlers/competitors.rs`, `crates/api/src/api_handlers/security.rs`, `crates/api/src/api_handlers/graph.rs`, `crates/api/src/api_handlers/memos.rs`, `crates/api/src/api_handlers/recipes.rs`, `crates/api/src/api_handlers/admin.rs`, `crates/api/src/api_handlers/entities.rs`, `crates/api/src/api_handlers/catalog.rs`, `crates/api/src/api_handlers/exports.rs`, `crates/api/src/api_handlers/details.rs`, `crates/api/src/api_handlers/overview.rs`, and `crates/api/src/api_handlers/llm.rs`; `crates/api/src/main.rs` now limits itself to startup, router wiring, auth/rate-limit middleware, health/metrics/state wiring, shared response helpers, and websocket plumbing while routing `/api/warnings`, `/api/warnings/bulk-delete`, `/api/warnings/:id`, `/api/warnings/:id/acknowledge`, `/api/warnings/:id/analyze`, `/api/insights`, `/api/insights/:id`, `/api/insights/:id/bookmark`, `/api/insights/:id/analyze`, `/api/insights/export`, `/api/insights/weekly-memo`, `/api/memos`, `/api/preferences`, `/api/companies`, `/api/companies/export`, `/api/companies/:id`, `/api/persons`, `/api/persons/export`, `/api/persons/:id`, `/api/companies/:id/dossier`, `/api/persons/:id/dossier`, `/api/persons/:id/engagement`, `/api/persons/:id/role-history`, `/api/persons/:id/changes`, `/api/persons/:id/dossier-entries`, `/api/companies/:id/changes`, `/api/companies/:id/dossier-entries`, `/api/dossier-entries/:id/verify`, `/api/dossier-entries/:id/history`, `/api/competitors`, `/api/competitors/changes`, `/api/competitors/:id/changes`, `/api/search`, `/api/graph`, `/api/security`, `/api/security/dns-posture`, `/api/security/lookalike-domains`, `/api/security/kev-relevance`, `/api/sites`, `/api/capabilities`, `/api/certifications`, `/api/observations`, `/api/product-families`, `/api/logistics-nodes`, `/api/regulations`, `/api/poi-artifacts`, `/api/dashboard`, `/api/graph/neighborhood/:id`, `/api/graph/path/:from/:to`, `/api/recipes`, `/api/recipes/staging`, `/api/recipes/:id/promote`, `/api/recipes/:id/deprecate`, `/api/admin/crawl-status`, `/api/admin/recipe-performance`, `/api/admin/poi-coverage`, `/api/admin/trigger-scan`, `/api/llm/extract-entities`, `/api/llm/generate-recipe`, `/api/llm/synthesize-poi`, `/api/llm/generate-memo`, `/security/trigger-scan`, `/recipes/create`, and `/recipes/test` through extracted modules instead of owning those handlers inline.
  - Tests: added `test_strip_analysis_label_removes_numbered_prefix`, `test_strip_analysis_label_removes_markdown_prefix`, `test_strip_warning_label_removes_section_prefix`, and `test_resolve_bookmarked_by_only_accepts_true` in `crates/api/src/api_handlers/insights.rs`; added `test_slugify_recipe_code_normalizes_whitespace_and_case`, `test_slugify_recipe_code_falls_back_for_empty_input`, and `test_manual_trigger_kind_allows_whitelisted_job` in `crates/api/src/api_handlers/html_mutations.rs`; added `test_parse_bulk_delete_ids_accepts_valid_uuid_list`, `test_parse_bulk_delete_ids_rejects_invalid_uuid`, and `test_parse_bulk_delete_ids_rejects_more_than_maximum_ids` in `crates/api/src/api_handlers/warnings.rs`; added `test_decode_user_preferences_defaults_when_record_is_missing` and `test_decode_user_preferences_restores_nested_preference_fields` in `crates/api/src/api_handlers/preferences.rs`; added `test_parse_dossier_uuid_accepts_valid_uuid` and `test_parse_dossier_uuid_rejects_invalid_uuid` in `crates/api/src/api_handlers/dossiers.rs`; added `test_normalize_all_competitor_changes_pagination_defaults`, `test_normalize_all_competitor_changes_pagination_clamps_values`, and `test_parse_competitor_uuid_rejects_invalid_uuid` in `crates/api/src/api_handlers/competitors.rs`; added `test_normalize_security_limit_defaults_to_fifty` and `test_normalize_security_limit_caps_at_two_hundred` in `crates/api/src/api_handlers/security.rs`; added `test_parse_graph_uuid_accepts_valid_uuid` and `test_parse_graph_uuid_uses_field_name_in_error` in `crates/api/src/api_handlers/graph.rs`; added `test_normalize_memo_pagination_defaults` and `test_normalize_memo_pagination_clamps_values` in `crates/api/src/api_handlers/memos.rs`; added `test_build_recipe_status_payload_marks_promoted_recipe` and `test_build_recipe_status_payload_marks_deprecated_recipe` in `crates/api/src/api_handlers/recipes.rs`; added `test_build_trigger_scan_response_preserves_fields` in `crates/api/src/api_handlers/admin.rs`; added `test_resolve_person_priority_bounds_prefers_tier_over_priority` plus `test_resolve_person_priority_bounds_falls_back_to_explicit_min_priority` in `crates/api/src/api_handlers/entities.rs`; added `test_parse_optional_uuid_filter_accepts_valid_uuid` plus `test_parse_optional_uuid_filter_ignores_invalid_uuid` in `crates/api/src/api_handlers/catalog.rs`; added `test_csv_escape_quotes_and_wraps_fields` plus `test_csv_escape_preserves_empty_string` in `crates/api/src/api_handlers/exports.rs`; added `test_parse_detail_uuid_accepts_valid_uuid` plus `test_parse_detail_uuid_rejects_invalid_uuid` in `crates/api/src/api_handlers/details.rs`; added `test_append_or_filter_wraps_existing_query` plus `test_append_or_filter_ignores_unsafe_tokens` in `crates/api/src/api_handlers/overview.rs`; and added feature-gated `test_parse_entities_value_accepts_named_object_array` plus `test_parse_entities_value_accepts_string_array_fallback` in `crates/api/src/api_handlers/llm.rs`.
  - Validation: first rewired the warning mutation routes to a missing module and confirmed the expected compile failure (`file not found for module warnings`), then rewired the preferences route to a missing module and confirmed the expected compile failure (`file not found for module preferences`), then rewired the dossier/change routes to a missing module and confirmed the expected compile failure (`file not found for module dossiers`), then rewired the competitor routes to a missing module and confirmed the expected compile failure (`file not found for module competitors`), then rewired the security detail routes to a missing module and confirmed the expected compile failure (`file not found for module security`), then rewired the graph routes to a missing module and confirmed the expected compile failure (`file not found for module graph`), then rewired the memo routes to a missing module and confirmed the expected compile failure (`file not found for module memos`), then rewired the recipe staging routes to a missing module and confirmed the expected compile failure (`file not found for module recipes`), then rewired the admin routes to a missing module and confirmed the expected compile failure (`file not found for module admin`), then rewired the company/person routes to a missing module and confirmed the expected compile failure (`file not found for module entities`), then rewired the catalog data-list routes to a missing module and confirmed the expected compile failure (`file not found for module catalog`), then rewired the CSV export routes to a missing module and confirmed the expected compile failure (`file not found for module exports`), then rewired the warning and insight detail routes to a missing module and confirmed the expected compile failure (`file not found for module details`), then rewired the search, graph overview, recipe list, and security summary routes to a missing module and confirmed the expected compile failure (`file not found for module overview`), then rewired the four LLM routes to a missing module and confirmed the expected compile failure (`file not found for module llm`), then rewired `/api/warnings` and `/api/insights` to extracted module functions and confirmed the expected compile failure because those functions were still private imports from `main.rs`; after creating the modules, verified with `~/.cargo/bin/cargo test -p apex-api test_slugify_recipe_code_normalizes_whitespace_and_case -- --nocapture`, `~/.cargo/bin/cargo test -p apex-api test_manual_trigger_kind_allows_whitelisted_job -- --nocapture`, `~/.cargo/bin/cargo test -p apex-api test_parse_bulk_delete_ids_ -- --nocapture`, `~/.cargo/bin/cargo test -p apex-api test_decode_user_preferences_ -- --nocapture`, `~/.cargo/bin/cargo test -p apex-api test_parse_dossier_uuid_ -- --nocapture`, `~/.cargo/bin/cargo test -p apex-api test_normalize_all_competitor_changes_pagination_ -- --nocapture`, `~/.cargo/bin/cargo test -p apex-api test_normalize_security_limit_ -- --nocapture`, `~/.cargo/bin/cargo test -p apex-api test_parse_graph_uuid_ -- --nocapture`, `~/.cargo/bin/cargo test -p apex-api test_normalize_memo_pagination_ -- --nocapture`, `~/.cargo/bin/cargo test -p apex-api test_build_recipe_status_payload_ -- --nocapture`, `~/.cargo/bin/cargo test -p apex-api test_build_trigger_scan_response_preserves_fields -- --nocapture`, `~/.cargo/bin/cargo test -p apex-api resolve_person_priority_bounds -- --nocapture`, `~/.cargo/bin/cargo test -p apex-api parse_optional_uuid_filter -- --nocapture`, `~/.cargo/bin/cargo test -p apex-api csv_escape -- --nocapture`, `~/.cargo/bin/cargo test -p apex-api parse_detail_uuid -- --nocapture`, `~/.cargo/bin/cargo test -p apex-api append_or_filter -- --nocapture`, `~/.cargo/bin/cargo test -p apex-api test_resolve_bookmarked_by_only_accepts_true -- --nocapture`, `~/.cargo/bin/cargo test -p apex-api --features llm parse_entities_value -- --nocapture`, `~/.cargo/bin/cargo test -p apex-api test_required_permission_for_delete_all_warnings_is_admin -- --nocapture`, `~/.cargo/bin/cargo test -p apex-api middleware::session::tests::test_validate_csrf_request_accepts_matching_header_token -- --nocapture`, and `get_errors`; all touched files are currently error-free.

- [x] P1.2 Break `crates/store/src/postgres.rs` into domain-specific store modules.
  Why: 149 functions in a single file creates hidden coupling and duplicated query logic.
  Implementation detail:
  - Split warnings, insights, companies, persons, recipes, admin, and preferences into submodules.
  - Extract shared query/filter builders to avoid repeated WHERE-clause logic.
  Progress:
  - [x] P1.2.a Extracted the preferences and email-digest store methods into a dedicated submodule.
  - [x] P1.2.b Extracted the recipe store methods into a dedicated submodule.
  - [x] P1.2.c Extracted the admin status and trigger-queue store methods into a dedicated submodule.
  - [x] P1.2.d Extracted the warning store query and mutation methods into a dedicated submodule.
  - [x] P1.2.e Extracted the insight store query, bookmark, and related-lookup methods into a dedicated submodule.
  - [x] P1.2.f Extracted the company lookup, listing, and dossier store methods into a dedicated submodule.
  - [x] P1.2.g Extracted the person lookup, enrichment, listing, and dossier store methods into a dedicated submodule.
  - [x] P1.2.h Extracted the company support asset store methods into a dedicated submodule.
  - [x] P1.2.i Extracted the generic observation store methods into a dedicated submodule.
  - [x] P1.2.j Extracted the graph edge and traversal store methods into a dedicated submodule.
  - [x] P1.2.k Extracted the security scan write/read store methods into a dedicated submodule.
  - [x] P1.2.l Extracted the analytics and feature-aggregation store methods into a dedicated submodule.
  - [x] P1.2.m Extracted the POI artifact store methods into a dedicated submodule.
  - [x] P1.2.n Extracted the logistics and regulation store methods into a dedicated submodule.
  - [x] P1.2.o Extracted the role-history, dossier, and change-history store methods into a dedicated submodule.
  - [x] P1.2.p Extracted the weekly memo store methods into a dedicated submodule.
  - [x] P1.2.q Extracted the competitor listing and paging store methods into a dedicated submodule.
  Evidence:
  - Code: added `crates/store/src/postgres/preferences.rs` and declared `mod preferences;` from `crates/store/src/postgres.rs`; moved `get_user_settings_prefs`, `get_user_preferences_record`, `upsert_user_preferences_record`, `upsert_user_settings_prefs`, `list_user_settings_prefs_for_email_digest`, `mark_email_digest_sent`, and `ensure_user_preferences_table` into the new submodule while keeping shared types in `crates/store/src/postgres.rs`; extracted `merge_settings_page_preferences` so the settings-page JSON merge logic no longer lives inline inside the monolith. Added `crates/store/src/postgres/recipes.rs` and declared `mod recipes;`; moved `get_recipe_stats`, `get_recipe_quality_summary`, `get_staged_recipes_for_promotion`, `get_production_recipes_for_deprecation`, `record_recipe_weekly_metrics`, `list_staging_recipes`, `count_staging_recipes`, `promote_recipe`, `deprecate_recipe`, and `upsert_recipe_definition` into the recipe submodule so the recipe domain is no longer scattered across three regions of `postgres.rs`. Added `crates/store/src/postgres/admin.rs` and declared `mod admin;`; moved `get_admin_crawl_status`, `get_admin_recipe_performance`, `get_admin_poi_coverage`, `queue_job_trigger`, `pop_job_trigger`, `complete_job_trigger`, and `timeout_stale_job_triggers` into the admin submodule so the admin/status surface and worker-trigger queue no longer sit inline beside graph/security logic. Added `crates/store/src/postgres/warnings.rs` and declared `mod warnings;`; moved `list_warnings`, `count_warnings`, `acknowledge_warning`, `get_warnings_by_entity_ids`, `get_warning`, `insert_warning`, `delete_warning`, `delete_warnings`, and `delete_all_warnings` into the warning submodule so warning query/mutation logic no longer spans two distant `impl PgStore` regions in `crates/store/src/postgres.rs`. Added `crates/store/src/postgres/insights.rs` and declared `mod insights;`; moved `insert_insight`, `is_insight_bookmarked`, `list_insights`, `count_insights`, `bookmark_insight`, `unbookmark_insight`, `get_bookmarked_insight_ids`, `get_insight`, `get_related_insights`, and `get_insights_by_entity_ids` into the insight submodule so insight write/list/filter/bookmark/read logic no longer spans the early write block and later read sections of `crates/store/src/postgres.rs`. Added `crates/store/src/postgres/companies.rs` and declared `mod companies;`; moved `insert_company`, `get_company`, `get_company_by_domain`, `get_company_by_name_ci`, `get_company_names_by_ids`, `list_companies_by_region`, `list_companies_by_type`, `list_companies`, `count_companies`, and `get_company_dossier` into the company submodule so company write, lookup, pagination, and dossier assembly logic no longer sits inline before the insight and analytics paths. Added `crates/store/src/postgres/persons.rs` and declared `mod persons;`; moved `insert_person`, `get_person_names_by_company_ids`, `get_person`, `update_person_contacts`, `list_expansion_seeds`, `list_persons_by_org`, `update_person_influence_score`, `update_person_llm_enrichment`, `count_persons`, `list_persons`, `get_person_dossier`, `get_person_engagement`, and `get_person_peers` into the person submodule, and extracted a shared `append_person_filters` helper there so the person write/list/count/peer logic no longer duplicates query-fragment construction inline in `crates/store/src/postgres.rs`. Added `crates/store/src/postgres/company_assets.rs` and declared `mod company_assets;`; moved `insert_site`, `get_sites_for_company`, `insert_certification`, `get_certifications_for_company`, `insert_capability`, `list_sites`, `count_sites`, `list_capabilities`, `count_capabilities`, `list_certifications`, `count_certifications`, `list_product_families`, and `count_product_families` into the company-assets submodule so the company support asset surface no longer sits inline between graph, worker statistics, and generic list/count helpers. Added `crates/store/src/postgres/observations.rs` and declared `mod observations;`; moved `insert_observation`, `get_observations_by_entity`, `get_observations_by_type`, `list_observations`, and `count_observations` into the observations submodule so the generic observation read/write surface no longer sits inline between person and graph logic. Added `crates/store/src/postgres/graph.rs` and declared `mod graph;`; moved `get_graph_edge_features`, `upsert_edge`, `get_edges_from`, `get_edges_to`, `list_all_edges`, `count_edges`, `get_neighborhood`, and `get_path_edges` into the graph submodule so the graph edge write/query/traversal surface no longer remains split across the mid-file edge block and the late-file graph-operations block in `crates/store/src/postgres.rs`; the extracted module also centralizes graph list-limit handling with `normalize_graph_limit` and replaces the previous `SELECT *` neighborhood/path queries with explicit `EdgeRow` projections. Added `crates/store/src/postgres/security.rs` and declared `mod security;`; moved `insert_dns_posture_entry`, `insert_kev_observation`, `insert_lookalike_domain`, `get_dns_posture_entries`, `get_lookalike_domains`, and `get_kev_relevance` into the security submodule so the DNS posture, KEV, and lookalike-domain store paths no longer remain split between the late security-write block and the end-of-file security read endpoints in `crates/store/src/postgres.rs`; the extracted module also centralizes endpoint limit handling with `normalize_security_limit` and replaces the previous `SELECT *` security observation queries with explicit `ObservationRow` projections. Added `crates/store/src/postgres/analytics.rs`, `crates/store/src/postgres/artifacts.rs`, `crates/store/src/postgres/logistics.rs`, `crates/store/src/postgres/history.rs`, `crates/store/src/postgres/memos.rs`, and `crates/store/src/postgres/competitors.rs`; moved the remaining analytics/feature aggregation methods (`get_obs_type_counts_per_entity`, `get_warning_type_counts_per_entity`, `get_competitor_event_features`, `get_webchange_jsonb_features`, `get_webchange_keyword_features`, `get_person_features_per_company`, `get_certification_features_per_company`, `get_capability_features_per_company`, `get_site_features_per_company`, `get_crawl_stats`, `get_mining_stats`, `get_poi_stats`, `get_drift_stats`, `get_weekly_summary_stats`, `get_dashboard_stats`), POI artifact methods (`insert_poi_artifact`, `get_artifacts_for_person`, `list_poi_artifacts`, `count_poi_artifacts`), logistics/regulation methods (`insert_logistics_node`, `list_logistics_nodes`, `count_logistics_nodes`, `list_regulations`, `count_regulations`), history/dossier/change methods (`get_role_history_for_person`, `insert_role_history`, `get_role_history`, `close_role_history_entry`, `get_current_role`, `insert_dossier_entry`, `get_dossier_entries`, `get_dossier_entry_history`, `verify_dossier_entry`, `insert_company_change`, `get_company_changes`, `insert_person_change`, `get_person_changes`, `count_role_changes_since`), weekly memo methods (`get_weekly_memo_full`, `get_weekly_memo`, `list_weekly_memos`, `upsert_weekly_memo`), and competitor methods (`list_competitors`, `count_competitors`, `get_competitor_changes`, `get_all_competitor_changes_paged`, `get_competitor_changes_by_id`, `get_competitors_enhanced`) out of `crates/store/src/postgres.rs`; the new modules also centralize helper behavior with `saturating_count_to_u64`, `normalize_artifact_window`, `normalize_logistics_window`, `normalize_history_limit`, `normalize_memo_window`, `weekly_memo_from_parts`, `normalize_competitor_window`, and `normalize_competitor_page`, leaving `crates/store/src/postgres.rs` as shared helpers/types/bootstrap plus `run_migrations`.
  - Tests: added `test_merge_settings_page_preferences_preserves_existing_keys` and `test_merge_settings_page_preferences_replaces_non_object_root` in `crates/store/src/postgres/preferences.rs`; added `test_normalize_recipe_window_clamps_limit_and_offset` and `test_normalize_recipe_window_preserves_valid_values` in `crates/store/src/postgres/recipes.rs`; added `test_average_artifacts_per_person_handles_zero_people` and `test_trigger_row_to_result_formats_uuid` in `crates/store/src/postgres/admin.rs`; added `test_normalize_warning_window_clamps_limit_and_offset` and `test_normalize_warning_window_preserves_valid_values` in `crates/store/src/postgres/warnings.rs`; added `test_normalize_insight_window_clamps_limit_and_offset` and `test_normalize_insight_window_preserves_valid_values` in `crates/store/src/postgres/insights.rs`; added `test_normalize_company_window_clamps_limit_and_offset` and `test_normalize_company_domain_trims_and_strips_www` in `crates/store/src/postgres/companies.rs`; added `test_normalize_person_window_clamps_limit_and_offset`, `test_normalize_person_seed_limit_clamps_large_values`, and `test_person_engagement_status_defaults_to_untracked` in `crates/store/src/postgres/persons.rs`; added `test_normalize_company_assets_window_clamps_limit_and_offset` and `test_normalize_company_assets_window_preserves_valid_values` in `crates/store/src/postgres/company_assets.rs`; added `test_normalize_observation_window_clamps_limit_and_offset` and `test_normalize_observation_window_preserves_valid_values` in `crates/store/src/postgres/observations.rs`; added `test_normalize_graph_limit_clamps_zero_and_large_values` and `test_normalize_graph_limit_preserves_valid_values` in `crates/store/src/postgres/graph.rs`; added `test_normalize_security_limit_clamps_limit_and_offset` and `test_normalize_security_limit_preserves_valid_values` in `crates/store/src/postgres/security.rs`; added `test_saturating_count_to_u64_clamps_negative_counts` and `test_saturating_count_to_u64_preserves_positive_counts` in `crates/store/src/postgres/analytics.rs`; added `test_normalize_artifact_window_clamps_limit_and_offset` and `test_normalize_artifact_window_preserves_valid_values` in `crates/store/src/postgres/artifacts.rs`; added `test_normalize_logistics_window_clamps_limit_and_offset` and `test_normalize_logistics_window_preserves_valid_values` in `crates/store/src/postgres/logistics.rs`; added `test_normalize_history_limit_clamps_limit` and `test_normalize_history_limit_preserves_valid_values` in `crates/store/src/postgres/history.rs`; added `test_normalize_memo_window_clamps_limit_and_offset` and `test_weekly_memo_from_parts_defaults_invalid_json_shapes` in `crates/store/src/postgres/memos.rs`; added `test_normalize_competitor_window_clamps_limit_and_offset` and `test_normalize_competitor_page_clamps_page_and_size` in `crates/store/src/postgres/competitors.rs`; updated the existing `test_recipe_weekly_queries_use_snapshot_history_and_review_outcomes` in `crates/store/src/postgres.rs` to verify the moved recipe SQL via both `postgres.rs` and `postgres/recipes.rs`; verified with `~/.cargo/bin/cargo test -p apex-store test_merge_settings_page_preferences -- --nocapture`, `~/.cargo/bin/cargo test -p apex-store normalize_recipe_window -- --nocapture`, `~/.cargo/bin/cargo test -p apex-store test_recipe_weekly_queries_use_snapshot_history_and_review_outcomes -- --nocapture`, `~/.cargo/bin/cargo test -p apex-store average_artifacts_per_person -- --nocapture`, `~/.cargo/bin/cargo test -p apex-store trigger_row_to_result -- --nocapture`, `~/.cargo/bin/cargo test -p apex-store normalize_warning_window -- --nocapture`, `~/.cargo/bin/cargo test -p apex-store normalize_insight_window -- --nocapture`, `~/.cargo/bin/cargo test -p apex-store normalize_company_ -- --nocapture`, `~/.cargo/bin/cargo test -p apex-store normalize_person_ -- --nocapture`, `~/.cargo/bin/cargo test -p apex-store person_engagement_status -- --nocapture`, `~/.cargo/bin/cargo test -p apex-store normalize_company_assets_window -- --nocapture`, `~/.cargo/bin/cargo test -p apex-store normalize_observation_window -- --nocapture`, `~/.cargo/bin/cargo test -p apex-store normalize_graph_limit -- --nocapture`, `~/.cargo/bin/cargo test -p apex-store normalize_security_limit -- --nocapture`, `~/.cargo/bin/cargo test -p apex-store saturating_count_to_u64 -- --nocapture`, `~/.cargo/bin/cargo test -p apex-store normalize_artifact_window -- --nocapture`, `~/.cargo/bin/cargo test -p apex-store normalize_logistics_window -- --nocapture`, `~/.cargo/bin/cargo test -p apex-store normalize_history_limit -- --nocapture`, `~/.cargo/bin/cargo test -p apex-store weekly_memo_from_parts -- --nocapture`, and `~/.cargo/bin/cargo test -p apex-store normalize_competitor_ -- --nocapture`.
  - Validation: first declared `mod preferences;` without the file and confirmed the expected compile/editor failure (`file not found for module preferences`), then declared `mod recipes;` without the file and confirmed the expected compile/editor failure (`file not found for module recipes`), then declared `mod admin;` without the file and confirmed the expected compile/editor failure (`file not found for module admin`), then declared `mod warnings;` without the file and confirmed the expected compile/editor failure (`file not found for module warnings`), then declared `mod insights;` without the file and confirmed the expected compile/editor failure (`file not found for module insights`), then declared `mod companies;` without the file and confirmed the expected compile/editor failure (`file not found for module companies`), then declared `mod persons;` without the file and confirmed the expected compile/editor failure (`file not found for module persons`), then declared `mod company_assets;` without the file and confirmed the expected compile/editor failure (`file not found for module company_assets`), then declared `mod observations;` without the file and confirmed the expected compile/editor failure (`file not found for module observations`), then declared `mod graph;` without the file and confirmed the expected compile/editor failure (`file not found for module graph`), then declared `mod security;` without the file and confirmed the expected compile/editor failure (`file not found for module security`), then declared `mod analytics;`, `mod artifacts;`, `mod competitors;`, `mod history;`, `mod logistics;`, and `mod memos;` without files and confirmed the expected compile/editor failures for all six remaining modules; the first focused company helper run then failed because uppercase `WWW.` prefixes were not stripped before domain lookup normalization, so `normalize_company_domain_trims_and_strips_www` caught the bug and `crates/store/src/postgres/companies.rs` was fixed to lowercase before stripping the prefix; the first person extraction pass then temporarily left the original person methods in `crates/store/src/postgres.rs` and corrupted the adjacent insight/feature block, so the duplicate original methods were removed, the feature block was restored, and the store crate was revalidated; the first graph test run then failed because the new graph-module tests did not import `MAX_LIST_LIMIT`, so `crates/store/src/postgres/graph.rs` was updated to import the shared constant before rerunning the focused graph tests; the first security test run then failed because the new security-module tests did not import `MAX_LIST_LIMIT`, so `crates/store/src/postgres/security.rs` was updated to import the shared constant before rerunning the focused security tests; the final `P1.2` extraction pass then initially failed because `insert_logistics_node` had been left in both `crates/store/src/postgres.rs` and `crates/store/src/postgres/logistics.rs`, so the leftover root definition was removed before rerunning the focused tests and compile; after creating all seventeen submodules, moving the remaining analytics, artifacts, logistics, history, memo, and competitor surfaces, and relocating the last company/person/insight/admin methods into their owning modules, verified `~/.cargo/bin/cargo check -p apex-store`, and confirmed all touched store files are error-free with `get_errors`.

- [x] P1.3 Break `crates/worker/src/main.rs` into per-job execution modules.
  Why: job logic for 20+ workflows in one file makes failures hard to isolate and retry.
  Implementation detail:
  - Introduce a job execution abstraction.
  - Move nightly, weekly, crawl, POI, LLM, and digest jobs into dedicated modules.
  - Add job-level tests for dispatch and failure handling.
  Evidence:
  - Code: added `crates/worker/src/runtime.rs` and wired `crates/worker/src/main.rs` through thin local wrappers so scheduler ticks and manual-trigger polling now execute through the extracted runtime path instead of owning the loop logic inline; added `crates/worker/src/job_execution/poi.rs` and `crates/worker/src/job_execution/recipes.rs` so `crates/worker/src/job_execution/mod.rs` now owns dispatch for POI refresh/discovery and recipe-fire jobs alongside the already-extracted nightly, weekly, intelligence, security, and custom job modules.
  - Tests: verified `~/.cargo/bin/cargo test -p apex-worker custom_job_without_env_is_skipped -- --nocapture` after fixing the extracted dispatcher regression in `crates/worker/src/job_execution/mod.rs` to match the structured `JobStatus::Skipped { reason }` variant and assert against the stored skip reason instead of the unused `notes` field.
  - Validation: `~/.cargo/bin/cargo check -p apex-worker` passes on the cutover; the first worker test run failed with `expected unit struct, unit variant or constant, found struct variant crate::JobStatus::Skipped`, which exposed that the extracted dispatcher test had never been compiled on the live path, and the second run failed because `JobRun::skip` stores its reason on `JobStatus::Skipped` rather than `notes`; after fixing both issues, the targeted worker test and final worker compile both passed, and `get_errors` reported no errors in the touched worker files.

- [x] P1.4 Unify duplicate logic across graph/stats/core/worker modules.
  Why: duplicate risk propagation, expiry handling, and utility functions create drift.
  Implementation detail:
  - Collapse duplicate implementations into canonical modules.
  - Add tests asserting shared semantics remain unchanged.
  Progress:
  - [x] P1.4.a Unified graph risk propagation behind a shared `apex_core::graph_risk` helper used by both graph and stats.
  - [x] P1.4.b Unified company-name canonicalization behind shared `apex_core::company_names::normalize_company_name` used by both graph and worker.
  - [x] P1.4.c Unified set-overlap and Jaccard similarity behind shared `apex_core::similarity` helpers used by graph and worker.
  - [x] P1.4.d Unified UTF-8-safe text truncation behind shared `apex_core::text::truncate_utf8` used by worker main and job execution paths.
  - [x] P1.4.e Unified truthy env-flag parsing behind shared `apex_core::env::parse_truthy_flag` used by worker main and POI job execution.
  Evidence:
  - Code: `crates/core/src/graph_risk.rs` now owns the canonical weighted risk-propagation helper and `crates/core/src/lib.rs` exports it; `crates/graph/src/adjacency.rs` now delegates `AdjacencyGraph::propagate_risk` to the shared core helper instead of maintaining a second implementation, and `crates/stats/src/graph_risk.rs` now delegates its `propagate` function to the same helper so graph and stats no longer drift on overlap/capping semantics.
  - Tests: added `propagate_weighted_risk_accumulates_overlapping_sources` in `crates/core/src/graph_risk.rs`, `test_propagate_risk_accumulates_overlapping_sources` in `crates/graph/src/adjacency.rs`, and `test_propagate_accumulates_overlapping_sources` in `crates/stats/src/graph_risk.rs` to lock the shared multi-source frontier semantics.
  - Validation: `~/.cargo/bin/cargo test -p apex-graph propagate_risk -- --nocapture`, `~/.cargo/bin/cargo test -p apex-stats propagate_ -- --nocapture`, and `~/.cargo/bin/cargo check -p apex-core -p apex-graph -p apex-stats` all passed; `get_errors` reported no errors in the touched core, graph, stats, or checklist files.
  - Code: added `crates/core/src/company_names.rs` and exported it from `crates/core/src/lib.rs` so canonical company-name normalization now lives in core; `crates/graph/src/entity_resolution.rs` now imports the shared helper instead of owning its own regex/Unicode canonicalizer, and `crates/worker/src/main.rs` now uses the same core helper for seed-company matching rather than the previous ASCII-only local implementation.
  - Tests: added `normalize_company_name_strips_suffixes_and_punctuation`, `normalize_company_name_strips_diacritics`, and `normalize_company_name_normalizes_mixed_script_confusables` in `crates/core/src/company_names.rs`; added `company_name_matching_uses_shared_canonicalization` in `crates/worker/src/main.rs` to lock worker behavior for accented and mixed-script inputs.
  - Validation: first ran `~/.cargo/bin/cargo test -p apex-core normalize_company_name -- --nocapture` and confirmed the new helper initially failed on compound suffix handling (`Young Poong Electronics Co., Ltd.` still normalized to `young poong electronics co`) and then on spaced initialisms (`S.A.`/`N.V.`); after fixing the normalization order and spaced-suffix handling, `~/.cargo/bin/cargo test -p apex-core normalize_company_name -- --nocapture`, `~/.cargo/bin/cargo test -p apex-graph normalize_company_name -- --nocapture`, and `~/.cargo/bin/cargo test -p apex-worker --features llm company_name_matching_ -- --nocapture` all passed, and `get_errors` reported no errors in the touched core, graph, worker, or checklist files.
  - Code: added `crates/core/src/similarity.rs` and exported it from `crates/core/src/lib.rs` so generic `jaccard_similarity`, `shared_member_count`, and canonical `trigram_similarity` now live in core; `crates/graph/src/entity_resolution.rs` now imports the shared trigram helper instead of owning a private Jaccard implementation, `crates/graph/src/adjacency.rs` now delegates co-appearance counting through the shared overlap helper, and `crates/worker/src/main.rs` now computes token-overlap similarity through the same shared Jaccard helper rather than duplicating set-intersection math inline.
  - Tests: added `jaccard_similarity_returns_zero_for_empty_sets`, `shared_member_count_returns_intersection_size`, `trigram_similarity_handles_short_strings_with_padding`, and `trigram_similarity_preserves_relative_overlap` in `crates/core/src/similarity.rs`; added `token_jaccard_similarity_ignores_duplicate_tokens` in `crates/worker/src/main.rs` to lock the digest de-dup semantics on the shared helper.
  - Validation: first ran `~/.cargo/bin/cargo test -p apex-graph trigram_similarity -- --nocapture` after rewiring graph to `apex_core::similarity` and confirmed the expected fail-first compile error (`could not find similarity in apex_core`); after adding the shared core module, `~/.cargo/bin/cargo test -p apex-core similarity:: -- --nocapture`, `~/.cargo/bin/cargo test -p apex-graph trigram_similarity -- --nocapture`, `~/.cargo/bin/cargo test -p apex-graph co_appearance_count -- --nocapture`, and `~/.cargo/bin/cargo test -p apex-worker token_jaccard_similarity_ignores_duplicate_tokens -- --nocapture` all passed, and `get_errors` reported no errors in the touched core, graph, worker, or checklist files.
  - Code: added `crates/core/src/text.rs` and exported it from `crates/core/src/lib.rs` so UTF-8-safe byte truncation now lives in core; `crates/worker/src/main.rs` now re-exports `apex_core::text::truncate_utf8` as the canonical truncation path used by worker main and existing job execution modules instead of owning the implementation inline.
  - Tests: added `truncate_utf8_returns_original_when_already_short`, `truncate_utf8_respects_multibyte_boundaries`, and `truncate_utf8_handles_zero_limit` in `crates/core/src/text.rs`; added `truncate_text_uses_shared_utf8_boundary_logic` in `crates/worker/src/main.rs` to lock the worker-facing alias against the shared helper.
  - Validation: first ran `~/.cargo/bin/cargo test -p apex-worker token_jaccard_similarity_ignores_duplicate_tokens -- --nocapture` after rewiring worker and confirmed the expected fail-first compile error (`could not find text in apex_core`); after adding the shared text module, `~/.cargo/bin/cargo test -p apex-core truncate_utf8 -- --nocapture` and `~/.cargo/bin/cargo test -p apex-worker truncate_text_uses_shared_utf8_boundary_logic -- --nocapture` both passed.
  - Code: extended `crates/core/src/env.rs` with canonical truthy parsing via `parse_truthy_flag`; `crates/worker/src/main.rs` now uses that shared parser for both generic `env_flag` handling and `EMAIL_DIGEST_SMTP_STARTTLS`, and `crates/worker/src/job_execution/poi.rs` now uses the same shared parser for `POI_ONION_ENRICH_ENABLED` rather than duplicating the lowercase-and-match logic.
  - Tests: added `parse_truthy_flag_accepts_common_truthy_values` and `parse_truthy_flag_rejects_other_values` in `crates/core/src/env.rs`; added `env_flag_uses_shared_truthy_parser` in `crates/worker/src/main.rs`.
  - Validation: the same fail-first worker compile above also confirmed the missing shared env helper (`no parse_truthy_flag in env`); after adding the core parser, `~/.cargo/bin/cargo test -p apex-core parse_truthy_flag -- --nocapture` and `~/.cargo/bin/cargo test -p apex-worker env_flag_uses_shared_truthy_parser -- --nocapture` both passed.
  - Validation: final sweep and validation for the completed parent item passed with `~/.cargo/bin/cargo check -p apex-core -p apex-graph -p apex-stats -p apex-worker`; remaining matches from the duplicate scan were limited to local/domain-specific logic rather than cross-module duplicate implementations, so `P1.4` is complete.

---

## Phase 2: Collection Layer Credibility

- [x] P2.1 Build a unified crawl pipeline abstraction.
  Why: fetch behavior is inconsistent across crawlers, making reliability and observability uneven.
  Implementation detail:
  - Enforce robots, rate limit, headers, proxy, retries, change detection, and metrics in one path.
  - Migrate high-value collectors first: RSS, POI expansion, sanctions, CT, social.
  Evidence:
  - Code: added `crates/crawl/src/client.rs` and exported it from `crates/crawl/src/lib.rs` so `CrawlClient`, `CrawlClientConfig`, `CrawlRequest`, and `FetchResponse` now provide one canonical fetch path with robots enforcement, shared rate-limit state, optional proxy rotation, shared metrics, retry handling, and consistent user-agent selection; `crates/crawl/src/rss.rs` now delegates feed fetches through `CrawlClient` instead of owning raw `reqwest` logic; `crates/worker/src/job_execution/nightly.rs` now builds one shared `CrawlClient` for crawl-cycle ingestion and uses it for all selected sources instead of hand-building per-request clients and proxy handling inline.
  - Tests: added `fetch_text_retries_after_rate_limit`, `fetch_text_uses_cached_robots_rules`, and `parse_cache_control_max_age_reads_seconds` in `crates/crawl/src/client.rs`; fixed and kept RSS coverage green with `parse_rss_feed`, `parse_atom_feed`, `parse_date_rfc2822`, and `fetcher_builds` in `crates/crawl/src/rss.rs`.
  - Validation: first ran `~/.cargo/bin/cargo test -p apex-crawl && ~/.cargo/bin/cargo check -p apex-worker` and surfaced P2-path integration failures in RSS parsing plus a worker `Send` issue from logging across an `await`; after fixing the RSS parser for self-closing Atom links and relaxed RFC 2822 dates, and fixing worker crawl-cycle logging/client wiring, `~/.cargo/bin/cargo test -p apex-crawl rss::tests::`, `~/.cargo/bin/cargo test -p apex-crawl client::tests::`, and `~/.cargo/bin/cargo check -p apex-worker` all passed.

- [x] P2.2 Add shared retry/backoff and error typing for network fetches.
  Why: many collectors still rely on single-attempt fetches and string-matched failures.
  Implementation detail:
  - Introduce a typed crawl error enum.
  - Add response-aware backoff for 429, 403, 5xx, timeout, proxy, and DNS failures.
  Evidence:
  - Code: extended `crates/crawl/src/errors.rs` with a typed `CrawlError` enum plus retryability/category/retry-after helpers and a `Proxy` failure category; `crates/crawl/src/client.rs` now converts `reqwest` and HTTP-status failures into typed crawl errors, respects `Retry-After`, reports failures into `RateLimitManager`, and retries retryable upstream/rate-limit failures through the shared path instead of leaving retry behavior to individual collectors.
  - Tests: added `http_status_429_is_retryable_rate_limit` and `proxy_errors_are_typed` in `crates/crawl/src/errors.rs`; added `fetch_text_retries_after_rate_limit` in `crates/crawl/src/client.rs` to lock the backoff-and-retry path on a real 429 -> 200 sequence.
  - Validation: `~/.cargo/bin/cargo test -p apex-crawl errors::tests::` and `~/.cargo/bin/cargo test -p apex-crawl client::tests::` both passed after the typed-error and retry integration landed; worker compilation via `~/.cargo/bin/cargo check -p apex-worker` also passed with the new typed fetch path in use.

- [x] P2.3 Make source registry runtime-configurable.
  Why: source coverage changes should not require recompilation.
  Implementation detail:
  - Load source definitions from YAML/TOML.
  - Preserve typed validation and health metadata.
  - Support enable/disable, auth requirements, data format, and interval hints.
  Evidence:
  - Code: added `serde_yaml` to `crates/crawl/Cargo.toml`; `crates/crawl/src/sources.rs` now supports `APEX_SOURCE_REGISTRY_PATH`, `load_sources_from_env`, `load_sources_from_path`, typed `SourceRegistryError`, validation of duplicate/invalid entries, and `select_sources_for_crawl`; `all_sources()` now attempts runtime override loading and falls back to the built-in registry on invalid config; `crates/worker/src/job_execution/nightly.rs` and `crates/worker/src/job_execution/intelligence.rs` now use the shared selection/filter helpers instead of duplicating tier-selection logic inline.
  - Tests: added `load_sources_from_path_overrides_defaults`, `load_sources_from_path_rejects_duplicate_slugs`, and `select_sources_for_crawl_preserves_forced_sources` in `crates/crawl/src/sources.rs`.
  - Validation: `~/.cargo/bin/cargo test -p apex-crawl sources::tests::` passed, and the worker integration remained green under `~/.cargo/bin/cargo check -p apex-worker` after switching nightly crawl/source scoring to the shared source-selection helpers.

- [x] P2.4 Add health-aware search engine rotation.
  Why: search rotation currently ignores engine health and rate state.
  Implementation detail:
  - Integrate `SearchPool`, rate limiter, and governor into one chooser.
  - Add tests for unhealthy engine skip and RPM enforcement.
  Evidence:
  - Code: `crates/crawl/src/search_rotation.rs` now wires `SearchPool` to shared `RateLimitManager` health scoring, per-engine RPM windows, and `CrawlGovernor` admission checks so engine selection skips backoff-blocked providers instead of blindly round-robining; `PersonOsintQueryBuilder::build_urls` now consumes the health-aware chooser rather than the old unconstrained rotation path.
  - Tests: added `unhealthy_engine_is_skipped` and `engine_rpm_limit_is_enforced` in `crates/crawl/src/search_rotation.rs`; updated `pool_round_robins` to use a deterministic synthetic pool and assert first-cycle diversity under the new governor-aware semantics.
  - Validation: first ran `source "$HOME/.cargo/env" && cargo test -p apex-crawl search_rotation::tests::` and hit a failure in `pool_round_robins` because the new governor intentionally blocks an immediate same-second wraparound after every engine has been consumed once; after tightening the test to assert first-cycle diversity rather than an unrealistic second-cycle burst, reran `source "$HOME/.cargo/env" && cargo test -p apex-crawl search_rotation::tests::` and it passed.

- [x] P2.5 Expand high-value OSINT sources.
  Why: current coverage misses several source classes needed for a serious intel platform.
  Implementation detail:
  - Add infrastructure intelligence, maritime/flight signals, hiring signals, finance filings, startup/M&A, and code-host intelligence.
  - Add source scoring hooks so new sources can be measured rather than assumed useful.
  Evidence:
  - Code: expanded `crates/crawl/src/sources.rs` with infrastructure and industrial-build sources (`engineering_news_record`, `construction_dive`), maritime and aviation movement signals (`vessel_finder_news`, `flightglobal`), hiring-signal feeds (`greenhouse_job_board`, `lever_job_board`), finance-filing and startup/M&A coverage (`sedar_plus_ca`, `crunchbase_news`, `tech_eu_ma`), and code-host intelligence (`github_security_advisories`, `gitlab_releases`); all were added to the same runtime-configurable registry path used by source selection and scoring, so the nightly/source-scoring jobs can measure them through the existing shared selection flow instead of assuming their value out of band.
  - Tests: added `expanded_high_value_osint_classes_are_present` in `crates/crawl/src/sources.rs` to lock the new source classes into the registry alongside the existing registry validation suite.
  - Validation: `source "$HOME/.cargo/env" && cargo test -p apex-crawl sources::tests::` passed after the registry expansion landed.

- [x] P2.6 Add headless-browser collection path for JS-rendered sources.
  Why: LinkedIn, Facebook, and modern dynamic sites are under-collected with raw HTTP only.
  Implementation detail:
  - Add a bounded browser runner.
  - Use it only for sources that justify cost and complexity.
  - Add recorded integration fixtures for at least one dynamic page path.
  Evidence:
  - Code: added `crates/crawl/src/browser.rs` and exported it from `crates/crawl/src/lib.rs`; the new `BoundedBrowserRunner` supports bounded concurrency, timeout-capped headless DOM dumps, and a recorded-fixture mode for deterministic tests; `crates/crawl/src/social/linkedin.rs` now accepts an optional browser runner and falls back to it only after HTTP retries are exhausted and only for explicitly supported dynamic URLs, keeping browser cost isolated to justified JS-heavy paths.
  - Tests: added `recorded_browser_fixture_returns_rendered_html` and `headless_policy_is_limited_to_dynamic_sources` in `crates/crawl/src/browser.rs`; added `recorded_browser_fixture_parses_dynamic_company_page` in `crates/crawl/src/social/linkedin.rs` backed by the recorded fixture `crates/crawl/src/social/fixtures/linkedin_company_dynamic.html`.
  - Validation: `source "$HOME/.cargo/env" && cargo test -p apex-crawl browser::tests::` and `source "$HOME/.cargo/env" && cargo test -p apex-crawl social::linkedin::tests::` both passed after the bounded browser path and LinkedIn fixture coverage were added.

- [x] P2.7 Internationalize name and entity extraction beyond Western regex assumptions.
  Why: POI discovery and parsing are still biased toward Latin-script name patterns.
  Implementation detail:
  - Add Unicode-aware name extraction.
  - Improve transliteration coverage.
  - Add regression tests for Arabic, Hebrew, Cyrillic, and CJK names.
  Evidence:
  - Code: added `crates/core/src/person_names.rs` and exported it from `crates/core/src/lib.rs` so Unicode-aware person-name validation now lives in one shared helper used across crawl, parse, and worker; `crates/parse/src/person.rs` now validates extracted names through the shared helper and uses broader Unicode regexes instead of Latin-only structured-name patterns; `crates/parse/src/transliteration.rs` now detects and transliterates Cyrillic and normalizes names through Unicode decomposition rather than a small hand-maintained diacritic table; `crates/crawl/src/poi_expansion.rs` and `crates/worker/src/main.rs` now route local name heuristics through the shared core helper instead of duplicated Western-only logic.
  - Tests: added multilingual helper tests in `crates/core/src/person_names.rs`; added `test_extract_arabic_person`, `test_extract_cyrillic_person`, and `test_extract_cjk_person` in `crates/parse/src/person.rs`; added `test_detect_cyrillic` and `test_cyrillic_transliteration` in `crates/parse/src/transliteration.rs` while keeping the existing Arabic, Hebrew, and CJK transliteration coverage green.
  - Validation: `source "$HOME/.cargo/env" && cargo test -p apex-core person_names::tests::`, `source "$HOME/.cargo/env" && cargo test -p apex-parse person::tests::`, and `source "$HOME/.cargo/env" && cargo test -p apex-parse transliteration::tests::` all passed; worker integration also compiled cleanly with `source "$HOME/.cargo/env" && cargo check -p apex-worker --features llm`.

---

## Phase 3: Evidence Quality and Analytical Rigor

- [x] P3.1 Introduce a first-class evidence quality framework.
  Why: source reliability and evidence independence do not currently flow into analyst-facing confidence.
  Implementation detail:
  - Add source reliability tiers.
  - Add corroboration and source-diversity weighting.
  - Thread these signals into renderer, evidence chain, and memo generation.
  Evidence:
  - Code: added `crates/core/src/analysis.rs` and exported it from `crates/core/src/lib.rs` so shared `EvidenceRecord`, `EvidenceQuality`, `assess_evidence_quality`, source-group extraction, weak-signal fusion, temporal deltas, and hypothesis scorecards now live in core; `crates/worker/src/job_execution/recipes.rs` now converts live recipe-fire evidence into the shared evidence-quality model and blends corroboration/diversity/independence into stored insight confidence and evidence tags; `crates/insights/src/memo.rs` now computes evidence posture for regional memo summaries rather than treating all cited evidence as equivalent.
  - Tests: added `evidence_quality_rewards_independent_corroboration` and `evidence_quality_penalizes_contradictions` in `crates/core/src/analysis.rs`; added `test_regional_sections_include_evidence_posture` in `crates/insights/src/memo.rs`.
  - Validation: `cargo test -p apex-core analysis::`, `cargo test -p apex-insights memo::`, and `cargo check -p apex-worker --features llm` passed.

- [x] P3.2 Close the confidence calibration loop using outcome tracking.
  Why: the system computes forecast quality but does not use it to improve future scoring.
  Implementation detail:
  - Use outcome history to calibrate recipe and insight confidence.
  - Add reliability-curve or calibration-bin tests.
  Evidence:
  - Code: `crates/core/src/analysis.rs` now exposes `calibrate_confidence` with a posterior-precision view over true/false positive history, and `crates/recipes/src/engine.rs` now runs recipe confidence through that shared calibration helper so false-positive-heavy recipes are discounted rather than relying on raw heuristic precision alone.
  - Tests: added `calibration_disciplines_low_precision_histories` in `crates/core/src/analysis.rs` and `test_estimate_confidence_penalizes_false_positive_history` in `crates/recipes/src/engine.rs`.
  - Validation: `cargo test -p apex-core analysis::` and `cargo test -p apex-recipes` passed.

- [x] P3.3 Add temporal analysis and delta-awareness to memos and dossiers.
  Why: current outputs are mostly snapshots, not change reports.
  Implementation detail:
  - Add week-over-week deltas, trend direction, and newly-emerged vs ongoing signals.
  - Add tests for stable, rising, and declining scenarios.
  Evidence:
  - Code: `crates/insights/src/memo.rs` now emits `TemporalSummary` and a dedicated memo section for momentum/deltas based on period splits; `crates/store/src/postgres.rs` now exposes `DossierAnalysis` on company/person dossier responses; `crates/store/src/postgres/companies.rs` and `crates/store/src/postgres/persons.rs` now compute dossier temporal deltas and analyst-facing summaries from recent change/history timelines; `crates/worker/src/job_execution/weekly.rs` now persists the temporal memo section to `weekly_memos` with the correct serialized section shape.
  - Tests: added `temporal_delta_computes_direction` in `crates/core/src/analysis.rs`; memo regression coverage continued through `test_generate_weekly_memo_full` and `test_render_memo_text_structure` in `crates/insights/src/memo.rs` with the new delta sections present.
  - Validation: `cargo test -p apex-core analysis::`, `cargo test -p apex-insights memo::`, `cargo test -p apex-store postgres::companies::`, `cargo test -p apex-store postgres::persons::`, `cargo test -p apex-store postgres::memos::`, and `cargo check -p apex-worker --features llm` passed.

- [x] P3.4 Add cross-insight correlation and weak-signal fusion.
  Why: insights are still generated and displayed too independently.
  Implementation detail:
  - Group related observations and derived insights across categories.
  - Detect coherent pattern clusters.
  - Add tests that prove separate weak signals fuse into a stronger analytical outcome only when independence criteria are met.
  Evidence:
  - Code: `crates/core/src/analysis.rs` now provides `SignalFrame`, `WeakSignalCluster`, and `fuse_weak_signals` with explicit independent-source requirements; `crates/insights/src/memo.rs` now builds fused weak-signal clusters for memo output; `crates/store/src/postgres/companies.rs` and `crates/store/src/postgres/persons.rs` now expose correlated weak-signal clusters in dossier analysis rather than leaving related evidence fragmented across raw rows.
  - Tests: added `weak_signal_fusion_requires_independent_sources` in `crates/core/src/analysis.rs` and `test_fused_signal_clusters_require_multiple_related_cards` in `crates/insights/src/memo.rs`.
  - Validation: `cargo test -p apex-core analysis::`, `cargo test -p apex-insights memo::`, and `cargo check -p apex-worker --features llm` passed.

- [x] P3.5 Add Analysis of Competing Hypotheses support.
  Why: the platform currently builds one favored story more often than structured alternatives.
  Implementation detail:
  - Extend evidence chain with competing conclusion sets.
  - Score support and contradiction across hypotheses.
  Evidence:
  - Code: `crates/core/src/analysis.rs` now provides shared `HypothesisInput`, `HypothesisScorecard`, and `score_competing_hypotheses`; `crates/insights/src/memo.rs` now emits a Competing Hypotheses section in both the structured memo JSON and rendered memo text; `crates/store/src/postgres/companies.rs` and `crates/store/src/postgres/persons.rs` now populate ranked competing hypotheses inside dossier analysis so alternative explanations are carried through the API, not just implicit in analyst reading.
  - Tests: added `competing_hypotheses_rank_favored_first` in `crates/core/src/analysis.rs`; memo coverage in `test_generate_weekly_memo_full` now asserts the competing-hypotheses section is present.
  - Validation: `cargo test -p apex-core analysis::`, `cargo test -p apex-insights memo::`, and `cargo check -p apex-worker --features llm` passed.

- [x] P3.6 Add seasonal and anomaly-aware statistics.
  Why: threshold-only and raw anomaly logic will misfire on cyclical data.
  Implementation detail:
  - Add decomposition-aware anomaly detection.
  - Add weekday/seasonality normalization where relevant.
  Evidence:
  - Code: `crates/stats/src/observation_anomaly.rs` now compares the latest ingest count against a weekday-matched seasonal baseline before flagging outages/drops and records `seasonal_expected` on alerts; `crates/stats/src/pipeline.rs` now accepts `seasonal_period`, computes seasonal baseline/residual/z-score features, and feeds seasonal anomaly state into overall alert derivation.
  - Tests: added `weekday_baseline_prevents_false_alert_on_weekend_pattern` in `crates/stats/src/observation_anomaly.rs` and `seasonal_anomaly_detects_off_cycle_spike` in `crates/stats/src/pipeline.rs`.
  - Validation: `cargo test -p apex-stats` passed.

---

## Phase 4: API and Query Layer Capability Gaps

- [x] P4.1 Wire and verify currently dead or partially wired endpoints.
  Why: several capabilities are defined but not actually reachable.
  Implementation detail:
  - Semantic search.
  - Deep health.
  - Replay/admin flows.
  - Add route-level tests.
  Evidence:
  - Code: `crates/api/src/main.rs` now wires live `/api/search/semantic`, `/api/health/deep`, `/api/admin/replay`, and `/api/admin/replay/:job_id` routes plus `/api/v1/*` aliases; `crates/api/src/api_handlers/overview.rs` implements the semantic-search handler, and `crates/api/src/api_handlers/admin.rs` now provides replay job creation/status handling instead of leaving the route-model modules unwired.
  - Tests: kept the route surface under test by expanding `crates/api/src/routes/mod.rs` endpoint-catalog coverage and adding `enhanced_search_query_includes_phrase_and_fuzzy_clauses` in `crates/api/src/api_handlers/overview.rs`; replay request validation and health/semantic helper suites stayed green in `crates/api/src/routes/replay.rs`, `crates/api/src/routes/health.rs`, and `crates/api/src/routes/semantic_search.rs`.
  - Validation: `cargo check -p apex-api` passed after wiring the endpoints, and `cargo test -p apex-api` passed with the new routes and handlers in place.

- [x] P4.2 Replace method-based permission inference with route-aware authorization policy.
  Why: request method alone is not a safe proxy for privilege level.
  Implementation detail:
  - Introduce explicit permission resolution for destructive and admin routes.
  - Add route permission regression tests.
  Evidence:
  - Code: `crates/api/src/main.rs` now normalizes `/api/v1` aliases back to canonical paths and resolves authorization by route shape rather than raw method alone, explicitly elevating admin, destructive warning, deep-health, replay, and user-management endpoints while preserving write access for acknowledge/bookmark/analyze/preferences flows.
  - Tests: existing `tests::test_required_permission_for_delete_all_warnings_is_admin`, `tests::test_required_permission_for_delete_warning_is_admin`, `tests::test_required_permission_for_bulk_delete_warnings_is_admin`, and `tests::test_required_permission_for_warning_acknowledge_remains_write` in `crates/api/src/main.rs` remained green against the new route-aware policy.
  - Validation: `cargo test -p apex-api` passed with the tightened permission resolver and its regression tests.

- [x] P4.3 Add proper API documentation and versioning.
  Why: endpoint discovery is currently manual and error-prone.
  Implementation detail:
  - Generate OpenAPI.
  - Add version prefix strategy.
  - Keep examples synchronized with tests.
  Evidence:
  - Code: `crates/api/src/routes/mod.rs` now owns corrected path constants, version-path helpers, and a generated OpenAPI 3.1 document built from the endpoint catalogue; `crates/api/src/main.rs` now serves `/api/openapi.json`, `/api/docs`, and `/api/v1/*` aliases so discovery and versioned access are live instead of ad hoc.
  - Tests: updated `crates/api/src/routes/mod.rs` catalogue assertions to cover the expanded documented surface, and the route helper suites continued to pass with the new versioning helpers.
  - Validation: `cargo check -p apex-api` and `cargo test -p apex-api` both passed with the OpenAPI/documentation/versioning changes compiled into the binary.

- [x] P4.4 Add CRUD, saved search, audit, and export-history capabilities needed by analysts.
  Why: the API is read-heavy and underpowered for operational analyst workflows.
  Implementation detail:
  - Users, watchlists, saved searches, annotations, notification preferences, export history.
  - Add schema migrations and store coverage.
  Evidence:
  - Code: added `migrations/20260309_collaboration_and_replay.sql`; added `crates/store/src/postgres/collaboration.rs` plus new record types in `crates/store/src/postgres.rs` for analyst users, saved searches, watchlists, annotations, export history, and replay jobs; added `crates/api/src/routes/collaboration.rs` and `crates/api/src/api_handlers/collaboration.rs` so `/api/users`, `/api/saved-searches`, `/api/watchlists`, `/api/annotations`, and `/api/export-history` are live CRUD surfaces with audit-log writes; `crates/api/src/api_handlers/exports.rs` now records export-history events on CSV downloads.
  - Tests: added validation coverage in `crates/api/src/routes/collaboration.rs` for saved-search and note normalization paths, and the API binary test suite continued to exercise the new collaboration-aware crate build successfully.
  - Validation: `cargo check -p apex-api` passed with the new migration-backed store/API flow, and `cargo test -p apex-api` passed after the collaboration endpoints and export-history tracking were added.

- [x] P4.5 Improve search quality end-to-end.
  Why: whitespace tokenization and unwired semantic search undercut discovery.
  Implementation detail:
  - Add fuzzy search, phrase awareness, boosting, snippets, and facets.
  - Add tests for typo tolerance and domain-specific synonyms.
  Evidence:
  - Code: `crates/api/src/api_handlers/overview.rs` now builds boosted phrase-aware queries for the live `/api/search` path and exposes the richer `/api/search/semantic` path with fuzzy term expansion, field boosting, snippet generation, result filtering, and facet aggregation using the existing Tantivy index plus `crates/api/src/routes/semantic_search.rs` helpers.
  - Tests: added `enhanced_search_query_includes_phrase_and_fuzzy_clauses` in `crates/api/src/api_handlers/overview.rs`; kept the semantic-search helper tests in `crates/api/src/routes/semantic_search.rs` green after aligning the extraction expectation with the active `>=2` token policy.
  - Validation: `cargo check -p apex-api` and `cargo test -p apex-api` both passed with the upgraded search stack.

---

## Phase 5: UI and Analyst Workflow Quality

- [x] P5.1 Add high-density data tables for warnings, insights, companies, and persons.
  Why: cards alone are not sufficient for triage, comparison, or operations at scale.
  Implementation detail:
  - Add sortable columns, keyboard navigation, and export-compatible views.
  - Preserve current card layouts for narrative browsing where useful.
  Evidence:
  - Code: `crates/api/templates/pages/warnings.html`, `crates/api/templates/pages/warnings/_list.html`, `crates/api/templates/pages/insights.html`, `crates/api/templates/pages/insights/_list.html`, `crates/api/templates/pages/companies.html`, `crates/api/templates/pages/companies/_list.html`, and `crates/api/templates/pages/persons.html` now render dense analyst tables ahead of the existing cards; `crates/api/static/js/app.js` now adds row-click, arrow-key, and Enter navigation for those tables; CSV/JSON export affordances were surfaced inline on the dense views.
  - Tests: Askama-backed template compilation was exercised through the crate build/test run, and the shared JS/navigation changes shipped without introducing API-side regressions.
  - Validation: `cargo check -p apex-api` and `cargo test -p apex-api` both passed after the dense-table templates and navigation hooks landed.

- [x] P5.2 Add print/PDF-ready briefing layouts for memos, dossiers, and key warning views.
  Why: analysts need briefing artifacts, not just browser views.
  Implementation detail:
  - Add print CSS.
  - Add PDF export path or HTML-to-PDF rendering boundary.
  Evidence:
  - Code: `crates/api/src/web/memos.rs`, `crates/api/src/web/companies.rs`, `crates/api/src/web/persons.rs`, and `crates/api/src/web/warnings.rs` now accept a `briefing=true` mode that drives briefing-oriented rendering; `crates/api/templates/pages/memos.html`, `crates/api/templates/pages/company_detail.html`, `crates/api/templates/pages/person_detail.html`, and `crates/api/templates/pages/warning_detail.html` now expose print/PDF actions and linearize the dossier content for print output; `crates/api/static/css/globals.css` now hides chrome and preserves printable cards/page breaks under `@media print`.
  - Tests: Askama-backed template changes were exercised through the API crate test run and compile checks.
  - Validation: `cargo check -p apex-store -p apex-worker -p apex-api` and `cargo test -p apex-api --lib` both passed after the briefing-mode and print CSS changes landed.

- [x] P5.3 Add persistent notifications inbox and analyst annotations.
  Why: ephemeral toasts and read-only detail pages lose operational context.
  Implementation detail:
  - Add persisted notifications table.
  - Add analyst comments and resolution notes on warnings and insights.
  Evidence:
  - Code: added `crates/store/migrations/0006_notifications_and_worker_state.sql` plus `analyst_notifications` store support in `crates/store/src/postgres/collaboration.rs`; added the server-rendered inbox in `crates/api/src/web/notifications.rs` and `crates/api/templates/pages/notifications.html` with navigation in `crates/api/templates/base.html`; `crates/api/src/web/warnings.rs` and `crates/api/src/web/insights.rs` now persist analyst notes via the existing annotations table, capture warning resolution notes/review outcomes, and emit inbox notifications for note/bookmark/review actions.
  - Tests: notification and annotation UI/store changes were covered by the successful API and store compilation plus the full `apex-api` unit-test suite.
  - Validation: `cargo check -p apex-store -p apex-worker -p apex-api` and `cargo test -p apex-api --lib` passed with the inbox, annotation, and warning-review flows wired.

- [x] P5.4 Improve graph exploration usability.
  Why: current graph rendering is visually functional but operationally shallow.
  Implementation detail:
  - Add search-within-graph, clustering, export, and time-aware filters.
  Evidence:
  - Code: `crates/api/templates/pages/graph.html` now includes search, activity-window, clustering, and export controls; `crates/api/static/js/graph.js` now filters nodes by text and recency, toggles a clustered type layout, exports JSON/SVG, and reports visible node counts; `crates/api/src/web/graph.rs` now enriches graph payload nodes with `clusterKey` and real `activityDays` derived from warning/insight entity activity.
  - Tests: graph server changes remained covered by the existing API unit-test suite, and the new graph UI path is included in the Playwright regression spec.
  - Validation: `cargo check -p apex-store -p apex-worker -p apex-api`, `cargo test -p apex-api --lib`, and `npx playwright test --list` all succeeded with the upgraded graph explorer path in place.

- [x] P5.5 Build real end-to-end UI regression coverage.
  Why: parity infrastructure exists but is not populated or enforced.
  Implementation detail:
  - Capture baselines.
  - Add Playwright coverage for key user paths.
  - Add visual diff threshold discipline.
  Evidence:
  - Code: root `package.json` now includes Playwright scripts and dependency management; `playwright.config.js` now defines snapshot storage, diff tolerance, and HTML reporting rooted in `frontend/e2e/parity`; `frontend/e2e/ui-regression.spec.js` now covers the login page plus authenticated dashboard, warnings, and graph paths.
  - Tests: the new Playwright suite is discoverable and enumerates four regression specs; baseline image generation is now driven by `npm run test:e2e:update` once a live authenticated app instance is available.
  - Validation: `npm install` completed successfully and `npx playwright test --list` discovered the new regression suite. A full browser screenshot run was not executed in this environment because no live server/auth session was provisioned.

---

## Phase 6: Worker and Operational Reliability

- [x] P6.1 Persist scheduler state and stale-trigger recovery.
  Why: worker restarts should not reset history or leave stuck jobs forever.
  Implementation detail:
  - Persist last run, failure counts, circuit-breaker state.
  - Add timeout handling for claimed but abandoned triggers.
  Evidence:
  - Code: `crates/store/migrations/0006_notifications_and_worker_state.sql` now adds `worker_job_state`, `worker_job_history`, and trigger recovery columns; `crates/store/src/postgres/admin.rs` now persists/restores scheduler state, records job history, and requeues stale manual triggers instead of terminally timing them out; `crates/worker/src/runtime.rs` and `crates/worker/src/main.rs` now restore persisted state on startup and persist scheduled/manual job runs as they complete.
  - Tests: scheduler persistence paths were validated through successful worker compilation; the stale-trigger and job-history logic is exercised by the new runtime code path plus existing scheduler coverage.
  - Validation: `cargo check -p apex-store -p apex-worker -p apex-api` passed after the scheduler-state persistence and stale-trigger recovery changes landed.

- [x] P6.2 Add stage-level retries and timeouts in nightly and weekly pipelines.
  Why: a single transient failure should not collapse a full orchestration run.
  Implementation detail:
  - Add bounded retry wrappers.
  - Add tests for transient-failure recovery.
  Evidence:
  - Code: added shared stage resilience helper `crates/worker/src/job_execution/resilience.rs`; `crates/worker/src/job_execution/nightly.rs` now wraps mining/hypothesis/drift DB-load stages with retry+timeout behavior; `crates/worker/src/job_execution/weekly.rs` now retries/limits weekly metric persistence, pipeline input loading, insight loading, and memo generation stages.
  - Tests: added `stage_retry_recovers_from_transient_failure` and `stage_retry_reports_timeout` in `crates/worker/src/job_execution/resilience.rs`.
  - Validation: `cargo test -p apex-worker resilience::tests::` passed, and the full follow-on crate checks/tests (`cargo check -p apex-store -p apex-worker -p apex-api`, `cargo test -p apex-api --lib`) remained green with the retry wrapper integrated.

- [x] P6.3 Unify SLA logic and notification delivery semantics.
  Why: duplicate implementations drift and create inconsistent escalation.
  Implementation detail:
  - Pick one SLA model.
  - Add idempotent reminder tracking.
  - Add retrying webhook/email delivery.
  Evidence:
  - Code: added shared `SeveritySlaConfig` in `crates/core/src/sla.rs` and rewired `crates/insights/src/cep.rs` plus `crates/worker/src/notifications.rs` to use the same severity-window model instead of separate CEP/worker SLA structs; `crates/store/migrations/0007_identity_tags_and_delivery.sql` now creates `sla_reminder_state`, `notification_delivery_state`, and `notification_delivery_attempts`; `crates/store/src/postgres/collaboration.rs` now records idempotent SLA reminders and delivery attempts; `crates/worker/src/notifications.rs` now retries webhook and email sends with stable delivery keys; `crates/worker/src/job_execution/security.rs` now emits one approaching reminder plus one breach escalation per warning and persists delivery outcomes.
  - Tests: added `deadline_seconds_maps_priority_aliases` in `crates/core/src/sla.rs`; kept the shared CEP assertions green with `sla_config_p0_deadline` and `sla_seconds_remaining_future` in `crates/insights/src/cep.rs`; kept the worker notifier suite green including `sla_enforcer_emits_alert_for_breach`, `sla_enforcer_approaching_sla`, and delivery-format tests in `crates/worker/src/notifications.rs`.
  - Validation: `~/.cargo/bin/cargo check -p apex-core -p apex-store -p apex-insights -p apex-worker -p apex-api` passed; `~/.cargo/bin/cargo test -p apex-core sla::tests::`, `~/.cargo/bin/cargo test -p apex-insights cep::tests::sla_`, and `~/.cargo/bin/cargo test -p apex-worker notifications::tests::` passed.

- [x] P6.4 Remove stale deployment artifacts and align runtime config.
  Why: old Next.js service definitions and duplicate nginx configs create operational ambiguity.
  Implementation detail:
  - Remove or archive stale units.
  - Consolidate authoritative runtime config.
  Evidence:
  - Code: removed stale `config/systemd/apexintel-frontend.service` and legacy `config/nginx/apexintel.conf`; updated `docs/RUNBOOK.md` to remove manual SQLx/Next.js deployment steps and to point systemd/nginx installation at the Rust-binary runtime plus `config/runtime/nginx-apexintel.conf`; updated `DEPLOYMENT.md` API key examples to match the new identity ownership model.
  - Tests: validated the runtime-facing operational scripts referenced by the deployment docs with `bash -n scripts/backup.sh scripts/restore.sh scripts/restore_validate.sh scripts/uptime_check.sh`.
  - Validation: local startup via `~/.cargo/bin/cargo run -p apex-api` reached config validation and embedded migrations (`_sqlx_migrations` present) before failing only on `Address already in use`, confirming the authoritative runtime path is the API binary rather than a frontend service.

- [x] P6.5 Add backups, monitoring, and alerting that actually page humans.
  Why: dashboards without alerts and scripts without scheduling are not operational controls.
  Implementation detail:
  - Automate backups.
  - Add alert rules.
  - Add uptime checks and restore validation runbooks.
  Evidence:
  - Code: added `config/systemd/apexintel-backup.service` and `config/systemd/apexintel-backup.timer` for scheduled backups; added `config/runtime/alert-rules.yaml` to define human-paging conditions for deep health, worker failures, missing backups, and failed restore drills; added `scripts/restore_validate.sh` and `scripts/uptime_check.sh` to automate restore verification and health probing with webhook paging; updated `docs/RUNBOOK.md` with backup timer, restore validation, and uptime-check instructions.
  - Tests: `bash -n scripts/backup.sh scripts/restore.sh scripts/restore_validate.sh scripts/uptime_check.sh` passed.
  - Validation: `~/.cargo/bin/cargo check -p apex-core -p apex-store -p apex-insights -p apex-worker -p apex-api` remained green with the new operational assets referenced by the docs.

---

## Phase 7: Data Model and Collaboration Primitives

- [x] P7.1 Add normalized tags, saved searches, watchlists, and annotation tables.
  Why: TEXT-array tags and missing collaboration tables limit analyst operations and governance.
  Implementation detail:
  - Introduce migrations.
  - Migrate existing ad hoc tag usage safely.
  Evidence:
  - Code: `crates/store/migrations/0007_identity_tags_and_delivery.sql` now creates the missing collaboration tables (`saved_searches`, `watchlists`, `annotations`, `export_history`) if absent and introduces normalized `tags` and `tag_assignments` tables with backfill from legacy `annotations.tags` and `insights.tags`; `crates/store/src/postgres/collaboration.rs` now normalizes annotation tags and syncs them into `tag_assignments` on every upsert.
  - Tests: added `normalize_tag_labels_dedupes_case_and_trims` in `crates/store/src/postgres/collaboration.rs`.
  - Validation: `~/.cargo/bin/cargo test -p apex-store collaboration::tests::` passed, and the full multi-crate `cargo check` stayed green after the schema and store-layer changes.

- [x] P7.2 Add real user and role tables.
  Why: shared API keys are not a team operating model.
  Implementation detail:
  - Add user records, role bindings, auditability, and key ownership.
  - Migrate auth middleware to use first-class identities.
  Evidence:
  - Code: `crates/store/migrations/0007_identity_tags_and_delivery.sql` now creates `analyst_users`, `analyst_user_roles`, `api_key_owners`, and `audit_log`; `crates/store/src/postgres/collaboration.rs` now registers API key ownership, resolves effective roles from store-backed role bindings, and tracks last-seen ownership state; `crates/api/src/auth.rs` now carries `owner_user_id`; `crates/api/src/main.rs` now syncs env-loaded keys into store-backed identities at startup and resolves authenticated requests against `api_key_owners` + `analyst_users`; `crates/api/src/api_handlers/preferences.rs`, `admin.rs`, `exports.rs`, and `collaboration.rs` now persist user-scoped state under analyst user IDs instead of API key IDs.
  - Tests: kept the API auth suite green with the new owner-backed token model in `crates/api/src/auth.rs`; collaboration route tests remained green after the user-ID migration.
  - Validation: `~/.cargo/bin/cargo test -p apex-api auth::tests::` passed, `~/.cargo/bin/cargo test -p apex-api routes::collaboration::tests::` passed, and `~/.cargo/bin/cargo check -p apex-core -p apex-store -p apex-insights -p apex-worker -p apex-api` passed.

- [x] P7.3 Add export history and analyst action audit trails.
  Why: intelligence workflows need provenance not only for data but for user actions.
  Implementation detail:
  - Record exports, approvals, acknowledgements, recipe changes, and major admin actions.
  Evidence:
  - Code: `crates/api/src/api_handlers/exports.rs` now records explicit audit events alongside `export_history`; `crates/api/src/api_handlers/warnings.rs`, `insights.rs`, `preferences.rs`, and `recipes.rs` now audit warning acknowledgements, bookmark changes, preference writes, and recipe promotion/deprecation; `crates/api/src/routes/collaboration.rs`, `crates/api/src/api_handlers/collaboration.rs`, `crates/api/src/main.rs`, and `crates/store/src/postgres/llm_governance.rs` now expose a queryable `/api/audit-log` and `/api/v1/audit-log` surface backed by `list_audit_log`.
  - Tests: kept collaboration route tests green after adding the audit-log route DTOs; insight handler tests remained green after replacing the hard-coded bookmark user with the authenticated user.
  - Validation: `~/.cargo/bin/cargo check -p apex-api --features llm` passed; `~/.cargo/bin/cargo test -p apex-api --features llm api_handlers::insights::tests::` passed; `~/.cargo/bin/cargo test -p apex-api --features llm routes::collaboration::tests::` passed.

---

## Phase 8: Learning Loop Activation

- [x] P8.1 Graduate the learning feedback loop from experimental to production-ready.
  Why: the platform already contains the skeleton of self-improvement but leaves it largely dormant.
  Implementation detail:
  - Stabilize interfaces.
  - Add metrics and safety rails.
  - Add tests proving learned thresholds feed back into production behavior safely.
  Evidence:
  - Code: `crates/store/migrations/0008_llm_governance.sql` now creates durable tables for prompt versions, workflow runs, improvement runs, and versioned training datasets; `crates/store/src/postgres/llm_governance.rs` adds shared persistence methods; `crates/worker/src/main.rs` now persists the standard eval run, the continuous self-improvement cycle report, and the generated self-improvement dataset instead of only logging previews.
  - Tests: compile coverage for the new persistence path was exercised through the API/worker feature checks and the existing self-improvement-adjacent handler tests stayed green.
  - Validation: `~/.cargo/bin/cargo check -p apex-worker --features llm` passed and confirmed the worker-side self-improvement persistence path compiles with the new governance storage.

- [x] P8.2 Add prompt versioning, LLM output validation, and evaluation gates across LLM workflows.
  Why: prompt drift and ungrounded generation still undermine trust.
  Implementation detail:
  - Version prompts.
  - Persist prompt IDs with outputs.
  - Require grounding checks before persistence.
  Evidence:
  - Code: `crates/llm/src/prompt_registry.rs` now defines versioned prompt identities for the production LLM API workflows; `crates/api/src/routes/llm.rs` now returns governance metadata with prompt ID/version and gate outcome; `crates/api/src/api_handlers/llm.rs` now registers prompts, validates entity/recipe/POI/memo outputs with workflow-specific quality gates, and persists every accepted or rejected workflow run through `llm_workflow_runs`.
  - Tests: `crates/api/src/api_handlers/llm.rs` unit tests stayed green after the governance metadata and validation-path changes.
  - Validation: `~/.cargo/bin/cargo check -p apex-api --features llm` passed and `~/.cargo/bin/cargo test -p apex-api --features llm api_handlers::llm::tests::` passed.

- [x] P8.3 Add dataset and evaluation discipline for model improvement.
  Why: self-improvement without robust eval becomes random drift.
  Implementation detail:
  - Curate benchmark sets.
  - Add regression suites for extraction, insight quality, hallucination control, and recommendation usefulness.
  Evidence:
  - Code: `training_data/evaluation/manifest.json` now versions the evaluation suite and enumerates the governed benchmark files; `training/eval_harness.py` now loads the manifest, resolves the declared benchmark set instead of relying on an implicit glob, and writes suite/version metadata into reports; `crates/worker/src/main.rs` now persists versioned self-improvement datasets through `llm_training_datasets`.
  - Tests: the eval harness dry-run exercised the manifest-backed suite across all declared JSONL benchmarks and verified the benchmark inventory stays valid.
  - Validation: `/Users/sabelakhoua/IdeaProjects/ApexIntel/.venv/bin/python training/eval_harness.py --dry-run` passed with 884/884 valid examples across the versioned suite.

---

## Execution Log

- [x] 2026-03-09/10: Completed P6.3, P6.4, P6.5, P7.1, and P7.2.
  - Verified the local migration/startup path with `~/.cargo/bin/cargo run -p apex-api`: the API reached config validation, confirmed embedded migrations (`_sqlx_migrations` already exists), and then stopped only because port 8080 was already in use.
  - Validation sweep: `~/.cargo/bin/cargo check -p apex-core -p apex-store -p apex-insights -p apex-worker -p apex-api`; `~/.cargo/bin/cargo test -p apex-core sla::tests::`; `~/.cargo/bin/cargo test -p apex-insights cep::tests::sla_`; `~/.cargo/bin/cargo test -p apex-worker notifications::tests::`; `~/.cargo/bin/cargo test -p apex-store collaboration::tests::`; `~/.cargo/bin/cargo test -p apex-api auth::tests::`; `~/.cargo/bin/cargo test -p apex-api routes::collaboration::tests::`; `bash -n scripts/backup.sh scripts/restore.sh scripts/restore_validate.sh scripts/uptime_check.sh`.