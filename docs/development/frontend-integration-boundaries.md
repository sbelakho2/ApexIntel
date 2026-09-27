# Frontend Integration Boundaries

Date: 2026-03-13 (updated 2026-09-27)

This repository has **one shipped frontend surface**:

- **`crates/api/`** is the Axum server that serves both the REST API and the
  server-rendered HTML UI via Askama templates + HTMX. It owns every
  browser-facing route, the `<base.html>` layout, shared macros, all
  template-based pages, and the server-side SVG chart fragments.

The Leptos/WASM single-page application that used to live in the removed
`frontend` crate was retired from the workspace and archived (unbuilt,
untested, unshipped) under `experiments/wasm-frontend/`. See
[`experiments/README.md`](../experiments/README.md).
`scripts/ci/check_frontend_removed.sh` fails if it is referenced by the
workspace, CI, tests, Docker, or package scripts again.

API integration expectations:

- Browser-facing API consumers should target `/api/*` routes.
- Machine-readable contract discovery lives at `/api/openapi.json`.
- Capability discovery lives at `/api/features`.
- Human-oriented API discovery lives at `/api/docs`.

Ownership guidance:

- Changes to HTTP base paths, auth expectations, or response contracts should be
  validated against `crates/api/` — it is the only integration owner.
- Visualization work belongs in `crates/api/templates/macros.html` or a
  server-side SVG builder such as
  `crates/api/src/api_handlers/charts.rs`. There are no Rust/WASM chart
  components to update.
- Design tokens (`--rams-*`) are defined once in
  `crates/api/static/css/globals.css`.
- Documentation must not describe a second frontend surface; if WASM relocation
  era work is referenced, point at `experiments/wasm-frontend/` and mark it
  unsupported.

### Directory Structure

```
crates/
└── api/                      # Axum server (REST API + HTML UI)
    ├── src/
    │   ├── api_handlers/     # JSON API endpoint handlers + SVG chart builders
    │   ├── routes/           # API route registration
    │   └── web/              # HTML page handlers (one per page module)
    ├── templates/            # Askama templates
    │   ├── base.html         # Base layout (sidebar, header, content)
    │   ├── macros.html       # Shared component macros
    │   ├── pages/            # Full-page templates
    │   └── partials/         # HTMX partial templates
    └── static/               # Static assets (CSS, JS, fonts, icons)
        ├── css/globals.css   # Tailwind + Rams CSS variables
        ├── css/tailwind.css  # Compiled Tailwind output
        ├── js/app.js         # Client-side JS
        └── js/htmx.min.js    # HTMX library
```
