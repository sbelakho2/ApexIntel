# Sensei-Rams Implementation Guide (ApexIntel)

> Technical implementation guide for applying Sensei-Rams consistently in ApexIntel.

---

## 1. Project Configuration

### 1.1 Tailwind Requirements

`frontend/tailwind.config.ts` must include:

- `rams` color family (`chassis`, `module`, `panel`, `line`, `muted`, `orange`, `green`, `red`, `steel`)
- micro radii (`rams-sm`, `rams-md`, `rams-lg`)
- Rams spacing scale (`rams-1..rams-16`)
- Rams shadows (`rams-inset`, `rams-pressed`, `rams-focus`)
- fast transition durations (`rams-instant..rams-slow`)
- plugins: `@tailwindcss/forms`, `@tailwindcss/typography`, `tailwindcss-animate`

### 1.2 Global CSS Requirements

`frontend/src/app/globals.css` must define:

- core Rams variables for light and dark themes
- anti-blur text rendering settings
- tabular numeric utility (`[data-numeric], .tabular-nums`)
- density modes (`density-compact`, `density-comfortable`, `density-expanded`)
- reduced motion fallback

---

## 2. Layout Infrastructure

### 2.1 Root Layout

`frontend/src/app/layout.tsx` provides:

- Industrial bezel frame (`fixed` border)
- App shell wrapper
- Bottom system metadata strip (desktop)

### 2.2 Shell Expectations

`frontend/src/components/app-shell.tsx` should maintain:

- rack-like left navigation
- active state rail/indicator
- compact uppercase operational metadata in top strip
- bottom padding to prevent collision with metadata bar

---

## 3. Shared UI Component Rules

`frontend/src/components/ui.tsx` is the baseline style layer.

### Required Characteristics

- Visual hierarchy via borders and section dividers
- Icon + text pairing for primary headers and empty states
- Compact uppercase labels for metrics/metadata
- Structured empty-state format (not plain sentence only)

### Stateless Components

Shared primitives should remain stateless and reusable:

- `PageHeader`
- `StatCard`
- `SurfaceCard`
- `EmptyState`
- `DataTable`

---

## 4. Rams Class Contract

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
- [ ] No `500` UI/server response regressions in screenshot tests

---

## 7. Testing Workflow

Run in `frontend/`:

```bash
npm run type-check
npm run test:ui:update
npm run test:ui
```

Use Playwright snapshots under `frontend/e2e/ui-sense-rams.spec.ts-snapshots` as the baseline artifact.

---

## 8. Migration Guidance for Existing Screens

When converting existing pages:

1. Normalize surface/background to Rams tokens
2. Move header/body into clear module containers
3. Replace plain text-only empty states with structured icon + heading + context
4. Tighten spacing to 4px grid increments
5. Re-run screenshot tests and compare

