# Frontend Integration Boundaries

Date: 2026-03-13

This repository currently has two frontend surfaces with different ownership boundaries:

- `frontend/` is the canonical product web application. It owns browser-facing route composition, API consumption, and end-to-end checks.
- `crates/frontend/` is a Rust workspace crate that contains reusable WASM/chart components. It is not the primary application shell and should not be treated as the default location for API contract changes.

API integration expectations:

- Browser-facing API consumers should target `/api/*` routes.
- Machine-readable contract discovery lives at `/api/openapi.json`.
- Capability discovery lives at `/api/features`.
- Human-oriented API discovery lives at `/api/docs`.

Ownership guidance:

- Changes to HTTP base paths, auth expectations, or response contracts should be validated against `frontend/` first.
- Accessibility or visualization work that is specific to Rust/WASM chart components may still touch `crates/frontend/`.
- New documentation should explicitly name which frontend surface it refers to so `frontend/` and `crates/frontend/` are not conflated.