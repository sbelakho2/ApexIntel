# ApexIntel

Self-hosted competitive intelligence and OSINT platform: crawl hundreds of
public sources, build an entity graph of companies and people, generate
LLM-grounded insights and warnings, triage them like an analyst, and publish
battlecards and digests for decision-makers.

## What it does

ApexIntel implements the full intelligence loop that commercial CI platforms
(Klue, Crayon, Kompyte, Contify) monetize, plus OSINT capabilities usually
reserved for threat-intel suites (Recorded Future, OpenCTI):

| Loop stage | ApexIntel implementation |
|---|---|
| **Collect** | 680-source registry (news, filings, tenders, CVE/KEV, dark web, social, OpenAlex, RDAP, DNS), scheduled worker with timeouts, concurrency caps, and circuit breakers. Sources are registered — not operational — until a successful fetch/parser contract check; coverage reports Registered / Validated / Operational / Credential-blocked / Unsupported / Temporarily degraded, and only validated sources count toward the operational metric. Browser-strategy sources render through headless Chromium (installed in `Dockerfile.worker`, enabled with `ENABLE_HEADLESS_BROWSER`); without it they are reported unavailable and never downgraded to plain HTTP |
| **Normalize & dedup** | Content-derived observation IDs — the same fact fetched twice is stored once; entity linking via name gazettees + LLM extraction |
| **Analyze** | LLM insight generation grounded in observation text (anti-hallucination word/proper-noun checks), recipe engine with thresholds and cooldowns, predictive pattern priors, anomaly/volume scanners |
| **Triage** | Scored triage queue with semantic near-duplicate detection, acknowledge/resolve/dismiss workflow, score overrides |
| **Deliver** | Web dashboard (Rams UI), warnings with realtime WebSocket/SSE, email digests, CSV/PDF exports, battlecards with feature matrices, objection handlers and kill shots |
| **Learn** | Recipe precision/recall tracking, feedback ingestion, promotion/deprecation board, weekly self-improvement cycle |

## Architecture

```
┌────────────┐   crawls    ┌─────────────┐   inserts    ┌──────────────┐
│  sources   │────────────▶│ apex-crawl  │─────────────▶│ observations │
│ (registry) │             │ (politeness,│              │  (dedup IDs) │
└────────────┘             │ robots, RL) │              └──────┬───────┘
                           └─────────────┘                     │
                     ┌─────────────────────────────────────────┘
                     ▼
        ┌────────────────────────┐        ┌──────────────────┐
        │ apex-worker (scheduler)│───────▶│ insights/warnings│
        │ 46 jobs, timeouts,     │        │ battlecards, memos│
        │ concurrency, leases    │        └────────┬─────────┘
        └───────────┬────────────┘                 │
                    │ PostgreSQL (+pgvector, Tantivy FST/FTS)
                    ▼
        ┌────────────────────────┐        ┌──────────────────┐
        │ apex-api (axum)        │───────▶│ Web UI (Askama + │
        │ REST + HTML + WS/SSE   │        │ HTMX, Rams theme)│
        └────────────────────────┘        └──────────────────┘
```

**Crates** (18):

- `core` — entities, validation, analysis primitives
- `crawl` — HTTP politeness layer, per-source clients (RSS, SEC EDGAR, CVE, OpenAlex, RDAP, DNS, dark web, social, contact enrichment)
- `parse` — HTML/text extraction, multilingual detection, normalization
- `store` — PostgreSQL (sqlx) repositories, Tantivy search index, autocomplete FST
- `graph` — adjacency, community detection, entity resolution, risk propagation
- `stats` — anomaly detection, changepoints, Bayesian updating, Granger, FDR
- `learning` — pattern mining, hypothesis generation, backtesting
- `llm` — inference clients, validators, anti-hallucination grounding, embeddings, RAG, agents
- `insights` — insight generation, battlecards, predictive patterns, entity relevance, feedback
- `poi` — persons of interest: features, buying centers, role classification, psych profiles
- `recipes` — signal→threshold→action recipe engine
- `triage` — scored queue, semantic dedup, LLM scoring
- `threat_intel` — threat-actor database, MITRE ATT&CK mapping
- `investigation` — analyst investigation engine (workflows, narratives, threats)
- `worker` — job scheduler and all pipeline execution
- `api` — REST API + server-rendered web application
- `shared`, `frontend` — shared types, WASM frontend experiments

## Quick start

```bash
# 1. Provision PostgreSQL (with pgvector) and (optionally) Redis
docker compose up -d postgres redis

# 2. Configure
cp .env.example .env
#   DATABASE_URL, SESSION_SECRET (random 32+ bytes),
#   API_KEY_1..N (keys + roles), LLM_BASE_URL (OpenAI-compatible)

# 3. Migrate
cargo run -p apex-api --bin apex-api   # runs sqlx migrations on boot
# (or) sqlx migrate run

# 4. Run
cargo run --release -p apex-worker &   # pipeline
cargo run --release -p apex-api        # UI on :8080 (see SERVER_PORT)
```

Log in with `WEB_USERS` credentials; API clients use `Authorization: Bearer
<API_KEY_N>` keys (roles: admin > analyst > viewer). Browser sessions now
also authenticate read + CSRF-checked write calls to `/api/*` endpoints
(exports, charts, graph actions work directly from the UI).

## Key operations

- **Worker jobs**: 46 scheduled kinds (crawl cycle hourly; SLA enforcement
  every 10 min; insight generation every 6 h; nightly pattern mining,
  hypothesis, POI refresh; weekly strategy memo + self-improvement).
  Timeouts and `WORKER_MAX_CONCURRENT_JOBS` are enforced; jobs claim a
  DB lease so replicas cannot double-fire.
- **Manual triggers**: `POST /api/admin/trigger-scan` (admin key) or the
  Security page in the UI.
- **Metrics**: `GET /metrics` (Prometheus format, requires an API key).
- **Digests**: set `SMTP_*` env; users opt in per-category and receive a
  curated update email when due.

## Documentation

- `docs/RUNBOOK.md` — operating the platform (jobs, alerts, recovery)
- `DEPLOYMENT.md` — deployment topologies (Docker, proxy notes)
- `docs/CHANGELOG.md` — fix-batch history (B### = backend, U### = UI)
- `docs/development/` — frontend guides and style references

## Testing

```bash
cargo test --workspace        # ~2,000 unit/integration tests
cargo clippy --workspace --all-targets
npx tailwindcss -i crates/api/static/css/globals.css \
  -o crates/api/static/css/tailwind.css --minify   # after CSS edits

# Container-level browser check: builds Dockerfile.worker's browser-check stage
# and renders a JS-only fixture (late network + lazy-loaded DOM) inside it.
bash scripts/ci/browser_container_check.sh          # requires docker
```
