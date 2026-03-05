# ApexIntel Frontend Architecture

## Overview

ApexIntel uses a server-rendered frontend built with **Askama** (compile-time Rust templates), **HTMX** for partial-page updates, and **Tailwind CSS** for styling. There is no client-side JavaScript framework; interactivity is delivered via HTMX attributes and minimal inline scripts.

## Technology Stack

| Layer | Technology | Version |
|-------|-----------|---------|
| Templates | Askama | 0.12 |
| Template-Axum Integration | askama_axum | 0.4 |
| Partial Updates | HTMX | 2.0 |
| Styling | Tailwind CSS | 3.x (standalone CLI) |
| Icons | Inline SVG via `icon` macro | — |

## Directory Structure

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
    ├── css/tailwind.css
    └── js/app.js
```

## Template Patterns

### Base Layout

All full pages extend `base.html`, which provides:
- Sidebar navigation with active-state highlighting
- Header bar with user info
- `{% block breadcrumbs %}` for page-level breadcrumbs
- `{% block content %}` for main content area
- HTMX configuration via `<meta name="htmx-config">` with `historyCacheSize: 10`
- `hx-boost="true"` on `<body>` for automatic HTMX link boosting

### Macros (`macros.html`)

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

## Adding a New Page

1. Create `templates/pages/new_page.html` extending `base.html`
2. Create `src/web/new_page.rs` with template struct and handler
3. Add `pub mod new_page;` to `src/web/mod.rs`
4. Wire route in `main.rs` under `html_protected`
5. Add nav item in `base.html` sidebar

## Compiling Tailwind

```bash
npx tailwindcss -i crates/api/static/css/input.css -o crates/api/static/css/tailwind.css --minify
```

The Tailwind config scans `crates/api/templates/**/*.html` for class names.

## Build Verification

Askama templates are checked at compile time. Run:
```bash
cargo check -p apex-api
```
Any template syntax errors, missing fields, or type mismatches will be caught as compile errors.
