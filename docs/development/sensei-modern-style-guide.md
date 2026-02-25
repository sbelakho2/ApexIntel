# ApexIntel Design System: Sensei-Rams (Version 3.0)

> Authoritative frontend style specification for ApexIntel.
>
> Philosophy: **Less, but better** — Dieter Rams
> Paradigm: **Industrial Functionalism for Operational Intelligence**

---

## Preamble: Why This Style Exists

ApexIntel is an operational intelligence workstation, not a consumer SaaS app. The interface must communicate:

- Permanence of data
- Precision of action
- Professional focus
- Dense, readable information

### Explicitly Rejected

- Glassmorphism and blur-heavy surfaces
- Gradient-heavy CTA styling
- Oversized rounded cards and pill buttons
- Decorative animation without functional value
- Colorful dashboard ornamentation

---

## 1. Design Principles (Rams Applied)

| Principle | ApexIntel Interpretation |
|---|---|
| Useful | Every visual element supports an intelligence task |
| Understandable | Labels, statuses, and controls are explicit |
| Unobtrusive | Data dominates over chrome |
| Honest | No fake depth or misleading affordances |
| Thorough | 4px grid discipline and consistent type rhythm |
| Minimal | Prefer fewer, clearer components per module |

---

## 2. Typography

### Font Stack

- UI: `--font-apex` + system sans
- Numeric/Code: `--font-mono`

### Scale Guidance

- 10px (`text-2xs`) system labels / metadata
- 12px (`text-xs`) secondary labels
- 14px (`text-sm`) body / table rows
- 16px (`text-base`) core content
- 20–36px for headings based on hierarchy

### Rendering Requirements

- `text-rendering: optimizeLegibility`
- `-webkit-font-smoothing: antialiased`
- `-moz-osx-font-smoothing: grayscale`
- Numeric values must use tabular numerals (`data-numeric`, `.tabular-nums`)

---

## 3. Color System

### Core Rams Tokens

- `--rams-chassis`: structural background
- `--rams-module`: module/card surface
- `--rams-panel`: inset/panel surface
- `--rams-line`: borders and dividers
- `--rams-muted`: secondary text and inactive elements
- `--rams-foreground`: primary text

### Functional Accents (semantic only)

- `--rams-orange`: primary action / active state
- `--rams-green`: operational/success
- `--rams-red`: critical/error
- `--rams-steel`: informational cue

### Rules

- Never rely on color alone for status
- Keep accents sparse and semantic
- Avoid gradients in core operational surfaces

---

## 4. Spatial System (4px Grid)

All spacing derives from 4px increments:

- 4, 8, 12, 16, 24, 32, 48, 64

Tailwind helpers:

- `rams-1`, `rams-2`, `rams-3`, `rams-4`, `rams-6`, `rams-8`, `rams-12`, `rams-16`

---

## 5. Layout Metaphor: Control Station

ApexIntel should visually read as an instrument panel:

- Industrial bezel frame at viewport edge
- Rack-like sidebar with active indicator rails
- Structural module borders instead of floating cards
- Bottom metadata/status strip on desktop

---

## 6. Component Doctrine

### Modules, Not Floating Cards

Use:

- `bg-rams-module` + `border-rams-line`
- micro radius (`2px–4px`)
- compact, information-dense header + body structure

Avoid:

- Large corner radii
- heavy outer shadows
- oversized whitespace

### Buttons

Use imperative labels (`Run`, `Acknowledge`, `Export`) with precise dimensions and border-driven states.

### Tables

- Monospace/uppercase compact headers
- consistent row rhythm
- border-defined structure
- no zebra-striping as primary hierarchy mechanism

---

## 7. Motion & Feedback

- Motion is utilitarian and minimal
- Use short durations (`50ms–200ms`)
- Respect reduced-motion user preference
- Prefer state-change signals (border, marker, label) over decorative animation

---

## 8. Do / Don’t Summary

### Do

- Use Rams tokens and structural borders
- Keep copy concise and operational
- Use icon + label for important actions
- Preserve keyboard-visible focus with strong ring

### Don’t

- Introduce gradient branding blocks
- use emoji or playful microcopy in core workflows
- add decorative loaders/transitions
- expand component variants beyond functional need

---

## 9. Source of Truth in ApexIntel

- Theme tokens: `frontend/src/app/globals.css`
- Tailwind primitives: `frontend/tailwind.config.ts`
- Shared shell/components: `frontend/src/components/app-shell.tsx`, `frontend/src/components/ui.tsx`
- Root framing: `frontend/src/app/layout.tsx`

