# Dogfooding — repo-wide adversarial self-testing

Dogfooding exists because pipelines fail silently. The 2026-10-06 incidents
proved the point: 78 sources were mass-403-blocked for weeks, "silent for N
days" warnings fired for healthy quiet feeds, and `ON CONFLICT DO NOTHING`
no-ops were counted as ingestion — all while every unit test was green. The
dogfood framework asserts *behavior and honesty*, not just compilation.

Two arms, both assert-based (any violation exits non-zero):

## 1. Analytical dogfood (CI, deterministic)

`crates/worker/examples/analytical_adversarial_dogfood.rs`

- 12 handcrafted attacks: fabricated statistics, number manipulation,
  temporal fabrication, citation injection, poisoned corroboration,
  single-source overclaim, ignored contradictions, shallow filler,
  recommendation smuggling, org hallucination, red-team attacks — plus the
  deep-dossier control that must publish.
- 400 mutation-fuzz iterations over a clean dossier with monotonicity
  assertions (corruption may never raise released confidence).
- Optional live probe (`APEX_DOGFOOD_LIVE=1`) against a real model.

## 2. Source-pipeline dogfood (CI invariants + ops live audit)

`crates/worker/examples/source_pipeline_dogfood.rs`

**CI mode (default, no network/DB):**
- registry floors (merged registry and supplement may only grow; a silent
  source loss is a hard failure),
- 0 excluded sources, unique slugs, IntelSlavaZ present, ≥12 telegram
  channels,
- onion sources route through the Tor path; all clearnet endpoints http(s),
- UA policy defaults to browser-class (`apex_crawl::fetch_policy`).

**Ops mode (`--db <DATABASE_URL>`), asserting against real state:**
- capability gaps (`unavailable:`) never accumulate failure counts,
- **UA-policy regression check**: no new 403/auth-required attempts after the
  policy deploy cutoff (default 2026-10-06T14:20Z, override `--since`),
- impossible states (feed freshness newer than last success),
- **detection cross-check**: every post-cutoff `source_outage` warning must
  name a source with a genuinely fresh feed (ingestion stall); every
  `source_fetch_failure` must match a runtime row with ≥3 failures and a real
  error; no warning title may contain day-count/"silent for" wording.

**Live mode (`--live --sample N`):** paced sample across the registry fetched
with the real fetch policy; asserts a reachability floor and reports the
unreachable set with errors.

`crates/crawl` unit tests lock the mechanisms the audits rely on:
- `fetch_policy` tests (browser default, bot override),
- mock-server test: a UA-filtering host 403s the bot UA and 200s the browser
  UA — the exact production failure class,
- robots-denied fetches are identifiable as robots errors,
- registry invariant tests (floors, enabled-only, supplement coverage).

## 3. Whole-repo coverage manifest (jobs + subsystems)

`crates/worker/src/dogfood.rs` registers **every worker job** (all 44
`JobKind` variants) and **every subsystem** with how each is dogfooded:

- `Live` — scheduled; the audit asserts the latest terminal run is within the
  job's own cadence (`max_age_hours`), its status is not `failed`, the failure
  streak is below the circuit policy, and the circuit is closed.
- `Manual` — trigger-only (insight analysis, supplier pricing); asserted for
  failure streaks/circuit, execution proven by the trigger path's tests.
- `SubStep` — runs inside a parent job (hypothesis generation under pattern
  mining); asserted through the parent.
- `IdleOk` — legitimately idle when unconfigured or when queues are empty
  (StarzCRM sync, empty triage); skips are healthy.
- `Ci` — enforced by CI gates (browser suites, static guards).

Enforcement so the manifest cannot drift:
- `scheduler.rs::dogfood_coverage_tests::all_job_kinds_match_the_dogfood_manifest`
  asserts `ALL_JOB_KINDS` round-trips and that the manifest covers exactly the
  enum's job set in both directions (a new job without a manifest row fails CI);
- the manifest's own tests assert unique names, non-empty checks, and a
  cadence for every Live job;
- `source_pipeline_dogfood --db` executes the live matrix on production.

This layer exists because the analytical and source dogfoods together still
missed whole subsystems: the 2026-10-06 audit found a KEV fetch circuit at 40
consecutive failures, a dark-web scan degraded forever for "no monitored
entities", a sanctions job hitting its 30-minute timeout as the portfolio
grew, and a crawl cycle wrongly reporting `failed` for upstream source state.

### Subsystems registered (23)
Source acquisition, source health detection, crawl scheduling, feed
parsing/ingest, dark web/Tor, sanctions screening, analytical pipeline,
insight generation, insight surfaces (retraction filtering), warning
lifecycle, recipes, learning/calibration, triage/feedback, search, API routes,
web UI, notifications/outbox, jobs/scheduler, store/schema,
observability, model portability, exports/artifacts, config/env contract —
see `SUBSYSTEMS` for the executable check behind each.

## Running

```bash
# CI arm (both harnesses)
bash scripts/ci/check_dogfood.sh

# Live audit on a host with DATABASE_URL and crawl egress (prod)
DATABASE_URL=... bash scripts/ops/source_health_audit.sh --live 40
# evidence: release-evidence/source-health/audit-<utc>.txt
```

## Incident classes → the check that catches them

| Incident class (2026-10-06 and before) | Catch |
|---|---|
| Bot UA 403s on 78 sources, hidden for weeks | `fetch_policy` + mock-server test; live audit reachability floor; ops `--db` UA-regression check |
| "Silent for N days" on healthy quiet feeds | detection cross-check (fresh-feed requirement); pg tests |
| Warning spam (title changed daily, occurrence 88+) | detection cross-check (no day-count wording); stable-title dedup |
| Supplement silently shrank by 48 sources | registry floors + supplement-coverage invariant |
| `ON CONFLICT DO NOTHING` counted as ingestion | insert-accounting contract (bool) + cycle new/duplicates log; feed accounting tests |
| Parsed items never ingested (zero new rows) | feed freshness (`last_item_at`) + ingestion-stall detection |
| robots denials retried on the failure ladder | robots mock test + 24h capability backoff |
| Legacy Python daemon still running, writing template insights ("X is appearing in N recent reports... If this matters commercially...") alongside the Rust pipeline | insight-quality audit check (banned pattern list over recent insights); daemon service stopped and disabled 2026-10-06, its 2 products retracted |
| Sanctions fuzzy matching flooding 607 warnings/day with pair-level false positives ("Mohammed Khalil -> MOHAMMED, Ali") | token-gated matcher (`name_match_score`, production pairs asserted in unit tests) + one consolidated warning per screened entity with a stable title |
| volume_anomaly/signal_shift storms from titles embedding per-run details (221 and 170/day) | stable per-entity titles, absolute-delta and baseline floors, generic observation-type filtering, warning-hygiene budgets in the audit |
