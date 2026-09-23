# Changelog

All notable changes to ApexIntel are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versions correspond to internal fix-batch identifiers (B### = backend fix, U### = UI fix).

---

## [Unreleased] — Full-platform audit batch (B290–B351)

Outcome of a line-by-line audit of all 18 crates plus the web UI, benchmarked
against commercial CI platforms (Klue, Crayon, Kompyte, Contify, AlphaSense,
Recorded Future, OpenCTI). Theme: make the product actually usable — every
core workflow (search, exports, charts, creation forms, intelligence
generation) had at least one defect that broke or discredited it.

### Security

- **B290** Browser sessions now authenticate `/api/*` requests as Analyst-level
  principals (cookie + double-submit CSRF on unsafe methods). Every UI button
  that linked to the API — CSV/PDF exports, activity charts, graph
  neighborhood/path, alert settings, battlecard regeneration — previously
  failed with 401 because the API only accepted Bearer keys.
- **B292** `require_admin` middleware enforces admin role on `/api/admin/*`
  and destructive warning operations (single delete, bulk delete) —
  previously Viewer/Analyst keys sufficed.
- **B293** WebSocket `/ws/warnings` origin checking is now a real same-origin
  check instead of a no-op that accepted every origin.
- **B294** WebSocket tokens are validated: any non-empty `?token=` value used
  to grant a live alert stream. API keys or a valid session are now required.
- **B295/B296** SSE/WebSocket connections register under per-connection UUIDs
  and unregister on disconnect (leak fix; nil-UUID-for-everyone previously).
- **B299** `/metrics` moved behind API auth — it exposed platform scale and
  ran DB aggregates per scrape, unauthenticated.
- **B301** XSS closed at five `Html(format!())` sites (warning/insight titles,
  review notes, reflected scan-type errors, acknowledge echo) via a shared
  `escape_html` helper.
- **B298** Rate limiter keys on the client socket address instead of the
  spoofable `X-Forwarded-For` (honored only with `API_TRUST_PROXY=1`).
- **B317** All 50 documented `API_KEYS_ENV_SLOTS` load (was hardcoded 16).
- **B318** CSV exports neutralize spreadsheet formula injection (`=`, `+`,
  `-`, `@` prefixes get a guard apostrophe).
- **B349** Service worker no longer caches authenticated HTML — analyst pages
  used to persist in Cache Storage across logout on shared machines.
- **B297** CORS allows PATCH (13 registered PATCH routes previously failed
  every cross-origin preflight).

### Fixed — API & web

- **B300** `/api/trends*` no longer 500s: the handlers' `Extension<PgStore>`
  was provided only to the web-page router (all three endpoints were
  dead-on-arrival).
- **B302** Live search works: the HTMX branch of `/search` returned an HTML
  comment placeholder, so typing in the search box wiped the results area.
  A real results partial (facets + list + pager) is now rendered.
- **B303** Search facets return results (templates emitted plural slugs —
  `companies` — while the index stores singular types) and show real counts
  instead of permanent zeros. Query input is sanitized for the Tantivy
  parser; pagination controls are rendered.
- **B307** `POST /recipes/create-form` registered — the only creation form in
  the product 404'd on submit. Validates JSON sections, derives a unique
  recipe code, stores as `staging`.
- **B308** `GET /workspaces/new` renders an actual creation form (previously
  re-rendered the empty list; workspace creation was impossible via UI).
- **B309** Warning detail entity links use `/companies/:id` (the templated
  pluralization produced `/companys/…` → 404).
- **B310** Warning badge updates: duplicated `notification-badge` ids replaced
  with classes (both mobile and desktop badges now update), and the refresh
  poll hits the existing `/warnings/unread-count` route instead of a
  nonexistent API path.
- **B311** Entity activity chart "Insights" series now reads from a new
  `get_daily_insight_counts_per_entity` aggregate — it was previously fed
  warning counts (two identical mislabeled lines).
- **B312/B313** Executive page: competitor names come from the tracked
  competitor set instead of a hardcoded demo list; week-over-week topic
  deltas are computed from actual insight timestamps instead of a fabricated
  `count × 7.5%`; recommended actions use the real 0–1 priority scale (the
  old `>= 7.0` cutoff filtered everything, leaving the module permanently
  empty).
- **B314** Battlecard list API derives threat level / win probability from
  card content and omits them when unknown, instead of hardcoding
  `"medium"` / `0.5` / `0` for every card.
- **B315** Company list shows real per-entity warning/insight counts (two
  GROUP BY queries) instead of zeros.
- **B316** Admin page shows real telemetry (process uptime, pool size, DB
  size via `pg_size_pretty`, live ingestion volume/freshness per observation
  type). The fabricated tiles (12 DB connections, 87% cache hit, 4 workers,
  hardcoded source lists, fake endpoint metrics) are gone.
- **B305** Autocomplete index lock poisoning no longer 500s every suggest
  request (recovers via `into_inner`).

### Fixed — worker & pipeline

- **B320** Due jobs execute concurrently under `WORKER_MAX_CONCURRENT_JOBS`
  (default 4). The strictly sequential tick let one long job starve SLA
  enforcement, triage, and digest delivery for hours.
- **B322** Declared per-job timeouts are enforced (`tokio::time::timeout`);
  a hung LLM/HTTP/DB call records a failure instead of wedging the scheduler.
- **B323** Job panics are contained and recorded as Failed runs (previously
  the tick task unwound silently with no history, no failure count, no
  circuit-breaker tick).
- **B319** Scheduled jobs claim a DB lease (`try_claim_scheduled_job`) so
  multiple worker replicas cannot double-fire; crashed runs release after a
  2× timeout lease instead of blocking the job forever.
- **B324** SIGTERM handled with graceful drain (Docker/K8s stop signals were
  previously ignored; the worker was killed mid-job).
- **B326** Content-derived observation IDs (UUIDv5 over stable content keys)
  for the recurring ingest paths: crawl-cycle web changes, CVE, OpenAlex,
  RDAP, SEC filings, DNS posture, social posts, dark-web posts, threat-actor
  matches, supply-chain heuristics. Every one of these previously re-inserted
  identical rows every cycle — unbounded table growth and duplicate signals
  inflating every downstream count, anomaly detector, and pattern miner.
- **B327** Dark-web scan only warns on first store of a post (deduped
  re-scans previously re-warned every 6 h).
- **B328** KEV catalog is actually persisted (downloaded, counted, and
  discarded before) as `VulnNotice` observations with stable IDs.
- **B329** Lookalike-domain scan verifies DNS registration before asserting a
  threat; unregistered typosquat variants are no longer persisted as
  active 0.8-confidence threats.
- **B330** CRM competitor extraction fixed (uppercase check ran against
  lowercased text → always empty) with stopword filtering.
- **B331** Contact enrichment: Hunter name params URL-encoded and API key
  moved to header; Apollo uses `X-Api-Key`; provider failures log loudly;
  single-token names no longer anchor-match every mailto on a page.
- **B332** POI LLM enrichment bounded (default 40/run; was unbounded
  `i64::MAX` — worst case days of sequential 90 s LLM calls in a 1 h job).
- **B333** Email digest loop continues past per-recipient failures (one bad
  address previously cancelled everyone else's digest).
- **B334** Tender scan byte-slicing is char-boundary-safe (MENA Arabic/French
  content previously panicked the scan with "byte index is not a char
  boundary").
- **B335** Insight generation reads crawled text from `observations.value`
  JSON keys — the previous query selected a nonexistent `content` column and
  errored at runtime, so LLM insights never saw any crawl text.
- **B336** Insights store real evidence URLs from observation provenance
  (narratives with `[1][2]` citation markers previously shipped with zero
  sources).
- **SQL fix** anomaly scan precedence: `(A AND B) OR C` parsed as
  `A AND (B OR C)`, bypassing the 30-day filter and full-scanning the
  observations table.

### Fixed — intelligence quality

- **B337** Battlecard feature matrix uses symmetric support levels for both
  sides — the "Us" advantage arm was dead code and every card's summary read
  "0 advantage us".
- **B338** Predictive patterns are labeled honestly: the hardcoded patterns
  invented precision/recall/CI numbers and observation counts ("95% CI …
  based on 150 historical observations") with no dataset anywhere in the
  codebase. They are now explicit heuristic priors
  (`observation_count == 0`) and render as such; calibrated statistics only
  print for data-derived patterns.
- **B339** Entity attribution no longer hardcodes NVIDIA as the beneficiary
  of all AI/GPU/semiconductor signals.
- **B340** Entity matching requires forward containment (text mentions the
  entity name); the previous reverse check matched arbitrarily whenever an
  entity name contained the signal text.
- **B341** Dynamically discovered entities pass their own relevance gate —
  an explicit name mention now floors relevance above threshold (previously
  max achievable ~0.25 < 0.3 for thin profiles, so the pipeline rejected
  insights about entities it had just discovered).
- **B342** Diversity selection no longer terminates after ~2×categories
  iterations total; it now runs full rounds until satisfied or exhausted.
- **B343** Semantic dedup actually compares embeddings: the code computed an
  embedding and then searched by raw text, so the 0.92 threshold was checked
  against trigram-Jaccard scores that essentially never reach it —
  near-duplicate detection never fired. Embeddings are now stored and
  compared by cosine similarity (with text fallback for legacy items).

### Fixed — UI/UX

- **B344** Amber is amber: the `--chart-series-amber` token and
  `.metric-rail-amber` aliased violet, so any chart distinguishing the two
  was showing one color.
- **B345** Missing CSS classes defined (`.apex-select`, `.apex-panel`,
  `.apex-state-panel`, toast severity variants); missing sprite icons added
  (edit, trash, share, plus, list, folder, file, map-pin, layers, settings).
- **B346** Arrow-key row navigation implemented for `[data-dense-table]`
  tables (nine templates advertised it; no script existed).
- **B347** Destructive actions (close workspace, complete queue item,
  triage acknowledge/resolve) ask for confirmation.
- **B348** Severity colors consistent across pages (dashboard now matches
  warnings/insights: High=orange, Medium=amber).
- **B350** Graph page renders theme-aware stage colors in dark mode.
- Admin nav entry added to the sidebar (the page was unreachable);
  triage breadcrumbs render (block name mismatch fixed); duplicate
  `main-results` ids removed from competitors/memos pages.
- **B351** Stale `test_agent_system_prompts` assertion updated to the
  grounding contract the hardened prompts actually guarantee.

### Fixed — fresh-database bootstrap (B352/B353)

A brand-new PostgreSQL instance could not complete `sqlx migrate run` — the
pipeline aborted at seven distinct defects, in order:

- **B352** `persons."current_role"` quoted — the bare name is a reserved
  word on PostgreSQL ≤14.
- **B353** `recipes` table moved above `recipe_weekly_metrics` (the latter's
  FK referenced a table created 70 lines later).
- **B353** `global_alert_defaults` seed uses `OVERRIDING SYSTEM VALUE`
  (explicit id into a `GENERATED ALWAYS` column).
- **B353** test-seed data aligned with CHECK constraints (`technological` →
  `market`; `workspace` → `investigation` entity types).
- **B353** `RETURN NEXT`-style validation functions switched from
  `RETURNS TABLE` to `RETURNS SETOF TEXT`.
- **B353** 15 migrations shared version numbers (sqlx requires unique);
  the set is renumbered to a deterministic `000`–`043` sequence preserving
  execution order.
- **B353** RLS migration hardened: `ALTER TABLE … ENABLE ROW LEVEL SECURITY`
  and owner-scoped policies guard on table/column existence (several target
  tables are created by later migrations); the 4096-dim ivfflat index and
  the `now()` partial-index predicate are guarded/IMMUTABLE-safe; the
  `title_hash`/`entity_ids` performance indexes guard on column existence;
  the duplicate `sentiment_time_series` index set guards on schema shape.

**Verified**: `apex-api` now bootstraps an empty database (migrations
applied, server listening) and every page + key API route was smoke-tested
live (login, all 20 pages 200, session-authenticated `/api/*` 200,
admin routes correctly 403 for Analyst sessions, recipe creation form
persisting a `staging` recipe end-to-end with CSRF).

### Fixed — discovered during the production deployment (B355–B358)

- **B355** Worker jobs no longer pass job-source tags (`"anomaly_scan"`,
  `"breach_scan"`, `"crawl_cycle"`, …) as `insert_warning`'s FK-constrained
  `recipe_code` parameter — 9 call sites produced nothing but FK violations
  on real data.
- **B356** `list_companies` (and the by-region/by-type variants) now SELECT
  `is_competitor`, which `CompanyRow` decodes — every company list errored
  with "no column found for name: is_competitor" on any non-empty table.
  Same class: `competitor_changes.impact_score` decoded as f32 against a
  FLOAT8 column → f64.
- **B357** Insight generation reads `observations.entity_id` directly. The
  previous query joined `observation_entity_graph` — a table with **no
  writer anywhere in the codebase** (0 rows in production) — so the join
  was always empty and LLM insight generation silently skipped every run.
  Verified live: the loader went from 0 → 14 companies and llama-server
  began evaluating 4.2k-token insight prompts.
- **B358** `LLM_INSIGHT_MAX_COMPANIES` (default 10) bounds the per-run LLM
  pass; `LLM_TIMEOUT_SECS=600` set in production for CPU-speed 30B
  inference (the 180 s default expired mid-prompt).

---

## [Previous batches]

### Security

- **B283** `#[serde(deny_unknown_fields)]` added to every API input struct in `crates/api/src/`.  Unknown JSON keys now return a 422 rather than being silently dropped, preventing field-injection attacks.
- **B199** `ModelConfig::redacted_api_key()` — API keys are never logged in plaintext; only the first 4 and last 4 characters are shown in logs.
- **B193** Robots.txt checked before every crawl request (fail-closed: disallowed if unreachable).

---

### Added

#### Pipeline & Orchestration

- **B269** End-to-end nightly pipeline integration tests (7 new tests in `nightly.rs`):
  - Happy-path item/error aggregation across all four stages.
  - `should_proceed()` blocks Mining when Crawl fails.
  - Pipeline health formula verified (succeed=1.0, skip=0.5, fail=0.0 per stage).
  - Run-ID uniqueness across stages.
  - `finished_at ≥ started_at` invariant.
  - All-skipped → `overall_success = false`.
  - Realistic error-count path (1 000 crawl failures → PARTIAL FAILURE).

- **B285** Batch/memory logging added to nightly and weekly pipelines.  Every stage now emits structured `tracing` spans with item counts, error rates, and memory deltas.

- **B286** `MAX_BATCH_SIZE` constant enforced in nightly and weekly pipeline runners.  Stages that exceed the ceiling return an early error outcome rather than processing unbounded data.

#### Validation & Input Safety

- **B276** `crates/core/src/validation.rs` created with centralised helpers:
  - `validate_non_empty(field, value)` — rejects blank/whitespace-only strings.
  - `validate_length(field, value, max)` — rejects strings exceeding `max` bytes.
  - `validate_url(url)` — rewrites through `normalize_url()` and rejects malformed input.
  - `validate_email(email)` — RFC-compliant regex check.
  - `validate_date_range(from, to)` — ensures `from ≤ to`.

- **B279** `validate_tags()` in `store/postgres.rs` — tags capped at 64 Unicode code points before any INSERT.

- **B280** `safe_concat(parts, separator, max_len)` in `crates/core/src/validation.rs` — checks combined length before allocating to prevent unbounded string growth.

- **B291** Numeric range validation helpers: `validate_positive_f64`, `validate_non_negative_i64`, `validate_bounded_f64(min, max)`.

#### API

- **B284** `map_apex_error(err)` centralised error-to-HTTP mapping in `crates/api/src/error.rs`.  All route handlers delegate to this function; no ad-hoc `StatusCode` literals.

- **B272** `# Examples` sections added to all public functions in `crates/api/src/responses.rs` (`success()`, `error_response()`, `aggregate_health()`, `ApiResponse`, `PagedResponse`).

#### Testing

- **B266** Golden JSON tests in `nightly.rs` and `weekly.rs` — serialised `NightlyReport` and `WeeklyMemo` must match pinned JSON fixtures.

- **B267** Adversarial fuzz tests in `crates/parse/src/normalizer.rs` (~40 tests) covering Unicode surrogates, null bytes, RTL override characters, excessively long inputs, and empty strings.

- **B268** Criterion benchmarks in `crates/graph/benches/adjacency_bench.rs` for BFS, DFS, and shortest-path on 100/1 000/10 000-node graphs.

- **B270** Unit-test coverage gaps closed:
  - 8 new tests for `BetaUpdater::variance()`, `total_observations()`, and `bayes_factor()` in `crates/stats/src/bayesian.rs`.
  - 3 new tests for `SiteType::as_str()` (all variants including `Other`), `RoleFamily::Other` round-trip, and `PriorityVector::dominant()` field sensitivity in `crates/core/src/entities.rs`.

- **B287** Empty/zero-input tests added for all public functions with collection or string parameters.

#### Documentation

- **B271** Comprehensive `///` doc comments added to all public structs and enums in:
  - `crates/worker/src/nightly.rs` (7 types)
  - `crates/worker/src/weekly.rs` (8 types)
  - `crates/worker/src/scheduler.rs` (`JobSummary`)
  - `crates/core/src/entities.rs` (18 types)
  - `crates/llm/src/lib.rs` (5 types)

- **B273** `README.md` created for every crate:
  [`core`](crates/core/README.md),
  [`api`](crates/api/README.md),
  [`worker`](crates/worker/README.md),
  [`graph`](crates/graph/README.md),
  [`parse`](crates/parse/README.md),
  [`insights`](crates/insights/README.md),
  [`poi`](crates/poi/README.md),
  [`recipes`](crates/recipes/README.md),
  [`stats`](crates/stats/README.md),
  [`llm`](crates/llm/README.md),
  [`store`](crates/store/README.md),
  [`crawl`](crates/crawl/README.md),
  [`learning`](crates/learning/README.md).

- **B275** LLM provider compatibility table added to `crates/llm/src/lib.rs` module doc.  Covers LlamaCpp vs OpenAI vs AzureOpenAI for: base_url format, api_key handling, model name, streaming support, function calling.

#### Graph & Algorithms

- **B238** `adjacency.rs` — BFS/DFS return sorted, deterministic node lists (B292).
- **B245** `shortest_path` returns `None` instead of panicking when source or target are absent from the graph.

#### Learning & Mining

- **B220** `DEFAULT_WINDOW_DAYS = 30` — default observation window for contingency tables.
- **B222** `MAX_SWEEP_LAG_DAYS = 365` — hard ceiling on sweep lag to bound runtime.

#### Store

- **B190** `ilike_pattern()` escapes `%`, `_`, `\` in user search terms before ILIKE queries.
- **B191** `clamp_limit(limit)` caps all list queries at `MAX_LIST_LIMIT = 500`.

---

### Changed

- **B284** All API route handlers now use `map_apex_error` instead of inline `StatusCode` selection.
- **B283** API input structs reject unknown fields (`deny_unknown_fields`) — **breaking change** for clients sending extra JSON keys.
- **B199** Logging format for LLM configs now always redacts API keys.

---

### Fixed

- **B001–B189** (prior sessions) — comprehensive bug fixes across validation, parsing, graph algorithms, pipeline orchestration, and API response formatting. See individual commit messages for details.

---

### Removed

*(nothing removed in this batch)*

---

## Versioning note

ApexIntel uses date-based internal releases.  Public semantic versioning will be introduced after the initial production launch.
