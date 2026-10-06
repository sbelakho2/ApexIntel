# ApexIntel UI — Sensei-Rams system

The global UI is the **Sensei-Rams industrial design system** (ported from the
StarzCRM-v2 `sensei-rams.css`, v3.7). It replaces the previous Tailwind-only
look app-wide; there are no per-page styling hacks.

## Load order and files

1. `static/css/tailwind.css` — compiled layout utilities (`globals.css` source).
2. `static/css/sensei-rams.css` — the design system: tokens, sidebar, modules,
   buttons, tables, badges, andon lights, forms, modals, dark theme.
3. `static/css/apex-rams.css` — the single ApexIntel layer:
   - token alignment (Sensei v3.7 values, incl. Tailwind accent RGB triplets),
   - shell rules extracted from the former inline `<style>` blocks,
   - the `apex-*` component vocabulary mapped onto Sensei visuals,
   - toast + auth rules.

Page templates must not carry inline styles or page-specific CSS. Dynamic
values (bar heights, score colors) are the only permitted inline bindings.

## Shell

- 240px sidebar (`aside[aria-label="Main navigation"]`), mono group labels,
  orange active item; mobile off-canvas drawer (`#mobile-nav`).
- Top command bar: search (`form[action="/search"]`), command palette (⌘K),
  notifications, theme toggle, user menu (`details.apex-user-menu`).
- Status strip (system / operator / data freshness) and the fixed 5-item
  mobile tab bar (`Home | Signals | Entities | Triage | More`).
- Theme: `data-theme="light|dark"` + `.dark`, persisted in localStorage;
  dark values come from the Sensei dark token block.

## Navigation (consolidated 2026-10)

Six workflow groups; secondary screens stay routable and are linked from the
screens that own them instead of occupying sidebar space:

| Group | Items |
|---|---|
| Command Center | Overview · Executive · Triage · Activity |
| Entities | Companies · Competitors · People |
| Signals | Warnings (count badge) · Insights · Trends · Security |
| Investigations | Workspaces · Graph · Memos |
| Sales Intelligence | Pipeline · Battlecards |
| Automations | Recipes · Sources · Jobs · Admin Panel (admin only) |

Route removals from the nav (Queue, Supplier Risk, Evidence, Team Assignments,
Buying Centres) keep their URLs; `scripts/ci/e2e_server_ui.mjs` still passes
because the six required group labels, the search form, the user menu and the
mobile tab bar are unchanged.

## Dashboard rules

The Overview is one decision surface: four "what changed" cards, the action
queue, opportunities, measured health + coverage, one trend pair, and two
feeds. Duplicate KPI rows and secondary stat strips are not allowed — every
number must be actionable or a measured health signal.

## Enforcement

- Visual baselines (`e2e/server-ui-visual.spec.js-snapshots/*-linux.png`) were
  reset with this global redesign; regenerate in the Linux CI container with
  `npm run test:server-ui:specs:update` before merging.
- `scripts/ci/e2e_server_ui.mjs` enforces the shell contract (nav groups,
  mobile tabs, search form, no vertical h1 wrapping, no horizontal overflow).
