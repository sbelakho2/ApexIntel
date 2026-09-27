# Sensei-Rams Accessibility & WCAG Guide (ApexIntel)

> Accessibility requirements for Sensei-Rams implementation in ApexIntel.

---

## Accessibility Intent

ApexIntel treats accessibility as core product quality. An operational UI that cannot be efficiently used by all operators is a design failure.

---

## 1. WCAG 2.1 AA Baseline

### Perceivable

- Use semantic headings and landmarks
- Ensure non-text elements have accessible names
- Maintain sufficient text and UI contrast
- Never use color alone for status meaning

### Operable

- Full keyboard navigation for all controls
- Focus visible at all times
- Escape closes popovers/modals/overlays
- No keyboard trap

### Understandable

- Clear labels and predictable navigation order
- Explicit error text and correction hints
- Consistent placement for repeated controls

### Robust

- Semantic HTML first
- valid ARIA only where needed
- live regions for dynamic status updates

---

## 2. Contrast Targets

For operational text and controls:

- Normal text: `>= 4.5:1`
- Large text/UI affordances: `>= 3:1`
- Focus ring and active boundaries must remain visible against module/chassis backgrounds

Recommended color usage:

- Primary text on chassis/module: very high contrast
- Muted text only for secondary metadata
- Orange should not carry meaning alone without icon/text context

---

## 3. Focus and Keyboard Rules

- `Tab`/`Shift+Tab`: logical sequence
- `Enter`/`Space`: activate controls
- `Escape`: dismiss context UI
- icon-only buttons require `aria-label`

### Focus Appearance

Use clear border/ring treatment with strong offset against both light and dark surfaces.

---

## 4. Motion, Density, and Readability

- Respect `prefers-reduced-motion`
- Density changes must not reduce touch/click affordance below acceptable target sizes
- Tabular numbers for metrics improve readability and scanning

---

## 5. Screen Reader Landmarks

Recommended structure:

- `header` for station/shell metadata
- `nav` for rack navigation
- `main` for primary content
- optional `aside` for non-blocking supplementary status
- `footer` for system metadata strip

Include a skip-link for keyboard-first workflows where applicable.

---

## 6. QA Checklist

Before merge:

- [ ] Keyboard-only flow works on primary routes
- [ ] Focus indicator visible in all shells and cards
- [ ] No text below contrast target in light/dark modes
- [ ] Icon-only controls have accessible names
- [ ] Reduced-motion mode does not break usability

---

## 7. Validation Commands

### Server-Rendered UI (Askama/HTMX)

```bash
# Compile-time template validation (catches syntax errors)
cargo check -p apex-api

# Accessibility + visual contract specs (server must already be running)
npx playwright test -c playwright.server-ui.config.cjs e2e/server-ui-a11y.spec.js

# Full route sweep with axe checks
BASE_URL=http://127.0.0.1:9095 ADMIN_USER=admin ADMIN_PASS=adminpassword \
  node scripts/ci/e2e_server_ui.mjs

# Compile Tailwind CSS
npx tailwindcss -i crates/api/static/css/globals.css -o crates/api/static/css/tailwind.css --minify
```

The Leptos/WASM frontend was retired from the workspace
(`experiments/wasm-frontend/`); it has no build or test commands.

Use browser a11y tooling (axe/Lighthouse) during review for additional verification.
