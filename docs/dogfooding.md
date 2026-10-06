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
