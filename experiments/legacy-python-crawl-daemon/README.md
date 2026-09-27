# Legacy Python crawl daemon (archived — tooling only)

`crawl_daemon.py` was the original standalone acquisition service (aiohttp +
asyncpg + BeautifulSoup, deployed as a systemd unit on the Hetzner VPS). It is
**not** part of production anymore.

## Decision (2026-09-27)

There is exactly one acquisition implementation: the Rust pipeline
`crates/crawl` (per-source clients, politeness, robots, rate limits) driven by
`crates/worker` (scheduling, leases, retries, dead letters). Keeping a second,
independent crawler in Python duplicated source logic, bypassed the shared
warning ingress / dedup / outbox guarantees, and could not be covered by the
workspace CI.

The daemon is kept here, outside the workspace, because
`scripts/inspect_prod_veracity_candidates.py` reuses its *offline* veracity
scoring helpers for production audits. It must never be deployed, scheduled, or
used to insert data into the database.

## Files

- `crawl_daemon.py` — the archived daemon; its scoring functions are imported by
  the audit tool.
- `test_crawl_daemon_quality.py` — regression tests for those scoring helpers.
  Run from this directory: `python3 test_crawl_daemon_quality.py` (requires
  `pip install -r requirements.txt` first).
- `requirements.txt` — dependencies for the archived daemon/tests only; the
  shipped Rust binaries need none of them.

## Re-adding a Python crawler is a non-goal

New sources go through `crates/crawl` and are registered in the source registry
so coverage, validation, and failure semantics stay in one place.
