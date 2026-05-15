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

### 1.3 WASM Frontend Style Requirements

[`crates/frontend/style.css`](../crates/frontend/style.css:1) defines Rams-compatible CSS custom properties for the Leptos/WASM app. These include:

- All `--rams-*` tokens matching the design system
- Chart visualization variables (`--chart-series-*`)
- Bayesian evidence badge colors
- Component-specific styles (surface-card, stat-card, filter-bar, etc.)

---

## 2. Layout Infrastructure

The ApexIntel frontend has two rendering surfaces with different layout approaches:

### 2.1 Leptos/WASM App Shell (Interactive SPA)

The WASM frontend at [`crates/frontend/`](../crates/frontend/) uses a client-side router with a single app shell defined in [`app.rs`](../crates/frontend/src/app.rs:72):

- Desktop: CSS Grid layout with 260px sidebar + flexible main area
- Mobile: Single-column layout with sticky topbar and slide-in navigation
- Navigation: 16 nav items rendered from a `NAV_ITEMS` constant
- Routes: All defined via `<Routes>` in [`app.rs`](../crates/frontend/src/app.rs:154) with `<Route>` components

### 2.2 Askama Template Shell (Server-Rendered)

The server-rendered UI at [`crates/api/`](../crates/api/) uses:

- [`templates/base.html`](../crates/api/templates/base.html:1) — Base layout with sidebar, header bar, and content slot
- Sidebar navigation with active-state highlighting via Askama template blocks
- HTMX `hx-boost="true"` for partial-page navigation
- `{% block breadcrumbs %}` and `{% block content %}` for page-level customization

---

## 3. Shared UI Component Rules

### WASM Components ([`crates/frontend/src/components/`](../crates/frontend/src/components/mod.rs:1))

The WASM frontend uses Leptos components organized into modules:

- [`cards.rs`](../crates/frontend/src/components/cards.rs:1) — SurfaceCard, StatCard components
- [`filters.rs`](../crates/frontend/src/components/filters.rs:1) — FilterBar, FilterChip components
- [`panels.rs`](../crates/frontend/src/components/panels.rs:1) — Side panels, toolbars
- [`badges/`](../crates/frontend/src/components/badges/mod.rs:1) — BayesianBadge, SourceReliabilityBadge, TemporalFlag
- [`charts/`](../crates/frontend/src/components/charts/mod.rs:1) — SVG chart components (probability gauge, reliability diagram, community graph, sparkline, etc.)

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

### Required Characteristics (Both Surfaces)

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

### For the WASM Frontend (CSS Custom Properties)

Use the CSS variables defined in [`crates/frontend/style.css`](../crates/frontend/style.css:1):

| Token | CSS Variable |
|---|---|
| Background | `var(--background)` |
| Foreground | `var(--foreground)` |
| Card surface | `var(--card)` |
| Border | `var(--border)` |
| Muted text | `var(--muted)` |
| Primary accent | `var(--primary)` |
| Success | `var(--success)` |
| Destructive | `var(--destructive)` |

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
- [ ] No `500` UI/server response regressions in screenshot tests

---

## 7. Testing Workflow

### WASM Frontend Tests (Playwright)

Run E2E tests for the Leptos/WASM frontend:

```bash
# Start trunk dev server first (or use the webServer config in playwright.config.cjs)
cd crates/frontend && trunk serve --port 8080

# In another terminal, run tests
npx playwright test -c playwright.config.cjs

# Run specific test file
npx playwright test -c playwright.config.cjs e2e/html-ui.spec.js

# Update snapshots
npx playwright test -c playwright.config.cjs e2e/html-ui.spec.js --update-snapshots
```

### Server-Rendered UI Verification

```bash
# Compile-time template validation
cargo check -p apex-api

# Build the WASM frontend
cd crates/frontend && trunk build

# Compile Tailwind
npx tailwindcss -i crates/api/static/css/input.css -o crates/api/static/css/tailwind.css --minify
```

---

## 8. Migration Guidance for Existing Screens

When converting existing pages:

1. Normalize surface/background to Rams tokens
2. Move header/body into clear module containers
3. Replace plain text-only empty states with structured icon + heading + context
4. Tighten spacing to 4px grid increments
5. Re-run screenshot tests and compare
