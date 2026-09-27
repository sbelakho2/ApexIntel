# ApexIntel Frontend Architecture

## Overview

ApexIntel ships **one web UI**: the server-rendered application in
`crates/api/` built with **Askama** (compile-time Rust templates), **HTMX** for
partial-page updates, and **Tailwind CSS** for styling. It covers every product
route, including charts, as plain HTML/SVG fragments that HTMX swaps into the
page.

A Leptos/WASM single-page application previously duplicated route composition,
API clients, and chart components. It was never deployed and drifted from the
shipped UI, so it was retired from the workspace and archived under
[`experiments/wasm-frontend/`](../experiments/wasm-frontend/). It is not built,
tested, or shipped; see [`experiments/README.md`](../experiments/README.md).
`scripts/ci/check_frontend_removed.sh` fails if it reappears in the workspace,
CI, or package scripts.

---

## Technology Stack (Server-Rendered UI)

| Layer | Technology | Version |
|-------|-----------|---------|
| Templates | Askama | 0.12 |
| Template-Axum Integration | askama_axum | 0.4 |
| Partial Updates | HTMX | 2.0 |
| Styling | Tailwind CSS | 3.x (standalone CLI) |
| Charts | Server-rendered SVG (inline templates / Rust string builders) | — |
| Icons | Inline SVG via `icon` macro | — |

---

## Server-Rendered UI (`crates/api/`)

### Directory Structure

```
crates/api/
├── src/web/           # Rust handler modules (one per page)
│   ├── mod.rs         # Shared helpers: is_htmx_request(), PageContext
│   ├── dashboard.rs
│   ├── warnings.rs
│   ├── insights.rs
│   ├── companies.rs
│   ├── persons.rs
│   ├── competitors.rs
│   ├── memos.rs
│   ├── security.rs
│   ├── recipes.rs
│   ├── settings.rs
│   ├── admin.rs
│   ├── auth.rs
│   ├── graph.rs
│   ├── search.rs
│   └── errors.rs
├── src/api_handlers/  # JSON API endpoints and server-side SVG chart builders
│   ├── charts.rs      # Entity activity chart data + SVG fragment rendering
│   └── ...
├── templates/
│   ├── base.html      # Base layout (sidebar, header, content slot)
│   ├── macros.html    # Shared macros (icon, badges, charts, etc.)
│   ├── pages/         # Full-page templates (extend base.html)
│   │   ├── dashboard.html
│   │   ├── warnings.html
│   │   ├── warnings/_list.html    # HTMX list partial
│   │   ├── warning_detail.html
│   │   ├── insights.html
│   │   ├── insights/_list.html
│   │   ├── insight_detail.html
│   │   ├── companies.html
│   │   ├── companies/_list.html
│   │   ├── company_detail.html
│   │   ├── competitors.html
│   │   ├── competitors/_list.html
│   │   ├── memos.html
│   │   ├── person_detail.html
│   │   ├── recipes.html
│   │   ├── recipes/_list.html
│   │   └── security.html
│   └── partials/      # Reusable card/component fragments
│       ├── warning_card.html
│       ├── insight_card.html
│       ├── company_card.html
│       ├── competitor_card.html
│       ├── memo_card.html
│       ├── pagination.html
│       ├── filter_bar.html
│       ├── filter_chip.html
│       ├── company_changes_tab.html
│       ├── company_dossier_tab.html
│       ├── person_affiliations_tab.html
│       ├── person_history_tab.html
│       ├── person_peers_tab.html
│       ├── security_dns_tab.html
│       ├── security_kev_tab.html
│       └── security_lookalike_tab.html
└── static/
    ├── css/
    │   ├── globals.css    # Tailwind directives + Rams CSS variables
    │   └── tailwind.css   # Compiled Tailwind output
    ├── fonts/             # Inter and JetBrains Mono variable fonts
    ├── icons/sprite.svg   # SVG icon sprite
    └── js/
        ├── app.js         # Client-side JavaScript
        ├── graph.js       # Graph visualization helpers
        └── htmx.min.js    # HTMX library
```

### Askama Template Patterns

All full pages extend [`templates/base.html`](../crates/api/templates/base.html:1), which provides:
- Sidebar navigation with active-state highlighting
- Header bar with user info
- `{% block breadcrumbs %}` for page-level breadcrumbs
- `{% block content %}` for main content area
- HTMX configuration via `<meta name="htmx-config">` with `historyCacheSize: 10`
- `hx-boost="true"` on `<body>` for automatic HTMX link boosting

### Shared Macros

Import with `{% import "macros.html" as m %}` and call with `{% call m::macro_name(args) %}`.

Key macros:
- `icon(name, size)` — renders inline SVG icon
- `page_header(title, icon, subtitle, badge)` — page heading block
- `severity_badge(level)` — colored badge for critical/high/medium/low
- `status_badge(status)` — badge for active/resolved/pending
- `region_badge(region)` — region indicator
- `score_ring(score, size)` — SVG circular score gauge
- `progress_bar(value, max, color, label)` — horizontal progress bar
- `confidence_meter(pct)` — stepped confidence display
- `tag_pill(tag)` — small tag chip
- `empty_state(message, icon)` — placeholder for empty lists
- `donut_chart(segments, size, stroke_width, half, radius)` — SVG donut chart from precomputed segments
- `entity_activity_chart(entity_id, days)` — HTMX loader for the per-entity activity SVG
- `country_flag(code)` — flag emoji from 2-letter country code
- `tier_color_class(tier)` — Tailwind color class for priority tiers
- `role_family_color_class(family)` — Tailwind color class for role families

### HTMX Partial Responses

Each list handler checks `is_htmx_request(&headers)` (looks for the `hx-request` header). When true, the handler returns the full Askama template which HTMX swaps into `#main-results`. The base layout's `hx-boost="true"` ensures navigations automatically use HTMX.

Pattern in Rust handler:
```rust
if is_htmx_request(&headers) {
    // Return the template (HTMX will extract #main-results content)
    tpl.into_response()
} else {
    tpl.into_response()
}
```

For list pages, dedicated `_list.html` partial templates exist that render just the list content without `base.html`, suitable for HTMX swaps targeting specific containers.

### HTMX Action Buttons

Interactive actions use HTMX POST attributes:
```html
<button hx-post="/warnings/{{ id }}/acknowledge"
        hx-target="#ack-status"
        hx-swap="innerHTML">
  Acknowledge
</button>
```

Loading indicators use `hx-indicator`:
```html
<button hx-post="/warnings/{{ id }}/analyze"
        hx-target="#analysis-panel"
        hx-swap="outerHTML"
        hx-indicator="#analyze-spinner">
  Analyze
  <span id="analyze-spinner" class="htmx-indicator ..."></span>
</button>
```

### Tab Navigation

Detail pages use tabs (client-side toggle or HTMX-loaded):
- **Company detail**: HTMX-loaded tabs via `hx-get="/companies/:id/changes"` and `hx-get="/companies/:id/dossier"`
- **Person detail**: Client-side tabs (Affiliations / Role History / Peers) toggled via inline JS
- **Security**: Client-side tabs (DNS / KEV / Lookalike) toggled via inline JS

### Server-Rendered Charts

Charts are HTML/SVG fragments rendered on the server, not client-side chart
components:

- **Entity activity chart** — `GET /api/charts/entity/:id/activity/svg?days=7|30|90`
  returns an inline SVG built by
  [`render_activity_chart_svg`](../crates/api/src/api_handlers/charts.rs:1).
  The `days` query parameter selects the window; the returned series length
  equals that window (7/30/90 points). Observation and insight counts share the
  left count axis; the 0–1 activity score has its own right axis so it is not
  flattened against count magnitudes. Axis ticks are derived from the rendered
  data range, so a non-zero minimum is labelled with its real value instead of
  a hard-coded `0`. The macro's `7d/30d/90d` chips HTMX-swap this fragment into
  the card body.
- **JSON observation buckets** — `GET /api/charts/entity/:id/observations?days=N&bucket=week`
  returns bucketed counts for API consumers. The entity *activity* JSON twin
  (`/api/charts/entity/:id/activity`) was removed with the WASM SPA cleanup:
  the SVG fragment is the only activity-chart surface, so there is no
  second, unverified rendering path to drift.
- **Macro bar charts** — dashboard trend and crawl-activity charts are Askama
  macros over precomputed per-bucket heights (`warning_trend_chart`,
  `insight_trend_chart`, `crawl_activity_chart`, `observation_trend_chart`).

## Rust Handler Structure

Each handler module follows this structure:

1. **Data structs** — Plain `#[derive(Clone, Debug)]` structs for template data
2. **Template struct** — `#[derive(Template)]` with `#[template(path = "...")]`
3. **Handler function** — `async fn` returning `impl IntoResponse`

Template structs must include `PageContext` fields (`current_path`, `username`, `warning_count`, `theme`).

## Adding a New Page

1. Create `templates/pages/new_page.html` extending `base.html`
2. Create `src/web/new_page.rs` with template struct and handler
3. Add `pub mod new_page;` to `src/web/mod.rs`
4. Wire route in `main.rs` under `html_protected`
5. Add nav item in `base.html` sidebar

## Compiling Tailwind

```bash
npx tailwindcss -i crates/api/static/css/globals.css -o crates/api/static/css/tailwind.css --minify
```

The Tailwind config scans `crates/api/templates/**/*.html` for class names.

## Build Verification

Askama templates are checked at compile time. Run:
```bash
cargo check -p apex-api
```
Any template syntax errors, missing fields, or type mismatches will be caught as compile errors.

### E2E Tests

```bash
# Route sweep + task contracts against a running apex-api
BASE_URL=http://127.0.0.1:9095 ADMIN_USER=admin ADMIN_PASS=adminpassword \
  node scripts/ci/e2e_server_ui.mjs

# Playwright contract/visual specs (server must already be running)
npx playwright test -c playwright.server-ui.config.cjs
```
