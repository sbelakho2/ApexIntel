# Experiments (not built, not shipped, not supported)

Code in this directory is outside the Cargo workspace and outside every CI
build/deploy path. Nothing here is a product artifact, and no production or test
code may depend on it. It is kept only as reference for completed or retired
experiments.

## `wasm-frontend/` — retired Leptos/WASM SPA

- **Status:** retired 2026-09-27; was `crates/frontend` (package `apex-frontend`).
- **Why retired:** production has exactly one web surface — the server-rendered
  Askama + HTMX application in `crates/api`. The SPA duplicated route
  composition, API clients, and chart components without being deployed or
  covered by CI, so it drifted from the shipped UI.
- **What replaced it:** `crates/api/templates/` (Askama) + `crates/api/static/js/htmx.min.js`,
  styled by `crates/api/static/css/globals.css`. Chart rendering lives in the API
  handlers (e.g. `crates/api/src/api_handlers/charts.rs`).
- **Do not re-add:** `scripts/ci/check_frontend_removed.sh` fails if
  `crates/frontend` reappears in the workspace, in CI, in package scripts, or in
  any tracked file outside this directory.

## `legacy-python-crawl-daemon/` — retired second acquisition implementation

- **Status:** retired 2026-09-27; was `scripts/crawl_daemon.py` (plus its tests
  and `scripts/requirements.txt`).
- **Why retired:** production must have exactly one acquisition implementation.
  The Rust `crates/crawl` + `crates/worker` pipeline is it; the Python daemon
  duplicated source logic and bypassed shared dedup/ingress guarantees.
- **Why kept at all:** `scripts/inspect_prod_veracity_candidates.py` reuses the
  daemon's offline veracity-scoring helpers for audits. See
  [`legacy-python-crawl-daemon/README.md`](legacy-python-crawl-daemon/README.md).
- **Never deploy or schedule it.**

## `legacy-parity-tools/` — retired screenshot-parity capture/diff

- **Status:** retired 2026-09-27; was `scripts/parity_capture.py` and
  `scripts/parity_diff.py`.
- **Why retired:** they captured and diffed screenshots from the removed
  frontend parity directories (`frontend/e2e/parity/*`), which no longer exist.
- **What replaced it:** the server-UI visual specs
  (`e2e/server-ui-visual.spec.js`) and `scripts/ci/e2e_server_ui.mjs`.

