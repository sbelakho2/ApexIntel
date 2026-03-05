# ApexIntel Frontend Migration Report

## Node/JS/TS → Rust-Based Infrastructure

**Date:** 2026-03-04
**Author:** Engineering Analysis
**Scope:** Complete elimination of Node.js runtime; replace Next.js 14 frontend with Rust-native serving

---

## Table of Contents

1. [Executive Summary](#1-executive-summary)
2. [Current Infrastructure Audit](#2-current-infrastructure-audit)
3. [Resource Impact Analysis](#3-resource-impact-analysis)
4. [Rust Frontend Framework Evaluation](#4-rust-frontend-framework-evaluation)
5. [Recommended Architecture](#5-recommended-architecture)
6. [Feature-by-Feature Migration Map](#6-feature-by-feature-migration-map)
7. [Migration Phases](#7-migration-phases)
8. [Risk Assessment](#8-risk-assessment)
9. [Detailed Migration Checklist](#9-detailed-migration-checklist)

---

## 1. Executive Summary

### The Problem

The current ApexIntel frontend runs a full Node.js 20 + Next.js 14 runtime on a single Hetzner ARM64 server (16-core Neoverse-N1, 30 GB RAM). This creates:

| Metric | Next.js Frontend | Rust API (apex-api) | Ratio |
|--------|-----------------|---------------------|-------|
| **RSS Memory** | 112 MB | 15 MB | **7.4×** |
| **Virtual Memory** | 13.0 GB | 1.2 GB | **10.8×** |
| **Disk (total)** | 764 MB | 19 MB binary | **40×** |
| **node_modules** | 587 MB | 0 | ∞ |
| **.next build cache** | 170 MB | 0 | ∞ |
| **Threads** | 11 | 18 | — |
| **Startup time** | ~5s | <200ms | **25×** |
| **Dependencies** | 48 npm packages | 0 JS deps | ∞ |

The server has 30 GB RAM with only 1.3 GB free. The Node.js process is the single largest memory consumer on the box.

### The Opportunity

The ApexIntel frontend is architecturally simple:
- **0 SSR logic** — every page is `"use client"` with client-side `useQuery` data fetching
- **0 used Radix/form libraries** — all UI is hand-built HTML/Tailwind
- **1 real server function** — the `/api/proxy/[...path]` catch-all that adds a Bearer token
- **1 auth system** — HMAC SHA-256 sessions (already trivial in Rust)
- **~11,000 lines** of TypeScript total

### Recommended Strategy

**Hybrid approach:** Server-side HTML templating (Askama/Tera) in the existing Rust `apex-api` binary + a thin client-side JS layer (~2 KB) for interactivity. This eliminates Node.js entirely.

**Projected outcome:**

| Metric | After Migration |
|--------|----------------|
| **RSS Memory** | +3-8 MB on apex-api (total ~23 MB vs current 127 MB combined) |
| **Disk** | +2 MB in binary, -764 MB node_modules/.next | Net: **-762 MB** |
| **Processes** | 1 (unified) vs 2 (api + next-server) |
| **Dependencies** | 0 npm, 0 Node.js |
| **Build** | `cargo build --release` only — no `npm install`, no `npm run build` |
| **TTFB** | <5ms (in-process templates) vs ~50-100ms (nginx → next → react hydration) |
| **Page load** | ~50-100 KB HTML+CSS vs ~110 KB JS + hydration waterfall |

---

## 2. Current Infrastructure Audit

### 2.1 Source Inventory

| Category | Files | Lines | Key Complexity |
|----------|-------|-------|----------------|
| **Page routes** | 22 .tsx | ~5,800 | All `"use client"`, all `useQuery`-based |
| **API routes** | 6 route.ts | ~320 | Auth login/logout/me + catch-all proxy |
| **Components** | 5 .tsx | ~2,010 | ui.tsx (1,205 lines, 34 components), app-shell (314), interactive-graph (445) |
| **Lib** | 5 .ts/.tsx | ~1,795 | api.ts (1,309 lines, ~30 fetch fns, ~40 interfaces), auth (134+65), WebSocket hook (275) |
| **Root files** | 5 | ~560 | layout, providers, middleware, instrumentation, globals.css |
| **E2E tests** | 3 | ~570 | Playwright: visual regression, dark mode, device compat |
| **Config** | 5 | ~370 | next.config, tailwind, tsconfig, postcss, playwright |
| **TOTAL** | **51 files** | **~11,033 lines** | — |

### 2.2 Dependency Analysis

#### Actually Used Dependencies (6 packages)

| Package | Usage | Rust Equivalent |
|---------|-------|-----------------|
| `next` 14.1.0 | Routing, SSR runtime, middleware | Axum (already in use) + Askama templates |
| `react` / `react-dom` 18.2.0 | Component rendering, hooks (useState, useEffect, useCallback, useMemo, useRef, useContext) | Askama templates + ~2 KB vanilla JS |
| `@tanstack/react-query` 5.17.9 | Client-side data caching, refetch, stale-while-revalidate | Server-side queries (already in Rust) → embed in HTML |
| `recharts` 2.10.4 | 1 PieChart on dashboard | SVG template or lightweight charting JS (<5 KB) |
| `framer-motion` 10.18.0 | 1 `motion.div` slide animation on graph info panel | CSS `@keyframes` / `transition` (zero JS) |
| `next-themes` 0.2.1 | Dark mode class toggle | 10-line `<script>` in `<head>` |
| `lucide-react` 0.312.0 | 36 SVG icons (re-exported in ui.tsx) | Inline SVG in templates (copy SVG paths) |
| `date-fns` 3.2.0 | `formatDistanceToNow`, `format` (3-4 call sites) | `chrono` (already a dependency) |

#### Completely Unused Dependencies (16+ packages — install bloat)

| Package | Status |
|---------|--------|
| `@hookform/resolvers`, `react-hook-form`, `zod` | Never imported. Recipe builder uses raw useState. |
| `axios` | Never imported. All fetches use native `fetch`. |
| `class-variance-authority`, `clsx`, `tailwind-merge` | Never imported. Classes are template literals. |
| `server-only` | Never imported. |
| `@tanstack/react-table` | Never imported. DataTable in ui.tsx is custom. |
| `@radix-ui/*` (16 packages) | None imported. All UI is hand-built. |
| `@tanstack/react-query-devtools` | Dev-only, not needed in production. |

**Conclusion:** The project already avoids all heavyweight React libraries. The "React" layer is essentially a templating engine with `fetch` calls — exactly what server-side templates do natively.

### 2.3 Page Complexity Classification

#### Tier 1: Pure Data Display (trivial migration) — 13 pages

These fetch JSON via `useQuery` and render lists/cards/tables. Zero client-side state beyond pagination/filters.

| Page | Lines | Data Sources | Interactive Elements |
|------|-------|-------------|---------------------|
| `/` (Dashboard) | 252 | 6 queries | None (read-only KPIs + charts) |
| `/companies` | ~200 | 1 query | Search, sort, region filter, pagination |
| `/companies/[id]` | ~240 | 3 queries | Tab switch |
| `/competitors` | ~250 | 2 queries | View toggle, pagination |
| `/insights` | ~180 | 1 query | Type filter, search, pagination |
| `/memos` | ~190 | 1 query | Sidebar selection |
| `/persons/[id]` | ~800 | 2 queries | 5 tabs |
| `/search` | ~150 | 1 query | Search input |
| `/settings` | ~300 | 3 queries | 5 tabs (mostly read-only) |
| `/security` | ~300 | 5 queries | 4 tabs, acknowledge button |
| `/graph` | ~200 | 1 query | Type filter, table toggle |
| `/admin` | 274 | 5 queries | 7 trigger buttons |
| `/login` | ~170 | 0 | Form submit |

#### Tier 2: Client-Side Interactivity Required — 4 pages

| Page | Lines | Why Client JS Is Needed |
|------|-------|------------------------|
| `/warnings` | ~280 | Checkbox select-all, bulk delete, optimistic mutation |
| `/warnings/[id]` | ~450 | AI analysis trigger → streaming response render |
| `/insights/[id]` | 564 | AI analysis trigger → streaming response render, bookmark toggle |
| `/recipes/new` | 751 | Dynamic form builder (add/remove signal/transform/threshold/action sections) |

#### Tier 3: Heavy Client-Side Logic — 1 component

| Component | Lines | Why It Must Remain JS |
|-----------|-------|-----------------------|
| `interactive-graph.tsx` | 445 | Force-directed physics simulation, `requestAnimationFrame`, SVG drag/zoom, `ResizeObserver` |

### 2.4 Server-Side Features Currently in Next.js

| Feature | Current Implementation | Rust Equivalent |
|---------|----------------------|-----------------|
| **Session auth** | HMAC SHA-256 in `auth.ts` (Node `crypto`) | `hmac-sha256` crate or `ring` (apex-api already uses `sha2` + `subtle`) |
| **Middleware** | Edge middleware validates `apex_session` cookie | Axum `from_fn` middleware (already done for API key auth) |
| **Proxy** | `/api/proxy/[...path]/route.ts` adds Bearer token | Axum handler with `reqwest` forwarding (unnecessary — templates query DB directly) |
| **Static assets** | `/_next/static/` served by Next.js | `tower-http::ServeDir` (2 lines of code) |
| **Image optimization** | `next/image` (not used on any page) | Not needed |
| **Font loading** | `next/font/local` for Inter + JetBrains Mono | Direct `<link>` tags to `/fonts/*.woff2` |
| **API rewrites** | `/api/health` → backend | Unnecessary when frontend IS the backend |

### 2.5 Deployment Architecture

```
Current (3 processes):
  nginx:443 → next-server:3000 (112 MB) → apex-api:8080 (15 MB)
                                          ← apex-worker (background)

After migration (2 processes):
  nginx:443 → apex-api:8080 (23 MB, serves HTML + API + WebSocket)
               ← apex-worker (background)
```

The entire Next.js proxy layer (`/api/proxy/*`) becomes unnecessary because the Rust server directly queries the database and renders HTML.

---

## 3. Resource Impact Analysis

### 3.1 Memory Projection

| Component | Current | After | Savings |
|-----------|---------|-------|---------|
| next-server (Node.js) | 112 MB RSS | 0 | **-112 MB** |
| apex-api (Rust) | 15 MB RSS | ~23 MB (with templates + static serving) | +8 MB |
| Template cache (Askama) | 0 | ~1 MB (compiled into binary) | +1 MB |
| Static file serving | Handled by Node.js | ~2 MB (tower-http cache) | +2 MB |
| **Net RSS savings** | — | — | **~101 MB** |

### 3.2 Disk Projection

| Component | Current | After | Savings |
|-----------|---------|-------|---------|
| `node_modules/` | 587 MB | 0 | **-587 MB** |
| `.next/` build cache | 170 MB | 0 | **-170 MB** |
| Frontend source (src/) | 7 MB | 0 (templates compiled in) | **-7 MB** |
| apex-api binary | 19 MB | ~22 MB (+templates + static) | +3 MB |
| Static assets (fonts, CSS) | in .next/ | ~2 MB standalone | +2 MB |
| **Net disk savings** | — | — | **~762 MB** |

### 3.3 Network Latency

| Metric | Current | After |
|--------|---------|-------|
| **TTFB (HTML)** | ~50-120ms (nginx → Node.js SSR → React render) | **<5ms** (nginx → Askama template) |
| **Full page load** | ~200-400ms (HTML + 110 KB JS bundle + hydration + fetch → render) | **<50ms** (HTML arrives pre-rendered with data) |
| **API proxy overhead** | +10-30ms per request (Next.js proxy adds Bearer token) | **0ms** (direct DB query in same process) |
| **WebSocket** | Direct to :8080 (bypasses Next.js) | Same | 

### 3.4 Build Time

| Step | Current | After |
|------|---------|-------|
| `npm install` | 30-60s | **0s** (eliminated) |
| `npm run build` | 20-40s | **0s** (eliminated) |
| `cargo build --release` (apex-api) | 2-5min | 2-6min (+templates, marginal) |
| **Total** | 3-6min | **2-6min** |
| Deploy complexity | Copy binary + copy frontend + npm install + npm build + restart 2 services | Copy 1 binary + restart 1 service |

---

## 4. Rust Frontend Framework Evaluation

### 4.1 Options Evaluated

| Framework | Type | Maturity | Approach |
|-----------|------|----------|----------|
| **Leptos** 0.7 | Full-stack SPA/SSR | Production-ready | Rust → WASM components, server functions, hydration |
| **Dioxus** 0.6 | Full-stack SPA/SSR | Production-ready | React-like RSX, WASM or server-rendered |
| **Yew** 0.21 | Client-side SPA | Mature | React-like, WASM-only (no SSR) |
| **Perseus** 0.4 | SSG/SSR meta-framework | Beta | Sycamore-based, SSR + hydration |
| **Askama** 0.12 + HTMX | Server-side templates | Stable (10+ years) | Jinja2-like compiled templates + hypermedia |
| **Tera** 0.19 + HTMX | Server-side templates | Stable | Django-like runtime templates + hypermedia |
| **Maud** 0.26 | Server-side markup | Stable | Rust macro-based HTML |

### 4.2 Evaluation Matrix

| Criterion | Weight | Leptos | Dioxus | Askama+HTMX | Tera+HTMX | Maud |
|-----------|--------|--------|--------|-------------|-----------|------|
| **Memory footprint** | 25% | 7 | 7 | **10** | 9 | **10** |
| **Build simplicity** | 20% | 5 | 5 | **10** | **10** | 9 |
| **Migration effort** | 20% | 4 | 5 | **8** | **8** | 7 |
| **Axum integration** | 15% | 8 | 7 | **10** | **10** | **10** |
| **Runtime performance** | 10% | 8 | 8 | **10** | 9 | **10** |
| **Ecosystem/docs** | 10% | 7 | 6 | **9** | **9** | 7 |
| **Weighted Score** | 100% | 6.1 | 6.1 | **9.5** | **9.2** | 8.7 |

### 4.3 Why Leptos/Dioxus/Yew Are Wrong for This Project

1. **WASM overhead is pointless here.** The current pages do zero client-side computation — they fetch JSON and display it. Shipping 200-500 KB of WASM to render HTML that a server can produce in microseconds is architecturally backwards.

2. **Hydration tax.** Leptos/Dioxus SSR still require shipping Rust-compiled WASM to "hydrate" server-rendered HTML. This recreates the exact problem we're solving (JS hydration waterfall → WASM hydration waterfall).

3. **Build complexity.** WASM frameworks require `wasm-pack`, `wasm-bindgen`, `trunk`, or `cargo-leptos` — additional toolchains on top of Cargo. The goal is *simplification*.

4. **Developer experience.** RSX/component trees in Rust are verbose and harder to iterate on than templates. The current UI is simple enough that template syntax is a better fit.

5. **The 1 exception** (interactive graph) can remain as a standalone `<script>` — it's 445 lines of self-contained JS/SVG that doesn't benefit from Rust.

### 4.4 Recommendation: Askama + HTMX + Vanilla JS

| Layer | Technology | Role |
|-------|-----------|------|
| **Templates** | Askama 0.12 (compile-time Jinja2) | All HTML rendering — pages, partials, components |
| **Styling** | globals.css (kept as-is) | Direct `<link>` tag, no Tailwind build step needed |
| **Interactivity** | HTMX 2.0 (~14 KB gzipped) | Tab switching, search, pagination, form submit, bookmark toggle, acknowledge — all without custom JS |
| **Complex forms** | Alpine.js 3 (~8 KB gzipped) or Vanilla JS | Recipe builder's dynamic add/remove sections |
| **Charts** | Chart.js 4 (~16 KB gzipped) or inline SVG | Dashboard pie chart, sparklines |
| **Force graph** | Standalone vanilla JS (~5 KB) | Interactive graph — extract current logic, remove React deps |
| **Dark mode** | 10-line `<script>` in `<head>` | Read/write `document.documentElement.classList` |
| **WebSocket** | Native `WebSocket` (~20 lines) | Real-time warning alerts |
| **Server** | Axum (existing apex-api) | Unified server for API + HTML + static assets |
| **Auth** | Axum middleware (existing pattern) | Session cookie validation on HTML routes |

**Why Askama over Tera:**
- Askama templates are compiled into the Rust binary at build time → zero runtime parsing overhead, zero filesystem reads, compile-time error checking for template syntax
- Tera parses templates at runtime → slower cold start, runtime errors possible
- Askama has first-class Axum integration via `askama_axum`

---

## 5. Recommended Architecture

### 5.1 New Crate Structure

```
crates/
  api/          ← existing: Axum JSON API server
  web/          ← NEW: Axum HTML server (or merged into api)
    Cargo.toml
    src/
      main.rs           ← (or lib.rs if merged into api)
      routes/
        mod.rs
        dashboard.rs    ← GET / → HTML
        warnings.rs     ← GET /warnings, /warnings/:id → HTML
        insights.rs     ← GET /insights, /insights/:id → HTML
        companies.rs    ← GET /companies, /companies/:id → HTML
        persons.rs      ← GET /persons, /persons/:id → HTML
        competitors.rs
        graph.rs
        memos.rs
        recipes.rs
        search.rs
        security.rs
        settings.rs
        admin.rs
        auth.rs         ← GET /login, POST /login, POST /logout
      middleware/
        session.rs      ← cookie validation (port from middleware.ts)
      templates/        ← Askama .html files
        base.html       ← layout: <head>, nav shell, scripts
        pages/
          dashboard.html
          warnings/
            list.html
            detail.html
          insights/
            list.html
            detail.html
          companies/
            list.html
            detail.html
          ... (1 per page)
        partials/
          nav.html
          stat_card.html
          warning_card.html
          insight_card.html
          company_card.html
          person_card.html
          competitor_card.html
          pagination.html
          severity_badge.html
          filter_bar.html
          data_table.html
          toast.html
          score_ring.html   ← SVG partial
    static/
      css/
        globals.css     ← current globals.css (as-is)
      fonts/
        InterVariable.woff2
        InterVariable-Italic.woff2
        JetBrainsMonoVariable.woff2
        JetBrainsMonoVariable-Italic.woff2
      js/
        htmx.min.js     ← HTMX 2.0 (~14 KB gzipped)
        app.js           ← dark mode toggle, toast system, WS (~2 KB)
        graph.js         ← force-directed graph (~5 KB, extracted from interactive-graph.tsx)
        recipe-builder.js ← dynamic form builder (~3 KB)
      icons/
        sprite.svg       ← all 36 lucide icons as a single SVG sprite
```

### 5.2 Merged vs Separate Binary

**Recommended: Merge into `apex-api`.** Rationale:
- Both need `PgStore`, `AppConfig`, request routing — sharing eliminates duplication
- Axum routers compose trivially via `.merge()` or `.nest()`
- Single systemd service to manage
- Template compilation adds ~1-2s to build time, negligible

```rust
// In apex-api/src/main.rs, add:
let html_routes = Router::new()
    .route("/", get(web::dashboard))
    .route("/warnings", get(web::warnings_list))
    .route("/warnings/:id", get(web::warning_detail))
    // ... all HTML routes
    .route("/login", get(web::login_page).post(web::login_submit))
    .route("/logout", post(web::logout))
    .route_layer(middleware::from_fn_with_state(state.clone(), require_session));

let static_files = Router::new()
    .nest_service("/static", ServeDir::new("static").precompressed_gzip());

let app = api_routes      // existing JSON API
    .merge(html_routes)    // new HTML pages
    .merge(static_files)   // CSS, fonts, JS
    .merge(ws_routes);     // existing WebSocket
```

### 5.3 Auth Flow (Simplified)

```
Current (3 hops):
  Browser → nginx → Next.js middleware (validate session) → Next.js page
  Page → useQuery → fetch /api/proxy/... → Next.js route.ts (add Bearer) → Rust API

After (1 hop):
  Browser → nginx → Rust Axum (validate session + query DB + render HTML)
```

The entire proxy layer disappears. The Axum handler validates the session cookie, queries the database directly (via `PgStore` — already available in request state), and renders the Askama template with the data.

### 5.4 Client-Side Interactivity Plan

| Current React Pattern | HTMX Equivalent |
|-----------------------|-----------------|
| `useQuery` → render | Server renders data directly in HTML template |
| Search with debounce | `<input hx-get="/warnings" hx-trigger="keyup changed delay:300ms" hx-target="#results">` |
| Pagination | `<a hx-get="/warnings?page=2" hx-target="#results" hx-push-url="true">` |
| Tab switching | `<button hx-get="/companies/123?tab=changes" hx-target="#tab-content">` |
| Filter chips | `<button hx-get="/insights?type=supply_chain" hx-target="#results">` |
| Acknowledge warning | `<button hx-post="/api/warnings/123/acknowledge" hx-swap="outerHTML">` |
| Bookmark insight | `<button hx-post="/api/insights/123/bookmark" hx-swap="outerHTML">` |
| Delete warnings (bulk) | `<form hx-delete="/api/warnings/bulk-delete" hx-confirm="Delete selected?">` |
| AI analysis trigger | `<button hx-post="/api/insights/123/analyze" hx-target="#analysis-panel" hx-indicator="#spinner">` |
| Sort toggle | `<th hx-get="/companies?sort=threat_score&dir=desc" hx-target="#table-body">` |
| Toast notification | Response header `HX-Trigger: {"showToast": {"message": "Saved", "type": "success"}}` |

**What HTMX cannot do (needs JS):**
1. **Recipe builder** — dynamic add/remove of form sections → Alpine.js or ~100 lines vanilla JS
2. **Force graph** — physics simulation, RAF, SVG manipulation → keep as standalone JS
3. **CSV export** — client-side file generation → ~20 lines vanilla JS
4. **Offline detection** — `navigator.onLine` listener → ~10 lines vanilla JS

---

## 6. Feature-by-Feature Migration Map

### 6.1 Template Structure

Each current React page maps to an Askama template struct:

```rust
// Example: warnings list page
#[derive(Template)]
#[template(path = "pages/warnings/list.html")]
struct WarningsListTemplate {
    warnings: Vec<WarningItem>,
    total: i64,
    page: i64,
    page_size: i64,
    severity_filter: Option<String>,
    status_filter: Option<String>,
    type_filter: Option<String>,
    severity_counts: SeverityCounts,
}

async fn warnings_list(
    State(state): State<AppState>,
    session: Session,           // extracted by middleware
    Query(params): Query<WarningListParams>,
) -> impl IntoResponse {
    let warnings = state.store.list_warnings(&params).await?;
    WarningsListTemplate {
        warnings: warnings.data,
        total: warnings.total,
        page: params.page.unwrap_or(1),
        // ...
    }
}
```

### 6.2 Component → Partial Mapping

| React Component (ui.tsx) | Askama Partial | Notes |
|--------------------------|---------------|-------|
| `PageHeader` | `partials/page_header.html` | `{% include %}` |
| `StatCard` | `partials/stat_card.html` | Macro with parameters |
| `SurfaceCard` | `partials/surface_card.html` | Collapsible via HTMX |
| `WarningCard` | `partials/warning_card.html` | Replaces React component |
| `InsightCard` | `partials/insight_card.html` | Bookmark via `hx-post` |
| `CompanyCard` | `partials/company_card.html` | |
| `PersonCard` | `partials/person_card.html` | |
| `CompetitorCard` | `partials/competitor_card.html` | |
| `SeverityBadge` | Askama macro | `{% call severity_badge(sev) %}` |
| `StatusBadge` | Askama macro | |
| `ProgressBar` | Inline HTML | `<div style="width:{{ pct }}%">` |
| `ScoreRing` | `partials/score_ring.html` | SVG template |
| `Pagination` | `partials/pagination.html` | HTMX links |
| `FilterChip` | Inline `<button>` | HTMX get with active class |
| `DataTable` | Askama `{% for %}` loop | |
| `SparkBar` | SVG `<rect>` elements | |
| `Toast` | JS function + CSS | ~30 lines total |
| `AppShell` | `base.html` | Sidebar + header in layout template |
| `InteractiveGraph` | `graph.js` | Standalone JS file |

### 6.3 Icon Migration (36 Lucide Icons → SVG Sprite)

Current approach re-exports lucide-react components. Migration: create a single SVG sprite sheet containing all 36 used icons, reference via `<svg><use href="/static/icons/sprite.svg#icon-name"/></svg>`.

Icons used: `ShieldAlert`, `Lightbulb`, `Building2`, `Users`, `BarChart3`, `Search`, `Settings`, `Shield`, `GitBranch`, `FlaskConical`, `LayoutDashboard`, `Network`, `FileText`, `AlertTriangle`, `ArrowUpRight`, `ArrowDownRight`, `Minus`, `ExternalLink`, `Clock`, `Tag`, `Eye`, `EyeOff`, `MapPin`, `Globe`, `Mail`, `Linkedin`, `Phone`, `Activity`, `TrendingUp`, `TrendingDown`, `CheckCircle2`, `XCircle`, `ChevronRight`, `ChevronDown`, `ChevronLeft`, `Copy`.

### 6.4 CSS Migration

**Tailwind is NOT needed in production.** Analysis of `globals.css`:

The current CSS only uses Tailwind for class utilities *authored in .tsx files*. In the Askama approach, we have two options:

**Option A (Recommended): Keep Tailwind as a dev-time tool.**
- Run `npx tailwindcss` as a one-time build step scanning `.html` templates
- Output a single `styles.css` (~30-50 KB) with only used classes
- Ship this alongside `globals.css`
- No Node.js runtime needed — `tailwindcss` CLI is a standalone binary
- OR use the Rust `tailwind-rs` crate (experimental) / download the standalone Tailwind CLI binary

**Option B: Replace Tailwind utility classes with semantic CSS.**
- Each component gets a CSS class: `.warning-card`, `.stat-card`, etc.
- Write the styles in `globals.css` extensions
- More maintenance but zero build tools
- Better long-term for a small team

**The custom design system (Sensei-Rams 3.0) CSS stays exactly as-is** — it's pure CSS custom properties, no preprocessing needed.

### 6.5 Dark Mode Migration

Current: `next-themes` (100 KB npm package) toggles a `dark` class on `<html>`.

Replacement (10 lines):
```html
<script>
  (function() {
    var t = localStorage.getItem('theme');
    if (t === 'dark' || (!t && matchMedia('(prefers-color-scheme:dark)').matches))
      document.documentElement.classList.add('dark');
  })();
</script>
```
Plus a toggle button that writes to `localStorage` and toggles the class.

### 6.6 WebSocket Migration

Current: 275-line `useWarningsWebSocket.ts` React hook with reconnection logic.

Replacement: ~50 lines of vanilla JS:
```javascript
(function() {
  var ws, attempts = 0, maxAttempts = 10;
  function connect() {
    var proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
    ws = new WebSocket(proto + '//' + location.host + '/ws/warnings');
    ws.onmessage = function(e) { showToast(JSON.parse(e.data).title, 'warning'); };
    ws.onclose = function() { if (++attempts < maxAttempts) setTimeout(connect, Math.min(1000 * Math.pow(2, attempts), 30000)); };
    ws.onopen = function() { attempts = 0; };
  }
  connect();
})();
```

---

## 7. Migration Phases

### Phase 0: Pre-Migration Cleanup (1-2 days)

Remove unused npm dependencies, validate current feature parity, establish baseline metrics.

### Phase 1: Infrastructure Setup (2-3 days)

Add Askama + HTMX + static serving to `apex-api`. Create base template. Serve `/login` as first HTML page.

### Phase 2: Core Pages (5-7 days)

Migrate Tier 1 pages (13 pure data-display pages) one at a time. Each page = 1 Askama template + 1 Axum handler.

### Phase 3: Interactive Pages (3-5 days)

Migrate Tier 2 pages (warnings, insights detail, recipes). Add HTMX patterns for mutations. Extract graph JS.

### Phase 4: Cutover & Cleanup (1-2 days)

Remove Next.js service. Update nginx. Delete `frontend/` directory. Update systemd.

**Total estimated effort: 11-17 days.**

---

## 8. Risk Assessment

### 8.1 High-Risk Items

| Risk | Probability | Impact | Mitigation |
|------|------------|--------|------------|
| **E2E test regression** | Medium | High | Keep Playwright tests; point at new URLs. Visual snapshots catch CSS drift. |
| **Recipe builder complexity** | Medium | Medium | Most complex client form (751 lines). Port last; may need Alpine.js. |
| **Accessibility regression** | Low | High | Askama HTML is semantic by default. HTMX preserves focus. Test with screen reader. |
| **Dark mode flash** | Low | Low | Inline `<script>` in `<head>` runs before paint — no flash. |
| **Performance regression on graph** | Low | Medium | Graph JS is already vanilla SVG — extraction is mechanical. |

### 8.2 Low-Risk Items

| Risk | Why Low |
|------|---------|
| Auth port | Already have SHA-256 + HMAC in Rust. Same algorithm, same constants. |
| Data fetching | Every useQuery maps 1:1 to existing PgStore methods already used by API routes. |
| Styling | globals.css is pure CSS — copy to `static/css/` unchanged. |
| Deployment | Single binary simplifies deployment vs current 2-service setup. |

### 8.3 What Could Go Wrong

1. **Tailwind class usage discovery** — Need to audit every .tsx file for Tailwind classes used in JSX and ensure they're preserved in templates. Missing classes = broken styles.

2. **Chart rendering** — The dashboard PieChart uses recharts (React wrapper around D3). Need a replacement. Options: Chart.js (~16 KB), vanilla SVG `<circle>` elements, or server-side SVG generation.

3. **AI analysis streaming** — `analyzeInsight` / `analyzeWarning` return complex structured JSON that gets rendered into 10+ subsections. The template must handle all fields. HTMX's `hx-swap` handles this if the server returns pre-rendered HTML.

4. **Keyboard shortcuts** — AppShell has `Alt+1` through `Alt+0` shortcuts. Need ~20 lines of vanilla JS.

---

## 9. Detailed Migration Checklist

### Phase 0: Pre-Migration Cleanup

- [ ] **0.1** Remove unused npm dependencies from `package.json` (16 packages: all @radix-ui, react-hook-form, @hookform/resolvers, zod, axios, class-variance-authority, clsx, tailwind-merge, server-only, @tanstack/react-table, @tanstack/react-query-devtools)
- [ ] **0.2** Run current E2E tests (`npm run test:ui`) and save screenshot baselines
- [ ] **0.3** Record baseline metrics on server: `free -h`, `ps aux --sort=-%mem`, page load times for all 22 routes via `curl -o /dev/null -w '%{time_total}' https://starzerp.fi/...`
- [ ] **0.4** Document every Tailwind utility class used across all .tsx files: `grep -ohP 'className="[^"]*"' frontend/src/**/*.tsx | tr ' ' '\n' | sort -u > tailwind-classes-used.txt`
- [ ] **0.5** Extract all 36 Lucide icon SVG paths into a single sprite file `sprite.svg`
- [ ] **0.6** Verify all 30 fetch functions in `api.ts` map 1:1 to existing Rust API endpoints in `crates/api/src/main.rs`

### Phase 1: Infrastructure Setup

- [ ] **1.1** Add dependencies to `crates/api/Cargo.toml`:
  ```toml
  askama = { version = "0.12", features = ["with-axum"] }
  askama_axum = "0.4"
  tower-http = { workspace = true, features = ["cors", "trace", "compression-gzip", "fs"] }
  ```
- [ ] **1.2** Create directory structure:
  ```
  crates/api/templates/         ← Askama scans this by default
  crates/api/static/css/
  crates/api/static/fonts/
  crates/api/static/js/
  crates/api/static/icons/
  ```
- [ ] **1.3** Copy `frontend/src/app/globals.css` → `crates/api/static/css/globals.css`
- [ ] **1.4** Copy `frontend/public/fonts/*.woff2` → `crates/api/static/fonts/`
- [ ] **1.5** Download HTMX 2.0 minified → `crates/api/static/js/htmx.min.js`
- [ ] **1.6** Create `crates/api/static/js/app.js` with:
  - Dark mode toggle (read/write localStorage + class toggle)
  - Toast notification system (CSS + JS, ~30 lines)
  - WebSocket warnings listener (~50 lines)
  - Keyboard shortcuts (Alt+1...0 for nav)
  - Offline detection (navigator.onLine)
  - CSV export helper function
  - Clipboard copy helper
- [ ] **1.7** Create `crates/api/static/icons/sprite.svg` with all 36 icon `<symbol>` elements
- [ ] **1.8** Create `crates/api/templates/base.html`:
  - `<!DOCTYPE html>`, charset, viewport meta
  - `<link>` for fonts (preload + stylesheet)
  - `<link>` for globals.css
  - `<script>` inline for dark mode (FOUC prevention)
  - `<script src="/static/js/htmx.min.js" defer>`
  - `<script src="/static/js/app.js" defer>`
  - Skip link for accessibility
  - Sidebar navigation (13 items) with active state via Askama `{% if %}` on current path
  - Header with breadcrumbs, search input (hx-get), user display, logout button
  - `{% block content %}{% endblock %}` main area
  - Warning badge via `hx-get="/api/warnings?status=unacknowledged&limit=0" hx-trigger="every 30s"` for live count
  - RAMS chassis 8px border frame
- [ ] **1.9** Create session middleware in `crates/api/src/middleware/session.rs`:
  - Read `apex_session` cookie
  - Verify HMAC SHA-256 signature (same algorithm as auth.ts)
  - Check 24h expiry
  - Reject → redirect to `/login`
  - Attach `Session { username, issued_at }` to request extensions
- [ ] **1.10** Add `ServeDir` for static files:
  ```rust
  .nest_service("/static", ServeDir::new("static")
      .precompressed_gzip()
      .append_index_html_on_directories(false))
  ```
- [ ] **1.11** Add HTML routes router merged into existing app:
  ```rust
  let html = Router::new()
      .route("/", get(web::dashboard))
      .route("/login", get(web::login_page).post(web::login_submit))
      .route("/logout", post(web::logout))
      // ... all page routes
      .route_layer(middleware::from_fn_with_state(state.clone(), require_session));
  ```
- [ ] **1.12** Create `/login` page (first end-to-end test):
  - Askama template with form (username + password)
  - POST handler: validate credentials (reuse existing SHA-256 logic), create session cookie, redirect to `/`
  - GET handler: render login page (standalone layout, no shell)
  - Test: can log in, cookie set, redirect works
- [ ] **1.13** Create `/logout` handler: clear cookie, redirect to `/login`
- [ ] **1.14** Run: `cargo build --release` — verify templates compile, binary starts, login works

### Phase 2: Tier 1 Pages — Pure Data Display

Each step: create template + handler + partial components. Test alongside existing Next.js (run both on different ports).

#### Dashboard
- [ ] **2.1** Create `templates/pages/dashboard.html`:
  - 4 KPI StatCards (warnings, insights, companies, API status)
  - Recent warnings list (top 5)
  - Region distribution (SVG pie chart or server-rendered `<svg>` circles)
  - Warning sparkline
  - Security center stats
  - Insights summary
- [ ] **2.2** Create `StatCard` Askama macro in `templates/macros.html`
- [ ] **2.3** Create dashboard handler: `async fn dashboard(State, Session) → DashboardTemplate`
  - Query: `store.get_dashboard_stats()`, `store.list_warnings(top 5)`, `store.list_insights(top 5)`, `store.get_security_summary()`
  - Render all data server-side (no client fetch needed)
- [ ] **2.4** Create SVG pie chart partial for region distribution (replace recharts PieChart)

#### Warnings List
- [ ] **2.5** Create `templates/pages/warnings/list.html`:
  - Severity/status/type filter bar with HTMX filter chips
  - Warning cards list with checkbox selection
  - Pagination with HTMX
  - KPI cards per severity count
  - Bulk delete form + clear all (hx-confirm)
  - CSV export button (client-side JS)
- [ ] **2.6** Create `WarningCard` partial: `templates/partials/warning_card.html`
- [ ] **2.7** Create `SeverityBadge` macro
- [ ] **2.8** Create `Pagination` partial with HTMX page links
- [ ] **2.9** Create `FilterBar` + `FilterChip` partials
- [ ] **2.10** Create warnings list handler with query params (severity, status, type, page, search)
- [ ] **2.11** Create HTMX-only partial response: when `HX-Request` header present, return just the `#results` fragment (no full page)

#### Warning Detail
- [ ] **2.12** Create `templates/pages/warnings/detail.html`:
  - Severity-colored border
  - Confidence meter (CSS bar)
  - Status/source badges
  - Acknowledgement section with history
  - AI analysis panel (initially hidden, loaded via hx-post)
- [ ] **2.13** Create acknowledge handler: `POST /warnings/:id/acknowledge` → returns updated card HTML
- [ ] **2.14** Create analysis handler: `POST /warnings/:id/analyze` → returns rendered analysis HTML panel
  - Render all 10+ subsections: threat assessment, severity justification, indicators table, detailed analysis, source analysis, impact assessment, response plan, escalation criteria, monitoring indicators, confidence
- [ ] **2.15** Add `hx-indicator` spinner for analysis button

#### Insights List
- [ ] **2.16** Create `templates/pages/insights/list.html`:
  - 16 insight type filter chips
  - Search with HTMX debounce
  - InsightCard list with bookmark toggle
  - Pagination
  - CSV export
- [ ] **2.17** Create `InsightCard` partial with `hx-post` bookmark button
- [ ] **2.18** Create insights list handler with filters (type, search, page)
- [ ] **2.19** Implement HTMX bookmark toggle: POST returns updated button HTML

#### Insight Detail
- [ ] **2.20** Create `templates/pages/insights/detail.html`:
  - Confidence/impact/sources/observations/entities
  - Related warnings + related insights
  - AI analysis panel (same pattern as warning detail)
  - Bookmark toggle
- [ ] **2.21** Create analysis handler: `POST /insights/:id/analyze` → rendered HTML

#### Companies List
- [ ] **2.22** Create `templates/pages/companies/list.html`:
  - Search with HTMX debounce
  - Sort controls (name/threat_score/updated_at) via HTMX
  - Region filter chips (12 regions)
  - CompanyCard grid
  - Pagination
  - CSV export + region pie chart
- [ ] **2.23** Create `CompanyCard` partial with ScoreRing SVG
- [ ] **2.24** Create companies list handler

#### Company Detail
- [ ] **2.25** Create `templates/pages/companies/detail.html`:
  - 3 tabs (Profile, Changes, Dossier) via HTMX tab loading
  - StatCards (threat score, overlap, sites, key persons)
  - Capabilities/certifications lists
  - Sites DataTable
  - Key persons with links
  - Recent events
- [ ] **2.26** Create HTMX tab handlers: `GET /companies/:id?tab=changes` → partial
- [ ] **2.27** Create dossier entries handler: `GET /companies/:id?tab=dossier` → partial

#### Persons Detail
- [ ] **2.28** Create `templates/pages/persons/detail.html`:
  - 5 tabs (Profile, Intelligence, Network, History, Engagement) via HTMX
  - Priority vector ProgressBars (5 bars)
  - Data completeness SVG ring
  - Affiliations table
  - Timeline
  - Decision profile (behavioral traits)
  - Engagement brief
  - Peers table
  - Role history with confidence bars
- [ ] **2.29** Create 5 tab partial templates
- [ ] **2.30** Create persons detail handler with tab partials
- [ ] **2.31** Port helper functions to Rust: `country_flag()` (emoji), `getTierColor()`, `getRoleFamilyColor()`

#### Competitors
- [ ] **2.32** Create `templates/pages/competitors/list.html`:
  - Grid/changes view toggle via HTMX
  - CompetitorCard with 5 metric bars
  - Change timeline with color-coded types
  - Pagination
- [ ] **2.33** Create `CompetitorCard` partial
- [ ] **2.34** Create competitors handler

#### Memos
- [ ] **2.35** Create `templates/pages/memos/list.html`:
  - Sidebar archive list (HTMX partial load)
  - Detail panel with metrics grid
  - Executive summary, priority sections, action items
- [ ] **2.36** Create memo detail partial template
- [ ] **2.37** Create memos handler

#### Search
- [ ] **2.38** Create `templates/pages/search.html`:
  - Search input with HTMX (hx-get with delay)
  - Grouped results by entity type
  - Color-coded type badges
  - Score display
  - Navigation links
- [ ] **2.39** Create search handler

#### Graph
- [ ] **2.40** Create `templates/pages/graph.html`:
  - Entity type filter (HTMX)
  - Graph/Table toggle (HTMX)
  - KPI cards (companies/POIs/warnings/insights/edges)
  - SVG container for force-directed graph
  - Edge type counts table
  - `<script src="/static/js/graph.js">`
- [ ] **2.41** Extract `interactive-graph.tsx` to standalone `graph.js`:
  - Remove React hooks (useState → let variables)
  - Remove framer-motion (use CSS transitions)
  - Keep: force simulation, SVG rendering, zoom/pan, node click/hover
  - Fetch graph data via `fetch('/api/graph')` on page load
  - ~200-250 lines vanilla JS
- [ ] **2.42** Test graph: zoom, pan, node click, hover labels, edge rendering

#### Security
- [ ] **2.43** Create `templates/pages/security.html`:
  - 4 tabs (Overview, DNS, Lookalike, KEV) via HTMX
  - DNS posture checks (SPF/DKIM/DMARC) per domain
  - Lookalike domain list with threat type/distance
  - KEV CVE matching table
  - Acknowledge action (hx-post)
- [ ] **2.44** Create tab partial templates
- [ ] **2.45** Create security handler

#### Settings
- [ ] **2.46** Create `templates/pages/settings.html`:
  - 5 tabs (System, Regions, Watchlist, Schedules, API Keys)
  - Health checks display
  - 12 regions with toggles
  - Worker jobs table (17 jobs)
  - API key table (10 services)
- [ ] **2.47** Create settings handler

#### Admin
- [ ] **2.48** Create `templates/pages/admin.html`:
  - System health dashboard
  - 7 trigger scan buttons (hx-post with hx-indicator)
  - Crawl pipeline stats
  - Recipe performance table
  - POI coverage metrics
- [ ] **2.49** Create trigger scan handler (reuse existing `post_trigger_scan` API)
- [ ] **2.50** Test all 7 scan triggers

#### Error Pages
- [ ] **2.51** Create `templates/pages/404.html` (not found)
- [ ] **2.52** Create `templates/pages/500.html` (server error)
- [ ] **2.53** Add fallback handler for unmatched routes

### Phase 3: Interactive Features

#### Recipe Builder
- [ ] **3.1** Create `templates/pages/recipes/list.html`:
  - Status filter (All/Production/Staging/Deprecated)
  - DataTable with recipe stats
  - Promote/deprecate buttons (hx-post)
- [ ] **3.2** Create `templates/pages/recipes/new.html`:
  - Form with Signal/Transform/Threshold/Action sections
  - "Add" buttons use either HTMX `hx-get="/recipes/new/signal-editor?index=N"` to load new section HTML, or Alpine.js `x-data` for pure client-side cloning
  - AI generation button (`hx-post="/api/llm/generate-recipe"`)
  - Test button (`hx-post="/api/proxy/recipes/test"`)
  - Submit (`hx-post="/api/recipes"`)
- [ ] **3.3** Create `crates/api/static/js/recipe-builder.js`:
  - Array section management (add/remove signal/transform/threshold/action)
  - Validation (name + 1 signal + 1 threshold + 1 action)
  - ~100-150 lines
- [ ] **3.4** Test recipe creation end-to-end: AI generate → edit → test → save

#### Real-Time Features
- [ ] **3.5** Implement WebSocket connection in `app.js`:
  - Connect to `/ws/warnings`
  - On message → `showToast(data.title, 'warning')`
  - Reconnection with exponential backoff
  - ~50 lines
- [ ] **3.6** Implement HTMX warning badge poll:
  - `<span hx-get="/api/warnings/unread-count" hx-trigger="every 30s" hx-swap="innerHTML">0</span>` in nav
  - Create endpoint that returns just the count number
- [ ] **3.7** Test: trigger a warning via admin → see toast + badge update

#### Client-Side Utilities
- [ ] **3.8** Implement CSV export in `app.js`:
  - `function exportCSV(url, filename)` — fetch JSON, convert to CSV, trigger download
  - Used by: warnings list, insights list, companies list, persons list
- [ ] **3.9** Implement clipboard copy for CopyIdButton:
  - `function copyId(text, el)` — `navigator.clipboard.writeText(text)` with visual feedback
- [ ] **3.10** Implement keyboard shortcuts:
  - `Alt+1` through `Alt+0` → navigate to 10 primary pages
  - ~20 lines in `app.js`
- [ ] **3.11** Implement offline detection:
  - `window.addEventListener('online'/'offline')` → show/hide banner
  - ~10 lines

### Phase 4: Tailwind CSS Compilation

- [ ] **4.1** Option A: Install Tailwind CLI standalone binary (no Node.js needed):
  ```bash
  curl -sLO https://github.com/tailwindlabs/tailwindcss/releases/latest/download/tailwindcss-linux-arm64
  chmod +x tailwindcss-linux-arm64
  ```
- [ ] **4.2** Create `tailwind.config.js` scanning `crates/api/templates/**/*.html` for class usage
- [ ] **4.3** Generate `crates/api/static/css/tailwind.css` with only used utility classes
- [ ] **4.4** Add `<link rel="stylesheet" href="/static/css/tailwind.css">` to `base.html`
- [ ] **4.5** Verify all pages render correctly with generated CSS (compare screenshots)
- [ ] **4.6** Alternative: If Tailwind utility classes are few enough, manually extract them into `globals.css` custom classes and skip Tailwind entirely

### Phase 5: Cutover

- [ ] **5.1** Run full E2E test suite (Playwright) against Rust-served pages
- [ ] **5.2** Compare visual regression screenshots (current vs new)
- [ ] **5.3** Load test: `wrk -t4 -c100 -d30s https://starzerp.fi/` for TTFB comparison
- [ ] **5.4** Verify all HTMX interactions work:
  - [ ] Search with debounce (warnings, insights, companies, search page)
  - [ ] Pagination (all list pages)
  - [ ] Tab switching (company detail 3 tabs, person detail 5 tabs, security 4 tabs, settings 5 tabs)
  - [ ] Filter chips (warnings severity/status/type, insights 16 types, companies region, competitors)
  - [ ] Sort toggles (companies)
  - [ ] Acknowledge warning
  - [ ] Bookmark/unbookmark insight
  - [ ] Bulk delete warnings
  - [ ] Clear all warnings
  - [ ] AI analysis trigger (insights detail, warnings detail)
  - [ ] Admin scan triggers (7 buttons)
  - [ ] Recipe promote/deprecate
  - [ ] Recipe builder (create/test/generate)
  - [ ] Graph force layout (zoom, pan, click, hover labels)
  - [ ] CSV export (warnings, insights, companies)
  - [ ] Dark mode toggle
  - [ ] Keyboard shortcuts (Alt+1...0)
  - [ ] Offline banner
  - [ ] WebSocket toast notifications
  - [ ] Login/logout flow
  - [ ] 404 page
  - [ ] Copy ID button
- [ ] **5.5** Update nginx config:
  ```nginx
  # Remove: upstream apexintel_frontend
  # Remove: all location blocks proxying to apexintel_frontend
  # Change: location / { proxy_pass http://apexintel_api; }
  # Keep: location /api/ (already points to api)
  # Keep: location /ws/ (already points to api)
  # Add: location /static/ { proxy_pass http://apexintel_api; proxy_cache_valid 200 365d; add_header Cache-Control "public, immutable, max-age=31536000"; }
  ```
- [ ] **5.6** Stop and disable Next.js service:
  ```bash
  systemctl stop apexintel-frontend.service
  systemctl disable apexintel-frontend.service
  rm /etc/systemd/system/apexintel-frontend.service
  systemctl daemon-reload
  ```
- [ ] **5.7** Remove Node.js frontend from server:
  ```bash
  rm -rf /opt/apexintel/frontend
  ```
- [ ] **5.8** Optionally remove Node.js from server:
  ```bash
  apt remove nodejs npm  # if nothing else uses it
  ```
- [ ] **5.9** Verify memory savings: `free -h` (expect ~100 MB freed)
- [ ] **5.10** Verify disk savings: `df -h /` (expect ~760 MB freed)
- [ ] **5.11** Update `DEPLOYMENT.md` with new single-binary deployment instructions
- [ ] **5.12** Update CI/CD pipeline: remove `npm install` + `npm build` steps
- [ ] **5.13** Archive `frontend/` directory in repo (move to `archive/frontend-legacy/` or delete)

### Phase 6: Post-Migration Optimization

- [ ] **6.1** Enable `askama` compile-time template checking in CI
- [ ] **6.2** Add Brotli pre-compression for static assets (`tower-http` `precompressed_br`)
- [ ] **6.3** Tune HTMX: add `hx-boost="true"` to `<body>` for automatic SPA-like navigations (full page loads become HTMX swaps)
- [ ] **6.4** Add `ETag` / `Last-Modified` headers to templates for browser caching
- [ ] **6.5** Profile Axum handler response times — target <2ms for all HTML responses
- [ ] **6.6** Consider inlining critical CSS in `<head>` for FCP optimization
- [ ] **6.7** Add `<meta name="htmx-config" content='{"historyCacheSize": 10}'>` for HTMX history cache
- [ ] **6.8** Update Playwright E2E tests to work with new HTML structure (selectors may change)
- [ ] **6.9** Set up template hot-reload for development (askama supports this with a feature flag)
- [ ] **6.10** Document the new architecture in `docs/development/frontend.md`

---

## Appendix A: Template Example

### Current React (warnings list, simplified)

```tsx
"use client";
export default function WarningsPage() {
  const { data } = useQuery({ queryKey: ["warnings", page, severity], queryFn: () => fetchWarnings({ page, severity }) });
  return (
    <div>
      <PageHeader title="Warnings" icon={ShieldAlert} />
      <FilterBar>
        {severities.map(s => <FilterChip key={s} active={severity === s} onClick={() => setSeverity(s)}>{s}</FilterChip>)}
      </FilterBar>
      {data?.data.map(w => <WarningCard key={w.id} warning={w} />)}
      <Pagination page={page} total={data?.total} pageSize={25} onPageChange={setPage} />
    </div>
  );
}
```

### New Askama + HTMX (equivalent)

**Template** (`templates/pages/warnings/list.html`):
```html
{% extends "base.html" %}
{% block content %}
<div id="warnings-page">
  {% call page_header("Warnings", "shield-alert") %}

  <div class="flex gap-2 overflow-x-auto py-2" role="group" aria-label="Severity filter">
    {% for s in ["all", "critical", "high", "medium", "low"] %}
    <button
      hx-get="/warnings?severity={{ s }}"
      hx-target="#results"
      hx-push-url="true"
      class="apex-card px-3 py-1 text-sm {% if severity_filter == s %}bg-primary text-primary-foreground{% endif %}"
    >{{ s|title }}</button>
    {% endfor %}
  </div>

  <div id="results">
    {% for w in warnings %}
      {% include "partials/warning_card.html" %}
    {% endfor %}
    {% include "partials/pagination.html" %}
  </div>
</div>
{% endblock %}
```

**Handler** (`crates/api/src/routes/web/warnings.rs`):
```rust
#[derive(Template)]
#[template(path = "pages/warnings/list.html")]
struct WarningsListPage {
    warnings: Vec<WarningRow>,
    total: i64,
    page: i64,
    severity_filter: String,
    // ... injected by base.html
    current_path: String,
    username: String,
}

async fn warnings_list(
    State(state): State<AppState>,
    session: Session,
    Query(p): Query<WarningListParams>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let result = state.store.list_warnings(/* params */).await?;

    // If HTMX partial request, return just #results fragment
    if headers.contains_key("hx-request") {
        return WarningsResultsPartial { warnings: result.data, ... }.into_response();
    }

    WarningsListPage {
        warnings: result.data,
        total: result.total,
        page: p.page.unwrap_or(1),
        severity_filter: p.severity.unwrap_or("all".into()),
        current_path: "/warnings".into(),
        username: session.username,
    }.into_response()
}
```

---

## Appendix B: Dependency Comparison

### Before (Node.js + Next.js)

```
Runtime:    Node.js 20.20.0 (V8 engine + libuv + npm)
Framework:  Next.js 14.1.0 (React 18, Webpack, SWC compiler)
Packages:   48 direct dependencies → 1,200+ transitive deps
Disk:       587 MB node_modules + 170 MB .next = 757 MB
Memory:     112 MB RSS (V8 heap + compiled JS + Next.js runtime)
Binary:     None (interpreted)
Build:      npm install (30-60s) + npm run build (20-40s)
Processes:  1 (next-server, 11 threads)
Startup:    ~5s (Node.js init + Next.js page compilation)
```

### After (Rust + Askama + HTMX)

```
Runtime:    None (compiled native binary)
Framework:  Axum 0.7 (already deployed) + Askama 0.12 (compiled templates)
Packages:   2 new Rust crates (askama, askama_axum) + 1 JS file (htmx.min.js, 14 KB)
Disk:       +3 MB (binary growth) + 2 MB (static assets) = 5 MB total
Memory:     +5-8 MB RSS on apex-api (template rendering + static cache)
Binary:     Single apex-api binary (now ~22 MB, was 19 MB)
Build:      cargo build --release (+1-2s for templates)
Processes:  0 new (merged into existing apex-api)
Startup:    +0ms (templates compiled into binary)
```

---

## Appendix C: Files to Create (Complete List)

| # | File | Purpose | Est. Lines |
|---|------|---------|------------|
| 1 | `crates/api/templates/base.html` | Root layout (shell, nav, scripts) | ~150 |
| 2 | `crates/api/templates/base_standalone.html` | Login-only layout (no shell) | ~40 |
| 3 | `crates/api/templates/macros.html` | Reusable macros (badges, cards, icons) | ~200 |
| 4 | `crates/api/templates/pages/dashboard.html` | Dashboard | ~120 |
| 5 | `crates/api/templates/pages/warnings/list.html` | Warnings list | ~80 |
| 6 | `crates/api/templates/pages/warnings/detail.html` | Warning detail | ~150 |
| 7 | `crates/api/templates/pages/insights/list.html` | Insights list | ~70 |
| 8 | `crates/api/templates/pages/insights/detail.html` | Insight detail + analysis | ~200 |
| 9 | `crates/api/templates/pages/companies/list.html` | Companies list | ~80 |
| 10 | `crates/api/templates/pages/companies/detail.html` | Company detail (3 tabs) | ~200 |
| 11 | `crates/api/templates/pages/persons/detail.html` | Person detail (5 tabs) | ~300 |
| 12 | `crates/api/templates/pages/competitors/list.html` | Competitors | ~100 |
| 13 | `crates/api/templates/pages/memos/list.html` | Memos | ~100 |
| 14 | `crates/api/templates/pages/search.html` | Search | ~60 |
| 15 | `crates/api/templates/pages/graph.html` | Graph | ~80 |
| 16 | `crates/api/templates/pages/security.html` | Security (4 tabs) | ~150 |
| 17 | `crates/api/templates/pages/settings.html` | Settings (5 tabs) | ~120 |
| 18 | `crates/api/templates/pages/admin.html` | Admin | ~100 |
| 19 | `crates/api/templates/pages/recipes/list.html` | Recipes list | ~60 |
| 20 | `crates/api/templates/pages/recipes/new.html` | Recipe builder | ~200 |
| 21 | `crates/api/templates/pages/login.html` | Login | ~60 |
| 22 | `crates/api/templates/pages/404.html` | Not found | ~20 |
| 23 | `crates/api/templates/pages/500.html` | Server error | ~20 |
| 24 | `crates/api/templates/partials/warning_card.html` | Warning card | ~30 |
| 25 | `crates/api/templates/partials/insight_card.html` | Insight card | ~30 |
| 26 | `crates/api/templates/partials/company_card.html` | Company card | ~30 |
| 27 | `crates/api/templates/partials/person_card.html` | Person card | ~25 |
| 28 | `crates/api/templates/partials/competitor_card.html` | Competitor card | ~30 |
| 29 | `crates/api/templates/partials/pagination.html` | Pagination | ~25 |
| 30 | `crates/api/templates/partials/stat_card.html` | Stat card | ~15 |
| 31 | `crates/api/templates/partials/score_ring.html` | SVG score ring | ~15 |
| 32 | `crates/api/templates/partials/analysis_panel.html` | AI analysis render | ~100 |
| 33 | `crates/api/src/web/mod.rs` | Web route module | ~50 |
| 34 | `crates/api/src/web/templates.rs` | Template structs | ~200 |
| 35 | `crates/api/src/web/dashboard.rs` | Dashboard handler | ~40 |
| 36 | `crates/api/src/web/warnings.rs` | Warnings handlers | ~80 |
| 37 | `crates/api/src/web/insights.rs` | Insights handlers | ~80 |
| 38 | `crates/api/src/web/companies.rs` | Companies handlers | ~60 |
| 39 | `crates/api/src/web/persons.rs` | Persons handlers | ~60 |
| 40 | `crates/api/src/web/competitors.rs` | Competitors handler | ~40 |
| 41 | `crates/api/src/web/memos.rs` | Memos handler | ~40 |
| 42 | `crates/api/src/web/search.rs` | Search handler | ~30 |
| 43 | `crates/api/src/web/graph.rs` | Graph handler | ~30 |
| 44 | `crates/api/src/web/security.rs` | Security handler | ~50 |
| 45 | `crates/api/src/web/settings.rs` | Settings handler | ~40 |
| 46 | `crates/api/src/web/admin.rs` | Admin handler | ~50 |
| 47 | `crates/api/src/web/recipes.rs` | Recipes handlers | ~60 |
| 48 | `crates/api/src/web/auth.rs` | Login/logout handlers | ~60 |
| 49 | `crates/api/src/middleware/session.rs` | Session validation | ~60 |
| 50 | `static/css/globals.css` | Design system CSS (copied) | ~406 |
| 51 | `static/css/tailwind.css` | Generated utility CSS | ~500 |
| 52 | `static/js/htmx.min.js` | HTMX library (vendored) | — |
| 53 | `static/js/app.js` | Dark mode, toast, WS, shortcuts | ~150 |
| 54 | `static/js/graph.js` | Force-directed graph | ~250 |
| 55 | `static/js/recipe-builder.js` | Dynamic form builder | ~150 |
| 56 | `static/icons/sprite.svg` | 36 Lucide icons | ~200 |
| 57 | `static/fonts/` (4 files) | Copied from frontend/public | — |
| — | **TOTAL new Rust code** | | **~1,070** |
| — | **TOTAL new templates** | | **~2,700** |
| — | **TOTAL new JS** | | **~550** |
| — | **TOTAL new CSS** | | **~906** |
| — | **GRAND TOTAL** | | **~5,226** |

vs. current: **~11,033 lines** of TypeScript/CSS → **53% reduction** in source code.

---

## Appendix D: Decision Log

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Framework | Askama+HTMX (not Leptos/Dioxus) | No WASM overhead for a read-heavy dashboard app. Server-rendered HTML is faster and simpler. |
| Merge vs separate binary | Merge into apex-api | Fewer processes, shared database pool, simpler deployment. |
| Template engine | Askama (not Tera/Maud) | Compile-time validation, zero runtime overhead, Jinja2 syntax is familiar. |
| Tailwind strategy | Standalone CLI binary (no Node.js) | Preserves utility-first authoring without requiring Node.js runtime. |
| Interactive graph | Vanilla JS extraction | 445 lines of React → ~250 lines of plain JS. Physics sim doesn't benefit from Rust/WASM. |
| Recipe builder | Vanilla JS or Alpine.js | Too dynamic for pure HTMX. ~150 lines handles the add/remove/validate lifecycle. |
| Charts | SVG templates (not Chart.js) | Only 1 pie chart exists. Server-generated `<svg>` avoids another JS dependency. |
| Icons | SVG sprite sheet | Eliminates lucide-react dependency. Single HTTP request for all icons. |
| E2E tests | Keep Playwright | Tests the HTML output regardless of how it's generated. Update selectors as needed. |
