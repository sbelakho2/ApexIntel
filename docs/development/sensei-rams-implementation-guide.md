# Sensei-Rams Implementation Guide (ApexIntel)

> Technical implementation guide for applying Sensei-Rams consistently in ApexIntel.

---

## 1. Project Configuration

### 1.1 Tailwind Requirements (Server-Rendered UI)

[`tailwind.config.js`](../tailwind.config.js:1) must include:

- `rams` color family (`chassis`, `module`, `panel`, `line`, `muted`, `orange`, `green`, `red`, `steel`)
- micro radii (`rams-sm`, `rams-md`, `rams-lg`)
- Rams spacing scale (`rams-1..rams-16`)
- Rams shadows (`rams-inset`, `rams-pressed`, `rams-focus`)
- fast transition durations (`rams-instant..rams-slow`)
- plugins: `@tailwindcss/forms`, `@tailwindcss/typography`, `tailwindcss-animate`

### 1.2 Global CSS Requirements (Server-Rendered UI)

[`crates/api/static/css/globals.css`](../crates/api/static/css/globals.css:1) must define:

- core Rams variables for light and dark themes
- anti-blur text rendering settings
- tabular numeric utility (`[data-numeric], .tabular-nums`)
- density modes (`density-compact`, `density-comfortable`, `density-expanded`)
- reduced motion fallback

### 1.3 Archived WASM Experiment

The Leptos/WASM SPA was retired from the workspace and now lives (unsupported,
unbuilt) in [`experiments/wasm-frontend/`](../experiments/wasm-frontend/). Its
chart variables (`--chart-series-*`) and Rams tokens were folded into
[`crates/api/static/css/globals.css`](../crates/api/static/css/globals.css:1),
which is the single source of truth. Do not build or style against the archive.

---

## 2. Layout Infrastructure

ApexIntel has one shipped rendering surface: the server-rendered Askama
template shell at [`crates/api/`](../crates/api/).

### 2.1 Askama Template Shell (Server-Rendered)

The server-rendered UI at [`crates/api/`](../crates/api/) uses:

- [`templates/base.html`](../crates/api/templates/base.html:1) — Base layout with sidebar, header bar, and content slot
- Sidebar navigation with active-state highlighting via Askama template blocks
- HTMX `hx-boost="true"` for partial-page navigation
- `{% block breadcrumbs %}` and `{% block content %}` for page-level customization

---

## 3. Shared UI Component Rules

### Server-Rendered Components ([`crates/api/templates/macros.html`](../crates/api/templates/macros.html:1))

Shared Askama macros providing:

- `icon(name, size)` — inline SVG icons
- `page_header(title, icon, subtitle, badge)` — page heading block
- `severity_badge(level)` / `status_badge(status)` / `region_badge(region)`
- `score_ring(score, size)` — SVG circular score gauge
- `progress_bar(value, max, color, label)` — horizontal progress bar
- `confidence_meter(pct)` — stepped confidence display
- `empty_state(message, icon)` — placeholder for empty lists
- SVG chart helpers (`donut_chart`, `country_flag`, `tier_color_class`)

### Required Characteristics (Server-Rendered UI)

- Visual hierarchy via borders and section dividers
- Icon + text pairing for primary headers and empty states
- Compact uppercase labels for metrics/metadata
- Structured empty-state format (not plain sentence only)

### Stateless Component Patterns

Shared primitives should remain stateless and reusable:

- `PageHeader`
- `StatCard`
- `SurfaceCard`
- `EmptyState`
- `DataTable`

---

## 4. Rams Class Contract

### For the Server-Rendered UI (Tailwind)

Use these classes as stable design primitives:

- `bg-rams-chassis`, `bg-rams-module`, `bg-rams-panel`
- `border-rams-line`
- `text-rams-muted`
- `text-rams-orange|green|red|steel`
- `rounded-rams-sm|md|lg`
- `shadow-rams-inset|rams-pressed|rams-focus`

If a new component needs Rams styling, compose from this contract before inventing ad-hoc tokens.

---

## 5. Accessibility & Interaction Contract

All interactive controls must support:

- keyboard operation (tab/enter/space/escape where relevant)
- visible focus state with sufficient contrast
- role + label semantics for icon-only actions
- reduced-motion compliance

Status indicators must not rely on color alone.

---

## 6. Verification Checklist (PR Gate)

### Visual

- [ ] No gradient-heavy or glassmorphism regressions
- [ ] No oversized radii or floating card shadows
- [ ] Structural borders and module hierarchy are visible
- [ ] Accent color usage remains semantic

### Code

- [ ] New classes consume existing Rams tokens
- [ ] No hard-coded random hex colors for core UI
- [ ] Shared components not duplicated with divergent styles

### Runtime

- [ ] No console runtime errors
- [ ] No `500` UI/server response regressions in the server-UI visual specs

---

## 7. Testing Workflow

### Server-Rendered UI Verification

```bash
# Compile-time template validation
cargo check -p apex-api

# Route/interaction e2e against a running apex-api
BASE_URL=http://127.0.0.1:9095 ADMIN_USER=admin ADMIN_PASS=adminpassword \
  node scripts/ci/e2e_server_ui.mjs

# Playwright contract/visual specs (server must already be running)
npx playwright test -c playwright.server-ui.config.cjs

# Compile Tailwind
npx tailwindcss -i crates/api/static/css/globals.css -o crates/api/static/css/tailwind.css --minify
```

There is no WASM build step: `experiments/wasm-frontend/` is archived and is
not compiled, tested, or shipped.

---

## 8. Migration Guidance for Existing Screens

When converting existing pages:

1. Normalize surface/background to Rams tokens
2. Move header/body into clear module containers
3. Replace plain text-only empty states with structured icon + heading + context
4. Tighten spacing to 4px grid increments
5. Re-run the server-UI visual specs and compare
