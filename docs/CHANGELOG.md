# Changelog

All notable changes to ApexIntel are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versions correspond to internal fix-batch identifiers (B### = backend fix, U### = UI fix).

---

## [Unreleased]

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
