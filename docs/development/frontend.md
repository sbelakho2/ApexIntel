# ApexIntel Frontend Architecture

## Overview

ApexIntel has **two frontend surfaces**, each serving different interaction models:

1. **Leptos/WASM Single-Page Application** (`crates/frontend/`) — An interactive client-side rendered SPA for data exploration, charts, and graph visualization. Built with Leptos (Rust/WASM) and served via Trunk.

2. **Server-Rendered HTML UI** (`crates/api/`) — Classic page-based rendering using **Askama** (compile-time Rust templates), **HTMX** for partial-page updates, and **Tailwind CSS** for styling. Used for rapid page loads, list views, and form interactions.

Both frontends share the same design tokens (`--rams-*` CSS variables) for visual consistency.

---

## Technology Stack

### WASM Frontend

| Layer | Technology | Version |
|-------|-----------|---------|
| Framework | Leptos (CSR) | 0.6 |
| Router | leptos_router | 0.6 |
| Bundler | Trunk | — |
| Styling | Plain CSS + Custom Properties | — |
| Charts | SVG components (native) | — |

### Server-Rendered UI

| Layer | Technology | Version |
|-------|-----------|---------|
| Templates | Askama | 0.12 |
| Template-Axum Integration | askama_axum | 0.4 |
| Partial Updates | HTMX | 2.0 |
| Styling | Tailwind CSS | 3.x (standalone CLI) |
| Icons | Inline SVG via `icon` macro | — |

---

## WASM Frontend (`crates/frontend/`)

### Directory Structure

```
crates/frontend/
├── Cargo.toml              # Rust crate manifest
├── Trunk.toml              # Trunk bundler configuration
├── index.html              # Entry HTML (loaded by browser)
├── style.css               # Design tokens, layout, components
└── src/
    ├── lib.rs              # WASM entry point (mount_to_body)
    ├── app.rs              # App shell, router, navigation
    ├── api.rs              # HTTP API client
    ├── routes/             # Page components (one per route)
    │   ├── mod.rs
    │   ├── overview.rs
    │   ├── warnings.rs
    │   ├── insights.rs
    │   ├── companies.rs
    │   ├── company_detail.rs
    │   ├── persons.rs
    │   ├── person_detail.rs
    │   ├── search.rs
    │   ├── memos.rs
    │   ├── calibration.rs
    │   ├── graph.rs
    │   ├── competitors.rs
    │   ├── security.rs
    │   ├── recipes.rs
    │   ├── settings.rs
    │   ├── admin.rs
    │   ├── causality.rs
    │   ├── timeline.rs
    │   ├── login.rs
    │   └── adversarial.rs
    └── components/         # Shared UI components
        ├── mod.rs
        ├── cards.rs        # StatCard, SurfaceCard
        ├── filters.rs      # FilterBar, FilterChip
        ├── panels.rs       # Side panels, toolbars
        ├── badges/         # BayesianBadge, SourceReliabilityBadge, TemporalFlag
        └── charts/         # SVG chart components
            ├── mod.rs
            ├── probability_gauge.rs
            ├── reliability_diagram.rs
            ├── community_graph.rs
            ├── causal_graph.rs
            ├── confidence_band.rs
            ├── sparkline.rs
            ├── survival_curve.rs
            ├── brier_score_heatmap.rs
            └── source_entropy_gauge.rs
```

### App Shell & Routing

The app shell is defined in [`crates/frontend/src/app.rs`](../crates/frontend/src/app.rs:72). Key characteristics:

- **Router base**: `/wasm` (all routes are prefixed with `/wasm/`)
- **Layout**: CSS Grid with 260px sidebar + flexible main area (desktop); single-column with slide-in nav (mobile)
- **Navigation**: 16 nav items + 1 "Adversarial" link, rendered from a `NAV_ITEMS` constant
- **Routes**: Defined via `<Routes base="/wasm">` with nested `<Route path="..." view=... />` components

### Navigation

```rust
const NAV_ITEMS: [(&str, &str); 16] = [
    ("Overview", "/"),
    ("Warnings", "/warnings"),
    ("Insights", "/insights"),
    ("Companies", "/companies"),
    ("Persons", "/persons"),
    ("Search", "/search"),
    ("Memos", "/memos"),
    ("Calibration", "/calibration"),
    ("Graph", "/graph"),
    ("Competitors", "/competitors"),
    ("Security", "/security"),
    ("Recipes", "/recipes"),
    ("Settings", "/settings"),
    ("Admin", "/admin"),
    ("Causality", "/causality"),
    ("Timeline", "/entities/demo/timeline"),
];
```

### API Client

The WASM frontend communicates with the server via the API client in [`crates/frontend/src/api.rs`](../crates/frontend/src/api.rs:1). It fetches data from `/api/*` endpoints using `gloo-net`.

### Styling

All styles are in [`crates/frontend/style.css`](../crates/frontend/style.css:1) using CSS custom properties. The style defines:

- Design tokens (colors, spacing, typography)
- Layout components (app-shell, site-nav, app-main)
- Surface components (surface-card, stat-card)
- Chart components (.probability-gauge, .community-graph, etc.)
- Responsive breakpoints at 900px (mobile layout switch)

### Building & Running

```bash
# Development server (serves on http://127.0.0.1:8080)
cd crates/frontend && trunk serve --port 8080

# Production build
cd crates/frontend && trunk build

# Output goes to crates/frontend/dist/
```

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

## Rust Handler Structure

Each handler module follows this structure:

1. **Data structs** — Plain `#[derive(Clone, Debug)]` structs for template data
2. **Template struct** — `#[derive(Template)]` with `#[template(path = "...")]`
3. **Handler function** — `async fn` returning `impl IntoResponse`

Template structs must include `PageContext` fields (`current_path`, `username`, `warning_count`, `theme`).

## Adding a New Page (Server-Rendered)

1. Create `templates/pages/new_page.html` extending `base.html`
2. Create `src/web/new_page.rs` with template struct and handler
3. Add `pub mod new_page;` to `src/web/mod.rs`
4. Wire route in `main.rs` under `html_protected`
5. Add nav item in `base.html` sidebar

## Adding a New Route (WASM Frontend)

1. Create `src/routes/new_page.rs` with a Leptos component
2. Add `pub mod new_page;` to `src/routes/mod.rs`
3. Add `<Route path="new-page" view=NewPage />` in `app.rs`
4. Optionally add the route to `NAV_ITEMS` in `app.rs`

## Compiling Tailwind

```bash
npx tailwindcss -i crates/api/static/css/input.css -o crates/api/static/css/tailwind.css --minify
```

The Tailwind config scans `crates/api/templates/**/*.html` for class names.

## Build Verification

### Server-Rendered UI

Askama templates are checked at compile time. Run:
```bash
cargo check -p apex-api
```
Any template syntax errors, missing fields, or type mismatches will be caught as compile errors.

### WASM Frontend

```bash
cd crates/frontend && trunk build
```
Trunk compiles the Leptos app to WASM and outputs to `crates/frontend/dist/`.

### E2E Tests

```bash
# Run all Playwright E2E tests against the WASM frontend
npx playwright test -c playwright.config.cjs

# Run specific test suites
npx playwright test -c playwright.config.cjs e2e/html-ui.spec.js
npx playwright test -c playwright.config.cjs e2e/chart-pages.spec.js
```
