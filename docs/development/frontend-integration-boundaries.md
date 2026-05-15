# Frontend Integration Boundaries

Date: 2026-03-13 (updated 2026-05-14)

This repository has two frontend surfaces with different ownership boundaries:

- **`crates/frontend/`** is a Leptos/WASM single-page application. It owns interactive browser-facing route composition, API consumption from the server, and chart/graph visualization components. This is the canonical product web application.

- **`crates/api/`** is the Axum server that serves both the REST API and a server-rendered HTML UI via Askama templates + HTMX. It provides the `<base.html>` layout, shared macros, and all template-based pages.

API integration expectations:

- Browser-facing API consumers should target `/api/*` routes.
- Machine-readable contract discovery lives at `/api/openapi.json`.
- Capability discovery lives at `/api/features`.
- Human-oriented API discovery lives at `/api/docs`.

Ownership guidance:

- Changes to HTTP base paths, auth expectations, or response contracts should be validated against `crates/api/` first.
- Accessibility or visualization work that is specific to Rust/WASM chart components lives in `crates/frontend/src/components/charts/`.
- The server-rendered pages in `crates/api/templates/` share design tokens with the WASM frontend via matching `--rams-*` CSS variables.
- New documentation should explicitly name which frontend surface it refers to so `crates/frontend/` and `crates/api/` are not conflated.

### Directory Structure

```
crates/
├── frontend/                 # Leptos/WASM interactive SPA
│   ├── src/
│   │   ├── app.rs            # App shell, router, navigation
│   │   ├── api.rs            # API client (fetches from server)
│   │   ├── routes/           # Page components (one per route)
│   │   └── components/       # Shared UI components (cards, charts, badges, filters)
│   ├── style.css             # Rams design tokens + component styles
│   ├── index.html            # Entry point
│   └── Trunk.toml            # Trunk WASM bundler config
│
├── api/                      # Axum server (REST API + HTML UI)
│   ├── src/
│   │   ├── api_handlers/     # JSON API endpoint handlers
│   │   ├── routes/           # API route registration
│   │   └── web/              # HTML page handlers (one per page module)
│   ├── templates/            # Askama templates
│   │   ├── base.html         # Base layout (sidebar, header, content)
│   │   ├── macros.html       # Shared component macros
│   │   ├── pages/            # Full-page templates
│   │   └── partials/         # HTMX partial templates
│   └── static/               # Static assets (CSS, JS, fonts, icons)
│       ├── css/globals.css   # Tailwind + Rams CSS variables
│       ├── css/tailwind.css  # Compiled Tailwind output
│       ├── js/app.js         # Client-side JS
│       └── js/htmx.min.js    # HTMX library
```
