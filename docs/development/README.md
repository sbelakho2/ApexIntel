# Development Docs

## Expected warnings in the local/test environment

Worker runs against a local or CI test setup emit warnings that are known,
environmental, and not failures:

- `LLM` not configured — no model endpoint is reachable, so LLM-backed jobs
  degrade instead of running.
- DNS warnings for `.test` fixture domains — those hostnames are intentionally
  unresolvable.
- OFAC/sanctions list not downloadable — the sandbox has no access to the
  public list; the source records a parse/transport failure and retries.
- `headless browser disabled` — Chromium is not installed, so browser-strategy
  sources report unavailable.

The CI pass/fail contract is unaffected by these; they disappear in a
configured deployment. Treat any *other* warning as a defect.

## Sensei-Rams Documentation Set

- [Sensei Modern Style Guide](./sensei-modern-style-guide.md)
- [Sensei-Rams Implementation Guide](./sensei-rams-implementation-guide.md)
- [Sensei-Rams Anti-Patterns](./sensei-rams-anti-patterns.md)
- [Sensei-Rams Accessibility Guide](./sensei-rams-accessibility.md)

These documents define the canonical UI/UX design and implementation standards for ApexIntel's Sensei-Rams infrastructure.

## Frontend Architecture

- [Frontend Architecture Overview](./frontend.md)
- [Frontend Integration Boundaries](./frontend-integration-boundaries.md)
