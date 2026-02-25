# apex-api

Axum HTTP API server exposing the ApexIntel intelligence platform over REST.

## Responsibilities

- **Route handlers** (`routes/`): Handlers for companies, persons (POIs), recipes, insights, admin, and health endpoints.
- **Authentication** (`auth.rs`): API key management, role-based access (`Admin`, `Analyst`, `Viewer`, `Service`), bearer token extraction, origin checking, and constant-time comparison.
- **Pagination** (`pagination.rs`): `PageParams` and `CursorParams` with `#[serde(deny_unknown_fields)]` to reject malformed inputs.
- **Filters** (`filters.rs`): Typed query filter structs (`WarningFilters`, `CompanyFilters`, `PersonFilters`, etc.) with strict-mode deserialization.
- **Responses** (`responses.rs`): Standard `ApiResponse<T>` envelope, `PagedResponse<T>`, `ApiError`, `ErrorCode`, and `ResponseMeta`.  Use `map_apex_error()` to convert domain errors consistently.
- **Main** (`main.rs`): Router construction, middleware (tracing, CORS, request-id, body limiting), graceful shutdown.

## Key conventions

- All API inputs carry `#[serde(deny_unknown_fields)]` to prevent silent field drift.
- Errors are mapped centrally via `map_apex_error(err: &ApexError) -> ApiError` (B284).
- HTTP status codes follow `ErrorCode::http_status()` — never hardcode status integers in handlers.
- All list endpoints return `PagedResponse<T>` inside `ApiResponse`.

## Running

```bash
cargo run --bin apex-api
```

Reads configuration from environment variables — see `crates/core/src/config.rs`.

## Endpoints (summary)

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| GET | `/health` | None | Liveness + component health |
| GET | `/v1/companies` | Analyst | List companies with filters |
| GET | `/v1/persons` | Analyst | List POIs |
| GET | `/v1/recipes` | Analyst | List recipes |
| POST | `/v1/recipes/:id/promote` | Admin | Promote staged recipe |
| GET | `/v1/insights` | Analyst | List insight cards |
| GET | `/admin/crawl-status` | Admin | Crawl pipeline status |
